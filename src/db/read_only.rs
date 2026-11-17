//! Headless read-only database sessions for automation clients (MCP).
//!
//! This type deliberately exposes no write, import, plan/apply or export method.
//! Every statement runs through the driver's Safe-mode path, which the server
//! itself enforces (PostgreSQL `transaction_read_only`, MySQL/MariaDB
//! `START TRANSACTION READ ONLY`, SQLite `query_only` plus a read-only file
//! handle). Sessions are reused across calls so warm operations keep the same
//! exchange budget as the editor and table browser.
use super::{Browse, Driver, Object, SchemaCatalog, ScriptResult, TableData};
use std::sync::{Arc, atomic::AtomicBool};

pub(crate) struct ReadOnlySession {
    driver: Box<dyn Driver>,
    cancel: Arc<AtomicBool>,
}

pub(crate) struct TablePage {
    pub grid: super::Grid,
}

impl ReadOnlySession {
    /// Opens the file without write access; Safe mode is still applied per query.
    pub(crate) fn sqlite(path: &std::path::Path, cancel: Arc<AtomicBool>) -> Result<Self, String> {
        let driver = super::sqlite::SqliteDriver::open_read_only(path, cancel.clone())?;
        Ok(Self { driver: Box::new(driver), cancel })
    }

    pub(crate) fn postgres(
        config: &crate::connection::PostgresConfig,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let driver = super::postgres::PostgresDriver::open_read_only(config, cancel.clone())?;
        Ok(Self { driver: Box::new(driver), cancel })
    }

    pub(crate) fn mysql(
        config: &crate::connection::MysqlConfig,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let mut driver = super::mysql::open_read_only(config, &cancel)?;
        driver.set_cancellation(cancel.clone());
        Ok(Self { driver, cancel })
    }

