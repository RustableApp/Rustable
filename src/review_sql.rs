//! Display-only expansion of generated SQLite/PostgreSQL statements. Execution stays bound.
use crate::db::Value;

fn literal(value: &Value) -> String {
    match value {
        Value::Null => "NULL".into(),
        Value::Integer(n) => n.to_string(),
        Value::Real(n) if n.is_nan() => "NULL".into(),
        Value::Real(n) if n.is_infinite() => if n.is_sign_negative() {
            "-9e999"
        } else {
            "9e999"
        }
        .into(),
        Value::Real(n) => format!("{n:?}"),
        Value::Text(s) if s.contains('\0') => format!("CAST({} AS TEXT)", blob(s.as_bytes())),
        Value::Text(s) => format!("'{}'", s.replace('\'', "''")),
        Value::Blob(bytes) => blob(bytes),
    }
}
fn blob(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::from("X'");
    for byte in bytes {
        write!(out, "{byte:02X}").unwrap();
    }
    out.push('\'');
    out
}

pub fn expand(sql: &str, parameters: &[Value]) -> String {
    expand_impl(sql, parameters, false)
}

/// Readable presentation of our generated bound statements; never used for execution.
/// Only recognize the exact cast chain emitted by the PostgreSQL change planner.
pub fn readable(sql: &str, parameters: &[Value]) -> String {
    expand_impl(sql, parameters, true)
}

