//! MySQL/MariaDB driver using a persistent native connection.
use super::*;
use ::mysql::{Conn, OptsBuilder, prelude::Queryable};
use crate::{connection::MysqlConfig, sql::mysql::Mode};

mod editing;
mod export;
#[cfg(test)]
mod transport_tests;

type CatalogRow = (String, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>);

struct MysqlDriver {
    connection: Option<Conn>,
    cancel: Arc<AtomicBool>,
    config: MysqlConfig,
    engine: &'static str,
    schemas: SchemaCatalog,
    mode: Mode,
    mode_tracking: bool,
    session_dirty: bool,
}

impl TryFrom<&MysqlConfig> for Box<dyn Driver> {
    type Error = String;
    fn try_from(config: &MysqlConfig) -> Result<Self, String> {
        MysqlDriver::connect(config).map(|driver| Box::new(driver) as Box<dyn Driver>)
    }
}

impl MysqlDriver {
    fn connect(config: &MysqlConfig) -> Result<Self, String> {
        crate::connection_profiles::MysqlProfile::from(config).validate()?;
        let mut connection = Conn::new(OptsBuilder::from(config)).map_err(|e| e.to_string())?;
        connection.query_drop("SET SESSION session_track_system_variables = 'sql_mode'").map_err(|e| e.to_string())?;
        let (version, database, sql_mode): (String, Option<String>, String) = connection.query_first(
            "SELECT VERSION(), DATABASE(), @@SESSION.sql_mode").map_err(|e| e.to_string())?
            .ok_or("Server did not return session metadata")?;
        let names = connection.query("SHOW DATABASES").map_err(|e| e.to_string())?;
        Ok(Self {
            connection: Some(connection),
            cancel: Arc::new(AtomicBool::new(false)),
            config: config.clone(),
            engine: if version.contains("MariaDB") { "MariaDB" } else { "MySQL" },
            schemas: SchemaCatalog { names, selected: database },
            mode: parse_mode(&sql_mode),
            mode_tracking: true,
            session_dirty: false,
        })
    }
}

fn parse_mode(value: &str) -> Mode {
    let modes: Vec<_> = value.split(',').map(str::trim).collect();
    Mode {
        ansi_quotes: modes.iter().any(|s| s.eq_ignore_ascii_case("ANSI_QUOTES")),
        no_backslash_escapes: modes.iter().any(|s| s.eq_ignore_ascii_case("NO_BACKSLASH_ESCAPES")),
    }
}
// Session-mutating statements need authoritative refresh even if the user has
// disabled protocol tracking. Ordinary SELECT/DML keeps the single-query path.
fn may_change_session(mut sql: &str) -> bool {
    loop {
        sql = sql.trim_start();
        if sql.starts_with("/*!") || sql.starts_with("/*M!") { return true; }
        if sql.starts_with("/*") && let Some(end) = sql.find("*/") { sql = &sql[end + 2..]; continue; }
        if (sql.starts_with('#') || (sql.starts_with("--") && sql.as_bytes().get(2).is_some_and(u8::is_ascii_whitespace)))
            && let Some(end) = sql.find('\n') { sql = &sql[end + 1..]; continue; }
        break;
    }
    matches!(sql.split(|c: char| !c.is_ascii_alphabetic()).next().unwrap_or("").to_ascii_uppercase().as_str(),
        "SET" | "CALL" | "EXECUTE" | "USE")
}

fn quote(name: &str) -> String { format!("`{}`", name.replace('`', "``")) }

/// The scoped watcher cannot outlive a query and cancel the next operation.
/// A control connection is opened only after the user requests cancellation.
fn with_cancel<T>(config: &MysqlConfig, id: u32, cancel: &AtomicBool, run: impl FnOnce() -> T) -> (T, Option<String>) {
    std::thread::scope(|scope| {
        let (done, completion) = std::sync::mpsc::channel::<()>();
        let watch = scope.spawn(move || -> Result<(), String> {
            let mut control: Option<Conn> = None;
            loop {
                match completion.recv_timeout(Duration::from_millis(if control.is_some() { 50 } else { 10 })) {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                }
                if !cancel.load(Ordering::Acquire) { continue; }
                if control.is_none() {
                    let mut options = config.clone();
                    options.connect_timeout = Duration::from_secs(1);
                    options.read_timeout = Duration::from_secs(1);
                    control = Some(Conn::new(OptsBuilder::from(&options)
                        .write_timeout(Some(Duration::from_secs(1))))
                        .map_err(|e| format!("Could not connect to cancel query: {e}"))?);
                }
                control.as_mut().unwrap().query_drop(format!("KILL QUERY {id}"))
                    .map_err(|e| format!("Could not cancel query: {e}"))?;
                // Repeat until completion to cover cancellation racing query arrival.
            }
        });
        let result = run();
        drop(done);
        let error = match watch.join() {
            Ok(result) => result.err(),
            Err(_) => Some("Query cancellation worker failed".into()),
        };
        (result, error)
    })
}

