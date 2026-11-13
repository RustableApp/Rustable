//! Bounded COPY import: one transaction, fresh schema, no per-row round trips.
use super::*;
use std::io::Write;

pub(super) fn import_csv(
    driver: &mut PostgresDriver,
    request: &csv_import::Request,
    cancel: &AtomicBool,
) -> Result<u64, String> {
    request.postgres_sql()?;
    driver.require_idle_transaction()?;
    driver.check_tunnel()?;
    let watch = driver.watch();
    let mut tx = driver.client.transaction().map_err(error)?;
    let result = (|| {
        let relation = tx.query_opt(
            "SELECT n.nspname,c.relname FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE c.oid=pg_catalog.to_regclass($1) AND c.relkind IN ('r','p')",
            &[&request.table],
        ).map_err(error)?.ok_or("Import destination is not an existing table")?;
        let table = format!("{}.{}", quote(relation.get(0)), quote(relation.get(1)));
        // Parse an INSERT to hold RowExclusiveLock through this transaction.
        // Explicit LOCK requires table-wide grants, whereas imports may have
        // only column INSERT grants. Never execute this statement: defaults,
        // sequences and triggers must run only for actual imported rows.
        let _lock = lock_destination(&mut tx, &table)?;
        let columns = tx.query(
            "SELECT a.attname,pg_catalog.format_type(a.atttypid,a.atttypmod),a.attnotnull,COALESCE(pg_catalog.pg_get_expr(d.adbin,d.adrelid),''),EXISTS(SELECT 1 FROM pg_catalog.pg_index i WHERE i.indrelid=a.attrelid AND i.indisprimary AND a.attnum=ANY(i.indkey)),(a.attgenerated<>'' OR a.attidentity<>'') FROM pg_catalog.pg_attribute a LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum WHERE a.attrelid=pg_catalog.to_regclass($1) AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum",
            &[&table],
        ).map_err(error)?.into_iter().map(|r| Column {
            name:r.get(0), data_type:r.get(1), not_null:r.get(2), default:r.get(3), pk:r.get(4), generated:r.get(5)
        }).collect::<Vec<_>>();
        if columns != request.columns {
            return Err(
                "Destination schema changed; select the table again before importing".into(),
            );
        }
        let mut canonical = request.clone();
        canonical.table = table;
        let sql = canonical.postgres_sql()?;
        if canonical.postgres_inserts {
            return insert_batches(&mut tx, &canonical, &sql, cancel);
        }
        let mut writer = tx.copy_in(&sql).map_err(error)?;
        let mut encoded = Vec::new();
        let count = csv_preview::visit(
            &request.source,
            request.options,
            request.destinations.len(),
            cancel,
            |record| {
                encoded.clear();
                for (position, (source, _)) in request
                    .destinations
                    .iter()
                    .enumerate()
                    .filter(|(_, destination)| destination.is_some())
                    .enumerate()
                {
                    if position > 0 {
                        encoded.push(b'\t');
                    }
                    encode_field(&mut encoded, &record[source], request.empty_as_null);
                }
                encoded.push(b'\n');
                writer.write_all(&encoded).map_err(|e| e.to_string())
            },
        )?;
        if cancel.load(Ordering::Relaxed) {
            return Err("CSV import cancelled".into());
        }
        let written = writer.finish().map_err(error)?;
        if written != count {
            return Err("Import row count did not match source".into());
        }
        Ok(count)
    })();
    drop(watch);
    match result {
        Ok(count) if !cancel.load(Ordering::Relaxed) => {
            tx.commit().map_err(error)?;
            driver.browse_catalog.clear();
            Ok(count)
        }
        result => {
            cancel.store(false, Ordering::Relaxed);
            tx.rollback().map_err(error)?;
            Err(result
                .err()
                .unwrap_or_else(|| "CSV import cancelled".into()))
        }
    }
}

