//! Timestamped execution history, with honest undated migration for old profiles.
use super::*;
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Entry {
    pub sql: String,
    pub timestamp: Option<i64>,
}
impl Entry {
    fn labels(&self) -> (String, String) {
        self.timestamp.and_then(|time| chrono::DateTime::from_timestamp(time, 0))
            .map(|time| {
                let local = time.with_timezone(&chrono::Local);
                (local.format("%B %-d, %Y").to_string(), local.format("%-I:%M:%S %p").to_string())
            }).unwrap_or_else(|| ("Earlier queries".into(), "Time unavailable".into()))
    }
}
type DateGroup<'a> = (String, Vec<(usize, &'a Entry, String)>);

impl Rustable {
    pub(super) fn record_query_history(&mut self, sql: String) {
        self.prefs.query_history.insert(0, Entry { sql, timestamp: Some(chrono::Utc::now().timestamp()) });
        self.prefs.query_history.truncate(500);
        self.shared_preferences_dirty = true;
        self.history_selected = None;
    }
    pub(super) fn history_sidebar(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().corner_radius(16.0)
            .fill(ui.visuals().extreme_bg_color)
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .inner_margin(egui::Margin::symmetric(9, 5)).show(ui, |ui| {
                ui.add(TextEdit::singleline(&mut self.history_search).frame(false).hint_text("Search for history…").desired_width(f32::INFINITY));
            });
        ui.add_space(8.0);
        let search = self.history_search.to_lowercase();
        let matcher = crate::search::Query::new(&search);
        let mut groups: Vec<DateGroup<'_>> = Vec::new();
        for (index, entry) in self.prefs.query_history.iter().enumerate() {
            if matcher.rank(&entry.sql, "").is_none() { continue; }
            let (date, time) = entry.labels();
            if groups.last().is_none_or(|(label, _)| label != &date) { groups.push((date, Vec::new())); }
            groups.last_mut().unwrap().1.push((index, entry, time));
        }
        let mut chosen = None;
        egui::ScrollArea::vertical().id_salt("query-history").show(ui, |ui| {
            if groups.is_empty() { ui.weak(if search.is_empty() { "Queries you run appear here." } else { "No matching queries" }); }
            for (date, entries) in groups {
                egui::CollapsingHeader::new(RichText::new(&date).monospace().size(13.0)).id_salt(("history-date", &date)).default_open(true).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    for (index, entry, time) in entries {
                        let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 64.0), egui::Sense::click());
                        if self.history_selected == Some(index) {
                            ui.painter().rect_filled(rect, 8.0, ui.visuals().widgets.active.weak_bg_fill);
                        }
                        let content = rect.shrink2(egui::vec2(8.0, 5.0));
                        ui.painter().text(content.left_top(), egui::Align2::LEFT_TOP, time, egui::FontId::proportional(11.0), ui.visuals().weak_text_color());
                        let sql: String = entry.sql.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(300).collect();
                        let galley = crate::editor::preview(ui, &sql, content.width());
                        let clip = egui::Rect::from_min_max(content.min + egui::vec2(0.0, 17.0), content.max);
                        ui.painter().with_clip_rect(clip).galley(clip.min, galley, ui.visuals().text_color());
                        if primary_pressed(&response) { self.history_selected = Some(index); }
                        if response.clicked() { chosen = Some(entry.sql.clone()); }
                        response.on_hover_text(&entry.sql);
                    }
                });
            }
        });
        if let Some(sql) = chosen {
            self.tabs.push(QueryTab::new(sql));
            self.activate_query(self.tabs.len() - 1);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browsing_stays_in_activity_and_only_explicit_queries_enter_history() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        let (sender, receiver) = std::sync::mpsc::channel();
        app.worker.rx = receiver;
        let generated = "SELECT * FROM customers LIMIT 100";
        for _ in 0..2 {
            app.pending = Some(Pending::Table);
            sender.send(Ok(Payload::Table {
                sql: generated.into(), parameters: vec![], grid: Grid::default(),
                columns: vec![], indexes: vec![], triggers: vec![], foreign_keys: vec![],
            })).unwrap();
            app.poll();
        }
        assert!(app.prefs.query_history.is_empty());
        assert!(app.prefs.history.is_empty());
        assert!(app.activity.iter().any(|entry| entry.contains(generated)));

        // The same SQL belongs in history when explicitly run from the editor.
        app.show_query();
        app.tabs[app.tab].sql = generated.into();
        app.run_query(true);
        assert_eq!(app.prefs.query_history.len(), 1);
        assert_eq!(app.prefs.query_history[0].sql, generated);
    }

    #[test]
    fn legacy_history_does_not_invent_dates_and_repeated_executions_remain() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_optional_connection(&ctx, Preferences { history: vec!["SELECT 1".into()], ..Default::default() }, None, None);
        assert_eq!(app.prefs.query_history[0].labels().0, "Earlier queries");
        app.record_query_history("SELECT 1".into());
        app.record_query_history("SELECT 1".into());
        assert_eq!(app.prefs.query_history.len(), 3);
        assert!(app.prefs.query_history[0].timestamp.is_some());
        let restored: Preferences = serde_json::from_str(&serde_json::to_string(&app.prefs).unwrap()).unwrap();
        assert_eq!(restored.query_history.len(), 3);
        assert_eq!(restored.query_history[0].timestamp, app.prefs.query_history[0].timestamp);
    }
}
