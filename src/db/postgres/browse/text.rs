//! Text-format browsing avoids both projection casts and result-type lookups.
use super::*;
use ::postgres::SimpleQueryMessage;

pub(super) fn browse(
    driver: &mut PostgresDriver,
    request: &Browse,
    hint: Object,
    started: Instant,
) -> Result<TableData, String> {
    let table = format!(
        "{}.{}",
        quote(hint.schema.as_deref().ok_or("Missing table schema")?),
        quote(hint.display_name.as_deref().ok_or("Missing table name")?)
    );
    let (sql, parameters) = match query(request, &hint.columns, &table, "*") {
        Ok(query) => query,
        Err(_) => {
            driver.browse_hints.retain(|o| o.name != request.table);
            driver
                .browse_catalog
                .retain(|(name, _)| name != &request.table);
            return super::browse(driver, request);
        }
    };
    let (metadata, data) = driver
        .client
        .query_text_with_metadata(
            CATALOG,
            &[(&request.table, Type::TEXT)],
            &sql,
            bind_values(&parameters),
            request.page_size() + 1,
            BYTE_LIMIT,
        )
        .map_err(error)?;
    if driver.cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }
    let row = metadata
        .into_iter()
        .next()
        .ok_or("Table no longer exists")?;
    let raw: Vec<CatalogColumn> =
        serde_json::from_str(row.get(3)).map_err(|e| format!("Invalid column metadata: {e}"))?;
    let columns: Vec<Column> = raw
        .iter()
        .map(
            |(name, data_type, not_null, default, pk, generated, _, _)| Column {
                name: name.clone(),
                data_type: data_type.clone(),
                not_null: *not_null,
                default: default.clone(),
                pk: *pk,
                generated: *generated,
            },
        )
        .collect();
    // The cached sidebar shape is only a query-building hint. Never attach
    // stale editing metadata to rows, including after ALTER TABLE or PK changes.
    let same = Some(row.get::<_, &str>(0)) == hint.schema.as_deref()
        && Some(row.get::<_, &str>(1)) == hint.display_name.as_deref()
        && columns.len() == hint.columns.len()
        && columns
            .iter()
            .zip(&hint.columns)
            .all(|(a, b)| a.name == b.name && a.data_type == b.data_type && a.pk == b.pk);
    if !same {
        driver.browse_hints.retain(|o| o.name != request.table);
        driver
            .browse_catalog
            .retain(|(name, _)| name != &request.table);
        let mut result = super::browse(driver, request)?;
        result.grid.elapsed = started.elapsed();
        return Ok(result);
    }
    let (messages, truncated) = data.map_err(error)?;
    let mut grid = Grid {
        truncated,
        ..Grid::default()
    };
    let mut bytes = 0usize;
    let mut decoded_truncated = false;
    for message in messages {
        match message {
            SimpleQueryMessage::RowDescription(fields) => {
                if fields.len() != raw.len()
                    || fields.iter().zip(&raw).any(|(f, c)| {
                        f.name() != c.0 || (f.type_oid() != c.6 && f.type_oid() != c.7)
                    })
                {
                    return Err(
                        "Table structure changed while loading. Reload the table before editing."
                            .into(),
                    );
                }
                grid.columns = fields.iter().map(|f| f.name().to_owned()).collect();
            }
            SimpleQueryMessage::Row(row) => {
                if decoded_truncated || grid.rows.len() >= request.page_size() {
                    grid.truncated = true;
                    continue;
                }
                let mut values = Vec::with_capacity(raw.len());
                let mut row_bytes = 0usize;
                for (index, column) in raw.iter().enumerate() {
                    let value = decode(row.try_get(index).map_err(error)?, column.7)?;
                    row_bytes = row_bytes.saturating_add(value_size(&value));
                    values.push(value);
                }
                if row_bytes > BYTE_LIMIT.saturating_sub(bytes) {
                    decoded_truncated = true;
                    grid.truncated = true;
                    continue;
                }
                bytes += row_bytes;
                crate::db::progress::decoded_row(&values);
                grid.rows.push(values);
            }
            SimpleQueryMessage::CommandComplete(n) => grid.affected = n as usize,
            _ => {}
        }
    }
    finish(
        driver, request, row, grid, columns, sql, parameters, started,
    )
}

fn decode(value: Option<&str>, oid: u32) -> Result<Value, String> {
    let Some(value) = value else {
        return Ok(Value::Null);
    };
    let invalid = || format!("Invalid PostgreSQL text value for type {oid}");
    Ok(match Type::from_oid(oid) {
        Some(Type::BOOL) => Value::Text(
            match value {
                "t" => "true",
                "f" => "false",
                _ => return Err(invalid()),
            }
            .into(),
        ),
        Some(Type::INT2 | Type::INT4 | Type::INT8 | Type::OID) => {
            Value::Integer(value.parse().map_err(|_| invalid())?)
        }
        Some(Type::FLOAT4) => Value::Real(value.parse::<f32>().map_err(|_| invalid())? as f64),
        Some(Type::FLOAT8) => Value::Real(value.parse().map_err(|_| invalid())?),
        Some(Type::BYTEA) => Value::Blob(bytea(value)?),
        // Numeric stays decimal text; arbitrary extension/array/enum types use
        // the server's own text representation without an information-losing cast.
        _ => Value::Text(value.into()),
    })
}

