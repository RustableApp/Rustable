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
enum Destination { Database(String), Schema(String), Object(String), Tab(usize), Saved(String), Action(Action) }
#[derive(Clone)]
enum Action { Refresh, Connections, Databases, Cancel, OpenSql, NewQuery, Activity, Inspector }
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
            let command = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" };
            let actions = [
                ("Sync Database", "".to_owned(), Action::Refresh),
                ("Open Connection", format!("{command} N"), Action::Connections),
                ("Open Database", format!("{command} K"), Action::Databases),
                ("Cancel Running Queries", String::new(), Action::Cancel),
                ("Open SQL File…", format!("{command} ⇧ O"), Action::OpenSql),
                ("New SQL Query", format!("{command} T"), Action::NewQuery),
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
            for sql in &self.prefs.favorites {
                entries.push(Entry { label: sql.lines().next().unwrap_or(sql).chars().take(100).collect(), detail: "Saved query".into(), destination: Destination::Saved(sql.clone()) });
            }
        }
        let mut open = true;
        let mut chosen = None;
        let databases_only = self.navigator.databases_only;
        let size = if databases_only { egui::vec2(400.0, 430.0) } else { egui::vec2(560.0, 440.0) };
        egui::Window::new(if databases_only { "Open database" } else { "Find anything" })
            .id(egui::Id::new("database-navigator"))
            .title_bar(false).collapsible(false).resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .fixed_size(size)
            .frame(egui::Frame::window(&ctx.style()).corner_radius(16.0).inner_margin(16.0))
            .show(ctx, |ui| {
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
                let entries: Vec<_> = entries.iter().filter(|entry| matches_query(&format!("{} {}", entry.label, entry.detail), &self.navigator.query)).collect();
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
        self.navigator.open = open;
        if let Some(destination) = chosen {
            if self.pending.is_some() && !matches!(destination, Destination::Action(Action::Cancel | Action::Connections | Action::Activity | Action::Inspector)) { return; }
            self.navigator.open = false;
            match destination {
                Destination::Action(action) => match action {
                    Action::Refresh => { self.navigator.requested = false; self.dispatch(Command::RefreshSchema, Pending::Export); }
                    Action::Connections => { self.manager_requested = true; }
                    Action::Databases => self.open_navigator(true),
                    Action::Cancel => self.worker.cancel(),
                    Action::OpenSql => self.script_dialog(ctx, false),
                    Action::NewQuery => { self.tabs.push(QueryTab::new(String::new())); self.activate_query(self.tabs.len() - 1); }
                    Action::Activity => self.prefs.activity_visible = !self.prefs.activity_visible,
                    Action::Inspector => self.inspector = !self.inspector,
                },
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
fn navigation_row(ui: &mut egui::Ui, entry: &Entry, selected: bool, database_only: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::click());
    if selected { ui.painter().rect_filled(rect, 8.0, ui.visuals().widgets.active.weak_bg_fill); }
    let c = egui::pos2(rect.left() + 18.0, rect.center().y);
    let ink = ui.visuals().text_color();
    if matches!(entry.destination, Destination::Database(_)) {
        let color = Color32::from_rgb(76, 141, 186);
        ui.painter().rect_filled(egui::Rect::from_center_size(c, egui::vec2(16.0, 12.0)), 1.0, color);
        ui.painter().add(egui::Shape::ellipse_filled(c + egui::vec2(0.0, 6.0), egui::vec2(8.0, 3.0), color));
        ui.painter().add(egui::Shape::ellipse_filled(c - egui::vec2(0.0, 6.0), egui::vec2(8.0, 3.0), Color32::from_rgb(133, 183, 216)));
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
pub(super) fn paint_object_icon(painter: &egui::Painter, center: egui::Pos2, kind: &str, size: f32) {
    let rect = egui::Rect::from_center_size(center, egui::vec2(size, size));
    if kind.contains("view") {
        painter.rect_filled(rect, 0.0, Color32::from_rgb(142, 109, 166));
        let eye = Color32::from_rgb(182, 197, 202);
        let points = (0..=32).map(|i| { let angle = i as f32 / 32.0 * std::f32::consts::TAU; center + egui::vec2(angle.cos() * size * 0.36, angle.sin() * size * 0.24) }).collect();
        painter.add(egui::Shape::convex_polygon(points, eye, Stroke::NONE));
        painter.circle_filled(center, size * 0.14, Color32::from_rgb(142, 109, 166));
    } else {
        let color = Color32::from_rgb(67, 130, 159);
        painter.rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(size, size * 0.25)), 0.0, color);
        let gap = size * 0.07;
        let cell = (size - gap * 2.0) / 3.0;
        for row in 0..2 { for column in 0..3 {
            painter.rect_filled(egui::Rect::from_min_size(rect.min + egui::vec2(column as f32 * (cell + gap), size * 0.34 + row as f32 * (cell + gap)), egui::vec2(cell, cell)), 0.0, color);
        } }
    }
}
fn paint_navigation_icon(painter: &egui::Painter, c: egui::Pos2, destination: &Destination, ink: Color32) {
    let stroke = Stroke::new(1.4_f32, ink);
    let p = |x, y| c + egui::vec2(x, y);
    let line = |a: [f32; 2], b: [f32; 2]| { painter.line_segment([p(a[0], a[1]), p(b[0], b[1])], stroke); };
    let rect = |min: [f32; 2], max: [f32; 2], radius| { painter.rect_stroke(egui::Rect::from_min_max(p(min[0], min[1]), p(max[0], max[1])), radius, stroke, egui::StrokeKind::Inside); };
    match destination {
        Destination::Action(Action::Refresh) => {
            let points = (0..=24).map(|i| { let angle = 0.5 + i as f32 / 24.0 * 5.2; p(angle.cos() * 8.0, angle.sin() * 8.0) }).collect();
            painter.add(egui::Shape::line(points, stroke));
            line([8.0, -8.0], [7.0, -3.0]); line([7.0, -3.0], [2.0, -4.0]);
        }
        Destination::Action(Action::Connections) => {
            painter.circle_stroke(c, 10.0, stroke);
            rect([-4.0, -3.0], [4.0, 4.0], 2.0);
            line([-2.5, -6.0], [-2.5, -2.0]); line([2.5, -6.0], [2.5, -2.0]);
            line([0.0, 4.0], [0.0, 10.0]);
        }
        Destination::Action(Action::Databases) | Destination::Database(_) => {
            for y in [-6.0, 0.0, 6.0] { painter.add(egui::Shape::ellipse_stroke(p(0.0, y), egui::vec2(8.0, 3.0), stroke)); }
            line([-8.0, -6.0], [-8.0, 6.0]); line([8.0, -6.0], [8.0, 6.0]);
        }
        Destination::Action(Action::Cancel) => {
            painter.circle_stroke(c, 10.0, stroke);
            painter.rect_filled(egui::Rect::from_center_size(c, egui::vec2(7.0, 7.0)), 1.0, ink);
        }
        Destination::Action(Action::Activity | Action::Inspector) => {
            rect([-10.0, -7.0], [10.0, 7.0], 2.0);
            if matches!(destination, Destination::Action(Action::Activity)) {
                line([-9.0, 2.0], [9.0, 2.0]);
                for x in [-5.0, 0.0, 5.0] { painter.circle_filled(p(x, 4.5), 0.8, ink); }
            } else {
                line([3.0, -6.0], [3.0, 6.0]);
                for y in [-3.0, 0.0, 3.0] { line([5.0, y], [7.0, y]); }
            }
        }
        Destination::Object(_) | Destination::Schema(_) => {
            rect([-9.0, -8.0], [9.0, 8.0], 1.0);
            for x in [-3.0, 3.0] { line([x, -7.0], [x, 7.0]); }
            for y in [-2.0, 3.0] { line([-8.0, y], [8.0, y]); }
        }
        Destination::Action(Action::OpenSql | Action::NewQuery) | Destination::Tab(_) | Destination::Saved(_) => {
            painter.add(egui::Shape::closed_line(vec![p(-7.0, -9.0), p(2.0, -9.0), p(7.0, -4.0), p(7.0, 9.0), p(-7.0, 9.0)], stroke));
            line([2.0, -9.0], [2.0, -4.0]); line([2.0, -4.0], [7.0, -4.0]);
            if matches!(destination, Destination::Action(Action::NewQuery)) {
                line([-3.0, 2.0], [3.0, 2.0]); line([0.0, -1.0], [0.0, 5.0]);
            } else {
                for y in [-1.0, 3.0, 6.0] { line([-4.0, y], [4.0, y]); }
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
