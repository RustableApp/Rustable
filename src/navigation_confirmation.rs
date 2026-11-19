use super::*;

pub(super) enum NavigationAction {
    Refresh,
    RefreshSchema,
    View(View),
    Table(String),
    TableTab(u64),
    Query(usize),
    NewQuery,
    ShowQuery,
    Connection(Box<Command>, Pending),
}

pub(super) struct NavigationConfirmation {
    action: NavigationAction,
    table: Option<String>,
}

impl NavigationConfirmation {
    pub(super) fn connection(command: Command, pending: Pending) -> Self {
        Self { action: NavigationAction::Connection(Box::new(command), pending), table: None }
    }
}

impl Rustable {
    pub(super) fn cell_draft_changed(&self) -> bool {
        self.edit.as_ref().is_some_and(|edit| {
            if !edit.bulk_rows.is_empty() { return edit.bulk_changed; }
            // Staged values are already covered by changes; comparing to the
            // database value here requires no formatting or text allocation.
            match (&edit.original_value, edit.kind) {
                (_, 4) => false,
                (Value::Null, 0) => !edit.text.is_empty(),
                (Value::Integer(value), 1) => edit.text.parse::<i64>().ok() != Some(*value),
                (Value::Real(value), 2) => edit.text.parse::<f64>().ok() != Some(*value),
                (Value::Text(value), 3) => &edit.text != value,
                (Value::Text(value), 0) if !edit.text.is_empty() => &edit.text != value,
                _ => true,
            }
        })
    }

    pub(super) fn inline_rename_changed(&self) -> bool {
        self.rename_inline && self.rename.as_ref().is_some_and(|(from, to)| from != to)
    }

    pub(super) fn navigation_needs_discard(&self, table: Option<&str>) -> bool {
        let includes = |name: &str| table.is_none_or(|table| table == name);
        self.changes.iter().any(|change| includes(change.table()))
            || (self.cell_draft_changed() && self.edit.as_ref().is_some_and(|edit| includes(&edit.table)))
            || (self.inline_rename_changed() && includes(&self.browse.table))
    }

    pub(super) fn defer_navigation(&mut self, action: NavigationAction) -> bool {
        if self.navigation_confirmation.is_some() { return true; }
        let table = self.edit.as_ref().map(|edit| edit.table.clone())
            .or_else(|| (!self.edit_table().is_empty()).then(|| self.edit_table().to_owned()));
        if self.navigation_needs_discard(table.as_deref()) {
            if self.pending.is_some() { return true; }
            self.navigation_confirmation = Some(NavigationConfirmation { action, table });
            self.error = None;
            true
        } else {
            self.clear_untouched_editors();
            false
        }
    }

    pub(super) fn clear_untouched_editors(&mut self) {
        if !self.cell_draft_changed() { self.edit = None; }
        if self.rename_inline && !self.inline_rename_changed() {
            self.rename = None;
            self.rename_inline = false;
        }
    }

    pub(super) fn change_table_view(&mut self, view: View) {
        if view == self.view || self.defer_navigation(NavigationAction::View(view)) { return; }
        self.view = view;
    }

    pub(super) fn navigation_confirmation(&mut self, ctx: &egui::Context) {
        let Some(confirmation) = &self.navigation_confirmation else { return; };
        let verb = match confirmation.action {
            NavigationAction::Refresh | NavigationAction::RefreshSchema => "refresh",
            NavigationAction::Connection(..) => "switch connections",
            _ => "change views",
        };
        let message = match &confirmation.table {
            Some(table) => format!("Discard changes to {table} and {verb}?"),
            None => format!("Discard all changes and {verb}?"),
        };
        let blocked = self.pending.is_some() || (matches!(confirmation.action, NavigationAction::Connection(..))
            && self.worker.transaction_state().blocks_replacement());
        if let Some(discard) = crate::session_windows::discard_confirmation(ctx, &message, blocked) {
            self.resolve_navigation(discard);
        }
    }

