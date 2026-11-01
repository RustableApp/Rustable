use super::*;
pub(super) struct SaveDraft {
    tab_id: u64,
    name: String,
    group: String,
    focus: bool,
    error: Option<String>,
}
pub(super) struct Document {
    path: PathBuf,
    sql: String,
    tab_id: Option<u64>,
}
impl Rustable {
    pub(in crate::app) fn poll_browser_documents(&mut self) {
        let Some(result) = self.query_library.document_operation.as_ref().and_then(|rx| rx.try_recv().ok()) else { return; };
        self.query_library.document_operation = None;
        match result {
            Ok(document) => {
                if self.error.as_deref().is_some_and(|error| error.starts_with("Browser query storage:")) {
                    self.error = None;
                }
                if let Some(id) = document.tab_id {
                    if let Some(tab) = self.tabs.iter_mut().chain(self.closed_tabs.iter_mut()).find(|tab| tab.id == id) {
                        tab.saved(document.path.clone(), document.sql);
                    }
                    self.query_library.save_dialog = None;
                    self.status = "Query saved in this browser".into();
                } else {
                    let activate = self.finish_cell_edit();
                    let mut tab = QueryTab::new(document.sql);
                    tab.saved(document.path.clone(), tab.sql.clone());
                    self.tabs.push(tab);
                    if activate { self.activate_query(self.tabs.len()-1); }
                }
                self.query_library.selected = Some(Selection::File(document.path));
                self.query_library.error = None;
                self.query_library.invalidate();
            }
            Err(error) => {
                self.error = Some(error.clone());
                if let Some(draft) = &mut self.query_library.save_dialog { draft.error = Some(error.clone()); }
                self.query_library.error = Some(error);
            }
        }
    }
    pub(in crate::app) fn open_browser_query(&mut self, ctx: &egui::Context, path: PathBuf) {
        if self.query_library.document_operation.is_some() || self.query_library.group_operation.is_some() { return; }
        let (tx, rx) = mpsc::channel(); self.query_library.document_operation = Some(rx);
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::browser_library::request(serde_json::json!({"op":"read", "path":path})).await.and_then(|reply| {
                let sql = reply["sql"].as_str().ok_or("Missing saved query contents")?.to_owned();
                Ok(Document { path, sql, tab_id: None })
            });
            let _ = tx.send(result); ctx.request_repaint();
        });
    }
    pub(in crate::app) fn save_browser_query(&mut self, ctx: &egui::Context, save_as: bool) {
        if self.query_library.document_operation.is_some() { return; }
        let tab = &self.tabs[self.tab];
        if !save_as && let Some(path) = &tab.source_path {
            self.write_browser_query(ctx, tab.id, path.clone());
            return;
        }
        self.query_library.save_dialog = Some(SaveDraft {
            tab_id: tab.id,
            name: tab.source_name.as_deref().unwrap_or("Query.sql").trim_end_matches(".sql").into(),
            group: tab.source_path.as_ref().and_then(|p| p.parent()).and_then(|p| p.strip_prefix("/Queries").ok()).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            focus: true,
            error: None,
        });
    }
    fn write_browser_query(&mut self, ctx: &egui::Context, id: u64, path: PathBuf) {
        let Some(tab) = self.tabs.iter().find(|tab| tab.id == id) else { return; };
        let sql = tab.sql.clone();
        let expected = if tab.source_path.as_ref() == Some(&path) { tab.saved_sql.clone() } else { None };
        let (tx, rx) = mpsc::channel(); self.query_library.document_operation = Some(rx);
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::browser_library::request(serde_json::json!({"op":"save", "path":path, "sql":sql, "expected":expected})).await.map(|_| Document { path, sql, tab_id: Some(id) });
            let _ = tx.send(result); ctx.request_repaint();
        });
    }
    pub(in crate::app) fn browser_query_save_window(&mut self, ctx: &egui::Context) {
        if self.query_library.save_dialog.is_none() { return; }
        self.query_library.poll(ctx, self.query_directory());
        let busy = self.query_library.document_operation.is_some();
        let mut cancel = false; let mut save = None;
        let draft = self.query_library.save_dialog.as_mut().unwrap();
        egui::Window::new("Save query").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(ctx, |ui| {
            ui.label("Saved in this browser. Download a SQL copy from the File menu.");
            ui.add_enabled_ui(!busy, |ui| {
                ui.label("Name");
                let response = ui.add(TextEdit::singleline(&mut draft.name).desired_width(320.0).char_limit(200));
                if draft.focus { response.request_focus(); draft.focus = false; }
                egui::ComboBox::from_label("Group").selected_text(if draft.group.is_empty() { "Ungrouped" } else { &draft.group }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.group, String::new(), "Ungrouped");
                    for group in &self.query_library.groups { ui.selectable_value(&mut draft.group, group.clone(), group); }
                });
                if let Some(error) = &draft.error { ui.colored_label(ui.visuals().error_fg_color, error); }
                ui.horizontal(|ui| {
                    cancel = ui.button("Cancel").clicked() || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                    if ui.button("Save").clicked() || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                        let name = draft.name.trim().trim_end_matches(".sql").trim();
                        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':']) {
                            draft.error = Some("Choose a query name without path separators.".into());
                        } else { save = Some((draft.tab_id, PathBuf::from("/Queries").join(&draft.group).join(format!("{name}.sql")))); }
                    }
                });
            });
            if busy { ui.label("Saving…"); }
        });
        if cancel { self.query_library.save_dialog = None; }
        if let Some((id, path)) = save {
            if let Some(draft) = &mut self.query_library.save_dialog { draft.error = None; }
            self.write_browser_query(ctx, id, path);
        }
    }
}
