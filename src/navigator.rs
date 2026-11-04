//! Keyboard-first navigation without replacing the active database session.
use super::*;
#[derive(Clone)]
enum ServerConfig {
    Postgres(crate::connection::PostgresConfig),
    Mysql(crate::connection::MysqlConfig),
}
impl ServerConfig {
    fn database(self, name: String) -> crate::session_windows::WindowConnection {
        match self {
            Self::Postgres(mut config) => { config.database = name; crate::session_windows::WindowConnection::Postgres(Box::new(config)) }
            Self::Mysql(mut config) => { config.database = name; crate::session_windows::WindowConnection::Mysql(Box::new(config)) }
        }
    }
}
#[derive(Default)]
pub(super) struct Navigator {
    pub open: bool,
    pub loaded: bool,
    pub databases: Vec<String>,
    config: Option<ServerConfig>,
    query: String,
    selected: usize,
    focus: bool,
    databases_only: bool,
    requested: bool,
}
impl Navigator {
    pub fn new(command: Option<&Command>) -> Self {
        Self { config: match command { Some(Command::OpenPostgres(config)) => Some(ServerConfig::Postgres(config.clone())), Some(Command::OpenMysql(config)) => Some(ServerConfig::Mysql(config.clone())), _ => None }, ..Default::default() }
    }
}
#[derive(Clone)]
enum Destination { Database(String), Schema(String), Object(String), Tab(usize), Saved(PathBuf), Action(Action) }
#[derive(Clone)]
enum Action { Refresh, Connections, Databases, Cancel, OpenSql, ImportCsv, NewQuery, Activity, Inspector }
struct Entry { label: String, detail: String, destination: Destination }
impl Rustable {
    pub(super) fn open_database_workspace(&mut self, name: String) {
        let current = if matches!(self.engine.as_str(), "MySQL" | "MariaDB") { self.schemas.selected.as_deref() } else { self.connected_database.as_deref() };
        if current == Some(&name) || self.navigator.config.is_none() { return; }
        if current.is_none() && matches!(self.navigator.config, Some(ServerConfig::Mysql(_))) {
            self.dispatch(Command::SelectSchema(name), Pending::Export);
        } else if let Some(config) = self.navigator.config.clone() {
            self.window_requests.push(crate::session_windows::WindowRequest { title: format!("{} · {name}", self.active_connection_name()), recover_sql: false, identity: self.active_profile.clone(), connection: config.database(name) });
        }
    }
    pub(super) fn open_navigator(&mut self, databases_only: bool) {
        self.navigator.open = true;
        self.navigator.requested = false;
        self.navigator.databases_only = databases_only;
        self.navigator.query.clear();
        self.navigator.selected = 0;
        self.navigator.focus = true;
    }
    pub(super) fn navigator_window(&mut self, ctx: &egui::Context) {
        if !self.navigator.open { return; }
        if !self.navigator.requested && self.navigator.config.is_some() && self.pending.is_none() {
            self.navigator.requested = true;
            self.dispatch(Command::ListDatabases, Pending::Export);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.navigator.open = false;
            return;
        }
        let mut entries = Vec::new();
        for name in &self.navigator.databases {
            entries.push(Entry { label: name.clone(), detail: "Database · opens in this window".into(), destination: Destination::Database(name.clone()) });
        }
        if self.navigator.config.is_none() {
            entries.push(Entry { label: self.label.clone(), detail: "Current SQLite database".into(), destination: Destination::Database(self.label.clone()) });
        }
        if !self.navigator.databases_only {
            let actions = [
                ("Sync Database", "".to_owned(), Action::Refresh),
                ("Open Connection", crate::icons::shortcut(ctx, false, "N"), Action::Connections),
                ("Open Database", crate::icons::shortcut(ctx, false, "K"), Action::Databases),
                ("Cancel Running Queries", String::new(), Action::Cancel),
                ("Open SQL File…", crate::icons::shortcut(ctx, true, "O"), Action::OpenSql),
                ("Import CSV file…", String::new(), Action::ImportCsv),
                ("New SQL Query", crate::icons::shortcut(ctx, false, "T"), Action::NewQuery),
                ("Toggle SQL Activity", String::new(), Action::Activity),
                ("Toggle Inspector", String::new(), Action::Inspector),
            ];
            let mut commands: Vec<_> = actions.into_iter().map(|(label, detail, action)| Entry { label: label.into(), detail, destination: Destination::Action(action) }).collect();
            commands.append(&mut entries);
            entries = commands;
            for schema in &self.schemas.names {
                entries.push(Entry { label: schema.clone(), detail: "Schema".into(), destination: Destination::Schema(schema.clone()) });
            }
            for object in &self.objects {
                if matches!(object.kind.as_str(), "table" | "view" | "materialized view") {
                    entries.push(Entry { label: object.display_name().to_owned(), detail: format!("{} · {}", object.kind, object.schema.as_deref().unwrap_or("main")), destination: Destination::Object(object.name.clone()) });
                }
            }
            for (index, tab) in self.tabs.iter().enumerate().filter(|(_, tab)| tab.open) {
                entries.push(Entry { label: tab.source_path.as_ref().and_then(|path| path.file_name()).map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| format!("SQL Query {}", index + 1)), detail: tab.sql.chars().take(100).collect(), destination: Destination::Tab(index) });
            }
            self.query_library.poll(ctx, self.query_directory());
            for file in &self.query_library.files {
                entries.push(Entry { label: file.name.clone(), detail: format!("Saved query · {}", file.group), destination: Destination::Saved(file.path.clone()) });
            }
        }
        let mut open = true;
        let mut chosen = None;
        let databases_only = self.navigator.databases_only;
        let size = if databases_only { egui::vec2(400.0, 430.0) } else { egui::vec2(560.0, 440.0) };
        let modal = egui::Modal::new(egui::Id::new("database-navigator"))
            .frame(egui::Frame::window(&ctx.style()).corner_radius(16.0).inner_margin(16.0))
            .show(ctx, |ui| {
                ui.set_min_size(size);
                ui.set_max_width(size.x);
                if databases_only {
                    ui.vertical_centered(|ui| { ui.label(RichText::new("Open database").strong().size(18.0)); });
                    ui.add_space(12.0);
                }
                let search = egui::Frame::new().corner_radius(20.0)
                    .fill(ui.visuals().extreme_bg_color)
                    .stroke(Stroke::new(1.5_f32, ui.visuals().selection.stroke.color))
                    .inner_margin(egui::Margin::symmetric(12, 7))
                    .show(ui, |ui| {
                        ui.add(TextEdit::singleline(&mut self.navigator.query)
                            .frame(false).font(egui::FontId::proportional(if databases_only { 16.0 } else { 22.0 }))
                            .hint_text(if databases_only { "Search for database…" } else { "Search for tables, databases, files…" })
                            .desired_width(f32::INFINITY))
                    }).inner;
                if std::mem::take(&mut self.navigator.focus) { search.request_focus(); }
                if search.changed() { self.navigator.selected = 0; }
                ui.add_space(10.0);
                if !databases_only { ui.separator(); ui.add_space(5.0); }
                let entries = ranked_entries(&entries, &self.navigator.query);
                self.navigator.selected = self.navigator.selected.min(entries.len().saturating_sub(1));
                let down = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
                let up = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
                if down { self.navigator.selected = (self.navigator.selected + 1).min(entries.len().saturating_sub(1)); }
                if up { self.navigator.selected = self.navigator.selected.saturating_sub(1); }
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) && let Some(entry) = entries.get(self.navigator.selected) {
                    chosen = Some(entry.destination.clone());
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(if databases_only { 305.0 } else { 360.0 }).show(ui, |ui| {
                    for (index, entry) in entries.iter().enumerate() {
                        let response = navigation_row(ui, entry, index == self.navigator.selected, databases_only);
                        if (up || down) && index == self.navigator.selected { response.scroll_to_me(Some(egui::Align::Center)); }
                        if response.clicked() { self.navigator.selected = index; }
                        if response.double_clicked() || (!databases_only && response.clicked()) { chosen = Some(entry.destination.clone()); }
                    }
                    if entries.is_empty() {
                        ui.weak(if self.pending.is_some() && !self.navigator.loaded { "Loading databases…" } else { "No matches" });
                    }
                });
                if let Some(error) = &self.error { ui.colored_label(ui.visuals().error_fg_color, error); }
                if databases_only {
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.add_sized([90.0, 28.0], egui::Button::new("Cancel").corner_radius(7.0)).clicked() { open = false; }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let button = egui::Button::new(RichText::new("Open").color(Color32::WHITE)).fill(Color32::from_rgb(0, 122, 255)).corner_radius(7.0);
                            if ui.add_enabled_ui(!entries.is_empty() && self.pending.is_none(), |ui| ui.add_sized([90.0, 28.0], button)).inner.clicked() {
                                chosen = entries.get(self.navigator.selected).map(|entry| entry.destination.clone());
                            }
                        });
                    });
                }
            });
        self.navigator.open = open && !modal.should_close();
        if let Some(destination) = chosen {
            if self.pending.is_some() && !matches!(destination, Destination::Action(Action::Cancel | Action::Connections | Action::Activity | Action::Inspector)) { return; }
            self.navigator.open = false;
            match destination {
                Destination::Action(action) => match action {
                    Action::Refresh => { self.navigator.requested = false; self.dispatch(Command::RefreshSchema, Pending::Export); }
                    Action::Connections => { self.manager_requested = true; }
                    Action::Databases => self.open_navigator(true),
                    Action::Cancel => self.request_query_cancel(ctx),
                    Action::OpenSql => self.script_dialog(ctx, false),
                    Action::ImportCsv => self.preview_csv_dialog(ctx),
                    Action::NewQuery => { self.tabs.push(QueryTab::new(String::new())); self.activate_query(self.tabs.len() - 1); }
                    Action::Activity => { self.prefs.activity_visible = !self.prefs.activity_visible; self.shared_preferences_dirty = true; },
                    Action::Inspector => self.inspector = !self.inspector,
                },
                Destination::Database(name) => {
                    self.open_database_workspace(name);
                }
                Destination::Schema(name) => self.dispatch(Command::SelectSchema(name), Pending::Export),
                Destination::Object(name) => self.open_table(name),
                Destination::Tab(index) => self.activate_query(index),
                Destination::Saved(path) => { self.dispatch(Command::OpenScript(path), Pending::Export); }
            }
        }
    }
}
fn navigation_row(ui: &mut egui::Ui, entry: &Entry, selected: bool, database_only: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::click());
    if selected { ui.painter().rect_filled(rect, 8.0, ui.visuals().widgets.active.weak_bg_fill); }
    let c = egui::pos2(rect.left() + 18.0, rect.center().y);
    let ink = ui.visuals().text_color();
    if matches!(entry.destination, Destination::Database(_)) {
        paint_database_icon(ui.painter(), c, Color32::from_rgb(100, 165, 205), 22.0);
    } else if matches!(entry.destination, Destination::Object(_)) {
        paint_object_icon(ui.painter(), c, entry.detail.split(" · ").next().unwrap_or("table"), 19.0);
    } else {
        paint_navigation_icon(ui.painter(), c, &entry.destination, ui.visuals().weak_text_color());
    }
    let shortcut = !database_only && matches!(entry.destination, Destination::Action(_));
    let schema = if matches!(entry.destination, Destination::Object(_)) { entry.detail.split_once(" · ").map(|(_, schema)| schema) } else { None };
    let schema_width = schema.map(|name| ui.painter().layout_no_wrap(name.to_owned(), egui::FontId::proportional(14.0), ui.visuals().weak_text_color()).size().x.min(rect.width() * 0.45)).unwrap_or(0.0);
    let right = if shortcut && !entry.detail.is_empty() { 110.0 } else if schema.is_some() { schema_width + 22.0 } else { 10.0 };
    let clip = egui::Rect::from_min_max(rect.min + egui::vec2(36.0, 0.0), rect.max - egui::vec2(right, 0.0));
    ui.painter().with_clip_rect(clip).text(clip.left_center(), egui::Align2::LEFT_CENTER, &entry.label, egui::FontId::proportional(16.0), ink);
    if shortcut { ui.painter().text(rect.right_center() - egui::vec2(10.0, 0.0), egui::Align2::RIGHT_CENTER, &entry.detail, egui::FontId::proportional(12.0), Color32::from_rgb(236, 148, 63)); }
    if let Some(schema) = schema {
        let clip = egui::Rect::from_min_max(egui::pos2(rect.right() - schema_width - 10.0, rect.top()), rect.max - egui::vec2(10.0, 0.0));
        ui.painter().with_clip_rect(clip).text(clip.right_center(), egui::Align2::RIGHT_CENTER, schema, egui::FontId::proportional(14.0), ui.visuals().weak_text_color());
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), selected, &entry.label));
    response
}
pub(crate) fn paint_database_icon(painter: &egui::Painter, center: egui::Pos2, color: Color32, size: f32) {
    crate::icons::paint(painter, center, "database", size, color);
}
pub(super) fn paint_object_icon(painter: &egui::Painter, center: egui::Pos2, kind: &str, size: f32) {
    let (icon, color) = if kind.contains("view") { ("eye", Color32::from_rgb(162, 129, 186)) } else { ("sheet", Color32::from_rgb(87, 150, 179)) };
    crate::icons::paint(painter, center, icon, size, color);
}
fn paint_navigation_icon(painter: &egui::Painter, center: egui::Pos2, destination: &Destination, ink: Color32) {
    let name = match destination {
        Destination::Action(Action::Refresh) => "refresh-cw",
        Destination::Action(Action::Connections) => "plug",
        Destination::Action(Action::Databases) | Destination::Database(_) => "database",
        Destination::Action(Action::Cancel) => "circle-stop",
        Destination::Action(Action::Activity) => "panel-bottom",
        Destination::Action(Action::Inspector) => "panel-right",
        Destination::Object(_) => "sheet",
        Destination::Schema(_) => "folder",
        Destination::Action(Action::NewQuery) => "file-plus",
        Destination::Action(Action::ImportCsv) => "upload",
        _ => "file-code",
    };
    crate::icons::paint(painter, center, name, 20.0, ink);
}
fn ranked_entries<'a>(entries: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    crate::search::Query::new(query).ranked(entries, |entry| (&entry.label, &entry.detail))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn key(ctx: &egui::Context, app: &mut Rustable, key: egui::Key, modifiers: egui::Modifiers) {
        let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0))), events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }], ..Default::default() }, |ctx| {
            app.workspace_shortcuts(ctx);
            app.navigator_window(ctx);
        });
    }
    #[test]
    fn picker_owns_enter_and_escape_and_opens_query_without_execution() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        key(&ctx, &mut app, egui::Key::P, egui::Modifiers::COMMAND);
        assert!(app.navigator.open);
        assert!(app.dialog_owns_workspace_shortcuts());
        app.navigator.query = "SQL Query".into();
        key(&ctx, &mut app, egui::Key::Enter, egui::Modifiers::NONE);
        assert!(!app.navigator.open);
        assert!(app.pending.is_none(), "Picking a query must not execute it");
        assert!(app.view == View::Query);
        key(&ctx, &mut app, egui::Key::K, egui::Modifiers::COMMAND);
        assert!(app.navigator.open && app.navigator.databases_only);
        key(&ctx, &mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(!app.navigator.open);
    }
    #[test]
    fn outside_click_dismisses_palette() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        key(&ctx, &mut app, egui::Key::P, egui::Modifiers::COMMAND);
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0))),
                ..Default::default()
            }, |ctx| app.navigator_window(ctx));
        }
        for pressed in [true, false] {
            let pos = egui::pos2(10.0, 10.0);
            let _ = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0))),
                events: vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }],
                ..Default::default()
            }, |ctx| app.navigator_window(ctx));
        }
        assert!(!app.navigator.open);
    }

    fn entry(label: &str, detail: &str) -> Entry {
        Entry { label: label.into(), detail: detail.into(), destination: Destination::Object(label.into()) }
    }
    #[test]
    fn ranking_prefers_exact_prefix_boundary_substring_then_abbreviation() {
        let entries = vec![entry("campaign_store_property_values", "table"), entry("bookstore", "table"),
            entry("stores", "table"), entry("store_locations", "table"), entry("store", "table"),
            entry("s_t_o_r_e", "table"), entry("other", "store")];
        let names: Vec<_> = ranked_entries(&entries, "STORE").into_iter().map(|entry| entry.label.as_str()).collect();
        assert_eq!(names, ["store", "stores", "store_locations", "campaign_store_property_values", "bookstore", "s_t_o_r_e", "other"]);
    }
    #[test]
    fn abbreviations_reward_initials_and_contiguous_runs() {
        let entries = vec![entry("catsoupvalue", ""), entry("campaign_store_property_values", ""), entry("CustomerStorePropertyValues", "")];
        let matches = ranked_entries(&entries, "cspv");
        assert_eq!(matches.len(), 3);
        assert_eq!(matches.last().unwrap().label, "catsoupvalue");
        let entries = vec![entry("axxbxxc", ""), entry("abxc", ""), entry("a_b_c", "")];
        assert_eq!(ranked_entries(&entries, "abc").last().unwrap().label, "axxbxxc");
        assert!(ranked_entries(&entries, "cba").is_empty());
    }
    #[test]
    fn search_matches_metadata_unicode_and_words_in_any_order() {
        let entries = vec![entry("Address", "table public"), entry("Événements", "table")];
        assert_eq!(ranked_entries(&entries, "public addr")[0].label, "Address");
        assert_eq!(ranked_entries(&entries, "ADDRESS")[0].label, "Address");
        assert!(ranked_entries(&entries, "private addr").is_empty());
        assert_eq!(ranked_entries(&entries, "ÉVÉ")[0].label, "Événements");
    }
    #[test]
    fn empty_queries_and_ties_keep_original_order() {
        let entries = vec![entry("stores", "z_schema"), entry("stores", "a_schema"), entry("abc", "")];
        assert_eq!(ranked_entries(&entries, "  ")[0].detail, "z_schema");
        let matches = ranked_entries(&entries, "store");
        assert_eq!(matches[0].detail, "z_schema");
        assert_eq!(matches[1].detail, "a_schema");
    }

}

