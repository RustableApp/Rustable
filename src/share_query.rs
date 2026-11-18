//! Render captured bindings for sharing only; never execute the rendered SQL.
use crate::{
    db::{QuerySource, Value},
    sql::Dialect,
};
use sqlparser::{
    dialect::{PostgreSqlDialect, SQLiteDialect},
    tokenizer::{Token, Tokenizer},
};

pub(super) fn render(source: &QuerySource, dialect: Dialect) -> Result<String, String> {
    if source.parameters.is_empty() {
        return Ok(source.sql.clone());
    }
    let mysql = crate::sql::mysql::Lexer(match dialect {
        Dialect::Mysql(mode) => mode,
        _ => Default::default(),
    });
    let lexer: &dyn sqlparser::dialect::Dialect = match dialect {
        Dialect::Postgres => &PostgreSqlDialect {},
        Dialect::Sqlite => &SQLiteDialect {},
        Dialect::Mysql(_) => &mysql,
    };
    let tokens = Tokenizer::new(lexer, &source.sql)
        .with_unescape(false)
        .tokenize_with_location()
        .map_err(|_| "Cannot resolve parameters in this query.")?;
    let mut cursor = source.sql.char_indices().peekable();
    let mut location = (1, 1);
    let mut seek = |target: sqlparser::tokenizer::Location| -> Result<usize, String> {
        while location < (target.line, target.column) {
            let (_, ch) = cursor.next().ok_or("Invalid query token location")?;
            if ch == '\n' {
                location = (location.0 + 1, 1);
            } else {
                location.1 += 1;
            }
        }
        if location != (target.line, target.column) {
            return Err("Invalid query token location".into());
        }
        Ok(cursor.peek().map_or(source.sql.len(), |(byte, _)| *byte))
    };
    let mut result = String::new();
    let mut previous = 0;
    let mut next = 0usize;
    for token in tokens {
        let Token::Placeholder(placeholder) = &token.token else {
            continue;
        };
        let index = match dialect {
            Dialect::Postgres => placeholder
                .strip_prefix('$')
                .and_then(|s| s.parse::<usize>().ok()),
            Dialect::Sqlite if placeholder.starts_with('?') && placeholder.len() > 1 => {
                placeholder[1..].parse::<usize>().ok()
            }
            Dialect::Sqlite | Dialect::Mysql(_) if placeholder == "?" => Some(next + 1),
            _ => None,
        }
        .ok_or("Unsupported query parameter; cannot safely include this query.")?;
        next = next.max(index);
        let value = index
            .checked_sub(1)
            .and_then(|i| source.parameters.get(i))
            .ok_or("A query parameter value is missing; cannot safely include this query.")?;
        let start = seek(token.span.start)?;
        let end = seek(token.span.end)?;
        let literal = literal(value, dialect)?;
        if result.len() + start - previous + literal.len() + source.sql.len() - end
            > 32 * 1024 * 1024
        {
            return Err("Expanded query exceeds the 32 MiB clipboard limit.".into());
        }
        result.push_str(&source.sql[previous..start]);
        result.push_str(&literal);
        previous = end;
    }
    result.push_str(&source.sql[previous..]);
    Ok(result)
}
fn literal(value: &Value, dialect: Dialect) -> Result<String, String> {
    Ok(match value {
        Value::Null => "NULL".into(),
        Value::Integer(value) => value.to_string(),
        Value::Real(value) if value.is_finite() => format!("{value:?}"),
        Value::Real(_) => return Err("Cannot safely share a non-finite query parameter.".into()),
        Value::Text(value) if matches!(dialect, Dialect::Postgres) && value.contains('\0') => {
            return Err("Cannot share a PostgreSQL text parameter containing NUL.".into());
        }
        Value::Text(value) => {
            // Ordinary MySQL strings remain readable, independently of sql_mode.
            if matches!(dialect, Dialect::Mysql(_)) && !value.contains(['\\', '\0', '\u{1a}']) {
                format!("'{}'", value.replace('\'', "''"))
            } else {
                crate::db::filters::literal(value, dialect)
            }
        }
        Value::Blob(value) => {
            use std::fmt::Write;
            let mut hex = String::with_capacity(value.len() * 2);
            for byte in value {
                write!(hex, "{byte:02x}").unwrap();
            }
            if matches!(dialect, Dialect::Postgres) {
                format!("decode('{hex}', 'hex')")
            } else {
                format!("X'{hex}'")
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resolve(sql: &str, parameters: Vec<Value>, dialect: Dialect) -> String {
        render(
            &QuerySource {
                sql: sql.into(),
                parameters,
            },
            dialect,
        )
        .unwrap()
    }
    #[test]
    fn postgres_preserves_quoted_tokens_and_resolves_numbered_bindings() {
        let params = (1..=10).map(Value::Integer).collect();
        assert_eq!(
            resolve(
                "SELECT '雪 $1', \"$2\", $$ $3 $$, $10, $1, $1 -- $2\n/* $3 */ $2",
                params,
                Dialect::Postgres
            ),
            "SELECT '雪 $1', \"$2\", $$ $3 $$, 10, 1, 1 -- $2\n/* $3 */ 2"
        );
        assert_eq!(
            resolve(
                "SELECT $1, $2, $3",
                vec![
                    Value::Text("O'Brien\\x".into()),
                    Value::Null,
                    Value::Blob(vec![0, 255])
                ],
                Dialect::Postgres
            ),
            "SELECT E'O''Brien\\\\x', NULL, decode('00ff', 'hex')"
        );
    }
    #[test]
    fn sqlite_literals_round_trip_and_numbering_is_preserved() {
        let parameters = vec![
            Value::Text("雪'\0\\".into()),
            Value::Blob(vec![0, 255]),
            Value::Real(1.25),
            Value::Null,
        ];
        let sql = resolve("SELECT ?1, ?2, ?, ?, ?1", parameters, Dialect::Sqlite);
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.query_row(&sql, [], |row| {
            assert_eq!(row.get::<_, String>(0)?, "雪'\0\\");
            assert_eq!(row.get::<_, Vec<u8>>(1)?, vec![0, 255]);
            assert_eq!(row.get::<_, f64>(2)?, 1.25);
            assert_eq!(row.get::<_, Option<String>>(3)?, None);
            assert_eq!(row.get::<_, String>(4)?, "雪'\0\\");
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn mysql_placeholders_ignore_strings_and_comments() {
        assert_eq!(
            resolve(
                "SELECT '?', ?, `?`, ? # ?\n",
                vec![Value::Text("O'Brien".into()), Value::Integer(42)],
                Dialect::Mysql(Default::default())
            ),
            "SELECT '?', 'O''Brien', `?`, 42 # ?\n"
        );
    }
    #[test]
    fn missing_values_fail_and_unbound_queries_are_preserved() {
        assert!(
            render(
                &QuerySource {
                    sql: "SELECT $2".into(),
                    parameters: vec![Value::Null]
                },
                Dialect::Postgres
            )
            .is_err()
        );
        assert_eq!(resolve("SELECT $1", vec![], Dialect::Postgres), "SELECT $1");
    }
}
