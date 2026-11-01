//! Output names of complete derived-table queries. Never executes SQL.
use super::*;
use sqlparser::{ast::{Expr, Query, SelectItem, SetExpr, Statement, TableFactor}, parser::Parser};

pub(super) type Bindings = std::collections::HashMap<String, Vec<String>>;

pub(super) fn columns(sql: &str, dialect: Dialect, objects: &[Object], bindings: &Bindings) -> Vec<String> {
    if sql.len() > 64 * 1024 { return Vec::new(); }
    let parsed = match dialect {
        Dialect::Postgres => Parser::parse_sql(&sqlparser::dialect::PostgreSqlDialect {}, sql),
        Dialect::Sqlite => Parser::parse_sql(&sqlparser::dialect::SQLiteDialect {}, sql),
        Dialect::Mysql(mode) => Parser::parse_sql(&crate::sql::mysql::Lexer(mode), sql),
    };
    let Ok(statements) = parsed else { return Vec::new(); };
    match statements.first() {
        Some(Statement::Query(query)) => query_columns(query, objects, bindings, dialect, 0),
        _ => Vec::new(),
    }
}
// PostgreSQL permits renaming only a prefix of a relation's output columns.
// Empty strings represent unknown generated names; keep their positions until
// the final suggestion list, rather than shifting subsequent named columns.
pub(super) fn rename_columns(mut columns: Vec<String>, aliases: Vec<String>, dialect: Dialect) -> Vec<String> {
    if aliases.is_empty() { return columns; }
    if !matches!(dialect, Dialect::Postgres) { return aliases; }
    if columns.len() < aliases.len() { columns.resize(aliases.len(), String::new()); }
    for (column, alias) in columns.iter_mut().zip(aliases) { *column = alias; }
    columns
}
fn query_columns(query: &Query, objects: &[Object], bindings: &Bindings, dialect: Dialect, depth: usize) -> Vec<String> {
    if depth >= 16 { return Vec::new(); }
    let mut local = bindings.clone();
    if let Some(with) = &query.with {
        for cte in with.cte_tables.iter().take(128) {
            let name = cte.alias.name.value.to_lowercase();
            if local.len() >= 128 && !local.contains_key(&name) { break; }
            if with.recursive { local.insert(name.clone(), Vec::new()); }
            let mut output = query_columns(&cte.query, objects, &local, dialect, depth + 1);
            if !cte.alias.columns.is_empty() { output = rename_columns(output, cte.alias.columns.iter().take(1024).map(|c| c.name.value.clone()).collect(), dialect); }
            local.insert(name, output);
        }
    }
    body_columns(&query.body, objects, &local, dialect, depth)
}
fn body_columns(body: &SetExpr, objects: &[Object], bindings: &Bindings, dialect: Dialect, depth: usize) -> Vec<String> {
    let select = match body {
        SetExpr::Select(select) => select,
        SetExpr::Query(query) => return query_columns(query, objects, bindings, dialect, depth + 1),
        SetExpr::SetOperation { left, .. } => return body_columns(left, objects, bindings, dialect, depth + 1),
        _ => return Vec::new(),
    };
    if depth >= 16 { return Vec::new(); }
    let sources: Vec<_> = select.from.iter().flat_map(|from|
        std::iter::once(&from.relation).chain(from.joins.iter().map(|join| &join.relation)))
        .take(128).filter_map(|factor| match factor {
            TableFactor::Table { name, alias, args: None, .. } => {
                let names: Vec<_> = name.0.iter().filter_map(|part| part.as_ident().map(|id| id.value.as_str())).collect();
                let table = *names.last()?;
                let mut columns = if names.len() == 1 && bindings.contains_key(&table.to_lowercase()) {
                    bindings[&table.to_lowercase()].clone()
                } else {
                let object = objects.iter().find(|o| o.display_name.as_deref().unwrap_or(&o.name).eq_ignore_ascii_case(table)
                    && (names.len() == 1 || o.schema.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(names[names.len()-2]))))?;
                object.columns.iter().take(1024).map(|c| c.name.clone()).collect::<Vec<_>>()
                };
                if let Some(alias) = alias && !alias.columns.is_empty() { columns = rename_columns(columns, alias.columns.iter().take(1024).map(|c| c.name.value.clone()).collect(), dialect); }
                Some((alias.as_ref().map_or(table, |a| a.name.value.as_str()).to_owned(), columns))
            }
            TableFactor::Derived { subquery, alias: Some(alias), .. } => {
                let mut columns = query_columns(subquery, objects, bindings, dialect, depth + 1);
                if !alias.columns.is_empty() { columns = rename_columns(columns, alias.columns.iter().take(1024).map(|c| c.name.value.clone()).collect(), dialect); }
                Some((alias.name.value.clone(), columns))
            }
            _ => None,
        }).collect();
    let mut result = Vec::new();
    for item in &select.projection {
        match item {
            SelectItem::ExprWithAlias { alias, .. } => result.push(alias.value.clone()),
            SelectItem::UnnamedExpr(Expr::Function(function)) if matches!(dialect, Dialect::Postgres) => {
                let name = function.name.0.last().and_then(|part| part.as_ident())
                    .map(|id| if id.quote_style.is_some() { id.value.clone() } else { id.value.to_lowercase() })
                    .unwrap_or_default();
                result.push(name);
            }
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
            _ => result.push(String::new()), // Keep an unknown-name slot so positional aliases stay aligned.
        }
        result.truncate(1024);
    }
    result
}

