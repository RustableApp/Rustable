//! Copy to lsp_sqls/tests/rustable_acceptance.rs at the revision in the research log.
//! Reports capability observations; it does not assert that the candidate is accepted.
use sql_lsp::dialect::Dialect;
use sql_lsp::dialects::MysqlDialect;
use sql_lsp::schema::{Column, Schema, SchemaId, Table};
use tower_lsp::lsp_types::Position;

#[tokio::test]
async fn rustable_mysql_candidate_matrix() {
    let schema = Schema {
        id: SchemaId::new(), server: None, namespace: None, catalog: None,
        database: "shop".into(), server_version: None, functions: vec![], source_uri: None,
        tables: vec![Table {
            name: "customers".into(),
            columns: ["id", "name", "日本語"].into_iter().map(|name| Column {
                name: name.into(), data_type: "varchar".into(), nullable: true,
                ..Default::default()
            }).collect(), ..Default::default()
        }],
    };
    let dialect = MysqlDialect::new();
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
        let items = dialect.completion(&sql, Position::new(0, before.encode_utf16().count() as u32), Some(&schema)).await;
        let labels: Vec<_> = items.iter().map(|item| item.label.as_str()).collect();
        println!("{case}: expected={expected:?}, present={}, labels={labels:?}", labels.contains(&expected));
    }
    for (case, sql) in [
        ("ordinary SELECT", "SELECT name FROM customers"),
        ("MariaDB RETURNING", "INSERT INTO customers(id) VALUES (1) RETURNING id"),
        ("backslash mode dependent", "SELECT 'a\\' AS label"),
        ("ANSI mode dependent", "SELECT \"name\" FROM customers"),
    ] {
        let diagnostics = dialect.parse(sql, Some(&schema)).await;
        println!("parse {case}: {:?}", diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>());
    }
}
