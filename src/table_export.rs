//! Sidebar selection and export review. No database or file I/O runs in a frame.
use super::*;
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct Selection {
    pub names: BTreeSet<String>,
    anchor: Option<String>,
}
impl Selection {
    pub fn click<'a>(
        &mut self,
        name: &str,
        command: bool,
        shift: bool,
        visible: impl FnOnce() -> Vec<&'a str>,
    ) {
        if shift {
            let visible = visible();
            let end = visible.iter().position(|&value| value == name);
            let start = self
                .anchor
                .as_ref()
                .and_then(|anchor| visible.iter().position(|&value| value == anchor));
            if !command {
                self.names.clear();
            }
            if let (Some(start), Some(end)) = (start, end) {
                self.names.extend(
                    visible[start.min(end)..=start.max(end)]
                        .iter()
                        .map(|name| (*name).to_owned()),
                );
            } else {
                self.names.insert(name.to_owned());
            }
        } else {
            if !command {
                self.names.clear();
            }
            if !command || !self.names.remove(name) {
                self.names.insert(name.to_owned());
            }
            self.anchor = Some(name.to_owned());
        }
    }
    pub fn context(&mut self, name: &str) {
        if !self.names.contains(name) {
            self.names.clear();
            self.names.insert(name.to_owned());
            self.anchor = Some(name.to_owned());
        }
    }
}

struct Entry {
    name: String,
    label: String,
    selected: bool,
    filename: String,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Contents {
    Data,
    Schema,
    SchemaAndData,
}

pub(super) struct Dialog {
    contents: Contents,
    entries: Vec<Entry>,
    format: db::ExportFormat,
    csv_options: db::csv_options::Options,
    count: usize,
    preview: String,
    choosing: bool,
    #[cfg(not(target_arch = "wasm32"))]
    epoch: u64,
}
impl Dialog {
    fn effective_format(&self) -> db::ExportFormat {
        if self.format.is_csv() {
            db::ExportFormat::CsvCustom(self.csv_options)
        } else {
            self.format
        }
    }
    fn single_file(&self) -> bool {
        self.format == db::ExportFormat::Sql
            || (self.count == 1 && self.contents != Contents::SchemaAndData)
    }
    fn update_files(&mut self) {
        self.count = 0;
        self.preview.clear();
        for entry in &mut self.entries {
            entry.filename = if entry.selected {
                let data = db::table_export::filename(&entry.label, self.count, self.format);
                let name = if self.contents == Contents::Data {
                    data
                } else {
                    let schema = PathBuf::from(&data)
                        .with_extension("sql")
                        .to_string_lossy()
                        .into_owned();
                    if self.contents == Contents::Schema || self.format == db::ExportFormat::Sql {
                        schema
                    } else {
                        format!("schema.sql + {data}")
                    }
                };
                self.count += 1;
                if self.count == 1 {
                    self.preview.clone_from(&name);
                }
                name
            } else {
                String::new()
            };
        }
        if self.format == db::ExportFormat::Sql && self.count > 1 {
            self.preview = "tables.sql".into();
        }
    }
}

fn export_choice<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut T,
    options: &[(T, &str)],
) {
    let label = options
        .iter()
        .find(|(candidate, _)| candidate == value)
        .map(|(_, label)| *label)
        .unwrap_or_default();
    egui::ComboBox::from_id_salt(id)
        .selected_text(label)
        .width(150.0)
        .show_ui(ui, |ui| {
            for (choice, label) in options {
                ui.selectable_value(value, *choice, *label);
            }
        });
}

impl Rustable {
    pub(super) fn begin_sidebar_export(&mut self) {
        if self.pending.is_some() || !self.supports_table_export() {
            return;
        }
        self.sidebar_export_epoch = self.sidebar_export_epoch.wrapping_add(1);
        let mut dialog = Dialog {
            contents: Contents::Data,
            entries: self
                .objects
                .iter()
                .filter(|o| matches!(o.kind.as_str(), "table" | "view"))
                .map(|object| Entry {
                    name: object.name.clone(),
                    label: object.display_name().to_owned(),
                    selected: self.sidebar_selection.names.contains(&object.name),
                    filename: String::new(),
                })
                .collect(),
            count: 0,
            preview: String::new(),
            format: db::ExportFormat::Csv,
            csv_options: Default::default(),
            choosing: false,
            #[cfg(not(target_arch = "wasm32"))]
            epoch: self.sidebar_export_epoch,
        };
        dialog.update_files();
        self.sidebar_export = Some(dialog);
    }

