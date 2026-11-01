//! Lossless MySQL/MariaDB wire values for the shared grid.
//! DECIMAL and unsigned integers above i64::MAX remain exact text; never f64.
use super::Value;
use ::mysql::{Column, Value as Wire, consts::ColumnType as Ty};

/// Bind values through the prepared protocol. Exact numeric text remains text
/// until MySQL converts it using the destination column's type.
impl TryFrom<&Value> for Wire {
    type Error = String;
    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        match value {
            Value::Null => Ok(Self::NULL),
            Value::Integer(value) => Ok(Self::Int(*value)),
            Value::Real(value) if value.is_finite() => Ok(Self::Double(*value)),
            Value::Real(_) => Err("MySQL parameters cannot contain NaN or infinity.".into()),
            Value::Text(value) => Ok(Self::Bytes(value.as_bytes().to_vec())),
            Value::Blob(value) => Ok(Self::Bytes(value.clone())),
        }
    }
}

impl TryFrom<(&Column, Wire)> for Value {
    type Error = String;

    fn try_from((column, value): (&Column, Wire)) -> Result<Self, Self::Error> {
        match value {
            Wire::NULL => Ok(Self::Null),
            Wire::Int(value) => Ok(Self::Integer(value)),
            Wire::UInt(value) => Ok(i64::try_from(value).map(Self::Integer)
                .unwrap_or_else(|_| Self::Text(value.to_string()))),
            Wire::Float(value) => Ok(Self::Real(f64::from(value))),
            Wire::Double(value) => Ok(Self::Real(value)),
            Wire::Bytes(bytes) => decode_text(column, bytes),
            Wire::Date(year, month, day, hour, minute, second, micros) => {
                let date = format!("{year:04}-{month:02}-{day:02}");
                if matches!(column.column_type(), Ty::MYSQL_TYPE_DATE | Ty::MYSQL_TYPE_NEWDATE) {
                    Ok(Self::Text(date))
                } else {
                    Ok(Self::Text(format!("{date} {hour:02}:{minute:02}:{second:02}{}", fraction(micros, column.decimals()))))
                }
            }
            Wire::Time(negative, days, hours, minutes, seconds, micros) => {
                let hours = u64::from(days) * 24 + u64::from(hours);
                Ok(Self::Text(format!("{}{hours:02}:{minutes:02}:{seconds:02}{}",
                    if negative { "-" } else { "" }, fraction(micros, column.decimals()))))
            }
        }
    }
}

fn fraction(micros: u32, decimals: u8) -> String {
    let precision = if decimals == 0 && micros != 0 { 6 } else { decimals.min(6) as usize };
    if precision == 0 { String::new() } else {
        let digits = format!("{micros:06}");
        format!(".{}", &digits[..precision])
    }
}

