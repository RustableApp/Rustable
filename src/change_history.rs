//! Undo concerns staged UI intentions only, never previously committed SQL.
//! Retain the current state and reversible changed spans, not full snapshots.
use super::*;

#[derive(Default)]
pub(super) struct ChangeHistory {
    current: Vec<Change>,
    steps: Vec<HistoryStep>,
    position: usize,
}

struct HistoryStep {
    start: usize,
    len: usize,
    replacement: Vec<Change>,
}

impl HistoryStep {
    /// Exchange the stored span with the current one. Calling twice restores
    /// both sides, so undo and redo share the same operation and owned values.
    fn exchange(&mut self, current: &mut Vec<Change>) {
        let replacement = std::mem::take(&mut self.replacement);
        let next_len = replacement.len();
        self.replacement = current
            .splice(self.start..self.start + self.len, replacement)
            .collect();
        self.len = next_len;
    }
}

impl ChangeHistory {
    fn record(&mut self, changes: &[Change]) {
        let prefix = self.current.iter().zip(changes)
            .take_while(|(old, new)| old == new).count();
        if prefix == self.current.len() && prefix == changes.len() {
            return;
        }
        // Never let the suffix overlap the common prefix (including pure
        // insertion/deletion and repeated identical changes).
        let suffix = self.current[prefix..].iter().rev()
            .zip(changes[prefix..].iter().rev())
            .take_while(|(old, new)| old == new).count();
        let new_end = changes.len() - suffix;
        let old_end = self.current.len() - suffix;
        self.steps.truncate(self.position);
        let replacement = self.current
            .splice(prefix..old_end, changes[prefix..new_end].iter().cloned())
            .collect();
        self.steps.push(HistoryStep {
            start: prefix,
            len: new_end - prefix,
            replacement,
        });
        self.position += 1;
    }

    fn undo(&mut self) -> Option<Vec<Change>> {
        if self.position == 0 {
            return None;
        }
        self.position -= 1;
        self.steps[self.position].exchange(&mut self.current);
        Some(self.current.clone())
    }

    fn redo(&mut self) -> Option<Vec<Change>> {
        if self.position == self.steps.len() {
            return None;
        }
        self.steps[self.position].exchange(&mut self.current);
        self.position += 1;
        Some(self.current.clone())
    }

    fn cell_target(&self, redo: bool) -> Option<(Change, Option<String>)> {
        let index = if redo { self.position } else { self.position.checked_sub(1)? };
        let step = self.steps.get(index)?;
        let current = &self.current[step.start..step.start + step.len];
        let change = current.iter().chain(&step.replacement)
            .find(|change| matches!(change, Change::Cell { .. } | Change::InsertRow { .. } | Change::DeleteRow { .. }))?;
        let column = if let Change::InsertRow { id, fields, .. } = change {
            let other = step.replacement.iter().find_map(|change| match change {
                Change::InsertRow { id: other, fields, .. } if other == id => Some(fields),
                _ => None,
            });
            other.and_then(|other| fields.iter().chain(other).find_map(|(name, _)| {
                let before = fields.iter().find(|(field, _)| field == name).map(|(_, value)| value);
                let after = other.iter().find(|(field, _)| field == name).map(|(_, value)| value);
                (before != after).then(|| name.clone())
            }))
        } else { None };
        Some((change.clone(), column))
    }

    fn clear(&mut self) {
        // A successful commit/discard releases the session's history storage.
        *self = Self::default();
    }
}

impl Rustable {
    pub(super) fn record_changes(&mut self) {
        self.keep_table_tab(&self.browse.table.clone());
        self.change_history.record(&self.changes);
        self.review = None;
        self.review_sql.clear();
    }

    pub(super) fn clear_change_history(&mut self) {
        self.change_history.clear();
    }