impl MysqlDriver {
    fn catalog_objects(&mut self, table: Option<&str>) -> Result<Vec<Object>, String> {
        let Some(schema) = self.schemas.selected.clone() else { return Ok(Vec::new()); };
        let mut result = Vec::<Object>::new();
        let mut sql = "SELECT t.TABLE_NAME,t.TABLE_TYPE,c.COLUMN_NAME,c.COLUMN_TYPE,c.IS_NULLABLE,c.COLUMN_DEFAULT,c.COLUMN_KEY,c.EXTRA FROM information_schema.TABLES t LEFT JOIN information_schema.COLUMNS c ON c.TABLE_SCHEMA=t.TABLE_SCHEMA AND c.TABLE_NAME=t.TABLE_NAME WHERE t.TABLE_SCHEMA=?".to_owned();
        let mut parameters = vec![::mysql::Value::from(schema.as_str())];
        if let Some(table) = table {
            sql.push_str(" AND t.TABLE_NAME=?");
            parameters.push(::mysql::Value::from(table));
        }
        sql.push_str(" ORDER BY t.TABLE_NAME,c.ORDINAL_POSITION");
        self.with_connection(|conn| {
            let rows = conn.exec_iter(sql, parameters)?;
            for row in rows {
                let row = row?;
                let (name, kind, column, ty, nullable, default, key, extra): CatalogRow =
                    ::mysql::from_row_opt(row)?;
                if result.last().is_none_or(|object| object.name != name) {
                    result.push(Object { schema: Some(schema.clone()), display_name: None, metadata_error: None,
                        name, kind: if kind == "VIEW" { "view" } else { "table" }.into(), columns: vec![], ddl: String::new() });
                }
                if let Some(name) = column {
                    result.last_mut().unwrap().columns.push(Column { name, data_type: ty.unwrap_or_default(),
                        not_null: nullable.as_deref() == Some("NO"), default: default.unwrap_or_default(),
                        pk: key.as_deref() == Some("PRI"), generated: extra.is_some_and(|v| v.contains("VIRTUAL GENERATED") || v.contains("STORED GENERATED") || v.contains("PERSISTENT")) });
                }
            }
            Ok(result)
        })
    }
    fn refresh_session(&mut self) -> Result<(), String> {
        let result = self.conn()?.query_first::<(String, String, Option<String>), _>(
            "SELECT @@SESSION.sql_mode, @@SESSION.session_track_system_variables, DATABASE()");
        match result {
            Ok(Some((mode, tracking, schema))) => {
                self.mode = parse_mode(&mode);
                self.mode_tracking = tracking.split(',').any(|name| name.trim() == "*" || name.trim().eq_ignore_ascii_case("sql_mode"));
                self.schemas.selected = schema;
                self.session_dirty = false;
                Ok(())
            }
            Ok(None) => Err("Missing MySQL session state".into()),
            Err(error) => {
                if error.is_connectivity_error() { self.connection = None; }
                Err(error.to_string())
            }
        }
    }
    fn conn(&mut self) -> Result<&mut Conn, String> {
        self.connection.as_mut().ok_or_else(|| "MySQL connection is disconnected".into())
    }
    fn with_connection<T>(&mut self, operation: impl FnOnce(&mut Conn) -> Result<T, ::mysql::Error>) -> Result<T, String> {
        let (value, cancellation_error) = self.with_connection_status(operation)?;
        match cancellation_error { Some(error) => Err(error), None => Ok(value) }
    }
    fn with_connection_status<T>(&mut self, operation: impl FnOnce(&mut Conn) -> Result<T, ::mysql::Error>) -> Result<(T, Option<String>), String> {
        if self.cancel.load(Ordering::Acquire) { return Err("Cancelled before metadata request".into()); }
        let config = self.config.clone();
        let cancel = self.cancel.clone();
        let conn = self.conn()?;
        let (result, cancellation_error) = with_cancel(&config, conn.connection_id(), &cancel, || operation(conn));
        match result {
            Ok(value) => Ok((value, cancellation_error)),
            Err(error) => {
                if error.is_connectivity_error() { self.connection = None; }
                Err(match cancellation_error {
                    Some(cancel_error) => format!("{error}\n{cancel_error}"),
                    None => error.to_string(),
                })
            }
        }
    }
    fn metadata_query_sets(&mut self, sql: &str) -> Result<Vec<Vec<::mysql::Row>>, String> {
        self.with_connection(|conn| {
            let mut results = conn.query_iter(sql)?;
            let mut sets = Vec::new();
            while let Some(set) = results.iter() {
                sets.push(set.collect::<Result<Vec<::mysql::Row>, ::mysql::Error>>()?);
            }
            Ok(sets)
        })
    }
    fn table_metadata(&mut self, schema: &str, table: &str) -> Result<(Vec<Index>, Vec<Object>), String> {
        let qualified = format!("{}.{}", quote(schema), quote(table));
        // These independent reads share one COM_QUERY round trip. Keep the
        // existing session (including temporary tables and transaction state),
        // and consume every result so a later server error cannot be hidden.
        let mut sets = self.metadata_query_sets(&format!("SHOW CREATE TABLE {qualified}; SHOW INDEX FROM {qualified}"))?;
        if sets.len() != 2 { return Err("Missing table metadata results".into()); }
        let rows = sets.pop().unwrap();
        let ddl = sets.pop().unwrap().into_iter().next().and_then(|row| row.get::<String, _>(1)).ok_or("Missing table definition")?;
        let mut indexes = std::collections::BTreeMap::<String, Index>::new();
        // SHOW INDEX is ordered by index then sequence by the server. Sort
        // explicitly so multipart keys remain ordered across server versions.
        let mut entries = Vec::new();
        for row in rows {
            let name = row.get::<String, _>("Key_name").ok_or("Missing index name")?;
            let position = row.get::<u64, _>("Seq_in_index").ok_or("Missing index position")?;
            let non_unique = row.get::<u64, _>("Non_unique").ok_or("Missing index uniqueness")?;
            let column = row.get::<Option<String>, _>("Column_name").flatten()
                .or_else(|| row.get::<Option<String>, _>("Expression").flatten())
                .unwrap_or_else(|| "(expression)".into());
            entries.push((name, position, non_unique, column));
        }
        entries.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        for (name, _, non_unique, column) in entries {
            let index = indexes.entry(name.clone()).or_insert_with(|| Index {
                name, unique: non_unique == 0, origin: "table definition".into(), partial: false,
                columns: Vec::new(),
                // Use server-authored DDL, retaining prefix lengths, expressions,
                // ordering, visibility and engine-specific attributes exactly.
                ddl: ddl.clone(),
            });
            index.columns.push(column);
        }
        let names: Vec<String> = self.with_connection(|conn| conn.exec(
            "SELECT TRIGGER_NAME FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA=? AND EVENT_OBJECT_TABLE=? ORDER BY TRIGGER_NAME", (schema, table)))?;
        let mut triggers = Vec::new();
        for name in names {
            let definition = self.with_connection(|conn| {
                match conn.query_first::<::mysql::Row, _>(format!("SHOW CREATE TRIGGER {}.{}", quote(schema), quote(&name))) {
                    // MariaDB can expose a trigger name while refusing its DDL.
                    // Preserve that distinction without hiding cancellation or
                    // transport failures, which still abort the browse request.
                    Err(::mysql::Error::MySqlError(error)) if matches!(error.code, 1142 | 1227) => Ok(Err(error.to_string())),
                    result => result.map(|row| row.and_then(|row| row.get::<String, _>("SQL Original Statement"))
                        .ok_or_else(|| "Trigger definition is unavailable to this account.".to_owned())),
                }
            })?;
            let (ddl, metadata_error) = match definition {
                Ok(ddl) => (ddl, None),
                Err(error) => (String::new(), Some(error)),
            };
            triggers.push(Object { schema: Some(schema.into()), name, display_name: None,
                metadata_error, columns: Vec::new(), kind: "trigger".into(), ddl });
        }
        Ok((indexes.into_values().collect(), triggers))
    }
    fn edit_schema_snapshot(&mut self, schema: &str, table: &str) -> Result<String, String> {
        let row: Option<::mysql::Row> = self.with_connection(|conn| conn.query_first(
            format!("SHOW CREATE TABLE {}.{}", quote(schema), quote(table))))?;
        let definition: String = row.and_then(|row| row.get(1)).ok_or("Missing table definition")?;
        let definition = editing::schema_signature(&definition, self.mode)?;
        // Trigger execution depends on its creation context, not just its body.
        // Capture order, definer, SQL mode and character settings as well.
        type Trigger = (String, String, String, String, u64, String, String, String, String, String);
        let triggers: Vec<Trigger> = self.with_connection(|conn| conn.exec(
            "SELECT TRIGGER_NAME,ACTION_TIMING,EVENT_MANIPULATION,ACTION_STATEMENT,ACTION_ORDER,SQL_MODE,DEFINER,CHARACTER_SET_CLIENT,COLLATION_CONNECTION,DATABASE_COLLATION FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA=? AND EVENT_OBJECT_TABLE=? ORDER BY TRIGGER_NAME",
            (schema, table)))?;
        // Structured encoding avoids delimiter collisions in arbitrary SQL text.
        serde_json::to_string(&(definition, triggers)).map_err(|error| error.to_string())
    }
    fn table_select(&mut self, request: &Browse) -> Result<(Object, String, Vec<Value>), String> {
        let schema = self.schemas.selected.clone().ok_or("Choose a database first")?;
        let object = self.catalog_objects(Some(&request.table))?.into_iter().next().ok_or("Table no longer exists")?;
        // SELECT * omits invisible columns, including possible primary keys.
        // Project the catalog order explicitly so values and editing metadata
        // stay aligned, and full-table exports match the table browser.
        if object.columns.is_empty() { return Err("No readable columns found for this table.".into()); }
        let projection = object.columns.iter().map(|column| quote(&column.name)).collect::<Vec<_>>().join(", ");
        let mut sql = format!("SELECT {projection} FROM {}.{}", quote(&schema), quote(&object.name));
        let mut parameters = Vec::new();
        if !request.filter.is_empty() {
            let column = request.filter_column.as_ref().ok_or("Choose a filter column")?;
            if !object.columns.iter().any(|c| &c.name == column) { return Err("Unknown filter column".into()); }
            sql.push_str(&format!(" WHERE CAST({} AS CHAR) LIKE ? ESCAPE '!'", quote(column)));
            parameters.push(Value::Text(format!("%{}%", request.filter.replace('!', "!!").replace('%', "!%").replace('_', "!_"))));
        }
        if let Some((column, descending)) = &request.sort {
            if !object.columns.iter().any(|c| &c.name == column) { return Err("Unknown sort column".into()); }
            sql.push_str(&format!(" ORDER BY {} {}", quote(column), if *descending { "DESC" } else { "ASC" }));
        }
        Ok((object, sql, parameters))
    }
    fn require_idle(&mut self) -> Result<(), String> {
        match self.transaction_state() {
            TransactionState::Idle => Ok(()),
            TransactionState::Disconnected => Err("Database connection is disconnected. Reconnect before continuing.".into()),
            _ => Err("Finish the SQL transaction before this operation.".into()),
        }
    }
}

impl Driver for MysqlDriver {
    fn set_cancellation(&mut self, cancel: Arc<AtomicBool>) { self.cancel = cancel; }
    fn engine_name(&self) -> &'static str { self.engine }
    fn sql_dialect(&self) -> crate::sql::Dialect { crate::sql::Dialect::Mysql(self.mode) }
    fn transaction_state(&mut self) -> TransactionState {
        let Some(conn) = self.connection.as_mut() else { return TransactionState::Disconnected; };
        if conn.transaction_status().is_none() && conn.ping().is_err() {
            self.connection = None;
            return TransactionState::Disconnected;
        }
        match conn.transaction_status() {
            Some(false) => TransactionState::Idle,
            Some(true) => TransactionState::Active,
            None => TransactionState::Failed,
        }
    }
    fn databases(&mut self) -> Result<Vec<String>, String> {
        self.with_connection(|conn| conn.query("SHOW DATABASES"))
    }
    fn schema_catalog(&self) -> SchemaCatalog { self.schemas.clone() }
    fn select_schema(&mut self, name: &str) -> Result<Vec<Object>, String> {
        self.require_idle()?;
        // Quote the whole identifier; a database name is never a SQL fragment.
        self.with_connection(|conn| conn.query_drop(format!("USE {}", quote(name))))?;
        self.schemas.selected = Some(name.into());
        self.objects()
    }
    fn objects(&mut self) -> Result<Vec<Object>, String> {
        self.catalog_objects(None)
    }

