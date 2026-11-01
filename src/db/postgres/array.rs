//! PostgreSQL binary arrays -> valid PostgreSQL array input text.
//! Syntax: https://www.postgresql.org/docs/current/arrays.html
//! Wire layout follows upstream array_send/array_recv in arrayfuncs.c.
use super::*;
type DecodeError = Box<dyn Error + Sync + Send>;

fn integer(raw: &mut &[u8]) -> Result<i32, DecodeError> {
    let (head, rest) = raw.split_at_checked(4).ok_or("Truncated array header")?;
    *raw = rest;
    Ok(i32::from_be_bytes(head.try_into()?))
}

pub(super) fn decode(element: &Type, raw: &[u8]) -> Result<String, DecodeError> {
    decode_bounded(element, raw, usize::MAX)?.ok_or_else(|| "Array output length overflow".into())
}

pub(super) fn decode_bounded(
    element: &Type,
    mut raw: &[u8],
    budget: usize,
) -> Result<Option<String>, DecodeError> {
    let dimensions = integer(&mut raw)?;
    if !(0..=6).contains(&dimensions) {
        return Err("Invalid PostgreSQL array dimensions".into());
    }
    let nulls = integer(&mut raw)?;
    if !(0..=1).contains(&nulls) {
        return Err("Invalid PostgreSQL array null flag".into());
    }
    if integer(&mut raw)? as u32 != element.oid() {
        return Err("Array element type mismatch".into());
    }
    let mut bounds = Vec::with_capacity(dimensions as usize);
    let mut count = usize::from(dimensions != 0);
    for _ in 0..dimensions {
        let length = integer(&mut raw)?;
        let lower = integer(&mut raw)?;
        if length <= 0 {
            return Err("Invalid PostgreSQL array dimension length".into());
        }
        let upper = lower
            .checked_add(length - 1)
            .ok_or("Array bound overflow")?;
        count = count
            .checked_mul(length as usize)
            .ok_or("Array length overflow")?;
        bounds.push((length as usize, lower, upper));
    }
    // Every value requires at least its length word; reject hostile dimensions
    // before allocating or walking their claimed element count.
    if count > raw.len() / 4 {
        return Err("Truncated array elements".into());
    }
    let mut out = String::new();
    if bounds.iter().any(|(_, lower, _)| *lower != 1) {
        for (_, lower, upper) in &bounds {
            if !append(&mut out, &format!("[{lower}:{upper}]"), budget) {
                return Ok(None);
            }
        }
        if !append(&mut out, "=", budget) {
            return Ok(None);
        }
    }
    if dimensions == 0 {
        if !append(&mut out, "{}", budget) {
            return Ok(None);
        }
    } else {
        if !dimension(&mut out, &bounds, element, &mut raw, nulls != 0, budget)? {
            return Ok(None);
        }
    }
    if !raw.is_empty() {
        return Err("Trailing PostgreSQL array bytes".into());
    }
    Ok(Some(out))
}

// Geometric growth stays inside the budget, including near its boundary.
fn reserve(out: &mut String, additional: usize, budget: usize) {
    let required = out.len() + additional;
    if required > out.capacity() {
        let target = required.max(out.capacity().saturating_mul(2)).min(budget);
        out.reserve_exact(target - out.len());
    }
}
fn append(out: &mut String, value: &str, budget: usize) -> bool {
    if value.len() > budget.saturating_sub(out.len()) {
        return false;
    }
    reserve(out, value.len(), budget);
    out.push_str(value);
    true
}

