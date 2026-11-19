//! Explicit catalog refresh is separate from latency-sensitive data refresh.
use super::*;

impl Rustable {
    pub(super) fn refresh_schema(&mut self) {
        if self.pending.is_some() || self.review.is_some() { return; }
        if self.defer_navigation(NavigationAction::RefreshSchema) { return; }
        self.dispatch(Command::RefreshSchema, Pending::Schema);
    }

    pub(super) fn install_schema_catalog(&mut self, objects: Vec<Object>, schemas: db::SchemaCatalog, refresh: bool) {
        let removed: std::collections::HashSet<_> = if refresh {
            let names: std::collections::HashSet<_> = objects.iter().map(|object| object.name.as_str()).collect();
            self.objects.iter().filter(|object| !names.contains(object.name.as_str())).map(|object| object.name.clone()).collect()
        } else { Default::default() };
        if refresh {
            // Completed query SQL is intentionally not retained for replay. Keep
            // editing enabled only when its source metadata is unchanged.
            let fresh: std::collections::HashMap<_, _> = objects.iter().map(|object| (object.name.as_str(), object)).collect();
            let unchanged: std::collections::HashSet<_> = self.objects.iter().filter(|old| {
                fresh.get(old.name.as_str()).is_some_and(|new| new.schema == old.schema
                    && new.kind == old.kind && new.columns == old.columns
                    && new.metadata_error.is_none())
            }).map(|object| object.name.as_str()).collect();
            for tab in &mut self.tabs {
                if tab.edit_target.as_ref().is_some_and(|target| !unchanged.contains(target.table.as_str())) {
                    tab.edit_target = None;
                }
            }
        }
        self.navigator.loaded = false;
        if matches!(self.engine.as_str(), "MySQL" | "MariaDB") { self.connected_database = schemas.selected.clone(); }
        self.schemas = schemas;
        self.objects = objects;
        self.language.invalidate();
        self.status = "Schema refreshed".into();
        if !refresh { return; }

        self.invalidate_schema_snapshots(&removed);
        self.sidebar_selection = Default::default();
        if removed.contains(&self.browse.table) {
            self.browse = Browse::default();
            self.table = Arc::default();
            self.columns.clear();
            self.indexes.clear();
            self.triggers.clear();
            self.foreign_keys.clear();
            self.selected = None;
            self.row_selection.clear();
            self.status = "Schema refreshed · the selected table no longer exists".into();
        } else if !self.browse.table.is_empty() {
            // Explicit schema refresh reloads the current table once. Paging,
            // sorting, filtering and ordinary data refresh never take this path.
            self.refresh();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(app: &mut Rustable) {
        let until = Instant::now() + std::time::Duration::from_secs(5);
        while app.pending.is_some() {
            assert!(Instant::now() < until, "refresh timed out");
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    fn fixture() -> (tempfile::NamedTempFile, rusqlite::Connection, Rustable) {
        let file = tempfile::NamedTempFile::new().unwrap();
        let external = rusqlite::Connection::open(file.path()).unwrap();
        external.execute_batch("CREATE TABLE a(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO a VALUES(1,'a'); CREATE TABLE b(id INTEGER PRIMARY KEY); INSERT INTO b VALUES(2);").unwrap();
        let ctx = egui::Context::default();
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        app.dispatch(Command::Open(Some(file.path().into())), Pending::Connect(None));
        settle(&mut app);
        (file, external, app)
    }

    #[test]
    fn external_schema_refresh_updates_catalog_current_table_and_cached_revisit() {
        let (_file, external, mut app) = fixture();
        app.open_table("a".into()); settle(&mut app); app.keep_table_tab("a");
        app.open_table("b".into()); settle(&mut app); app.keep_table_tab("b");
        app.open_table("a".into()); settle(&mut app);
        external.execute_batch("CREATE TABLE c(id INTEGER); ALTER TABLE a ADD COLUMN added TEXT; ALTER TABLE b ADD COLUMN other INTEGER; CREATE INDEX a_name ON a(name);").unwrap();
        app.refresh_schema();
        assert!(matches!(app.pending, Some(Pending::Schema)));
        settle(&mut app);
        assert!(app.objects.iter().any(|o| o.name == "c"));
        assert!(app.objects.iter().find(|o| o.name == "a").unwrap().columns.iter().any(|c| c.name == "added"));
        assert_eq!(app.table.columns, ["id", "name", "added"]);
        assert!(app.indexes.iter().any(|i| i.name == "a_name"));
        app.open_table("b".into());
        assert!(matches!(app.pending, Some(Pending::Table)), "revisit must not restore stale columns");
        settle(&mut app);
        assert_eq!(app.table.columns, ["id", "other"]);
        // Ordinary data refresh remains a table-only command.
        app.refresh_current_view();
        assert!(matches!(app.pending, Some(Pending::Table)));
        settle(&mut app);
    }

    #[test]
    fn removed_table_is_cleared_without_reopening_or_querying_it() {
        let (_file, external, mut app) = fixture();
        app.open_table("a".into()); settle(&mut app);
        external.execute_batch("DROP TABLE a").unwrap();
        app.refresh_schema(); settle(&mut app);
        assert!(!app.objects.iter().any(|o| o.name == "a"));
        assert!(app.browse.table.is_empty());
        assert!(app.table.columns.is_empty());
        assert!(!app.table_tabs.iter().any(|tab| tab.browse.table == "a"));
        assert!(app.status.contains("no longer exists"));
    }

    #[test]
    fn schema_refresh_from_editor_preserves_sql_and_results() {
        let (_file, external, mut app) = fixture();
        app.show_query();
        app.tabs[0].sql = "SELECT * FROM a".into();
        app.run_query(false); settle(&mut app);
        let result = app.tabs[0].grid.clone();
        assert!(app.tabs[0].edit_target.is_some());
        app.refresh_schema(); settle(&mut app);
        assert!(app.tabs[0].edit_target.is_some(), "unchanged metadata preserves editing");
        external.execute_batch("ALTER TABLE a ADD COLUMN added INTEGER; CREATE TABLE c(id INTEGER)").unwrap();
        app.refresh_current_view(); settle(&mut app);
        assert_eq!(app.tabs[0].sql, "SELECT * FROM a");
        assert!(Arc::ptr_eq(&result, &app.tabs[0].grid), "never rerun editor SQL on schema refresh");
        assert!(app.objects.iter().any(|o| o.name == "c"));
        assert!(app.tabs[0].edit_target.is_none(), "stale SELECT * result must no longer be editable");
    }
}
