//! Stream bounded INSERT batches inside one transaction. UI remains gated.
use super::*;

pub(super) fn table(driver: &mut MysqlDriver, request: &csv_import::Request, cancel: &AtomicBool) -> Result<u64, String> {
    request.sql()?; // Shared mapping validation (generated/duplicate/missing columns).
    driver.require_idle()?;
    if cancel.load(Ordering::Acquire) { return Err("CSV import cancelled".into()); }
    let schema = driver.schemas.selected.clone().ok_or("Choose a database first")?;
    let qualified = format!("{}.{}", quote(&schema), quote(&request.table));
    let fields = request.destinations.iter().flatten().map(|i| quote(&request.columns[*i].name)).collect::<Vec<_>>();
    let checks = if driver.engine == "MariaDB" {
        "SELECT @@SESSION.foreign_key_checks, @@SESSION.unique_checks, @@SESSION.check_constraint_checks"
    } else { "SELECT @@SESSION.foreign_key_checks, @@SESSION.unique_checks, 1" };
    if driver.with_connection(|conn| conn.query_first::<(u8,u8,u8),_>(checks))? != Some((1,1,1)) {
        return Err("Restore foreign key, unique and CHECK enforcement before importing.".into());
    }
    let result = (|| {
        driver.with_connection(|conn| conn.query_drop("START TRANSACTION"))?;
        // Hold metadata through validation and every batch, including commit.
        driver.with_connection(|conn| conn.query_drop(format!("SELECT * FROM {qualified} LIMIT 0")))?;
        driver.edit_schema_snapshot(&schema, &request.table)?; // Reject temporary replacements.
        editing::require_trigger_visibility(driver, &schema, &request.table)?;
        let engine = driver.with_connection(|conn| conn.exec_first::<String,_,_>(
            "SELECT ENGINE FROM information_schema.TABLES WHERE TABLE_SCHEMA=? AND TABLE_NAME=?", (&schema,&request.table)))?;
        if engine.as_deref() != Some("InnoDB") { return Err("CSV import requires an InnoDB base table.".into()); }
        let triggers = driver.with_connection(|conn| conn.exec_first::<u64,_,_>(
            "SELECT COUNT(*) FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA=? AND EVENT_OBJECT_TABLE=?", (&schema,&request.table)))?;
        if triggers != Some(0) { return Err("CSV import for tables with triggers is not supported yet.".into()); }
        let object = driver.catalog_objects(Some(&request.table))?.into_iter().next().ok_or("Missing destination table")?;
        if object.kind != "table" || object.columns != request.columns {
            return Err("Destination schema changed; select the table again before importing".into());
        }
        let mut parameters = Vec::new();
        let (mut rows, mut bytes, mut inserted) = (0usize, 0usize, 0u64);
        // Stay below protocol parameter limits even for very wide CSVs.
        let batch_limit = 64usize.min(60_000 / fields.len()).max(1);
        let mut flush = |parameters: &mut Vec<::mysql::Value>, rows: &mut usize| -> Result<(), String> {
            if *rows == 0 { return Ok(()); }
            if cancel.load(Ordering::Acquire) { return Err("CSV import cancelled".into()); }
            let tuple = format!("({})", vec!["?"; fields.len()].join(","));
            let sql = format!("INSERT INTO {qualified} ({}) VALUES {}", fields.join(","), vec![tuple; *rows].join(","));
            let (affected, warnings) = driver.with_connection(|conn| {
                conn.exec_drop(sql, std::mem::take(parameters))?;
                Ok((conn.affected_rows(), conn.warnings()))
            })?;
            if warnings != 0 || affected != *rows as u64 {
                return Err("Import produced a conversion warning or unexpected row count; no rows were saved.".into());
            }
            inserted += affected;
            *rows = 0;
            Ok(())
        };
        let count = csv_preview::visit(&request.source, request.options, request.destinations.len(), cancel, |record| {
            let row_bytes = record.iter().map(str::len).sum::<usize>();
            if rows >= batch_limit || (rows > 0 && bytes.saturating_add(row_bytes) > 1024 * 1024) {
                flush(&mut parameters, &mut rows)?;
                bytes = 0;
            }
            parameters.extend(request.destinations.iter().enumerate().filter_map(|(source, target)| target.map(|_| {
                if request.empty_as_null && record[source].is_empty() { ::mysql::Value::NULL }
                else { ::mysql::Value::Bytes(record[source].as_bytes().to_vec()) }
            })));
            rows += 1;
            bytes = bytes.saturating_add(row_bytes);
            Ok(())
        })?;
        flush(&mut parameters, &mut rows)?;
        if cancel.load(Ordering::Acquire) { return Err("CSV import cancelled".into()); }
        if count != inserted { return Err("Import row count did not match source".into()); }
        Ok(count)
    })();
    match result {
        Ok(count) => {
            if let Err(error) = driver.conn()?.query_drop("COMMIT") {
                driver.connection = None;
                return Err(format!("Import commit outcome is unknown: {error}. Reconnect and verify data before retrying."));
            }
            Ok(count)
        }
        Err(error) => {
            if let Err(rollback) = driver.conn().and_then(|conn| conn.query_drop("ROLLBACK").map_err(|e| e.to_string())) {
                driver.connection = None;
                return Err(format!("{error}\nImport rollback could not be confirmed: {rollback}. Reconnect and verify data."));
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(driver: &mut MysqlDriver, text: &str) -> csv_import::Request {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), text).unwrap();
        let preview = csv_preview::load(file.path(), csv_preview::Options::default(), None, &driver.cancel).unwrap();
        csv_import::Request { source:preview.source.unwrap(), options:preview.options,
            table:"mysql_csv_probe".into(), columns:driver.catalog_objects(Some("mysql_csv_probe")).unwrap().remove(0).columns,
            destinations:vec![Some(1),Some(0)], empty_as_null:true, postgres_inserts:false }
    }
    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_csv_batches_are_atomic_and_validate_destination() {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316,43317].contains(&port));
        let config = MysqlConfig { host:"127.0.0.1".into(),port,user:"rustable".into(),password:"rustable-test-only".into(),database:"rustable_test".into(),tls:false,..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        let cancel = AtomicBool::new(false);
        driver.conn().unwrap().query_drop("DROP TABLE IF EXISTS mysql_csv_probe").unwrap();
        driver.conn().unwrap().query_drop("CREATE TABLE mysql_csv_probe(id BIGINT PRIMARY KEY, label VARCHAR(20), extra INT DEFAULT 7, doubled BIGINT GENERATED ALWAYS AS (id*2) STORED) ENGINE=InnoDB").unwrap();
        let mut text = String::from("label,id\n");
        for id in 1..=130 { text.push_str(&format!("日本,{id}\n")); }
        let good = request(&mut driver, &text);
        assert_eq!(driver.import_csv(&good,&cancel).unwrap(),130);
        assert_eq!(driver.conn().unwrap().query_first::<(String,i32,i64),_>("SELECT label,extra,doubled FROM mysql_csv_probe WHERE id=130").unwrap(),Some(("日本".into(),7,260)));
        let null = request(&mut driver,"label,id\n,9007199254740993\n");
        assert_eq!(driver.import_csv(&null,&cancel).unwrap(),1);
        assert_eq!(driver.conn().unwrap().query_first::<u8,_>("SELECT label IS NULL FROM mysql_csv_probe WHERE id=9007199254740993").unwrap(),Some(1));
        // Failure after a complete earlier batch must roll back that batch too.
        let mut text = String::from("label,id\n");
        for id in 200..270 { text.push_str(&format!("new,{id}\n")); }
        text.push_str("duplicate,1\n");
        let bad = request(&mut driver,&text);
        assert!(driver.import_csv(&bad,&cancel).is_err());
        assert_eq!(driver.conn().unwrap().query_first::<u64,_>("SELECT COUNT(*) FROM mysql_csv_probe").unwrap(),Some(131));
        assert_eq!(driver.transaction_state(),TransactionState::Idle);
        let stale = request(&mut driver,"label,id\nnew,500\n");
        driver.conn().unwrap().query_drop("ALTER TABLE mysql_csv_probe ADD newer INT").unwrap();
        assert!(driver.import_csv(&stale,&cancel).unwrap_err().contains("schema changed"));
        let fresh = request(&mut driver,"label,id\nnew,500\n");
        cancel.store(true,Ordering::Release);
        assert!(driver.import_csv(&fresh,&cancel).unwrap_err().contains("cancelled"));
        cancel.store(false,Ordering::Release);
        driver.conn().unwrap().query_drop("SET sql_mode=''").unwrap();
        let warning = request(&mut driver,"label,id\nthis text exceeds twenty characters,501\n");
        assert!(driver.import_csv(&warning,&cancel).unwrap_err().contains("warning"));
        assert_eq!(driver.conn().unwrap().query_first::<u64,_>("SELECT COUNT(*) FROM mysql_csv_probe WHERE id>=500 AND id<1000").unwrap(),Some(0));
        assert_eq!(driver.import_csv(&fresh,&cancel).unwrap(),1);
        driver.conn().unwrap().query_drop("START TRANSACTION").unwrap();
        assert!(driver.import_csv(&fresh,&cancel).unwrap_err().contains("transaction"));
        driver.conn().unwrap().query_drop("ROLLBACK").unwrap();
        driver.conn().unwrap().query_drop("DROP TABLE mysql_csv_probe").unwrap();
    }
}