fn dimension(
    out: &mut String,
    bounds: &[(usize, i32, i32)],
    element: &Type,
    raw: &mut &[u8],
    nulls: bool,
    budget: usize,
) -> Result<bool, DecodeError> {
    if !append(out, "{", budget) {
        return Ok(false);
    }
    for index in 0..bounds[0].0 {
        if index != 0 && !append(out, ",", budget) {
            return Ok(false);
        }
        if bounds.len() > 1 {
            if !dimension(out, &bounds[1..], element, raw, nulls, budget)? {
                return Ok(false);
            }
            continue;
        }
        let length = integer(raw)?;
        if length == -1 {
            if !nulls {
                return Err("Unexpected NULL array element".into());
            }
            if !append(out, "NULL", budget) {
                return Ok(false);
            }
            continue;
        }
        let length = usize::try_from(length).map_err(|_| "Invalid array element length")?;
        let (value, rest) = raw
            .split_at_checked(length)
            .ok_or("Truncated array element")?;
        *raw = rest;
        let remaining = budget.saturating_sub(out.len());
        if bytea(element) {
            // Two quotes, two escaped backslashes + x, and two hex digits/byte.
            let Some(required) = value.len().checked_mul(2).and_then(|n| n.checked_add(5)) else {
                return Ok(false);
            };
            if required > remaining {
                return Ok(false);
            }
            reserve(out, required, budget);
            out.push('"');
            out.push('\\');
            out.push('\\');
            out.push('x');
            const HEX: &[u8] = b"0123456789abcdef";
            for byte in value {
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 15) as usize] as char);
            }
            out.push('"');
            continue;
        }
        let Some(value) = PgValue::decode_bounded(element, value, remaining)? else {
            return Ok(false);
        };
        let text = match value {
            Value::Text(text) => text,
            Value::Integer(number) => number.to_string(),
            Value::Real(number) => number.to_string(),
            _ => return Err("Unsupported PostgreSQL array element encoding".into()),
        };
        let escapes = text
            .bytes()
            .filter(|byte| matches!(byte, b'\\' | b'"'))
            .count();
        let Some(required) = text
            .len()
            .checked_add(escapes)
            .and_then(|n| n.checked_add(2))
        else {
            return Ok(false);
        };
        if required > remaining {
            return Ok(false);
        }
        reserve(out, required, budget);
        out.push('"');
        for character in text.chars() {
            if matches!(character, '\\' | '"') {
                out.push('\\');
            }
            out.push(character);
        }
        out.push('"');
    }
    Ok(append(out, "}", budget))
}
fn bytea(ty: &Type) -> bool {
    match ty.kind() {
        ::postgres::types::Kind::Domain(base) => bytea(base),
        _ => *ty == Type::BYTEA,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wire(bounds: &[(i32, i32)], values: &[Option<&[u8]>]) -> Vec<u8> {
        let mut out = Vec::new();
        for integer in [
            bounds.len() as i32,
            i32::from(values.iter().any(Option::is_none)),
            Type::TEXT.oid() as i32,
        ] {
            out.extend(integer.to_be_bytes());
        }
        for (length, lower) in bounds {
            out.extend(length.to_be_bytes());
            out.extend(lower.to_be_bytes());
        }
        for value in values {
            out.extend(value.map_or(-1, |v| v.len() as i32).to_be_bytes());
            if let Some(value) = value {
                out.extend(*value);
            }
        }
        out
    }
    #[test]
    fn dimensions_nulls_and_escaped_values_are_preserved() {
        let bytes = wire(
            &[(2, -1), (2, 3)],
            &[Some(b"NULL"), None, Some(b""), Some(b"a,\"\\")],
        );
        assert_eq!(
            decode(&Type::TEXT, &bytes).unwrap(),
            r#"[-1:0][3:4]={{"NULL",NULL},{"","a,\"\\"}}"#
        );
        assert_eq!(decode(&Type::TEXT, &wire(&[], &[])).unwrap(), "{}");
    }
    #[test]
    fn budgets_are_exact_for_bounds_unicode_null_and_escape_expansion() {
        for bytes in [
            wire(&[], &[]),
            wire(
                &[(2, -2), (2, 3)],
                &[Some("日本\\\"".as_bytes()), None, Some(b""), Some(b"NULL")],
            ),
            wire(&[(1, 1)], &[Some(&vec![b'\\'; 65_536])]),
        ] {
            let expected = decode(&Type::TEXT, &bytes).unwrap();
            assert_eq!(
                decode_bounded(&Type::TEXT, &bytes, expected.len()).unwrap(),
                Some(expected.clone())
            );
            assert_eq!(
                decode_bounded(&Type::TEXT, &bytes, expected.len() - 1).unwrap(),
                None
            );
            assert_eq!(decode_bounded(&Type::TEXT, &bytes, 0).unwrap(), None);
        }
    }
    #[test]
    fn bytea_budget_counts_escaped_hex_without_a_blob_copy() {
        let blob = vec![255; 65_536];
        let mut bytes = wire(&[(1, 1)], &[Some(&blob)]);
        bytes[8..12].copy_from_slice(&Type::BYTEA.oid().to_be_bytes());
        let required = blob.len() * 2 + 7;
        let output = decode_bounded(&Type::BYTEA, &bytes, required)
            .unwrap()
            .unwrap();
        assert_eq!(output.len(), required);
        assert!(output.starts_with("{\"\\\\x"));
        assert_eq!(
            decode_bounded(&Type::BYTEA, &bytes, required - 1).unwrap(),
            None
        );
    }
    #[test]
    fn rejects_malformed_bounds_lengths_and_trailing_data() {
        let valid = wire(&[(1, 1)], &[Some(b"x")]);
        for length in 0..valid.len() {
            assert!(decode(&Type::TEXT, &valid[..length]).is_err());
        }
        let mut trailing = valid.clone();
        trailing.push(0);
        assert!(decode(&Type::TEXT, &trailing).is_err());
        for bounds in [
            vec![(2, i32::MAX)],
            vec![(-1, 1)],
            vec![(0, 1)],
            vec![(i32::MAX, 1), (i32::MAX, 1)],
        ] {
            assert!(decode(&Type::TEXT, &wire(&bounds, &[])).is_err());
        }
        assert!(decode(&Type::INT4, &valid).is_err());
        let mut bad = valid.clone();
        bad[20..24].copy_from_slice(&(-2_i32).to_be_bytes());
        assert!(decode(&Type::TEXT, &bad).is_err());
    }
    #[test]
    fn unknown_array_element_encoding_is_not_guessed_as_text() {
        let raw = b"printable is not proof of a text wire format";
        assert_eq!(
            PgValue::from_sql(&Type::POINT_ARRAY, raw).unwrap().0,
            Value::Blob(raw.to_vec())
        );
    }
    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
    fn live_array_results_roundtrip_dimensions_and_binary() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let mut client = Client::connect(
            "host=127.0.0.1 port=54329 dbname=rustable_qa user=postgres password=rustable_qa_only",
            NoTls,
        )
        .unwrap();
        for expression in [
            "'[0:1][-2:-1]={{1,NULL},{3,4}}'::int4[]",
            "ARRAY['NULL','',E'comma,quote\"slash\\\\','日本',NULL]::text[]",
            "ARRAY[decode('00ff','hex'),decode('','hex'),NULL]::bytea[]",
            "ARRAY[12345678901234567890.12345678,NULL]::numeric[]",
            "'{}'::text[]",
        ] {
            let row = client
                .query_one(&format!("SELECT {expression}"), &[])
                .unwrap();
            let Value::Text(text) = decode_row(&row).unwrap().remove(0) else {
                panic!("Array not decoded: {expression}")
            };
            let ty = row.columns()[0].type_().name();
            let equal: bool = client
                .query_one(
                    &format!(
                        "SELECT ($1::text)::pg_catalog.{} IS NOT DISTINCT FROM {expression}",
                        quote(ty)
                    ),
                    &[&text],
                )
                .unwrap()
                .get(0);
            assert!(equal, "Failed roundtrip {expression}: {text}");
        }
    }
}
