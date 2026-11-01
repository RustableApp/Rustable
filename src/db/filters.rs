//! Shared predicates for browsing and exporting the same filtered data.
use crate::sql::Dialect;
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Filter {
    pub enabled: bool,
    pub column: String,
    pub operator: String,
    pub value: String,
}
impl Filter {
    pub fn new(column: String) -> Self {
        Self {
            enabled: true,
            column,
            operator: "=".into(),
            value: String::new(),
        }
    }
}
pub const OPERATORS: &[&str] = &[
    "=",
    "<>",
    "<",
    ">",
    "<=",
    ">=",
    "Contains",
    "Not contains",
    "Has prefix",
    "Has suffix",
    "IN",
    "NOT IN",
    "IS NULL",
    "IS NOT NULL",
];
pub(crate) fn literal(value: &str, dialect: Dialect) -> String {
    match dialect {
        Dialect::Mysql(_) => format!(
            "CONVERT(X'{}' USING utf8mb4)",
            value
                .as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ),
        Dialect::Postgres => format!("E'{}'", value.replace('\\', "\\\\").replace('\'', "''")),
        Dialect::Sqlite if value.contains('\0') => format!(
            "CAST(X'{}' AS TEXT)",
            value
                .as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ),
        Dialect::Sqlite => format!("'{}'", value.replace('\'', "''")),
    }
}
pub fn predicate(
    filters: &[Filter],
    columns: &[String],
    dialect: Dialect,
) -> Result<String, String> {
    let mut parts = Vec::new();
    for filter in filters.iter().filter(|f| f.enabled) {
        if filter.column == "Raw SQL" {
            // MySQL executable comments and session-dependent backslash escapes
            // must not change a validated expression into additional SQL.
            if filter.value.contains("/*")
                || filter.value.contains("--")
                || (matches!(dialect, Dialect::Mysql(_)) && filter.value.contains(['#', '\\']))
            {
                return Err("Raw filters cannot contain comments or MySQL backslash escapes. Use a column filter for these literal values.".into());
            }
            use sqlparser::{
                dialect::{
                    Dialect as ParserDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect,
                },
                parser::Parser,
                tokenizer::Token,
            };
            let parser_dialect: &dyn ParserDialect = match dialect {
                Dialect::Sqlite => &SQLiteDialect {},
                Dialect::Postgres => &PostgreSqlDialect {},
                Dialect::Mysql(_) => &MySqlDialect {},
            };
            let mut parser = Parser::new(parser_dialect)
                .try_with_sql(&filter.value)
                .map_err(|e| e.to_string())?;
            let expression = parser.parse_expr().map_err(|e| e.to_string())?;
            if !read_only_expression(&expression) {
                return Err("Raw filters support column comparisons, AND/OR, LIKE, IN, BETWEEN, and NULL checks. Use the SQL editor for functions or subqueries.".into());
            }
            if parser.peek_token().token != Token::EOF {
                return Err("Raw SQL must be a single filter expression.".into());
            }
            // A newline keeps a trailing line comment from swallowing our closing parenthesis.
            parts.push(format!("({}\n)", filter.value));
            continue;
        }
        if !columns.contains(&filter.column) {
            return Err(format!("Unknown filter column: {}", filter.column));
        }
        let quote = if matches!(dialect, Dialect::Mysql(_)) {
            '`'
        } else {
            '"'
        };
        let column = format!(
            "{quote}{}{quote}",
            filter.column.replace(quote, &format!("{quote}{quote}"))
        );
        let op = filter.operator.as_str();
        let value = literal(&filter.value, dialect);
        let part = match op {
            "=" | "<>" | "<" | ">" | "<=" | ">=" => format!("{column} {op} {value}"),
            "IS NULL" | "IS NOT NULL" => format!("{column} {op}"),
            "IN" | "NOT IN" => {
                // JSON arrays avoid ambiguous comma splitting and preserve commas inside values.
                let values: Vec<serde_json::Value> = serde_json::from_str(&filter.value).map_err(
                    |_| "IN requires a JSON array, for example [1, 2] or [\"a\", \"b\"].",
                )?;
                if values.is_empty() {
                    return Err("IN requires at least one value.".into());
                }
                let values = values
                    .iter()
                    .map(|v| match v {
                        serde_json::Value::Null => Ok("NULL".into()),
                        serde_json::Value::String(s) => Ok(literal(s, dialect)),
                        serde_json::Value::Number(n) => Ok(n.to_string()),
                        serde_json::Value::Bool(b) => Ok(b.to_string()),
                        _ => Err("IN values must be strings, numbers, booleans, or null."),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                format!("{column} {op} ({})", values.join(", "))
            }
            "Contains" | "Not contains" | "Has prefix" | "Has suffix" => {
                let escaped = filter
                    .value
                    .replace('!', "!!")
                    .replace('%', "!%")
                    .replace('_', "!_");
                let pattern = match op {
                    "Has prefix" => format!("{escaped}%"),
                    "Has suffix" => format!("%{escaped}"),
                    _ => format!("%{escaped}%"),
                };
                let ty = if matches!(dialect, Dialect::Mysql(_)) {
                    "CHAR"
                } else {
                    "TEXT"
                };
                format!(
                    "CAST({column} AS {ty}) {}LIKE {} ESCAPE '!'",
                    if op == "Not contains" { "NOT " } else { "" },
                    literal(&pattern, dialect)
                )
            }
            _ => return Err("Unknown filter operator".into()),
        };
        parts.push(format!("({part})"));
    }
    Ok(parts.join(" AND "))
}
fn read_only_expression(expr: &sqlparser::ast::Expr) -> bool {
    use sqlparser::ast::Expr;
    match expr {
        Expr::Identifier(_) | Expr::CompoundIdentifier(_) | Expr::Value(_) => true,
        Expr::Nested(e)
        | Expr::UnaryOp { expr: e, .. }
        | Expr::IsNull(e)
        | Expr::IsNotNull(e)
        | Expr::IsTrue(e)
        | Expr::IsFalse(e)
        | Expr::IsNotTrue(e)
        | Expr::IsNotFalse(e) => read_only_expression(e),
        Expr::BinaryOp { left, right, .. }
        | Expr::IsDistinctFrom(left, right)
        | Expr::IsNotDistinctFrom(left, right) => {
            read_only_expression(left) && read_only_expression(right)
        }
        Expr::InList { expr, list, .. } => {
            read_only_expression(expr) && list.iter().all(read_only_expression)
        }
        Expr::Between {
            expr, low, high, ..
        } => read_only_expression(expr) && read_only_expression(low) && read_only_expression(high),
        Expr::Like { expr, pattern, .. } | Expr::ILike { expr, pattern, .. } => {
            read_only_expression(expr) && read_only_expression(pattern)
        }
        _ => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sqlite_predicates_preserve_literals_and_numeric_affinity() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t (n INTEGER, s TEXT); INSERT INTO t VALUES (2, 'a%_!''b'), (10, 'other');").unwrap();
        let mut filters = vec![Filter {
            enabled: true,
            column: "n".into(),
            operator: ">".into(),
            value: "3".into(),
        }];
        let count = |f: &[Filter]| {
            db.query_row(
                &format!(
                    "SELECT count(*) FROM t WHERE {}",
                    predicate(f, &["n".into(), "s".into()], Dialect::Sqlite).unwrap()
                ),
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
        };
        assert_eq!(count(&filters), 1);
        filters[0] = Filter {
            enabled: true,
            column: "s".into(),
            operator: "Contains".into(),
            value: "%_!'".into(),
        };
        assert_eq!(count(&filters), 1);
        filters.push(Filter {
            enabled: true,
            column: "Raw SQL".into(),
            operator: "=".into(),
            value: "n = 2".into(),
        });
        assert_eq!(count(&filters), 1);
        filters[1].value = "set_config('transaction_read_only', 'off', true) = 'off'".into();
        assert!(predicate(&filters, &["s".into()], Dialect::Postgres).is_err());
        filters[1].value = "1); DELETE FROM t; --".into();
        assert!(predicate(&filters, &["s".into()], Dialect::Sqlite).is_err());
    }
}
