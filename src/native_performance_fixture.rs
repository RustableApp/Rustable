//! Disposable results for the opt-in native process profiler. No database I/O.
use super::*;

impl Rustable {
    pub(crate) fn native_profile_tabs(count: usize) -> Vec<(String, Arc<Grid>)> {
        (0..count).map(|tab| {
            let sql = format!("-- Native performance fixture {tab}\n{}", "SELECT id, payload FROM fixture;\n".repeat(500));
            let rows = (0..500).map(|row| vec![
                Value::Integer(row),
                Value::Text(format!("tab {tab}, row {row}: {}", "x".repeat(16 * 1024))),
            ]).collect();
            (sql, Arc::new(Grid { columns: vec!["id".into(), "payload".into()], rows, ..Default::default() }))
        }).collect()
    }

    pub(crate) fn native_profile_workspace(ctx: &egui::Context, tabs: Vec<(String, Arc<Grid>)>) -> Self {
        let mut app = Self::with_optional_connection(ctx, Preferences::default(), None, None);
        app.child_window = true;
        app.native_profile_load(tabs);
        app
    }

    pub(crate) fn native_profile_load(&mut self, tabs: Vec<(String, Arc<Grid>)>) {
        self.tabs = tabs.into_iter().map(|(sql, grid)| {
            let mut tab = QueryTab::new(sql);
            tab.open = true;
            tab.results.push(grid.clone());
            tab.grid = grid;
            tab
        }).collect();
        self.tab = 0;
        self.engine = "SQLite".into();
        self.label = "Disposable performance fixture".into();
        self.connected_database = Some("In-memory results".into());
        self.connections = false;
        self.view = View::Query;
        self.inspector = false;
        self.prefs.sidebar_hidden = true;
        self.prefs.activity_visible = false;
        self.row_selection.clear();
        self.selected = None;
    }

    pub(crate) fn native_profile_close_tabs(&mut self, keep: usize) {
        while self.tabs.iter().filter(|tab| tab.open).count() > keep {
            let index = self.tabs.iter().rposition(|tab| tab.open).unwrap();
            self.close_tab(index);
        }
    }
}
