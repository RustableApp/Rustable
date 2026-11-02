//! Preserve the first Rust panic before a native callback abort hides its cause.
use std::{
    fs::OpenOptions,
    io::Write,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

fn report_path() -> Option<std::path::PathBuf> {
    let app = if cfg!(feature = "qa") {
        "Rustable QA"
    } else if cfg!(debug_assertions) {
        "Rustable Development"
    } else {
        "Rustable"
    };
    eframe::storage_dir(app).map(|directory| directory.join("last-panic.log"))
}

pub fn install() {
    let Some(path) = report_path() else {
        return;
    };
    let previous = std::panic::take_hook();
    let first = AtomicBool::new(true);
    std::panic::set_hook(Box::new(move |info| {
        // In particular, do not replace the original failure with the subsequent
        // "panic in a function that cannot unwind" from an Objective-C callback.
        if first.swap(false, Ordering::Relaxed) {
            let report = format!(
                "Rustable {} · {} / {}\nRecorded: {}\nThread: {}\n{info}\n{}\n",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                chrono::Utc::now().to_rfc3339(),
                std::thread::current().name().unwrap_or("unnamed"),
                std::backtrace::Backtrace::force_capture()
            );
            let _ = write_report(&path, &report);
        }
        previous(info);
    }));
}

fn write_report(path: &Path, report: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(report.as_bytes())?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    #[test]
    fn report_is_private_and_replaces_previous_report() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logs/last-panic.log");
        super::write_report(&path, "first longer report").unwrap();
        super::write_report(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

// Public ingestion key, not an account credential. No client is initialized at
// startup: only the user's explicit Send action reaches this endpoint.
const DSN: &str =
    "https://0f8c84cae872d9d128cdee909c0bf636@o606245.ingest.us.sentry.io/4512363452170240";
const ENDPOINT: &str = "https://o606245.ingest.us.sentry.io/api/4512363452170240/envelope/";
const MAX_REPORT: u64 = 1024 * 1024;

pub struct CrashPrompt {
    path: std::path::PathBuf,
    original_digest: String,
    report: String,
    event_id: String,
    review: bool,
    pending: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    status: String,
    done: bool,
}

fn digest(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn pending_report(path: &Path) -> Option<(String, String)> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_REPORT + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_REPORT {
        return None;
    }
    let hash = digest(&bytes);
    if std::fs::read_to_string(path.with_extension("handled"))
        .ok()
        .as_deref()
        == Some(&hash)
    {
        return None;
    }
    Some((String::from_utf8(bytes).ok()?, hash))
}

impl CrashPrompt {
    pub fn load() -> Option<Self> {
        let path = report_path()?;
        let (mut report, original_digest) = pending_report(&path)?;
        // Remove the local home path before showing exactly what will be sent.
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            let home = home.to_string_lossy();
            if !home.is_empty() {
                report = report.replace(home.as_ref(), "~");
            }
        }
        Some(Self {
            path,
            original_digest: original_digest.clone(),
            report,
            event_id: original_digest[..32].to_owned(),
            review: false,
            pending: None,
            status: String::new(),
            done: false,
        })
    }

    fn acknowledge(&mut self) -> bool {
        match write_report(&self.path.with_extension("handled"), &self.original_digest) {
            Ok(()) => true,
            Err(_) => {
                self.status = "Could not remember your choice. The report remains on disk.".into();
                false
            }
        }
    }

    pub fn show(&mut self, ctx: &eframe::egui::Context) -> bool {
        use eframe::egui;
        if let Some(receiver) = &self.pending {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Report sender stopped. You can retry.".into()))
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.pending = None;
                match result {
                    Ok(()) => {
                        self.done = true;
                        self.status = "Report sent. Thank you for helping improve Rustable.".into();
                        self.acknowledge();
                    }
                    Err(error) => self.status = error,
                }
            }
        }
        let mut close = false;
        egui::Modal::new(egui::Id::new("previous-crash-report")).show(ctx, |ui| {
            ui.set_width(520.0_f32.min(ctx.content_rect().width() - 48.0));
            ui.heading("Rustable stopped unexpectedly");
            ui.label("A Rust panic was recorded in the previous session. You can send its diagnostic report to the Rustable team through Sentry (US). Nothing is sent automatically.");
            ui.label("The report includes the panic text, backtrace and build details. Panic text can contain data from the operation that failed. Review and remove anything private before sending.");
            if !self.done {
                ui.add_enabled_ui(self.pending.is_none(), |ui| {
                    ui.checkbox(&mut self.review, "Review report");
                    if self.review {
                        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                            ui.add(egui::TextEdit::multiline(&mut self.report).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
                        });
                    }
                });
            }
            if !self.status.is_empty() { ui.label(&self.status); }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if self.done {
                    if ui.button("Close").clicked() { close = true; }
                } else if self.pending.is_some() {
                    ui.spinner(); ui.label("Sending report…");
                } else {
                    if ui.button("Don't send").clicked() { close = self.acknowledge(); }
                    if ui.add_enabled(!self.report.trim().is_empty(), egui::Button::new("Send report")).clicked() {
                        let report = self.report.clone();
                        let id = self.event_id.clone();
                        let ctx = ctx.clone();
                        let (tx, rx) = std::sync::mpsc::channel();
                        self.pending = Some(rx);
                        self.status.clear();
                        std::thread::spawn(move || {
                            let result = send_report(ENDPOINT, &id, &report);
                            let _ = tx.send(result);
                            ctx.request_repaint();
                        });
                    }
                }
            });
        });
        close
    }
}

