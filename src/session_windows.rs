//! Native session windows. Only the root owns saved connection metadata/storage.
use crate::{app::Rustable, connection::PostgresConfig};
use eframe::egui;
use std::path::PathBuf;

const LAUNCHER_SIZE_MIGRATION: &str = "native-launcher-size-v1";
pub const LAUNCHER_SIZE: egui::Vec2 = egui::vec2(960.0, 860.0);

pub enum WindowConnection {
    #[cfg(any(feature = "qa", test))]
    Demo,
    Sqlite(PathBuf),
    Postgres(Box<PostgresConfig>),
    Mysql(Box<crate::connection::MysqlConfig>),
    Redis { settings: Box<crate::redis_settings::Settings>, password: String, sentinel_password: String },
}
pub struct WindowRequest {
    pub recover_sql: bool,
    pub identity: Option<crate::app::ProfileIdentity>,
    pub title: String,
    pub connection: WindowConnection,
}
impl WindowRequest {
    pub(crate) fn draft_scope(&self) -> Option<String> {
        let id = self.identity.as_ref()?.id.0;
        let database = match &self.connection {
            WindowConnection::Postgres(config) => config.database.clone(),
            WindowConnection::Mysql(config) => config.database.clone(),
            WindowConnection::Sqlite(path) => path.to_string_lossy().into_owned(),
            _ => String::new(),
        };
        Some(format!("{id}:{database}"))
    }
}
struct SessionWindow {
    id: egui::ViewportId,
    title: String,
    size_key: Option<String>,
    initial_size: [f32; 2],
    request: Option<WindowRequest>,
    app: Option<Box<Rustable>>,
    parked: Vec<Box<Rustable>>,
    active_slot: usize,
    closed: bool,
    connected: bool,
}

impl SessionWindow {
    fn viewport_builder(&self) -> egui::ViewportBuilder {
        // Keep the creation size stable. Feeding observed sizes back into this
        // builder emits InnerSize commands and can fight native resize/DPI events.
        egui::ViewportBuilder::default().with_title("Rustable")
            .with_inner_size(self.initial_size).with_min_inner_size([960.0, 640.0])
    }

    fn remember_size(&self, size: egui::Vec2, saved: &mut std::collections::HashMap<String, [f32; 2]>) {
        if size.is_finite() && size.x >= 960.0 && size.y >= 640.0
            && let Some(key) = &self.size_key {
            saved.insert(key.clone(), [size.x, size.y]);
        }
    }

    fn switch_workspace(&mut self, index: usize) {
        let target = self.parked.remove(index);
        let next_slot = index + usize::from(index >= self.active_slot);
        if let Some(previous) = self.app.replace(target) {
            let previous_slot = self.active_slot - usize::from(next_slot < self.active_slot);
            self.parked.insert(previous_slot, previous);
        }
        self.active_slot = next_slot;
    }

    fn close_workspace(&mut self, slot: usize) -> Result<Box<Rustable>, &'static str> {
        let active = slot == self.active_slot;
        let index = slot.saturating_sub(usize::from(slot > self.active_slot));
        let target = if active { self.app.as_deref() } else { self.parked.get(index).map(Box::as_ref) }
            .ok_or("Database workspace is no longer open.")?;
        if target.close_blocked() {
            if !active { self.switch_workspace(index); }
            return Err("Finish the transaction, save or discard pending edits, and wait for running work before closing this database. SQL drafts are retained.");
        }
        if active {
            let closed = self.app.take().unwrap();
            if self.parked.is_empty() {
                self.closed = true;
            } else {
                self.active_slot = self.active_slot.min(self.parked.len() - 1);
                self.app = Some(self.parked.remove(self.active_slot));
            }
            Ok(closed)
        } else {
            let closed = self.parked.remove(index);
            if slot < self.active_slot { self.active_slot -= 1; }
            Ok(closed)
        }
    }

    fn connection_completed(&mut self, connected: bool) -> bool {
        let completed = connected && !self.connected;
        self.connected |= connected;
        completed
    }
}