    pub(super) fn resolve_navigation(&mut self, discard: bool) {
        if discard && (self.pending.is_some() || (self.navigation_confirmation.as_ref()
            .is_some_and(|c| matches!(c.action, NavigationAction::Connection(..)))
            && self.worker.transaction_state().blocks_replacement())) { return; }
        let Some(confirmation) = self.navigation_confirmation.take() else { return; };
        if !discard {
            if let Some(edit) = &mut self.edit { edit.focus = true; }
            return;
        }
        let includes = |name: &str| confirmation.table.as_deref().is_none_or(|table| table == name);
        if self.edit.as_ref().is_some_and(|edit| includes(&edit.table)) { self.edit = None; }
        if includes(&self.browse.table) { self.rename = None; self.rename_inline = false; }
        self.changes.retain(|change| !includes(change.table()));
        self.record_changes();
        self.error = None;
        match confirmation.action {
            NavigationAction::Refresh => self.refresh_current_view(),
            NavigationAction::RefreshSchema => self.refresh_schema(),
            NavigationAction::View(view) => self.change_table_view(view),
            NavigationAction::Table(name) => self.open_table(name),
            NavigationAction::TableTab(id) => self.activate_table_tab_id(id),
            NavigationAction::Query(index) => self.activate_query(index),
            NavigationAction::NewQuery => self.new_tab(),
            NavigationAction::ShowQuery => self.show_query(),
            NavigationAction::Connection(command, pending) => self.dispatch(*command, pending),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(app: &mut Rustable) {
        let until = Instant::now() + std::time::Duration::from_secs(5);
        while app.pending.is_some() {
            app.poll();
            assert!(Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    fn fixture() -> (egui::Context, tempfile::NamedTempFile, Rustable) {
        let file = tempfile::NamedTempFile::new().unwrap();
        rusqlite::Connection::open(file.path()).unwrap().execute_batch(
            "CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO t VALUES(1,'original');"
        ).unwrap();
        let ctx = egui::Context::default();
        let mut app = Rustable::with_initial_connection(&ctx, Preferences::default(), Command::Open(Some(file.path().into())), None);
        settle(&mut app);
        app.safe = false;
        app.open_table("t".into());
        settle(&mut app);
        (ctx, file, app)
    }

    fn draft(app: &Rustable, text: &str, inspector: bool) -> CellEdit {
        CellEdit {
            bulk_rows: vec![], bulk_changed: false, undo_initialized: false,
            insert_id: None, advance: 0, table: "t".into(), row: app.table.rows[0].clone(),
            original_value: Value::Text("original".into()), column: "name".into(),
            text: text.into(), kind: 3, focus: true, inspector,
        }
    }

    #[test]
    fn untouched_grid_and_inspector_allow_refresh_views_and_close() {
        for inspector in [false, true] {
            let (_ctx, _file, mut app) = fixture();
            app.edit = Some(draft(&app, "original", inspector));
            assert!(!app.has_pending_edits());
            assert!(!app.close_blocked());
            app.refresh_current_view();
            assert!(app.navigation_confirmation.is_none());
            assert!(app.edit.is_none());
            assert!(matches!(app.pending, Some(Pending::Table)));
            settle(&mut app);
            app.edit = Some(draft(&app, "original", inspector));
            app.change_table_view(View::Structure);
            assert!(app.view == View::Structure);
            assert!(app.edit.is_none());
            assert!(app.navigation_confirmation.is_none());
        }
    }

    #[test]
    fn refresh_cancel_preserves_draft_and_discard_preserves_other_tables() {
        for inspector in [false, true] {
            let (_ctx, file, mut app) = fixture();
            app.edit = Some(draft(&app, "changed", inspector));
            app.changes.push(Change::Cell { original_columns: None, table: "elsewhere".into(),
                column: "name".into(), original: vec![Value::Integer(1)], value: Value::Text("keep".into()) });
            app.record_changes();
            let other = app.changes.clone();
            app.refresh_current_view();
            assert!(app.pending.is_none());
            assert!(app.error.is_none());
            assert!(app.navigation_confirmation.is_some());
            assert!(!app.finish_cell_edit(), "opening the dialog must not stage a blur");
            app.resolve_navigation(false);
            assert_eq!(app.edit.as_ref().unwrap().text, "changed");
            assert_eq!(app.changes, other);
            app.refresh_current_view();
            app.resolve_navigation(true);
            assert!(app.edit.is_none());
            assert_eq!(app.changes, other);
            settle(&mut app);
            let text: String = rusqlite::Connection::open(file.path()).unwrap()
                .query_row("SELECT name FROM t", [], |row| row.get(0)).unwrap();
            assert_eq!(text, "original", "discard never writes to the database");
        }
    }

    #[test]
    fn schema_refresh_cancel_preserves_draft_and_discard_preserves_other_tables() {
        for inspector in [false, true] {
            let (_ctx, file, mut app) = fixture();
            app.edit = Some(draft(&app, "changed", inspector));
            app.changes.push(Change::Cell { original_columns: None, table: "elsewhere".into(),
                column: "name".into(), original: vec![Value::Integer(1)], value: Value::Text("keep".into()) });
            app.record_changes();
            let other = app.changes.clone();
            app.refresh_schema();
            assert!(app.pending.is_none());
            assert!(app.error.is_none());
            assert!(app.navigation_confirmation.is_some());
            assert!(!app.finish_cell_edit(), "opening the dialog must not stage a blur");
            app.resolve_navigation(false);
            assert_eq!(app.edit.as_ref().unwrap().text, "changed");
            assert_eq!(app.changes, other);
            app.refresh_schema();
            app.resolve_navigation(true);
            assert!(app.edit.is_none());
            assert_eq!(app.changes, other);
            settle(&mut app);
            let text: String = rusqlite::Connection::open(file.path()).unwrap()
                .query_row("SELECT name FROM t", [], |row| row.get(0)).unwrap();
            assert_eq!(text, "original", "discard never writes to the database");
        }
    }

    #[test]
    fn view_change_prompts_for_staged_and_invalid_values() {
        let (_ctx, _file, mut app) = fixture();
        app.edit = Some(draft(&app, "staged", false));
        assert!(app.finish_cell_edit());
        let staged = app.changes.clone();
        app.change_table_view(View::Structure);
        assert!(app.view == View::Data);
        app.resolve_navigation(false);
        assert_eq!(app.changes, staged);
        app.edit = Some(draft(&app, "invalid number", true));
        app.edit.as_mut().unwrap().kind = 1;
        app.change_table_view(View::Structure);
        assert!(app.error.is_none());
        app.resolve_navigation(true);
        assert!(app.view == View::Structure);
        assert!(app.edit.is_none() && app.changes.is_empty());
    }

    #[test]
    fn no_op_text_numeric_and_rename_drafts_are_not_dirty() {
        let (_ctx, _file, mut app) = fixture();
        app.edit = Some(draft(&app, "original", false));
        app.edit.as_mut().unwrap().bulk_changed = true; // Typed and undone.
        assert!(!app.has_pending_edits());
        let edit = app.edit.as_mut().unwrap();
        edit.original_value = Value::Integer(1);
        edit.text = "001".into(); edit.kind = 1;
        assert!(!app.has_pending_edits());
        let edit = app.edit.as_mut().unwrap();
        edit.original_value = Value::Text(String::new()); edit.text.clear(); edit.kind = 0;
        assert!(app.has_pending_edits(), "changing empty text to NULL is an edit");
        app.edit = None;
        app.rename_inline = true; app.rename = Some(("name".into(), "name".into()));
        assert!(!app.has_pending_edits());
        app.change_table_view(View::Structure);
        assert!(app.rename.is_none() && !app.rename_inline);
    }

    #[test]
    fn connection_switch_ignores_untouched_editor_and_prompts_for_changes() {
        let (_ctx, _file, mut app) = fixture();
        app.edit = Some(draft(&app, "changed", true));
        app.dispatch(Command::Open(None), Pending::Connect(None));
        assert!(app.navigation_confirmation.is_some());
        assert!(app.pending.is_none());
        app.resolve_navigation(false);
        assert_eq!(app.edit.as_ref().unwrap().text, "changed");
        app.edit.as_mut().unwrap().text = "original".into();
        app.dispatch(Command::Open(None), Pending::Connect(None));
        assert!(app.navigation_confirmation.is_none());
        assert!(app.edit.is_none());
        assert!(matches!(app.pending, Some(Pending::Connect(_))));
        settle(&mut app);
    }

    #[test]
    #[ignore = "manual navigation discard dialog CPU benchmark"]
    fn navigation_discard_frame_benchmark() {
        let (ctx, _file, mut app) = fixture();
        app.edit = Some(draft(&app, "changed", true));
        app.refresh_current_view();
        let mut samples = Vec::new();
        for frame in 0..180 {
            let start = Instant::now();
            let output = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(760.0, 600.0))),
                time: Some(frame as f64 / 120.0 + if frame >= 90 { 30.0 } else { 0.0 }),
                ..Default::default()
            }, |ctx| app.navigation_confirmation(ctx));
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 20 { samples.push(start.elapsed().as_secs_f64() * 1000.0); }
            assert!(app.navigation_confirmation.is_some() && app.pending.is_none());
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("navigation/discard-confirmation", &samples);
    }

    #[test]
    fn discard_rechecks_busy_work_and_keeps_confirmation() {
        let (_ctx, _file, mut app) = fixture();
        app.edit = Some(draft(&app, "changed", false));
        app.change_table_view(View::Structure);
        app.pending = Some(Pending::Export);
        app.resolve_navigation(true);
        assert!(app.navigation_confirmation.is_some());
        assert_eq!(app.edit.as_ref().unwrap().text, "changed");
        assert!(app.view == View::Data);
        app.pending = None;
        app.resolve_navigation(false);
        assert!(app.navigation_confirmation.is_none());
    }
}
