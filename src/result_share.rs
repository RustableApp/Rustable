//! Local query/result sharing. No queries are executed and no connection details are included.
use super::*;
#[path = "share_query.rs"]
mod share_query;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Format {
    #[default]
    Markdown,
    Plain,
    Json,
}
impl Format {
    fn label(self) -> &'static str {
        match self {
            Self::Markdown => "Markdown",
            Self::Plain => "Plain text",
            Self::Json => "JSON",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Scope {
    #[default]
    Selected,
    Page,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Options {
    format: Format,
    scope: Scope,
    query: bool,
    details: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            format: Format::Markdown,
            scope: Scope::Selected,
            query: true,
            details: false,
        }
    }
}
#[derive(Clone)]
struct Snapshot {
    grid: Arc<Grid>,
    selected: Vec<usize>,
    all_selected: bool,
    page: std::ops::Range<usize>,
    changes: Arc<Vec<Change>>,
    table: Option<String>,
    dialect: crate::sql::Dialect,
}
#[derive(Debug)]
struct Output {
    text: String,
    preview: String,
    shortened: bool,
    query_range: Option<std::ops::Range<usize>>,
}
struct Job {
    receive: mpsc::Receiver<Result<Output, String>>,
    cancel: Arc<AtomicBool>,
    options: Options,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
pub(super) struct Dialog {
    snapshot: Snapshot,
    options: Options,
    job: Option<Job>,
    output: Option<Output>,
    error: Option<String>,
    copy_when_ready: bool,
    quick: bool,
    copied_until: Option<Instant>,
}
impl Snapshot {
    fn count(&self, scope: Scope) -> usize {
        if scope == Scope::Selected && self.all_selected {
            return self.grid.row_count();
        }
        if scope == Scope::Selected && !self.selected.is_empty() {
            self.selected.len()
        } else {
            self.page.len()
        }
    }
    fn contains(&self, index: usize, scope: Scope) -> bool {
        if scope == Scope::Selected && self.all_selected {
            return true;
        }
        if scope == Scope::Selected && !self.selected.is_empty() {
            self.selected.binary_search(&index).is_ok()
        } else {
            self.page.contains(&index)
        }
    }
}
fn format(snapshot: &Snapshot, options: Options, cancel: &AtomicBool) -> Result<Output, String> {
    let source = snapshot.grid.query_source.as_ref();
    if options.query && source.is_none() {
        return Err("The source query is unavailable for this result. Turn off Include query to share its rows.".into());
    }
    if options.query && source.is_some_and(|s| s.sql.len() > 1024 * 1024) {
        return Err(
            "The query exceeds the 1 MiB sharing limit. Save the SQL as a file instead.".into(),
        );
    }
    if options.query
        && source.is_some_and(|s| db::result_store::row_bytes(&s.parameters) > 1024 * 1024)
    {
        return Err(
            "Query parameters exceed the 1 MiB sharing limit. Export the rows instead.".into(),
        );
    }
    let query = if options.query {
        source
            .map(|source| share_query::render(source, snapshot.dialect))
            .transpose()?
    } else {
        None
    };
    let mut rows = Vec::new();
    let mut bytes = 0usize;
    let last = if options.scope == Scope::Selected && snapshot.all_selected {
        snapshot.grid.row_count()
    } else if options.scope == Scope::Selected && !snapshot.selected.is_empty() {
        snapshot
            .selected
            .last()
            .copied()
            .unwrap_or(0)
            .saturating_add(1)
    } else {
        snapshot.page.end
    };
    snapshot.grid.visit_rows(cancel, |index, row| {
        if index >= last {
            return Ok(false);
        }
        if snapshot.contains(index, options.scope) {
            bytes = bytes.saturating_add(db::result_store::row_bytes(row));
            #[cfg(target_arch = "wasm32")]
            if bytes > 64 * 1024 || rows.len() >= 256 {
                return Err("Browser sharing supports 256 rows or 64 KiB. Select fewer rows or use the desktop app.".into());
            }
            if bytes > 4 * 1024 * 1024 {
                return Err(
                    "Sharing is limited to 4 MiB of row data. Select fewer rows or use Export."
                        .into(),
                );
            }
            rows.push(row.to_vec());
        }
        Ok(true)
    })?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }
    let row_format = match options.format {
        Format::Markdown => row_copy::Format::Markdown,
        Format::Plain => row_copy::Format::CsvHeaders,
        Format::Json => row_copy::Format::Json,
    };
    let data = if options.format == Format::Json {
        String::new()
    } else if options.format == Format::Plain {
        // Plain text is a header-bearing tab-separated table, with quoted multiline values.
        let values = row_copy::formatted(
            &snapshot.grid.columns,
            rows.iter().map(Vec::as_slice),
            &snapshot.changes,
            snapshot.table.as_deref(),
            None,
            row_copy::Format::Plain,
            snapshot.dialect,
        )?;
        format!(
            "{}\n{}",
            row_copy::format_rows(
                &snapshot.grid.columns,
                std::iter::empty(),
                &[],
                None,
                '\t',
                true
            )?
            .trim_end(),
            values
        )
    } else {
        row_copy::formatted(
            &snapshot.grid.columns,
            rows.iter().map(Vec::as_slice),
            &snapshot.changes,
            snapshot.table.as_deref(),
            None,
            row_format,
            snapshot.dialect,
        )?
    };
    let edited = snapshot.changes.iter().any(|change| matches!(change,
        Change::Cell {table, original, ..} if snapshot.table.as_ref() == Some(table) && rows.iter().any(|row| row == original)));
    let summary = format!(
        "{} shared rows · {} retained rows{}{}",
        rows.len(),
        snapshot.grid.row_count(),
        if snapshot.grid.truncated {
            " · result truncated"
        } else {
            ""
        },
        if edited {
            " · includes unsaved cell edits"
        } else {
            ""
        }
    );
    let details = format!(
        "Elapsed: {:.1} ms",
        snapshot.grid.elapsed.as_secs_f64() * 1000.0
    );
    let text = if options.format == Format::Json {
        let mut bundle = serde_json::Map::new();
        if let Some(query) = &query {
            bundle.insert("query".into(), serde_json::json!(query));
        }
        bundle.insert("columns".into(), serde_json::json!(snapshot.grid.columns));
        let json_rows: Vec<Vec<serde_json::Value>> = rows
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(index, original)| {
                        let value = snapshot
                            .changes
                            .iter()
                            .find_map(|change| match change {
                                Change::Cell {
                                    table,
                                    column,
                                    original: source,
                                    value,
                                    ..
                                } if snapshot.table.as_ref() == Some(table)
                                    && snapshot.grid.columns.get(index) == Some(column)
                                    && source == row =>
                                {
                                    Some(value)
                                }
                                _ => None,
                            })
                            .unwrap_or(original);
                        match value {
                            Value::Null => serde_json::Value::Null,
                            Value::Integer(n) => serde_json::json!(n),
                            Value::Real(n) if n.is_finite() => serde_json::json!(n),
                            Value::Real(n) => {
                                serde_json::json!({"non_finite_number": n.to_string()})
                            }
                            Value::Text(text) => serde_json::json!(text),
                            Value::Blob(bytes) => serde_json::json!(bytes),
                        }
                    })
                    .collect()
            })
            .collect();
        bundle.insert("rows".into(), serde_json::json!(json_rows));
        bundle.insert("scope".into(), serde_json::json!({"shared_rows":rows.len(),"retained_rows":snapshot.grid.row_count(),"truncated":snapshot.grid.truncated,"unsaved_edits":edited}));
        if options.details {
            bundle.insert(
                "elapsed_ms".into(),
                serde_json::json!(snapshot.grid.elapsed.as_secs_f64() * 1000.0),
            );
        }
        serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())?
    } else {
        let mut text = String::new();
        if let Some(query) = &query {
            if options.format == Format::Markdown {
                // A longer fence keeps SQL containing backticks inside the code block.
                let longest = query.split(|c| c != '`').map(str::len).max().unwrap_or(0);
                let fence = "`".repeat(longest.saturating_add(1).max(3));
                text.push_str(&format!("{fence}sql\n{}\n{fence}\n\n", query.trim()));
            } else {
                text.push_str(query.trim());
                text.push_str("\n\n");
            }
        }
        text.push_str(&data);
        text.push_str("\n\n");
        text.push_str(&summary);
        if options.details {
            text.push('\n');
            text.push_str(&details);
        }
        text
    };
    if text.len() > 32 * 1024 * 1024 {
        return Err("Share exceeds the 32 MiB clipboard limit. Select fewer rows.".into());
    }
    let mut end = text.len().min(12 * 1024);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let shortened = end < text.len();
    let preview = text[..end].to_owned();
    let query_range = query
        .as_ref()
        .filter(|_| options.query && options.format != Format::Json)
        .and_then(|query| {
            let sql = query.trim();
            let start = preview.find(sql)?;
            (start + sql.len() <= preview.len()).then_some(start..start + sql.len())
        });
    Ok(Output {
        query_range,
        text,
        preview,
        shortened,
    })
}
impl Dialog {
    fn options_ui(&mut self, ui: &mut egui::Ui) {
        ui.weak("Format");
        egui::ComboBox::from_id_salt("share-format")
            .width(170.0)
            .selected_text(self.options.format.label())
            .show_ui(ui, |ui| {
                for f in [Format::Markdown, Format::Plain, Format::Json] {
                    ui.selectable_value(&mut self.options.format, f, f.label());
                }
            });
        ui.add_space(12.0);
        ui.weak("Rows");
        let selected = if self.snapshot.all_selected {
            self.snapshot.grid.row_count()
        } else {
            self.snapshot.selected.len()
        };
        let page = self.snapshot.page.len();
        egui::ComboBox::from_id_salt("share-scope")
            .width(170.0)
            .selected_text(if self.options.scope == Scope::Selected && selected > 0 {
                format!("Selected ({selected})")
            } else {
                format!("Current page ({page})")
            })
            .show_ui(ui, |ui| {
                ui.add_enabled_ui(selected > 0, |ui| {
                    ui.selectable_value(
                        &mut self.options.scope,
                        Scope::Selected,
                        format!("Selected rows ({selected})"),
                    );
                });
                ui.selectable_value(
                    &mut self.options.scope,
                    Scope::Page,
                    format!("Current page ({page})"),
                );
            });
        ui.add_space(16.0);
        ui.checkbox(&mut self.options.query, "Include query");
        ui.checkbox(&mut self.options.details, "Execution details");
        ui.add_space(16.0);
        ui.add(egui::Label::new(RichText::new("Uses the executed query. Selection and truncation are labeled in the copied text.").weak()).wrap());
    }
    fn preview_ui(&self, ui: &mut egui::Ui, width: f32, height: f32) {
        ui.set_width(width);
        ui.strong("Clipboard preview");
        ui.add_space(8.0);
        egui::Frame::new()
            .fill(ui.visuals().extreme_bg_color)
            .inner_margin(10.0)
            .corner_radius(5.0)
            .show(ui, |ui| {
                ui.set_width((width - 20.0).max(100.0));
                egui::ScrollArea::both()
                    .id_salt("share-preview")
                    .min_scrolled_height(height)
                    .max_height(height)
                    .show(ui, |ui| {
                        if let Some(error) = &self.error {
                            ui.colored_label(ui.visuals().error_fg_color, error);
                        } else if let Some(output) = &self.output {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            if let Some(range) = &output.query_range {
                                let before = output.preview[..range.start].trim_end_matches('\n');
                                if !before.is_empty() {
                                    ui.add(
                                        egui::Label::new(RichText::new(before).monospace())
                                            .selectable(true),
                                    );
                                }
                                let galley = crate::editor::layout(
                                    ui,
                                    &(&output.preview[range.clone()]),
                                    width - 20.0,
                                );
                                ui.add(egui::Label::new(galley).selectable(true));
                                if self.options.format == Format::Plain {
                                    ui.add_space(12.0);
                                }
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(
                                            output.preview[range.end..].trim_start_matches('\n'),
                                        )
                                        .monospace(),
                                    )
                                    .selectable(true),
                                );
                            } else {
                                ui.add(
                                    egui::Label::new(RichText::new(&output.preview).monospace())
                                        .selectable(true),
                                );
                            }
                        } else {
                            ui.weak("Preparing preview…");
                        }
                    });
            });
        if self.output.as_ref().is_some_and(|o| o.shortened) {
            ui.weak("Preview shortened; copy includes all shared rows.");
        }
    }
    fn start(&mut self, ctx: &egui::Context) {
        self.output = None;
        self.error = None;
        self.copied_until = None;
        if self.job.is_some() {
            return;
        }
        let snapshot = self.snapshot.clone();
        let options = self.options;
        let cancel = Arc::new(AtomicBool::new(false));
        let (send, receive) = mpsc::channel();
        self.job = Some(Job {
            receive,
            cancel: cancel.clone(),
            options,
        });
        let ctx = ctx.clone();
        let task = move || {
            let result = format(&snapshot, options, &cancel);
            if send.send(result).is_ok() {
                ctx.request_repaint();
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(task);
        #[cfg(target_arch = "wasm32")]
        task();
    }
    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.job
            && let Ok(result) = job.receive.try_recv()
        {
            let current = job.options == self.options;
            self.job = None;
            if current {
                match result {
                    Ok(output) => self.output = Some(output),
                    Err(error) => self.error = Some(error),
                }
            } else {
                self.start(ctx);
            }
        }
    }
    fn copy(&mut self, ctx: &egui::Context) {
        if let Some(output) = &self.output {
            ctx.copy_text(output.text.clone());
            self.copied_until = Some(Instant::now() + Duration::from_secs(2));
            self.copy_when_ready = false;
            ctx.request_repaint_after(Duration::from_secs(2));
        } else if self.error.is_none() {
            self.copy_when_ready = true;
        }
    }
}
pub(super) fn menu(ui: &mut egui::Ui) -> Option<bool> {
    let copied = ui
        .ctx()
        .data(|d| d.get_temp::<Instant>(egui::Id::new("share-copied")))
        .is_some_and(|until| Instant::now() < until);
    if ui
        .button(if copied {
            "✓ Copied"
        } else {
            "Copy Query + Rows"
        })
        .clicked()
    {
        return Some(true);
    }
    if ui
        .add(crate::icons::shortcut_menu_item(
            "Share Results…",
            true,
            "J",
        ))
        .on_hover_text("Choose format, rows and query inclusion")
        .clicked()
    {
        ui.close();
        return Some(false);
    }
    None
}

