//! Stream a full matching result to the shared atomic file publisher.
use super::*;
use std::{io, path::Path};

pub(super) fn table(driver: &mut MysqlDriver, path: &Path, browse: &Browse, cancel: &AtomicBool, format: ExportFormat) -> Result<(), String> {
    driver.require_idle()?;
    let (_, sql, parameters) = driver.table_select(browse)?;
    let parameters = parameters.iter().map(::mysql::Value::try_from).collect::<Result<Vec<_>, _>>()?;
    let config = driver.config.clone();
    let destination = driver.destination;
    let conn = driver.conn()?;
    let mut disconnected = false;
    let (result, cancellation_error) = with_cancel(&config, destination, conn.connection_id(), cancel, || {
        csv::publish(path, cancel, |output| {
            let mut results = conn.exec_iter(&sql, parameters).map_err(|error| {
                disconnected |= error.is_connectivity_error();
                io::Error::other(error_message(&error))
            })?;
            let columns = results.columns().as_ref().to_vec();
            let names = columns.iter().map(|column| column.name_str().into_owned()).collect::<Vec<_>>();
            let result = (|| {
                let mut writer = row_export::Writer::new(output, &names, cancel, format)?;
                for row in results.by_ref() {
                    let row = row.map_err(|error| {
                        disconnected |= error.is_connectivity_error();
                        io::Error::other(error_message(&error))
                    })?;
                    // One decoded row at a time, independent of table size.
                    let values = columns.iter().zip(row.unwrap()).map(Value::try_from)
                        .collect::<Result<Vec<_>, _>>().map_err(io::Error::other)?;
                    writer.row(values.iter().map(|value| Ok(csv::Cell::from(value))))?;
                }
                writer.finish()
            })();
            // QueryResult drains on drop. Stop the server if encoding fails so
            // a disk error does not silently consume an entire large export.
            if result.is_err() { cancel.store(true, Ordering::Release); }
            result
        })
    });
    // A failure while draining QueryResult during drop is not yielded to the
    // row loop. Verify synchronization before retaining an errored session.
    if result.is_err() && conn.ping().is_err() { disconnected = true; }
    if disconnected { driver.connection = None; }
    match (result, cancellation_error) {
        (Ok(()), None) => Ok(()),
        (Err(error), None) => Err(error.to_string()),
        (Ok(()), Some(error)) => Err(error),
        (Err(error), Some(cancel_error)) => Err(format!("{error}\n{cancel_error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_full_export_streams_filters_and_cancellation_preserves_destination() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = MysqlConfig { host:"127.0.0.1".into(), port, user:"rustable".into(), password:"rustable-test-only".into(), database:"rustable_test".into(), tls:false, ..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        driver.set_cancellation(cancel.clone());
        driver.conn().unwrap().query_drop("DROP VIEW IF EXISTS slow_export_probe").unwrap();
        driver.conn().unwrap().query_drop("DROP TABLE IF EXISTS full_export_probe").unwrap();
        driver.conn().unwrap().query_drop("CREATE TABLE full_export_probe(id BIGINT PRIMARY KEY, label VARCHAR(80), data BLOB, nullable INT) ENGINE=InnoDB").unwrap();
        let rows = (1..=1200).map(|id| format!("({id},'日本 100%',X'00FF',NULL)")).collect::<Vec<_>>().join(",");
        driver.conn().unwrap().query_drop(format!("INSERT INTO full_export_probe VALUES {rows}")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rows.json");
        let browse = Browse { filters: Vec::new(), limit: 0, table:"full_export_probe".into(), offset:500, filter_column:Some("label".into()), filter:"100%".into(), sort:Some(("id".into(),true)) };
        driver.export_table(&path, &browse, &cancel, ExportFormat::Json).unwrap();
        let rows: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 1200);
        assert_eq!(rows[0]["id"], 1200);
        assert_eq!(rows[1199]["id"], 1);
        assert_eq!(rows[0]["label"], "日本 100%");
        assert_eq!(rows[0]["data"], serde_json::json!([0,255]));
        assert!(rows[0]["nullable"].is_null());
        driver.export_table(&path, &browse, &cancel, ExportFormat::Csv).unwrap();
        let csv = std::fs::read_to_string(&path).unwrap();
        assert_eq!(csv.lines().count(), 1201);
        assert!(csv.lines().nth(1).unwrap().starts_with("\"1200\","));
        let mut empty = browse.clone(); empty.filter = "100_".into();
        driver.export_table(&path, &empty, &cancel, ExportFormat::Json).unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap(), serde_json::json!([]));
        std::fs::write(&path, "preserve me").unwrap();
        driver.conn().unwrap().query_drop("START TRANSACTION").unwrap();
        assert!(driver.export_table(&path, &browse, &cancel, ExportFormat::Json).is_err());
        driver.conn().unwrap().query_drop("ROLLBACK").unwrap();
        cancel.store(true, Ordering::Release);
        assert!(driver.export_table(&path, &browse, &cancel, ExportFormat::Json).is_err());
        cancel.store(false, Ordering::Release);
        driver.conn().unwrap().query_drop("CREATE VIEW slow_export_probe AS SELECT id,SLEEP(0.01) AS delay FROM full_export_probe").unwrap();
        let stop = cancel.clone();
        let signal = std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(50)); stop.store(true, Ordering::Release); });
        let started = std::time::Instant::now();
        let result = driver.export_table(&path, &Browse { table:"slow_export_probe".into(), ..Default::default() }, &cancel, ExportFormat::Json);
        signal.join().unwrap();
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "preserve me");
        assert_eq!(directory.path().read_dir().unwrap().count(),1);
        cancel.store(false, Ordering::Release);
        assert_eq!(driver.conn().unwrap().query_first::<u8,_>("SELECT 42").unwrap(), Some(42));
        driver.conn().unwrap().query_drop("DROP VIEW slow_export_probe").unwrap();
        driver.conn().unwrap().query_drop("DROP TABLE full_export_probe").unwrap();
    }
}
