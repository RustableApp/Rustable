//! MySQL/MariaDB driver foundation. Not exposed by the connection picker yet.
use super::*;
use ::mysql::{Conn, OptsBuilder, prelude::Queryable};
use crate::{connection::MysqlConfig, sql::mysql::Mode};

type CatalogRow = (String, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>);

struct MysqlDriver {
    connection: Option<Conn>,
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
        let mut connection = Conn::new(OptsBuilder::from(config)).map_err(|e| e.to_string())?;
        connection.query_drop("SET SESSION session_track_system_variables = 'sql_mode'").map_err(|e| e.to_string())?;
        let (version, database, sql_mode): (String, Option<String>, String) = connection.query_first(
            "SELECT VERSION(), DATABASE(), @@SESSION.sql_mode").map_err(|e| e.to_string())?
            .ok_or("Server did not return session metadata")?;
        let names = connection.query("SHOW DATABASES").map_err(|e| e.to_string())?;
        Ok(Self {
            connection: Some(connection),
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
        let rows = self.conn()?.exec_iter(sql, parameters).map_err(|e| e.to_string())?;
        for row in rows {
            let row = row.map_err(|e| e.to_string())?;
            let (name, kind, column, ty, nullable, default, key, extra): CatalogRow =
                ::mysql::from_row_opt(row).map_err(|e| e.to_string())?;
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
    fn table_metadata(&mut self, schema: &str, table: &str) -> Result<(Vec<Index>, Vec<Object>), String> {
        let qualified = format!("{}.{}", quote(schema), quote(table));
        let definition: Option<::mysql::Row> = self.conn()?.query_first(format!("SHOW CREATE TABLE {qualified}"))
            .map_err(|e| e.to_string())?;
        let ddl = definition.and_then(|row| row.get::<String, _>(1)).ok_or("Missing table definition")?;
        let mut indexes = std::collections::BTreeMap::<String, Index>::new();
        let rows: Vec<::mysql::Row> = self.conn()?.query(format!("SHOW INDEX FROM {qualified}"))
            .map_err(|e| e.to_string())?;
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
        let names: Vec<String> = self.conn()?.exec(
            "SELECT TRIGGER_NAME FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA=? AND EVENT_OBJECT_TABLE=? ORDER BY TRIGGER_NAME", (schema, table))
            .map_err(|e| e.to_string())?;
        let mut triggers = Vec::new();
        for name in names {
            let row: Option<::mysql::Row> = self.conn()?.query_first(format!("SHOW CREATE TRIGGER {}.{}", quote(schema), quote(&name)))
                .map_err(|e| e.to_string())?;
            let ddl = row.and_then(|row| row.get::<String, _>("SQL Original Statement"))
                .ok_or("Missing trigger definition")?;
            triggers.push(Object { schema: Some(schema.into()), name, display_name: None,
                metadata_error: None, columns: Vec::new(), kind: "trigger".into(), ddl });
        }
        Ok((indexes.into_values().collect(), triggers))
    }
    fn require_idle(&mut self) -> Result<(), String> {
        match self.transaction_state() {
            TransactionState::Idle => Ok(()),
            _ => Err("Finish the SQL transaction before switching databases.".into()),
        }
    }
}

impl Driver for MysqlDriver {
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
        self.conn()?.query("SHOW DATABASES").map_err(|e| e.to_string())
    }
    fn schema_catalog(&self) -> SchemaCatalog { self.schemas.clone() }
    fn select_schema(&mut self, name: &str) -> Result<Vec<Object>, String> {
        self.require_idle()?;
        // Quote the whole identifier; a database name is never a SQL fragment.
        self.conn()?.query_drop(format!("USE {}", quote(name))).map_err(|e| e.to_string())?;
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
                    report.error = Some(error.to_string());
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
                if result.error.is_some() { report.error = result.error; }
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
        let object = self.catalog_objects(Some(&request.table))?.into_iter().next().ok_or("Table no longer exists")?;
        let mut sql = format!("SELECT * FROM {}.{}", quote(&schema), quote(&object.name));
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
        sql.push_str(&format!(" LIMIT {} OFFSET {}", PAGE_SIZE + 1, request.offset));
        let params: Vec<::mysql::Value> = parameters.iter().map(|value| match value {
            Value::Text(text) => ::mysql::Value::Bytes(text.as_bytes().to_vec()), _ => unreachable!(),
        }).collect();
        let mut remaining = BYTE_LIMIT;
        let outcome = self.conn()?.exec_iter(&sql, params).map(|results|
            super::mysql_results::collect_budget(results, &mut remaining, PAGE_SIZE + 1, 1));
        let (report, disconnected) = match outcome {
            Ok(result) => result,
            Err(error) => {
                if error.is_connectivity_error() { self.connection = None; }
                return Err(error.to_string());
            }
        };
        if disconnected { self.connection = None; }
        if let Some(error) = report.error { return Err(error); }
        let mut grid = Arc::unwrap_or_clone(report.grids.into_iter().next().ok_or("Missing table result")?);
        if grid.rows.len() > PAGE_SIZE { grid.rows.truncate(PAGE_SIZE); grid.truncated = true; }
        grid.set_browse_total(request.offset, None);
        let (indexes, triggers) = if object.kind == "view" { (Vec::new(), Vec::new()) }
            else { self.table_metadata(&schema, &object.name)? };
        Ok(TableData { sql, parameters, grid, columns: object.columns, indexes, triggers })
    }
    fn plan(&mut self, _changes: &[Change]) -> Result<ChangePlan, String> {
        Err("MySQL table editing is still being implemented.".into())
    }
    fn apply(&mut self, _plan: &ChangePlan) -> Result<(), String> {
        Err("MySQL table editing is still being implemented.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let mut driver = Box::<dyn Driver>::try_from(&config).unwrap();
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

}