/// Read a WITH prefix even when the main SELECT is unfinished at the caret.
/// Only preceding declarations are visible inside a non-recursive CTE body.
pub(super) fn with_bindings(tokens: &[sqlparser::tokenizer::TokenWithSpan], caret: usize, dialect: Dialect, objects: &[Object], inherited: &Bindings) -> Bindings {
    use sqlparser::tokenizer::Token;
    let mut bindings = inherited.clone();
    let word = |i: usize, value: &str| tokens.get(i).is_some_and(|t| matches!(&t.token, Token::Word(w) if w.quote_style.is_none() && w.value.eq_ignore_ascii_case(value)));
    let end = tokens.len();
    if !word(0, "WITH") { return bindings; }
    let mut i = 1;
    let recursive = word(i, "RECURSIVE");
    if recursive { i += 1; }
    for _ in 0..128 {
        let Some(Token::Word(name)) = tokens.get(i).map(|t| &t.token) else { break; };
        let name = name.value.to_lowercase();
        if bindings.len() >= 128 && !bindings.contains_key(&name) { break; }
        i += 1;
        let mut aliases = Vec::new();
        if tokens.get(i).is_some_and(|t| t.token == Token::LParen) {
            i += 1;
            while i < end && tokens[i].token != Token::RParen {
                if let Token::Word(w) = &tokens[i].token && aliases.len() < 1024 { aliases.push(w.value.clone()); }
                i += 1;
            }
            i += 1;
        }
        if !word(i, "AS") { break; }
        i += 1;
        if word(i, "NOT") { i += 1; }
        if word(i, "MATERIALIZED") { i += 1; }
        if !tokens.get(i).is_some_and(|t| t.token == Token::LParen) { break; }
        let body = i + 1;
        let mut nesting = 1usize;
        i += 1;
        while i < end {
            match tokens[i].token { Token::LParen => nesting += 1, Token::RParen => nesting -= 1, _ => {} }
            if nesting == 0 { break; }
            i += 1;
        }
        if recursive { bindings.insert(name.clone(), aliases.clone()); }
        if body <= caret && caret <= i {
            // The nonrecursive anchor determines recursive output names. Parse
            // that arm separately when the caret leaves the recursive term
            // incomplete (for example, SELECT cte.).
            if recursive && (aliases.is_empty() || matches!(dialect, Dialect::Postgres)) {
                let mut depth = 0usize;
                for arm_end in body..i {
                    match &tokens[arm_end].token {
                        Token::LParen => depth += 1,
                        Token::RParen => depth = depth.saturating_sub(1),
                        Token::Word(w) if depth == 0 && w.quote_style.is_none() && w.value.eq_ignore_ascii_case("UNION") => {
                            let sql = tokens[body..arm_end].iter().map(|t| t.token.to_string()).collect::<Vec<_>>().join(" ");
                            let output = rename_columns(columns(&sql, dialect, objects, &bindings), aliases.clone(), dialect);
                            bindings.insert(name, output);
                            break;
                        }
                        _ => {}
                    }
                }
            }
            break;
        }
        if i >= end { break; }
        let sql = tokens[body..i].iter().map(|t| t.token.to_string()).collect::<Vec<_>>().join(" ");
        let output = if !aliases.is_empty() && !matches!(dialect, Dialect::Postgres) { aliases }
            else { rename_columns(columns(&sql, dialect, objects, &bindings), aliases, dialect) };
        bindings.insert(name, output);
        i += 1;
        if !tokens.get(i).is_some_and(|t| t.token == Token::Comma) { break; }
        i += 1;
    }
    bindings
}

#[cfg(test)]
mod live_tests {
    use super::*;

    #[test]
    #[ignore = "requires disposable PostgreSQL fixture on 54329"]
    fn postgres_function_projection_matches_server_labels() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let mut client = ::postgres::Client::connect(
            "host=127.0.0.1 port=54329 dbname=rustable_qa user=postgres password=rustable_qa_only",
            ::postgres::NoTls).unwrap();
        for sql in [
            "SELECT COUNT(*)",
            "SELECT pg_catalog.LOWER('A'), count(*) OVER ()",
            "SELECT COUNT(*) AS total",
            "WITH totals(renamed) AS (SELECT COUNT(*), MAX(1)) SELECT * FROM totals",
            "SELECT pg_catalog.\"lower\"('A')",
        ] {
            let statement = client.prepare(sql).unwrap();
            let server: Vec<_> = statement.columns().iter().map(|column| column.name().to_owned()).collect();
            assert_eq!(columns(sql, Dialect::Postgres, &[], &Bindings::new()), server, "{sql}");
        }
    }
}
