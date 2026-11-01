//! MySQL/MariaDB driver foundation. Not exposed by the connection picker yet.
use super::*;
use ::mysql::{Conn, OptsBuilder, prelude::Queryable};
use crate::{connection::MysqlConfig, sql::mysql::Mode};

type CatalogRow = (String, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>);

struct MysqlDriver {
    connection: Option<Conn>,
    engine: &'static str,
    schemas: SchemaCatalog,
    mode: Mode,
}

impl TryFrom<&MysqlConfig> for Box<dyn Driver> {
    type Error = String;
    fn try_from(config: &MysqlConfig) -> Result<Self, String> {
        let mut connection = Conn::new(OptsBuilder::from(config)).map_err(|e| e.to_string())?;
        let (version, database, sql_mode): (String, Option<String>, String) = connection.query_first(
            "SELECT VERSION(), DATABASE(), @@SESSION.sql_mode").map_err(|e| e.to_string())?
            .ok_or("Server did not return session metadata")?;
        let names = connection.query("SHOW DATABASES").map_err(|e| e.to_string())?;
        Ok(Box::new(MysqlDriver {
            connection: Some(connection),
            engine: if version.contains("MariaDB") { "MariaDB" } else { "MySQL" },
            schemas: SchemaCatalog { names, selected: database },
            mode: parse_mode(&sql_mode),
        }))
    }
}

fn parse_mode(value: &str) -> Mode {
    let modes: Vec<_> = value.split(',').map(str::trim).collect();
    Mode {
        ansi_quotes: modes.iter().any(|s| s.eq_ignore_ascii_case("ANSI_QUOTES")),
        no_backslash_escapes: modes.iter().any(|s| s.eq_ignore_ascii_case("NO_BACKSLASH_ESCAPES")),
    }
}
fn quote(name: &str) -> String { format!("`{}`", name.replace('`', "``")) }

impl MysqlDriver {
    fn conn(&mut self) -> Result<&mut Conn, String> {
        self.connection.as_mut().ok_or_else(|| "MySQL connection is disconnected".into())
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
        let Some(schema) = self.schemas.selected.clone() else { return Ok(Vec::new()); };
        let mut result = Vec::<Object>::new();
        let rows = self.conn()?.exec_iter(
            "SELECT t.TABLE_NAME,t.TABLE_TYPE,c.COLUMN_NAME,c.COLUMN_TYPE,c.IS_NULLABLE,c.COLUMN_DEFAULT,c.COLUMN_KEY,c.EXTRA FROM information_schema.TABLES t LEFT JOIN information_schema.COLUMNS c ON c.TABLE_SCHEMA=t.TABLE_SCHEMA AND c.TABLE_NAME=t.TABLE_NAME WHERE t.TABLE_SCHEMA=? ORDER BY t.TABLE_NAME,c.ORDINAL_POSITION", (&schema,)).map_err(|e| e.to_string())?;
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
    fn execute(&mut self, sql: &str, safe: bool, cancel: &AtomicBool) -> ScriptResult {
        let started = std::time::Instant::now();
        let mut report = ScriptResult::default();
        if safe { report.error = Some("MySQL read-only execution is not yet available.".into()); return report; }
        let statements = match crate::sql::mysql::ranges(sql, self.mode, true) {
            Ok(statements) => statements,
            Err(error) => { report.error = Some(error); return report; }
        };
        let mut remaining = BYTE_LIMIT;
        for range in statements {
            if report.grids.len() >= 128 {
                report.error = Some("Stopped after 128 result sets; later statements were not executed.".into());
                break;
            }
            if cancel.load(Ordering::Relaxed) { report.error = Some("Cancelled before next statement".into()); break; }
            let Some(conn) = self.connection.as_mut() else { report.error = Some("MySQL connection is disconnected".into()); break; };
            let outcome = conn.query_iter(&sql[range]).map(|results|
                super::mysql_results::collect_budget(results, &mut remaining, QUERY_LIMIT, 128 - report.grids.len()));
            let result = match outcome {
                Ok(result) => result,
                Err(error) => {
                    let disconnected = error.is_connectivity_error();
                    report.error = Some(error.to_string());
                    if disconnected { self.connection = None; }
                    break;
                }
            };
            report.grids.extend(result.grids);
            if result.error.is_some() {
                // Until transport error classification is carried through the collector,
                // conservatively close on stream errors. Never reuse an uncertain stream.
                report.error = result.error;
                self.connection = None;
                break;
            }
        }
        report.elapsed = started.elapsed();
        report
    }
    fn browse(&mut self, request: &Browse) -> Result<TableData, String> {
        let schema = self.schemas.selected.clone().ok_or("Choose a database first")?;
        let object = self.objects()?.into_iter().find(|o| o.name == request.table).ok_or("Table no longer exists")?;
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
        let report = ScriptResult::from(self.conn()?.exec_iter(&sql, params).map_err(|e| e.to_string())?);
        if let Some(error) = report.error { return Err(error); }
        let mut grid = Arc::unwrap_or_clone(report.grids.into_iter().next().ok_or("Missing table result")?);
        if grid.rows.len() > PAGE_SIZE { grid.rows.truncate(PAGE_SIZE); grid.truncated = true; }
        grid.set_browse_total(request.offset, None);
        Ok(TableData { sql, parameters, grid, columns: object.columns, indexes: vec![], triggers: vec![] })
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
        let mut driver = Box::<dyn Driver>::try_from(&config).unwrap();
        assert_eq!(driver.engine_name(), if port == 43317 { "MariaDB" } else { "MySQL" });
        assert!(driver.databases().unwrap().contains(&"rustable_test".into()));
        let cancel = AtomicBool::new(false);
        let setup = driver.execute("DROP TABLE IF EXISTS `driver``probe`; CREATE TABLE `driver``probe` (id BIGINT PRIMARY KEY, label VARCHAR(100), doubled BIGINT GENERATED ALWAYS AS (id*2) STORED); INSERT INTO `driver``probe` (id,label) VALUES (1,'100%'),(2,'100x'),(3,'日本');", false, &cancel);
        assert!(setup.error.is_none(), "{:?}", setup.error);
        let object = driver.objects().unwrap().into_iter().find(|o| o.name == "driver`probe").unwrap();
        assert_eq!(object.columns.len(), 3);
        assert!(object.columns[0].pk);
        assert!(object.columns[2].generated);
        let table = driver.browse(&Browse { table: "driver`probe".into(), filter_column: Some("label".into()), filter: "%".into(), ..Default::default() }).unwrap();
        assert_eq!(table.grid.rows.len(), 1);
        assert_eq!(table.grid.rows[0][0], Value::Integer(1));
        assert_eq!(table.grid.rows[0][2], Value::Integer(2));
        let sorted = driver.browse(&Browse { table: "driver`probe".into(), sort: Some(("id".into(), true)), offset: 1, ..Default::default() }).unwrap();
        assert_eq!(sorted.grid.rows[0][0], Value::Integer(2));
        assert!(driver.execute("START TRANSACTION; UPDATE `driver``probe` SET label='changed' WHERE id=1;", false, &cancel).error.is_none());
        assert_eq!(driver.transaction_state(), TransactionState::Active);
        assert!(driver.select_schema("rustable_test").is_err());
        assert!(driver.execute("ROLLBACK", false, &cancel).error.is_none());
        assert_eq!(driver.transaction_state(), TransactionState::Idle);
        assert!(driver.select_schema("rustable_test").is_ok());
        assert!(driver.execute("DROP TABLE `driver``probe`", false, &cancel).error.is_none());
    }
}
