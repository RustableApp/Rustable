//! Keyboard-first navigation without replacing the active database session.
use super::*;
#[derive(Default)]
pub(super) struct Navigator {
    pub open: bool,
    pub loaded: bool,
    pub databases: Vec<String>,
    config: Option<crate::connection::PostgresConfig>,
    query: String,
    selected: usize,
    focus: bool,
    databases_only: bool,
    requested: bool,
}
impl Navigator {
    pub fn new(command: Option<&Command>) -> Self {
        Self { config: match command { Some(Command::OpenPostgres(config)) => Some(config.clone()), _ => None }, ..Default::default() }
    }
}
#[derive(Clone)]
enum Destination { Database(String), Schema(String), Object(String), Tab(usize), Saved(String) }
struct Entry { label: String, detail: String, destination: Destination }
impl Rustable {
    pub(super) fn open_navigator(&mut self, databases_only: bool) {
        self.navigator.open = true;
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
            entries.push(Entry { label: name.clone(), detail: "Database · opens in a new window".into(), destination: Destination::Database(name.clone()) });
        }
        if self.navigator.config.is_none() {
            entries.push(Entry { label: self.label.clone(), detail: "Current SQLite database".into(), destination: Destination::Database(self.label.clone()) });
        }
        if !self.navigator.databases_only {
            for schema in &self.schemas.names {
                entries.push(Entry { label: schema.clone(), detail: "Schema".into(), destination: Destination::Schema(schema.clone()) });
            }
            for object in &self.objects {
                if matches!(object.kind.as_str(), "table" | "view" | "materialized view") {
                    entries.push(Entry { label: object.display_name().to_owned(), detail: format!("{} · {}", object.kind, object.schema.as_deref().unwrap_or("main")), destination: Destination::Object(object.name.clone()) });
                }
            }
            for (index, tab) in self.tabs.iter().enumerate().filter(|(_, tab)| tab.open) {
                entries.push(Entry { label: format!("SQL Query {}", index + 1), detail: tab.sql.chars().take(100).collect(), destination: Destination::Tab(index) });
            }
            for sql in &self.prefs.favorites {
                entries.push(Entry { label: sql.lines().next().unwrap_or(sql).chars().take(100).collect(), detail: "Saved query".into(), destination: Destination::Saved(sql.clone()) });
            }
        }
        let mut open = true;
        let mut chosen = None;
        egui::Window::new(if self.navigator.databases_only { "Databases" } else { "Find anything" })
            .id(egui::Id::new("database-navigator"))
            .open(&mut open).collapsible(false).resizable(true).default_width(520.0)
            .show(ctx, |ui| {
                let search = ui.add(TextEdit::singleline(&mut self.navigator.query).hint_text("Search…").desired_width(f32::INFINITY));
                if std::mem::take(&mut self.navigator.focus) { search.request_focus(); }
                if search.changed() { self.navigator.selected = 0; }
                let query = self.navigator.query.to_lowercase();
                let entries: Vec<_> = entries.iter().filter(|entry| matches_query(&format!("{} {}", entry.label, entry.detail), &query)).collect();
                self.navigator.selected = self.navigator.selected.min(entries.len().saturating_sub(1));
                let down = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
                let up = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
                if down { self.navigator.selected = (self.navigator.selected + 1).min(entries.len().saturating_sub(1)); }
                if up { self.navigator.selected = self.navigator.selected.saturating_sub(1); }
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) && let Some(entry) = entries.get(self.navigator.selected) {
                    chosen = Some(entry.destination.clone());
                }
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for (index, entry) in entries.iter().enumerate() {
                        let response = ui.add_sized([ui.available_width(), 42.0], egui::Button::new(format!("{}\n{}", entry.label, entry.detail)).selected(index == self.navigator.selected));
                        if (up || down) && index == self.navigator.selected { response.scroll_to_me(Some(egui::Align::Center)); }
                        if response.clicked() { chosen = Some(entry.destination.clone()); }
                    }
                    if entries.is_empty() { ui.weak("No matches"); }
                });
                ui.horizontal(|ui| {
                    ui.weak("↑ ↓ Navigate    Enter Open    Esc Close");
                    if self.navigator.config.is_some() && ui.add_enabled(self.pending.is_none(), egui::Button::new("Refresh databases")).clicked() { self.navigator.requested = false; }
                });
                if self.navigator.config.is_some() && !self.navigator.loaded && self.pending.is_some() { ui.weak("Loading databases…"); }
                if let Some(error) = &self.error { ui.colored_label(ui.visuals().error_fg_color, error); }
            });
        self.navigator.open = open;
        if let Some(destination) = chosen {
            if self.pending.is_some() { return; }
            self.navigator.open = false;
            match destination {
                Destination::Database(name) => {
                    if self.connected_database.as_deref() == Some(&name) || self.navigator.config.is_none() { return; }
                    if let Some(mut config) = self.navigator.config.clone() {
                        config.database = name.clone();
                        self.window_requests.push(crate::session_windows::WindowRequest { title: format!("{} · {name}", self.active_connection_name()), recover_sql: false, identity: self.active_profile.clone(), connection: crate::session_windows::WindowConnection::Postgres(Box::new(config)) });
                    }
                }
                Destination::Schema(name) => self.dispatch(Command::SelectSchema(name), Pending::Export),
                Destination::Object(name) => self.open_table(name),
                Destination::Tab(index) => self.activate_query(index),
                Destination::Saved(sql) => { self.tabs.push(QueryTab::new(sql)); self.activate_query(self.tabs.len() - 1); }
            }
        }
    }
}
fn matches_query(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    query.to_lowercase().split_whitespace().all(|word| text.contains(word))
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
    fn search_matches_names_and_kinds_without_case_or_word_order_constraints() {
        assert!(matches_query("Address table public", "public addr"));
        assert!(matches_query("Address table public", "ADDRESS"));
        assert!(!matches_query("Address table public", "private addr"));
    }
}