#[cfg(test)]
mod mysql_navigation_tests {
    use super::*;
    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_first_database_selection_keeps_session_and_sql_draft() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316,43317].contains(&port));
        let ctx = egui::Context::default();
        let config = crate::connection::MysqlConfig { host:"127.0.0.1".into(), port, user:"rustable".into(), password:"rustable-test-only".into(), database:String::new(), tls:false, ..Default::default() };
        let mut app = Rustable::with_initial_connection(&ctx, Preferences::default(), Command::OpenMysql(config), None);
        let settle = |app: &mut Rustable| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while app.pending.is_some() { assert!(std::time::Instant::now() < deadline); app.poll(); std::thread::sleep(std::time::Duration::from_millis(5)); }
            assert!(app.error.is_none(), "{:?}", app.error);
        };
        settle(&mut app);
        app.tabs[0].sql = "SELECT 'keep this draft'".into();
        app.open_database_workspace("rustable_test".into());
        assert!(app.window_requests.is_empty());
        settle(&mut app);
        assert_eq!(app.workspace_database(), Some("rustable_test"));
        assert_eq!(app.tabs[0].sql, "SELECT 'keep this draft'");
        app.open_database_workspace("information_schema".into());
        assert_eq!(app.window_requests.len(), 1);
        assert_eq!(app.workspace_database(), Some("rustable_test"));
        assert_eq!(app.tabs[0].sql, "SELECT 'keep this draft'");
    }

    #[test]
    fn database_navigation_preserves_mysql_transport_and_changes_only_database() {
        let config = crate::connection::MysqlConfig { password: "session-only".into(), root_certificate: Some("/ca.pem".into()), ..Default::default() };
        let navigator = Navigator::new(Some(&Command::OpenMysql(config.clone())));
        let crate::session_windows::WindowConnection::Mysql(next) = navigator.config.unwrap().database("other".into()) else { panic!("wrong driver"); };
        assert_eq!(next.database, "other");
        assert_eq!(next.password, config.password);
        assert_eq!(next.root_certificate, config.root_certificate);
        assert_eq!(next.tls, config.tls);
    }
}
