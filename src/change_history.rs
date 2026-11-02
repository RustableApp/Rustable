//! Undo concerns staged UI intentions only, never previously committed SQL.
//! Snapshot creation happens on completed actions, not frames or keystrokes.
use super::*;

#[derive(Default)]
pub(super) struct ChangeHistory {
    states: Vec<Vec<Change>>,
    position: usize,
}

impl ChangeHistory {
    fn record(&mut self, changes: &[Change]) {
        let current = self
            .states
            .get(self.position)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if current == changes {
            return;
        }
        if self.states.is_empty() {
            self.states.push(Vec::new());
        }
        self.states.truncate(self.position + 1);
        self.states.push(changes.to_vec());
        self.position += 1;
    }

    fn undo(&mut self) -> Option<Vec<Change>> {
        if self.position == 0 {
            return None;
        }
        self.position -= 1;
        Some(self.states[self.position].clone())
    }

    fn redo(&mut self) -> Option<Vec<Change>> {
        if self.position + 1 >= self.states.len() {
            return None;
        }
        self.position += 1;
        Some(self.states[self.position].clone())
    }

    fn clear(&mut self) {
        self.states.clear();
        self.position = 0;
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

    pub(super) fn staged_history_shortcuts(&mut self, ctx: &egui::Context) {
        // Text fields own their own edit history, including SQL and inspector
        // drafts. A focused grid also wants keyboard input, so inspect actual
        // TextEdit state rather than treating every focused widget as text.
        let text_focused = ctx
            .memory(|memory| memory.focused())
            .is_some_and(|id| TextEdit::load_state(ctx, id).is_some());
        if text_focused || self.edit.is_some() || self.pending.is_some() {
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
            self.restore_change_history(redo);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(value: &str) -> Change {
        Change::Cell {
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
        assert_eq!(history.states[history.position], vec![cell("one")]);
        history.clear();
        assert_eq!(history.undo(), None);
        assert_eq!(history.redo(), None);
        history.record(&[cell("after save")]);
        assert_eq!(history.undo(), Some(Vec::new()));
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
            bulk_changed: false,
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
            bulk_changed: true,
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