fn lock_destination(
    tx: &mut ::postgres::Transaction<'_>,
    table: &str,
) -> Result<::postgres::Statement, String> {
    tx.prepare(&format!("INSERT INTO {table} DEFAULT VALUES"))
        .map_err(error)
}

fn insert_batches(
    tx: &mut ::postgres::Transaction<'_>,
    request: &csv_import::Request,
    sql: &str,
    cancel: &AtomicBool,
) -> Result<u64, String> {
    let statement = tx.prepare(sql).map_err(error)?;
    let mut batch = Vec::new();
    let mut bytes = 0usize;
    let mut inserted = 0u64;
    let mut flush = |batch: &mut Vec<serde_json::Value>| -> Result<(), String> {
        if batch.is_empty() {
            return Ok(());
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("CSV import cancelled".into());
        }
        let payload = serde_json::to_string(batch).map_err(|e| e.to_string())?;
        let count = tx.execute(&statement, &[&payload]).map_err(error)?;
        if count != batch.len() as u64 {
            return Err("Import row count did not match source".into());
        }
        inserted += count;
        batch.clear();
        Ok(())
    };
    let count = csv_preview::visit(
        &request.source,
        request.options,
        request.destinations.len(),
        cancel,
        |record| {
            let row_bytes = record.iter().map(str::len).sum::<usize>();
            if batch.len() >= 64 || (!batch.is_empty() && bytes + row_bytes > 1024 * 1024) {
                flush(&mut batch)?;
                bytes = 0;
            }
            let mut row = serde_json::Map::new();
            for (source, destination) in request.destinations.iter().enumerate() {
                if let Some(destination) = destination {
                    let value = if request.empty_as_null && record[source].is_empty() {
                        serde_json::Value::Null
                    } else {
                        serde_json::Value::String(record[source].to_owned())
                    };
                    row.insert(request.columns[*destination].name.clone(), value);
                }
            }
            batch.push(serde_json::Value::Object(row));
            bytes += row_bytes;
            Ok(())
        },
    )?;
    flush(&mut batch)?;
    if inserted != count {
        return Err("Import row count did not match source".into());
    }
    Ok(count)
}

