//! Recognizes direct columns from a single base table without catalog requests.
//! Result order is retained; editing requires every primary-key column. Joins,
//! aliases of columns, expressions and aggregate results remain read-only.
//! Save planners validate named snapshots against fresh metadata and check the
//! selected original values before writing. Omitted columns are never inferred.
use crate::db::{Column, Object};
use crate::sql::Dialect;
use sqlparser::ast::{
    Ident, ObjectNamePart, Query, SelectItem, SelectItemQualifiedWildcardKind, SetExpr, Statement,
    TableFactor,
};
use sqlparser::parser::Parser;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Target {
    /// Table reference passed to the planner in `Change::table`.
    pub table: String,
    /// Selected table columns, in result order.
    pub columns: Vec<Column>,
    /// Short name for status text.
    pub label: String,
    pub missing_key: bool,
    pub projected: bool,
}

pub(crate) fn target(
    sql: &str,
    dialect: Dialect,
    objects: &[Object],
    selected_schema: Option<&str>,
    result_columns: &[String],
) -> Option<Target> {
    if sql.len() > 64 * 1024 {
        return None;
    }
    let (reference, projection) = single_table(sql, dialect)?;
    let (object, table) = resolve(&reference, dialect, objects, selected_schema)?;
    if object.kind != "table" || object.metadata_error.is_some() { return None; }
    let columns = if let Some(names) = projection {
        if names.len() != result_columns.len() { return None; }
        names.iter().zip(result_columns).map(|(name, result)| {
            let column = object.columns.iter().find(|column| match dialect {
                Dialect::Postgres => column.name == folded(name, dialect),
                _ => column.name.eq_ignore_ascii_case(&name.value),
            })?;
            (column.name == *result).then(|| column.clone())
        }).collect::<Option<Vec<_>>>()?
    } else {
        if object.columns.len() != result_columns.len() || !object.columns.iter().zip(result_columns).all(|(c,n)| c.name == *n) { return None; }
        object.columns.clone()
    };
    let mut seen = std::collections::HashSet::new();
    if !columns.iter().all(|column| seen.insert(&column.name)) { return None; }
    let missing_key = !object.columns.iter().any(|column| column.pk)
        || object.columns.iter().filter(|column| column.pk).any(|key| !columns.iter().any(|column| column.name == key.name));
    let projected = columns != object.columns;
    Some(Target { table, columns, label: object.display_name().to_owned(), missing_key, projected })
}

type Reference = Vec<Ident>;

fn parse(sql: &str, dialect: Dialect) -> Option<Vec<Statement>> {
    match dialect {
        Dialect::Postgres => Parser::parse_sql(&sqlparser::dialect::PostgreSqlDialect {}, sql),
        Dialect::Sqlite => Parser::parse_sql(&sqlparser::dialect::SQLiteDialect {}, sql),
        Dialect::Mysql(mode) => Parser::parse_sql(&crate::sql::mysql::Lexer(mode), sql),
    }
    .ok()
}

