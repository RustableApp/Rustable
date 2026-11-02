//! Replaceable previews and explicitly kept tables; relationships always append a preview.
use super::*;
#[derive(Clone)]
pub(super) struct TableTab {
    pub id: u64,
    pub browse: Browse,
    pub title: String,
    pub kept: bool,
    filters: Vec<db::filters::Filter>,
    originals: Vec<Option<db::filters::Filter>>,
    filters_visible: bool,
}
impl Rustable {
    pub(super) fn remember_table_tab(&mut self) {
        if self.browse.table.is_empty() {
            return;
        }
        let edited = self
            .edit
            .as_ref()
            .is_some_and(|e| e.table == self.browse.table)
            || self.changes.iter().any(|c| c.table() == self.browse.table);
        let existing = self.table_tabs.iter().position(|t| {
            Some(t.id) == self.active_table_tab && t.browse.table == self.browse.table
        });
        let kept = edited || existing.is_some_and(|i| self.table_tabs[i].kept);
        let same_filter = self.filter_rules_table == self.browse.table;
        let id = existing.map(|i| self.table_tabs[i].id).unwrap_or_else(|| {
            self.next_table_tab_id = self.next_table_tab_id.wrapping_add(1);
            self.next_table_tab_id
        });
        self.active_table_tab = Some(id);
        let tab = TableTab {
            id,
            browse: self.browse.clone(),
            title: self.current_table_name().to_owned(),
            kept,
            filters: if same_filter {
                self.filter_rules.clone()
            } else {
                self.browse.filters.clone()
            },
            originals: if same_filter {
                self.filter_rules_original.clone()
            } else {
                self.browse.filters.iter().cloned().map(Some).collect()
            },
            filters_visible: self.filters_visible,
        };
        if let Some(i) = existing {
            self.table_tabs[i] = tab;
        } else {
            self.table_tabs.push(tab);
        }
    }
    pub(super) fn keep_table_tab(&mut self, name: &str) {
        self.remember_table_tab();
        if let Some(tab) = self
            .table_tabs
            .iter_mut()
            .find(|t| Some(t.id) == self.active_table_tab && t.browse.table == name)
        {
            tab.kept = true;
        }
    }
    pub(super) fn restore_table_tab(&mut self, name: &str) -> bool {
        let Some(id) = self
            .table_tabs
            .iter()
            .rev()
            .find(|t| t.browse.table == name)
            .map(|t| t.id)
        else {
            return false;
        };
        self.restore_table_tab_id(id)
    }
    pub(super) fn activate_table_tab_id(&mut self, id: u64) {
        if self.pending.is_some() || !self.finish_cell_edit() {
            return;
        }
        self.remember_table_tab();
        if self.active_table_tab == Some(id) {
            self.view = View::Data;
            return;
        }
        if self.restore_table_tab_id(id) {
            self.refresh();
        }
    }
    pub(super) fn restore_table_tab_id(&mut self, id: u64) -> bool {
        let Some(tab) = self.table_tabs.iter().find(|t| t.id == id).cloned() else {
            return false;
        };
        self.active_table_tab = Some(id);
        self.browse = tab.browse;
        self.table_display_name = Some((self.browse.table.clone(), tab.title));
        self.filter_rules_table = self.browse.table.clone();
        self.filter_rules = tab.filters;
        self.filter_rules_original = tab.originals;
        self.filters_visible = tab.filters_visible;
        self.view = View::Data;
        self.selected = None;
        self.row_selection.clear();
        true
    }
    #[cfg(test)]
    pub(super) fn close_named_table_tab(&mut self, name: &str) {
        if let Some(id) = self
            .table_tabs
            .iter()
            .find(|t| t.browse.table == name)
            .map(|t| t.id)
        {
            self.close_table_tab_id(id);
        }
    }
    pub(super) fn close_table_tab_id(&mut self, id: u64) {
        let Some(name) = self
            .table_tabs
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.browse.table.clone())
        else {
            return;
        };
        if self.pending.is_some() || !self.finish_cell_edit() {
            return;
        }
        if self.changes.iter().any(|c| c.table() == name) {
            self.error =
                Some("Save or discard this table's pending changes before closing it.".into());
            return;
        }
        self.table_tabs.retain(|tab| tab.id != id);
        if self.active_table_tab != Some(id) {
            return;
        }
        let query_active = self.view == View::Query;
        let query_selection =
            query_active.then(|| (self.selected, std::mem::take(&mut self.row_selection)));
        self.active_table_tab = None;
        self.browse = Browse::default();
        self.table = Arc::default();
        self.foreign_keys.clear();
        self.selected_column = None;
        self.selected = None;
        self.row_selection.clear();
        if let Some(next) = self.table_tabs.last().map(|t| t.id) {
            self.restore_table_tab_id(next);
            self.refresh();
            if query_active {
                self.view = View::Query;
            }
        } else if query_active || self.tabs.iter().any(|t| t.open) {
            self.show_query();
        } else {
            self.view = View::Data;
        }
        if let Some((selected, selection)) = query_selection {
            self.selected = selected;
            self.row_selection = selection;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> Rustable {
        Rustable::with_optional_connection(
            &egui::Context::default(),
            Preferences::default(),
            None,
            None,
        )
    }
    #[test]
    fn preview_replaces_but_kept_tabs_preserve_filters_and_page() {
        let mut app = app();
        app.prepare_table("first".into());
        app.prepare_table("second".into());
        assert_eq!(app.table_tabs.len(), 1);
        assert_eq!(app.table_tabs[0].browse.table, "second");
        app.keep_table_tab("second");
        app.browse.offset = 300;
        app.browse.filters = vec![db::filters::Filter {
            enabled: true,
            column: "id".into(),
            operator: "=".into(),
            value: "3".into(),
        }];
        app.prepare_table("third".into());
        assert_eq!(app.table_tabs.len(), 2);
        app.prepare_table("second".into());
        assert_eq!(app.browse.offset, 300);
        assert_eq!(app.browse.filters[0].value, "3");
        app.prepare_table("fourth".into());
        assert_eq!(app.table_tabs.len(), 2);
        assert!(
            app.table_tabs
                .iter()
                .any(|t| t.browse.table == "second" && t.kept)
        );
        assert!(
            app.table_tabs
                .iter()
                .any(|t| t.browse.table == "fourth" && !t.kept)
        );
    }
    #[test]
    fn staged_edits_keep_a_preview_and_block_closing_it() {
        let mut app = app();
        app.prepare_table("first".into());
        app.changes.push(Change::Cell {
            table: "first".into(),
            column: "name".into(),
            original: vec![Value::Integer(1)],
            value: Value::Text("new".into()),
        });
        app.record_changes();
        assert!(app.table_tabs[0].kept);
        app.prepare_table("second".into());
        assert_eq!(app.table_tabs.len(), 2);
        app.close_named_table_tab("first");
        assert_eq!(app.table_tabs.len(), 2);
        assert!(app.error.is_some());
        app.changes.clear();
        app.close_named_table_tab("first");
        assert_eq!(app.table_tabs.len(), 1);
    }
    #[test]
    fn relationships_always_append_independent_preview_tabs() {
        let mut app = app();
        app.engine = "SQLite".into();
        app.prepare_table("people".into());
        let source = app.active_table_tab.unwrap();
        let key = db::foreign_keys::ForeignKey {
            name: "manager".into(),
            schema: None,
            table: "people".into(),
            columns: vec!["manager_id".into()],
            referenced_columns: vec!["id".into()],
        };
        app.follow_foreign_key(&key, &["manager_id".into()], &[Value::Integer(9)]);
        let first = app.active_table_tab.unwrap();
        app.pending = None;
        app.follow_foreign_key(&key, &["manager_id".into()], &[Value::Integer(12)]);
        let second = app.active_table_tab.unwrap();
        app.pending = None;
        app.remember_table_tab();
        assert_eq!(app.table_tabs.len(), 3);
        assert!(app.table_tabs.iter().all(|tab| !tab.kept));
        assert_ne!(source, first);
        assert_ne!(first, second);
        assert!(app.restore_table_tab_id(first));
        assert_eq!(app.browse.filters[0].value, "9");
        assert!(app.restore_table_tab_id(second));
        assert_eq!(app.browse.filters[0].value, "12");
        assert!(app.restore_table_tab_id(source));
        assert!(app.browse.filters.is_empty());
        app.close_table_tab_id(first);
        assert_eq!(app.table_tabs.len(), 2);
        assert_eq!(app.active_table_tab, Some(source));
        assert!(app.table_tabs.iter().any(|tab| tab.id == second));
    }
    #[test]
    fn relation_prepares_filter_before_request_and_keeps_pinned_source() {
        let mut app = app();
        app.engine = "SQLite".into();
        app.prepare_table("child".into());
        app.keep_table_tab("child");
        let key = db::foreign_keys::ForeignKey {
            name: "fk".into(),
            schema: None,
            table: "parent".into(),
            columns: vec!["parent_id".into()],
            referenced_columns: vec!["id".into()],
        };
        app.follow_foreign_key(&key, &["parent_id".into()], &[Value::Integer(9)]);
        assert_eq!(app.browse.table, "parent");
        assert!(app.filters_visible);
        assert_eq!(app.browse.filters[0].column, "id");
        assert_eq!(app.browse.filters[0].value, "9");
        assert!(matches!(app.pending, Some(Pending::Table)));
        assert_eq!(app.table_tabs.len(), 2);
        assert!(app.prefs.query_history.is_empty());
    }
}