    fn restore_change_history(&mut self, redo: bool) {
        if self.pending.is_some() || self.edit.is_some() {
            return;
        }
        let changes = if redo {
            self.change_history.redo()
        } else {
            self.change_history.undo()
        };
        let Some(changes) = changes else {
            return;
        };
        // An insert's row index can change when neighboring inserts disappear.
        let selected_insert = self.selected_insert();
        let previous_inserts = self.insert_ids();
        self.changes = changes;
        self.review = None;
        self.review_sql.clear();
        self.rename = None;
        self.add_column = None;
        self.alter_column = None;
        self.index_draft = None;
        if let Some(id) = selected_insert {
            self.selected = self
                .insert_ids()
                .iter()
                .position(|item| *item == id)
                .map(|index| self.table.rows.len() + index);
        } else if self
            .selected
            .is_some_and(|index| index >= self.table.rows.len() + self.insert_ids().len())
        {
            self.selected = None;
        }
        if self.selected.is_none() {
            self.selected = self
                .insert_ids()
                .iter()
                .position(|id| !previous_inserts.contains(id))
                .map(|index| self.table.rows.len() + index);
        }
        self.row_selection.clear();
        self.error = None;
        self.status = if redo {
            "Change redone"
        } else {
            "Change undone"
        }
        .into();
    }

    fn focus_history_cell(&mut self, change: Change) {
        let Change::Cell { table, column, original, original_columns, .. } = change else { return; };
        let grid = self.grid();
        if table != self.edit_table() || !self.can_edit_rows()
            || original_columns.as_ref().is_some_and(|columns| columns != &grid.columns) { return; }
        let Some(row) = grid.rows.iter().position(|row| row == &original) else { return; };
        let Some(col) = grid.columns.iter().position(|name| name == &column) else { return; };
        if self.edit_columns().get(col).is_some_and(|column| column.generated)
            || row_is_deleted(&self.changes, &table, &original) { return; }
        let value = self.changes.iter().find_map(|change| match change {
            Change::Cell { table: t, column: c, original: r, value, original_columns: columns }
                if t == &table && c == &column && r == &original && columns == &original_columns => Some(value),
            _ => None,
        }).unwrap_or(&original[col]);
        if matches!(value, Value::Blob(_)) { return; }
        let text = if matches!(value, Value::Null) { String::new() } else { db::display(value) };
        if text.len() > 65536 { return; }
        self.edit = Some(CellEdit {
            bulk_rows: Vec::new(), bulk_changed: false, undo_initialized: false,
            insert_id: None, advance: 0, table, column, original_value: original[col].clone(),
            kind: match value { Value::Null => 0, Value::Integer(_) => 1, Value::Real(_) => 2, _ => 3 },
            row: original, text, focus: true, inspector: false,
        });
        self.selected = self.row_selection.click(row, false, false, self.selected);
        self.row_selection.cell_column = col;
        self.row_selection.cell_columns = Some((col, col));
        self.row_selection.pending_focus = Some(row);
        self.row_selection.pending_cell_scroll = true;
    }

