//! Dedicated MONITOR transport. Never shares the browsing connection.
//! RESP2 MONITOR uses simple-string lines, so cap the wire frame before allocation.
use crate::redis_settings::Settings;
#[cfg(test)]
use std::io::{BufReader, Write};
use eframe::egui;
use std::{
    io::BufRead,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
const WIRE_LIMIT: usize = 65_536;
const PREVIEW_LIMIT: usize = 4096;
const CAPTURE_SECONDS: u64 = 60;

#[derive(Default)]
struct Shared {
    control: crate::redis_stream::Control,
    status: Mutex<String>,
    done: AtomicBool,
    dropped: AtomicU64,
}
pub(crate) struct Capture {
    shared: Arc<Shared>,
    pub receiver: mpsc::Receiver<String>,
}
impl Capture {
    #[cfg(test)]
    pub fn start(settings: Settings, password: String, ctx: egui::Context) -> Self {
        Self::start_with_sentinel(settings, password, String::new(), ctx)
    }
    pub fn start_with_sentinel(settings: Settings, password: String, sentinel_password: String, ctx: egui::Context) -> Self {
        let password = zeroize::Zeroizing::new(password);
        let sentinel_password = zeroize::Zeroizing::new(sentinel_password);
        let shared = Arc::new(Shared::default());
        *shared.status.lock().unwrap() = "Connecting…".into();
        let (sender, receiver) = mpsc::sync_channel(64);
        let worker = shared.clone();
        std::thread::spawn(move || {
            let result = run(&settings, &password, &sentinel_password, &worker, &sender, &ctx);
            let message = if worker.control.cancelled() {
                "Stopped".into()
            } else {
                result.unwrap_or_else(|error| format!("Capture stopped: {error}"))
            };
            *worker.status.lock().unwrap() = message;
            worker.control.release();
            worker.done.store(true, Ordering::Release);
            ctx.request_repaint();
        });
        Self { shared, receiver }
    }
    pub fn stop(&self) {
        self.shared.control.stop();
    }
    pub fn done(&self) -> bool {
        self.shared.done.load(Ordering::Acquire)
    }
    pub fn status(&self) -> String {
        self.shared.status.lock().unwrap().clone()
    }
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}
fn run(
    settings: &Settings,
    password: &str,
    sentinel_password: &str,
    shared: &Shared,
    sender: &mpsc::SyncSender<String>,
    ctx: &egui::Context,
) -> Result<String, String> {
    let mut reader = shared.control.start(settings, password, sentinel_password, &redis::cmd("MONITOR"))?;
    let mut line = Vec::new();
    expect_ok(&mut reader, &mut line)?;
    *shared.status.lock().unwrap() = "Capturing · stops after 60 seconds".into();
    ctx.request_repaint();
    shared.control.read_timeout(Duration::from_millis(200))?;
    let end = Instant::now() + Duration::from_secs(CAPTURE_SECONDS);
    let mut count = 0usize;
    loop {
        if shared.control.cancelled() {
            return Ok("Stopped".into());
        }
        if Instant::now() >= end {
            return Ok("Finished · 60-second capture limit".into());
        }
        match read_line(&mut reader, &mut line) {
            Ok(()) => {
                if line.first() != Some(&b'+') {
                    return Err("Unexpected MONITOR response".into());
                }
                let bytes = &line[1..line.len() - 2];
                let mut preview =
                    String::from_utf8_lossy(&bytes[..bytes.len().min(PREVIEW_LIMIT)]).into_owned();
                if bytes.len() > PREVIEW_LIMIT {
                    preview.push_str(" … [truncated]");
                }
                match sender.try_send(preview) {
                    Ok(()) => ctx.request_repaint_after(Duration::from_millis(100)),
                    Err(mpsc::TrySendError::Full(_)) => {
                        shared.dropped.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => return Ok("Stopped".into()),
                }
                line.clear();
                count += 1;
                if count >= 100_000 {
                    return Ok("Finished · 100,000-command capture limit".into());
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
}
fn expect_ok(reader: &mut impl BufRead, line: &mut Vec<u8>) -> Result<(), String> {
    line.clear();
    read_line(reader, line).map_err(|e| e.to_string())?;
    if line == b"+OK\r\n" {
        line.clear();
        Ok(())
    } else if line.first() == Some(&b'-') {
        Err(String::from_utf8_lossy(&line[1..line.len().min(1025) - 2]).into_owned())
    } else {
        Err("Unexpected Redis setup response".into())
    }
}
/// Keeps partial bytes across socket timeouts; never asks a RESP parser to
/// resume an interrupted value or allocates an unbounded command argument.
fn read_line(reader: &mut impl BufRead, line: &mut Vec<u8>) -> std::io::Result<()> {
    loop {
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Redis closed the capture connection",
            ));
        }
        let count = bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |i| i + 1);
        if line.len() + count > WIRE_LIMIT {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "MONITOR frame exceeds 64 KiB; capture stopped",
            ));
        }
        line.extend_from_slice(&bytes[..count]);
        reader.consume(count);
        if line.last() == Some(&b'\n') {
            if !line.ends_with(b"\r\n") {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Malformed MONITOR line",
                ));
            }
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    #[test]
    fn bounded_lines_and_multiple_frames() {
        let mut reader = BufReader::with_capacity(3, &b"+one\r\n+two\r\n"[..]);
        let mut line = Vec::new();
        read_line(&mut reader, &mut line).unwrap();
        assert_eq!(line, b"+one\r\n");
        line.clear();
        read_line(&mut reader, &mut line).unwrap();
        assert_eq!(line, b"+two\r\n");
        let oversized = vec![b'x'; WIRE_LIMIT + 1];
        line.clear();
        assert_eq!(
            read_line(&mut &oversized[..], &mut line)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidData
        );
        assert!(line.len() <= WIRE_LIMIT);
        assert!(read_line(&mut &b"+bad\n"[..], &mut Vec::new()).is_err());
    }
    #[test]
    fn partial_line_survives_timeout() {
        let mut line = b"+123 ".to_vec();
        struct Timeout;
        impl Read for Timeout {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::TimedOut.into())
            }
        }
        assert!(read_line(&mut BufReader::new(Timeout), &mut line).is_err());
        read_line(&mut &b"[0 client] \"GET\" \"x\"\r\n"[..], &mut line).unwrap();
        assert_eq!(line, b"+123 [0 client] \"GET\" \"x\"\r\n");
    }
}