    pub(super) fn sidebar_export_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.sidebar_export else {
            return;
        };
        let mut close = false;
        let mut choose = false;
        let mut changed = false;
        let modal = egui::Modal::new(egui::Id::new((ctx.viewport_id(), "sidebar-export"))).show(
            ctx,
            |ui| {
                let width = (ctx.content_rect().width() - 48.0).clamp(480.0, 680.0);
                let panel_height = (ctx.content_rect().height() - 180.0).clamp(240.0, 370.0);
                ui.set_width(width);
                ui.strong("Export tables");
                ui.add_space(10.0);
                ui.horizontal_top(|ui| {
                    let left_width = (width * 0.40).floor();
                    ui.allocate_ui_with_layout(egui::vec2(left_width, panel_height + 32.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(left_width);
                        ui.add_enabled_ui(!dialog.choosing, |ui| {
                            crate::controls::table_surface(ui, |ui| {
                                crate::controls::table_header_background(ui, 26.0);
                                TableBuilder::new(ui)
                                    .id_salt("export-tables-list")
                                    .striped(true)
                                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                                    .column(TableColumn::remainder().clip(true))
                                    .auto_shrink([false, false])
                                    .min_scrolled_height(panel_height + 24.0)
                                    .max_scroll_height(panel_height + 24.0)
                                    .header(26.0, |mut header| { header.col(|ui| { ui.strong("Items"); }); })
                                    .body(|body| {
                                        body.rows(24.0, dialog.entries.len(), |mut row| {
                                            let entry = &mut dialog.entries[row.index()];
                                            row.col(|ui| {
                                                changed |= ui.add(egui::Checkbox::new(&mut entry.selected, &entry.label))
                                                    .on_hover_text(&entry.name).changed();
                                            });
                                        });
                                    });
                                if dialog.entries.is_empty() { ui.weak("No tables in this schema."); }
                            });
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if ui.small_button("All").clicked() { for entry in &mut dialog.entries { entry.selected = true; } changed = true; }
                                if ui.small_button("None").clicked() { for entry in &mut dialog.entries { entry.selected = false; } changed = true; }
                                ui.weak(format!("{} selected", dialog.count));
                            });
                        });
                    });
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.set_width((width - left_width - 16.0).max(260.0));
                        ui.add_enabled_ui(!dialog.choosing, |ui| {
                            ui.horizontal(|ui| {
                                for (format, label) in [(db::ExportFormat::Csv,"CSV"),(db::ExportFormat::Json,"JSON"),(db::ExportFormat::Sql,"SQL")] {
                                    if ui.selectable_value(&mut dialog.format,format,label).changed() {
                                        if format != db::ExportFormat::Sql && dialog.contents == Contents::Schema { dialog.contents = Contents::Data; }
                                        changed = true;
                                    }
                                }
                            });
                            ui.add_space(6.0);
                            egui::Frame::group(ui.style()).inner_margin(16.0).show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.set_min_height(panel_height - 64.0);
                                ui.strong(match dialog.format { db::ExportFormat::Csv | db::ExportFormat::CsvCustom(_) => "CSV options", db::ExportFormat::Json => "JSON options", db::ExportFormat::Sql => "SQL options" });
                                ui.add_space(16.0);
                                if dialog.format == db::ExportFormat::Sql {
                                    changed |= ui.radio_value(&mut dialog.contents,Contents::Data,"Data only").changed();
                                    changed |= ui.radio_value(&mut dialog.contents,Contents::Schema,"Schema only").changed();
                                    changed |= ui.radio_value(&mut dialog.contents,Contents::SchemaAndData,"Schema + data").changed();
                                    ui.add_space(16.0);
                                    ui.weak(match dialog.contents {
                                        Contents::Data => "INSERT statements for all saved rows.",
                                        Contents::Schema => "CREATE definitions for selected objects.",
                                        Contents::SchemaAndData => "CREATE definitions and INSERT statements for all selected tables in one SQL file.",
                                    });
                                } else {
                                    let mut schema = dialog.contents == Contents::SchemaAndData;
                                    if ui.checkbox(&mut schema,"Include SQL schema").changed() {
                                        dialog.contents = if schema { Contents::SchemaAndData } else { Contents::Data }; changed = true;
                                    }
                                    ui.add_space(16.0);
                                    if dialog.format.is_csv() {
                                        use db::csv_options::{Delimiter,Quoting,LineEnding};
                                        ui.checkbox(&mut dialog.csv_options.null_to_empty,"Convert NULL to empty").on_hover_text("When off, NULL values are written as the unquoted text NULL.");
                                        ui.checkbox(&mut dialog.csv_options.flatten_lines,"Convert line breaks to spaces");
                                        ui.checkbox(&mut dialog.csv_options.header,"Field names in the first row");
                                        ui.add_space(12.0);
                                        egui::Grid::new("export-csv-options").spacing(egui::vec2(12.0,6.0)).show(ui,|ui| {
                                            ui.label("Delimiter"); export_choice(ui,"csv-delimiter",&mut dialog.csv_options.delimiter,&[(Delimiter::Comma,"Comma (,)"),(Delimiter::Semicolon,"Semicolon (;)"),(Delimiter::Tab,"Tab"),(Delimiter::Pipe,"Pipe (|)")]); ui.end_row();
                                            ui.label("Quoting"); export_choice(ui,"csv-quoting",&mut dialog.csv_options.quoting,&[(Quoting::IfNeeded,"Quote if needed"),(Quoting::Always,"Always quote")]); ui.end_row();
                                            ui.label("Line break"); export_choice(ui,"csv-lines",&mut dialog.csv_options.line_ending,&[(LineEnding::Lf,"LF (\\n)"),(LineEnding::CrLf,"CRLF (\\r\\n)"),(LineEnding::Cr,"CR (\\r)")]); ui.end_row();
                                            ui.label("Decimal"); export_choice(ui,"csv-decimal",&mut dialog.csv_options.decimal_comma,&[(false,"Point (.)"),(true,"Comma (,)")]); ui.end_row();
                                        });
                                    } else {
                                        egui::Grid::new("export-json-details").spacing(egui::vec2(16.0,8.0)).show(ui,|ui| {
                                            for (label,value) in [("Rows","Array of objects"),("Binary values","Byte arrays"),("NULL values","null")] { ui.weak(label); ui.label(value); ui.end_row(); }
                                        });
                                    }
                                }
                                ui.add_space(16.0);
                                if dialog.contents != Contents::Schema { ui.weak("All saved rows · every page").on_hover_text("Current filters and unsaved edits are not included."); }
                                if dialog.contents != Contents::Data {
                                    let details = match self.engine.as_str() {
                                        "PostgreSQL" => "Selected-object definitions only. Ownership and grants are excluded. Referenced types, sequences and trigger functions must already exist. Partitioned/inherited tables are not supported. Restore referenced tables first.",
                                        "MySQL" | "MariaDB" => "Selected-object SHOW CREATE definitions only. External dependencies, ownership, grants, triggers and routines are not included. Restore referenced tables first.",
                                        _ => "Selected-object definitions only. External dependencies, ownership and grants are not included. Restore referenced tables first.",
                                    };
                                    ui.weak("Schema scope (details)").on_hover_text(details);
                                }
                            });
                        });
                        if changed { dialog.update_files(); }
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            ui.label("File name:");
                            ui.add(egui::Label::new(RichText::new(if dialog.count == 0 { "No tables selected" } else { &dialog.preview }).monospace()).truncate())
                                .on_hover_text(&dialog.preview);
                        });
                        ui.weak(if dialog.single_file() {
                            "Choose a name and location on export."
                        } else { "Separate files in a new export folder." });
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            if dialog.choosing { ui.spinner(); }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                crate::controls::rounded_action_buttons(ui);
                                choose = ui.add_enabled(dialog.count > 0 && !dialog.choosing && self.pending.is_none(), egui::Button::new("Export…")).clicked();
                                close = ui.button("Cancel").clicked();
                            });
                        });
                    });
                });
            },
        );
        if close || modal.should_close() {
            self.sidebar_export = None;
            self.sidebar_export_epoch = self.sidebar_export_epoch.wrapping_add(1);
        } else if choose {
            self.choose_sidebar_export_destination(ctx);
        }
    }

    fn choose_sidebar_export_destination(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.sidebar_export else {
            return;
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            dialog.choosing = true;
            let single_file = dialog.single_file();
            let filename = dialog.preview.clone();
            let epoch = dialog.epoch;
            let (label, extension) = if dialog.contents == Contents::Schema {
                ("SQL schema", "sql")
            } else {
                match dialog.format {
                    db::ExportFormat::Csv | db::ExportFormat::CsvCustom(_) => ("CSV", "csv"),
                    db::ExportFormat::Json => ("JSON", "json"),
                    db::ExportFormat::Sql => ("SQL inserts", "sql"),
                }
            };
            let tx = self.files_tx.clone();
            let viewport = ctx.viewport_id();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let picker = rfd::FileDialog::new().set_title("Export tables");
                let path = if single_file {
                    picker
                        .add_filter(label, &[extension])
                        .set_file_name(filename)
                        .save_file()
                } else {
                    picker.pick_folder()
                };
                let _ = tx.send(FileAction::SidebarExportDestination(epoch, path));
                ctx.request_repaint_of(viewport);
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (dialog, ctx);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn finish_sidebar_export(&mut self, epoch: u64, path: Option<PathBuf>) {
        let Some(dialog) = &mut self.sidebar_export else {
            return;
        };
        if dialog.epoch != epoch || self.sidebar_export_epoch != epoch {
            return;
        }
        dialog.choosing = false;
        let Some(path) = path else {
            return;
        };
        if self.pending.is_some() {
            self.error = Some("A database operation is running. Choose the export destination again when it finishes.".into());
            return;
        }
        let dialog = self.sidebar_export.take().unwrap();
        let format = dialog.effective_format();
        let tables: Vec<_> = dialog
            .entries
            .into_iter()
            .filter(|entry| entry.selected)
            .map(|entry| (entry.name, entry.label))
            .collect();
        let count = tables.len();
        if count == 0 {
            return;
        }
        let command = if format == db::ExportFormat::Sql {
            Command::ExportSql {
                path,
                tables,
                schema: dialog.contents != Contents::Data,
                data: dialog.contents != Contents::Schema,
            }
        } else if dialog.contents != Contents::Data {
            Command::ExportSchema {
                path,
                tables,
                data_format: (dialog.contents == Contents::SchemaAndData).then_some(format),
            }
        } else if count == 1 {
            Command::ExportTable {
                path,
                format,
                browse: Browse {
                    table: tables[0].0.clone(),
                    ..Default::default()
                },
            }
        } else {
            Command::ExportTables {
                directory: path,
                tables,
                format,
            }
        };
        self.dispatch(command, Pending::Export);
        if self.pending.is_some() {
            self.status = format!(
                "Exporting {count} {}…",
                if count == 1 { "table" } else { "tables" }
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_ranges_follow_visible_order_and_context_preserves_bulk_selection() {
        let mut selection = Selection::default();
        let visible = || vec!["z", "a", "b", "c"];
        selection.click("z", false, false, visible);
        selection.click("b", false, true, visible);
        assert_eq!(selection.names, ["z", "a", "b"].map(String::from).into());
        selection.context("a");
        assert_eq!(selection.names.len(), 3);
        selection.click("a", true, false, visible);
        assert_eq!(selection.names, ["z", "b"].map(String::from).into());
        selection.context("c");
        assert_eq!(selection.names, ["c".to_owned()].into());
        selection.click("b", false, true, || vec!["b"]);
        assert_eq!(selection.names, ["b".to_owned()].into());
    }

    #[test]
    fn cancelled_or_stale_picker_cannot_export_from_another_review() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        app.connections = false;
        app.engine = "SQLite".into();
        app.pending = None;
        app.objects = vec![Object {
            name: "customers".into(),
            kind: "table".into(),
            schema: None,
            display_name: None,
            metadata_error: None,
            columns: vec![],
            ddl: String::new(),
        }];
        app.sidebar_selection.context("customers");
        app.begin_sidebar_export();
        let epoch = app.sidebar_export_epoch;
        app.sidebar_export.as_mut().unwrap().choosing = true;
        app.finish_sidebar_export(epoch, None);
        assert!(!app.sidebar_export.as_ref().unwrap().choosing);
        app.begin_sidebar_export();
        app.finish_sidebar_export(epoch, Some(PathBuf::from("must-not-export.csv")));
        assert!(app.pending.is_none());
        assert!(app.sidebar_export.is_some());
        assert!(app.dialog_owns_workspace_shortcuts());
        let epoch = app.sidebar_export_epoch;
        app.sidebar_export.as_mut().unwrap().choosing = true;
        app.pending = Some(Pending::Export);
        app.finish_sidebar_export(epoch, Some(PathBuf::from("must-not-export.csv")));
        assert!(!app.sidebar_export.as_ref().unwrap().choosing);
        assert!(app.error.as_ref().unwrap().contains("operation is running"));
    }
}

#[cfg(any(test, feature = "qa"))]
#[path = "table_export_qa.rs"]
mod qa;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Open,
    Definition,
    Truncate,
    Drop,
}

/// Build identifiers from separate catalog components, never interpolating a
/// display string or trusting the current search path.
fn destructive_sql(engine: &str, action: Action, objects: &[&Object]) -> Result<String, String> {
    if objects.is_empty() || objects.iter().any(|object| object.kind != "table") {
        return Err("Select one or more tables".into());
    }
    let mysql = matches!(engine, "MySQL" | "MariaDB");
    let quote = |value: &str| {
        let delimiter = if mysql { '`' } else { '"' };
        format!(
            "{delimiter}{}{delimiter}",
            value.replace(delimiter, &format!("{delimiter}{delimiter}"))
        )
    };
    let tables = objects
        .iter()
        .map(|object| match engine {
            "SQLite" => Ok(quote(&object.name)),
            "PostgreSQL" => Ok(format!(
                "{}.{}",
                quote(
                    object
                        .schema
                        .as_deref()
                        .ok_or("Reload the schema before changing this table")?
                ),
                quote(
                    object
                        .display_name
                        .as_deref()
                        .ok_or("Reload the schema before changing this table")?
                )
            )),
            "MySQL" | "MariaDB" => Ok(format!(
                "{}.{}",
                quote(
                    object
                        .schema
                        .as_deref()
                        .ok_or("Reload the schema before changing this table")?
                ),
                quote(&object.name)
            )),
            _ => Err("Table actions are not supported by this database"),
        })
        .collect::<Result<Vec<_>, &str>>()
        .map_err(str::to_owned)?;
    match (engine, action) {
        ("PostgreSQL", Action::Truncate) => Ok(format!("TRUNCATE TABLE {};", tables.join(", "))),
        (_, Action::Drop) if engine != "SQLite" => Ok(format!("DROP TABLE {};", tables.join(", "))),
        (_, Action::Drop) => Ok(tables
            .iter()
            .map(|table| format!("DROP TABLE {table};"))
            .collect::<Vec<_>>()
            .join("\n")),
        ("SQLite", Action::Truncate) => Ok(tables
            .iter()
            .map(|table| format!("DELETE FROM {table};"))
            .collect::<Vec<_>>()
            .join("\n")),
        (_, Action::Truncate) => Ok(tables
            .iter()
            .map(|table| format!("TRUNCATE TABLE {table};"))
            .collect::<Vec<_>>()
            .join("\n")),
        _ => Err("This action does not generate SQL".into()),
    }
}

impl Rustable {
    pub(super) fn sidebar_table_action(&mut self, action: Action) {
        if self.pending.is_some() {
            return;
        }
        let names: Vec<_> = self.sidebar_selection.names.iter().cloned().collect();
        if names.is_empty() {
            return;
        }
        match action {
            Action::Open if names.len() == 1 => {
                self.keep_table_tab(&self.browse.table.clone());
                self.open_table(names[0].clone());
                self.keep_table_tab(&names[0]);
            }
            Action::Definition if names.len() == 1 => {
                self.dispatch(Command::Definition(names[0].clone()), Pending::Export)
            }
            Action::Truncate | Action::Drop => {
                if self.safe_mode_active() {
                    self.error =
                        Some("Turn off safe mode before truncating or dropping tables.".into());
                    return;
                }
                if self.has_pending_edits() {
                    self.error = Some(
                        "Save or discard pending table edits before truncating or dropping tables."
                            .into(),
                    );
                    return;
                }
                let objects: Vec<_> = self
                    .objects
                    .iter()
                    .filter(|object| self.sidebar_selection.names.contains(&object.name))
                    .collect();
                if objects.len() != names.len() {
                    self.error = Some(
                        "The selected tables changed. Refresh the sidebar and select them again."
                            .into(),
                    );
                    return;
                }
                let sql = match destructive_sql(&self.engine, action, &objects) {
                    Ok(sql) => sql,
                    Err(error) => {
                        self.error = Some(error);
                        return;
                    }
                };
                // These statements are generated from a closed action enum.
                // Do not parse thousands of identifiers again on the UI thread.
                let mut preview: String = sql.chars().take(240).collect();
                if preview.len() < sql.len() {
                    preview.push('…');
                }
                let reason = match action {
                    Action::Drop => "DROP removes a database object and its data",
                    _ if self.engine == "SQLite" => "DELETE without WHERE removes every row",
                    _ => "TRUNCATE removes every row",
                };
                // Preserve every editor tab. The existing destructive review is
                // always shown for a context action, even with session skip on.
                self.tabs.push(QueryTab::new(sql.clone()));
                self.tab = self.tabs.len() - 1;
                self.show_query();
                self.destructive_confirm = Some(DestructiveRun {
                    invalidate_tables: names,
                    tab_id: self.tabs[self.tab].id,
                    source: (sql.clone(), 0),
                    findings: vec![crate::sql::destructive::Finding {
                        statement: preview,
                        reason,
                    }],
                    sql,
                    skip_for_session: false,
                });
            }
            _ => (),
        }
    }
}

#[cfg(test)]
mod action_tests {
    use super::*;
    fn object(name: &str, schema: &str) -> Object {
        Object {
            name: name.into(),
            display_name: Some(name.into()),
            schema: Some(schema.into()),
            kind: "table".into(),
            columns: vec![],
            metadata_error: None,
            ddl: String::new(),
        }
    }
    #[test]
    fn destructive_actions_quote_catalog_names_and_never_disable_constraints() {
        let a = object("odd\"; DROP SCHEMA public; --", "s\"x");
        let b = object("b", "s");
        assert_eq!(
            destructive_sql("PostgreSQL", Action::Truncate, &[&a, &b]).unwrap(),
            "TRUNCATE TABLE \"s\"\"x\".\"odd\"\"; DROP SCHEMA public; --\", \"s\".\"b\";"
        );
        let mysql = object("a`b", "s`x");
        assert_eq!(
            destructive_sql("MySQL", Action::Drop, &[&mysql]).unwrap(),
            "DROP TABLE `s``x`.`a``b`;"
        );
        assert_eq!(
            destructive_sql("SQLite", Action::Truncate, &[&b]).unwrap(),
            "DELETE FROM \"b\";"
        );
        let mut view = b.clone();
        view.kind = "view".into();
        assert!(destructive_sql("PostgreSQL", Action::Drop, &[&view]).is_err());
    }
    #[test]
    fn destructive_context_actions_always_review_and_respect_safe_mode() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        app.pending = None;
        app.connections = false;
        app.engine = "SQLite".into();
        app.objects = vec![object("customers", "main")];
        app.sidebar_selection.context("customers");
        app.safe = true;
        app.sidebar_table_action(Action::Drop);
        assert!(app.pending.is_none());
        assert!(app.destructive_confirm.is_none());
        app.safe = false;
        app.skip_destructive_confirm = true;
        let original = app.tabs[0].sql.clone();
        app.sidebar_table_action(Action::Truncate);
        assert!(app.pending.is_none());
        assert_eq!(
            app.destructive_confirm.as_ref().unwrap().sql,
            "DELETE FROM \"customers\";"
        );
        assert_eq!(app.destructive_confirm.as_ref().unwrap().findings.len(), 1);
        assert_eq!(app.tabs[0].sql, original);
    }
    #[test]
    fn confirmed_context_actions_execute_and_do_not_reuse_stale_table_rows() {
        fn wait(app: &mut Rustable) {
            let started = Instant::now();
            while app.pending.is_some() && started.elapsed().as_secs() < 5 {
                app.poll();
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(app.pending.is_none());
            assert!(app.error.is_none(), "{:?}", app.error);
        }
        let ctx = egui::Context::default();
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        app.dispatch(Command::Open(None), Pending::Connect(None));
        wait(&mut app);
        app.safe = false;
        app.dispatch(Command::Query { sql: "CREATE TABLE context_action_a(id INTEGER); INSERT INTO context_action_a VALUES(42)".into(), safe:false },Pending::Query(0));
        wait(&mut app);
        app.open_table("context_action_a".into());
        wait(&mut app);
        assert_eq!(app.table.rows.len(), 1);
        app.sidebar_selection.context("context_action_a");
        app.sidebar_table_action(Action::Truncate);
        assert!(app.pending.is_none());
        let run = app.destructive_confirm.take().unwrap();
        app.execute_query(run);
        wait(&mut app);
        app.open_table("context_action_a".into());
        wait(&mut app);
        assert!(app.table.rows.is_empty());
        app.sidebar_table_action(Action::Drop);
        let run = app.destructive_confirm.take().unwrap();
        app.execute_query(run);
        wait(&mut app);
        assert!(
            !app.objects
                .iter()
                .any(|object| object.name == "context_action_a")
        );
    }
}
