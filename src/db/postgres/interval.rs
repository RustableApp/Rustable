//! Wire layout: PostgreSQL timestamp.c interval_send (microseconds, days, months).
//! Keep the three components independent: a month is not a fixed number of days.
use super::*;

pub(super) fn decode(raw: &[u8]) -> Result<String, Box<dyn Error + Send + Sync>> {
    if raw.len() != 16 {
        return Err("Invalid PostgreSQL interval length".into());
    }
    let micros = i64::from_be_bytes(raw[..8].try_into()?);
    let days = i32::from_be_bytes(raw[8..12].try_into()?);
    let months = i32::from_be_bytes(raw[12..].try_into()?);
    if (micros, days, months) == (i64::MAX, i32::MAX, i32::MAX) {
        return Ok("infinity".into());
    }
    if (micros, days, months) == (i64::MIN, i32::MIN, i32::MIN) {
        return Ok("-infinity".into());
    }
    let magnitude = micros.unsigned_abs();
    let seconds = magnitude / 1_000_000;
    let mut clock = format!(
        "{}{:02}:{:02}:{:02}",
        if micros < 0 { '-' } else { '+' },
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
    );
    if magnitude % 1_000_000 != 0 {
        clock.push_str(format!(".{:06}", magnitude % 1_000_000).trim_end_matches('0'));
    }
    // Explicit signs avoid SQL-standard IntervalStyle applying a leading minus
    // to subsequent positive fields. No floating-point precision loss.
    let mut parts = Vec::with_capacity(3);
    if months != 0 {
        parts.push(format!("{months:+} mons"));
    }
    if days != 0 {
        parts.push(format!("{days:+} days"));
    }
    if micros != 0 || parts.is_empty() {
        parts.push(clock);
    }
    Ok(parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(micros: i64, days: i32, months: i32) -> Vec<u8> {
        [
            micros.to_be_bytes().as_slice(),
            days.to_be_bytes().as_slice(),
            months.to_be_bytes().as_slice(),
        ]
        .concat()
    }

    #[test]
    fn interval_preserves_independent_signs_precision_and_extremes() {
        assert_eq!(
            decode(&wire(3_661_123_400, -2, 14)).unwrap(),
            "+14 mons -2 days +01:01:01.1234"
        );
        assert_eq!(decode(&wire(-1, 0, 0)).unwrap(), "-00:00:00.000001");
        assert_eq!(decode(&wire(0, 0, 0)).unwrap(), "+00:00:00");
        assert_eq!(
            decode(&wire(i64::MIN, 0, 0)).unwrap(),
            "-2562047788:00:54.775808"
        );
        assert_eq!(
            decode(&wire(i64::MAX, i32::MAX, i32::MAX)).unwrap(),
            "infinity"
        );
        assert_eq!(
            decode(&wire(i64::MIN, i32::MIN, i32::MIN)).unwrap(),
            "-infinity"
        );
        assert!(decode(&[0; 15]).is_err());
        assert!(decode(&[0; 17]).is_err());
        let raw = wire(3_661_123_400, -2, 14);
        let length = decode(&raw).unwrap().len();
        assert!(
            PgValue::decode_bounded(&Type::INTERVAL, &raw, length)
                .unwrap()
                .is_some()
        );
        assert!(
            PgValue::decode_bounded(&Type::INTERVAL, &raw, length - 1)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn interval_display_roundtrips_all_styles_and_arrays() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let mut client = Client::connect(
            "host=127.0.0.1 port=54329 dbname=rustable_qa user=postgres password=rustable_qa_only",
            NoTls,
        )
        .unwrap();
        for style in ["postgres", "postgres_verbose", "sql_standard", "iso_8601"] {
            client
                .batch_execute(&format!("SET intervalstyle = '{style}'"))
                .unwrap();
            for input in [
                "P14M-2DT3661.1234S",
                "P-2M3DT-0.000001S",
                "PT0S",
                "PT9223372036854.775807S",
                "PT-9223372036854.775808S",
            ] {
                let row = client.query_one("SELECT $1::text::interval, ARRAY[$1::text::interval,NULL,interval '1 day']", &[&input]).unwrap();
                for (index, cast) in [(0, "interval"), (1, "interval[]")] {
                    let Value::Text(display) = row.get::<_, PgValue>(index).0 else {
                        panic!("Interval must display as text")
                    };
                    let sql = "SELECT pg_catalog.interval_send($1::text::interval) = pg_catalog.interval_send($2::text::interval)";
                    if index == 0 {
                        assert!(
                            client
                                .query_one(sql, &[&input, &display])
                                .unwrap()
                                .get::<_, bool>(0),
                            "{style}: {input} -> {display}"
                        );
                    } else {
                        assert!(client.query_one(&format!("SELECT ARRAY[$1::text::interval,NULL,interval '1 day'] = $2::text::{cast}"), &[&input, &display]).unwrap().get::<_, bool>(0));
                    }
                }
            }
        }
    }
}
