use super::*;

impl Rustable {
    pub(super) fn page_grid_shortcut(&mut self, response: &egui::Response) {
        if !response.enabled() || !response.has_focus() || self.view != View::Data
            || self.pending.is_some() || self.edit.is_some() || self.rename.is_some()
            || self.dialog_owns_workspace_shortcuts() || egui::Popup::is_any_open(&response.ctx)
        {
            return;
        }
        // Paging clears row selection. Keep arrows on the grid even before
        // a row is selected on the new page, instead of egui focus traversal.
        response.ctx.memory_mut(|memory| {
            memory.set_focus_lock_filter(response.id, egui::EventFilter {
                horizontal_arrows: true, vertical_arrows: true, tab: true,
                ..Default::default()
            });
        });
        let direction = response.ctx.input_mut(|input| {
            if input.modifiers.shift || input.modifiers.alt { return None; }
            if input.consume_key(egui::Modifiers::COMMAND, egui::Key::ArrowRight) {
                Some(true)
            } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::ArrowLeft) {
                Some(false)
            } else { None }
        });
        if let Some(next) = direction {
            self.change_table_page(next);
            self.focus_page_grid = matches!(self.pending, Some(Pending::Table));
        }
    }

    pub(super) fn row_delete_shortcut(&self, ctx: &egui::Context, row_focused: bool) -> bool {
        if !row_focused || !self.can_delete_rows() || self.row_shortcuts_blocked() {
            return false;
        }
        ctx.input_mut(|input| {
            input.modifiers.is_none()
                && (input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
                    || input.consume_key(egui::Modifiers::NONE, egui::Key::Delete))
        })
    }

    fn row_shortcuts_blocked(&self) -> bool {
        self.edit.is_some()
            || self.dialog_owns_workspace_shortcuts()
            || self.rename.is_some()
            || self.row_selection.rows.is_empty()
    }

    pub(super) fn select_all_rows_shortcut(&mut self, ctx: &egui::Context, focused: Option<usize>) {
        let Some(focused) = focused else { return };
        if self.edit.is_some() || self.dialog_owns_workspace_shortcuts() || self.rename.is_some()
            || egui::Popup::is_any_open(ctx) || !matches!(self.view, View::Data | View::Query)
        {
            return;
        }
        let count = self.grid().rows.len();
        if focused >= count {
            return;
        }
        if ctx.input_mut(|input| {
            !input.modifiers.shift
                && !input.modifiers.alt
                && input.consume_key(egui::Modifiers::COMMAND, egui::Key::A)
        }) {
            self.selected = self.row_selection.select_all(count, focused);
            ctx.request_repaint();
        }
    }

    pub(super) fn restore_grid_keyboard_focus(
        &self,
        grid: &egui::Response,
        released_over_row: bool,
        starting_edit: bool,
    ) {
        if !starting_edit
            && !self.row_shortcuts_blocked()
            && !egui::Popup::is_any_open(&grid.ctx)
            && released_over_row
            && grid
                .ctx
                .input(|input| input.pointer.button_released(egui::PointerButton::Primary))
        {
            // The noninteractive keyboard owner can lose focus on release over a
            // child label. Restore ownership without applying selection twice.
            grid.request_focus();
            grid.ctx.request_repaint();
        }
    }

    pub(super) fn navigate_rows_shortcut(&mut self, response: &egui::Response, index: usize) {
        if !response.has_focus()
            || self.row_shortcuts_blocked()
            || egui::Popup::is_any_open(&response.ctx)
            || !matches!(self.view, View::Data | View::Query)
        {
            return;
        }
        response.ctx.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    vertical_arrows: true,
                    horizontal_arrows: true,
                    tab: true,
                    ..Default::default()
                },
            )
        });
        let grid = self.grid();
        let count = grid.rows.len();
        let column_count = grid.columns.len();
        let mut focused = index;
        let mut moved = false;
        response.ctx.input_mut(|input| {
            // Process repeats in arrival order without allocating a second event
            // queue. Retain unrelated keys, releases and modified shortcuts.
            input.events.retain(|event| {
                if let egui::Event::Key {
                    key:
                        key @ (egui::Key::ArrowDown
                        | egui::Key::ArrowUp
                        | egui::Key::ArrowLeft
                        | egui::Key::ArrowRight
                        | egui::Key::Tab),
                    pressed: true,
                    modifiers,
                    ..
                } = event
                    && !modifiers.command
                    && !modifiers.ctrl
                    && !modifiers.alt
                {
                    if matches!(
                        key,
                        egui::Key::ArrowLeft | egui::Key::ArrowRight | egui::Key::Tab
                    ) {
                        let forward = *key == egui::Key::ArrowRight
                            || (*key == egui::Key::Tab && !modifiers.shift);
                        self.row_selection.cell_column = if forward {
                            self.row_selection
                                .cell_column
                                .saturating_add(1)
                                .min(column_count.saturating_sub(1))
                        } else {
                            self.row_selection.cell_column.saturating_sub(1)
                        };
                        if self.row_selection.cell_columns.is_some() {
                            let column = self.row_selection.cell_column;
                            self.row_selection.cell_columns = Some((column, column));
                        }
                        self.row_selection.pending_focus = Some(focused);
                        self.row_selection.pending_cell_scroll = true;
                        moved = true;
                    } else if let Some(next) = self.row_selection.move_vertical(
                        focused,
                        *key == egui::Key::ArrowDown,
                        modifiers.shift,
                        count,
                    ) {
                        focused = next;
                        moved = true;
                    }
                    false
                } else {
                    true
                }
            });
        });
        if moved {
            self.selected = (self.row_selection.rows.len() == 1)
                .then(|| *self.row_selection.rows.first().unwrap());
            response.ctx.request_repaint();
        }
    }

    pub(super) fn copy_rows_shortcut(&mut self, ctx: &egui::Context, row_focused: bool) {
        if !row_focused || self.row_shortcuts_blocked() {
            return;
        }
        // Native integrations emit Copy rather than a literal Cmd+C key event.
        let copy = ctx.input_mut(|input| {
            let copy = input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Copy));
            if copy {
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::Copy));
            }
            let key = input.consume_key(egui::Modifiers::COMMAND, egui::Key::C);
            copy || key
        });
        if !copy {
            return;
        }
        let grid = self.grid();
        if self
            .row_selection
            .rows
            .iter()
            .any(|index| *index >= grid.rows.len())
        {
            self.error = Some(
                "The selected rows are no longer available. Select them again to copy.".into(),
            );
            return;
        }
        let columns = self.row_selection.cell_columns.unwrap_or((0, grid.columns.len().saturating_sub(1)));
        let text = row_copy::plain_cells(
            &grid.columns,
            self.row_selection.rows.iter().map(|index| grid.rows[*index].as_slice()),
            &self.changes,
            self.staged_table(),
            columns,
        );
        match text { Ok(text) => ctx.copy_text(text), Err(error) => self.error = Some(error) }
    }

    pub(super) fn can_delete_rows(&self) -> bool {
        self.can_edit_rows()
    }

    pub(super) fn selected_rows_deleted(&self) -> bool {
        self.staged_table().is_some()
            && !self.row_selection.rows.is_empty()
            && self.row_selection.rows.iter().all(|index| {
                self.grid()
                    .rows
                    .get(*index)
                    .is_some_and(|row| row_is_deleted(&self.changes, self.edit_table(), row))
            })
    }

    pub(super) fn set_selected_rows_deleted(&mut self, deleted: bool) {
        if !self.can_delete_rows() || self.row_selection.rows.is_empty() {
            return;
        }
        // Capture the intended set before finalizing an active inline draft can move focus.
        let indices: Vec<_> = self.row_selection.rows.iter().copied().collect();
        if indices.iter().any(|index| *index >= self.grid().rows.len()) {
            return;
        }
        if !self.finish_cell_edit() {
            return;
        }
        let (grid, table) = (self.grid(), self.edit_table().to_owned());
        for index in indices {
            let row = &grid.rows[index];
            if row_is_deleted(&self.changes, &table, row) != deleted {
                toggle_row_deletion(&mut self.changes, &table, row.clone());
            }
        }
        self.record_changes();
        self.review = None;
        self.selected = None;
        self.row_selection.clear();
    }
}

