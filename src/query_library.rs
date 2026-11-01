//! Named SQL files; directories are groups. Filesystem scans never run on the UI thread.
use super::*;
#[derive(Clone)]
pub(super) struct QueryFile { pub path: PathBuf, pub name: String, pub group: String }
#[derive(PartialEq)]
enum Selection { File(PathBuf), Recovered(usize) }
type Scan = (PathBuf, Result<(Vec<QueryFile>, Vec<String>), String>);
#[derive(Default)]
pub(super) struct Library {
    pub files: Vec<QueryFile>,
    groups: Vec<String>,
    root: Option<PathBuf>,
    receiver: Option<mpsc::Receiver<Scan>>,
    refreshed: Option<Instant>,
    search: String,
    selected: Option<Selection>,
    group: String,
    error: Option<String>,
}
impl Library {
    pub fn invalidate(&mut self) { self.refreshed = None; }
    pub fn poll(&mut self, ctx: &egui::Context, root: PathBuf) {
        if self.root.as_ref() != Some(&root) { self.root = Some(root.clone()); self.files.clear(); self.groups.clear(); self.selected = None; self.receiver = None; self.refreshed = None; }
        if let Some(rx) = &self.receiver && let Ok((scanned, result)) = rx.try_recv() {
            if scanned == root { match result { Ok((files, groups)) => { self.files = files; self.groups = groups; self.error = None; }, Err(error) => self.error = Some(error) } }
            self.receiver = None; self.refreshed = Some(Instant::now());
        }
        if self.receiver.is_none() && self.refreshed.is_none_or(|when| when.elapsed().as_secs() >= 5) {
            let (tx, rx) = mpsc::channel(); self.receiver = Some(rx);
            let ctx = ctx.clone(); let viewport = ctx.viewport_id();
            std::thread::spawn(move || { let result = scan(&root); let _ = tx.send((root, result)); ctx.request_repaint_of(viewport); });
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(5));
    }
}
fn scan(root: &std::path::Path) -> Result<(Vec<QueryFile>, Vec<String>), String> {
    if !root.exists() { return Ok((Vec::new(), Vec::new())); }
    let mut groups = Vec::new();
    let mut files = Vec::new(); let mut todo = vec![(root.to_owned(), 0)];
    while let Some((directory, depth)) = todo.pop() {
        for item in std::fs::read_dir(&directory).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?; let kind = item.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() { continue; }
            let path = item.path();
            if kind.is_dir() && depth < 8 { groups.push(path.strip_prefix(root).unwrap().to_string_lossy().into_owned()); todo.push((path, depth + 1)); }
            else if kind.is_file() && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("sql")) {
                let group = path.parent().unwrap().strip_prefix(root).unwrap().to_string_lossy().into_owned();
                let name = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                files.push(QueryFile { path, name, group });
                if files.len() >= 5000 { return Err("Query folder exceeds 5,000 files. Choose a smaller folder.".into()); }
            }
        }
    }
    files.sort_by(|a,b| (&a.group, &a.name).cmp(&(&b.group, &b.name)));
    Ok((files, groups))
}
impl Rustable {
    pub(super) fn query_directory(&self) -> PathBuf {
        self.prefs.query_directory.clone().unwrap_or_else(|| eframe::storage_dir(if cfg!(feature="qa") { "Rustable QA" } else { "Rustable" }).unwrap_or_else(std::env::temp_dir).join("Queries"))
    }
    pub(super) fn save_query_file(&mut self, ctx: &egui::Context, save_as: bool) {
        if self.pending.is_some() { return; }
        let tab = &self.tabs[self.tab];
        if !save_as && let Some(path) = tab.source_path.clone() {
            self.dispatch(Command::SaveScript { path: path.clone(), sql: tab.sql.clone(), tab_id: tab.id, expected: tab.saved_sql.clone().map(|sql| crate::script_file::SaveSource { path, sql }) }, Pending::Export);
        } else { self.script_dialog(ctx, true); }
    }
    pub(super) fn choose_query_directory(&self, ctx: &egui::Context) {
        let directory = self.query_directory(); let tx = self.files_tx.clone(); let ctx = ctx.clone(); let viewport = ctx.viewport_id();
        std::thread::spawn(move || { if let Some(path) = rfd::FileDialog::new().set_directory(directory).set_title("Saved queries folder").pick_folder() { let _ = tx.send(FileAction::QueryDirectory(path)); ctx.request_repaint_of(viewport); } });
    }
    pub(super) fn query_library_sidebar(&mut self, ui: &mut egui::Ui) {
        let root = self.query_directory(); self.query_library.poll(ui.ctx(), root.clone());
        ui.horizontal(|ui| {
            egui::Frame::new().corner_radius(16.0).fill(ui.visuals().extreme_bg_color).stroke(ui.visuals().widgets.noninteractive.bg_stroke).inner_margin(egui::Margin::symmetric(8,5)).show(ui, |ui| {
                ui.add(TextEdit::singleline(&mut self.query_library.search).frame(false).hint_text("Search for query…").desired_width((ui.available_width()-40.0).max(70.0)));
            });
            ui.menu_button("⋯", |ui| {
                ui.label(root.display().to_string());
                if ui.button("Choose storage folder…").clicked() { self.choose_query_directory(ui.ctx()); ui.close(); }
                if ui.button("Refresh").clicked() { self.query_library.invalidate(); ui.close(); }
                ui.separator(); ui.label("New group");
                ui.text_edit_singleline(&mut self.query_library.group);
                let name = self.query_library.group.trim();
                let valid = !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':']);
                if ui.add_enabled(valid, egui::Button::new("Create group")).clicked() {
                    let path = root.join(name); let tx = self.files_tx.clone(); let ctx = ui.ctx().clone(); let viewport = ctx.viewport_id(); let root = root.clone();
                    std::thread::spawn(move || { let action = match std::fs::create_dir_all(path) { Ok(()) => FileAction::QueryDirectory(root), Err(e) => FileAction::QueryLibraryError(e.to_string()) }; let _ = tx.send(action); ctx.request_repaint_of(viewport); });
                    self.query_library.group.clear(); ui.close();
                }
            });
        });
        ui.add_space(8.0);
        let search = self.query_library.search.to_lowercase();
        let mut chosen = None;
        let mut groups: std::collections::BTreeMap<&str, Vec<&QueryFile>> = Default::default();
        for group in &self.query_library.groups { if group.to_lowercase().contains(&search) { groups.entry(group).or_default(); } }
        for file in &self.query_library.files {
            if format!("{} {}", file.group, file.name).to_lowercase().contains(&search) { groups.entry(if file.group.is_empty() { "Ungrouped" } else { &file.group }).or_default().push(file); }
        }
        egui::ScrollArea::vertical().id_salt("saved-query-library").show(ui, |ui| {
            if groups.is_empty() { ui.weak("Save a named query with ⌘S. Folders are groups."); }
            for (group, files) in groups {
                egui::CollapsingHeader::new(RichText::new(format!("▣ {group}")).monospace()).default_open(true).show(ui, |ui| {
                    for file in files {
                        let selection = Selection::File(file.path.clone());
                        let active = self.query_library.selected.as_ref() == Some(&selection);
                        let response = ui.add(egui::Button::new(format!("▤ {}", file.name)).selected(active).frame(false)).on_hover_text(file.path.display().to_string());
                        if response.clicked() { self.query_library.selected = Some(selection); }
                        if response.double_clicked() { chosen = Some(file.path.clone()); }
                    }
                });
            }
            if !self.prefs.favorites.is_empty() {
                egui::CollapsingHeader::new("Recovered unsaved queries").show(ui, |ui| {
                    for (index, sql) in self.prefs.favorites.iter().enumerate() {
                        let selection = Selection::Recovered(index);
                        let active = self.query_library.selected.as_ref() == Some(&selection);
                        let response = ui.add(egui::Button::new(format!("Recovered query {}", index + 1)).selected(active).frame(false)).on_hover_text("Double-click to open, then Cmd/Ctrl+S to name and save as a file");
                        if response.clicked() { self.query_library.selected = Some(selection); }
                        if response.double_clicked() { self.tabs.push(QueryTab::new(sql.clone())); self.tab = self.tabs.len()-1; self.view = View::Query; }
                    }
                });
            }
        });
        if let Some(error) = &self.query_library.error { ui.colored_label(ui.visuals().error_fg_color, error); }
        if let Some(path) = chosen {
            if let Some(index) = self.tabs.iter().position(|tab| tab.open && tab.source_path.as_ref() == Some(&path)) { self.activate_query(index); }
            else { self.dispatch(Command::OpenScript(path), Pending::Export); }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn files_are_named_and_grouped_without_reading_sql_contents() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("Reports")).unwrap();
        std::fs::write(temp.path().join("Reports/Revenue.sql"), "SELECT 1").unwrap();
        std::fs::write(temp.path().join("Health.SQL"), "SELECT 2").unwrap();
        std::fs::write(temp.path().join("notes.txt"), "ignore").unwrap();
        let (files, groups) = scan(temp.path()).unwrap(); assert!(groups.contains(&"Reports".into())); assert_eq!(files.len(),2);
        assert_eq!((&*files[0].name, &*files[0].group), ("Health", ""));
        assert_eq!((&*files[1].name, &*files[1].group), ("Revenue", "Reports"));
    }
}
