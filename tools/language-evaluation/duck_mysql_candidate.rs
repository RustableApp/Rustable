//! Observation harness for the pinned duck-sqllsp revision in the research log.
use dsl_catalog::{CATALOG_VERSION, Catalog, Column, Schema, Table, TableKind};
use dsl_completion::complete;
use dsl_parse::{parse, Dialect};
use dsl_resolve::resolve_with_source;
use text_size::TextSize;

#[test]
fn rustable_mysql_candidate_matrix() {
    let table = Table {
        schema: "shop".into(), name: "customers".into(), kind: TableKind::Table,
        columns: ["id", "name", "日本語"].into_iter().map(|name| Column {
            name: name.into(), data_type: "varchar".into(), nullable: true,
            default: None, comment: None, generated: None, json_keys: None,
        }).collect(),
        constraints: vec![], indexes: vec![], triggers: vec![], policies: vec![],
        comment: None, row_estimate: None, owner: None, definition: None,
        strict: false, options: None,
    };
    let catalog = Catalog {
        version: CATALOG_VERSION, connection_id: "rustable-evaluation".into(),
        schemas: vec![Schema { name: "shop".into(), tables: vec![table] }],
        functions: vec![], types: vec![], roles: vec![], sequences: vec![], extensions: vec![],
    };
    for (case, source, expected) in [
        ("alias", "SELECT c.| FROM customers c", "name"),
        ("quoted alias", "SELECT `c`.| FROM customers AS `c`", "name"),
        ("derived alias", "SELECT d.| FROM (SELECT name AS label FROM customers) d", "label"),
        ("CTE", "WITH c AS (SELECT name AS label FROM customers) SELECT c.| FROM c", "label"),
        ("Unicode prefix", "SELECT c.日| FROM customers c", "日本語"),
        ("string suppression", "SELECT 'c.na|me' FROM customers c", "name"),
        ("comment suppression", "SELECT 1 /* c.na|me */ FROM customers c", "name"),
        ("ANSI quoted alias", "SELECT \"c\".| FROM customers AS \"c\"", "name"),
    ] {
        let (before, after) = source.split_once('|').unwrap();
        let sql = format!("{before}{after}");
        let file = parse(&sql, Dialect::MySql);
        let scopes = resolve_with_source(&file.statements, &sql);
        let items = complete(&sql, &file, &scopes, &catalog, TextSize::from(before.len() as u32));
        let labels: Vec<_> = items.iter().map(|item| item.label.as_str()).collect();
        let postgres_only: Vec<_> = labels.iter().filter(|label| label.starts_with("pg_")).take(5).collect();
        println!("{case}: expected={expected:?}, present={}, count={}, first_labels={:?}, postgres_examples={postgres_only:?}", labels.contains(&expected), labels.len(), &labels[..labels.len().min(8)]);
    }
}
