use web_time::Instant;
#[path = "query_library.rs"]
mod query_library;
#[path = "query_history.rs"]
mod query_history;
#[path = "navigator.rs"]
mod navigator;
#[path = "bulk_rows.rs"]
mod bulk_rows;
#[path = "change_history.rs"]
mod change_history;
#[path = "connections.rs"]
mod connections;
#[path = "csv_mapping.rs"]
mod csv_mapping;
#[path = "inserts.rs"]
mod inserts;
#[path = "pane_toggles.rs"]
mod pane_toggles;
#[path = "row_copy.rs"]
mod row_copy;
#[path = "row_selection.rs"]
mod row_selection;
#[cfg(target_arch = "wasm32")]
#[path = "browser_files.rs"]
mod browser_files;
use crate::db::{self, Browse, Column, Command, Grid, Object, Payload, Worker};
use crate::db::{Change, ChangePlan, Value};
use eframe::egui::{self, Color32, RichText, Stroke, TextEdit};
use egui_extras::{Column as TableColumn, TableBuilder};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
};

const BLUE: Color32 = Color32::from_rgb(50, 104, 224);
const MUTED: Color32 = Color32::from_rgb(125, 137, 156);
const GREEN: Color32 = Color32::from_rgb(40, 145, 110);

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct ConnectionStyle {
    environment: String,
    color: [u8; 3],
}
impl Default for ConnectionStyle {
    fn default() -> Self {
        Self {
            environment: "Local".into(),
            color: [44, 111, 80],
        }
    }
}

/// Stable saved-profile identity for a live session. Endpoint strings are not IDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProfileIdentity {
    id: crate::connection_profiles::ProfileId,
    name: String,
    environment: String,
    color: [u8; 3],
}
impl From<&crate::connection_profiles::Profile> for ProfileIdentity {
    fn from(profile: &crate::connection_profiles::Profile) -> Self {
        Self {
            id: profile.id,
            name: profile.name.clone(),
            environment: profile.environment.clone(),
            color: profile.color,
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    recent: Vec<PathBuf>,
    saved_connections: crate::connection_profiles::ConnectionProfiles,
    connections_migrated: bool,
    history: Vec<String>,
    query_history: Vec<query_history::Entry>,
    favorites: Vec<String>,
    query_directory: Option<PathBuf>,
    appearance: Appearance,
    // Legacy sessions stored only SQL strings. Read them once during migration.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    queries: Vec<String>,
    documents: Vec<QueryDocument>,
    closed_documents: Vec<QueryDocument>,
    window_documents: Vec<QueryDocument>,
    connection_styles: std::collections::HashMap<String, ConnectionStyle>,
    activity_visible: bool,
    column_widths: crate::column_widths::ColumnWidths,
    pane_sizes: crate::pane_sizes::PaneSizes,
}

/// Session data contains drafts, never query results. A clean file uses `sql`
/// as its baseline, avoiding a second serialized copy of large scripts.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct QueryDocument {
    sql: String,
    path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    saved_sql: Option<String>,
    closed: bool,
}
impl QueryDocument {
    fn recoverable(&self) -> bool {
        !self.sql.trim().is_empty() || self.path.is_some()
    }
    fn capture(tab: &QueryTab) -> Self {
        Self {
            sql: tab.sql.clone(),
            path: tab.source_path.clone(),
            saved_sql: tab
                .saved_sql
                .as_ref()
                .filter(|saved| ***saved != tab.sql)
                .map(|saved| (**saved).clone()),
            closed: !tab.open,
        }
    }
    fn restore(self) -> QueryTab {
        let mut tab = QueryTab::new(self.sql);
        if let Some(path) = self.path {
            let baseline = self.saved_sql.unwrap_or_else(|| tab.sql.clone());
            tab.saved(path, baseline);
        }
        tab.open = !self.closed;
        tab
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}
impl Appearance {
    fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
    fn preference(self) -> egui::ThemePreference {
        match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
enum ResultView { #[default] Data, Message, Chart }

struct QueryTab {
    completion: crate::completion::Completion,
    open: bool,
    id: u64,
    sql: String,
    source_name: Option<String>,
    source_path: Option<PathBuf>,
    saved_sql: Option<Arc<String>>,
    dirty: bool,
    grid: Arc<Grid>,
    results: Vec<Arc<Grid>>,
    result_index: usize,
    result_view: ResultView,
    chart: crate::result_chart::Chart,
    formatter: crate::sql_format::Formatter,
    result_error: Option<String>,
    result_search: crate::result_search::Search,
    result_grid_focus: Option<egui::Id>,
    pins: crate::result_pins::Pins,
    selected_pin: Option<usize>,
    cursor: std::ops::Range<usize>,
    focus_editor: bool,
    search: crate::find::Search,
}
impl QueryTab {
    fn remove_pin(&mut self, index: usize) {
        self.pins.items.remove(index);
        if self.selected_pin == Some(index) {
            self.selected_pin = None;
            self.grid = self.results.get(self.result_index).cloned().unwrap_or_default();
        } else if let Some(selected) = &mut self.selected_pin && *selected > index {
            *selected -= 1;
        }
    }
    fn refresh_dirty(&mut self) {
        self.dirty = self
            .saved_sql
            .as_ref()
            .is_some_and(|saved| **saved != self.sql);
    }
    fn saved(&mut self, path: PathBuf, sql: String) {
        self.source_name = path.file_name().map(|s| s.to_string_lossy().into_owned());
        self.source_path = Some(path);
        self.saved_sql = Some(Arc::new(sql));
        self.refresh_dirty();
    }
    fn new(sql: String) -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            completion: Default::default(),
            open: true,
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            sql,
            source_name: None,
            source_path: None,
            saved_sql: None,
            dirty: false,
            grid: Arc::default(),
            results: Vec::new(),
            result_index: 0,
            result_view: ResultView::Data,
            chart: Default::default(),
            formatter: Default::default(),
            result_error: None,
            result_search: Default::default(),
            result_grid_focus: None,
            pins: Default::default(),
            selected_pin: None,
            cursor: 0..0,
            focus_editor: true,
            search: Default::default(),
        }
    }
}
#[derive(PartialEq, Clone, Copy)]
enum View {
    Data,
    Structure,
    Indexes,
    Triggers,
    Query,
}
enum Pending {
    Connect(Option<PathBuf>),
    ConnectProfile {
        recent: Option<PathBuf>,
        identity: ProfileIdentity,
    },
    Table,
    Query(usize),
    Export,
    ImportMetadata,
    Import,
    Plan(bool),
    Apply,
}
enum FileAction {
    #[cfg(target_arch = "wasm32")]
    BrowserExportDone(Result<String, String>),
    #[cfg(target_arch = "wasm32")]
    BrowserScript(String, String),
    #[cfg(target_arch = "wasm32")]
    BrowserDownload(String),
    QueryDirectory(PathBuf),
    QueryLibraryError(String),
    ProfilePath(u64, bool, PathBuf),
    OpenScript(PathBuf),
    SaveScript(PathBuf, String, u64, Option<crate::script_file::SaveSource>),
    PreviewCsv(PathBuf),
    Open(PathBuf),
    Export(PathBuf, Arc<Grid>, db::ExportFormat),
    ExportTable(PathBuf, Browse, String, db::ExportFormat),
}
#[derive(Clone)]
struct CellEdit {
    bulk_rows: Vec<(Vec<Value>, Value)>,
    bulk_changed: bool,
    insert_id: Option<u64>,
    advance: i8,
    table: String,
    row: Vec<Value>,
    original_value: Value,
    column: String,
    text: String,
    kind: usize,
    focus: bool,
    inspector: bool,
}

struct IndexDraft {
    table: String,
    name: String,
    available_columns: Vec<String>,
    keys: Vec<(String, bool)>,
    unique: bool,
}

struct AlterDraft {
    table: String,
    column: String,
    data_type: String,
    original_type: String,
    not_null: bool,
    original_not_null: bool,
    default_kind: usize,
    default_text: String,
    error: Option<String>,
}
impl AlterDraft {
    fn change(&self) -> Result<Change, String> {
        let default = match self.default_kind {
            0 => None,
            1 => Some(None),
            2 => Some(Some(Value::Null)),
            3 => Some(Some(Value::Integer(
                self.default_text
                    .parse::<i64>()
                    .map_err(|e| e.to_string())?,
            ))),
            4 => Some(Some(Value::Real(
                self.default_text
                    .parse::<f64>()
                    .map_err(|e| e.to_string())?,
            ))),
            _ => Some(Some(Value::Text(self.default_text.clone()))),
        };
        Ok(Change::AlterColumn {
            table: self.table.clone(),
            column: self.column.clone(),
            data_type: (self.data_type != self.original_type).then(|| self.data_type.clone()),
            not_null: (self.not_null != self.original_not_null).then_some(self.not_null),
            default,
        })
    }
}

struct ColumnDraft {
    table: String,
    name: String,
    data_type: String,
    not_null: bool,
    default_kind: usize,
    default_text: String,
    error: Option<String>,
}
impl ColumnDraft {
    fn definition(&self) -> Result<db::ColumnDefinition, String> {
        let default = match self.default_kind {
            0 => None,
            1 => Some(Value::Null),
            2 => Some(Value::Integer(
                self.default_text
                    .parse::<i64>()
                    .map_err(|e| e.to_string())?,
            )),
            3 => Some(Value::Real(
                self.default_text
                    .parse::<f64>()
                    .map_err(|e| e.to_string())?,
            )),
            _ => Some(Value::Text(self.default_text.clone())),
        };
        if self.name.trim().is_empty() || self.data_type.trim().is_empty() {
            return Err("Enter a column name and data type.".into());
        }
        Ok(db::ColumnDefinition {
            name: self.name.clone(),
            data_type: self.data_type.clone(),
            not_null: self.not_null,
            default,
        })
    }
}

pub struct Rustable {
    child_window: bool,
    recover_next_window: bool,
    show_recovered_on_connect: bool,
    shared_preferences_revision: u64,
    shared_preferences_dirty: bool,
    manager_requested: bool,
    window_requests: Vec<crate::session_windows::WindowRequest>,
    connection_manager: connections::Manager,
    language: crate::editor_language::LanguageSession,
    active_profile: Option<ProfileIdentity>,
    profiles_revision: u64,
    worker: Worker,
    prefs: Preferences,
    pending: Option<Pending>,
    started: Instant,
    label: String,
    engine: String,
    schemas: db::SchemaCatalog,
    connected_database: Option<String>,
    table_display_name: Option<(String, String)>,
    objects: Vec<Object>,
    browse: Browse,
    columns: Vec<Column>,
    indexes: Vec<db::Index>,
    triggers: Vec<Object>,
    selected_index: Option<usize>,
    table: Arc<Grid>,
    tabs: Vec<QueryTab>,
    closed_tabs: Vec<QueryTab>,
    window_recovery: Vec<QueryTab>,
    tab: usize,
    view: View,
    search: String,
    filter: String,
    filters_visible: bool,
    focus_filter: bool,
    focus_page_grid: bool,
    filter_column: String,
    safe: bool,
    status: String,
    error: Option<String>,
    selected: Option<usize>,
    row_selection: row_selection::RowSelection,
    saved_selection: Option<(String, Vec<(String, Value)>)>,
    selected_column: Option<usize>,
    structure_search: String,
    field_search: String,
    csv_preview: Option<db::csv_preview::Preview>,
    csv_mapping: csv_mapping::Mapping,
    csv_empty_as_null: bool,
    csv_postgres_inserts: bool,
    csv_review: Option<(db::csv_import::Request, Vec<String>)>,
    inspector: bool,
    history: bool,
    query_library: query_library::Library,
    #[cfg(target_arch = "wasm32")]
    browser_export: std::cell::RefCell<Option<crate::browser_jobs::Handle>>,
    navigator: navigator::Navigator,
    activity: std::collections::VecDeque<String>,
    last_operation_ms: Option<f64>,
    sidebar_section: usize,
    history_search: String,
    history_selected: Option<usize>,
    connections: bool,
    edit: Option<CellEdit>,
    changes: Vec<Change>,
    change_history: change_history::ChangeHistory,
    review: Option<ChangePlan>,
    review_sql: Vec<String>,
    rename: Option<(String, String)>,
    rename_inline: bool,
    rename_focus: bool,
    add_column: Option<ColumnDraft>,
    alter_column: Option<AlterDraft>,
    index_draft: Option<IndexDraft>,
    files_tx: mpsc::Sender<FileAction>,
    files_rx: mpsc::Receiver<FileAction>,
    frame_ms: f32,
}

impl Rustable {
    fn refresh_profile_identity_from(
        profiles: &crate::connection_profiles::ConnectionProfiles,
        identity: &mut ProfileIdentity,
    ) {
        if let Some(profile) = profiles.profile(identity.id) {
            *identity = ProfileIdentity::from(profile);
        }
        // Removing a saved profile does not rename or disconnect its live session.
    }
    fn profiles_changed(&mut self) {
        self.profiles_revision = self.profiles_revision.wrapping_add(1);
        if let Some(identity) = &mut self.active_profile {
            Self::refresh_profile_identity_from(&self.prefs.saved_connections, identity);
        }
        if let Some(Pending::ConnectProfile { identity, .. }) = &mut self.pending {
            Self::refresh_profile_identity_from(&self.prefs.saved_connections, identity);
        }
    }
    pub(crate) fn sync_window_profile(&self, child: &mut Self) -> bool {
        if child.profiles_revision == self.profiles_revision {
            return false;
        }
        if let Some(identity) = &mut child.active_profile {
            Self::refresh_profile_identity_from(&self.prefs.saved_connections, identity);
        }
        if let Some(Pending::ConnectProfile { identity, .. }) = &mut child.pending {
            Self::refresh_profile_identity_from(&self.prefs.saved_connections, identity);
        }
        child.profiles_revision = self.profiles_revision;
        true
    }
    fn active_connection_style(&self) -> ConnectionStyle {
        self.active_profile
            .as_ref()
            .map(|identity| ConnectionStyle {
                environment: identity.environment.clone(),
                color: identity.color,
            })
            .unwrap_or_else(|| {
                self.prefs
                    .connection_styles
                    .get(&self.label)
                    .cloned()
                    .unwrap_or_default()
            })
    }
    pub(crate) fn active_connection_name(&self) -> &str {
        if let Some(identity) = &self.active_profile {
            return &identity.name;
        }
        if self.label.contains("in memory") {
            return "Studio demo";
        }
        std::path::Path::new(&self.label)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&self.label)
    }

    pub(crate) fn take_window_requests(&mut self) -> Vec<crate::session_windows::WindowRequest> {
        let mut requests = std::mem::take(&mut self.window_requests);
        if let Some(first) = requests.first_mut() {
            first.recover_sql = std::mem::take(&mut self.recover_next_window);
        }
        requests
    }
    pub(crate) fn manager_shortcut(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::N)) {
            if self.child_window {
                self.manager_requested = true;
            } else {
                self.connections = true;
            }
        }
    }
    /// The launcher owns preferences and recovery; it never renders a database workspace.
    pub(crate) fn render_launcher(&mut self, ctx: &egui::Context) -> bool {
        self.poll();
        self.manager_shortcut(ctx);
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::W)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.connections = true;
        self.connections_window(ctx);
        false
    }
    pub(crate) fn take_manager_request(&mut self) -> bool {
        std::mem::take(&mut self.manager_requested)
    }
    pub(crate) fn open_connection_manager(&mut self) {
        self.connections = true;
    }
    pub(crate) fn window_error(&mut self, message: &str) {
        self.error = Some(message.into());
    }
    pub(crate) fn has_connection(&self) -> bool {
        !self.engine.is_empty()
    }
    pub(crate) fn close_blocked(&self) -> bool {
        self.worker.transaction_state().blocks_replacement()
            || !self.changes.is_empty()
            || self.edit.is_some()
            || (self.rename_inline && self.rename.is_some())
            || self.pending.is_some()
    }
    pub(crate) fn recovery_documents(&self) -> Vec<QueryDocument> {
        self.tabs
            .iter()
            .chain(self.closed_tabs.iter())
            .chain(self.window_recovery.iter())
            .filter(|tab| !tab.sql.trim().is_empty() || tab.source_path.is_some())
            .map(|tab| {
                let mut document = QueryDocument::capture(tab);
                document.closed = true;
                document
            })
            .collect()
    }
    pub(crate) fn archive_window(&mut self, mut child: Self) {
        for mut tab in child.tabs.drain(..).chain(child.closed_tabs.drain(..)) {
            tab.open = false;
            tab.grid = Arc::default();
            tab.results.clear();
            tab.chart = Default::default();
            tab.formatter = Default::default();
            tab.pins.clear();
            tab.selected_pin = None;
            if !tab.sql.trim().is_empty() || tab.source_path.is_some() {
                self.window_recovery.push(tab);
            }
        }
        self.window_recovery.append(&mut child.window_recovery);
        self.merge_window_preferences(&mut child);
        self.error = Some(format!(
            "SQL drafts from {} are available in Connections. Select recovery before opening a connection; nothing is executed automatically.",
            child.label
        ));
    }
    pub(crate) fn publish_shared_preferences(&mut self) -> bool {
        if !std::mem::take(&mut self.shared_preferences_dirty) {
            return false;
        }
        self.shared_preferences_revision = self.shared_preferences_revision.wrapping_add(1);
        true
    }
    pub(crate) fn sync_window_preferences(&self, child: &mut Self) {
        if child.shared_preferences_revision == self.shared_preferences_revision {
            return;
        }
        child.prefs.appearance = self.prefs.appearance;
        child.prefs.favorites = self.prefs.favorites.clone();
        child.prefs.query_directory = self.prefs.query_directory.clone();
        child.prefs.history = self.prefs.history.clone();
        child.prefs.query_history = self.prefs.query_history.clone();
        child.prefs.column_widths = self.prefs.column_widths.clone();
        child.prefs.pane_sizes = self.prefs.pane_sizes;
        child.shared_preferences_revision = self.shared_preferences_revision;
    }
    pub(crate) fn merge_window_preferences(&mut self, child: &mut Self) -> bool {
        if !std::mem::take(&mut child.shared_preferences_dirty) {
            return false;
        }
        // Child received the authoritative snapshot before rendering. Publish its
        // exact edits, including removals, instead of append-only union semantics.
        self.prefs.appearance = child.prefs.appearance;
        self.prefs.favorites = child.prefs.favorites.clone();
        self.prefs.query_directory = child.prefs.query_directory.clone();
        self.prefs.history = child.prefs.history.clone();
        self.prefs.query_history = child.prefs.query_history.clone();
        self.prefs.column_widths = child.prefs.column_widths.clone();
        self.prefs.pane_sizes = child.prefs.pane_sizes;
        self.prefs.history.truncate(100);
        self.shared_preferences_revision = self.shared_preferences_revision.wrapping_add(1);
        child.shared_preferences_revision = self.shared_preferences_revision;
        true
    }
    pub(crate) fn new_window(
        &mut self,
        ctx: &egui::Context,
        request: crate::session_windows::WindowRequest,
    ) -> Self {
        let prefs = Preferences {
            appearance: self.prefs.appearance,
            favorites: self.prefs.favorites.clone(),
            query_directory: self.prefs.query_directory.clone(),
            history: self.prefs.history.clone(),
            query_history: self.prefs.query_history.clone(),
            connection_styles: self.prefs.connection_styles.clone(),
            activity_visible: self.prefs.activity_visible,
            column_widths: self.prefs.column_widths.clone(),
            pane_sizes: self.prefs.pane_sizes,
            connections_migrated: true,
            documents: vec![QueryDocument {
                sql: String::new(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let command = match request.connection {
            crate::session_windows::WindowConnection::Demo => Command::Open(None),
            crate::session_windows::WindowConnection::Sqlite(path) => Command::Open(Some(path)),
            crate::session_windows::WindowConnection::Postgres(config) => {
                Command::OpenPostgres(*config)
            }
        };
        let mut app = Self::with_initial_connection(ctx, prefs, command, request.identity);
        if request.recover_sql && !self.window_recovery.is_empty() {
            app.tabs = std::mem::take(&mut self.window_recovery);
            for tab in &mut app.tabs {
                tab.open = true;
            }
            app.tab = 0;
            app.view = View::Query;
            app.show_recovered_on_connect = true;
        }
        app.child_window = true;
        app.shared_preferences_revision = self.shared_preferences_revision;
        app.profiles_revision = self.profiles_revision;
        // A queued request may predate a profile rename. Refresh only its identity.
        if let Some(Pending::ConnectProfile { identity, .. }) = &mut app.pending {
            Self::refresh_profile_identity_from(&self.prefs.saved_connections, identity);
        }
        app
    }
    pub(crate) fn save_with_window_documents(
        &mut self,
        storage: &mut dyn eframe::Storage,
        extra: Vec<QueryDocument>,
    ) {
        self.prefs.queries.clear();
        self.prefs.documents = self.tabs.iter().map(QueryDocument::capture).collect();
        self.prefs.closed_documents = self
            .closed_tabs
            .iter()
            .map(QueryDocument::capture)
            .collect();
        self.prefs.window_documents = self
            .window_recovery
            .iter()
            .map(QueryDocument::capture)
            .chain(extra)
            .filter(QueryDocument::recoverable)
            .collect();
        eframe::set_value(storage, "workspace", &self.prefs);
        self.prefs.documents.clear();
        self.prefs.closed_documents.clear();
        self.prefs.window_documents.clear();
    }

    fn supports_csv_import(&self) -> bool {
        matches!(self.engine.as_str(), "SQLite" | "PostgreSQL")
    }

    fn supports_schema_changes(&self) -> bool {
        matches!(self.engine.as_str(), "SQLite" | "PostgreSQL")
            && self
                .objects
                .iter()
                .any(|object| object.name == self.browse.table && object.kind == "table")
    }

    /// Row planners are distinct from schema rebuild/import capabilities.
    fn supports_data_changes(&self) -> bool {
        self.engine == "SQLite" || (self.engine == "PostgreSQL" && !self.columns.is_empty())
    }

    fn supports_table_export(&self) -> bool {
        matches!(self.engine.as_str(), "SQLite" | "PostgreSQL")
    }

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::fonts::install(&cc.egui_ctx);
        let prefs: Preferences = cc
            .storage
            .and_then(|s| eframe::get_value(s, "workspace"))
            .unwrap_or_default();
        Self::with_launcher_preferences(&cc.egui_ctx, prefs)
    }

    /// Host the real workspace directly on the disposable sample database.
    /// Demo sessions deliberately ignore saved connections and recovered drafts.
    pub fn new_demo(cc: &eframe::CreationContext<'_>) -> Self {
        crate::fonts::install(&cc.egui_ctx);
        Self::with_initial_connection(
            &cc.egui_ctx,
            Preferences::default(),
            Command::Open(None),
            None,
        )
    }

    #[cfg(test)]
    fn with_preferences(ctx: &egui::Context, prefs: Preferences) -> Self {
        Self::with_initial_connection(ctx, prefs, Command::Open(None), None)
    }

    fn with_launcher_preferences(ctx: &egui::Context, prefs: Preferences) -> Self {
        let mut app = Self::with_optional_connection(ctx, prefs, None, None);
        for mut tab in app.tabs.drain(..).chain(app.closed_tabs.drain(..)) {
            if !tab.sql.trim().is_empty() || tab.source_path.is_some() {
                tab.open = false;
                app.window_recovery.push(tab);
            }
        }
        app.tabs.push(QueryTab::new(String::new()));
        app.connections = true;
        app.label = "Not connected".into();
        app
    }

    fn with_initial_connection(
        ctx: &egui::Context,
        prefs: Preferences,
        initial: Command,
        identity: Option<ProfileIdentity>,
    ) -> Self {
        Self::with_optional_connection(ctx, prefs, Some(initial), identity)
    }

    fn with_optional_connection(
        ctx: &egui::Context,
        mut prefs: Preferences,
        initial: Option<Command>,
        identity: Option<ProfileIdentity>,
    ) -> Self {
        connections::migrate_recent(&mut prefs);
        if prefs.query_history.is_empty() {
            prefs.query_history = prefs.history.iter().map(|sql| query_history::Entry { sql: sql.clone(), timestamp: None }).collect();
        }
        apply_theme(ctx, prefs.appearance);
        let tabs = if !prefs.documents.is_empty() {
            std::mem::take(&mut prefs.documents)
                .into_iter()
                .map(QueryDocument::restore)
                .collect()
        } else if prefs.queries.is_empty() && initial.is_none() {
            vec![QueryTab::new(String::new())]
        } else if prefs.queries.is_empty() {
            vec![QueryTab::new("-- Run current: Cmd/Ctrl+Enter. Run all: Cmd/Ctrl+Shift+Enter.\nSELECT plan, COUNT(*) AS customers,\n       ROUND(SUM(lifetime_value), 2) AS revenue\nFROM customers\nGROUP BY plan\nORDER BY revenue DESC;\n\nSELECT * FROM customers LIMIT 100;".into())]
        } else {
            std::mem::take(&mut prefs.queries)
                .into_iter()
                .map(QueryTab::new)
                .collect()
        };
        let closed_tabs = std::mem::take(&mut prefs.closed_documents)
            .into_iter()
            .map(QueryDocument::restore)
            .collect();
        let window_recovery = std::mem::take(&mut prefs.window_documents)
            .into_iter()
            .filter(QueryDocument::recoverable)
            .map(QueryDocument::restore)
            .collect();
        prefs.queries.clear();
        let worker = Worker::new(ctx.clone());
        let (files_tx, files_rx) = mpsc::channel();
        let mut app = Self {
            child_window: false,
            recover_next_window: false,
            show_recovered_on_connect: false,
            shared_preferences_revision: 0,
            shared_preferences_dirty: false,
            manager_requested: false,
            window_requests: Vec::new(),
            worker,
            prefs,
            pending: None,
            started: Instant::now(),
            label: "Connecting…".into(),
            engine: String::new(),
            schemas: db::SchemaCatalog::default(),
            connected_database: None,
            table_display_name: None,
            connection_manager: Default::default(),
            language: Default::default(),
            active_profile: None,
            profiles_revision: 0,
            objects: vec![],
            browse: Browse::default(),
            columns: vec![],
            indexes: vec![],
            triggers: vec![],
            selected_index: None,
            table: Arc::default(),
            tabs,
            closed_tabs,
            window_recovery,
            tab: 0,
            view: View::Data,
            search: String::new(),
            filter: String::new(),
            filters_visible: false,
            focus_filter: false,
            focus_page_grid: false,
            filter_column: String::new(),
            safe: false,
            status: "Ready".into(),
            error: None,
            selected: None,
            row_selection: Default::default(),
            saved_selection: None,
            selected_column: None,
            structure_search: String::new(),
            field_search: String::new(),
            csv_preview: None,
            csv_mapping: csv_mapping::Mapping::default(),
            csv_empty_as_null: false,
            csv_postgres_inserts: false,
            csv_review: None,
            inspector: true,
            history: false,
            query_library: Default::default(),
            #[cfg(target_arch = "wasm32")]
            browser_export: Default::default(),
            navigator: navigator::Navigator::new(initial.as_ref()),
            activity: Default::default(),
            last_operation_ms: None,
            sidebar_section: 0,
            history_search: String::new(),
            history_selected: None,
            connections: false,
            edit: None,
            changes: Vec::new(),
            change_history: change_history::ChangeHistory::default(),
            review: None,
            review_sql: Vec::new(),
            rename: None,
            rename_inline: false,
            rename_focus: false,
            add_column: None,
            alter_column: None,
            index_draft: None,
            files_tx,
            files_rx,
            frame_ms: 0.0,
        };
        let pending = identity.map_or(Pending::Connect(None), |identity| Pending::ConnectProfile {
            recent: None,
            identity,
        });
        if let Some(initial) = initial {
            app.dispatch(initial, pending);
        }
        app
    }

    fn dispatch(&mut self, command: Command, pending: Pending) {
        if self.pending.is_some() {
            return;
        }
        if matches!(command, Command::Open(_) | Command::OpenPostgres(_))
            && (self.worker.transaction_state().blocks_replacement()
                || !self.changes.is_empty()
                || self.edit.is_some()
                || (self.rename_inline && self.rename.is_some()))
        {
            self.error =
                Some("Finish SQL transactions with COMMIT or ROLLBACK and save or discard pending changes before switching connections.".into());
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if matches!(command, Command::Open(None)) { self.worker.restart_if_failed(); }
        let history_sql = match &command { Command::Query { sql, .. } => Some(sql.clone()), _ => None };
        let description = activity_description(&command);
        match self.worker.send(command) {
            Ok(()) => {
                if let Some(sql) = description { self.log_activity(sql); }
                if let Some(sql) = history_sql { self.record_query_history(sql); }
                self.pending = Some(pending);
                self.started = Instant::now();
                self.error = None;
                self.status = "Working…".into();
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn poll(&mut self) {
        self.poll_query_groups();
        #[cfg(target_arch = "wasm32")]
        self.worker.check_health();
        #[cfg(target_arch = "wasm32")]
        if self.browser_export.borrow().as_ref().is_some_and(|worker| worker.expired()) {
            self.browser_export.borrow_mut().take();
            self.error = Some("Export timed out. No download was created.".into());
        }
        while let Ok(result) = self.worker.rx.try_recv() {
            let elapsed = self.started.elapsed().as_secs_f64() * 1000.0;
            self.last_operation_ms = Some(elapsed);
            match &result {
                Ok(Payload::Table { grid, .. }) => self.last_operation_ms = Some(grid.elapsed.as_secs_f64() * 1000.0),
                Ok(Payload::Query { report, .. }) => self.last_operation_ms = Some(report.elapsed.as_secs_f64() * 1000.0),
                _ => {}
            }
            if let Ok(Payload::Table {
                sql, parameters, ..
            }) = &result
            {
                let statement = bounded_log_text(&crate::review_sql::readable(sql, parameters));
                self.record_query_history(statement.clone());
                self.log_activity(statement);
            }
            let pending = self.pending.take();
            match result {
                Ok(Payload::Databases(names)) => { self.navigator.databases = names; self.navigator.loaded = true; }
                Err(e) => {
                    if matches!(pending, Some(Pending::Table)) {
                        self.saved_selection = None;
                    }
                    self.status = "Operation failed".into();
                    self.error = Some(e);
                }
                Ok(Payload::Planned(plan)) => {
                    self.review_sql = plan
                        .statements
                        .iter()
                        .map(|s| crate::review_sql::readable(&s.sql, &s.parameters))
                        .collect();
                    self.review = Some(plan);
                    self.status = "Review generated changes".into();
                    if matches!(pending, Some(Pending::Plan(true))) {
                        self.save_changes(false);
                    }
                }
                Ok(Payload::Applied { objects }) => {
                    self.saved_selection = self
                        .selected
                        .and_then(|i| self.table.rows.get(i))
                        .and_then(|row| {
                            saved_row_identity(
                                &self.browse.table,
                                &self.columns,
                                row,
                                &self.changes,
                            )
                        })
                        .map(|keys| (self.browse.table.clone(), keys));
                    self.changes.clear();
                    self.clear_change_history();
                    self.review = None;
                    self.status = "Changes saved".into();
                    match objects {
                        Ok(objects) => {
                            self.objects = objects;
                            self.language.invalidate();
                            self.refresh();
                        }
                        Err(error) => {
                            self.objects.clear();
                            self.error =
                                Some(format!("Changes saved, but schema refresh failed: {error}"));
                        }
                    }
                }
                Ok(Payload::Connected {
                    engine,
                    label,
                    database,
                    objects,
                    schemas,
                }) => {
                    self.clear_change_history();
                    self.csv_preview = None;
                    self.csv_review = None;
                    self.csv_mapping = csv_mapping::Mapping::default();
                    let (recent, identity) = match pending {
                        Some(Pending::ConnectProfile { recent, identity }) => {
                            (recent, Some(identity))
                        }
                        Some(Pending::Connect(recent)) => (recent, None),
                        _ => (None, None),
                    };
                    self.active_profile = identity;
                    if let Some(path) = recent {
                        self.prefs.recent.retain(|p| p != &path);
                        self.prefs.recent.insert(0, path);
                        self.prefs.recent.truncate(12);
                    }
                    self.edit = None;
                    self.rename = None;
                    self.add_column = None;
                    self.alter_column = None;
                    self.index_draft = None;
                    self.selected_column = None;
                    self.structure_search.clear();
                    self.label = label;
                    self.connected_database = database;
                    self.engine = engine;
                    self.schemas = schemas;
                    self.objects = objects;
                    self.language.invalidate();
                    self.safe = false;
                    self.table = Arc::default();
                    self.columns.clear();
                    self.indexes.clear();
                    self.triggers.clear();
                    self.selected_index = None;
                    self.browse = Browse::default();
                    for tab in &mut self.tabs {
                        tab.grid = Arc::default();
                        tab.results.clear();
                        tab.chart = Default::default();
            tab.formatter = Default::default();
            tab.pins.clear();
            tab.selected_pin = None;
                    }
                    self.status = "Connected".into();
                    self.connections = false;
                    if let Some(object) = self.objects.first() {
                        self.open_table(object.name.clone());
                    }
                    if std::mem::take(&mut self.show_recovered_on_connect) {
                        self.show_query();
                    }
                }
                Ok(Payload::Table {
                    sql: _,
                    parameters: _,
                    grid,
                    columns,
                    indexes,
                    triggers,
                }) => {
                    self.status = format!(
                        "{} rows loaded in {:.1} ms",
                        grid.rows.len(),
                        grid.elapsed.as_secs_f64() * 1000.0
                    );
                    self.table = Arc::new(grid);
                    self.columns = columns;
                    self.indexes = indexes;
                    self.triggers = triggers;
                    self.row_selection.clear();
                    self.selected = self.saved_selection.take().and_then(|(table, keys)| {
                        if table != self.browse.table {
                            return None;
                        }
                        find_saved_row(&self.table, &keys)
                    });
                    if !self.columns.iter().any(|c| c.name == self.filter_column) {
                        self.filter_column = self
                            .columns
                            .first()
                            .map(|c| c.name.clone())
                            .unwrap_or_default();
                    }
                }
                Ok(Payload::Query {
                    report,
                    objects,
                    schemas,
                }) => {
                    self.schemas = schemas;
                    self.status = format!(
                        "{} statements completed · {:.1} ms",
                        report.grids.len(),
                        report.elapsed.as_secs_f64() * 1000.0
                    );
                    if let Some(Pending::Query(index)) = pending {
                        self.tabs[index].result_view = if report.grids.is_empty() && report.error.is_some() { ResultView::Message } else { ResultView::Data };
                        self.tabs[index].result_error = report.error.clone();
                        self.tabs[index].results = report.grids;
                        self.tabs[index].result_index = 0;
                        self.tabs[index].selected_pin = None;
                        self.tabs[index].grid = self.tabs[index]
                            .results
                            .first()
                            .cloned()
                            .unwrap_or_default();
                    }
                    self.error = report.error;
                    match objects {
                        None => {}
                        Some(Ok(objects)) => {
                            self.objects = objects;
                            self.language.invalidate();
                        }
                        Some(Err(error)) => {
                            self.objects.clear();
                            let warning = format!(
                                "Schema refresh failed after execution: {error}. Query results are retained. Review execution results before retrying writes."
                            );
                            self.error = Some(match self.error.take() {
                                Some(execution_error) => format!("{execution_error}\n\n{warning}"),
                                None => warning,
                            });
                        }
                    }
                    self.row_selection.clear();
                    self.selected = None;
                }
                Ok(Payload::Imported(count)) => {
                    self.csv_preview = None;
                    self.csv_review = None;
                    self.status = format!("Imported {count} CSV rows");
                    if !self.browse.table.is_empty() {
                        self.refresh();
                    }
                }
                Ok(Payload::ImportTarget(object)) => {
                    self.status = format!("Import destination ready: {}", object.name);
                    if let Some(preview) = &self.csv_preview {
                        self.csv_mapping = csv_mapping::Mapping::new(
                            object.name,
                            object.columns,
                            &preview.columns,
                            preview.options.header,
                        );
                    }
                }
                Ok(Payload::CsvPreview(preview)) => {
                    self.status = format!("Previewed {} CSV rows", preview.rows.len());
                    let (table, columns) = if self
                        .csv_preview
                        .as_ref()
                        .is_some_and(|old| old.path == preview.path)
                    {
                        (
                            self.csv_mapping.table.clone(),
                            self.csv_mapping.columns.clone(),
                        )
                    } else {
                        (String::new(), Vec::new())
                    };
                    self.csv_mapping = csv_mapping::Mapping::new(
                        table,
                        columns,
                        &preview.columns,
                        preview.options.header,
                    );
                    self.csv_preview = Some(preview);
                }
                Ok(Payload::ScriptOpened { path, sql }) => {
                    let activate = self.finish_cell_edit();
                    let mut tab = QueryTab::new(sql);
                    tab.saved(path.clone(), tab.sql.clone());
                    self.tabs.push(tab);
                    if activate {
                        self.activate_query(self.tabs.len() - 1);
                    }
                    self.status = format!("Opened SQL: {}", path.display());
                }
                Ok(Payload::ScriptSaved { path, sql, tab_id }) => {
                    if let Some(tab) = self
                        .tabs
                        .iter_mut()
                        .chain(self.closed_tabs.iter_mut())
                        .find(|t| t.id == tab_id)
                    {
                        tab.saved(path.clone(), sql);
                    }
                    self.query_library.invalidate();
                    self.status = format!("Saved SQL: {}", path.display())
                }
                Ok(Payload::Schema { objects, schemas }) => {
                    self.schemas = schemas;
                    self.objects = objects;
                    self.language.invalidate();
                    self.status = "Schema refreshed".into();
                }
                #[cfg(target_arch = "wasm32")]
                Ok(Payload::Download { bytes, format }) => self.download_table(&bytes, format),
                Ok(Payload::Exported(path)) => {
                    self.status = format!("Exported to {}", path.display())
                }
            }
        }
        while let Ok(action) = self.files_rx.try_recv() {
            // Browser file reads are independent of database requests. Never discard
            // a completed import merely because a query is still running.
            #[cfg(target_arch = "wasm32")]
            let action = match action {
                FileAction::BrowserExportDone(result) => {
                    self.browser_export.borrow_mut().take();
                    match result { Ok(name) => self.status = format!("Download requested: {name}"), Err(error) => self.error = Some(error) }
                    continue;
                }
                FileAction::BrowserScript(name, sql) => {
                    let activate = self.finish_cell_edit();
                    let mut tab = QueryTab::new(sql);
                    tab.source_name = Some(name.clone());
                    self.tabs.push(tab);
                    if activate { self.activate_query(self.tabs.len() - 1); }
                    self.status = format!("Imported SQL: {name}");
                    continue;
                }
                FileAction::BrowserDownload(name) => {
                    self.status = format!("Download requested: {name}");
                    continue;
                }
                action => action,
            };
            if self.pending.is_some() {
                self.error = Some(
                    "A database operation is running. Try the file action again when it finishes."
                        .into(),
                );
                continue;
            }
            match action {
                #[cfg(target_arch = "wasm32")]
                FileAction::BrowserScript(..) | FileAction::BrowserDownload(..) | FileAction::BrowserExportDone(..) => unreachable!(),
                FileAction::QueryLibraryError(error) => self.error = Some(error),
                FileAction::QueryDirectory(path) => { self.prefs.query_directory = Some(path); self.query_library.invalidate(); self.shared_preferences_dirty = true; }
                FileAction::ProfilePath(generation, key, path) => {
                    self.connection_manager.set_path(generation, key, path)
                }
                FileAction::OpenScript(path) => {
                    self.dispatch(Command::OpenScript(path), Pending::Export)
                }
                FileAction::SaveScript(path, sql, tab_id, expected) => self.dispatch(
                    Command::SaveScript {
                        path,
                        sql,
                        tab_id,
                        expected,
                    },
                    Pending::Export,
                ),
                FileAction::PreviewCsv(path) => self.dispatch(
                    Command::PreviewCsv {
                        path,
                        options: Default::default(),
                        source: None,
                    },
                    Pending::Export,
                ),
                FileAction::Open(path) => self.dispatch(
                    Command::Open(Some(path.clone())),
                    Pending::Connect(Some(path)),
                ),
                FileAction::ExportTable(path, browse, label, format) => {
                    if self.label != label {
                        self.error = Some("Connection changed while choosing the export destination. Start export again.".into());
                    } else {
                        #[cfg(not(target_arch = "wasm32"))]
                        self.dispatch(Command::ExportTable { path, browse, format }, Pending::Export);
                        #[cfg(target_arch = "wasm32")]
                        { let _ = path; self.dispatch(Command::DownloadTable { browse, format }, Pending::Export); }
                    }
                }
                FileAction::Export(path, grid, format) => {
                    self.dispatch(Command::Export { path, grid, format }, Pending::Export)
                }
            }
        }
    }

    fn open_table(&mut self, name: String) {
        if self.pending.is_some() {
            return;
        }
        if !self.finish_cell_edit() {
            return;
        }
        self.focus_page_grid = false;
        self.table_display_name = self
            .objects
            .iter()
            .find(|object| object.name == name)
            .map(|object| (name.clone(), object.display_name().to_owned()));
        self.saved_selection = None;
        self.rename = None;
        self.selected_column = None;
        self.structure_search.clear();
        self.browse = Browse {
            table: name,
            ..Default::default()
        };
        self.filter.clear();
        self.view = View::Data;
        self.refresh();
    }

    fn current_table_name(&self) -> &str {
        self.table_display_name
            .as_ref()
            .filter(|(name, _)| name == &self.browse.table)
            .map(|(_, display)| display.as_str())
            .unwrap_or(&self.browse.table)
    }

    fn change_table_page(&mut self, next: bool) {
        if self.view != View::Data || self.pending.is_some() || self.browse.table.is_empty() {
            return;
        }
        let offset = if next {
            if !self.table.truncated || self.table.rows.len() != db::PAGE_SIZE { return; }
            let Some(offset) = self.browse.offset.checked_add(db::PAGE_SIZE) else { return; };
            offset
        } else {
            self.browse.offset.saturating_sub(db::PAGE_SIZE)
        };
        if offset == self.browse.offset || !self.finish_cell_edit() { return; }
        self.browse.offset = offset;
        self.refresh();
    }

    fn refresh(&mut self) {
        if !self.finish_cell_edit() {
            return;
        }
        if self.pending.is_some() || self.browse.table.is_empty() {
            return;
        }
        self.table = Arc::default();
        self.columns.clear();
        self.indexes.clear();
        self.triggers.clear();
        self.selected_index = None;
        self.selected_column = None;
        self.row_selection.clear();
        self.selected = None;
        self.dispatch(Command::Browse(self.browse.clone()), Pending::Table);
    }

    fn run_query(&mut self, all: bool) {
        if self.pending.is_some() {
            return;
        }
        if self.view != View::Query {
            self.show_query();
            self.tabs[self.tab].focus_editor = true;
            return;
        }
        let tab = &self.tabs[self.tab];
        let Some(range) = crate::sql::execution_range_for_dialect(
            &tab.sql,
            tab.cursor.clone(),
            all,
            crate::sql::Dialect::for_engine(&self.engine),
        ) else {
            return;
        };
        let sql = tab.sql[range].trim().to_owned();
        if sql.is_empty() {
            return;
        }
        self.prefs.history.retain(|s| s != &sql);
        self.prefs.history.insert(0, sql.clone());
        self.prefs.history.truncate(100);
        self.shared_preferences_dirty = true;
        self.show_query();
        self.dispatch(
            Command::Query {
                sql,
                safe: self.safe,
            },
            Pending::Query(self.tab),
        );
    }

    fn grid(&self) -> Arc<Grid> {
        if self.view == View::Query {
            self.tabs[self.tab].grid.clone()
        } else {
            self.table.clone()
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn preview_csv_dialog(&self, ctx: &egui::Context) {
        let tx = self.files_tx.clone();
        let viewport = ctx.viewport_id();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("CSV", &["csv"])
                .pick_file()
            {
                let _ = tx.send(FileAction::PreviewCsv(path));
                ctx.request_repaint_of(viewport);
            }
        });
    }

    fn csv_preview_window(&mut self, ctx: &egui::Context) {
        let Some(preview) = &self.csv_preview else {
            return;
        };
        let mut open = true;
        let mut options = preview.options;
        let mut target = None;
        egui::Window::new("CSV preview").id(egui::Id::new((ctx.viewport_id(), "CSV preview")))
            .open(&mut open)
            .default_size([760.0, 420.0])
            .show(ctx, |ui| {
                ui.label(
                    preview
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                );
                ui.weak("Review the mapping before importing. No changes until you confirm.");
                if let Some(source) = &preview.source {
                    ui.weak(format!("Source snapshot · {} bytes", source.bytes)).on_hover_text("Changing the original file will not change this preview. Reopen the file to load a new version.");
                }
                ui.horizontal(|ui| {
                    ui.label("Destination");
                    ui.add_enabled_ui(self.pending.is_none(), |ui| {
                        egui::ComboBox::from_id_salt("csv-target-table")
                            .selected_text(if self.csv_mapping.table.is_empty() {
                                "Select a table…"
                            } else {
                                &self.csv_mapping.table
                            })
                            .show_ui(ui, |ui| {
                                for object in self.objects.iter().filter(|o| o.kind == "table") {
                                    if ui
                                        .selectable_label(
                                            self.csv_mapping.table == object.name,
                                            &object.name,
                                        )
                                        .clicked()
                                    {
                                        target = Some(object.name.clone());
                                    }
                                }
                            });
                    });
                    match self.csv_mapping.validate() {
                        Ok(count) => {
                            ui.weak(format!("{count} columns mapped"));
                        }
                        Err(error) => {
                            ui.colored_label(ui.visuals().warn_fg_color, error);
                        }
                    }
                });
                ui.label(format!(
                    "{} columns · {} preview rows{}",
                    preview.columns.len(),
                    preview.rows.len(),
                    if preview.more {
                        " · more rows in file"
                    } else {
                        ""
                    }
                ));
                ui.add_enabled_ui(self.pending.is_none(), |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut options.header, "First row is header");
                        egui::ComboBox::from_id_salt("csv-delimiter")
                            .selected_text(format!(
                                "Delimiter: {}",
                                if options.delimiter == b'\t' {
                                    "Tab".into()
                                } else {
                                    (options.delimiter as char).to_string()
                                }
                            ))
                            .show_ui(ui, |ui| {
                                for (value, label) in [
                                    (b',', "Comma"),
                                    (b';', "Semicolon"),
                                    (b'\t', "Tab"),
                                    (b'|', "Pipe"),
                                ] {
                                    ui.selectable_value(&mut options.delimiter, value, label);
                                }
                            });
                        egui::ComboBox::from_id_salt("csv-quote")
                            .selected_text(format!("Quote: {}", options.quote as char))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut options.quote, b'"', "Double quote");
                                ui.selectable_value(&mut options.quote, b'\'', "Single quote");
                            });
                        ui.weak("UTF-8");
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.csv_empty_as_null, "Empty fields become NULL").on_hover_text("Otherwise empty fields stay empty text. The database converts CSV text using each destination column type.");
                    if self.engine == "PostgreSQL" {
                        ui.checkbox(&mut self.csv_postgres_inserts, "Use INSERT (supports row-level security)")
                            .on_hover_text("Batches rows through INSERT so PostgreSQL applies row policies. COPY is faster for tables without row-level security.");
                    }
                    if ui.add_enabled(matches!(self.engine.as_str(), "SQLite" | "PostgreSQL") && !self.safe && self.pending.is_none() && options == preview.options && self.csv_mapping.validate().is_ok(), egui::Button::new("Review import…")).clicked()
                        && let Some(source) = &preview.source {
                        let request = db::csv_import::Request {
                            source: source.clone(), options, table: self.csv_mapping.table.clone(), columns: self.csv_mapping.columns.clone(),
                            destinations: self.csv_mapping.destinations.clone(), empty_as_null: self.csv_empty_as_null, postgres_inserts: self.csv_postgres_inserts,
                        };
                        match if self.engine == "PostgreSQL" { request.postgres_sql() } else { request.sql() } {
                            Ok(sql) => {
                                let mut bytes = 0;
                                let mut samples: Vec<String> = preview.rows.iter().take(10).map(|row| {
                                    let values = request.values(row);
                                    let size: usize = values.iter().map(|v| match v { Value::Text(t) => t.len(), _ => 8 }).sum();
                                    bytes += size;
                                    if bytes > 32_768 { "-- Large row omitted from the visual sample; its full value will be imported.".into() }
                                    else if self.engine == "PostgreSQL" { request.source_sample(row) }
                                    else { crate::review_sql::expand(&sql, &values) }
                                }).collect();
                                if self.engine == "PostgreSQL" { samples.insert(0, sql); }
                                self.csv_review = Some((request, samples));
                            }
                            Err(error) => self.error = Some(error),
                        }
                    }
                });
                egui::ScrollArea::both()
                    .max_height(340.0)
                    .show_viewport(ui, |ui, viewport| {
                        let cell = egui::vec2(170.0, 24.0);
                        let origin = ui.cursor().min;
                        ui.set_min_size(egui::vec2(
                            cell.x * preview.columns.len() as f32,
                            cell.y * (preview.rows.len() + 2) as f32,
                        ));
                        let start_row = (viewport.min.y / cell.y).floor().max(0.0) as usize;
                        let end_row =
                            ((viewport.max.y / cell.y).ceil() as usize).min(preview.rows.len() + 2);
                        let start_col = (viewport.min.x / cell.x).floor().max(0.0) as usize;
                        let end_col =
                            ((viewport.max.x / cell.x).ceil() as usize).min(preview.columns.len());
                        for row in start_row..end_row {
                            for column in start_col..end_col {
                                let rect = egui::Rect::from_min_size(
                                    origin
                                        + egui::vec2(column as f32 * cell.x, row as f32 * cell.y),
                                    cell,
                                );
                                if row == 0 {
                                    let selected = self.csv_mapping.destinations[column];
                                    ui.scope_builder(
                                        egui::UiBuilder::new().max_rect(rect.shrink(2.0)),
                                        |ui| {
                                            egui::ComboBox::from_id_salt(("csv-map", column))
                                                .width(cell.x - 8.0)
                                                .selected_text(
                                                    selected
                                                        .and_then(|i| {
                                                            self.csv_mapping.columns.get(i)
                                                        })
                                                        .map_or("Do not import", |c| {
                                                            c.name.as_str()
                                                        }),
                                                )
                                                .show_ui(ui, |ui| {
                                                    ui.selectable_value(
                                                        &mut self.csv_mapping.destinations[column],
                                                        None,
                                                        "Do not import",
                                                    );
                                                    for (index, target) in self
                                                        .csv_mapping
                                                        .columns
                                                        .iter()
                                                        .enumerate()
                                                        .filter(|(_, c)| !c.generated)
                                                    {
                                                        ui.selectable_value(
                                                            &mut self.csv_mapping.destinations
                                                                [column],
                                                            Some(index),
                                                            format!(
                                                                "{} · {}",
                                                                target.name, target.data_type
                                                            ),
                                                        );
                                                    }
                                                });
                                        },
                                    );
                                    continue;
                                }
                                let painter =
                                    ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                                if row == 0 || row % 2 == 0 {
                                    painter.rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
                                }
                                let value = if row == 1 {
                                    &preview.columns[column]
                                } else {
                                    &preview.rows[row - 2][column]
                                };
                                let text: String = value.chars().take(120).collect();
                                painter.text(
                                    rect.left_center() + egui::vec2(5.0, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    text,
                                    egui::FontId::monospace(12.0),
                                    ui.visuals().text_color(),
                                );
                                painter.line_segment(
                                    [rect.right_top(), rect.right_bottom()],
                                    ui.visuals().widgets.noninteractive.bg_stroke,
                                );
                            }
                        }
                    });
            });
        let reload = (options != preview.options).then(|| preview.path.clone());
        if let Some(path) = reload {
            self.dispatch(
                Command::PreviewCsv {
                    path,
                    options,
                    source: preview.source.clone(),
                },
                Pending::Export,
            );
        } else if let Some(table) = target {
            // Do not keep a previous destination's mapping while metadata loads.
            self.csv_mapping = csv_mapping::Mapping::new(
                table.clone(),
                Vec::new(),
                &preview.columns,
                preview.options.header,
            );
            self.dispatch(Command::ImportTarget(table), Pending::ImportMetadata);
        }
        if !open {
            self.csv_preview = None;
        }
    }

    fn refresh_current_view(&mut self) {
        if self.pending.is_some() || self.review.is_some() {
            return;
        }
        if !self.changes.is_empty()
            || self.edit.is_some()
            || (self.rename_inline && self.rename.is_some())
        {
            self.error = Some("Save or discard pending changes before refreshing.".into());
            return;
        }
        if self.view == View::Query || self.browse.table.is_empty() {
            self.dispatch(Command::RefreshSchema, Pending::Export);
        } else {
            self.refresh();
        }
    }

    fn dialog_owns_workspace_shortcuts(&self) -> bool {
        self.query_library.owns_keyboard() || self.navigator.open || self.connections || self.review.is_some() || self.history
            || (self.rename.is_some() && !self.rename_inline)
            || self.add_column.is_some() || self.alter_column.is_some()
            || self.index_draft.is_some() || self.csv_preview.is_some() || self.csv_review.is_some()
    }

    fn workspace_shortcuts(&mut self, ctx: &egui::Context) {
        // Import windows own the keyboard, including their first visible frame.
        if self.csv_preview.is_some() || self.csv_review.is_some() {
            return;
        }
        if self.review.is_some() {
            // Review owns confirmation; Run All must never execute the editor
            // behind it, including on the first frame before egui assigns focus.
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Enter));
            let save = ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::S)
                    || i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)
                    || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
            });
            if save { self.save_changes(false); }
            if self.pending.is_none() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.review = None;
            }
            return;
        }
        if self.dialog_owns_workspace_shortcuts() { return; }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::P)) {
            self.open_navigator(false);
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
            self.open_navigator(true);
            return;
        }
        self.workspace_tab_shortcuts(ctx);
        self.staged_history_shortcuts(ctx);
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::R)) {
            self.refresh_current_view();
        }
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::O,
            )
        }) && self.pending.is_none()
        {
            self.script_dialog(ctx, false);
        }
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::Backspace,
            )
        }) {
            self.discard_changes();
        }
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::P,
            )
        }) {
            self.preview_changes();
        }
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::S,
            )
        }) {
            if self.view == View::Query { self.save_query_file(ctx, true); } else { self.save_changes(true); }
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            if self.view == View::Query { self.save_query_file(ctx, false); } else { self.save_changes(false); }
        }
        // Consume the more specific chord first: Cmd+Shift+Enter must not become Run Current.
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::Enter,
            )
        }) {
            self.run_query(true);
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
            if self.review.is_some() {
                self.save_changes(false);
            } else {
                self.run_query(false);
            }
        }
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::T,
            )
        }) {
            self.reopen_tab();
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::T)) {
            self.new_tab();
        }
        if self.view == View::Data
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::I))
        {
            self.add_insert();
        }
        if self.view == View::Data
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::D))
        {
            self.duplicate_selected();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::E)) {
            self.show_query();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::W)) {
            self.close_active_tab_or_window(ctx);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O))
            && self.pending.is_none()
        {
            self.file_dialog(ctx, false);
        }
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::CTRL,
                egui::Key::C,
            )
        }) {
            self.prefs.activity_visible = !self.prefs.activity_visible;
        }
        if self.view == View::Data
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F))
            && self.finish_cell_edit()
        {
            self.filters_visible = true;
            self.focus_filter = true;
        }
        if self.view == View::Query {
            let results_focused = self.tabs[self.tab].result_search.has_focus(ctx)
                || self.tabs[self.tab].result_grid_focus.is_some_and(|id| ctx.memory(|m| m.has_focus(id)));
            if !results_focused && self.tabs[self.tab].search.open
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                self.tabs[self.tab].search.open = false;
                self.tabs[self.tab].focus_editor = true;
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
                self.tabs[self.tab].focus_editor = false;
                if results_focused {
                    let tab = &mut self.tabs[self.tab];
                    tab.result_view = ResultView::Data;
                    tab.result_search.open = true;
                    tab.result_search.focus = true;
                } else {
                    let search = &mut self.tabs[self.tab].search;
                    search.open = true;
                    search.focus = true;
                }
            }
            if ctx.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::G,
                )
            }) {
                if results_focused { if self.tabs[self.tab].result_search.open { self.tabs[self.tab].result_search.step = -1; } }
                else { self.tabs[self.tab].search.action = Some(crate::find::Action::Previous); }
            } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::G)) {
                if results_focused { if self.tabs[self.tab].result_search.open { self.tabs[self.tab].result_search.step = 1; } }
                else { self.tabs[self.tab].search.action = Some(crate::find::Action::Next); }
            }
        }
    }

    fn commit_csv_import(&mut self) {
        if !self.supports_csv_import() || self.safe || self.pending.is_some() {
            return;
        }
        if let Some((request, _)) = &self.csv_review {
            let request = request.clone();
            self.dispatch(
                Command::ImportCsv {
                    request,
                    safe: self.safe,
                },
                Pending::Import,
            );
        }
    }

    fn csv_import_review(&mut self, ctx: &egui::Context) {
        let Some((request, samples)) = &self.csv_review else {
            return;
        };
        let mut commit = false;
        let mut close = false;
        let importing = matches!(self.pending, Some(Pending::Import));
        egui::Modal::new(egui::Id::new((ctx.viewport_id(), "csv-import-review"))).show(ctx, |ui| {
            ui.set_max_width(700.0);
            ui.heading(format!("Import into {}", request.table));
            ui.label("All source rows will be inserted in one transaction. An error or cancellation rolls back the entire import.");
            ui.weak(if self.engine == "PostgreSQL" { "Import command followed by up to 10 source rows with destination column names. Quoted values are CSV text; the database converts them to column types." } else { "Up to 10 inserts shown below. Remaining rows use the same column mapping." });
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                for (index, sample) in samples.iter().enumerate() {
                    ui.push_id(("import-review-sample", index), |ui| {
                        let is_sql = self.engine != "PostgreSQL" || index == 0;
                        if self.engine == "PostgreSQL" && index <= 1 {
                            ui.strong(if index == 0 { "Import command" } else { "Mapped source rows" });
                        }
                        if is_sql {
                            if clipboard_button(ui).on_hover_text("Copy import SQL").clicked() { ui.ctx().copy_text(sample.clone()); }
                            ui.add(egui::Label::new(crate::editor::layout(ui, &sample.as_str(), ui.available_width())).selectable(true));
                        } else {
                            ui.add(egui::Label::new(RichText::new(sample).monospace()).wrap().selectable(true));
                        }
                    });
                }
            });
            if let Some(error) = &self.error {
                egui::Frame::new().fill(ui.visuals().extreme_bg_color).inner_margin(8.0).show(ui, |ui| {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                    ui.weak("Your source and mapping are retained. Return to mapping to adjust the import.");
                });
            }
            if importing {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Importing rows…");
                    if ui.button("Cancel import").clicked() { self.worker.cancel(); }
                });
            }
            ui.horizontal(|ui| {
                commit = ui.add_enabled(matches!(self.engine.as_str(), "SQLite" | "PostgreSQL") && !self.safe && self.pending.is_none(), egui::Button::new("Import all rows")).on_hover_text("Cmd/Ctrl+Enter or Cmd/Ctrl+S").clicked();
                close = ui.add_enabled(!importing, egui::Button::new("Back to mapping")).on_hover_text("Escape").clicked();
            });
        });
        // Exact chords prevent Shift+Enter (run all) from confirming an import.
        ctx.input_mut(|i| {
            i.events.retain(|event| {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = event
                    && matches!(key, egui::Key::Enter | egui::Key::S)
                    && modifiers.command
                    && !modifiers.shift
                    && !modifiers.alt
                    && !(modifiers.ctrl && modifiers.mac_cmd)
                {
                    commit = true;
                    false
                } else {
                    true
                }
            });
            close |= i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
        });
        if close && !importing {
            self.csv_review = None;
        } else if commit {
            self.commit_csv_import();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn export_table_dialog(&self, ctx: &egui::Context, format: db::ExportFormat) {
        if !self.supports_table_export() {
            return;
        }
        let tx = self.files_tx.clone();
        let viewport = ctx.viewport_id();
        let ctx = ctx.clone();
        let browse = self.browse.clone();
        let label = self.label.clone();
        let (name, extension) = match format { db::ExportFormat::Csv => ("CSV", "csv"), db::ExportFormat::Json => ("JSON", "json") };
        std::thread::spawn(move || {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter(name, &[extension])
                .set_file_name(format!("table.{extension}"))
                .save_file()
            {
                let _ = tx.send(FileAction::ExportTable(path, browse, label, format));
                ctx.request_repaint_of(viewport);
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn script_dialog(&self, ctx: &egui::Context, save: bool) {
        let tab_id = self.tabs[self.tab].id;
        let source_path = self.tabs[self.tab].source_path.clone();
        let saved_sql = self.tabs[self.tab].saved_sql.clone();
        let tx = self.files_tx.clone();
        let viewport = ctx.viewport_id();
        let ctx = ctx.clone();
        let sql = if save {
            Some(self.tabs[self.tab].sql.clone())
        } else {
            None
        };
        let directory = self.query_directory();
        let name = self.tabs[self.tab]
            .source_name
            .clone()
            .unwrap_or_else(|| "query.sql".into());
        std::thread::spawn(move || {
            if save && let Err(error) = std::fs::create_dir_all(&directory) {
                let _ = tx.send(FileAction::QueryLibraryError(error.to_string()));
                ctx.request_repaint_of(viewport);
                return;
            }
            let dialog = rfd::FileDialog::new().set_directory(directory).add_filter("SQL script", &["sql"]);
            let action = if let Some(sql) = sql {
                dialog.set_file_name(name).save_file().map(|path| {
                    let expected = source_path
                        .zip(saved_sql)
                        .map(|(path, sql)| crate::script_file::SaveSource { path, sql });
                    FileAction::SaveScript(path, sql, tab_id, expected)
                })
            } else {
                dialog.pick_file().map(FileAction::OpenScript)
            };
            if let Some(action) = action {
                let _ = tx.send(action);
                ctx.request_repaint_of(viewport);
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn json_export_dialog(&self, ctx: &egui::Context) {
        let tx = self.files_tx.clone();
        let viewport = ctx.viewport_id();
        let ctx = ctx.clone();
        let grid = self.grid();
        std::thread::spawn(move || {
            if let Some(path) = rfd::FileDialog::new().add_filter("JSON", &["json"])
                .set_file_name("results.json").save_file() {
                let _ = tx.send(FileAction::Export(path, grid, db::ExportFormat::Json));
                ctx.request_repaint_of(viewport);
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn file_dialog(&self, ctx: &egui::Context, export: bool) {
        let tx = self.files_tx.clone();
        let viewport = ctx.viewport_id();
        let ctx = ctx.clone();
        let grid = self.grid();
        std::thread::spawn(move || {
            let action = if export {
                rfd::FileDialog::new()
                    .add_filter("CSV", &["csv"])
                    .set_file_name("results.csv")
                    .save_file()
                    .map(|p| FileAction::Export(p, grid, db::ExportFormat::Csv))
            } else {
                rfd::FileDialog::new()
                    .add_filter("SQLite database", &["db", "sqlite", "sqlite3"])
                    .add_filter("All files", &["*"])
                    .pick_file()
                    .map(FileAction::Open)
            };
            if let Some(action) = action {
                let _ = tx.send(action);
                ctx.request_repaint_of(viewport);
            }
        });
    }

    fn log_activity(&mut self, message: String) {
        let seconds = web_time::SystemTime::now()
            .duration_since(web_time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            % 86400;
        self.activity.push_back(format!(
            "-- {:02}:{:02}:{:02} UTC\n{}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60,
            bounded_log_text(&message)
        ));
        while self.activity.len() > 200 {
            self.activity.pop_front();
        }
    }

    fn activity_panel(&mut self, ui: &mut egui::Ui) {
        if self.prefs.activity_visible {
            let max_height = (ui.available_height() - 180.0).max(80.0);
            let id = egui::Id::new((ui.ctx().viewport_id(), "activity-log-pane"));
            crate::pane_sizes::restore(ui.ctx(), id, self.prefs.pane_sizes.activity, 180.0, 1);
            egui::TopBottomPanel::bottom(id)
                .resizable(true)
                .default_height(180.0)
                .height_range(80.0..=max_height)
                .show_inside(ui, |ui| self.activity_log(ui));
            self.shared_preferences_dirty |= crate::pane_sizes::record_drag(
                ui.ctx(),
                id,
                &mut self.prefs.pane_sizes.activity,
                1,
            );
        }
    }

    fn activity_log(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("SQL activity");
            if ui.small_button("Clear").clicked() {
                self.activity.clear();
            }
            if ui
                .small_button("×")
                .on_hover_text("Hide activity log")
                .clicked()
            {
                self.prefs.activity_visible = false;
            }
        });
        egui::ScrollArea::vertical()
            .id_salt("activity-scroll")
            // Keep the user's panel height even when the log has little content.
            // Shrinking this viewport makes TopBottomPanel persist the content
            // height instead of the dragged height on the very next frame.
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if self.activity.is_empty() {
                    ui.weak("Executed SQL will appear here.");
                }
                for entry in &self.activity {
                    ui.add(
                        egui::Label::new(crate::editor::layout(ui, entry, ui.available_width()))
                            .wrap()
                            .selectable(true),
                    );
                }
            });
    }

    fn connection_status(&mut self, ui: &mut egui::Ui) {
        let style = self.active_connection_style();
        let fill = Color32::from_rgb(style.color[0], style.color[1], style.color[2]);
        let ink = if fill.intensity() > 0.6 {
            Color32::BLACK
        } else {
            Color32::WHITE
        };
        let width = (ui.available_width() - 230.0).max(100.0);
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, 27.0), egui::Sense::click());
        let painter = ui.painter().with_clip_rect(rect);
        let background = if self.pending.is_some() {
            Color32::from_rgb(
                (f32::from(fill.r()) * 0.65) as u8,
                (f32::from(fill.g()) * 0.65) as u8,
                (f32::from(fill.b()) * 0.65) as u8,
            )
        } else {
            fill
        };
        painter.rect_filled(rect, 13.0, background);
        if self.pending.is_some() {
            // Query totals and network throughput are unknown here. Animate a
            // repeating activity pattern, never an invented completion fraction.
            paint_connection_activity(&painter, rect, self.started.elapsed().as_secs_f32(), ink);
        }
        let progress = self.worker.progress();
        let elapsed = if progress.active {
            progress.elapsed
        } else {
            self.started.elapsed()
        };
        let timing = if self.pending.is_some() {
            format!("{:.1}s", elapsed.as_secs_f64())
        } else {
            self.last_operation_ms
                .map(|ms| format!("{ms:.1} ms"))
                .unwrap_or_else(|| "Ready".into())
        };
        let name = self.active_connection_name();
        let location = if self.view == View::Query {
            format!("Query {}", self.tab + 1)
        } else {
            self.current_table_name().to_owned()
        };
        let database = self
            .connected_database
            .as_ref()
            .map(|database| format!("DB {database}  ·  "))
            .unwrap_or_default();
        let label = format!(
            "{}{}  |  {}  ·  {}{}  ·  {}",
            style.environment.to_uppercase(),
            self.worker.transaction_state().label(),
            self.engine,
            database,
            name,
            location
        );
        let text_rect = egui::Rect::from_min_max(
            rect.min + egui::vec2(10.0, 0.0),
            egui::pos2(
                (rect.right() - 100.0).max(rect.left() + 10.0),
                rect.bottom(),
            ),
        );
        painter.with_clip_rect(text_rect).text(
            text_rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::monospace(11.0),
            ink,
        );
        painter.text(
            rect.right_center() - egui::vec2(10.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            timing,
            egui::FontId::monospace(11.0),
            ink,
        );
        if response
            .on_hover_text(format!(
                "{}\n{}{}\nClick to edit connection appearance",
                self.label,
                self.status,
                if self.pending.is_some() && progress.active && progress.rows > 0 {
                    format!(
                        "\n{} rows decoded · {:.1} KiB decoded (not network bytes)",
                        progress.rows,
                        progress.decoded_bytes as f64 / 1024.0
                    )
                } else {
                    String::new()
                }
            ))
            .clicked()
        {
            self.connections = true;
        }
        if self.pending.is_some()
            && ui
                .small_button("×")
                .on_hover_text("Cancel database operation")
                .clicked()
        {
            self.worker.cancel();
        }
    }

    fn toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top(egui::Id::new((ctx.viewport_id(), "toolbar")))
            .exact_height(38.0)
            .show(ctx, |ui| {
                if self.dialog_owns_workspace_shortcuts() { ui.disable(); }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    if toolbar_action(ui, 0, self.connections)
                        .on_hover_text("Connections")
                        .clicked()
                    {
                        self.connections = true;
                    }
                    if toolbar_action(ui, 1, self.navigator.open)
                        .on_hover_text("Browse databases · Cmd/Ctrl+K · Find anything: Cmd/Ctrl+P")
                        .clicked() { self.open_navigator(true); }
                    if toolbar_action(ui, 2, self.view == View::Query)
                        .on_hover_text("Open query editor")
                        .clicked()
                    {
                        self.show_query();
                        self.tabs[self.tab].focus_editor = true;
                    }
                    self.connection_status(ui);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(6.0);
                        pane_toggles::show(
                            ui,
                            &mut self.prefs.activity_visible,
                            &mut self.inspector,
                        );
                        let before = self.prefs.appearance;
                        ui.menu_button("⚙", |ui| {
                            ui.label("Appearance");
                            for choice in [Appearance::System, Appearance::Light, Appearance::Dark]
                            {
                                if ui
                                    .selectable_value(
                                        &mut self.prefs.appearance,
                                        choice,
                                        choice.label(),
                                    )
                                    .clicked()
                                {
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_hover_text("Settings");
                        if before != self.prefs.appearance {
                            self.shared_preferences_dirty = true;
                            ctx.set_theme(self.prefs.appearance.preference());
                        }
                    });
                });
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        let id = egui::Id::new((ctx.viewport_id(), "sidebar"));
        crate::pane_sizes::restore(ctx, id, self.prefs.pane_sizes.sidebar, 220.0, 0);
        egui::SidePanel::left(id)
            .resizable(true)
            .default_width(220.0)
            .width_range(160.0..=360.0)
            .show(ctx, |ui| {
                if self.dialog_owns_workspace_shortcuts() { ui.disable(); }
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                    ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
                    ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;
                    ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::NONE;
                    for (index, label) in ["Items", "Queries", "History"].iter().enumerate() {
                        let selected = self.sidebar_section == index;
                        let mut button = egui::Button::new(*label).corner_radius(5.0);
                        if selected {
                            button = button.fill(ui.visuals().widgets.active.bg_fill);
                        }
                        if ui
                            .add_sized([ui.available_width().min(64.0), 23.0], button)
                            .clicked()
                        {
                            self.sidebar_section = index;
                        }
                    }
                });
                // Object rows remain quiet on hover; only the section tabs react.
                let resting = ui.visuals().widgets.inactive;
                ui.visuals_mut().widgets.hovered = resting;
                ui.visuals_mut().widgets.active = resting;
                ui.add_space(4.0);
                if self.sidebar_section == 2 {
                    self.history_sidebar(ui);
                    return;
                }
                if self.sidebar_section == 1 {
                    self.query_library_sidebar(ui);
                    return;
                }
                if !self.schemas.names.is_empty() {
                    let mut selected = self.schemas.selected.clone();
                    ui.add_enabled_ui(self.pending.is_none(), |ui| {
                        egui::ComboBox::from_id_salt("object-schema")
                            .width(ui.available_width())
                            .selected_text(selected.as_deref().unwrap_or("Select schema"))
                            .show_ui(ui, |ui| {
                                for schema in &self.schemas.names {
                                    ui.selectable_value(
                                        &mut selected,
                                        Some(schema.clone()),
                                        schema,
                                    );
                                }
                            })
                            .response
                            .on_hover_text("Browse objects in this schema");
                    });
                    if selected != self.schemas.selected
                        && let Some(schema) = selected
                    {
                        self.dispatch(Command::SelectSchema(schema), Pending::Export);
                    }
                    ui.add_space(4.0);
                }
                ui.add(
                    TextEdit::singleline(&mut self.search)
                        .hint_text("Search for item…")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(4.0);
                let filter = self.search.to_lowercase();
                let changed_tables: std::collections::HashSet<&str> =
                    self.changes.iter().map(Change::table).collect();
                let mut open = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (kind, title) in [("table", "Tables"), ("view", "Views")] {
                        ui.label(
                            RichText::new(format!(
                                "{} ({})",
                                title,
                                self.objects.iter().filter(|o| o.kind == kind).count()
                            ))
                            .size(11.0)
                            .color(MUTED),
                        );
                        for object in self.objects.iter().filter(|o| {
                            o.kind == kind && o.display_name().to_lowercase().contains(&filter)
                        }) {
                            let selected =
                                self.browse.table == object.name && self.view != View::Query;
                            let changed = changed_tables.contains(object.name.as_str());
                            ui.add_enabled_ui(self.pending.is_none(), |ui| {
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), 22.0),
                                    egui::Sense::click(),
                                );
                                // Item backgrounds reflect committed selection or pending edits,
                                // never hover state. Primary press below commits selection.
                                if selected || changed {
                                    ui.painter().rect_filled(
                                        rect,
                                        4.0,
                                        if changed {
                                            pending_fill(ui)
                                        } else {
                                            ui.visuals().selection.bg_fill
                                        },
                                    );
                                }
                                navigator::paint_object_icon(ui.painter(), rect.left_center() + egui::vec2(14.0, 0.0), &object.kind, 13.0);
                                let painter = ui.painter().with_clip_rect(rect);
                                painter.text(
                                    rect.left_center() + egui::vec2(27.0, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    object.display_name(),
                                    egui::FontId::monospace(12.0),
                                    if selected && !changed {
                                        ui.visuals().selection.stroke.color
                                    } else {
                                        ui.visuals().text_color()
                                    },
                                );
                                let response = response.on_hover_text(match &object.schema {
                                    Some(schema) => format!("Schema: {schema}\n{}", object.name),
                                    None => object.name.clone(),
                                });
                                let response = if let Some(error) = &object.metadata_error {
                                    ui.painter().text(
                                        rect.right_center() - egui::vec2(8.0, 0.0),
                                        egui::Align2::RIGHT_CENTER,
                                        "!",
                                        egui::FontId::proportional(13.0),
                                        Color32::from_rgb(210, 150, 50),
                                    );
                                    response.on_hover_text(format!(
                                        "Column information unavailable: {error}"
                                    ))
                                } else {
                                    response
                                };
                                let response = if changed {
                                    response.on_hover_text("Pending changes · ⌘S to review")
                                } else {
                                    response
                                };
                                if primary_pressed(&response) {
                                    open = Some(object.name.clone());
                                }
                            });
                        }
                        ui.add_space(6.0);
                    }
                });
                if let Some(name) = open {
                    self.open_table(name);
                }
            });
        self.shared_preferences_dirty |=
            crate::pane_sizes::record_drag(ctx, id, &mut self.prefs.pane_sizes.sidebar, 0);
    }

    fn footer(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom(egui::Id::new((ctx.viewport_id(), "status"))).exact_height(24.0).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(10.0);
                if self.pending.is_some() { ui.add(egui::Spinner::new().size(12.0)); } else { ui.label(RichText::new("•").color(GREEN)); }
                ui.label(RichText::new(&self.status).size(11.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(10.0);
                    ui.label(RichText::new(format!("UI {:.2} ms  ·  {}", self.frame_ms, self.engine)).size(10.0).color(MUTED))
                        .on_hover_text("UI measures CPU time spent building the previous frame, not display FPS or GPU time.");
                });
            });
        });
    }

    fn inspector(&mut self, ctx: &egui::Context) {
        if !self.inspector {
            return;
        }
        let mut finish = false;
        let mut start_edit = None;
        let mut stage_constant = false;
        let id = egui::Id::new((ctx.viewport_id(), "inspector"));
        crate::pane_sizes::restore(ctx, id, self.prefs.pane_sizes.inspector, 240.0, 0);
        egui::SidePanel::right(id)
            .default_width(240.0)
            .width_range(190.0..=420.0)
            .show(ctx, |ui| {
                if self.dialog_owns_workspace_shortcuts() { ui.disable(); }
                ui.set_min_width(ui.available_width());
                ui.add_space(4.0);
                if self.view == View::Indexes {
                    ui.strong("Index inspector");
                    if let Some(index) = self.selected_index.and_then(|i|self.indexes.get(i)) {
                        ui.label(&index.name);
                        ui.label(if index.unique { "Unique index" } else { "Non-unique index" });
                        ui.label(format!("Columns: {}",index.columns.join(", ")));
                        if index.ddl.is_empty() { ui.label("Defined by a table constraint. Edit the table constraint to change this index."); }
                        else { if clipboard_button(ui).clicked() { ui.ctx().copy_text(index.ddl.clone()); } ui.code(&index.ddl); }
                    } else { ui.label("Select an index to inspect its definition."); }
                    return;
                }
                if self.view == View::Triggers { ui.strong("Triggers"); ui.label("Trigger definitions are shown in the workspace."); return; }
                if self.view == View::Structure {
                    self.column_inspector(ui);
                    return;
                }
                inspector_search(ui, &mut self.field_search);
                let search = self.field_search.to_lowercase();
                ui.add_space(3.0);
                if self.row_selection.rows.len() > 1 {
                    self.bulk_inspector(ui, &search);
                    return;
                }
                if let Some(id) = self.selected_insert() {
                    self.insert_inspector(ui, id);
                    return;
                }
                let grid = self.grid();
                if let Some((index, row)) =
                    self.selected.and_then(|i| grid.rows.get(i).map(|r| (i, r)))
                {

                    if self.view == View::Data && row_is_deleted(&self.changes, &self.browse.table, row) { ui.colored_label(Color32::from_rgb(220, 90, 90), "Pending deletion · restore from the table toolbar"); }
                    ui.add_space(2.0);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        compact_inspector_spacing(ui);
                        for (name, original_value) in grid.columns.iter().zip(row) {
                            if !name.to_lowercase().contains(&search) {
                                continue;
                            }
                            let staged = if self.view == View::Data {
                                self.changes.iter().find_map(|change| match change {
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
                            } else {
                                None
                            };
                            let value = staged.unwrap_or(original_value);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(name).size(11.0).color(ui.visuals().weak_text_color()));
                                if self.view == View::Data
                                    && let Some(column) =
                                        self.columns.iter().find(|c| c.name == *name)
                                {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.add(egui::Label::new(
                                            RichText::new(column.data_type.to_lowercase()).size(10.0).color(ui.visuals().weak_text_color()),
                                        ).truncate()).on_hover_text(&column.data_type);
                                    });
                                }
                            });
                            let writable = self.view == View::Data && self.supports_data_changes() && !self.safe && self.pending.is_none()
                                && !row_is_deleted(&self.changes, &self.browse.table, row)
                                && self.columns.iter().any(|c| c.pk)
                                && self.objects.iter().any(|o| o.name == self.browse.table && o.kind == "table")
                                && self.columns.iter().any(|c| c.name == *name && !c.generated)
                                && !matches!(value, Value::Blob(_))
                                && !matches!(value, Value::Text(t) if t.len() > 65536);
                            let text = db::preview(value, 4096);
                            egui::Frame::new()
                                .fill(if staged.is_some() { pending_fill(ui) } else { ui.visuals().extreme_bg_color })
                                .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                                .corner_radius(4.0)
                                .inner_margin(1.0)
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 22.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.add_enabled_ui(writable, |ui| {
                                            ui.push_id(("field-actions", name), |ui| {
                                                field_value_menu(ui, |ui| {
                                                    let nullable = self.columns.iter().any(|c| c.name == *name && !c.not_null);
                                                    let mut kind = None;
                                                    if ui.add_enabled(nullable, egui::Button::new("Set NULL")).clicked() { kind = Some(0); }
                                                    if ui.button("Set empty text").clicked() { kind = Some(3); }
                                                    if let Some(kind) = kind {
                                                        start_edit = Some(CellEdit { bulk_rows: Vec::new(), bulk_changed: false, insert_id: None, advance: 0, table: self.browse.table.clone(), row: row.clone(), original_value: original_value.clone(), column: name.clone(), text: String::new(), kind, focus: false, inspector: true });
                                                        stage_constant = true;
                                                        ui.close();
                                                    }
                                                }).on_hover_text("Set field value");
                                            });
                                        });
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {

                                        if ui.push_id(("copy-field", &self.browse.table, self.view == View::Query, self.tab, index, name), clipboard_button).inner.clicked() {
                                            ui.ctx().copy_text(db::display(value));
                                        }
                                        if self.edit.as_ref().is_some_and(|e| e.inspector && e.table == self.browse.table && e.column == *name && e.row == *row) {
                                            finish |= inline_value(ui, &mut self.edit);
                                        } else {
                                            let response = inspector_value_preview(ui, text.as_ref()).on_hover_text(text.as_ref());
                                            if response.clicked() && writable {
                                                start_edit = Some(CellEdit { bulk_rows: Vec::new(), bulk_changed: false, insert_id: None, advance: 0, table: self.browse.table.clone(), row: row.clone(), original_value: original_value.clone(), column: name.clone(), text: if matches!(value,Value::Null) { String::new() } else { db::display(value) }, kind: match value { Value::Null => 0, Value::Integer(_) => 1, Value::Real(_) => 2, _ => 3 }, focus: true, inspector: true });
                                            }
                                        }
                                });
                                    });
                                });
                            ui.add_space(1.0);
                        }
                    });
                } else {
                    let empty = ui.available_rect_before_wrap();
                    ui.painter().text(
                        egui::pos2(empty.center().x, empty.top() + empty.height() * 0.72),
                        egui::Align2::CENTER_CENTER,
                        "No row selected",
                        egui::FontId::proportional(15.0),
                        ui.visuals().weak_text_color(),
                    );
                }
            });
        self.shared_preferences_dirty |=
            crate::pane_sizes::record_drag(ctx, id, &mut self.prefs.pane_sizes.inspector, 0);
        if finish {
            self.finish_cell_edit();
        }
        if start_edit.is_some() && self.finish_cell_edit() {
            self.edit = start_edit;
            if stage_constant {
                self.finish_cell_edit();
            }
        }
    }

    fn workspace(&mut self, ctx: &egui::Context) {
        let schema_editable = self.supports_schema_changes();
        let dialog_open = self.dialog_owns_workspace_shortcuts();
        if self.view != View::Data || dialog_open { self.focus_page_grid = false; }
        egui::CentralPanel::default().show(ctx, |ui| {
            if dialog_open { ui.disable(); }
            self.activity_panel(ui);
            let mut close = None;
            let active = (self.view == View::Query, self.tabs[self.tab].id);
            let visible_tab_id = egui::Id::new((ctx.viewport_id(), "visible-workspace-tab"));
            let reveal_active = ui.ctx().data_mut(|data| {
                let id = visible_tab_id;
                let previous = data.get_temp::<(bool, u64)>(id);
                data.insert_temp(id, active);
                previous != Some(active)
            });
            egui::ScrollArea::horizontal()
                .id_salt("workspace-tabs")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if !self.browse.table.is_empty() {
                            let (response, closed) = workspace_tab(
                                ui,
                                self.current_table_name(),
                                self.view != View::Query,
                                true,
                            );
                            let response = response.on_hover_text(&self.browse.table);
                            if reveal_active && self.view != View::Query {
                                ui.scroll_to_rect(response.rect, None);
                            }
                            if closed || response.clicked_by(egui::PointerButton::Middle) {
                                self.close_table_tab();
                            } else if response.clicked() {
                                self.view = View::Data;
                                self.row_selection.clear();
                                self.selected = None;
                            }
                        }
                        for index in 0..self.tabs.len() {
                            if !self.tabs[index].open {
                                continue;
                            }
                            let (response, closed) = workspace_tab(
                                ui,
                                &format!("{}{}", if self.tabs[index].dirty { "• " } else { "" }, self.tabs[index].source_name.clone().unwrap_or_else(|| format!("SQL Query {}", index + 1))),
                                self.view == View::Query && self.tab == index,
                                self.pending.is_none(),
                            );
                            if reveal_active && self.view == View::Query && self.tab == index {
                                ui.scroll_to_rect(response.rect, None);
                            }
                            if closed || response.clicked_by(egui::PointerButton::Middle) {
                                close = Some(index);
                            } else if response.clicked() {
                                self.activate_query(index);
                            }
                        }
                        if ui.button("+").on_hover_text("New query tab").clicked() {
                            self.new_tab();
                        }
                    });
                });
            if let Some(index) = close {
                self.close_tab(index);
            }
            ui.separator();
            ui.add_space(8.0);
            if let Some(error) = self.error.clone() {
                egui::Frame::new()
                    .fill(Color32::from_rgb(100, 38, 40))
                    .inner_margin(10.0)
                    .corner_radius(5.0)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(error).color(Color32::WHITE));
                            #[cfg(target_arch = "wasm32")]
                            if self.worker.transaction_state() == db::TransactionState::Disconnected
                                && ui.add_enabled(self.changes.is_empty() && self.edit.is_none(), egui::Button::new("Start fresh demo"))
                                    .on_hover_text("Starts a new disposable database. Discard any staged edits first. Saved queries remain in this browser.")
                                    .clicked() {
                                self.dispatch(Command::Open(None), Pending::Connect(None));
                            }
                            if ui.small_button("Dismiss").clicked() {
                                self.error = None;
                            }
                        });
                    });
                ui.add_space(8.0);
            }
            if self.view == View::Query {
                let has_results = !self.tabs[self.tab].results.is_empty()
                    || self.tabs[self.tab].result_error.is_some()
                    || matches!(self.pending, Some(Pending::Query(index)) if index == self.tab);
                if !has_results {
                    self.query_editor(ui);
                    return;
                }
                let max_height = (ui.available_height() - 140.0).max(120.0);
                egui::TopBottomPanel::top(egui::Id::new((ctx.viewport_id(), "query-editor-pane")))
                    .resizable(true)
                    .default_height(260.0)
                    .height_range(120.0..=max_height)
                    .show_inside(ui, |ui| {
                        ui.set_min_height(ui.available_height());
                        self.query_editor(ui);
                    });
            }
            if self.view != View::Query {
                let grid = self.grid();
                let selected_count = self.row_selection.rows.len().max(usize::from(self.selected.is_some()));
                let page_range = table_status_label(self.browse.offset, &grid, selected_count);
                let control_geometry = table_control_geometry(ui, self.view, self.changes.len(), &page_range);
                egui::TopBottomPanel::bottom(egui::Id::new((ctx.viewport_id(), "table-view-controls")))
                    .exact_height(control_geometry.height)
                    .show_inside(ui, |ui| {
                        let origin = ui.available_rect_before_wrap().min.to_vec2() + egui::vec2(0.0, 7.0);
                        rounded_action_buttons(ui);
                        ui.spacing_mut().interact_size.y = 28.0;
                        ui.spacing_mut().button_padding = egui::vec2(6.0, 5.0);
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.scope_builder(egui::UiBuilder::new().max_rect(control_geometry.left.translate(origin)), |ui| {
                          ui.horizontal_wrapped(|ui| {
                            table_view_switch(ui, &mut self.view);
                            if self.view == View::Data {
                                if ui
                                    .add_enabled(self.can_insert(), egui::Button::new("+ Row"))
                                    .on_hover_text("Add row · Cmd+I")
                                    .clicked()
                                {
                                    self.add_insert();
                                }
                                ui.menu_button("Rows", |ui| {
                                    if ui
                                        .add_enabled(
                                            self.can_duplicate(),
                                            egui::Button::new("Duplicate"),
                                        )
                                        .on_hover_text(
                                            "Duplicate row · Cmd+D · primary keys use DEFAULT",
                                        )
                                        .clicked()
                                    {
                                        self.duplicate_selected();
                                    }
                                    if self.selected_insert().is_some()
                                        && ui.button("Remove new row").clicked()
                                    {
                                        self.remove_selected_insert();
                                    }
                                    if self.row_selection.rows.len() > 1 {
                                        let restore = self.selected_rows_deleted();
                                        let label = format!(
                                            "{} {} rows",
                                            if restore { "Restore" } else { "Delete" },
                                            self.row_selection.rows.len()
                                        );
                                        if ui
                                        .add_enabled(
                                            self.can_delete_rows(),
                                            egui::Button::new(label),
                                        )
                                        .on_hover_text(
                                            "Stage selected rows · Cmd+S reviews SQL before saving",
                                        )
                                        .clicked()
                                    {
                                        self.set_selected_rows_deleted(!restore);
                                    }
                                    } else {
                                        let row = self
                                            .selected
                                            .and_then(|index| self.table.rows.get(index))
                                            .cloned();
                                        let deleting = row.as_ref().is_some_and(|row| {
                                            row_is_deleted(&self.changes, &self.browse.table, row)
                                        });
                                        let enabled = row.is_some()
                                            && self.supports_data_changes() && !self.safe
                                            && self.pending.is_none()
                                            && self.columns.iter().any(|c| c.pk)
                                            && self.objects.iter().any(|o| {
                                                o.name == self.browse.table && o.kind == "table"
                                            });
                                        if ui
                                .add_enabled(
                                    enabled,
                                    egui::Button::new(if deleting {
                                        "Restore row"
                                    } else {
                                        "Delete row"
                                    }),
                                )
                                .on_hover_text(
                                    "Stage or revert deletion · Cmd+S reviews SQL before saving",
                                )
                                .clicked()
                                && self.finish_cell_edit()
                                && let Some(row) = row
                            {
                                toggle_row_deletion(&mut self.changes, &self.browse.table, row);
                                self.record_changes();
                                self.review = None;
                                if !deleting {
                                    self.row_selection.clear();
                                    self.selected = self.selected.and_then(|i| {
                                        (i + 1 < self.table.rows.len()).then_some(i + 1)
                                    });
                                }
                            }
                                    }
                                });
                            }
                            if self.view == View::Structure
                                && ui
                                    .add_enabled(
                                        schema_editable && !self.safe && self.pending.is_none(),
                                        egui::Button::new("+ Column"),
                                    )
                                    .clicked()
                            {
                                self.add_column = Some(ColumnDraft {
                                    table: self.browse.table.clone(),
                                    name: String::new(),
                                    data_type: "TEXT".into(),
                                    not_null: false,
                                    default_kind: 0,
                                    default_text: String::new(),
                                    error: None,
                                });
                            }
                            if ui
                                .add_enabled(self.pending.is_none(), egui::Button::new("↻"))
                                .on_hover_text("Refresh")
                                .clicked()
                            {
                                self.refresh_current_view();
                            }
                            if !self.changes.is_empty() {
                                ui.separator();
                                let mut remove = None;
                                ui.menu_button(format!("Pending ({})", self.changes.len()), |ui| {
                                if ui
                                    .button(format!("Review {} changes", self.changes.len()))
                                    .on_hover_text("Cmd+Shift+P · Preview SQL without executing")
                                    .clicked()
                                {
                                    self.preview_changes();
                                }
                                if ui
                                    .add_enabled(
                                        self.pending.is_none(),
                                        egui::Button::new("Discard"),
                                    )
                                    .on_hover_text("Cmd+Shift+Backspace · Discard pending changes")
                                    .clicked()
                                {
                                    self.discard_changes();
                                }
                                    ui.separator();
                                    egui::ScrollArea::vertical()
                                        .max_height(300.0)
                                        .show(ui, |ui| {
                                            for (index, change) in self.changes.iter().enumerate() {
                                                ui.horizontal(|ui| {
                                                    if ui
                                                        .add_enabled(
                                                            self.pending.is_none(),
                                                            egui::Button::new("×"),
                                                        )
                                                        .on_hover_text("Remove this staged change")
                                                        .clicked()
                                                    {
                                                        remove = Some(index);
                                                    }
                                                    let description = match change {
                                                        Change::InsertRow { table, .. } => {
                                                            format!("Insert row into {table}")
                                                        }
                                                        Change::DeleteRow { table, .. } => {
                                                            format!("Delete row from {table}")
                                                        }
                                                        Change::Cell {
                                                            table,
                                                            column,
                                                            value,
                                                            ..
                                                        } => format!(
                                                            "{table}.{column} = {}",
                                                            db::preview(value, 80)
                                                        ),
                                                        Change::RenameColumn {
                                                            table,
                                                            from,
                                                            to,
                                                        } => {
                                                            format!("Rename {table}.{from} → {to}")
                                                        }
                                                        Change::AlterColumn {
                                                            table,
                                                            column,
                                                            ..
                                                        } => format!("Alter {table}.{column}"),
                                                        Change::CreateIndex {
                                                            table, name, ..
                                                        } => {
                                                            format!(
                                                                "Create index {name} on {table}"
                                                            )
                                                        }
                                                        Change::DropIndex { table, name } => {
                                                            format!("Drop index {name} on {table}")
                                                        }
                                                        Change::AddColumn { table, definition } => {
                                                            format!(
                                                                "Add {table}.{} · {}",
                                                                definition.name,
                                                                definition.data_type
                                                            )
                                                        }
                                                    };
                                                    ui.label(description);
                                                });
                                            }
                                        });
                                });
                                if let Some(index) = remove {
                                    self.changes.remove(index);
                                    self.record_changes();
                                    self.review = None;
                                }

                            }
                          });
                        });
                        if self.view == View::Data {
                            let center_rect = control_geometry.center.translate(origin);
                            ui.painter().with_clip_rect(center_rect).text(center_rect.center(), egui::Align2::CENTER_CENTER, &page_range, egui::FontId::proportional(12.0), ui.visuals().weak_text_color());
                        }
                        ui.scope_builder(egui::UiBuilder::new().max_rect(control_geometry.right.translate(origin)).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                                    let grid = self.grid();
                                    if self.view == View::Data {
                                        if ui
                                            .add_enabled(
                                                self.pending.is_none()
                                                    && grid.truncated
                                                    && grid.rows.len() == db::PAGE_SIZE,
                                                navigation_button(true),
                                            )
                                            .clicked()
                                        {
                                            self.change_table_page(true);
                                        }
                                        if ui
                                            .add_enabled(
                                                self.pending.is_none() && self.browse.offset > 0,
                                                navigation_button(false),
                                            )
                                            .clicked()
                                        {
                                            self.change_table_page(false);
                                        }

                                    }
                                    ui.menu_button("Export", |ui| {
                                        if ui.add_enabled(self.pending.is_none(), egui::Button::new("Preview CSV file…")).clicked() {
                                            self.preview_csv_dialog(ctx);
                                            ui.close();
                                        }
                                        ui.separator();
                                        if ui.button("Loaded rows as CSV…").clicked() {
                                            self.file_dialog(ctx, true);
                                            ui.close();
                                        }
                                        if ui.add_enabled(self.pending.is_none(), egui::Button::new("Loaded rows as JSON…"))
                                            .on_hover_text("JSON objects; binary values are byte arrays. Exports loaded rows only.").clicked() {
                                            self.json_export_dialog(ctx);
                                            ui.close();
                                        }
                                        if ui.add_enabled(self.supports_table_export() && self.view == View::Data && self.pending.is_none(), egui::Button::new("All matching rows as CSV…"))
                                            .on_hover_text("Export saved database values using the current filter and sort, across all pages")
                                            .clicked() {
                                            self.export_table_dialog(ctx, db::ExportFormat::Csv);
                                            ui.close();
                                        }
                                        if ui.add_enabled(self.supports_table_export() && self.view == View::Data && self.pending.is_none(), egui::Button::new("All matching rows as JSON…"))
                                            .on_hover_text("Stream saved database values using the current filter and sort, across all pages. Binary values are byte arrays.").clicked() {
                                            self.export_table_dialog(ctx, db::ExportFormat::Json);
                                            ui.close();
                                        }
                                    });
                                    if self.view == View::Data
                                        && ui
                                            .add(filter_button(
                                                self.filters_visible
                                                    || self.browse.filter_column.is_some(),
                                            ))
                                            .clicked()
                                    {
                                        self.filters_visible = !self.filters_visible;
                                        self.focus_filter = self.filters_visible;
                                    }
                        });
                    });
            }
            ui.set_clip_rect(ui.clip_rect().intersect(ui.available_rect_before_wrap()));
            if self.view == View::Indexes {
                self.index_view(ui);
                return;
            }
            if self.view == View::Triggers {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if self.triggers.is_empty() {
                        ui.label("This table has no triggers.");
                    }
                    for trigger in &self.triggers {
                        ui.horizontal(|ui| {
                            ui.strong(&trigger.name);
                            if clipboard_button(ui).clicked() {
                                ui.ctx().copy_text(trigger.ddl.clone());
                            }
                        });
                        ui.code(&trigger.ddl);
                        ui.separator();
                    }
                });
                return;
            }
            if self.view == View::Structure {
                self.structure(ui);
                return;
            }
            if self.view == View::Data && self.filters_visible {
                self.filters(ui);
            }
            if self.view == View::Query {
                self.query_result_footer(ui);
                if self.tabs[self.tab].result_view == ResultView::Message {
                    self.query_result_message(ui);
                    return;
                }
                if self.tabs[self.tab].result_view == ResultView::Chart {
                    let grid = self.grid();
                    self.tabs[self.tab].chart.show(ui, &grid);
                    return;
                }
            }
            if self.view == View::Query {
                let grid = self.grid();
                let was_open = self.tabs[self.tab].result_search.open;
                if ui.is_enabled() && let Some((row, column)) = self.tabs[self.tab].result_search.show(ui, &grid) {
                    self.selected = self.row_selection.click(row, false, false, None);
                    self.row_selection.cell_column = column;
                    self.row_selection.pending_focus = Some(row);
                    self.row_selection.pending_cell_scroll = true;
                }
                if was_open && !self.tabs[self.tab].result_search.open
                    && let Some(id) = self.tabs[self.tab].result_grid_focus {
                    ui.memory_mut(|m| m.request_focus(id));
                }
            }
            self.data_grid(ui);
        });
    }

    fn query_result_footer(&mut self, ui: &mut egui::Ui) {
        let grid = self.grid();
        let failed_without_result = self.tabs[self.tab].selected_pin.is_none() && self.tabs[self.tab].results.is_empty() && self.tabs[self.tab].result_error.is_some();
        let count = if failed_without_result {
            String::new()
        } else if grid.columns.is_empty() {
            format!("{} {} affected", grid.affected, if grid.affected == 1 { "row" } else { "rows" })
        } else {
            format!("{}{} {}", grid.rows.len(), if grid.truncated { "+" } else { "" }, if grid.rows.len() == 1 && !grid.truncated { "row" } else { "rows" })
        };
        let pending = self.tabs[self.tab].selected_pin.is_none() && matches!(self.pending, Some(Pending::Query(index)) if index == self.tab);
        let elapsed_ms = grid.elapsed.as_secs_f64() * 1000.0;
        let duration = if pending { "Running…".into() } else if failed_without_result { "Failed".into() } else if elapsed_ms < 10.0 { format!("{elapsed_ms:.1} ms") } else { format!("{elapsed_ms:.0} ms") };
        let frame = egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(8, 0));
        let geometry = query_result_control_geometry(ui.available_width() - 16.0);
        egui::TopBottomPanel::bottom(egui::Id::new((ui.ctx().viewport_id(), "query-result-controls")))
            .frame(frame)
            .exact_height(geometry.height)
            .show_inside(ui, |ui| {
                // Position controls within the panel content, excluding its margins.
                let geometry = query_result_control_geometry(ui.available_width());
                let origin = ui.available_rect_before_wrap().min.to_vec2() + egui::vec2(0.0, 7.0);
                rounded_action_buttons(ui);
                ui.spacing_mut().interact_size.y = 28.0;
                ui.spacing_mut().button_padding = egui::vec2(8.0, 5.0);
                ui.scope_builder(egui::UiBuilder::new().max_rect(geometry.left.translate(origin)), |ui| {
                    ui.horizontal(|ui| {
                        query_result_view_switch(ui, &mut self.tabs[self.tab].result_view);
                        ui.label(RichText::new(duration).color(ui.visuals().weak_text_color()));
                    });
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(geometry.center.translate(origin)), |ui| {
                    ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                        ui.label(RichText::new(if pending { String::new() } else { count }).color(ui.visuals().weak_text_color()))
                            .on_hover_text(if grid.truncated { "Result limit reached; more rows may be available" } else { "Rows in the selected result" });
                    });
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(geometry.right.translate(origin)), |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_enabled_ui(self.pending.is_none() && !grid.columns.is_empty(), |ui| {
                            ui.menu_button("Export…", |ui| {
                                if ui.button("Loaded rows as CSV…").clicked() { self.file_dialog(ui.ctx(), true); ui.close(); }
                                if ui.button("Loaded rows as JSON…").on_hover_text("JSON objects; binary values are byte arrays. Exports loaded rows only.").clicked() { self.json_export_dialog(ui.ctx()); ui.close(); }
                            });
                        });
                        if ui.add_enabled(!grid.columns.is_empty(), result_search_button())
                            .on_hover_text("Find in loaded results · ⌘F when results are focused").clicked() {
                            let tab = &mut self.tabs[self.tab];
                            tab.result_view = ResultView::Data;
                            tab.result_search.open = !tab.result_search.open;
                            tab.result_search.focus = tab.result_search.open;
                            if !tab.result_search.open && let Some(id) = tab.result_grid_focus {
                                ui.memory_mut(|m| m.request_focus(id));
                            }
                        }
                        let pinned = self.tabs[self.tab].pins.find(&grid).is_some();
                        if ui.add_enabled(!pending && !grid.columns.is_empty(), result_pin_button(pinned))
                            .on_hover_text(if pinned { "Unpin this result" } else { "Pin this result across query runs" }).clicked() {
                            self.toggle_result_pin();
                        }
                    });
                });
            });
    }

    fn toggle_result_pin(&mut self) {
        let tab = &mut self.tabs[self.tab];
        if let Some(index) = tab.pins.find(&tab.grid) {
            tab.remove_pin(index);
        } else if let Err(error) = tab.pins.add(tab.grid.clone(), tab.result_index) {
            self.error = Some(error.into());
        }
        self.row_selection.clear();
        self.selected = None;
    }

    fn query_result_message(&self, ui: &mut egui::Ui) {
        let tab = &self.tabs[self.tab];
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(8.0);
            if tab.selected_pin.is_none() && matches!(self.pending, Some(Pending::Query(index)) if index == self.tab) {
                ui.label("Running query…");
                return;
            }
            if tab.results.is_empty() && tab.selected_pin.is_none() {
                if let Some(error) = &tab.result_error {
                    ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
                }
                return;
            }
            ui.label(format!("Statement {} completed in {:.1} ms.", tab.selected_pin.map_or(tab.result_index, |index| tab.pins.items[index].statement) + 1, tab.grid.elapsed.as_secs_f64() * 1000.0));
            ui.label(if tab.grid.columns.is_empty() {
                format!("{} {} affected.", tab.grid.affected, if tab.grid.affected == 1 { "row" } else { "rows" })
            } else {
                format!("{} {} returned{}.", tab.grid.rows.len(), if tab.grid.rows.len() == 1 { "row" } else { "rows" }, if tab.grid.truncated { "; result limit reached" } else { "" })
            });
            if let Some(error) = &tab.result_error && tab.selected_pin.is_none() {
                ui.add_space(8.0);
                ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
            }
        });
    }

    fn show_query(&mut self) {
        self.activate_query(self.tab);
    }

    fn workspace_tab_shortcuts(&mut self, ctx: &egui::Context) {
        if self.connections
            || self.review.is_some()
            || self.history
            || self.rename.is_some()
            || self.add_column.is_some()
            || self.alter_column.is_some()
            || self.index_draft.is_some()
            || self.csv_preview.is_some()
            || self.csv_review.is_some()
        {
            return;
        }
        let mut directions = Vec::new();
        ctx.input_mut(|input| {
            input.events.retain(|event| {
                if let egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                    && modifiers.command
                    && modifiers.shift
                    && !modifiers.alt
                    && !(modifiers.ctrl && modifiers.mac_cmd)
                {
                    let direction = match key {
                        egui::Key::OpenBracket => Some(false),
                        egui::Key::CloseBracket => Some(true),
                        _ => match physical_key {
                            Some(egui::Key::OpenBracket) => Some(false),
                            Some(egui::Key::CloseBracket) => Some(true),
                            _ => None,
                        },
                    };
                    if let Some(next) = direction {
                        directions.push(next);
                        return false;
                    }
                }
                true
            })
        });
        for next in directions {
            self.navigate_workspace_tab(next);
        }
    }

    fn navigate_workspace_tab(&mut self, next: bool) {
        // Mirror the rendered order: the table, then all open SQL tabs.
        // The retained closed last editor is deliberately excluded.
        let mut visible = Vec::new();
        if !self.browse.table.is_empty() {
            visible.push(None);
        }
        visible.extend(
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, tab)| tab.open)
                .map(|(index, _)| Some(index)),
        );
        if visible.len() < 2 {
            return;
        }
        let active = (self.view == View::Query).then_some(self.tab);
        let target = match visible.iter().position(|tab| *tab == active) {
            Some(index) if next => (index + 1) % visible.len(),
            Some(index) => (index + visible.len() - 1) % visible.len(),
            None if next => 0,
            None => visible.len() - 1,
        };
        if !self.finish_cell_edit() {
            return;
        }
        match visible[target] {
            Some(index) => self.activate_query(index),
            None => {
                self.view = View::Data;
                self.row_selection.clear();
                self.selected = None;
            }
        }
    }

    fn activate_query(&mut self, index: usize) {
        if index >= self.tabs.len() || !self.finish_cell_edit() {
            return;
        }
        if self.view != View::Query || self.tab != index {
            self.row_selection.clear();
            self.selected = None;
        }
        self.tab = index;
        self.tabs[index].open = true;
        self.tabs[index].focus_editor = true;
        self.view = View::Query;
    }

    fn close_active_tab_or_window(&mut self, ctx: &egui::Context) {
        if self.view == View::Query && self.tabs.get(self.tab).is_some_and(|tab| tab.open) {
            self.close_tab(self.tab);
        } else if !self.browse.table.is_empty() {
            self.close_table_tab();
        } else if self.tabs.iter().any(|tab| tab.open) {
            // A retained editor can be open while the table workspace is empty.
            self.show_query();
            self.close_tab(self.tab);
        } else if self.close_blocked() {
            self.window_error("Finish SQL transactions with COMMIT or ROLLBACK, save or discard pending database changes, and wait for work before closing this window.");
        } else {
            // SessionWindows archives SQL drafts and removes only this viewport.
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn close_table_tab(&mut self) {
        if self.pending.is_some() || !self.finish_cell_edit() {
            return;
        }
        let closes_active = self.view != View::Query;
        self.browse = Browse::default();
        self.table = Arc::default();
        self.selected_column = None;
        if closes_active {
            self.selected = None;
            self.row_selection.clear();
            if self.tabs.iter().any(|tab| tab.open) {
                self.show_query();
            } else {
                self.view = View::Data;
            }
        }
    }

    fn new_tab(&mut self) {
        if !self.finish_cell_edit() {
            return;
        }
        // A closed last tab remains here for Cmd+E. Preserve its draft in the
        // reopen stack before replacing that retained slot with a new editor.
        let mut retained = Vec::with_capacity(self.tabs.len());
        for mut tab in self.tabs.drain(..) {
            if tab.open {
                retained.push(tab);
            } else {
                tab.grid = Arc::default();
                tab.results.clear();
                tab.chart = Default::default();
            tab.formatter = Default::default();
            tab.pins.clear();
            tab.selected_pin = None;
                self.closed_tabs.push(tab);
            }
        }
        self.tabs = retained;
        if self.closed_tabs.len() > 20 {
            self.closed_tabs.drain(..self.closed_tabs.len() - 20);
        }
        self.tabs.push(QueryTab::new(String::new()));
        self.tab = self.tabs.len() - 1;
        self.show_query();
        self.row_selection.clear();
        self.selected = None;
    }

    fn close_tab(&mut self, index: usize) {
        if self.pending.is_some() {
            return;
        }
        if index >= self.tabs.len() || !self.tabs[index].open {
            return;
        }
        let closes_active = self.view == View::Query && index == self.tab;
        if closes_active && !self.finish_cell_edit() {
            return;
        }
        if self.tabs.len() == 1 {
            // Keep the last editor draft available to Cmd+E, as TablePlus does.
            self.tabs[0].pins.clear();
            self.tabs[0].selected_pin = None;
            self.tabs[0].grid = Arc::default();
            self.tabs[0].results.clear();
            self.tabs[0].chart = Default::default();
            self.tabs[0].formatter = Default::default();
            self.tabs[0].open = false;
            self.view = View::Data;
        } else {
            let mut tab = self.tabs.remove(index);
            tab.grid = Arc::default();
            tab.results.clear();
            tab.chart = Default::default();
            tab.formatter = Default::default();
            tab.pins.clear();
            tab.selected_pin = None;
            self.closed_tabs.push(tab);
            if self.closed_tabs.len() > 20 {
                self.closed_tabs.remove(0);
            }
            if index < self.tab {
                self.tab -= 1;
            }
            self.tab = self.tab.min(self.tabs.len() - 1);
        }
        if closes_active {
            self.row_selection.clear();
            self.selected = None;
            if self.view == View::Query {
                self.tabs[self.tab].focus_editor = true;
            }
        }
    }

    fn reopen_tab(&mut self) {
        if self.tabs.len() == 1 && !self.tabs[0].open {
            self.show_query();
            return;
        }
        if let Some(tab) = self
            .closed_tabs
            .pop()
            .or_else(|| self.window_recovery.pop())
        {
            self.tabs.push(tab);
            self.tab = self.tabs.len() - 1;
            self.show_query();
        }
    }

    fn query_editor(&mut self, ui: &mut egui::Ui) {
        let tab = &mut self.tabs[self.tab];
        if let Some(message) = tab.formatter.status.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(message).color(ui.visuals().warn_fg_color));
                if ui.small_button("Dismiss").clicked() { tab.formatter.status = None; }
            });
        }
        if let Some(items) = self.language.update(
            ui.ctx(),
            &self.engine,
            &self.objects,
            tab.id,
            &tab.sql,
            tab.cursor.start,
        ) && tab.cursor.is_empty()
        {
            tab.completion
                .receive_server(&tab.sql, tab.cursor.start, &items);
        }
        let search_action = {
            let tab = &mut self.tabs[self.tab];
            let was_open = tab.search.open;
            let action = ui
                .push_id(("query-find", tab.id), |ui| if ui.is_enabled() { tab.search.show(ui, &tab.sql) } else { None })
                .inner;
            if was_open && !tab.search.open {
                tab.focus_editor = true;
            }
            action
        };
        // Reserve every control below the viewport. Under-reserving causes the
        // resizable panel to grow each frame after result tabs appear.
        let footer_height = if self.tabs[self.tab].results.len() <= 1 && self.tabs[self.tab].pins.items.is_empty() {
            36.0
        } else {
            76.0
        } + if self.language.diagnostics.is_empty() {
            0.0
        } else {
            22.0
        };
        let editor_height = (ui.available_height() - footer_height).max(0.0);
        let line_index = crate::editor::line_index(ui.ctx(), &self.tabs[self.tab].sql);
        let digits = line_index.line_count().to_string().len().max(2);
        let gutter_width = digits as f32 * 8.0 + 16.0;
        let mut active_caret = self.tabs[self.tab].cursor.start;
        let editor_background = if ui.visuals().dark_mode {
            Color32::from_gray(30)
        } else {
            Color32::from_gray(250)
        };
        let editor_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), editor_height),
        );
        ui.painter()
            .rect_filled(editor_rect, 0.0, editor_background);
        ui.horizontal_top(|ui| {
            let (gutter, _) = ui.allocate_exact_size(
                egui::vec2(gutter_width, editor_height),
                egui::Sense::hover(),
            );
            let scrolled = egui::ScrollArea::vertical()
                .id_salt(("sql-editor-scroll", self.tabs[self.tab].id))
                .max_height(editor_height)
                .auto_shrink([false, false])
                .min_scrolled_height(0.0)
                .show(ui, |ui| {
                    let tab = &mut self.tabs[self.tab];
                    egui::Frame::new()
                        .fill(editor_background)
                        .corner_radius(3.0)
                        .show(ui, |ui| {
                            let slot = ui.painter().add(egui::Shape::Noop);
                            let editor_id =
                                ui.make_persistent_id(egui::Id::new(("query-editor", tab.id)));
                            if ui.is_enabled() && ui.memory(|m| m.has_focus(editor_id))
                                && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::I)) {
                                tab.formatter.start(ui.ctx(), &tab.sql, tab.cursor.clone(), crate::sql::Dialect::for_engine(&self.engine));
                            }
                            let was_formatting = tab.formatter.busy();
                            if let Some(formatted) = tab.formatter.poll(&tab.sql) {
                                let mut state = egui::TextEdit::load_state(ui.ctx(), editor_id).unwrap_or_default();
                                apply_formatted_sql(tab, formatted, &mut state);
                                state.store(ui.ctx(), editor_id);
                            }
                            if was_formatting && !tab.formatter.busy() { ui.ctx().request_repaint(); }
                            let accepted = if ui.is_enabled() { tab.completion.before(ui.ctx(), editor_id) }
                                else { tab.completion.dismiss(); None };
                            if let Some(value) = accepted {
                                let mut state = egui::TextEdit::load_state(ui.ctx(), editor_id)
                                    .unwrap_or_default();
                                accept_completion(tab, &value, &mut state);
                                state.store(ui.ctx(), editor_id);
                            }
                            let mut output = TextEdit::multiline(&mut tab.sql)
                                .frame(false)
                                .id_salt(("query-editor", tab.id))
                                .font(egui::TextStyle::Monospace)
                                .code_editor()
                                .layouter(&mut crate::editor::layout)
                                .desired_rows(1)
                                .min_size(egui::vec2(0.0, editor_height))
                                .vertical_align(egui::Align::Min)
                                .desired_width(f32::INFINITY)
                                .lock_focus(true)
                                .show(ui);
                            if tab.search.scroll {
                                let rect = output
                                    .galley
                                    .pos_from_cursor(egui::text::CCursor::new(tab.cursor.start))
                                    .translate(output.galley_pos.to_vec2());
                                ui.scroll_to_rect(rect, Some(egui::Align::Center));
                                tab.search.scroll = false;
                            }
                            if let Some(cursor) = output.cursor_range {
                                tab.cursor = cursor.as_sorted_char_range();
                            }
                            let explicit_completion = output.response.has_focus()
                                && ui.input_mut(|i| {
                                    i.consume_key(egui::Modifiers::CTRL, egui::Key::Space)
                                });
                            tab.completion.cursor_changed(tab.cursor.clone());
                            if output.response.changed() {
                                tab.refresh_dirty();
                            }
                            if output.response.changed() || explicit_completion {
                                if explicit_completion {
                                    self.language.request_again();
                                    ui.ctx().request_repaint();
                                }
                                tab.completion.refresh(
                                    &tab.sql,
                                    tab.cursor.start,
                                    &self.objects,
                                    &self.columns,
                                    explicit_completion,
                                    crate::sql::Dialect::for_engine(&self.engine),
                                );
                            }
                            if output.response.has_focus()
                                || tab.completion.pointer_over_menu(ui.ctx())
                            {
                                let caret_rect = output
                                    .galley
                                    .pos_from_cursor(egui::text::CCursor::new(tab.cursor.start))
                                    .translate(output.galley_pos.to_vec2());
                                if let Some(value) = tab.completion.menu(
                                    ui.ctx(),
                                    editor_id,
                                    caret_rect.left_bottom(),
                                ) {
                                    accept_completion(tab, &value, &mut output.state);
                                    output.state.clone().store(ui.ctx(), editor_id);
                                    tab.focus_editor = true;
                                }
                            } else {
                                tab.completion.dismiss();
                            }
                            crate::editor::paint_execution_target(
                                ui,
                                slot,
                                &output,
                                &tab.sql,
                                tab.cursor.clone(),
                                crate::sql::Dialect::for_engine(&self.engine),
                            );
                            self.language.paint(ui, &output, &tab.sql);
                            if tab.focus_editor && ui.is_enabled() {
                                output.response.request_focus();
                                tab.focus_editor = false;
                            }
                            if let Some(action) = search_action
                                && let Ok(matcher) = tab.search.matcher(ui.ctx())
                            {
                                let before_cursor = egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(tab.cursor.start),
                                    egui::text::CCursor::new(tab.cursor.end),
                                );
                                let mut undoer = output.state.undoer();
                                if matches!(
                                    action,
                                    crate::find::Action::Replace | crate::find::Action::All
                                ) {
                                    undoer.add_undo(&(before_cursor, tab.sql.clone()));
                                }
                                let next = match action {
                                    crate::find::Action::Next | crate::find::Action::Previous => {
                                        crate::find::locate(
                                            &tab.sql,
                                            &matcher,
                                            tab.cursor.clone(),
                                            matches!(action, crate::find::Action::Previous),
                                        )
                                    }
                                    crate::find::Action::Replace | crate::find::Action::All => {
                                        Some(crate::find::replace(
                                            &mut tab.sql,
                                            &matcher,
                                            &tab.search.replacement,
                                            tab.cursor.clone(),
                                            matches!(action, crate::find::Action::All),
                                        ))
                                    }
                                };
                                if let Some(next) = next {
                                    let range = egui::text::CCursorRange::two(
                                        egui::text::CCursor::new(next.start),
                                        egui::text::CCursor::new(next.end),
                                    );
                                    tab.cursor = next;
                                    output.state.cursor.set_char_range(Some(range));
                                    if matches!(
                                        action,
                                        crate::find::Action::Replace | crate::find::Action::All
                                    ) {
                                        tab.refresh_dirty();
                                        undoer.add_undo(&(range, tab.sql.clone()));
                                        output.state.set_undoer(undoer);
                                    }
                                    output.state.clone().store(ui.ctx(), output.response.id);
                                    tab.search.scroll = true;
                                    tab.focus_editor = true;
                                    ui.ctx().request_repaint();
                                }
                            }
                            output
                        })
                        .inner
                });
            let current = crate::editor::line_index(ui.ctx(), &self.tabs[self.tab].sql);
            let caret = scrolled
                .inner
                .cursor_range
                .map(|r| r.primary.index)
                .unwrap_or(self.tabs[self.tab].cursor.start);
            active_caret = caret;
            crate::editor::paint_line_numbers(
                ui,
                gutter,
                &scrolled.inner,
                current.line_column(caret).0,
            );
        });
        ui.add_space(4.0);
        if !self.language.diagnostics.is_empty() {
            let message = &self.language.diagnostics[0].message;
            ui.label(
                RichText::new(format!(
                    "{} · {} diagnostic{}",
                    message.lines().next().unwrap_or("SQL diagnostic"),
                    self.language.diagnostics.len(),
                    if self.language.diagnostics.len() == 1 {
                        ""
                    } else {
                        "s"
                    }
                ))
                .small()
                .color(ui.visuals().warn_fg_color),
            )
            .on_hover_text(
                self.language
                    .diagnostics
                    .iter()
                    .map(|item| item.message.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        let current = crate::editor::line_index(ui.ctx(), &self.tabs[self.tab].sql);
        let (line, column) = current.line_column(active_caret);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Line {line}, column {column}"))
                    .size(11.0)
                    .color(ui.visuals().weak_text_color()),
            )
            .on_hover_text(&self.language.status);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                rounded_action_buttons(ui);
                ui.menu_button("File", |ui| {
                    if ui
                        .add_enabled(self.pending.is_none(), egui::Button::new("Save SQL"))
                        .clicked()
                    {
                        self.save_query_file(ui.ctx(), false);
                        ui.close();
                    }
                    if ui
                        .add_enabled(self.pending.is_none(), egui::Button::new("Open SQL…"))
                        .clicked()
                    {
                        self.script_dialog(ui.ctx(), false);
                        ui.close();
                    }
                    if ui
                        .add_enabled(self.pending.is_none(), egui::Button::new("Save SQL As…"))
                        .clicked()
                    {
                        self.save_query_file(ui.ctx(), true);
                        ui.close();
                    }
                    #[cfg(target_arch = "wasm32")]
                    if ui.button("Download SQL copy").clicked() {
                        self.script_dialog(ui.ctx(), true); ui.close();
                    }
                });
                let tab = &mut self.tabs[self.tab];
                if ui.add_enabled(!tab.formatter.busy() && !tab.sql.trim().is_empty(),
                    egui::Button::new(if tab.formatter.busy() { "Formatting…" } else { "Beautify" })
                        .shortcut_text(if ui.ctx().os() == egui::os::OperatingSystem::Mac { "⌘I" } else { "Ctrl+I" }))
                    .on_hover_text("Format selection, or the whole SQL document. Undo with Cmd/Ctrl+Z.")
                    .clicked() {
                    tab.formatter.start(ui.ctx(), &tab.sql, tab.cursor.clone(), crate::sql::Dialect::for_engine(&self.engine));
                }
                if ui.button("Save").on_hover_text("Save named SQL query · Cmd/Ctrl+S").clicked() {
                    self.save_query_file(ui.ctx(), false);
                }
                if self.pending.is_some() && ui.button("Cancel").clicked() {
                    self.worker.cancel();
                }
                if ui
                    .add_enabled(
                        self.pending.is_none(),
                        egui::Button::new("Run all").shortcut_text(if ui.ctx().os() == egui::os::OperatingSystem::Mac {
                            if cfg!(target_arch = "wasm32") { "Shift+⌘Enter" } else { "⇧⌘↵" }
                        } else {
                            "Ctrl+Shift+Enter"
                        }),
                    )
                    .on_hover_text("Cmd/Ctrl + Shift + Enter")
                    .clicked()
                {
                    self.run_query(true);
                }
                if ui
                    .add_enabled(
                        self.pending.is_none(),
                        egui::Button::new(
                            RichText::new(if self.tabs[self.tab].cursor.is_empty() {
                                "Run current"
                            } else {
                                "Run selection"
                            })
                            .color(Color32::WHITE),
                        )
                        .fill(BLUE)
                        .shortcut_text(
                            RichText::new(if ui.ctx().os() == egui::os::OperatingSystem::Mac {
                                if cfg!(target_arch = "wasm32") { "⌘Enter" } else { "⌘↵" }
                            } else {
                                "Ctrl+Enter"
                            })
                            .color(Color32::WHITE),
                        ),
                    )
                    .clicked()
                {
                    self.run_query(false);
                }
            });
        });
        if self.tabs[self.tab].results.len() > 1 || !self.tabs[self.tab].pins.items.is_empty() {
            ui.add_space(8.0);
            egui::ScrollArea::horizontal()
                .id_salt("result-tabs")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let tab = &mut self.tabs[self.tab];
                        let mut close_pin = None;
                        for (index, pin) in tab.pins.items.iter().enumerate() {
                            let response = ui.selectable_label(tab.selected_pin == Some(index), format!("Pinned {} · {} rows", pin.id, pin.grid.rows.len()));
                            if response.clicked() {
                                tab.selected_pin = Some(index);
                                tab.grid = pin.grid.clone();
                                self.row_selection.clear();
                                self.selected = None;
                            }
                            response.context_menu(|ui| {
                                if ui.button("Close pinned result").clicked() { close_pin = Some(index); ui.close(); }
                            });
                        }
                        if let Some(index) = close_pin {
                            tab.remove_pin(index);
                            self.row_selection.clear();
                            self.selected = None;
                        }
                        for (index, grid) in tab.results.iter().enumerate() {
                            let label = if grid.columns.is_empty() {
                                format!("{} · {} affected", index + 1, grid.affected)
                            } else {
                                format!(
                                    "{} · {} rows{}",
                                    index + 1,
                                    grid.rows.len(),
                                    if grid.truncated { "+" } else { "" }
                                )
                            };
                            if ui
                                .selectable_label(tab.selected_pin.is_none() && tab.result_index == index, label)
                                .clicked()
                            {
                                tab.result_index = index;
                                tab.selected_pin = None;
                                tab.grid = grid.clone();
                                self.view = View::Query;
                                self.row_selection.clear();
                                self.selected = None;
                            }
                        }
                    });
                });
        }
    }

    fn apply_table_filter(&mut self, clear: bool) {
        // Keep the displayed rows and their applied filter in agreement when
        // an invalid draft prevents leaving the current edit.
        if self.pending.is_some() || self.browse.table.is_empty() || !self.finish_cell_edit() {
            return;
        }
        if clear { self.filter.clear(); }
        self.browse.offset = 0;
        self.browse.filter_column = if self.filter.is_empty() {
            None
        } else {
            Some(self.filter_column.clone())
        };
        self.browse.filter = self.filter.clone();
        self.refresh();
    }

    fn filters(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(self.pending.is_none(), |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Filter").color(MUTED));
                egui::ComboBox::from_id_salt("filter-column")
                    .selected_text(&self.filter_column)
                    .width(130.0)
                    .show_ui(ui, |ui| {
                        for column in &self.columns {
                            ui.selectable_value(
                                &mut self.filter_column,
                                column.name.clone(),
                                &column.name,
                            );
                        }
                    });
                ui.label(RichText::new("contains").size(11.0).color(MUTED));
                let response = ui.add(
                    TextEdit::singleline(&mut self.filter)
                        .id(egui::Id::new("table-filter-value"))
                        .hint_text("Filter values…")
                        .desired_width(170.0),
                );
                if self.focus_filter && ui.is_enabled() {
                    response.request_focus();
                    self.focus_filter = false;
                }
                if ui.button("Apply").clicked()
                    || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    self.apply_table_filter(false);
                }
                if self.browse.filter_column.is_some() && ui.small_button("Clear").clicked() {
                    self.apply_table_filter(true);
                }
            });
        });
        ui.add_space(12.0);
    }

    /// Pending requests retain the table's geometry without exposing stale cells.
    fn empty_data_grid(&self, ui: &mut egui::Ui, columns: &[String]) {
        let count = if columns.is_empty() { 5 } else { columns.len() };
        let scope = self.active_profile.as_ref().map(|identity| {
            if self.view == View::Query {
                crate::column_widths::Scope::Query {
                    profile: identity.id,
                    columns: columns.to_vec(),
                }
            } else {
                crate::column_widths::Scope::Table {
                    profile: identity.id,
                    table: self.browse.table.clone(),
                }
            }
        });
        let restored_id =
            ui.make_persistent_id(("restored-grid-widths", &scope, columns, self.tab));
        let defaults_id = ui.make_persistent_id(("content-column-widths", &scope, columns));
        let mut widths = ui
            .ctx()
            .data(|data| data.get_temp::<Vec<f32>>(restored_id))
            .or_else(|| {
                ui.ctx()
                    .data(|data| {
                        data.get_temp::<(std::sync::Weak<Grid>, Arc<Vec<f32>>)>(defaults_id)
                    })
                    .map(|(_, widths)| widths.as_ref().clone())
            })
            .unwrap_or_else(|| {
                if columns.is_empty() {
                    vec![crate::column_widths::DEFAULT_WIDTH; count]
                } else {
                    content_column_widths(
                        ui,
                        &Grid {
                            columns: columns.to_vec(),
                            ..Default::default()
                        },
                    )
                }
            });
        if let Some(scope) = &scope {
            let cache_id = ui.make_persistent_id(("prepared-column-widths", scope, columns));
            let keys = ui.ctx().data_mut(|data| {
                data.get_temp_mut_or_insert_with(cache_id, || {
                    Arc::new(crate::column_widths::Prepared::new(scope, columns))
                })
                .clone()
            });
            let saved = self.prefs.column_widths.prepared_widths(&keys);
            for (index, width) in widths.iter_mut().enumerate() {
                if self.prefs.column_widths.has_width(&keys, index) {
                    *width = saved[index];
                }
            }
        }
        let content_width = 38.0
            + widths.iter().sum::<f32>()
            + ui.spacing().item_spacing.x * (widths.len() + 1) as f32;
        let horizontal_overflow = content_width > ui.available_width() + 0.5;
        with_grid_scrollbars(ui, |ui| {
            egui::ScrollArea::horizontal()
                .scroll_bar_visibility(if horizontal_overflow {
                    egui::scroll_area::ScrollBarVisibility::AlwaysVisible
                } else {
                    egui::scroll_area::ScrollBarVisibility::AlwaysHidden
                })
                .id_salt((
                    "data-horizontal",
                    &self.browse.table,
                    self.view == View::Query,
                    self.tab,
                ))
                .show(ui, |ui| {
                    ui.set_min_width(content_width.max(ui.available_width()));
                    let mut table = TableBuilder::new(ui)
                        .id_salt(("loading-data", &scope, columns, self.tab))
                        .auto_shrink([false, false])
                        .striped(false)
                        .resizable(false)
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .column(TableColumn::exact(38.0));
                    for width in &widths {
                        table = table.column(TableColumn::exact(*width).clip(true));
                    }
                    table
                        .sense(egui::Sense::hover())
                        .min_scrolled_height(0.0)
                        .header(25.0, |mut header| {
                            header.col(paint_grid_corner);
                            for index in 0..count {
                                header.col(|ui| {
                                    if let Some(name) = columns.get(index) {
                                        paint_column_header(ui, name, None);
                                    }
                                });
                            }
                        })
                        .body(|mut body| {
                            let widths = body.widths().to_vec();
                            paint_grid_backdrop(body.ui_mut(), &widths);
                        });
                });
        });
    }

    fn data_grid(&mut self, ui: &mut egui::Ui) {
        let grid = self.grid();
        if self.pending.is_some() {
            let names;
            let columns = if grid.columns.is_empty() && self.view == View::Data {
                names = self
                    .columns
                    .iter()
                    .map(|column| column.name.clone())
                    .collect::<Vec<_>>();
                &names
            } else {
                &grid.columns
            };
            self.empty_data_grid(ui, columns);
            return;
        }
        if grid.columns.is_empty() {
            ui.add_space(50.0);
            ui.vertical_centered(|ui| {
                if self.view == View::Query
                    && !self.tabs[self.tab].results.is_empty()
                    && self.pending.is_none()
                {
                    ui.heading("Statement completed");
                    ui.label(format!(
                        "{} rows affected · {:.1} ms",
                        grid.affected,
                        grid.elapsed.as_secs_f64() * 1000.0
                    ));
                    return;
                }
                ui.label(
                    RichText::new("Your results will appear here")
                        .size(20.0)
                        .color(MUTED),
                );
                ui.add_space(8.0);
                ui.label("Choose a table or run a SQL query.");
            });
            return;
        }
        // Keep keyboard ownership on the grid as selection moves between
        // virtualized rows. Moving focus to a new row resets egui's key filter.
        let grid_focus = ui.interact(
            ui.available_rect_before_wrap(),
            ui.make_persistent_id((
                "grid-keyboard",
                &self.browse.table,
                self.view == View::Query,
                self.tab,
            )),
            egui::Sense::focusable_noninteractive(),
        );
        if self.view == View::Query {
            self.tabs[self.tab].result_grid_focus = Some(grid_focus.id);
        }
        if self.view == View::Data && self.focus_page_grid && self.pending.is_none() {
            self.focus_page_grid = false;
            if ui.is_enabled() && ui.ctx().memory(|m| m.focused().is_none() || m.has_focus(grid_focus.id)) {
                grid_focus.request_focus();
            }
        }
        let mut sort = None;
        let mut explicit_sort = None;
        let insert_ids = self.insert_ids();
        let mut edit = None;
        let mut cell_action = None;
        let mut row_action = None;
        let mut bulk_delete = None;
        let selected_deleted = self.selected_rows_deleted();
        let navigation_target = self
            .row_selection
            .pending_focus
            .take()
            .filter(|i| *i < grid.rows.len());
        let mut finish = false;
        let editable = self.view == View::Data
            && self.supports_data_changes()
            && !self.safe
            && self.pending.is_none()
            && self.columns.iter().any(|c| c.pk)
            && self
                .objects
                .iter()
                .any(|o| o.name == self.browse.table && o.kind == "table");
        let start_edit_key = editable
            && grid_focus.has_focus()
            && self.edit.is_none()
            && self.review.is_none()
            && !self.connections
            && !self.history
            && !egui::Popup::is_any_open(ui.ctx())
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let width_scope = self.active_profile.as_ref().map(|identity| {
            if self.view == View::Query {
                crate::column_widths::Scope::Query {
                    profile: identity.id,
                    columns: grid.columns.clone(),
                }
            } else {
                crate::column_widths::Scope::Table {
                    profile: identity.id,
                    table: self.browse.table.clone(),
                }
            }
        });
        let prepared_widths = width_scope.as_ref().map(|scope| {
            let cache_id = ui.make_persistent_id(("prepared-column-widths", scope, &grid.columns));
            ui.ctx().data_mut(|data| {
                data.get_temp_mut_or_insert_with(cache_id, || {
                    Arc::new(crate::column_widths::Prepared::new(scope, &grid.columns))
                })
                .clone()
            })
        });
        let defaults_id =
            ui.make_persistent_id(("content-column-widths", &width_scope, &grid.columns));
        let defaults = ui
            .ctx()
            .data(|data| data.get_temp::<(std::sync::Weak<Grid>, Arc<Vec<f32>>)>(defaults_id))
            .filter(|(source, _)| std::ptr::eq(source.as_ptr(), Arc::as_ptr(&grid)))
            .map(|(_, widths)| widths)
            .unwrap_or_else(|| {
                let widths = Arc::new(content_column_widths(ui, &grid));
                ui.ctx().data_mut(|data| {
                    data.insert_temp(defaults_id, (Arc::downgrade(&grid), widths.clone()))
                });
                widths
            });
        let mut initial_widths = defaults.as_ref().clone();
        if let Some(keys) = &prepared_widths {
            let saved = self.prefs.column_widths.prepared_widths(keys);
            for (index, width) in initial_widths.iter_mut().enumerate() {
                if self.prefs.column_widths.has_width(keys, index) {
                    *width = saved[index];
                }
            }
        }
        let sizing_id = ui.make_persistent_id((
            "grid-column-sizing",
            &self.browse.table,
            self.view == View::Query,
            self.tab,
            &grid.columns,
        ));
        let restored_widths_id = ui.make_persistent_id((
            "restored-grid-widths",
            &width_scope,
            &grid.columns,
            self.tab,
        ));
        let previous_widths = ui
            .ctx()
            .data(|data| data.get_temp::<Vec<f32>>(restored_widths_id));
        let mut restore_widths = previous_widths.as_ref() != Some(&initial_widths);
        if prepared_widths.is_none()
            && let Some(previous) = &previous_widths
        {
            initial_widths.clone_from(previous);
            restore_widths = false;
        }
        let fit_columns = ui
            .ctx()
            .data_mut(|data| data.remove_temp::<bool>(sizing_id).unwrap_or(false));
        let widths_id = sizing_id.with("widths");
        let measured_width = ui.ctx().data(|data| data.get_temp::<f32>(widths_id));
        let expected_width = 38.0
            + initial_widths.iter().sum::<f32>()
            + ui.spacing().item_spacing.x * (initial_widths.len() + 1) as f32;
        let content_width = if restore_widths {
            expected_width
        } else {
            measured_width.unwrap_or(expected_width)
        };
        let horizontal_overflow = content_width > ui.available_width() + 0.5;
        with_grid_scrollbars(ui, |ui| {
            egui::ScrollArea::horizontal()
            .scroll_bar_visibility(if horizontal_overflow { egui::scroll_area::ScrollBarVisibility::AlwaysVisible } else { egui::scroll_area::ScrollBarVisibility::AlwaysHidden })
            .id_salt((
                "data-horizontal",
                &self.browse.table,
                self.view == View::Query,
                self.tab,
            ))
            .show(ui, |ui| {
                let width = content_width.max(ui.available_width());
                ui.set_min_width(width);
                let table_salt = (
                    "data", &width_scope, &grid.columns, &self.browse.table,
                    self.view == View::Query, self.tab,
                );
                let table_state_id = ui.id().with(egui::Id::new(table_salt));
                let resize_intent_id = table_state_id.with("user-resized-columns");
                let resized_columns = ui.ctx().data_mut(|data| data.remove_temp::<Vec<usize>>(resize_intent_id).unwrap_or_default());
                let mut active_cell_overlay = None;
                let mut builder = TableBuilder::new(ui)
                    .id_salt(table_salt)
                    .striped(false)
                    .auto_shrink([false, false])
                    .resizable(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(TableColumn::exact(38.0))
                    .sense(egui::Sense::click())
                    .min_scrolled_height(0.0);
                for initial_width in &initial_widths {
                    builder = builder.column(
                        TableColumn::initial(*initial_width)
                            .at_least(crate::column_widths::MIN_WIDTH)
                            .at_most(crate::column_widths::MAX_WIDTH)
                            .clip(true)
                            .auto_size_this_frame(fit_columns),
                    );
                }
                if restore_widths { builder.reset(); }
                if let Some(index) = navigation_target {
                    builder = builder.scroll_to_row(index, None);
                }
                if let Some(edit) = self.edit.as_ref().filter(|e| e.focus && !e.inspector)
                    && let Some(index) = grid.rows.iter().position(|row| *row == edit.row) {
                    builder = builder.scroll_to_row(index, None);
                }
                if let Some(id) = self.edit.as_ref().filter(|e| e.focus).and_then(|e| e.insert_id)
                    && let Some(i) = insert_ids.iter().position(|item| *item == id) {
                    builder = builder.scroll_to_row(grid.rows.len() + i, None);
                }
                builder.header(25.0, |mut header| {
                        header.col(|ui| {
                            paint_grid_corner(ui);
                            if ui.interact(ui.max_rect(), ui.id().with("fit-columns"), egui::Sense::click())
                                .on_hover_text("Fit columns to visible content")
                                .clicked()
                            {
                                ui.ctx().data_mut(|data| data.insert_temp(sizing_id, true));
                                ui.ctx().request_repaint();
                            }
                        });
                        for name in &grid.columns {
                            header.col(|ui| {
                                if self.view == View::Query {
                                    paint_column_header(ui, name, None);
                                    return;
                                }
                                let direction = self
                                    .browse
                                    .sort
                                    .as_ref()
                                    .filter(|(c, _)| c == name)
                                    .map(|(_, d)| *d);
                                let response = paint_column_header(ui, name, direction);
                                response.context_menu(|ui| {
                                    ui.add_enabled_ui(self.pending.is_none(), |ui| {
                                        for (label, descending) in [("Sort ascending", false), ("Sort descending", true)] {
                                            if ui.selectable_label(direction == Some(descending), label).clicked() {
                                                explicit_sort = Some(Some((name.clone(), descending)));
                                                ui.close();
                                            }
                                        }
                                        ui.separator();
                                        if ui.add_enabled(self.browse.sort.is_some(), egui::Button::new("Restore default order"))
                                            .on_hover_text("Clear the selected column sort and use the table's default order")
                                            .clicked() {
                                            explicit_sort = Some(None);
                                            ui.close();
                                        }
                                    });
                                });
                                if response.on_hover_text(match direction {
                                    None => "Sort descending",
                                    Some(true) => "Sort ascending",
                                    Some(false) => "Clear column sort",
                                }).clicked() && self.pending.is_none() {
                                    sort = Some(name.clone());
                                }
                            });
                        }
                    })
                    .body(|mut body| {
                        let table_clip = body.ui_mut().clip_rect();
                        let widths = body.widths().to_vec();
                        if !body.ui_mut().is_sizing_pass() {
                            if !restore_widths && let (Some(keys), Some(previous)) = (&prepared_widths, &previous_widths) {
                                self.shared_preferences_dirty |= self.prefs.column_widths.record_changed(keys, &widths[1..], previous, &resized_columns);
                            }
                            body.ui_mut().ctx().data_mut(|data| data.insert_temp(restored_widths_id, widths[1..].to_vec()));
                        }
                        let total_width = widths.iter().sum::<f32>() + body.ui_mut().spacing().item_spacing.x * widths.len() as f32;
                        body.ui_mut().ctx().data_mut(|data| data.insert_temp(widths_id, total_width));
                        paint_grid_backdrop(body.ui_mut(), &widths);
                        body.rows(22.0, grid.rows.len() + insert_ids.len(), |mut row| {
                            let index = row.index();
                            if index >= grid.rows.len() {
                                self.insert_grid_row(&mut row, insert_ids[index - grid.rows.len()], index, table_clip);
                                return;
                            }
                            let deleting = self.view == View::Data && row_is_deleted(&self.changes, &self.browse.table, &grid.rows[index]);
                            let row_selected = self.selected == Some(index) || self.row_selection.rows.contains(&index);
                            row.set_hovered(false);
                            row.set_selected(false);
                            row.col(|ui| {
                                paint_row_number(ui, &(index
                                            + 1
                                            + if self.view == View::Data {
                                                self.browse.offset
                                            } else {
                                                0
                                            })
                                        .to_string());
                            });
                            row.set_selected(row_selected);
                            let mut pressed = false;
                            let mut released = false;
                            for (column, original_value) in grid.rows[index].iter().enumerate() {
                                let staged = if self.view == View::Data { self.changes.iter().find_map(|change| match change {
                                    Change::Cell { table, column: name, original, value } if table == &self.browse.table && name == &grid.columns[column] && original == &grid.rows[index] => Some(value),
                                    _ => None,
                                }) } else { None };
                                let value = staged.unwrap_or(original_value);
                                let mut double_clicked = false;
                                let mut cell_pressed = false;
                                let mut cell_released = false;
                                let active_cell = self.row_selection.focused == Some(index)
                                    && self.row_selection.rows.contains(&index)
                                    && self.row_selection.cell_column.min(grid.columns.len().saturating_sub(1)) == column;
                                let editing = self.edit.as_ref().is_some_and(|e| !e.inspector && e.table == self.browse.table && e.column == grid.columns[column] && e.row == grid.rows[index]);
                                let (_, response) = row.col(|ui| {
                                    if deleting {
                                        let fill = if ui.visuals().dark_mode { Color32::from_rgb(100, 35, 40) } else { Color32::from_rgb(255, 210, 213) };
                                        paint_cell_background(ui, table_clip, fill);
                                    } else if staged.is_some() {
                                        paint_cell_background(ui, table_clip, pending_fill(ui));
                                    }
                                    if active_cell {
                                        active_cell_overlay = Some((ui.max_rect().expand2(ui.spacing().item_spacing * 0.5), table_clip));
                                        if self.row_selection.pending_cell_scroll {
                                            ui.scroll_to_rect(ui.max_rect(), None);
                                            self.row_selection.pending_cell_scroll = false;
                                        }
                                    }
                                    if editing {
                                        finish |= inline_value(ui, &mut self.edit);
                                        if self.edit.is_none() {
                                            grid_focus.request_focus();
                                            ui.ctx().request_repaint();
                                        }
                                        return;
                                    }
                                    let text = db::preview(value, 180);
                                    let color = if deleting || staged.is_some() { ui.visuals().text_color() } else if row_selected { ui.visuals().selection.stroke.color } else { match value {
                                        Value::Null | Value::Blob(_) => MUTED,
                                        Value::Integer(_) | Value::Real(_) => ui.visuals().hyperlink_color,
                                        _ if text == "Active" || text == "Paid" => GREEN,
                                        _ => ui.visuals().text_color(),
                                    }};
                                    let label = ui.add(
                                        egui::Label::new(
                                            RichText::new(text.as_ref()).monospace().size(12.0).color(color),
                                        )
                                        .wrap_mode(if fit_columns { egui::TextWrapMode::Extend } else { egui::TextWrapMode::Truncate })
                                        .selectable(false).sense(egui::Sense::click()),
                                    );
                                    double_clicked = label.double_clicked();
                                    cell_pressed = primary_pressed(&label);
                                    cell_released = label.clicked();
                                    label.context_menu(|ui| {
                                        if ui.button("Copy cell").clicked() {
                                            ui.ctx().copy_text(db::display(value));
                                            ui.close();
                                        }
                                        ui.menu_button("Copy rows as", |ui| {
                                            for (label, delimiter, headers) in [("Tab-separated", '\t', false), ("Tab-separated with headers", '\t', true), ("CSV", ',', false), ("CSV with headers", ',', true)] {
                                                if ui.button(label).clicked() {
                                                    match row_copy::format_rows(&grid.columns, self.row_selection.copy_indices(index).into_iter().filter_map(|i| grid.rows.get(i).map(Vec::as_slice)), &self.changes, (self.view == View::Data).then_some(self.browse.table.as_str()), delimiter, headers) {
                                                        Ok(text) => ui.ctx().copy_text(text),
                                                        Err(error) => self.error = Some(error),
                                                    }
                                                    ui.close();
                                                }
                                            }
                                        });
                                        if self.row_selection.rows.len() > 1 && self.row_selection.rows.contains(&index) {
                                            let restore = selected_deleted;
                                            let label = format!("{} {} selected rows", if restore { "Restore" } else { "Delete" }, self.row_selection.rows.len());
                                            if ui.add_enabled(editable, egui::Button::new(label)).clicked() {
                                                bulk_delete = Some(!restore);
                                                ui.close();
                                            }
                                        } else if ui.add_enabled(editable, egui::Button::new(if deleting { "Restore row" } else { "Delete row" })).clicked() {
                                            row_action = Some((index, grid.rows[index].clone(), deleting));
                                            ui.close();
                                        }
                                        let writable = editable && !deleting && !self.columns.get(column).is_some_and(|c| c.generated);
                                        if ui.add_enabled(writable && !self.columns.get(column).is_some_and(|c| c.not_null), egui::Button::new("Set NULL")).clicked() {
                                            cell_action = Some(CellEdit { bulk_rows: Vec::new(), bulk_changed: false, insert_id: None, advance: 0, table: self.browse.table.clone(), row: grid.rows[index].clone(), original_value: original_value.clone(), column: grid.columns[column].clone(), text: String::new(), kind: 0, focus: false, inspector: false });
                                            ui.close();
                                        }
                                        if ui.add_enabled(writable && staged.is_some(), egui::Button::new("Revert cell")).clicked() {
                                            cell_action = Some(CellEdit { bulk_rows: Vec::new(), bulk_changed: false, insert_id: None, advance: 0, table: self.browse.table.clone(), row: grid.rows[index].clone(), original_value: original_value.clone(), column: grid.columns[column].clone(), text: if matches!(original_value, Value::Null) { String::new() } else { db::display(original_value) }, kind: match original_value { Value::Null => 0, Value::Integer(_) => 1, Value::Real(_) => 2, Value::Blob(_) => 4, _ => 3 }, focus: false, inspector: false });
                                            ui.close();
                                        }
                                    });
                                });
                                let cell_press = cell_pressed || primary_pressed(&response);
                                if cell_press { self.row_selection.cell_column = column; }
                                pressed |= cell_press;
                                released |= cell_released || response.clicked();
                                if (response.double_clicked() || double_clicked || (start_edit_key && active_cell))
                                    && editable
                                    && !deleting
                                    && !self.columns.get(column).is_some_and(|c| c.generated)
                                    && !matches!(value, Value::Blob(_))
                                {
                                    if matches!(value, Value::Text(text) if text.len() > 65536) {
                                        self.error = Some("This value exceeds the 64 KiB inline editor limit. Use SQL to edit it; Copy full value preserves the complete text.".into());
                                        return;
                                    }
                                    edit = Some(CellEdit {
            bulk_rows: Vec::new(), bulk_changed: false, insert_id: None,                                        advance: 0,                                        table: self.browse.table.clone(),
                                        focus: true,
                                        inspector: false,
                                        row: grid.rows[index].clone(),
                                        original_value: original_value.clone(),
                                        column: grid.columns[column].clone(),
                                        text: if matches!(value, Value::Null) {
                                            String::new()
                                        } else {
                                            db::display(value)
                                        },
                                        kind: match value {
                                            Value::Null => 0,
                                            Value::Integer(_) => 1,
                                            Value::Real(_) => 2,
                                            _ => 3,
                                        },
                                    });
                                }
                            }
                            if pressed || primary_pressed(&row.response()) {
                                if self.edit.as_ref().is_some_and(|e| !e.bulk_rows.is_empty()) && !self.finish_cell_edit() {
                                    return;
                                }
                                let modifiers = row.response().ctx.input(|i| i.events.iter().rev().find_map(|event| match event {
                                    egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: true, modifiers, .. } => Some(*modifiers),
                                    _ => None,
                                }).unwrap_or(i.modifiers));
                                self.selected = self.row_selection.click(index, modifiers.shift, modifiers.command, self.selected.filter(|i| *i < grid.rows.len()));
                                if self.edit.is_none() && edit.is_none() { grid_focus.request_focus(); grid_focus.ctx.request_repaint(); }
                            }
                            self.restore_grid_keyboard_focus(&grid_focus, released || row.response().clicked(), edit.is_some());

                        });
                    });
                if let Some((rect, clip)) = active_cell_overlay { paint_active_cell(ui, rect, clip); }
                // TableBuilder applies drag/Fit geometry after the body callback.
                // Record the intent now, and capture only those columns' resulting
                // widths in the next frame. Layout changes alone are never saved.
                let resized: Vec<usize> = (0..grid.columns.len()).filter(|column| {
                    fit_columns || ui.ctx().read_response(table_state_id.with("resize_column").with(column + 1))
                        .is_some_and(|response| response.dragged() || response.double_clicked())
                }).collect();
                if !resized.is_empty() {
                    ui.ctx().data_mut(|data| data.insert_temp(resize_intent_id, resized));
                    ui.ctx().request_repaint();
                }
            });
        });
        self.page_grid_shortcut(&grid_focus);
        if let Some(index) = self.row_selection.focused.filter(|i| *i < grid.rows.len()) {
            self.navigate_rows_shortcut(&grid_focus, index);
            self.select_all_rows_shortcut(&grid_focus.ctx, grid_focus.has_focus().then_some(index));
            self.copy_rows_shortcut(&grid_focus.ctx, grid_focus.has_focus());
            if self.row_delete_shortcut(&grid_focus.ctx, grid_focus.has_focus()) {
                bulk_delete = Some(true);
            }
        }
        if let Some(order) = explicit_sort {
            if self.finish_cell_edit() {
                self.browse.sort = order;
                self.browse.offset = 0;
                self.refresh();
            }
        } else if let Some(column) = sort
            && self.finish_cell_edit()
        {
            self.browse.cycle_sort(column);
            self.refresh();
        }
        if finish {
            self.finish_cell_edit();
        }
        if edit.is_some() && self.finish_cell_edit() {
            self.edit = edit;
        }
        if let Some(deleted) = bulk_delete {
            self.set_selected_rows_deleted(deleted);
        }
        if let Some((index, row, deleting)) = row_action
            && self.finish_cell_edit()
        {
            toggle_row_deletion(&mut self.changes, &self.browse.table, row);
            self.record_changes();
            self.review = None;
            self.row_selection.clear();
            self.selected = if deleting {
                Some(index)
            } else {
                (index + 1 < grid.rows.len()).then_some(index + 1)
            };
        }
        if cell_action.is_some() && self.finish_cell_edit() {
            self.edit = cell_action;
            self.finish_cell_edit();
        }
    }

    fn column_inspector(&mut self, ui: &mut egui::Ui) {
        let schema_editable = self.supports_schema_changes();
        ui.spacing_mut().item_spacing.y = 3.0;
        let Some(index) = self.selected_column else {
            ui.label("Select a column to inspect its properties.");
            return;
        };
        let Some(column) = self.columns.get(index) else {
            return;
        };
        let pending_name = self.changes.iter().find_map(|change| match change {
            Change::RenameColumn { table, from, to }
                if table == &self.browse.table && from == &column.name =>
            {
                Some(to.as_str())
            }
            _ => None,
        });
        let (preview, has_changes) = column_preview(column, &self.browse.table, &self.changes);
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (label, value) in [
                ("Ordinal position", (index + 1).to_string()),
                (
                    "Column name",
                    pending_name.unwrap_or(&column.name).to_owned(),
                ),
                ("Data type", preview.data_type.clone()),
                (
                    "Nullable",
                    if preview.not_null || preview.pk {
                        "NO"
                    } else {
                        "YES"
                    }
                    .into(),
                ),
                (
                    "Default",
                    if preview.default.is_empty() {
                        "No default".into()
                    } else {
                        preview.default.clone()
                    },
                ),
                ("Primary key", if column.pk { "YES" } else { "NO" }.into()),
                (
                    "Generated",
                    if column.generated { "YES" } else { "NO" }.into(),
                ),
            ] {
                compact_property(ui, label, &value);
            }
            if has_changes {
                ui.label(RichText::new("Changes staged · Cmd+S to review").color(Color32::from_rgb(210,150,50)));
            }
            if ui.add_enabled(schema_editable && !self.safe && self.pending.is_none(),egui::Button::new("Edit properties…")).clicked() {
                let mut draft = AlterDraft { table:self.browse.table.clone(),column:column.name.clone(),data_type:column.data_type.clone(),original_type:column.data_type.clone(),not_null:column.not_null,original_not_null:column.not_null,default_kind:0,default_text:String::new(),error:None };
                if let Some(Change::AlterColumn { data_type, not_null, default, .. }) = self.changes.iter().find(|c|matches!(c,Change::AlterColumn {table,column: name,..} if table == &draft.table && name == &draft.column)) {
                    if let Some(value) = data_type { draft.data_type = value.clone(); }
                    if let Some(value) = not_null { draft.not_null = *value; }
                    match default {
                        None => {}, Some(None) => draft.default_kind = 1,
                        Some(Some(value)) => { draft.default_kind = match value { Value::Null => 2,Value::Integer(_) => 3,Value::Real(_) => 4,_ => 5 }; draft.default_text = db::display(value); }
                    }
                }
                self.alter_column = Some(draft);
            }

            if ui
                .add_enabled(
                    schema_editable && !self.safe && self.pending.is_none(),
                    egui::Button::new("Rename column…"),
                )
                .clicked()
            {
                self.rename_inline = false;
                self.rename = Some((
                    column.name.clone(),
                    pending_name.unwrap_or(&column.name).into(),
                ));
            }
        });
    }

    fn index_view(&mut self, ui: &mut egui::Ui) {
        ui.style_mut().interaction.selectable_labels = false;
        let schema_editable = self.supports_schema_changes();
        ui.horizontal(|ui| {
            ui.strong(self.current_table_name()).on_hover_text(&self.browse.table);
            if ui.add_enabled(schema_editable && !self.safe && self.pending.is_none() && !self.columns.is_empty(),egui::Button::new("+ Index")).clicked() {
                let first = self.columns[0].name.clone();
                self.index_draft = Some(IndexDraft {table:self.browse.table.clone(),name:format!("{}_{}_idx",self.current_table_name(),first),available_columns:self.columns.iter().map(|c|c.name.clone()).collect(),keys:vec![(first,false)],unique:false});
            }
            if let Some(index) = self.selected_index.and_then(|i|self.indexes.get(i)) {
                let staged = self.changes.iter().any(|c|matches!(c,Change::DropIndex {table,name} if table == &self.browse.table && name == &index.name));
                if ui.add_enabled(schema_editable && !self.safe && self.pending.is_none() && index.origin == "c" && !staged,egui::Button::new("− Index")).on_hover_text("Stage index removal; review with Cmd+S").clicked() {
                    self.changes.push(Change::DropIndex {table:self.browse.table.clone(),name:index.name.clone()}); self.record_changes(); self.review=None;
                }
                if staged {ui.label(RichText::new("Removal staged").color(Color32::from_rgb(210,150,50)));}
            }
        });
        if self.indexes.is_empty() {
            ui.label(if self.engine == "SQLite" { "This table has no separate indexes. SQLite rowid primary keys do not create a separate index." } else { "This table has no indexes." });
            return;
        }
        egui::ScrollArea::horizontal().show(ui, |ui| {
            TableBuilder::new(ui)
                .id_salt(("indexes", &self.browse.table))
                .striped(true)
                .resizable(true)
                .sense(egui::Sense::click())
                .column(TableColumn::initial(240.0))
                .column(TableColumn::initial(260.0))
                .columns(TableColumn::initial(100.0), 3)
                .header(25.0, |mut row| {
                    for title in ["Name", "Columns", "Unique", "Origin", "Partial"] {
                        row.col(|ui| {
                            paint_column_header(ui, title, None);
                        });
                    }
                })
                .body(|body| {
                    body.rows(24.0, self.indexes.len(), |mut row| {
                        let i = row.index();
                        let index = &self.indexes[i];
                        row.set_hovered(false);
                        row.set_selected(self.selected_index == Some(i));
                        row.col(|ui| {
                            ui.monospace(&index.name);
                        });
                        row.col(|ui| {
                            ui.monospace(index.columns.join(", "));
                        });
                        row.col(|ui| {
                            ui.label(if index.unique { "YES" } else { "NO" });
                        });
                        row.col(|ui| {
                            ui.label(match index.origin.as_str() {
                                "pk" => "Primary key",
                                "u" => "Constraint",
                                _ => "Index",
                            });
                        });
                        row.col(|ui| {
                            ui.label(if index.partial { "YES" } else { "NO" });
                        });
                        if primary_pressed(&row.response()) {
                            self.selected_index = Some(i);
                        }
                    });
                });
        });
    }

    fn structure(&mut self, ui: &mut egui::Ui) {
        ui.style_mut().interaction.selectable_labels = false;
        let schema_editable = self.supports_schema_changes();
        ui.horizontal(|ui| {
            ui.strong(self.current_table_name())
                .on_hover_text(&self.browse.table);
            ui.add(
                TextEdit::singleline(&mut self.structure_search).hint_text("Search for column…"),
            );
            ui.menu_button("DDL", |ui| {
                if let Some(object) = self.objects.iter().find(|o| o.name == self.browse.table) {
                    if clipboard_button(ui).clicked() {
                        ui.ctx().copy_text(object.ddl.clone());
                    }
                    egui::ScrollArea::both().max_height(300.0).show(ui, |ui| {
                        ui.code(&object.ddl);
                    });
                }
            });
        });
        let mut set_nullability = None;
        let mut start_rename = None;
        let mut finish_rename = false;
        let mut cancel_rename = false;
        let search = self.structure_search.to_lowercase();
        let visible: Vec<usize> = self
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.name.to_lowercase().contains(&search))
            .map(|(i, _)| i)
            .collect();
        egui::ScrollArea::horizontal().show(ui, |ui| {
            TableBuilder::new(ui)
                .id_salt(("structure-grid", &self.browse.table))
                .striped(false)
                .auto_shrink([false, false])
                .min_scrolled_height(0.0)
                .resizable(true)
                .sense(egui::Sense::click())
                .column(TableColumn::exact(38.0))
                .columns(TableColumn::initial(150.0), 5)
                .header(25.0, |mut row| {
                    row.col(paint_grid_corner);
                    for title in ["Column name", "Data type", "Nullable", "Default", "Key"] {
                        row.col(|ui| {
                            paint_column_header(ui, title, None);
                        });
                    }
                })
                .body(|mut body| {
                    let widths = body.widths().to_vec();
                    paint_grid_backdrop(body.ui_mut(), &widths);
                    let table_clip = body.ui_mut().clip_rect();
                    body.rows(22.0, visible.len(), |mut row| {
                        let index = visible[row.index()];
                        row.set_hovered(false);
                        row.set_selected(false);
                        let column = &self.columns[index];
                        let (preview, changed) =
                            column_preview(column, &self.browse.table, &self.changes);
                        row.col(|ui| {
                            paint_row_number(ui, &(index + 1).to_string());
                        });
                        row.set_selected(self.selected_column == Some(index));
                        let (_, response) = row.col(|ui| {
                            let pending_name =
                                self.changes.iter().find_map(|change| match change {
                                    Change::RenameColumn { table, from, to }
                                        if table == &self.browse.table && from == &column.name =>
                                    {
                                        Some(to)
                                    }
                                    _ => None,
                                });
                            if self.rename_inline
                                && self
                                    .rename
                                    .as_ref()
                                    .is_some_and(|(from, _)| from == &column.name)
                            {
                                paint_cell_background(ui, table_clip, pending_fill(ui));
                                let editor_id = ui.id().with("column-name-editor");
                                cancel_rename |= ui.input_mut(|i| {
                                    i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                                });
                                let (_, to) = self.rename.as_mut().unwrap();
                                let response = ui.add(
                                    TextEdit::singleline(to)
                                        .id(editor_id)
                                        .font(egui::TextStyle::Monospace)
                                        .frame(false)
                                        .desired_width(f32::INFINITY),
                                );
                                if self.rename_focus {
                                    response.request_focus();
                                    self.rename_focus = false;
                                } else {
                                    finish_rename |= response.lost_focus();
                                }
                                return;
                            }
                            if pending_name.is_some() {
                                paint_cell_background(ui, table_clip, pending_fill(ui));
                            }
                            let mut text =
                                RichText::new(pending_name.unwrap_or(&column.name)).monospace();
                            if pending_name.is_some() {
                                text = text.color(Color32::from_rgb(210, 150, 50));
                            }
                            ui.label(text);
                        });
                        if response.double_clicked()
                            && schema_editable
                            && !self.safe
                            && self.pending.is_none()
                        {
                            start_rename = Some((
                                column.name.clone(),
                                self.changes
                                    .iter()
                                    .find_map(|change| match change {
                                        Change::RenameColumn { table, from, to }
                                            if table == &self.browse.table
                                                && from == &column.name =>
                                        {
                                            Some(to.clone())
                                        }
                                        _ => None,
                                    })
                                    .unwrap_or_else(|| column.name.clone()),
                            ));
                        }
                        row.col(|ui| {
                            let mut text = RichText::new(&preview.data_type).monospace();
                            if changed && preview.data_type != column.data_type {
                                text = text.color(Color32::from_rgb(210, 150, 50));
                            }
                            ui.label(text);
                        });
                        row.col(|ui| {
                            if preview.not_null != column.not_null {
                                paint_cell_background(ui, table_clip, pending_fill(ui));
                            }
                            let mut required = preview.not_null || preview.pk;
                            ui.add_enabled_ui(
                                schema_editable
                                    && !self.safe
                                    && self.pending.is_none()
                                    && !column.pk
                                    && !column.generated,
                                |ui| {
                                    ui.spacing_mut().button_padding = egui::vec2(2.0, 1.0);
                                    ui.spacing_mut().interact_size.y = 20.0;
                                    let widgets = &mut ui.visuals_mut().widgets;
                                    for visuals in [
                                        &mut widgets.inactive,
                                        &mut widgets.hovered,
                                        &mut widgets.active,
                                    ] {
                                        visuals.bg_fill = Color32::TRANSPARENT;
                                        visuals.weak_bg_fill = Color32::TRANSPARENT;
                                        visuals.bg_stroke = egui::Stroke::NONE;
                                    }
                                    egui::ComboBox::from_id_salt(("column-nullable", index))
                                        .selected_text(if required { "NO" } else { "YES" })
                                        .width(ui.available_width())
                                        .icon(|ui, rect, visuals, _| {
                                            paint_arrow(ui, rect, true, visuals.fg_stroke.color)
                                        })
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(&mut required, false, "YES");
                                            ui.selectable_value(&mut required, true, "NO");
                                        });
                                },
                            );
                            if required != (preview.not_null || preview.pk) {
                                set_nullability = Some((index, required));
                            }
                        });
                        row.col(|ui| {
                            let mut text = RichText::new(&preview.default).monospace();
                            if preview.default != column.default {
                                text = text.color(Color32::from_rgb(210, 150, 50));
                            }
                            ui.label(text);
                        });
                        row.col(|ui| {
                            ui.label(if column.pk {
                                "PRIMARY KEY"
                            } else if column.generated {
                                "GENERATED"
                            } else {
                                ""
                            });
                        });
                        if primary_pressed(&row.response()) {
                            self.selected_column = Some(index);
                        }
                    });
                });
        });
        if cancel_rename {
            self.rename = None;
            self.rename_inline = false;
        } else if finish_rename && !self.finish_inline_rename() {
            return;
        }
        if let Some((index, required)) = set_nullability
            && self.finish_inline_rename()
        {
            self.stage_column_nullability(index, required);
        }
        if let Some(draft) = start_rename
            && self.finish_inline_rename()
        {
            self.rename = Some(draft);
            self.rename_inline = true;
            self.rename_focus = true;
        }
    }

    fn stage_column_nullability(&mut self, index: usize, required: bool) {
        if self.safe || self.pending.is_some() || !self.supports_schema_changes() {
            return;
        }
        let Some(column) = self.columns.get(index) else {
            return;
        };
        if column.pk || column.generated {
            return;
        }
        let name = column.name.clone();
        let not_null = (required != column.not_null).then_some(required);
        let mut data_type = None;
        let mut default = None;
        self.changes.retain(|change| {
            if let Change::AlterColumn {
                table,
                column,
                data_type: old_type,
                default: old_default,
                ..
            } = change
                && table == &self.browse.table
                && column == &name
            {
                data_type = old_type.clone();
                default = old_default.clone();
                false
            } else {
                true
            }
        });
        if data_type.is_some() || not_null.is_some() || default.is_some() {
            self.changes.push(Change::AlterColumn {
                table: self.browse.table.clone(),
                column: name,
                data_type,
                not_null,
                default,
            });
        }
        self.record_changes();
    }

    fn finish_inline_rename(&mut self) -> bool {
        if !self.rename_inline {
            return true;
        }
        let Some((from, to)) = self.rename.as_ref() else {
            self.rename_inline = false;
            return true;
        };
        if to.trim().is_empty() || to.contains('\0') {
            self.error = Some("Column name cannot be empty or contain a null character.".into());
            self.rename_focus = true;
            return false;
        }
        if self.safe || self.pending.is_some() || !self.supports_schema_changes() {
            self.error = Some("Column rename cannot be staged while editing is disabled.".into());
            return false;
        }
        let (from, to) = (from.clone(), to.clone());
        self.changes.retain(|c| !matches!(c, Change::RenameColumn {table, from: old, ..} if table == &self.browse.table && old == &from));
        if from != to {
            self.changes.push(Change::RenameColumn {
                table: self.browse.table.clone(),
                from,
                to,
            });
        }
        self.record_changes();
        self.review = None;
        self.rename = None;
        self.rename_inline = false;
        true
    }

    fn finish_cell_edit(&mut self) -> bool {
        if !self.finish_inline_rename() {
            return false;
        }
        let Some(mut edit) = self.edit.take() else {
            return true;
        };
        if !edit.bulk_rows.is_empty() {
            return self.finish_bulk_edit(edit);
        }
        let value = parse_cell_value(&edit);
        match value {
            Ok(value) => {
                if edit.insert_id.is_some() {
                    self.finish_insert_edit(edit, value);
                    return true;
                }
                let navigation = (edit.advance != 0).then(|| edit.clone());
                stage_cell_value(&mut self.changes, edit, value);
                self.record_changes();
                if let Some(source) = navigation {
                    self.edit = next_cell_edit(
                        &source,
                        &self.table,
                        &self.columns,
                        &self.changes,
                        &self.field_search,
                    );
                    if let Some(next) = &self.edit {
                        self.selected = self.table.rows.iter().position(|row| *row == next.row);
                        if next.inspector {
                            self.row_selection.clear();
                        } else if let Some(row) = self.selected {
                            self.row_selection.click(row, false, false, self.selected);
                            self.row_selection.cell_column = self
                                .table
                                .columns
                                .iter()
                                .position(|column| *column == next.column)
                                .unwrap_or(0);
                            self.row_selection.pending_cell_scroll = true;
                        }
                    } else if source.advance > 0
                        && !source.inspector
                        && source.table == self.browse.table
                    {
                        self.focus_first_insert();
                    }
                }
                self.review = None;
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

    fn discard_changes(&mut self) {
        // A running apply owns its plan; never hide changes while it can still commit.
        if self.pending.is_some() {
            return;
        }
        self.edit = None;
        self.changes.clear();
        self.record_changes();
        self.review = None;
        self.rename = None;
        self.add_column = None;
        self.alter_column = None;
        self.index_draft = None;
        self.status = "Changes discarded".into();
    }

    fn preview_changes(&mut self) {
        if self.pending.is_some() || !self.finish_cell_edit() || self.changes.is_empty() {
            return;
        }
        // Preview is idempotent: pressing its shortcut again must never apply a plan.
        if self.review.is_none() {
            self.dispatch(Command::Plan(self.changes.clone()), Pending::Plan(false));
        }
    }

    fn save_changes(&mut self, bypass: bool) {
        if self.pending.is_some() {
            return;
        }
        if !self.finish_cell_edit() {
            return;
        }
        if self.pending.is_some() || self.changes.is_empty() {
            return;
        }
        if self.safe {
            self.error = Some("Turn off Safe mode before saving changes.".into());
            return;
        }
        if let Some(plan) = self.review.clone() {
            self.dispatch(Command::Apply(plan), Pending::Apply);
        } else {
            self.dispatch(Command::Plan(self.changes.clone()), Pending::Plan(bypass));
        }
    }

    fn windows(&mut self, ctx: &egui::Context) {
        let schema_editable = self.supports_schema_changes();
        if let Some(mut draft) = self.index_draft.take() {
            let mut open = true;
            let mut stage = false;
            egui::Window::new("Create index")
                .id(egui::Id::new((ctx.viewport_id(), "Create index")))
                .open(&mut open)
                .collapsible(false)
                .default_width(460.0)
                .show(ctx, |ui| {
                    ui.strong(&draft.table);
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut draft.name);
                    });
                    ui.checkbox(&mut draft.unique, "Unique · reject duplicate key values");
                    ui.label("Index columns, in order");
                    let mut remove = None;
                    let mut movement = None;
                    let key_count = draft.keys.len();
                    for (i, (name, descending)) in draft.keys.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt(("index-key", i))
                                .selected_text(name.as_str())
                                .show_ui(ui, |ui| {
                                    for column in &draft.available_columns {
                                        ui.selectable_value(name, column.clone(), column);
                                    }
                                });
                            ui.checkbox(descending, "Descending")
                                .on_hover_text("Descending order");
                            if ui
                                .add_enabled_ui(i > 0, |ui| {
                                    arrow_button(ui, false, "Move column earlier")
                                })
                                .inner
                                .on_hover_text("Move column earlier")
                                .clicked()
                            {
                                movement = Some((i, i - 1));
                            }
                            if ui
                                .add_enabled_ui(i + 1 < key_count, |ui| {
                                    arrow_button(ui, true, "Move column later")
                                })
                                .inner
                                .on_hover_text("Move column later")
                                .clicked()
                            {
                                movement = Some((i, i + 1));
                            }
                            if ui
                                .small_button("×")
                                .on_hover_text("Remove column from index")
                                .clicked()
                            {
                                remove = Some(i);
                            }
                        });
                    }
                    if let Some(i) = remove {
                        draft.keys.remove(i);
                    }
                    if let Some((a, b)) = movement {
                        draft.keys.swap(a, b);
                    }
                    let next = draft
                        .available_columns
                        .iter()
                        .find(|c| !draft.keys.iter().any(|(key, _)| key == *c))
                        .cloned();
                    if ui
                        .add_enabled(next.is_some(), egui::Button::new("+ Column"))
                        .clicked()
                    {
                        draft.keys.push((next.unwrap(), false));
                    }
                    stage = ui
                        .add_enabled(
                            schema_editable
                                && !self.safe
                                && self.pending.is_none()
                                && !draft.name.trim().is_empty()
                                && !draft.keys.is_empty(),
                            egui::Button::new("Stage index"),
                        )
                        .clicked();
                });
            if stage {
                self.changes.push(Change::CreateIndex {
                    table: draft.table,
                    name: draft.name,
                    columns: draft.keys,
                    unique: draft.unique,
                });
                self.record_changes();
                self.review = None;
            } else if open {
                self.index_draft = Some(draft);
            }
        }
        if let Some(mut draft) = self.alter_column.take() {
            let mut open = true;
            let mut stage = false;
            egui::Window::new("Edit column properties").id(egui::Id::new((ctx.viewport_id(), "Edit column properties"))).open(&mut open).collapsible(false).show(ctx,|ui| {
                ui.strong(format!("{}.{}",draft.table,draft.column));
                ui.label(if self.engine == "SQLite" { "SQLite will rebuild this table and validate existing rows. Review the migration with Cmd+S." } else { "Review ALTER TABLE statements with Cmd+S. Existing rows must satisfy the new type and constraints." });
                egui::Grid::new("alter-column-fields").num_columns(2).show(ui,|ui| {
                    ui.label("Data type"); ui.text_edit_singleline(&mut draft.data_type); ui.end_row();
                    ui.label("Required"); ui.checkbox(&mut draft.not_null,"NOT NULL"); ui.end_row();
                    ui.label("Default");
                    egui::ComboBox::from_id_salt("alter-default-kind").selected_text(["Keep existing","Remove default","NULL","Integer","Real","Text"][draft.default_kind]).show_ui(ui,|ui| {
                        for (i,label) in ["Keep existing","Remove default","NULL","Integer","Real","Text"].iter().enumerate() { ui.selectable_value(&mut draft.default_kind,i,*label); }
                    }); ui.end_row();
                    if draft.default_kind >= 3 { ui.label("Default value"); ui.text_edit_singleline(&mut draft.default_text); ui.end_row(); }
                });
                if let Some(error) = &draft.error { ui.colored_label(Color32::from_rgb(220,100,90),error); }
                stage = ui.add_enabled(schema_editable && !self.safe && self.pending.is_none(),egui::Button::new("Stage column changes")).clicked();
            });
            if stage {
                match draft.change() {
                    Ok(change) => {
                        self.changes.retain(|c|!matches!(c,Change::AlterColumn {table,column,..} if table == &draft.table && column == &draft.column));
                        if !matches!(
                            &change,
                            Change::AlterColumn {
                                data_type: None,
                                not_null: None,
                                default: None,
                                ..
                            }
                        ) {
                            self.changes.push(change);
                        }
                        self.record_changes();
                        self.review = None;
                    }
                    Err(error) => {
                        draft.error = Some(error);
                        self.alter_column = Some(draft);
                    }
                }
            } else if open {
                self.alter_column = Some(draft);
            }
        }
        if let Some(mut draft) = self.add_column.take() {
            let mut open = true;
            let mut stage = false;
            egui::Window::new("Add column")
                .id(egui::Id::new((ctx.viewport_id(), "Add column")))
                .open(&mut open)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(format!("New column in {}", draft.table));
                    egui::Grid::new("new-column-fields")
                        .num_columns(2)
                        .show(ui, |ui| {
                            ui.label("Name");
                            ui.text_edit_singleline(&mut draft.name);
                            ui.end_row();
                            ui.label("Data type");
                            ui.text_edit_singleline(&mut draft.data_type);
                            ui.end_row();
                            ui.label("Required");
                            ui.checkbox(&mut draft.not_null, "NOT NULL");
                            ui.end_row();
                            ui.label("Default");
                            egui::ComboBox::from_id_salt("column-default-kind")
                                .selected_text(
                                    ["No default", "NULL", "Integer", "Real", "Text"]
                                        [draft.default_kind],
                                )
                                .show_ui(ui, |ui| {
                                    for (index, label) in
                                        ["No default", "NULL", "Integer", "Real", "Text"]
                                            .iter()
                                            .enumerate()
                                    {
                                        ui.selectable_value(&mut draft.default_kind, index, *label);
                                    }
                                });
                            ui.end_row();
                            if draft.default_kind >= 2 {
                                ui.label("Default value");
                                ui.text_edit_singleline(&mut draft.default_text);
                                ui.end_row();
                            }
                        });
                    if draft.not_null {
                        ui.label("Existing rows require a non-NULL constant default.");
                    }
                    if let Some(error) = &draft.error {
                        ui.colored_label(Color32::from_rgb(220, 100, 90), error);
                    }
                    stage = ui
                        .add_enabled(
                            schema_editable && !self.safe && self.pending.is_none(),
                            egui::Button::new("Stage column"),
                        )
                        .clicked();
                });
            if stage {
                match draft.definition() {
                    Ok(definition) => {
                        self.changes.push(Change::AddColumn {
                            table: draft.table,
                            definition,
                        });
                        self.record_changes();
                        self.review = None;
                    }
                    Err(error) => {
                        draft.error = Some(error);
                        self.add_column = Some(draft);
                    }
                }
            } else if open {
                self.add_column = Some(draft);
            }
        }
        if let Some(plan) = &self.review {
            let schema_plan = self.engine == "PostgreSQL"
                && plan
                    .statements
                    .iter()
                    .all(|statement| statement.expected_rows.is_none());
            let mut open = true;
            let mut apply = false;
            let mut edit_changes = false;
            egui::Window::new("Review changes").id(egui::Id::new((ctx.viewport_id(), "Review changes")))
                .open(&mut open)
                .default_width(720.0)
                .show(ctx, |ui| {
                    if let Some(error) = &self.error {
                        ui.colored_label(Color32::from_rgb(230, 105, 105), error);
                        ui.label("Pending edits are retained. Return to the table to correct them, then review again.");
                    }
                    ui.label("These changes will run in one transaction. Values are shown inline.");
                    if !plan.preconditions.is_empty() {
                        ui.label(if schema_plan { "Table and type definitions are checked again inside the transaction. Changed definitions cancel the save." } else { "Rows are verified immediately before each change, inside the same transaction. Concurrent edits cancel the save." });
                        ui.collapsing(format!("{} ({})", if schema_plan { "Schema checks" } else { "Concurrency checks" }, plan.preconditions.len()), |ui| {
                            egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                                for check in &plan.preconditions {
                                    ui.weak(format!("Before statement {} · {}", check.before_statement + 1, if schema_plan { "must match the reviewed definition" } else { "must match exactly one row" }));
                                    let sql = crate::review_sql::readable(&check.sql, &check.parameters);
                                    if clipboard_button(ui).on_hover_text("Copy check SQL").clicked() { ui.ctx().copy_text(sql.clone()); }
                                    ui.add(egui::Label::new(crate::editor::layout(ui, &sql.as_str(), ui.available_width())).selectable(true));
                                }
                            });
                        });
                    }
                    if plan.rebuild { ui.label("Migration: temporarily disable foreign-key enforcement before BEGIN; check all foreign keys before COMMIT; restore enforcement on success or rollback. Table copying may take time."); }
                    egui::ScrollArea::both().max_height(430.0).show(ui, |ui| {
                        for (index, sql) in self.review_sql.iter().enumerate() {
                            ui.push_id(("review-sql", index), |ui| {
                                if clipboard_button(ui).on_hover_text("Copy SQL").clicked() { ui.ctx().copy_text(sql.clone()); }
                                ui.add(egui::Label::new(crate::editor::layout(ui, &sql.as_str(), ui.available_width())).selectable(true));
                            });
                            ui.separator();
                        }
                    });
                    apply = ui
                        .add_enabled(
                            matches!(self.engine.as_str(), "SQLite" | "PostgreSQL") && !self.safe && self.pending.is_none(),
                            egui::Button::new("Save changes · ⌘S / ⌘Enter"),
                        )
                        .clicked();
                    if matches!(self.pending, Some(Pending::Apply))
                        && ui.button("Cancel save").clicked() {
                        self.worker.cancel();
                    }
                    edit_changes = ui.add_enabled(self.pending.is_none(), egui::Button::new("Back to editing")).clicked();
                });
            if apply {
                self.save_changes(false);
            }
            if !open || edit_changes {
                self.review = None;
            }
        }
        if !self.rename_inline
            && let Some((from, mut to)) = self.rename.take()
        {
            let mut open = true;
            let mut stage = false;
            egui::Window::new("Rename column")
                .id(egui::Id::new((ctx.viewport_id(), "Rename column")))
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label(format!("Rename {}.{}", self.browse.table, from));
                    ui.text_edit_singleline(&mut to);
                    stage = ui
                        .add_enabled(
                            schema_editable
                                && !self.safe
                                && self.pending.is_none()
                                && !to.trim().is_empty()
                                && to != from,
                            egui::Button::new("Stage rename"),
                        )
                        .clicked();
                });
            if stage {
                self.changes.retain(|c| !matches!(c, Change::RenameColumn { table, from: old, .. } if table == &self.browse.table && old == &from));
                self.changes.push(Change::RenameColumn {
                    table: self.browse.table.clone(),
                    from,
                    to,
                });
                self.record_changes();
                self.review = None;
            } else if open {
                self.rename = Some((from, to));
            }
        }
        let mut history = self.history;
        egui::Window::new("Query history & recovered queries")
            .id(egui::Id::new((
                ctx.viewport_id(),
                "Query history & recovered queries",
            )))
            .open(&mut history)
            .default_width(600.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(450.0)
                    .show(ui, |ui| {
                        for (title, entries) in [
                            ("Recovered unsaved queries", &self.prefs.favorites),
                            ("Recent queries", &self.prefs.history),
                        ] {
                            ui.heading(title);
                            for sql in entries {
                                ui.horizontal(|ui| {
                                    if ui.button("Open").clicked() {
                                        self.tabs[self.tab].sql = sql.clone();
                                        self.tabs[self.tab].refresh_dirty();
                                        self.tabs[self.tab].open = true;
                                        self.tabs[self.tab].focus_editor = true;
                                        self.view = View::Query;
                                    }
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(
                                                sql.lines()
                                                    .filter(|l| !l.trim().starts_with("--"))
                                                    .collect::<Vec<_>>()
                                                    .join(" "),
                                            )
                                            .monospace(),
                                        )
                                        .truncate(),
                                    );
                                });
                            }
                            ui.add_space(16.0);
                        }
                    });
            });
        self.history = history;
        self.connections_window(ctx);
    }
}

impl eframe::App for Rustable {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let start = Instant::now();
        self.poll();
        self.workspace_shortcuts(ctx);
        self.toolbar(ctx);
        if cfg!(feature = "qa") { self.footer(ctx); }
        self.sidebar(ctx);
        self.inspector(ctx);
        self.workspace(ctx);
        self.windows(ctx);
        self.navigator_window(ctx);
        self.csv_preview_window(ctx);
        self.csv_import_review(ctx);
        #[cfg(target_arch = "wasm32")]
        self.browser_query_save_window(ctx);
        if ctx.input(|i| i.viewport().close_requested())
            && (!self.changes.is_empty()
                || self.edit.is_some()
                || (self.rename_inline && self.rename.is_some())
                || self.pending.is_some())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.error = Some(
                "Save or discard pending changes and wait for database work before closing.".into(),
            );
        }
        if self.pending.is_some() {
            // The spinner needs animation only while a task is active.
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
        self.frame_ms = start.elapsed().as_secs_f32() * 1000.0;
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.save_with_window_documents(storage, Vec::new());
    }
}

fn apply_theme(ctx: &egui::Context, appearance: Appearance) {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let dark = theme == egui::Theme::Dark;
        let mut visuals = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.selection.bg_fill = if dark {
            Color32::from_rgb(24, 91, 186)
        } else {
            Color32::from_rgb(225, 235, 255)
        };
        visuals.selection.stroke = Stroke::new(1.0_f32, BLUE);
        if dark {
            visuals.panel_fill = Color32::from_rgb(29, 29, 29);
            visuals.window_fill = Color32::from_rgb(38, 38, 38);
            visuals.extreme_bg_color = Color32::from_rgb(32, 32, 32);
            visuals.faint_bg_color = Color32::from_rgb(44, 44, 44);
            visuals.override_text_color = Some(Color32::from_rgb(214, 214, 214));
            visuals.hyperlink_color = Color32::from_rgb(127, 177, 255);
            visuals.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(220, 233, 255));
            visuals.widgets.noninteractive.bg_stroke =
                Stroke::new(1.0_f32, Color32::from_rgb(57, 57, 57));
        } else {
            visuals.hyperlink_color = BLUE;
            visuals.panel_fill = Color32::from_rgb(248, 250, 253);
            visuals.window_fill = Color32::WHITE;
            visuals.extreme_bg_color = Color32::WHITE;
            visuals.faint_bg_color = Color32::from_rgb(241, 245, 250);
            visuals.override_text_color = Some(Color32::from_rgb(47, 59, 77));
            visuals.widgets.noninteractive.bg_stroke =
                Stroke::new(1.0_f32, Color32::from_rgb(225, 231, 240));
        }
        let mut style = (*ctx.style_of(theme)).clone();
        style.visuals = visuals;
        {
            style.spacing.item_spacing = egui::vec2(6.0, 3.0);
            style.spacing.button_padding = egui::vec2(8.0, 4.0);
            style.spacing.interact_size.y = 23.0;
            style.animation_time = 0.12;
        }
        ctx.set_style_of(theme, style);
    }
    ctx.set_theme(appearance.preference());
}

fn toolbar_action(ui: &mut egui::Ui, kind: u8, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(34.0, 26.0), egui::Sense::click());
    let visuals = ui.style().interact(&response);
    ui.painter().rect_filled(
        rect,
        12.0,
        if selected {
            ui.visuals().widgets.active.bg_fill
        } else {
            visuals.weak_bg_fill
        },
    );
    let c = rect.center();
    let stroke = Stroke::new(1.2_f32, visuals.fg_stroke.color);
    let line = |a: [f32; 2], b: [f32; 2]| {
        ui.painter().line_segment(
            [c + egui::vec2(a[0], a[1]), c + egui::vec2(b[0], b[1])],
            stroke,
        );
    };
    match kind {
        0 => {
            ui.painter().rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(16.0, 14.0)),
                2.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            line([-3.0, -6.0], [-3.0, 6.0]);
        }
        1 => {
            for y in [-5.0, 5.0] {
                ui.painter().add(egui::Shape::ellipse_stroke(c + egui::vec2(0.0, y), egui::vec2(7.0, 3.0), stroke));
            }
            line([-7.0, -5.0], [-7.0, 5.0]);
            line([7.0, -5.0], [7.0, 5.0]);
        }
        2 => {
            ui.painter().text(
                c,
                egui::Align2::CENTER_CENTER,
                "SQL",
                egui::FontId::proportional(10.0),
                stroke.color,
            );
        }
        _ => {
            for y in [-5.0, 0.0, 5.0] {
                line([-7.0, y], [7.0, y]);
            }
        }
    }
    response
}

fn apply_formatted_sql(tab: &mut QueryTab, formatted: crate::sql_format::Formatted, state: &mut egui::text_edit::TextEditState) {
    if tab.sql != formatted.original || tab.sql == formatted.text { return; }
    let mut undoer = state.undoer();
    let before = egui::text::CCursorRange::two(egui::text::CCursor::new(tab.cursor.start), egui::text::CCursor::new(tab.cursor.end));
    undoer.add_undo(&(before, tab.sql.clone()));
    tab.sql = formatted.text;
    tab.cursor = formatted.selection;
    let after = egui::text::CCursorRange::two(egui::text::CCursor::new(tab.cursor.start), egui::text::CCursor::new(tab.cursor.end));
    state.cursor.set_char_range(Some(after));
    undoer.add_undo(&(after, tab.sql.clone()));
    state.set_undoer(undoer);
    tab.completion.dismiss();
    tab.refresh_dirty();
    tab.focus_editor = true;
}

fn accept_completion(tab: &mut QueryTab, value: &str, state: &mut egui::text_edit::TextEditState) {
    let mut undoer = state.undoer();
    let before = egui::text::CCursorRange::two(
        egui::text::CCursor::new(tab.cursor.start),
        egui::text::CCursor::new(tab.cursor.end),
    );
    undoer.add_undo(&(before, tab.sql.clone()));
    let caret = tab.completion.apply(&mut tab.sql, value);
    tab.refresh_dirty();
    tab.cursor = caret..caret;
    let after = egui::text::CCursorRange::one(egui::text::CCursor::new(caret));
    state.cursor.set_char_range(Some(after));
    undoer.add_undo(&(after, tab.sql.clone()));
    state.set_undoer(undoer);
}

fn inspector_search(ui: &mut egui::Ui, text: &mut String) {
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(14.0, 18.0), egui::Sense::hover());
                let center = rect.center() - egui::vec2(1.5, 1.5);
                let stroke = Stroke::new(1.3_f32, ui.visuals().weak_text_color());
                ui.painter().circle_stroke(center, 4.5, stroke);
                ui.painter().line_segment(
                    [center + egui::vec2(3.2, 3.2), center + egui::vec2(6.5, 6.5)],
                    stroke,
                );
                ui.add(
                    TextEdit::singleline(text)
                        .frame(false)
                        .hint_text("Search for field…")
                        .desired_width(f32::INFINITY),
                );
            });
        });
}

fn grouped_row_count(value: u64) -> String {
    let digits = value.to_string();
    let mut result = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(digit);
    }
    result
}

fn pagination_label(offset: usize, grid: &Grid) -> String {
    let end = offset.saturating_add(grid.rows.len()) as u64;
    let range = if grid.rows.is_empty() {
        "0".to_owned()
    } else {
        format!(
            "{}–{}",
            grouped_row_count(offset.saturating_add(1) as u64),
            grouped_row_count(end)
        )
    };
    match grid.total_rows {
        Some(total) => format!(
            "{range} of {}{} rows",
            if total.estimated { "~" } else { "" },
            grouped_row_count(total.rows)
        ),
        None if grid.truncated || offset > 0 => format!("{range} of … rows"),
        None => format!("{range} rows"),
    }
}

fn table_status_label(offset: usize, grid: &Grid, selected: usize) -> String {
    if selected == 0 {
        return pagination_label(offset, grid);
    }
    if let Some(total) = grid.total_rows {
        format!(
            "{} of {}{} rows selected",
            grouped_row_count(selected as u64),
            if total.estimated { "~" } else { "" },
            grouped_row_count(total.rows)
        )
    } else {
        format!("{selected} of {} loaded rows selected", grid.rows.len())
    }
}

#[derive(Debug)]
struct TableControlGeometry {
    height: f32,
    left: egui::Rect,
    center: egui::Rect,
    right: egui::Rect,
}

fn table_control_geometry(
    ui: &egui::Ui,
    view: View,
    pending: usize,
    page_range: &str,
) -> TableControlGeometry {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let measure = |text: &str| {
        ui.painter()
            .layout_no_wrap(text.into(), font.clone(), Color32::WHITE)
            .size()
            .x
    };
    let gap = 6.0;
    let right = if view == View::Data {
        76.0 + 56.0 + measure("Export") + 14.0 + gap * 3.0
    } else {
        measure("Export") + 14.0
    };
    let left = 248.0
        + measure("↻")
        + 12.0
        + gap
        + if view == View::Data {
            measure("+ Row") + measure("Rows") + 24.0 + gap * 2.0
        } else if view == View::Structure {
            measure("+ Column") + 12.0 + gap
        } else {
            0.0
        }
        + if pending > 0 {
            measure(&format!("Pending ({pending})")) + 24.0 + gap * 2.0
        } else {
            0.0
        };
    let center = if view == View::Data {
        ui.painter()
            .layout_no_wrap(
                page_range.into(),
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            )
            .size()
            .x
            + 8.0
    } else {
        0.0
    };
    let width = (ui.available_width() - 16.0).max(1.0);
    let rect = |x, y, w, h| egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
    let fits_one_line = if center == 0.0 {
        left + right + gap <= width
    } else {
        2.0 * left.max(right) + center + gap * 2.0 <= width
    };
    if fits_one_line {
        TableControlGeometry {
            height: 44.0,
            left: rect(0.0, 0.0, left, 28.0),
            center: rect((width - center) * 0.5, 0.0, center, 28.0),
            right: rect(width - right, 0.0, right, 28.0),
        }
    } else if left + center + right + gap * 2.0 <= width {
        // Keep the compact single row when only perfect geometric centering
        // conflicts with the wider action group. Center within the free space.
        let center_x = (left + gap + width - right - gap - center) * 0.5;
        TableControlGeometry {
            height: 44.0,
            left: rect(0.0, 0.0, left, 28.0),
            center: rect(center_x, 0.0, center, 28.0),
            right: rect(width - right, 0.0, right, 28.0),
        }
    } else {
        let left_rows = (left / width).ceil().max(1.0);
        let last_y = left_rows * 34.0;
        let same_line = center + right * 2.0 + gap * 2.0 <= width;
        TableControlGeometry {
            height: 44.0 + last_y + if same_line { 0.0 } else { 34.0 },
            left: rect(0.0, 0.0, width, left_rows * 34.0 - 6.0),
            center: rect(
                (width - center.min(width)) * 0.5,
                last_y,
                center.min(width),
                28.0,
            ),
            right: rect(
                (width - right).max(0.0),
                last_y + if same_line { 0.0 } else { 34.0 },
                right.min(width),
                28.0,
            ),
        }
    }
}

fn query_result_control_geometry(width: f32) -> TableControlGeometry {
    let width = width.max(1.0);
    let rect = |x, y, w| egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, 28.0));
    // Keep the count truly centered; wrap it below the actions on narrow panes.
    if width >= 780.0 {
        TableControlGeometry { height: 44.0, left: rect(0.0, 0.0, 310.0), center: rect((width - 140.0) * 0.5, 0.0, 140.0), right: rect(width - 164.0, 0.0, 164.0) }
    } else if width >= 500.0 {
        TableControlGeometry { height: 78.0, left: rect(0.0, 0.0, width - 170.0), center: rect(0.0, 34.0, width), right: rect(width - 164.0, 0.0, 164.0) }
    } else {
        TableControlGeometry { height: 112.0, left: rect(0.0, 0.0, width), center: rect(0.0, 68.0, width), right: rect(0.0, 34.0, width) }
    }
}

fn query_result_view_switch(ui: &mut egui::Ui, current: &mut ResultView) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(210.0, 28.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 6.0, ui.visuals().widgets.inactive.weak_bg_fill);
    for (index, title) in ["Data", "Message", "Chart"].iter().enumerate() {
        let segment = egui::Rect::from_min_size(rect.min + egui::vec2(index as f32 * 70.0, 0.0), egui::vec2(70.0, 28.0));
        if ui.interact(segment, ui.id().with(("query-result-view", index)), egui::Sense::click()).clicked() {
            *current = [ResultView::Data, ResultView::Message, ResultView::Chart][index];
        }
        let selected = *current == [ResultView::Data, ResultView::Message, ResultView::Chart][index];
        if selected {
            ui.painter().rect_filled(segment.shrink(1.0), 5.0, ui.visuals().selection.bg_fill);
        }
        ui.painter().text(segment.center(), egui::Align2::CENTER_CENTER, *title, egui::FontId::proportional(13.0),
            if selected { ui.visuals().selection.stroke.color } else { ui.visuals().text_color() });
    }
}

fn table_view_switch(ui: &mut egui::Ui, current: &mut View) {
    let segments = [
        (View::Data, "Data", 52.0),
        (View::Structure, "Structure", 76.0),
        (View::Indexes, "Index", 54.0),
        (View::Triggers, "Triggers", 66.0),
    ];
    let (rect, _) = ui.allocate_exact_size(egui::vec2(248.0, 28.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 6.0, ui.visuals().widgets.inactive.weak_bg_fill);
    let mut x = rect.left();
    for (index, (view, title, width)) in segments.iter().enumerate() {
        let segment =
            egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(*width, rect.height()));
        let response = ui.interact(
            segment,
            ui.id().with(("table-view", index)),
            egui::Sense::click(),
        );
        if response.clicked() {
            *current = *view;
        }
        if *current == *view {
            ui.painter()
                .rect_filled(segment.shrink(1.0), 5.0, ui.visuals().selection.bg_fill);
        }
        if index > 0 && *current != *view && *current != segments[index - 1].0 {
            ui.painter().line_segment(
                [
                    egui::pos2(x, rect.top() + 6.0),
                    egui::pos2(x, rect.bottom() - 6.0),
                ],
                Stroke::new(1.0_f32, ui.visuals().weak_text_color().gamma_multiply(0.7)),
            );
        }
        let color = if *current == *view {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().text_color()
        };
        ui.painter().text(
            segment.center(),
            egui::Align2::CENTER_CENTER,
            *title,
            egui::FontId::proportional(13.0),
            color,
        );
        x += width;
    }
}

/// Match the custom pagination controls across every interaction state.
fn rounded_action_buttons(ui: &mut egui::Ui) {
    let widgets = &mut ui.visuals_mut().widgets;
    for widget in [
        &mut widgets.noninteractive,
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(5);
    }
}

fn filter_button(selected: bool) -> impl egui::Widget {
    move |ui: &mut egui::Ui| {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(76.0, 28.0), egui::Sense::click());
        let visuals = ui.style().interact_selectable(&response, selected);
        ui.painter().rect_filled(rect, 5.0, visuals.weak_bg_fill);
        let stroke = visuals.fg_stroke;
        for (y, x) in [(-5.0, -2.0), (0.0, 3.0), (5.0, -1.0)] {
            let center = egui::pos2(rect.left() + 14.0, rect.center().y + y);
            ui.painter().line_segment(
                [
                    center + egui::vec2(-6.0, 0.0),
                    center + egui::vec2(6.0, 0.0),
                ],
                stroke,
            );
            ui.painter()
                .circle_filled(center + egui::vec2(x, 0.0), 2.0, stroke.color);
        }
        ui.painter().text(
            egui::pos2(rect.left() + 28.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            "Filters",
            egui::FontId::proportional(12.0),
            stroke.color,
        );
        response.on_hover_text("Show or hide table filters · Cmd/Ctrl+F")
    }
}

fn result_pin_button(pinned: bool) -> impl egui::Widget {
    move |ui: &mut egui::Ui| {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
        let visuals = ui.style().interact_selectable(&response, pinned);
        ui.painter().rect_filled(rect, 5.0, visuals.weak_bg_fill);
        let center = rect.center();
        let stroke = Stroke::new(1.5_f32, visuals.fg_stroke.color);
        let point = |x: f32, y: f32| center + egui::vec2(x, y);
        ui.painter().add(egui::Shape::closed_line(vec![point(-4.0,-7.0),point(4.0,-7.0),point(3.0,-1.0),point(6.0,3.0),point(-6.0,3.0),point(-3.0,-1.0)], stroke));
        ui.painter().line_segment([point(0.0,3.0),point(0.0,9.0)], stroke);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), if pinned { "Unpin result" } else { "Pin result" }));
        response
    }
}

fn result_search_button() -> impl egui::Widget {
    |ui: &mut egui::Ui| {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
        let visuals = ui.style().interact(&response);
        ui.painter().rect_filled(rect, 5.0, visuals.weak_bg_fill);
        let center = rect.center() - egui::vec2(2.0, 2.0);
        let stroke = Stroke::new(1.5_f32, visuals.fg_stroke.color);
        ui.painter().circle_stroke(center, 5.5, stroke);
        ui.painter().line_segment([center + egui::vec2(4.0, 4.0), center + egui::vec2(9.0, 9.0)], stroke);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Find in loaded results"));
        response
    }
}

fn navigation_button(next: bool) -> impl egui::Widget {
    move |ui: &mut egui::Ui| {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
        let visuals = ui.style().interact(&response);
        ui.painter().rect_filled(rect, 5.0, visuals.weak_bg_fill);
        let center = rect.center();
        let direction = if next { 1.0 } else { -1.0 };
        ui.painter().add(egui::Shape::line(
            vec![
                center + egui::vec2(-3.0 * direction, -6.0),
                center + egui::vec2(3.0 * direction, 0.0),
                center + egui::vec2(-3.0 * direction, 6.0),
            ],
            Stroke::new(1.5_f32, visuals.fg_stroke.color),
        ));
        response.on_hover_text(if next { "Next page · Cmd/Ctrl+Right (table grid)" } else { "Previous page · Cmd/Ctrl+Left (table grid)" })
    }
}

fn workspace_tab(
    ui: &mut egui::Ui,
    title: &str,
    selected: bool,
    closable: bool,
) -> (egui::Response, bool) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(180.0, 27.0), egui::Sense::click());
    let visuals = ui.style().interact(&response);
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect.shrink(1.0),
            12.0,
            if selected {
                ui.visuals().widgets.active.bg_fill
            } else {
                visuals.weak_bg_fill
            },
        );
    }
    let text_rect = egui::Rect::from_min_max(
        rect.min + egui::vec2(10.0, 0.0),
        rect.max - egui::vec2(26.0, 0.0),
    );
    let mut job = egui::text::LayoutJob::simple_singleline(
        title.to_owned(),
        egui::FontId::proportional(12.0),
        ui.visuals().text_color(),
    );
    job.wrap.max_width = text_rect.width();
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let pos = text_rect.center() - galley.size() * 0.5;
    ui.painter()
        .with_clip_rect(text_rect.intersect(ui.clip_rect()))
        .galley(pos, galley, ui.visuals().text_color());
    let mut closed = false;
    if closable {
        let close_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 12.0, rect.center().y),
            egui::vec2(20.0, 22.0),
        );
        let close = ui.interact(close_rect, response.id.with("close"), egui::Sense::click());
        let center = close_rect.center();
        let stroke = Stroke::new(1.0_f32, ui.style().interact(&close).fg_stroke.color);
        ui.painter().line_segment(
            [
                center + egui::vec2(-3.0, -3.0),
                center + egui::vec2(3.0, 3.0),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                center + egui::vec2(-3.0, 3.0),
                center + egui::vec2(3.0, -3.0),
            ],
            stroke,
        );
        closed = close.clicked_by(egui::PointerButton::Middle)
            || close.on_hover_text("Close tab · ⌘W · Reopen ⌘⇧T").clicked();
    }
    (response.on_hover_text(title), closed)
}

fn field_value_menu(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(16.0, 22.0), egui::Sense::click());
    let visuals = ui.style().interact(&response);
    let center = rect.center();
    ui.painter().add(egui::Shape::line(
        vec![
            center + egui::vec2(-3.5, -1.5),
            center + egui::vec2(0.0, 2.0),
            center + egui::vec2(3.5, -1.5),
        ],
        Stroke::new(1.3_f32, visuals.fg_stroke.color),
    ));
    egui::Popup::menu(&response).show(contents);
    response
}

// Keep metadata fields compact regardless of viewport height or value length.
fn compact_property(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).small().color(MUTED));
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(4, 2))
        .show(ui, |ui| {
            let width = ui.available_width();
            ui.allocate_ui_with_layout(
                egui::vec2(width, 22.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if clipboard_button(ui).clicked() {
                        ui.ctx().copy_text(value.to_owned());
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(value).monospace()).truncate())
                            .on_hover_text(value);
                    });
                },
            );
        });
    ui.add_space(3.0);
}

// Query the press event, not persistent button-down state: command-click toggles
// once, and releasing or dragging a held button never applies selection again.
fn primary_pressed(response: &egui::Response) -> bool {
    response.enabled()
        && response.contains_pointer()
        && response
            .ctx
            .input(|input| input.pointer.button_pressed(egui::PointerButton::Primary))
}

// Full-height moving blocks are an indeterminate busy indicator. Clip their
// geometry to the connection pill itself, not just its rectangular bounds.
fn paint_connection_activity(
    painter: &egui::Painter,
    rect: egui::Rect,
    seconds: f32,
    ink: Color32,
) {
    for points in connection_activity_blocks(rect, seconds) {
        painter.add(egui::Shape::convex_polygon(
            points,
            ink.gamma_multiply(0.12),
            Stroke::NONE,
        ));
    }
}

fn connection_activity_blocks(rect: egui::Rect, seconds: f32) -> Vec<Vec<egui::Pos2>> {
    if !rect.is_positive() || !seconds.is_finite() {
        return Vec::new();
    }
    let radius = 13.0_f32.min(rect.height() / 2.0).min(rect.width() / 2.0);
    let gap = 2.0_f32.min(rect.height() / 8.0);
    let size = (rect.height() - gap * 3.0) / 4.0;
    let period = size + gap;
    let patch = (rect.width() * 0.38).max(period * 4.0).min(rect.width());
    // A moving patch of small squares indicates activity, not completion. Its
    // size never grows with elapsed time or guessed query progress.
    let cycle = rect.width() + patch;
    let offset = (seconds * 80.0).rem_euclid(cycle);
    let patch_left = rect.left() + offset - patch;
    let mut left = rect.left() + (offset % period) - period;
    let mut blocks = Vec::new();
    let edge = |x: f32| {
        let distance = (rect.left() + radius - x)
            .max(x - (rect.right() - radius))
            .max(0.0);
        radius - (radius * radius - distance * distance).max(0.0).sqrt()
    };
    while left < rect.right() {
        let start = left.max(rect.left()).max(patch_left);
        let end = (left + size).min(rect.right()).min(patch_left + patch);
        if end > start {
            // Conservative clipping keeps every corner inside the pill caps.
            let inset = edge(start).max(edge(end));
            for row in 0..4 {
                let top = (rect.top() + row as f32 * period).max(rect.top() + inset);
                let bottom = (rect.top() + row as f32 * period + size).min(rect.bottom() - inset);
                if bottom > top {
                    blocks.push(vec![
                        egui::pos2(start, top),
                        egui::pos2(end, top),
                        egui::pos2(end, bottom),
                        egui::pos2(start, bottom),
                    ]);
                }
            }
        }
        left += period;
    }
    blocks
}

fn clipboard_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Copy full value")
    });
    let now = ui.input(|i| i.time);
    let feedback_id = response.id.with("copied-until");
    if response.clicked() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(feedback_id, now + 1.1));
    }
    let until = ui
        .ctx()
        .data(|data| data.get_temp::<f64>(feedback_id))
        .unwrap_or(0.0);
    let copied = now < until;
    let stroke = ui.style().interact(&response).fg_stroke;
    if copied {
        let center = rect.center();
        ui.painter().add(egui::Shape::line(
            vec![
                center + egui::vec2(-5.0, 0.0),
                center + egui::vec2(-1.0, 4.0),
                center + egui::vec2(6.0, -4.0),
            ],
            egui::Stroke::new(1.8_f32, stroke.color),
        ));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(until - now));
    } else {
        let r = egui::Rect::from_center_size(rect.center(), egui::vec2(10.0, 12.0));
        ui.painter()
            .rect_stroke(r, 2.0, stroke, egui::StrokeKind::Inside);
        ui.painter().rect_filled(
            egui::Rect::from_center_size(r.center_top(), egui::vec2(6.0, 3.0)),
            1.0,
            stroke.color,
        );
    }
    response.on_hover_text(if copied { "Copied" } else { "Copy full value" })
}

fn column_preview(column: &Column, table: &str, changes: &[Change]) -> (Column, bool) {
    let mut preview = column.clone();
    let mut changed = false;
    for change in changes {
        match change {
            Change::RenameColumn {
                table: target,
                from,
                to,
            } if target == table && from == &column.name => {
                preview.name = to.clone();
                changed = true;
            }
            Change::AlterColumn {
                table: target,
                column: name,
                data_type,
                not_null,
                default,
            } if target == table && name == &column.name => {
                if let Some(value) = data_type {
                    preview.data_type = value.clone();
                }
                if let Some(value) = not_null {
                    preview.not_null = *value;
                }
                if let Some(value) = default {
                    preview.default = match value {
                        None => String::new(),
                        Some(Value::Text(text)) => format!("'{}'", text.replace('\'', "''")),
                        Some(value) => db::display(value),
                    };
                }
                changed = true;
            }
            _ => {}
        }
    }
    (preview, changed)
}

#[cfg(test)]
mod tests {
    #[test]
    fn staged_schema_preview_combines_changes_without_mutating_loaded_metadata() {
        let column = super::Column {
            name: "value".into(),
            data_type: "TEXT".into(),
            not_null: false,
            default: "'old'".into(),
            pk: false,
            generated: false,
        };
        let changes = vec![
            super::Change::RenameColumn {
                table: "t".into(),
                from: "value".into(),
                to: "renamed".into(),
            },
            super::Change::AlterColumn {
                table: "t".into(),
                column: "value".into(),
                data_type: Some("INTEGER".into()),
                not_null: Some(true),
                default: Some(None),
            },
        ];
        let (preview, changed) = super::column_preview(&column, "t", &changes);
        assert!(changed);
        assert_eq!(preview.name, "renamed");
        assert_eq!(preview.data_type, "INTEGER");
        assert!(preview.not_null);
        assert!(preview.default.is_empty());
        assert_eq!(column.name, "value");
        assert!(!super::column_preview(&column, "other", &changes).1);
    }

    use super::*;

    #[test]
    fn system_theme_tracks_os_and_override_stays_fixed() {
        let ctx = egui::Context::default();
        apply_theme(&ctx, Appearance::System);
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let _ = ctx.run(
                egui::RawInput {
                    system_theme: Some(theme),
                    ..Default::default()
                },
                |_| {},
            );
            assert_eq!(ctx.style().visuals.dark_mode, theme == egui::Theme::Dark);
        }
        ctx.set_theme(Appearance::Dark.preference());
        let _ = ctx.run(
            egui::RawInput {
                system_theme: Some(egui::Theme::Light),
                ..Default::default()
            },
            |_| {},
        );
        assert!(ctx.style().visuals.dark_mode);
        ctx.set_theme(Appearance::System.preference());
        let _ = ctx.run(
            egui::RawInput {
                system_theme: Some(egui::Theme::Light),
                ..Default::default()
            },
            |_| {},
        );
        assert!(!ctx.style().visuals.dark_mode);
    }

    #[test]
    fn editor_tabs_have_distinct_state_ids() {
        let a = QueryTab::new("SELECT 1".into());
        let b = QueryTab::new("SELECT 2".into());
        assert_ne!(a.id, b.id);
        assert_eq!(Appearance::default(), Appearance::System);
    }
}

fn bounded_log_text(text: &str) -> String {
    if text.len() <= 8192 {
        return text.to_owned();
    }
    let mut end = 8192;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [log truncated]", &text[..end])
}

fn activity_description(command: &Command) -> Option<String> {
    match command {
        Command::Query { sql, .. } => Some(bounded_log_text(sql)),
        Command::Apply(plan) => {
            let mut sql = String::from("BEGIN;\n");
            for statement in &plan.statements {
                sql.push_str(&crate::review_sql::readable(&statement.sql, &statement.parameters));
                sql.push('\n');
                if sql.len() > 8192 { break; }
            }
            // Logged on dispatch: these are submitted statements, not a claim of success.
            Some(bounded_log_text(&sql))
        }
        _ => None,
    }
}

// Keep empty and short values as easy to activate as a populated input field.
fn compact_inspector_spacing(ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing = egui::vec2(3.0, 1.0);
    ui.spacing_mut().interact_size.y = 14.0;
}

fn inspector_value_preview(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let label = if text.is_empty() {
        RichText::new("EMPTY")
            .monospace()
            .color(ui.visuals().weak_text_color())
    } else {
        RichText::new(text).monospace()
    };
    let size = egui::vec2(ui.available_width(), 22.0);
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(size);
            ui.add(
                egui::Label::new(label)
                    .truncate()
                    .halign(egui::Align::Min)
                    .selectable(false),
            );
        },
    )
    .response
    .interact(egui::Sense::click())
}

// Match egui_extras' gapless selection/stripe geometry, including inter-cell spacing.
/// Background only: blank space never acquires row IDs, values or click targets.
/// Keep visible grid scroll handles steady during row selection and keyboard
/// navigation. Restore the caller's style so inspectors/editors retain their own.
fn with_grid_scrollbars(ui: &mut egui::Ui, render: impl FnOnce(&mut egui::Ui)) {
    let previous = ui.spacing().scroll;
    let mut scroll = previous;
    scroll.floating_width = scroll.bar_width;
    scroll.dormant_handle_opacity = 0.7;
    scroll.active_handle_opacity = 0.7;
    scroll.interact_handle_opacity = 0.7;
    scroll.dormant_background_opacity = 0.0;
    scroll.active_background_opacity = 0.0;
    scroll.interact_background_opacity = 0.0;
    ui.spacing_mut().scroll = scroll;
    render(ui);
    ui.spacing_mut().scroll = previous;
}

fn paint_column_header(ui: &mut egui::Ui, name: &str, direction: Option<bool>) -> egui::Response {
    let rect = ui.max_rect();
    let response = ui.interact(rect, ui.id().with("sort-column"), egui::Sense::click());
    let color = ui.visuals().text_color();
    let galley = ui.painter().layout_no_wrap(
        name.to_owned(),
        egui::TextStyle::Button.resolve(ui.style()),
        color,
    );
    let arrow_width = if direction.is_some() { 16.0 } else { 0.0 };
    let width = galley.size().x + arrow_width;
    ui.allocate_exact_size(egui::vec2(width + 8.0, 20.0), egui::Sense::hover());
    let origin = egui::pos2(
        rect.center().x - width * 0.5,
        rect.center().y - galley.size().y * 0.5,
    );
    ui.painter()
        .with_clip_rect(rect)
        .galley(origin, galley.clone(), color);
    if let Some(descending) = direction {
        paint_arrow(
            ui,
            egui::Rect::from_center_size(
                egui::pos2(origin.x + galley.size().x + 10.0, rect.center().y),
                egui::vec2(12.0, 18.0),
            ),
            descending,
            color,
        );
    }
    response
}

fn content_column_widths(ui: &egui::Ui, grid: &Grid) -> Vec<f32> {
    // Bound first-render font measurement; cache until the result allocation changes.
    let sample_rows = (4096 / grid.columns.len().max(1)).clamp(1, 64);
    ui.fonts_mut(|fonts| {
        grid.columns
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let heading = fonts
                    .layout_no_wrap(
                        name.clone(),
                        egui::FontId::proportional(14.0),
                        Color32::WHITE,
                    )
                    .size()
                    .x
                    + 24.0;
                let content = grid
                    .rows
                    .iter()
                    .take(sample_rows)
                    .filter_map(|row| row.get(index))
                    .map(|value| {
                        fonts
                            .layout_no_wrap(
                                db::preview(value, 100).into_owned(),
                                egui::FontId::monospace(12.0),
                                Color32::WHITE,
                            )
                            .size()
                            .x
                            + 10.0
                    })
                    .fold(0.0f32, f32::max);
                heading
                    .max(content)
                    .clamp(
                        crate::column_widths::MIN_WIDTH,
                        crate::column_widths::DEFAULT_WIDTH,
                    )
                    .ceil()
            })
            .collect()
    })
}

fn paint_grid_backdrop(ui: &egui::Ui, widths: &[f32]) {
    let clip = ui.clip_rect();
    let spacing = ui.spacing().item_spacing;
    let origin = ui.cursor().min;
    let gutter_right = (origin.x + widths.first().copied().unwrap_or(0.0) + spacing.x * 0.5)
        .clamp(clip.left(), clip.right());
    ui.painter().rect_filled(
        egui::Rect::from_min_max(clip.min, egui::pos2(gutter_right, clip.bottom())),
        0.0,
        ui.visuals().panel_fill,
    );
    let pitch = 22.0 + spacing.y;
    let first = ((clip.top() - origin.y + spacing.y * 0.5) / pitch)
        .floor()
        .max(0.0) as usize;
    let last = ((clip.bottom() - origin.y + spacing.y * 0.5) / pitch)
        .ceil()
        .max(0.0) as usize;
    for index in first..last {
        if index.is_multiple_of(2) {
            let y = origin.y + index as f32 * pitch - spacing.y * 0.5;
            let rect = egui::Rect::from_min_max(
                egui::pos2(gutter_right, y),
                egui::pos2(clip.right(), y + pitch),
            )
            .intersect(clip);
            ui.painter()
                .rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
        }
    }
    let stroke = egui::Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color);
    let mut x = origin.x - spacing.x * 0.5;
    for width in widths {
        x += width + spacing.x;
        if x >= clip.left() && x <= clip.right() {
            ui.painter().line_segment(
                [egui::pos2(x, clip.top()), egui::pos2(x, clip.bottom())],
                stroke,
            );
        }
    }
}

fn paint_row_number(ui: &mut egui::Ui, number: &str) {
    ui.painter().text(
        ui.max_rect().right_center() - egui::vec2(3.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        number,
        egui::FontId::monospace(11.0),
        MUTED,
    );
}

fn paint_grid_corner(ui: &mut egui::Ui) {
    let center = ui.max_rect().center();
    let stroke = egui::Stroke::new(1.2_f32, MUTED);
    // Small vector expansion mark: independent of installed icon fonts.
    for direction in [
        egui::vec2(-1.0, 0.0),
        egui::vec2(1.0, 0.0),
        egui::vec2(0.0, -1.0),
        egui::vec2(0.0, 1.0),
    ] {
        let tip = center + direction * 7.0;
        let side = egui::vec2(-direction.y, direction.x) * 2.5;
        ui.painter()
            .line_segment([center + direction * 2.5, tip], stroke);
        ui.painter()
            .line_segment([tip - direction * 2.5 + side, tip], stroke);
        ui.painter()
            .line_segment([tip - direction * 2.5 - side, tip], stroke);
    }
}

fn paint_cell_background(ui: &egui::Ui, table_clip: egui::Rect, fill: Color32) {
    let rect = ui.max_rect().expand2(ui.spacing().item_spacing * 0.5);
    let mut painter = ui.painter().clone();
    painter.set_clip_rect(table_clip);
    painter.rect_filled(rect, 0.0, fill);
}

fn paint_active_cell(ui: &egui::Ui, rect: egui::Rect, table_clip: egui::Rect) {
    let mut painter = ui.painter().clone();
    painter.set_clip_rect(table_clip);
    let coral = Color32::from_rgb(255, 83, 103);
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(1.5_f32, coral),
        egui::StrokeKind::Inside,
    );
    painter.circle_filled(rect.left_top(), 2.5, coral);
    painter.circle_filled(rect.right_bottom(), 2.5, coral);
}

fn pending_fill(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(89, 73, 24)
    } else {
        Color32::from_rgb(255, 237, 158)
    }
}

// Keep the same draft while focus moves; Escape cancels only this draft.
fn inline_value(ui: &mut egui::Ui, edit: &mut Option<CellEdit>) -> bool {
    let Some(draft) = edit.as_mut() else {
        return false;
    };
    let id = ui.make_persistent_id((
        "cell-edit",
        &draft.table,
        &draft.column,
        draft.inspector,
        draft.insert_id,
    ));
    // Capture navigation before TextEdit/egui's focus traversal consumes Tab.
    // Event modifiers, rather than the frame's final modifier state, define direction.
    let advance = if ui.memory(|m| m.has_focus(id)) {
        ui.input_mut(|input| {
            if input.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab) {
                -1
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                1
            } else {
                0
            }
        })
    } else {
        0
    };
    let response = ui.add(
        TextEdit::singleline(&mut draft.text)
            .hint_text(if draft.kind == 5 { "DEFAULT" } else { "" })
            .id(id)
            .font(egui::TextStyle::Monospace)
            .background_color(Color32::TRANSPARENT)
            .desired_width(f32::INFINITY),
    );
    draft.bulk_changed |= response.changed();
    if draft.focus {
        response.scroll_to_me(None);
        response.request_focus();
        draft.focus = false;
        return false;
    }
    if (response.has_focus() || response.lost_focus())
        && ui.input(|i| i.key_pressed(egui::Key::Escape))
    {
        *edit = None;
        response.surrender_focus();
        return false;
    }
    if advance != 0 {
        draft.advance = advance;
        response.surrender_focus();
        return true;
    }
    response.lost_focus()
}

fn parse_cell_value(edit: &CellEdit) -> Result<Value, String> {
    if edit.text.len() > 65536 {
        return Err(
            "This value exceeds the 64 KiB inline editor limit. Use SQL for larger values.".into(),
        );
    }
    match edit.kind {
        4 => Ok(edit.original_value.clone()),
        0 if edit.text.is_empty() => Ok(Value::Null),
        1 => edit
            .text
            .parse::<i64>()
            .map(Value::Integer)
            .map_err(|e| e.to_string()),
        2 => edit
            .text
            .parse::<f64>()
            .map_err(|e| e.to_string())
            .and_then(|v| {
                if v.is_finite() {
                    Ok(Value::Real(v))
                } else {
                    Err("Enter a finite number.".into())
                }
            }),
        _ => Ok(Value::Text(edit.text.clone())),
    }
}

#[cfg(test)]
mod inline_edit_tests {
    use super::*;
    fn draft(kind: usize, text: &str) -> CellEdit {
        CellEdit {
            bulk_rows: Vec::new(),
            bulk_changed: false,
            insert_id: None,
            advance: 0,
            table: "customers".into(),
            row: vec![],
            original_value: Value::Null,
            column: "value".into(),
            text: text.into(),
            kind,
            focus: false,
            inspector: false,
        }
    }
    #[test]
    fn typed_inline_values_preserve_null_and_text_distinction() {
        assert_eq!(parse_cell_value(&draft(0, "")), Ok(Value::Null));
        assert_eq!(
            parse_cell_value(&draft(0, "hello")),
            Ok(Value::Text("hello".into()))
        );
        assert_eq!(
            parse_cell_value(&draft(3, "NULL")),
            Ok(Value::Text("NULL".into()))
        );
        assert_eq!(
            parse_cell_value(&draft(3, "")),
            Ok(Value::Text(String::new()))
        );
        assert_eq!(parse_cell_value(&draft(1, "42")), Ok(Value::Integer(42)));
        assert!(parse_cell_value(&draft(1, "42.2")).is_err());
        assert!(parse_cell_value(&draft(2, "NaN")).is_err());
        assert!(parse_cell_value(&draft(2, "inf")).is_err());
    }
}

fn paint_arrow(ui: &egui::Ui, rect: egui::Rect, descending: bool, color: Color32) {
    let center = rect.center();
    let direction = if descending { 1.0 } else { -1.0 };
    let tip = center + egui::vec2(0.0, 4.0 * direction);
    let stroke = Stroke::new(1.4_f32, color);
    ui.painter()
        .line_segment([center - egui::vec2(0.0, 4.0 * direction), tip], stroke);
    ui.painter().add(egui::Shape::line(
        vec![
            tip + egui::vec2(-3.0, -3.0 * direction),
            tip,
            tip + egui::vec2(3.0, -3.0 * direction),
        ],
        stroke,
    ));
}

fn arrow_button(ui: &mut egui::Ui, descending: bool, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let visuals = ui.style().interact(&response);
    ui.painter().rect_filled(rect, 3.0, visuals.weak_bg_fill);
    paint_arrow(ui, rect, descending, visuals.fg_stroke.color);
    response.on_hover_text(label)
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }
    fn edit_frame(
        ctx: &egui::Context,
        edit: &mut Option<CellEdit>,
        events: Vec<egui::Event>,
    ) -> bool {
        let mut finish = false;
        let _ = ctx.run(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    finish = inline_value(ui, edit);
                });
            },
        );
        finish
    }
    fn draft() -> Option<CellEdit> {
        Some(CellEdit {
            bulk_rows: Vec::new(),
            bulk_changed: false,
            insert_id: None,
            advance: 0,
            table: "customers".into(),
            row: vec![Value::Text("old".into())],
            original_value: Value::Text("old".into()),
            column: "name".into(),
            text: "old".into(),
            kind: 3,
            focus: true,
            inspector: false,
        })
    }
    #[test]
    fn connection_activity_covers_height_and_stays_inside_rounded_bar() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(700.0, 27.0));
        let first = connection_activity_blocks(rect, 2.0);
        let moved = connection_activity_blocks(rect, 2.5);
        assert_ne!(first, moved);
        assert!(first.iter().flatten().any(|point| point.y == rect.top()));
        assert!(first.iter().flatten().any(|point| point.y == rect.bottom()));
        assert!(
            first
                .iter()
                .all(|points| points[1].x - points[0].x <= 5.251
                    && points[2].y - points[1].y <= 5.251),
            "matrix must contain small cells, not full-height stripes"
        );
        for points in first.iter().chain(moved.iter()) {
            for point in points {
                assert!(rect.contains(*point));
                let center_x = point.x.clamp(rect.left() + 13.0, rect.right() - 13.0);
                let center_y = point.y.clamp(rect.top() + 13.0, rect.bottom() - 13.0);
                assert!(
                    point.distance(egui::pos2(center_x, center_y)) <= 13.001,
                    "block escapes rounded capsule: {point:?}"
                );
            }
        }
        assert!(connection_activity_blocks(rect, f32::NAN).is_empty());
    }

    #[test]
    fn pagination_reports_ranges_and_distinguishes_estimates() {
        let mut grid = Grid {
            rows: vec![vec![]; 300],
            truncated: true,
            total_rows: Some(db::RowTotal {
                rows: 33_627_412,
                estimated: true,
            }),
            ..Default::default()
        };
        assert_eq!(pagination_label(0, &grid), "1–300 of ~33,627,412 rows");
        grid.total_rows = Some(db::RowTotal {
            rows: 1_200,
            estimated: false,
        });
        assert_eq!(pagination_label(900, &grid), "901–1,200 of 1,200 rows");
        grid.total_rows = None;
        assert_eq!(pagination_label(0, &grid), "1–300 of … rows");
        grid.rows.clear();
        grid.truncated = false;
        grid.total_rows = Some(db::RowTotal {
            rows: 0,
            estimated: false,
        });
        assert_eq!(pagination_label(0, &grid), "0 of 0 rows");
    }

    #[test]
    fn empty_inspector_value_has_full_width_click_target() {
        let ctx = egui::Context::default();
        let mut rect = egui::Rect::NOTHING;
        let mut clicked = false;
        let mut frame = |events| {
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.set_width(300.0);
                        let response = inspector_value_preview(ui, "");
                        rect = response.rect;
                        clicked = response.clicked();
                    });
                },
            );
            (rect, clicked)
        };
        let (rect, _) = frame(vec![]);
        assert!(rect.width() >= 300.0);
        assert!(rect.height() >= 22.0);
        let pos = egui::pos2(rect.right() - 5.0, rect.center().y);
        let pointer = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(pos), pointer(true)]);
        assert!(frame(vec![pointer(false)]).1);
    }

    #[test]
    fn enter_finishes_inline_draft_and_escape_cancels_without_staging() {
        let ctx = egui::Context::default();
        let mut edit = draft();
        assert!(!edit_frame(&ctx, &mut edit, vec![]));
        assert!(edit_frame(&ctx, &mut edit, vec![key(egui::Key::Enter)]));
        assert!(edit.is_some());
        let ctx = egui::Context::default();
        let mut edit = draft();
        edit_frame(&ctx, &mut edit, vec![]);
        assert!(!edit_frame(&ctx, &mut edit, vec![key(egui::Key::Escape)]));
        assert!(edit.is_none());
    }
    #[test]
    fn shift_tab_finishes_draft_and_requests_previous_cell_from_event_modifiers() {
        let ctx = egui::Context::default();
        let mut edit = draft();
        edit_frame(&ctx, &mut edit, vec![]);
        let event = egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::SHIFT,
        };
        // Modifier release can share this frame; direction must follow the key event.
        assert!(edit_frame(&ctx, &mut edit, vec![event]));
        assert_eq!(edit.as_ref().unwrap().advance, -1);
        assert_eq!(edit.as_ref().unwrap().text, "old");
    }

    #[test]
    fn segmented_control_hides_separators_beside_the_selected_view() {
        for mut current in [View::Data, View::Structure, View::Indexes, View::Triggers] {
            let ctx = egui::Context::default();
            let output = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| table_view_switch(ui, &mut current));
            });
            let lines = output.shapes.iter().filter(|shape| matches!(&shape.shape,
                egui::Shape::LineSegment { points, .. } if points[0].x == points[1].x && (points[1].y - points[0].y - 16.0).abs() < 0.1)).count();
            assert_eq!(
                lines,
                if matches!(current, View::Data | View::Triggers) {
                    2
                } else {
                    1
                }
            );
        }
    }

    #[test]
    fn blank_grid_stripes_fill_the_viewport_and_columns_follow_horizontal_scroll() {
        for offset in [0.0, 80.0] {
            let ctx = egui::Context::default();
            let clip = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(600.0, 400.0));
            let mut stripe = Color32::TRANSPARENT;
            let mut expected_line = 0.0;
            let output = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let rect = clip.translate(egui::vec2(-offset, 0.0));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                        ui.set_clip_rect(clip);
                        stripe = ui.visuals().faint_bg_color;
                        expected_line = rect.left() + 100.0 + ui.spacing().item_spacing.x * 0.5;
                        paint_grid_backdrop(ui, &[100.0, 100.0]);
                    });
                });
            });
            let stripes: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.fill == stripe => Some(rect.rect),
                    _ => None,
                })
                .collect();
            assert!(stripes.len() > 5 && stripes.len() < 20);
            assert!(
                stripes
                    .iter()
                    .all(|rect| (rect.left() - expected_line).abs() < 0.1
                        && rect.right() == clip.right())
            );
            assert!(stripes.last().unwrap().bottom() > clip.bottom() - 50.0);
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::LineSegment { points, .. } if (points[0].x - expected_line).abs() < 0.1 && points[0].y == clip.top() && points[1].y == clip.bottom())));
        }
    }

    #[test]
    fn grid_gutter_numbers_align_right_and_header_uses_vectors() {
        let ctx = egui::Context::default();
        let mut edge = 0.0;
        let output = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.allocate_ui(egui::vec2(38.0, 22.0), |ui| {
                    edge = ui.max_rect().right() - 3.0;
                    paint_row_number(ui, "123");
                    paint_grid_corner(ui);
                });
            });
        });
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 1);
        assert_eq!(texts[0].galley.text(), "123");
        assert!((texts[0].pos.x + texts[0].galley.size().x - edge).abs() < 1.0);
        assert_eq!(
            output
                .shapes
                .iter()
                .filter(|shape| matches!(shape.shape, egui::Shape::LineSegment { .. }))
                .count(),
            12
        );
    }

    #[test]
    fn loading_grid_keeps_headers_without_loading_copy_or_stale_cells() {
        let ctx = egui::Context::default();
        let app = Rustable::with_preferences(&ctx, Preferences::default());
        for columns in [vec![], vec!["known_column".to_owned()]] {
            let output = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.empty_data_grid(ui, &columns));
            });
            let text = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                !text.contains("Loading") && !text.contains("appear here") && !text.contains('#')
            );
            if !columns.is_empty() {
                assert!(text.contains("known_column"));
            }
        }
    }

    #[test]
    fn data_grid_restores_profile_geometry_after_restart_and_column_reorder() {
        let scope = crate::column_widths::Scope::Table {
            profile: crate::connection_profiles::ProfileId(42),
            table: "customers".into(),
        };
        let mut prefs = Preferences::default();
        prefs
            .column_widths
            .record(&scope, &["id".into(), "email".into()], &[90.0, 330.0]);
        let serialized = serde_json::to_string(&prefs).unwrap();
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, serde_json::from_str(&serialized).unwrap());
        app.active_profile = Some(ProfileIdentity {
            id: crate::connection_profiles::ProfileId(42),
            name: "Renamed profile".into(),
            environment: "Local".into(),
            color: [0, 0, 0],
        });
        app.pending = None;
        app.view = View::Data;
        app.browse.table = "customers".into();
        app.table = Arc::new(Grid {
            columns: vec!["email".into(), "id".into()],
            ..Default::default()
        });
        for _ in 0..3 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.data_grid(ui));
                },
            );
        }
        assert_eq!(
            app.prefs.column_widths.widths(&scope, &app.table.columns),
            vec![330.0, 90.0]
        );
        assert!(
            !app.shared_preferences_dirty,
            "restoring geometry must not publish writes every frame"
        );
        let mut root = Rustable::with_launcher_preferences(&ctx, Preferences::default());
        assert!(
            app.prefs
                .column_widths
                .record(&scope, &app.table.columns, &[360.0, 90.0])
        );
        app.shared_preferences_dirty = true;
        assert!(root.merge_window_preferences(&mut app));
        // A live grid's egui cache must yield to geometry received from another window.
        for _ in 0..2 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.data_grid(ui));
                },
            );
        }
        assert_eq!(
            app.prefs.column_widths.widths(&scope, &app.table.columns),
            vec![360.0, 90.0]
        );
        let mut sibling = Rustable::with_launcher_preferences(&ctx, Preferences::default());
        root.sync_window_preferences(&mut sibling);
        assert_eq!(
            sibling
                .prefs
                .column_widths
                .widths(&scope, &app.table.columns),
            vec![360.0, 90.0]
        );
    }

    #[test]
    fn column_width_drag_persists_after_release_without_resetting() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.active_profile = Some(ProfileIdentity {
            id: crate::connection_profiles::ProfileId(43),
            name: "Resize QA".into(),
            environment: "Local".into(),
            color: [0, 0, 0],
        });
        app.view = View::Data;
        app.browse.table = "customers".into();
        app.table = Arc::new(Grid {
            columns: vec!["id".into(), "email".into()],
            ..Default::default()
        });
        let scope = crate::column_widths::Scope::Table {
            profile: crate::connection_profiles::ProfileId(43),
            table: "customers".into(),
        };
        let mut render = |events: Vec<egui::Event>| {
            ctx.run(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.data_grid(ui));
                },
            )
        };
        render(vec![]);
        let output = render(vec![]);
        let edge = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::LineSegment { points, .. }
                    if (points[0].x - points[1].x).abs() < 0.1
                        && points[0].x > 70.0
                        && points[0].x < 120.0
                        && points[0].y < 25.0
                        && points[1].y > 400.0 =>
                {
                    Some(points[0].x)
                }
                _ => None,
            })
            .expect("first data column resize handle");
        let body_start = egui::pos2(edge, 100.0);
        let body_end = body_start + egui::vec2(80.25, 0.0);
        let hover = render(vec![egui::Event::PointerMoved(body_start)]);
        assert_ne!(hover.platform_output.cursor_icon, egui::CursorIcon::ResizeColumn);
        render(vec![egui::Event::PointerButton { pos: body_start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }]);
        render(vec![egui::Event::PointerMoved(body_end)]);
        render(vec![egui::Event::PointerButton { pos: body_end, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }]);
        let start = egui::pos2(edge, 18.0);
        // A header-acquired resize must continue if the pointer enters the body.
        let end = start + egui::vec2(80.25, 100.0);
        render(vec![egui::Event::PointerMoved(start)]);
        render(vec![egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }]);
        render(vec![egui::Event::PointerMoved(end)]);
        render(vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        for _ in 0..3 {
            render(vec![]);
        }
        let keys = crate::column_widths::Prepared::new(&scope, &app.table.columns);
        assert!(app.prefs.column_widths.has_width(&keys, 0));
        assert!(
            !app.prefs.column_widths.has_width(&keys, 1),
            "unresized column must remain automatic"
        );
        let widths = app.prefs.column_widths.widths(&scope, &app.table.columns);
        assert!(
            (widths[0] - 120.25).abs() < 0.01,
            "resize reverted or was not captured: {widths:?}"
        );
        assert!(
            (widths[1] - 155.0).abs() < 1.0,
            "unrelated column changed: {widths:?}"
        );
        let restarted: Preferences =
            serde_json::from_str(&serde_json::to_string(&app.prefs).unwrap()).unwrap();
        assert_eq!(
            restarted.column_widths.widths(&scope, &app.table.columns),
            widths
        );
    }

    #[test]
    fn grid_scrollbar_style_has_no_hover_or_navigation_fade_and_does_not_leak() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let original = ui.spacing().scroll;
                with_grid_scrollbars(ui, |ui| {
                    let scroll = ui.spacing().scroll;
                    assert!(scroll.dormant_handle_opacity > 0.0);
                    assert_eq!(scroll.dormant_handle_opacity, scroll.active_handle_opacity);
                    assert_eq!(scroll.active_handle_opacity, scroll.interact_handle_opacity);
                    assert_eq!(scroll.floating_width, scroll.bar_width);
                    assert_eq!(
                        scroll.dormant_background_opacity,
                        scroll.active_background_opacity
                    );
                    assert_eq!(
                        scroll.active_background_opacity,
                        scroll.interact_background_opacity
                    );
                });
                assert_eq!(
                    ui.spacing().scroll,
                    original,
                    "grid styling must not affect surrounding UI"
                );
            });
        });
    }

    #[test]
    fn loading_grid_preserves_saved_geometry_during_refresh() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.active_profile = Some(ProfileIdentity {
            id: crate::connection_profiles::ProfileId(46),
            name: "Loading geometry".into(),
            environment: "Local".into(),
            color: [0, 0, 0],
        });
        app.view = View::Data;
        app.browse.table = "customers".into();
        app.table = Arc::new(Grid {
            columns: vec!["id".into(), "email".into()],
            rows: vec![vec![Value::Integer(1), Value::Text("a@example.com".into())]],
            ..Default::default()
        });
        let scope = crate::column_widths::Scope::Table {
            profile: crate::connection_profiles::ProfileId(46),
            table: "customers".into(),
        };
        app.prefs
            .column_widths
            .record(&scope, &app.table.columns, &[80.25, 430.0]);
        let mut positions = Vec::new();
        for loading in [false, false, true] {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        if loading {
                            app.empty_data_grid(ui, &app.table.columns);
                        } else {
                            app.data_grid(ui);
                        }
                    });
                },
            );
            positions.push(
                output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::LineSegment { points, .. }
                            if (points[0].x - points[1].x).abs() < 0.01
                                && points[0].y > 20.0
                                && points[1].y > 400.0 =>
                        {
                            Some(points[0].x)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            );
        }
        assert!(!positions[1].is_empty());
        assert_eq!(
            positions[1], positions[2],
            "loading must not reset column boundaries"
        );
    }

    #[test]
    fn column_header_centers_the_text_and_sort_arrow_as_one_group() {
        let ctx = egui::Context::default();
        let mut center = egui::Pos2::ZERO;
        let output = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.allocate_ui(egui::vec2(240.0, 25.0), |ui| {
                    center = ui.max_rect().center();
                    paint_column_header(ui, "Name", Some(false));
                });
            });
        });
        let text = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Name" => Some(text),
                _ => None,
            })
            .unwrap();
        assert!((text.pos.x + text.galley.size().x * 0.5 + 8.0 - center.x).abs() < 0.5);
        assert!((text.pos.y + text.galley.size().y * 0.5 - center.y).abs() < 0.5);
    }

    #[test]
    fn content_defaults_keep_ids_compact_without_persisting_automatic_geometry() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.active_profile = Some(ProfileIdentity {
            id: crate::connection_profiles::ProfileId(45),
            name: "Widths".into(),
            environment: "Local".into(),
            color: [0, 0, 0],
        });
        app.view = View::Data;
        app.browse.table = "customers".into();
        app.table = Arc::new(Grid {
            columns: vec!["id".into(), "email".into()],
            rows: vec![vec![
                Value::Integer(1),
                Value::Text("long.email.address@long.example.domain.test".repeat(20)),
            ]],
            ..Default::default()
        });
        let scope = crate::column_widths::Scope::Table {
            profile: crate::connection_profiles::ProfileId(45),
            table: "customers".into(),
        };
        let keys = crate::column_widths::Prepared::new(&scope, &app.table.columns);
        for _ in 0..3 {
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let widths = content_column_widths(ui, &app.table);
                    assert!(widths[0] <= 50.0, "{widths:?}");
                    assert_eq!(
                        widths[1],
                        crate::column_widths::DEFAULT_WIDTH,
                        "long content must not exceed the 155px automatic cap"
                    );
                    let widths_id = ui.make_persistent_id((
                        "restored-grid-widths",
                        Some(&scope),
                        &app.table.columns,
                        app.tab,
                    ));
                    app.data_grid(ui);
                    let rendered = ctx.data(|data| data.get_temp::<Vec<f32>>(widths_id).unwrap());
                    assert_eq!(
                        rendered, widths,
                        "rendered widths must match content defaults"
                    );
                });
            });
        }
        assert!(!app.prefs.column_widths.has_width(&keys, 0));
        assert!(!app.prefs.column_widths.has_width(&keys, 1));
        assert!(!app.shared_preferences_dirty);
        app.prefs
            .column_widths
            .record(&scope, &app.table.columns, &[40.0, 420.0]);
        // Sorting/page refresh replaces the result allocation. Manual widths
        // must still win even when the new content would fit a tiny field.
        app.table = Arc::new(Grid {
            columns: vec!["id".into(), "email".into()],
            rows: vec![vec![Value::Integer(2), Value::Text("x".into())]],
            ..Default::default()
        });
        for _ in 0..2 {
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let id = ui.make_persistent_id((
                        "grid-column-sizing",
                        &app.browse.table,
                        false,
                        app.tab,
                        &app.table.columns,
                    ));
                    app.data_grid(ui);
                    let measured =
                        ctx.data(|data| data.get_temp::<f32>(id.with("widths")).unwrap());
                    assert!(
                        (measured - (38.0 + 40.0 + 420.0 + ui.spacing().item_spacing.x * 3.0))
                            .abs()
                            < 1.0,
                        "manual width lost during refresh: {measured}"
                    );
                });
            });
        }
    }

    #[test]
    fn header_whitespace_sorts_and_selection_keeps_horizontal_extent_stable() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.view = View::Data;
        app.browse.table = "customers".into();
        app.table = Arc::new(Grid {
            columns: vec!["id".into(), "email".into()],
            rows: vec![vec![
                Value::Integer(1),
                Value::Text("customer@example.com".into()),
            ]],
            ..Default::default()
        });
        let fixture = app.table.clone();
        let mut extent = 0.0;
        let mut render = |app: &mut Rustable, events: Vec<egui::Event>| {
            let output = ctx.run(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let id = ui.make_persistent_id((
                            "grid-column-sizing",
                            &app.browse.table,
                            false,
                            app.tab,
                            &app.table.columns,
                        ));
                        app.data_grid(ui);
                        extent = ctx.data(|data| data.get_temp::<f32>(id.with("widths")).unwrap_or(extent));
                    });
                },
            );
            (output, extent)
        };
        render(&mut app, vec![]);
        let (_, before) = render(&mut app, vec![]);
        let row = egui::pos2(70.0, 45.0);
        render(
            &mut app,
            vec![
                egui::Event::PointerMoved(row),
                egui::Event::PointerButton {
                    pos: row,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let (_, after) = render(
            &mut app,
            vec![egui::Event::PointerButton {
                pos: row,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(app.selected, Some(0));
        assert_eq!(before, after, "selection must not grow horizontal content");
        // Email header has substantial whitespace beyond its short label.
        let header_blank = egui::pos2(200.0, 20.0);
        render(
            &mut app,
            vec![
                egui::Event::PointerMoved(header_blank),
                egui::Event::PointerButton {
                    pos: header_blank,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        render(
            &mut app,
            vec![egui::Event::PointerButton {
                pos: header_blank,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(app.browse.sort, Some(("email".into(), true)));

        // Right-click choices work without cycling through intermediate sorts.
        fn text_center(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label =>
                    Some(text.pos + text.galley.size() * 0.5),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_center(shape, label)),
                _ => None,
            }
        }
        for (label, expected) in [
            ("Sort ascending", Some(("email".to_owned(), false))),
            ("Restore default order", None),
        ] {
            app.pending = None;
            app.table = fixture.clone();
            render(&mut app, vec![]);
            app.browse.offset = 500;
            for pressed in [true, false] {
                render(&mut app, vec![egui::Event::PointerMoved(header_blank), egui::Event::PointerButton {
                    pos: header_blank, button: egui::PointerButton::Secondary, pressed,
                    modifiers: egui::Modifiers::NONE,
                }]);
            }
            let (output, _) = render(&mut app, vec![]);
            let point = output.shapes.iter().find_map(|shape| text_center(&shape.shape, label)).expect("sort menu item");
            for pressed in [true, false] {
                render(&mut app, vec![egui::Event::PointerMoved(point), egui::Event::PointerButton {
                    pos: point, button: egui::PointerButton::Primary, pressed,
                    modifiers: egui::Modifiers::NONE,
                }]);
            }
            assert_eq!(app.browse.sort, expected);
            assert_eq!(app.browse.offset, 0);
        }
    }

    #[test]
    fn fit_columns_measures_untruncated_values_and_keeps_widths_afterward() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.view = View::Data;
        app.table = Arc::new(Grid {
            columns: vec!["id".into(), "email".into()],
            rows: vec![vec![
                Value::Integer(1),
                Value::Text("a.long.customer.address@an.example.company.with.a.longer.organisation.name.test".into()),
            ]],
            ..Default::default()
        });
        let mut widths = Vec::new();
        for frame in 0..7 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let id = ui.make_persistent_id((
                            "grid-column-sizing",
                            &app.browse.table,
                            false,
                            app.tab,
                            &app.table.columns,
                        ));
                        if frame == 3 {
                            ctx.data_mut(|data| data.insert_temp(id, true));
                        }
                        app.data_grid(ui);
                        widths.push(
                            ctx.data(|data| data.get_temp::<f32>(id.with("widths")).unwrap()),
                        );
                    });
                },
            );
        }
        assert!(widths[6] > widths[2] + 30.0, "{widths:?}");
        assert!((widths[6] - widths[5]).abs() < 1.0, "{widths:?}");
    }

    #[test]
    fn data_row_press_selects_once_and_command_release_does_not_retoggle() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_launcher_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.view = View::Data;
        app.table = Arc::new(Grid {
            columns: vec!["name".into()],
            rows: vec![
                vec![Value::Text("press_first_row".into())],
                vec![Value::Text("press_second_row".into())],
            ],
            ..Default::default()
        });
        let frame = |app: &mut Rustable, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 500.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.data_grid(ui));
                },
            )
        };
        frame(&mut app, vec![]);
        let output = frame(&mut app, vec![]);
        let position = |name| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == name => {
                        Some(text.pos + egui::vec2(3.0, 3.0))
                    }
                    _ => None,
                })
                .unwrap()
        };
        let first = position("press_first_row");
        let second = position("press_second_row");
        let pointer = |pos, pressed, modifiers| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                },
            ]
        };
        frame(&mut app, vec![egui::Event::PointerMoved(first)]);
        assert_eq!(app.selected, None, "hover must not select");
        frame(&mut app, pointer(first, true, egui::Modifiers::NONE));
        assert_eq!(app.selected, Some(0), "text label must select on press");
        frame(&mut app, pointer(first, false, egui::Modifiers::NONE));
        frame(&mut app, pointer(second, true, egui::Modifiers::COMMAND));
        assert_eq!(
            app.row_selection.rows.iter().copied().collect::<Vec<_>>(),
            vec![0, 1]
        );
        frame(&mut app, pointer(second, false, egui::Modifiers::COMMAND));
        assert_eq!(
            app.row_selection.rows.iter().copied().collect::<Vec<_>>(),
            vec![0, 1],
            "release must not toggle again"
        );
        // The gutter selects too, even though its background stays neutral.
        let gutter = egui::pos2(20.0, second.y);
        frame(&mut app, pointer(gutter, true, egui::Modifiers::NONE));
        assert_eq!(app.selected, Some(1));
        frame(&mut app, pointer(gutter, false, egui::Modifiers::NONE));
        assert_eq!(app.selected, Some(1));
    }

    #[test]
    fn short_and_empty_results_keep_full_width_striping_without_fake_rows() {
        for count in [0, 2] {
            let ctx = egui::Context::default();
            let mut app = Rustable::with_preferences(&ctx, Preferences::default());
            app.pending = None;
            app.view = View::Data;
            app.selected = None;
            app.table = Arc::new(Grid {
                columns: vec!["id".into()],
                rows: (0..count)
                    .map(|index| vec![Value::Integer(index as i64)])
                    .collect(),
                ..Default::default()
            });
            let mut stripe = Color32::TRANSPARENT;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        stripe = ui.visuals().faint_bg_color;
                        app.data_grid(ui);
                    });
                },
            );
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == stripe && rect.rect.width() > 700.0 && rect.rect.bottom() > 440.0)));
            assert_eq!(app.table.rows.len(), count);
            assert!(app.selected.is_none());
        }
    }

    #[test]
    fn inline_drafts_stay_neutral_until_staged_and_active_cell_excludes_gutter() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_launcher_preferences(&ctx, Preferences::default());
        app.view = View::Data;
        app.browse.table = "customers".into();
        app.table = Arc::new(Grid {
            columns: vec!["name".into()],
            rows: vec![vec![Value::Text("old".into())]],
            ..Default::default()
        });
        app.selected = app.row_selection.click(0, false, false, None);
        app.edit = draft();
        let render = |app: &mut Rustable| {
            let mut pending = Color32::TRANSPARENT;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        pending = pending_fill(ui);
                        app.data_grid(ui);
                    });
                },
            );
            (output, pending)
        };
        render(&mut app);
        let (untouched, pending) = render(&mut app);
        let has_pending = |output: &egui::FullOutput| {
            output.shapes.iter().any(
                |shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == pending),
            )
        };
        assert!(app.edit.is_some());
        assert!(
            !has_pending(&untouched),
            "entering edit must not imply a staged change"
        );
        app.edit.as_mut().unwrap().text = "new draft".into();
        let (typing, _) = render(&mut app);
        assert!(
            !has_pending(&typing),
            "typing is still a draft until focus leaves/commit"
        );
        assert!(app.changes.is_empty());

        app.edit = None;
        app.changes.push(Change::Cell {
            table: "customers".into(),
            column: "name".into(),
            original: vec![Value::Text("old".into())],
            value: Value::Text("new draft".into()),
        });
        let (staged, _) = render(&mut app);
        assert!(
            has_pending(&staged),
            "staged changes need the pending-save background"
        );
        let coral = Color32::from_rgb(255, 83, 103);
        let outlines: Vec<_> = staged
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.stroke.color == coral => Some(rect.rect),
                _ => None,
            })
            .collect();
        assert_eq!(outlines.len(), 1, "exactly one active data-cell outline");
        let pending_rect = staged
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill == pending => Some(rect.rect),
                _ => None,
            })
            .expect("staged cell background");
        assert_eq!(
            outlines[0], pending_rect,
            "outline must follow the full cell boundaries, not an inset"
        );
        let gutter_right = staged
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "1" => {
                    Some(text.pos.x + text.galley.size().x)
                }
                _ => None,
            })
            .expect("row-number gutter");
        assert!(
            outlines[0].left() > gutter_right,
            "the gutter must not get the cell outline"
        );
        let corners: Vec<_> = staged
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Circle(circle) if circle.fill == coral => Some(circle.center),
                _ => None,
            })
            .collect();
        assert_eq!(
            corners,
            vec![outlines[0].left_top(), outlines[0].right_bottom()]
        );
    }

    #[test]
    fn pending_cell_paint_covers_spacing_and_respects_table_viewport() {
        let ctx = egui::Context::default();
        let mut expected = egui::Rect::NOTHING;
        let mut viewport = egui::Rect::NOTHING;
        let fill = Color32::from_rgb(89, 73, 24);
        let output = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                TableBuilder::new(ui)
                    .column(TableColumn::exact(100.0).clip(true))
                    .body(|mut body| {
                        viewport = body.ui_mut().clip_rect();
                        body.row(22.0, |mut row| {
                            row.col(|ui| {
                                expected = ui.max_rect().expand2(ui.spacing().item_spacing * 0.5);
                                paint_cell_background(ui, viewport, fill);
                            });
                        });
                    });
            });
        });
        let painted = output
            .shapes
            .iter()
            .find(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == fill))
            .unwrap();
        let egui::Shape::Rect(rect) = &painted.shape else {
            unreachable!()
        };
        assert_eq!(rect.rect, expected);
        assert_eq!(painted.clip_rect, viewport);
    }

    #[test]
    fn tab_finishes_draft_and_requests_next_cell() {
        let ctx = egui::Context::default();
        let mut edit = draft();
        edit_frame(&ctx, &mut edit, vec![]);
        assert!(edit_frame(&ctx, &mut edit, vec![key(egui::Key::Tab)]));
        assert_eq!(edit.as_ref().unwrap().advance, 1);
    }
    #[test]
    fn clipboard_click_emits_copy_and_schedules_feedback_expiry() {
        let ctx = egui::Context::default();
        let mut rect = egui::Rect::NOTHING;
        let mut id = egui::Id::NULL;
        let mut frame = |time, events| {
            ctx.run(
                egui::RawInput {
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let response = clipboard_button(ui);
                        rect = response.rect;
                        id = response.id;
                        if response.clicked() {
                            ctx.copy_text("full value".into());
                        }
                    });
                },
            )
        };
        frame(1.0, vec![]);
        // Default CentralPanel padding positions the 22px button at (8,8).
        let pos = egui::pos2(19.0, 19.0);
        frame(
            1.1,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let output = frame(
            1.2,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(
            output
                .platform_output
                .commands
                .iter()
                .any(|c| matches!(c, egui::OutputCommand::CopyText(text) if text == "full value"))
        );
        frame(2.4, vec![]);
        let until = ctx
            .data(|d| d.get_temp::<f64>(id.with("copied-until")))
            .unwrap();
        assert!((until - 2.3).abs() < 0.001);
        assert!(rect.contains(pos));
    }
}

fn stage_cell_value(changes: &mut Vec<Change>, edit: CellEdit, value: Value) {
    if changes.iter().any(|change| matches!(change,
        Change::Cell { table, column, original, value: staged }
        if table == &edit.table && column == &edit.column && original == &edit.row && staged == &value
    )) { return; }
    changes.retain(|change| {
        !matches!(change, Change::Cell { table, column, original, .. }
        if table == &edit.table && column == &edit.column && original == &edit.row)
    });
    if edit.original_value != value {
        changes.push(Change::Cell {
            table: edit.table,
            column: edit.column,
            original: edit.row,
            value,
        });
    }
}

#[cfg(test)]
mod staging_tests {
    use super::*;
    fn draft(table: &str) -> CellEdit {
        CellEdit {
            bulk_rows: Vec::new(),
            bulk_changed: false,
            insert_id: None,
            advance: 0,
            table: table.into(),
            row: vec![Value::Integer(1), Value::Text("old".into())],
            original_value: Value::Text("old".into()),
            column: "name".into(),
            text: "new".into(),
            kind: 3,
            focus: false,
            inspector: false,
        }
    }
    #[test]
    fn edits_with_identical_rows_are_isolated_by_source_table() {
        let mut changes = vec![];
        stage_cell_value(
            &mut changes,
            draft("customers"),
            Value::Text("first".into()),
        );
        stage_cell_value(
            &mut changes,
            draft("suppliers"),
            Value::Text("other".into()),
        );
        stage_cell_value(
            &mut changes,
            draft("customers"),
            Value::Text("latest".into()),
        );
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().any(|c| matches!(c, Change::Cell { table, value: Value::Text(value), .. } if table == "suppliers" && value == "other")));
        assert!(changes.iter().any(|c| matches!(c, Change::Cell { table, value: Value::Text(value), .. } if table == "customers" && value == "latest")));
        stage_cell_value(&mut changes, draft("customers"), Value::Text("old".into()));
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], Change::Cell { table, .. } if table == "suppliers"));
    }
    #[test]
    fn binary_revert_preserves_original_bytes() {
        let mut edit = draft("files");
        edit.original_value = Value::Blob(vec![0, 255, 128]);
        let mut changes = vec![];
        stage_cell_value(&mut changes, edit, Value::Null);
        let mut edit = draft("files");
        edit.original_value = Value::Blob(vec![0, 255, 128]);
        edit.kind = 4;
        let value = parse_cell_value(&edit).unwrap();
        assert_eq!(value, Value::Blob(vec![0, 255, 128]));
        stage_cell_value(&mut changes, edit, value);
        assert!(changes.is_empty());
    }
}

fn row_is_deleted(changes: &[Change], table: &str, row: &[Value]) -> bool {
    changes.iter().any(
        |c| matches!(c, Change::DeleteRow { table: t, original } if t == table && original == row),
    )
}
fn toggle_row_deletion(changes: &mut Vec<Change>, table: &str, row: Vec<Value>) {
    if row_is_deleted(changes, table, &row) {
        changes.retain(|c| !matches!(c, Change::DeleteRow { table: t, original } if t == table && original == &row));
    } else {
        changes.push(Change::DeleteRow {
            table: table.into(),
            original: row,
        });
    }
}

#[cfg(test)]
mod deletion_staging_tests {
    use super::*;
    #[test]
    fn restoring_deletion_keeps_previously_staged_cell_edits() {
        let row = vec![Value::Integer(1), Value::Text("old".into())];
        let mut changes = vec![Change::Cell {
            table: "t".into(),
            column: "name".into(),
            original: row.clone(),
            value: Value::Text("new".into()),
        }];
        toggle_row_deletion(&mut changes, "t", row.clone());
        assert!(row_is_deleted(&changes, "t", &row));
        assert!(!row_is_deleted(&changes, "other", &row));
        toggle_row_deletion(&mut changes, "t", row);
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0],Change::Cell { value:Value::Text(value),.. } if value=="new"));
    }
}

fn saved_row_identity(
    table: &str,
    columns: &[Column],
    row: &[Value],
    changes: &[Change],
) -> Option<Vec<(String, Value)>> {
    if row_is_deleted(changes, table, row) {
        return None;
    }
    let mut keys = Vec::new();
    for (index, column) in columns.iter().enumerate().filter(|(_, c)| c.pk) {
        let mut value = row.get(index)?.clone();
        let mut name = column.name.clone();
        for change in changes {
            match change {
                Change::Cell {
                    table: t,
                    column: c,
                    original,
                    value: v,
                } if t == table && *c == column.name && original == row => value = v.clone(),
                Change::RenameColumn { table: t, from, to }
                    if t == table && *from == column.name =>
                {
                    name = to.clone()
                }
                _ => {}
            }
        }
        keys.push((name, value));
    }
    (!keys.is_empty()).then_some(keys)
}
fn find_saved_row(grid: &Grid, keys: &[(String, Value)]) -> Option<usize> {
    if keys.is_empty() {
        return None;
    }
    grid.rows.iter().position(|row| {
        keys.iter().all(|(name, value)| {
            grid.columns
                .iter()
                .position(|c| c == name)
                .and_then(|i| row.get(i))
                == Some(value)
        })
    })
}

#[cfg(test)]
mod selection_restore_tests {
    use super::*;
    #[test]
    fn restores_by_updated_key_after_reordering_and_key_rename() {
        let column = Column {
            name: "id".into(),
            data_type: "INTEGER".into(),
            not_null: true,
            default: String::new(),
            pk: true,
            generated: false,
        };
        let row = vec![Value::Integer(1)];
        let changes = vec![
            Change::Cell {
                table: "t".into(),
                column: "id".into(),
                original: row.clone(),
                value: Value::Integer(9),
            },
            Change::RenameColumn {
                table: "t".into(),
                from: "id".into(),
                to: "key".into(),
            },
        ];
        let identity = saved_row_identity("t", &[column], &row, &changes).unwrap();
        let grid = Grid {
            columns: vec!["key".into()],
            rows: vec![vec![Value::Integer(2)], vec![Value::Integer(9)]],
            ..Default::default()
        };
        assert_eq!(find_saved_row(&grid, &identity), Some(1));
        assert_eq!(find_saved_row(&Grid::default(), &identity), None);
        assert!(
            saved_row_identity(
                "t",
                &[],
                &row,
                &[Change::DeleteRow {
                    table: "t".into(),
                    original: row.clone()
                }]
            )
            .is_none()
        );
    }
}

fn next_cell_edit(
    source: &CellEdit,
    grid: &Grid,
    columns: &[Column],
    changes: &[Change],
    field_search: &str,
) -> Option<CellEdit> {
    let width = grid.columns.len();
    if width == 0 || source.advance == 0 {
        return None;
    }
    let row_index = grid.rows.iter().position(|row| *row == source.row)?;
    let column_index = grid
        .columns
        .iter()
        .position(|name| *name == source.column)?;
    cell_edit_from_slot(
        source,
        grid,
        columns,
        changes,
        field_search,
        (row_index * width + column_index) as isize,
    )
}

fn cell_edit_from_slot(
    source: &CellEdit,
    grid: &Grid,
    columns: &[Column],
    changes: &[Change],
    field_search: &str,
    mut slot: isize,
) -> Option<CellEdit> {
    let width = grid.columns.len();
    if width == 0 {
        return None;
    }
    let limit = (grid.rows.len() * width) as isize;
    let filter = field_search.to_lowercase();
    loop {
        slot += isize::from(source.advance.signum());
        if slot < 0 || slot >= limit {
            return None;
        }
        let row_index = slot as usize / width;
        let column_index = slot as usize % width;
        let row = &grid.rows[row_index];
        let name = &grid.columns[column_index];
        if source.inspector && *row != source.row {
            return None;
        }
        if source.inspector && !name.to_lowercase().contains(&filter) {
            continue;
        }
        if row_is_deleted(changes, &source.table, row) {
            continue;
        }
        if columns
            .iter()
            .find(|c| c.name == *name)
            .is_none_or(|c| c.generated)
        {
            continue;
        }
        let original = row.get(column_index)?;
        let value = changes
            .iter()
            .find_map(|c| match c {
                Change::Cell {
                    table,
                    column,
                    original: r,
                    value,
                } if *table == source.table && column == name && r == row => Some(value),
                _ => None,
            })
            .unwrap_or(original);
        if matches!(value, Value::Blob(_)) || matches!(value,Value::Text(t) if t.len()>65536) {
            continue;
        }
        return Some(CellEdit {
            bulk_rows: Vec::new(),
            bulk_changed: false,
            insert_id: None,
            advance: 0,
            table: source.table.clone(),
            row: row.clone(),
            original_value: original.clone(),
            column: name.clone(),
            text: if matches!(value, Value::Null) {
                String::new()
            } else {
                db::display(value)
            },
            kind: match value {
                Value::Null => 0,
                Value::Integer(_) => 1,
                Value::Real(_) => 2,
                _ => 3,
            },
            focus: true,
            inspector: source.inspector,
        });
    }
}

#[cfg(test)]
mod cell_navigation_tests {
    use super::*;
    #[test]
    fn navigation_skips_readonly_deleted_and_binary_cells_and_keeps_staged_text() {
        let columns: Vec<_> = ["id", "computed", "blob", "name"]
            .into_iter()
            .map(|name| Column {
                name: name.into(),
                data_type: "TEXT".into(),
                not_null: false,
                default: String::new(),
                pk: name == "id",
                generated: name == "computed",
            })
            .collect();
        let row = |id| {
            vec![
                Value::Integer(id),
                Value::Text("generated".into()),
                Value::Blob(vec![0]),
                Value::Text("original".into()),
            ]
        };
        let grid = Grid {
            columns: columns.iter().map(|c| c.name.clone()).collect(),
            rows: vec![row(1), row(2), row(3)],
            ..Default::default()
        };
        let mut source = CellEdit {
            bulk_rows: Vec::new(),
            bulk_changed: false,
            insert_id: None,
            advance: 1,
            table: "t".into(),
            row: row(1),
            original_value: Value::Integer(1),
            column: "id".into(),
            text: "1".into(),
            kind: 1,
            focus: false,
            inspector: false,
        };
        let changes = vec![
            Change::Cell {
                table: "t".into(),
                column: "name".into(),
                original: row(1),
                value: Value::Text("staged".into()),
            },
            Change::DeleteRow {
                table: "t".into(),
                original: row(2),
            },
        ];
        let mut next = next_cell_edit(&source, &grid, &columns, &changes, "").unwrap();
        assert_eq!(next.column, "name");
        assert_eq!(next.text, "staged");
        next.advance = 1;
        let mut third = next_cell_edit(&next, &grid, &columns, &changes, "").unwrap();
        assert_eq!(third.row, row(3));
        assert_eq!(third.column, "id");
        third.advance = -1;
        assert_eq!(
            next_cell_edit(&third, &grid, &columns, &changes, "")
                .unwrap()
                .column,
            "name"
        );
        source.advance = -1;
        assert!(next_cell_edit(&source, &grid, &columns, &changes, "").is_none());
        next.inspector = true;
        assert!(next_cell_edit(&next, &grid, &columns, &changes, "").is_none());
        source.advance = 1;
        source.inspector = true;
        assert!(next_cell_edit(&source, &grid, &columns, &changes, "id").is_none());
    }
}

#[cfg(test)]
mod workspace_layout_tests {
    use super::*;

    fn text_positions(shape: &egui::Shape, found: &mut Vec<(String, f32)>) {
        match shape {
            egui::Shape::Text(text) => found.push((text.galley.job.text.clone(), text.pos.y)),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    text_positions(shape, found);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn beautify_shortcut_is_one_undoable_editor_change() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        let original = "select id,name from customers where id=2;";
        app.tabs[0].sql = original.into();
        app.show_query();
        let frame = |app: &mut Rustable, events| {
            let _ = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 920.0))),
                events, ..Default::default()
            }, |ctx| app.workspace(ctx));
        };
        let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        frame(&mut app, vec![key(egui::Key::I, egui::Modifiers::COMMAND)]);
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while app.tabs[0].formatter.busy() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
            frame(&mut app, vec![]);
        }
        let formatted = app.tabs[0].sql.clone();
        assert_ne!(formatted, original);
        assert!(formatted.contains('\n'));
        frame(&mut app, vec![key(egui::Key::Z, egui::Modifiers::COMMAND)]);
        assert_eq!(app.tabs[0].sql, original);
        frame(&mut app, vec![key(egui::Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT)]);
        assert_eq!(app.tabs[0].sql, formatted);
    }
    #[test]
    fn completion_tab_accepts_and_undo_redo_preserve_editor_caret() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.tabs[0].sql.clear();
        app.show_query();
        let mut frame = |events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 920.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.workspace(ctx),
            );
            (app.tabs[0].sql.clone(), app.tabs[0].cursor.clone())
        };
        frame(vec![]);
        frame(vec![]);
        frame(vec![egui::Event::Text("sel".into())]);
        let accepted = frame(vec![egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(accepted, ("SELECT".into(), 6..6));
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let undone = frame(vec![key(egui::Key::Z, egui::Modifiers::COMMAND)]);
        assert_eq!(undone, ("sel".into(), 3..3));
        let redone = frame(vec![key(
            egui::Key::Z,
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        )]);
        assert_eq!(redone, ("SELECT".into(), 6..6));
        let typed = frame(vec![egui::Event::Text(" 42".into())]);
        assert_eq!(typed, ("SELECT 42".into(), 9..9));
    }

    #[test]
    fn clicking_completion_accepts_and_keeps_typing_in_editor() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.tabs[0].sql.clear();
        app.show_query();
        fn suggestion(shape: &egui::Shape) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text == "SELECT" => {
                    Some(text.pos + egui::vec2(4.0, 4.0))
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(suggestion),
                _ => None,
            }
        }
        let mut frame = |events| {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 920.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.workspace(ctx),
            );
            (
                app.tabs[0].sql.clone(),
                output.shapes.iter().find_map(|s| suggestion(&s.shape)),
            )
        };
        frame(vec![]);
        frame(vec![]);
        frame(vec![egui::Event::Text("sel".into())]);
        let (_, point) = frame(vec![]);
        let point = point.expect("visible completion suggestion");
        frame(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let (sql, _) = frame(vec![egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(sql, "SELECT");
        assert_eq!(frame(vec![egui::Event::Text(" 42".into())]).0, "SELECT 42");
    }

    #[test]
    fn inspector_fields_stay_compact_in_a_tall_window() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.inspector = true;
        app.view = View::Data;
        app.table = Arc::new(Grid {
            columns: vec![
                "id".into(),
                "name".into(),
                "metadata".into(),
                "created_at".into(),
            ],
            rows: vec![vec![
                Value::Integer(12),
                Value::Text("Alice".into()),
                Value::Text(r#"{"source":"website"}"#.into()),
                Value::Text("2026-01-01 03:00:00".into()),
            ]],
            ..Default::default()
        });
        app.selected = Some(0);
        for _ in 0..3 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 1000.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.inspector(ctx),
            );
            let mut positions = Vec::new();
            for clipped in output.shapes {
                text_positions(&clipped.shape, &mut positions);
            }
            let y = |text: &str| positions.iter().find(|(s, _)| s == text).unwrap().1;
            for fields in ["id", "name", "metadata", "created_at"].windows(2) {
                assert!(y(fields[1]) - y(fields[0]) <= 46.0);
            }
            assert!(y("Alice") - y("12") <= 46.0);
        }
    }

    #[test]
    fn table_log_is_separate_from_editor_and_query_editor_precedes_results() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.tabs[0].sql = "SELECT '日本';\n\nSELECT 42;\n".into();
        app.tabs[0].results.push(Arc::new(Grid::default()));
        app.prefs.activity_visible = true;
        app.log_activity("SELECT 42;".into());
        let mut render = |view| {
            app.view = view;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 920.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    app.workspace(ctx);
                },
            );
            let mut positions = Vec::new();
            for clipped in output.shapes {
                text_positions(&clipped.shape, &mut positions);
            }
            positions
        };
        let table = render(View::Data);
        assert!(table.iter().any(|(s, _)| s == "SQL activity"));
        assert!(!table.iter().any(|(s, _)| s == "Run current"));
        let query = render(View::Query);
        let y = |text: &str| query.iter().find(|(s, _)| s == text).unwrap().1;
        assert!(y("1") < y("2"));
        assert!(y("2") < y("3"));
        assert!(y("3") < y("4"));
        assert!(y("4") < y("Run current"));
        assert!(y("Run current") < y("Message"));
        assert!(y("Message") < y("SQL activity"));
    }

    #[test]
    fn query_result_footer_centers_counts_and_keeps_controls_separate() {
        for width in [1200.0, 800.0, 640.0, 520.0, 330.0, 260.0] {
            let geometry = query_result_control_geometry(width);
            assert!((geometry.center.center().x - width * 0.5).abs() < 0.01);
            for rect in [geometry.left, geometry.center, geometry.right] {
                assert!(rect.left() >= 0.0 && rect.right() <= width);
                assert!(rect.bottom() + 7.0 <= geometry.height);
            }
            assert!(!geometry.left.intersects(geometry.center));
            assert!(!geometry.left.intersects(geometry.right));
            assert!(!geometry.center.intersects(geometry.right));
        }
    }

    #[test]
    fn query_result_export_fits_panel_content_and_submillisecond_time_is_visible() {
        for width in [400.0, 800.0] {
            let ctx = egui::Context::default();
            let mut app = Rustable::with_preferences(&ctx, Preferences::default());
            app.pending = None;
            app.view = View::Query;
            app.tabs[0].grid = Arc::new(Grid { columns: vec!["id".into()], elapsed: std::time::Duration::from_micros(400), ..Default::default() });
            let output = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 300.0))), ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.query_result_footer(ui));
            });
            let mut texts = Vec::new();
            for clipped in &output.shapes {
                text_positions(&clipped.shape, &mut texts);
                if let egui::Shape::Text(text) = &clipped.shape
                    && text.galley.text() == "Export…" {
                    let right = text.pos.x + text.galley.size().x + 8.0;
                    assert!(right <= clipped.clip_rect.right(), "Export button escaped panel: {right} > {}", clipped.clip_rect.right());
                }
            }
            assert!(texts.iter().any(|(text, _)| text == "Export…"));
            assert!(texts.iter().any(|(text, _)| text == "0.4 ms"));
        }
    }

    #[test]
    fn query_result_message_preserves_the_selected_result() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.view = View::Query;
        app.tabs[0].grid = Arc::new(Grid { columns: vec!["id".into()], rows: vec![vec![Value::Integer(42)]], ..Default::default() });
        app.tabs[0].results = vec![Arc::default(), app.tabs[0].grid.clone()];
        app.tabs[0].result_index = 1;
        app.tabs[0].result_view = ResultView::Message;
        let selected = app.tabs[0].grid.clone();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.query_result_message(ui));
        });
        let mut positions = Vec::new();
        for clipped in output.shapes { text_positions(&clipped.shape, &mut positions); }
        assert!(positions.iter().any(|(text, _)| text == "Statement 2 completed in 0.0 ms."));
        assert!(positions.iter().any(|(text, _)| text == "1 row returned."));
        assert!(Arc::ptr_eq(&selected, &app.tabs[0].grid));
        assert_eq!(app.tabs[0].result_index, 1);
        app.tabs[0].results.clear();
        app.tabs[0].result_error = Some("syntax error".into());
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.query_result_message(ui));
        });
        positions.clear();
        for clipped in output.shapes { text_positions(&clipped.shape, &mut positions); }
        assert!(positions.iter().any(|(text, _)| text == "syntax error"));
        assert!(!positions.iter().any(|(text, _)| text.contains("completed")));
    }

    #[test]
    fn table_controls_with_pending_changes_wrap_without_overlapping_or_clipping() {
        for width in [1200.0, 800.0, 520.0] {
            let ctx = egui::Context::default();
            let mut app = Rustable::with_preferences(&ctx, Preferences::default());
            app.pending = None;
            app.view = View::Data;
            app.prefs.activity_visible = false;
            app.changes.push(Change::Cell {
                table: "t".into(),
                column: "name".into(),
                original: vec![],
                value: Value::Text("pending".into()),
            });
            let mut controls = Vec::new();
            for _ in 0..2 {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 700.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.workspace(ctx),
                );
                controls = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(text)
                            if ["Data", "+ Row", "Rows", "Pending (1)", "Filters", "Export"]
                                .contains(&text.galley.text()) =>
                        {
                            Some((
                                text.galley.text().to_owned(),
                                egui::Rect::from_min_size(text.pos, text.galley.size()),
                            ))
                        }
                        _ => None,
                    })
                    .collect();
            }
            assert_eq!(controls.len(), 6, "width {width}: {controls:?}");
            for (index, (name, rect)) in controls.iter().enumerate() {
                assert!(
                    rect.left() >= 0.0 && rect.right() <= width && rect.bottom() <= 700.0,
                    "{width}: {name} {rect:?}"
                );
                for (other, other_rect) in controls.iter().skip(index + 1) {
                    assert!(
                        !rect.expand(2.0).intersects(other_rect.expand(2.0)),
                        "{width}: {name} overlaps {other}: {rect:?}/{other_rect:?}"
                    );
                }
            }
            if width == 1200.0 {
                let rows: Vec<_> = controls.iter().map(|(_, rect)| rect.center().y).collect();
                assert!(
                    rows.iter().all(|y| (*y - rows[0]).abs() < 5.0),
                    "wide toolbar wrapped: {rows:?}"
                );
            }
        }
    }

    #[test]
    fn table_footer_centers_status_and_anchors_navigation_across_widths() {
        for width in [1280.0, 800.0, 520.0] {
            let ctx = egui::Context::default();
            let mut app = Rustable::with_preferences(&ctx, Preferences::default());
            app.pending = None;
            app.view = View::Data;
            app.prefs.activity_visible = false;
            app.table = Arc::new(Grid {
                columns: vec!["id".into()],
                rows: vec![vec![Value::Integer(1)]],
                total_rows: Some(db::RowTotal {
                    rows: 4_312_244,
                    estimated: true,
                }),
                ..Default::default()
            });
            app.selected = Some(0);
            app.changes.push(Change::Cell {
                table: "t".into(),
                column: "id".into(),
                original: vec![Value::Integer(1)],
                value: Value::Integer(2),
            });
            let expected = "1 of ~4,312,244 rows selected";
            assert_eq!(table_status_label(0, &app.table, 1), expected);
            let mut status_rect = egui::Rect::NOTHING;
            let mut filters_rect = egui::Rect::NOTHING;
            let mut last_button = egui::Rect::NOTHING;
            for _ in 0..2 {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 700.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.workspace(ctx),
                );
                for shape in &output.shapes {
                    match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == expected => {
                            status_rect = egui::Rect::from_min_size(text.pos, text.galley.size())
                        }
                        egui::Shape::Text(text) if text.galley.text() == "Filters" => {
                            filters_rect = egui::Rect::from_min_size(text.pos, text.galley.size())
                        }
                        egui::Shape::Rect(rect)
                            if (rect.rect.width() - 28.0).abs() < 0.1
                                && (rect.rect.height() - 28.0).abs() < 0.1
                                && rect.rect.bottom() > 500.0
                                && (!last_button.is_finite()
                                    || rect.rect.right() > last_button.right()) =>
                        {
                            last_button = rect.rect;
                        }
                        _ => {}
                    }
                }
            }
            assert!(
                status_rect.is_finite(),
                "missing centered status at {width}"
            );
            assert!(
                (status_rect.center().x - width * 0.5).abs() < 12.0,
                "status not centered at {width}: {status_rect:?}"
            );
            assert!(last_button.is_finite(), "missing navigation at {width}");
            assert!(
                width - last_button.right() < 35.0,
                "navigation not right anchored at {width}: {last_button:?}"
            );
            assert!(
                !status_rect.expand(3.0).intersects(filters_rect),
                "status overlaps right controls at {width}"
            );
        }
    }

    #[test]
    fn new_query_uses_available_height_until_it_has_results() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.view = View::Query;
        app.tabs[0].sql.clear();
        app.tabs[0].results.clear();
        app.prefs.activity_visible = false;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 850.0),
                )),
                ..Default::default()
            },
            |ctx| app.workspace(ctx),
        );
        let action_rect = |label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing action {label}"))
        };
        let run = action_rect("Run current");
        let all = action_rect("Run all");
        let favorite = action_rect("Save");
        let file = action_rect("File");
        let status = action_rect("Line 1, column 1");
        assert!(
            (status.center().y - run.center().y).abs() < 2.0,
            "status and actions share a baseline"
        );
        assert!(status.right() < run.left());
        assert!(
            run.right() < all.left()
                && all.right() < favorite.left()
                && favorite.right() < file.left()
        );
        assert!(
            file.right() > 1230.0,
            "query actions must align with the right edge: {file:?}"
        );
        let mut positions = Vec::new();
        for shape in output.shapes {
            text_positions(&shape.shape, &mut positions);
        }
        assert!(!positions.iter().any(|(text, _)| text == "Message"));
        let run_y = positions
            .iter()
            .find(|(text, _)| text == "Run current")
            .unwrap()
            .1;
        assert!(run_y > 650.0, "editor left unused vertical space: {run_y}");
        assert!(run_y < 850.0, "run controls escaped viewport: {run_y}");
    }

    #[test]
    fn results_do_not_grow_editor_on_successive_frames() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.view = View::Query;
        app.prefs.activity_visible = true;
        app.tabs[0].results.push(Arc::new(Grid::default()));
        let mut result_y = Vec::new();
        for _ in 0..30 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 920.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    app.workspace(ctx);
                },
            );
            let mut positions = Vec::new();
            for clipped in output.shapes {
                text_positions(&clipped.shape, &mut positions);
            }
            let y = positions
                .iter()
                .find(|(s, _)| s == "Run current")
                .unwrap()
                .1;
            result_y.push(y);
        }
        assert!(
            (result_y[29] - result_y[2]).abs() < 1.0,
            "editor grew: {result_y:?}"
        );
        assert!(result_y[29] < 500.0, "results crowded out: {result_y:?}");
    }

    #[test]
    fn activity_panel_keeps_dragged_height_after_release_and_content_changes() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.prefs.activity_visible = true;
        let id = egui::Id::new((egui::ViewportId::ROOT, "activity-log-pane"));
        let frame = |app: &mut Rustable, events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.activity_panel(ui));
                },
            );
            egui::containers::panel::PanelState::load(&ctx, id)
                .unwrap()
                .rect
        };
        let initial = frame(&mut app, vec![]);
        assert!(app.prefs.pane_sizes.activity.is_none());
        let handle = initial.center_top();
        frame(&mut app, vec![egui::Event::PointerMoved(handle)]);
        frame(
            &mut app,
            vec![egui::Event::PointerButton {
                pos: handle,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let target = handle - egui::vec2(0.0, 120.0);
        frame(&mut app, vec![egui::Event::PointerMoved(target)]);
        let dragged = frame(&mut app, vec![]);
        assert!(dragged.height() > initial.height() + 100.0);
        assert_eq!(app.prefs.pane_sizes.activity, Some(dragged.height()));
        let encoded = serde_json::to_string(&app.prefs).unwrap();
        let restored: Preferences = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored.pane_sizes.activity, Some(dragged.height()));
        frame(
            &mut app,
            vec![egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        for content in [Some("A short entry"), None, Some("Another entry")] {
            app.activity.clear();
            if let Some(content) = content {
                app.log_activity(content.into());
            }
            for _ in 0..3 {
                let settled = frame(&mut app, vec![]);
                assert!(
                    (settled.height() - dragged.height()).abs() < 1.0,
                    "resized panel snapped from {} to {}",
                    dragged.height(),
                    settled.height()
                );
            }
        }
    }

    #[test]
    fn activity_is_bounded_and_connection_settings_are_independent() {
        let text = "🙂".repeat(4000);
        let clipped = bounded_log_text(&text);
        assert!(clipped.len() < 8250);
        assert!(clipped.ends_with("[log truncated]"));
        assert!(activity_description(&Command::RefreshSchema).is_none());
        assert!(activity_description(&Command::SelectSchema("public".into())).is_none());
        assert!(activity_description(&Command::Open(None)).is_none());
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        for i in 0..300 {
            app.log_activity(format!("entry {i}"));
        }
        assert_eq!(app.activity.len(), 200);
        assert!(app.activity.back().unwrap().ends_with("entry 299"));
        app.prefs.connection_styles.insert(
            "a.sqlite".into(),
            ConnectionStyle {
                environment: "Production".into(),
                color: [180, 0, 0],
            },
        );
        assert_eq!(
            app.prefs
                .connection_styles
                .get("a.sqlite")
                .unwrap()
                .environment,
            "Production"
        );
        assert!(!app.prefs.connection_styles.contains_key("b.sqlite"));
    }
}

#[cfg(test)]
mod find_editor_tests {
    use super::*;
    fn frame(ctx: &egui::Context, app: &mut Rustable, events: Vec<egui::Event>) {
        let _ = ctx.run(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.query_editor(ui));
            },
        );
    }
    #[test]
    fn replace_all_is_one_editor_undo_step() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.tabs[0].sql = "SELECT '日本', '日本';".into();
        frame(&ctx, &mut app, vec![]);
        app.tabs[0].search.needle = "日本".into();
        app.tabs[0].search.replacement = "🙂".into();
        app.tabs[0].search.action = Some(crate::find::Action::All);
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.tabs[0].sql, "SELECT '🙂', '🙂';");
        frame(&ctx, &mut app, vec![]);
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Z,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
        );
        assert_eq!(app.tabs[0].sql, "SELECT '日本', '日本';");
    }
}

#[cfg(test)]
mod review_recovery_tests {
    use super::*;
    fn text_rect(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(s) if s.galley.job.text == text => {
                Some(egui::Rect::from_min_size(s.pos, s.galley.size()))
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_rect(s, text)),
            _ => None,
        }
    }
    #[test]
    fn applying_review_exposes_its_own_cancel_action() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.connections = false;
        app.review = Some(ChangePlan::default());
        app.pending = Some(Pending::Apply);
        let mut output = None;
        for _ in 0..2 {
            output = Some(ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 920.0))),
                ..Default::default()
            }, |ctx| app.windows(ctx)));
        }
        assert!(output.unwrap().shapes.iter().any(|s| text_rect(&s.shape, "Cancel save").is_some()));
    }
    #[test]
    fn failed_review_shows_error_and_returns_without_losing_pending_values() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.connections = false;
        app.error = Some("UNIQUE constraint failed: customers.email".into());
        app.review = Some(db::ChangePlan::default());
        app.changes.push(Change::InsertRow {
            id: 998,
            table: "customers".into(),
            fields: vec![("email".into(), Value::Text("duplicate@example.test".into()))],
        });
        let mut frame = |events| {
            ctx.run(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 920.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.windows(ctx),
            )
        };
        frame(vec![]);
        let output = frame(vec![]);
        assert!(
            output.shapes.iter().any(|s| text_rect(
                &s.shape,
                "UNIQUE constraint failed: customers.email"
            )
            .is_some())
        );
        let pos = output
            .shapes
            .iter()
            .find_map(|s| text_rect(&s.shape, "Back to editing"))
            .unwrap()
            .center();
        frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(app.review.is_none());
        assert!(
            matches!(&app.changes[0], Change::InsertRow { fields, .. } if fields[0].1 == Value::Text("duplicate@example.test".into()))
        );
        assert!(app.pending.is_none());
    }
}

#[cfg(test)]
mod tab_lifecycle_tests {
    use super::*;
    use eframe::Storage;
    #[derive(Default)]
    struct MemoryStorage(std::collections::HashMap<String, String>);
    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn flush(&mut self) {}
    }

    #[test]
    fn session_roundtrip_keeps_dirty_baseline_and_reopenable_documents() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("draft.sql");
        std::fs::write(&path, "SELECT 1;").unwrap();
        let mut tab = QueryTab::new("SELECT 2;".into());
        tab.saved(path.clone(), "SELECT 1;".into());
        app.tabs = vec![tab, QueryTab::new("SELECT 'closed';".into())];
        app.close_tab(1);
        let mut storage = MemoryStorage::default();
        eframe::App::save(&mut app, &mut storage);
        let prefs: Preferences = eframe::get_value(&storage, "workspace").unwrap();
        assert!(prefs.queries.is_empty());
        let mut restored = Rustable::with_preferences(&ctx, prefs);
        assert_eq!(restored.tabs[0].source_path.as_ref(), Some(&path));
        assert_eq!(restored.tabs[0].sql, "SELECT 2;");
        assert!(restored.tabs[0].dirty);
        let baseline = restored.tabs[0].saved_sql.as_ref().unwrap();
        assert_eq!(baseline.as_str(), "SELECT 1;");
        std::fs::write(&path, "SELECT 'external';").unwrap();
        assert!(
            crate::script_file::write_checked(
                &path,
                &restored.tabs[0].sql,
                Some(baseline),
                &std::sync::atomic::AtomicBool::new(false)
            )
            .is_err()
        );
        restored.reopen_tab();
        assert_eq!(restored.tabs[restored.tab].sql, "SELECT 'closed';");
    }

    #[test]
    fn new_tab_preserves_closed_last_document_across_restart() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        let path = PathBuf::from("last-draft.sql");
        let mut tab = QueryTab::new("SELECT 'unsaved';".into());
        tab.saved(path.clone(), "SELECT 'saved';".into());
        app.tabs = vec![tab];
        app.close_tab(0);
        app.new_tab();
        assert_eq!(app.tabs.len(), 1);
        assert!(app.tabs[0].sql.is_empty());
        assert_eq!(app.closed_tabs.len(), 1);
        let mut storage = MemoryStorage::default();
        eframe::App::save(&mut app, &mut storage);
        let prefs = eframe::get_value(&storage, "workspace").unwrap();
        let mut restored = Rustable::with_preferences(&ctx, prefs);
        restored.reopen_tab();
        let recovered = &restored.tabs[restored.tab];
        assert!(recovered.open);
        assert!(recovered.dirty);
        assert_eq!(recovered.source_path.as_ref(), Some(&path));
        assert_eq!(recovered.sql, "SELECT 'unsaved';");
        assert_eq!(recovered.saved_sql.as_deref().unwrap(), "SELECT 'saved';");
        assert!(restored.closed_tabs.is_empty());
    }

    #[test]
    fn legacy_sessions_migrate_and_clean_files_do_not_duplicate_baselines() {
        let ctx = egui::Context::default();
        let mut storage = MemoryStorage::default();
        // Actual legacy RON shape: new fields must deserialize with defaults.
        storage.set_string("workspace", "(queries:[\"SELECT 7;\"])".into());
        let prefs: Preferences = eframe::get_value(&storage, "workspace").unwrap();
        let mut app = Rustable::with_preferences(&ctx, prefs);
        assert_eq!(app.tabs[0].sql, "SELECT 7;");
        app.tabs[0].saved(PathBuf::from("clean.sql"), "SELECT 7;".into());
        app.tabs[0].open = false;
        eframe::App::save(&mut app, &mut storage);
        let prefs: Preferences = eframe::get_value(&storage, "workspace").unwrap();
        assert_eq!(prefs.documents.len(), 1);
        assert!(prefs.documents[0].saved_sql.is_none());
        let restored = Rustable::with_preferences(&ctx, prefs);
        assert!(!restored.tabs[0].open);
        assert!(!restored.tabs[0].dirty);
        assert_eq!(
            restored.tabs[0].source_path,
            Some(PathBuf::from("clean.sql"))
        );
        assert_eq!(restored.tabs[0].saved_sql.as_deref().unwrap(), "SELECT 7;");
    }

    #[test]
    fn script_save_snapshot_and_reopen_preserve_unsaved_edits() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        let mut tab = QueryTab::new("SELECT 1;".into());
        let id = tab.id;
        let path = PathBuf::from("query.sql");
        tab.saved(path.clone(), "SELECT 1;".into());
        assert!(!tab.dirty);
        tab.sql = "SELECT 2;".into();
        tab.refresh_dirty();
        assert!(tab.dirty);
        // A save finishing after another edit only marks its captured snapshot saved.
        tab.saved(path.clone(), "SELECT 1;".into());
        assert!(tab.dirty);
        tab.sql = "SELECT 1;".into();
        tab.refresh_dirty();
        assert!(!tab.dirty);
        app.tabs.push(tab);
        app.close_tab(1);
        app.reopen_tab();
        let tab = &app.tabs[app.tab];
        assert_eq!(tab.id, id);
        assert_eq!(tab.source_path, Some(path));
        assert_eq!(tab.saved_sql.as_deref().unwrap(), "SELECT 1;");
    }
    #[test]
    fn tab_switch_stages_valid_draft_but_keeps_invalid_draft_visible() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.view = View::Data;
        app.edit = Some(CellEdit {
            bulk_rows: vec![],
            bulk_changed: false,
            insert_id: None,
            advance: 0,
            table: "customers".into(),
            row: vec![Value::Integer(1)],
            original_value: Value::Integer(1),
            column: "id".into(),
            text: "invalid integer".into(),
            kind: 1,
            focus: false,
            inspector: true,
        });
        let count = app.tabs.len();
        app.new_tab();
        assert_eq!(app.tabs.len(), count);
        app.show_query();
        assert!(app.view == View::Data);
        assert!(app.edit.as_ref().unwrap().focus);
        assert!(app.changes.is_empty());
        app.edit.as_mut().unwrap().text = "42".into();
        app.show_query();
        assert!(app.view == View::Query);
        assert!(app.edit.is_none());
        assert!(
            matches!(&app.changes[0], Change::Cell { value: Value::Integer(42), table, .. } if table == "customers")
        );
    }

    #[test]
    fn inactive_tab_close_preserves_active_identity_and_row_selection() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.tabs = vec![
            QueryTab::new("one".into()),
            QueryTab::new("two".into()),
            QueryTab::new("three".into()),
        ];
        app.activate_query(1);
        let active_id = app.tabs[1].id;
        app.selected = Some(7);
        app.close_tab(0);
        assert_eq!(app.tabs[app.tab].id, active_id);
        assert_eq!(app.selected, Some(7));
        app.close_tab(1);
        assert_eq!(app.tabs[app.tab].id, active_id);
        assert_eq!(app.selected, Some(7));
        app.close_tab(0);
        assert_eq!(app.selected, None);
        assert!(app.view == View::Data);
    }

    #[test]
    fn editor_activation_clears_selection_only_when_changing_source() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.selected = Some(12);
        app.show_query();
        assert_eq!(app.selected, None);
        app.selected = Some(2);
        app.show_query();
        assert_eq!(app.selected, Some(2));
        app.browse.table = "customers".into();
        app.close_table_tab();
        assert_eq!(app.selected, Some(2));
        assert!(app.view == View::Query);
        app.new_tab();
        assert_eq!(app.selected, None);
        app.selected = Some(3);
        app.activate_query(0);
        assert_eq!(app.selected, None);
        assert!(app.tabs[0].focus_editor);
    }

    #[test]
    fn closing_last_query_hides_it_and_editor_shortcut_restores_draft() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.tabs = vec![QueryTab::new("SELECT 42;".into())];
        app.tab = 0;
        app.show_query();
        app.close_tab(0);
        assert!(!app.tabs[0].open);
        assert!(app.view == View::Data);
        app.show_query();
        assert!(app.tabs[0].open);
        assert_eq!(app.tabs[0].sql, "SELECT 42;");
        app.browse.table = "customers".into();
        app.view = View::Data;
        app.close_table_tab();
        assert!(app.browse.table.is_empty());
        assert!(app.view == View::Query);
        app.close_tab(0);
        app.new_tab();
        assert_eq!(app.tabs.len(), 1);
        assert!(app.tabs[0].sql.is_empty());
    }
}

#[cfg(test)]
mod close_workspace_keyboard_tests {
    use super::*;

    fn command_w(ctx: &egui::Context, app: &mut Rustable) -> bool {
        let output = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::W,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }],
                ..Default::default()
            },
            |ctx| app.workspace_shortcuts(ctx),
        );
        output.viewport_output.values().any(|viewport| {
            viewport
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Close))
        })
    }

    #[test]
    fn command_w_closes_tabs_then_only_requests_window_close_when_empty() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.tabs = vec![
            QueryTab::new("SELECT 1;".into()),
            QueryTab::new("SELECT 2;".into()),
        ];
        app.tab = 1;
        app.view = View::Query;
        app.browse.table = "customers".into();
        assert!(!command_w(&ctx, &mut app));
        assert_eq!(app.closed_tabs[0].sql, "SELECT 2;");
        assert!(!command_w(&ctx, &mut app));
        assert!(!app.tabs[0].open);
        assert!(!command_w(&ctx, &mut app));
        assert!(app.browse.table.is_empty());
        assert!(app.selected.is_none());
        assert!(command_w(&ctx, &mut app));
        let recovered = app.recovery_documents();
        assert!(recovered.iter().any(|document| document.sql == "SELECT 1;"));
        assert!(recovered.iter().any(|document| document.sql == "SELECT 2;"));
    }

    #[test]
    fn command_w_empty_workspace_does_not_close_running_or_staged_session() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.tabs[0].open = false;
        app.view = View::Data;
        app.browse.table.clear();
        app.pending = Some(Pending::Query(0));
        assert!(!command_w(&ctx, &mut app));
        app.pending = None;
        app.changes.push(Change::Cell {
            table: "customers".into(),
            column: "name".into(),
            original: vec![],
            value: Value::Text("pending".into()),
        });
        assert!(!command_w(&ctx, &mut app));
        assert_eq!(app.changes.len(), 1);
        app.changes.clear();
        assert!(command_w(&ctx, &mut app));
    }
}

#[cfg(test)]
mod csv_review_keyboard_tests {
    use super::*;
    fn wait_for_worker(app: &mut Rustable) {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while app.pending.is_some() {
            assert!(Instant::now() < deadline, "worker timed out");
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    fn setup(ctx: &egui::Context) -> Rustable {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, b"id\n1\n").unwrap();
        let preview = db::csv_preview::load(
            file.path(),
            Default::default(),
            None,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        let mut app = Rustable::with_preferences(ctx, Preferences::default());
        wait_for_worker(&mut app);
        app.csv_review = Some((
            db::csv_import::Request {
                source: preview.source.clone().unwrap(),
                options: preview.options,
                table: "test".into(),
                columns: vec![],
                destinations: vec![],
                empty_as_null: false,
                postgres_inserts: false,
            },
            vec![],
        ));
        app.csv_preview = Some(preview);
        app
    }
    fn key(ctx: &egui::Context, app: &mut Rustable, key: egui::Key, modifiers: egui::Modifiers) {
        let _ = ctx.run(
            egui::RawInput {
                modifiers: egui::Modifiers::NONE,
                events: vec![
                    egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    },
                    egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ctx| {
                app.workspace_shortcuts(ctx);
                app.csv_import_review(ctx);
            },
        );
    }
    #[test]
    fn review_owns_shortcuts_and_escape_preserves_mapping() {
        let ctx = egui::Context::default();
        let mut app = setup(&ctx);
        let tabs = app.tabs.len();
        app.safe = true; // Exercise the internal read-only guard explicitly.
        for k in [
            egui::Key::T,
            egui::Key::W,
            egui::Key::E,
            egui::Key::Enter,
            egui::Key::S,
        ] {
            key(&ctx, &mut app, k, egui::Modifiers::COMMAND);
            assert!(app.csv_review.is_some()); // Safe mode blocks confirmation.
            assert!(app.pending.is_none());
            assert_eq!(app.tabs.len(), tabs);
        }
        app.safe = false;
        key(
            &ctx,
            &mut app,
            egui::Key::Enter,
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        );
        assert!(app.csv_review.is_some());
        assert!(app.pending.is_none());
        key(&ctx, &mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(app.csv_review.is_none());
        assert!(app.csv_preview.is_some());
    }
    #[test]
    fn confirmation_obeys_busy_guard_and_mac_command_chord() {
        for k in [egui::Key::Enter, egui::Key::S] {
            let ctx = egui::Context::default();
            let mut app = setup(&ctx);
            app.safe = false;
            app.pending = Some(Pending::Query(0));
            let modifiers = egui::Modifiers {
                command: true,
                mac_cmd: true,
                ..Default::default()
            };
            key(&ctx, &mut app, k, modifiers);
            assert!(app.csv_review.is_some());
            app.pending = None;
            key(&ctx, &mut app, k, modifiers);
            assert!(app.csv_review.is_some());
            assert!(matches!(app.pending, Some(Pending::Import)));
            key(&ctx, &mut app, egui::Key::Escape, egui::Modifiers::NONE);
            assert!(app.csv_review.is_some());
        }
    }
    #[test]
    fn failed_import_retains_review_and_renders_error_above_mapping() {
        let ctx = egui::Context::default();
        let mut app = setup(&ctx);
        app.safe = false;
        // The real worker rejects this deliberately unmapped request.
        app.commit_csv_import();
        wait_for_worker(&mut app);
        assert!(
            app.error
                .as_ref()
                .unwrap()
                .contains("Choose at least one destination column")
        );
        assert!(app.csv_preview.is_some());
        assert!(app.csv_review.is_some());
        let _ = ctx.run(Default::default(), |ctx| app.csv_import_review(ctx));
        let output = ctx.run(Default::default(), |ctx| app.csv_import_review(ctx));
        fn contains(shape: &egui::Shape, needle: &str) -> bool {
            match shape {
                egui::Shape::Text(text) => text.galley.job.text.contains(needle),
                egui::Shape::Vec(shapes) => shapes.iter().any(|s| contains(s, needle)),
                _ => false,
            }
        }
        assert!(
            output
                .shapes
                .iter()
                .any(|s| contains(&s.shape, "Choose at least one destination column"))
        );
        key(&ctx, &mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(app.csv_review.is_none());
        assert!(app.csv_preview.is_some());
    }
}

#[cfg(test)]
mod refresh_shortcut_tests {
    use super::*;
    #[test]
    fn empty_table_workspace_refreshes_schema() {
        let mut app = Rustable::with_preferences(&egui::Context::default(), Preferences::default());
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while app.pending.is_some() && Instant::now() < deadline {
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.pending.is_none());
        app.view = View::Data;
        app.browse.table.clear();
        app.refresh_current_view();
        assert!(matches!(app.pending, Some(Pending::Export)));
        assert!(app.browse.table.is_empty());
    }

    #[test]
    fn refresh_preserves_filter_sort_and_page() {
        let mut app = Rustable::with_preferences(&egui::Context::default(), Preferences::default());
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while app.pending.is_some() && Instant::now() < deadline {
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.pending.is_none());
        app.browse = Browse {
            table: "customers".into(),
            filter_column: Some("name".into()),
            filter: "Avery".into(),
            sort: Some(("id".into(), true)),
            offset: 500,
        };
        let browse = app.browse.clone();
        app.view = View::Data;
        app.refresh_current_view();
        assert_eq!(app.browse, browse);
        assert!(matches!(app.pending, Some(Pending::Table)));
    }
}

#[cfg(test)]
mod session_window_tests {
    use super::*;
    use crate::session_windows::{WindowConnection, WindowRequest};
    #[test]
    fn recovery_omits_blank_scratch_tabs_but_preserves_empty_file_edits() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_launcher_preferences(&ctx, Preferences::default());
        app.tabs = vec![QueryTab::new(String::new()), QueryTab::new("  \n".into()), QueryTab::new("SELECT 42;".into())];
        let mut file = QueryTab::new(String::new());
        file.saved(PathBuf::from("/tmp/recovery-empty-file.sql"), "SELECT 1;".into());
        app.tabs.push(file);
        let documents = app.recovery_documents();
        assert_eq!(documents.len(), 2);
        assert_eq!(documents[1].saved_sql.as_deref(), Some("SELECT 1;"));
        let mut storage = Storage::default();
        let mut root = Rustable::with_launcher_preferences(&ctx, Preferences::default());
        let mut legacy = documents;
        legacy.push(QueryDocument::default());
        root.save_with_window_documents(&mut storage, legacy);
        let mut prefs: Preferences = eframe::get_value(&storage, "workspace").unwrap();
        assert_eq!(prefs.window_documents.len(), 2);
        prefs.window_documents.push(QueryDocument::default()); // Older app snapshots.
        let restored = Rustable::with_launcher_preferences(&ctx, prefs);
        assert_eq!(restored.window_recovery.len(), 2);
        assert_eq!(restored.window_recovery[0].sql, "SELECT 42;");
        assert_eq!(restored.window_recovery[1].source_path.as_deref(), Some(std::path::Path::new("/tmp/recovery-empty-file.sql")));
        assert!(restored.window_recovery[1].dirty);
        assert!(restored.pending.is_none());
    }

    #[test]
    fn launcher_starts_disconnected_and_preserves_legacy_documents_for_explicit_recovery() {
        let ctx = egui::Context::default();
        let fresh = Rustable::with_launcher_preferences(&ctx, Preferences::default());
        assert!(fresh.window_recovery.is_empty());
        assert!(fresh.tabs.iter().all(|tab| tab.sql.is_empty()));
        assert!(fresh.pending.is_none());
        let mut launcher = Rustable::with_launcher_preferences(
            &ctx,
            Preferences {
                documents: vec![QueryDocument {
                    sql: "DROP TABLE customers;".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        assert!(launcher.pending.is_none());
        assert!(!launcher.has_connection());
        assert!(launcher.objects.is_empty());
        assert!(launcher.activity.is_empty());
        assert_eq!(launcher.window_recovery.len(), 1);
        assert!(launcher.connections);
        launcher.recover_next_window = true;
        launcher.window_requests.push(WindowRequest {
            title: "Demo".into(),
            recover_sql: false,
            identity: None,
            connection: WindowConnection::Demo,
        });
        let request = launcher.take_window_requests().pop().unwrap();
        assert!(request.recover_sql);
        assert!(!launcher.recover_next_window);
        let mut child = launcher.new_window(&ctx, request);
        wait(&mut child);
        assert!(launcher.window_recovery.is_empty());
        assert!(!launcher.has_connection());
        assert!(
            child
                .objects
                .iter()
                .any(|object| object.name == "customers")
        );
        assert_eq!(child.tabs[0].sql, "DROP TABLE customers;");
        assert!(child.view == View::Query);
        assert!(
            child.prefs.history.is_empty(),
            "Recovery must not run the restored SQL"
        );
        launcher.archive_window(child);
        assert_eq!(launcher.window_recovery.len(), 1);
        let mut storage = Storage::default();
        launcher.save_with_window_documents(&mut storage, Vec::new());
        let restored: Preferences = eframe::get_value(&storage, "workspace").unwrap();
        let restored = Rustable::with_launcher_preferences(&ctx, restored);
        assert_eq!(restored.window_recovery[0].sql, "DROP TABLE customers;");
        assert!(restored.pending.is_none());
    }

    #[test]
    fn command_n_requests_the_launcher_from_child_without_touching_its_session() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_launcher_preferences(&ctx, Preferences::default());
        app.child_window = true;
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::N,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }],
                ..Default::default()
            },
            |ctx| app.manager_shortcut(ctx),
        );
        assert!(app.take_manager_request());
        assert!(app.pending.is_none());
        assert!(app.window_requests.is_empty());
    }

    #[test]
    fn repeated_launcher_opens_create_distinct_sessions_for_the_same_database() {
        let ctx = egui::Context::default();
        let mut launcher = Rustable::with_launcher_preferences(&ctx, Preferences::default());
        let file = tempfile::NamedTempFile::new().unwrap();
        for _ in 0..2 {
            launcher.window_requests.push(WindowRequest {
                title: "Repeated".into(),
                recover_sql: false,
                identity: None,
                connection: WindowConnection::Sqlite(file.path().into()),
            });
        }
        let requests = launcher.take_window_requests();
        assert_eq!(requests.len(), 2);
        let mut children: Vec<_> = requests
            .into_iter()
            .map(|request| launcher.new_window(&ctx, request))
            .collect();
        for child in &mut children {
            wait(child);
        }
        assert_ne!(children[0].tabs[0].id, children[1].tabs[0].id);
        children[0].tabs[0].sql = "SELECT 'first window';".into();
        assert!(children[1].tabs[0].sql.is_empty());
        assert!(!launcher.has_connection());
        assert!(launcher.pending.is_none());
    }
    #[derive(Default)]
    struct Storage(std::collections::HashMap<String, String>);
    impl eframe::Storage for Storage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn flush(&mut self) {}
    }
    fn wait(app: &mut Rustable) {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while app.pending.is_some() && Instant::now() < deadline {
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.pending.is_none(), "worker did not complete");
    }
    #[test]
    fn shared_theme_favorites_and_recent_history_publish_without_resurrection() {
        let ctx = egui::Context::default();
        let mut root = Rustable::with_preferences(&ctx, Preferences::default());
        let mut child = Rustable::with_preferences(&ctx, Preferences::default());
        let mut sibling = Rustable::with_preferences(&ctx, Preferences::default());
        root.prefs.history = (0..100).map(|i| format!("old {i}")).collect();
        root.shared_preferences_dirty = true;
        assert!(root.publish_shared_preferences());
        root.sync_window_preferences(&mut child);
        child.prefs.appearance = Appearance::Dark;
        child.prefs.favorites.push("SELECT 'favorite'".into());
        child.prefs.history.insert(0, "SELECT 'latest'".into());
        child.shared_preferences_dirty = true;
        assert!(root.merge_window_preferences(&mut child));
        root.sync_window_preferences(&mut sibling);
        assert_eq!(root.prefs.appearance, Appearance::Dark);
        assert_eq!(sibling.prefs.appearance, Appearance::Dark);
        assert_eq!(sibling.prefs.favorites, child.prefs.favorites);
        assert_eq!(root.prefs.history.len(), 100);
        assert_eq!(root.prefs.history[0], "SELECT 'latest'");
        assert!(!root.prefs.history.contains(&"old 99".into()));
        child.prefs.favorites.clear();
        child.shared_preferences_dirty = true;
        root.merge_window_preferences(&mut child);
        // Stale sibling snapshot cannot resurrect removed favorites during save/close.
        assert!(!root.merge_window_preferences(&mut sibling));
        assert!(root.prefs.favorites.is_empty());
        root.sync_window_preferences(&mut sibling);
        assert!(sibling.prefs.favorites.is_empty());
        let file = tempfile::NamedTempFile::new().unwrap();
        let new_child = root.new_window(
            &ctx,
            WindowRequest {
                recover_sql: false,
                identity: None,
                title: "New".into(),
                connection: WindowConnection::Sqlite(file.path().into()),
            },
        );
        assert_eq!(new_child.prefs.appearance, Appearance::Dark);
        let mut storage = Storage::default();
        root.save_with_window_documents(&mut storage, vec![]);
        let prefs: Preferences = eframe::get_value(&storage, "workspace").unwrap();
        assert_eq!(prefs.appearance, Appearance::Dark);
        assert!(prefs.favorites.is_empty());
        assert_eq!(prefs.history[0], "SELECT 'latest'");
    }
    #[test]
    fn windows_have_independent_connections_and_recover_drafts_without_profile_overwrite() {
        let ctx = egui::Context::default();
        let mut root = Rustable::with_preferences(&ctx, Preferences::default());
        wait(&mut root);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("child.sqlite");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE child_only(id INTEGER);")
            .unwrap();
        let mut child = root.new_window(
            &ctx,
            WindowRequest {
                recover_sql: false,
                identity: None,
                title: "Child".into(),
                connection: WindowConnection::Sqlite(path),
            },
        );
        assert!(child.close_blocked());
        wait(&mut child);
        assert!(!child.close_blocked());
        assert!(root.objects.iter().any(|object| object.name == "customers"));
        assert!(
            !root
                .objects
                .iter()
                .any(|object| object.name == "child_only")
        );
        assert!(
            child
                .objects
                .iter()
                .any(|object| object.name == "child_only")
        );
        assert!(
            !child
                .objects
                .iter()
                .any(|object| object.name == "customers")
        );
        assert_ne!(root.tabs[0].id, child.tabs[0].id);
        child.tabs[0].saved("/tmp/window-query.sql".into(), "SELECT 1;".into());
        child.tabs[0].sql = "SELECT 'recover this';".into();
        child.tabs[0].refresh_dirty();
        let mut storage = Storage::default();
        root.save_with_window_documents(&mut storage, child.recovery_documents());
        assert!(root.prefs.window_documents.is_empty());
        let preferences: Preferences = eframe::get_value(&storage, "workspace").unwrap();
        assert_eq!(preferences.window_documents.len(), 1);
        assert_eq!(
            preferences.window_documents[0].saved_sql.as_deref(),
            Some("SELECT 1;")
        );
        let mut restored = Rustable::with_preferences(&ctx, preferences);
        wait(&mut restored);
        assert!(restored.window_recovery[0].dirty);
        assert_eq!(restored.window_recovery[0].sql, "SELECT 'recover this';");
        root.archive_window(child);
        assert_eq!(root.window_recovery.len(), 1);
        root.reopen_tab();
        assert!(root.window_recovery.is_empty());
        assert_eq!(root.tabs[root.tab].sql, "SELECT 'recover this';");
        assert!(root.tabs[root.tab].dirty);
    }
    #[test]
    fn child_manager_requests_are_routed_to_root_without_creating_local_profiles() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.child_window = true;
        app.connections = true;
        app.connections_window(&ctx);
        assert!(!app.connections);
        assert!(app.take_manager_request());
        assert!(!app.take_manager_request());
        assert!(app.prefs.saved_connections.profiles.is_empty());
    }
}

#[cfg(test)]
mod schema_display_tests {
    use super::*;

    fn inline_rename_app() -> Rustable {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_launcher_preferences(&ctx, Preferences::default());
        app.engine = "SQLite".into();
        app.safe = false;
        app.browse.table = "customers".into();
        app.objects.push(db::Object {
            name: "customers".into(),
            kind: "table".into(),
            schema: None,
            display_name: None,
            metadata_error: None,
            columns: vec![],
            ddl: String::new(),
        });
        app
    }

    #[test]
    fn inline_nullability_preserves_other_alterations_and_can_revert() {
        let mut app = inline_rename_app();
        app.columns.push(Column {
            name: "name".into(),
            data_type: "TEXT".into(),
            not_null: false,
            default: String::new(),
            pk: false,
            generated: false,
        });
        let original = Change::AlterColumn {
            table: "customers".into(),
            column: "name".into(),
            data_type: Some("VARCHAR(100)".into()),
            not_null: None,
            default: Some(Some(Value::Text("Guest".into()))),
        };
        app.changes.push(original.clone());
        app.stage_column_nullability(0, true);
        assert_eq!(app.changes.len(), 1);
        assert!(
            matches!(&app.changes[0], Change::AlterColumn { data_type: Some(t), not_null: Some(true), default: Some(Some(Value::Text(d))), .. } if t == "VARCHAR(100)" && d == "Guest")
        );
        app.stage_column_nullability(0, false);
        assert_eq!(app.changes, vec![original]);
        app.changes.clear();
        app.stage_column_nullability(0, true);
        assert_eq!(app.changes.len(), 1);
        app.stage_column_nullability(0, false);
        assert!(app.changes.is_empty());
    }

    #[test]
    fn inline_nullability_respects_safe_mode_and_protected_columns() {
        let mut app = inline_rename_app();
        app.columns.push(Column {
            name: "id".into(),
            data_type: "INTEGER".into(),
            not_null: false,
            default: String::new(),
            pk: true,
            generated: false,
        });
        app.stage_column_nullability(0, true);
        assert!(app.changes.is_empty());
        app.columns[0].pk = false;
        app.columns[0].generated = true;
        app.stage_column_nullability(0, true);
        assert!(app.changes.is_empty());
        app.columns[0].generated = false;
        app.safe = true;
        app.stage_column_nullability(0, true);
        assert!(app.changes.is_empty());
        app.safe = false;
        app.stage_column_nullability(0, true);
        assert_eq!(app.changes.len(), 1);
    }

    #[test]
    fn escape_cancels_inline_name_without_staging() {
        let ctx = egui::Context::default();
        let mut app = inline_rename_app();
        app.columns.push(Column {
            name: "name".into(),
            data_type: "TEXT".into(),
            not_null: false,
            default: String::new(),
            pk: false,
            generated: false,
        });
        app.rename_inline = true;
        app.rename_focus = true;
        app.rename = Some(("name".into(), "draft_name".into()));
        let mut frame = |events| {
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.structure(ui));
                },
            );
        };
        frame(vec![]);
        frame(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(app.rename.is_none());
        assert!(app.changes.is_empty());
    }

    #[test]
    fn inline_rename_revises_and_reverts_one_staged_action() {
        let mut app = inline_rename_app();
        for name in ["display name", "customer_name", "name"] {
            app.rename_inline = true;
            app.rename = Some(("name".into(), name.into()));
            assert!(app.finish_cell_edit()); // Save and table switches use this path too.
            assert!(app.rename.is_none());
            assert!(!app.rename_inline);
            if name == "name" {
                assert!(app.changes.is_empty());
            } else {
                assert_eq!(
                    app.changes,
                    vec![Change::RenameColumn {
                        table: "customers".into(),
                        from: "name".into(),
                        to: name.into()
                    }]
                );
            }
        }
    }

    #[test]
    fn inline_rename_preserves_invalid_drafts_and_obeys_safe_mode() {
        let mut app = inline_rename_app();
        for name in ["", "   ", "invalid\0name"] {
            app.rename_inline = true;
            app.rename = Some(("name".into(), name.into()));
            assert!(!app.finish_cell_edit());
            assert!(app.rename.is_some());
            assert!(app.changes.is_empty());
        }
        app.rename = Some(("name".into(), "valid".into()));
        app.safe = true;
        assert!(!app.finish_cell_edit());
        assert!(app.changes.is_empty());
        app.safe = false;
        assert!(app.finish_cell_edit());
        assert_eq!(app.changes.len(), 1);
    }

    #[test]
    fn clicking_column_name_selects_structure_row_on_press() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_launcher_preferences(&ctx, Preferences::default());
        app.columns = vec![Column {
            name: "click_column_name".into(),
            data_type: "text".into(),
            not_null: false,
            default: String::new(),
            pk: false,
            generated: false,
        }];
        let frame = |app: &mut Rustable, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 500.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.structure(ui));
                },
            )
        };
        frame(&mut app, vec![]);
        let output = frame(&mut app, vec![]);
        let position = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.text() == "click_column_name"
                {
                    Some(text.pos + egui::vec2(5.0, 5.0))
                } else {
                    None
                }
            })
            .expect("Column name must be painted");
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert_eq!(app.selected_column, Some(0));
        frame(
            &mut app,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(app.selected_column, Some(0));
    }

    #[test]
    fn schema_actions_require_a_supported_table_not_a_view_or_unknown_target() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_launcher_preferences(&ctx, Preferences::default());
        app.browse.table = "\"public\".\"no_primary_key\"".into();
        app.objects = vec![Object {
            schema: Some("public".into()),
            display_name: Some("no_primary_key".into()),
            name: app.browse.table.clone(),
            metadata_error: None,
            columns: Vec::new(),
            kind: "table".into(),
            ddl: String::new(),
        }];
        for engine in ["SQLite", "PostgreSQL"] {
            app.engine = engine.into();
            assert!(app.supports_schema_changes());
            app.objects[0].kind = "view".into();
            assert!(!app.supports_schema_changes());
            app.objects[0].kind = "table".into();
        }
        app.engine = "Unimplemented".into();
        assert!(!app.supports_schema_changes());
        app.engine = "PostgreSQL".into();
        app.objects.clear();
        assert!(!app.supports_schema_changes());
    }

    #[test]
    fn plain_tab_name_survives_schema_catalog_switch_without_losing_identity() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_launcher_preferences(&ctx, Preferences::default());
        let qualified = "\"qa.schema\".\"odd\"\"table\"".to_owned();
        app.objects = vec![Object {
            schema: Some("qa.schema".into()),
            display_name: Some("odd\"table".into()),
            name: qualified.clone(),
            metadata_error: None,
            columns: Vec::new(),
            kind: "table".into(),
            ddl: String::new(),
        }];
        app.open_table(qualified.clone());
        assert_eq!(app.current_table_name(), "odd\"table");
        assert_eq!(app.browse.table, qualified);
        app.objects.clear();
        app.schemas = db::SchemaCatalog {
            selected: Some("different_schema".into()),
            names: vec!["different_schema".into()],
        };
        assert_eq!(app.current_table_name(), "odd\"table");
        assert_eq!(app.browse.table, qualified);
        app.browse.table = "a SQLite table".into();
        assert_eq!(app.current_table_name(), "a SQLite table");
    }
}

#[cfg(test)]
mod bracket_tab_navigation_tests {
    use super::*;
    fn fixture() -> (egui::Context, Rustable) {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_launcher_preferences(&ctx, Preferences::default());
        app.connections = false;
        app.browse.table = "table".into();
        app.view = View::Data;
        app.tabs = vec![
            QueryTab::new("SELECT 1".into()),
            QueryTab::new("closed".into()),
            QueryTab::new("SELECT 3".into()),
        ];
        app.tabs[1].open = false;
        app.tab = 0;
        (ctx, app)
    }
    #[test]
    fn wraps_rendered_order_and_skips_closed_retained_tabs() {
        let (_ctx, mut app) = fixture();
        app.navigate_workspace_tab(true);
        assert!(app.view == View::Query);
        assert_eq!(app.tab, 0);
        app.navigate_workspace_tab(true);
        assert_eq!(app.tab, 2);
        app.navigate_workspace_tab(true);
        assert!(app.view == View::Data);
        app.navigate_workspace_tab(false);
        assert!(app.view == View::Query);
        assert_eq!(app.tab, 2);
        app.navigate_workspace_tab(false);
        assert_eq!(app.tab, 0);
        app.navigate_workspace_tab(false);
        assert!(app.view == View::Data);
        assert!(!app.tabs[1].open);
        app.browse.table.clear();
        app.view = View::Query;
        app.tab = 0;
        app.navigate_workspace_tab(false);
        assert_eq!(app.tab, 2);
        app.tabs[2].open = false;
        app.tab = 0;
        app.navigate_workspace_tab(true);
        assert_eq!(app.tab, 0);
    }
    #[test]
    fn invalid_inline_value_blocks_switch_and_valid_value_stages_before_switch() {
        let (_ctx, mut app) = fixture();
        app.edit = Some(CellEdit {
            bulk_rows: Vec::new(),
            bulk_changed: false,
            insert_id: None,
            advance: 0,
            table: "table".into(),
            row: vec![Value::Integer(1)],
            original_value: Value::Integer(1),
            column: "id".into(),
            text: "not an integer".into(),
            kind: 1,
            focus: false,
            inspector: false,
        });
        app.navigate_workspace_tab(true);
        assert!(app.view == View::Data);
        assert!(app.edit.is_some());
        assert!(app.changes.is_empty());
        app.edit.as_mut().unwrap().text = "2".into();
        app.navigate_workspace_tab(true);
        assert!(app.view == View::Query);
        assert!(app.edit.is_none());
        assert!(matches!(
            &app.changes[0],
            Change::Cell {
                value: Value::Integer(2),
                ..
            }
        ));
    }
    fn event(
        key: egui::Key,
        physical_key: Option<egui::Key>,
        pressed: bool,
        shift: bool,
    ) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers {
                command: true,
                shift,
                ..Default::default()
            },
        }
    }
    #[test]
    fn event_modifiers_physical_fallback_and_modal_ownership() {
        let (ctx, mut app) = fixture();
        let _ = ctx.run(
            egui::RawInput {
                events: vec![
                    event(egui::Key::CloseBracket, None, true, true),
                    event(egui::Key::CloseBracket, None, false, true),
                ],
                ..Default::default()
            },
            |ctx| app.workspace_tab_shortcuts(ctx),
        );
        assert!(app.view == View::Query);
        assert_eq!(app.tab, 0);
        let _ = ctx.run(
            egui::RawInput {
                events: vec![event(
                    egui::Key::Num0,
                    Some(egui::Key::CloseBracket),
                    true,
                    true,
                )],
                ..Default::default()
            },
            |ctx| app.workspace_tab_shortcuts(ctx),
        );
        assert_eq!(app.tab, 2);
        let _ = ctx.run(
            egui::RawInput {
                events: vec![event(egui::Key::OpenBracket, None, true, false)],
                ..Default::default()
            },
            |ctx| app.workspace_tab_shortcuts(ctx),
        );
        assert_eq!(app.tab, 2);
        app.review = Some(ChangePlan::default());
        let _ = ctx.run(
            egui::RawInput {
                events: vec![event(egui::Key::OpenBracket, None, true, true)],
                ..Default::default()
            },
            |ctx| app.workspace_tab_shortcuts(ctx),
        );
        assert_eq!(app.tab, 2);
        app.review = None;
        app.connections = true;
        let _ = ctx.run(
            egui::RawInput {
                events: vec![event(egui::Key::OpenBracket, None, true, true)],
                ..Default::default()
            },
            |ctx| app.workspace_tab_shortcuts(ctx),
        );
        assert_eq!(app.tab, 2);
    }
}

#[cfg(test)]
mod result_pin_tests {
    use super::*;
    fn wait(app: &mut Rustable) {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while app.pending.is_some() {
            assert!(Instant::now() < deadline, "worker timed out");
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    #[test]
    fn pagination_preserves_page_and_rows_when_edit_is_invalid() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        wait(&mut app);
        app.view = View::Data;
        app.browse.table = "customers".into();
        app.browse.offset = db::PAGE_SIZE;
        app.table = Arc::new(Grid {
            columns: vec!["id".into()],
            rows: vec![vec![Value::Integer(1)]; db::PAGE_SIZE],
            truncated: true, ..Default::default()
        });
        app.edit = Some(CellEdit {
            bulk_rows: Vec::new(), bulk_changed: false, insert_id: None, advance: 0,
            table: "customers".into(), row: vec![Value::Integer(1)],
            original_value: Value::Integer(1), column: "id".into(),
            text: "invalid".into(), kind: 1, focus: false, inspector: false,
        });
        let grid = app.table.clone();
        for next in [true, false] {
            app.change_table_page(next);
            assert_eq!(app.browse.offset, db::PAGE_SIZE);
            assert!(Arc::ptr_eq(&grid, &app.table));
            assert!(app.pending.is_none());
            assert_eq!(app.edit.as_ref().unwrap().text, "invalid");
        }
        app.edit = None;
        app.browse.offset = usize::MAX;
        app.change_table_page(true);
        assert_eq!(app.browse.offset, usize::MAX);
        assert!(app.pending.is_none());
        app.browse.offset = db::PAGE_SIZE;
        app.change_table_page(false);
        assert_eq!(app.browse.offset, 0);
        assert!(matches!(app.pending, Some(Pending::Table)));
        wait(&mut app);
        app.change_table_page(false);
        assert!(app.pending.is_none(), "previous at the first page is a no-op");
    }
    #[test]
    fn invalid_cell_draft_blocks_filter_apply_and_clear_atomically() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        wait(&mut app);
        app.browse.table = "customers".into();
        app.browse.offset = 500;
        app.browse.filter_column = Some("name".into());
        app.browse.filter = "applied".into();
        app.filter_column = "email".into();
        app.filter = "draft filter".into();
        app.edit = Some(CellEdit {
            bulk_rows: Vec::new(), bulk_changed: false, insert_id: None, advance: 0,
            table: "customers".into(), row: vec![Value::Integer(1)],
            original_value: Value::Integer(1), column: "id".into(),
            text: "not an integer".into(), kind: 1, focus: false, inspector: false,
        });
        let grid = app.table.clone();
        for clear in [false, true] {
            app.apply_table_filter(clear);
            assert_eq!(app.browse.offset, 500);
            assert_eq!(app.browse.filter_column.as_deref(), Some("name"));
            assert_eq!(app.browse.filter, "applied");
            assert_eq!(app.filter, "draft filter");
            assert!(Arc::ptr_eq(&grid, &app.table));
            assert!(app.pending.is_none());
            assert_eq!(app.edit.as_ref().unwrap().text, "not an integer");
            assert!(app.error.is_some());
        }
        // Resolving the draft permits the same Clear action to reset all
        // applied filter state and dispatch a new first page.
        app.edit = None;
        app.apply_table_filter(true);
        assert_eq!(app.browse.offset, 0);
        assert!(app.browse.filter_column.is_none());
        assert!(app.browse.filter.is_empty());
        assert!(app.filter.is_empty());
        assert!(matches!(app.pending, Some(Pending::Table)));
        wait(&mut app);
    }
    #[test]
    fn table_find_focuses_filter_without_changing_applied_filter() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        wait(&mut app);
        app.view = View::Data;
        app.browse.filter_column = Some("name".into());
        app.browse.filter = "applied".into();
        app.filter = "draft".into();
        let frame = |app: &mut Rustable| {
            let _ = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events: vec![egui::Event::Key {
                    key: egui::Key::F, physical_key: None, pressed: true,
                    repeat: false, modifiers: egui::Modifiers::COMMAND,
                }],
                ..Default::default()
            }, |ctx| { app.workspace_shortcuts(ctx); app.workspace(ctx); });
        };
        // A dialog owns Cmd+F even on its first frame.
        app.history = true;
        frame(&mut app);
        assert!(!app.filters_visible);
        app.history = false;
        frame(&mut app);
        assert!(app.filters_visible);
        assert!(ctx.memory(|m| m.has_focus(egui::Id::new("table-filter-value"))));
        assert_eq!(app.filter, "draft");
        assert_eq!(app.browse.filter, "applied");
        assert_eq!(app.browse.filter_column.as_deref(), Some("name"));
        assert!(app.pending.is_none());
        assert!(!app.tabs[0].search.open);
        assert!(!app.tabs[0].result_search.open);
    }
    #[test]
    fn find_shortcuts_follow_result_grid_and_search_focus() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        wait(&mut app);
        app.show_query();
        app.tabs[0].focus_editor = false;
        app.tabs[0].grid = Arc::new(Grid {
            columns: vec!["answer".into()], rows: vec![vec![Value::Integer(42)]],
            ..Default::default()
        });
        app.tabs[0].results = vec![app.tabs[0].grid.clone()];
        let frame = |app: &mut Rustable, key: Option<(egui::Key, egui::Modifiers)>| {
            let _ = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events: key.map(|(key, modifiers)| egui::Event::Key {
                    key, physical_key: None, pressed: true, repeat: false, modifiers,
                }).into_iter().collect(),
                ..Default::default()
            }, |ctx| { app.workspace_shortcuts(ctx); app.workspace(ctx); });
        };
        frame(&mut app, None);
        let grid_id = app.tabs[0].result_grid_focus.expect("result grid rendered");
        ctx.memory_mut(|m| m.request_focus(grid_id));
        frame(&mut app, Some((egui::Key::F, egui::Modifiers::COMMAND)));
        assert!(app.tabs[0].result_search.open);
        assert!(app.tabs[0].result_search.has_focus(&ctx));
        assert!(!app.tabs[0].search.open);
        frame(&mut app, Some((egui::Key::G, egui::Modifiers::COMMAND)));
        assert!(app.tabs[0].search.action.is_none());
        frame(&mut app, Some((egui::Key::Escape, egui::Modifiers::NONE)));
        assert!(!app.tabs[0].result_search.open);
        assert!(ctx.memory(|m| m.has_focus(grid_id)));
        app.tabs[0].focus_editor = true;
        frame(&mut app, None);
        frame(&mut app, Some((egui::Key::F, egui::Modifiers::COMMAND)));
        assert!(app.tabs[0].search.open);
        assert!(!app.tabs[0].result_search.open);
    }
    #[test]
    fn pinned_result_survives_success_failure_and_releases_on_close() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        wait(&mut app);
        app.show_query();
        app.tabs[0].sql = "SELECT 42 AS answer;".into();
        app.run_query(true);
        wait(&mut app);
        let original = app.tabs[0].grid.clone();
        let weak = Arc::downgrade(&original);
        app.toggle_result_pin();
        assert_eq!(app.tabs[0].pins.items.len(), 1);
        app.tabs[0].sql = "SELECT 99 AS answer;".into();
        app.run_query(true);
        wait(&mut app);
        assert_eq!(app.tabs[0].grid.rows[0][0], Value::Integer(99));
        assert!(Arc::ptr_eq(&app.tabs[0].pins.items[0].grid, &original));
        app.tabs[0].sql = "SELECT missing_column FROM missing_table;".into();
        app.run_query(true);
        wait(&mut app);
        assert!(app.tabs[0].result_error.is_some());
        assert_eq!(app.tabs[0].pins.items[0].grid.rows[0][0], Value::Integer(42));
        // Viewing an older snapshot must not claim the latest query failed.
        app.tabs[0].selected_pin = Some(0);
        app.tabs[0].grid = original.clone();
        let output = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.query_result_message(ui));
        });
        fn has(shape: &egui::Shape, needle: &str) -> bool {
            match shape { egui::Shape::Text(text) => text.galley.text().contains(needle), egui::Shape::Vec(shapes) => shapes.iter().any(|s| has(s, needle)), _ => false }
        }
        assert!(output.shapes.iter().any(|s| has(&s.shape,"Statement 1 completed")));
        assert!(!output.shapes.iter().any(|s| has(&s.shape,"missing_table")));
        drop(original);
        app.close_tab(0);
        assert!(weak.upgrade().is_none(), "closing last editor retained snapshot");
        assert!(app.tabs[0].pins.items.is_empty());
    }
    #[test]
    fn removing_pin_updates_selection_and_persistence_excludes_results() {
        let mut tab = QueryTab::new("SELECT 1".into());
        let current = Arc::new(Grid::default());
        tab.results = vec![current.clone()];
        tab.pins.add(Arc::default(), 0).unwrap();
        tab.pins.add(Arc::default(), 1).unwrap();
        tab.selected_pin = Some(1);
        tab.grid = tab.pins.items[1].grid.clone();
        let pinned = tab.grid.clone();
        tab.remove_pin(0);
        assert_eq!(tab.selected_pin, Some(0));
        assert!(Arc::ptr_eq(&tab.grid, &pinned));
        let restored = QueryDocument::capture(&tab).restore();
        assert!(restored.pins.items.is_empty());
        assert!(restored.grid.rows.is_empty());
        tab.remove_pin(0);
        assert!(tab.selected_pin.is_none());
        assert!(Arc::ptr_eq(&tab.grid, &current));
    }
}

#[cfg(test)]
mod dialog_shortcut_tests {
    use super::*;
    fn key(ctx: &egui::Context, app: &mut Rustable, key: egui::Key, modifiers: egui::Modifiers) {
        let _ = ctx.run(egui::RawInput {
            events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
            ..Default::default()
        }, |ctx| app.workspace_shortcuts(ctx));
    }
    #[test]
    fn dialogs_block_workspace_commands_on_their_first_frame() {
        for dialog in 0..4 {
            let ctx = egui::Context::default();
            let mut app = Rustable::with_preferences(&ctx, Preferences::default());
            app.pending = None;
            app.connections = false;
            app.show_query();
            app.tabs[0].sql = "SELECT 123;".into();
            app.prefs.history.clear();
            app.error = None;
            match dialog {
                0 => app.review = Some(ChangePlan::default()),
                1 => app.history = true,
                2 => { app.rename = Some(("old".into(), "new".into())); app.rename_inline = false; }
                _ => app.add_column = Some(ColumnDraft { table: "customers".into(), name: "test".into(), data_type: "TEXT".into(), not_null: false, default_kind: 0, default_text: String::new(), error: None }),
            }
            let count = app.tabs.len();
            for (key_code, modifiers) in [
                (egui::Key::R, egui::Modifiers::COMMAND),
                (egui::Key::T, egui::Modifiers::COMMAND),
                (egui::Key::W, egui::Modifiers::COMMAND),
                (egui::Key::Enter, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT),
                (egui::Key::Backspace, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT),
                (egui::Key::F, egui::Modifiers::COMMAND),
            ] {
                key(&ctx, &mut app, key_code, modifiers);
                assert!(app.pending.is_none(), "dialog {dialog}, key {key_code:?}");
                assert_eq!(app.tabs.len(), count);
                assert!(app.tabs[0].open);
                assert!(!app.tabs[0].search.open);
                assert!(app.prefs.history.is_empty());
                assert!(app.error.is_none());
            }
        }
    }
    #[test]
    fn review_disables_editor_input_and_beautify_on_first_frame() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.connections = false;
        app.show_query();
        app.tabs[0].sql = "select 1;".into();
        let frame = |app: &mut Rustable, events| {
            let _ = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                events, ..Default::default()
            }, |ctx| { app.workspace_shortcuts(ctx); app.workspace(ctx); });
        };
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        app.review = Some(ChangePlan::default());
        frame(&mut app, vec![egui::Event::Text("should not appear".into()),
            egui::Event::Key { key: egui::Key::I, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND }]);
        assert_eq!(app.tabs[0].sql, "select 1;");
        assert!(!app.tabs[0].formatter.busy());
        assert!(app.pending.is_none());
    }
    #[test]
    fn review_escape_retains_changes_and_confirmation_keeps_safe_mode_guard() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        app.pending = None;
        app.connections = false;
        app.safe = true;
        app.changes.push(Change::Cell { table: "customers".into(), original: vec![Value::Integer(1)], column: "name".into(), value: Value::Text("pending".into()) });
        app.review = Some(ChangePlan::default());
        key(&ctx, &mut app, egui::Key::Enter, egui::Modifiers::COMMAND);
        assert!(app.error.as_deref().unwrap().contains("Safe mode"));
        assert!(app.pending.is_none());
        assert!(app.review.is_some());
        key(&ctx, &mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(app.review.is_none());
        assert_eq!(app.changes.len(), 1);
    }
}

#[cfg(test)]
mod transaction_window_tests {
    use super::*;


    #[test]
    #[ignore = "requires opt-in disposable local PostgreSQL QA fixture"]
    fn disconnected_transaction_allows_recovery_without_claiming_commit_or_rollback() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let ctx = egui::Context::default();
        let config = crate::connection::PostgresConfig {
            host: "127.0.0.1".into(), port: 54329, database: "rustable_qa".into(),
            password: "rustable_qa_only".into(), tls: false, ..Default::default()
        };
        let mut observer = ::postgres::Client::connect(
            "host=127.0.0.1 port=54329 dbname=rustable_qa user=postgres password=rustable_qa_only",
            ::postgres::NoTls).unwrap();
        fn settle(app: &mut Rustable) {
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            while app.pending.is_some() {
                assert!(Instant::now() < deadline, "worker timeout");
                app.poll();
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        let mut app = Rustable::with_initial_connection(&ctx, Preferences::default(),
            Command::OpenPostgres(config.clone()), None);
        settle(&mut app);
        for start in ["BEGIN; SELECT pg_backend_pid();", "SELECT pg_backend_pid();"] {
            app.dispatch(Command::Query { sql: start.into(), safe: false }, Pending::Query(0));
            settle(&mut app);
            assert!(app.tabs[0].result_error.is_none(), "{:?}", app.tabs[0].result_error);
            let Value::Integer(pid) = app.tabs[0].results.last().unwrap().rows[0][0] else { panic!("expected backend PID") };
            let killed: bool = observer.query_one("SELECT pg_terminate_backend($1)", &[&(pid as i32)])
                .unwrap().get(0);
            assert!(killed);
            app.dispatch(Command::Query { sql: "SELECT 1".into(), safe: false }, Pending::Query(0));
            settle(&mut app);
            assert!(app.tabs[0].result_error.is_some());
            assert_eq!(app.worker.transaction_state(), db::TransactionState::Disconnected);
            assert!(app.worker.transaction_state().label().contains("VERIFY WRITES"));
            assert!(!app.close_blocked(), "a dead connection cannot accept ROLLBACK");
            // Local pending edits still prevent replacement, even on a dead socket.
            app.changes.push(Change::Cell { table: "t".into(), original: vec![Value::Integer(1)],
                column: "id".into(), value: Value::Integer(2) });
            assert!(app.close_blocked());
            app.dispatch(Command::OpenPostgres(config.clone()), Pending::Connect(None));
            assert!(app.pending.is_none());
            app.changes.clear();
            app.dispatch(Command::OpenPostgres(config.clone()), Pending::Connect(None));
            settle(&mut app);
            assert_eq!(app.worker.transaction_state(), db::TransactionState::Idle);
            assert_eq!(app.engine, "PostgreSQL");
        }
    }

    #[test]
    fn sqlite_transaction_survives_runs_errors_and_cancel_until_explicit_finish() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let observer = rusqlite::Connection::open(file.path()).unwrap();
        observer.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY)").unwrap();
        let ctx = egui::Context::default();
        let mut app = Rustable::with_initial_connection(&ctx, Preferences::default(),
            Command::Open(Some(file.path().into())), None);
        fn settle(app: &mut Rustable) {
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            while app.pending.is_some() {
                assert!(Instant::now() < deadline, "worker timeout");
                app.poll();
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        fn run(app: &mut Rustable, sql: &str, safe: bool) {
            app.dispatch(Command::Query { sql: sql.into(), safe }, Pending::Query(0));
            settle(app);
        }
        let count = || observer.query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0)).unwrap();
        settle(&mut app);
        run(&mut app, "BEGIN", false);
        run(&mut app, "INSERT INTO t VALUES(1)", false);
        assert_eq!(app.worker.transaction_state(), db::TransactionState::Active);
        assert!(app.close_blocked());
        assert_eq!(count(), 0, "other connections cannot see pending inserts");
        run(&mut app, "INSERT INTO t VALUES(1)", false);
        assert!(app.error.as_deref().unwrap().contains("transaction remains open"));
        assert_eq!(app.worker.transaction_state(), db::TransactionState::Active);
        app.worker.send(Command::Apply(db::ChangePlan::default())).unwrap();
        let error = app.worker.rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap().err().unwrap();
        assert!(error.contains("COMMIT or ROLLBACK"));
        let destination = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(destination.path(), "keep").unwrap();
        app.worker.send(Command::ExportTable { path: destination.path().into(),
            browse: db::Browse { table: "t".into(), ..Default::default() }, format: db::ExportFormat::Csv }).unwrap();
        let error = app.worker.rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap().err().unwrap();
        assert!(error.contains("COMMIT or ROLLBACK"));
        assert_eq!(std::fs::read_to_string(destination.path()).unwrap(), "keep");
        let csv = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(csv.path(), "id\n9\n").unwrap();
        let preview = db::csv_preview::load(csv.path(), Default::default(), None,
            &std::sync::atomic::AtomicBool::new(false)).unwrap();
        app.worker.send(Command::ImportCsv { request: db::csv_import::Request {
            source: preview.source.unwrap(), options: preview.options, table: "t".into(),
            columns: vec![], destinations: vec![], empty_as_null: false, postgres_inserts: false,
        }, safe: false }).unwrap();
        let error = app.worker.rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap().err().unwrap();
        assert!(error.contains("COMMIT or ROLLBACK"));
        app.dispatch(Command::Open(None), Pending::Connect(None));
        assert!(app.pending.is_none());
        assert_eq!(app.worker.transaction_state(), db::TransactionState::Active);
        app.dispatch(Command::Query {
            sql: "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n) SELECT sum(x) FROM n".into(), safe: true,
        }, Pending::Query(0));
        app.worker.cancel();
        settle(&mut app);
        assert_eq!(app.worker.transaction_state(), db::TransactionState::Active);
        run(&mut app, "ROLLBACK", true);
        assert_eq!(app.worker.transaction_state(), db::TransactionState::Idle);
        assert!(!app.close_blocked());
        assert_eq!(count(), 0);
        run(&mut app, "BEGIN", false);
        run(&mut app, "INSERT INTO t VALUES(2)", false);
        assert_eq!(count(), 0);
        run(&mut app, "COMMIT", true);
        assert_eq!(app.worker.transaction_state(), db::TransactionState::Idle);
        assert!(!app.close_blocked());
        assert_eq!(count(), 1);
        run(&mut app, "BEGIN; INSERT INTO t VALUES(3)", false);
        run(&mut app, "INSERT OR ROLLBACK INTO t VALUES(2)", false);
        assert_eq!(app.worker.transaction_state(), db::TransactionState::Idle,
            "engine-driven rollback must be reflected accurately");
        assert_eq!(count(), 1);
        run(&mut app, "SAVEPOINT edit_scope", false);
        run(&mut app, "INSERT INTO t VALUES(4)", false);
        run(&mut app, "ROLLBACK TO edit_scope", true);
        assert_eq!(app.worker.transaction_state(), db::TransactionState::Active);
        run(&mut app, "RELEASE edit_scope", true);
        assert_eq!(app.worker.transaction_state(), db::TransactionState::Idle);
        assert_eq!(count(), 1);
    }

    #[test]
    #[ignore = "requires opt-in disposable local PostgreSQL QA fixture"]
    fn open_sql_transaction_blocks_close_and_connection_replacement_until_recovery() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let ctx = egui::Context::default();
        let config = crate::connection::PostgresConfig {
            host: "127.0.0.1".into(), port: 54329, database: "rustable_qa".into(),
            password: "rustable_qa_only".into(), tls: false, ..Default::default()
        };
        let mut app = Rustable::with_initial_connection(&ctx, Preferences::default(),
            Command::OpenPostgres(config), None);
        fn settle(app: &mut Rustable) {
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            while app.pending.is_some() {
                assert!(Instant::now() < deadline, "worker timeout");
                app.poll();
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        settle(&mut app);
        assert_eq!(app.engine, "PostgreSQL");
        assert!(!app.close_blocked());
        for (sql, expected) in [
            ("BEGIN", db::TransactionState::Active),
            ("SELECT 1/0", db::TransactionState::Failed),
        ] {
            app.dispatch(Command::Query { sql: sql.into(), safe: false }, Pending::Query(0));
            settle(&mut app);
            assert_eq!(app.worker.transaction_state(), expected);
            assert!(app.close_blocked());
            assert!(expected.label().contains("TRANSACTION"));
            app.dispatch(Command::Open(None), Pending::Connect(None));
            assert!(app.pending.is_none());
            assert!(app.error.as_deref().unwrap().contains("COMMIT or ROLLBACK"));
            // Enforce the same rule on the owner thread, even if a caller bypasses UI guards.
            app.worker.send(Command::Open(None)).unwrap();
            let error = app.worker.rx.recv_timeout(std::time::Duration::from_secs(5))
                .unwrap().err().expect("replacement rejected");
            assert!(error.contains("COMMIT or ROLLBACK"));
            assert_eq!(app.worker.transaction_state(), expected);
        }
        app.dispatch(Command::Query { sql: "ROLLBACK".into(), safe: true }, Pending::Query(0));
        settle(&mut app);
        assert_eq!(app.worker.transaction_state(), db::TransactionState::Idle);
        assert!(!app.close_blocked());
        for (sql, expected) in [("BEGIN", db::TransactionState::Active), ("COMMIT", db::TransactionState::Idle)] {
            app.dispatch(Command::Query { sql: sql.into(), safe: sql == "COMMIT" }, Pending::Query(0));
            settle(&mut app);
            assert_eq!(app.worker.transaction_state(), expected);
            assert_eq!(app.close_blocked(), expected != db::TransactionState::Idle);
        }
        app.dispatch(Command::Open(None), Pending::Connect(None));
        settle(&mut app);
        assert_eq!(app.engine, "SQLite");
    }
}
