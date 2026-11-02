//! All database and file work happens on a single owner thread, never in a frame.
mod csv;
pub mod filters;
pub mod foreign_keys;
mod json;
mod row_export;
#[cfg(not(target_arch = "wasm32"))]
mod mysql_values;
#[cfg(not(target_arch = "wasm32"))]
mod mysql_results;
#[cfg(not(target_arch = "wasm32"))]
mod mysql;
#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) mod browser_export;

#[derive(Clone, Copy)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum ExportFormat { Csv, Json }
pub mod csv_import;
pub mod csv_preview;
#[cfg(not(target_arch = "wasm32"))]
mod postgres;
pub mod progress;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use postgres::async_client_config as probe_client_config;
mod sqlite;
#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) mod transport;
#[cfg(target_arch = "wasm32")]
pub(crate) mod browser;
#[cfg(target_arch = "wasm32")]
pub use browser::Worker;

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::Duration,
};

/// Driver-independent cell values. Adapters own conversion and parameter binding.
#[derive(Clone, Debug, PartialEq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// Transaction/session status published by the database owner thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum TransactionState {
    #[default]
    Idle,
    Active,
    Failed,
    Disconnected,
}
impl TransactionState {
    pub fn blocks_replacement(self) -> bool {
        matches!(self, Self::Active | Self::Failed)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "",
            Self::Disconnected => "  ·  CONNECTION LOST — VERIFY WRITES BEFORE RETRY",
            Self::Active => "  ·  TRANSACTION OPEN",
            Self::Failed => "  ·  TRANSACTION FAILED — ROLLBACK REQUIRED",
        }
    }
}

/// A session owns one driver on its worker thread; only owned results cross threads.
trait Driver: Send {
    fn set_cancellation(&mut self, _cancel: Arc<AtomicBool>) {}
    fn definition(&mut self, name: &str) -> Result<String, String> {
        self.objects()?.into_iter().find(|object| object.name == name)
            .map(|object| object.ddl).filter(|ddl| !ddl.trim().is_empty())
            .ok_or_else(|| "No definition is available for this object".into())
    }
    fn databases(&mut self) -> Result<Vec<String>, String> { Ok(Vec::new()) }
    fn transaction_state(&mut self) -> TransactionState { TransactionState::Idle }
    fn engine_name(&self) -> &'static str;
    fn sql_dialect(&self) -> crate::sql::Dialect { crate::sql::Dialect::for_engine(self.engine_name()) }
    fn objects(&mut self) -> Result<Vec<Object>, String>;
    fn schema_catalog(&self) -> SchemaCatalog {
        SchemaCatalog::default()
    }
    fn select_schema(&mut self, _name: &str) -> Result<Vec<Object>, String> {
        Err("Schema selection is not supported by this connection".into())
    }
    fn execute(&mut self, sql: &str, safe: bool, cancel: &AtomicBool) -> ScriptResult;
    fn browse(&mut self, request: &Browse) -> Result<TableData, String>;
    fn validate_export_path(&self, _path: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn export_table(
        &mut self,
        _path: &std::path::Path,
        _browse: &Browse,
        _cancel: &AtomicBool,
        _format: ExportFormat,
    ) -> Result<(), String> {
        Err("Full-table export is not supported by this driver".into())
    }
    #[cfg(any(test, target_arch = "wasm32"))]
    fn export_table_bytes(&mut self, _browse: &Browse, _cancel: &AtomicBool, _format: ExportFormat) -> Result<Vec<u8>, String> {
        Err("Browser table export is not supported by this driver".into())
    }
    fn import_csv(
        &mut self,
        _request: &csv_import::Request,
        _cancel: &AtomicBool,
    ) -> Result<u64, String> {
        Err("CSV import is not supported by this driver".into())
    }
    fn plan(&mut self, changes: &[Change]) -> Result<ChangePlan, String>;
    fn apply(&mut self, plan: &ChangePlan) -> Result<(), String>;
}

pub const PAGE_SIZE: usize = 500;
pub const QUERY_LIMIT: usize = 5_000;
const BYTE_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Object {
    pub schema: Option<String>,
    pub display_name: Option<String>,
    pub metadata_error: Option<String>,
    pub columns: Vec<Column>,
    pub name: String,
    pub kind: String,
    pub ddl: String,
}
impl Object {
    pub fn display_name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }
}
#[derive(Clone, Debug, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct SchemaCatalog {
    pub names: Vec<String>,
    pub selected: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub not_null: bool,
    pub default: String,
    pub pk: bool,
    pub generated: bool,
}

#[derive(Clone, Debug, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Grid {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
    pub affected: usize,
    pub elapsed: Duration,
    pub total_rows: Option<RowTotal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct RowTotal {
    pub rows: u64,
    pub estimated: bool,
}

impl Grid {
    /// Catalog statistics are optional and may be stale. A complete final page
    /// is stronger evidence; an empty page beyond the end cannot locate that end.
    fn set_browse_total(&mut self, offset: usize, estimate: Option<u64>) {
        let end = (offset as u64).saturating_add(self.rows.len() as u64);
        self.total_rows = if !self.truncated && (!self.rows.is_empty() || offset == 0) {
            Some(RowTotal {
                rows: end,
                estimated: false,
            })
        } else {
            estimate.map(|rows| RowTotal {
                rows: if self.rows.is_empty() && !self.truncated {
                    rows
                } else {
                    rows.max(end.saturating_add(u64::from(self.truncated)))
                },
                estimated: true,
            })
        };
    }
}

#[derive(Debug)]
struct TableData {
    sql: String,
    parameters: Vec<Value>,
    grid: Grid,
    columns: Vec<Column>,
    indexes: Vec<Index>,
    triggers: Vec<Object>,
    foreign_keys: Vec<foreign_keys::ForeignKey>,
}

impl From<TableData> for Payload {
    fn from(table: TableData) -> Self {
        Self::Table {
            sql: table.sql,
            parameters: table.parameters,
            grid: table.grid,
            columns: table.columns,
            indexes: table.indexes,
            triggers: table.triggers,
            foreign_keys: table.foreign_keys,
        }
    }
}