fn expand_impl(sql: &str, parameters: &[Value], readable: bool) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    let mut parameter = 0;
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' | '`' | '[' => {
                let end = if c == '[' { ']' } else { c };
                out.push(c);
                while let Some(c) = chars.next() {
                    out.push(c);
                    if c == end {
                        if end != ']' && chars.peek() == Some(&end) {
                            out.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                }
            }
            '-' if chars.peek() == Some(&'-') => {
                out.push(c);
                for c in chars.by_ref() {
                    out.push(c);
                    if c == '\n' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                out.push(c);
                out.push(chars.next().unwrap());
                while let Some(c) = chars.next() {
                    out.push(c);
                    if c == '*' && chars.peek() == Some(&'/') {
                        out.push(chars.next().unwrap());
                        break;
                    }
                }
            }
            '?' => {
                let mut digits = String::new();
                while chars.peek().is_some_and(char::is_ascii_digit) {
                    digits.push(chars.next().unwrap());
                }
                let index = digits
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| i.checked_sub(1))
                    .unwrap_or(parameter);
                if let Some(value) = parameters.get(index) {
                    out.push_str(&literal(value));
                } else {
                    out.push('?');
                    out.push_str(&digits);
                }
                parameter = parameter.max(index + 1);
            }
            '$' if chars.peek().is_some_and(char::is_ascii_digit) => {
                let mut digits = String::new();
                while chars.peek().is_some_and(char::is_ascii_digit) {
                    digits.push(chars.next().unwrap());
                }
                if let Some(value) = digits
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| parameters.get(index))
                {
                    if readable {
                        let remainder: String = chars.clone().collect();
                        if let Some((length, schema, ty)) = generated_cast(&remainder) {
                            out.push_str(&typed_literal(value, &schema, &ty));
                            for _ in 0..remainder[..length].chars().count() {
                                chars.next();
                            }
                            continue;
                        }
                    }
                    match value {
                        // E strings preserve backslashes even if the server's
                        // standard_conforming_strings setting differs.
                        Value::Text(text) => out.push_str(&format!(
                            "E'{}'",
                            text.replace('\\', "\\\\").replace('\'', "''")
                        )),
                        Value::Blob(bytes) => {
                            let hex = blob(bytes);
                            out.push_str(&format!("decode({}, 'hex')", &hex[1..]));
                        }
                        other => out.push_str(&literal(other)),
                    }
                } else {
                    out.push('$');
                    out.push_str(&digits);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

// Parse quoted identifiers, including embedded doubled quotes. No transformations of
// arbitrary SQL, string contents, comments, or user-authored casts are performed.
fn generated_cast(sql: &str) -> Option<(usize, String, String)> {
    fn identifier(input: &str) -> Option<(usize, String)> {
        let mut chars = input.char_indices();
        if chars.next()?.1 != '"' {
            return None;
        }
        let mut name = String::new();
        while let Some((offset, c)) = chars.next() {
            if c == '"' {
                if chars.clone().next().is_some_and(|(_, c)| c == '"') {
                    chars.next();
                    name.push('"');
                } else {
                    return Some((offset + 1, name));
                }
            } else {
                name.push(c);
            }
        }
        None
    }
    let prefix = "::pg_catalog.text::";
    let rest = sql.strip_prefix(prefix)?;
    let (a, schema) = identifier(rest)?;
    let (b, ty) = identifier(rest[a..].strip_prefix('.')?)?;
    Some((prefix.len() + a + 1 + b, schema, ty))
}

fn numeric_literal(text: &str) -> bool {
    // SQL numeric tokens, without loss from parsing large integers/precise decimals.
    let text = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (mantissa, exponent) = text
        .find(['e', 'E'])
        .map_or((text, None), |i| (&text[..i], Some(&text[i + 1..])));
    let mut digits = 0;
    let mut dots = 0;
    for c in mantissa.chars() {
        if c.is_ascii_digit() {
            digits += 1;
        } else if c == '.' {
            dots += 1;
        } else {
            return false;
        }
    }
    digits > 0
        && dots <= 1
        && exponent.is_none_or(|exponent| {
            let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
            !exponent.is_empty() && exponent.bytes().all(|c| c.is_ascii_digit())
        })
}

fn pg_string(text: &str) -> String {
    if text.contains('\\') {
        format!("E'{}'", text.replace('\\', "\\\\").replace('\'', "''"))
    } else {
        format!("'{}'", text.replace('\'', "''"))
    }
}

fn typed_literal(value: &Value, schema: &str, ty: &str) -> String {
    if matches!(value, Value::Null) {
        return "NULL".into();
    }
    let Value::Text(text) = value else {
        return literal(value);
    };
    if schema == "pg_catalog" {
        match ty {
            "int2" | "int4" | "int8" | "oid" | "numeric" | "float4" | "float8"
                if numeric_literal(text) =>
            {
                return text.clone();
            }
            "bool" if matches!(text.as_str(), "t" | "true") => return "TRUE".into(),
            "bool" if matches!(text.as_str(), "f" | "false") => return "FALSE".into(),
            "text" | "varchar" | "bpchar" | "name" => return pg_string(text),
            _ => {}
        }
    }
    // Preserve domains, enums, arrays, dates, nonfinite floats, and other typed values.
    format!(
        "{}::\"{}\".\"{}\"",
        pg_string(text),
        schema.replace('"', "\"\""),
        ty.replace('"', "\"\"")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readable_changes_render_numbers_without_binding_casts() {
        let sql = r#"UPDATE "public"."address" SET "available" = $1::pg_catalog.text::"pg_catalog"."numeric" WHERE "id" = $2::pg_catalog.text::"pg_catalog"."int4";"#;
        let values = [
            Value::Text("1.00000000000000000001".into()),
            Value::Text("5".into()),
        ];
        assert_eq!(
            readable(sql, &values),
            r#"UPDATE "public"."address" SET "available" = 1.00000000000000000001 WHERE "id" = 5;"#
        );
        // Exact expansion remains available for diagnostics, execution remains bound.
        assert!(expand(sql, &values).contains("::pg_catalog.text::"));
    }

    #[test]
    fn readable_values_keep_text_quoting_and_meaningful_types() {
        for (ty, value, expected) in [
            ("text", "123", "'123'"),
            ("text", "O'Brien", "'O''Brien'"),
            ("text", r"a\b", r"E'a\\b'"),
            ("bool", "t", "TRUE"),
            ("bool", "false", "FALSE"),
            ("numeric", "-1.25e+30", "-1.25e+30"),
            ("numeric", "NaN", "'NaN'::\"pg_catalog\".\"numeric\""),
            (
                "int4",
                "1; DELETE FROM t",
                "'1; DELETE FROM t'::\"pg_catalog\".\"int4\"",
            ),
            ("_int4", "{1,2}", "'{1,2}'::\"pg_catalog\".\"_int4\""),
        ] {
            let sql = format!("$1::pg_catalog.text::\"pg_catalog\".\"{ty}\"");
            assert_eq!(readable(&sql, &[Value::Text(value.into())]), expected);
            assert_eq!(readable(&sql, &[Value::Null]), "NULL");
        }
        assert_eq!(
            readable(
                r#"$1::pg_catalog.text::"custom"."my""enum""#,
                &[Value::Text("ready".into())]
            ),
            r#"'ready'::"custom"."my""enum""#
        );
    }

    #[test]
    fn readable_only_rewrites_generated_parameter_casts() {
        let sql = r#"SELECT '$1::pg_catalog.text::"pg_catalog"."int4"', "$1", $1::pg_catalog.int4, $2; -- $1::pg_catalog.text::"pg_catalog"."int4""#;
        assert_eq!(
            readable(sql, &[Value::Text("4".into())]),
            expand(sql, &[Value::Text("4".into())])
        );
        for invalid in ["", ".", "-", "1e", "1e+", "1e2e3", "1.2.3", "1 --comment"] {
            assert!(!numeric_literal(invalid), "{invalid}");
        }
    }

    #[test]
    fn postgres_numbered_parameters_keep_typed_casts_and_escape_literal_backslashes() {
        assert_eq!(
            expand(
                "UPDATE \"$1\" SET \"name\"=$2::pg_catalog.text WHERE \"id\"=$1::pg_catalog.int4 AND \"nullable\" IS NOT DISTINCT FROM $3; -- $2",
                &[
                    Value::Text("4".into()),
                    Value::Text("O'Brien\\path".into()),
                    Value::Null
                ]
            ),
            "UPDATE \"$1\" SET \"name\"=E'O''Brien\\\\path'::pg_catalog.text WHERE \"id\"=E'4'::pg_catalog.int4 AND \"nullable\" IS NOT DISTINCT FROM NULL; -- $2"
        );
    }
    #[test]
    fn expansion_preserves_quoted_identifiers_comments_and_parameter_order() {
        assert_eq!(
            expand(
                "UPDATE \"why?\" SET x=? WHERE y=?; -- ?\n/* ? */ SELECT '?', ?1;",
                &[Value::Text("O'Brien".into()), Value::Null]
            ),
            "UPDATE \"why?\" SET x='O''Brien' WHERE y=NULL; -- ?\n/* ? */ SELECT '?', 'O''Brien';"
        );
    }
    #[test]
    fn rendered_literals_round_trip_through_sqlite() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        for value in [
            Value::Integer(i64::MIN),
            Value::Real(607.0),
            Value::Text("".into()),
            Value::Text("a'\n?日本".into()),
            Value::Text("a\0b".into()),
            Value::Blob(vec![0, 255]),
            Value::Null,
        ] {
            let sql = expand("SELECT ?", std::slice::from_ref(&value));
            let rendered: rusqlite::types::Value = db.query_row(&sql, [], |r| r.get(0)).unwrap();
            let bound: rusqlite::types::Value = db
                .query_row("SELECT ?", rusqlite::params![&value], |r| r.get(0))
                .unwrap();
            assert_eq!(rendered, bound);
        }
    }
}