    fn execute(&mut self, sql: &str, safe: bool, cancel: &AtomicBool) -> ScriptResult {
        let started = std::time::Instant::now();
        let mut report = ScriptResult::default();
        if safe { report.error = Some("MySQL read-only execution is not yet available.".into()); return report; }
        let mut remaining = BYTE_LIMIT;
        let mut session_refresh_failed = false;
        let mut offset = 0;
        let mut delimiter = ";".to_owned();
        while offset < sql.len() {
            if cancel.load(Ordering::Relaxed) { report.error = Some("Cancelled before next statement".into()); break; }
            if (self.session_dirty || !self.mode_tracking) && let Err(error) = self.refresh_session() {
                session_refresh_failed = true;
                report.error = Some(error); break;
            }
            let (range, consumed, next_delimiter) = match crate::sql::mysql::next_statement(&sql[offset..], self.mode, &delimiter) {
                Ok(next) => next,
                Err(error) => { report.error = Some(error); break; }
            };
            delimiter = next_delimiter;
            let Some(range) = range else { break; };
            let range = offset + range.start..offset + range.end;
            offset += consumed;
            if report.grids.len() >= 128 {
                report.error = Some("Stopped after 128 result sets; later statements were not executed.".into());
                break;
            }
            if cancel.load(Ordering::Relaxed) { report.error = Some("Cancelled before next statement".into()); break; }
            self.session_dirty = may_change_session(&sql[range.clone()]);
            let Some(conn) = self.connection.as_mut() else { report.error = Some("MySQL connection is disconnected".into()); break; };
            let (outcome, cancellation_error) = with_cancel(&self.config, conn.connection_id(), cancel, || {
                conn.query_iter(&sql[range]).map(|results|
                    super::mysql_results::collect_budget(results, &mut remaining, QUERY_LIMIT, 128 - report.grids.len()))
            });
            if let Some(error) = cancellation_error { report.error = Some(error); }
            let (result, disconnected) = match outcome {
                Ok(result) => result,
                Err(error) => {
                    let disconnected = error.is_connectivity_error();
                    report.error = Some(match report.error.take() {
                        Some(cancel_error) => format!("{error}\n{cancel_error}"),
                        None => error.to_string(),
                    });
                    if disconnected { self.connection = None; }
                    break;
                }
            };
            // Session tracking reports sql_mode changes in the existing OK packet,
            // avoiding a metadata round trip after ordinary editor queries.
            if !disconnected {
                let changes = self.connection.as_ref().unwrap().session_state_changes();
                match changes {
                    Ok(changes) => {
                        for change in changes {
                            match change.decode() {
                                Ok(::mysql::session_state_change::SessionStateChange::SystemVariables(variables)) => {
                                    for variable in variables {
                                        if variable.name_str().eq_ignore_ascii_case("sql_mode") { self.mode = parse_mode(&variable.value_str()); }
                                    }
                                }
                                Ok(::mysql::session_state_change::SessionStateChange::Schema(schema)) => {
                                    self.schemas.selected = Some(schema.as_str().into_owned());
                                }
                                Ok(_) => {}
                                Err(error) => { report.error = Some(format!("Invalid MySQL session-state update: {error}")); }
                            }
                        }
                    }
                    Err(error) => { report.error = Some(format!("Invalid MySQL session-state update: {error}")); }
                }
            }
            report.grids.extend(result.grids);
            if result.error.is_some() || report.error.is_some() {
                if let Some(error) = result.error {
                    report.error = Some(match report.error.take() {
                        Some(previous) => format!("{error}\n{previous}"),
                        None => error,
                    });
                }
                if disconnected { self.connection = None; }
                break;
            }
            if self.session_dirty && let Err(error) = self.refresh_session() {
                session_refresh_failed = true;
                report.error = Some(error); break;
            }
        }
        // Failed SET/CALL/EXECUTE can still alter session settings. Publish an
        // authoritative dialect/catalog with the reply, not only on the next run.
        if self.session_dirty && !session_refresh_failed && self.connection.is_some()
            && let Err(error) = self.refresh_session() {
                let message = format!("Could not refresh MySQL session state: {error}");
                report.error = Some(match report.error.take() {
                    Some(original) => format!("{original}\n{message}"),
                    None => message,
                });
            }
        report.elapsed = started.elapsed();
        report
    }
    fn browse(&mut self, request: &Browse) -> Result<TableData, String> {
        let schema = self.schemas.selected.clone().ok_or("Choose a database first")?;
        let (object, mut sql, parameters) = self.table_select(request)?;
        sql.push_str(&format!(" LIMIT {} OFFSET {}", PAGE_SIZE + 1, request.offset));
        let params = parameters.iter().map(::mysql::Value::try_from).collect::<Result<Vec<_>, _>>()?;
        let mut remaining = BYTE_LIMIT;
        let ((report, disconnected), cancellation_error) = self.with_connection_status(|conn| conn.exec_iter(&sql, params).map(|results|
            super::mysql_results::collect_budget(results, &mut remaining, PAGE_SIZE + 1, 1)))?;
        if disconnected { self.connection = None; }
        if let Some(error) = report.error {
            return Err(match cancellation_error { Some(cancel_error) => format!("{error}\n{cancel_error}"), None => error });
        }
        if let Some(error) = cancellation_error { return Err(error); }
        let mut grid = Arc::unwrap_or_clone(report.grids.into_iter().next().ok_or("Missing table result")?);
        if grid.rows.len() > PAGE_SIZE { grid.rows.truncate(PAGE_SIZE); grid.truncated = true; }
        grid.set_browse_total(request.offset, None);
        let (indexes, triggers) = if object.kind == "view" { (Vec::new(), Vec::new()) }
            else { self.table_metadata(&schema, &object.name)? };
        Ok(TableData { sql, parameters, grid, columns: object.columns, indexes, triggers })
    }
    fn plan(&mut self, changes: &[Change]) -> Result<ChangePlan, String> {
        self.require_idle()?;
        let schema = self.schemas.selected.clone().ok_or("Choose a database first")?;
        let mut objects = Vec::new();
        let mut snapshots = Vec::new();
        for change in changes {
            if objects.iter().any(|object: &Object| object.name == change.table()) { continue; }
            let object = self.catalog_objects(Some(change.table()))?.into_iter().next().ok_or("Table no longer exists")?;
            let engine: Option<String> = self.with_connection(|conn| conn.exec_first(
                "SELECT ENGINE FROM information_schema.TABLES WHERE TABLE_SCHEMA=? AND TABLE_NAME=?", (&schema, change.table())))?;
            if engine.as_deref() != Some("InnoDB") { return Err("Reviewed data changes currently require an InnoDB base table.".into()); }
            snapshots.push((format!("{}.{}", quote(&schema), quote(change.table())),
                self.edit_schema_snapshot(&schema, change.table())?));
            objects.push(object);
        }
        let mut plan = editing::plan(changes, &objects)?;
        plan.schemas = snapshots;
        Ok(plan)
    }
    fn export_table(&mut self, path: &std::path::Path, browse: &Browse, cancel: &AtomicBool, format: ExportFormat) -> Result<(), String> {
        export::table(self, path, browse, cancel, format)
    }
    fn apply(&mut self, plan: &ChangePlan) -> Result<(), String> {
        editing::apply(self, plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bincode::Options;
    #[test]
    #[ignore = "requires disposable TLS fixture and RUSTABLE_MYSQL_TEST_CA"]
    fn live_driver_over_verified_tls() {
        let port = std::env::var("RUSTABLE_MYSQL_TLS_TEST_PORT").unwrap().parse().unwrap();
        assert!([43318, 43319].contains(&port));
        let config = MysqlConfig { host: "localhost".into(), port, user: "rustable".into(),
            password: "rustable-test-only".into(), database: "rustable_test".into(),
            root_certificate: Some(std::env::var_os("RUSTABLE_MYSQL_TEST_CA").unwrap().into()), ..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        let id = driver.conn().unwrap().connection_id();
        let cancel = AtomicBool::new(false);
        let result = driver.execute("SELECT '日本 🦀';", false, &cancel);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.grids[0].rows[0][0], Value::Text("日本 🦀".into()));
        assert_eq!(driver.transaction_state(), TransactionState::Idle);
        assert!(driver.objects().is_ok());
        assert_eq!(driver.conn().unwrap().connection_id(), id);
        let (_, cipher): (String, String) = driver.conn().unwrap().query_first("SHOW SESSION STATUS LIKE 'Ssl_cipher'").unwrap().unwrap();
        assert!(!cipher.is_empty());
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_driver_catalog_query_and_browse() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host: "127.0.0.1".into(), port, database: "rustable_test".into(),
            user: "rustable".into(), password: "rustable-test-only".into(), tls: false, ..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        assert_eq!(driver.engine_name(), if port == 43317 { "MariaDB" } else { "MySQL" });
        assert!(driver.databases().unwrap().contains(&"rustable_test".into()));
        let cancel = AtomicBool::new(false);
        let setup = driver.execute("DROP TABLE IF EXISTS `driver``probe`; CREATE TABLE `driver``probe` (id BIGINT PRIMARY KEY, label VARCHAR(100), doubled BIGINT GENERATED ALWAYS AS (id*2) STORED); INSERT INTO `driver``probe` (id,label) VALUES (1,'100%'),(2,'100x'),(3,'日本');", false, &cancel);
        assert!(setup.error.is_none(), "{:?}", setup.error);
        let object = driver.objects().unwrap().into_iter().find(|o| o.name == "driver`probe").unwrap();
        let targeted = driver.catalog_objects(Some("driver`probe")).unwrap();
        assert_eq!(targeted.len(), 1);
        assert_eq!(targeted[0].name, object.name);
        assert!(driver.catalog_objects(Some("driver`probe' OR 1=1 --")).unwrap().is_empty());
        // Exercise catalog -> qualified completion -> executable SQL on both engines.
        for alias in ["`customer alias`", "`where`", "`tick``alias`", "`客戶`"] {
            let head = format!("SELECT {alias}.lab");
            let mut sql = format!("{head} FROM `driver``probe` AS {alias} WHERE {alias}.id=1");
            let mut completion = crate::completion::Completion::default();
            completion.refresh(&sql, head.chars().count(), std::slice::from_ref(&object), &[], true, driver.sql_dialect());
            completion.apply(&mut sql, "label");
            assert!(sql.starts_with(&format!("SELECT {alias}.`label`")), "{sql}");
            let completed = driver.execute(&sql, false, &cancel);
            assert!(completed.error.is_none(), "{sql}: {:?}", completed.error);
        }
        assert_eq!(object.columns.len(), 3);
        assert!(object.columns[0].pk);
        assert!(object.columns[2].generated);
        let metadata_setup = driver.execute("CREATE INDEX label_prefix ON `driver``probe` (label(8),id); CREATE TRIGGER `probe``trigger` BEFORE UPDATE ON `driver``probe` FOR EACH ROW SET NEW.label=UPPER(NEW.label);", false, &cancel);
        assert!(metadata_setup.error.is_none(), "{:?}", metadata_setup.error);
        let table = driver.browse(&Browse { table: "driver`probe".into(), filter_column: Some("label".into()), filter: "%".into(), ..Default::default() }).unwrap();
        assert_eq!(table.grid.rows.len(), 1);
        assert_eq!(table.grid.rows[0][0], Value::Integer(1));
        assert_eq!(table.grid.rows[0][2], Value::Integer(2));
        let index = table.indexes.iter().find(|index| index.name == "label_prefix").unwrap();
        assert_eq!(index.columns, ["label", "id"]);
        assert!(!index.unique);
        assert!(index.ddl.contains("label_prefix") && index.ddl.contains("(8)"));
        assert!(table.indexes.iter().find(|index| index.name == "PRIMARY").unwrap().unique);
        assert_eq!(table.triggers.len(), 1);
        assert_eq!(table.triggers[0].name, "probe`trigger");
        assert!(table.triggers[0].ddl.contains("UPPER(NEW.label)"));
        let sorted = driver.browse(&Browse { table: "driver`probe".into(), sort: Some(("id".into(), true)), offset: 1, ..Default::default() }).unwrap();
        assert_eq!(sorted.grid.rows[0][0], Value::Integer(2));
        assert!(driver.execute("START TRANSACTION; UPDATE `driver``probe` SET label='changed' WHERE id=1;", false, &cancel).error.is_none());
        assert_eq!(driver.transaction_state(), TransactionState::Active);
        assert!(driver.select_schema("rustable_test").is_err());
        assert!(driver.execute("SELECT * FROM missing_rustable_table_7423", false, &cancel).error.is_some());
        assert_eq!(driver.transaction_state(), TransactionState::Active);
        assert!(driver.execute("ROLLBACK", false, &cancel).error.is_none());
        assert_eq!(driver.transaction_state(), TransactionState::Idle);
        assert!(driver.select_schema("rustable_test").is_ok());
        assert!(driver.execute("DROP TABLE `driver``probe`", false, &cancel).error.is_none());
    }
    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_stream_error_keeps_transaction_but_timeout_disconnects() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host: "127.0.0.1".into(), port, database: "rustable_test".into(),
            user: "rustable".into(), password: "rustable-test-only".into(), tls: false,
            read_timeout: Duration::from_millis(100), ..Default::default() };
        let mut driver = Box::<dyn Driver>::try_from(&config).unwrap();
        let cancel = AtomicBool::new(false);
        let setup = "DROP PROCEDURE IF EXISTS rustable_error_probe;";
        assert!(driver.execute(setup, false, &cancel).error.is_none());
        let setup = "DELIMITER $$\nCREATE PROCEDURE rustable_error_probe() BEGIN SELECT 42 AS retained; SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='expected fixture error'; END$$";
        assert!(driver.execute(setup, false, &cancel).error.is_none());
        assert!(driver.execute("CREATE TEMPORARY TABLE error_tx_probe(value INT) ENGINE=InnoDB; INSERT INTO error_tx_probe VALUES(7); START TRANSACTION; UPDATE error_tx_probe SET value=8;", false, &cancel).error.is_none());
        let report = driver.execute("CALL rustable_error_probe()", false, &cancel);
        assert!(report.error.unwrap().contains("expected fixture error"));
        assert_eq!(report.grids[0].rows, [vec![Value::Integer(42)]]);
        assert_eq!(driver.transaction_state(), TransactionState::Active);
        let report = driver.execute("SELECT value FROM error_tx_probe", false, &cancel);
        assert_eq!(report.grids[0].rows, [vec![Value::Integer(8)]]);
        assert!(driver.execute("ROLLBACK; DROP PROCEDURE rustable_error_probe;", false, &cancel).error.is_none());
        assert_eq!(driver.transaction_state(), TransactionState::Idle);
        let report = driver.execute("SELECT SLEEP(1)", false, &cancel);
        assert!(report.error.is_some());
        assert_eq!(driver.transaction_state(), TransactionState::Disconnected);
        assert!(driver.execute("SELECT 1", false, &cancel).error.unwrap().contains("disconnected"));
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_driver_cancellation_preserves_transaction_and_has_no_late_kill() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host: "127.0.0.1".into(), port, database: "rustable_test".into(),
            user: "rustable".into(), password: "rustable-test-only".into(), tls: false,
            read_timeout: Duration::from_secs(3), ..Default::default() };
        check_driver_cancellation(&config);
    }

    #[test]
    #[ignore = "requires disposable TLS fixture and RUSTABLE_MYSQL_TEST_CA"]
    fn live_tls_cancellation_preserves_transaction_and_connection() {
        let port = std::env::var("RUSTABLE_MYSQL_TLS_TEST_PORT").unwrap().parse().unwrap();
        assert!([43318, 43319].contains(&port));
        let config = MysqlConfig { host: "localhost".into(), port, database: "rustable_test".into(),
            user: "rustable".into(), password: "rustable-test-only".into(),
            root_certificate: Some(std::env::var_os("RUSTABLE_MYSQL_TEST_CA").unwrap().into()),
            read_timeout: Duration::from_secs(3), ..Default::default() };
        check_driver_cancellation(&config);
    }

    #[test]
    #[ignore = "requires disposable TLS fixture and RUSTABLE_MYSQL_TEST_CA"]
    fn live_tls_cancel_rejects_untrusted_control_connection() {
        let port = std::env::var("RUSTABLE_MYSQL_TLS_TEST_PORT").unwrap().parse().unwrap();
        assert!([43318, 43319].contains(&port));
        let config = MysqlConfig { host: "localhost".into(), port, database: "rustable_test".into(),
            user: "rustable".into(), password: "rustable-test-only".into(),
            root_certificate: Some(std::env::var_os("RUSTABLE_MYSQL_TEST_CA").unwrap().into()),
            read_timeout: Duration::from_secs(1), ..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        // Fault injection: the established query connection remains trusted,
        // but a newly opened cancellation connection no longer has the CA.
        driver.config.root_certificate = None;
        let cancel = AtomicBool::new(false);
        let started = std::time::Instant::now();
        let report = std::thread::scope(|scope| {
            scope.spawn(|| { std::thread::sleep(Duration::from_millis(100)); cancel.store(true, Ordering::Release); });
            driver.execute("SELECT SLEEP(5)", false, &cancel)
        });
        let error = report.error.expect("cancellation failure and query timeout must be reported");
        assert!(error.contains("Could not connect to cancel query") && error.to_lowercase().contains("certificate"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(driver.transaction_state(), TransactionState::Disconnected);
        assert!(driver.connection.is_none());
    }

    fn check_driver_cancellation(config: &MysqlConfig) {
        let mut driver = MysqlDriver::connect(config).unwrap();
        let connection_id = driver.conn().unwrap().connection_id();
        let cancel = AtomicBool::new(false);
        assert!(driver.execute("CREATE TEMPORARY TABLE cancel_driver_probe(value INT) ENGINE=InnoDB; INSERT INTO cancel_driver_probe VALUES(7); START TRANSACTION; UPDATE cancel_driver_probe SET value=8;", false, &cancel).error.is_none());
        let started = std::time::Instant::now();
        let report = std::thread::scope(|scope| {
            scope.spawn(|| { std::thread::sleep(Duration::from_millis(100)); cancel.store(true, Ordering::Release); });
            driver.execute("UPDATE cancel_driver_probe SET value=IF(SLEEP(10)=0,99,100); UPDATE cancel_driver_probe SET value=200;", false, &cancel)
        });
        assert!(started.elapsed() < Duration::from_secs(2), "cancellation waited for read timeout");
        assert!(report.error.is_some());
        assert_eq!(driver.transaction_state(), TransactionState::Active);
        cancel.store(false, Ordering::Release);
        let result = driver.execute("SELECT value FROM cancel_driver_probe", false, &cancel);
        assert_eq!(result.grids[0].rows, [vec![Value::Integer(8)]]);
        // A delayed control request must not cancel a subsequent query.
        let result = driver.execute("SELECT SLEEP(0.2),42", false, &cancel);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(driver.execute("ROLLBACK", false, &cancel).error.is_none());
        let result = driver.execute("SELECT value FROM cancel_driver_probe", false, &cancel);
        assert_eq!(result.grids[0].rows, [vec![Value::Integer(7)]]);
        assert_eq!(driver.conn().unwrap().connection_id(), connection_id);
        if config.tls {
            let (_, cipher): (String, String) = driver.conn().unwrap().query_first("SHOW SESSION STATUS LIKE 'Ssl_cipher'").unwrap().unwrap();
            assert!(!cipher.is_empty());
        }
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_sql_mode_changes_apply_to_following_statements() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host: "127.0.0.1".into(), port, database: "rustable_test".into(),
            user: "rustable".into(), password: "rustable-test-only".into(), tls: false, ..Default::default() };
        let mut driver = Box::<dyn Driver>::try_from(&config).unwrap();
        let cancel = AtomicBool::new(false);
        let script = r#"SET SESSION sql_mode='NO_BACKSLASH_ESCAPES'; SELECT 'a\' AS value; SET SESSION sql_mode=''; SELECT 'a\'b' AS value; SET SESSION sql_mode='ANSI_QUOTES'; SELECT 1 AS "a\";"#;
        let result = driver.execute(script, false, &cancel);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.grids.len(), 6);
        assert_eq!(result.grids[1].rows, [vec![Value::Text("a\\".into())]]);
        assert_eq!(result.grids[3].rows, [vec![Value::Text("a'b".into())]]);
        assert_eq!(result.grids[5].columns, ["a\\"]);
        let result = driver.execute(r#"SELECT 2 AS "b\";"#, false, &cancel);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.grids[0].columns, ["b\\"]);
        let result = driver.execute(r#"SET session_track_system_variables=''; SET sql_mode='NO_BACKSLASH_ESCAPES'; SELECT 'c\'; SET sql_mode=''; SELECT 'c\'d';"#, false, &cancel);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.grids[2].rows, [vec![Value::Text("c\\".into())]]);
        assert_eq!(result.grids[4].rows, [vec![Value::Text("c'd".into())]]);
        let result = driver.execute("SET session_track_system_variables='sql_mode'; SET session_track_schema=OFF; USE information_schema;", false, &cancel);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(driver.schema_catalog().selected.as_deref(), Some("information_schema"));
        let result = driver.execute("-- leading comment\nUSE rustable_test;", false, &cancel);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(driver.schema_catalog().selected.as_deref(), Some("rustable_test"));
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn failed_routine_refreshes_tracking_before_reply() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host: "127.0.0.1".into(), port, database: "rustable_test".into(),
            user: "rustable".into(), password: "rustable-test-only".into(), tls: false, ..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        let cancel = AtomicBool::new(false);
        let setup = "DROP PROCEDURE IF EXISTS driver_tracking_failure;\nDELIMITER $$\nCREATE PROCEDURE driver_tracking_failure() BEGIN SET session_track_system_variables=''; SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='tracking changed before error'; END$$\nDELIMITER ;";
        let result = driver.execute(setup, false, &cancel);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(driver.mode_tracking);
        let result = driver.execute("CALL driver_tracking_failure(); SELECT 123;", false, &cancel);
        assert!(result.error.as_deref().unwrap().contains("tracking changed before error"));
        assert!(!driver.mode_tracking, "tracking state must refresh before returning the failed call");
        assert!(!driver.session_dirty);
        assert!(result.grids.iter().all(|grid| grid.rows.is_empty()), "later SQL must not run");
        let server_mode: String = driver.conn().unwrap().query_first("SELECT @@sql_mode").unwrap().unwrap();
        assert_eq!(driver.sql_dialect(), crate::sql::Dialect::Mysql(parse_mode(&server_mode)));
        let cleanup = driver.execute("DROP PROCEDURE driver_tracking_failure;", false, &cancel);
        assert!(cleanup.error.is_none(), "{:?}", cleanup.error);
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn batched_metadata_preserves_session_and_reports_later_errors() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host: "127.0.0.1".into(), port, database: "rustable_test".into(),
            user: "rustable".into(), password: "rustable-test-only".into(), tls: false, ..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        let id = driver.conn().unwrap().connection_id();
        driver.conn().unwrap().query_drop("CREATE TEMPORARY TABLE `batch``metadata` (id INT PRIMARY KEY, label VARCHAR(20), INDEX by_label(label)) ENGINE=InnoDB").unwrap();
        driver.conn().unwrap().query_drop("START TRANSACTION").unwrap();
        driver.conn().unwrap().query_drop("INSERT INTO `batch``metadata` VALUES (1,'uncommitted')").unwrap();
        let (indexes, triggers) = driver.table_metadata("rustable_test", "batch`metadata").unwrap();
        assert!(indexes.iter().any(|index| index.name == "by_label" && index.columns == ["label"]));
        assert!(triggers.is_empty());
        assert_eq!(driver.conn().unwrap().connection_id(), id);
        assert_eq!(driver.transaction_state(), TransactionState::Active);
        driver.conn().unwrap().query_drop("ROLLBACK").unwrap();
        assert_eq!(driver.conn().unwrap().query_first::<u64, _>("SELECT COUNT(*) FROM `batch``metadata`").unwrap(), Some(0));
        // A later statement error must propagate after a successful first set,
        // leaving the protocol synchronized for the next command.
        assert!(driver.metadata_query_sets("SELECT 1; SELECT * FROM missing_batch_metadata_probe_741").is_err());
        assert_eq!(driver.conn().unwrap().query_first::<u64, _>("SELECT 42").unwrap(), Some(42));
        assert_eq!(driver.conn().unwrap().connection_id(), id);
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn metadata_transport_failure_disconnects_but_sql_error_preserves_session() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host: "127.0.0.1".into(), port, database: "rustable_test".into(),
            user: "rustable".into(), password: "rustable-test-only".into(), tls: false, ..Default::default() };
        let mut control = Conn::new(OptsBuilder::from(&config)).unwrap();
        for operation in 0..3 {
            let mut driver = MysqlDriver::connect(&config).unwrap();
            assert!(driver.table_metadata("rustable_test", "missing_metadata_probe_741").is_err());
            assert!(driver.connection.is_some(), "ordinary SQL errors must not discard the session");
            assert_eq!(driver.transaction_state(), TransactionState::Idle);
            let id = driver.conn().unwrap().connection_id();
            control.query_drop(format!("KILL CONNECTION {id}")).unwrap();
            let failed = match operation {
                0 => driver.objects().map(|_| ()),
                1 => driver.databases().map(|_| ()),
                _ => driver.table_metadata("rustable_test", "customers").map(|_| ()),
            };
            assert!(failed.is_err());
            assert!(driver.connection.is_none(), "metadata failure must discard the socket immediately");
            assert_eq!(driver.transaction_state(), TransactionState::Disconnected);
        }
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_browse_can_cancel_a_locked_table() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host: "127.0.0.1".into(), port, database: "rustable_test".into(),
            user: "rustable".into(), password: "rustable-test-only".into(), tls: false,
            read_timeout: Duration::from_secs(3), ..Default::default() };
        let mut lock = Conn::new(OptsBuilder::from(&config)).unwrap();
        lock.query_drop("DROP TABLE IF EXISTS driver_locked_browse").unwrap();
        lock.query_drop("CREATE TABLE driver_locked_browse(id INT PRIMARY KEY)").unwrap();
        lock.query_drop("INSERT INTO driver_locked_browse VALUES (1)").unwrap();
        let mut driver = MysqlDriver::connect(&config).unwrap();
        lock.query_drop("LOCK TABLES driver_locked_browse WRITE").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        driver.set_cancellation(cancel.clone());
        let started = std::time::Instant::now();
        let result = std::thread::scope(|scope| {
            scope.spawn(|| { std::thread::sleep(Duration::from_millis(100)); cancel.store(true, Ordering::Release); });
            driver.browse(&Browse { table: "driver_locked_browse".into(), ..Default::default() })
        });
        lock.query_drop("UNLOCK TABLES").unwrap();
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(2), "cancellation should beat read timeout");
        assert!(driver.connection.is_some(), "query cancellation should preserve the session");
        cancel.store(false, Ordering::Release);
        let table = driver.browse(&Browse { table: "driver_locked_browse".into(), ..Default::default() }).unwrap();
        assert_eq!(table.grid.rows, [vec![Value::Integer(1)]]);
        lock.query_drop("DROP TABLE driver_locked_browse").unwrap();
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn failed_cancel_connection_reports_both_errors_and_discards_timed_out_session() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host: "127.0.0.1".into(), port, database: "rustable_test".into(),
            user: "rustable".into(), password: "rustable-test-only".into(), tls: false,
            read_timeout: Duration::from_millis(300), ..Default::default() };
        for metadata in [false, true] {
            let mut driver = MysqlDriver::connect(&config).unwrap();
            // Only the future control connection gets bad credentials.
            driver.config.password = "deliberately-invalid-fixture-password".into();
            let cancel = Arc::new(AtomicBool::new(false));
            driver.set_cancellation(cancel.clone());
            let began = std::time::Instant::now();
            let error = std::thread::scope(|scope| {
                scope.spawn(|| { std::thread::sleep(Duration::from_millis(50)); cancel.store(true, Ordering::Release); });
                if metadata {
                    driver.with_connection(|conn| conn.query_first::<u8, _>("SELECT SLEEP(2)")).unwrap_err()
                } else {
                    driver.execute("SELECT SLEEP(2); SELECT 123;", false, &cancel).error.unwrap()
                }
            });
            assert!(error.contains("Could not connect to cancel query"), "{error}");
            assert!(error.lines().count() >= 2, "retain primary and cancellation failure: {error}");
            assert!(began.elapsed() < Duration::from_secs(2));
            assert!(driver.connection.is_none());
            assert_eq!(driver.transaction_state(), TransactionState::Disconnected);
        }
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn worker_open_is_atomic_and_protects_existing_transactions() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host: "127.0.0.1".into(), port, database: "rustable_test".into(),
            user: "rustable".into(), password: "rustable-test-only".into(), tls: false, ..Default::default() };
        crate::db::probe_mysql(&config, &AtomicBool::new(false)).unwrap();
        assert!(crate::db::probe_mysql(&config, &AtomicBool::new(true)).is_err());
        let cancel = Arc::new(AtomicBool::new(false));
        let mut session = None;
        handle(&mut session, Command::Open(None), &cancel).unwrap();
        let mut bad = config.clone(); bad.password = "invalid-fixture-password".into();
        assert!(handle(&mut session, Command::OpenMysql(bad), &cancel).is_err());
        assert_eq!(session.as_ref().unwrap().engine_name(), "SQLite");
        cancel.store(true, Ordering::Release);
        assert!(handle(&mut session, Command::OpenMysql(config.clone()), &cancel).is_err());
        assert_eq!(session.as_ref().unwrap().engine_name(), "SQLite");
        cancel.store(false, Ordering::Release);
        handle(&mut session, Command::Query { sql: "BEGIN".into(), safe: false }, &cancel).unwrap();
        assert!(handle(&mut session, Command::OpenMysql(config.clone()), &cancel).is_err());
        handle(&mut session, Command::Query { sql: "ROLLBACK".into(), safe: false }, &cancel).unwrap();
        let reply = handle(&mut session, Command::OpenMysql(config.clone()), &cancel).unwrap();
        let Payload::Connected { engine, database, dialect, label, .. } = reply else { panic!("connected payload expected"); };
        assert_eq!(engine, if port == 43317 { "MariaDB" } else { "MySQL" });
        assert_eq!(database.as_deref(), Some("rustable_test"));
        assert!(matches!(dialect, crate::sql::Dialect::Mysql(_)));
        assert!(!label.contains(&config.password));
        let reply = handle(&mut session, Command::Query { sql: "SELECT 42".into(), safe: false }, &cancel).unwrap();
        let Payload::Query { report, .. } = reply else { panic!("query payload expected"); };
        assert_eq!(report.grids[0].rows, [vec![Value::Integer(42)]]);
        assert!(super::super::transport::codec().serialize(&Command::OpenMysql(config)).is_err(), "credentials must never cross browser transport");
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_data_plan_groups_updates_and_detects_case_only_conflicts() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host:"127.0.0.1".into(), port, user:"rustable".into(), password:"rustable-test-only".into(), database:"rustable_test".into(), tls:false, ..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        let cancel = AtomicBool::new(false);
        let setup = driver.execute("DROP TABLE IF EXISTS plan_data_probe; CREATE TABLE plan_data_probe (id INT PRIMARY KEY, name VARCHAR(80) COLLATE utf8mb4_general_ci, n INT, generated_value INT GENERATED ALWAYS AS (n*2) STORED) ENGINE=InnoDB; INSERT INTO plan_data_probe(id,name,n) VALUES (1,'Before',2);", false, &cancel);
        assert!(setup.error.is_none(), "{:?}", setup.error);
        let original = driver.browse(&Browse { table:"plan_data_probe".into(), ..Default::default() }).unwrap().grid.rows[0].clone();
        let edits = vec![Change::Cell { table:"plan_data_probe".into(), column:"name".into(), original:original.clone(), value:Value::Text("after'; DROP TABLE plan_data_probe; --".into()) },
            Change::Cell { table:"plan_data_probe".into(), column:"n".into(), original:original.clone(), value:Value::Integer(3) }];
        let plan = driver.plan(&edits).unwrap();
        assert_eq!(plan.statements.len(),1);
        assert_eq!(plan.preconditions.len(),1);
        assert_eq!(plan.statements[0].parameters.len(),3);
        assert!(!plan.statements[0].sql.contains("DROP TABLE"));
        assert_eq!(plan.schemas.len(),1);
        assert_eq!(driver.plan(&edits).unwrap().schemas, plan.schemas, "unchanged schema must have a stable snapshot");
        driver.conn().unwrap().query_drop("CREATE TRIGGER plan_trigger_probe BEFORE UPDATE ON plan_data_probe FOR EACH ROW SET NEW.n=NEW.n+1").unwrap();
        let with_trigger = driver.plan(&edits).unwrap().schemas;
        assert_ne!(with_trigger, plan.schemas, "adding a trigger must invalidate review");
        assert_eq!(driver.plan(&edits).unwrap().schemas, with_trigger);
        let original_mode: String = driver.conn().unwrap().query_first("SELECT @@sql_mode").unwrap().unwrap();
        driver.conn().unwrap().query_drop("DROP TRIGGER plan_trigger_probe").unwrap();
        driver.conn().unwrap().query_drop("SET sql_mode='ANSI_QUOTES'").unwrap();
        driver.conn().unwrap().query_drop("CREATE TRIGGER plan_trigger_probe BEFORE UPDATE ON plan_data_probe FOR EACH ROW SET NEW.n=NEW.n+1").unwrap();
        driver.conn().unwrap().exec_drop("SET sql_mode=?", (&original_mode,)).unwrap();
        assert_ne!(driver.plan(&edits).unwrap().schemas, with_trigger, "same trigger text with different creation SQL mode must invalidate review");
        driver.conn().unwrap().query_drop("DROP TRIGGER plan_trigger_probe").unwrap();
        driver.conn().unwrap().query_drop("CREATE TRIGGER plan_trigger_probe BEFORE UPDATE ON plan_data_probe FOR EACH ROW SET NEW.n=NEW.n+2").unwrap();
        assert_ne!(driver.plan(&edits).unwrap().schemas, with_trigger, "changing a trigger body must invalidate review");
        driver.conn().unwrap().query_drop("DROP TRIGGER plan_trigger_probe").unwrap();
        assert_eq!(driver.plan(&edits).unwrap().schemas, plan.schemas);
        let check = &plan.preconditions[0];
        let params = check.parameters.iter().map(::mysql::Value::try_from).collect::<Result<Vec<_>,_>>().unwrap();
        let matches: Vec<u8> = driver.conn().unwrap().exec(&check.sql, params.clone()).unwrap();
        assert_eq!(matches,[1]);
        driver.conn().unwrap().query_drop("UPDATE plan_data_probe SET name='before' WHERE id=1").unwrap();
        let matches: Vec<u8> = driver.conn().unwrap().exec(&check.sql, params).unwrap();
        assert!(matches.is_empty(), "case-insensitive collation must not conceal a conflict");
        assert!(driver.plan(&[Change::Cell { table:"plan_data_probe".into(), column:"generated_value".into(), original:original.clone(), value:Value::Integer(4) }]).is_err());
        let mut deleted = edits;
        deleted.push(Change::DeleteRow { table:"plan_data_probe".into(), original });
        let plan = driver.plan(&deleted).unwrap();
        assert_eq!(plan.statements.len(),1);
        assert!(plan.statements[0].sql.starts_with("DELETE"));
        let inserted = driver.plan(&[Change::InsertRow { id:1, table:"plan_data_probe".into(), fields:vec![("id".into(),Value::Integer(2))] }]).unwrap();
        assert!(!inserted.statements[0].sql.contains("generated_value"));
        driver.conn().unwrap().query_drop("DROP TABLE plan_data_probe").unwrap();
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_invisible_columns_align_browse_export_and_edit_preconditions() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host:"127.0.0.1".into(), port, user:"rustable".into(), password:"rustable-test-only".into(), database:"rustable_test".into(), tls:false, ..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        let mut other = Conn::new(OptsBuilder::from(&config)).unwrap();
        driver.conn().unwrap().query_drop("DROP TABLE IF EXISTS invisible_probe").unwrap();
        driver.conn().unwrap().query_drop("CREATE TABLE invisible_probe(id INT NOT NULL DEFAULT 0 PRIMARY KEY INVISIBLE, label VARCHAR(30), `hidden``value` INT INVISIBLE, qty INT) ENGINE=InnoDB").unwrap();
        driver.conn().unwrap().query_drop("INSERT INTO invisible_probe(id,label,`hidden``value`,qty) VALUES(7,'visible',42,3)").unwrap();
        let handwritten = driver.execute("SELECT * FROM invisible_probe", false, &AtomicBool::new(false));
        assert_eq!(handwritten.grids[0].rows[0].len(),2, "handwritten SQL keeps server star semantics");
        let request = Browse { table:"invisible_probe".into(), ..Default::default() };
        let data = driver.browse(&request).unwrap();
        assert_eq!(data.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id","label","hidden`value","qty"]);
        assert_eq!(data.grid.rows[0], [Value::Integer(7),Value::Text("visible".into()),Value::Integer(42),Value::Integer(3)]);
        assert!(data.columns[0].pk);
        let change = Change::Cell { table:"invisible_probe".into(), column:"label".into(), original:data.grid.rows[0].clone(), value:Value::Text("edited".into()) };
        let plan = driver.plan(&[change]).unwrap();
        driver.apply(&plan).unwrap();
        let data = driver.browse(&request).unwrap();
        let change = Change::Cell { table:"invisible_probe".into(), column:"label".into(), original:data.grid.rows[0].clone(), value:Value::Text("again".into()) };
        let stale = driver.plan(&[change]).unwrap();
        other.query_drop("UPDATE invisible_probe SET `hidden``value`=43 WHERE id=7").unwrap();
        assert!(driver.apply(&stale).unwrap_err().contains("Row changed"));
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("invisible.csv");
        driver.export_table(&path, &request, &AtomicBool::new(false), ExportFormat::Csv).unwrap();
        let mut csv = ::csv::Reader::from_path(&path).unwrap();
        assert_eq!(csv.headers().unwrap().iter().collect::<Vec<_>>(), ["id","label","hidden`value","qty"]);
        assert_eq!(csv.records().next().unwrap().unwrap().iter().collect::<Vec<_>>(), ["7","edited","43","3"]);
        other.query_drop("DROP TABLE invisible_probe").unwrap();
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_review_insert_defaults_binary_values_delete_and_failed_batch() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host:"127.0.0.1".into(), port, user:"rustable".into(), password:"rustable-test-only".into(), database:"rustable_test".into(), tls:false, ..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        let mut observer = Conn::new(OptsBuilder::from(&config)).unwrap();
        driver.conn().unwrap().query_drop("DROP TABLE IF EXISTS apply_crud_probe").unwrap();
        driver.conn().unwrap().query_drop("CREATE TABLE apply_crud_probe(id BIGINT AUTO_INCREMENT PRIMARY KEY, label VARCHAR(80) DEFAULT 'default label', data VARBINARY(8), qty INT NOT NULL DEFAULT 2, doubled INT GENERATED ALWAYS AS (qty*2) STORED) ENGINE=InnoDB").unwrap();
        let insert = |id, fields| Change::InsertRow { id, table:"apply_crud_probe".into(), fields };
        let plan = driver.plan(&[insert(1, vec![]), insert(2, vec![("label".into(),Value::Text("O'Brien 東京 🦀".into())), ("data".into(),Value::Blob(vec![0,255,39,92])), ("qty".into(),Value::Integer(7))])]).unwrap();
        assert_eq!(plan.statements.len(),2);
        driver.apply(&plan).unwrap();
        let rows = driver.browse(&Browse { table:"apply_crud_probe".into(), sort:Some(("id".into(),false)), ..Default::default() }).unwrap().grid;
        assert_eq!(rows.rows.len(),2);
        assert_eq!(&rows.rows[0][1..], &[Value::Text("default label".into()),Value::Null,Value::Integer(2),Value::Integer(4)]);
        assert_eq!(&rows.rows[1][1..], &[Value::Text("O'Brien 東京 🦀".into()),Value::Blob(vec![0,255,39,92]),Value::Integer(7),Value::Integer(14)]);
        assert!(driver.plan(&[insert(3, vec![("doubled".into(),Value::Integer(9))])]).is_err());
        assert!(driver.plan(&[insert(3, vec![("qty".into(),Value::Null)])]).is_err());
        let original = rows.rows[0].clone();
        let deletion = Change::DeleteRow { table:"apply_crud_probe".into(), original:original.clone() };
        // The delete is executed before the duplicate insert fails; rollback
        // must put the deleted row back, not leave a partial batch behind.
        let duplicate = insert(3, vec![("id".into(), rows.rows[1][0].clone())]);
        let plan = driver.plan(&[deletion.clone(), duplicate]).unwrap();
        assert!(driver.apply(&plan).is_err());
        assert_eq!(driver.transaction_state(), TransactionState::Idle);
        let count: Option<u64> = observer.query_first("SELECT COUNT(*) FROM apply_crud_probe").unwrap();
        assert_eq!(count,Some(2));
        let mut replacement = vec![("id".into(),original[0].clone()), ("label".into(),Value::Null)];
        replacement.push(("qty".into(),Value::Integer(3)));
        let plan = driver.plan(&[deletion,insert(4,replacement)]).unwrap();
        driver.apply(&plan).unwrap();
        let rows = driver.browse(&Browse { table:"apply_crud_probe".into(), sort:Some(("id".into(),false)), ..Default::default() }).unwrap().grid;
        assert_eq!(&rows.rows[0][1..], &[Value::Null,Value::Null,Value::Integer(3),Value::Integer(6)]);
        let deletion = Change::DeleteRow { table:"apply_crud_probe".into(), original:rows.rows[1].clone() };
        let plan = driver.plan(&[deletion.clone(),deletion]).unwrap();
        assert_eq!(plan.statements.len(),1, "deduplicate the same deleted row");
        driver.apply(&plan).unwrap();
        assert!(driver.apply(&plan).unwrap_err().contains("Row changed or was deleted"));
        let count: Option<u64> = observer.query_first("SELECT COUNT(*) FROM apply_crud_probe").unwrap();
        assert_eq!(count,Some(1));
        observer.query_drop("DROP TABLE apply_crud_probe").unwrap();
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_review_rejects_disabled_session_constraints_without_changing_settings() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host:"127.0.0.1".into(), port, user:"rustable".into(), password:"rustable-test-only".into(), database:"rustable_test".into(), tls:false, ..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        driver.conn().unwrap().query_drop("DROP TABLE IF EXISTS apply_constraints_probe").unwrap();
        driver.conn().unwrap().query_drop("CREATE TABLE apply_constraints_probe(id INT PRIMARY KEY, n INT CHECK(n>0)) ENGINE=InnoDB").unwrap();
        driver.conn().unwrap().query_drop("INSERT INTO apply_constraints_probe VALUES(1,1)").unwrap();
        let original = driver.browse(&Browse { table:"apply_constraints_probe".into(), ..Default::default() }).unwrap().grid.rows[0].clone();
        let plan = driver.plan(&[Change::Cell { table:"apply_constraints_probe".into(), column:"n".into(), original, value:Value::Integer(2) }]).unwrap();
        let mut settings = vec!["foreign_key_checks", "unique_checks"];
        if port == 43317 { settings.push("check_constraint_checks"); }
        for setting in settings {
            driver.conn().unwrap().query_drop(format!("SET SESSION {setting}=0")).unwrap();
            let error = driver.apply(&plan).unwrap_err();
            assert!(error.contains("constraint enforcement"), "{error}");
            assert_eq!(driver.transaction_state(), TransactionState::Idle);
            let enabled: Option<u8> = driver.conn().unwrap().query_first(format!("SELECT @@SESSION.{setting}")).unwrap();
            assert_eq!(enabled, Some(0), "do not silently override the user's SQL settings");
            let n: Option<i64> = driver.conn().unwrap().query_first("SELECT n FROM apply_constraints_probe WHERE id=1").unwrap();
            assert_eq!(n, Some(1));
            driver.conn().unwrap().query_drop(format!("SET SESSION {setting}=1")).unwrap();
        }
        driver.apply(&plan).unwrap();
        let n: Option<i64> = driver.conn().unwrap().query_first("SELECT n FROM apply_constraints_probe WHERE id=1").unwrap();
        assert_eq!(n, Some(2));
        driver.conn().unwrap().query_drop("DROP TABLE apply_constraints_probe").unwrap();
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_review_cancel_after_first_write_rolls_back_batch_and_reuses_connection() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host:"127.0.0.1".into(), port, user:"rustable".into(), password:"rustable-test-only".into(), database:"rustable_test".into(), tls:false, read_timeout:Duration::from_secs(5), ..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        let mut blocker = Conn::new(OptsBuilder::from(&config)).unwrap();
        let mut observer = Conn::new(OptsBuilder::from(&config)).unwrap();
        // Test-only dirty reads observe the first uncommitted write directly.
        observer.query_drop("SET SESSION TRANSACTION ISOLATION LEVEL READ UNCOMMITTED").unwrap();
        driver.conn().unwrap().query_drop("DROP TABLE IF EXISTS apply_cancel_probe").unwrap();
        driver.conn().unwrap().query_drop("CREATE TABLE apply_cancel_probe(id INT PRIMARY KEY, label VARCHAR(20)) ENGINE=InnoDB").unwrap();
        driver.conn().unwrap().query_drop("INSERT INTO apply_cancel_probe VALUES(1,'before1'),(2,'before2')").unwrap();
        let rows = driver.browse(&Browse { table:"apply_cancel_probe".into(), sort:Some(("id".into(),false)), ..Default::default() }).unwrap().grid;
        let changes = rows.rows.iter().map(|original| Change::Cell { table:"apply_cancel_probe".into(), column:"label".into(), original:original.clone(), value:Value::Text("saved".into()) }).collect::<Vec<_>>();
        let plan = driver.plan(&changes).unwrap();
        let retry = plan.clone();
        let id = driver.conn().unwrap().connection_id();
        let cancel = driver.cancel.clone();
        blocker.query_drop("START TRANSACTION").unwrap();
        blocker.query_drop("SELECT * FROM apply_cancel_probe WHERE id=2 FOR UPDATE").unwrap();
        let worker = std::thread::spawn(move || {
            let result = driver.apply(&plan);
            (driver, result)
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut wrote_first = false;
        while std::time::Instant::now() < deadline {
            let label: Option<String> = observer.query_first("SELECT label FROM apply_cancel_probe WHERE id=1").unwrap();
            if label.as_deref() == Some("saved") { wrote_first = true; break; }
            std::thread::sleep(Duration::from_millis(10));
        }
        let waiting: Option<(String, Option<String>)> = observer.exec_first("SELECT STATE, INFO FROM information_schema.PROCESSLIST WHERE ID=?", (id,)).unwrap();
        let cancelled_at = std::time::Instant::now();
        cancel.store(true, Ordering::Release);
        let (mut driver, result) = worker.join().unwrap();
        blocker.query_drop("ROLLBACK").unwrap();
        assert!(wrote_first, "must observe a real first write before requesting cancellation; result={result:?}; waiting={waiting:?}");
        assert!(cancelled_at.elapsed() < Duration::from_secs(3));
        assert!(result.is_err(), "cancelled save must not commit");
        assert_eq!(driver.transaction_state(), TransactionState::Idle);
        let labels: Vec<String> = observer.query("SELECT label FROM apply_cancel_probe ORDER BY id").unwrap();
        assert_eq!(labels, ["before1", "before2"]);
        cancel.store(false, Ordering::Release);
        assert_eq!(driver.conn().unwrap().connection_id(), id, "successful rollback keeps the original session");
        driver.apply(&retry).unwrap();
        let labels: Vec<String> = observer.query("SELECT label FROM apply_cancel_probe ORDER BY id").unwrap();
        assert_eq!(labels, ["saved", "saved"]);
        observer.query_drop("DROP TABLE apply_cancel_probe").unwrap();
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_review_apply_rejects_hidden_trigger_metadata() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let mut config = MysqlConfig { host:"127.0.0.1".into(), port, user:"root".into(), password:"rustable-test-only".into(), database:"rustable_test".into(), tls:false, ..Default::default() };
        let mut admin = Conn::new(OptsBuilder::from(&config)).unwrap();
        admin.query_drop("DROP USER IF EXISTS 'rustable_review_limited'@'%'").unwrap();
        admin.query_drop("CREATE USER 'rustable_review_limited'@'%' IDENTIFIED BY 'rustable-test-only'").unwrap();
        admin.query_drop("GRANT SELECT, UPDATE ON rustable_test.* TO 'rustable_review_limited'@'%'").unwrap();
        admin.query_drop("DROP TABLE IF EXISTS hidden_trigger_probe").unwrap();
        admin.query_drop("CREATE TABLE hidden_trigger_probe(id INT PRIMARY KEY, label VARCHAR(20)) ENGINE=InnoDB").unwrap();
        admin.query_drop("INSERT INTO hidden_trigger_probe VALUES(1,'original')").unwrap();
        admin.query_drop("CREATE TRIGGER hidden_trigger_probe_t BEFORE UPDATE ON hidden_trigger_probe FOR EACH ROW SET NEW.label='trigger'").unwrap();
        config.user = "rustable_review_limited".into();
        let mut driver = MysqlDriver::connect(&config).unwrap();
        if port == 43316 {
            let count: Option<u64> = driver.conn().unwrap().query_first("SELECT COUNT(*) FROM information_schema.TRIGGERS WHERE EVENT_OBJECT_TABLE='hidden_trigger_probe'").unwrap();
            assert_eq!(count, Some(0), "MySQL hides triggers from this account");
        } else {
            assert!(driver.conn().unwrap().query_drop("SHOW CREATE TRIGGER hidden_trigger_probe_t").is_err(), "MariaDB must deny the trigger definition");
        }
        let limited_rows = driver.browse(&Browse { table:"hidden_trigger_probe".into(), ..Default::default() }).unwrap();
        assert_eq!(limited_rows.grid.rows, [vec![Value::Integer(1), Value::Text("original".into())]]);
        if port == 43317 {
            assert_eq!(limited_rows.triggers.len(), 1);
            assert!(limited_rows.triggers[0].metadata_error.is_some());
            assert!(limited_rows.triggers[0].ddl.is_empty());
        } else {
            assert!(limited_rows.triggers.is_empty());
        }
        // Review produced by a privileged session must still be rejected when
        // applied under a session that cannot establish complete visibility.
        let mut review_config = config.clone();
        review_config.user = "rustable".into();
        let mut reviewer = MysqlDriver::connect(&review_config).unwrap();
        let rows = reviewer.browse(&Browse { table:"hidden_trigger_probe".into(), ..Default::default() }).unwrap();
        let plan = reviewer.plan(&[Change::Cell { table:"hidden_trigger_probe".into(), column:"label".into(), original:rows.grid.rows[0].clone(), value:Value::Text("edited".into()) }]).unwrap();
        assert!(driver.apply(&plan).unwrap_err().contains("direct privileges"));
        assert_eq!(driver.transaction_state(), TransactionState::Idle);
        assert_eq!(admin.query_first::<String,_>("SELECT label FROM hidden_trigger_probe WHERE id=1").unwrap().as_deref(),Some("original"));
        drop(driver);
        admin.query_drop("DROP TABLE hidden_trigger_probe").unwrap();
        admin.query_drop("DROP USER 'rustable_review_limited'@'%'").unwrap();
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_review_apply_commit_conflict_warning_and_batch_rollback() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host:"127.0.0.1".into(), port, user:"rustable".into(), password:"rustable-test-only".into(), database:"rustable_test".into(), tls:false, ..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        let mut other = Conn::new(OptsBuilder::from(&config)).unwrap();
        driver.conn().unwrap().query_drop("DROP TABLE IF EXISTS apply_probe").unwrap();
        driver.conn().unwrap().query_drop("CREATE TABLE apply_probe(id INT PRIMARY KEY, label VARCHAR(8)) ENGINE=InnoDB").unwrap();
        driver.conn().unwrap().query_drop("INSERT INTO apply_probe VALUES(1,'before'),(2,'other')").unwrap();
        let edit = |driver: &mut MysqlDriver, id: i64, column: &str, value: Value| {
            let rows = driver.browse(&Browse { table:"apply_probe".into(), ..Default::default() }).unwrap().grid;
            let original = rows.rows.iter().find(|r| r[0] == Value::Integer(id)).unwrap().clone();
            Change::Cell { table:"apply_probe".into(), column:column.into(), original, value }
        };
        let change = edit(&mut driver, 1, "label", Value::Text("saved".into()));
        let plan = driver.plan(&[change]).unwrap();
        driver.apply(&plan).unwrap();
        assert_eq!(other.query_first::<String,_>("SELECT label FROM apply_probe WHERE id=1").unwrap().as_deref(),Some("saved"));
        assert!(driver.apply(&plan).unwrap_err().contains("Row changed"));
        let unchanged = edit(&mut driver, 1, "label", Value::Text("saved".into()));
        let plan = driver.plan(&[unchanged]).unwrap();
        driver.apply(&plan).unwrap();
        let first = edit(&mut driver, 1, "label", Value::Text("rollback".into()));
        let second = edit(&mut driver, 2, "id", Value::Integer(1));
        let plan = driver.plan(&[first, second]).unwrap();
        assert!(driver.apply(&plan).is_err());
        assert_eq!(other.query_first::<String,_>("SELECT label FROM apply_probe WHERE id=1").unwrap().as_deref(),Some("saved"));
        assert_eq!(driver.transaction_state(), TransactionState::Idle);
        driver.conn().unwrap().query_drop("SET sql_mode=''").unwrap();
        let long = edit(&mut driver, 1, "label", Value::Text("silently truncated".into()));
        let plan = driver.plan(&[long]).unwrap();
        assert!(driver.apply(&plan).unwrap_err().contains("warning"));
        assert_eq!(other.query_first::<String,_>("SELECT label FROM apply_probe WHERE id=1").unwrap().as_deref(),Some("saved"));
        let change = edit(&mut driver, 1, "label", Value::Text("next".into()));
        let plan = driver.plan(&[change]).unwrap();
        other.query_drop("CREATE TRIGGER apply_probe_trigger BEFORE UPDATE ON apply_probe FOR EACH ROW SET NEW.label='trigger'").unwrap();
        assert!(driver.apply(&plan).unwrap_err().contains("triggers"));
        other.query_drop("DROP TRIGGER apply_probe_trigger").unwrap();
        driver.cancel.store(true, Ordering::Release);
        assert!(driver.apply(&plan).is_err());
        driver.cancel.store(false, Ordering::Release);
        assert_eq!(driver.transaction_state(), TransactionState::Idle);
        driver.conn().unwrap().query_drop("START TRANSACTION").unwrap();
        assert!(driver.apply(&plan).is_err());
        assert_eq!(driver.transaction_state(), TransactionState::Active);
        driver.conn().unwrap().query_drop("ROLLBACK").unwrap();
        other.query_drop("ALTER TABLE apply_probe ADD COLUMN extra INT").unwrap();
        assert!(driver.apply(&plan).unwrap_err().contains("definition changed"));
        assert_eq!(driver.transaction_state(), TransactionState::Idle);
        other.query_drop("DROP TABLE apply_probe").unwrap();
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_review_snapshot_ignores_auto_increment_progress() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host:"127.0.0.1".into(), port, user:"rustable".into(), password:"rustable-test-only".into(), database:"rustable_test".into(), tls:false, ..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        let mut other = Conn::new(OptsBuilder::from(&config)).unwrap();
        driver.conn().unwrap().query_drop("DROP TABLE IF EXISTS review_counter_probe").unwrap();
        driver.conn().unwrap().query_drop("CREATE TABLE review_counter_probe(id BIGINT AUTO_INCREMENT PRIMARY KEY, label VARCHAR(60) DEFAULT 'AUTO_INCREMENT=22') ENGINE=InnoDB").unwrap();
        let empty = driver.edit_schema_snapshot("rustable_test", "review_counter_probe").unwrap();
        other.query_drop("INSERT INTO review_counter_probe(label) VALUES ('first')").unwrap();
        assert_eq!(empty, driver.edit_schema_snapshot("rustable_test", "review_counter_probe").unwrap());
        let original = driver.browse(&Browse { table:"review_counter_probe".into(), ..Default::default() }).unwrap().grid.rows[0].clone();
        let changes = [Change::Cell { table:"review_counter_probe".into(), column:"label".into(), original, value:Value::Text("edited".into()) }];
        let plan = driver.plan(&changes).unwrap();
        other.query_drop("INSERT INTO review_counter_probe(label) VALUES ('unrelated')").unwrap();
        assert_eq!(plan.schemas, driver.plan(&changes).unwrap().schemas, "unrelated inserts must not invalidate the reviewed schema");
        other.query_drop("ALTER TABLE review_counter_probe AUTO_INCREMENT=9000").unwrap();
        assert_eq!(plan.schemas, driver.plan(&changes).unwrap().schemas);
        other.query_drop("ALTER TABLE review_counter_probe ALTER COLUMN label SET DEFAULT 'AUTO_INCREMENT=23'").unwrap();
        assert_ne!(plan.schemas, driver.plan(&changes).unwrap().schemas, "literal defaults remain part of the fingerprint");
        other.query_drop("DROP TABLE review_counter_probe").unwrap();
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_review_transaction_holds_metadata_and_row_locks_until_rollback() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host:"127.0.0.1".into(), port, user:"rustable".into(), password:"rustable-test-only".into(), database:"rustable_test".into(), tls:false, ..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        let mut other = Conn::new(OptsBuilder::from(&config)).unwrap();
        other.query_drop("SET SESSION lock_wait_timeout=1, innodb_lock_wait_timeout=1").unwrap();
        driver.conn().unwrap().query_drop("DROP TABLE IF EXISTS review_lock_probe").unwrap();
        driver.conn().unwrap().query_drop("CREATE TABLE review_lock_probe(id INT PRIMARY KEY, label VARCHAR(30)) ENGINE=InnoDB").unwrap();
        driver.conn().unwrap().query_drop("INSERT INTO review_lock_probe VALUES(1,'original')").unwrap();
        let original = driver.browse(&Browse { table:"review_lock_probe".into(), ..Default::default() }).unwrap().grid.rows[0].clone();
        let plan = driver.plan(&[Change::Cell { table:"review_lock_probe".into(), column:"label".into(), original, value:Value::Text("edited".into()) }]).unwrap();
        driver.conn().unwrap().query_drop("START TRANSACTION").unwrap();
        // Open the table without reading rows, before validating its snapshot.
        // Both engines must retain this metadata lock until transaction end.
        driver.conn().unwrap().query_drop("SELECT * FROM review_lock_probe LIMIT 0").unwrap();
        assert_eq!(driver.edit_schema_snapshot("rustable_test", "review_lock_probe").unwrap(), plan.schemas[0].1);
        let ddl_error = other.query_drop("ALTER TABLE review_lock_probe ADD COLUMN extra INT").unwrap_err();
        assert!(matches!(ddl_error, ::mysql::Error::MySqlError(ref e) if e.code == 1205), "{ddl_error}");
        let trigger_error = other.query_drop("CREATE TRIGGER review_lock_trigger BEFORE UPDATE ON review_lock_probe FOR EACH ROW SET NEW.label='trigger'").unwrap_err();
        assert!(matches!(trigger_error, ::mysql::Error::MySqlError(ref e) if e.code == 1205), "{trigger_error}");
        let check = &plan.preconditions[0];
        let parameters = check.parameters.iter().map(::mysql::Value::try_from).collect::<Result<Vec<_>,_>>().unwrap();
        let matches: Vec<u8> = driver.conn().unwrap().exec(&check.sql, parameters.clone()).unwrap();
        assert_eq!(matches, [1]);
        let update_error = other.query_drop("UPDATE review_lock_probe SET label='concurrent' WHERE id=1").unwrap_err();
        assert!(matches!(update_error, ::mysql::Error::MySqlError(ref e) if e.code == 1205), "{update_error}");
        let statement = &plan.statements[0];
        let values = statement.parameters.iter().map(::mysql::Value::try_from).collect::<Result<Vec<_>,_>>().unwrap();
        driver.conn().unwrap().exec_drop(&statement.sql, values).unwrap();
        assert_eq!(driver.conn().unwrap().affected_rows(), 1);
        driver.conn().unwrap().query_drop("ROLLBACK").unwrap();
        let value: Option<String> = other.query_first("SELECT label FROM review_lock_probe WHERE id=1").unwrap();
        assert_eq!(value.as_deref(), Some("original"));
        other.query_drop("UPDATE review_lock_probe SET label='concurrent' WHERE id=1").unwrap();
        driver.conn().unwrap().query_drop("START TRANSACTION").unwrap();
        let matches: Vec<u8> = driver.conn().unwrap().exec(&check.sql, parameters).unwrap();
        assert!(matches.is_empty(), "a stale review must not overwrite a committed competing change");
        driver.conn().unwrap().query_drop("ROLLBACK").unwrap();
        other.query_drop("ALTER TABLE review_lock_probe ADD COLUMN extra INT").unwrap();
        assert_ne!(driver.edit_schema_snapshot("rustable_test", "review_lock_probe").unwrap(), plan.schemas[0].1);
        other.query_drop("DROP TABLE review_lock_probe").unwrap();
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_edit_preconditions_preserve_floating_and_temporal_values() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host:"127.0.0.1".into(), port, user:"rustable".into(), password:"rustable-test-only".into(), database:"rustable_test".into(), tls:false, ..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        driver.conn().unwrap().query_drop("DROP TABLE IF EXISTS edit_types_probe").unwrap();
        driver.conn().unwrap().query_drop("CREATE TABLE edit_types_probe(id INT PRIMARY KEY, f FLOAT, d DOUBLE, amount DECIMAL(30,10), dt DATETIME(6), duration TIME(6), bits BIT(9), data VARBINARY(8), label VARCHAR(20)) ENGINE=InnoDB").unwrap();
        driver.conn().unwrap().query_drop("INSERT INTO edit_types_probe VALUES(1,1.2,1.2345678901234567,12345678901234567890.1234567890,'2026-11-01 12:34:56.000123','-817:59:59.000001',b'100000001',X'00FF','before')").unwrap();
        let original = driver.browse(&Browse { table:"edit_types_probe".into(), ..Default::default() }).unwrap().grid.rows[0].clone();
        let plan = driver.plan(&[Change::Cell { table:"edit_types_probe".into(), column:"label".into(), original, value:Value::Text("after".into()) }]).unwrap();
        let check = &plan.preconditions[0];
        let params = check.parameters.iter().map(::mysql::Value::try_from).collect::<Result<Vec<_>,_>>().unwrap();
        let matches: Vec<u8> = driver.conn().unwrap().exec(&check.sql, params.clone()).unwrap();
        assert_eq!(matches, [1], "unchanged row must pass its review precondition");
        driver.conn().unwrap().query_drop("UPDATE edit_types_probe SET f=1.2000002").unwrap();
        let matches: Vec<u8> = driver.conn().unwrap().exec(&check.sql, params.clone()).unwrap();
        assert!(matches.is_empty(), "small FLOAT changes must not be rounded away");
        driver.conn().unwrap().query_drop("UPDATE edit_types_probe SET f=1.2,d=1.2345678901234569").unwrap();
        let matches: Vec<u8> = driver.conn().unwrap().exec(&check.sql, params).unwrap();
        assert!(matches.is_empty(), "small DOUBLE changes must not be rounded away");
        driver.conn().unwrap().query_drop("DROP TABLE edit_types_probe").unwrap();
    }

}
