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
        ExportFormat::Csv => "csv",
        ExportFormat::Json => "json",
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
                ExportFormat::Csv => {
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
