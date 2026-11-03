//! Pending rows keep a stable identity independent of their values and list position.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_INSERT: AtomicU64 = AtomicU64::new(1);

impl Rustable {
    pub(super) fn can_insert(&self) -> bool {
        self.view == View::Data
            && self.supports_data_changes()
            && !self.safe
            && self.pending.is_none()
            && !self.columns.is_empty()
            && (self.engine != "PostgreSQL" || self.columns.iter().any(|column| column.pk))
            && self
                .objects
                .iter()
                .any(|o| o.name == self.browse.table && o.kind == "table")
    }

    pub(super) fn insert_ids(&self) -> Vec<u64> {
        if self.view != View::Data {
            return Vec::new();
        }
        self.changes
            .iter()
            .filter_map(|c| match c {
                Change::InsertRow { id, table, .. } if table == &self.browse.table => Some(*id),
                _ => None,
            })
            .collect()
    }

    pub(super) fn selected_insert(&self) -> Option<u64> {
        let index = self.selected?.checked_sub(self.table.rows.len())?;
        self.insert_ids().get(index).copied()
    }

    pub(super) fn add_insert(&mut self) {
        self.add_insert_with_fields(Vec::new());
    }

    fn add_insert_with_fields(&mut self, fields: Vec<(String, Value)>) {
        if !self.can_insert() || !self.finish_cell_edit() {
            return;
        }
        let id = NEXT_INSERT.fetch_add(1, Ordering::Relaxed);
        self.changes.push(Change::InsertRow {
            id,
            table: self.browse.table.clone(),
            fields,
        });
        self.record_changes();
        self.review = None;
        self.row_selection.clear();
        self.selected = Some(self.table.rows.len() + self.insert_ids().len() - 1);
        if let Some(column) = self
            .columns
            .iter()
            .find(|c| self.insert_field_writable(id, &c.name))
            .map(|c| c.name.clone())
        {
            self.start_insert_edit(id, &column, false);
        }
    }

    pub(super) fn can_duplicate(&self) -> bool {
        self.can_insert()
            && (self.selected_insert().is_some()
                || self
                    .selected
                    .and_then(|i| self.table.rows.get(i))
                    .is_some_and(|row| !row_is_deleted(&self.changes, &self.browse.table, row)))
    }

    pub(super) fn duplicate_selected(&mut self) {
        if !self.can_duplicate() || !self.finish_cell_edit() {
            return;
        }
        let fields = if let Some(id) = self.selected_insert() {
            self.columns
                .iter()
                .filter(|c| !c.pk && !c.generated)
                .filter_map(|c| {
                    self.insert_value(id, &c.name)
                        .map(|v| (c.name.clone(), v.clone()))
                })
                .collect()
        } else if let Some(row) = self.selected.and_then(|i| self.table.rows.get(i)) {
            self.table
                .columns
                .iter()
                .zip(row)
                .filter(|(name, _)| {
                    self.columns
                        .iter()
                        .any(|c| c.name == **name && !c.pk && !c.generated)
                })
                .map(|(name, value)| {
                    let value = self
                        .changes
                        .iter()
                        .find_map(|change| match change {
                            Change::Cell {
                                table,
                                column,
                                original,
                                value,
                            } if table == &self.browse.table
                                && column == name
                                && original == row =>
                            {
                                Some(value)
                            }
                            _ => None,
                        })
                        .unwrap_or(value);
                    (name.clone(), value.clone())
                })
                .collect()
        } else {
            return;
        };
        self.add_insert_with_fields(fields);
    }

    pub(super) fn remove_selected_insert(&mut self) {
        if self.pending.is_some() {
            return;
        }
        if let Some(id) = self.selected_insert() {
            if self.edit.as_ref().is_some_and(|e| e.insert_id == Some(id)) {
                self.edit = None;
            }
            self.changes
                .retain(|c| !matches!(c, Change::InsertRow { id: item, .. } if *item == id));
            self.record_changes();
            self.review = None;
            self.row_selection.clear();
            self.selected = None;
        }
    }

    fn insert_value(&self, id: u64, column: &str) -> Option<&Value> {
        self.changes.iter().find_map(|c| match c {
            Change::InsertRow {
                id: item, fields, ..
            } if *item == id => fields
                .iter()
                .find(|(name, _)| name == column)
                .map(|(_, value)| value),
            _ => None,
        })
    }