fn field_summary<'a>(
    grid: &'a Grid,
    indices: &[usize],
    column: usize,
    changes: &'a [Change],
    table: Option<&str>,
) -> (Option<&'a Value>, bool) {
    let mut common = None;
    let mut mixed = false;
    let mut pending = false;
    for index in indices {
        let Some(row) = grid.rows.get(*index) else {
            return (None, pending);
        };
        let Some(original) = row.get(column) else {
            return (None, pending);
        };
        let staged = changes.iter().find_map(|change| match change {
            Change::Cell {
                table: target,
                column: name,
                original: source,
                value,
            } if table == Some(target.as_str())
                && grid.columns.get(column) == Some(name)
                && source == row =>
            {
                Some(value)
            }
            _ => None,
        });
        pending |= staged.is_some();
        let value = staged.unwrap_or(original);
        if let Some(first) = common {
            mixed |= first != value;
        } else {
            common = Some(value);
        }
    }
    (if mixed { None } else { common }, pending)
}

// Borrow values and scan pending changes once per selected row, not once per field.
// Scratch space is proportional to column count; no cell payloads are cloned.
fn field_summaries<'a>(
    grid: &'a Grid,
    indices: &[usize],
    changes: &'a [Change],
    table: Option<&str>,
) -> Vec<(Option<&'a Value>, bool, bool)> {
    let mut summaries = vec![(None, false, indices.len() >= 2); grid.columns.len()];
    let mut mixed = vec![false; grid.columns.len()];
    let mut staged = vec![None; grid.columns.len()];
    let editable = |value: &Value| {
        !matches!(value, Value::Blob(_))
            && !matches!(value, Value::Text(text) if text.len() > 65536)
    };
    for index in indices {
        let Some(row) = grid.rows.get(*index) else {
            return vec![(None, false, false); grid.columns.len()];
        };
        staged.fill(None);
        let mut deleted = false;
        if let Some(table) = table {
            for change in changes {
                match change {
                    Change::Cell {
                        table: target,
                        column,
                        original,
                        value,
                    } if target == table && original == row => {
                        if let Some(i) = grid.columns.iter().position(|name| name == column) {
                            // Preserve the same first-match semantics as field_summary.
                            staged[i].get_or_insert(value);
                        }
                    }
                    Change::DeleteRow {
                        table: target,
                        original,
                    } if target == table && original == row => deleted = true,
                    _ => {}
                }
            }
        }
        for (column, (common, pending, writable)) in summaries.iter_mut().enumerate() {
            let Some(original) = row.get(column) else {
                mixed[column] = true;
                *writable = false;
                continue;
            };
            let value = staged[column].unwrap_or(original);
            *pending |= staged[column].is_some();
            *writable &= !deleted && editable(original) && editable(value);
            if let Some(first) = *common {
                mixed[column] |= first != value;
            } else {
                *common = Some(value);
            }
        }
    }
    for (column, summary) in summaries.iter_mut().enumerate() {
        if mixed[column] {
            summary.0 = None;
        }
    }
    summaries
}

