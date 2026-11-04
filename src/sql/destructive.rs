//! Flags editor statements that can irreversibly remove data or objects, so the
//! app can ask for confirmation before sending them. This is a guard rail for
//! typos and wrong-window mistakes, not a security boundary: the database's own
//! permissions (and Safe mode) remain the authority.
use super::{Dialect, mysql};
use sqlparser::ast::{AlterColumnOperation, AlterTableOperation, Query, SetExpr, Statement};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Statement text as written, trimmed and shortened for display.
    pub statement: String,
    pub reason: &'static str,
}

const PREVIEW_CHARS: usize = 240;

/// Every statement in `sql` that drops, truncates, deletes or updates without
/// a WHERE clause, or alters a table destructively. Statements the parser does
/// not understand are scanned for those keywords instead, erring towards asking.
pub fn findings(sql: &str, dialect: Dialect) -> Vec<Finding> {
    statement_ranges(sql, dialect)
        .into_iter()
        .filter_map(|range| {
            let text = sql.get(range)?.trim();
            classify(text, dialect).map(|reason| Finding { statement: preview(text), reason })
        })
        .collect()
}

fn statement_ranges(sql: &str, dialect: Dialect) -> Vec<Range<usize>> {
    let ranges = match dialect {
        Dialect::Sqlite => super::statements(sql),
        Dialect::Postgres => super::postgres_ranges(sql, false).unwrap_or_default(),
        Dialect::Mysql(mode) => mysql::editor_ranges(sql, mode),
    };
    // A splitter that finds nothing must not turn into "nothing to confirm".
    if ranges.is_empty() && !sql.trim().is_empty() { std::iter::once(0..sql.len()).collect() } else { ranges }
}

fn classify(text: &str, dialect: Dialect) -> Option<&'static str> {
    let body = text.trim_end_matches(|c: char| c == ';' || c.is_whitespace());
    let parsed = match dialect {
        Dialect::Sqlite => sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::SQLiteDialect {}, body),
        Dialect::Postgres => sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::PostgreSqlDialect {}, body),
        Dialect::Mysql(_) => sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::MySqlDialect {}, body),
    };
    match parsed {
        Ok(statements) if !statements.is_empty() => statements.iter().find_map(statement),
        _ => keywords(body),
    }
}

fn statement(statement: &Statement) -> Option<&'static str> {
    match statement {
        Statement::Truncate(_) => Some("TRUNCATE removes every row"),
        Statement::Delete(delete) if delete.selection.is_none() => Some("DELETE without WHERE removes every row"),
        Statement::Update(update) if update.selection.is_none() => Some("UPDATE without WHERE changes every row"),
        Statement::AlterTable(alter) => alter.operations.iter().find_map(|operation| match operation {
            AlterTableOperation::AlterColumn { op: AlterColumnOperation::SetDataType { .. }, .. }
            | AlterTableOperation::ModifyColumn { .. }
            | AlterTableOperation::ChangeColumn { .. } => Some("ALTER TABLE changes a column definition, which can discard data"),
            other if other.to_string().trim_start().to_ascii_uppercase().starts_with("DROP") => {
                Some("ALTER TABLE drops part of the table")
            }
            _ => None,
        }),
        Statement::Query(query) => query_statement(query),
        // Covers every DROP variant (table, schema, database, function, …)
        // without tracking each AST variant the parser adds.
        other if other.to_string().trim_start().to_ascii_uppercase().starts_with("DROP") => {
            Some("DROP removes a database object and its data")
        }
        _ => None,
    }
}

/// Data-modifying CTEs (`WITH gone AS (DELETE FROM t RETURNING *) SELECT …`).
fn query_statement(query: &Query) -> Option<&'static str> {
    query
        .with
        .iter()
        .flat_map(|with| &with.cte_tables)
        .find_map(|cte| query_statement(&cte.query))
        .or_else(|| set_expression(&query.body))
}

fn set_expression(expression: &SetExpr) -> Option<&'static str> {
    match expression {
        SetExpr::Delete(inner) | SetExpr::Update(inner) => statement(inner),
        SetExpr::Query(query) => query_statement(query),
        SetExpr::SetOperation { left, right, .. } => set_expression(left).or_else(|| set_expression(right)),
        _ => None,
    }
}