fn envelope(id: &str, report: &str) -> String {
    // Preserve crash-time version rather than the version reopened after an update.
    let release = report
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("Rustable "))
        .and_then(|line| line.split_whitespace().next())
        .unwrap_or("unknown");
    let location = report.lines().find(|line| line.starts_with("panicked at "));
    let fingerprint = location.map_or_else(
        || vec!["{{ default }}".to_owned()],
        |line| vec!["rust-panic".to_owned(), line.to_owned()],
    );
    let event = serde_json::json!({
        "event_id": id, "platform": "native", "level": "fatal",
        "release": format!("rustable@{release}"),
        "exception": {"values": [{"type": "RustPanic", "value": "User-submitted Rustable panic"}]},
        "fingerprint": fingerprint,
        "extra": {"panic_report": report}
    })
    .to_string();
    format!(
        "{}\n{}\n{}\n{}\n{}\n",
        serde_json::json!({"event_id": id, "dsn": DSN}),
        serde_json::json!({"type": "event", "length": event.len()}),
        event,
        serde_json::json!({"type": "attachment", "length": report.len(), "filename": "panic-report.txt", "content_type": "text/plain"}),
        report
    )
}

fn send_report(endpoint: &str, id: &str, report: &str) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not start report sender.".to_owned())?;
    let response = client
        .post(endpoint)
        .header("Content-Type", "application/x-sentry-envelope")
        .body(envelope(id, report))
        .send()
        .map_err(|_| {
            "Could not reach Sentry. Nothing was marked as sent; please retry.".to_owned()
        })?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "Sentry returned HTTP {}. Please retry later.",
            response.status().as_u16()
        ))
    }
}

#[cfg(test)]
mod reporting_tests {
    use super::*;
    #[test]
    fn acknowledgement_only_suppresses_the_same_report() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("last-panic.log");
        write_report(&path, "first").unwrap();
        let (_, hash) = pending_report(&path).unwrap();
        write_report(&path.with_extension("handled"), &hash).unwrap();
        assert!(pending_report(&path).is_none());
        write_report(&path, "second").unwrap();
        assert!(pending_report(&path).is_some());
    }
    #[test]
    fn envelope_preserves_reviewed_report_and_crash_version() {
        let text = "Rustable 0.4.4 · macos / aarch64\nreviewed ☃ report";
        let data = envelope("0123456789abcdef0123456789abcdef", text);
        let lines: Vec<_> = data.lines().collect();
        let header: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(header["length"], lines[2].len());
        let event: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(event["release"], "rustable@0.4.4");
        assert_eq!(event["extra"]["panic_report"], text);
        assert!(event.get("user").is_none());
        let attachment: serde_json::Value = serde_json::from_str(lines[3]).unwrap();
        assert_eq!(attachment["length"], text.len());
        assert_eq!(attachment["type"], "attachment");
        assert_eq!(data.splitn(5, '\n').nth(4).unwrap(), format!("{text}\n"));
    }
    #[test]
    fn oversized_report_is_not_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("last-panic.log");
        write_report(&path, &"x".repeat(MAX_REPORT as usize + 1)).unwrap();
        assert!(pending_report(&path).is_none());
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use std::io::{Read, Write};
    #[test]
    fn delivery_checks_http_status_and_does_not_follow_redirects() {
        for status in [200, 302, 429, 500] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}/", listener.local_addr().unwrap());
            let worker = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let count = socket.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&request[..end]).to_lowercase();
                        let length: usize = header
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                write!(socket, "HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nLocation: http://127.0.0.1:1/\r\nConnection: close\r\n\r\n").unwrap();
            });
            assert_eq!(
                send_report(&endpoint, "0123456789abcdef0123456789abcdef", "synthetic").is_ok(),
                status == 200
            );
            worker.join().unwrap();
        }
    }
    #[test]
    #[ignore = "sends one synthetic setup event to the configured Sentry project"]
    fn sentry_setup_delivery() {
        let report = format!(
            "Rustable {} · synthetic / setup\nSentry setup verification only. No user data or actual crash.\n{}",
            env!("CARGO_PKG_VERSION"),
            chrono::Utc::now()
        );
        let id = digest(report.as_bytes());
        send_report(ENDPOINT, &id[..32], &report).unwrap();
        println!("Sentry accepted setup event {}", &id[..32]);
    }
}