pub(crate) struct View {
    pub open: bool,
    settings: Settings,
    password: zeroize::Zeroizing<String>,
    sentinel_password: zeroize::Zeroizing<String>,
    capture: Option<Capture>,
    lines: std::collections::VecDeque<String>,
    received: u64,
    follow: bool,
}
impl View {
    pub fn new(settings: Settings, password: String, sentinel_password: String) -> Self {
        Self {
            open: false,
            settings,
            password: zeroize::Zeroizing::new(password),
            sentinel_password: zeroize::Zeroizing::new(sentinel_password),
            capture: None,
            lines: Default::default(),
            received: 0,
            follow: true,
        }
    }
    pub fn show(&mut self, ctx: &egui::Context, connected: bool) {
        if let Some(capture) = &self.capture {
            // Bound result processing independently of the virtualized table.
            let mut drained = 0;
            while drained < 16 {
                let Ok(line) = capture.receiver.try_recv() else {
                    break;
                };
                if self.lines.len() == 2000 {
                    self.lines.pop_front();
                }
                self.lines.push_back(line);
                self.received += 1;
                drained += 1;
            }
            if drained == 16 {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        if !self.open {
            return;
        }
        let mut open = true;
        egui::Window::new("Live command profiler")
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(840.0)
            .default_height(430.0)
            .max_height(600.0)
            .vscroll(true)
            .show(ctx, |ui| {
                crate::controls::base_spacing(ui.spacing_mut());
                crate::controls::rounded_action_buttons(ui);
                ui.add(egui::Label::new("Capture commands across all databases on this server. MONITOR adds server overhead and may expose sensitive arguments. Capture stops after 60 seconds; nothing is saved to disk.").wrap());
                ui.add_space(8.0);
                let active = self.capture.as_ref().is_some_and(|c| !c.done());
                ui.horizontal(|ui| {
                    if ui.add_enabled(connected && !active, egui::Button::new("Start capture")).clicked() {
                        self.lines.clear();
                        self.received = 0;
                        self.capture = Some(Capture::start_with_sentinel(self.settings.clone(), self.password.to_string(), self.sentinel_password.to_string(), ctx.clone()));
                    }
                    if ui.add_enabled(active, egui::Button::new("Stop")).clicked() {
                        self.capture.as_ref().unwrap().stop();
                    }
                    if ui.add_enabled(!self.lines.is_empty(), egui::Button::new("Clear")).clicked() {
                        self.lines.clear();
                        self.received = 0;
                    }
                    ui.separator();
                    ui.checkbox(&mut self.follow, "Follow latest");
                });
                ui.add_space(5.0);
                if let Some(capture) = &self.capture {
                    ui.label(capture.status());
                    ui.weak(format!("{} received · {} retained · {} dropped while busy", self.received, self.lines.len(), capture.dropped()));
                } else { ui.weak("Ready · 2,000 retained commands · 4 KiB argument previews"); }
                ui.separator();
                ui.horizontal(|ui| { ui.strong("Timestamp · database / client · command and arguments"); });
                if self.lines.is_empty() {
                    ui.add_space(24.0);
                    ui.weak(if active { "Waiting for commands…" } else { "Start a short capture to see live Redis activity." });
                } else {
                    egui::ScrollArea::both().id_salt("monitor-lines").max_height(220.0)
                        .stick_to_bottom(self.follow).auto_shrink([false, false])
                        .show_rows(ui, 20.0, self.lines.len(), |ui, range| {
                            for i in range {
                                ui.add(egui::Label::new(egui::RichText::new(&self.lines[i]).monospace()).extend());
                            }
                        });
                }
                ui.add_space(5.0);
                ui.add(egui::Label::new(egui::RichText::new("Timestamps show arrival at Redis, not execution duration. Use Server & slow log for slow-command timings. Frames over 64 KiB stop capture; busy UI queues drop commands and report the count.").weak()).wrap());
            });
        if !open && let Some(capture) = &self.capture {
            capture.stop();
        }
        self.open = open;
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use std::io::Read;
    fn wait_until(mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(4);
        while !ready() {
            assert!(Instant::now() < deadline, "capture worker did not settle");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn pipelined_start_bounded_queue_and_idle_stop() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut parser = redis::Parser::new();
            let auth: Vec<String> =
                redis::from_redis_value(parser.parse_value(&mut socket).unwrap()).unwrap();
            let monitor: Vec<String> =
                redis::from_redis_value(parser.parse_value(&mut socket).unwrap()).unwrap();
            assert_eq!(auth, ["AUTH", "fixture-only"]);
            assert_eq!(monitor, ["MONITOR"]);
            // Both requests must arrive before either reply; no setup preflight.
            socket.write_all(b"+OK\r\n+OK\r\n").unwrap();
            for _ in 0..200 {
                socket
                    .write_all(b"+123.456 [0 client] \"GET\" \"key\"\r\n")
                    .unwrap();
            }
            let mut byte = [0];
            assert_eq!(
                socket.read(&mut byte).unwrap(),
                0,
                "Stop closes without cleanup commands"
            );
        });
        let capture = Capture::start(
            Settings {
                host: "127.0.0.1".into(),
                port,
                tls: false,
                ..Default::default()
            },
            "fixture-only".into(),
            egui::Context::default(),
        );
        wait_until(|| capture.dropped() == 136);
        assert_eq!(capture.receiver.try_iter().count(), 64);
        let started = Instant::now();
        capture.stop();
        wait_until(|| capture.done());
        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(capture.status(), "Stopped");
        peer.join().unwrap();
    }
    #[test]
    fn oversized_stream_stops_without_unbounded_allocation() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut parser = redis::Parser::new();
            let _: redis::Value = parser.parse_value(&mut socket).unwrap();
            socket.write_all(b"+OK\r\n+").unwrap();
            let _ = socket.write_all(&vec![b'x'; WIRE_LIMIT + 10]);
        });
        let capture = Capture::start(
            Settings {
                host: "127.0.0.1".into(),
                port,
                tls: false,
                ..Default::default()
            },
            String::new(),
            egui::Context::default(),
        );
        wait_until(|| capture.done());
        assert!(capture.status().contains("64 KiB"), "{}", capture.status());
        assert_eq!(capture.receiver.try_iter().count(), 0);
        peer.join().unwrap();
    }
    #[test]
    #[ignore = "disposable Redis plain and TLS fixtures via scripts/test-redis.sh"]
    fn live_plain_tls_and_identity_validation() {
        for tls in [false, true] {
            let settings = Settings {
                host: "127.0.0.1".into(),
                port: if tls { 16380 } else { 16379 },
                tls,
                root_certificate: Some(
                    std::env::var_os("RUSTABLE_REDIS_TEST_CA")
                        .expect("fixture CA")
                        .into(),
                ),
                ..Default::default()
            };
            let capture = Capture::start(
                settings.clone(),
                "rustable-redis-test-only".into(),
                egui::Context::default(),
            );
            wait_until(|| capture.status().starts_with("Capturing") || capture.done());
            assert!(!capture.done(), "{}", capture.status());
            let mut client =
                crate::redis_session::Session::connect(&settings, "rustable-redis-test-only")
                    .unwrap();
            client.execute("PING", true).unwrap();
            let mut observed = false;
            wait_until(|| {
                observed |= capture
                    .receiver
                    .try_iter()
                    .any(|line| line.contains("PING") || line.contains("ping"));
                observed
            });
            capture.stop();
            wait_until(|| capture.done());
            if tls {
                let untrusted = Capture::start(
                    Settings {
                        root_certificate: None,
                        ..settings.clone()
                    },
                    "rustable-redis-test-only".into(),
                    egui::Context::default(),
                );
                wait_until(|| untrusted.done());
                assert!(untrusted.status().contains("TLS"));
                let wrong_host = Capture::start(
                    Settings {
                        host: "localhost".into(),
                        ..settings
                    },
                    "rustable-redis-test-only".into(),
                    egui::Context::default(),
                );
                wait_until(|| wrong_host.done());
                assert!(wrong_host.status().contains("TLS"));
            }
        }
    }
}

