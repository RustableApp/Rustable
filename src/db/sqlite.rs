//! SQLite adapter. SQLite-specific SQL, binding, and transaction rules stay here.
mod export_path;
mod schema;
#[cfg(any(test, target_arch = "wasm32"))]
mod browser_limits;
use super::*;
use rusqlite::{Connection, OpenFlags, fallible_iterator::FallibleIterator, params_from_iter};
use web_time::Instant;

pub(super) struct SqliteDriver {
    connection: Connection,
    cursor: Option<PageCursor>,
    metadata: Option<(i64, Vec<Object>)>,
}

impl SqliteDriver {
    pub(super) fn open(
        path: Option<&std::path::Path>,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        #[cfg(target_arch = "wasm32")]
        browser_limits::limit_heap();
        let connection = match path {
            Some(path) => Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE),
            None => demo(),
        }
        .map_err(|e| e.to_string())?;
        Self::from_connection(connection, cancel)
    }
    #[cfg(any(test, target_arch = "wasm32"))]
    pub(super) fn open_bytes(bytes: &[u8], cancel: Arc<AtomicBool>) -> Result<Self, String> {
        if bytes.len() > 8 * 1024 * 1024 { return Err("Browser SQLite files are limited to 8 MiB".into()); }
        if bytes.len() < 100 || !bytes.starts_with(b"SQLite format 3\0") { return Err("Choose a valid SQLite database file".into()); }
        if bytes[18] != 1 || bytes[19] != 1 {
            return Err("This database uses WAL mode. Export a standalone SQLite backup before opening it in the browser.".into());
        }
        #[cfg(target_arch = "wasm32")]
        browser_limits::limit_heap();
        let mut connection = Connection::open_in_memory().map_err(|e| e.to_string())?;
        connection.deserialize_read_exact(rusqlite::MAIN_DB, bytes, bytes.len(), false).map_err(|e| e.to_string())?;
        let driver = Self::from_connection(connection, cancel)?;
        let check: String = driver.connection.query_row("PRAGMA quick_check", [], |row| row.get(0)).map_err(|e| e.to_string())?;
        if check != "ok" { return Err(format!("Invalid SQLite database: {check}")); }
        Ok(driver)
    }
    fn from_connection(connection: Connection, cancel: Arc<AtomicBool>) -> Result<Self, String> {
        #[cfg(target_arch = "wasm32")]
        browser_limits::configure(&connection).map_err(|e| e.to_string())?;
        connection
            .busy_timeout(Duration::from_millis(750))
            .map_err(|e| e.to_string())?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(|e| e.to_string())?;
        connection.progress_handler(1_000, Some(move || {
            #[cfg(target_arch = "wasm32")]
            if super::browser::should_interrupt() { return true; }
            cancel.load(Ordering::Relaxed)
        }))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            connection,
            cursor: None,
            metadata: None,
        })
    }
}

impl Driver for SqliteDriver {
    fn transaction_state(&mut self) -> TransactionState {
        if self.connection.is_autocommit() { TransactionState::Idle } else { TransactionState::Active }
    }
    fn engine_name(&self) -> &'static str {
        "SQLite"
    }
    fn objects(&mut self) -> Result<Vec<Object>, String> {
        // Uncommitted DDL can reuse schema cookies after rollback. Only retain
        // metadata from autocommit snapshots, never from an open transaction.
        if !self.connection.is_autocommit() {
            self.metadata = None;
            return objects(&self.connection).map_err(|e| e.to_string());
        }
        let version: i64 = self
            .connection
            .pragma_query_value(None, "schema_version", |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if let Some((cached_version, objects)) = &self.metadata
            && *cached_version == version
        {
            return Ok(objects.clone());
        }
        let objects = objects(&self.connection).map_err(|e| e.to_string())?;
        let after: i64 = self
            .connection
            .pragma_query_value(None, "schema_version", |r| r.get(0))
            .map_err(|e| e.to_string())?;
        // A concurrent schema change during enumeration must not bless a mixed
        // snapshot as a reusable cache. The next refresh will load it again.
        self.metadata = (after == version).then(|| (version, objects.clone()));
        Ok(objects)
    }
    fn execute(&mut self, sql: &str, safe: bool, cancel: &AtomicBool) -> ScriptResult {
        self.cursor = None;
        script(&self.connection, sql, safe, cancel)
    }
    fn browse(&mut self, request: &Browse) -> Result<TableData, String> {
        let version = page_version(&self.connection).map_err(|e| e.to_string())?;
        let cursor = self
            .cursor
            .take()
            .filter(|cursor| cursor.matches(request, version));
        let data = browse_table_after(&self.connection, request, cursor.as_ref())
            .map_err(|e| e.to_string())?;
        self.cursor = PageCursor::from_page(request, &data, version);
        Ok(data)
    }
    fn validate_export_path(&self, path: &std::path::Path) -> Result<(), String> {
        export_path::validate(&self.connection, path).map_err(|e| e.to_string())
    }
    fn export_table(
        &mut self,
        path: &std::path::Path,
        browse: &Browse,
        cancel: &AtomicBool,
        format: ExportFormat,
    ) -> Result<(), String> {
        require_idle_transaction(&self.connection)?;
        self.validate_export_path(path)?;
        export_table(&self.connection, path, browse, cancel, format).map_err(|e| e.to_string())
    }
    #[cfg(any(test, target_arch = "wasm32"))]
    fn export_table_bytes(&mut self, browse: &Browse, cancel: &AtomicBool, format: ExportFormat) -> Result<Vec<u8>, String> {
        require_idle_transaction(&self.connection)?;
        let mut output = super::browser_export::Bounded::new();
        export_table_to(&self.connection, &mut output, browse, cancel, format).map_err(|e| e.to_string())?;
        Ok(output.into_bytes())
    }
    fn import_csv(
        &mut self,
        request: &csv_import::Request,
        cancel: &AtomicBool,
    ) -> Result<u64, String> {
        self.cursor = None;
        require_idle_transaction(&self.connection)?;
        let sql = request.sql()?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            let kind: String = tx
                .query_row(
                    "SELECT type FROM sqlite_schema WHERE name = ?",
                    [&request.table],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if kind != "table"
                || columns(&tx, &request.table).map_err(|e| e.to_string())? != request.columns
            {
                return Err(
                    "Destination schema changed; select the table again before importing".into(),
                );
            }
            let mut statement = tx.prepare(&sql).map_err(|e| e.to_string())?;
            let count = csv_preview::visit(
                &request.source,
                request.options,
                request.destinations.len(),
                cancel,
                |record| {
                    let values = request.destinations.iter().enumerate().filter_map(
                        |(index, destination)| {
                            destination.map(|_| {
                                if request.empty_as_null && record[index].is_empty() {
                                    None
                                } else {
                                    Some(&record[index])
                                }
                            })
                        },
                    );
                    statement
                        .execute(params_from_iter(values))
                        .map_err(|e| e.to_string())?;
                    Ok(())
                },
            )?;
            if cancel.load(Ordering::Relaxed) {
                return Err("CSV import cancelled".into());
            }
            Ok(count)
        })();
        match result {
            Ok(count) => {
                if let Err(error) = tx.commit() {
                    cancel.store(false, Ordering::Relaxed);
                    if !self.connection.is_autocommit() {
                        self.connection
                            .execute_batch("ROLLBACK")
                            .map_err(|rollback| format!("{error}; rollback failed: {rollback}"))?;
                    }
                    return Err(error.to_string());
                }
                Ok(count)
            }
            Err(error) => {
                cancel.store(false, Ordering::Relaxed);
                tx.rollback()
                    .map_err(|rollback| format!("{error}; rollback failed: {rollback}"))?;
                Err(error)
            }
        }
    }
    fn plan(&mut self, changes: &[Change]) -> Result<ChangePlan, String> {
        plan_changes(&self.connection, changes)
    }
    fn apply(&mut self, plan: &ChangePlan) -> Result<(), String> {
        self.cursor = None;
        apply_plan(&mut self.connection, plan)
    }
}

impl From<rusqlite::types::Value> for Value {
    fn from(value: rusqlite::types::Value) -> Self {
        match value {
            rusqlite::types::Value::Null => Self::Null,
            rusqlite::types::Value::Integer(v) => Self::Integer(v),
            rusqlite::types::Value::Real(v) => Self::Real(v),
            rusqlite::types::Value::Text(v) => Self::Text(v),
            rusqlite::types::Value::Blob(v) => Self::Blob(v),
        }
    }
}

impl rusqlite::types::ToSql for Value {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        use rusqlite::types::{ToSqlOutput, ValueRef};
        Ok(ToSqlOutput::Borrowed(match self {
            Self::Null => ValueRef::Null,
            Self::Integer(v) => ValueRef::Integer(*v),
            Self::Real(v) => ValueRef::Real(*v),
            Self::Text(v) => ValueRef::Text(v.as_bytes()),
            Self::Blob(v) => ValueRef::Blob(v),
        }))
    }
}

pub fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn objects(conn: &Connection) -> rusqlite::Result<Vec<Object>> {
    let mut objects = conn.prepare("SELECT name, type, coalesce(sql, '') FROM sqlite_schema WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY type, name")?
        .query_map([], |r| Ok(Object { schema:None, display_name:None, metadata_error: None, columns: vec![], name: r.get(0)?, kind: r.get(1)?, ddl: r.get(2)? }))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for object in &mut objects {
        match columns(conn, &object.name) {
            Ok(columns) => object.columns = columns,
            // An unresolved view is still a valid catalog entry. Preserve its
            // definition for repair without hiding unrelated healthy objects.
            Err(error)
                if object.kind == "view"
                    && matches!(&error,
                rusqlite::Error::SqliteFailure(code, _) if code.extended_code == rusqlite::ffi::SQLITE_ERROR) =>
            {
                object.metadata_error = Some(error.to_string());
            }
            Err(error) => return Err(error),
        }
    }
    Ok(objects)
}

fn columns(conn: &Connection, table: &str) -> rusqlite::Result<Vec<Column>> {
    conn.prepare("SELECT name, type, \"notnull\", dflt_value, pk, hidden FROM pragma_table_xinfo(?) WHERE hidden != 1")?
        .query_map([table], |r| Ok(Column {
            name: r.get(0)?, data_type: r.get(1)?, not_null: r.get(2)?,
            default: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            pk: r.get::<_, i64>(4)? > 0, generated: r.get::<_, i64>(5)? != 0,
        }))?.collect()
}