impl Rustable {
    pub(super) fn open_result_share(
        &mut self,
        ctx: &egui::Context,
        quick: bool,
        clicked: Option<usize>,
    ) {
        if !matches!(self.view, View::Data | View::Query) {
            return;
        }
        let displayed = self.grid();
        if displayed.fetching {
            self.error = Some("Wait for fetching to finish before sharing the result.".into());
            return;
        }
        if displayed.columns.is_empty() {
            return;
        }
        if self.row_selection.rows.is_empty()
            && self
                .row_selection
                .focused
                .is_some_and(|index| index >= displayed.rows.len())
        {
            self.error =
                Some("New unsaved rows are not query results. Save them before sharing.".into());
            return;
        }
        let stored = self.view == View::Query && displayed.is_stored();
        let (grid, offset) = if stored {
            let tab = &self.tabs[self.tab];
            (tab.grid.clone(), tab.result_pages.share_offset())
        } else {
            (displayed.clone(), 0)
        };
        let mut selected = if let Some(index) = clicked {
            self.row_selection.copy_indices(index)
        } else {
            let mut rows: Vec<_> = self.row_selection.rows.iter().copied().collect();
            if rows.is_empty()
                && let Some(index) = self.selected
            {
                rows.push(index);
            }
            rows
        };
        selected.retain(|index| *index < displayed.rows.len());
        for index in &mut selected {
            *index += offset;
        }
        selected.sort_unstable();
        selected.dedup();
        let table = if self.view == View::Data {
            Some(self.browse.table.clone())
        } else {
            self.query_edit_target().map(|target| target.table.clone())
        };
        let mut changes = Vec::new();
        let mut change_bytes = 0usize;
        for change in &self.changes {
            if let Change::Cell {
                table: target,
                original,
                original_columns,
                value,
                ..
            } = change
                && table.as_ref() == Some(target)
                && original_columns
                    .as_ref()
                    .is_none_or(|names| names == &displayed.columns)
            {
                change_bytes = change_bytes
                    .saturating_add(db::result_store::row_bytes(original))
                    .saturating_add(db::result_store::row_bytes(std::slice::from_ref(value)));
                if change_bytes > 4 * 1024 * 1024 {
                    self.error = Some(
                        "Too many staged cell values to share. Save or discard edits first.".into(),
                    );
                    return;
                }
                changes.push(change.clone());
            }
        }
        let snapshot = Snapshot {
            grid,
            selected,
            all_selected: stored && self.tabs[self.tab].result_pages.all_selected,
            page: if stored {
                self.tabs[self.tab]
                    .result_pages
                    .share_page_range(&self.tabs[self.tab].grid)
            } else {
                0..displayed.rows.len()
            },
            changes: Arc::new(changes),
            table,
            dialect: self
                .session_dialect
                .unwrap_or_else(|| crate::sql::Dialect::for_engine(&self.engine)),
        };
        let mut options = self.prefs.result_share;
        // This explicit quick action always includes the query; the dialog remembers its own checkbox.
        if quick {
            options.query = true;
        }
        let mut dialog = Dialog {
            snapshot,
            options,
            job: None,
            output: None,
            error: None,
            quick,
            copy_when_ready: quick,
            copied_until: None,
        };
        dialog.start(ctx);
        self.result_share = Some(dialog);
    }
    pub(super) fn result_share_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.result_share else {
            return;
        };
        dialog.poll(ctx);
        if dialog.copy_when_ready && dialog.output.is_some() {
            dialog.copy(ctx);
        }
        if dialog.quick {
            if dialog.copied_until.is_some() {
                ctx.data_mut(|d| {
                    d.insert_temp(
                        egui::Id::new("share-copied"),
                        Instant::now() + Duration::from_secs(2),
                    )
                });
                self.status = "Copied query + rows".into();
                self.result_share = None;
            } else if let Some(error) = &dialog.error {
                self.error = Some(error.clone());
                self.result_share = None;
            }
            return;
        }
        let mut close = false;
        let before = dialog.options;
        let copy_shortcut = ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::Enter,
            );
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
        });
        let modal =
            egui::Modal::new(egui::Id::new((ctx.viewport_id(), "share-results"))).show(ctx, |ui| {
                let width = (ctx.content_rect().width() - 48.0).clamp(280.0, 700.0);
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.strong("Share results");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui.button("Close").clicked();
                    });
                });
                ui.add_space(10.0);
                let height = (ctx.content_rect().height() - 200.0).clamp(160.0, 340.0);
                if width >= 560.0 {
                    ui.horizontal_top(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(185.0, height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                dialog.options_ui(ui);
                            },
                        );
                        let (divider, _) = ui.allocate_exact_size(
                            egui::vec2(1.0, height + 36.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().line_segment(
                            [divider.left_top(), divider.left_bottom()],
                            ui.visuals().widgets.noninteractive.bg_stroke,
                        );
                        ui.vertical(|ui| {
                            if before != dialog.options {
                                dialog.copy_when_ready = false;
                                dialog.start(ctx);
                            }
                            dialog.preview_ui(ui, width - 210.0, height);
                        });
                    });
                } else {
                    dialog.options_ui(ui);
                    if before != dialog.options {
                        dialog.copy_when_ready = false;
                        dialog.start(ctx);
                    }
                    dialog.preview_ui(ui, width, height);
                }
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    ui.weak(format!(
                        "{} rows · uses the executed query",
                        dialog.snapshot.count(dialog.options.scope)
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let copied = dialog
                            .copied_until
                            .is_some_and(|until| Instant::now() < until);
                        if ui
                            .add_enabled(
                                dialog.error.is_none(),
                                crate::icons::shortcut_button(
                                    if copied {
                                        "Copied"
                                    } else if dialog.copy_when_ready {
                                        "Copy when ready…"
                                    } else {
                                        "Copy"
                                    },
                                    false,
                                    "Enter",
                                    Some(BLUE),
                                ),
                            )
                            .on_hover_text(format!(
                                "Copy · {}",
                                crate::icons::shortcut(ctx, false, "Enter")
                            ))
                            .clicked()
                            || copy_shortcut
                        {
                            dialog.copy(ctx);
                        }
                    });
                });
            });
        if before != dialog.options {
            self.prefs.result_share = dialog.options;
            self.shared_preferences_dirty = true;
        }
        if close || modal.should_close() {
            self.result_share = None;
        }
    }
}