    pub(crate) fn engine(&self) -> &'static str {
        self.driver.engine_name()
    }

    /// Local state check only; reconnects after a lost transport but never
    /// replays an earlier operation.
    fn ready(&mut self) -> Result<&mut dyn Driver, String> {
        self.driver.set_cancellation(self.cancel.clone());
        self.driver.ensure_connected()?;
        Ok(self.driver.as_mut())
    }

    pub(crate) fn objects(&mut self) -> Result<Vec<Object>, String> {
        self.ready()?.objects()
    }

    pub(crate) fn select_schema(&mut self, name: &str) -> Result<Vec<Object>, String> {
        self.ready()?.select_schema(name)
    }

    pub(crate) fn schema_catalog(&self) -> SchemaCatalog {
        self.driver.schema_catalog()
    }

    pub(crate) fn definition(&mut self, name: &str) -> Result<String, String> {
        self.ready()?.definition(name)
    }

    /// Bounded page through the table browser's single-exchange path.
    pub(crate) fn browse(&mut self, request: &Browse) -> Result<TablePage, String> {
        let TableData { grid, .. } = self.ready()?.browse(request)?;
        Ok(TablePage { grid })
    }

    /// Always Safe mode. There is intentionally no variant that disables it.
    pub(crate) fn query(&mut self, sql: &str) -> ScriptResult {
        let cancel = self.cancel.clone();
        match self.ready() {
            Ok(driver) => driver.execute(sql, true, &cancel),
            Err(error) => ScriptResult { error: Some(error), ..ScriptResult::default() },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Value;

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture.sqlite");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE people(id INTEGER PRIMARY KEY, name TEXT NOT NULL);
                 INSERT INTO people(name) VALUES ('Ada'), ('Grace'), ('Linus');",
            )
            .unwrap();
        (directory, path)
    }

    #[test]
    fn sqlite_session_reads_but_never_writes() {
        let (_directory, path) = fixture();
        let mut session = ReadOnlySession::sqlite(&path, Arc::new(AtomicBool::new(false))).unwrap();
        let report = session.query("SELECT name FROM people ORDER BY id");
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(report.grids[0].rows[0][0], Value::Text("Ada".into()));
        for sql in [
            "DELETE FROM people",
            "UPDATE people SET name = 'x'",
            "INSERT INTO people(name) VALUES ('x')",
            "DROP TABLE people",
            "CREATE TABLE t(i)",
            "PRAGMA query_only = OFF; DELETE FROM people",
            "ATTACH DATABASE ':memory:' AS other",
            "ATTACH DATABASE 'created-by-attach.sqlite' AS other",
            "VACUUM INTO 'copy.sqlite'",
            "BEGIN; DELETE FROM people; COMMIT",
            "WITH x AS (SELECT 1) DELETE FROM people",
        ] {
            assert!(session.query(sql).error.is_some(), "read-only session accepted {sql}");
        }
        let check = rusqlite::Connection::open(&path).unwrap();
        let count: i64 = check.query_row("SELECT count(*) FROM people", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 3);
        assert!(!std::path::Path::new("created-by-attach.sqlite").exists());
        assert!(!std::path::Path::new("copy.sqlite").exists());
        assert!(session.objects().unwrap().iter().any(|object| object.name == "people"));
        let page = session
            .browse(&Browse { table: "people".into(), limit: 2, ..Browse::default() })
            .unwrap();
        assert_eq!(page.grid.rows.len(), 2);
    }

    #[test]
    #[ignore = "requires disposable PostgreSQL on port 54329"]
    fn postgres_session_is_read_only() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let mut admin = ::postgres::Client::connect(
            "host=127.0.0.1 port=54329 user=postgres password=rustable_qa_only dbname=rustable_qa",
            ::postgres::NoTls,
        )
        .unwrap();
        admin
            .batch_execute(
                "DROP SCHEMA IF EXISTS rustable_mcp_qa CASCADE; CREATE SCHEMA rustable_mcp_qa;
                 CREATE TABLE rustable_mcp_qa.items(id serial PRIMARY KEY, label text);
                 INSERT INTO rustable_mcp_qa.items(label) VALUES ('a'), ('b');
                 CREATE TABLE rustable_mcp_qa.audit(n int);
                 CREATE FUNCTION rustable_mcp_qa.touch() RETURNS int LANGUAGE plpgsql VOLATILE AS
                   $$ BEGIN INSERT INTO rustable_mcp_qa.audit VALUES (1); RETURN 1; END $$;
                 CREATE VIEW rustable_mcp_qa.touching AS SELECT rustable_mcp_qa.touch() AS n;",
            )
            .unwrap();
        let config = crate::connection::PostgresConfig {
            host: "127.0.0.1".into(),
            port: 54329,
            database: "rustable_qa".into(),
            user: "postgres".into(),
            password: "rustable_qa_only".into(),
            tls: crate::connection::PostgresTlsMode::Disable,
            ssh: None,
        };
        let mut session = ReadOnlySession::postgres(&config, Arc::new(AtomicBool::new(false))).unwrap();
        let report = session.query("SELECT label FROM rustable_mcp_qa.items ORDER BY id");
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(report.grids[0].rows.len(), 2);
        for sql in [
            "INSERT INTO rustable_mcp_qa.items(label) VALUES ('x')",
            "UPDATE rustable_mcp_qa.items SET label = 'x'",
            "DELETE FROM rustable_mcp_qa.items",
            "DROP TABLE rustable_mcp_qa.items",
            "CREATE TABLE rustable_mcp_qa.other(i int)",
            "SELECT nextval('rustable_mcp_qa.items_id_seq')",
            "WITH gone AS (DELETE FROM rustable_mcp_qa.items RETURNING *) SELECT * FROM gone",
            "SELECT set_config('transaction_read_only', 'off', false); DELETE FROM rustable_mcp_qa.items",
            "BEGIN",
            "SET transaction_read_only = off",
            "COPY rustable_mcp_qa.items FROM STDIN",
        ] {
            assert!(session.query(sql).error.is_some(), "read-only session accepted {sql}");
        }
        // Session defaults cannot lift the per-statement read-only transaction.
        assert!(session.query("SELECT set_config('default_transaction_read_only', 'off', false)").error.is_none());
        assert!(session.query("DELETE FROM rustable_mcp_qa.items").error.is_some());
        // A rejected statement must not leave the reused session unusable.
        assert!(session.query("SELECT 1").error.is_none());
        let count: i64 = admin.query_one("SELECT count(*) FROM rustable_mcp_qa.items WHERE label IN ('a','b')", &[]).unwrap().get(0);
        assert_eq!(count, 2);
        let objects = session.select_schema("rustable_mcp_qa").unwrap();
        let items = objects.iter().find(|o| o.display_name() == "items").unwrap();
        let page = session.browse(&Browse { table: items.name.clone(), limit: 1, ..Browse::default() }).unwrap();
        assert_eq!(page.grid.rows.len(), 1);
        assert!(page.grid.truncated);
        // Table browsing must be read-only too: a view can call a function that
        // writes. Cover the cold (catalog-first) and warm (hinted/pipelined) paths,
        // including after SQL turned the session default off.
        let touching = objects.iter().find(|o| o.display_name() == "touching").unwrap().name.clone();
        let view = Browse { table: touching, limit: 10, ..Browse::default() };
        for attempt in 0..3 {
            if attempt == 2 {
                assert!(session.query("SELECT set_config('default_transaction_read_only', 'off', false)").error.is_none());
            }
            let error = session.browse(&view).err().unwrap_or_else(|| panic!("browse {attempt} wrote through a view"));
            assert!(error.contains("read-only"), "{error}");
        }
        let writes: i64 = admin.query_one("SELECT count(*) FROM rustable_mcp_qa.audit", &[]).unwrap().get(0);
        assert_eq!(writes, 0);
        assert!(session.browse(&Browse { table: items.name.clone(), limit: 1, ..Browse::default() }).is_ok());
        // Ordinary desktop sessions are unchanged (no read-only prefix).
        let mut desktop = crate::db::postgres::PostgresDriver::open(&config, Arc::new(AtomicBool::new(false))).unwrap();
        desktop.select_schema("rustable_mcp_qa").unwrap();
        assert!(desktop.browse(&view).is_ok());
        admin.batch_execute("DROP SCHEMA rustable_mcp_qa CASCADE").unwrap();
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn mysql_session_is_read_only() {
        use ::mysql::prelude::Queryable;
        let port: u16 = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = crate::connection::MysqlConfig {
            host: "127.0.0.1".into(),
            port,
            user: "rustable".into(),
            password: "rustable-test-only".into(),
            database: "rustable_test".into(),
            tls: false, // Loopback fixture only.
            ..Default::default()
        };
        let mut admin = ::mysql::Conn::new(::mysql::OptsBuilder::from(&config)).unwrap();
        admin
            .query_drop(
                "DROP TABLE IF EXISTS rustable_mcp_items;
                 CREATE TABLE rustable_mcp_items(id INT AUTO_INCREMENT PRIMARY KEY, label TEXT);
                 INSERT INTO rustable_mcp_items(label) VALUES ('a'), ('b');
                 DROP VIEW IF EXISTS rustable_mcp_touching;
                 DROP FUNCTION IF EXISTS rustable_mcp_touch;
                 DROP TABLE IF EXISTS rustable_mcp_audit;
                 CREATE TABLE rustable_mcp_audit(n INT);",
            )
            .unwrap();
        admin
            .query_drop(
                "CREATE FUNCTION rustable_mcp_touch() RETURNS INT DETERMINISTIC MODIFIES SQL DATA
                 BEGIN INSERT INTO rustable_mcp_audit VALUES (1); RETURN 1; END",
            )
            .unwrap();
        admin.query_drop("CREATE VIEW rustable_mcp_touching AS SELECT rustable_mcp_touch() AS n").unwrap();
        let mut session = ReadOnlySession::mysql(&config, Arc::new(AtomicBool::new(false))).unwrap();
        let report = session.query("SELECT label FROM rustable_mcp_items ORDER BY id");
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(report.grids[0].rows.len(), 2);
        for sql in [
            "INSERT INTO rustable_mcp_items(label) VALUES ('x')",
            "UPDATE rustable_mcp_items SET label = 'x'",
            "DELETE FROM rustable_mcp_items",
            "DROP TABLE rustable_mcp_items",
            "CREATE TABLE rustable_mcp_other(i INT)",
            "SELECT 1 INTO @x",
            "SET SESSION TRANSACTION READ WRITE",
            "START TRANSACTION",
            "/*!40000 DELETE FROM rustable_mcp_items */",
            "SELECT 1; DELETE FROM rustable_mcp_items",
        ] {
            assert!(session.query(sql).error.is_some(), "read-only session accepted {sql}");
        }
        assert!(session.query("SELECT 1").error.is_none());
        let count: Option<i64> = admin.query_first("SELECT count(*) FROM rustable_mcp_items WHERE label IN ('a','b')").unwrap();
        assert_eq!(count, Some(2));
        assert!(session.objects().unwrap().iter().any(|o| o.display_name() == "rustable_mcp_items"));
        // Table browsing runs in autocommit, not the editor's READ ONLY wrapper:
        // the session itself must refuse a view's write.
        let view = Browse { table: "rustable_mcp_touching".into(), limit: 10, ..Browse::default() };
        for attempt in 0..2 {
            assert!(session.browse(&view).is_err(), "browse {attempt} wrote through a view");
        }
        let writes: Option<i64> = admin.query_first("SELECT count(*) FROM rustable_mcp_audit").unwrap();
        assert_eq!(writes, Some(0));
        assert!(session.browse(&Browse { table: "rustable_mcp_items".into(), limit: 1, ..Browse::default() }).is_ok());
        admin
            .query_drop(
                "DROP VIEW rustable_mcp_touching; DROP FUNCTION rustable_mcp_touch;
                 DROP TABLE rustable_mcp_audit; DROP TABLE rustable_mcp_items",
            )
            .unwrap();
    }

    #[test]
    fn sqlite_session_does_not_create_missing_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing.sqlite");
        assert!(ReadOnlySession::sqlite(&path, Arc::new(AtomicBool::new(false))).is_err());
        assert!(!path.exists());
    }
}