fn indexes(conn: &Connection, table: &str) -> rusqlite::Result<Vec<Index>> {
    let mut indexes: Vec<Index> = conn
        .prepare("SELECT name,\"unique\",origin,partial FROM pragma_index_list(?1) ORDER BY seq")?
        .query_map([table], |r| {
            Ok(Index {
                name: r.get(0)?,
                unique: r.get(1)?,
                origin: r.get(2)?,
                partial: r.get(3)?,
                columns: vec![],
                ddl: String::new(),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    for index in &mut indexes {
        index.columns = conn
            .prepare(
                "SELECT name,desc,coll,cid FROM pragma_index_xinfo(?1) WHERE key=1 ORDER BY seqno",
            )?
            .query_map([&index.name], |r| {
                let name = r.get::<_, Option<String>>(0)?.unwrap_or_else(|| {
                    if r.get::<_, i64>(3).unwrap_or(-2) == -1 {
                        "rowid".into()
                    } else {
                        "(expression)".into()
                    }
                });
                let descending: bool = r.get(1)?;
                let coll: Option<String> = r.get(2)?;
                Ok(format!(
                    "{}{}{}",
                    name,
                    if descending { " ↓" } else { "" },
                    coll.filter(|c| c != "BINARY")
                        .map(|c| format!(" COLLATE {c}"))
                        .unwrap_or_default()
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        index.ddl = conn.query_row(
            "SELECT coalesce(sql,'') FROM sqlite_schema WHERE type='index' AND name=?1",
            [&index.name],
            |r| r.get(0),
        )?;
    }
    Ok(indexes)
}

#[cfg(test)]
fn query(conn: &Connection, sql: &str, safe: bool) -> rusqlite::Result<Grid> {
    query_with_budget(conn, sql, safe, BYTE_LIMIT)
}

fn script(conn: &Connection, sql: &str, safe: bool, cancel: &AtomicBool) -> ScriptResult {
    let start = Instant::now();
    let mut report = ScriptResult::default();
    let statements = crate::sql::statements(sql);
    if statements.len() > 128 {
        report.error = Some(
            "A run supports at most 128 statements. Select a smaller batch; nothing was executed."
                .into(),
        );
        return report;
    }
    let mut budget = BYTE_LIMIT;
    for (index, range) in statements.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            report.error = Some(format!(
                "Cancelled before statement {}. Earlier autocommitted statements remain committed.",
                index + 1
            ));
            break;
        }
        match query_with_budget(conn, &sql[range.clone()], safe, budget) {
            Ok(grid) => {
                let used = grid
                    .rows
                    .iter()
                    .flatten()
                    .map(|v| match v {
                        Value::Text(s) => s.len(),
                        Value::Blob(b) => b.len(),
                        _ => 8,
                    })
                    .sum::<usize>();
                budget = budget.saturating_sub(used);
                report.grids.push(Arc::new(grid));
            }
            Err(error) => {
                report.error = Some(format!(
                    "Statement {} failed: {}. Execution stopped; earlier autocommitted statements remain committed.",
                    index + 1,
                    if safe && matches!(error, rusqlite::Error::InvalidQuery) {
                        "Safe mode blocks this statement".into()
                    } else {
                        error.to_string()
                    }
                ));
                break;
            }
        }
    }
    if !conn.is_autocommit() && let Some(error) = &mut report.error {
        error.push_str(" The SQL transaction remains open; use COMMIT or ROLLBACK to finish it.");
    }
    report.elapsed = start.elapsed();
    report
}

fn query_with_budget(
    conn: &Connection,
    sql: &str,
    safe: bool,
    budget: usize,
) -> rusqlite::Result<Grid> {
    conn.pragma_update(None, "query_only", safe)?;
    let mut batch = rusqlite::Batch::new(conn, sql);
    let mut statement = batch.next()?.ok_or(rusqlite::Error::InvalidQuery)?;
    if batch.next()?.is_some() {
        return Err(rusqlite::Error::MultipleStatement);
    }
    if safe && !statement.readonly() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let start = Instant::now();
    let changes_before = conn.total_changes();
    let read_only = statement.readonly();
    let names = statement
        .column_names()
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    if names.is_empty() {
        statement.execute([])?;
        let affected = (conn.total_changes() - changes_before) as usize;
        return Ok(Grid {
            affected,
            elapsed: start.elapsed(),
            ..Default::default()
        });
    }
    let mut rows = statement.query([])?;
    let mut grid = collect_with_budget(&mut rows, names, QUERY_LIMIT, budget)?;
    if !read_only {
        // Finish RETURNING statements even when only a bounded preview is retained.
        // Otherwise affected counts and transaction completion are misleading.
        while rows.next()?.is_some() {}
    }
    drop(rows);
    grid.affected = (conn.total_changes() - changes_before) as usize;
    grid.elapsed = start.elapsed();
    Ok(grid)
}

fn collect(
    rows: &mut rusqlite::Rows<'_>,
    columns: Vec<String>,
    limit: usize,
) -> rusqlite::Result<Grid> {
    collect_with_budget(rows, columns, limit, BYTE_LIMIT)
}

fn collect_with_budget(
    rows: &mut rusqlite::Rows<'_>,
    columns: Vec<String>,
    limit: usize,
    budget: usize,
) -> rusqlite::Result<Grid> {
    let mut grid = Grid {
        columns,
        ..Default::default()
    };
    let mut bytes = 0;
    while let Some(row) = rows.next()? {
        if grid.rows.len() == limit {
            grid.truncated = true;
            break;
        }
        // Inspect borrowed SQLite values before allocating owned strings/blobs.
        // A single oversized cell must not be cloned merely to reject its row.
        let mut row_bytes = 0usize;
        for i in 0..grid.columns.len() {
            row_bytes = row_bytes.saturating_add(match row.get_ref(i)? {
                rusqlite::types::ValueRef::Text(text) => text.len(),
                rusqlite::types::ValueRef::Blob(blob) => blob.len(),
                _ => 8,
            });
        }
        if row_bytes > budget.saturating_sub(bytes) {
            grid.truncated = true;
            break;
        }
        let values = (0..grid.columns.len())
            .map(|i| row.get::<_, rusqlite::types::Value>(i).map(Value::from))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        bytes += row_bytes;
        super::progress::decoded_row(&values);
        grid.rows.push(values);
    }
    Ok(grid)
}

#[cfg(test)]
fn browse_table(conn: &Connection, browse: &Browse) -> rusqlite::Result<TableData> {
    browse_table_after(conn, browse, None)
}

fn export_table(
    conn: &Connection,
    path: &std::path::Path,
    browse: &Browse,
    cancel: &AtomicBool,
    format: ExportFormat,
) -> std::io::Result<()> {
    csv::publish(path, cancel, |writer| export_table_to(conn, writer, browse, cancel, format))
}

fn export_table_to(
    conn: &Connection,
    output: &mut dyn std::io::Write,
    browse: &Browse,
    cancel: &AtomicBool,
    format: ExportFormat,
) -> std::io::Result<()> {
    let metadata = columns(conn, &browse.table).map_err(std::io::Error::other)?;
    for column in browse.filter_column.iter().chain(browse.sort.iter().map(|(column, _)| column)) {
        if !metadata.iter().any(|field| &field.name == column) {
            return Err(std::io::Error::other("Export filter or sort column no longer exists"));
        }
    }
    let mut sql = format!("SELECT * FROM {}", quote(&browse.table));
    let mut parameters = Vec::new();
    if let Some(column) = &browse.filter_column {
        sql.push_str(&format!(
            " WHERE instr(lower(CAST({} AS TEXT)), lower(?)) > 0",
            quote(column)
        ));
        parameters.push(Value::Text(browse.filter.clone()));
    }
    if let Some((column, descending)) = &browse.sort {
        sql.push_str(&format!(
            " ORDER BY {} {}",
            quote(column),
            if *descending { "DESC" } else { "ASC" }
        ));
    } else {
        let keys: Vec<_> = metadata
            .iter()
            .filter(|c| c.pk)
            .map(|c| quote(&c.name))
            .collect();
        if !keys.is_empty() {
            sql.push_str(&format!(" ORDER BY {}", keys.join(", ")));
        }
    }
    let mut statement = conn.prepare(&sql).map_err(std::io::Error::other)?;
    let header = Grid {
        columns: statement
            .column_names()
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
        ..Default::default()
    };
    let width = header.columns.len();
    let mut rows = statement
        .query(params_from_iter(&parameters))
        .map_err(std::io::Error::other)?;
    {
        let mut writer = row_export::Writer::new(output, &header.columns, cancel, format)?;
        while let Some(row) = rows.next().map_err(std::io::Error::other)? {
            // SQLite owns these buffers until rows.next(); encode them before
            // advancing, without cloning an entire row or a large cell.
            let values = (0..width).map(|index| {
                use rusqlite::types::ValueRef;
                Ok(match row.get_ref(index).map_err(std::io::Error::other)? {
                    ValueRef::Null => csv::Cell::Null,
                    ValueRef::Integer(value) => csv::Cell::Integer(value),
                    ValueRef::Real(value) => csv::Cell::Real(value),
                    ValueRef::Text(bytes) => {
                        csv::Cell::Text(std::str::from_utf8(bytes).map_err(std::io::Error::other)?)
                    }
                    ValueRef::Blob(bytes) => csv::Cell::Blob(bytes),
                })
            });
            writer.row(values)?;
        }
        writer.finish()
    }
}

fn browse_table_after(
    conn: &Connection,
    browse: &Browse,
    cursor: Option<&PageCursor>,
) -> rusqlite::Result<TableData> {
    let start = Instant::now();
    let columns = columns(conn, &browse.table)?;
    let backward = cursor.is_some_and(|c| browse.offset < c.request.offset);
    let mut sql = format!("SELECT * FROM {}", quote(&browse.table));
    let mut params = Vec::<Value>::new();
    if let Some(column) = &browse.filter_column {
        sql.push_str(&format!(
            " WHERE instr(lower(CAST({} AS TEXT)), lower(?)) > 0",
            quote(column)
        ));
        params.push(Value::Text(browse.filter.clone()));
    }
    if let Some(cursor) = cursor {
        sql.push_str(if browse.filter_column.is_some() {
            " AND "
        } else {
            " WHERE "
        });
        sql.push_str(&format!(
            "{} {} ?",
            quote(&cursor.key),
            if cursor.descending != backward {
                "<"
            } else {
                ">"
            }
        ));
        params.push(if backward {
            cursor.first_value.clone()
        } else {
            cursor.value.clone()
        });
    }
    if let Some((column, descending)) = &browse.sort {
        sql.push_str(&format!(
            " ORDER BY {} {}",
            quote(column),
            if *descending != backward {
                "DESC"
            } else {
                "ASC"
            }
        ));
    } else {
        let keys: Vec<_> = columns
            .iter()
            .filter(|c| c.pk)
            .map(|c| {
                format!(
                    "{} {}",
                    quote(&c.name),
                    if backward { "DESC" } else { "ASC" }
                )
            })
            .collect();
        if !keys.is_empty() {
            sql.push_str(&format!(" ORDER BY {}", keys.join(", ")));
        }
    }
    sql.push_str(" LIMIT ? OFFSET ?");
    params.push(Value::Integer((PAGE_SIZE + 1) as i64));
    params.push(Value::Integer(if cursor.is_some() {
        0
    } else {
        browse.offset as i64
    }));
    let mut stmt = conn.prepare(&sql)?;
    let names = stmt.column_names().into_iter().map(String::from).collect();
    let mut rows = stmt.query(params_from_iter(params.iter()))?;
    let mut grid = collect(&mut rows, names, PAGE_SIZE)?;
    if backward {
        if grid.rows.len() != PAGE_SIZE {
            // A byte-limited reverse result would contain the end of the page,
            // not the start an OFFSET request returns. Preserve that behavior.
            return browse_table_after(conn, browse, None);
        }
        grid.rows.reverse();
        // The cursor's source page guarantees that another page follows this one.
        grid.truncated = true;
    }
    // ANALYZE statistics avoid scanning the entire table on every page. When
    // absent, leave the total unknown until a complete final page is observed.
    let estimate = if browse.filter_column.is_none() {
        sqlite_row_estimate(conn, &browse.table)
    } else {
        None
    };
    grid.set_browse_total(browse.offset, estimate);
    grid.elapsed = start.elapsed();
    let indexes = indexes(conn, &browse.table)?;
    let triggers = conn.prepare("SELECT name,type,sql FROM sqlite_schema WHERE type='trigger' AND tbl_name=?1 ORDER BY name")?
        .query_map([&browse.table],|r| Ok(Object { schema:None, display_name:None, metadata_error: None, columns: vec![], name:r.get(0)?,kind:r.get(1)?,ddl:r.get(2)? }))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(TableData {
        sql,
        parameters: params,
        grid,
        columns,
        indexes,
        triggers,
    })
}

fn sqlite_row_estimate(conn: &Connection, table: &str) -> Option<u64> {
    // Only the table or a non-partial index describes the whole relation.
    // sqlite_stat1 itself is absent until ANALYZE has run; that is normal.
    conn.query_row(
        "SELECT stat FROM sqlite_stat1 WHERE tbl=?1 AND (idx IS NULL OR idx IN (SELECT name FROM pragma_index_list(?1) WHERE partial=0)) ORDER BY idx IS NOT NULL LIMIT 1",
        [table],
        |row| row.get::<_, String>(0),
    ).ok()?.split_whitespace().next()?.parse().ok()
}

// A bounded one-page cursor accelerates Next and Previous, retaining arbitrary jumps. Any request, schema, or external-data change falls back to OFFSET.
struct PageCursor {
    request: Browse,
    key: String,
    value: Value,
    first_value: Value,
    descending: bool,
    version: (i64, i64),
}

fn page_version(conn: &Connection) -> rusqlite::Result<(i64, i64)> {
    Ok((
        conn.pragma_query_value(None, "schema_version", |r| r.get(0))?,
        conn.pragma_query_value(None, "data_version", |r| r.get(0))?,
    ))
}

impl PageCursor {
    fn matches(&self, request: &Browse, version: (i64, i64)) -> bool {
        let adjacent = self.request.offset.checked_add(PAGE_SIZE) == Some(request.offset)
            || self.request.offset.checked_sub(PAGE_SIZE) == Some(request.offset);
        let mut expected = self.request.clone();
        expected.offset = request.offset;
        adjacent && expected == *request && self.version == version
    }
    fn from_page(request: &Browse, data: &TableData, version: (i64, i64)) -> Option<Self> {
        if data.grid.rows.len() != PAGE_SIZE {
            return None;
        }
        let keys: Vec<_> = data
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.pk)
            .collect();
        let [(index, column)] = keys.as_slice() else {
            return None;
        };
        // SQLite's non-INTEGER rowid-table primary keys can contain NULL.
        // INTEGER PRIMARY KEY is a rowid alias only when no PK index exists
        // (the historical inline PRIMARY KEY DESC exception has such an index).
        let rowid_alias = column.data_type.eq_ignore_ascii_case("INTEGER")
            && !data.indexes.iter().any(|i| i.origin == "pk");
        if !column.not_null && !rowid_alias {
            return None;
        }
        let descending = match &request.sort {
            None => false,
            Some((name, descending)) if *name == column.name => *descending,
            _ => return None,
        };
        let value = data.grid.rows.last()?.get(*index)?.clone();
        if matches!(value, Value::Null) {
            return None;
        }
        Some(Self {
            request: request.clone(),
            key: column.name.clone(),
            value,
            first_value: data.grid.rows.first()?.get(*index)?.clone(),
            descending,
            version,
        })
    }
}

fn plan_changes(conn: &Connection, changes: &[Change]) -> Result<ChangePlan, String> {
    let mut plan = ChangePlan {
        // A changed trigger or index can alter a reviewed insert's effects even
        // when the table's CREATE statement is unchanged.
        schema_version: Some(
            conn.pragma_query_value(None, "schema_version", |r| r.get(0))
                .map_err(|e| e.to_string())?,
        ),
        ..Default::default()
    };
    let objects = objects(conn).map_err(|e| e.to_string())?;
    type RowChanges = (String, Vec<Value>, Vec<(String, Value)>);
    let mut rows: Vec<RowChanges> = Vec::new();
    for change in changes {
        let table = change.table();
        let object = objects
            .iter()
            .find(|o| o.name == table && o.kind == "table")
            .ok_or("Only existing tables can be edited")?;
        if !plan.schemas.iter().any(|(name, _)| name == table) {
            plan.schemas.push((table.to_owned(), object.ddl.clone()));
        }
        if let Change::Cell {
            column,
            original,
            value,
            ..
        } = change
        {
            let cols = columns(conn, table).map_err(|e| e.to_string())?;
            if cols.len() != original.len()
                || !cols.iter().any(|c| c.pk)
                || !cols.iter().any(|c| c.name == *column && !c.generated)
            {
                return Err(
                    "Editing requires a primary key and a writable column. Refresh the table."
                        .into(),
                );
            }
            let index = rows
                .iter()
                .position(|(t, r, _)| t == table && r == original);
            let index = index.unwrap_or_else(|| {
                rows.push((table.to_owned(), original.clone(), Vec::new()));
                rows.len() - 1
            });
            let fields = &mut rows[index].2;
            if let Some((_, old)) = fields.iter_mut().find(|(c, _)| c == column) {
                *old = value.clone();
            } else {
                fields.push((column.clone(), value.clone()));
            }
        }
    }
    // Data edits precede renames, so their original column identifiers remain valid.
    for (table, original, fields) in rows {
        if changes.iter().any(|c| matches!(c, Change::DeleteRow { table: t, original: r } if *t == table && *r == original)) {
            continue;
        }
        let cols = columns(conn, &table).map_err(|e| e.to_string())?;
        let set = fields
            .iter()
            .map(|(c, _)| format!("{} = ?", quote(c)))
            .collect::<Vec<_>>()
            .join(", ");
        let predicates = cols
            .iter()
            .filter(|c| c.pk)
            .map(|c| format!("{} IS ?", quote(&c.name)))
            .collect::<Vec<_>>()
            .join(" AND ");
        let parameters = fields
            .into_iter()
            .map(|(_, v)| v)
            .chain(
                cols.iter()
                    .zip(&original)
                    .filter(|(c, _)| c.pk)
                    .map(|(_, value)| value.clone()),
            )
            .collect();
        append_row_precondition(&mut plan, &table, &cols, &original);
        plan.statements.push(PlannedStatement {
            sql: format!("UPDATE {} SET {} WHERE {};", quote(&table), set, predicates),
            parameters,
            expected_rows: Some(1),
        });
    }
    let mut deleted = Vec::new();
    for change in changes {
        if let Change::DeleteRow { table, original } = change {
            if deleted.contains(&(table, original)) {
                continue;
            }
            deleted.push((table, original));
            let cols = columns(conn, table).map_err(|e| e.to_string())?;
            if cols.len() != original.len() || !cols.iter().any(|c| c.pk) {
                return Err("Deleting rows requires a primary key and current row metadata. Refresh the table.".into());
            }
            let predicates = cols
                .iter()
                .filter(|c| c.pk)
                .map(|c| format!("{} IS ?", quote(&c.name)))
                .collect::<Vec<_>>()
                .join(" AND ");
            append_row_precondition(&mut plan, table, &cols, original);
            plan.statements.push(PlannedStatement {
                sql: format!("DELETE FROM {} WHERE {};", quote(table), predicates),
                parameters: cols
                    .iter()
                    .zip(original)
                    .filter(|(c, _)| c.pk)
                    .map(|(_, value)| value.clone())
                    .collect(),
                expected_rows: Some(1),
            });
        }
    }
    // Inserts follow deletions (allow replacing a unique key) and precede
    // schema edits, so field names refer to the currently loaded schema.
    // Preserve their order and never deduplicate equal rows.
    for change in changes {
        if let Change::InsertRow { table, fields, .. } = change {
            let cols = columns(conn, table).map_err(|e| e.to_string())?;
            let mut seen = std::collections::HashSet::new();
            for (name, _) in fields {
                if !cols.iter().any(|c| c.name == *name && !c.generated) {
                    return Err(format!(
                        "Column {name} is missing or generated. Refresh the table."
                    ));
                }
                if !seen.insert(name) {
                    return Err(format!("Insert contains column {name} more than once"));
                }
            }
            let sql = if fields.is_empty() {
                format!("INSERT INTO {} DEFAULT VALUES;", quote(table))
            } else {
                format!(
                    "INSERT INTO {} ({}) VALUES ({});",
                    quote(table),
                    fields
                        .iter()
                        .map(|(name, _)| quote(name))
                        .collect::<Vec<_>>()
                        .join(", "),
                    vec!["?"; fields.len()].join(", ")
                )
            };
            plan.statements.push(PlannedStatement {
                sql,
                parameters: fields.iter().map(|(_, value)| value.clone()).collect(),
                expected_rows: Some(1),
            });
        }
    }
    append_rebuilds(conn, changes, &mut plan)?;
    append_index_changes(conn, changes, &mut plan)?;
    for change in changes {
        if let Change::AddColumn { table, definition } = change {
            let cols = columns(conn, table).map_err(|e| e.to_string())?;
            if cols
                .iter()
                .any(|c| c.name.eq_ignore_ascii_case(&definition.name))
            {
                return Err(format!("Column {} already exists", definition.name));
            }
            let sql = add_column_sql(conn, table, definition)?;
            plan.statements.push(PlannedStatement {
                sql,
                parameters: vec![],
                expected_rows: None,
            });
        }
        if let Change::RenameColumn { table, from, to } = change {
            if to.trim().is_empty() || to.contains('\0') {
                return Err("Column name must not be empty or contain NUL".into());
            }
            let cols = columns(conn, table).map_err(|e| e.to_string())?;
            if !cols.iter().any(|c| c.name == *from)
                || cols.iter().any(|c| c.name.eq_ignore_ascii_case(to))
            {
                return Err("Column is missing or the new name already exists".into());
            }
            plan.statements.push(PlannedStatement {
                sql: format!(
                    "ALTER TABLE {} RENAME COLUMN {} TO {};",
                    quote(table),
                    quote(from),
                    quote(to)
                ),
                parameters: vec![],
                expected_rows: None,
            });
        }
    }
    Ok(plan)
}

fn append_index_changes(
    conn: &Connection,
    changes: &[Change],
    plan: &mut ChangePlan,
) -> Result<(), String> {
    let drops: Vec<_> = changes
        .iter()
        .filter_map(|c| {
            if let Change::DropIndex { table, name } = c {
                Some((table, name))
            } else {
                None
            }
        })
        .collect();
    let creates: Vec<_> = changes
        .iter()
        .filter_map(|c| {
            if let Change::CreateIndex {
                table,
                name,
                columns,
                unique,
            } = c
            {
                Some((table, name, columns, unique))
            } else {
                None
            }
        })
        .collect();
    if drops.is_empty() && creates.is_empty() {
        return Ok(());
    }
    plan.schema_version = Some(
        conn.pragma_query_value(None, "schema_version", |r| r.get(0))
            .map_err(|e| e.to_string())?,
    );
    let mut removed = Vec::new();
    for (table, name) in &drops {
        if removed.contains(name) {
            return Err("An index is staged for removal more than once".into());
        }
        let indexes = indexes(conn, table).map_err(|e| e.to_string())?;
        let index = indexes
            .iter()
            .find(|index| index.name == **name)
            .ok_or("The index no longer exists on this table")?;
        if index.origin != "c" || index.ddl.is_empty() {
            return Err(
                "This index belongs to a table constraint. Edit the constraint instead.".into(),
            );
        }
        plan.statements.push(PlannedStatement {
            sql: format!("DROP INDEX {};", quote(name)),
            parameters: vec![],
            expected_rows: None,
        });
        removed.push(*name);
    }
    let mut created: Vec<&String> = Vec::new();
    for (table, name, keys, unique) in creates {
        if name.trim().is_empty()
            || name.contains('\0')
            || name.to_ascii_lowercase().starts_with("sqlite_")
        {
            return Err("Choose a nonempty index name outside SQLite's reserved namespace.".into());
        }
        if keys.is_empty() {
            return Err("Choose at least one index column.".into());
        }
        if created.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            return Err("Two staged indexes have the same name.".into());
        }
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name=?1 COLLATE NOCASE)",
                [name],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if exists && !drops.iter().any(|(_, n)| n.eq_ignore_ascii_case(name)) {
            return Err(format!("Schema object {name} already exists."));
        }
        let cols = columns(conn, table).map_err(|e| e.to_string())?;
        let mut seen = Vec::new();
        for (key, _) in keys {
            if !cols.iter().any(|c| c.name == *key) {
                return Err(format!(
                    "Column {key} is missing; refresh before creating the index."
                ));
            }
            if seen.contains(&key) {
                return Err("Select each index column only once.".into());
            }
            seen.push(key);
        }
        let keys = keys
            .iter()
            .map(|(key, descending)| {
                format!(
                    "{} {}",
                    quote(key),
                    if *descending { "DESC" } else { "ASC" }
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        plan.statements.push(PlannedStatement {
            sql: format!(
                "CREATE {}INDEX {} ON {} ({keys});",
                if *unique { "UNIQUE " } else { "" },
                quote(name),
                quote(table)
            ),
            parameters: vec![],
            expected_rows: None,
        });
        created.push(name);
    }
    Ok(())
}

fn add_column_sql(
    conn: &Connection,
    table: &str,
    definition: &ColumnDefinition,
) -> Result<String, String> {
    for (label, value) in [
        ("Column name", &definition.name),
        ("Data type", &definition.data_type),
    ] {
        if value.trim().is_empty() || value.contains('\0') {
            return Err(format!("{label} must not be empty or contain NUL"));
        }
    }
    if definition.not_null && matches!(definition.default, None | Some(Value::Null)) {
        return Err(
            "A new NOT NULL column needs a non-NULL constant default for existing rows.".into(),
        );
    }
    let mut sql = format!(
        "ALTER TABLE {} ADD COLUMN {} {}",
        quote(table),
        quote(&definition.name),
        quote(&definition.data_type)
    );
    if definition.not_null {
        sql.push_str(" NOT NULL");
    }
    if let Some(value) = &definition.default {
        if matches!(value, Value::Text(s) if s.contains('\0'))
            || matches!(value, Value::Real(v) if !v.is_finite())
        {
            return Err("Schema defaults cannot contain NUL or non-finite numbers.".into());
        }
        // SQLite's own literal encoder handles strings, binary and numeric defaults.
        // DDL cannot bind parameters, so only this encoded literal enters the statement.
        let literal: String = conn
            .query_row("SELECT quote(?1)", [value], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        sql.push_str(" DEFAULT ");
        sql.push_str(&literal);
    }
    sql.push(';');
    Ok(sql)
}

fn append_rebuilds(
    conn: &Connection,
    changes: &[Change],
    plan: &mut ChangePlan,
) -> Result<(), String> {
    let mut tables: Vec<(String, String)> = Vec::new();
    for change in changes {
        if let Change::AlterColumn {
            table,
            column,
            data_type,
            not_null,
            default,
        } = change
        {
            let cols = columns(conn, table).map_err(|e| e.to_string())?;
            let col = cols
                .iter()
                .find(|c| c.name == *column)
                .ok_or("Column no longer exists")?;
            if col.pk && data_type.is_some() {
                return Err("Primary-key type changes require a dedicated key migration.".into());
            }
            let index = if let Some(index) = tables.iter().position(|(t, _)| t == table) {
                index
            } else {
                let ddl: String = conn
                    .query_row(
                        "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
                        [table],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                tables.push((table.clone(), ddl));
                tables.len() - 1
            };
            let literal = match default {
                Some(Some(value)) => {
                    if matches!(value, Value::Text(s) if s.contains('\0'))
                        || matches!(value,Value::Real(v) if !v.is_finite())
                    {
                        return Err("Unsupported schema default".into());
                    }
                    Some(Some(
                        conn.query_row("SELECT quote(?1)", [value], |r| r.get::<_, String>(0))
                            .map_err(|e| e.to_string())?,
                    ))
                }
                Some(None) => Some(None),
                None => None,
            };
            tables[index].1 = schema::edit_column(
                &tables[index].1,
                column,
                data_type.as_deref(),
                *not_null,
                literal.as_ref().map(|v| v.as_deref()),
            )?;
        }
    }
    if tables.is_empty() {
        return Ok(());
    }
    let temporary_schema: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_temp_schema WHERE type IN ('view','trigger'))",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if temporary_schema {
        return Err("Temporary views/triggers are active. Remove them before rebuilding a table so their dependencies are not lost.".into());
    }
    plan.rebuild = true;
    plan.schema_version = Some(
        conn.pragma_query_value(None, "schema_version", |r| r.get(0))
            .map_err(|e| e.to_string())?,
    );
    let mut stmt = conn.prepare("SELECT type,name,sql FROM sqlite_schema WHERE type IN ('view','trigger') AND sql IS NOT NULL ORDER BY type,name").map_err(|e|e.to_string())?;
    let dependents: Vec<(String, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let push = |plan: &mut ChangePlan, sql: String| {
        plan.statements.push(PlannedStatement {
            sql,
            parameters: vec![],
            expected_rows: None,
        })
    };
    // Temporarily remove views/triggers to avoid invalid references between DROP
    // and RENAME. Recreate their exact SQL before FK validation and commit.
    for (kind, name, _) in &dependents {
        push(
            plan,
            format!("DROP {} {};", kind.to_uppercase(), quote(name)),
        );
    }
    for (number, (table, ddl)) in tables.into_iter().enumerate() {
        let temp = format!("__rustable_rebuild_{number}");
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name=?1)",
                [&temp],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if exists {
            return Err(
                "A migration staging table already exists; rename it before retrying.".into(),
            );
        }
        let cols = columns(conn, &table).map_err(|e| e.to_string())?;
        let mut names: Vec<String> = cols
            .iter()
            .filter(|c| !c.generated)
            .map(|c| quote(&c.name))
            .collect();
        let without_rowid: bool = conn
            .query_row(
                "SELECT wr FROM pragma_table_list WHERE schema='main' AND name=?1",
                [&table],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !without_rowid {
            let alias = ["rowid", "_rowid_", "oid"]
                .iter()
                .find(|alias| !cols.iter().any(|c| c.name.eq_ignore_ascii_case(alias)))
                .ok_or("All rowid aliases are shadowed; cannot preserve hidden row identities")?;
            names.insert(0, quote(alias));
        }
        let list = names.join(", ");
        push(plan, schema::with_table_name(&ddl, &temp)?);
        push(
            plan,
            format!(
                "INSERT OR ABORT INTO {} ({list}) SELECT {list} FROM {};",
                quote(&temp),
                quote(&table)
            ),
        );
        let sequence: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='sqlite_sequence')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if sequence {
            // Carry forward high-water marks even when the highest row was deleted.
            let old = table.replace('\'', "''");
            let new = temp.replace('\'', "''");
            push(
                plan,
                format!(
                    "UPDATE sqlite_sequence SET seq=max(seq,COALESCE((SELECT seq FROM sqlite_sequence WHERE name='{old}'),0)) WHERE name='{new}';"
                ),
            );
        }
        let mut indexes = conn.prepare("SELECT sql FROM sqlite_schema WHERE type='index' AND tbl_name=?1 AND sql IS NOT NULL").map_err(|e|e.to_string())?;
        let indexes = indexes
            .query_map([&table], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        push(plan, format!("DROP TABLE {};", quote(&table)));
        push(
            plan,
            format!("ALTER TABLE {} RENAME TO {};", quote(&temp), quote(&table)),
        );
        for sql in indexes {
            push(plan, sql);
        }
    }
    for (_, _, sql) in dependents {
        push(plan, sql);
    }
    Ok(())
}

fn append_row_precondition(
    plan: &mut ChangePlan,
    table: &str,
    columns: &[Column],
    original: &[Value],
) {
    let predicates = columns
        .iter()
        .map(|column| format!("{} IS ?", quote(&column.name)))
        .collect::<Vec<_>>()
        .join(" AND ");
    plan.preconditions.push(PlannedPrecondition {
        before_statement: plan.statements.len(),
        sql: format!(
            "SELECT COUNT(*) FROM {} WHERE {};",
            quote(table),
            predicates
        ),
        parameters: original.to_vec(),
    });
}

#[cfg(test)]
mod primary_key_write_tests {
    use super::*;

    #[test]
    fn writes_use_only_original_composite_key_and_checks_preserve_concurrency() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(a TEXT, b INTEGER, value TEXT, PRIMARY KEY(a,b)); INSERT INTO t VALUES('old',7,'before');").unwrap();
        let original = vec![
            Value::Text("old".into()),
            Value::Integer(7),
            Value::Text("before".into()),
        ];
        let plan = plan_changes(
            &conn,
            &[
                Change::Cell {
                    table: "t".into(),
                    column: "a".into(),
                    original: original.clone(),
                    value: Value::Text("new".into()),
                },
                Change::Cell {
                    table: "t".into(),
                    column: "value".into(),
                    original: original.clone(),
                    value: Value::Text("after".into()),
                },
            ],
        )
        .unwrap();
        assert_eq!(
            plan.statements[0].sql,
            "UPDATE \"t\" SET \"a\" = ?, \"value\" = ? WHERE \"a\" IS ? AND \"b\" IS ?;"
        );
        assert_eq!(
            plan.statements[0].parameters,
            vec![
                Value::Text("new".into()),
                Value::Text("after".into()),
                Value::Text("old".into()),
                Value::Integer(7)
            ]
        );
        assert_eq!(plan.preconditions.len(), 1);
        assert_eq!(plan.preconditions[0].parameters, original);
        assert_eq!(plan.preconditions[0].before_statement, 0);
        assert!(plan.preconditions[0].sql.contains("\"value\" IS ?"));
        apply_plan(&mut conn, &plan).unwrap();
        assert_eq!(
            conn.query_row("SELECT a,value FROM t", [], |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?
            )))
            .unwrap(),
            ("new".into(), "after".into())
        );
        let deletion = plan_changes(
            &conn,
            &[Change::DeleteRow {
                table: "t".into(),
                original: vec![
                    Value::Text("new".into()),
                    Value::Integer(7),
                    Value::Text("after".into()),
                ],
            }],
        )
        .unwrap();
        assert_eq!(
            deletion.statements[0].sql,
            "DELETE FROM \"t\" WHERE \"a\" IS ? AND \"b\" IS ?;"
        );
        assert_eq!(deletion.statements[0].parameters.len(), 2);
        conn.execute("UPDATE t SET value='concurrent'", []).unwrap();
        assert!(apply_plan(&mut conn, &deletion).is_err());
        assert_eq!(
            conn.query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn triggers_cannot_silently_overwrite_a_later_planned_row() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY,n INTEGER); INSERT INTO t VALUES(1,0),(2,0); CREATE TRIGGER change_other AFTER UPDATE ON t WHEN NEW.id=1 BEGIN UPDATE t SET n=100 WHERE id=2; END;").unwrap();
        let changes: Vec<_> = [1, 2]
            .into_iter()
            .map(|id| Change::Cell {
                table: "t".into(),
                column: "n".into(),
                original: vec![Value::Integer(id), Value::Integer(0)],
                value: Value::Integer(id + 10),
            })
            .collect();
        let plan = plan_changes(&conn, &changes).unwrap();
        assert_eq!(plan.preconditions[1].before_statement, 1);
        assert!(apply_plan(&mut conn, &plan).is_err());
        assert_eq!(
            conn.query_row("SELECT sum(n) FROM t", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn nullable_ambiguous_keys_and_missing_primary_keys_still_cannot_mass_update() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(id TEXT PRIMARY KEY,value TEXT); INSERT INTO t VALUES(NULL,'one'),(NULL,'two'); CREATE TABLE no_key(value TEXT); INSERT INTO no_key VALUES('one');").unwrap();
        let plan = plan_changes(
            &conn,
            &[Change::Cell {
                table: "t".into(),
                column: "value".into(),
                original: vec![Value::Null, Value::Text("one".into())],
                value: Value::Text("changed".into()),
            }],
        )
        .unwrap();
        // The original-row check succeeds for one row, but the PK write matches
        // two: the exact-one affected-row guard must roll the transaction back.
        assert!(apply_plan(&mut conn, &plan).is_err());
        assert_eq!(
            conn.query_row("SELECT count(*) FROM t WHERE value='changed'", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(
            plan_changes(
                &conn,
                &[Change::Cell {
                    table: "no_key".into(),
                    column: "value".into(),
                    original: vec![Value::Text("one".into())],
                    value: Value::Text("changed".into())
                }]
            )
            .is_err()
        );
    }
}

fn require_idle_transaction(conn: &Connection) -> Result<(), String> {
    if conn.is_autocommit() { Ok(()) } else {
        Err("Finish the SQL transaction with COMMIT or ROLLBACK before saving table changes, importing or exporting.".into())
    }
}

fn apply_plan(conn: &mut Connection, plan: &ChangePlan) -> Result<(), String> {
    require_idle_transaction(conn)?;
    let foreign_keys: bool = conn
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if plan.rebuild {
        conn.pragma_update(None, "foreign_keys", false)
            .map_err(|e| e.to_string())?;
    }
    let result = apply_transaction(conn, plan);
    if plan.rebuild {
        conn.pragma_update(None, "foreign_keys", foreign_keys)
            .map_err(|e| format!("Foreign-key setting could not be restored: {e}"))?;
    }
    result
}

fn apply_transaction(conn: &mut Connection, plan: &ChangePlan) -> Result<(), String> {
    conn.pragma_update(None, "query_only", false)
        .map_err(|e| e.to_string())?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    if plan.rebuild {
        let temporary_schema: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_temp_schema WHERE type IN ('view','trigger'))",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if temporary_schema {
            return Err(
                "Temporary schema dependencies changed; regenerate the migration plan.".into(),
            );
        }
    }
    if let Some(expected) = plan.schema_version {
        let current: i64 = tx
            .pragma_query_value(None, "schema_version", |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if current != expected {
            return Err("Schema changed after review; regenerate the migration plan.".into());
        }
    }
    let current = objects(&tx).map_err(|e| e.to_string())?;
    for (name, ddl) in &plan.schemas {
        if !current.iter().any(|o| o.name == *name && o.ddl == *ddl) {
            return Err("Table structure changed since review. Nothing was saved; refresh and review again.".into());
        }
    }
    for (index, statement) in plan.statements.iter().enumerate() {
        // Check just before each write, not merely at transaction start: an
        // earlier write's trigger may have changed a later target row.
        for check in plan
            .preconditions
            .iter()
            .filter(|check| check.before_statement == index)
        {
            let matching: i64 = tx
                .query_row(&check.sql, params_from_iter(&check.parameters), |row| {
                    row.get(0)
                })
                .map_err(|e| e.to_string())?;
            if matching != 1 {
                return Err("A row changed since it was loaded. Nothing was saved; pending changes are retained.".into());
            }
        }
        let count = tx
            .execute(&statement.sql, params_from_iter(&statement.parameters))
            .map_err(|e| e.to_string())?;
        if statement
            .expected_rows
            .is_some_and(|expected| count != expected)
        {
            return Err("A row changed since it was loaded. Nothing was saved; pending changes are retained.".into());
        }
    }
    if plan.rebuild {
        let mut check = tx
            .prepare("PRAGMA foreign_key_check")
            .map_err(|e| e.to_string())?;
        if check
            .query([])
            .map_err(|e| e.to_string())?
            .next()
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err("Migration would leave foreign-key violations. Nothing was saved.".into());
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
fn edit(
    conn: &mut Connection,
    table: &str,
    column: &str,
    original: &[Value],
    value: Value,
) -> Result<(), String> {
    let plan = plan_changes(
        conn,
        &[Change::Cell {
            table: table.into(),
            column: column.into(),
            original: original.to_vec(),
            value,
        }],
    )?;
    apply_plan(conn, &plan)
}

fn demo() -> rusqlite::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(include_str!("../demo.sql"))?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_with_unresolved_view_opens_and_recovers_after_repair() {
        let path = std::env::temp_dir().join(format!(
            "rustable-broken-view-{}.sqlite",
            std::process::id()
        ));
        assert!(!path.exists());
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE healthy(id INTEGER); CREATE VIEW unresolved AS SELECT * FROM missing",
            )
            .unwrap();
        drop(connection);
        let mut driver = SqliteDriver::open(Some(&path), Arc::new(AtomicBool::new(false))).unwrap();
        let catalog = driver.objects().unwrap();
        let healthy = catalog.iter().find(|o| o.name == "healthy").unwrap();
        assert_eq!(healthy.columns[0].name, "id");
        let view = catalog.iter().find(|o| o.name == "unresolved").unwrap();
        assert!(view.metadata_error.as_ref().unwrap().contains("missing"));
        assert!(view.ddl.contains("missing"));
        driver
            .connection
            .execute_batch("CREATE TABLE missing(repaired TEXT)")
            .unwrap();
        let catalog = driver.objects().unwrap();
        let view = catalog.iter().find(|o| o.name == "unresolved").unwrap();
        assert!(view.metadata_error.is_none());
        assert_eq!(view.columns[0].name, "repaired");
        drop(driver);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "manual metadata benchmark"]
    fn metadata_cache_benchmark() {
        let mut driver = SqliteDriver::open(None, Arc::new(AtomicBool::new(false))).unwrap();
        for n in 0..500 {
            driver.connection.execute_batch(&format!("CREATE TABLE bench_{n}(id INTEGER PRIMARY KEY,a TEXT,b TEXT,c TEXT,d REAL,e INTEGER,f TEXT,g BLOB)")).unwrap();
        }
        driver.objects().unwrap();
        let mut uncached = Vec::new();
        let mut cached = Vec::new();
        for _ in 0..30 {
            let start = Instant::now();
            std::hint::black_box(objects(&driver.connection).unwrap());
            uncached.push(start.elapsed());
            let start = Instant::now();
            std::hint::black_box(driver.objects().unwrap());
            cached.push(start.elapsed());
        }
        uncached.sort();
        cached.sort();
        eprintln!(
            "505 objects, 30 samples: uncached p95 {:?}; cached p95 {:?} (includes owned result clone)",
            uncached[28], cached[28]
        );
    }

    #[test]
    fn metadata_reuses_reads_and_refreshes_schema_and_rollback() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = SqliteDriver::open(None, cancel.clone()).unwrap();
        driver.objects().unwrap();
        let cached = driver.metadata.as_ref().unwrap().1.as_ptr();
        driver.execute("SELECT 42", true, &cancel);
        driver.objects().unwrap();
        assert_eq!(cached, driver.metadata.as_ref().unwrap().1.as_ptr());
        driver.execute(
            "ALTER TABLE customers ADD COLUMN nickname TEXT",
            false,
            &cancel,
        );
        assert!(
            driver
                .objects()
                .unwrap()
                .iter()
                .find(|o| o.name == "customers")
                .unwrap()
                .columns
                .iter()
                .any(|c| c.name == "nickname")
        );
        // Explicit transaction state must not leak rolled-back metadata.
        driver
            .connection
            .execute_batch("BEGIN; ALTER TABLE customers ADD COLUMN temporary_field TEXT")
            .unwrap();
        assert!(
            driver
                .objects()
                .unwrap()
                .iter()
                .find(|o| o.name == "customers")
                .unwrap()
                .columns
                .iter()
                .any(|c| c.name == "temporary_field")
        );
        assert!(driver.metadata.is_none());
        driver
            .connection
            .execute_batch("ROLLBACK; ALTER TABLE customers ADD COLUMN replacement TEXT")
            .unwrap();
        let objects = driver.objects().unwrap();
        let columns = &objects
            .iter()
            .find(|o| o.name == "customers")
            .unwrap()
            .columns;
        assert!(columns.iter().any(|c| c.name == "replacement"));
        assert!(!columns.iter().any(|c| c.name == "temporary_field"));
    }

    #[test]
    fn metadata_detects_other_connection_schema_changes() {
        let path =
            std::env::temp_dir().join(format!("rustable-metadata-{}.sqlite", std::process::id()));
        assert!(!path.exists());
        let writer = Connection::open(&path).unwrap();
        writer.execute_batch("CREATE TABLE t(id INTEGER)").unwrap();
        let mut driver = SqliteDriver::open(Some(&path), Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(driver.objects().unwrap()[0].columns.len(), 1);
        writer
            .execute_batch("ALTER TABLE t ADD COLUMN label TEXT")
            .unwrap();
        assert_eq!(driver.objects().unwrap()[0].columns[1].name, "label");
        writer.execute_batch("DROP TABLE t").unwrap();
        assert!(driver.objects().unwrap().is_empty());
        drop(driver);
        drop(writer);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn scripts_return_independent_results_and_stop_on_failure() {
        let conn = Connection::open_in_memory().unwrap();
        let cancel = AtomicBool::new(false);
        let report = script(
            &conn,
            "CREATE TABLE t(id INTEGER); INSERT INTO t VALUES (1); SELECT * FROM t; SELECT * FROM missing; INSERT INTO t VALUES(2);",
            false,
            &cancel,
        );
        assert_eq!(report.grids.len(), 3);
        assert!(report.error.unwrap().contains("Statement 4 failed"));
        assert_eq!(report.grids[0].affected, 0);
        assert_eq!(report.grids[1].affected, 1);
        assert_eq!(report.grids[2].rows[0][0], Value::Integer(1));
        assert_eq!(
            query(&conn, "SELECT count(*) FROM t", true).unwrap().rows[0][0],
            Value::Integer(1)
        );
    }

    #[test]
    fn script_transactions_persist_until_explicit_recovery_and_safe_mode_cannot_be_bypassed() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY)")
            .unwrap();
        let cancel = AtomicBool::new(false);
        assert!(
            script(
                &conn,
                "BEGIN; INSERT INTO t VALUES(1); COMMIT;",
                false,
                &cancel
            )
            .error
            .is_none()
        );
        assert!(
            script(
                &conn,
                "BEGIN; INSERT INTO t VALUES(2); SELECT * FROM missing; COMMIT;",
                false,
                &cancel
            )
            .error
            .unwrap()
            .contains("transaction remains open")
        );
        assert!(!conn.is_autocommit());
        assert!(script(&conn, "ROLLBACK", true, &cancel).error.is_none());
        assert!(conn.is_autocommit());
        assert!(
            script(&conn, "BEGIN; INSERT INTO t VALUES(3);", false, &cancel)
                .error
                .is_none()
        );
        assert!(!conn.is_autocommit());
        assert!(script(&conn, "ROLLBACK", true, &cancel).error.is_none());
        assert!(
            script(
                &conn,
                "PRAGMA query_only = OFF; DELETE FROM t;",
                true,
                &cancel
            )
            .error
            .is_some()
        );
        assert_eq!(
            query(&conn, "SELECT count(*) FROM t", true).unwrap().rows[0][0],
            Value::Integer(1)
        );
    }

    #[test]
    fn returning_finishes_with_bounded_preview_and_accurate_count() {
        let conn = demo().unwrap();
        let grid = query_with_budget(
            &conn,
            "UPDATE customers SET status='Test' RETURNING id",
            false,
            16,
        )
        .unwrap();
        assert!(grid.truncated);
        assert_eq!(grid.rows.len(), 2);
        assert_eq!(grid.affected, 2400);
        assert_eq!(
            query(
                &conn,
                "SELECT count(*) FROM customers WHERE status='Test'",
                true
            )
            .unwrap()
            .rows[0][0],
            Value::Integer(2400)
        );
    }

    #[test]
    fn trigger_scripts_use_sqlite_boundaries_and_live_schema() {
        let conn = Connection::open_in_memory().unwrap();
        let report = script(
            &conn,
            "CREATE TABLE t(x); CREATE TABLE audit(x); CREATE TRIGGER log AFTER INSERT ON t BEGIN INSERT INTO audit VALUES('a;b'); INSERT INTO audit VALUES(new.x); END; INSERT INTO t VALUES('日本'); SELECT * FROM audit;",
            false,
            &AtomicBool::new(false),
        );
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(report.grids.len(), 5);
        assert_eq!(report.grids[4].rows.len(), 2);
    }

    #[test]
    fn header_sort_cycle_preserves_filter_and_restores_default_order() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE sample(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO sample VALUES (1,'match-b'),(2,'match-a'),(3,'match-c'),(4,'other');").unwrap();
        let mut request = Browse {
            table: "sample".into(),
            offset: 500,
            filter_column: Some("name".into()),
            filter: "match".into(),
            ..Default::default()
        };
        for expected in [[3, 1, 2], [2, 1, 3], [1, 2, 3]] {
            request.cycle_sort("name".into());
            assert_eq!(request.offset, 0);
            let result = browse_table(&conn, &request).unwrap();
            let ids: Vec<_> = result.grid.rows.iter().map(|row| row[0].clone()).collect();
            assert_eq!(ids, expected.map(Value::Integer));
            request.offset = 500;
        }
        assert!(request.sort.is_none());
        request.cycle_sort("name".into());
        request.cycle_sort("id".into());
        assert_eq!(request.sort, Some(("id".into(), true)));
        assert_eq!(request.filter, "match");
    }

    #[test]
    fn demo_browse_filter_sort_and_page() {
        let conn = demo().unwrap();
        let TableData { grid, columns, .. } = browse_table(
            &conn,
            &Browse {
                table: "customers".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(grid.rows.len(), PAGE_SIZE);
        assert!(grid.truncated);
        assert!(columns[0].pk);
        let TableData { grid, .. } = browse_table(
            &conn,
            &Browse {
                table: "customers".into(),
                offset: 500,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(grid.rows[0][0], Value::Integer(501));
        let TableData { grid, .. } = browse_table(
            &conn,
            &Browse {
                table: "customers".into(),
                filter_column: Some("name".into()),
                filter: "Avery".into(),
                sort: Some(("id".into(), true)),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!grid.rows.is_empty());
        assert!(grid.rows.iter().all(|r| display(&r[1]).contains("Avery")));
    }

    #[test]
    fn safe_queries_and_multi_statements_cannot_mutate() {
        let conn = demo().unwrap();
        assert!(query(&conn, "DELETE FROM customers", true).is_err());
        assert!(query(&conn, "DELETE FROM customers; SELECT 1", false).is_err());
        assert_eq!(
            query(&conn, "SELECT count(*) FROM customers", true)
                .unwrap()
                .rows[0][0],
            Value::Integer(2400)
        );
        assert_eq!(
            query(&conn, "SELECT ';' AS punctuation; -- comment", true)
                .unwrap()
                .rows
                .len(),
            1
        );
    }

    #[test]
    fn edits_are_optimistic_and_preserve_types() {
        let mut conn = demo().unwrap();
        let original = query(&conn, "SELECT * FROM customers WHERE id = 1", true)
            .unwrap()
            .rows
            .remove(0);
        edit(
            &mut conn,
            "customers",
            "name",
            &original,
            Value::Text("O'Brien".into()),
        )
        .unwrap();
        assert!(
            edit(
                &mut conn,
                "customers",
                "name",
                &original,
                Value::Text("stale".into())
            )
            .is_err()
        );
        assert_eq!(
            query(&conn, "SELECT name FROM customers WHERE id=1", true)
                .unwrap()
                .rows[0][0],
            Value::Text("O'Brien".into())
        );
    }

    #[test]
    fn results_are_bounded_and_queries_cancel() {
        let conn = demo().unwrap();
        let sql = "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000) SELECT x FROM n";
        let grid = query(&conn, sql, true).unwrap();
        assert_eq!(grid.rows.len(), QUERY_LIMIT);
        assert!(grid.truncated);
        conn.progress_handler(100, Some(|| true)).unwrap();
        assert!(query(&conn, sql, true).is_err());
    }

    #[test]
    fn quoted_identifiers_and_csv_are_lossless() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE \"odd\"\"table\" (id INTEGER PRIMARY KEY, \"a b\" TEXT); INSERT INTO \"odd\"\"table\" VALUES(1, 'hello');").unwrap();
        let TableData { grid, .. } = browse_table(
            &conn,
            &Browse {
                table: "odd\"table".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(grid.rows[0][1], Value::Text("hello".into()));
        let path =
            std::env::temp_dir().join(format!("rustable-export-test-{}.csv", std::process::id()));
        let grid = Grid {
            columns: vec!["text".into()],
            rows: vec![vec![Value::Text("a,\"b\"\nc".into())]],
            ..Default::default()
        };
        csv::export(&path, &grid, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "\"text\"\n\"a,\"\"b\"\"\nc\"\n"
        );
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod adapter_tests {
    use super::*;

    #[test]
    fn engine_independent_values_round_trip_through_binding() {
        let connection = Connection::open_in_memory().unwrap();
        let expected = vec![
            Value::Null,
            Value::Integer(i64::MAX),
            Value::Real(1.25),
            Value::Text("日本\0é".into()),
            Value::Blob(vec![0, 255, 42]),
        ];
        let mut stmt = connection.prepare("SELECT ?, ?, ?, ?, ?").unwrap();
        let names = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let mut rows = stmt.query(params_from_iter(expected.iter())).unwrap();
        let grid = collect(&mut rows, names, 10).unwrap();
        assert_eq!(grid.rows, vec![expected]);
    }
}

#[cfg(test)]
mod change_tests {
    use super::*;
    fn cell(id: i64, column: &str, value: Value) -> Change {
        Change::Cell {
            table: "t".into(),
            column: column.into(),
            original: vec![
                Value::Integer(id),
                Value::Text("old".into()),
                Value::Integer(0),
            ],
            value,
        }
    }
    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT, n INTEGER); INSERT INTO t VALUES(1,'old',0),(2,'old',0);").unwrap();
        conn
    }
    #[test]
    fn review_merges_row_edits_and_binds_values_without_sql_interpolation() {
        let mut conn = connection();
        let text = "a'; DROP TABLE t; --";
        let plan = plan_changes(
            &conn,
            &[
                cell(1, "name", Value::Text(text.into())),
                cell(1, "n", Value::Integer(9)),
            ],
        )
        .unwrap();
        assert_eq!(plan.statements.len(), 1);
        assert!(!plan.statements[0].sql.contains(text));
        assert_eq!(
            conn.query_row("SELECT n FROM t WHERE id=1", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        apply_plan(&mut conn, &plan).unwrap();
        assert_eq!(
            conn.query_row("SELECT name,n FROM t WHERE id=1", [], |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?
            )))
            .unwrap(),
            (text.into(), 9)
        );
    }
    #[test]
    fn conflict_rolls_back_all_statements_and_plan_can_be_retried() {
        let mut conn = connection();
        let plan = plan_changes(
            &conn,
            &[
                cell(1, "n", Value::Integer(8)),
                cell(2, "n", Value::Integer(9)),
            ],
        )
        .unwrap();
        conn.execute("UPDATE t SET n=3 WHERE id=2", []).unwrap();
        assert!(apply_plan(&mut conn, &plan).is_err());
        assert_eq!(
            conn.query_row("SELECT n FROM t WHERE id=1", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        conn.execute("UPDATE t SET n=0 WHERE id=2", []).unwrap();
        apply_plan(&mut conn, &plan).unwrap();
    }
    #[test]
    fn rename_is_quoted_and_stale_schema_invalidates_review() {
        let mut conn = connection();
        let change = Change::RenameColumn {
            table: "t".into(),
            from: "name".into(),
            to: "display \"name".into(),
        };
        let plan = plan_changes(&conn, &[change]).unwrap();
        apply_plan(&mut conn, &plan).unwrap();
        assert!(
            columns(&conn, "t")
                .unwrap()
                .iter()
                .any(|c| c.name == "display \"name")
        );
        assert!(apply_plan(&mut conn, &plan).is_err());
    }
}

#[cfg(test)]
mod add_column_tests {
    use super::*;

    fn change(name: &str, data_type: &str, default: Option<Value>, not_null: bool) -> Change {
        Change::AddColumn {
            table: "items".into(),
            definition: ColumnDefinition {
                name: name.into(),
                data_type: data_type.into(),
                default,
                not_null,
            },
        }
    }
    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE items(id INTEGER PRIMARY KEY); INSERT INTO items VALUES(1),(2);",
        )
        .unwrap();
        conn
    }
    #[test]
    fn added_column_preserves_quoted_names_and_literal_default() {
        let mut conn = connection();
        let value = "'); DROP TABLE items; --";
        let plan = plan_changes(
            &conn,
            &[change(
                "display \"name",
                "VARCHAR(100)",
                Some(Value::Text(value.into())),
                true,
            )],
        )
        .unwrap();
        assert_eq!(
            columns(&conn, "items").unwrap().len(),
            1,
            "review must not mutate schema"
        );
        apply_plan(&mut conn, &plan).unwrap();
        let cols = columns(&conn, "items").unwrap();
        assert_eq!(cols[1].name, "display \"name");
        assert_eq!(cols[1].data_type, "VARCHAR(100)");
        assert!(cols[1].not_null);
        assert_eq!(
            conn.query_row(
                "SELECT \"display \"\"name\" FROM items WHERE id=1",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            value
        );
    }
    #[test]
    fn invalid_defaults_are_rejected_before_execution() {
        let conn = connection();
        for default in [
            None,
            Some(Value::Null),
            Some(Value::Text("a\0b".into())),
            Some(Value::Real(f64::INFINITY)),
        ] {
            assert!(plan_changes(&conn, &[change("required", "TEXT", default, true)]).is_err());
        }
        assert!(plan_changes(&conn, &[change("ID", "TEXT", None, false)]).is_err());
    }
    #[test]
    fn failed_schema_batch_rolls_back_prior_additions() {
        let mut conn = connection();
        let plan = plan_changes(
            &conn,
            &[
                change("extra", "TEXT", None, false),
                change("extra", "INTEGER", None, false),
            ],
        )
        .unwrap();
        assert!(apply_plan(&mut conn, &plan).is_err());
        assert_eq!(columns(&conn, "items").unwrap().len(), 1);
        assert_eq!(
            conn.query_row("SELECT count(*) FROM items", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    fn alteration(
        table: &str,
        column: &str,
        kind: Option<&str>,
        required: Option<bool>,
        default: Option<Option<Value>>,
    ) -> Change {
        Change::AlterColumn {
            table: table.into(),
            column: column.into(),
            data_type: kind.map(str::to_owned),
            not_null: required,
            default,
        }
    }
    #[test]
    fn rebuild_preserves_rows_dependencies_generated_values_and_sequence() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;
            CREATE TABLE parent(id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL UNIQUE, n INTEGER DEFAULT 1, doubled INTEGER GENERATED ALWAYS AS (n*2) STORED);
            CREATE TABLE child(id INTEGER PRIMARY KEY, parent INTEGER REFERENCES parent(id));
            CREATE TABLE audit(message TEXT);
            CREATE INDEX parent_n ON parent(n) WHERE n>0;
            CREATE VIEW labels AS SELECT label,n FROM parent;
            CREATE TRIGGER log_parent AFTER UPDATE ON parent BEGIN INSERT INTO audit VALUES(new.label); END;
            INSERT INTO parent(id,label,n) VALUES(1,'one',3),(99,'deleted',4);
            DELETE FROM parent WHERE id=99;
            INSERT INTO child VALUES(1,1);").unwrap();
        let plan = plan_changes(
            &conn,
            &[alteration(
                "parent",
                "n",
                Some("REAL"),
                Some(true),
                Some(Some(Value::Real(2.5))),
            )],
        )
        .unwrap();
        assert!(plan.rebuild);
        apply_plan(&mut conn, &plan).unwrap();
        assert!(
            conn.pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
                .unwrap()
        );
        assert_eq!(
            conn.query_row("SELECT n,doubled FROM parent WHERE id=1", [], |r| Ok((
                r.get::<_, f64>(0)?,
                r.get::<_, i64>(1)?
            )))
            .unwrap(),
            (3.0, 6)
        );
        assert_eq!(
            conn.query_row("SELECT label FROM labels", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "one"
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM child", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        conn.execute("INSERT INTO parent(label) VALUES('two')", [])
            .unwrap();
        assert_eq!(conn.last_insert_rowid(), 100);
        conn.execute("UPDATE parent SET label='changed' WHERE id=1", [])
            .unwrap();
        assert_eq!(
            conn.query_row("SELECT message FROM audit", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "changed"
        );
        assert!(conn.execute("INSERT INTO child VALUES(2,500)", []).is_err());
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='parent_n'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    #[test]
    fn invalid_existing_data_rolls_back_schema_and_restores_foreign_keys() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE t(id INTEGER PRIMARY KEY, value TEXT); INSERT INTO t VALUES(1,NULL); CREATE VIEW v AS SELECT * FROM t;").unwrap();
        let ddl: String = conn
            .query_row("SELECT sql FROM sqlite_schema WHERE name='t'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let plan =
            plan_changes(&conn, &[alteration("t", "value", None, Some(true), None)]).unwrap();
        assert!(apply_plan(&mut conn, &plan).is_err());
        assert_eq!(
            conn.query_row("SELECT sql FROM sqlite_schema WHERE name='t'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            ddl
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM v", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(
            conn.pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
                .unwrap()
        );
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name LIKE '__rustable_rebuild_%'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
    #[test]
    fn dependency_change_after_review_prevents_rebuild() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, n INTEGER);")
            .unwrap();
        let plan = plan_changes(&conn, &[alteration("t", "n", Some("TEXT"), None, None)]).unwrap();
        conn.execute_batch("CREATE INDEX new_index ON t(n);")
            .unwrap();
        assert!(
            apply_plan(&mut conn, &plan)
                .unwrap_err()
                .contains("Schema changed")
        );
        assert_eq!(columns(&conn, "t").unwrap()[1].data_type, "INTEGER");
    }
    #[test]
    fn affinity_conflicts_cannot_silently_discard_rows() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(value TEXT UNIQUE ON CONFLICT IGNORE); INSERT INTO t VALUES('01'),('1');").unwrap();
        let plan = plan_changes(
            &conn,
            &[alteration("t", "value", Some("INTEGER"), None, None)],
        )
        .unwrap();
        assert!(apply_plan(&mut conn, &plan).is_err());
        assert_eq!(
            conn.query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(columns(&conn, "t").unwrap()[0].data_type, "TEXT");
    }
    #[test]
    fn autoincrement_high_water_survives_an_empty_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY AUTOINCREMENT, value TEXT); INSERT INTO t VALUES(100,'a'); DELETE FROM t;").unwrap();
        let plan = plan_changes(
            &conn,
            &[alteration(
                "t",
                "value",
                None,
                None,
                Some(Some(Value::Text("default".into()))),
            )],
        )
        .unwrap();
        apply_plan(&mut conn, &plan).unwrap();
        conn.execute("INSERT INTO t DEFAULT VALUES", []).unwrap();
        assert_eq!(conn.last_insert_rowid(), 101);
    }

    #[test]
    fn preserves_hidden_rowids_and_without_rowid_tables() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(value TEXT); INSERT INTO t(rowid,value) VALUES(99,'a'); CREATE TABLE wr(id TEXT PRIMARY KEY, value TEXT) WITHOUT ROWID; INSERT INTO wr VALUES('key','b');").unwrap();
        let plan = plan_changes(
            &conn,
            &[
                alteration(
                    "t",
                    "value",
                    None,
                    None,
                    Some(Some(Value::Text("new".into()))),
                ),
                alteration("wr", "value", None, Some(true), None),
            ],
        )
        .unwrap();
        apply_plan(&mut conn, &plan).unwrap();
        assert_eq!(
            conn.query_row("SELECT rowid FROM t", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            99
        );
        assert_eq!(
            conn.query_row("SELECT value FROM wr WHERE id='key'", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "b"
        );
    }
}

#[cfg(test)]
mod index_tests {
    use super::*;
    #[test]
    fn metadata_distinguishes_constraint_expression_and_partial_indexes() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT UNIQUE, score REAL);
            CREATE INDEX \"by name\" ON t(name COLLATE NOCASE DESC, score) WHERE score>0;
            CREATE INDEX expression ON t(lower(name));
            CREATE TRIGGER audit AFTER INSERT ON t BEGIN SELECT new.id; END;",
        )
        .unwrap();
        let table = browse_table(
            &conn,
            &Browse {
                table: "t".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            table
                .indexes
                .iter()
                .any(|i| i.unique && i.origin == "u" && i.ddl.is_empty())
        );
        let index = table.indexes.iter().find(|i| i.name == "by name").unwrap();
        assert!(index.partial);
        assert!(!index.unique);
        assert_eq!(index.columns, vec!["name ↓ COLLATE NOCASE", "score"]);
        assert!(index.ddl.contains("WHERE score>0"));
        assert_eq!(
            table
                .indexes
                .iter()
                .find(|i| i.name == "expression")
                .unwrap()
                .columns,
            vec!["(expression)"]
        );
        assert_eq!(table.triggers[0].name, "audit");
        assert!(table.triggers[0].ddl.contains("SELECT new.id"));
    }
}

#[cfg(test)]
mod index_edit_tests {
    use super::*;
    fn create(name: &str, unique: bool) -> Change {
        Change::CreateIndex {
            table: "t".into(),
            name: name.into(),
            columns: vec![("name".into(), true), ("score".into(), false)],
            unique,
        }
    }
    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT, score INTEGER, code TEXT UNIQUE); INSERT INTO t VALUES(1,'a',1,'one'),(2,'a',1,'two'); CREATE INDEX old_index ON t(name);").unwrap();
        conn
    }
    #[test]
    fn index_creation_quotes_identifiers_and_preserves_key_order() {
        let mut conn = connection();
        let name = "index\"; SELECT 1; --";
        let plan = plan_changes(&conn, &[create(name, false)]).unwrap();
        assert!(!indexes(&conn, "t").unwrap().iter().any(|i| i.name == name));
        apply_plan(&mut conn, &plan).unwrap();
        let indexes = indexes(&conn, "t").unwrap();
        let index = indexes.iter().find(|i| i.name == name).unwrap();
        assert_eq!(index.columns, vec!["name ↓", "score"]);
        assert!(!index.unique);
    }
    #[test]
    fn unique_failure_restores_dropped_index_atomically() {
        let mut conn = connection();
        let plan = plan_changes(
            &conn,
            &[
                Change::DropIndex {
                    table: "t".into(),
                    name: "old_index".into(),
                },
                create("old_index", true),
            ],
        )
        .unwrap();
        assert!(apply_plan(&mut conn, &plan).is_err());
        let indexes = indexes(&conn, "t").unwrap();
        let index = indexes.iter().find(|i| i.name == "old_index").unwrap();
        assert!(!index.unique);
        assert_eq!(index.columns, vec!["name"]);
    }
    #[test]
    fn implicit_indexes_and_stale_reviews_are_protected() {
        let mut conn = connection();
        let implicit = indexes(&conn, "t")
            .unwrap()
            .into_iter()
            .find(|i| i.origin == "u")
            .unwrap();
        assert!(
            plan_changes(
                &conn,
                &[Change::DropIndex {
                    table: "t".into(),
                    name: implicit.name
                }]
            )
            .is_err()
        );
        let plan = plan_changes(&conn, &[create("new_index", false)]).unwrap();
        conn.execute_batch("CREATE INDEX concurrent ON t(score);")
            .unwrap();
        assert!(
            apply_plan(&mut conn, &plan)
                .unwrap_err()
                .contains("Schema changed")
        );
        assert!(
            !indexes(&conn, "t")
                .unwrap()
                .iter()
                .any(|i| i.name == "new_index")
        );
    }
    #[test]
    fn successful_drop_and_replacement_share_reviewed_plan() {
        let mut conn = connection();
        let plan = plan_changes(
            &conn,
            &[
                create("old_index", false),
                Change::DropIndex {
                    table: "t".into(),
                    name: "old_index".into(),
                },
            ],
        )
        .unwrap();
        assert!(plan.statements[0].sql.starts_with("DROP INDEX"));
        apply_plan(&mut conn, &plan).unwrap();
        assert_eq!(
            indexes(&conn, "t")
                .unwrap()
                .iter()
                .find(|i| i.name == "old_index")
                .unwrap()
                .columns,
            vec!["name ↓", "score"]
        );
    }
}

#[cfg(test)]
mod performance_tests {
    use super::*;

    #[test]
    fn oversized_rows_are_rejected_before_owned_conversion() {
        let conn = Connection::open_in_memory().unwrap();
        let grid = query_with_budget(&conn, "SELECT zeroblob(20000000)", true, 1024).unwrap();
        assert!(grid.rows.is_empty());
        assert!(grid.truncated);
        // Invalid UTF-8 would fail owned Value conversion; a rejected row should
        // never attempt that conversion, even if its first field is invalid.
        let grid = query_with_budget(
            &conn,
            "SELECT CAST(x'ff' AS TEXT), zeroblob(2048)",
            true,
            1024,
        )
        .unwrap();
        assert!(grid.rows.is_empty());
        assert!(grid.truncated);
        let grid = query_with_budget(
            &conn,
            "SELECT '1234', 7 UNION ALL SELECT '5678', 8",
            true,
            12,
        )
        .unwrap();
        assert_eq!(grid.rows.len(), 1);
        assert!(grid.truncated);
    }

    #[test]
    #[ignore = "manual release-mode million-row performance benchmark"]
    fn million_row_browse_baseline() {
        let path =
            std::env::temp_dir().join(format!("rustable-bench-{}.sqlite", std::process::id()));
        assert!(!path.exists(), "Refusing to overwrite a previous fixture");
        struct Fixture(std::path::PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let fixture = Fixture(path);
        let conn = Connection::open(&fixture.0).unwrap();
        conn.execute_batch("CREATE TABLE records(id INTEGER PRIMARY KEY, name TEXT NOT NULL, score INTEGER NOT NULL);
            WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000)
            INSERT INTO records SELECT x, printf('Record %07d',x), x%10000 FROM n;
            CREATE INDEX records_score ON records(score); ANALYZE;").unwrap();
        println!(
            "fixture_bytes={}",
            std::fs::metadata(&fixture.0).unwrap().len()
        );
        for (label, offset, sort) in [
            ("first_pk_page", 0, None),
            ("deep_pk_page", 900000, None),
            ("indexed_sort_first", 0, Some(("score".into(), false))),
            ("indexed_sort_deep", 900000, Some(("score".into(), false))),
        ] {
            let request = Browse {
                table: "records".into(),
                offset,
                sort,
                ..Default::default()
            };
            let mut samples = Vec::new();
            for i in 0..55 {
                let start = Instant::now();
                let data = browse_table(&conn, &request).unwrap();
                assert_eq!(data.grid.rows.len(), PAGE_SIZE);
                assert!(data.grid.truncated);
                if i >= 5 {
                    samples.push(start.elapsed().as_secs_f64() * 1000.0);
                }
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "{label}: p50={:.3}ms p95={:.3}ms p99={:.3}ms n={}",
                samples[24],
                samples[47],
                samples[49],
                samples.len()
            );
        }
        let previous = Browse {
            table: "records".into(),
            offset: 899500,
            ..Default::default()
        };
        let previous_data = browse_table(&conn, &previous).unwrap();
        let cursor =
            PageCursor::from_page(&previous, &previous_data, page_version(&conn).unwrap()).unwrap();
        let next = Browse {
            offset: 900000,
            ..previous
        };
        let expected = browse_table(&conn, &next).unwrap();
        let mut samples = Vec::new();
        for i in 0..55 {
            let start = Instant::now();
            assert!(cursor.matches(&next, page_version(&conn).unwrap()));
            let data = browse_table_after(&conn, &next, Some(&cursor)).unwrap();
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(data.grid.rows, expected.grid.rows);
            if i >= 5 {
                samples.push(elapsed);
            }
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "deep_pk_keyset_next: p50={:.3}ms p95={:.3}ms p99={:.3}ms n={}",
            samples[24],
            samples[47],
            samples[49],
            samples.len()
        );
        let current = Browse {
            table: "records".into(),
            offset: 900500,
            ..Default::default()
        };
        let data = browse_table(&conn, &current).unwrap();
        let cursor = PageCursor::from_page(&current, &data, page_version(&conn).unwrap()).unwrap();
        let previous = Browse {
            offset: 900000,
            ..current
        };
        let expected = browse_table(&conn, &previous).unwrap();
        let mut samples = Vec::new();
        for i in 0..55 {
            let start = Instant::now();
            assert!(cursor.matches(&previous, page_version(&conn).unwrap()));
            let data = browse_table_after(&conn, &previous, Some(&cursor)).unwrap();
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(data.grid.rows, expected.grid.rows);
            assert_eq!(data.grid.truncated, expected.grid.truncated);
            if i >= 5 {
                samples.push(elapsed);
            }
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "deep_pk_keyset_previous: p50={:.3}ms p95={:.3}ms p99={:.3}ms n={}",
            samples[24],
            samples[47],
            samples[49],
            samples.len()
        );
        let grid = query(&conn, "SELECT * FROM records", true).unwrap();
        assert_eq!(grid.rows.len(), QUERY_LIMIT);
        assert!(grid.truncated);
        let cancel = Arc::new(AtomicBool::new(false));
        let callback = cancel.clone();
        conn.progress_handler(1000, Some(move || callback.load(Ordering::Relaxed))).unwrap();
        std::thread::scope(|scope| {
            let trigger = cancel.clone();
            scope.spawn(move || {
                std::thread::sleep(Duration::from_millis(25));
                trigger.store(true, Ordering::Relaxed);
            });
            let start = Instant::now();
            let result = query(
                &conn,
                "SELECT sum(a.score*b.score) FROM records a CROSS JOIN records b",
                true,
            );
            assert!(result.is_err());
            println!(
                "cancel_total_ms={:.3} (includes 25ms trigger delay)",
                start.elapsed().as_secs_f64() * 1000.0
            );
        });
        cancel.store(false, Ordering::Relaxed);
        assert_eq!(query(&conn, "SELECT 1", true).unwrap().rows.len(), 1);
    }
}

#[cfg(test)]
mod paging_tests {
    use super::*;
    fn driver() -> SqliteDriver {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, label TEXT); WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1600) INSERT INTO t SELECT x*3, printf('row %d',x) FROM n;").unwrap();
        SqliteDriver {
            connection,
            cursor: None,
            metadata: None,
        }
    }
    #[test]
    fn sequential_pages_match_offset_with_gaps_filters_and_descending_order() {
        for descending in [false, true] {
            let mut driver = driver();
            let mut request = Browse {
                table: "t".into(),
                sort: Some(("id".into(), descending)),
                filter_column: Some("label".into()),
                filter: "row".into(),
                ..Default::default()
            };
            for offset in [0, 500, 1000, 500, 0, 1000, 1500, 1000, 0, 1000, 500] {
                request.offset = offset;
                let expected = browse_table(&driver.connection, &request).unwrap();
                let actual = driver.browse(&request).unwrap();
                assert_eq!(actual.grid.rows, expected.grid.rows);
                assert_eq!(actual.grid.truncated, expected.grid.truncated);
            }
        }
    }
    #[test]
    fn reverse_page_with_large_values_keeps_offset_prefix_and_limit() {
        let mut driver = driver();
        driver
            .connection
            .execute_batch("UPDATE t SET label=printf('%035000d',id) WHERE id<=1500;")
            .unwrap();
        let mut request = Browse {
            table: "t".into(),
            offset: 500,
            ..Default::default()
        };
        driver.browse(&request).unwrap();
        request.offset = 0;
        assert!(
            driver
                .cursor
                .as_ref()
                .unwrap()
                .matches(&request, page_version(&driver.connection).unwrap())
        );
        let expected = browse_table(&driver.connection, &request).unwrap();
        assert!(expected.grid.rows.len() < PAGE_SIZE);
        let actual = driver.browse(&request).unwrap();
        assert_eq!(actual.grid.rows, expected.grid.rows);
        assert_eq!(actual.grid.truncated, expected.grid.truncated);
        assert!(driver.cursor.is_none());
    }

    #[test]
    fn cursor_rejects_changed_request_and_database_version() {
        let mut driver = driver();
        let request = Browse {
            table: "t".into(),
            ..Default::default()
        };
        driver.browse(&request).unwrap();
        let cursor = driver.cursor.as_ref().unwrap();
        let mut next = request.clone();
        next.offset = 500;
        assert!(cursor.matches(&next, cursor.version));
        assert!(!cursor.matches(&next, (cursor.version.0 + 1, cursor.version.1)));
        assert!(!cursor.matches(&next, (cursor.version.0, cursor.version.1 + 1)));
        next.filter = "changed".into();
        assert!(!cursor.matches(&next, cursor.version));
        next = request.clone();
        next.offset = 1000;
        assert!(!cursor.matches(&next, cursor.version));
        let report = driver.execute("SELECT 1", true, &AtomicBool::new(false));
        assert!(report.error.is_none());
        assert!(driver.cursor.is_none());
    }
    #[test]
    fn external_writes_invalidate_cached_page_boundaries() {
        let path = std::env::temp_dir().join(format!(
            "rustable-cursor-test-{}.sqlite",
            std::process::id()
        ));
        assert!(!path.exists());
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY); WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1200) INSERT INTO t SELECT x*3 FROM n;").unwrap();
        let mut driver = SqliteDriver {
            connection: conn,
            cursor: None,
            metadata: None,
        };
        let mut request = Browse {
            table: "t".into(),
            ..Default::default()
        };
        driver.browse(&request).unwrap();
        let writer = Connection::open(&path).unwrap();
        writer.execute("INSERT INTO t VALUES(1)", []).unwrap();
        request.offset = 500;
        assert!(
            !driver
                .cursor
                .as_ref()
                .unwrap()
                .matches(&request, page_version(&driver.connection).unwrap())
        );
        let expected = browse_table(&driver.connection, &request).unwrap();
        assert_eq!(
            driver.browse(&request).unwrap().grid.rows,
            expected.grid.rows
        );
        drop(writer);
        drop(driver);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn nullable_keys_and_non_unique_sorting_use_offset() {
        let mut driver = driver();
        let request = Browse {
            table: "t".into(),
            sort: Some(("label".into(), false)),
            ..Default::default()
        };
        driver.browse(&request).unwrap();
        assert!(driver.cursor.is_none());
        driver.connection.execute_batch("CREATE TABLE nullable(id TEXT PRIMARY KEY, label TEXT); INSERT INTO nullable SELECT NULL,label FROM t; CREATE TABLE descending_pk(id INTEGER PRIMARY KEY DESC, label TEXT); INSERT INTO descending_pk SELECT NULL,label FROM t;").unwrap();
        for table in ["nullable", "descending_pk"] {
            let request = Browse {
                table: table.into(),
                ..Default::default()
            };
            driver.browse(&request).unwrap();
            assert!(driver.cursor.is_none());
        }
    }
}

#[cfg(test)]
mod delete_tests {
    use super::*;
    fn fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE t(id INTEGER PRIMARY KEY, value TEXT); INSERT INTO t VALUES(1,'one'),(2,'two');").unwrap();
        conn
    }
    #[test]
    fn delete_supersedes_cell_edits_and_deduplicates_rows() {
        let mut conn = fixture();
        let original = vec![Value::Integer(1), Value::Text("one".into())];
        let deletion = Change::DeleteRow {
            table: "t".into(),
            original: original.clone(),
        };
        let plan = plan_changes(
            &conn,
            &[
                Change::Cell {
                    table: "t".into(),
                    column: "value".into(),
                    original,
                    value: Value::Text("changed".into()),
                },
                deletion.clone(),
                deletion,
            ],
        )
        .unwrap();
        assert_eq!(plan.statements.len(), 1);
        assert!(plan.statements[0].sql.starts_with("DELETE"));
        assert_eq!(plan.statements[0].expected_rows, Some(1));
        apply_plan(&mut conn, &plan).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn stale_delete_rolls_back_entire_batch_and_fk_failure_preserves_parent() {
        let mut conn = fixture();
        let changes = vec![
            Change::DeleteRow {
                table: "t".into(),
                original: vec![Value::Integer(1), Value::Text("one".into())],
            },
            Change::DeleteRow {
                table: "t".into(),
                original: vec![Value::Integer(2), Value::Text("two".into())],
            },
        ];
        let plan = plan_changes(&conn, &changes).unwrap();
        conn.execute("UPDATE t SET value='external' WHERE id=2", [])
            .unwrap();
        assert!(apply_plan(&mut conn, &plan).is_err());
        assert_eq!(
            conn.query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        conn.execute_batch(
            "CREATE TABLE child(id INTEGER REFERENCES t(id)); INSERT INTO child VALUES(1);",
        )
        .unwrap();
        let plan = plan_changes(&conn, &changes[..1]).unwrap();
        assert!(apply_plan(&mut conn, &plan).is_err());
        assert_eq!(
            conn.query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
}

#[cfg(test)]
mod insert_tests {
    use super::*;

    fn insert(table: &str, fields: Vec<(&str, Value)>) -> Change {
        Change::InsertRow {
            id: 0,
            table: table.into(),
            fields: fields.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        }
    }

    #[test]
    fn defaults_null_types_generated_values_and_identical_rows() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT DEFAULT 'default', n INTEGER DEFAULT 7, r REAL, b BLOB, computed TEXT GENERATED ALWAYS AS (name || n) STORED);").unwrap();
        let explicit = insert(
            "t",
            vec![
                ("name", Value::Null),
                ("n", Value::Integer(42)),
                ("r", Value::Real(1.25)),
                ("b", Value::Blob(vec![0, 255, 12])),
            ],
        );
        let plan = plan_changes(&conn, &[insert("t", vec![]), explicit.clone(), explicit]).unwrap();
        assert_eq!(plan.statements[0].sql, "INSERT INTO \"t\" DEFAULT VALUES;");
        assert_eq!(plan.statements.len(), 3);
        assert_eq!(
            conn.query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        apply_plan(&mut conn, &plan).unwrap();
        type Inserted = (
            i64,
            Option<String>,
            i64,
            Option<f64>,
            Option<Vec<u8>>,
            Option<String>,
        );
        let rows: Vec<Inserted> = conn
            .prepare("SELECT * FROM t ORDER BY id")
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            rows[0],
            (
                1,
                Some("default".into()),
                7,
                None,
                None,
                Some("default7".into())
            )
        );
        assert_eq!(
            rows[1],
            (2, None, 42, Some(1.25), Some(vec![0, 255, 12]), None)
        );
        assert_eq!(rows[2].0, 3);
        assert!(plan_changes(&conn, &[insert("t", vec![("computed", Value::Null)])]).is_err());
        assert!(plan_changes(&conn, &[insert("t", vec![("missing", Value::Null)])]).is_err());
        assert!(
            plan_changes(
                &conn,
                &[insert("t", vec![("n", Value::Null), ("n", Value::Null)])]
            )
            .is_err()
        );
    }

    #[test]
    fn bound_values_and_quoted_identifiers_are_not_sql() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE \"odd\"\"table\" (\"a\"\"b\" TEXT);")
            .unwrap();
        let text = "'); DROP TABLE t; -- 日本";
        let plan = plan_changes(
            &conn,
            &[insert(
                "odd\"table",
                vec![("a\"b", Value::Text(text.into()))],
            )],
        )
        .unwrap();
        assert!(!plan.statements[0].sql.contains(text));
        apply_plan(&mut conn, &plan).unwrap();
        assert_eq!(
            conn.query_row("SELECT * FROM \"odd\"\"table\"", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            text
        );
    }

    #[test]
    fn insert_failure_rolls_back_updates_deletes_and_prior_inserts() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE); INSERT INTO t VALUES(1,'one'),(2,'two');").unwrap();
        let changes = vec![
            Change::Cell {
                table: "t".into(),
                column: "name".into(),
                original: vec![Value::Integer(1), Value::Text("one".into())],
                value: Value::Text("changed".into()),
            },
            Change::DeleteRow {
                table: "t".into(),
                original: vec![Value::Integer(2), Value::Text("two".into())],
            },
            insert("t", vec![("name", Value::Text("new".into()))]),
            insert("t", vec![("name", Value::Text("new".into()))]),
        ];
        let plan = plan_changes(&conn, &changes).unwrap();
        assert!(apply_plan(&mut conn, &plan).is_err());
        assert_eq!(
            conn.query_row(
                "SELECT group_concat(name, ',') FROM t ORDER BY id",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "one,two"
        );
        let plan = plan_changes(&conn, &changes[..3]).unwrap();
        apply_plan(&mut conn, &plan).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn insert_constraints_triggers_and_without_rowid_keys() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE parent(id INTEGER PRIMARY KEY); INSERT INTO parent VALUES(1); CREATE TABLE child(a INTEGER REFERENCES parent(id), b TEXT NOT NULL, PRIMARY KEY(a,b)) WITHOUT ROWID; CREATE TABLE audit(value TEXT); CREATE TRIGGER log_child AFTER INSERT ON child BEGIN INSERT INTO audit VALUES(new.b); END;").unwrap();
        let valid = insert(
            "child",
            vec![("a", Value::Integer(1)), ("b", Value::Text("valid".into()))],
        );
        let invalid = insert(
            "child",
            vec![
                ("a", Value::Integer(2)),
                ("b", Value::Text("invalid".into())),
            ],
        );
        let plan = plan_changes(&conn, &[valid.clone(), invalid]).unwrap();
        assert!(apply_plan(&mut conn, &plan).is_err());
        assert_eq!(
            conn.query_row("SELECT count(*) FROM audit", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        let plan = plan_changes(&conn, &[valid]).unwrap();
        apply_plan(&mut conn, &plan).unwrap();
        assert_eq!(
            conn.query_row("SELECT value FROM audit", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "valid"
        );
        let missing_required =
            plan_changes(&conn, &[insert("child", vec![("a", Value::Integer(1))])]).unwrap();
        assert!(apply_plan(&mut conn, &missing_required).is_err());
        let plan = plan_changes(
            &conn,
            &[insert(
                "child",
                vec![
                    ("a", Value::Integer(1)),
                    ("b", Value::Text("second".into())),
                ],
            )],
        )
        .unwrap();
        conn.execute_batch("DROP TRIGGER log_child;").unwrap();
        assert!(apply_plan(&mut conn, &plan).is_err());
    }

    #[test]
    fn trigger_ignore_and_schema_changes_do_not_report_success() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY); CREATE TRIGGER reject BEFORE INSERT ON t BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        let plan = plan_changes(&conn, &[insert("t", vec![])]).unwrap();
        assert!(apply_plan(&mut conn, &plan).is_err());
        conn.execute_batch("DROP TRIGGER reject; ALTER TABLE t ADD COLUMN name TEXT;")
            .unwrap();
        assert!(apply_plan(&mut conn, &plan).is_err());
        assert_eq!(
            conn.query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[cfg(test)]
mod full_export_tests {
    #[test]
    fn browser_table_download_spans_pages_and_preserves_filter_sort() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = SqliteDriver::open(None, cancel.clone()).unwrap();
        driver.connection.execute_batch("CREATE TABLE download_probe(id INTEGER PRIMARY KEY, label TEXT); WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1200) INSERT INTO download_probe SELECT x, 'matching' FROM n;").unwrap();
        let browse = Browse { table: "download_probe".into(), offset: 500, filter_column: Some("label".into()), filter: "matching".into(), sort: Some(("id".into(), true)) };
        let bytes = driver.export_table_bytes(&browse, &cancel, ExportFormat::Json).unwrap();
        let rows: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1200);
        assert_eq!(rows[0]["id"], 1200);
        assert_eq!(rows[1199]["id"], 1);
        let mut missing = browse.clone(); missing.filter = "absent".into();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&driver.export_table_bytes(&missing, &cancel, ExportFormat::Json).unwrap()).unwrap(), serde_json::json!([]));
        cancel.store(true, Ordering::Relaxed);
        assert!(driver.export_table_bytes(&browse, &cancel, ExportFormat::Csv).is_err());
        cancel.store(false, Ordering::Relaxed);
        driver.connection.execute_batch("BEGIN").unwrap();
        assert!(driver.export_table_bytes(&browse, &cancel, ExportFormat::Csv).is_err());
        driver.connection.execute_batch("ROLLBACK").unwrap();
        driver.connection.execute_batch("UPDATE download_probe SET label=printf('%20000d',id)").unwrap();
        let large = Browse { table: "download_probe".into(), ..Default::default() };
        assert!(driver.export_table_bytes(&large, &cancel, ExportFormat::Csv).unwrap_err().contains("16 MiB"));
        assert!(driver.connection.is_autocommit());
        assert_eq!(driver.connection.query_row("SELECT count(*) FROM download_probe", [], |r| r.get::<_, i64>(0)).unwrap(), 1200);
    }
    #[test]
    #[ignore = "manual release export throughput and process RSS measurement"]
    fn export_streaming_benchmark() {
        use std::io::BufRead;
        let count: usize = std::env::var("RUSTABLE_EXPORT_BENCH_ROWS")
            .unwrap_or_else(|_| "1000000".into())
            .parse()
            .unwrap();
        assert!((1..=1_000_000).contains(&count));
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("source.sqlite");
        {
            let connection = Connection::open(&database).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE records(id INTEGER PRIMARY KEY, label TEXT, score INTEGER)",
                )
                .unwrap();
            connection.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<?) INSERT INTO records SELECT x, printf('%096d', x), x%1000 FROM n", [count as i64]).unwrap();
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = SqliteDriver::open(Some(&database), cancel.clone()).unwrap();
        let format = match std::env::var("RUSTABLE_EXPORT_BENCH_FORMAT").as_deref() {
            Ok("json") => ExportFormat::Json,
            Ok("csv") | Err(_) => ExportFormat::Csv,
            _ => panic!("Expected csv or json benchmark format"),
        };
        let destination = directory.path().join("export.data");
        let browse = Browse {
            table: "records".into(),
            ..Default::default()
        };
        let mut durations = Vec::new();
        for _ in 0..3 {
            let start = Instant::now();
            driver.export_table(&destination, &browse, &cancel, format).unwrap();
            durations.push(start.elapsed());
        }
        // Validate every key without loading the exported file into memory.
        let mut reader = std::io::BufReader::new(std::fs::File::open(&destination).unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        match format {
            ExportFormat::Csv => assert_eq!(line, "\"id\",\"label\",\"score\"\n"),
            ExportFormat::Json => assert_eq!(line, "[\n"),
        }
        for id in 1..=count {
            line.clear();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            match format {
                ExportFormat::Csv => assert!(line.starts_with(&format!("\"{id}\","))),
                ExportFormat::Json => {
                    let row: serde_json::Value = serde_json::from_str(line.trim().trim_end_matches(',')).unwrap();
                    assert_eq!(row["id"], id);
                    assert_eq!(row["label"], format!("{id:096}"));
                    assert_eq!(row["score"], id % 1000);
                }
            }
        }
        if matches!(format, ExportFormat::Json) {
            line.clear();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            assert_eq!(line, "]\n");
        }
        line.clear();
        assert_eq!(reader.read_line(&mut line).unwrap(), 0);
        eprintln!(
            "export rows={count}, bytes={}, three elapsed samples={durations:?}",
            std::fs::metadata(&destination).unwrap().len()
        );
    }

    #[test]
    fn borrowed_export_preserves_large_values_and_rejects_invalid_utf8_atomically() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = SqliteDriver::open(None, cancel.clone()).unwrap();
        driver.connection.execute_batch("CREATE TABLE large_export(text_value TEXT, blob_value BLOB, number_value INTEGER, real_value REAL, null_value TEXT)").unwrap();
        let text = "日本,\r\n\"quoted\"".repeat(100_000);
        let blob: Vec<u8> = (0..2_000_000).map(|i| (i % 256) as u8).collect();
        driver
            .connection
            .execute(
                "INSERT INTO large_export VALUES (?, ?, ?, ?, NULL)",
                rusqlite::params![text, blob, i64::MIN, 1.25],
            )
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.csv");
        driver
            .export_table(
                &path,
                &Browse {
                    table: "large_export".into(),
                    ..Default::default()
                },
                &cancel, ExportFormat::Csv)
            .unwrap();
        let mut expected = Vec::new();
        csv::encode(
            &mut expected,
            &Grid {
                columns: [
                    "text_value",
                    "blob_value",
                    "number_value",
                    "real_value",
                    "null_value",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect(),
                rows: vec![vec![
                    Value::Text(text),
                    Value::Blob(blob),
                    Value::Integer(i64::MIN),
                    Value::Real(1.25),
                    Value::Null,
                ]],
                ..Default::default()
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), expected);
        driver
            .connection
            .execute_batch("INSERT INTO large_export(text_value) VALUES (CAST(x'ff' AS TEXT))")
            .unwrap();
        assert!(
            driver
                .export_table(
                    &path,
                    &Browse {
                        table: "large_export".into(),
                        ..Default::default()
                    },
                    &cancel, ExportFormat::Csv)
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), expected);
        assert_eq!(directory.path().read_dir().unwrap().count(), 1);
    }

    use super::*;
    #[test]
    fn exports_beyond_preview_limits_with_filter_sort_and_no_page_offset() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = SqliteDriver::open(None, cancel.clone()).unwrap();
        driver.connection.execute_batch("CREATE TABLE export_test(id INTEGER PRIMARY KEY, label TEXT); WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<6001) INSERT INTO export_test SELECT x, CASE WHEN x%2=0 THEN 'even' ELSE 'odd' END FROM n").unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("all.csv");
        let mut browse = Browse {
            table: "export_test".into(),
            offset: 500,
            ..Default::default()
        };
        driver.export_table(&path, &browse, &cancel, ExportFormat::Csv).unwrap();
        let csv = std::fs::read_to_string(&path).unwrap();
        assert_eq!(csv.lines().count(), 6002);
        assert!(csv.starts_with("\"id\",\"label\"\n\"1\",\"odd\"\n"));
        assert!(csv.ends_with("\"6001\",\"odd\"\n"));
        browse.filter_column = Some("label".into());
        browse.filter = "EVEN".into();
        browse.sort = Some(("id".into(), true));
        driver.export_table(&path, &browse, &cancel, ExportFormat::Csv).unwrap();
        let csv = std::fs::read_to_string(&path).unwrap();
        assert_eq!(csv.lines().count(), 3001);
        assert!(csv.lines().nth(1).unwrap().starts_with("\"6000\""));
        assert!(csv.ends_with("\"2\",\"even\"\n"));
        cancel.store(true, Ordering::Relaxed);
        assert!(driver.export_table(&path, &browse, &cancel, ExportFormat::Csv).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), csv);
        cancel.store(false, Ordering::Relaxed);
        browse.table = "missing_table".into();
        assert!(driver.export_table(&path, &browse, &cancel, ExportFormat::Csv).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), csv);
        assert_eq!(directory.path().read_dir().unwrap().count(), 1);
        assert!(driver.connection.is_autocommit());
    }
}

#[cfg(test)]
mod csv_import_tests {
    use super::*;
    fn request(driver: &SqliteDriver, text: &str, cancel: &AtomicBool) -> csv_import::Request {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), text).unwrap();
        let preview =
            csv_preview::load(file.path(), csv_preview::Options::default(), None, cancel).unwrap();
        csv_import::Request {
            source: preview.source.unwrap(),
            options: preview.options,
            table: "import_target".into(),
            columns: columns(&driver.connection, "import_target").unwrap(),
            destinations: vec![Some(0), Some(1)],
            empty_as_null: false,
            postgres_inserts: false,
        }
    }
    fn setup() -> (SqliteDriver, Arc<AtomicBool>) {
        let cancel = Arc::new(AtomicBool::new(false));
        let driver = SqliteDriver::open(None, cancel.clone()).unwrap();
        driver.connection.execute_batch("CREATE TABLE import_target(id INTEGER PRIMARY KEY, value TEXT CHECK(value != 'bad'))").unwrap();
        (driver, cancel)
    }
    fn count(driver: &SqliteDriver) -> i64 {
        driver
            .connection
            .query_row("SELECT COUNT(*) FROM import_target", [], |r| r.get(0))
            .unwrap()
    }
    #[test]
    fn streams_all_rows_and_preserves_explicit_empty_policy() {
        let (mut driver, cancel) = setup();
        let mut data = String::from("id,value\n");
        for n in 1..=6001 {
            data.push_str(&format!("{n},\"日本, 'quoted'\"\n"));
        }
        let request = request(&driver, &data, &cancel);
        assert_eq!(driver.import_csv(&request, &cancel).unwrap(), 6001);
        assert_eq!(count(&driver), 6001);
        let value: String = driver
            .connection
            .query_row("SELECT value FROM import_target WHERE id=6001", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(value, "日本, 'quoted'");
        let mut empty = self::request(&driver, "id,value\n6002,\n", &cancel);
        driver.import_csv(&empty, &cancel).unwrap();
        assert!(
            driver
                .connection
                .query_row(
                    "SELECT value = '' FROM import_target WHERE id=6002",
                    [],
                    |r| r.get::<_, bool>(0)
                )
                .unwrap()
        );
        empty = self::request(&driver, "id,value\n6003,\n", &cancel);
        empty.empty_as_null = true;
        driver.import_csv(&empty, &cancel).unwrap();
        assert!(
            driver
                .connection
                .query_row(
                    "SELECT value IS NULL FROM import_target WHERE id=6003",
                    [],
                    |r| r.get::<_, bool>(0)
                )
                .unwrap()
        );
    }
    #[test]
    fn late_failure_cancellation_and_schema_change_leave_no_partial_import() {
        let (mut driver, cancel) = setup();
        let mut text = String::from("id,value\n");
        for n in 1..=200 {
            text.push_str(&format!("{n},good\n"));
        }
        text.push_str("201,bad\n");
        let bad = request(&driver, &text, &cancel);
        assert!(
            driver
                .import_csv(&bad, &cancel)
                .unwrap_err()
                .contains("row 201")
        );
        assert_eq!(count(&driver), 0);
        let malformed = request(&driver, &text.replace("201,bad", "201,good,extra"), &cancel);
        assert!(driver.import_csv(&malformed, &cancel).is_err());
        assert_eq!(count(&driver), 0);
        let good = request(&driver, &text.replace("201,bad", "201,good"), &cancel);
        let stop = cancel.clone();
        driver.connection.progress_handler(
            100,
            Some(move || {
                stop.store(true, Ordering::Relaxed);
                true
            }),
        ).unwrap();
        assert!(driver.import_csv(&good, &cancel).is_err());
        driver.connection.progress_handler(0, None::<fn() -> bool>).unwrap();
        assert!(driver.connection.is_autocommit());
        assert_eq!(count(&driver), 0);
        cancel.store(false, Ordering::Relaxed);
        driver
            .connection
            .execute_batch("ALTER TABLE import_target ADD COLUMN extra TEXT")
            .unwrap();
        assert!(
            driver
                .import_csv(&good, &cancel)
                .unwrap_err()
                .contains("schema changed")
        );
        assert_eq!(count(&driver), 0);
    }
}

#[cfg(test)]
mod row_total_tests {
    use super::*;

    #[test]
    fn browse_uses_optional_analyze_stats_without_counting_filtered_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, value TEXT); WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1200) INSERT INTO t SELECT x, 'match' FROM n;").unwrap();
        let mut browse = Browse {
            table: "t".into(),
            ..Default::default()
        };
        assert_eq!(browse_table(&conn, &browse).unwrap().grid.total_rows, None);
        conn.execute_batch("ANALYZE;").unwrap();
        assert_eq!(
            browse_table(&conn, &browse).unwrap().grid.total_rows,
            Some(RowTotal {
                rows: 1200,
                estimated: true
            })
        );
        browse.filter_column = Some("value".into());
        browse.filter = "match".into();
        assert_eq!(browse_table(&conn, &browse).unwrap().grid.total_rows, None);
        browse.offset = 1000;
        assert_eq!(
            browse_table(&conn, &browse).unwrap().grid.total_rows,
            Some(RowTotal {
                rows: 1200,
                estimated: false
            })
        );
    }

    #[test]
    fn partial_index_statistics_do_not_count_the_whole_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(id INTEGER); INSERT INTO t VALUES (1),(2),(3); CREATE INDEX partial_t ON t(id) WHERE id=1; ANALYZE;").unwrap();
        assert_eq!(sqlite_row_estimate(&conn, "t"), Some(3));
    }
}

#[cfg(test)]
mod json_export_tests {
    use super::*;
    #[test]
    fn full_json_streams_all_rows_preserves_types_and_publishes_atomically() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = SqliteDriver::open(None, cancel.clone()).unwrap();
        driver.connection.execute_batch("CREATE TABLE json_export(id INTEGER PRIMARY KEY, label TEXT, bytes BLOB, score REAL, missing TEXT); WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<6001) INSERT INTO json_export SELECT x, CASE WHEN x%2=0 THEN 'even' ELSE 'odd' END, x'00ff', 1.25, NULL FROM n").unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("all.json");
        let mut browse = Browse { table: "json_export".into(), offset: 5000, ..Default::default() };
        driver.export_table(&path, &browse, &cancel, ExportFormat::Json).unwrap();
        let all: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(all.len(), 6001);
        for (index, row) in all.iter().enumerate() {
            let id = index + 1;
            assert_eq!(*row, serde_json::json!({"id": id, "label": if id%2==0 {"even"} else {"odd"}, "bytes": [0,255], "score": 1.25, "missing": null}));
        }
        browse.filter_column = Some("label".into());
        browse.filter = "EVEN".into();
        browse.sort = Some(("id".into(), true));
        driver.export_table(&path, &browse, &cancel, ExportFormat::Json).unwrap();
        let original = std::fs::read(&path).unwrap();
        let filtered: Vec<serde_json::Value> = serde_json::from_slice(&original).unwrap();
        assert_eq!(filtered.len(), 3000);
        for (index, row) in filtered.iter().enumerate() { assert_eq!(row["id"], (3000-index)*2); }
        driver.connection.execute_batch("UPDATE json_export SET score=1e999 WHERE id=100").unwrap();
        assert!(driver.export_table(&path, &browse, &cancel, ExportFormat::Json).unwrap_err().contains("non-finite"));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        browse.sort = Some(("missing_column".into(), false));
        assert!(driver.export_table(&path, &browse, &cancel, ExportFormat::Json).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        browse.sort = None;
        browse.filter = "no matches".into();
        driver.export_table(&path, &browse, &cancel, ExportFormat::Json).unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap(), serde_json::json!([]));
        assert_eq!(directory.path().read_dir().unwrap().count(), 1);
    }
}