#[cfg(feature = "qa")]
impl View {
    pub fn preview(&mut self, ctx: &egui::Context, step: usize) -> bool {
        self.open = true;
        match step {
            0 => true,
            1 => {
                if self.capture.is_none() {
                    self.capture = Some(Capture::start_with_sentinel(
                        self.settings.clone(),
                        self.password.to_string(),
                        self.sentinel_password.to_string(),
                        ctx.clone(),
                    ));
                }
                self.capture
                    .as_ref()
                    .unwrap()
                    .status()
                    .starts_with("Capturing")
            }
            _ => {
                if let Some(capture) = &self.capture {
                    capture.stop();
                    capture.done()
                } else {
                    true
                }
            }
        }
    }
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    fn fixture() -> View {
        let mut view = View::new(Settings::default(), String::new(), String::new());
        view.open = true;
        for i in 0..2000 {
            view.lines.push_back(format!(
                "1794082100.{i:06} [0 127.0.0.1:48126] \"GET\" \"app:users:{i:05}\""
            ));
        }
        view
    }
    fn frame(ctx: &egui::Context, view: &mut View, i: usize) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 760.0),
                )),
                time: Some(i as f64 / 120.0),
                ..Default::default()
            },
            |ctx| view.show(ctx, true),
        )
    }
    #[test]
    fn profiler_window_is_bounded_and_idle_settles() {
        let ctx = egui::Context::default();
        let mut view = fixture();
        for i in 0..60 {
            frame(&ctx, &mut view, i);
        }
        let rect = ctx
            .memory(|m| m.area_rect(egui::Id::new("Live command profiler")))
            .unwrap();
        assert!(rect.width() <= 1200.0 && rect.height() <= 760.0, "{rect:?}");
        let output = frame(&ctx, &mut view, 61);
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
                > Duration::from_millis(500)
        );
    }
    #[test]
    #[ignore = "manual live profiler CPU workload"]
    fn redis_monitor_frame_benchmark() {
        let ctx = egui::Context::default();
        let mut view = fixture();
        let mut samples = Vec::new();
        for i in 0..180 {
            let started = Instant::now();
            let output = frame(&ctx, &mut view, i);
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if i >= 20 {
                samples.push(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/live-profiler", &samples);
        let (sender, receiver) = mpsc::sync_channel(64);
        let shared = Arc::new(Shared::default());
        *shared.status.lock().unwrap() = "Capturing · stops after 60 seconds".into();
        view.capture = Some(Capture { shared, receiver });
        samples.clear();
        for i in 0..180 {
            for j in 0..16 {
                sender
                    .try_send(format!(
                        "1794082100.{i:06} [0 client] \"SET\" \"key:{j}\" \"{}\"",
                        "x".repeat(4000)
                    ))
                    .unwrap();
            }
            let started = Instant::now();
            let output = frame(&ctx, &mut view, i + 180);
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if i >= 20 {
                samples.push(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/live-profiler-incoming", &samples);
    }
}