pub struct SessionWindows {
    #[cfg(not(target_arch = "wasm32"))]
    crash_prompt: Option<crate::crash_log::CrashPrompt>,
    root: Rustable,
    windows: Vec<SessionWindow>,
    next_id: u64,
    launcher_hidden: bool,
    quitting: bool,
    native_menu_installed: bool,
    migrate_launcher_size: bool,
    update_save_error: Option<String>,
    window_sizes: std::collections::HashMap<String, [f32; 2]>,
}
impl SessionWindows {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::scroll_response::install(&cc.egui_ctx);
        crate::updates::initialize(&cc.egui_ctx);
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            crash_prompt: crate::crash_log::CrashPrompt::load(),
            window_sizes: cc.storage.and_then(|storage| eframe::get_value(storage, "connection-window-sizes")).unwrap_or_default(),
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
        let requests = self.root.take_window_requests();
        let mut opened = None;
        for request in requests {
            let id = egui::ViewportId::from_hash_of(("database-session", self.next_id));
            self.next_id += 1;
            opened = Some(id);
            self.windows.push(SessionWindow {
                id,
                initial_size: request.draft_scope().and_then(|key| self.window_sizes.get(&key).copied()).unwrap_or([1280.0, 820.0]),
                size_key: request.draft_scope(),
                title: request.title.clone(),
                request: Some(request),
                app: None,
                parked: Vec::new(),
                active_slot: 0,
                closed: false,
                connected: false,
            });
        }
        opened
    }
    fn hand_off_launcher(&mut self, ctx: &egui::Context) {
        // Immediate child viewports redirect redraws through the root viewport.
        // On Windows a hidden root cannot receive the WM_PAINT needed to drive
        // those frames, leaving the child frozen before worker replies are polled.
        // Keep the launcher visible behind the focused session on Windows until
        // session windows have independent (deferred) render callbacks.
        self.launcher_hidden = !cfg!(target_os = "windows");
        ctx.send_viewport_cmd_to(
            egui::ViewportId::ROOT,
            egui::ViewportCommand::Visible(!self.launcher_hidden),
        );
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
        if !self.launcher_hidden { crate::ssh_prompt::show(ctx); }
        #[cfg(not(target_arch = "wasm32"))]
        if self.crash_prompt.as_mut().is_some_and(|prompt| prompt.show(ctx)) {
            self.crash_prompt = None;
        }
        let mut shared_changed = self.root.publish_shared_preferences();
        let opened_window = self.collect_requests();
        let had_windows = !self.windows.is_empty();
        let mut closed_database = false;
        let root_close_requested = ctx.input(|input| input.viewport().close_requested());
        let mut manager_requested = false;
        for window in &mut self.windows {
            ctx.show_viewport_immediate(window.id, window.viewport_builder(), |child_ctx, class| {
                if class == egui::ViewportClass::Embedded {
                    // Native desktop backends support immediate viewports. Never draw
                    // a second full workspace over the root on an unsupported backend.
                    self.root.window_error("This renderer does not support native session windows.");
                    window.closed = true;
                    return;
                }
                if let Some(rect) = child_ctx.input(|input| input.viewport().inner_rect) {
                    window.remember_size(rect.size(), &mut self.window_sizes);
                }
                for app in &mut window.parked {
                    app.poll_with_notifications(child_ctx);
                    self.root.sync_window_preferences(app);
                    self.root.sync_window_profile(app);
                    shared_changed |= self.root.merge_window_preferences(app);
                }
                let requests = window.app.as_mut().map(|app| app.take_window_requests()).unwrap_or_default();
                for request in requests {
                    let database = match &request.connection {
                        WindowConnection::Mysql(config) => Some(config.database.as_str()),
                        WindowConnection::Postgres(config) => Some(config.database.as_str()),
                        _ => None,
                    };
                    if let Some(index) = window.parked.iter().position(|app| app.workspace_database() == database) {
                        window.switch_workspace(index);
                    } else {
                        let target = Box::new(self.root.new_window(child_ctx, request));
                        if let Some(previous) = window.app.replace(target) { window.parked.insert(window.active_slot, previous); }
                        window.active_slot = window.parked.len();
                    }
                }
                {
                    let app = window.app.get_or_insert_with(|| Box::new(self.root.new_window(child_ctx, window.request.take().expect("new session request"))));
                    app.manager_shortcut(child_ctx);
                    self.root.sync_window_preferences(app);
                    self.root.sync_window_profile(app);
                    app.begin_workspace_frame(child_ctx);
                }
                let mut switch = None;
                let mut close_database = None;
                if !window.parked.is_empty() {
                    egui::SidePanel::left("database-workspaces").exact_width(92.0).resizable(false)
                        .frame(egui::Frame::new().fill(child_ctx.style().visuals.extreme_bg_color))
                        .show(child_ctx, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for slot in 0..=window.parked.len() {
                            let active = slot == window.active_slot;
                            let parked_index = slot.saturating_sub(usize::from(slot > window.active_slot));
                            let app = if active { window.app.as_deref() } else { Some(window.parked[parked_index].as_ref()) };
                            if let Some(app) = app {
                                let name = app.workspace_database().unwrap_or("Connecting…");
                                let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 90.0), egui::Sense::click());
                                if active { ui.painter().rect_filled(rect, 0.0, ui.visuals().panel_fill); }
                                else { ui.painter().line_segment([rect.right_top(), rect.right_bottom()], ui.visuals().widgets.noninteractive.bg_stroke); }
                                crate::app::navigator::paint_database_icon(ui.painter(), rect.center_top() + egui::vec2(0.0, 31.0), app.workspace_color(), 24.0);
                                let mut job = egui::text::LayoutJob::simple_singleline(name.into(), egui::FontId::proportional(12.0), ui.visuals().text_color());
                                job.wrap.max_width = rect.width() - 12.0; job.wrap.max_rows = 1;
                                let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
                                ui.painter().galley(egui::pos2(rect.center().x - galley.size().x * 0.5, rect.top() + 57.0), galley, ui.visuals().text_color());
                                response.context_menu(|ui| {
                                    if ui.button("Close database").clicked() {
                                        close_database = Some(slot);
                                        ui.close();
                                    }
                                });
                                if response.on_hover_text(name).clicked() && !active { switch = Some(parked_index); }
                            }
                        }
                    });
                }
                if let Some(index) = switch {
                    window.switch_workspace(index);
                    child_ctx.memory_mut(|memory| { if let Some(id) = memory.focused() { memory.surrender_focus(id); } });
                }
                if let Some(slot) = close_database {
                    match window.close_workspace(slot) {
                        Ok(app) => {
                            self.root.archive_window(*app);
                            closed_database = true;
                            shared_changed = true;
                            child_ctx.memory_mut(|memory| { if let Some(id) = memory.focused() { memory.surrender_focus(id); } });
                        }
                        Err(message) => {
                            if let Some(app) = &mut window.app { app.window_error(message); }
                        }
                    }
                    if window.closed { return; }
                }
                let parked_blocked = window.parked.iter().any(|app| app.close_blocked());
                let app = window.app.get_or_insert_with(|| Box::new(self.root.new_window(child_ctx, window.request.take().expect("new session request"))));
                quit_requested |= consume_quit(child_ctx);
                app.manager_shortcut(child_ctx);
                self.root.sync_window_preferences(app);
                if self.root.sync_window_profile(app) { child_ctx.request_repaint(); }
                app.render_workspace_frame(child_ctx);
                if self.launcher_hidden { crate::ssh_prompt::show(child_ctx); }
                if app.has_connection() && window.title != app.active_connection_name() {
                    window.title = app.active_connection_name().to_owned();
                    child_ctx.request_repaint();
                }
                shared_changed |= self.root.merge_window_preferences(app);
                if app.take_manager_request() {
                    self.root.select_manager_profile(app.active_profile_id());
                    manager_requested = true;
                }
                if child_ctx.input(|input| input.viewport().close_requested()) {
                    if app.close_blocked() || parked_blocked {
                        child_ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                        app.window_error("Finish SQL transactions with COMMIT or ROLLBACK, save or discard pending database changes, and wait for work in every database workspace before closing. SQL drafts are retained for the next time you open the SQL editor.");
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
            if let Some(app) = window.app.take() { self.root.archive_window(*app); }
            for app in window.parked.drain(..) { self.root.archive_window(*app); }
        }
        let closed_window = self.windows.iter().any(|window| window.closed);
        self.windows.retain(|window| !window.closed);
        if (closed_window || closed_database) && let Some(storage) = frame.storage_mut() {
            self.save(storage);
            storage.flush();
        }
        let blocked_window = self.windows.iter().position(|window| {
            window.request.is_some() || window.app.as_ref().is_some_and(|app| app.close_blocked()) || window.parked.iter().any(|app| app.close_blocked())
        });
        if quit_requested {
            if self.root.close_blocked() || blocked_window.is_some() {
                self.quitting = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if let Some(index) = blocked_window {
                    let window = &mut self.windows[index];
                    if let Some(app) = &mut window.app {
                        app.window_error("Finish SQL transactions with COMMIT or ROLLBACK, save or discard pending database edits, and wait for running work before quitting. SQL drafts will be retained for the next time you open the SQL editor.");
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
            self.hand_off_launcher(ctx);
            if !self.launcher_hidden && let Some(window) = self.windows.first() {
                ctx.send_viewport_cmd_to(window.id, egui::ViewportCommand::Focus);
            }
        }
        if let Some(id) = opened_window.filter(|_| !self.quitting && !manager_requested) {
            // Hand off immediately to the new window, which owns connection
            // progress and errors. Later connection completion must not re-hide
            // a launcher explicitly reopened by the user.
            self.hand_off_launcher(ctx);
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
    fn auto_save_interval(&self) -> std::time::Duration { std::time::Duration::from_secs(5) }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "connection-window-sizes", &self.window_sizes);
        if !self.migrate_launcher_size {
            storage.set_string(LAUNCHER_SIZE_MIGRATION, "done".into());
        }
        // Child documents join the recovery pool, never overwrite root profiles or
        // execute automatically after restart. Password-bearing requests are excluded.
        for window in &mut self.windows {
            for app in window.app.iter_mut().chain(window.parked.iter_mut()) { self.root.merge_window_preferences(app); }
        }
        let documents = self
            .windows
            .iter()
            .flat_map(|window| window.app.iter().chain(window.parked.iter()))
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

    fn test_window() -> SessionWindow {
        SessionWindow {
            id: egui::ViewportId::from_hash_of("test-session"),
            title: "Test".into(),
            size_key: None,
            initial_size: [1280.0, 820.0],
            request: None,
            app: None,
            parked: Vec::new(),
                active_slot: 0,
            closed: false,
            connected: false,
        }
    }

    #[test]
    fn closing_database_preserves_other_workspaces_and_selection() {
        for active_slot in 0..3 {
            for close_slot in 0..3 {
                let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
                let mut apps: Vec<_> = (0..3).map(|_| Box::new(Rustable::new(&cc))).collect();
                let identities: Vec<_> = apps.iter().map(|app| &**app as *const Rustable).collect();
                let mut window = test_window();
                window.app = Some(apps.remove(active_slot));
                window.parked = apps;
                window.active_slot = active_slot;
                let closed = window.close_workspace(close_slot).ok().unwrap();
                assert_eq!(&*closed as *const Rustable, identities[close_slot]);
                assert!(!window.closed);
                let mut remaining: Vec<_> = window.parked.iter().map(|app| &**app as *const Rustable).collect();
                remaining.insert(window.active_slot, &**window.app.as_ref().unwrap() as *const Rustable);
                let expected: Vec<_> = identities.iter().enumerate().filter_map(|(i, id)| (i != close_slot).then_some(*id)).collect();
                assert_eq!(remaining, expected);
                if close_slot != active_slot {
                    assert_eq!(&**window.app.as_ref().unwrap() as *const Rustable, identities[active_slot]);
                }
            }
        }
    }

    #[test]
    fn closing_busy_database_keeps_it_open_and_brings_it_forward() {
        let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        let mut window = test_window();
        window.app = Some(Box::new(Rustable::new(&cc)));
        // The demo's initial load remains pending until the UI polls its reply.
        window.parked.push(Box::new(Rustable::new_demo(&cc)));
        assert!(window.close_workspace(1).is_err());
        assert_eq!(window.active_slot, 1);
        assert_eq!(window.parked.len(), 1);
        assert!(!window.closed);
    }

    #[test]
    fn observed_monitor_resizes_are_saved_without_requesting_another_resize() {
        let mut window = test_window();
        window.size_key = Some("connection:database".into());
        let mut saved = std::collections::HashMap::new();
        let mut applied = window.viewport_builder();
        // Includes fractional sizes, a Retina/non-Retina transition, and normal
        // user resizing. None should turn an observation into a resize request.
        for size in [[1280.5, 820.5], [1920.0, 1080.0], [960.0, 640.0], [1440.0, 900.0]] {
            window.remember_size(size.into(), &mut saved);
            let (commands, recreate) = applied.patch(window.viewport_builder());
            assert!(commands.is_empty(), "unexpected resize feedback: {commands:?}");
            assert!(!recreate);
            assert_eq!(saved["connection:database"], size);
        }
        for size in [[0.0, 0.0], [f32::NAN, 820.0], [f32::INFINITY, 820.0]] {
            window.remember_size(size.into(), &mut saved);
        }
        assert_eq!(saved["connection:database"], [1440.0, 900.0]);
        assert_eq!(window.initial_size, [1280.0, 820.0]);
        let mut reopened = test_window();
        reopened.initial_size = saved["connection:database"];
        assert_eq!(reopened.viewport_builder().inner_size, Some(egui::vec2(1440.0, 900.0)));
    }

    #[test]
    fn launcher_dismissal_waits_for_success_and_does_not_repeat() {
        let mut window = test_window();
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

#[cfg(feature = "qa")]
#[path = "native_performance.rs"]
pub(crate) mod native_performance;