    fn set_insert_value(&mut self, id: u64, column: &str, value: Option<Value>) {
        if self.insert_value(id, column) == value.as_ref() {
            return;
        }
        if let Some(Change::InsertRow { fields, .. }) = self
            .changes
            .iter_mut()
            .find(|c| matches!(c, Change::InsertRow { id: item, .. } if *item == id))
        {
            fields.retain(|(name, _)| name != column);
            if let Some(value) = value {
                fields.push((column.into(), value));
            }
            self.record_changes();
            self.review = None;
        }
    }

    fn start_insert_edit(&mut self, id: u64, column: &str, inspector: bool) {
        if !self.can_insert() || !self.finish_cell_edit() {
            return;
        }
        let value = self.insert_value(id, column).cloned();
        if matches!(&value, Some(Value::Blob(_)))
            || matches!(&value, Some(Value::Text(t)) if t.len() > 65536)
        {
            return;
        }
        self.edit = Some(CellEdit {
            bulk_rows: Vec::new(),
            bulk_changed: false,
            insert_id: Some(id),
            advance: 0,
            table: self.browse.table.clone(),
            row: Vec::new(),
            original_value: value.clone().unwrap_or(Value::Null),
            column: column.into(),
            text: value
                .as_ref()
                .filter(|v| !matches!(v, Value::Null))
                .map(db::display)
                .unwrap_or_default(),
            kind: match value {
                None => 5,
                Some(Value::Null) => 0,
                Some(Value::Integer(_)) => 1,
                Some(Value::Real(_)) => 2,
                _ => 3,
            },
            focus: true,
            inspector,
        });
    }

    pub(super) fn finish_insert_edit(&mut self, edit: CellEdit, value: Value) {
        let id = edit.insert_id.unwrap();
        let value = if edit.kind == 5 && edit.text.is_empty() {
            None
        } else {
            Some(value)
        };
        self.set_insert_value(id, &edit.column, value);
        if edit.advance != 0 {
            self.advance_insert_edit(id, &edit.column, edit.inspector, edit.advance);
        }
    }

    pub(super) fn focus_first_insert(&mut self) {
        for (index, id) in self.insert_ids().into_iter().enumerate() {
            if let Some(name) = self
                .columns
                .iter()
                .find(|c| self.insert_field_writable(id, &c.name))
                .map(|c| c.name.clone())
            {
                self.row_selection.clear();
                self.selected = Some(self.table.rows.len() + index);
                self.start_insert_edit(id, &name, false);
                return;
            }
        }
    }

    fn focus_last_loaded(&mut self) {
        if !self.can_insert() || !self.columns.iter().any(|c| c.pk) {
            return;
        }
        let source = CellEdit {
            bulk_rows: Vec::new(),
            bulk_changed: false,
            insert_id: None,
            advance: -1,
            table: self.browse.table.clone(),
            row: Vec::new(),
            original_value: Value::Null,
            column: String::new(),
            text: String::new(),
            kind: 0,
            focus: true,
            inspector: false,
        };
        self.edit = cell_edit_from_slot(
            &source,
            &self.table,
            &self.columns,
            &self.changes,
            "",
            (self.table.rows.len() * self.table.columns.len()) as isize,
        );
        if let Some(edit) = &self.edit {
            self.row_selection.clear();
            self.selected = self.table.rows.iter().position(|row| *row == edit.row);
        }
    }

    fn insert_field_writable(&self, id: u64, name: &str) -> bool {
        self.columns.iter().any(|c| c.name == name && !c.generated)
            && !matches!(self.insert_value(id, name), Some(Value::Blob(_)))
            && !matches!(self.insert_value(id, name), Some(Value::Text(t)) if t.len() > 65536)
    }

    fn advance_insert_edit(&mut self, id: u64, column: &str, inspector: bool, direction: i8) {
        let ids = self.insert_ids();
        let names: Vec<_> = crate::search::Query::new(if inspector { &self.field_search } else { "" })
            .ranked(self.columns.iter(), |column| (&column.name, ""))
            .into_iter().map(|c| c.name.clone()).collect();
        let Some(row) = ids.iter().position(|item| *item == id) else {
            return;
        };
        if names.is_empty() { return; }
        let col = names.iter().position(|name| name == column)
            .map_or(if direction > 0 { -1 } else { names.len() as isize }, |index| index as isize);
        let mut position = (row * names.len()) as isize + col;
        loop {
            position += if direction < 0 { -1 } else { 1 };
            if position < 0 {
                if !inspector {
                    self.focus_last_loaded();
                }
                return;
            }
            if position as usize >= ids.len() * names.len() {
                return;
            }
            let next_row = position as usize / names.len();
            if inspector && next_row != row {
                return;
            }
            let name = &names[position as usize % names.len()];
            let next_id = ids[next_row];
            if self.insert_field_writable(next_id, name) {
                self.row_selection.clear();
                self.selected = Some(self.table.rows.len() + next_row);
                self.start_insert_edit(next_id, name, inspector);
                return;
            }
        }
    }

