//! Batch orchestration only; each driver retains its bounded, cancellable export path.
use super::*;

/// Portable basename with no path components. An ordinal disambiguates collisions
/// (including case folding and sanitization) inside the new batch directory.
pub(crate) fn filename(label: &str, index: usize, format: ExportFormat) -> String {
    let mut stem = String::new();
    for c in label.chars() {
        let c = if c.is_alphanumeric() || matches!(c, '-' | '_') {
            c
        } else {
            '_'
        };
        if stem.len() + c.len_utf8() > 180 {
            break;
        }
        stem.push(c);
    }
    let extension = match format {
        ExportFormat::Csv | ExportFormat::CsvCustom(_) => "csv",
        ExportFormat::Json => "json",
        ExportFormat::Sql => "sql",
    };
    format!(
        "{:03}_{}.{extension}",
        index + 1,
        if stem.is_empty() { "table" } else { &stem }
    )
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn export(
    driver: &mut dyn Driver,
    directory: &std::path::Path,
    tables: &[(String, String)],
    format: ExportFormat,
    cancel: &AtomicBool,
) -> Result<std::path::PathBuf, String> {
    if tables.is_empty() {
        return Err("Select at least one table to export".into());
    }
    // A new owned directory prevents collisions with existing exports, database
    // files and aliases. It is removed on any error or cancellation.
    let staging = tempfile::Builder::new()
        .prefix("Rustable export ")
        .tempdir_in(directory)
        .map_err(|error| error.to_string())?;
    for (index, (name, label)) in tables.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("Export cancelled; no files saved".into());
        }
        let path = staging.path().join(filename(label, index, format));
        driver
            .export_table(
                &path,
                &Browse {
                    table: name.clone(),
                    ..Default::default()
                },
                cancel,
                format,
            )
            .map_err(|error| format!("Could not export {label}: {error}. No files saved."))?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("Export cancelled; no files saved".into());
    }
    Ok(staging.keep())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn names_are_portable_bounded_and_collision_free() {
        let names = ["../outside", "a/b", "a\\b", "A", "a", "", "日本"];
        let files: std::collections::HashSet<_> = names
            .iter()
            .enumerate()
            .map(|(index, name)| filename(name, index, ExportFormat::Csv))
            .collect();
        assert_eq!(files.len(), names.len());
        assert!(
            files
                .iter()
                .all(|name| std::path::Path::new(name).components().count() == 1)
        );
        assert!(filename(&"日本".repeat(100), 0, ExportFormat::Json).len() < 240);
    }

    #[test]
    fn batch_exports_every_row_and_cleans_up_on_failure_or_cancellation() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = sqlite::SqliteDriver::open(None, cancel.clone()).unwrap();
        assert!(driver.execute("CREATE TABLE export_many(id INTEGER PRIMARY KEY, name TEXT); WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1200) INSERT INTO export_many SELECT x, '日本' FROM n; CREATE TABLE export_empty(value TEXT);", false, &cancel).error.is_none());
        let directory = tempfile::tempdir().unwrap();
        let tables = vec![
            ("export_many".into(), "many".into()),
            ("export_empty".into(), "empty".into()),
        ];
        for format in [ExportFormat::Csv, ExportFormat::Json] {
            let path = export(&mut driver, directory.path(), &tables, format, &cancel).unwrap();
            let first = std::fs::read_to_string(path.join(filename("many", 0, format))).unwrap();
            let empty = std::fs::read_to_string(path.join(filename("empty", 1, format))).unwrap();
            match format {
                ExportFormat::Sql => unreachable!(),
                ExportFormat::Csv | ExportFormat::CsvCustom(_) => {
                    assert_eq!(first.lines().count(), 1201);
                    assert_eq!(empty.lines().count(), 1);
                }
                ExportFormat::Json => {
                    assert_eq!(
                        serde_json::from_str::<serde_json::Value>(&first)
                            .unwrap()
                            .as_array()
                            .unwrap()
                            .len(),
                        1200
                    );
                    assert_eq!(
                        serde_json::from_str::<serde_json::Value>(&empty).unwrap(),
                        serde_json::json!([])
                    );
                }
            }
        }
        let existing = std::fs::read_dir(directory.path()).unwrap().count();
        let bad = vec![
            tables[0].clone(),
            ("missing_table".into(), "missing".into()),
        ];
        assert!(
            export(
                &mut driver,
                directory.path(),
                &bad,
                ExportFormat::Csv,
                &cancel
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            existing
        );
        cancel.store(true, Ordering::Relaxed);
        assert!(
            export(
                &mut driver,
                directory.path(),
                &tables,
                ExportFormat::Csv,
                &cancel
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            existing
        );
        cancel.store(false, Ordering::Relaxed);
        assert!(driver.execute("BEGIN", false, &cancel).error.is_none());
        assert!(
            export(
                &mut driver,
                directory.path(),
                &tables,
                ExportFormat::Csv,
                &cancel
            )
            .is_err()
        );
        assert_eq!(driver.transaction_state(), TransactionState::Active);
        assert!(driver.execute("ROLLBACK", false, &cancel).error.is_none());
    }
    #[test]
    fn batch_command_dispatches_complete_exports() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut connection = None;
        handle(&mut connection, Command::Open(None), &cancel).unwrap();
        let objects = connection.as_mut().unwrap().objects().unwrap();
        let tables: Vec<_> = objects
            .iter()
            .filter(|object| object.kind == "table")
            .take(2)
            .map(|object| (object.name.clone(), object.display_name().to_owned()))
            .collect();
        assert_eq!(tables.len(), 2);
        let directory = tempfile::tempdir().unwrap();
        let result = handle(
            &mut connection,
            Command::ExportTables {
                directory: directory.path().to_owned(),
                tables,
                format: ExportFormat::Json,
            },
            &cancel,
        )
        .unwrap();
        let Payload::Exported(path) = result else {
            panic!("expected completed export")
        };
        assert_eq!(std::fs::read_dir(path).unwrap().count(), 2);
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_mysql_batch_export() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT")
            .unwrap()
            .parse()
            .unwrap();
        assert!([43316, 43317].contains(&port));
        let cancel = Arc::new(AtomicBool::new(false));
        let config = crate::connection::MysqlConfig {
            host: "127.0.0.1".into(),
            port,
            user: "rustable".into(),
            password: "rustable-test-only".into(),
            database: "rustable_test".into(),
            tls: false,
            ..Default::default()
        };
        let mut driver = mysql::open(&config, &cancel).unwrap();
        driver.set_cancellation(cancel.clone());
        let report = driver.execute("CREATE TEMPORARY TABLE batch_export_a(id INTEGER PRIMARY KEY, value TEXT); INSERT INTO batch_export_a VALUES(1,'日本'),(2,NULL); CREATE TEMPORARY TABLE batch_export_b(id INTEGER)",false,&cancel);
        assert!(report.error.is_none(), "{:?}", report.error);
        let destination = tempfile::tempdir().unwrap();
        let tables = vec![
            ("batch_export_a".into(), "a".into()),
            ("batch_export_b".into(), "b".into()),
        ];
        let schema_bundle = export_schema(
            driver.as_mut(),
            destination.path(),
            &tables,
            Some(ExportFormat::Csv),
            &cancel,
        )
        .unwrap();
        for (name, _) in &tables {
            let ddl = std::fs::read_to_string(schema_bundle.join("schema.sql")).unwrap();
            assert!(ddl.contains("CREATE"));
            assert!(ddl.contains(name));
        }
        let path = export(
            driver.as_mut(),
            destination.path(),
            &tables,
            ExportFormat::Json,
            &cancel,
        )
        .unwrap();
        let rows: serde_json::Value = serde_json::from_slice(
            &std::fs::read(path.join(filename("a", 0, ExportFormat::Json))).unwrap(),
        )
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 2);
        assert_eq!(rows[0]["value"], "日本");
        assert!(rows[1]["value"].is_null());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(
                &std::fs::read(path.join(filename("b", 1, ExportFormat::Json))).unwrap()
            )
            .unwrap(),
            serde_json::json!([])
        );
    }
}