    pub(super) fn staged_history_shortcuts(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() { return; }
        // Avoid inspecting text history on ordinary rendering/input frames.
        if !ctx.input(|input| input.events.iter().any(|event| matches!(event,
            egui::Event::Key { key: egui::Key::Z, pressed: true, repeat: false, modifiers, .. }
                if modifiers.command && !modifiers.alt && !(modifiers.ctrl && modifiers.mac_cmd)
        ))) { return; }
        let focused = ctx.memory(|memory| memory.focused());
        let key = egui::Id::new((ctx.viewport_id(), "active-cell-editor"));
        let cell_editor = ctx.data(|data| data.get_temp::<egui::Id>(key));
        let state = focused.and_then(|id| TextEdit::load_state(ctx, id));
        if let Some(edit) = &self.edit {
            let starting = edit.focus && !edit.undo_initialized;
            if starting && state.is_some() && focused != cell_editor { return; }
            if !starting && (focused != cell_editor || focused.is_none()) { return; }
            let redo = ctx.input(|input| input.events.iter().any(|event| matches!(event,
                egui::Event::Key { key: egui::Key::Z, pressed: true, modifiers, .. } if modifiers.shift
            )));
            if !starting && state.as_ref().is_some_and(|state| {
                if redo { state.has_text_undo(&edit.text) || state.has_redo(&(state.cursor.char_range().unwrap_or_default(), edit.text.clone())) }
                else { state.has_text_undo(&edit.text) }
            }) { return; }
            // No typing remains to undo. Drop the unmodified editor rather than
            // staging it (which could manufacture a history step or fail parsing).
            self.edit = None;
            if let Some(id) = focused { ctx.memory_mut(|memory| memory.surrender_focus(id)); }
            if self.view == View::Data { self.focus_page_grid = true; }
            else if let Some(id) = self.tabs[self.tab].result_grid_focus {
                ctx.memory_mut(|memory| memory.request_focus(id));
            }
        } else if state.is_some() {
            // SQL, filters and other text inputs retain their independent history.
            return;
        }
        let mut actions = Vec::new();
        ctx.input_mut(|input| {
            input.events.retain(|event| {
                if let egui::Event::Key {
                    key: egui::Key::Z,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = event
                    && modifiers.command
                    && !modifiers.alt
                    && !(modifiers.ctrl && modifiers.mac_cmd)
                {
                    actions.push(modifiers.shift);
                    false
                } else {
                    true
                }
            })
        });
        for redo in actions {
            self.edit = None;
            let target = self.change_history.cell_target(redo);
            self.restore_change_history(redo);
            if let Some((target, column)) = target {
                match target {
                    Change::InsertRow { id, table, .. } if table == self.browse.table && self.view == View::Data => {
                        if let Some(index) = self.insert_ids().iter().position(|item| *item == id) {
                            let row = self.table.rows.len() + index;
                            let col = column.and_then(|name| self.table.columns.iter().position(|c| c == &name)).unwrap_or(0);
                            self.selected = Some(row);
                            self.row_selection.focus_pending(row);
                            self.row_selection.cell_column = col;
                            self.row_selection.cell_columns = Some((col, col));
                            self.row_selection.pending_focus = Some(row);
                            self.row_selection.pending_cell_scroll = true;
                        }
                    }
                    Change::DeleteRow { table, original } if table == self.edit_table() => {
                        if let Some(row) = self.grid().rows.iter().position(|r| r == &original) {
                            self.selected = self.row_selection.click(row, false, false, None);
                            self.row_selection.pending_focus = Some(row);
                        }
                    }
                    target => self.focus_history_cell(target),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(value: &str) -> Change {
        Change::Cell { original_columns: None,
            table: "t".into(),
            column: "name".into(),
            original: vec![Value::Integer(1), Value::Text("original".into())],
            value: Value::Text(value.into()),
        }
    }

    #[test]
    fn ordered_history_reaches_clean_and_redoes_each_intermediate_value() {
        let mut history = ChangeHistory::default();
        let first = vec![cell("first")];
        let second = vec![cell("second")];
        let mut third = second.clone();
        third.push(Change::InsertRow {
            id: 10,
            table: "t".into(),
            fields: Vec::new(),
        });
        history.record(&first);
        history.record(&second);
        history.record(&third);
        assert_eq!(history.undo(), Some(second.clone()));
        assert_eq!(history.undo(), Some(first.clone()));
        assert_eq!(history.undo(), Some(Vec::new()));
        assert_eq!(history.undo(), None);
        assert_eq!(history.redo(), Some(first));
        assert_eq!(history.redo(), Some(second));
        assert_eq!(history.redo(), Some(third));
        assert_eq!(history.redo(), None);
    }

    #[test]
    fn no_op_does_not_consume_undo_and_new_edit_discards_redo() {
        let mut history = ChangeHistory::default();
        history.record(&[cell("one")]);
        history.record(&[cell("one")]);
        history.record(&[cell("two")]);
        history.undo();
        history.record(&[cell("replacement")]);
        assert_eq!(history.redo(), None);
        assert_eq!(history.undo(), Some(vec![cell("one")]));
        assert_eq!(history.undo(), Some(Vec::new()));
    }

    #[test]
    fn committed_save_resets_both_directions_but_failed_save_keeps_history() {
        let mut history = ChangeHistory::default();
        history.record(&[cell("one")]);
        history.record(&[cell("two")]);
        assert_eq!(history.undo(), Some(vec![cell("one")]));
        // A failed plan/apply does not mutate history. Retrying at this point
        // uses exactly "one", not an action from the discarded future.
        assert_eq!(history.current, vec![cell("one")]);
        history.clear();
        assert_eq!(history.undo(), None);
        assert_eq!(history.redo(), None);
        history.record(&[cell("after save")]);
        assert_eq!(history.undo(), Some(Vec::new()));
    }

    #[test]
    fn mixed_edits_match_snapshot_history_through_branching_and_clear() {
        let mut history = ChangeHistory::default();
        let mut snapshots = vec![Vec::<Change>::new()];
        let mut position = 0;
        let mut seed = 7u64;
        for iteration in 0..2_000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let mut next = snapshots[position].clone();
            match (seed >> 32) % 10 {
                0 => {
                    let expected = if position > 0 {
                        position -= 1;
                        Some(snapshots[position].clone())
                    } else { None };
                    assert_eq!(history.undo(), expected);
                }
                1 => {
                    let expected = if position + 1 < snapshots.len() {
                        position += 1;
                        Some(snapshots[position].clone())
                    } else { None };
                    assert_eq!(history.redo(), expected);
                }
                2 if iteration % 17 == 0 => {
                    history.clear();
                    snapshots = vec![Vec::new()];
                    position = 0;
                }
                action => {
                    let index = seed as usize % (next.len() + 1);
                    match action {
                        3 => next.clear(),
                        4 if !next.is_empty() => { next.remove(index.min(next.len() - 1)); }
                        5 if !next.is_empty() => {
                            let index = index.min(next.len() - 1);
                            next[index] = cell(&iteration.to_string());
                        }
                        6 => next.reverse(),
                        7 => {}, // no-op must preserve redo
                        _ => next.insert(index, cell(if iteration % 2 == 0 { "same" } else { "other" })),
                    }
                    history.record(&next);
                    if next != snapshots[position] {
                        snapshots.truncate(position + 1);
                        snapshots.push(next);
                        position += 1;
                    }
                }
            }
            assert_eq!(history.current, snapshots[position], "step {iteration}");
        }
        while position > 0 {
            position -= 1;
            assert_eq!(history.undo(), Some(snapshots[position].clone()));
        }
        for expected in snapshots.iter().skip(1) {
            assert_eq!(history.redo(), Some(expected.clone()));
        }
    }

    #[test]
    fn unchanged_large_values_are_owned_once_and_clear_releases_storage() {
        let mut history = ChangeHistory::default();
        let mut changes = Vec::new();
        for _ in 0..128 {
            changes.push(cell(&"x".repeat(16 * 1024)));
            history.record(&changes);
        }
        let retained_changes = history.current.len()
            + history.steps.iter().map(|step| step.replacement.len()).sum::<usize>();
        assert_eq!(retained_changes, 128);
        let pointer = match &history.current[0] {
            Change::Cell { value: Value::Text(text), .. } => text.as_ptr(),
            _ => unreachable!(),
        };
        // A middle replacement must preserve the prefix and suffix allocations.
        changes[64] = cell("replacement");
        history.record(&changes);
        assert!(matches!(&history.current[0], Change::Cell { value: Value::Text(text), .. } if text.as_ptr() == pointer));
        assert_eq!(history.steps.last().unwrap().replacement.len(), 1);
        for _ in 0..129 { assert!(history.undo().is_some()); }
        for _ in 0..129 { assert!(history.redo().is_some()); }
        assert_eq!(history.current, changes);
        history.clear();
        assert_eq!(history.current.capacity(), 0);
        assert_eq!(history.steps.capacity(), 0);
    }

    fn wait(app: &mut Rustable) {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while app.pending.is_some() {
            app.poll();
            assert!(Instant::now() < deadline, "database worker did not finish");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn fixture() -> (egui::Context, tempfile::NamedTempFile, Rustable) {
        let file = tempfile::NamedTempFile::new().unwrap();
        let conn = rusqlite::Connection::open(file.path()).unwrap();
        conn.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT NOT NULL); INSERT INTO t VALUES(1,'original'),(2,'second');").unwrap();
        drop(conn);
        let ctx = egui::Context::default();
        let mut app = Rustable::with_initial_connection(
            &ctx,
            Preferences::default(),
            Command::Open(Some(file.path().into())),
            None,
        );
        wait(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        app.safe = false;
        app.view = View::Data;
        (ctx, file, app)
    }

    fn edit(app: &mut Rustable, value: &str) {
        app.edit = Some(CellEdit {
            bulk_rows: Vec::new(),
            bulk_changed: false, undo_initialized: false,
            insert_id: None,
            advance: 0,
            table: "t".into(),
            row: app.table.rows[0].clone(),
            original_value: app.table.rows[0][1].clone(),
            column: "name".into(),
            text: value.into(),
            kind: 3,
            focus: false,
            inspector: false,
        });
        assert!(app.finish_cell_edit());
    }

    #[test]
    fn actual_cell_insert_remove_and_schema_actions_undo_in_order() {
        let (_ctx, _file, mut app) = fixture();
        edit(&mut app, "first");
        let first = app.changes.clone();
        edit(&mut app, "second");
        let second = app.changes.clone();
        app.add_insert();
        app.edit = None;
        let inserted = app.changes.clone();
        let id = app.selected_insert().unwrap();
        app.remove_selected_insert();
        assert_eq!(app.changes, second);
        app.restore_change_history(false);
        assert_eq!(app.changes, inserted);
        assert_eq!(app.selected_insert(), Some(id));
        app.restore_change_history(false);
        assert_eq!(app.changes, second);
        assert_eq!(app.selected_insert(), None);
        app.restore_change_history(false);
        assert_eq!(app.changes, first);
        app.restore_change_history(false);
        assert!(app.changes.is_empty());
        app.restore_change_history(true);
        assert_eq!(app.changes, first);
        app.changes.push(Change::RenameColumn {
            table: "t".into(),
            from: "name".into(),
            to: "label".into(),
        });
        app.record_changes();
        assert!(app.change_history.redo().is_none());
        app.restore_change_history(false);
        assert_eq!(app.changes, first);
    }

    #[test]
    fn save_at_intermediate_point_and_failed_save_preserve_correct_history() {
        let (_ctx, file, mut app) = fixture();
        edit(&mut app, "first");
        edit(&mut app, "second");
        app.restore_change_history(false);
        app.save_changes(true);
        wait(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        let conn = rusqlite::Connection::open(file.path()).unwrap();
        assert_eq!(
            conn.query_row("SELECT name FROM t WHERE id=1", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "first"
        );
        assert!(app.changes.is_empty());
        assert!(app.change_history.undo().is_none());
        assert!(app.change_history.redo().is_none());
        edit(&mut app, "mine");
        let intended = app.changes.clone();
        conn.execute("UPDATE t SET name='external' WHERE id=1", [])
            .unwrap();
        app.save_changes(true);
        wait(&mut app);
        assert!(app.error.is_some());
        assert_eq!(app.changes, intended);
        app.restore_change_history(false);
        assert!(app.changes.is_empty());
        assert!(app.review.is_none());
        app.restore_change_history(true);
        assert_eq!(app.changes, intended);
        assert_eq!(
            conn.query_row("SELECT name FROM t WHERE id=1", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "external"
        );
    }

    #[test]
    fn bulk_edit_and_delete_are_each_one_undo_step() {
        let (_ctx, _file, mut app) = fixture();
        app.row_selection.rows.extend([0, 1]);
        app.edit = Some(CellEdit {
            bulk_rows: app
                .table
                .rows
                .iter()
                .map(|row| (row.clone(), row[1].clone()))
                .collect(),
            bulk_changed: true, undo_initialized: false,
            insert_id: None,
            advance: 0,
            table: "t".into(),
            row: Vec::new(),
            original_value: Value::Null,
            column: "name".into(),
            text: "bulk".into(),
            kind: 3,
            focus: false,
            inspector: true,
        });
        assert!(app.finish_cell_edit());
        let bulk = app.changes.clone();
        assert_eq!(bulk.len(), 2);
        app.set_selected_rows_deleted(true);
        assert_eq!(app.changes.len(), 4);
        app.restore_change_history(false);
        assert_eq!(app.changes, bulk);
        app.restore_change_history(false);
        assert!(app.changes.is_empty());
    }

    fn key(redo: bool, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Z,
            physical_key: Some(egui::Key::Z),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers {
                command: true,
                shift: redo,
                ..Default::default()
            },
        }
    }

    #[test]
    #[ignore = "manual cell typing to staged undo CPU benchmark"]
    fn cell_undo_handoff_frame_benchmark() {
        let (ctx, _file, mut app) = fixture();
        let mut samples = Vec::new();
        for frame in 0..900 {
            let phase = frame % 5;
            if phase == 0 {
                app.edit = None;
                app.changes.clear();
                app.clear_change_history();
                app.add_insert();
            }
            let events = match phase {
                1 => vec![egui::Event::Text("123".into())],
                3 | 4 => vec![key(false, true), key(false, false)],
                _ => vec![],
            };
            let start = std::time::Instant::now();
            let output = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0))),
                time: Some(frame as f64 + if frame >= 450 { 30.0 } else { 0.0 }), events,
                ..Default::default()
            }, |ctx| {
                app.staged_history_shortcuts(ctx);
                egui::CentralPanel::default().show(ctx, |ui| { inline_value(ui, &mut app.edit); });
            });
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if phase == 4 {
                if frame >= 100 { samples.push(start.elapsed().as_secs_f64() * 1000.0); }
                assert!(app.changes.is_empty());
                assert!(app.edit.is_none());
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("history/cell-undo-handoff", &samples);
    }

    fn grid_frame(ctx: &egui::Context, app: &mut Rustable, time: &mut f64, events: Vec<egui::Event>) -> egui::FullOutput {
        *time += 0.05;
        ctx.run(egui::RawInput {
            time: Some(*time), events,
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 500.0))),
            ..Default::default()
        }, |ctx| {
            app.staged_history_shortcuts(ctx);
            egui::CentralPanel::default().show(ctx, |ui| app.data_grid(ui));
        })
    }

    fn click_grid(ctx: &egui::Context, app: &mut Rustable, time: &mut f64, pos: egui::Pos2) {
        for pressed in [true, false] {
            grid_frame(ctx, app, time, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default(),
            }]);
        }
    }

