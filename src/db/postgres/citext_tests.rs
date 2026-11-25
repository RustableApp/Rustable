use super::*;

#[test]
fn citext_scalar_domain_and_array_are_bounded_text() {
    use ::postgres::types::Kind;
    // Extension OIDs and schemas vary between databases.
    let ty = Type::new("citext".into(), 50001, Kind::Simple, "extensions".into());
    let domain = Type::new(
        "email".into(),
        50002,
        Kind::Domain(ty.clone()),
        "public".into(),
    );
    let text = "MiXeD café 東京";
    for ty in [&ty, &domain] {
        assert!(native_display_type(ty));
        assert_eq!(
            PgValue::from_sql(ty, text.as_bytes()).unwrap().0,
            Value::Text(text.into())
        );
        assert_eq!(
            PgValue::decode_bounded(ty, text.as_bytes(), text.len()).unwrap(),
            Some(Value::Text(text.into()))
        );
        assert_eq!(
            PgValue::decode_bounded(ty, text.as_bytes(), text.len() - 1).unwrap(),
            None
        );
        assert!(PgValue::from_sql(ty, &[255]).is_err());
    }
    let array = Type::new(
        "_citext".into(),
        50003,
        Kind::Array(ty.clone()),
        "extensions".into(),
    );
    let mut wire = Vec::new();
    for n in [1_i32, 1, ty.oid() as i32, 3, 1] {
        wire.extend(n.to_be_bytes());
    }
    for value in [Some(text.as_bytes()), Some(&b""[..]), None] {
        wire.extend(value.map_or(-1, |v| v.len() as i32).to_be_bytes());
        if let Some(value) = value {
            wire.extend(value);
        }
    }
    let expected = format!("{{\"{text}\",\"\",NULL}}");
    assert_eq!(
        PgValue::from_sql(&array, &wire).unwrap().0,
        Value::Text(expected.clone())
    );
    assert_eq!(
        PgValue::decode_bounded(&array, &wire, expected.len()).unwrap(),
        Some(Value::Text(expected.clone()))
    );
    assert_eq!(
        PgValue::decode_bounded(&array, &wire, expected.len() - 1).unwrap(),
        None
    );
    let opaque = Type::new("opaque".into(), 50004, Kind::Simple, "public".into());
    assert!(!native_display_type(&opaque));
    assert_eq!(
        PgValue::from_sql(&opaque, b"text-looking bytes").unwrap().0,
        Value::Blob(b"text-looking bytes".to_vec())
    );
}

#[test]
#[ignore = "requires opted-in disposable PostgreSQL on port 54329"]
fn live_citext_cold_warm_domains_arrays_and_nulls() {
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
    driver.client.batch_execute("CREATE EXTENSION IF NOT EXISTS citext;
        CREATE DOMAIN pg_temp.case_text AS citext;
        CREATE TEMP TABLE citext_values(id integer PRIMARY KEY, label citext, wrapped pg_temp.case_text, labels citext[], blank citext, absent citext);
        INSERT INTO citext_values VALUES (1, 'MiXeD café 東京', 'Case', ARRAY['One',NULL,'']::citext[], '', NULL);").unwrap();
    let sql = "SELECT * FROM citext_values ORDER BY id";
    for _ in 0..2 {
        let report = driver.execute(sql, true, &cancel);
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(
            report.grids[0].rows[0],
            vec![
                Value::Integer(1),
                Value::Text("MiXeD café 東京".into()),
                Value::Text("Case".into()),
                Value::Text(r#"{"One",NULL,""}"#.into()),
                Value::Text(String::new()),
                Value::Null
            ]
        );
    }
    let empty = driver.execute("SELECT * FROM citext_values WHERE FALSE", true, &cancel);
    assert!(empty.error.is_none(), "{:?}", empty.error);
    assert!(empty.grids[0].rows.is_empty());
    assert_eq!(empty.grids[0].columns.len(), 6);
    let transaction = driver.execute(
        "BEGIN; SELECT label FROM citext_values; ROLLBACK;",
        false,
        &cancel,
    );
    assert!(transaction.error.is_none(), "{:?}", transaction.error);
    assert!(transaction.grids.iter().any(|grid| {
        grid.rows
            .first()
            .is_some_and(|row| row[0] == Value::Text("MiXeD café 東京".into()))
    }));
}