impl Rustable {
    pub(super) fn bulk_inspector(&mut self, ui: &mut egui::Ui, search: &str) {
        let grid = self.grid();
        let staged_table = self.staged_table().map(str::to_owned);
        let edit_columns = self.edit_columns().to_vec();
        let indices: Vec<_> = self.row_selection.rows.iter().copied().collect();
        let writable = self.can_delete_rows();
        let summaries = field_summaries(
            &grid,
            &indices,
            &self.changes,
            staged_table.as_deref(),
        );
        ui.strong(format!("{} rows selected", indices.len()));
        let mut start = None;
        let mut constant = None;
        let mut finish = false;
        egui::ScrollArea::vertical()
            .id_salt(("bulk-inspector", search))
            .show(ui, |ui| {
                compact_inspector_spacing(ui);
                for (column, name) in crate::search::Query::new(search).ranked(grid.columns.iter().enumerate(), |(_, name)| (name.as_str(), "")) {
                    let (common, pending, values_writable) = summaries[column];
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(name)
                                .size(11.0)
                                .color(ui.visuals().weak_text_color()),
                        );
                        if let Some(metadata) = edit_columns.get(column)
                        {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(metadata.data_type.to_lowercase())
                                                .size(10.0)
                                                .color(ui.visuals().weak_text_color()),
                                        )
                                        .truncate(),
                                    );
                                },
                            );
                        }
                    });
                    let constant_enabled = writable
                        && values_writable
                        && edit_columns.get(column).is_some_and(|c| !c.generated);
                    let active = self.edit.as_ref().is_some_and(|e| {
                        !e.bulk_rows.is_empty() && staged_table.as_deref() == Some(e.table.as_str()) && e.column == *name
                    });
                    egui::Frame::new()
                        .fill(if pending || active {
                            pending_fill(ui)
                        } else {
                            ui.visuals().extreme_bg_color
                        })
                        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                        .corner_radius(4.0)
                        .inner_margin(1.0)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), 22.0),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.add_enabled_ui(constant_enabled, |ui| {
                                        field_value_menu(ui, |ui| {
                                            let editable = edit_columns
                                                .get(column)
                                                .is_some_and(|c| !c.generated);
                                            let nullable = edit_columns
                                                .get(column)
                                                .is_some_and(|c| !c.not_null);
                                            if ui
                                                .add_enabled(
                                                    editable && nullable,
                                                    egui::Button::new("Set NULL"),
                                                )
                                                .clicked()
                                            {
                                                constant = Some((column, None));
                                                ui.close();
                                            }
                                            if ui
                                                .add_enabled(
                                                    editable,
                                                    egui::Button::new("Set empty text"),
                                                )
                                                .clicked()
                                            {
                                                constant = Some((column, Some(String::new())));
                                                ui.close();
                                            }
                                            if let Some(kind) = edit_columns.get(column).and_then(|c| crate::date_picker::kind(&c.data_type))
                                                && ui.add_enabled(editable, egui::Button::new(now_label(kind))).clicked()
                                            {
                                                constant = Some((column, Some(crate::date_picker::now_text(kind))));
                                                ui.close();
                                            }
                                        })
                                        .on_hover_text("Set value for selected rows");
                                    });
                                    ui.with_layout(
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            if let Some(value) = common
                                                && ui
                                                    .push_id(("bulk-copy", name), clipboard_button)
                                                    .inner
                                                    .clicked()
                                            {
                                                ui.ctx().copy_text(db::display(value));
                                            }
                                            if active {
                                                finish |= inline_value(ui, &mut self.edit);
                                            } else {
                                                let text = common
                                                    .map(|v| db::preview(v, 4096))
                                                    .unwrap_or_else(|| "Multiple values".into());
                                                let response = inspector_value_preview(ui, &text, matches!(common, Some(Value::Null)));
                                                if response.clicked() && constant_enabled {
                                                    start = Some(column);
                                                }
                                            }
                                        },
                                    );
                                },
                            );
                        });
                    ui.add_space(1.0);
                }
            });
        if finish {
            self.finish_cell_edit();
        }
        if let Some(column) = start {
            self.start_bulk_edit(column);
        }
        if let Some((column, text)) = constant {
            self.set_bulk_constant(column, text);
        }
    }

    /// Stages one value on every selected row: NULL for `None`, else the text.
    pub(super) fn set_bulk_constant(&mut self, column: usize, text: Option<String>) {
        let null = text.is_none();
        if !self.can_delete_rows()
            || self
                .edit_columns()
                .get(column)
                .is_none_or(|c| c.generated || (null && c.not_null))
            || !self.finish_cell_edit()
        {
            return;
        }
        self.start_bulk_edit(column);
        let Some(edit) = self.edit.as_mut() else {
            return;
        };
        // A constant is an explicit edit, even when a mixed-value draft starts empty.
        edit.kind = if null { 0 } else { 3 };
        edit.text = text.unwrap_or_default();
        edit.bulk_changed = true;
        self.finish_cell_edit();
    }

    pub(super) fn start_bulk_edit(&mut self, column: usize) {
        if !self.can_delete_rows() || !self.finish_cell_edit() {
            return;
        }
        let indices: Vec<_> = self.row_selection.rows.iter().copied().collect();
        if !self.bulk_field_writable(&indices, column) {
            return;
        }
        let grid = self.grid();
        let table = self.edit_table().to_owned();
        let Some(name) = self.edit_columns().get(column).map(|c| c.name.clone()) else { return };
        let (common, _) = field_summary(&grid, &indices, column, &self.changes, Some(&table));
        if matches!(common, Some(Value::Blob(_)))
            || matches!(common, Some(Value::Text(t)) if t.len()>65536)
        {
            return;
        }
        let text = common
            .filter(|v| !matches!(v, Value::Null))
            .map(db::display)
            .unwrap_or_default();
        let kind = match common {
            Some(Value::Integer(_)) => 1,
            Some(Value::Real(_)) => 2,
            Some(Value::Null) => 0,
            _ => 3,
        };
        let row = grid.rows[indices[0]].clone();
        self.edit = Some(CellEdit {
            bulk_rows: indices
                .iter()
                .map(|i| (grid.rows[*i].clone(), grid.rows[*i][column].clone()))
                .collect(),
            bulk_changed: false,
            insert_id: None,
            advance: 0,
            table,
            original_value: row[column].clone(),
            row,
            column: name,
            text,
            kind,
            focus: true,
            inspector: true,
        });
    }

    fn bulk_field_writable(&self, indices: &[usize], column: usize) -> bool {
        indices.len() >= 2
            && self.edit_columns().get(column).is_some_and(|c| !c.generated)
            && indices.iter().all(|index| {
                self.grid().rows.get(*index).is_some_and(|row| {
                    !row_is_deleted(&self.changes, self.edit_table(), row)
                        && row.get(column).is_some_and(|value| {
                            !matches!(value, Value::Blob(_))
                                && !matches!(value,Value::Text(t) if t.len()>65536)
                        })
                        && field_summary(
                            &self.grid(),
                            &[*index],
                            column,
                            &self.changes,
                            Some(self.edit_table()),
                        )
                        .0
                        .is_some_and(|value| {
                            !matches!(value, Value::Blob(_))
                                && !matches!(value,Value::Text(t) if t.len()>65536)
                        })
                })
            })
    }

    fn advance_bulk_field(&mut self, column: usize, direction: i8) {
        if direction == 0 || !self.can_delete_rows() {
            return;
        }
        let indices: Vec<_> = self.row_selection.rows.iter().copied().collect();
        let visible = crate::search::Query::new(&self.field_search)
            .ranked(self.edit_columns().iter().enumerate(), |(_, column)| (&column.name, ""));
        let position = visible.iter().position(|(index, _)| *index == column)
            .map_or(if direction > 0 { -1 } else { visible.len() as isize }, |index| index as isize);
        let mut next = position + direction.signum() as isize;
        while next >= 0 && (next as usize) < visible.len() {
            let index = visible[next as usize].0;
            if self.bulk_field_writable(&indices, index) {
                self.start_bulk_edit(index);
                return;
            }
            next += direction.signum() as isize;
        }
    }

    pub(super) fn finish_bulk_edit(&mut self, mut edit: CellEdit) -> bool {
        // Navigation must not silently move editing to a newly selected group.
        let next = (edit.advance != 0
            && edit.table == self.edit_table()
            && self.row_selection.rows.len() == edit.bulk_rows.len()
            && self
                .row_selection
                .rows
                .iter()
                .zip(&edit.bulk_rows)
                .all(|(index, (row, _))| self.grid().rows.get(*index) == Some(row)))
        .then(|| self.edit_columns().iter().position(|c| c.name == edit.column))
        .flatten();
        if !edit.bulk_changed {
            if let Some(column) = next {
                self.advance_bulk_field(column, edit.advance);
            }
            return true;
        }
        let result = parse_cell_value(&edit).and_then(|value| {
            if !self.can_delete_rows() || edit.table != self.edit_table()
                || self.edit_columns().iter().find(|c| c.name == edit.column).is_none_or(|c| c.generated || (c.not_null && matches!(value,Value::Null)))
                || edit.bulk_rows.iter().any(|(row,_)| row_is_deleted(&self.changes, &edit.table, row)) {
                Err("These selected rows cannot be edited in the current state. Restore deleted rows or cancel the draft.".into())
            } else { Ok(value) }
        });
        match result {
            Ok(value) => {
                for (row, original_value) in edit.bulk_rows {
                    if self.changes.iter().any(|change| matches!(change, Change::Cell { table, column, original, value: staged } if table == &edit.table && column == &edit.column && original == &row && staged == &value)) { continue; }
                    self.changes.retain(|change| !matches!(change, Change::Cell { table, column, original, .. } if table == &edit.table && column == &edit.column && original == &row));
                    if original_value != value {
                        self.changes.push(Change::Cell {
                            table: edit.table.clone(),
                            column: edit.column.clone(),
                            original: row,
                            value: value.clone(),
                        });
                    }
                }
                self.record_changes();
                self.review = None;
                if let Some(column) = next {
                    self.advance_bulk_field(column, edit.advance);
                }
                true
            }
            Err(error) => {
                self.error = Some(error);
                edit.advance = 0;
                edit.focus = true;
                self.edit = Some(edit);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> Rustable {
        let ctx = egui::Context::default();
        // UI-only fixture: do not race a queued demo-open command against paging.
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        app.engine = "SQLite".into();
        app.safe = false;
        app.view = View::Data;
        app.browse.table = "t".into();
        app.objects = vec![Object {
            schema: None,
            display_name: None,
            metadata_error: None,
            columns: vec![],
            name: "t".into(),
            kind: "table".into(),
            ddl: String::new(),
        }];
        app.columns = vec![Column {
            name: "id".into(),
            data_type: "INTEGER".into(),
            pk: true,
            not_null: true,
            default: String::new(),
            generated: false,
        }];
        app.table = Arc::new(Grid {
            columns: vec!["id".into()],
            rows: vec![
                vec![Value::Integer(1)],
                vec![Value::Integer(2)],
                vec![Value::Integer(3)],
            ],
            ..Default::default()
        });
        app
    }
    #[test]
    fn paging_keeps_arrow_ownership_without_selected_rows() {
        let ctx = egui::Context::default();
        let grid_id = egui::Id::new("page-grid-no-selection");
        let mut app = app();
        app.browse.offset = db::PAGE_SIZE;
        app.row_selection.clear();
        let frame = |app: &mut Rustable, key: Option<egui::Key>| {
            let _ = ctx.run(egui::RawInput {
                events: key.map(|key| egui::Event::Key {
                    key, physical_key: None, pressed: true, repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }).into_iter().collect(), ..Default::default()
            }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = ui.button("Previous control");
                    let response = ui.interact(egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(200.0, 200.0)), grid_id, egui::Sense::focusable_noninteractive());
                    app.page_grid_shortcut(&response);
                    let _ = ui.button("Next control");
                });
            });
        };
        frame(&mut app, None);
        ctx.memory_mut(|m| m.request_focus(grid_id));
        frame(&mut app, None);
        frame(&mut app, Some(egui::Key::ArrowLeft));
        assert_eq!(app.browse.offset, 0);
        assert!(matches!(app.pending, Some(Pending::Table)));
    }
    #[test]
    fn page_shortcuts_only_belong_to_enabled_table_grid() {
        let ctx = egui::Context::default();
        let grid_id = egui::Id::new("test-page-grid");
        let mut app = app();
        app.browse.offset = db::PAGE_SIZE;
        let frame = |app: &mut Rustable, focus: bool, enabled: bool| {
            ctx.memory_mut(|m| m.request_focus(if focus { grid_id } else { egui::Id::new("text-field") }));
            let _ = ctx.run(egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::ArrowLeft, physical_key: None, pressed: true,
                    repeat: false, modifiers: egui::Modifiers::COMMAND,
                }], ..Default::default()
            }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if !enabled { ui.disable(); }
                    let response = ui.interact(ui.available_rect_before_wrap(), grid_id, egui::Sense::focusable_noninteractive());
                    app.page_grid_shortcut(&response);
                });
            });
        };
        frame(&mut app, false, true);
        assert_eq!(app.browse.offset, db::PAGE_SIZE);
        frame(&mut app, true, false);
        assert_eq!(app.browse.offset, db::PAGE_SIZE);
        app.history = true;
        frame(&mut app, true, true);
        assert_eq!(app.browse.offset, db::PAGE_SIZE);
        app.history = false;
        app.view = View::Query;
        frame(&mut app, true, true);
        assert_eq!(app.browse.offset, db::PAGE_SIZE);
        app.view = View::Data;
        frame(&mut app, true, true);
        assert_eq!(app.browse.offset, 0);
        assert!(matches!(app.pending, Some(Pending::Table)));
        assert!(app.focus_page_grid);
    }
    #[test]
    fn postgres_bulk_edits_and_deletes_stage_without_changing_loaded_values() {
        let mut app = app();
        app.engine = "PostgreSQL".into();
        app.row_selection.rows.extend([0, 2]);
        app.start_bulk_edit(0);
        let draft = app.edit.as_mut().unwrap();
        draft.text = "9".into();
        draft.kind = 1;
        draft.bulk_changed = true;
        assert!(app.finish_cell_edit());
        assert_eq!(app.changes.len(), 2);
        assert_eq!(app.table.rows[0][0], Value::Integer(1));
        app.set_selected_rows_deleted(true);
        assert!(
            app.changes
                .iter()
                .any(|change| matches!(change, Change::DeleteRow { .. }))
        );
        assert!(app.review.is_none());
    }

    #[test]
    fn drag_cells_and_command_copy_use_plain_displayed_values() {
        let mut app = app();
        app.connections = false;
        let table = Arc::make_mut(&mut app.table);
        table.columns.extend(["a".into(), "b".into()]);
        for (i, row) in table.rows.iter_mut().enumerate() {
            row.extend([Value::Text(format!("start-{i}")), Value::Text(format!("end-{i}"))]);
        }
        let ctx = egui::Context::default();
        let frame = |app: &mut Rustable, events| ctx.run(egui::RawInput {
            events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            ..Default::default()
        }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| app.data_grid(ui)); });
        fn position(shape: &egui::Shape, text: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(t) if t.galley.job.text == text => Some(t.pos + t.galley.size() * 0.5),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| position(s, text)),
                _ => None,
            }
        }
        frame(&mut app, vec![]);
        let output = frame(&mut app, vec![]);
        let point = |text| output.shapes.iter().find_map(|s| position(&s.shape, text)).unwrap();
        let start = point("end-2");
        let end = point("start-0");
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(&mut app, vec![egui::Event::PointerMoved(start), button(start, true)]);
        frame(&mut app, vec![egui::Event::PointerMoved(end)]);
        frame(&mut app, vec![button(end, false)]);
        assert_eq!(app.row_selection.cell_columns, Some((1, 2)));
        assert_eq!(app.row_selection.rows.iter().copied().collect::<Vec<_>>(), vec![0, 1, 2]);
        assert!(app.edit.is_none());
        app.changes.push(Change::Cell { table: "t".into(), column: "a".into(), original: app.table.rows[0].clone(), value: Value::Text("a, \"quoted\"".into()) });
        for event in [egui::Event::Copy, egui::Event::Key {
            key: egui::Key::C, physical_key: None, pressed: true, repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }] {
            let output = frame(&mut app, vec![event]);
            assert!(matches!(&output.platform_output.commands[..], [egui::OutputCommand::CopyText(text)] if text == "a, \"quoted\"\tend-0\nstart-1\tend-1\nstart-2\tend-2"));
        }
        // Dragging the counter gutter selects every column, even if the
        // pointer ends over a data cell. Copy follows that full-row range.
        let gutter = point("1");
        frame(&mut app, vec![egui::Event::PointerMoved(gutter), button(gutter, true)]);
        frame(&mut app, vec![egui::Event::PointerMoved(start)]);
        frame(&mut app, vec![button(start, false)]);
        assert_eq!(app.row_selection.cell_columns, None);
        assert_eq!(app.row_selection.rows.iter().copied().collect::<Vec<_>>(), vec![0, 1, 2]);
        let output = frame(&mut app, vec![egui::Event::Copy]);
        assert!(matches!(&output.platform_output.commands[..], [egui::OutputCommand::CopyText(text)] if text == "1\ta, \"quoted\"\tend-0\n2\tstart-1\tend-1\n3\tstart-2\tend-2"));
    }

    #[test]
    fn grid_focus_survives_arrow_then_shift_arrow() {
        let mut app = app();
        app.connections = false;
        let table = Arc::make_mut(&mut app.table);
        table.columns.push("description".into());
        for row in &mut table.rows {
            row.push(Value::Text("cell text".into()));
        }
        let ctx = egui::Context::default();
        fn frame(
            app: &mut Rustable,
            ctx: &egui::Context,
            events: Vec<egui::Event>,
        ) -> egui::FullOutput {
            ctx.run(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.data_grid(ui));
                },
            )
        }
        fn position(shape: &egui::Shape) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(t) if t.galley.job.text == "1" => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(position),
                _ => None,
            }
        }
        frame(&mut app, &ctx, vec![]);
        let output = frame(&mut app, &ctx, vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|s| position(&s.shape))
            .unwrap();
        // Click the visible row-number label, not empty cell padding.
        for pressed in [true, false] {
            frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(app.selected, Some(0));
        frame(&mut app, &ctx, vec![]);
        let key = |key, shift| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                shift,
                ..Default::default()
            },
        };
        // Separate this second click from the gutter click so egui does not
        // classify the sequence as a double-click and deliberately enter edit.
        for _ in 0..40 {
            frame(&mut app, &ctx, vec![]);
        }
        let text_position = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "cell text" => {
                    Some(text.pos + egui::vec2(3.0, 3.0))
                }
                _ => None,
            })
            .expect("second-column cell label");
        for pressed in [true, false] {
            frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(text_position),
                    egui::Event::PointerButton {
                        pos: text_position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(
            app.row_selection.cell_column, 1,
            "cell press tracks its column"
        );

        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![key(egui::Key::Tab, true)]);
        assert_eq!(
            app.row_selection.cell_column, 0,
            "Shift+Tab moves left without losing focus"
        );
        frame(&mut app, &ctx, vec![key(egui::Key::Tab, false)]);
        assert_eq!(
            app.row_selection.cell_column, 1,
            "Tab moves right without losing focus"
        );
        frame(&mut app, &ctx, vec![key(egui::Key::Enter, false)]);
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![key(egui::Key::Tab, false)]);
        assert!(
            app.edit.is_some(),
            "editing Tab advances to the next editable cell"
        );
        assert_eq!(app.row_selection.focused, Some(1));
        assert_eq!(app.row_selection.cell_column, 0);
        assert_eq!(
            app.row_selection.rows.iter().copied().collect::<Vec<_>>(),
            vec![1]
        );
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![key(egui::Key::Tab, true)]);
        assert_eq!(
            app.row_selection.focused,
            Some(0),
            "editing Shift+Tab tracks its previous editable cell"
        );
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![key(egui::Key::Tab, false)]);
        assert_eq!(app.row_selection.focused, Some(1));
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![key(egui::Key::Escape, false)]);
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowRight, false)]);
        assert_eq!(
            app.row_selection.cell_column, 1,
            "Escape after editing Tab restores arrows"
        );
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowUp, false)]);
        assert_eq!(app.selected, Some(0));

        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter, false)]);
        assert!(app.edit.is_some(), "Enter edits the highlighted cell");
        frame(&mut app, &ctx, vec![]);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Text("changed draft".into())],
        );
        frame(&mut app, &ctx, vec![key(egui::Key::Escape, false)]);
        assert!(app.edit.is_none(), "Escape cancels editing");
        assert!(app.changes.is_empty(), "Escape must not stage the draft");
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowLeft, false)]);
        assert_eq!(
            app.row_selection.cell_column, 0,
            "Escape restores grid keyboard ownership"
        );
        // Completing an edit must restore navigation just like cancelling it.
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowRight, false)]);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter, false)]);
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![egui::Event::Text("pending".into())]);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter, false)]);
        assert!(app.edit.is_none());
        assert_eq!(app.changes.len(), 1);
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowLeft, false)]);
        assert_eq!(app.row_selection.cell_column, 0, "staging an edit restores arrow navigation");
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowRight, false)]);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter, false)]);
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![egui::Event::Text("more".into())]);
        let next_row = text_position + egui::vec2(0.0, 26.0);
        for pressed in [true, false] {
            frame(&mut app, &ctx, vec![egui::Event::PointerMoved(next_row), egui::Event::PointerButton {
                pos: next_row, button: egui::PointerButton::Primary, pressed,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        assert!(app.edit.is_none(), "clicking another cell stages the editor");
        assert_eq!(app.selected, Some(1));
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowUp, false)]);
        assert_eq!(app.selected, Some(0), "click-away staging restores vertical arrows");
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowLeft, false)]);
        assert_eq!(app.row_selection.cell_column, 0);
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowRight, false)]);
        assert_eq!(app.row_selection.cell_column, 1);
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowRight, false)]);
        assert_eq!(app.row_selection.cell_column, 1, "right edge must clamp");
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowLeft, false)]);
        assert_eq!(app.row_selection.cell_column, 0);
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowLeft, false)]);
        assert_eq!(app.row_selection.cell_column, 0, "left edge must clamp");
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown, false)]);
        assert_eq!(app.selected, Some(1));
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown, true)]);
        frame(&mut app, &ctx, vec![]);
        assert_eq!(
            app.row_selection.rows.iter().copied().collect::<Vec<_>>(),
            vec![1, 2]
        );
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowUp, true)]);
        assert_eq!(app.selected, Some(1));
        let mut repeated = key(egui::Key::ArrowDown, true);
        if let egui::Event::Key { repeat, .. } = &mut repeated {
            *repeat = true;
        }
        frame(
            &mut app,
            &ctx,
            vec![
                key(egui::Key::ArrowUp, false),
                key(egui::Key::ArrowDown, true),
                repeated,
                key(egui::Key::ArrowUp, true),
            ],
        );
        assert_eq!(
            app.row_selection.rows.iter().copied().collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(app.row_selection.focused, Some(1));
    }

    #[test]
    fn row_arrows_leave_events_for_cell_editors_popups_and_other_focus() {
        let mut app = app();
        app.connections = false;
        app.selected = app.row_selection.click(0, false, false, None);
        let ctx = egui::Context::default();
        let fire = |app: &mut Rustable, focus: bool, popup: bool| {
            let mut remains = false;
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::ArrowDown,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let response = ui.allocate_response(
                            egui::vec2(200.0, 100.0),
                            egui::Sense::focusable_noninteractive(),
                        );
                        if focus {
                            response.request_focus();
                        } else {
                            response.surrender_focus();
                        }
                        if popup {
                            egui::Popup::open_id(ctx, egui::Id::new("column-choice"));
                        }
                        app.navigate_rows_shortcut(&response, 0);
                        remains = ctx.input(|input| {
                            input.events.iter().any(|event| {
                                matches!(
                                    event,
                                    egui::Event::Key {
                                        key: egui::Key::ArrowDown,
                                        pressed: true,
                                        ..
                                    }
                                )
                            })
                        });
                    });
                },
            );
            remains
        };
        assert!(
            fire(&mut app, false, false),
            "other focused controls own their arrows"
        );
        app.row_selection.rows.insert(1);
        app.start_bulk_edit(0);
        assert!(app.edit.is_some());
        assert!(fire(&mut app, true, false), "cell editor owns its arrows");
        app.edit = None;
        assert!(
            fire(&mut app, true, true),
            "open choice popup owns its arrows"
        );
        assert_eq!(app.row_selection.focused, Some(0));
    }

    #[test]
    fn import_dialogs_keep_grid_shortcuts_and_selection_untouched() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, b"id\n1\n").unwrap();
        for review in [false, true] {
            let mut app = app();
            app.connections = false;
            app.selected = app.row_selection.click(0, false, false, None);
            let preview = db::csv_preview::load(
                file.path(), Default::default(), None,
                &std::sync::atomic::AtomicBool::new(false),
            ).unwrap();
            if review {
                app.csv_review = Some((db::csv_import::Request {
                    schema: None,
                    source: preview.source.clone().unwrap(), options: preview.options,
                    table: "t".into(), columns: vec![], destinations: vec![],
                    empty_as_null: false, postgres_inserts: false,
                }, vec![]));
            } else {
                app.csv_preview = Some(preview);
            }
            let ctx = egui::Context::default();
            let keys = [
                (egui::Key::ArrowDown, egui::Modifiers::NONE),
                (egui::Key::Tab, egui::Modifiers::NONE),
                (egui::Key::A, egui::Modifiers::COMMAND),
                (egui::Key::Backspace, egui::Modifiers::NONE),
            ];
            let mut events = keys.into_iter().map(|(key, modifiers)| egui::Event::Key {
                key, physical_key: None, pressed: true, repeat: false, modifiers,
            }).collect::<Vec<_>>();
            events.push(egui::Event::Copy);
            let expected = events.clone();
            let output = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response = ui.allocate_response(
                        egui::vec2(200.0, 100.0), egui::Sense::focusable_noninteractive(),
                    );
                    // Simulate the first dialog frame while grid focus still exists.
                    response.request_focus();
                    app.navigate_rows_shortcut(&response, 0);
                    app.select_all_rows_shortcut(ctx, Some(0));
                    app.copy_rows_shortcut(ctx, true);
                    assert!(!app.row_delete_shortcut(ctx, true));
                    assert_eq!(ctx.input(|i| i.events.clone()), expected);
                });
            });
            assert!(output.platform_output.commands.is_empty());
            assert_eq!(app.row_selection.rows.iter().copied().collect::<Vec<_>>(), vec![0]);
            assert_eq!(app.row_selection.cell_column, 0);
            assert_eq!(app.selected, Some(0));
            assert!(app.changes.is_empty());
        }
    }

    #[test]
    fn shared_field_summaries_preserve_staged_values_and_editability() {
        let mut app = app();
        app.row_selection.rows.extend([0, 2]);
        app.set_bulk_constant(0, Some(String::new()));
        for table in [Some("t"), None, Some("unrelated")] {
            let summaries = field_summaries(&app.table, &[0, 2], &app.changes, table);
            for (column, summary) in summaries.iter().enumerate() {
                assert_eq!(
                    (summary.0, summary.1),
                    field_summary(&app.table, &[0, 2], column, &app.changes, table)
                );
                if table == Some("t") {
                    assert_eq!(summary.2, app.bulk_field_writable(&[0, 2], column));
                }
            }
        }
        app.changes.push(Change::DeleteRow {
            table: "t".into(),
            original: app.table.rows[2].clone(),
        });
        assert!(!field_summaries(&app.table, &[0, 2], &app.changes, Some("t"))[0].2);
        assert!(!field_summaries(&app.table, &[0, 99], &app.changes, Some("t"))[0].2);
        app.changes.clear();
        app.changes.push(Change::Cell {
            table: "t".into(),
            column: "id".into(),
            original: app.table.rows[0].clone(),
            value: Value::Blob(vec![1]),
        });
        assert!(!field_summaries(&app.table, &[0, 2], &app.changes, Some("t"))[0].2);
    }

    #[test]
    #[ignore = "manual release performance measurement"]
    fn benchmark_bulk_field_summaries() {
        let grid = Grid {
            columns: (0..12).map(|i| format!("c{i}")).collect(),
            rows: (0..500)
                .map(|i| (0..12).map(|c| Value::Integer(i * 12 + c)).collect())
                .collect(),
            ..Default::default()
        };
        let indices: Vec<_> = (0..500).collect();
        let changes: Vec<_> = (0..500)
            .map(|i| Change::Cell {
                table: "t".into(),
                column: grid.columns[i % 12].clone(),
                original: grid.rows[i].clone(),
                value: Value::Text("pending".into()),
            })
            .collect();
        let expected: Vec<_> = (0..12)
            .map(|c| field_summary(&grid, &indices, c, &changes, Some("t")))
            .collect();
        let actual = field_summaries(&grid, &indices, &changes, Some("t"));
        assert_eq!(
            expected,
            actual.iter().map(|s| (s.0, s.1)).collect::<Vec<_>>()
        );
        for shared in [false, true] {
            let mut samples = Vec::new();
            for _ in 0..25 {
                let start = Instant::now();
                if shared {
                    std::hint::black_box(field_summaries(&grid, &indices, &changes, Some("t")));
                } else {
                    for c in 0..12 {
                        std::hint::black_box(field_summary(
                            &grid,
                            &indices,
                            c,
                            &changes,
                            Some("t"),
                        ));
                    }
                }
                samples.push(start.elapsed());
            }
            samples.sort();
            eprintln!(
                "shared={shared} 500 rows/12 columns/500 pending cells: median={:?} p95={:?}",
                samples[12], samples[23]
            );
        }
    }

    #[test]
    fn select_all_shortcut_is_focus_scoped_and_readonly_safe() {
        let mut app = app();
        app.safe = true;
        app.row_selection.rows.insert(1);
        app.selected = Some(1);
        let ctx = egui::Context::default();
        let frame = |app: &mut Rustable, focused| {
            let _ = ctx.run(
                egui::RawInput {
                    modifiers: egui::Modifiers::COMMAND,
                    events: vec![egui::Event::Key {
                        key: egui::Key::A,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    }],
                    ..Default::default()
                },
                |ctx| app.select_all_rows_shortcut(ctx, focused),
            );
        };
        frame(&mut app, None);
        assert_eq!(app.row_selection.rows.len(), 1);
        app.connections = true;
        frame(&mut app, Some(1));
        assert_eq!(app.row_selection.rows.len(), 1);
        app.connections = false;
        frame(&mut app, Some(1));
        assert_eq!(app.row_selection.copy_indices(1), vec![0, 1, 2]);
        assert_eq!(app.selected, None);
        assert!(app.changes.is_empty());
    }

    #[test]
    fn grid_copy_uses_native_event_and_leaves_text_editors_alone() {
        let mut app = app();
        app.safe = true;
        app.row_selection.rows.extend([0, 2]);
        app.changes.push(Change::Cell {
            table: "t".into(),
            column: "id".into(),
            original: app.table.rows[0].clone(),
            value: Value::Integer(9),
        });
        let ctx = egui::Context::default();
        let frame = |app: &mut Rustable, focused| {
            let mut retained = false;
            let output = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Copy],
                    ..Default::default()
                },
                |ctx| {
                    app.copy_rows_shortcut(ctx, focused);
                    retained =
                        ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)));
                },
            );
            (output.platform_output.commands, retained)
        };
        let (commands, retained) = frame(&mut app, false);
        assert!(commands.is_empty());
        assert!(retained);
        let (commands, retained) = frame(&mut app, true);
        assert!(!retained);
        assert!(matches!(&commands[..], [egui::OutputCommand::CopyText(text)] if text == "9\n3"));
        app.connections = true;
        let (commands, retained) = frame(&mut app, true);
        assert!(commands.is_empty());
        assert!(retained);
        app.connections = false;
        app.row_selection.rows.insert(99);
        assert!(frame(&mut app, true).0.is_empty());
        assert!(app.error.is_some());
    }

    #[test]
    fn keyboard_delete_requires_grid_focus_and_no_editor_or_window() {
        let mut app = app();
        app.row_selection.rows.extend([0, 2]);
        let ctx = egui::Context::default();
        let fire = |app: &Rustable, focused, modifiers| {
            let mut handled = false;
            let _ = ctx.run(
                egui::RawInput {
                    modifiers,
                    events: vec![egui::Event::Key {
                        key: egui::Key::Backspace,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    handled = app.row_delete_shortcut(ctx, focused);
                },
            );
            handled
        };
        assert!(!fire(&app, false, egui::Modifiers::NONE));
        assert!(!fire(&app, true, egui::Modifiers::COMMAND));
        app.connections = true;
        assert!(!fire(&app, true, egui::Modifiers::NONE));
        app.connections = false;
        app.start_bulk_edit(0);
        assert!(!fire(&app, true, egui::Modifiers::NONE));
        app.edit = None;
        app.safe = true;
        assert!(!fire(&app, true, egui::Modifiers::NONE));
        app.safe = false;
        assert!(fire(&app, true, egui::Modifiers::NONE));
        app.set_selected_rows_deleted(true);
        assert_eq!(app.changes.len(), 2);
        assert!(row_is_deleted(&app.changes, "t", &app.table.rows[0]));
        assert!(!row_is_deleted(&app.changes, "t", &app.table.rows[1]));
        assert!(row_is_deleted(&app.changes, "t", &app.table.rows[2]));
        assert!(!fire(&app, true, egui::Modifiers::NONE));
    }

    #[test]
    fn mixed_selection_delete_is_idempotent_and_restore_preserves_edits() {
        let mut app = app();
        let first = app.table.rows[0].clone();
        app.changes.push(Change::Cell {
            table: "t".into(),
            column: "id".into(),
            original: first.clone(),
            value: Value::Integer(10),
        });
        toggle_row_deletion(&mut app.changes, "t", first);
        app.row_selection.rows.extend([0, 2]);
        assert!(!app.selected_rows_deleted());
        app.set_selected_rows_deleted(true);
        assert_eq!(
            app.changes
                .iter()
                .filter(|c| matches!(c, Change::DeleteRow { .. }))
                .count(),
            2
        );
        assert!(!row_is_deleted(&app.changes, "t", &app.table.rows[1]));
        assert!(app.row_selection.rows.is_empty());
        app.row_selection.rows.extend([0, 2]);
        assert!(app.selected_rows_deleted());
        app.set_selected_rows_deleted(false);
        assert_eq!(app.changes.len(), 1);
        assert!(matches!(
            &app.changes[0],
            Change::Cell {
                value: Value::Integer(10),
                ..
            }
        ));
    }
    #[test]
    fn invalid_active_draft_and_busy_worker_leave_selection_unchanged() {
        let mut app = app();
        app.row_selection.rows.extend([0, 1]);
        app.pending = Some(Pending::Apply);
        app.set_selected_rows_deleted(true);
        assert!(app.changes.is_empty());
        assert_eq!(app.row_selection.rows.len(), 2);
        app.pending = None;
        app.edit = Some(CellEdit {
            bulk_rows: Vec::new(),
            bulk_changed: false,
            insert_id: None,
            advance: 0,
            table: "t".into(),
            row: app.table.rows[0].clone(),
            original_value: Value::Integer(1),
            column: "id".into(),
            text: "not an integer".into(),
            kind: 1,
            focus: false,
            inspector: false,
        });
        app.set_selected_rows_deleted(true);
        assert!(app.changes.is_empty());
        assert!(app.edit.is_some());
        assert!(app.error.is_some());
        assert_eq!(app.row_selection.rows.len(), 2);
    }
    #[test]
    fn refuses_unsafe_targets_and_stale_selection_without_partial_staging() {
        let mut app = app();
        app.row_selection.rows.extend([0, 99]);
        app.set_selected_rows_deleted(true);
        assert!(app.changes.is_empty());
        app.row_selection.rows.remove(&99);
        app.safe = true;
        app.set_selected_rows_deleted(true);
        assert!(app.changes.is_empty());
        app.safe = false;
        app.view = View::Query;
        app.set_selected_rows_deleted(true);
        assert!(app.changes.is_empty());
        app.view = View::Data;
        app.columns[0].pk = false;
        app.set_selected_rows_deleted(true);
        assert!(app.changes.is_empty());
    }
    #[test]
    fn bulk_inspector_edit_targets_captured_rows_and_uses_pending_values() {
        let mut app = app();
        app.row_selection.rows.extend([0, 2]);
        app.start_bulk_edit(0);
        let edit = app.edit.as_mut().unwrap();
        assert_eq!(edit.bulk_rows.len(), 2);
        edit.text = "9".into();
        edit.bulk_changed = true;
        edit.kind = 1;
        app.row_selection.rows.clear();
        app.row_selection.rows.insert(1);
        assert!(app.finish_cell_edit());
        assert_eq!(app.changes.len(), 2);
        let (common, pending) = field_summary(&app.table, &[0, 2], 0, &app.changes, Some("t"));
        assert_eq!(common, Some(&Value::Integer(9)));
        assert!(pending);
        assert_eq!(
            field_summary(&app.table, &[1], 0, &app.changes, Some("t")),
            (Some(&Value::Integer(2)), false)
        );
        assert_eq!(
            field_summary(&app.table, &[0, 2], 0, &app.changes, None),
            (None, false)
        );
    }

    #[test]
    fn untouched_mixed_values_are_not_overwritten_and_invalid_bulk_edit_is_retained() {
        let mut app = app();
        app.row_selection.rows.extend([0, 2]);
        app.start_bulk_edit(0);
        assert!(app.finish_cell_edit());
        assert!(app.changes.is_empty());
        app.start_bulk_edit(0);
        app.edit.as_mut().unwrap().kind = 1;
        app.edit.as_mut().unwrap().text = "invalid".into();
        app.edit.as_mut().unwrap().bulk_changed = true;
        assert!(!app.finish_cell_edit());
        assert!(app.changes.is_empty());
        assert_eq!(app.edit.as_ref().unwrap().bulk_rows.len(), 2);
        app.edit.as_mut().unwrap().text = "7".into();
        toggle_row_deletion(&mut app.changes, "t", app.table.rows[2].clone());
        assert!(!app.finish_cell_edit());
        assert_eq!(app.changes.len(), 1);
        assert!(matches!(app.changes[0], Change::DeleteRow { .. }));
    }
    #[test]
    fn bulk_constants_distinguish_null_empty_and_enforce_safety() {
        let mut app = app();
        app.row_selection.rows.extend([0, 2]);
        app.columns[0].not_null = true;
        app.set_bulk_constant(0, None);
        assert!(app.changes.is_empty());
        app.columns[0].not_null = false;
        app.set_bulk_constant(0, None);
        assert_eq!(
            field_summary(&app.table, &[0, 2], 0, &app.changes, Some("t")),
            (Some(&Value::Null), true)
        );
        assert_eq!(app.changes.len(), 2);
        app.set_bulk_constant(0, Some(String::new()));
        assert_eq!(
            field_summary(&app.table, &[0, 2], 0, &app.changes, Some("t")),
            (Some(&Value::Text(String::new())), true)
        );
        assert_eq!(app.changes.len(), 2);
        assert_eq!(
            field_summary(&app.table, &[1], 0, &app.changes, Some("t")),
            (Some(&Value::Integer(2)), false)
        );
        app.safe = true;
        app.set_bulk_constant(0, None);
        assert_eq!(
            field_summary(&app.table, &[0, 2], 0, &app.changes, Some("t")).0,
            Some(&Value::Text(String::new()))
        );
    }

    #[test]
    fn bulk_tab_skips_readonly_fields_and_preserves_group_and_staged_values() {
        let mut app = app();
        for name in ["computed", "binary", "large", "name"] {
            let mut column = app.columns[0].clone();
            column.name = name.into();
            column.pk = false;
            column.generated = name == "computed";
            app.columns.push(column);
        }
        let grid = Arc::make_mut(&mut app.table);
        grid.columns = app.columns.iter().map(|c| c.name.clone()).collect();
        for row in &mut grid.rows {
            row.extend([
                Value::Integer(1),
                Value::Blob(vec![1]),
                Value::Text("x".repeat(65537)),
                Value::Text("old".into()),
            ]);
        }
        app.row_selection.rows.extend([0, 2]);
        app.start_bulk_edit(0);
        app.edit.as_mut().unwrap().advance = 1;
        assert!(app.finish_cell_edit());
        assert_eq!(app.edit.as_ref().unwrap().column, "name");
        let draft = app.edit.as_mut().unwrap();
        draft.text = "new".into();
        draft.bulk_changed = true;
        draft.advance = -1;
        assert!(app.finish_cell_edit());
        assert_eq!(app.edit.as_ref().unwrap().column, "id");
        assert_eq!(app.changes.len(), 2);
        app.edit.as_mut().unwrap().advance = 1;
        app.field_search = "name".into();
        assert!(app.finish_cell_edit());
        assert_eq!(app.edit.as_ref().unwrap().text, "new");
        app.edit.as_mut().unwrap().advance = -1;
        assert!(app.finish_cell_edit());
        assert!(app.edit.is_none());
        app.start_bulk_edit(4);
        app.edit.as_mut().unwrap().advance = -1;
        app.row_selection.rows.clear();
        app.row_selection.rows.extend([0, 1]);
        assert!(app.finish_cell_edit());
        assert!(app.edit.is_none());
    }
}