/// One atomic SQL file, with all definitions before rows and deferred objects last.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn sql_bundle(
    driver: &mut dyn Driver,
    path: &std::path::Path,
    tables: &[(String, String)],
    schema: bool,
    data: bool,
    cancel: &AtomicBool,
) -> Result<std::path::PathBuf, String> {
    write_bundle(
        driver,
        path,
        tables,
        schema,
        data.then_some(ExportFormat::Sql),
        None,
        cancel,
    )
}
#[cfg(not(target_arch = "wasm32"))]
fn write_bundle(
    driver: &mut dyn Driver,
    path: &std::path::Path,
    tables: &[(String, String)],
    schema: bool,
    data_format: Option<ExportFormat>,
    directory: Option<&std::path::Path>,
    cancel: &AtomicBool,
) -> Result<std::path::PathBuf, String> {
    use std::io::{Seek, SeekFrom, Write};
    if tables.is_empty() || (!schema && data_format.is_none()) {
        return Err("Select tables and export contents".into());
    }
    if driver.transaction_state() != TransactionState::Idle {
        return Err("Finish the SQL transaction before exporting".into());
    }
    driver.validate_export_path(path)?;
    let engine = driver.engine_name();
    csv::publish(path,cancel,|output| {
        writeln!(output,"-- Rustable SQL export. External dependencies and permissions are not bundled.")?;
        if engine == "SQLite" { writeln!(output,"BEGIN;\nPRAGMA defer_foreign_keys = ON;")?; }
        if engine == "PostgreSQL" { writeln!(output,"BEGIN;")?; }
        if matches!(engine,"MySQL"|"MariaDB") {
            writeln!(output,"SET @rustable_bundle_sql_mode = @@SESSION.sql_mode;\nSET @rustable_bundle_fk_checks = @@SESSION.foreign_key_checks;\nSET SESSION foreign_key_checks = 0;")?;
        }
        let mut rows=tempfile::tempfile()?;
        let mut deferred=tempfile::tempfile()?;
        for (index,(name,label)) in tables.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) { return Err(std::io::Error::new(std::io::ErrorKind::Interrupted,"Export cancelled")); }
            if schema {
                let (before,after)=driver.export_definition_parts(name).map_err(std::io::Error::other)?;
                if before.trim().is_empty() { return Err(std::io::Error::other("No schema definition returned")); }
                writeln!(output,"{}{}",before.trim_end(),if before.trim_end().ends_with(';') { "" } else { ";" })?;
                writeln!(deferred,"{after}")?;
            }
            if let Some(format)=data_format {
                if let Some(directory)=directory {
                    driver.export_table(&directory.join(filename(label,index,format)),&Browse {table:name.clone(),..Default::default()},cancel,format).map_err(std::io::Error::other)?;
                } else {
                    // Close the placeholder handle before the driver atomically replaces
                    // its path. Windows denies replacement of an open destination.
                    let temporary=tempfile::NamedTempFile::new_in(path.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(std::path::Path::new(".")))?.into_temp_path();
                    driver.export_table(&temporary,&Browse { table:name.clone(),..Default::default() },cancel,format).map_err(std::io::Error::other)?;
                    copy_cancel(&mut std::fs::File::open(&temporary)?,&mut rows,cancel)?;
                }
            }
        }
        rows.seek(SeekFrom::Start(0))?; copy_cancel(&mut rows,output,cancel)?;
        deferred.seek(SeekFrom::Start(0))?; copy_cancel(&mut deferred,output,cancel)?;
        if matches!(engine,"SQLite"|"PostgreSQL") { writeln!(output,"COMMIT;")?; }
        if matches!(engine,"MySQL"|"MariaDB") { writeln!(output,"SET SESSION sql_mode = @rustable_bundle_sql_mode;\nSET SESSION foreign_key_checks = @rustable_bundle_fk_checks;")?; }
        Ok(())
    }).map_err(|e|e.to_string())?;
    Ok(path.to_path_buf())
}
#[cfg(not(target_arch = "wasm32"))]
fn copy_cancel(
    input: &mut impl std::io::Read,
    output: &mut impl std::io::Write,
    cancel: &AtomicBool,
) -> std::io::Result<()> {
    let mut buffer = [0; 64 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Export cancelled",
            ));
        }
        let count = input.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        output.write_all(&buffer[..count])?;
    }
}