    fn insert_field(&mut self, ui: &mut egui::Ui, id: u64, column: &str, inspector: bool) {
        let generated = self.columns.iter().any(|c| c.name == column && c.generated);
        if self.edit.as_ref().is_some_and(|e| {
            e.insert_id == Some(id) && e.column == column && e.inspector == inspector
        }) {
            if inline_value(ui, &mut self.edit) {
                self.finish_cell_edit();
            }
            return;
        }
        let value = self.insert_value(id, column);
        let text = if generated {
            "GENERATED".into()
        } else {
            value
                .map(|v| db::preview(v, if inspector { 4096 } else { 180 }).into_owned())
                .unwrap_or_else(|| "DEFAULT".into())
        };
        let label = ui.add(
            egui::Label::new(RichText::new(text).monospace().color(if value.is_none() {
                MUTED
            } else {
                ui.visuals().text_color()
            }))
            .truncate()
            .sense(egui::Sense::click()),
        );
        let label = if generated {
            label.on_hover_text("Generated by the database when this row is saved · read-only")
        } else { label };
        if (label.clicked() && inspector || label.double_clicked()) && !generated {
            self.start_insert_edit(id, column, inspector);
        }
        label.context_menu(|ui| {
            if ui.button("Remove new row").clicked() {
                self.selected = self.insert_ids().iter().position(|item| *item == id).map(|index| self.table.rows.len() + index);
                self.remove_selected_insert();
                ui.close();
            }
            ui.add_enabled_ui(self.can_insert() && !generated, |ui| {
                for (title, value) in [
                    ("Use DEFAULT", None),
                    ("Set NULL", Some(Value::Null)),
                    ("Set empty text", Some(Value::Text(String::new()))),
                ] {
                    if ui.button(title).clicked() && self.finish_cell_edit() {
                        self.set_insert_value(id, column, value);
                        ui.close();
                    }
                }
            });
        });
    }

    pub(super) fn insert_grid_row(
        &mut self,
        row: &mut egui_extras::TableRow<'_, '_>,
        id: u64,
        index: usize,
        table_clip: egui::Rect,
    ) {
        row.set_hovered(false);
        row.set_selected(false);
        row.col(|ui| {
            paint_row_number(ui, "+");
        });
        row.set_selected(self.selected == Some(index));
        let names = self.table.columns.clone();
        for name in names {
            row.col(|ui| {
                paint_cell_background(ui, table_clip, pending_fill(ui));
                self.insert_field(ui, id, &name, false);
            });
        }
        if primary_pressed(&row.response()) {
            self.row_selection.clear();
            self.selected = Some(index);
        }
    }

