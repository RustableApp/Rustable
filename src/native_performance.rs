//! Native QA profiler using production SessionWindows and tab close/archive paths.
//! Commands and large fixture allocation live on background threads, never the UI.
use super::*;
use crate::db::Grid;
use std::{io::{BufRead, Write}, sync::{Arc, Weak, mpsc}, time::Instant};

type Tabs = Vec<(String, Arc<Grid>)>;
struct Command { name: String, tabs: Vec<Tabs> }
type Output = serde_json::Value;

#[derive(Clone, Copy, Default, serde::Serialize)]
struct Snapshot {
    elapsed_ms: f64, updates: u64, windows: usize,
    live_result_grids: usize, created_result_grids: usize,
    pixels_per_point: f32, viewport_points: [f32; 2],
}
fn snapshot_event(snapshot: Snapshot, event: &str, phase: &str) -> serde_json::Value {
    let mut value = serde_json::to_value(snapshot).unwrap();
    value["event"] = event.into();
    value["phase"] = phase.into();
    value
}

struct Profile {
    status: Arc<std::sync::Mutex<Snapshot>>,
    sessions: SessionWindows,
    commands: mpsc::Receiver<Command>,
    output: mpsc::Sender<Output>,
    began: Instant,
    updates: u64,
    first: bool,
    leases: Vec<Weak<Grid>>,
}
impl Profile {
    fn add_window(&mut self, ctx: &egui::Context, tabs: Tabs) {
        self.leases.extend(tabs.iter().map(|(_, grid)| Arc::downgrade(grid)));
        let id = egui::ViewportId::from_hash_of(("native-profile", self.sessions.next_id));
        self.sessions.next_id += 1;
        self.sessions.windows.push(SessionWindow {
            id, title: "Disposable performance fixture".into(), size_key: None,
            initial_size: [1280.0, 820.0], request: None,
            app: Some(Box::new(Rustable::native_profile_workspace(ctx, tabs))),
            parked: Vec::new(), active_slot: 0, closed: false, connected: true,
        });
    }
    fn report(&self, event: &str, phase: &str) {
        let state = *self.status.lock().unwrap();
        let _ = self.output.send(snapshot_event(state, event, phase));
    }

}
impl eframe::App for Profile {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.updates += 1;
        let mut report = None;
        while let Ok(mut command) = self.commands.try_recv() {
            match command.name.as_str() {
                "one-tab" => {
                    let tabs = command.tabs.pop().unwrap();
                    if let Some(window) = self.sessions.windows.first_mut() {
                        self.leases.extend(tabs.iter().map(|(_, grid)| Arc::downgrade(grid)));
                        window.app.as_mut().unwrap().native_profile_load(tabs);
                        ctx.request_repaint_of(window.id);
                    } else { self.add_window(ctx, tabs); }
                }
                "tabs20" => {
                    let tabs = command.tabs.pop().unwrap();
                    self.leases.extend(tabs.iter().map(|(_, grid)| Arc::downgrade(grid)));
                    let window = self.sessions.windows.first_mut().unwrap();
                    window.app.as_mut().unwrap().native_profile_load(tabs);
                    ctx.request_repaint_of(window.id);
                }
                "close-tabs" | "close-last" => {
                    let window = self.sessions.windows.first_mut().unwrap();
                    window.app.as_mut().unwrap().native_profile_close_tabs(usize::from(command.name == "close-tabs"));
                    ctx.request_repaint_of(window.id);
                }
                "three-windows" => {
                    for tabs in command.tabs { self.add_window(ctx, tabs); }
                }
                "close-windows" | "close-all-windows" => {
                    let keep = usize::from(command.name == "close-windows");
                    for window in self.sessions.windows.iter_mut().skip(keep) { window.closed = true; }
                }
                "quit" => {
                    self.sessions.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                "launcher-idle" | "snapshot" => {}
                _ => panic!("unsupported profiler command"),
            }
            report = Some(("phase", command.name));
        }
        self.sessions.update(ctx, frame);
        *self.status.lock().unwrap() = Snapshot {
            elapsed_ms: self.began.elapsed().as_secs_f64() * 1000.0,
            updates: self.updates, windows: self.sessions.windows.len(),
            live_result_grids: self.leases.iter().filter(|grid| grid.strong_count() > 0).count(),
            created_result_grids: self.leases.len(), pixels_per_point: ctx.pixels_per_point(),
            viewport_points: [ctx.viewport_rect().width(), ctx.viewport_rect().height()],
        };
        if self.first { self.first = false; self.report("ready", "startup"); }
        if let Some((event, phase)) = report { self.report(event, &phase); }
    }
    // Profile never writes normal preferences. SessionWindows' explicit close
    // persistence is confined to the dedicated output directory's storage file.
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {}
}

pub(crate) fn run() -> eframe::Result {
    let output_dir = std::path::PathBuf::from(std::env::var_os("RUSTABLE_NATIVE_PROFILE").unwrap());
    let began = Instant::now();
    let (output, events) = mpsc::channel();
    std::thread::spawn(move || {
        let mut stdout = std::io::stdout().lock();
        for json in events {
            let _ = writeln!(stdout, "RUSTABLE_NATIVE_PROFILE {json}");
            let _ = stdout.flush();
        }
    });
    eframe::run_native("Rustable Performance Fixture", eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Rustable Performance Fixture")
            .with_inner_size([960.0, 860.0]),
        persistence_path: Some(output_dir.join("preferences.ron")),
        persist_window: false,
        ..Default::default()
    }, Box::new(move |cc| {
        crate::scroll_response::install(&cc.egui_ctx);
        let (tx, commands) = mpsc::channel();
        let ctx = cc.egui_ctx.clone();
        let status = Arc::new(std::sync::Mutex::new(Snapshot::default()));
        let background_status = status.clone();
        let background_output = output.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(name) = line else { break };
                if name == "snapshot" {
                    // Read telemetry off-thread without waking the native UI.
                    let mut state = *background_status.lock().unwrap();
                    state.elapsed_ms = began.elapsed().as_secs_f64() * 1000.0;
                    let _ = background_output.send(snapshot_event(state, "snapshot", "snapshot"));
                    continue;
                }
                let tabs = match name.as_str() {
                    "one-tab" => vec![Rustable::native_profile_tabs(1)],
                    "tabs20" => vec![Rustable::native_profile_tabs(20)],
                    "three-windows" => vec![Rustable::native_profile_tabs(5), Rustable::native_profile_tabs(5)],
                    _ => Vec::new(),
                };
                if tx.send(Command { name, tabs }).is_err() { break; }
                ctx.request_repaint();
            }
        });
        let sessions = SessionWindows {
            crash_prompt: None, root: Rustable::new(cc), windows: Vec::new(), next_id: 1,
            launcher_hidden: false, quitting: false, native_menu_installed: false,
            migrate_launcher_size: false, update_save_error: None, window_sizes: Default::default(),
        };
        Ok(Box::new(Profile { sessions, commands, output, began, status, updates: 0,
            first: true, leases: Vec::new() }))
    }))
}