fn encode_field(output: &mut Vec<u8>, value: &str, empty_as_null: bool) {
    if empty_as_null && value.is_empty() {
        output.extend_from_slice(b"\\N");
        return;
    }
    for byte in value.bytes() {
        match byte {
            b'\\' => output.extend_from_slice(b"\\\\"),
            b'\t' => output.extend_from_slice(b"\\t"),
            b'\n' => output.extend_from_slice(b"\\n"),
            b'\r' => output.extend_from_slice(b"\\r"),
            _ => output.push(byte),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(driver: &mut PostgresDriver, text: &str) -> csv_import::Request {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), text).unwrap();
        let preview = csv_preview::load(
            file.path(),
            csv_preview::Options::default(),
            None,
            &driver.cancel,
        )
        .unwrap();
        csv_import::Request {
            schema: None,
            source: preview.source.unwrap(),
            options: preview.options,
            table: "pg_temp.import_qa".into(),
            columns: driver.columns("pg_temp.import_qa").unwrap(),
            destinations: vec![Some(1), Some(0)],
            empty_as_null: false,
            postgres_inserts: false,
        }
    }

    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn copy_import_streams_and_rolls_back_failures() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = PostgresDriver::open(
            &crate::connection::PostgresConfig {
                host: "127.0.0.1".into(),
                port: 54329,
                database: "rustable_qa".into(),
                user: "postgres".into(),
                password: "rustable_qa_only".into(),
                tls: crate::connection::PostgresTlsMode::Disable,
                ssh: None,
            },
            cancel.clone(),
        )
        .unwrap();
        driver.client.batch_execute("CREATE TEMP TABLE import_qa(id numeric PRIMARY KEY, value text CHECK(value <> 'bad'), extra text DEFAULT 'default')").unwrap();
        let mut text = String::from("value,id\n");
        for i in 1..=6001 {
            text.push_str(&format!("value{i},{i}\n"));
        }
        let good = request(&mut driver, &text);
        assert_eq!(driver.import_csv(&good, &cancel).unwrap(), 6001);
        let special = request(
            &mut driver,
            "value,id\n,9007199254740993\n\\N,6003\n\"hello\t世界\nline\",6004\n",
        );
        assert_eq!(driver.import_csv(&special, &cancel).unwrap(), 3);
        let row = driver
            .client
            .query_one(
                "SELECT value,extra FROM import_qa WHERE id=9007199254740993",
                &[],
            )
            .unwrap();
        assert_eq!(row.get::<_, String>(0), "");
        assert_eq!(row.get::<_, String>(1), "default");
        assert_eq!(
            driver
                .client
                .query_one("SELECT value FROM import_qa WHERE id=6003", &[])
                .unwrap()
                .get::<_, String>(0),
            "\\N"
        );
        assert_eq!(
            driver
                .client
                .query_one("SELECT value FROM import_qa WHERE id=6004", &[])
                .unwrap()
                .get::<_, String>(0),
            "hello\t世界\nline"
        );
        let mut null = request(&mut driver, "value,id\n,6005\n");
        null.empty_as_null = true;
        driver.import_csv(&null, &cancel).unwrap();
        assert!(
            driver
                .client
                .query_one("SELECT value IS NULL FROM import_qa WHERE id=6005", &[])
                .unwrap()
                .get::<_, bool>(0)
        );
        let bad = request(&mut driver, "value,id\nokay,7000\nbad,7001\n");
        assert!(driver.import_csv(&bad, &cancel).is_err());
        assert_eq!(
            driver
                .client
                .query_one(
                    "SELECT count(*) FROM import_qa WHERE id IN (7000,7001)",
                    &[]
                )
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        let stale = request(&mut driver, "value,id\nokay,7000\n");
        driver
            .client
            .batch_execute("ALTER TABLE import_qa ADD COLUMN later integer")
            .unwrap();
        assert!(
            driver
                .import_csv(&stale, &cancel)
                .unwrap_err()
                .contains("schema changed")
        );
        let fresh = request(&mut driver, "value,id\nokay,7000\n");
        driver.client.batch_execute("BEGIN").unwrap();
        assert!(
            driver
                .import_csv(&fresh, &cancel)
                .unwrap_err()
                .contains("transaction")
        );
        driver.client.batch_execute("ROLLBACK").unwrap();
        cancel.store(true, Ordering::Relaxed);
        assert!(driver.import_csv(&fresh, &cancel).is_err());
        cancel.store(false, Ordering::Relaxed);
        assert_eq!(driver.import_csv(&fresh, &cancel).unwrap(), 1);
        let mut inserts = request(
            &mut driver,
            "value,id\n\"hello\t世界\nline\",9007199254740995\n,9007199254740996\n",
        );
        inserts.postgres_inserts = true;
        assert_eq!(driver.import_csv(&inserts, &cancel).unwrap(), 2);
        assert_eq!(
            driver
                .client
                .query_one(
                    "SELECT value,extra FROM import_qa WHERE id=9007199254740995",
                    &[]
                )
                .unwrap()
                .get::<_, String>(0),
            "hello\t世界\nline"
        );
        assert_eq!(
            driver
                .client
                .query_one("SELECT extra FROM import_qa WHERE id=9007199254740995", &[])
                .unwrap()
                .get::<_, String>(0),
            "default"
        );
        assert_eq!(
            driver
                .client
                .query_one("SELECT value FROM import_qa WHERE id=9007199254740996", &[])
                .unwrap()
                .get::<_, String>(0),
            ""
        );
        let mut insert_null = request(
            &mut driver,
            "value,id\n,9007199254740997\n\\N,9007199254740998\n",
        );
        insert_null.postgres_inserts = true;
        insert_null.empty_as_null = true;
        assert_eq!(driver.import_csv(&insert_null, &cancel).unwrap(), 2);
        assert!(
            driver
                .client
                .query_one(
                    "SELECT value IS NULL FROM import_qa WHERE id=9007199254740997",
                    &[]
                )
                .unwrap()
                .get::<_, bool>(0)
        );
        assert_eq!(
            driver
                .client
                .query_one("SELECT value FROM import_qa WHERE id=9007199254740998", &[])
                .unwrap()
                .get::<_, String>(0),
            "\\N"
        );
        let mut invalid = fresh.clone();
        invalid.table = "pg_temp.import_qa; DROP TABLE import_qa".into();
        assert!(invalid.postgres_copy_sql().is_err());
        driver.client.batch_execute("CREATE FUNCTION pg_temp.slow_import() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(0.01); RETURN NEW; END $$; CREATE TRIGGER slow_import BEFORE INSERT ON import_qa FOR EACH ROW EXECUTE FUNCTION pg_temp.slow_import()").unwrap();
        let text = format!(
            "value,id\n{}",
            (8000..8100)
                .map(|i| format!("okay,{i}\n"))
                .collect::<String>()
        );
        let slow = request(&mut driver, &text);
        let signal = cancel.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            signal.store(true, Ordering::Relaxed);
        });
        assert!(driver.import_csv(&slow, &cancel).is_err());
        thread.join().unwrap();
        cancel.store(false, Ordering::Relaxed);
        assert_eq!(
            driver
                .client
                .query_one(
                    "SELECT count(*) FROM import_qa WHERE id BETWEEN 8000 AND 8099",
                    &[]
                )
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        driver
            .client
            .batch_execute("DROP TRIGGER slow_import ON import_qa")
            .unwrap();
        assert_eq!(driver.import_csv(&slow, &cancel).unwrap(), 100);
        driver.client.batch_execute("DELETE FROM import_qa WHERE id BETWEEN 8000 AND 8099; CREATE TRIGGER slow_import BEFORE INSERT ON import_qa FOR EACH ROW EXECUTE FUNCTION pg_temp.slow_import()").unwrap();
        let mut slow_inserts = slow.clone();
        slow_inserts.postgres_inserts = true;
        let signal = cancel.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            signal.store(true, Ordering::Relaxed);
        });
        assert!(driver.import_csv(&slow_inserts, &cancel).is_err());
        thread.join().unwrap();
        cancel.store(false, Ordering::Relaxed);
        assert_eq!(
            driver
                .client
                .query_one(
                    "SELECT count(*) FROM import_qa WHERE id BETWEEN 8000 AND 8099",
                    &[]
                )
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        driver.client.batch_execute("DROP TRIGGER slow_import ON import_qa; ALTER TABLE import_qa ADD COLUMN numbers integer[]").unwrap();
        let mut arrays = request(&mut driver, "value,id,numbers\narray,9000,\"{1,2,3}\"\n");
        arrays.destinations.push(Some(4));
        arrays.postgres_inserts = true;
        assert_eq!(driver.import_csv(&arrays, &cancel).unwrap(), 1);
        assert_eq!(
            driver
                .client
                .query_one("SELECT numbers FROM import_qa WHERE id=9000", &[])
                .unwrap()
                .get::<_, Vec<i32>>(0),
            vec![1, 2, 3]
        );
    }
    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn copy_import_permissions_rls_and_partitions() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = PostgresDriver::open(
            &crate::connection::PostgresConfig {
                host: "127.0.0.1".into(),
                port: 54329,
                database: "rustable_qa".into(),
                user: "postgres".into(),
                password: "rustable_qa_only".into(),
                tls: crate::connection::PostgresTlsMode::Disable,
                ssh: None,
            },
            cancel.clone(),
        )
        .unwrap();
        driver.client.batch_execute("CREATE TEMP TABLE import_qa(id integer, value text) PARTITION BY RANGE(id); CREATE TEMP TABLE import_low PARTITION OF import_qa FOR VALUES FROM (0) TO (100); CREATE TEMP TABLE import_high PARTITION OF import_qa FOR VALUES FROM (100) TO (200)").unwrap();
        let partitioned = request(&mut driver, "value,id\nlow,1\nhigh,101\n");
        assert_eq!(driver.import_csv(&partitioned, &cancel).unwrap(), 2);
        assert_eq!(
            driver
                .client
                .query_one("SELECT count(*) FROM import_low", &[])
                .unwrap()
                .get::<_, i64>(0),
            1
        );
        assert_eq!(
            driver
                .client
                .query_one("SELECT count(*) FROM import_high", &[])
                .unwrap()
                .get::<_, i64>(0),
            1
        );
        let missing = request(&mut driver, "value,id\nvalid,2\nmissing,201\n");
        assert!(driver.import_csv(&missing, &cancel).is_err());
        assert_eq!(
            driver
                .client
                .query_one("SELECT count(*) FROM import_qa", &[])
                .unwrap()
                .get::<_, i64>(0),
            2
        );
        driver
            .client
            .batch_execute("SET ROLE pg_read_all_data")
            .unwrap();
        let denied = driver.import_csv(&partitioned, &cancel).unwrap_err();
        assert!(denied.contains("permission denied"), "{denied}");
        driver.client.batch_execute("RESET ROLE; GRANT INSERT ON import_qa TO pg_read_all_data; ALTER TABLE import_qa ENABLE ROW LEVEL SECURITY; CREATE POLICY import_policy ON import_qa FOR INSERT TO pg_read_all_data WITH CHECK (id < 100); SET ROLE pg_read_all_data").unwrap();
        assert!(
            driver
                .client
                .query_one(
                    "SELECT row_security_active('pg_temp.import_qa'::regclass)",
                    &[]
                )
                .unwrap()
                .get::<_, bool>(0)
        );
        let blocked = driver.import_csv(&partitioned, &cancel).unwrap_err();
        assert!(blocked.contains("row-level security"), "{blocked}");
        let mut allowed = request(&mut driver, "value,id\nallowed,3\nallowed,4\n");
        allowed.postgres_inserts = true;
        assert_eq!(driver.import_csv(&allowed, &cancel).unwrap(), 2);
        let text = format!("value,id\n{}denied,101\n", "allowed,5\n".repeat(70));
        let mut policy_failure = request(&mut driver, &text);
        policy_failure.postgres_inserts = true;
        let denied = driver.import_csv(&policy_failure, &cancel).unwrap_err();
        assert!(denied.contains("row-level security"), "{denied}");
        driver.client.batch_execute("RESET ROLE").unwrap();
        assert_eq!(
            driver
                .client
                .query_one("SELECT count(*) FROM import_qa", &[])
                .unwrap()
                .get::<_, i64>(0),
            4
        );
        assert_eq!(driver.import_csv(&partitioned, &cancel).unwrap(), 2);
        driver.client.batch_execute("DROP TABLE import_qa CASCADE; CREATE TEMP SEQUENCE import_default_seq; CREATE TEMP TABLE import_qa(id integer DEFAULT nextval('import_default_seq'), value text, extra text DEFAULT 'default'); GRANT INSERT (id,value) ON import_qa TO pg_read_all_data").unwrap();
        let column_only = request(&mut driver, "value,id\ncolumn copy,1\n");
        let mut column_insert = request(&mut driver, "value,id\ncolumn insert,2\n");
        column_insert.postgres_inserts = true;
        driver.client.batch_execute("SET ROLE pg_read_all_data").unwrap();
        {
            let mut tx = driver.client.transaction().unwrap();
            let statement = lock_destination(&mut tx, "pg_temp.import_qa").unwrap();
            assert!(tx.query_one("SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_locks WHERE pid=pg_backend_pid() AND relation='pg_temp.import_qa'::regclass AND mode='RowExclusiveLock' AND granted)", &[]).unwrap().get::<_, bool>(0));
            drop(statement);
            assert_eq!(tx.query_one("SELECT count(*) FROM import_qa", &[]).unwrap().get::<_, i64>(0), 0);
            tx.rollback().unwrap();
        }
        assert_eq!(driver.import_csv(&column_only, &cancel).unwrap(), 1);
        assert_eq!(driver.import_csv(&column_insert, &cancel).unwrap(), 1);
        driver.client.batch_execute("RESET ROLE").unwrap();
        let rows = driver.client.query("SELECT id,value,extra FROM import_qa ORDER BY id", &[]).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get::<_, String>(1), "column copy");
        assert_eq!(rows[1].get::<_, String>(1), "column insert");
        assert!(rows.iter().all(|row| row.get::<_, String>(2) == "default"));
        assert!(!driver.client.query_one("SELECT is_called FROM import_default_seq", &[]).unwrap().get::<_, bool>(0), "preparing the lock must not evaluate defaults");
    }
    #[test]
    #[ignore = "manual opted-in PostgreSQL import throughput and process RSS benchmark"]
    fn postgres_import_streaming_benchmark() {
        use std::io::BufWriter;
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let rows: u64 = std::env::var("RUSTABLE_IMPORT_BENCH_ROWS")
            .unwrap_or_else(|_| "1000".into())
            .parse()
            .unwrap();
        assert!((1..=10_000_000).contains(&rows));
        let method =
            std::env::var("RUSTABLE_IMPORT_BENCH_METHOD").unwrap_or_else(|_| "copy".into());
        assert!(matches!(method.as_str(), "copy" | "insert"));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = PostgresDriver::open(
            &crate::connection::PostgresConfig {
                host: "127.0.0.1".into(),
                port: 54329,
                database: "rustable_qa".into(),
                user: "postgres".into(),
                password: "rustable_qa_only".into(),
                tls: crate::connection::PostgresTlsMode::Disable,
                ssh: None,
            },
            cancel.clone(),
        )
        .unwrap();
        driver
            .client
            .batch_execute(
                "CREATE TEMP TABLE import_qa(id bigint PRIMARY KEY, value text NOT NULL)",
            )
            .unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        {
            let mut output = BufWriter::new(file.as_file());
            writeln!(output, "value,id").unwrap();
            for id in 1..=rows {
                writeln!(output, "value{id},{id}").unwrap();
            }
            output.flush().unwrap();
        }
        let started = Instant::now();
        let preview =
            csv_preview::load(file.path(), csv_preview::Options::default(), None, &cancel).unwrap();
        let preview_ms = started.elapsed().as_secs_f64() * 1000.0;
        let source = preview.source.unwrap();
        let bytes = source.bytes;
        let request = csv_import::Request {
            schema: None,
            source,
            options: preview.options,
            table: "pg_temp.import_qa".into(),
            columns: driver.columns("pg_temp.import_qa").unwrap(),
            destinations: vec![Some(1), Some(0)],
            empty_as_null: false,
            postgres_inserts: method == "insert",
        };
        let started = Instant::now();
        assert_eq!(driver.import_csv(&request, &cancel).unwrap(), rows);
        let import_ms = started.elapsed().as_secs_f64() * 1000.0;
        // Check every imported value on the server without loading the result set.
        let verified = driver.client.query_one("SELECT count(*),COALESCE(sum(id),0)::text,count(*) FILTER (WHERE value <> 'value' || id::text) FROM import_qa", &[]).unwrap();
        assert_eq!(verified.get::<_, i64>(0), rows as i64);
        assert_eq!(
            verified.get::<_, String>(1),
            (rows * (rows + 1) / 2).to_string()
        );
        assert_eq!(verified.get::<_, i64>(2), 0);
        eprintln!(
            "PostgreSQL import method={method} rows={rows} bytes={bytes} preview_ms={preview_ms:.3} import_ms={import_ms:.3}"
        );
    }
}
