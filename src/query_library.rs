//! Named SQL files; directories are groups. Filesystem scans never run on the UI thread.
use super::*;
#[cfg(target_arch = "wasm32")]
#[path = "browser_query_library.rs"]
mod browser;

#[derive(Clone, Debug)]
pub(super) struct QueryFile { pub path: PathBuf, pub name: String, pub group: String }
#[derive(PartialEq)]
enum Selection { File(PathBuf), Recovered(usize) }
type GroupChange = Result<(PathBuf, Option<PathBuf>, PathBuf), String>;
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
    rename_group: Option<(PathBuf, String)>,
    focus_group: bool,
    rename_file: Option<(PathBuf, String)>,
    focus_file: bool,
    group_operation: Option<mpsc::Receiver<GroupChange>>,
    group_operation_root: Option<PathBuf>,
    error: Option<String>,
    #[cfg(target_arch = "wasm32")]
    save_dialog: Option<browser::SaveDraft>,
    #[cfg(target_arch = "wasm32")]
    document_operation: Option<mpsc::Receiver<Result<browser::Document, String>>>,
}
impl Library {
    pub fn renaming_inline(&self) -> bool {
        self.rename_group.is_some() || self.rename_file.is_some()
    }
    pub fn owns_keyboard(&self) -> bool {
        #[cfg(target_arch = "wasm32")]
        { self.save_dialog.is_some() }
        #[cfg(not(target_arch = "wasm32"))]
        { false }
    }
    pub fn invalidate(&mut self) { self.refreshed = None; self.receiver = None; }
    fn accept_listing(&mut self, root: &std::path::Path, files: Vec<QueryFile>, groups: Vec<String>) {
        if self.rename_file.as_ref().is_some_and(|(path, _)| !files.iter().any(|file| &file.path == path)) {
            self.rename_file = None;
            self.focus_file = false;
        }
        if self.rename_group.as_ref().is_some_and(|(path, _)| !groups.iter().any(|group| root.join(group) == *path)) {
            self.rename_group = None;
            self.focus_group = false;
        }
        if let Some(Selection::File(path)) = &self.selected
            && !files.iter().any(|file| &file.path == path) { self.selected = None; }
        self.files = files;
        self.groups = groups;
        self.error = None;
    }

