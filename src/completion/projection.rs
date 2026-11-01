//! Output names of complete derived-table queries. Never executes SQL.
use super::*;
use sqlparser::{ast::{Expr, Query, SelectItem, SetExpr, Statement, TableFactor}, parser::Parser};

pub(super) fn columns(sql: &str, dialect: Dialect, objects: &[Object]) -> Vec<String> {
    if sql.len() > 64 * 1024 { return Vec::new(); }
    let parsed = match dialect {
        Dialect::Postgres => Parser::parse_sql(&sqlparser::dialect::PostgreSqlDialect {}, sql),
        Dialect::Sqlite => Parser::parse_sql(&sqlparser::dialect::SQLiteDialect {}, sql),
        Dialect::Mysql(mode) => Parser::parse_sql(&crate::sql::mysql::Lexer(mode), sql),
    };
    let Ok(statements) = parsed else { return Vec::new(); };
    match statements.first() {
        Some(Statement::Query(query)) => query_columns(query, objects, 0),
        _ => Vec::new(),
    }
}
fn query_columns(query: &Query, objects: &[Object], depth: usize) -> Vec<String> {
    if depth >= 16 { return Vec::new(); }
    // Until WITH bindings are resolved here, never mistake a CTE shadowing a
    // catalog table for that table when expanding a wildcard.
    body_columns(&query.body, if query.with.is_some() { &[] } else { objects }, depth)
}
fn body_columns(body: &SetExpr, objects: &[Object], depth: usize) -> Vec<String> {
    let select = match body {
        SetExpr::Select(select) => select,
        SetExpr::Query(query) => return query_columns(query, objects, depth + 1),
        SetExpr::SetOperation { left, .. } => return body_columns(left, objects, depth + 1),
        _ => return Vec::new(),
    };
    if depth >= 16 { return Vec::new(); }
    let sources: Vec<_> = select.from.iter().flat_map(|from|
        std::iter::once(&from.relation).chain(from.joins.iter().map(|join| &join.relation)))
        .take(128).filter_map(|factor| match factor {
            TableFactor::Table { name, alias, args: None, .. } => {
                let names: Vec<_> = name.0.iter().filter_map(|part| part.as_ident().map(|id| id.value.as_str())).collect();
                let table = *names.last()?;
                let object = objects.iter().find(|o| o.display_name.as_deref().unwrap_or(&o.name).eq_ignore_ascii_case(table)
                    && (names.len() == 1 || o.schema.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(names[names.len()-2]))))?;
                let mut columns: Vec<_> = object.columns.iter().take(1024).map(|c| c.name.clone()).collect();
                if let Some(alias) = alias && !alias.columns.is_empty() { columns = alias.columns.iter().take(1024).map(|c| c.name.value.clone()).collect(); }
                Some((alias.as_ref().map_or(table, |a| a.name.value.as_str()).to_owned(), columns))
            }
            TableFactor::Derived { subquery, alias: Some(alias), .. } => {
                let mut columns = query_columns(subquery, objects, depth + 1);
                if !alias.columns.is_empty() { columns = alias.columns.iter().take(1024).map(|c| c.name.value.clone()).collect(); }
                Some((alias.name.value.clone(), columns))
            }
            _ => None,
        }).collect();
    let mut result = Vec::new();
    for item in &select.projection {
        match item {
            SelectItem::ExprWithAlias { alias, .. } => result.push(alias.value.clone()),
            SelectItem::UnnamedExpr(Expr::Identifier(id)) => result.push(id.value.clone()),
            SelectItem::UnnamedExpr(Expr::CompoundIdentifier(ids)) => {
                if let Some(id) = ids.last() { result.push(id.value.clone()); }
            }
            SelectItem::Wildcard(_) => result.extend(sources.iter().flat_map(|(_, columns)| columns.iter().cloned()).take(1024 - result.len())),
            SelectItem::QualifiedWildcard(sqlparser::ast::SelectItemQualifiedWildcardKind::ObjectName(name), _) => {
                if let Some(id) = name.0.last().and_then(|part| part.as_ident()) {
                    result.extend(sources.iter().filter(|(alias, _)| alias.eq_ignore_ascii_case(&id.value)).flat_map(|(_, columns)| columns.iter().cloned()).take(1024 - result.len()));
                }
            }
            _ => {} // Unaliased expressions have engine-specific generated names.
        }
        result.truncate(1024);
    }
    result
}
