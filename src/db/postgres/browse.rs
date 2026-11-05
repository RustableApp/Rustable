//! Sidebar hints allow a first autocommit page and fresh editing metadata to
//! share one exchange. Unknown relations and explicit transactions validate first.
use super::*;
pub(super) mod text;

const CATALOG: &str = r#"
WITH RECURSIVE relation AS (
 SELECT c.oid,n.nspname,c.relname,c.relkind,c.reltuples
 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
 WHERE c.oid=pg_catalog.to_regclass($1) AND c.relkind IN ('r','p','v','m','f')
), type_bases AS (
 SELECT a.attnum,a.atttypid AS original_oid,t.oid,t.typbasetype
 FROM relation r JOIN pg_catalog.pg_attribute a ON a.attrelid=r.oid
 JOIN pg_catalog.pg_type t ON t.oid=a.atttypid
 WHERE a.attnum>0 AND NOT a.attisdropped
 UNION ALL
 SELECT b.attnum,b.original_oid,t.oid,t.typbasetype
 FROM type_bases b JOIN pg_catalog.pg_type t ON t.oid=b.typbasetype
 WHERE b.typbasetype<>0
)
SELECT r.nspname,r.relname,
 CASE WHEN r.relkind IN ('r','p','m') THEN r.reltuples::double precision ELSE NULL END,
 COALESCE((SELECT pg_catalog.json_agg(pg_catalog.json_build_array(
 a.attname,pg_catalog.format_type(a.atttypid,a.atttypmod),a.attnotnull,
 COALESCE(pg_catalog.pg_get_expr(d.adbin,d.adrelid),''),
 EXISTS(SELECT 1 FROM pg_catalog.pg_index i WHERE i.indrelid=a.attrelid AND i.indisprimary AND a.attnum=ANY(i.indkey)),
 (a.attgenerated<>'' OR a.attidentity<>''),a.atttypid::bigint,b.oid::bigint
 ) ORDER BY a.attnum)
 FROM pg_catalog.pg_attribute a
 JOIN type_bases b ON b.attnum=a.attnum AND b.typbasetype=0
 LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum
 WHERE a.attrelid=r.oid AND a.attnum>0 AND NOT a.attisdropped),'[]'::json)::text,
 '[]'::text, '[]'::text,
 COALESCE((SELECT pg_catalog.json_agg(pg_catalog.json_build_object(
 'name', con.conname, 'schema', ns.nspname, 'table', target.relname,
 'columns', ARRAY(SELECT a.attname FROM unnest(con.conkey) WITH ORDINALITY k(num,ord) JOIN pg_catalog.pg_attribute a ON a.attrelid=con.conrelid AND a.attnum=k.num ORDER BY k.ord),
 'referenced_columns', ARRAY(SELECT a.attname FROM unnest(con.confkey) WITH ORDINALITY k(num,ord) JOIN pg_catalog.pg_attribute a ON a.attrelid=con.confrelid AND a.attnum=k.num ORDER BY k.ord)
 ) ORDER BY con.conname)
 FROM pg_catalog.pg_constraint con JOIN pg_catalog.pg_class target ON target.oid=con.confrelid
 JOIN pg_catalog.pg_namespace ns ON ns.oid=target.relnamespace
 WHERE con.conrelid=r.oid AND con.contype='f'),'[]'::json)::text
FROM relation r
"#;

const DETAILS: &str = r#"
SELECT
 COALESCE((SELECT pg_catalog.json_agg(pg_catalog.json_build_array(
 ci.relname,i.indisunique,i.indisprimary,i.indpred IS NOT NULL,
 pg_catalog.pg_get_indexdef(i.indexrelid),
 ARRAY(SELECT pg_catalog.pg_get_indexdef(i.indexrelid,k,true) FROM pg_catalog.generate_series(1,i.indnkeyatts) AS k),
 EXISTS(SELECT 1 FROM pg_catalog.pg_constraint con WHERE con.conindid=i.indexrelid AND con.contype IN ('u','x'))
 ) ORDER BY ci.relname)
 FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class ci ON ci.oid=i.indexrelid
 WHERE i.indrelid=r.oid),'[]'::json)::text,
 COALESCE((SELECT pg_catalog.json_agg(pg_catalog.json_build_array(t.tgname,pg_catalog.pg_get_triggerdef(t.oid)) ORDER BY t.tgname)
 FROM pg_catalog.pg_trigger t WHERE t.tgrelid=r.oid AND NOT t.tgisinternal),'[]'::json)::text