    #[cfg(target_arch = "wasm32")]
    pub fn poll(&mut self, ctx: &egui::Context, root: PathBuf) {
        if let Some(rx) = &self.receiver && let Ok((_, result)) = rx.try_recv() {
            match result { Ok((files, groups)) => self.accept_listing(&root, files, groups), Err(error) => self.error = Some(error) }
            self.receiver = None; self.refreshed = Some(Instant::now());
        }
        if self.receiver.is_none() && self.refreshed.is_none_or(|when| when.elapsed().as_secs() >= 5) {
            let (tx, rx) = mpsc::channel(); self.receiver = Some(rx);
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let result = async {
                    let records = crate::browser_library::request(serde_json::json!({"op":"list"})).await?;
                    let mut files = Vec::new(); let mut groups = Vec::new();
                    for row in records.as_array().ok_or("Invalid library listing")? {
                        let path = PathBuf::from(row["path"].as_str().ok_or("Invalid query path")?);
                        if row["kind"] == "group" { groups.push(path.strip_prefix(&root).map_err(|e| e.to_string())?.to_string_lossy().into_owned()); }
                        else {
                            let name = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                            let group = path.parent().ok_or("Missing query group")?.strip_prefix(&root).map_err(|e| e.to_string())?.to_string_lossy().into_owned();
                            files.push(QueryFile { path, name, group });
                        }
                    }
                    files.sort_by(|a,b| (&a.group, &a.name).cmp(&(&b.group, &b.name)));
                    groups.sort();
                    Ok::<_, String>((files, groups))
                }.await;
                let _ = tx.send((root, result)); ctx.request_repaint();
            });
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(5));
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn poll(&mut self, ctx: &egui::Context, root: PathBuf) {
        if self.root.as_ref() != Some(&root) {
            self.root = Some(root.clone()); self.files.clear(); self.groups.clear();
            self.selected = None; self.receiver = None; self.refreshed = None;
            self.rename_group = None; self.rename_file = None;
            self.focus_group = false; self.focus_file = false;
            self.search.clear(); self.error = None;
        }
        if let Some(rx) = &self.receiver && let Ok((scanned, result)) = rx.try_recv() {
            if scanned == root { match result { Ok((files, groups)) => self.accept_listing(&root, files, groups), Err(error) => self.error = Some(error) } }
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
    scan_with_limits(root, 5000, 1000, 20_000)
}
fn scan_with_limits(root: &std::path::Path, max_files: usize, max_groups: usize, max_entries: usize) -> Result<(Vec<QueryFile>, Vec<String>), String> {
    if !root.exists() { return Ok((Vec::new(), Vec::new())); }
    let mut groups = Vec::new();
    let mut files = Vec::new(); let mut todo = vec![(root.to_owned(), 0)];
    let mut entries = 0;
    while let Some((directory, depth)) = todo.pop() {
        for item in std::fs::read_dir(&directory).map_err(|e| e.to_string())? {
            entries += 1;
            if entries > max_entries { return Err(format!("Query folder exceeds {max_entries} directory entries. Choose a smaller folder.")); }
            let item = item.map_err(|e| e.to_string())?; let kind = item.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() { continue; }
            let path = item.path();
            if kind.is_dir() && depth < 8 {
                if groups.len() == max_groups { return Err(format!("Query folder exceeds {max_groups} groups. Choose a smaller folder.")); }
                groups.push(path.strip_prefix(root).unwrap().to_string_lossy().into_owned()); todo.push((path, depth + 1));
            }
            else if kind.is_file() && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("sql")) {
                let group = path.parent().unwrap().strip_prefix(root).unwrap().to_string_lossy().into_owned();
                let name = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                if files.len() == max_files { return Err(format!("Query folder exceeds {max_files} SQL files. Choose a smaller folder.")); }
                files.push(QueryFile { path, name, group });
            }
        }
    }
    files.sort_by(|a,b| (&a.group, &a.name).cmp(&(&b.group, &b.name)));
    Ok((files, groups))
}
fn query_row(ui: &mut egui::Ui, text: &str, selected: bool) -> egui::Response {
    library_row(ui, text, selected, false)
}
fn library_row(ui: &mut egui::Ui, text: &str, selected: bool, folder: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::click());
    let selected = selected || (!folder && (response.clicked() || response.secondary_clicked()));
    if selected {
        ui.painter().rect_filled(rect, 8.0, ui.visuals().selection.bg_fill);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 8.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let color = if selected { egui::Color32::WHITE } else { ui.visuals().text_color() };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), text));
    let center = rect.left_center() + egui::vec2(14.0, 0.0);
    let point = |x, y| center + egui::vec2(x, y);
    if folder {
        let purple = egui::Color32::from_rgb(157, 106, 192);
        ui.painter().rect_filled(egui::Rect::from_min_max(point(-7.0, -6.0), point(-1.0, -2.0)), 1.5, purple);
        ui.painter().rect_filled(egui::Rect::from_min_max(point(-7.0, -4.0), point(7.0, 6.0)), 1.5, purple.gamma_multiply(0.8));
        ui.painter().line_segment([point(-5.0, -1.5), point(5.0, -1.5)], Stroke::new(1.0_f32, purple));
    } else {
        let stroke = Stroke::new(1.2_f32, if selected { color } else { ui.visuals().weak_text_color() });
        ui.painter().add(egui::Shape::closed_line(vec![point(-5.0, -7.0), point(1.0, -7.0), point(5.0, -3.0), point(5.0, 7.0), point(-5.0, 7.0)], stroke));
        ui.painter().add(egui::Shape::line(vec![point(1.0, -7.0), point(1.0, -3.0), point(5.0, -3.0)], stroke));
    }
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), egui::FontId::monospace(13.0), color, (rect.width() - 36.0).max(1.0));
    job.wrap.max_rows = 1;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    ui.painter().galley(egui::pos2(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0), galley, color);
    response
}
fn change_group(root: &std::path::Path, rename: Option<(PathBuf, String)>) -> Result<PathBuf, String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    if let Some((old, name)) = rename {
        let name = name.trim();
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':']) { return Err("Choose a nonempty folder name without path separators.".into()); }
        let canonical_root = root.canonicalize().map_err(|e| e.to_string())?;
        if !old.canonicalize().map_err(|e| e.to_string())?.starts_with(&canonical_root) || old == root { return Err("Choose a group inside the saved queries folder.".into()); }
        let filename;
        let name = if old.is_file() {
            let stem = name.strip_suffix(".sql").or_else(|| name.strip_suffix(".SQL")).unwrap_or(name).trim();
            if stem.is_empty() { return Err("Choose a query name.".into()); }
            filename = format!("{stem}.sql");
            &filename
        } else { name };
        let target = old.parent().ok_or("Missing group parent")?.join(name);
        if target == old { return Ok(old); }
        if target.try_exists().map_err(|e| e.to_string())? {
            // On case-insensitive volumes the new spelling resolves to the old
            // item. Permit that rename, but not a distinct hard link or symlink
            // directory entry which happens to point at the same file.
            let same_item = same_file::is_same_file(&old, &target).map_err(|e| e.to_string())?;
            let mut distinct_entry = false;
            if same_item {
                for entry in std::fs::read_dir(old.parent().ok_or("Missing group parent")?).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    if entry.file_name() == target.file_name().unwrap() { distinct_entry = true; break; }
                }
            }
            if !same_item || distinct_entry { return Err("A query or folder with that name already exists.".into()); }
        }
        std::fs::rename(&old, &target).map_err(|e| e.to_string())?;
        Ok(target)
    } else {
        for suffix in 1..=5000 {
            let path = root.join(if suffix == 1 { "New group".into() } else { format!("New group {suffix}") });
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("Too many groups named New group.".into())
    }
}
impl Rustable {
    #[cfg(target_arch = "wasm32")]
    fn start_query_group_operation(&mut self, ctx: &egui::Context, rename: Option<(PathBuf, String)>) {
        if self.query_library.group_operation.is_some() || self.query_library.document_operation.is_some() { return; }
        let root = self.query_directory();
        let (tx, rx) = mpsc::channel(); self.query_library.group_operation = Some(rx);
        self.query_library.group_operation_root = Some(root.clone());
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let old = rename.as_ref().map(|(path, _)| path.clone());
            let request = match rename {
                Some((path, name)) => serde_json::json!({"op":"rename", "path":path, "name":name}),
                None => serde_json::json!({"op":"group"}),
            };
            let result = crate::browser_library::request(request).await.and_then(|reply| {
                let path = PathBuf::from(reply["path"].as_str().ok_or("Missing group path")?);
                Ok((root, old, path))
            });
            let _ = tx.send(result); ctx.request_repaint();
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn start_query_group_operation(&mut self, ctx: &egui::Context, rename: Option<(PathBuf, String)>) {
        if self.query_library.group_operation.is_some() { return; }
        if self.pending.is_some() { self.query_library.error = Some("Wait for the current operation before changing query folders.".into()); return; }
        let root = self.query_directory();
        let (tx, rx) = mpsc::channel(); self.query_library.group_operation = Some(rx);
        self.query_library.group_operation_root = Some(root.clone());
        let ctx = ctx.clone(); let viewport = ctx.viewport_id();
        std::thread::spawn(move || {
            let old = rename.as_ref().map(|(path, _)| path.clone());
            let result = change_group(&root, rename).map(|path| (root, old, path));
            let _ = tx.send(result); ctx.request_repaint_of(viewport);
        });
    }
    pub(super) fn poll_query_groups(&mut self) {
        #[cfg(target_arch = "wasm32")]
        self.poll_browser_documents();
        let Some(result) = self.query_library.group_operation.as_ref().and_then(|rx| rx.try_recv().ok()) else { return; };
        self.query_library.group_operation = None;
        let operation_root = self.query_library.group_operation_root.take();
        match result {
            Ok((root, old, path)) => {
                if let Some(old) = old {
                    for tab in self.tabs.iter_mut().chain(self.closed_tabs.iter_mut()) {
                        if let Some(source) = &tab.source_path && let Ok(relative) = source.strip_prefix(&old) {
                            let destination = if relative.as_os_str().is_empty() { path.clone() } else { path.join(relative) };
                            tab.source_name = destination.file_name().map(|name| name.to_string_lossy().into_owned());
                            tab.source_path = Some(destination);
                        }
                    }
                    // Documents still need their new paths even if the user
                    // switched libraries while the filesystem operation ran.
                    if root != self.query_directory() { return; }
                    if let Some(Selection::File(selected)) = &mut self.query_library.selected
                        && let Ok(relative) = selected.strip_prefix(&old) { *selected = if relative.as_os_str().is_empty() { path.clone() } else { path.join(relative) }; }
                    self.query_library.rename_group = None;
                    self.query_library.rename_file = None;
                } else if root == self.query_directory() {
                    self.query_library.search.clear();
                    self.query_library.rename_group = Some((path.clone(), path.file_name().unwrap().to_string_lossy().into_owned()));
                    self.query_library.focus_group = true;
                }
                if root == self.query_directory() {
                    self.query_library.error = None;
                    self.query_library.invalidate();
                }
            }
            Err(error) => {
                if operation_root.as_ref().is_none_or(|root| *root == self.query_directory()) {
                    self.query_library.error = Some(error);
                    self.query_library.focus_group = true;
                    self.query_library.focus_file = true;
                }
            }
        }
    }
    pub(super) fn query_directory(&self) -> PathBuf {
        #[cfg(target_arch = "wasm32")]
        return PathBuf::from("/Queries");
        #[cfg(not(target_arch = "wasm32"))]
        self.prefs.query_directory.clone().unwrap_or_else(|| eframe::storage_dir(if cfg!(feature="qa") { "Rustable QA" } else { "Rustable" }).unwrap_or_else(std::env::temp_dir).join("Queries"))
    }
    pub(super) fn save_query_file(&mut self, ctx: &egui::Context, save_as: bool) {
        if self.pending.is_some() || self.query_library.group_operation.is_some() { return; }
        #[cfg(target_arch = "wasm32")]
        { self.save_browser_query(ctx, save_as); }
        #[cfg(not(target_arch = "wasm32"))]
        {
        let tab = &self.tabs[self.tab];
        if !save_as && let Some(path) = tab.source_path.clone() {
            self.dispatch(Command::SaveScript { path: path.clone(), sql: tab.sql.clone(), tab_id: tab.id, expected: tab.saved_sql.clone().map(|sql| crate::script_file::SaveSource { path, sql }) }, Pending::Export);
        } else { self.script_dialog(ctx, true); }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn choose_query_directory(&self, ctx: &egui::Context) {
        let directory = self.query_directory(); let tx = self.files_tx.clone(); let ctx = ctx.clone(); let viewport = ctx.viewport_id();
        std::thread::spawn(move || { if let Some(path) = rfd::FileDialog::new().set_directory(directory).set_title("Saved queries folder").pick_folder() { let _ = tx.send(FileAction::QueryDirectory(path)); ctx.request_repaint_of(viewport); } });
    }
    pub(super) fn query_library_sidebar(&mut self, ui: &mut egui::Ui) {
        let root = self.query_directory(); self.query_library.poll(ui.ctx(), root.clone());
        let mut new_group = false;
        let mut rename = None;
        ui.horizontal(|ui| {
            egui::Frame::new().corner_radius(16.0).fill(ui.visuals().extreme_bg_color).stroke(ui.visuals().widgets.noninteractive.bg_stroke).inner_margin(egui::Margin::symmetric(8,5)).show(ui, |ui| {
                ui.add(TextEdit::singleline(&mut self.query_library.search).frame(false).hint_text("Search for query…").desired_width((ui.available_width()-40.0).max(70.0)));
            });
            let options = ui.add(egui::Button::new("").min_size(egui::vec2(28.0, 26.0)).frame(false));
            options.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Query library options"));
            let ink = ui.style().interact(&options).fg_stroke.color;
            for x in [-5.0, 0.0, 5.0] {
                ui.painter().circle_filled(options.rect.center() + egui::vec2(x, 0.0), 1.5, ink);
            }
            let options = options.on_hover_text("Query library options");
            egui::Popup::menu(&options).show(|ui| {
                ui.label(if cfg!(target_arch = "wasm32") { "Stored in this browser".into() } else { root.display().to_string() });
                if !cfg!(target_arch = "wasm32") && ui.button("Choose storage folder…").clicked() { self.choose_query_directory(ui.ctx()); ui.close(); }
                if ui.button("Refresh").clicked() { self.query_library.invalidate(); ui.close(); }
                ui.separator();
                if ui.add_enabled(self.query_library.group_operation.is_none(), egui::Button::new("New group")).clicked() {
                    new_group = true; ui.close();
                }
            });
        });
        ui.add_space(8.0);
        let search = self.query_library.search.to_lowercase();
        let mut chosen = None;
        let mut groups: std::collections::BTreeMap<&str, Vec<&QueryFile>> = Default::default();
        for group in &self.query_library.groups { if group.to_lowercase().contains(&search) { groups.entry(group).or_default(); } }
        for file in &self.query_library.files {
            if format!("{} {}", file.group, file.name).to_lowercase().contains(&search) { groups.entry(&file.group).or_default().push(file); }
        }
        egui::ScrollArea::vertical().id_salt("saved-query-library").show(ui, |ui| {
            if groups.is_empty() { ui.weak("Save a named query with ⌘S. Folders are groups."); }
            for (group, files) in groups {
                let path = root.join(group);
                let label = if group.is_empty() { "Ungrouped" } else { group };
                let editing = self.query_library.rename_group.as_ref().is_some_and(|(old, _)| old == &path);
                let mut toggle = false;
                let mut header = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), ui.make_persistent_id(("query-group", group)), true).show_header(ui, |ui| {
                    if editing {
                        let (_, name) = self.query_library.rename_group.as_mut().unwrap();
                        let response = ui.add(TextEdit::singleline(name).id_salt(("query-group-name", group)).desired_width(ui.available_width()));
                        if self.query_library.focus_group {
                            self.query_library.focus_group = false;
                            response.request_focus(); response.scroll_to_me(Some(egui::Align::Center));
                            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
                                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(name.chars().count()))));
                                state.store(ui.ctx(), response.id);
                            }
                        }
                        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) { self.query_library.rename_group = None; response.surrender_focus(); }
                        else if response.lost_focus() || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) { rename = Some((path.clone(), name.clone())); }
                    } else {
                        let response = library_row(ui, label, false, true);
                        toggle = response.clicked();
                        if !group.is_empty() {
                            response.context_menu(|ui| {
                                if ui.add_enabled(self.query_library.group_operation.is_none(), egui::Button::new("Rename group")).clicked() {
                                    self.query_library.rename_group = Some((path.clone(), path.file_name().unwrap().to_string_lossy().into_owned()));
                                    self.query_library.focus_group = true; ui.close();
                                }
                            });
                        }
                    }
                });
                if toggle { header.toggle(); }
                header.body(|ui| {
                    for file in files {
                        let selection = Selection::File(file.path.clone());
                        let active = self.query_library.selected.as_ref() == Some(&selection);
                        if self.query_library.rename_file.as_ref().is_some_and(|(path, _)| path == &file.path) {
                            let (_, name) = self.query_library.rename_file.as_mut().unwrap();
                            let response = ui.add(TextEdit::singleline(name).id_salt(("rename-query", &file.path)).desired_width(ui.available_width()));
                            if self.query_library.focus_file {
                                self.query_library.focus_file = false;
                                response.request_focus();
                                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
                                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(name.chars().count()))));
                                    state.store(ui.ctx(), response.id);
                                }
                            }
                            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) { self.query_library.rename_file = None; response.surrender_focus(); }
                            else if response.lost_focus() || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) { rename = Some((file.path.clone(), name.clone())); }
                        } else {
                            let response = ui.add_enabled_ui(self.query_library.group_operation.is_none(), |ui| query_row(ui, &file.name, active)).inner.on_hover_text(file.path.display().to_string());
                            if response.clicked() || response.secondary_clicked() { self.query_library.selected = Some(selection); }
                            if response.double_clicked() { chosen = Some(file.path.clone()); }
                            response.context_menu(|ui| {
                                if ui.button("Rename query").clicked() {
                                    self.query_library.rename_file = Some((file.path.clone(), file.name.clone()));
                                    self.query_library.focus_file = true; ui.close();
                                }
                            });
                        }
                    }
                });
            }
            if !self.prefs.favorites.is_empty() {
                egui::CollapsingHeader::new("Recovered unsaved queries").show(ui, |ui| {
                    for (index, sql) in self.prefs.favorites.iter().enumerate() {
                        let selection = Selection::Recovered(index);
                        let active = self.query_library.selected.as_ref() == Some(&selection);
                        let response = query_row(ui, &format!("Recovered query {}", index + 1), active).on_hover_text("Double-click to open, then Cmd/Ctrl+S to name and save as a file");
                        if response.clicked() { self.query_library.selected = Some(selection); }
                        if response.double_clicked() { self.tabs.push(QueryTab::new(sql.clone())); self.tab = self.tabs.len()-1; self.view = View::Query; }
                    }
                });
            }
        });
        if new_group { self.start_query_group_operation(ui.ctx(), None); }
        if let Some(rename) = rename { self.start_query_group_operation(ui.ctx(), Some(rename)); }
        if let Some(error) = &self.query_library.error { ui.colored_label(ui.visuals().error_fg_color, error); }
        if let Some(path) = chosen {
            if let Some(index) = self.tabs.iter().position(|tab| tab.open && tab.source_path.as_ref() == Some(&path)) { self.activate_query(index); }
            else {
                #[cfg(target_arch = "wasm32")]
                self.open_browser_query(ui.ctx(), path);
                #[cfg(not(target_arch = "wasm32"))]
                self.dispatch(Command::OpenScript(path), Pending::Export);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_rename_preserves_sql_and_rejects_an_existing_destination() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("Old.sql");
        std::fs::write(&old, "SELECT 'unchanged';").unwrap();
        std::fs::write(root.path().join("Existing.sql"), "SELECT 2;").unwrap();
        assert!(change_group(root.path(), Some((old.clone(), "Existing".into()))).is_err());
        assert!(change_group(root.path(), Some((old.clone(), ".sql".into()))).is_err());
        let new = change_group(root.path(), Some((old.clone(), "Customer report".into()))).unwrap();
        assert_eq!(new, root.path().join("Customer report.sql"));
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "SELECT 'unchanged';");
        assert!(!old.exists());
        let ctx = egui::Context::default();
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        app.tabs[0].saved(old.clone(), "SELECT 'unchanged';".into());
        app.tabs[0].sql = "SELECT 'unsaved edit';".into();
        app.tabs[0].refresh_dirty();
        let (tx, rx) = mpsc::channel();
        app.query_library.group_operation = Some(rx);
        tx.send(Ok((root.path().to_owned(), Some(old), new.clone()))).unwrap();
        app.poll_query_groups();
        assert_eq!(app.tabs[0].source_path, Some(new));
        assert_eq!(app.tabs[0].source_name.as_deref(), Some("Customer report.sql"));
        assert_eq!(app.tabs[0].sql, "SELECT 'unsaved edit';");
        assert!(app.tabs[0].dirty);
    }
    #[test]
    fn old_library_operation_does_not_reset_new_library_but_retargets_documents() {
        let ctx = egui::Context::default();
        let old_root = tempfile::tempdir().unwrap();
        let new_root = tempfile::tempdir().unwrap();
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        app.prefs.query_directory = Some(new_root.path().to_owned());
        let old = old_root.path().join("Old.sql");
        let renamed = old_root.path().join("Renamed.sql");
        app.tabs[0].source_path = Some(old.clone());
        let current = new_root.path().join("Current.sql");
        app.query_library.rename_file = Some((current.clone(), "Current draft".into()));
        app.query_library.selected = Some(Selection::File(current.clone()));
        app.query_library.error = Some("Current library message".into());
        app.query_library.refreshed = Some(Instant::now());
        for result in [Ok((old_root.path().to_owned(), Some(old), renamed.clone())), Err("Old operation failed".into())] {
            let (tx, rx) = mpsc::channel();
            app.query_library.group_operation = Some(rx);
            app.query_library.group_operation_root = Some(old_root.path().to_owned());
            tx.send(result).unwrap();
            app.poll_query_groups();
            assert_eq!(app.query_library.rename_file.as_ref().unwrap().1, "Current draft");
            assert!(app.query_library.selected == Some(Selection::File(current.clone())));
            assert_eq!(app.query_library.error.as_deref(), Some("Current library message"));
            assert!(app.query_library.refreshed.is_some());
        }
        assert_eq!(app.tabs[0].source_path, Some(renamed));
    }

    #[test]
    fn listing_refresh_preserves_live_rename_and_clears_removed_items() {
        let root = PathBuf::from("/Queries");
        let path = root.join("Reports/Customers.sql");
        let mut library = Library {
            rename_file: Some((path.clone(), "Unsaved name".into())),
            rename_group: Some((root.join("Reports"), "Unsaved group".into())),
            selected: Some(Selection::File(path.clone())),
            focus_file: true, focus_group: true,
            ..Default::default()
        };
        library.accept_listing(&root, vec![QueryFile { path: path.clone(), name: "Customers".into(), group: "Reports".into() }], vec!["Reports".into()]);
        assert_eq!(library.rename_file.as_ref().unwrap().1, "Unsaved name");
        assert_eq!(library.rename_group.as_ref().unwrap().1, "Unsaved group");
        assert!(library.selected.is_some());
        library.accept_listing(&root, vec![], vec![]);
        assert!(!library.renaming_inline());
        assert!(library.selected.is_none());
        assert!(!library.focus_file && !library.focus_group);
    }

    #[test]
    fn changing_storage_root_cancels_old_inline_edits() {
        let ctx = egui::Context::default();
        let old = tempfile::tempdir().unwrap();
        let new = tempfile::tempdir().unwrap();
        let mut library = Library {
            root: Some(old.path().to_owned()),
            rename_file: Some((old.path().join("Report.sql"), "Draft name".into())),
            rename_group: Some((old.path().join("Reports"), "Draft group".into())),
            search: "old filter".into(), error: Some("old error".into()),
            ..Default::default()
        };
        library.poll(&ctx, new.path().to_owned());
        assert!(!library.renaming_inline());
        assert!(library.search.is_empty());
        assert!(library.error.is_none());
        assert_eq!(library.root.as_deref(), Some(new.path()));
    }

    #[test]
    fn inline_library_rename_owns_workspace_shortcuts_without_disabling_sidebar() {
        let ctx = egui::Context::default();
        let root = tempfile::tempdir().unwrap();
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        app.prefs.query_directory = Some(root.path().to_owned());
        app.sidebar_section = 1;
        app.compact_panel = 1;
        for group in [false, true] {
            app.query_library.rename_group = group.then(|| (root.path().join("Reports"), "Reports".into()));
            app.query_library.rename_file = (!group).then(|| (root.path().join("Query.sql"), "Query".into()));
            assert!(!app.dialog_owns_workspace_shortcuts(), "an inline editor must not disable its own sidebar");
            for key in [egui::Key::T, egui::Key::W, egui::Key::P, egui::Key::K, egui::Key::S, egui::Key::Enter] {
                let tabs = app.tabs.len();
                let _ = ctx.run(egui::RawInput { events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND }], ..Default::default() }, |ctx| {
                    app.workspace_shortcuts(ctx);
                    assert!(ctx.input(|input| input.key_pressed(key)), "workspace must leave {key:?} to the inline editor");
                });
                assert_eq!(app.tabs.len(), tabs);
                assert!(app.pending.is_none());
                assert!(!app.navigator.open);
            }
        }
        app.query_library.rename_group = None;
        app.query_library.rename_file = None;
        let tabs = app.tabs.len();
        let _ = ctx.run(egui::RawInput { events: vec![egui::Event::Key { key: egui::Key::T, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND }], ..Default::default() }, |ctx| app.workspace_shortcuts(ctx));
        assert_eq!(app.tabs.len(), tabs + 1, "workspace shortcuts resume when renaming ends");
    }

    #[test]
    fn query_and_group_renames_allow_case_changes_but_reject_hard_link_collisions() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("Report.sql");
        std::fs::write(&original, "SELECT 'keep this';").unwrap();
        let renamed = change_group(root.path(), Some((original, "report".into()))).unwrap();
        assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "SELECT 'keep this';");
        let names: Vec<_> = std::fs::read_dir(root.path()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert!(names.iter().any(|name| name == "report.sql"));
        assert!(!names.iter().any(|name| name == "Report.sql"));
        let alias = root.path().join("Alias.sql");
        std::fs::hard_link(&renamed, &alias).unwrap();
        assert!(change_group(root.path(), Some((renamed.clone(), "Alias".into()))).is_err());
        assert!(renamed.exists() && alias.exists());
        let group = root.path().join("Reports");
        std::fs::create_dir(&group).unwrap();
        std::fs::write(group.join("Nested.sql"), "SELECT 2;").unwrap();
        let renamed_group = change_group(root.path(), Some((group, "reports".into()))).unwrap();
        assert_eq!(std::fs::read_to_string(renamed_group.join("Nested.sql")).unwrap(), "SELECT 2;");
        assert!(std::fs::read_dir(root.path()).unwrap().any(|entry| entry.unwrap().file_name() == "reports"));
    }

    #[test]
    fn groups_create_uniquely_and_rename_without_overwriting_files() {
        let root = tempfile::tempdir().unwrap();
        let first = change_group(root.path(), None).unwrap();
        let second = change_group(root.path(), None).unwrap();
        assert_ne!(first, second);
        std::fs::write(first.join("report.sql"), "SELECT 42;").unwrap();
        assert!(change_group(root.path(), Some((first.clone(), "New group 2".into()))).is_err());
        assert!(change_group(root.path(), Some((first.clone(), "../outside".into()))).is_err());
        let renamed = change_group(root.path(), Some((first, "Reports".into()))).unwrap();
        assert_eq!(std::fs::read_to_string(renamed.join("report.sql")).unwrap(), "SELECT 42;");
        assert!(second.is_dir());
    }
    #[test]
    fn renamed_groups_retarget_open_and_closed_query_documents() {
        let ctx = egui::Context::default();
        let root = tempfile::tempdir().unwrap();
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        app.prefs.query_directory = Some(root.path().to_owned());
        let old = root.path().join("Old"); let new = root.path().join("Reports");
        app.tabs[0].source_path = Some(old.join("nested/report.sql"));
        let mut closed = QueryTab::new("SELECT 1;".into());
        closed.source_path = Some(old.join("closed.sql"));
        app.closed_tabs.push(closed);
        let (tx, rx) = mpsc::channel();
        app.query_library.group_operation = Some(rx);
        tx.send(Ok((root.path().to_owned(), Some(old), new.clone()))).unwrap();
        app.poll_query_groups();
        assert_eq!(app.tabs[0].source_path, Some(new.join("nested/report.sql")));
        assert_eq!(app.closed_tabs[0].source_path, Some(new.join("closed.sql")));
        assert!(app.query_library.group_operation.is_none());
    }
    #[test]
    fn single_click_selects_and_double_click_opens_saved_query() {
        let ctx = egui::Context::default();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Customer report.sql");
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        app.prefs.query_directory = Some(temp.path().to_owned());
        app.query_library.root = Some(temp.path().to_owned());
        app.query_library.refreshed = Some(Instant::now());
        app.query_library.files.push(QueryFile { path: path.clone(), name: "Customer report".into(), group: String::new() });
        let original_tab = app.tab;
        let mut saved = QueryTab::new("SELECT 42;".into());
        saved.source_path = Some(path.clone());
        app.tabs.push(saved);
        let saved_tab = app.tabs.len() - 1;
        let mut time = 0.0;
        let mut frame = |app: &mut Rustable, events| {
            time += 0.04;
            ctx.run(egui::RawInput { time: Some(time), screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 700.0))), events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.query_library_sidebar(ui));
            })
        };
        frame(&mut app, vec![]);
        let output = frame(&mut app, vec![]);
        fn label(shape: &egui::Shape) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == "Customer report" => Some(text.pos + text.galley.size() * 0.5),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(label),
                _ => None,
            }
        }
        let pos = output.shapes.iter().find_map(|shape| label(&shape.shape)).expect("saved query is visible");
        for click in 0..2 {
            for pressed in [true, false] {
                frame(&mut app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }]);
            }
            assert!(app.query_library.selected == Some(Selection::File(path.clone())));
            let painted = frame(&mut app, vec![]);
            assert!(painted.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == ctx.style().visuals.selection.bg_fill && rect.rect.width() > 400.0)), "selection must paint a persistent full-width background");
            assert_eq!(app.tab, if click == 0 { original_tab } else { saved_tab });
            assert!(app.pending.is_none(), "Opening an already loaded query must not execute SQL");
        }
    }
    #[test]
    fn scan_limits_cover_groups_and_ignored_files_and_accept_exact_boundary() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("Reports")).unwrap();
        std::fs::write(root.path().join("Reports/A.sql"), "SELECT 1").unwrap();
        std::fs::write(root.path().join("B.SQL"), "SELECT 2").unwrap();
        let (files, groups) = scan_with_limits(root.path(), 2, 1, 3).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(groups, vec!["Reports"]);
        assert!(scan_with_limits(root.path(), 1, 1, 3).unwrap_err().contains("SQL files"));
        assert!(scan_with_limits(root.path(), 2, 0, 3).unwrap_err().contains("groups"));
        std::fs::write(root.path().join("ignored.txt"), "not SQL").unwrap();
        assert!(scan_with_limits(root.path(), 2, 1, 3).unwrap_err().contains("directory entries"));
        assert_eq!(scan_with_limits(root.path(), 2, 1, 4).unwrap().0.len(), 2);
    }

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
