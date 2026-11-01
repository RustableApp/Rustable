//! Native session windows. Only the root owns saved connection metadata/storage.
use crate::{app::Rustable, connection::PostgresConfig};
use eframe::egui;
use std::path::PathBuf;

const LAUNCHER_SIZE_MIGRATION: &str = "native-launcher-size-v1";
pub const LAUNCHER_SIZE: egui::Vec2 = egui::vec2(960.0, 860.0);

pub enum WindowConnection {
    Demo,
    Sqlite(PathBuf),
    Postgres(Box<PostgresConfig>),
    Mysql(Box<crate::connection::MysqlConfig>),
}
pub struct WindowRequest {
    pub recover_sql: bool,
    pub identity: Option<crate::app::ProfileIdentity>,
    pub title: String,
    pub connection: WindowConnection,
}
struct SessionWindow {
    id: egui::ViewportId,
    title: String,
    request: Option<WindowRequest>,
    app: Option<Box<Rustable>>,
    closed: bool,
    connected: bool,
}

impl SessionWindow {
    fn connection_completed(&mut self, connected: bool) -> bool {
        let completed = connected && !self.connected;
        self.connected |= connected;
        completed
    }
}

pub struct SessionWindows {
    root: Rustable,
    windows: Vec<SessionWindow>,
    next_id: u64,
    launcher_hidden: bool,
    quitting: bool,
    native_menu_installed: bool,
    migrate_launcher_size: bool,
    update_save_error: Option<String>,
}
impl SessionWindows {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::scroll_response::install(&cc.egui_ctx);
        crate::updates::initialize(&cc.egui_ctx);
        Self {
            root: Rustable::new(cc),
            windows: Vec::new(),
            next_id: 1,
            launcher_hidden: false,
            quitting: false,
            native_menu_installed: false,
            update_save_error: None,
            migrate_launcher_size: cc.storage.is_none_or(|storage| {
                storage.get_string(LAUNCHER_SIZE_MIGRATION).as_deref() != Some("done")
            }),
        }
    }
    fn collect_requests(&mut self) -> Option<egui::ViewportId> {
        let mut requests = self.root.take_window_requests();
        for window in &mut self.windows {
            if let Some(app) = &mut window.app { requests.extend(app.take_window_requests()); }
        }
        let mut opened = None;
        for request in requests {
            let id = egui::ViewportId::from_hash_of(("database-session", self.next_id));
            self.next_id += 1;
            opened = Some(id);
            self.windows.push(SessionWindow {
                id,
                title: request.title.clone(),
                request: Some(request),
                app: None,
                closed: false,
                connected: false,
            });
        }
        opened
    }
    fn show_launcher(&mut self, ctx: &egui::Context) {
        self.launcher_hidden = false;
        self.root.open_connection_manager();
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }
}
impl eframe::App for SessionWindows {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // AppKit termination is synchronous. Fail closed while UI state changes;
        // publish readiness only after all immediate child windows have updated.
        crate::native_menu::set_blocked(true);
        if std::mem::take(&mut self.migrate_launcher_size) {
            let size = launcher_size(ctx.input(|input| input.viewport().monitor_size));
            // Eframe restores the former database workspace's geometry before
            // this first frame. Migrate once, then respect later user resizing.
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(
                egui::vec2(760.0, 560.0).min(size),
            ));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }
        if !self.native_menu_installed {
            self.native_menu_installed = crate::native_menu::install(ctx);
        }
        crate::updates::poll();
        let mut quit_requested = consume_quit(ctx) || crate::native_menu::take_requested();
        quit_requested |= self.root.render_launcher(ctx);
        let mut shared_changed = self.root.publish_shared_preferences();
        let opened_window = self.collect_requests();
        let had_windows = !self.windows.is_empty();
        let root_close_requested = ctx.input(|input| input.viewport().close_requested());
        let mut manager_requested = false;
        for window in &mut self.windows {
            ctx.show_viewport_immediate(window.id, egui::ViewportBuilder::default()
                .with_title(format!("{} — Rustable", window.title))
                .with_inner_size([1280.0, 820.0]).with_min_inner_size([960.0, 640.0]), |child_ctx, class| {
                if class == egui::ViewportClass::Embedded {
                    // Native desktop backends support immediate viewports. Never draw
                    // a second full workspace over the root on an unsupported backend.
                    self.root.window_error("This renderer does not support native session windows.");
                    window.closed = true;
                    return;
                }
                let app = window.app.get_or_insert_with(|| Box::new(self.root.new_window(child_ctx, window.request.take().expect("new session request"))));
                quit_requested |= consume_quit(child_ctx);
                app.manager_shortcut(child_ctx);
                self.root.sync_window_preferences(app);
                if self.root.sync_window_profile(app) { child_ctx.request_repaint(); }
                app.update(child_ctx, frame);
                if app.has_connection() && window.title != app.active_connection_name() {
                    window.title = app.active_connection_name().to_owned();
                    child_ctx.request_repaint();
                }
                shared_changed |= self.root.merge_window_preferences(app);
                if app.take_manager_request() { manager_requested = true; }
                if child_ctx.input(|input| input.viewport().close_requested()) {
                    if app.close_blocked() {
                        child_ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                        app.window_error("Finish SQL transactions with COMMIT or ROLLBACK, save or discard pending database changes, and wait for work before closing. SQL drafts are recovered in Connections.");
                    } else { window.closed = true; }
                }
                let connected = app.has_connection();
                window.connection_completed(connected);
            });
        }
        if shared_changed {
            ctx.request_repaint_of(egui::ViewportId::ROOT);
            for window in &mut self.windows {
                if let Some(app) = &mut window.app {
                    self.root.sync_window_preferences(app);
                }
                ctx.request_repaint_of(window.id);
            }
        }
        for window in self.windows.iter_mut().filter(|window| window.closed) {
            if let Some(app) = window.app.take() {
                self.root.archive_window(*app);
            }
        }
        self.windows.retain(|window| !window.closed);
        let blocked_window = self.windows.iter().position(|window| {
            window.request.is_some() || window.app.as_ref().is_some_and(|app| app.close_blocked())
        });
        if quit_requested {
            if self.root.close_blocked() || blocked_window.is_some() {
                self.quitting = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if let Some(index) = blocked_window {
                    let window = &mut self.windows[index];
                    if let Some(app) = &mut window.app {
                        app.window_error("Finish SQL transactions with COMMIT or ROLLBACK, save or discard pending database edits, and wait for running work before quitting. SQL drafts will be recovered in Connections.");
                    }
                    ctx.send_viewport_cmd_to(window.id, egui::ViewportCommand::Focus);
                    ctx.request_repaint_of(window.id);
                } else {
                    self.root
                        .window_error("Wait for the current operation to finish before quitting.");
                    manager_requested = true;
                }
            } else {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if root_close_requested && !self.quitting && !self.windows.is_empty() {
            // The launcher is independently dismissible; closing it must not close
            // sessions or silently commit/discard their state.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.launcher_hidden = true;
        }
        if let Some(id) = opened_window.filter(|_| !self.quitting && !manager_requested) {
            // Hand off immediately to the new window, which owns connection
            // progress and errors. Later connection completion must not re-hide
            // a launcher explicitly reopened by the user.
            self.launcher_hidden = true;
            ctx.send_viewport_cmd_to(
                egui::ViewportId::ROOT,
                egui::ViewportCommand::Visible(false),
            );
            ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Focus);
        }
        if manager_requested || ((self.launcher_hidden || had_windows) && self.windows.is_empty()) {
            self.show_launcher(ctx);
        }
        let blocked = self.root.close_blocked() || blocked_window.is_some();
        crate::native_menu::set_blocked(blocked);
        if crate::updates::restart_pending() && blocked {
            egui::Window::new("Update ready").collapsible(false).resizable(false).show(ctx, |ui| {
                ui.label("Finish running work, commit or roll back SQL transactions, and save or discard database edits in all windows.");
                ui.label("Rustable will save your SQL drafts and restart to install the update when ready.");
            });
        }
        if let Some(error) = self.update_save_error.clone() {
            egui::Window::new("Update restart postponed").collapsible(false).resizable(false).show(ctx, |ui| {
                ui.label("Could not save your SQL drafts and settings. Rustable will stay open.");
                ui.label(error);
                if ui.button("Retry saving").clicked() { self.update_save_error = None; }
            });
        }
        if crate::updates::restart_pending() && !blocked && self.update_save_error.is_none() {
            // Persist all windows' drafts/preferences before releasing Sparkle's restart.
            if let Some(storage) = frame.storage_mut() {
                self.save(storage);
                match storage.flush_and_wait() {
                    Ok(()) => crate::updates::resume_restart(),
                    Err(error) => self.update_save_error = Some(error.to_string()),
                }
            } else {
                self.update_save_error = Some("Persistent storage is unavailable.".into());
            }
        }
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if !self.migrate_launcher_size {
            storage.set_string(LAUNCHER_SIZE_MIGRATION, "done".into());
        }
        // Child documents join the recovery pool, never overwrite root profiles or
        // execute automatically after restart. Password-bearing requests are excluded.
        for window in &mut self.windows {
            if let Some(app) = &mut window.app {
                self.root.merge_window_preferences(app);
            }
        }
        let documents = self
            .windows
            .iter()
            .filter_map(|window| window.app.as_ref())
            .flat_map(|app| app.recovery_documents())
            .collect();
        self.root.save_with_window_documents(storage, documents);
    }
}