#[derive(Debug, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ScriptResult {
    pub grids: Vec<Arc<Grid>>,
    pub error: Option<String>,
    pub elapsed: Duration,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Browse {
    #[serde(default)]
    pub filters: Vec<filters::Filter>,
    #[serde(default)]
    pub limit: usize,
    pub table: String,
    pub offset: usize,
    pub filter_column: Option<String>,
    pub filter: String,
    pub sort: Option<(String, bool)>,
}

impl Browse {
    pub fn page_size(&self) -> usize { if self.limit == 0 { PAGE_SIZE } else { self.limit.clamp(1, 5000) } }
    /// Header clicks cycle descending, ascending, then the default table order.
    pub fn cycle_sort(&mut self, column: String) {
        self.sort = match self.sort.as_ref() {
            Some((current, true)) if current == &column => Some((column, false)),
            Some((current, false)) if current == &column => None,
            _ => Some((column, true)),
        };
        self.offset = 0;
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum Command {
    Definition(String),
    ListDatabases,
    RefreshSchema,
    SelectSchema(String),
    OpenScript(PathBuf),
    SaveScript {
        path: PathBuf,
        sql: String,
        tab_id: u64,
        expected: Option<crate::script_file::SaveSource>,
    },
    ImportCsv {
        #[cfg_attr(not(target_arch = "wasm32"), serde(with = "unavailable_transport"))]
        request: csv_import::Request,
        safe: bool,
    },
    ImportTarget(String),
    PreviewCsv {
        path: PathBuf,
        options: csv_preview::Options,
        #[cfg_attr(not(target_arch = "wasm32"), serde(with = "unavailable_transport"))]
        source: Option<Arc<csv_preview::Source>>,
    },
    Open(Option<PathBuf>),
    #[cfg(any(test, target_arch = "wasm32"))]
    OpenBrowserDatabase { name: String, bytes: Vec<u8> },
    OpenPostgres(#[serde(with = "unavailable_transport")] crate::connection::PostgresConfig),
    OpenMysql(#[serde(with = "unavailable_transport")] crate::connection::MysqlConfig),
    Query {
        sql: String,
        safe: bool,
    },
    Browse(Browse),
    Plan(Vec<Change>),
    Apply(ChangePlan),
    Export {
        format: ExportFormat,
        path: PathBuf,
        grid: Arc<Grid>,
    },
    #[cfg(target_arch = "wasm32")]
    DownloadTable { format: ExportFormat, browse: Browse },
    ExportTable {
        format: ExportFormat,
        path: PathBuf,
        browse: Browse,
    },
}

impl Command {
    pub fn replaces_connection(&self) -> bool {
        match self {
            Self::Open(_) | Self::OpenPostgres(_) | Self::OpenMysql(_) => true,
            #[cfg(any(test, target_arch = "wasm32"))]
            Self::OpenBrowserDatabase { .. } => true,
            _ => false,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum Payload {
    Definition { name: String, sql: String },
    Databases(Vec<String>),
    Schema {
        objects: Vec<Object>,
        schemas: SchemaCatalog,
    },
    ScriptOpened {
        path: PathBuf,
        sql: String,
    },
    ScriptSaved {
        path: PathBuf,
        sql: String,
        tab_id: u64,
    },
    Imported(u64),
    ImportTarget(Object),
    CsvPreview(#[cfg_attr(not(target_arch = "wasm32"), serde(with = "unavailable_transport"))] csv_preview::Preview),
    Planned(ChangePlan),
    Applied {
        objects: Result<Vec<Object>, String>,
    },
    Connected {
        dialect: crate::sql::Dialect,
        engine: String,
        label: String,
        database: Option<String>,
        objects: Vec<Object>,
        schemas: SchemaCatalog,
    },
    Query {
        dialect: crate::sql::Dialect,
        report: ScriptResult,
        objects: Option<Result<Vec<Object>, String>>,
        schemas: SchemaCatalog,
    },
    Table {
        sql: String,
        parameters: Vec<Value>,
        grid: Grid,
        columns: Vec<Column>,
        indexes: Vec<Index>,
        triggers: Vec<Object>,
        foreign_keys: Vec<foreign_keys::ForeignKey>,
    },
    Exported(PathBuf),
    #[cfg(target_arch = "wasm32")]
    Download { bytes: Vec<u8>, format: ExportFormat },
}

#[cfg(not(target_arch = "wasm32"))]
pub struct Worker {
    tx: SyncSender<Command>,
    pub rx: Receiver<Result<Payload, String>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<progress::Progress>,
    transaction: Arc<AtomicU8>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Worker {
    pub fn new(ctx: eframe::egui::Context) -> Self {
        let viewport_id = ctx.viewport_id();
        let (tx, commands) = mpsc::sync_channel(1);
        let (results, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let progress = Arc::new(progress::Progress::default());
        let telemetry = progress.clone();
        let transaction = Arc::new(AtomicU8::new(TransactionState::Idle as u8));
        let transaction_status = transaction.clone();
        std::thread::Builder::new()
            .name("rustable-database".into())
            .spawn(move || {
                let mut connection = None;
                while let Ok(command) = commands.recv() {
                    let operation = telemetry.begin();
                    let result = handle(&mut connection, command, &stop);
                    transaction_status.store(connection.as_mut().map_or(TransactionState::Idle,
                        |driver| driver.transaction_state()) as u8, Ordering::Release);
                    drop(operation);
                    if results.send(result).is_err() {
                        break;
                    }
                    ctx.request_repaint_of(viewport_id);
                }
            })
            .expect("database worker thread");
        Self {
            tx,
            rx,
            cancel,
            progress,
            transaction,
        }
    }

    pub fn transaction_state(&self) -> TransactionState {
        match self.transaction.load(Ordering::Acquire) {
            1 => TransactionState::Active,
            2 => TransactionState::Failed,
            3 => TransactionState::Disconnected,
            _ => TransactionState::Idle,
        }
    }

    pub fn send(&self, command: Command) -> Result<(), String> {
        self.cancel.store(false, Ordering::Relaxed);
        self.tx.try_send(command).map_err(|e| e.to_string())
    }

    pub fn progress(&self) -> progress::Snapshot {
        self.progress.snapshot()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

// Catalog reads are expensive over SSH. Ordinary result queries and data edits
// leave the object list intact; unfamiliar statements conservatively refresh it.
fn query_needs_catalog_refresh(sql: &str, dialect: crate::sql::Dialect) -> bool {
    let ranges = match dialect {
        crate::sql::Dialect::Sqlite => crate::sql::statements(sql),
        crate::sql::Dialect::Mysql(mode) => match crate::sql::mysql::ranges(sql, mode, true) {
            Ok(ranges) => ranges,
            Err(_) => return true,
        },
        crate::sql::Dialect::Postgres => match crate::sql::postgres_statements(sql) {
            Ok(ranges) => ranges,
            Err(_) => return true,
        },
    };
    ranges.into_iter().any(|range| {
        let mut statement = sql[range].trim_start();
        loop {
            if let Some(comment) = statement.strip_prefix("--") {
                statement = comment
                    .split_once('\n')
                    .map_or("", |(_, rest)| rest)
                    .trim_start();
            } else if statement.starts_with("/*") {
                let bytes = statement.as_bytes();
                let mut depth = 1;
                let mut end = 2;
                while end + 1 < bytes.len() && depth > 0 {
                    if &bytes[end..end + 2] == b"*/" {
                        depth -= 1;
                        end += 2;
                    } else if dialect == crate::sql::Dialect::Postgres
                        && &bytes[end..end + 2] == b"/*"
                    {
                        depth += 1;
                        end += 2;
                    } else {
                        end += 1;
                    }
                }
                if depth != 0 {
                    return true;
                }
                statement = statement[end..].trim_start();
            } else {
                break;
            }
        }
        let keyword = statement
            .split(|c: char| !c.is_ascii_alphabetic())
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        match keyword.as_str() {
            "SELECT" | "WITH" => {
                // SELECT INTO creates a relation. Conservatively matching quoted
                // occurrences too costs only a rare extra refresh, never a stale catalog.
                statement
                    .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .any(|word| word.eq_ignore_ascii_case("into"))
            }
            "VALUES" | "TABLE" | "SHOW" | "EXPLAIN" | "INSERT" | "UPDATE" | "DELETE" | "MERGE"
            | "REPLACE" | "BEGIN" | "START" => false,
            // Transaction endings may roll back DDL whose catalog was already
            // displayed. Keep their refresh until transaction schema state is tracked.
            _ => true,
        }
    })
}

fn handle(
    connection: &mut Option<Box<dyn Driver>>,
    command: Command,
    cancel: &Arc<AtomicBool>,
) -> Result<Payload, String> {
    if command.replaces_connection()
        && connection.as_mut().is_some_and(|driver| driver.transaction_state().blocks_replacement()) {
        return Err("Finish the SQL transaction with COMMIT or ROLLBACK before switching connections.".into());
    }
    #[cfg(any(test, target_arch = "wasm32"))]
    if let Command::OpenBrowserDatabase { name, bytes } = command {
        let mut driver: Box<dyn Driver> = Box::new(sqlite::SqliteDriver::open_bytes(&bytes, cancel.clone())?);
        let objects = driver.objects()?;
        let schemas = driver.schema_catalog();
        *connection = Some(driver);
        return Ok(Payload::Connected { dialect: crate::sql::Dialect::Sqlite, engine: "SQLite".into(), label: format!("{name} · browser copy"), database: None, objects, schemas });
    }
    if let Command::Open(path) = command {
        let mut driver: Box<dyn Driver> =
            Box::new(sqlite::SqliteDriver::open(path.as_deref(), cancel.clone())?);
        let objects = driver.objects()?;
        let schemas = driver.schema_catalog();
        let engine = driver.engine_name().to_owned();
        let dialect = driver.sql_dialect();
        let label = path
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "Studio demo · in memory".into());
        // Replace the current session only after the candidate is fully usable.
        *connection = Some(driver);
        return Ok(Payload::Connected {
            dialect,
            engine,
            label,
            database: None,
            objects,
            schemas,
        });
    }
    #[cfg(target_arch = "wasm32")]
    if matches!(&command, Command::OpenPostgres(_) | Command::OpenMysql(_)) {
        return Err("Direct server connections require the desktop app.".into());
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Command::OpenPostgres(config) = command {
        let mut driver: Box<dyn Driver> =
            Box::new(postgres::PostgresDriver::open(&config, cancel.clone())?);
        let objects = driver.objects()?;
        let schemas = driver.schema_catalog();
        let engine = driver.engine_name().to_owned();
        let dialect = driver.sql_dialect();
        let label = config.label();
        *connection = Some(driver);
        return Ok(Payload::Connected {
            dialect,
            engine,
            label,
            database: Some(config.database.clone()),
            objects,
            schemas,
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Command::OpenMysql(config) = command {
        if cancel.load(Ordering::Acquire) { return Err("Connection cancelled".into()); }
        let mut driver = mysql::open(&config, cancel)?;
        driver.set_cancellation(cancel.clone());
        let objects = driver.objects()?;
        let schemas = driver.schema_catalog();
        let engine = driver.engine_name().to_owned();
        let dialect = driver.sql_dialect();
        let database = schemas.selected.clone();
        let label = config.to_string();
        if cancel.load(Ordering::Acquire) { return Err("Connection cancelled".into()); }
        *connection = Some(driver);
        return Ok(Payload::Connected { dialect, engine, label, database, objects, schemas });
    }
    if let Command::PreviewCsv {
        path,
        options,
        source,
    } = command
    {
        return csv_preview::load(&path, options, source, cancel).map(Payload::CsvPreview);
    }
    if let Command::OpenScript(path) = command {
        let sql = crate::script_file::read(&path)?;
        return Ok(Payload::ScriptOpened { path, sql });
    }
    if let Command::SaveScript {
        path,
        sql,
        tab_id,
        expected,
    } = command
    {
        let resolved = crate::script_file::destination(&path)?;
        if let Some(driver) = connection.as_ref() {
            driver.validate_export_path(&path)?;
            driver.validate_export_path(&resolved)?;
        }
        let baseline = expected
            .as_ref()
            .filter(|source| crate::script_file::same_source(&path, &resolved, source))
            .map(|source| source.sql.as_str());
        crate::script_file::write_checked(&resolved, &sql, baseline, cancel)?;
        return Ok(Payload::ScriptSaved { path, sql, tab_id });
    }
    if let Command::Export { path, grid, format } = command {
        if let Some(driver) = connection.as_ref() {
            driver.validate_export_path(&path)?;
        }
        match format {
            ExportFormat::Csv => csv::export(&path, &grid, cancel),
            ExportFormat::Json => json::export(&path, &grid, cancel),
        }.map_err(|e| e.to_string())?;
        return Ok(Payload::Exported(path));
    }
    let driver = connection.as_mut().ok_or("Open a database first")?;
    driver.set_cancellation(cancel.clone());
    match command {
        Command::Query { sql, safe } => {
            let refresh = query_needs_catalog_refresh(
                &sql,
                driver.sql_dialect(),
            );
            let report = driver.execute(&sql, safe, cancel);
            cancel.store(false, Ordering::Relaxed);
            Ok(Payload::Query {
                dialect: driver.sql_dialect(),
                report,
                // Execution may already have committed writes. Preserve its report
                // even when the independent schema refresh fails afterward.
                objects: refresh.then(|| driver.objects()),
                schemas: driver.schema_catalog(),
            })
        }
        #[cfg(target_arch = "wasm32")]
        Command::DownloadTable { browse, format } => {
            let bytes = driver.export_table_bytes(&browse, cancel, format)?;
            Ok(Payload::Download { bytes, format })
        }
        Command::ExportTable { path, browse, format } => {
            driver.export_table(&path, &browse, cancel, format)?;
            Ok(Payload::Exported(path))
        }
        Command::ImportCsv { request, safe } => {
            if safe {
                return Err("Disable safe mode before importing".into());
            }
            driver.import_csv(&request, cancel).map(Payload::Imported)
        }
        Command::ImportTarget(name) => {
            let object = driver
                .objects()?
                .into_iter()
                .find(|object| object.name == name && object.kind == "table")
                .ok_or_else(|| format!("Import destination table {name} no longer exists"))?;
            if let Some(error) = &object.metadata_error {
                return Err(error.clone());
            }
            Ok(Payload::ImportTarget(object))
        }
        Command::Definition(name) => driver.definition(&name).map(|sql| Payload::Definition { name, sql }),
        Command::ListDatabases => driver.databases().map(Payload::Databases),
        Command::RefreshSchema => {
            let objects = driver.objects()?;
            Ok(Payload::Schema {
                objects,
                schemas: driver.schema_catalog(),
            })
        }
        Command::SelectSchema(name) => {
            let objects = driver.select_schema(&name)?;
            Ok(Payload::Schema {
                objects,
                schemas: driver.schema_catalog(),
            })
        }
        Command::Browse(browse) => driver.browse(&browse).map(Payload::from),
        Command::Plan(changes) => driver.plan(&changes).map(Payload::Planned),
        Command::Apply(plan) => {
            driver.apply(&plan)?;
            // A metadata refresh failure must never turn a committed batch into
            // a retryable save failure: repeating it could duplicate mutations.
            cancel.store(false, Ordering::Relaxed);
            Ok(Payload::Applied {
                objects: driver.objects(),
            })
        }
        #[cfg(any(test, target_arch = "wasm32"))]
        Command::OpenBrowserDatabase { .. } => unreachable!(),
        Command::OpenScript(_)
        | Command::SaveScript { .. }
        | Command::Open(_)
        | Command::OpenPostgres(_)
        | Command::OpenMysql(_)
        | Command::Export { .. }
        | Command::PreviewCsv { .. } => unreachable!(),
    }
}

pub fn display(value: &Value) -> String {
    match value {
        Value::Null => "NULL".into(),
        Value::Integer(v) => v.to_string(),
        Value::Real(v) => v.to_string(),
        Value::Text(s) => s.clone(),
        Value::Blob(b) => format!("BLOB · {} bytes", b.len()),
    }
}

/// Text preview only: the value remains binary for editing, predicates and export.
fn readable_binary(bytes: &[u8]) -> Option<&str> {
    // Avoid repeatedly scanning arbitrarily large binary payloads while painting.
    if bytes.is_empty() || bytes.len() > 65536 { return None; }
    let text = std::str::from_utf8(bytes).ok()?;
    text.chars().all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t')).then_some(text)
}

/// Copy the complete displayed value. Binary values stay binary internally;
/// readable bytes copy as text, other bytes use the row-copy hexadecimal format.
/// Called only on an explicit copy action, never while painting.
pub fn clipboard_text(value: &Value) -> String {
    match value {
        Value::Blob(bytes) => {
            if let Some(text) = readable_binary(bytes) { return text.to_owned(); }
            use std::fmt::Write;
            let mut output = String::with_capacity(bytes.len().saturating_mul(2));
            for byte in bytes { write!(output, "{byte:02x}").expect("writing into String"); }
            output
        }
        _ => display(value),
    }
}

/// Render only a small UTF-8-safe prefix; never clone megabytes to paint a cell.
pub fn preview(value: &Value, characters: usize) -> std::borrow::Cow<'_, str> {
    let text = match value {
        Value::Text(text) => Some(text.as_str()),
        Value::Blob(bytes) => readable_binary(bytes),
        _ => None,
    };
    match text {
        Some(text) => match text.char_indices().nth(characters) {
            Some((end, _)) => std::borrow::Cow::Owned(format!("{}…", &text[..end])),
            None => std::borrow::Cow::Borrowed(text),
        },
        None => std::borrow::Cow::Owned(display(value)),
    }
}

#[cfg(test)]
mod binary_preview_tests {
    use super::*;
    #[test]
    fn binary_hash_and_unicode_preview_preserve_original_bytes() {
        let hash = "7fb8e699682a5ef678d03c6a50adcb5fa1d36fad775c48e1855d84b4fe8bed39";
        let value = Value::Blob(hash.as_bytes().to_vec());
        assert_eq!(preview(&value, 4096), hash);
        assert_eq!(clipboard_text(&value), hash);
        assert_eq!(clipboard_text(&Value::Blob(vec![0, 255, 10])), "00ff0a");
        assert_eq!(clipboard_text(&Value::Text("日本語".repeat(5000))), "日本語".repeat(5000));
        assert!(matches!(&value, Value::Blob(bytes) if bytes == hash.as_bytes()));
        assert_eq!(preview(&Value::Blob("日本語".as_bytes().to_vec()), 2), "日本…");
        assert_eq!(preview(&Value::Blob(vec![0, 255]), 4096), "BLOB · 2 bytes");
        assert_eq!(preview(&Value::Blob(b"text\0binary".to_vec()), 4096), "BLOB · 11 bytes");
    }
}

#[cfg(test)]
mod worker_tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn script_save_resolves_links_and_checks_alias_baselines_on_worker() {
        use std::os::unix::fs::symlink;
        let cancel = Arc::new(AtomicBool::new(false));
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let alias = dir.path().join("alias");
        symlink(&real, &alias).unwrap();
        let original = real.join("query.sql");
        let linked = dir.path().join("linked.sql");
        std::fs::write(&original, "SELECT 1;").unwrap();
        symlink(&original, &linked).unwrap();
        let mut connection = None;
        // Save As through a different path to the same source must still check
        // the baseline captured when the document was opened.
        for path in [alias.join("query.sql"), linked.clone()] {
            let error = handle(
                &mut connection,
                Command::SaveScript {
                    path,
                    sql: "SELECT 3;".into(),
                    tab_id: 1,
                    expected: Some(crate::script_file::SaveSource {
                        path: original.clone(),
                        sql: Arc::new("SELECT 0;".into()),
                    }),
                },
                &cancel,
            )
            .err()
            .unwrap();
            assert!(error.contains("changed outside"), "{error}");
            assert_eq!(std::fs::read_to_string(&original).unwrap(), "SELECT 1;");
        }
        handle(
            &mut connection,
            Command::SaveScript {
                path: linked.clone(),
                sql: "SELECT 2;".into(),
                tab_id: 1,
                expected: Some(crate::script_file::SaveSource {
                    path: original.clone(),
                    sql: Arc::new("SELECT 1;".into()),
                }),
            },
            &cancel,
        )
        .unwrap();
        assert!(
            std::fs::symlink_metadata(&linked)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&original).unwrap(), "SELECT 2;");
        // A separate copy does not compare against the source document baseline.
        let copy = real.join("copy.sql");
        handle(
            &mut connection,
            Command::SaveScript {
                path: copy.clone(),
                sql: "SELECT 4;".into(),
                tab_id: 1,
                expected: Some(crate::script_file::SaveSource {
                    path: original.clone(),
                    sql: Arc::new("stale baseline".into()),
                }),
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(copy).unwrap(), "SELECT 4;");
        std::fs::remove_file(&original).unwrap();
        assert!(
            handle(
                &mut connection,
                Command::SaveScript {
                    path: linked.clone(),
                    sql: "SELECT 5;".into(),
                    tab_id: 1,
                    expected: None
                },
                &cancel,
            )
            .is_err()
        );
        assert!(
            std::fs::symlink_metadata(linked)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!original.exists());
    }

    #[test]
    fn script_save_preserves_live_database_and_open_never_executes_sql() {
        let cancel = Arc::new(AtomicBool::new(false));
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("live.sqlite");
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute_batch("CREATE TABLE keep_me(id INTEGER);")
            .unwrap();
        let mut connection = None;
        handle(
            &mut connection,
            Command::Open(Some(database.clone())),
            &cancel,
        )
        .unwrap();
        let before = std::fs::read(&database).unwrap();
        assert!(
            handle(
                &mut connection,
                Command::SaveScript {
                    path: database.clone(),
                    sql: "SELECT 1".into(),
                    tab_id: 0,
                    expected: None
                },
                &cancel
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&database).unwrap(), before);
        #[cfg(unix)]
        {
            let link = dir.path().join("database-link.sql");
            std::os::unix::fs::symlink(&database, &link).unwrap();
            assert!(
                handle(
                    &mut connection,
                    Command::SaveScript {
                        path: link.clone(),
                        sql: "SELECT 1".into(),
                        tab_id: 0,
                        expected: None
                    },
                    &cancel,
                )
                .is_err()
            );
            assert!(
                std::fs::symlink_metadata(link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(std::fs::read(&database).unwrap(), before);
        }
        let path = dir.path().join("script.sql");
        let sql = "CREATE TABLE should_not_exist(id INTEGER);";
        handle(
            &mut connection,
            Command::SaveScript {
                path: path.clone(),
                sql: sql.into(),
                tab_id: 0,
                expected: None,
            },
            &cancel,
        )
        .unwrap();
        let Payload::ScriptOpened { sql: loaded, .. } =
            handle(&mut connection, Command::OpenScript(path), &cancel).unwrap()
        else {
            panic!()
        };
        assert_eq!(loaded, sql);
        assert!(
            connection
                .as_mut()
                .unwrap()
                .objects()
                .unwrap()
                .iter()
                .all(|o| o.name != "should_not_exist")
        );
    }

    #[test]
    fn import_target_refreshes_schema_and_rejects_views_and_missing_tables() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut connection = None;
        handle(&mut connection, Command::Open(None), &cancel).unwrap();
        let Payload::ImportTarget(before) = handle(
            &mut connection,
            Command::ImportTarget("customers".into()),
            &cancel,
        )
        .unwrap() else {
            panic!("expected metadata")
        };
        assert!(before.columns.iter().any(|c| c.name == "name"));
        let report = connection.as_mut().unwrap().execute(
            "ALTER TABLE customers ADD COLUMN import_note TEXT",
            false,
            &cancel,
        );
        assert!(report.error.is_none());
        let Payload::ImportTarget(after) = handle(
            &mut connection,
            Command::ImportTarget("customers".into()),
            &cancel,
        )
        .unwrap() else {
            panic!("expected metadata")
        };
        assert!(after.columns.iter().any(|c| c.name == "import_note"));
        assert!(
            handle(
                &mut connection,
                Command::ImportTarget("revenue_by_plan".into()),
                &cancel
            )
            .is_err()
        );
        assert!(
            handle(
                &mut connection,
                Command::ImportTarget("missing".into()),
                &cancel
            )
            .is_err()
        );
        let report =
            connection
                .as_mut()
                .unwrap()
                .execute("SELECT COUNT(*) FROM customers", true, &cancel);
        assert_eq!(report.grids[0].rows[0][0], Value::Integer(2400));
    }

    #[test]
    fn both_export_commands_preserve_the_open_database() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.sqlite");
        {
            let connection = rusqlite::Connection::open(&path).unwrap();
            connection
                .execute_batch("CREATE TABLE t(value TEXT); INSERT INTO t VALUES ('keep');")
                .unwrap();
        }
        let original = std::fs::read(&path).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut connection = None;
        handle(&mut connection, Command::Open(Some(path.clone())), &cancel).unwrap();
        for command in [
            Command::Export {
                format: ExportFormat::Csv,
                path: path.clone(),
                grid: Arc::new(Grid::default()),
            },
            Command::Export {
                format: ExportFormat::Json,
                path: path.clone(),
                grid: Arc::new(Grid::default()),
            },
            Command::ExportTable {
                format: ExportFormat::Csv,
                path: path.clone(),
                browse: Browse {
                    table: "t".into(),
                    ..Default::default()
                },
            },
            Command::ExportTable {
                format: ExportFormat::Json,
                path: path.clone(),
                browse: Browse {
                    table: "t".into(),
                    ..Default::default()
                },
            },
        ] {
            let result = handle(&mut connection, command, &cancel);
            assert!(matches!(result, Err(message) if message.contains("open SQLite database")));
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }
        let report = connection
            .as_mut()
            .unwrap()
            .execute("SELECT value FROM t", true, &cancel);
        assert!(report.error.is_none());
        assert_eq!(report.grids[0].rows[0][0], Value::Text("keep".into()));
    }

    fn receive(worker: &Worker) -> Result<Payload, String> {
        worker
            .rx
            .recv_timeout(Duration::from_secs(5))
            .expect("worker should reply without blocking the UI")
    }

    struct CatalogCounter(Arc<std::sync::atomic::AtomicUsize>);
    impl Driver for CatalogCounter {
        fn engine_name(&self) -> &'static str {
            "PostgreSQL"
        }
        fn objects(&mut self) -> Result<Vec<Object>, String> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(vec![])
        }
        fn execute(&mut self, _: &str, _: bool, _: &AtomicBool) -> ScriptResult {
            ScriptResult::default()
        }
        fn browse(&mut self, _: &Browse) -> Result<TableData, String> {
            unreachable!()
        }
        fn plan(&mut self, _: &[Change]) -> Result<ChangePlan, String> {
            unreachable!()
        }
        fn apply(&mut self, _: &ChangePlan) -> Result<(), String> {
            unreachable!()
        }
    }

    #[test]
    fn ordinary_queries_do_not_issue_catalog_reads() {
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut driver: Option<Box<dyn Driver>> = Some(Box::new(CatalogCounter(reads.clone())));
        let cancel = Arc::new(AtomicBool::new(false));
        for sql in [
            "SELECT 1",
            "VALUES (1)",
            "TABLE customers",
            "SHOW search_path",
            "EXPLAIN SELECT * FROM customers",
            "INSERT INTO customers VALUES (1)",
            "UPDATE customers SET id = 2",
            "DELETE FROM customers",
            "BEGIN",
            "/* outer /* inner */ comment */ SELECT '; DROP TABLE customers'",
            "WITH data AS (SELECT 1) SELECT * FROM data",
            "SELECT $$a;b$$; SELECT 2",
        ] {
            let Payload::Query { objects, .. } = handle(
                &mut driver,
                Command::Query {
                    sql: sql.into(),
                    safe: false,
                },
                &cancel,
            )
            .unwrap() else {
                panic!("query result")
            };
            assert!(objects.is_none(), "unneeded refresh: {sql}");
        }
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        for sql in [
            "CREATE TABLE t(id int)",
            "SELECT 1; ALTER TABLE t ADD COLUMN x int",
            "SELECT 1 INTO created_table",
            "DO $$BEGIN EXECUTE 'CREATE TABLE x(id int)'; END$$",
            "CALL alter_schema()",
            "ROLLBACK",
            "unknown_command",
        ] {
            let Payload::Query { objects, .. } = handle(
                &mut driver,
                Command::Query {
                    sql: sql.into(),
                    safe: false,
                },
                &cancel,
            )
            .unwrap() else {
                panic!("query result")
            };
            assert!(objects.is_some(), "missing refresh: {sql}");
        }
        assert_eq!(reads.load(Ordering::Relaxed), 7);
        assert!(query_needs_catalog_refresh(
            "CREATE TRIGGER t AFTER INSERT ON x BEGIN SELECT 1; SELECT 2; END; SELECT 3;",
            crate::sql::Dialect::Sqlite
        ));
        assert!(!query_needs_catalog_refresh(
            "-- note\nSELECT ';'; SELECT 2",
            crate::sql::Dialect::Sqlite
        ));
    }

    struct RefreshFailure;
    impl Driver for RefreshFailure {
        fn engine_name(&self) -> &'static str {
            "test"
        }
        fn objects(&mut self) -> Result<Vec<Object>, String> {
            Err("catalog unavailable".into())
        }
        fn execute(&mut self, _: &str, _: bool, _: &AtomicBool) -> ScriptResult {
            ScriptResult {
                grids: vec![Arc::new(Grid {
                    affected: 1,
                    ..Default::default()
                })],
                error: Some("second statement failed".into()),
                ..Default::default()
            }
        }
        fn browse(&mut self, _: &Browse) -> Result<TableData, String> {
            unreachable!()
        }
        fn plan(&mut self, _: &[Change]) -> Result<ChangePlan, String> {
            unreachable!()
        }
        fn apply(&mut self, _: &ChangePlan) -> Result<(), String> {
            unreachable!()
        }
    }

    #[test]
    fn catalog_failure_does_not_replace_execution_report() {
        let mut driver: Option<Box<dyn Driver>> = Some(Box::new(RefreshFailure));
        let result = handle(
            &mut driver,
            Command::Query {
                sql: "test".into(),
                safe: false,
            },
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let Payload::Query {
            report, objects, ..
        } = result
        else {
            panic!("expected execution report")
        };
        assert_eq!(report.grids[0].affected, 1);
        assert_eq!(report.error.as_deref(), Some("second statement failed"));
        assert_eq!(
            objects.unwrap().err().as_deref(),
            Some("catalog unavailable")
        );
    }

    #[test]
    fn broken_view_preserves_catalog_and_committed_query_results() {
        let worker = Worker::new(eframe::egui::Context::default());
        worker.send(Command::Open(None)).unwrap();
        receive(&worker).unwrap();
        // SQLite permits a view whose referenced table does not yet exist.
        // Enumerating its columns fails after the script has already completed.
        worker.send(Command::Query {
            sql: "CREATE TABLE refresh_probe(id INTEGER); INSERT INTO refresh_probe VALUES(7); CREATE VIEW broken_probe AS SELECT * FROM absent_probe; SELECT id FROM refresh_probe".into(),
            safe: false,
        }).unwrap();
        let Payload::Query {
            report, objects, ..
        } = receive(&worker).unwrap()
        else {
            panic!("execution report must survive metadata failure")
        };
        assert!(report.error.is_none());
        assert!(
            objects
                .unwrap()
                .unwrap()
                .iter()
                .find(|o| o.name == "broken_probe")
                .unwrap()
                .metadata_error
                .is_some()
        );
        assert_eq!(report.grids.last().unwrap().rows[0][0], Value::Integer(7));
        worker
            .send(Command::Query {
                sql: "SELECT 99; SELECT * FROM absent_probe".into(),
                safe: true,
            })
            .unwrap();
        let Payload::Query {
            report, objects, ..
        } = receive(&worker).unwrap()
        else {
            panic!("expected partial execution report")
        };
        assert!(report.error.as_ref().unwrap().contains("absent_probe"));
        assert!(
            objects.is_none(),
            "ordinary SELECT retains the existing catalog"
        );
        assert_eq!(report.grids[0].rows[0][0], Value::Integer(99));

        worker
            .send(Command::Query {
                sql: "DROP VIEW broken_probe; SELECT count(*) FROM refresh_probe".into(),
                safe: false,
            })
            .unwrap();
        let Payload::Query {
            report, objects, ..
        } = receive(&worker).unwrap()
        else {
            panic!("expected recovery report")
        };
        assert!(report.error.is_none());
        assert!(objects.unwrap().is_ok());
        assert_eq!(report.grids.last().unwrap().rows[0][0], Value::Integer(1));
    }

    #[test]
    fn committed_rename_returns_current_schema_to_the_ui() {
        let worker = Worker::new(eframe::egui::Context::default());
        worker.send(Command::Open(None)).unwrap();
        receive(&worker).unwrap();
        worker
            .send(Command::Plan(vec![Change::RenameColumn {
                table: "products".into(),
                from: "name".into(),
                to: "title".into(),
            }]))
            .unwrap();
        let Payload::Planned(plan) = receive(&worker).unwrap() else {
            panic!("expected plan")
        };
        worker.send(Command::Apply(plan)).unwrap();
        let Payload::Applied { objects } = receive(&worker).unwrap() else {
            panic!("expected committed result")
        };
        let objects = objects.unwrap();
        assert!(
            objects
                .iter()
                .find(|o| o.name == "products")
                .unwrap()
                .ddl
                .contains("\"title\"")
        );
        worker
            .send(Command::Browse(Browse {
                table: "products".into(),
                ..Default::default()
            }))
            .unwrap();
        let Payload::Table { columns, .. } = receive(&worker).unwrap() else {
            panic!("expected table")
        };
        assert!(columns.iter().any(|c| c.name == "title"));
        assert!(!columns.iter().any(|c| c.name == "name"));
    }

    #[test]
    fn failed_connection_keeps_the_current_session_usable() {
        let worker = Worker::new(eframe::egui::Context::default());
        worker.send(Command::Open(None)).unwrap();
        let Payload::Connected { engine, .. } = receive(&worker).unwrap() else {
            panic!("expected connection")
        };
        assert_eq!(engine, "SQLite");
        let absent = std::env::temp_dir().join(format!(
            "rustable-missing-{}-{}.db",
            std::process::id(),
            web_time::SystemTime::now()
                .duration_since(web_time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        worker.send(Command::Open(Some(absent.clone()))).unwrap();
        assert!(receive(&worker).is_err());
        assert!(!absent.exists());
        worker
            .send(Command::Query {
                sql: "SELECT count(*) FROM customers".into(),
                safe: true,
            })
            .unwrap();
        let Payload::Query { report, .. } = receive(&worker).unwrap() else {
            panic!("expected query")
        };
        assert!(report.error.is_none());
        assert_eq!(report.grids[0].rows[0][0], Value::Integer(2400));
    }

    #[test]
    fn cancelling_a_request_does_not_cancel_the_next_request() {
        let worker = Worker::new(eframe::egui::Context::default());
        worker.send(Command::Open(None)).unwrap();
        receive(&worker).unwrap();
        worker.send(Command::Query { sql: "WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<100000000) SELECT sum(x) FROM n".into(), safe: true }).unwrap();
        worker.cancel();
        let Payload::Query { report, .. } = receive(&worker).unwrap() else {
            panic!("expected cancelled query report")
        };
        assert!(report.error.is_some());
        worker
            .send(Command::Query {
                sql: "SELECT 42".into(),
                safe: true,
            })
            .unwrap();
        let Payload::Query { report, .. } = receive(&worker).unwrap() else {
            panic!("expected query")
        };
        assert!(report.error.is_none());
        assert_eq!(report.grids[0].rows[0][0], Value::Integer(42));
    }
}

/// Immutable review data: execution uses these exact statements and parameters.
#[derive(Clone, Debug, PartialEq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum Change {
    /// Omitted fields use database defaults; explicit Value::Null remains NULL.
    /// Each entry represents a distinct row, including identical entries.
    InsertRow {
        id: u64,
        table: String,
        fields: Vec<(String, Value)>,
    },
    DeleteRow {
        table: String,
        original: Vec<Value>,
    },
    CreateIndex {
        table: String,
        name: String,
        columns: Vec<(String, bool)>,
        unique: bool,
    },
    DropIndex {
        table: String,
        name: String,
    },
    AlterColumn {
        table: String,
        column: String,
        data_type: Option<String>,
        not_null: Option<bool>,
        default: Option<Option<Value>>,
    },
    AddColumn {
        table: String,
        definition: ColumnDefinition,
    },
    Cell {
        table: String,
        column: String,
        original: Vec<Value>,
        value: Value,
    },
    RenameColumn {
        table: String,
        from: String,
        to: String,
    },
}
impl Change {
    pub fn table(&self) -> &str {
        match self {
            Self::InsertRow { table, .. }
            | Self::DeleteRow { table, .. }
            | Self::CreateIndex { table, .. }
            | Self::DropIndex { table, .. }
            | Self::AlterColumn { table, .. }
            | Self::AddColumn { table, .. }
            | Self::Cell { table, .. }
            | Self::RenameColumn { table, .. } => table,
        }
    }
}

#[derive(Clone, Debug, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ChangePlan {
    pub rebuild: bool,
    pub schema_version: Option<i64>,
    pub statements: Vec<PlannedStatement>,
    /// Read checks executed immediately before the corresponding write, inside
    /// the same transaction. Keeping them separate makes primary-key writes
    /// readable without weakening optimistic concurrency protection.
    pub preconditions: Vec<PlannedPrecondition>,
    pub schemas: Vec<(String, String)>,
}
#[derive(Clone, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PlannedPrecondition {
    pub before_statement: usize,
    pub sql: String,
    pub parameters: Vec<Value>,
}
#[derive(Clone, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PlannedStatement {
    pub sql: String,
    pub parameters: Vec<Value>,
    pub expected_rows: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ColumnDefinition {
    pub name: String,
    pub data_type: String,
    pub not_null: bool,
    /// A typed constant default; None means no DEFAULT clause.
    pub default: Option<Value>,
}

#[derive(Clone, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Index {
    pub name: String,
    pub unique: bool,
    pub origin: String,
    pub partial: bool,
    pub columns: Vec<String>,
    pub ddl: String,
}

#[cfg(test)]
mod row_total_tests {
    use super::*;

    #[test]
    fn observed_page_bounds_override_stale_estimates() {
        let mut grid = Grid {
            rows: vec![vec![]; 2],
            truncated: true,
            ..Default::default()
        };
        grid.set_browse_total(500, Some(1));
        assert_eq!(
            grid.total_rows,
            Some(RowTotal {
                rows: 503,
                estimated: true
            })
        );
        grid.truncated = false;
        grid.set_browse_total(500, Some(1000));
        assert_eq!(
            grid.total_rows,
            Some(RowTotal {
                rows: 502,
                estimated: false
            })
        );
    }

    #[test]
    fn empty_out_of_range_page_does_not_invent_total() {
        let mut grid = Grid::default();
        grid.set_browse_total(500, None);
        assert_eq!(grid.total_rows, None);
        grid.set_browse_total(500, Some(20));
        assert_eq!(
            grid.total_rows,
            Some(RowTotal {
                rows: 20,
                estimated: true
            })
        );
        grid.set_browse_total(0, Some(20));
        assert_eq!(
            grid.total_rows,
            Some(RowTotal {
                rows: 0,
                estimated: false
            })
        );
    }
}

// Keep every enum discriminant stable, including native-only operations.
mod unavailable_transport {
    pub fn serialize<T, S: serde::Serializer>(_: &T, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("This operation requires the desktop app"))
    }
    pub fn deserialize<'de, T, D: serde::Deserializer<'de>>(_: D) -> Result<T, D::Error> {
        Err(serde::de::Error::custom("This operation requires the desktop app"))
    }
}

#[cfg(test)]
mod browser_database_tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        let source = rusqlite::Connection::open_in_memory().unwrap();
        source.execute_batch("CREATE TABLE imported(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO imported VALUES(1,'original');").unwrap();
        source.serialize(rusqlite::MAIN_DB).unwrap().to_vec()
    }
    #[test]
    fn browser_database_is_writable_copy_and_failed_replacement_preserves_session() {
        let bytes = fixture();
        let original = bytes.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut connection = None;
        let result = handle(&mut connection, Command::OpenBrowserDatabase { name: "fixture.sqlite".into(), bytes }, &cancel).unwrap();
        assert!(matches!(result, Payload::Connected { .. }));
        let driver = connection.as_mut().unwrap();
        assert!(driver.execute("UPDATE imported SET name='changed';", false, &cancel).error.is_none());
        for invalid in [b"not sqlite".to_vec(), original[..100].to_vec(), vec![0; 8 * 1024 * 1024 + 1]] {
            assert!(handle(&mut connection, Command::OpenBrowserDatabase { name: "invalid".into(), bytes: invalid }, &cancel).is_err());
        }
        let mut wal = original.clone(); wal[18] = 2; wal[19] = 2;
        assert!(handle(&mut connection, Command::OpenBrowserDatabase { name: "wal".into(), bytes: wal }, &cancel).err().unwrap().contains("WAL"));
        let report = connection.as_mut().unwrap().execute("SELECT name FROM imported", false, &cancel);
        assert_eq!(report.grids[0].rows[0][0], Value::Text("changed".into()));
        assert!(connection.as_mut().unwrap().execute("BEGIN", false, &cancel).error.is_none());
        assert!(handle(&mut connection, Command::OpenBrowserDatabase { name: "replacement".into(), bytes: original.clone() }, &cancel).is_err());
        assert!(connection.as_mut().unwrap().execute("ROLLBACK", false, &cancel).error.is_none());
        let mut fresh = sqlite::SqliteDriver::open_bytes(&original, cancel.clone()).unwrap();
        assert_eq!(fresh.execute("SELECT name FROM imported", false, &cancel).grids[0].rows[0][0], Value::Text("original".into()));
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn probe_mysql(config: &crate::connection::MysqlConfig, cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) { return Err("Connection test cancelled".into()); }
    let mut driver = mysql::open(config, cancel)?;
    let report = driver.execute("SELECT 1", false, cancel);
    match report.error { Some(error) => Err(error), None => Ok(()) }
}
