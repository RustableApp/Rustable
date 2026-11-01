use super::*;

#[test]
fn payload_budget_precedes_large_copy_or_text_validation() {
    assert!(
        PgValue::decode_bounded(&Type::TEXT, &[255; 1024], 10)
            .unwrap()
            .is_none()
    );
    assert!(PgValue::decode_bounded(&Type::TEXT, &[255; 10], 10).is_err());
    assert_eq!(
        PgValue::decode_bounded(&Type::TEXT, b"hello", 5).unwrap(),
        Some(Value::Text("hello".into()))
    );
    assert!(
        PgValue::decode_bounded(&Type::BYTEA, &[0; 1024], 10)
            .unwrap()
            .is_none()
    );
    assert!(
        PgValue::decode_bounded(&Type::INTERVAL, &[0; 1024], 10)
            .unwrap()
            .is_none()
    );
    assert!(
        PgValue::decode_bounded(&Type::JSONB, b"\x01{}", 2)
            .unwrap()
            .is_some()
    );
}

#[test]
fn numeric_expansion_uses_decoded_length_before_allocation() {
    // One base10000 digit but 4,001 decimal integer places.
    let raw = [0, 1, 3, 232, 0, 0, 0, 0, 0, 1];
    assert!(numeric_bounded(&raw, 4000).unwrap().is_none());
    assert_eq!(numeric_bounded(&raw, 4001).unwrap().unwrap().len(), 4001);
    let fractional = [0, 0, 0, 0, 0, 0, 0, 9];
    assert_eq!(
        numeric_bounded(&fractional, 11).unwrap(),
        Some("0.000000000".into())
    );
    assert!(numeric_bounded(&fractional, 10).unwrap().is_none());
}

#[test]
#[ignore = "requires opted-in disposable PostgreSQL QA server on port54329"]
fn collector_rejects_oversize_rows_and_retains_exact_budget_prefix() {
    assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
    let mut client = Client::connect(
        "host=127.0.0.1 port=54329 dbname=rustable_qa user=postgres password=rustable_qa_only",
        NoTls,
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let overhead = std::mem::size_of::<Value>();
    let mut run = |sql: &str, budget| {
        let statement = client.prepare(sql).unwrap();
        let columns = statement
            .columns()
            .iter()
            .map(|c| c.name().to_owned())
            .collect();
        collect(
            client
                .query_raw(&statement, std::iter::empty::<&(dyn ToSql + Sync)>())
                .unwrap(),
            columns,
            5000,
            budget,
            &cancel,
        )
        .unwrap()
    };
    for sql in [
        "SELECT repeat('x', 20000000)",
        "SELECT decode(repeat('ff',20000000),'hex')",
        "SELECT ARRAY[repeat(chr(92),10000000)]",
    ] {
        let grid = run(sql, BYTE_LIMIT);
        assert!(grid.truncated && grid.rows.is_empty());
    }
    let exact = run("SELECT 'hello'", overhead + 5);
    assert!(!exact.truncated && exact.rows.len() == 1);
    assert!(run("SELECT 'hello'", overhead + 4).truncated);
    let prefix = run(
        "SELECT CASE WHEN i=1 THEN 'ok' ELSE repeat('x',1000) END FROM generate_series(1,3) i",
        overhead + 2,
    );
    assert!(prefix.truncated);
    assert_eq!(prefix.rows, vec![vec![Value::Text("ok".into())]]);
    let row = client
        .query_one("SELECT NULL::text, 'abc'::text", &[])
        .unwrap();
    assert!(
        decode_row_bounded(&row, overhead * 2 + 2)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        decode_row_bounded(&row, overhead * 2 + 3).unwrap(),
        Some(vec![Value::Null, Value::Text("abc".into())])
    );
    assert_eq!(
        client.query_one("SELECT 42", &[]).unwrap().get::<_, i32>(0),
        42
    );
}