#[cfg(any(test, feature = "qa"))]
fn fixture(ctx: &egui::Context) -> Rustable {
    let mut app = Rustable::with_optional_connection(ctx, Preferences::default(), None, None);
    app.connections = false;
    app.pending = None;
    app.engine = "PostgreSQL".into();
    app.view = View::Query;
    app.inspector = false;
    let grid = Arc::new(Grid {
        columns: vec!["id".into(), "name".into(), "status".into()],
        rows: vec![
            vec![
                Value::Integer(101),
                Value::Text("Acme".into()),
                Value::Text("active".into()),
            ],
            vec![
                Value::Integer(108),
                Value::Text("Orbit".into()),
                Value::Text("active".into()),
            ],
            vec![
                Value::Integer(112),
                Value::Text("Northstar".into()),
                Value::Text("active".into()),
            ],
        ],
        query_source: Some(Arc::new(db::QuerySource {
            sql: "SELECT id, name, status\nFROM companies\nWHERE status = $1\nORDER BY id;".into(),
            parameters: vec![Value::Text("active".into())],
        })),
        elapsed: Duration::from_millis(12),
        ..Default::default()
    });
    app.tabs[0].sql = grid.query_source.as_ref().unwrap().sql.clone();
    app.tabs[0].grid = grid.clone();
    app.tabs[0].results = vec![grid];
    app.row_selection.rows.extend([0, 2]);
    app
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Snapshot {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx);
        app.open_result_share(&ctx, false, None);
        app.result_share.take().unwrap().snapshot
    }
    #[test]
    fn formats_query_selection_and_no_query_json() {
        let source = snapshot();
        let cancel = AtomicBool::new(false);
        let output = format(&source, Options::default(), &cancel).unwrap();
        assert!(output.text.contains("```sql\nSELECT"));
        assert!(output.text.contains("Northstar"));
        assert!(!output.text.contains("Orbit"));
        assert!(output.text.contains("2 shared rows · 3 retained rows"));
        let options = Options {
            format: Format::Json,
            query: false,
            scope: Scope::Page,
            details: true,
        };
        let output = format(&source, options, &cancel).unwrap();
        let value: serde_json::Value = serde_json::from_str(&output.text).unwrap();
        assert!(value.get("query").is_none());
        assert_eq!(value["rows"].as_array().unwrap().len(), 3);
        assert_eq!(value["elapsed_ms"], 12.0);
        let output = format(
            &source,
            Options {
                format: Format::Plain,
                ..Options::default()
            },
            &cancel,
        )
        .unwrap();
        assert!(output.text.contains("id\tname\tstatus\n101\tAcme\tactive"));
    }
    #[test]
    fn parameters_fences_truncation_and_empty_results_are_explicit() {
        let mut source = snapshot();
        let grid = Arc::make_mut(&mut source.grid);
        grid.query_source = Some(Arc::new(db::QuerySource {
            sql: "SELECT '```', $1".into(),
            parameters: vec![Value::Text("雪'\n".into())],
        }));
        grid.truncated = true;
        let output = format(&source, Options::default(), &AtomicBool::new(false)).unwrap();
        assert!(output.text.starts_with("````sql"));
        assert!(output.text.contains("E'雪''\n'"));
        assert!(!output.text.contains("$1"));
        assert!(output.query_range.is_some());
        for share_format in [Format::Plain, Format::Json] {
            let output = format(&source, Options { format: share_format, ..Options::default() }, &AtomicBool::new(false)).unwrap();
            assert!(!output.text.contains("$1"));
            if share_format == Format::Json {
                let json: serde_json::Value = serde_json::from_str(&output.text).unwrap();
                assert_eq!(json["query"], "SELECT '```', E'雪''\n'");
            }
        }
        assert!(output.text.contains("result truncated"));
        Arc::make_mut(&mut source.grid).rows.clear();
        source.selected.clear();
        source.page = 0..0;
        let output = format(&source, Options::default(), &AtomicBool::new(false)).unwrap();
        assert!(output.text.contains("0 shared rows"));
        assert!(output.text.contains("| id | name | status |"));
    }
    #[test]
    fn shares_complete_stored_values_and_bounds_memory() {
        let mut source = snapshot();
        let mut collector = db::result_store::Collector::new(0);
        collector
            .push(vec![Value::Text("complete".repeat(1000))])
            .unwrap();
        let grid = Arc::make_mut(&mut source.grid);
        grid.columns = vec!["value".into()];
        grid.rows.clear();
        collector.finish(grid).unwrap();
        source.selected = vec![0];
        source.page = 0..1;
        let output = format(&source, Options::default(), &AtomicBool::new(false)).unwrap();
        assert!(output.text.contains(&"complete".repeat(1000)));
        let grid = Arc::make_mut(&mut source.grid);
        grid.stored = None;
        grid.rows = vec![vec![Value::Text("x".repeat(5 * 1024 * 1024))]];
        assert!(
            format(&source, Options::default(), &AtomicBool::new(false))
                .unwrap_err()
                .contains("4 MiB")
        );
    }
    #[test]
    fn original_query_survives_editor_changes_and_dialog_owns_run_shortcut() {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx);
        app.tabs[0].sql = "DELETE FROM companies".into();
        let event = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let _ = ctx.run(
            egui::RawInput {
                events: vec![event(
                    egui::Key::J,
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                )],
                ..Default::default()
            },
            |ctx| app.workspace_shortcuts(ctx),
        );
        assert!(app.result_share.is_some());
        let dialog = app.result_share.as_mut().unwrap();
        dialog.job = None;
        dialog.output =
            Some(format(&dialog.snapshot, dialog.options, &AtomicBool::new(false)).unwrap());
        let output = ctx.run(
            egui::RawInput {
                events: vec![event(egui::Key::Enter, egui::Modifiers::COMMAND)],
                ..Default::default()
            },
            |ctx| {
                app.workspace_shortcuts(ctx);
                app.result_share_window(ctx);
            },
        );
        assert!(app.pending.is_none());
        assert!(output.platform_output.commands.iter().any(|command| matches!(command,egui::OutputCommand::CopyText(text) if text.contains("SELECT id") && !text.contains("DELETE"))));
        let _ = ctx.run(
            egui::RawInput {
                events: vec![event(egui::Key::Escape, egui::Modifiers::NONE)],
                ..Default::default()
            },
            |ctx| app.result_share_window(ctx),
        );
        assert!(app.result_share.is_none());
    }
    #[test]
    fn remembers_options_and_quick_copy_forces_query_without_changing_preferences() {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx);
        app.prefs.result_share = Options {
            format: Format::Plain,
            scope: Scope::Page,
            query: false,
            details: true,
        };
        let json = serde_json::to_string(&app.prefs).unwrap();
        let restored: Preferences = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.result_share, app.prefs.result_share);
        app.open_result_share(&ctx, true, None);
        let dialog = app.result_share.as_ref().unwrap();
        assert!(dialog.options.query);
        assert_eq!(dialog.options.scope, Scope::Page);
        assert!(!app.prefs.result_share.query);
    }
    #[test]
    fn obsolete_previews_are_discarded_and_early_copy_waits_for_current_options() {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx);
        app.open_result_share(&ctx, false, None);
        let dialog = app.result_share.as_mut().unwrap();
        dialog.job = None;
        let old_options = dialog.options;
        let stale = format(&dialog.snapshot, old_options, &AtomicBool::new(false)).unwrap();
        let (send, receive) = mpsc::channel();
        dialog.job = Some(Job {
            receive,
            cancel: Arc::new(AtomicBool::new(false)),
            options: old_options,
        });
        dialog.options.query = false;
        dialog.options.format = Format::Json;
        send.send(Ok(stale)).unwrap();
        dialog.copy(&ctx);
        assert!(dialog.copy_when_ready);
        dialog.poll(&ctx);
        assert!(dialog.output.is_none());
        for _ in 0..100 {
            dialog.poll(&ctx);
            if dialog.output.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let output = dialog.output.as_ref().expect("current preview completed");
        let json: serde_json::Value = serde_json::from_str(&output.text).unwrap();
        assert!(json.get("query").is_none());
        assert!(dialog.copy_when_ready);
    }

    #[test]
    fn json_preserves_duplicate_column_names_and_special_values() {
        let mut source = snapshot();
        let grid = Arc::make_mut(&mut source.grid);
        grid.columns = vec!["id".into(), "id".into(), "number".into(), "bytes".into()];
        grid.rows = vec![vec![
            Value::Integer(1),
            Value::Text("NULL".into()),
            Value::Real(f64::INFINITY),
            Value::Blob(vec![0, 255]),
        ]];
        source.selected = vec![0];
        source.page = 0..1;
        let output = format(
            &source,
            Options {
                format: Format::Json,
                ..Options::default()
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&output.text).unwrap();
        assert_eq!(
            value["columns"],
            serde_json::json!(["id", "id", "number", "bytes"])
        );
        assert_eq!(
            value["rows"][0],
            serde_json::json!([1,"NULL",{"non_finite_number":"inf"},[0,255]])
        );
    }

    #[test]
    fn edited_values_are_disclosed() {
        let mut source = snapshot();
        source.table = Some("companies".into());
        source.changes = Arc::new(vec![Change::Cell {
            table: "companies".into(),
            column: "name".into(),
            original: source.grid.rows[0].clone(),
            original_columns: Some(source.grid.columns.clone()),
            value: Value::Text("Changed".into()),
        }]);
        let output = format(&source, Options::default(), &AtomicBool::new(false)).unwrap();
        assert!(output.text.contains("Changed"));
        assert!(output.text.contains("includes unsaved cell edits"));
    }
}