    pub(super) fn insert_inspector(&mut self, ui: &mut egui::Ui, id: u64) {
        ui.label(RichText::new("New row · pending save").color(Color32::from_rgb(210, 150, 50)));
        let columns = self.columns.clone();
        let search = self.field_search.to_lowercase();
        egui::ScrollArea::vertical().id_salt(("insert-fields", &search)).show(ui, |ui| {
            compact_inspector_spacing(ui);
            for column in crate::search::Query::new(&search).ranked(columns, |column| (&column.name, "")) {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&column.name)
                            .size(11.0)
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(inspector_column_type(&column))
                                .size(10.0)
                                .color(ui.visuals().weak_text_color()),
                        );
                    });
                });
                egui::Frame::new()
                    .fill(pending_fill(ui))
                    .inner_margin(1.0)
                    .corner_radius(4.0)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), 22.0),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.add_enabled_ui(self.can_insert() && !column.generated, |ui| {
                                    field_value_menu(ui, |ui| {
                                        for (title, value, enabled) in [
                                            ("Use DEFAULT", None, true),
                                            ("Set NULL", Some(Value::Null), !column.not_null),
                                            (
                                                "Set empty text",
                                                Some(Value::Text(String::new())),
                                                true,
                                            ),
                                        ] {
                                            if ui
                                                .add_enabled(enabled, egui::Button::new(title))
                                                .clicked()
                                                && self.finish_cell_edit()
                                            {
                                                self.set_insert_value(id, &column.name, value);
                                                ui.close();
                                            }
                                        }
                                    })
                                    .on_hover_text("Set field value");
                                });
                                ui.with_layout(
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        if let Some(value) =
                                            self.insert_value(id, &column.name).cloned()
                                            && ui
                                                .push_id(
                                                    ("copy-insert", id, &column.name),
                                                    clipboard_button,
                                                )
                                                .inner
                                                .clicked()
                                        {
                                            ui.ctx().copy_text(db::display(&value));
                                        }
                                        self.insert_field(ui, id, &column.name, true);
                                    },
                                );
                            },
                        );
                    });
                ui.add_space(1.0);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn postgres_without_primary_key_cannot_stage_row_mutations() {
        let mut app = app();
        app.engine = "PostgreSQL".into();
        app.selected = Some(0);
        assert!(!app.can_insert());
        assert!(!app.can_duplicate());
        assert!(!app.can_delete_rows());
        app.add_insert();
        app.duplicate_selected();
        assert!(app.changes.is_empty());
        assert!(app.edit.is_none());
        assert!(!app.safe, "Manual SQL writes remain available");
    }

    #[test]
    fn postgres_row_capabilities_preserve_generated_and_readonly_guards() {
        let mut app = app();
        app.engine = "PostgreSQL".into();
        app.columns[0].pk = true;
        app.columns[0].generated = true;
        assert!(app.can_insert());
        assert!(app.can_delete_rows());
        assert!(app.supports_csv_import());
        app.add_insert();
        assert!(
            matches!(app.changes.as_slice(), [Change::InsertRow { fields, .. }] if fields.is_empty())
        );
        let id = app.selected_insert().unwrap();
        assert!(!app.insert_field_writable(id, "id"));
        assert!(!app.insert_field_writable(id, "generated"));
        assert!(app.insert_field_writable(id, "name"));
        app.edit = None;
        app.changes.clear();
        app.safe = true;
        assert!(!app.can_insert() && !app.can_delete_rows());
        app.safe = false;
        app.pending = Some(Pending::Table);
        assert!(!app.can_insert() && !app.can_delete_rows());
        app.pending = None;
        app.objects[0].kind = "view".into();
        assert!(!app.can_insert() && !app.can_delete_rows());
        app.objects[0].kind = "table".into();
        app.columns[2].data_type = "custom_domain".into();
        assert!(app.can_insert() && app.can_delete_rows());
        app.add_insert();
        assert!(
            !app.changes.is_empty(),
            "PostgreSQL validates catalog types during review"
        );
        app.changes.clear();
        app.engine = "UnknownDriver".into();
        app.add_insert();
        assert!(app.changes.is_empty());
    }

    fn app() -> Rustable {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.engine = "SQLite".into();
        app.safe = false;
        app.view = View::Data;
        app.browse.table = "t".into();
        app.objects = vec![db::Object {
            schema: None,
            display_name: None,
            metadata_error: None,
            columns: vec![],
            name: "t".into(),
            kind: "table".into(),
            ddl: String::new(),
        }];
        app.columns = ["id", "generated", "name"]
            .into_iter()
            .map(|name| Column {
                name: name.into(),
                data_type: "TEXT".into(),
                not_null: false,
                default: String::new(),
                pk: false,
                generated: name == "generated",
            })
            .collect();
        app.table = Arc::new(Grid {
            columns: vec!["id".into(), "generated".into(), "name".into()],
            ..Default::default()
        });
        app
    }
    #[test]
    fn tab_bridges_loaded_rows_and_pending_inserts_bidirectionally() {
        let mut app = app();
        app.columns[0].pk = true;
        app.table = Arc::new(Grid {
            columns: vec!["id".into(), "generated".into(), "name".into()],
            rows: vec![vec![
                Value::Integer(1),
                Value::Null,
                Value::Text("original".into()),
            ]],
            ..Default::default()
        });
        app.add_insert();
        let id = app.selected_insert().unwrap();
        app.finish_cell_edit();
        app.focus_last_loaded();
        assert_eq!(app.edit.as_ref().unwrap().column, "name");
        app.edit.as_mut().unwrap().text = "updated".into();
        app.edit.as_mut().unwrap().advance = 1;
        app.finish_cell_edit();
        assert_eq!(app.edit.as_ref().unwrap().insert_id, Some(id));
        assert_eq!(app.edit.as_ref().unwrap().column, "id");
        app.edit.as_mut().unwrap().advance = -1;
        app.finish_cell_edit();
        let edit = app.edit.as_ref().unwrap();
        assert_eq!(edit.insert_id, None);
        assert_eq!(edit.text, "updated");
        assert_eq!(app.selected, Some(0));
        app.edit = None;
        toggle_row_deletion(&mut app.changes, "t", app.table.rows[0].clone());
        app.focus_last_loaded();
        assert!(app.edit.is_none());
    }

    #[test]
    fn navigation_crosses_pending_rows_and_skips_readonly_copied_values() {
        let mut app = app();
        app.add_insert();
        let first = app.selected_insert().unwrap();
        app.add_insert();
        let second = app.selected_insert().unwrap();
        app.finish_cell_edit();
        app.set_insert_value(second, "id", Some(Value::Blob(vec![0, 1])));
        app.advance_insert_edit(first, "name", false, 1);
        assert_eq!(app.selected_insert(), Some(second));
        assert_eq!(app.edit.as_ref().unwrap().column, "name");
        app.edit = None;
        app.advance_insert_edit(second, "name", false, -1);
        assert_eq!(app.selected_insert(), Some(first));
        assert_eq!(app.edit.as_ref().unwrap().column, "name");
        app.edit = None;
        app.advance_insert_edit(first, "name", true, 1);
        assert!(app.edit.is_none(), "inspector must stay on its row");
        app.set_insert_value(second, "name", Some(Value::Text("x".repeat(65537))));
        app.advance_insert_edit(first, "name", false, 1);
        assert!(app.edit.is_none(), "all remaining cells are readonly");
    }

    #[test]
    fn duplication_uses_staged_values_and_omits_keys_and_generated_fields() {
        let mut app = app();
        app.columns[0].pk = true;
        app.table = Arc::new(Grid {
            columns: vec!["id".into(), "generated".into(), "name".into()],
            rows: vec![vec![
                Value::Integer(7),
                Value::Text("computed".into()),
                Value::Text("original".into()),
            ]],
            ..Default::default()
        });
        app.selected = Some(0);
        app.changes.push(Change::Cell {
            table: "t".into(),
            column: "name".into(),
            original: app.table.rows[0].clone(),
            value: Value::Text("pending".into()),
        });
        app.duplicate_selected();
        let first = app.selected_insert().unwrap();
        assert!(app.insert_value(first, "id").is_none());
        assert!(app.insert_value(first, "generated").is_none());
        assert_eq!(
            app.insert_value(first, "name"),
            Some(&Value::Text("pending".into()))
        );
        app.finish_cell_edit();
        app.set_insert_value(first, "name", Some(Value::Blob(vec![0, 255])));
        app.duplicate_selected();
        let second = app.selected_insert().unwrap();
        assert_ne!(first, second);
        assert_eq!(
            app.insert_value(second, "name"),
            Some(&Value::Blob(vec![0, 255]))
        );
        app.finish_cell_edit();
        app.start_insert_edit(second, "name", false);
        assert!(app.edit.is_none());
        assert_eq!(app.table.rows[0][2], Value::Text("original".into()));
        app.selected = Some(0);
        toggle_row_deletion(&mut app.changes, "t", app.table.rows[0].clone());
        assert!(!app.can_duplicate());
    }

    #[test]
    fn default_null_empty_and_identical_rows_remain_distinct() {
        let mut app = app();
        app.add_insert();
        let first = app.selected_insert().unwrap();
        assert_eq!(app.edit.as_ref().unwrap().column, "id");
        app.finish_cell_edit();
        assert!(app.insert_value(first, "id").is_none());
        app.add_insert();
        let second = app.selected_insert().unwrap();
        assert_ne!(first, second);
        app.finish_cell_edit();
        app.set_insert_value(first, "name", Some(Value::Null));
        app.set_insert_value(second, "name", Some(Value::Text(String::new())));
        assert_eq!(app.insert_value(first, "name"), Some(&Value::Null));
        assert_eq!(
            app.insert_value(second, "name"),
            Some(&Value::Text(String::new()))
        );
        app.set_insert_value(first, "name", None);
        assert!(app.insert_value(first, "name").is_none());
        app.remove_selected_insert();
        assert_eq!(app.insert_ids(), vec![first]);
        app.discard_changes();
        assert!(app.insert_ids().is_empty());
    }
    #[test]
    fn tab_stages_value_skips_generated_and_preserves_identity() {
        let mut app = app();
        app.add_insert();
        let id = app.selected_insert().unwrap();
        let draft = app.edit.as_mut().unwrap();
        draft.text = "42".into();
        draft.advance = 1;
        assert!(app.finish_cell_edit());
        assert_eq!(app.insert_value(id, "id"), Some(&Value::Text("42".into())));
        let draft = app.edit.as_ref().unwrap();
        assert_eq!(draft.column, "name");
        assert_eq!(draft.insert_id, Some(id));
        app.edit.as_mut().unwrap().advance = -1;
        app.finish_cell_edit();
        assert_eq!(app.edit.as_ref().unwrap().column, "id");
        app.safe = true;
        let before = app.insert_ids();
        app.add_insert();
        assert_eq!(app.insert_ids(), before);
    }
}