FROM pg_catalog.pg_class r WHERE r.oid=pg_catalog.to_regclass($1)
"#;

pub(super) fn details(driver: &mut PostgresDriver, table: &str) -> Result<(Vec<Index>, Vec<Object>), String> {
    driver.check_tunnel()?;
    let _watch = driver.watch();
    let row = driver.client.query_typed_raw_bounded_with_prefix(
        DETAILS, [(table, Type::TEXT)], &[], 1, BYTE_LIMIT,
    ).map_err(error)?.collect::<Vec<_>>().map_err(error)?
        .into_iter().next().ok_or("Table no longer exists")?;
    parse_details(row.get(0), row.get(1))
}

type CatalogColumn = (String, String, bool, String, bool, bool, u32, u32);
type CatalogIndex = (String, bool, bool, bool, String, Vec<String>, bool);

pub(super) fn browse(driver: &mut PostgresDriver, request: &Browse) -> Result<TableData, String> {
    driver.check_tunnel()?;
    let _watch = driver.watch();
    let started = Instant::now();
    if driver.client.transaction_status() == Some(b'I')
        && let Some(hint) = driver.browse_hints.iter().find(|o| o.name == request.table).cloned()
    {
        return text::browse(driver, request, hint, started);
    }
    // Only speculate in autocommit: stale-column errors must never abort an
    // explicit transaction. Keep up to 16 recently used table shapes.
    let cached = driver
        .browse_catalog
        .iter()
        .find(|(name, _)| {
            name == &request.table && driver.client.transaction_status() == Some(b'I')
        })
        .map(|(_, row)| row.clone());
    let pipelined = cached.is_some();
    let mut row = match cached {
        Some(row) => row,
        None => driver
            .client
            .query_typed_raw_bounded_with_prefix(
                CATALOG, [(&request.table, Type::TEXT)], &[], 1, BYTE_LIMIT,
            )
            .map_err(error)?
            .collect::<Vec<_>>()
            .map_err(error)?
            .into_iter().next().ok_or("Table no longer exists")?,
    };
    if driver.cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }
    let table = format!("{}.{}", quote(row.get(0)), quote(row.get(1)));
    let raw_columns: Vec<CatalogColumn> =
        serde_json::from_str(row.get(3)).map_err(|e| format!("Invalid column metadata: {e}"))?;
    // Ordinary columns need no conversion. Use the actual wildcard query in
    // both execution and activity; retain explicit projections only for casts.
    let projection = if raw_columns.iter().all(|(_, _, _, _, _, _, oid, base_oid)| {
        oid == base_oid && Type::from_oid(*base_oid).is_some_and(|ty| native_display_type(&ty))
    }) {
        "*".to_owned()
    } else {
        raw_columns
            .iter()
            .map(|(name, _, _, _, _, _, original_oid, base_oid)| {
                let source = format!("{table}.{}", quote(name));
                match Type::from_oid(*base_oid).filter(native_display_type) {
                    Some(ty) if original_oid != base_oid => format!(
                        "CAST({source} AS pg_catalog.{}) AS {}",
                        quote(ty.name()),
                        quote(name)
                    ),
                    Some(_) => source,
                    None => format!("CAST({source} AS pg_catalog.text) AS {}", quote(name)),
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let columns = raw_columns
        .into_iter()
        .map(
            |(name, data_type, not_null, default, pk, generated, _, _)| Column {
                name,
                data_type,
                not_null,
                default,
                pk,
                generated,
            },
        )
        .collect::<Vec<_>>();
    let (sql, parameters) = match query(request, &columns, &table, &projection) {
        Ok(query) => query,
        Err(_) if pipelined => {
            driver.browse_catalog.retain(|(name,_)| name != &request.table);
            return browse(driver, request);
        }
        Err(error) => return Err(error),
    };
    let values = bind_values(&parameters);
    let rows = if pipelined {
        let (fresh, rows) = driver
            .client
            .query_typed_raw_with_metadata(
                CATALOG, &[(&request.table, Type::TEXT)], &sql, values,
                request.page_size() + 1, BYTE_LIMIT,
            )
            .map_err(error)?;
        let fresh = fresh.into_iter().next().ok_or("Table no longer exists")?;
        if driver.cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        // Column types, order, primary keys and canonical relation identity
        // determine the query and decoder. Index/trigger/estimate changes do
        // not: use their fresh values without repeating the data request.
        let same_shape = [0, 1, 3]
            .into_iter()
            .all(|index| fresh.get::<_, &str>(index) == row.get::<_, &str>(index));
        if !same_shape {
            drop(rows);

            driver
                .browse_catalog
                .retain(|(name, _)| name != &request.table);
            let mut result = browse(driver, request)?;
            result.grid.elapsed = started.elapsed();
            return Ok(result);
        }
        row = fresh;
        rows.map_err(error)?
    } else {
        driver.client.query_typed_raw_bounded_with_prefix(
            &sql, values, &[], request.page_size() + 1, BYTE_LIMIT,
        ).map_err(error)?
    };
    let names = rows.columns().iter().map(|c| c.name().to_owned()).collect();
    let grid = collect(rows, names, request.page_size(), BYTE_LIMIT, &driver.cancel)?;
    finish(driver, request, row, grid, columns, sql, parameters, started)
}

fn bind_values(parameters: &[Value]) -> Vec<(&(dyn ToSql + Sync), Type)> {
    parameters.iter().map(|v| match v {
        Value::Text(v) => (v as &(dyn ToSql + Sync), Type::TEXT),
        Value::Integer(v) => (v as &(dyn ToSql + Sync), Type::INT8),
        _ => unreachable!("browse parameters are text, limit and offset"),
    }).collect()
}

fn query(request: &Browse, columns: &[Column], table: &str, projection: &str) -> Result<(String, Vec<Value>), String> {
    let mut sql = format!("SELECT {projection} FROM {table}");
    let mut parameters = Vec::new();

    if let Some(column) = &request.filter_column {
        if !columns.iter().any(|c| &c.name == column) {
            return Err("Filter column no longer exists".into());
        }
        sql.push_str(&format!(
            " WHERE pg_catalog.strpos(CAST({} AS text), $1) > 0",
            quote(column)
        ));
        parameters.push(Value::Text(request.filter.clone()));

    }
    let predicate = crate::db::filters::predicate(
        &request.filters,
        &columns.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
        crate::sql::Dialect::Postgres,
    )?;
    if !predicate.is_empty() {
        sql.push_str(if request.filter_column.is_some() {
            " AND "
        } else {
            " WHERE "
        });
        sql.push_str(&predicate);
    }
    if let Some((column, descending)) = request.sort.clone().or_else(|| {
        columns
            .iter()
            .find(|c| c.pk)
            .map(|c| (c.name.clone(), false))
    }) {
        if !columns.iter().any(|c| c.name == column) {
            return Err("Sort column no longer exists".into());
        }
        sql.push_str(&format!(
            " ORDER BY {table}.{} {}",
            quote(&column),
            if descending { "DESC" } else { "ASC" }
        ));
    }
    let limit = (request.page_size() + 1) as i64;
    let offset = i64::try_from(request.offset).map_err(|_| "Page offset is too large")?;
    sql.push_str(&format!(
        " LIMIT ${} OFFSET ${}",
        parameters.len() + 1,
        parameters.len() + 2
    ));
    parameters.push(Value::Integer(limit));
    parameters.push(Value::Integer(offset));
    Ok((sql, parameters))
}

#[allow(clippy::too_many_arguments)]
fn finish(driver: &mut PostgresDriver, request: &Browse, row: Row, mut grid: Grid,
    columns: Vec<Column>, sql: String, parameters: Vec<Value>, started: Instant) -> Result<TableData, String> {
    let estimate = row
        .get::<_, Option<f64>>(2)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .filter(|_| request.filter_column.is_none() && !request.filters.iter().any(|f| f.enabled))
        .map(|value| value.round() as u64);
    grid.set_browse_total(request.offset, estimate);
    let indexes = Vec::new();
    let triggers = Vec::new();

    driver
        .browse_catalog
        .retain(|(name, _)| name != &request.table);
    let foreign_keys = serde_json::from_str(row.get(6))
        .map_err(|e| format!("Invalid foreign key metadata: {e}"))?;
    driver.browse_hints.retain(|o| o.name != request.table);
    driver.browse_hints.push(Object {
        name: request.table.clone(), schema: Some(row.get(0)), display_name: Some(row.get(1)),
        kind: "table".into(), columns: columns.clone(), metadata_error: None, ddl: String::new(),
    });
    driver.browse_catalog.push((request.table.clone(), row));
    if driver.browse_catalog.len() > 16 {
        driver.browse_catalog.remove(0);
    }
    grid.elapsed = started.elapsed();
    Ok(TableData {
        details_loaded: false,
        foreign_keys,
        sql,
        parameters,
        grid,
        columns,
        indexes,
        triggers,
    })
}

fn parse_details(index_json: &str, trigger_json: &str) -> Result<(Vec<Index>, Vec<Object>), String> {
    let raw_indexes: Vec<CatalogIndex> =
        serde_json::from_str(index_json).map_err(|e| format!("Invalid index metadata: {e}"))?;
    let raw_triggers: Vec<(String, String)> =
        serde_json::from_str(trigger_json).map_err(|e| format!("Invalid trigger metadata: {e}"))?;
    let indexes = raw_indexes
        .into_iter()
        .map(
            |(name, unique, primary, partial, ddl, columns, constraint)| Index {
                name,
                unique,
                origin: if primary {
                    "pk"
                } else if constraint {
                    "u"
                } else {
                    "c"
                }
                .into(),
                partial,
                columns,
                ddl,
            },
        )
        .collect();
    let triggers = raw_triggers
        .into_iter()
        .map(|(name, ddl)| Object {
            name,
            ddl,
            schema: None,
            display_name: None,
            metadata_error: None,
            kind: "trigger".into(),
            columns: Vec::new(),
        })
        .collect();

    Ok((indexes, triggers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn stale_wildcard_with_large_uncached_enum_rows_recovers_without_stalling() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let (send, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let cancel = Arc::new(AtomicBool::new(false));
            let mut driver = PostgresDriver::open(&crate::connection::PostgresConfig {
                host: "127.0.0.1".into(), port: 54329, database: "rustable_qa".into(),
                user: "postgres".into(), password: "rustable_qa_only".into(), tls: false, ssh: None,
            }, cancel).unwrap();
            driver.client.batch_execute("CREATE TEMP TABLE wildcard_refresh(id integer PRIMARY KEY); INSERT INTO wildcard_refresh SELECT generate_series(1,40)").unwrap();
            let mut request = Browse { table: "wildcard_refresh".into(), limit: 20, ..Default::default() };
            let first = driver.browse(&request).unwrap();
            assert!(first.sql.starts_with("SELECT *"));
            assert_eq!(first.grid.rows.len(), 20);
            assert!(first.grid.truncated);
            // Cached wildcard now returns an unknown enum and more data than
            // the driver's bounded response channel can hold. Drain before
            // type lookup, then discard the stale result and rebuild casts.
            driver.client.batch_execute("CREATE TYPE pg_temp.wildcard_state AS ENUM ('active'); ALTER TABLE wildcard_refresh ADD COLUMN state pg_temp.wildcard_state DEFAULT 'active'; ALTER TABLE wildcard_refresh ADD COLUMN payload text DEFAULT repeat('x',100000)").unwrap();
            let refreshed = driver.browse(&request).unwrap();
            assert_eq!(refreshed.grid.columns, ["id", "state", "payload"]);
            assert_eq!(refreshed.grid.rows.len(), 20);
            assert!(refreshed.grid.truncated);
            assert!(refreshed.sql.contains("CAST("));
            assert_eq!(refreshed.grid.rows[0][1], Value::Text("active".into()));
            assert_eq!(refreshed.grid.rows[0][2], Value::Text("x".repeat(100000)));
            request.offset = 20;
            let second = driver.browse(&request).unwrap();
            assert_eq!(second.grid.rows.len(), 20);
            assert!(!second.grid.truncated);
            assert_eq!(second.grid.rows[0][0], Value::Integer(21));
            send.send(()).unwrap();
        });
        receive.recv_timeout(Duration::from_secs(10)).expect("stale wildcard refresh stalled or failed");
        worker.join().unwrap();
    }

    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn refresh_reads_fresh_metadata_and_preserves_domain_values_and_transactions() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = PostgresDriver::open(
            &crate::connection::PostgresConfig {
                host: "127.0.0.1".into(),
                port: 54329,
                database: "rustable_qa".into(),
                user: "postgres".into(),
                password: "rustable_qa_only".into(),
                tls: false,
                ssh: None,
            },
            cancel.clone(),
        )
        .unwrap();
        driver.client.batch_execute(r#"
            CREATE DOMAIN pg_temp.browse_domain AS integer CHECK (VALUE > 0);
            CREATE DOMAIN pg_temp.browse_nested AS pg_temp.browse_domain;
            CREATE TYPE pg_temp.browse_mood AS ENUM ('calm','busy');
            CREATE TEMP TABLE browse_refresh(id pg_temp.browse_nested PRIMARY KEY, label text DEFAULT 'hello', mood pg_temp.browse_mood, numbers integer[]);
            INSERT INTO browse_refresh VALUES(2,'original','calm',ARRAY[1,2]);
            CREATE INDEX browse_expression ON browse_refresh ((lower(label))) WHERE label IS NOT NULL;
            CREATE FUNCTION pg_temp.browse_touch() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN RETURN NEW; END$$;
            CREATE TRIGGER browse_touch BEFORE UPDATE ON browse_refresh FOR EACH ROW EXECUTE FUNCTION pg_temp.browse_touch();
        "#).unwrap();
        let mut request = Browse {
            table: "browse_refresh".into(),
            ..Default::default()
        };
        let initial = driver.browse(&request).unwrap();
        assert_eq!(
            initial.grid.rows[0],
            [
                Value::Integer(2),
                Value::Text("original".into()),
                Value::Text("calm".into()),
                Value::Text("{1,2}".into())
            ]
        );
        assert!(!initial.details_loaded);
        assert!(initial.indexes.is_empty());
        let (indexes, triggers) = driver.table_details(&request.table).unwrap();
        assert_eq!(indexes.len(), 2);
        assert!(
            indexes
                .iter()
                .any(|i| i.partial && i.columns[0].contains("lower"))
        );
        assert_eq!(triggers.len(), 1);
        assert!(initial.columns[0].pk);
        assert!(initial.columns[1].default.contains("hello"));
        // Visiting another table must retain the first table's validated hint.
        driver
            .client
            .batch_execute(
                "CREATE TEMP TABLE browse_other(id integer); INSERT INTO browse_other VALUES (42)",
            )
            .unwrap();
        let other = Browse {
            table: "browse_other".into(),
            ..Default::default()
        };
        let plain = driver.browse(&other).unwrap();
        assert!(plain.sql.starts_with("SELECT * FROM "), "{}", plain.sql);
        assert_eq!(plain.grid.rows, [vec![Value::Integer(42)]]);
        assert!(driver.browse(&other).unwrap().sql.starts_with("SELECT * FROM "));
        // A newly added domain invalidates the cached wildcard shape and uses
        // the conversion projection, retaining its typed value.
        driver.client.batch_execute("ALTER TABLE browse_other ADD COLUMN score pg_temp.browse_domain DEFAULT 5").unwrap();
        let converted = driver.browse(&other).unwrap();
        assert!(converted.sql.contains("CAST("), "{}", converted.sql);
        assert_eq!(converted.grid.rows, [vec![Value::Integer(42), Value::Integer(5)]]);
        driver.client.batch_execute("ALTER TABLE browse_other DROP COLUMN score").unwrap();
        assert!(driver.browse(&other).unwrap().sql.starts_with("SELECT * FROM "));
        assert_eq!(driver.browse_catalog.len(), 2);
        assert_eq!(
            driver.browse(&request).unwrap().grid.rows,
            initial.grid.rows
        );
        // Exercise bounded LRU eviction without retaining any table data.
        for index in 0..17 {
            let table = format!("browse_lru_{index}");
            driver
                .client
                .batch_execute(&format!("CREATE TEMP TABLE {table}(id integer)"))
                .unwrap();
            driver
                .browse(&Browse {
                    table,
                    ..Default::default()
                })
                .unwrap();
        }
        assert_eq!(driver.browse_catalog.len(), 16);
        assert!(
            !driver
                .browse_catalog
                .iter()
                .any(|(name, _)| name == "browse_refresh")
        );
        assert_eq!(
            driver.browse(&request).unwrap().grid.rows,
            initial.grid.rows
        );

        driver.client.batch_execute("ALTER TABLE browse_refresh ADD COLUMN extra bigint DEFAULT 7; DROP INDEX browse_expression; DROP TRIGGER browse_touch ON browse_refresh").unwrap();
        let refreshed = driver.browse(&request).unwrap();
        assert_eq!(refreshed.columns.len(), 5);
        assert_eq!(refreshed.grid.rows[0][4], Value::Integer(7));
        let (indexes, triggers) = driver.table_details(&request.table).unwrap();
        assert_eq!(indexes.len(), 1);
        assert!(triggers.is_empty());
        // Repeated reads validate the catalog without an extra network wait.
        assert_eq!(
            driver.browse(&request).unwrap().grid.rows,
            refreshed.grid.rows
        );
        // Stale projections can fail before yielding a RowStream. Keep the
        // metadata response so a dropped column still recovers transparently.
        driver.client.batch_execute("ALTER TABLE browse_refresh DROP COLUMN extra; ALTER TABLE browse_refresh RENAME COLUMN mood TO state").unwrap();
        let renamed = driver.browse(&request).unwrap();
        assert_eq!(renamed.grid.columns, ["id", "label", "state", "numbers"]);
        assert_eq!(renamed.grid.rows[0][2], Value::Text("calm".into()));
        driver
            .client
            .batch_execute("ALTER TABLE browse_refresh ADD COLUMN added integer DEFAULT 9")
            .unwrap();
        request.sort = Some(("added".into(), false));
        let added = driver.browse(&request).unwrap();
        assert_eq!(added.grid.rows[0][4], Value::Integer(9));
        request.sort = None;
        request.filter_column = Some("label".into());
        request.filter = "not present".into();
        let empty = driver.browse(&request).unwrap();
        assert!(empty.grid.rows.is_empty());
        assert_eq!(empty.grid.columns, added.grid.columns);
        driver
            .client
            .batch_execute("BEGIN; UPDATE browse_refresh SET label='pending'; ALTER TABLE browse_refresh RENAME COLUMN state TO transaction_state")
            .unwrap();
        request.filter_column = None;
        let pending = driver.browse(&request).unwrap();
        assert_eq!(pending.grid.rows[0][1], Value::Text("pending".into()));
        assert_eq!(pending.grid.columns[2], "transaction_state");
        assert_eq!(driver.client.transaction_status(), Some(b'T'));
        driver.client.batch_execute("ROLLBACK").unwrap();
        assert_eq!(
            driver.browse(&request).unwrap().grid.rows[0][1],
            Value::Text("original".into())
        );
        driver
            .client
            .batch_execute("DROP TABLE browse_refresh")
            .unwrap();
        assert!(
            driver
                .browse(&request)
                .unwrap_err()
                .contains("Table no longer exists")
        );
    }
    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn pipelined_browse_cancellation_recovers_connection() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = PostgresDriver::open(
            &crate::connection::PostgresConfig {
                host: "127.0.0.1".into(),
                port: 54329,
                database: "rustable_qa".into(),
                user: "postgres".into(),
                password: "rustable_qa_only".into(),
                tls: false,
                ssh: None,
            },
            cancel.clone(),
        )
        .unwrap();
        driver
            .client
            .batch_execute(
                "CREATE TEMP VIEW browse_cancel AS SELECT 1 AS id, pg_sleep(1)::text AS delay",
            )
            .unwrap();
        let request = Browse {
            table: "browse_cancel".into(),
            ..Default::default()
        };
        driver.browse(&request).unwrap();
        assert!(!driver.browse_catalog.is_empty());
        assert_eq!(driver.client.transaction_status(), Some(b'I'));
        let flag = cancel.clone();
        let trigger = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            flag.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        assert!(driver.browse(&request).is_err());
        trigger.join().unwrap();
        assert!(started.elapsed() < Duration::from_millis(900));
        cancel.store(false, Ordering::Relaxed);
        driver
            .client
            .batch_execute(
                "DROP VIEW browse_cancel; CREATE TEMP VIEW browse_cancel AS SELECT 2 AS id",
            )
            .unwrap();
        let recovered = driver.browse(&request).unwrap();
        assert_eq!(recovered.grid.columns, ["id"]);
        assert_eq!(recovered.grid.rows, [vec![Value::Integer(2)]]);
    }
}