/// Export selected object DDL, optionally alongside the existing row exports.
/// Dependencies outside the selection are deliberately not traversed.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn export_schema(
    driver: &mut dyn Driver,
    path: &std::path::Path,
    tables: &[(String, String)],
    data_format: Option<ExportFormat>,
    cancel: &AtomicBool,
) -> Result<std::path::PathBuf, String> {
    use std::io::Write;
    if tables.is_empty() {
        return Err("Select at least one table to export".into());
    }
    if driver.transaction_state() != TransactionState::Idle {
        return Err("Finish the SQL transaction before exporting schema".into());
    }
    if let Some(format) = data_format.filter(|format| *format != ExportFormat::Sql) {
        let directory = tempfile::Builder::new()
            .prefix("Rustable export ")
            .tempdir_in(path)
            .map_err(|e| e.to_string())?;
        write_bundle(
            driver,
            &directory.path().join("schema.sql"),
            tables,
            true,
            Some(format),
            Some(directory.path()),
            cancel,
        )?;
        if cancel.load(Ordering::Relaxed) {
            return Err("Export cancelled; no files saved".into());
        }
        return Ok(directory.keep());
    }
    let combined_sql = data_format == Some(ExportFormat::Sql);
    let single_file = tables.len() == 1 && (data_format.is_none() || combined_sql);
    if single_file {
        driver.validate_export_path(path)?;
    }
    let staging = if single_file {
        None
    } else {
        Some(
            tempfile::Builder::new()
                .prefix("Rustable export ")
                .tempdir_in(path)
                .map_err(|e| e.to_string())?,
        )
    };
    for (index, (name, label)) in tables.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("Export cancelled; no files saved".into());
        }
        let (ddl, after) = if combined_sql {
            driver.export_definition_parts(name)?
        } else {
            (driver.export_definition(name)?, String::new())
        };
        if ddl.trim().is_empty() {
            return Err(format!("No schema definition is available for {label}"));
        }
        let destination = staging.as_ref().map_or_else(
            || path.to_path_buf(),
            |dir| {
                dir.path().join(
                    std::path::Path::new(&filename(label, index, ExportFormat::Csv))
                        .with_extension("sql"),
                )
            },
        );
        // Stage the streamed INSERTs separately. Only the complete combined SQL
        // is published, so cancellation never replaces an existing destination.
        let data = if combined_sql {
            // Keep cleanup ownership without holding the destination open during rename.
            let temporary = tempfile::NamedTempFile::new_in(destination.parent().unwrap())
                .map_err(|e| e.to_string())?.into_temp_path();
            driver.export_table(
                &temporary,
                &Browse {
                    table: name.clone(),
                    ..Default::default()
                },
                cancel,
                ExportFormat::Sql,
            )?;
            Some(temporary)
        } else {
            None
        };
        csv::publish(&destination, cancel, |writer| {
            writer.write_all(b"-- Rustable selected-object schema export.\n-- Restore dependencies first. This is not a complete database backup.\n\n")?;
            writer.write_all(ddl.trim_end().as_bytes())?;
            if !ddl.trim_end().ends_with(';') { writer.write_all(b";")?; }
            writer.write_all(b"\n")?;
            if let Some(data) = &data {
                let mut input = std::fs::File::open(data)?;
                let mut buffer = [0; 64 * 1024];
                loop {
                    use std::io::Read;
                    if cancel.load(Ordering::Relaxed) { return Err(std::io::Error::new(std::io::ErrorKind::Interrupted,"Export cancelled")); }
                    let count = input.read(&mut buffer)?;
                    if count == 0 { break; }
                    writer.write_all(&buffer[..count])?;
                }
                writer.write_all(after.as_bytes())?;
            }
            Ok(())
        }).map_err(|e| e.to_string())?;
        if let Some(format) = data_format.filter(|_| !combined_sql) {
            let file = staging
                .as_ref()
                .unwrap()
                .path()
                .join(filename(label, index, format));
            driver.export_table(
                &file,
                &Browse {
                    table: name.clone(),
                    ..Default::default()
                },
                cancel,
                format,
            )?;
        }
    }
    if !single_file && cancel.load(Ordering::Relaxed) {
        return Err("Export cancelled; no files saved".into());
    }
    Ok(staging.map_or_else(|| path.to_path_buf(), |dir| dir.keep()))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod schema_tests {
    use super::*;
    #[test]
    fn schema_roundtrip_retains_constraints_indexes_and_triggers() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut source = sqlite::SqliteDriver::open(None, cancel.clone()).unwrap();
        assert!(source.execute("CREATE TABLE schema_export(id INTEGER PRIMARY KEY, value TEXT NOT NULL CHECK(length(value)>0)); CREATE UNIQUE INDEX schema_export_value ON schema_export(value); CREATE TRIGGER schema_export_trim AFTER INSERT ON schema_export BEGIN UPDATE schema_export SET value=trim(new.value) WHERE id=new.id; END; INSERT INTO schema_export VALUES(1,' alpha ');", false, &cancel).error.is_none());
        let dir = tempfile::tempdir().unwrap();
        let tables = vec![("schema_export".into(), "schema_export".into())];
        let path = dir.path().join("schema.sql");
        export_schema(&mut source, &path, &tables, None, &cancel).unwrap();
        let ddl = std::fs::read_to_string(&path).unwrap();
        let target = rusqlite::Connection::open_in_memory().unwrap();
        target.execute_batch(&ddl).unwrap();
        assert_eq!(
            target
                .query_row("SELECT count(*) FROM schema_export", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        target
            .execute("INSERT INTO schema_export VALUES (1,' beta ')", [])
            .unwrap();
        assert_eq!(
            target
                .query_row("SELECT value FROM schema_export", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "beta"
        );
        assert!(
            target
                .execute("INSERT INTO schema_export VALUES (2,'beta')", [])
                .is_err()
        );
        for format in [ExportFormat::Csv, ExportFormat::Json] {
            let bundle =
                export_schema(&mut source, dir.path(), &tables, Some(format), &cancel).unwrap();
            assert!(bundle.join("schema.sql").is_file());
            let data =
                std::fs::read_to_string(bundle.join(filename("schema_export", 0, format))).unwrap();
            assert!(data.contains("alpha"));
        }
        let before = std::fs::read_dir(dir.path()).unwrap().count();
        let missing = vec![tables[0].clone(), ("missing".into(), "missing".into())];
        assert!(export_schema(&mut source, dir.path(), &missing, None, &cancel).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), before);
        cancel.store(true, Ordering::Relaxed);
        assert!(export_schema(&mut source, &path, &tables, None, &cancel).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), ddl);
        cancel.store(false, Ordering::Relaxed);
        assert!(source.execute("BEGIN", false, &cancel).error.is_none());
        assert!(export_schema(&mut source, &path, &tables, None, &cancel).is_err());
        assert_eq!(source.transaction_state(), TransactionState::Active);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod sql_tests {
    use super::*;
    #[test]
    fn sqlite_combined_sql_roundtrip_generated_triggers_and_literals() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut source = sqlite::SqliteDriver::open(None, cancel.clone()).unwrap();
        let setup = "CREATE TABLE \"odd\"\"table\"(id INTEGER PRIMARY KEY AUTOINCREMENT, value TEXT, bytes BLOB, missing TEXT, n REAL, derived INTEGER GENERATED ALWAYS AS (id*2) STORED); CREATE TRIGGER bump AFTER INSERT ON \"odd\"\"table\" BEGIN UPDATE \"odd\"\"table\" SET n=n+1 WHERE id=new.id; END; INSERT INTO \"odd\"\"table\"(id,value,bytes,missing,n) VALUES(5,'日本 ''quote''; DROP TABLE nope; ' || char(0) || 'tail',X'00FF',NULL,1.25);";
        assert!(source.execute(setup, false, &cancel).error.is_none());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("combined.sql");
        let tables = vec![("odd\"table".into(), "odd".into())];
        export_schema(
            &mut source,
            &path,
            &tables,
            Some(ExportFormat::Sql),
            &cancel,
        )
        .unwrap();
        let script = std::fs::read_to_string(&path).unwrap();
        let restored = rusqlite::Connection::open_in_memory().unwrap();
        restored.execute_batch(&script).unwrap();
        let row = restored
            .query_row(
                "SELECT id,value,hex(bytes),missing,n,derived FROM \"odd\"\"table\"",
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, f64>(4)?,
                        r.get::<_, i64>(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            row,
            (
                5,
                "日本 'quote'; DROP TABLE nope; \0tail".into(),
                "00FF".into(),
                None,
                2.25,
                10
            )
        );
        restored
            .execute("INSERT INTO \"odd\"\"table\"(n) VALUES(4)", [])
            .unwrap();
        assert_eq!(
            restored
                .query_row("SELECT n FROM \"odd\"\"table\" WHERE id=6", [], |r| r
                    .get::<_, f64>(0))
                .unwrap(),
            5.0
        );
        cancel.store(true, Ordering::Relaxed);
        assert!(
            export_schema(
                &mut source,
                &path,
                &tables,
                Some(ExportFormat::Sql),
                &cancel
            )
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), script);
        cancel.store(false, Ordering::Relaxed);
        assert!(source.execute("CREATE TABLE empty_sql(id INTEGER); CREATE TABLE many_sql(id INTEGER); WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1200) INSERT INTO many_sql SELECT x FROM n;",false,&cancel).error.is_none());
        let bulk = vec![
            ("empty_sql".into(), "empty".into()),
            ("many_sql".into(), "many".into()),
        ];
        let folder = export_schema(
            &mut source,
            dir.path(),
            &bulk,
            Some(ExportFormat::Sql),
            &cancel,
        )
        .unwrap();
        for file in ["001_empty.sql", "002_many.sql"] {
            restored
                .execute_batch(&std::fs::read_to_string(folder.join(file)).unwrap())
                .unwrap();
        }
        assert_eq!(
            restored
                .query_row("SELECT count(*) FROM empty_sql", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            restored
                .query_row("SELECT count(*) FROM many_sql", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1200
        );
        let before = std::fs::read_dir(dir.path()).unwrap().count();
        assert!(
            export_schema(
                &mut source,
                dir.path(),
                &[
                    ("many_sql".into(), "many".into()),
                    ("missing_sql".into(), "missing".into())
                ],
                Some(ExportFormat::Sql),
                &cancel
            )
            .is_err()
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), before);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod bundle_tests {
    use super::*;
    #[test]
    fn one_sql_file_restores_multiple_tables_including_foreign_keys_and_empty_tables() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = sqlite::SqliteDriver::open(None, cancel.clone()).unwrap();
        assert!(driver.execute("CREATE TABLE parent(id INTEGER PRIMARY KEY); CREATE TABLE child(id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES parent(id)); CREATE TABLE empty_table(id INTEGER); INSERT INTO parent VALUES(1); INSERT INTO child VALUES(2,1);",false,&cancel).error.is_none());
        let tables = vec![
            ("child".into(), "child".into()),
            ("empty_table".into(), "empty".into()),
            ("parent".into(), "parent".into()),
        ];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tables.sql");
        sql_bundle(&mut driver, &path, &tables, true, true, &cancel).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        let script = std::fs::read_to_string(&path).unwrap();
        let target = rusqlite::Connection::open_in_memory().unwrap();
        target.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        target.execute_batch(&script).unwrap();
        assert_eq!(
            target
                .query_row("SELECT parent_id FROM child", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(target.execute("INSERT INTO child VALUES(3,9)", []).is_err());
        assert_eq!(
            target
                .query_row("SELECT count(*) FROM empty_table", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let bad = vec![
            tables[0].clone(),
            ("missing_table".into(), "missing".into()),
        ];
        assert!(sql_bundle(&mut driver, &path, &bad, true, true, &cancel).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), script);
        cancel.store(true, Ordering::Relaxed);
        assert!(sql_bundle(&mut driver, &path, &tables, true, true, &cancel).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), script);
        cancel.store(false, Ordering::Relaxed);
        sql_bundle(&mut driver, &path, &tables, true, false, &cancel).unwrap();
        let empty = rusqlite::Connection::open_in_memory().unwrap();
        empty
            .execute_batch(&std::fs::read_to_string(&path).unwrap())
            .unwrap();
        assert_eq!(
            empty
                .query_row("SELECT count(*) FROM child", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        sql_bundle(&mut driver, &path, &tables, false, true, &cancel).unwrap();
        empty
            .execute_batch(&std::fs::read_to_string(&path).unwrap())
            .unwrap();
        assert_eq!(
            empty
                .query_row("SELECT count(*) FROM child", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
