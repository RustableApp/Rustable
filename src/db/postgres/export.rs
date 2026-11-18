//! Full exports use a read-only transaction and a bounded server portal, never
//! the preview grid. PostgreSQL renders types itself so numeric precision and
//! extension types are preserved; bytea follows our CSV hexadecimal convention.
use super::*;
use std::{io, path::Path};

#[derive(Clone, Copy)]
enum CellKind {
    Text,
    Decimal,
    Integer,
    Real,
    Blob,
}

pub(super) fn export_table(
    driver: &mut PostgresDriver,
    path: &Path,
    browse: &Browse,
    cancel: &AtomicBool,
    format: ExportFormat,
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Export cancelled; destination unchanged".into());
    }
    driver.require_idle_transaction()?;
    driver.check_tunnel()?;
    let _watch = driver.watch();
    let mut transaction = driver
        .client
        .build_transaction()
        .read_only(true)
        .start()
        .map_err(error)?;
    let relation = transaction.query_opt(
        "SELECT n.nspname,c.relname,c.relkind::text FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE c.oid=pg_catalog.to_regclass($1) AND c.relkind IN ('r','p','v','m','f')",
        &[&browse.table],
    ).map_err(error)?.ok_or("Table no longer exists")?;
    if format == ExportFormat::Sql && !matches!(relation.get::<_, &str>(2), "r" | "p") {
        return Err("SQL inserts require an ordinary or partitioned table".into());
    }
    let table = format!("{}.{}", quote(relation.get(0)), quote(relation.get(1)));
    let metadata = transaction.query(
        "WITH RECURSIVE bases AS (SELECT a.attnum,t.oid,t.typbasetype FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_type t ON t.oid=a.atttypid WHERE a.attrelid=pg_catalog.to_regclass($1) AND a.attnum>0 AND NOT a.attisdropped UNION ALL SELECT b.attnum,t.oid,t.typbasetype FROM bases b JOIN pg_catalog.pg_type t ON t.oid=b.typbasetype WHERE b.typbasetype<>0) SELECT a.attname,a.atttypid='pg_catalog.bytea'::pg_catalog.regtype,EXISTS(SELECT 1 FROM pg_catalog.pg_index i WHERE i.indrelid=a.attrelid AND i.indisprimary AND a.attnum=ANY(i.indkey)),b.oid::bigint,a.attgenerated<>'',a.attidentity<>'' FROM pg_catalog.pg_attribute a JOIN bases b ON b.attnum=a.attnum AND b.typbasetype=0 WHERE a.attrelid=pg_catalog.to_regclass($1) AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum",
        &[&table],
    ).map_err(error)?;
    let columns: Vec<String> = metadata.iter().map(|row| row.get(0)).collect();
    let kinds: Vec<_> = metadata
        .iter()
        .map(|row| {
            if matches!(format, ExportFormat::Csv | ExportFormat::Sql) {
                return CellKind::Text;
            }
            if matches!(format, ExportFormat::CsvCustom(_)) {
                return match Type::from_oid(row.get::<_, i64>(3) as u32) {
                    Some(
                        Type::NUMERIC
                        | Type::FLOAT4
                        | Type::FLOAT8
                        | Type::INT2
                        | Type::INT4
                        | Type::INT8
                        | Type::OID,
                    ) => CellKind::Decimal,
                    _ => CellKind::Text,
                };
            }
            match Type::from_oid(row.get::<_, i64>(3) as u32) {
                Some(Type::INT2 | Type::INT4 | Type::INT8 | Type::OID) => CellKind::Integer,
                Some(Type::FLOAT4 | Type::FLOAT8) => CellKind::Real,
                Some(Type::BYTEA) => CellKind::Blob,
                _ => CellKind::Text,
            }
        })
        .collect();
    let projection = metadata
        .iter()
        .zip(&kinds)
        .map(|(row, kind)| {
            let name = quote(row.get(0));
            if format.is_csv() && row.get::<_, bool>(1) {
                format!("pg_catalog.encode({name}, 'hex')")
            } else {
                let target = match kind {
                    CellKind::Text | CellKind::Decimal => "text",
                    CellKind::Integer => "int8",
                    CellKind::Real => "float8",
                    CellKind::Blob => "bytea",
                };
                format!("CAST({name} AS pg_catalog.{target})")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!("SELECT {projection} FROM {table}");
    let mut parameters: Vec<&(dyn ToSql + Sync)> = Vec::new();
    if let Some(column) = &browse.filter_column {
        if !columns.contains(column) {
            return Err("Filter column no longer exists".into());
        }
        sql.push_str(&format!(
            " WHERE pg_catalog.strpos(CAST({} AS text), $1) > 0",
            quote(column)
        ));
        parameters.push(&browse.filter);
    }
    let predicate = crate::db::filters::predicate(&browse.filters, &columns, crate::sql::Dialect::Postgres)?;
    if !predicate.is_empty() { sql.push_str(if browse.filter_column.is_some() { " AND " } else { " WHERE " }); sql.push_str(&predicate); }
    let sort = browse.sort.clone().or_else(|| {
        metadata
            .iter()
            .find(|row| row.get::<_, bool>(2))
            .map(|row| (row.get::<_, String>(0), false))
    });
    if let Some((column, descending)) = sort {
        if !columns.contains(&column) {
            return Err("Sort column no longer exists".into());
        }
        sql.push_str(&format!(
            " ORDER BY {table}.{} {}",
            quote(&column),
            if descending { "DESC" } else { "ASC" }
        ));
    }
    let statement = transaction.prepare(&sql).map_err(error)?;
    let portal = transaction.bind(&statement, &parameters).map_err(error)?;
    csv::publish(path, cancel, |writer| {
        let mut writer = row_export::Writer::table(writer, &columns, cancel, format, || crate::db::sql_export::Spec {
            dialect: crate::db::sql_export::Dialect::Postgres, table: table.clone(),
            after: metadata.iter().filter(|row| row.get::<_,bool>(5)).map(|row| {
                let name: &str = row.get(0);
                let literal = |value: &str| format!("E'{}'", value.replace('\\', "\\\\").replace('\'', "''"));
                format!("SELECT pg_catalog.setval(pg_catalog.pg_get_serial_sequence({}, {}), MAX({}), true) FROM {table} HAVING MAX({}) IS NOT NULL;\n", literal(&table), literal(name), quote(name), quote(name))
            }).collect(),
            columns: columns.iter().map(|name| quote(name)).collect(),
            included: metadata.iter().map(|row| !row.get::<_, bool>(4)).collect(),
        })?;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Export cancelled",
                ));
            }
            // Fixed batches bound retained rows independently of table size.
            // Memory is at most 64 rows plus the CSV output buffer.
            let rows = transaction
                .query_portal(&portal, 64)
                .map_err(|e| io::Error::other(error(e)))?;
            if rows.is_empty() {
                break;
            }
            for row in &rows {
                writer.row((0..row.len()).map(|index| {
                    match kinds[index] {
                        CellKind::Decimal => row.try_get::<_,Option<&str>>(index).map(|v|v.map_or(csv::Cell::Null,csv::Cell::Decimal)),
                        CellKind::Text => row
                            .try_get::<_, Option<&str>>(index)
                            .map(|v| v.map_or(csv::Cell::Null, csv::Cell::Text)),
                        CellKind::Integer => row
                            .try_get::<_, Option<i64>>(index)
                            .map(|v| v.map_or(csv::Cell::Null, csv::Cell::Integer)),
                        CellKind::Real => row
                            .try_get::<_, Option<f64>>(index)
                            .map(|v| v.map_or(csv::Cell::Null, csv::Cell::Real)),
                        CellKind::Blob => row
                            .try_get::<_, Option<&[u8]>>(index)
                            .map(|v| v.map_or(csv::Cell::Null, csv::Cell::Blob)),
                    }
                    .map_err(io::Error::other)
                }))?;
            }
        }
        writer.finish()?;
        // Finish the read-only transaction before atomic publication.
        transaction.commit().map_err(|e| io::Error::other(error(e)))
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires disposable PostgreSQL QA fixture"]
    fn sql_insert_roundtrip() {
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
        driver.client.batch_execute("CREATE TYPE pg_temp.export_mood AS ENUM ('日本','sad'); CREATE TEMP TABLE sql_insert_probe(id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, txt TEXT, bytes BYTEA, amount NUMERIC(30,8), mood pg_temp.export_mood, items INTEGER[], doc JSONB, n DOUBLE PRECISION, derived BIGINT GENERATED ALWAYS AS(id*2) STORED); INSERT INTO sql_insert_probe(txt,bytes,amount,mood,items,doc,n) VALUES(E'日本 ''quoted'' \\\\ tail',decode('00ff','hex'),12345678901234567890.12345678,'日本',ARRAY[1,NULL,3],'{\"ok\":true}','Infinity'),(NULL,NULL,NULL,NULL,NULL,NULL,'NaN'); CREATE FUNCTION pg_temp.sql_export_bump() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN NEW.amount=NEW.amount+1; RETURN NEW; END$$; CREATE TRIGGER sql_export_bump BEFORE INSERT ON sql_insert_probe FOR EACH ROW EXECUTE FUNCTION pg_temp.sql_export_bump();").unwrap();
        let original: String = driver
            .client
            .query_one(
                "SELECT jsonb_agg(to_jsonb(t) ORDER BY id)::text FROM sql_insert_probe t",
                &[],
            )
            .unwrap()
            .get(0);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("combined.sql");
        driver.client.batch_execute("CREATE TEMP TABLE export_parent(id INT PRIMARY KEY, child_id INT); CREATE TEMP TABLE export_child(id INT PRIMARY KEY, parent_id INT REFERENCES export_parent(id)); INSERT INTO export_parent VALUES(1,2); INSERT INTO export_child VALUES(2,1); ALTER TABLE export_parent ADD CONSTRAINT parent_child FOREIGN KEY(child_id) REFERENCES export_child(id);").unwrap();
        crate::db::table_export::sql_bundle(
            &mut driver,
            &path,
            &[
                ("export_child".into(), "child".into()),
                ("sql_insert_probe".into(), "probe".into()),
                ("export_parent".into(), "parent".into()),
            ],
            true,
            true,
            &cancel,
        )
        .unwrap();
        let csv_path = dir.path().join("configured.csv");
        let options = crate::db::csv_options::Options {
            header: false,
            delimiter: crate::db::csv_options::Delimiter::Semicolon,
            decimal_comma: true,
            null_to_empty: false,
            ..Default::default()
        };
        driver
            .export_table(
                &csv_path,
                &Browse {
                    table: "sql_insert_probe".into(),
                    ..Default::default()
                },
                &cancel,
                ExportFormat::CsvCustom(options),
            )
            .unwrap();
        let mut reader = ::csv::ReaderBuilder::new()
            .has_headers(false)
            .delimiter(b';')
            .from_path(csv_path)
            .unwrap();
        let record = reader.records().next().unwrap().unwrap();
        assert_eq!(&record[3], "12345678901234567890,12345678");
        let script = std::fs::read_to_string(path).unwrap();
        driver
            .client
            .batch_execute("DROP TABLE export_child,export_parent,sql_insert_probe")
            .unwrap();
        driver.client.batch_execute(&script).unwrap();
        let restored: String = driver
            .client
            .query_one(
                "SELECT jsonb_agg(to_jsonb(t) ORDER BY id)::text FROM sql_insert_probe t",
                &[],
            )
            .unwrap()
            .get(0);
        assert_eq!(restored, original);
        assert!(
            driver
                .client
                .execute("INSERT INTO export_child VALUES(9,99)", &[])
                .is_err()
        );
        let next: i64 = driver
            .client
            .query_one(
                "INSERT INTO sql_insert_probe(amount) VALUES(4) RETURNING id",
                &[],
            )
            .unwrap()
            .get(0);
        assert_eq!(next, 3);
        assert_eq!(
            driver
                .client
                .query_one("SELECT amount::text FROM sql_insert_probe WHERE id=3", &[])
                .unwrap()
                .get::<_, String>(0),
            "5.00000000"
        );
    }

    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn full_export_filters_types_and_atomic_failure() {
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
        driver.client.batch_execute("CREATE TEMP TABLE export_fixture(id integer PRIMARY KEY, label text, amount numeric(30,8), bytes bytea, missing text, empty text); INSERT INTO export_fixture SELECT n, CASE WHEN n%2=0 THEN 'even' ELSE 'odd' END, 12345678901234567890.12345678, decode('00ff','hex'), NULL, '' FROM generate_series(1,6001) n").unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("full.csv");
        let mut browse = Browse {
            table: "export_fixture".into(),
            offset: 5000,
            ..Default::default()
        };
        driver
            .export_table(&path, &browse, &cancel, ExportFormat::Csv)
            .unwrap();
        let output = std::fs::read_to_string(&path).unwrap();
        assert_eq!(output.lines().count(), 6002);
        assert!(
            output
                .lines()
                .nth(1)
                .unwrap()
                .contains("\"1\",\"odd\",\"12345678901234567890.12345678\",\"00ff\",,\"\"")
        );
        browse.filter_column = Some("label".into());
        browse.filter = "even".into();
        browse.sort = Some(("id".into(), true));
        driver
            .export_table(&path, &browse, &cancel, ExportFormat::Csv)
            .unwrap();
        let filtered = std::fs::read_to_string(&path).unwrap();
        assert_eq!(filtered.lines().count(), 3001);
        assert!(filtered.lines().nth(1).unwrap().starts_with("\"6000\","));
        cancel.store(true, Ordering::Relaxed);
        assert!(
            driver
                .export_table(&path, &browse, &cancel, ExportFormat::Csv)
                .is_err()
        );
        cancel.store(false, Ordering::Relaxed);
        browse.sort = Some(("missing_column".into(), false));
        assert!(
            driver
                .export_table(&path, &browse, &cancel, ExportFormat::Csv)
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), filtered);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        driver.client.batch_execute("CREATE TEMP VIEW export_slow AS SELECT n, pg_sleep(0.01)::text AS wait FROM generate_series(1,1000) n").unwrap();
        browse = Browse {
            table: "export_slow".into(),
            ..Default::default()
        };
        let request_cancel = cancel.clone();
        let cancellation = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            request_cancel.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        assert!(
            driver
                .export_table(&path, &browse, &cancel, ExportFormat::Csv)
                .is_err()
        );
        cancellation.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(3));
        cancel.store(false, Ordering::Relaxed);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), filtered);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        driver.client.batch_execute("CREATE TEMP VIEW export_error AS SELECT 1/(n-500) AS value FROM generate_series(1,1000) n").unwrap();
        browse = Browse {
            table: "export_error".into(),
            ..Default::default()
        };
        assert!(
            driver
                .export_table(&path, &browse, &cancel, ExportFormat::Csv)
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), filtered);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        assert_eq!(
            driver
                .client
                .query_one("SELECT 1", &[])
                .unwrap()
                .get::<_, i32>(0),
            1
        );
    }
    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn full_json_export_types_domains_filter_and_atomic_failure() {
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
        driver.client.batch_execute("CREATE DOMAIN pg_temp.json_id AS bigint; CREATE DOMAIN pg_temp.json_nested AS pg_temp.json_id; CREATE DOMAIN pg_temp.json_bytes AS bytea; CREATE TEMP TABLE json_export(id pg_temp.json_nested PRIMARY KEY, label text, amount numeric(30,8), bytes pg_temp.json_bytes, score double precision, missing text); INSERT INTO json_export SELECT n, CASE WHEN n%2=0 THEN 'even' ELSE 'odd' END, 12345678901234567890.12345678, decode('00ff','hex'), 1.25, NULL FROM generate_series(1,6001) n").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("all.json");
        let mut browse = Browse {
            table: "json_export".into(),
            offset: 5000,
            ..Default::default()
        };
        driver
            .export_table(&path, &browse, &cancel, ExportFormat::Json)
            .unwrap();
        let all: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(all.len(), 6001);
        for (index, row) in all.iter().enumerate() {
            let id = index + 1;
            assert_eq!(
                *row,
                serde_json::json!({"id":id, "label": if id%2==0 {"even"} else {"odd"}, "amount":"12345678901234567890.12345678", "bytes":[0,255], "score":1.25, "missing":null})
            );
        }
        browse.filter_column = Some("label".into());
        browse.filter = "even".into();
        browse.sort = Some(("id".into(), true));
        driver
            .export_table(&path, &browse, &cancel, ExportFormat::Json)
            .unwrap();
        let original = std::fs::read(&path).unwrap();
        let filtered: Vec<serde_json::Value> = serde_json::from_slice(&original).unwrap();
        assert_eq!(filtered.len(), 3000);
        for (index, row) in filtered.iter().enumerate() {
            assert_eq!(row["id"], (3000 - index) * 2);
        }
        driver
            .client
            .batch_execute("UPDATE json_export SET score='Infinity' WHERE id=100")
            .unwrap();
        assert!(
            driver
                .export_table(&path, &browse, &cancel, ExportFormat::Json)
                .unwrap_err()
                .contains("non-finite")
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        // JSON cancellation after export starts leaves the previous file intact.
        driver.client.batch_execute("CREATE TEMP VIEW json_slow AS SELECT n, pg_sleep(0.01)::text AS wait FROM generate_series(1,1000) n").unwrap();
        browse = Browse {
            table: "json_slow".into(),
            ..Default::default()
        };
        let flag = cancel.clone();
        let trigger = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            flag.store(true, Ordering::Relaxed);
        });
        assert!(
            driver
                .export_table(&path, &browse, &cancel, ExportFormat::Json)
                .is_err()
        );
        trigger.join().unwrap();
        cancel.store(false, Ordering::Relaxed);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(dir.path().read_dir().unwrap().count(), 1);
        assert_eq!(
            driver
                .client
                .query_one("SELECT 1", &[])
                .unwrap()
                .get::<_, i32>(0),
            1
        );
        driver.client.batch_execute("BEGIN").unwrap();
        assert!(
            driver
                .export_table(&path, &browse, &cancel, ExportFormat::Json)
                .unwrap_err()
                .contains("Finish the SQL transaction")
        );
        assert_eq!(driver.client.transaction_status(), Some(b'T'));
        driver.client.batch_execute("ROLLBACK").unwrap();
    }
}
