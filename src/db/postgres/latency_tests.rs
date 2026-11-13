use super::*;

#[test]
#[ignore = "manual latency measurement against opted-in disposable PostgreSQL QA"]
fn query_completion_latency() {
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
    let mut samples = Vec::new();
    for index in 0..55 {
        let start = Instant::now();
        let report = driver.execute("SELECT 42", true, &cancel);
        assert!(report.error.is_none(), "{:?}", report.error);
        if index >= 5 {
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    samples.sort_by(f64::total_cmp);
    eprintln!(
        "PostgreSQL SELECT42 roundtrip (50 samples,5 warmups): p50={:.3}ms p95={:.3}ms p99={:.3}ms",
        samples[24], samples[47], samples[49]
    );
}

/// Measure real PostgreSQL traffic under controlled WAN latency, without touching
/// a user's server. Each direction adds15ms to a local disposable connection.
#[test]
#[ignore = "opted-in disposable PostgreSQL latency benchmark"]
fn editor_wan_latency_comparison() {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let proxy = std::thread::spawn(move || {
        let (downstream, _) = listener.accept().unwrap();
        let upstream = TcpStream::connect("127.0.0.1:54329").unwrap();
        upstream.set_nodelay(true).unwrap();
        downstream.set_nodelay(true).unwrap();
        fn forward(mut source: TcpStream, mut target: TcpStream) {
            // Delay arrival, not every sequential read: packets already in
            // flight must overlap just as they do on a real network link.
            let shutdown = source.try_clone().unwrap();
            let (send, receive) = std::sync::mpsc::sync_channel(64);
            let reader = std::thread::spawn(move || {
                let mut bytes = [0u8; 65536];
                while let Ok(length) = source.read(&mut bytes) {
                    if length == 0 {
                        break;
                    }
                    let deadline = Instant::now() + Duration::from_millis(15);
                    if send.send((deadline, bytes[..length].to_vec())).is_err() {
                        break;
                    }
                }
            });
            while let Ok((deadline, bytes)) = receive.recv() {
                std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                if target.write_all(&bytes).is_err() {
                    break;
                }
            }
            drop(receive);
            let _ = shutdown.shutdown(std::net::Shutdown::Both);
            let _ = target.shutdown(std::net::Shutdown::Both);
            reader.join().unwrap();
        }

        let reverse_source = upstream.try_clone().unwrap();
        let reverse_target = downstream.try_clone().unwrap();
        let reverse = std::thread::spawn(move || forward(reverse_source, reverse_target));
        forward(downstream, upstream);
        reverse.join().unwrap();
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let mut driver = PostgresDriver::open(
        &crate::connection::PostgresConfig {
            host: "127.0.0.1".into(),
            port,
            database: "rustable_qa".into(),
            user: "postgres".into(),
            password: "rustable_qa_only".into(),
            tls: crate::connection::PostgresTlsMode::Disable,
            ssh: None,
        },
        cancel.clone(),
    )
    .unwrap();
    driver.client.batch_execute("CREATE TEMP TABLE latency_address(id bigint PRIMARY KEY, address text); INSERT INTO latency_address VALUES(2,'latency fixture')").unwrap();
    let sql = "SELECT * FROM latency_address WHERE id = 2";
    let mut legacy = Vec::new();
    let mut current = Vec::new();
    let mut single = Vec::new();
    let mut describe_first = Vec::new();
    let mut readback_times = Vec::new();
    let readback = RowReadback {
        table: "latency_address".into(), columns: vec!["id".into(), "address".into()],
        originals: vec![vec![Value::Integer(2), Value::Text("latency fixture".into())]],
        sql: "SELECT 0 AS __rustable_row, latency_address.* FROM latency_address WHERE id=$1::text::bigint".into(),
        parameters: vec![Value::Text("2".into())],
    };
    for index in 0..9 {
        let timer = Instant::now();
        let mut tx = driver
            .client
            .build_transaction()
            .read_only(true)
            .start()
            .unwrap();
        tx.batch_execute("SET LOCAL standard_conforming_strings = on")
            .unwrap();
        let prepared = tx.prepare(sql).unwrap();
        let columns = prepared
            .columns()
            .iter()
            .map(|c| c.name().to_owned())
            .collect();
        let grid = collect(
            tx.query_raw(&prepared, std::iter::empty::<&(dyn ToSql + Sync)>())
                .unwrap(),
            columns,
            QUERY_LIMIT,
            BYTE_LIMIT,
            &cancel,
        )
        .unwrap();
        assert_eq!(grid.rows.len(), 1);
        tx.commit().unwrap();
        let old_ms = timer.elapsed().as_secs_f64() * 1000.0;
        let timer = Instant::now();
        let rows = driver.client.query_typed_raw_with_prefix(
            sql, std::iter::empty::<(&(dyn ToSql + Sync), Type)>(),
            &["SET LOCAL standard_conforming_strings = on", "SET LOCAL transaction_read_only = on"],
        ).unwrap();
        let columns = rows.columns().iter().map(|column| column.name().to_owned()).collect();
        let previous = collect(rows, columns, QUERY_LIMIT, BYTE_LIMIT, &cancel).unwrap();
        assert_eq!(previous.rows, grid.rows);
        let describe_ms = timer.elapsed().as_secs_f64() * 1000.0;
        let timer = Instant::now();
        let result = driver.execute(sql, true, &cancel);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.grids[0].rows, grid.rows);
        let new_ms = timer.elapsed().as_secs_f64() * 1000.0;
        let timer = Instant::now();
        // Simple Query is genuinely one exchange. query_typed_raw now uses
        // Describe then Execute, so it is not a single-round-trip baseline.
        let messages = driver.client.simple_query(sql).unwrap();
        assert_eq!(messages.iter().filter(|message| matches!(message,
            ::postgres::SimpleQueryMessage::Row(_))).count(), 1);
        let wire_ms = timer.elapsed().as_secs_f64() * 1000.0;
        let timer = Instant::now();
        let refreshed = driver.readback(&readback, BYTE_LIMIT).unwrap();
        assert_eq!(&refreshed.rows[0][1..], grid.rows[0].as_slice());
        let readback_ms = timer.elapsed().as_secs_f64() * 1000.0;
        if index >= 2 {
            readback_times.push(readback_ms);
            legacy.push(old_ms);
            current.push(new_ms);
            single.push(wire_ms);
            describe_first.push(describe_ms);
        }
    }
    for values in [&mut legacy, &mut current, &mut single, &mut describe_first] {
        values.sort_by(f64::total_cmp);
    }
    eprintln!(
        "30ms simulated RTT, indexed lookup,7samples: legacy p50={:.1}ms p95={:.1}ms; optimized p50={:.1}ms p95={:.1}ms; single-exchange reference p50={:.1}ms; describe-first p50={:.1}ms",
        legacy[3], legacy[6], current[3], current[6], single[3], describe_first[3]
    );
    readback_times.sort_by(f64::total_cmp);
    eprintln!("Bound affected-row readback p50={:.1}ms p95={:.1}ms", readback_times[3], readback_times[6]);
    assert!(readback_times[3] < single[3] * 1.5, "saved-row readback added a second exchange");
    assert!(current[3] < describe_first[3] * 0.75, "editor still pays a description round trip");
    assert!(
        current[3] < legacy[3] * 0.8,
        "round-trip reduction must lower measured latency"
    );
    // Complete disk-backed results retain one exchange, even beyond the former
    // 5,000-row ceiling. Compare the same 6,000 values over the delayed link.
    let mut spooled = Vec::new();
    let mut reference = Vec::new();
    for index in 0..9 {
        let sql = "SELECT generate_series(1,6000) AS id";
        let start = Instant::now();
        let baseline = driver.client.simple_query(sql).unwrap();
        assert_eq!(
            baseline
                .iter()
                .filter(|m| matches!(m, ::postgres::SimpleQueryMessage::Row(_)))
                .count(),
            6000
        );
        let baseline_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let rows = driver
            .client
            .query_typed_raw_spooled_with_prefix(
                sql,
                std::iter::empty::<(&(dyn ToSql + Sync), Type)>(),
                &[
                    "SET LOCAL standard_conforming_strings = on",
                    "SET LOCAL transaction_read_only = on",
                ],
                1024,
            )
            .unwrap();
        let grid = collect_complete(rows, vec!["id".into()], 1024, &cancel).unwrap();
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        assert!(grid.is_stored());
        assert_eq!(grid.row_count(), 6000);
        let (_, last) = grid
            .stored
            .as_ref()
            .unwrap()
            .page(5999, false, &cancel)
            .unwrap();
        assert_eq!(last[0][0], Value::Integer(6000));
        if index >= 2 {
            spooled.push(elapsed);
            reference.push(baseline_ms);
        }
    }
    spooled.sort_by(f64::total_cmp);
    reference.sort_by(f64::total_cmp);
    eprintln!(
        "30ms simulated RTT, complete 6000 rows,7samples: spooled p50={:.1}ms p95={:.1}ms; single-exchange reference p50={:.1}ms p95={:.1}ms",
        spooled[3], spooled[6], reference[3], reference[6]
    );
    assert!(
        spooled[3] < reference[3] + 25.0,
        "disk-backed results added an exchange or excessive local overhead"
    );
    drop(driver);
    proxy.join().unwrap();
}

#[test]
#[ignore = "opted-in disposable PostgreSQL latency benchmark"]
fn browse_wan_latency_comparison() {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
    let ssh_directory = std::env::var_os("RUSTABLE_SSH_QA_DIR");
    let upstream_port = if ssh_directory.is_some() { 54328 } else { 54329 };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let proxy = std::thread::spawn(move || {
        let (downstream, _) = listener.accept().unwrap();
        let upstream = TcpStream::connect(("127.0.0.1", upstream_port)).unwrap();
        upstream.set_nodelay(true).unwrap();
        downstream.set_nodelay(true).unwrap();
        fn forward(mut source: TcpStream, mut target: TcpStream) {
            // Delay arrival, not every sequential read: packets already in
            // flight must overlap just as they do on a real network link.
            let shutdown = source.try_clone().unwrap();
            let (send, receive) = std::sync::mpsc::sync_channel(64);
            let reader = std::thread::spawn(move || {
                let mut bytes = [0u8; 65536];
                while let Ok(length) = source.read(&mut bytes) {
                    if length == 0 {
                        break;
                    }
                    let deadline = Instant::now() + Duration::from_millis(15);
                    if send.send((deadline, bytes[..length].to_vec())).is_err() {
                        break;
                    }
                }
            });
            while let Ok((deadline, bytes)) = receive.recv() {
                std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                if target.write_all(&bytes).is_err() {
                    break;
                }
            }
            drop(receive);
            let _ = shutdown.shutdown(std::net::Shutdown::Both);
            let _ = target.shutdown(std::net::Shutdown::Both);
            reader.join().unwrap();
        }

        let reverse_source = upstream.try_clone().unwrap();
        let reverse_target = downstream.try_clone().unwrap();
        let reverse = std::thread::spawn(move || forward(reverse_source, reverse_target));
        forward(downstream, upstream);
        reverse.join().unwrap();
    });
    // Optional local SSH fixture exercises the real tunnel implementation too.
    let known_hosts = tempfile::NamedTempFile::new().unwrap();
    let ssh = ssh_directory.map(|directory| {
        let directory = std::path::PathBuf::from(directory);
        assert!(directory.starts_with("/tmp"));
        std::fs::write(known_hosts.path(), format!("[127.0.0.1]:{port} {}", std::fs::read_to_string(directory.join("host.pub")).unwrap())).unwrap();
        crate::ssh_tunnel::Config {
            host: "127.0.0.1".into(),
            port,
            user: std::env::var("USER").unwrap(),
            identity_file: Some(directory.join("client")),
            known_hosts_file: Some(known_hosts.path().to_owned()),
            config_file: Some("/dev/null".into()),
            remote_host: "127.0.0.1".into(),
            remote_port: 54329,
        }
    });
    eprintln!("browse transport: {}", if ssh.is_some() { "SSH over delayed TCP" } else { "delayed TCP" });
    let cancel = Arc::new(AtomicBool::new(false));
    let mut driver = PostgresDriver::open(
        &crate::connection::PostgresConfig {
            host: "127.0.0.1".into(),
            port: if ssh.is_some() { 54329 } else { port },
            database: "rustable_qa".into(),
            user: "postgres".into(),
            password: "rustable_qa_only".into(),
            tls: crate::connection::PostgresTlsMode::Disable,
            ssh,
        },
        cancel.clone(),
    )
    .unwrap();
    driver.client.batch_execute("CREATE TEMP TABLE latency_address(id bigint PRIMARY KEY, address text); INSERT INTO latency_address VALUES(2,'latency fixture')").unwrap();
    let request = Browse {
        table: "latency_address".into(),
        ..Default::default()
    };
    let mut legacy = Vec::new();
    let mut current = Vec::new();
    let mut sequential = Vec::new();
    let mut revisited = Vec::new();
    let mut cold = Vec::new();
    let mut baseline = Vec::new();
    driver.client.batch_execute("CREATE TEMP TABLE latency_other(id integer)").unwrap();
    for index in 0..9 {
        let timer = Instant::now();
        let old = legacy_browse(&mut driver, &request).unwrap();
        let old_ms = timer.elapsed().as_secs_f64() * 1000.0;
        driver.browse_catalog.clear();
        driver.browse_hints.clear();
        let timer = Instant::now();
        let first = driver.browse(&request).unwrap();
        let first_ms = timer.elapsed().as_secs_f64() * 1000.0;
        // Sidebar column hints exist before a normal first open; clear the
        // browse catalog to prove that this does not depend on a previous page.
        driver.browse_catalog.clear();
        let timer = Instant::now();
        let cold_page = driver.browse(&request).unwrap();
        let cold_ms = timer.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(cold_page.grid.rows, first.grid.rows);
        let timer = Instant::now();
        driver.client.simple_query("SELECT * FROM latency_address ORDER BY id LIMIT 501 OFFSET 0").unwrap();
        let baseline_ms = timer.elapsed().as_secs_f64() * 1000.0;
        let timer = Instant::now();
        let new = driver.browse(&request).unwrap();
        let new_ms = timer.elapsed().as_secs_f64() * 1000.0;
        driver.browse(&Browse { table: "latency_other".into(), ..Default::default() }).unwrap();
        let timer = Instant::now();
        let revisit = driver.browse(&request).unwrap();
        let revisit_ms = timer.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(revisit.grid.rows, new.grid.rows);
        assert_eq!(first.grid.rows, new.grid.rows);
        assert_eq!(old.grid.rows, new.grid.rows);
        assert_eq!(old.grid.columns, new.grid.columns);
        assert_eq!(old.columns, new.columns);
        let (indexes, triggers) = driver.table_details(&request.table).unwrap();
        assert_eq!(old.indexes.len(), indexes.len());
        assert_eq!(old.triggers.len(), triggers.len());
        if index >= 2 {
            cold.push(cold_ms);
            baseline.push(baseline_ms);
            legacy.push(old_ms);
            current.push(new_ms);
            sequential.push(first_ms);
            revisited.push(revisit_ms);
        }
    }
    cold.sort_by(f64::total_cmp);
    baseline.sort_by(f64::total_cmp);
    revisited.sort_by(f64::total_cmp);
    sequential.sort_by(f64::total_cmp);
    legacy.sort_by(f64::total_cmp);
    current.sort_by(f64::total_cmp);
    eprintln!(
        "30ms simulated RTT browse/refresh7samples: legacy p50={:.1}ms p95={:.1}ms; optimized p50={:.1}ms p95={:.1}ms",
        legacy[3], legacy[6], current[3], current[6]
    );
    assert!(
        current[3] < legacy[3] * 0.5,
        "fresh metadata plus data should remove serial RTTs"
    );
    eprintln!(
        "fresh sequential p50={:.1}ms p95={:.1}ms; validated pipeline p50={:.1}ms p95={:.1}ms",
        sequential[3], sequential[6], current[3], current[6]
    );
    assert!(
        current[3] < sequential[3] * 0.8,
        "pipelined refresh must remove a network wait"
    );
    eprintln!("table revisit p50={:.1}ms p95={:.1}ms", revisited[3], revisited[6]);
    assert!(revisited[3] < sequential[3] * 0.8, "table revisits must retain the pipeline benefit");
    eprintln!("first sidebar table load p50={:.1}ms p95={:.1}ms; one-exchange reference p50={:.1}ms", cold[3], cold[6], baseline[3]);
    assert!(cold[3] < sequential[3] * 0.8, "first sidebar load must remove the metadata wait");
    assert!(cold[3] < baseline[3] * 1.5, "first sidebar load must remain near one exchange");
    // Idle requests exercise a different transport pattern from burst refreshes.
    // Compare the complete browse operation with a genuine one-exchange query
    // after the same pause; do not attribute shared network idle latency to SQL.
    for pause_ms in [0, 1000, 3000] {
        let mut browse_samples = Vec::new();
        let mut reference_samples = Vec::new();
        for _ in 0..3 {
            std::thread::sleep(Duration::from_millis(pause_ms));
            let timer = Instant::now();
            let page = driver.browse(&request).unwrap();
            browse_samples.push(timer.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(page.grid.rows[0][0], Value::Integer(2));
            std::thread::sleep(Duration::from_millis(pause_ms));
            let timer = Instant::now();
            driver.client.simple_query("SELECT * FROM latency_address ORDER BY id LIMIT 501 OFFSET 0").unwrap();
            reference_samples.push(timer.elapsed().as_secs_f64() * 1000.0);
        }
        browse_samples.sort_by(f64::total_cmp);
        reference_samples.sort_by(f64::total_cmp);
        eprintln!("after {pause_ms}ms idle: browse={browse_samples:?}ms; one-exchange reference={reference_samples:?}ms");
        assert!(browse_samples[1] < reference_samples[1] * 1.5,
            "idle browse should remain near the reference without adding exchanges");
    }
    drop(driver);
    proxy.join().unwrap();
}

fn legacy_browse(driver: &mut PostgresDriver, request: &Browse) -> Result<TableData, String> {
    driver.check_tunnel()?;
    let _watch = driver.watch();
    let started = Instant::now();
    // Only catalogue-returned names enter SQL; user input never becomes an identifier unchecked.
    let rows = driver.client.query("SELECT n.nspname,c.relname,CASE WHEN c.relkind IN ('r','p','m') THEN c.reltuples::double precision ELSE NULL END FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE c.oid=pg_catalog.to_regclass($1) AND c.relkind IN ('r','p','v','m','f')", &[&request.table]).map_err(error)?;
    let row = rows.first().ok_or("Table no longer exists")?;
    let table = format!("{}.{}", quote(row.get(0)), quote(row.get(1)));
    let estimate = row
        .get::<_, Option<f64>>(2)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .filter(|_| request.filter_column.is_none())
        .map(|value| value.round() as u64);
    let columns = driver.columns(&table)?;
    // Preparing the shape fetches PostgreSQL's actual type metadata without
    // reading rows. Unsupported wire encodings use the server's lossless
    // text output (arrays, ranges, intervals, composite/extension types).
    let shape = driver
        .client
        .prepare(&format!("SELECT * FROM {table} LIMIT 0"))
        .map_err(error)?;
    let projection = shape
        .columns()
        .iter()
        .map(|column| {
            let source = format!("{table}.{}", quote(column.name()));
            if native_display_type(column.type_()) {
                source
            } else {
                format!(
                    "CAST({source} AS pg_catalog.text) AS {}",
                    quote(column.name())
                )
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
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
    let sort = request.sort.clone().or_else(|| {
        columns
            .iter()
            .find(|c| c.pk)
            .map(|c| (c.name.clone(), false))
    });
    if let Some((column, descending)) = sort {
        if !columns.iter().any(|c| c.name == column) {
            return Err("Sort column no longer exists".into());
        }
        sql.push_str(&format!(
            " ORDER BY {table}.{} {}",
            quote(&column),
            if descending { "DESC" } else { "ASC" }
        ));
    }
    let limit = (PAGE_SIZE + 1) as i64;
    let offset = i64::try_from(request.offset).map_err(|_| "Page offset is too large")?;
    sql.push_str(&format!(
        " LIMIT ${} OFFSET ${}",
        parameters.len() + 1,
        parameters.len() + 2
    ));
    let mut values: Vec<&(dyn ToSql + Sync)> = Vec::new();
    if request.filter_column.is_some() {
        values.push(&request.filter);
    }
    values.push(&limit);
    values.push(&offset);
    parameters.push(Value::Integer(limit));
    parameters.push(Value::Integer(offset));
    let prepared = driver.client.prepare(&sql).map_err(error)?;
    let names = prepared
        .columns()
        .iter()
        .map(|c| c.name().to_owned())
        .collect();
    let mut grid = collect(
        driver.client.query_raw(&prepared, values).map_err(error)?,
        names,
        PAGE_SIZE,
        BYTE_LIMIT,
        &driver.cancel,
    )?;
    grid.set_browse_total(request.offset, estimate);
    grid.elapsed = started.elapsed();
    let index_rows=driver.client.query("SELECT ci.relname,i.indisunique,i.indisprimary,i.indpred IS NOT NULL,pg_catalog.pg_get_indexdef(i.indexrelid), ARRAY(SELECT pg_catalog.pg_get_indexdef(i.indexrelid,k,true) FROM pg_catalog.generate_series(1,i.indnkeyatts) AS k), EXISTS(SELECT 1 FROM pg_catalog.pg_constraint con WHERE con.conindid=i.indexrelid AND con.contype IN ('u','x')) FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class ci ON ci.oid=i.indexrelid WHERE i.indrelid=pg_catalog.to_regclass($1) ORDER BY ci.relname", &[&table]).map_err(error)?;
    let indexes = index_rows
        .into_iter()
        .map(|r| Index {
            name: r.get(0),
            unique: r.get(1),
            origin: if r.get::<_, bool>(2) {
                "pk"
            } else if r.get::<_, bool>(6) {
                "u"
            } else {
                "c"
            }
            .into(),
            partial: r.get(3),
            columns: r.get(5),
            ddl: r.get(4),
        })
        .collect();
    let trigger_rows=driver.client.query("SELECT tgname,pg_catalog.pg_get_triggerdef(oid) FROM pg_catalog.pg_trigger WHERE tgrelid=pg_catalog.to_regclass($1) AND NOT tgisinternal ORDER BY tgname", &[&table]).map_err(error)?;
    let triggers = trigger_rows
        .into_iter()
        .map(|r| Object {
            schema: None,
            display_name: None,
            name: r.get(0),
            kind: "trigger".into(),
            ddl: r.get(1),
            columns: Vec::new(),
            metadata_error: None,
        })
        .collect();
    Ok(TableData { details_loaded: true,
        foreign_keys: vec![],
        sql,
        parameters,
        grid,
        columns,
        indexes,
        triggers,
    })
}

#[test]
#[ignore = "opted-in disposable PostgreSQL catalog latency benchmark"]
fn catalog_wan_latency_comparison() {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let proxy = std::thread::spawn(move || {
        let (downstream, _) = listener.accept().unwrap();
        let upstream = TcpStream::connect("127.0.0.1:54329").unwrap();
        upstream.set_nodelay(true).unwrap();
        downstream.set_nodelay(true).unwrap();
        fn forward(mut source: TcpStream, mut target: TcpStream) {
            // Delay arrival, not every sequential read: packets already in
            // flight must overlap just as they do on a real network link.
            let shutdown = source.try_clone().unwrap();
            let (send, receive) = std::sync::mpsc::sync_channel(64);
            let reader = std::thread::spawn(move || {
                let mut bytes = [0u8; 65536];
                while let Ok(length) = source.read(&mut bytes) {
                    if length == 0 {
                        break;
                    }
                    let deadline = Instant::now() + Duration::from_millis(15);
                    if send.send((deadline, bytes[..length].to_vec())).is_err() {
                        break;
                    }
                }
            });
            while let Ok((deadline, bytes)) = receive.recv() {
                std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                if target.write_all(&bytes).is_err() {
                    break;
                }
            }
            drop(receive);
            let _ = shutdown.shutdown(std::net::Shutdown::Both);
            let _ = target.shutdown(std::net::Shutdown::Both);
            reader.join().unwrap();
        }

        let reverse_source = upstream.try_clone().unwrap();
        let reverse_target = downstream.try_clone().unwrap();
        let reverse = std::thread::spawn(move || forward(reverse_source, reverse_target));
        forward(downstream, upstream);
        reverse.join().unwrap();
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let mut driver = PostgresDriver::open(
        &crate::connection::PostgresConfig {
            host: "127.0.0.1".into(),
            port,
            database: "rustable_qa".into(),
            user: "postgres".into(),
            password: "rustable_qa_only".into(),
            tls: crate::connection::PostgresTlsMode::Disable,
            ssh: None,
        },
        cancel.clone(),
    )
    .unwrap();

    driver.client.batch_execute("BEGIN; CREATE SCHEMA rustable_catalog_latency; CREATE TABLE rustable_catalog_latency.item(id integer PRIMARY KEY, name text NOT NULL DEFAULT 'test')").unwrap();
    driver.schemas.selected = Some("rustable_catalog_latency".into());
    let schema = "rustable_catalog_latency";
    let mut legacy = Vec::new();
    let mut current = Vec::new();
    for index in 0..9 {
        let timer = Instant::now();
        let names = driver.client.query("SELECT nspname FROM pg_catalog.pg_namespace WHERE pg_catalog.has_schema_privilege(oid,'USAGE') AND nspname NOT LIKE 'pg_toast%' AND nspname NOT LIKE 'pg_temp_%' ORDER BY nspname", &[]).unwrap();
        assert!(names.iter().any(|r| r.get::<_, &str>(0) == schema));
        let relations = driver.client.query("SELECT c.relname,c.relkind::text FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind IN ('r','p','v','m','f') ORDER BY c.relname", &[&schema]).unwrap();
        let columns = driver.client.query("SELECT c.relname,a.attname,pg_catalog.format_type(a.atttypid,a.atttypmod),a.attnotnull, COALESCE(pg_catalog.pg_get_expr(d.adbin,d.adrelid),''), EXISTS(SELECT 1 FROM pg_catalog.pg_index i WHERE i.indrelid=a.attrelid AND i.indisprimary AND a.attnum=ANY(i.indkey)), (a.attgenerated <> '' OR a.attidentity <> '') FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum WHERE n.nspname=$1 AND c.relkind IN ('r','p','v','m','f') AND a.attnum>0 AND NOT a.attisdropped ORDER BY c.relname,a.attnum", &[&schema]).unwrap();
        let old_ms = timer.elapsed().as_secs_f64() * 1000.0;
        let timer = Instant::now();
        let objects = driver.objects().unwrap();
        let new_ms = timer.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(objects.len(), relations.len());
        assert_eq!(objects[0].display_name(), relations[0].get::<_, &str>(0));
        assert_eq!(objects[0].columns.len(), columns.len());
        for (actual, expected) in objects[0].columns.iter().zip(&columns) {
            assert_eq!(
                actual,
                &Column {
                    name: expected.get(1),
                    data_type: expected.get(2),
                    not_null: expected.get(3),
                    default: expected.get(4),
                    pk: expected.get(5),
                    generated: expected.get(6)
                }
            );
        }
        if index >= 2 {
            legacy.push(old_ms);
            current.push(new_ms);
        }
    }
    legacy.sort_by(f64::total_cmp);
    current.sort_by(f64::total_cmp);
    eprintln!(
        "30ms simulated RTT catalog,7samples: legacy p50={:.1}ms p95={:.1}ms; streamed typed p50={:.1}ms p95={:.1}ms",
        legacy[3], legacy[6], current[3], current[6]
    );
    assert!(current[3] < legacy[3] * 0.65);
    driver.client.batch_execute("ROLLBACK").unwrap();
    drop(driver);
    proxy.join().unwrap();
}