    #[test]
    #[ignore = "manual populated cell history navigation CPU benchmark"]
    fn populated_cell_history_frame_benchmark() {
        let (ctx, _file, mut app) = fixture();
        let mut samples = Vec::new();
        let mut time = 0.0;
        for iteration in 0..180 {
            app.edit = None;
            app.changes.clear();
            app.clear_change_history();
            edit(&mut app, "earlier");
            let later = Change::Cell {
                original_columns: None, table: "t".into(), column: "name".into(),
                original: app.table.rows[1].clone(), value: Value::Text("later".into()),
            };
            app.changes.push(later.clone());
            app.record_changes();
            app.focus_history_cell(later);
            grid_frame(&ctx, &mut app, &mut time, vec![]);
            grid_frame(&ctx, &mut app, &mut time, vec![]);
            grid_frame(&ctx, &mut app, &mut time, vec![egui::Event::Text("typing".into())]);
            grid_frame(&ctx, &mut app, &mut time, vec![key(false, true), key(false, false)]);
            if iteration == 90 { time += 30.0; }
            let start = Instant::now();
            for _ in 0..2 {
                let output = grid_frame(&ctx, &mut app, &mut time, vec![key(false, true), key(false, false)]);
                let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            }
            if iteration >= 20 { samples.push(start.elapsed().as_secs_f64() * 1000.0); }
            assert!(app.changes.is_empty());
            assert_eq!(app.selected, Some(0));
            assert_eq!(app.edit.as_ref().unwrap().text, "original");
        assert_eq!(app.row_selection.cell_columns, Some((1, 1)));
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("history/populated-cell-navigation", &samples);
    }

    #[test]
    fn undo_nonempty_cells_follows_history_and_restores_entry_selection() {
        let (ctx, _file, mut app) = fixture();
        edit(&mut app, "earlier");
        let first = app.changes.clone();
        // Stage a second row, then reopen it and type more.
        app.edit = Some(CellEdit {
            bulk_rows: Vec::new(), bulk_changed: false, undo_initialized: false,
            insert_id: None, advance: 0, table: "t".into(), column: "name".into(),
            row: app.table.rows[1].clone(), original_value: app.table.rows[1][1].clone(),
            text: "later".into(), kind: 3, focus: true, inspector: false,
        });
        assert!(app.finish_cell_edit());
        let second = app.changes.clone();
        app.focus_history_cell(second[1].clone());
        let mut time = 0.0;
        grid_frame(&ctx, &mut app, &mut time, vec![]);
        grid_frame(&ctx, &mut app, &mut time, vec![]);
        grid_frame(&ctx, &mut app, &mut time, vec![egui::Event::Text("typing".into())]);
        let typed = app.edit.as_ref().unwrap().text.clone();
        assert_eq!(typed, "typing", "entry typing replaces the selected cell value");
        grid_frame(&ctx, &mut app, &mut time, vec![key(true, true), key(true, false)]);
        assert_eq!(app.edit.as_ref().unwrap().text, typed, "Redo must not discard fresh typing");
        grid_frame(&ctx, &mut app, &mut time, vec![key(false, true), key(false, false)]);
        assert_eq!(app.edit.as_ref().unwrap().text, "later");
        assert_eq!(app.changes, second);
        let id = ctx.memory(|m| m.focused()).unwrap();
        assert_eq!(TextEdit::load_state(&ctx, id).unwrap().cursor.char_range().unwrap().as_sorted_char_range(), 0..5);
        // Cursor movement and an idle history point must not trap Cmd+Z.
        grid_frame(&ctx, &mut app, &mut time, vec![egui::Event::Key {
            key: egui::Key::End, physical_key: None, pressed: true, repeat: false, modifiers: Default::default(),
        }]);
        time += 2.0;
        grid_frame(&ctx, &mut app, &mut time, vec![]);
        grid_frame(&ctx, &mut app, &mut time, vec![key(false, true), key(false, false)]);
        assert_eq!(app.changes, first);
        assert_eq!(app.edit.as_ref().unwrap().text, "second");
        assert_eq!(app.row_selection.cell_columns, Some((1, 1)));
        grid_frame(&ctx, &mut app, &mut time, vec![key(false, true), key(false, false)]);
        assert!(app.changes.is_empty());
        assert_eq!(app.selected, Some(0));
        assert_eq!(app.edit.as_ref().unwrap().text, "original");
        assert_eq!(app.row_selection.cell_columns, Some((1, 1)));
        grid_frame(&ctx, &mut app, &mut time, vec![key(true, true), key(true, false)]);
        assert_eq!(app.changes, first);
        grid_frame(&ctx, &mut app, &mut time, vec![key(true, true), key(true, false)]);
        assert_eq!(app.changes, second);
    }

    #[test]
    fn staged_cell_can_be_reopened_and_edited_with_real_grid_clicks() {
        let (ctx, _file, mut app) = fixture();
        let mut time = 0.0;
        grid_frame(&ctx, &mut app, &mut time, vec![]);
        let output = grid_frame(&ctx, &mut app, &mut time, vec![]);
        let pos = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "original" => Some(text.pos + egui::vec2(10.0, 6.0)),
            _ => None,
        }).unwrap();
        for value in ["first", "second"] {
            click_grid(&ctx, &mut app, &mut time, pos);
            click_grid(&ctx, &mut app, &mut time, pos);
            assert!(app.edit.is_some(), "could not reopen editor for {value}");
            grid_frame(&ctx, &mut app, &mut time, vec![]);
            // Replace through the text widget, then blur to stage.
            grid_frame(&ctx, &mut app, &mut time, vec![egui::Event::Key {
                key: egui::Key::A, physical_key: None, pressed: true, repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }, egui::Event::Text(value.into())]);
            click_grid(&ctx, &mut app, &mut time, egui::pos2(600.0, 250.0));
            assert!(app.edit.is_none());
            grid_frame(&ctx, &mut app, &mut time, vec![]);
            assert!(matches!(&app.changes[0], Change::Cell { value: Value::Text(text), .. } if text == value));
        }
    }

    #[test]
    fn cell_typing_undo_then_insert_then_prior_change_and_redo() {
        let (ctx, _file, mut app) = fixture();
        edit(&mut app, "earlier");
        let prior = app.changes.clone();
        app.add_insert();
        let inserted = app.changes.clone();
        let mut time = 0.0;
        let mut frame = |app: &mut Rustable, events| {
            time += 1.0;
            let _ = ctx.run(egui::RawInput { time: Some(time), events, ..Default::default() }, |ctx| {
                app.staged_history_shortcuts(ctx);
                egui::CentralPanel::default().show(ctx, |ui| { inline_value(ui, &mut app.edit); });
            });
        };
        frame(&mut app, vec![]);
        frame(&mut app, vec![egui::Event::Text("123".into())]);
        frame(&mut app, vec![]);
        assert_eq!(app.edit.as_ref().unwrap().text, "123");
        frame(&mut app, vec![key(false, true), key(false, false)]);
        assert_eq!(app.edit.as_ref().unwrap().text, "");
        assert_eq!(app.changes, inserted);
        frame(&mut app, vec![key(true, true), key(true, false)]);
        assert_eq!(app.edit.as_ref().unwrap().text, "123");
        frame(&mut app, vec![key(false, true), key(false, false)]);
        frame(&mut app, vec![key(false, true), key(false, false)]);
        assert!(app.edit.is_none());
        assert_eq!(app.changes, prior);
        frame(&mut app, vec![key(false, true), key(false, false)]);
        assert!(app.changes.is_empty());
        frame(&mut app, vec![key(true, true), key(true, false)]);
        assert_eq!(app.changes, prior);
        frame(&mut app, vec![key(true, true), key(true, false)]);
        assert_eq!(app.changes, inserted);
    }

    #[test]
    fn undo_can_remove_new_row_before_its_first_editor_frame() {
        let (ctx, _file, mut app) = fixture();
        app.add_insert();
        let _ = ctx.run(egui::RawInput { events: vec![key(false, true)], ..Default::default() }, |ctx| app.staged_history_shortcuts(ctx));
        assert!(app.edit.is_none());
        assert!(app.changes.is_empty());
    }

    #[test]
    fn untouched_new_row_can_be_undone_from_its_focused_editor() {
        let (ctx, _file, mut app) = fixture();
        app.add_insert();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| { inline_value(ui, &mut app.edit); });
        });
        let _ = ctx.run(egui::RawInput { events: vec![key(false, true)], ..Default::default() }, |ctx| app.staged_history_shortcuts(ctx));
        assert!(app.edit.is_none());
        assert!(app.changes.is_empty());
        assert!(app.pending.is_none());
    }

    #[test]
    fn released_command_chords_work_and_focused_text_keeps_its_own_undo() {
        let (ctx, _file, mut app) = fixture();
        edit(&mut app, "first");
        edit(&mut app, "second");
        let _ = ctx.run(
            egui::RawInput {
                events: vec![key(false, true), key(false, false)],
                ..Default::default()
            },
            |ctx| app.staged_history_shortcuts(ctx),
        );
        assert!(
            matches!(&app.changes[0],Change::Cell{value:Value::Text(value),..} if value=="first")
        );
        let _ = ctx.run(
            egui::RawInput {
                events: vec![key(true, true), key(true, false)],
                ..Default::default()
            },
            |ctx| app.staged_history_shortcuts(ctx),
        );
        let staged = app.changes.clone();
        let id = egui::Id::new("focused-field");
        egui::text_edit::TextEditState::default().store(&ctx, id);
        ctx.memory_mut(|m| m.request_focus(id));
        let _ = ctx.run(
            egui::RawInput {
                events: vec![key(false, true)],
                ..Default::default()
            },
            |ctx| {
                app.staged_history_shortcuts(ctx);
                assert!(ctx.input(|i| i.events.iter().any(|e| matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Z,
                        pressed: true,
                        ..
                    }
                ))));
            },
        );
        assert_eq!(app.changes, staged);
    }
}

#[cfg(test)]
#[path = "change_history_performance.rs"]
mod performance;