/// Accepts a wildcard or direct column list over one plain table, with
/// optional WHERE, ORDER BY, LIMIT/OFFSET and row locks. Every other clause is
/// compared against a parsed template so new parser fields default to read-only.
fn single_table(sql: &str, dialect: Dialect) -> Option<(Reference, Option<Vec<Ident>>)> {
    let mut statements = parse(sql, dialect)?;
    if statements.len() != 1 {
        return None;
    }
    let Statement::Query(query) = statements.pop()? else { return None };
    let Some(Statement::Query(template)) = parse("SELECT * FROM rustable_template", dialect)?.pop() else {
        return None;
    };
    let template = *template;
    let SetExpr::Select(user) = query.body.as_ref() else { return None };
    let SetExpr::Select(base) = template.body.as_ref() else { return None };
    // Query-level clauses: only ordering, limits and locking are permitted.
    let expected_query = Query {
        body: query.body.clone(),
        order_by: query.order_by.clone(),
        limit_clause: query.limit_clause.clone(),
        locks: query.locks.clone(),
        ..template.clone()
    };
    if *query != expected_query {
        return None;
    }
    let [from] = user.from.as_slice() else { return None };
    if !from.joins.is_empty() {
        return None;
    }
    let TableFactor::Table { name, alias, .. } = &from.relation else { return None };
    if alias.as_ref().is_some_and(|alias| !alias.columns.is_empty()) {
        return None;
    }
    let mut relation = base.from[0].relation.clone();
    if let TableFactor::Table { name: n, alias: a, .. } = &mut relation {
        *n = name.clone();
        *a = alias.clone();
    }
    if relation != from.relation {
        return None;
    }
    let mut expected_select = base.as_ref().clone();
    expected_select.select_token = user.select_token.clone();
    expected_select.from = user.from.clone();
    expected_select.selection = user.selection.clone();
    expected_select.projection = user.projection.clone();
    if **user != expected_select {
        return None;
    }
    let reference = name
        .0
        .iter()
        .map(|part| match part {
            ObjectNamePart::Identifier(ident) => Some(ident.clone()),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    if user.projection.iter().all(|item| matches!(item, SelectItem::UnnamedExpr(_))) {
        let columns = user.projection.iter().map(|item| {
            let SelectItem::UnnamedExpr(expr) = item else { return None };
            match expr {
                sqlparser::ast::Expr::Identifier(name) => Some(name.clone()),
                sqlparser::ast::Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
                    let qualifier = alias.as_ref().map(|alias| &alias.name).or_else(|| reference.last())?;
                    same_ident(&parts[0], qualifier, dialect).then(|| parts[1].clone())
                }
                _ => None,
            }
        }).collect::<Option<Vec<_>>>()?;
        return Some((reference, Some(columns)));
    }
    let [item] = user.projection.as_slice() else { return None };
    match item {
        SelectItem::Wildcard(options) if Some(options) == wildcard_options(&base.projection[0]) => {}
        SelectItem::QualifiedWildcard(SelectItemQualifiedWildcardKind::ObjectName(prefix), options)
            if Some(options) == wildcard_options(&base.projection[0]) =>
        {
            let prefix = prefix
                .0
                .iter()
                .map(|part| match part {
                    ObjectNamePart::Identifier(ident) => Some(ident.clone()),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            let matches = match alias {
                Some(alias) => prefix.len() == 1 && same_ident(&prefix[0], &alias.name, dialect),
                None => {
                    prefix.len() <= reference.len()
                        && prefix
                            .iter()
                            .rev()
                            .zip(reference.iter().rev())
                            .all(|(a, b)| same_ident(a, b, dialect))
                }
            };
            if !matches {
                return None;
            }
        }
        _ => return None,
    }
    Some((reference, None))
}

fn wildcard_options(item: &SelectItem) -> Option<&sqlparser::ast::WildcardAdditionalOptions> {
    match item {
        SelectItem::Wildcard(options) => Some(options),
        _ => None,
    }
}

/// Unquoted identifiers fold as the server does when resolving relations.
fn folded(ident: &Ident, dialect: Dialect) -> String {
    match (dialect, ident.quote_style) {
        (Dialect::Postgres, None) => ident.value.to_lowercase(),
        _ => ident.value.clone(),
    }
}

fn same_ident(a: &Ident, b: &Ident, dialect: Dialect) -> bool {
    match dialect {
        Dialect::Postgres => folded(a, dialect) == folded(b, dialect),
        Dialect::Sqlite | Dialect::Mysql(_) => a.value.eq_ignore_ascii_case(&b.value),
    }
}

fn resolve<'a>(
    reference: &[Ident],
    dialect: Dialect,
    objects: &'a [Object],
    selected_schema: Option<&str>,
) -> Option<(&'a Object, String)> {
    let tables = || objects.iter().filter(|object| object.kind == "table");
    match dialect {
        Dialect::Postgres => {
            let (schema, relation) = match reference {
                [relation] => (selected_schema?.to_owned(), folded(relation, dialect)),
                [schema, relation] => (folded(schema, dialect), folded(relation, dialect)),
                _ => return None,
            };
            let object = tables().find(|object| {
                object.schema.as_deref() == Some(schema.as_str())
                    && object.display_name() == relation
            })?;
            // The caller supplies the schema established by wire column origins,
            // so later search_path changes cannot redirect this result's edits.
            let table = object.name.clone();
            Some((object, table))
        }
        Dialect::Mysql(_) => {
            let relation = match reference {
                [relation] => relation,
                [schema, relation] if Some(schema.value.as_str()) == selected_schema => relation,
                _ => return None,
            };
            let object = tables().find(|object| object.name == relation.value).or_else(|| {
                let mut matches = tables().filter(|object| object.name.eq_ignore_ascii_case(&relation.value));
                let first = matches.next()?;
                matches.next().is_none().then_some(first)
            })?;
            Some((object, object.name.clone()))
        }
        Dialect::Sqlite => {
            let relation = match reference {
                [relation] => relation,
                [schema, relation] if schema.value.eq_ignore_ascii_case("main") => relation,
                _ => return None,
            };
            let object = tables().find(|object| object.name.eq_ignore_ascii_case(&relation.value))?;
            Some((object, object.name.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str, pk: bool) -> Column {
        Column {
            name: name.into(),
            data_type: "text".into(),
            not_null: pk,
            default: String::new(),
            pk,
            generated: false,
        }
    }
    fn object(schema: Option<&str>, name: &str, display: Option<&str>, kind: &str, pk: bool) -> Object {
        Object {
            schema: schema.map(Into::into),
            display_name: display.map(Into::into),
            metadata_error: None,
            columns: vec![column("id", pk), column("name", false)],
            name: name.into(),
            kind: kind.into(),
            ddl: String::new(),
        }
    }
    fn result() -> Vec<String> {
        vec!["id".into(), "name".into()]
    }
    fn sqlite(sql: &str) -> Option<Target> {
        target(sql, Dialect::Sqlite, &[object(None, "customers", None, "table", true)], None, &result())
    }
    fn postgres(sql: &str) -> Option<Target> {
        let objects = [object(Some("public"), "\"public\".\"customers\"", Some("customers"), "table", true)];
        target(sql, Dialect::Postgres, &objects, Some("public"), &result())
    }

    #[test]
    fn accepts_plain_single_table_row_images() {
        for sql in [
            "SELECT * FROM customers WHERE id = 123",
            "select * from customers",
            "SELECT * FROM customers WHERE name LIKE 'a%' ORDER BY id DESC LIMIT 10 OFFSET 5",
            "SELECT c.* FROM customers c WHERE c.id = 1",
            "SELECT customers.* FROM main.customers",
            "SELECT * FROM \"Customers\";",
        ] {
            let target = sqlite(sql).unwrap_or_else(|| panic!("{sql} should be editable"));
            assert_eq!(target.table, "customers");
            assert_eq!(target.label, "customers");
        }
    }

    #[test]
    fn rejects_shapes_that_are_not_one_table_row() {
        for sql in [
            "SELECT id + 1, name FROM customers",
            "SELECT *, 1 FROM customers",
            "SELECT DISTINCT * FROM customers",
            "SELECT * FROM customers GROUP BY id",
            "SELECT * FROM customers c JOIN customers d ON c.id = d.id",
            "SELECT * FROM customers, customers AS d",
            "WITH c AS (SELECT * FROM customers) SELECT * FROM c",
            "SELECT * FROM (SELECT * FROM customers) AS c",
            "SELECT * FROM customers UNION SELECT * FROM customers",
            "SELECT * FROM customers; SELECT * FROM customers",
            "SELECT d.* FROM customers c",
            "SELECT * FROM customers AS c(a, b)",
            "SELECT * FROM other.customers",
            "SELECT * FROM missing",
            "UPDATE customers SET name = 'x'",
        ] {
            assert_eq!(sqlite(sql), None, "{sql} must stay read-only");
        }
    }

    #[test]
    fn requires_matching_columns_primary_key_and_base_table() {
        let objects = [object(None, "customers", None, "table", true)];
        let reordered = vec!["name".to_owned(), "id".to_owned()];
        assert_eq!(target("SELECT * FROM customers", Dialect::Sqlite, &objects, None, &reordered), None);
        let no_pk = [object(None, "customers", None, "table", false)];
        assert!(target("SELECT * FROM customers", Dialect::Sqlite, &no_pk, None, &result()).unwrap().missing_key);
        let view = [object(None, "customers", None, "view", true)];
        assert_eq!(target("SELECT * FROM customers", Dialect::Sqlite, &view, None, &result()), None);
    }

    #[test]
    fn postgres_pins_resolved_names_to_the_origin_schema() {
        assert_eq!(postgres("SELECT * FROM customers WHERE id = 1").unwrap().table, "\"public\".\"customers\"");
        assert_eq!(postgres("SELECT * FROM Customers").unwrap().table, "\"public\".\"customers\"");
        assert_eq!(postgres("SELECT * FROM public.customers").unwrap().table, "\"public\".\"customers\"");
        assert_eq!(postgres("SELECT * FROM \"public\".\"customers\"").unwrap().table, "\"public\".\"customers\"");
        // Quoted identifiers are case-sensitive in PostgreSQL.
        assert_eq!(postgres("SELECT * FROM \"Customers\""), None);
        assert_eq!(postgres("SELECT * FROM sales.customers"), None);
        assert_eq!(postgres("SELECT * FROM db.public.customers"), None);
    }

    #[test]
    fn mysql_resolves_only_the_selected_database() {
        let objects = [object(Some("shop"), "customers", None, "table", true)];
        let mysql = |sql: &str| target(sql, Dialect::Mysql(Default::default()), &objects, Some("shop"), &result());
        assert_eq!(mysql("SELECT * FROM `customers` WHERE id = 1").unwrap().table, "customers");
        assert_eq!(mysql("SELECT * FROM shop.customers").unwrap().table, "customers");
        assert_eq!(mysql("SELECT * FROM other.customers"), None);
    }
}