#[cfg(feature = "qa")]
impl Rustable {
    pub fn run_share_preview() -> eframe::Result {
        struct Preview {
            app: Rustable,
            path: PathBuf,
            frames: usize,
            started: Instant,
            saved: bool,
            menu: bool,
        }
        impl eframe::App for Preview {
            fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
                if self.menu && self.frames == 4 {
                    let pos = egui::pos2(150.0, ctx.content_rect().height() * 0.46);
                    input.events.push(egui::Event::PointerMoved(pos));
                    for pressed in [true, false] {
                        input.events.push(egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Secondary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                }
            }

            fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
                if let Some(image) = ctx.input(|i| {
                    i.events.iter().find_map(|e| {
                        if let egui::Event::Screenshot { image, .. } = e {
                            Some(image.clone())
                        } else {
                            None
                        }
                    })
                }) {
                    let mut data =
                        format!("P6\n{} {}\n255\n", image.width(), image.height()).into_bytes();
                    for p in &image.pixels {
                        data.extend_from_slice(&p.to_array()[..3]);
                    }
                    let path = self.path.clone();
                    std::thread::spawn(move || std::fs::write(path, data).unwrap())
                        .join()
                        .unwrap();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                self.app.workspace(ctx);
                self.app.result_share_window(ctx);
                self.frames += 1;
                if !self.saved
                    && self.frames > 8
                    && self.started.elapsed() > Duration::from_millis(700)
                    && (if self.menu {
                        egui::Popup::is_any_open(ctx)
                    } else {
                        self.app
                            .result_share
                            .as_ref()
                            .is_some_and(|d| d.output.is_some())
                    })
                {
                    self.saved = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
                assert!(self.frames < 120, "Share preview did not settle");
                ctx.request_repaint_after(Duration::from_millis(40));
            }
        }
        let path = PathBuf::from(std::env::var_os("RUSTABLE_SHARE_CAPTURE").unwrap());
        let menu = std::env::var_os("RUSTABLE_SHARE_MENU").is_some();
        let light = std::env::var_os("RUSTABLE_SHARE_LIGHT").is_some();
        let width = std::env::var("RUSTABLE_SHARE_WIDTH")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(1120.0);
        eframe::run_native(
            "Share results preview",
            eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default().with_inner_size([width, 720.0]),
                persist_window: false,
                ..Default::default()
            },
            Box::new(move |cc| {
                crate::fonts::install(&cc.egui_ctx);
                let mut app = fixture(&cc.egui_ctx);
                app.prefs.appearance = if light {
                    Appearance::Light
                } else {
                    Appearance::Dark
                };
                apply_theme(&cc.egui_ctx, app.prefs.appearance);
                if !menu {
                    app.open_result_share(&cc.egui_ctx, false, None);
                }
                Ok(Box::new(Preview {
                    app,
                    path,
                    frames: 0,
                    started: Instant::now(),
                    saved: false,
                    menu,
                }))
            }),
        )
    }
}

#[cfg(test)]
mod performance_tests {
    use super::*;
    #[test]
    #[ignore = "release-mode UI benchmark"]
    fn share_dialog_frame_benchmark() {
        for width in [500.0, 1120.0] {
            let ctx = egui::Context::default();
            crate::fonts::install(&ctx);
            let mut app = fixture(&ctx);
            app.open_result_share(&ctx, false, None);
            let dialog = app.result_share.as_mut().unwrap();
            dialog.job = None;
            dialog.output =
                Some(format(&dialog.snapshot, dialog.options, &AtomicBool::new(false)).unwrap());
            let mut samples = Vec::new();
            for frame in 0..220 {
                let start = std::time::Instant::now();
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 720.0),
                        )),
                        time: Some(frame as f64 / 120.0 + if frame >= 100 { 30.0 } else { 0.0 }),
                        ..Default::default()
                    },
                    |ctx| app.result_share_window(ctx),
                );
                let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
                if frame >= 20 {
                    samples.push(start.elapsed().as_secs_f64() * 1000.0);
                }
            }
            samples.sort_by(f64::total_cmp);
            crate::ui_benchmark::report(&format!("share/dialog/{width}"), &samples);
        }
    }
}