/// Fallback for SQL the parser rejects (procedural blocks, vendor syntax).
/// Words inside string bodies count too: a `DO $$ … DROP … $$` block is exactly
/// the kind of statement that should be reviewed.
fn keywords(text: &str) -> Option<&'static str> {
    let words: Vec<String> = strip_comments(text)
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_uppercase)
        .collect();
    let has = |word: &str| words.iter().any(|w| w == word);
    if has("DROP") {
        return Some("Mentions DROP and could not be fully analysed");
    }
    if has("TRUNCATE") {
        return Some("Mentions TRUNCATE and could not be fully analysed");
    }
    // `ON DELETE CASCADE` and `FOR UPDATE` are clauses, not statements.
    let unbounded = words.iter().enumerate().any(|(index, word)| {
        matches!(word.as_str(), "DELETE" | "UPDATE")
            && !index.checked_sub(1).is_some_and(|previous| matches!(words[previous].as_str(), "ON" | "FOR" | "BEFORE" | "AFTER" | "OF"))
    }) && !has("WHERE");
    unbounded.then_some("DELETE or UPDATE without WHERE and could not be fully analysed")
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("--") {
            rest = after.split_once('\n').map_or("", |(_, tail)| tail);
            out.push('\n');
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after.split_once("*/").map_or("", |(_, tail)| tail);
            out.push(' ');
        } else {
            let next = rest.chars().next().unwrap_or_default();
            out.push(next);
            rest = &rest[next.len_utf8()..];
        }
    }
    out
}

fn preview(text: &str) -> String {
    let single_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= PREVIEW_CHARS {
        single_line
    } else {
        format!("{}…", single_line.chars().take(PREVIEW_CHARS).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reasons(sql: &str, dialect: Dialect) -> Vec<&'static str> {
        findings(sql, dialect).into_iter().map(|finding| finding.reason).collect()
    }

    #[test]
    fn reads_and_bounded_writes_need_no_confirmation() {
        for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::for_engine("MySQL")] {
            for sql in [
                "SELECT * FROM customers",
                "SELECT 'DROP TABLE customers; DELETE FROM x'",
                "-- DROP TABLE customers\nSELECT 1",
                "DELETE FROM customers WHERE id = 1",
                "UPDATE customers SET name = 'x' WHERE id = 1",
                "INSERT INTO customers(name) VALUES ('a')",
                "CREATE TABLE child(id int REFERENCES parent(id) ON DELETE CASCADE ON UPDATE CASCADE)",
                "ALTER TABLE customers ADD COLUMN note text",
                "SELECT * FROM customers FOR UPDATE",
            ] {
                assert!(findings(sql, dialect).is_empty(), "{dialect:?}: {sql}");
            }
        }
    }

    #[test]
    fn destructive_statements_are_flagged_per_statement() {
        let found = findings(
            "SELECT 1; DROP TABLE customers;\nDELETE FROM orders; UPDATE t SET x = 1 WHERE id = 2; TRUNCATE audit;",
            Dialect::Postgres,
        );
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].statement, "DROP TABLE customers;");
        assert_eq!(found[1].reason, "DELETE without WHERE removes every row");
        assert_eq!(found[2].reason, "TRUNCATE removes every row");
    }

    #[test]
    fn every_dialect_flags_drops_and_unbounded_writes() {
        for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::for_engine("MariaDB")] {
            for sql in [
                "DROP TABLE customers",
                "drop index if exists customers_name",
                "DELETE FROM customers",
                "UPDATE customers SET name = 'x'",
                // A WHERE inside a subquery does not bound the outer update.
                "UPDATE customers SET name = (SELECT name FROM other WHERE other.id = 1)",
                "ALTER TABLE customers DROP COLUMN name",
            ] {
                assert_eq!(findings(sql, dialect).len(), 1, "{dialect:?}: {sql}");
            }
        }
        assert_eq!(reasons("DROP SCHEMA app CASCADE", Dialect::Postgres), ["DROP removes a database object and its data"]);
        assert_eq!(reasons("DROP DATABASE app", Dialect::for_engine("MySQL")).len(), 1);
        assert_eq!(reasons("TRUNCATE TABLE audit", Dialect::for_engine("MySQL")).len(), 1);
        assert_eq!(reasons("ALTER TABLE t ALTER COLUMN n TYPE integer", Dialect::Postgres).len(), 1);
        assert_eq!(reasons("ALTER TABLE t MODIFY COLUMN n INT", Dialect::for_engine("MySQL")).len(), 1);
    }

    #[test]
    fn data_modifying_ctes_are_flagged() {
        assert_eq!(
            reasons("WITH gone AS (DELETE FROM customers RETURNING *) SELECT count(*) FROM gone", Dialect::Postgres),
            ["DELETE without WHERE removes every row"]
        );
    }

    #[test]
    fn unparseable_statements_fall_back_to_keywords() {
        assert_eq!(
            reasons("DO $$ BEGIN EXECUTE 'DROP TABLE customers'; END $$", Dialect::Postgres).len(),
            1
        );
        assert!(keywords("CREATE TRIGGER t AFTER DELETE ON x BEGIN SELECT 1; END").is_none());
        assert!(keywords("frobnicate DELETE everything").is_some());
    }

    #[test]
    fn previews_are_single_line_and_bounded() {
        let long = format!("DELETE FROM t -- {}\n", "x".repeat(400));
        let found = findings(&long, Dialect::Sqlite);
        assert_eq!(found.len(), 1);
        assert!(found[0].statement.chars().count() <= PREVIEW_CHARS + 1);
        assert!(!found[0].statement.contains('\n'));
    }
}