fn bytea(value: &str) -> Result<Vec<u8>, String> {
    let invalid = || "Invalid PostgreSQL bytea text value".to_owned();
    if let Some(hex) = value.strip_prefix("\\x") {
        if hex.len() % 2 != 0 {
            return Err(invalid());
        }
        return hex
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| {
                let a = (c[0] as char).to_digit(16).ok_or_else(invalid)?;
                let b = (c[1] as char).to_digit(16).ok_or_else(invalid)?;
                Ok((a * 16 + b) as u8)
            })
            .collect();
    }
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            output.push(bytes[i]);
            i += 1;
        } else if bytes.get(i + 1) == Some(&b'\\') {
            output.push(b'\\');
            i += 2;
        } else {
            let digits = bytes.get(i + 1..i + 4).ok_or_else(invalid)?;
            if digits[0] > b'3' || !digits.iter().all(|b| (b'0'..=b'7').contains(b)) {
                return Err(invalid());
            }
            output.push((digits[0] - b'0') * 64 + (digits[1] - b'0') * 8 + (digits[2] - b'0'));
            i += 4;
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn first_sidebar_load_handles_cold_types_empty_results_and_schema_changes() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
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
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        driver.client.batch_execute("CREATE TEMP TABLE init_text(id int); CREATE TYPE pg_temp.text_mood AS ENUM ('calm'); CREATE DOMAIN pg_temp.text_num AS numeric; CREATE TEMP TABLE first_text(id bigint PRIMARY KEY, mood pg_temp.text_mood, amount pg_temp.text_num, tags int[], bytes bytea, missing text); INSERT INTO first_text VALUES (2,'calm',12345678901234567890.123456789,ARRAY[1,2],decode('00ff5c','hex'),NULL)").unwrap();
        let schema: String = driver
            .client
            .query_one(
                "SELECT nspname FROM pg_namespace WHERE oid=pg_my_temp_schema()",
                &[],
            )
            .unwrap()
            .get(0);
        driver.schemas.selected = Some(schema);
        let objects = driver.selected_objects().unwrap();
        let object = objects
            .iter()
            .find(|o| o.display_name() == "first_text")
            .unwrap();
        let mut request = Browse {
            table: object.name.clone(),
            ..Default::default()
        };
        assert!(driver.browse_catalog.is_empty());
        let result = driver.browse(&request).unwrap();
        assert!(result.sql.starts_with("SELECT * FROM "));
        assert!(!result.details_loaded);
        assert_eq!(
            result.grid.rows[0],
            vec![
                Value::Integer(2),
                Value::Text("calm".into()),
                Value::Text("12345678901234567890.123456789".into()),
                Value::Text("{1,2}".into()),
                Value::Blob(vec![0, 255, 92]),
                Value::Null
            ]
        );
        request.offset = 100;
        let empty = driver.browse(&request).unwrap();
        assert!(empty.grid.rows.is_empty());
        assert_eq!(empty.grid.columns, result.grid.columns);
        request.offset = 0;
        driver
            .client
            .batch_execute(
                "SET bytea_output=escape; ALTER TABLE first_text ADD COLUMN extra int DEFAULT 7",
            )
            .unwrap();
        let changed = driver.browse(&request).unwrap();
        assert_eq!(changed.grid.rows[0][4], Value::Blob(vec![0, 255, 92]));
        assert_eq!(changed.grid.rows[0][6], Value::Integer(7));
        driver
            .client
            .batch_execute("BEGIN; ALTER TABLE first_text RENAME COLUMN id TO new_id")
            .unwrap();
        let transaction = driver.browse(&request).unwrap();
        assert_eq!(transaction.grid.columns[0], "new_id");
        assert_eq!(driver.client.transaction_status(), Some(b'T'));
        driver.client.batch_execute("ROLLBACK").unwrap();
        // Oversized text values must drain and leave the connection usable.
        driver
            .client
            .batch_execute("UPDATE first_text SET missing=repeat('x', 17000000)")
            .unwrap();
        let bounded = driver.browse(&request).unwrap();
        assert!(bounded.grid.truncated);
        assert!(bounded.grid.rows.is_empty());
        assert_eq!(
            driver
                .client
                .query_one("SELECT 42", &[])
                .unwrap()
                .get::<_, i32>(0),
            42
        );
    }

    #[test]
    fn text_values_preserve_null_numeric_and_bytea() {
        assert_eq!(decode(None, 25).unwrap(), Value::Null);
        assert_eq!(
            decode(Some("12345678901234567890.123456789"), 1700).unwrap(),
            Value::Text("12345678901234567890.123456789".into())
        );
        assert_eq!(bytea("\\x00ff5c").unwrap(), [0, 255, 92]);
        assert_eq!(bytea("\\000\\377\\\\").unwrap(), [0, 255, 92]);
        assert!(bytea("\\777").is_err());
        assert_eq!(decode(Some("t"), 16).unwrap(), Value::Text("true".into()));
    }
}