fn consume_quit(ctx: &egui::Context) -> bool {
    ctx.input_mut(|input| {
        let mut requested = false;
        input.events.retain(|event| {
            let matched = matches!(event, egui::Event::Key { key: egui::Key::Q, pressed: true, modifiers, .. }
                if modifiers.matches_exact(egui::Modifiers::COMMAND));
            requested |= matched;
            !matched
        });
        requested
    })
}

fn launcher_size(monitor: Option<egui::Vec2>) -> egui::Vec2 {
    monitor
        .filter(|size| size.is_finite() && size.x > 160.0 && size.y > 160.0)
        .map_or(LAUNCHER_SIZE, |size| {
            LAUNCHER_SIZE.min(size - egui::vec2(48.0, 100.0))
        })
}

#[cfg(test)]
mod launcher_tests {
    use super::*;

    #[test]
    fn launcher_dismissal_waits_for_success_and_does_not_repeat() {
        let mut window = SessionWindow {
            id: egui::ViewportId::from_hash_of("test-session"),
            title: "Test".into(),
            request: None,
            app: None,
            closed: false,
            connected: false,
        };
        // Pending and failed attempts must leave Connections available.
        assert!(!window.connection_completed(false));
        assert!(!window.connection_completed(false));
        assert!(window.connection_completed(true));
        // Reopening Connections while this session remains open must stick.
        assert!(!window.connection_completed(true));
        assert!(!window.connection_completed(false));
        assert!(!window.connection_completed(true));
    }

    #[test]
    fn quit_shortcut_does_not_consume_other_modifier_combinations() {
        let ctx = egui::Context::default();
        for (modifiers, expected) in [
            (egui::Modifiers::COMMAND, true),
            (egui::Modifiers::COMMAND | egui::Modifiers::ALT, false),
            (egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, false),
        ] {
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::Q,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    assert_eq!(consume_quit(ctx), expected);
                    assert_eq!(
                        ctx.input(|input| input.events.len()),
                        usize::from(!expected)
                    );
                },
            );
        }
    }
    #[test]
    fn launcher_geometry_fits_small_displays_and_does_not_fill_large_ones() {
        assert_eq!(launcher_size(None), LAUNCHER_SIZE);
        assert_eq!(
            launcher_size(Some(egui::vec2(1440.0, 900.0))),
            egui::vec2(960.0, 800.0)
        );
        assert_eq!(
            launcher_size(Some(egui::vec2(3840.0, 2160.0))),
            LAUNCHER_SIZE
        );
        assert_eq!(launcher_size(Some(egui::Vec2::ZERO)), LAUNCHER_SIZE);
    }
}