fn decode_text(column: &Column, bytes: Vec<u8>) -> Result<Value, String> {
    use Ty::*;
    let ty = column.column_type();
    let binary = matches!(ty, MYSQL_TYPE_BIT | MYSQL_TYPE_GEOMETRY | MYSQL_TYPE_VECTOR)
        || (column.character_set() == 63 && matches!(ty,
            MYSQL_TYPE_TINY_BLOB | MYSQL_TYPE_MEDIUM_BLOB | MYSQL_TYPE_LONG_BLOB
            | MYSQL_TYPE_BLOB | MYSQL_TYPE_VAR_STRING | MYSQL_TYPE_STRING | MYSQL_TYPE_VARCHAR));
    if binary { return Ok(Value::Blob(bytes)); }
    // Connections must request utf8mb4 results. Invalid text is an error, never
    // silently replaced with U+FFFD (which would corrupt copied/exported values).
    let text = String::from_utf8(bytes).map_err(|_| "MySQL returned invalid UTF-8 text".to_owned())?;
    match ty {
        MYSQL_TYPE_TINY | MYSQL_TYPE_SHORT | MYSQL_TYPE_LONG | MYSQL_TYPE_LONGLONG
        | MYSQL_TYPE_INT24 | MYSQL_TYPE_YEAR => {
            if let Ok(value) = text.parse::<i64>() { return Ok(Value::Integer(value)); }
            if text.parse::<u64>().is_ok() { return Ok(Value::Text(text)); }
            Err("MySQL returned an invalid integer".into())
        }
        MYSQL_TYPE_FLOAT | MYSQL_TYPE_DOUBLE => text.parse::<f64>().map(Value::Real)
            .map_err(|_| "MySQL returned an invalid floating-point value".into()),
        _ => Ok(Value::Text(text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text(ty: Ty, charset: u16, bytes: &[u8]) -> Value {
        Value::try_from((&Column::new(ty).with_character_set(charset), Wire::Bytes(bytes.to_vec()))).unwrap()
    }
    #[test]
    fn text_protocol_preserves_exact_numbers_and_binary_metadata() {
        assert_eq!(text(Ty::MYSQL_TYPE_LONGLONG, 63, b"9007199254740993"), Value::Integer(9007199254740993));
        assert_eq!(text(Ty::MYSQL_TYPE_LONGLONG, 63, b"18446744073709551615"), Value::Text("18446744073709551615".into()));
        assert_eq!(text(Ty::MYSQL_TYPE_NEWDECIMAL, 63, b"12345678901234567890.1234567890"), Value::Text("12345678901234567890.1234567890".into()));
        assert_eq!(text(Ty::MYSQL_TYPE_BLOB, 63, &[0, 255]), Value::Blob(vec![0, 255]));
        assert_eq!(text(Ty::MYSQL_TYPE_BLOB, 45, "日本".as_bytes()), Value::Text("日本".into()));
        assert_eq!(text(Ty::MYSQL_TYPE_JSON, 63, b"{\"a\":1}"), Value::Text("{\"a\":1}".into()));
        assert!(Value::try_from((&Column::new(Ty::MYSQL_TYPE_VAR_STRING).with_character_set(45), Wire::Bytes(vec![255]))).is_err());
    }
    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_text_and_prepared_protocols_preserve_grid_values() {
        use ::mysql::{Conn, OptsBuilder, prelude::Queryable};
        let port: u16 = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let mut conn = Conn::new(OptsBuilder::default().ip_or_hostname(Some("127.0.0.1"))
            .tcp_port(port).user(Some("rustable")).pass(Some("rustable-test-only"))
            .db_name(Some("rustable_test")).prefer_socket(false)).unwrap();
        conn.query_drop("SET NAMES utf8mb4").unwrap();
        let sql = "SELECT CAST('18446744073709551615' AS UNSIGNED) AS big, CAST('12345678901234567890.1234567890' AS DECIMAL(30,10)) AS exact_decimal, '日本' AS unicode_text, X'00FF' AS binary_value, NULL AS empty_value, CAST('-817:59:59.000001' AS TIME(6)) AS duration, CAST('12:34:56' AS TIME(6)) AS fractional_zero";
        let text_row: ::mysql::Row = conn.query_first(sql).unwrap().unwrap();
        let prepared_row: ::mysql::Row = conn.exec_first(sql, ()).unwrap().unwrap();
        let expected = vec![Value::Text("18446744073709551615".into()),
            Value::Text("12345678901234567890.1234567890".into()),
            Value::Text("日本".into()), Value::Blob(vec![0, 255]), Value::Null,
            Value::Text("-817:59:59.000001".into()), Value::Text("12:34:56.000000".into())];
        for row in [text_row, prepared_row] {
            let columns = row.columns_ref().to_vec();
            let values = columns.iter().zip(row.unwrap()).map(Value::try_from)
                .collect::<Result<Vec<_>, _>>().unwrap();
            assert_eq!(values, expected);
        }
        let empty = conn.query_iter("SELECT 1 AS preserved_header WHERE FALSE").unwrap();
        assert_eq!(empty.columns().as_ref()[0].name_str(), "preserved_header");
    }

    #[test]
    fn binary_protocol_preserves_null_unsigned_and_temporal_values() {
        let column = Column::new(Ty::MYSQL_TYPE_DATETIME);
        assert_eq!(Value::try_from((&column, Wire::NULL)).unwrap(), Value::Null);
        assert_eq!(Value::try_from((&column, Wire::UInt(u64::MAX))).unwrap(), Value::Text(u64::MAX.to_string()));
        assert_eq!(Value::try_from((&column, Wire::Date(0, 0, 0, 0, 0, 0, 0))).unwrap(), Value::Text("0000-00-00 00:00:00".into()));
        assert_eq!(Value::try_from((&column, Wire::Date(2026, 11, 1, 12, 34, 56, 123))).unwrap(), Value::Text("2026-11-01 12:34:56.000123".into()));
        assert_eq!(Value::try_from((&column, Wire::Time(true, 34, 22, 59, 59, 1))).unwrap(), Value::Text("-838:59:59.000001".into()));
    }
    #[test]
    fn parameters_reject_nonfinite_numbers_and_preserve_exact_bytes() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(Wire::try_from(&Value::Real(value)).is_err());
        }
        assert_eq!(Wire::try_from(&Value::Integer(i64::MIN)).unwrap(), Wire::Int(i64::MIN));
        assert_eq!(Wire::try_from(&Value::Null).unwrap(), Wire::NULL);
        assert_eq!(Wire::try_from(&Value::Blob(vec![0,255])).unwrap(), Wire::Bytes(vec![0,255]));
    }
    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_bound_edits_roundtrip_without_sql_interpolation() {
        use ::mysql::{Conn, OptsBuilder, prelude::Queryable};
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = crate::connection::MysqlConfig { host:"127.0.0.1".into(), port,
            user:"rustable".into(), password:"rustable-test-only".into(), database:"rustable_test".into(),
            tls:false, ..Default::default() };
        let mut conn = Conn::new(OptsBuilder::from(&config)).unwrap();
        conn.query_drop("CREATE TEMPORARY TABLE bound_value_probe (big BIGINT UNSIGNED, exact_decimal DECIMAL(30,10), txt TEXT, bin BLOB, nullable INT, signed_value BIGINT, real_value DOUBLE)").unwrap();
        let values = vec![Value::Text(u64::MAX.to_string()), Value::Text("12345678901234567890.1234567890".into()),
            Value::Text("日本\0'\\; DROP TABLE bound_value_probe; --".into()), Value::Blob(vec![0,255,39,92]),
            Value::Null, Value::Integer(i64::MIN), Value::Real(1.25)];
        for mode in ["", "NO_BACKSLASH_ESCAPES,ANSI_QUOTES"] {
            conn.exec_drop("SET SESSION sql_mode=?", (mode,)).unwrap();
            conn.query_drop("DELETE FROM bound_value_probe").unwrap();
            let parameters = values.iter().map(Wire::try_from).collect::<Result<Vec<_>,_>>().unwrap();
            conn.exec_drop("INSERT INTO bound_value_probe VALUES (?,?,?,?,?,?,?)", parameters).unwrap();
            let row: ::mysql::Row = conn.query_first("SELECT * FROM bound_value_probe").unwrap().unwrap();
            let columns = row.columns_ref().to_vec();
            let actual = columns.iter().zip(row.unwrap()).map(Value::try_from).collect::<Result<Vec<_>,_>>().unwrap();
            assert_eq!(actual, values);
            for value in &values {
                let sql = crate::review_sql::readable_dialect("SELECT ?", std::slice::from_ref(value),
                    crate::sql::Dialect::Mysql(crate::sql::mysql::Mode::default()));
                let row: ::mysql::Row = conn.query_first(&sql).unwrap().unwrap();
                let rendered = Value::try_from((&row.columns_ref()[0], row.get::<Wire,_>(0).unwrap())).unwrap();
                assert_eq!(&rendered, value, "review literal changed value under mode {mode}: {sql}");
            }
        }
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_beautify_preserves_values_under_session_quote_modes() {
        use ::mysql::{Conn, OptsBuilder, prelude::Queryable};
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let config = crate::connection::MysqlConfig { host:"127.0.0.1".into(), port,
            user:"rustable".into(), password:"rustable-test-only".into(), database:"rustable_test".into(),
            tls:false, ..Default::default() };
        let mut conn = Conn::new(OptsBuilder::from(&config)).unwrap();
        for ansi_quotes in [false, true] {
            for no_backslash_escapes in [false, true] {
                let modes = [if ansi_quotes { "ANSI_QUOTES" } else { "" }, if no_backslash_escapes { "NO_BACKSLASH_ESCAPES" } else { "" }].join(",");
                conn.exec_drop("SET sql_mode=?", (modes.trim_matches(','),)).unwrap();
                let literal = if no_backslash_escapes { "'ends in \\'" } else { "'escaped \\' quote'" };
                let sql = format!("select {literal} as value, \"two  words\" from (select 42 as `two  words`) t where 1=1; # unchanged\n");
                let dialect = crate::sql::Dialect::Mysql(crate::sql::mysql::Mode { ansi_quotes, no_backslash_escapes });
                let formatted = crate::sql_format::format_document(sql.clone(), 0..0, dialect).unwrap();
                let before: ::mysql::Row = conn.query_first(&sql).unwrap().unwrap();
                let after: ::mysql::Row = conn.query_first(&formatted.text).unwrap().unwrap();
                assert_eq!(before.unwrap(), after.unwrap(), "mode {modes}");
            }
        }
    }

}
