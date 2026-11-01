//! Offline, catalog-backed completion using the upstream PostgreSQL completion engine.
use pgls_catalog::{Column, ColumnClassKind, Schema, Snapshot, Table};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, Read, Write},
    path::Path,
    sync::Arc,
};

#[path = "completion_scope.rs"]
mod scope;

const FRAME_LIMIT: usize = 2 * 1024 * 1024;
const DOCUMENT_LIMIT: usize = 1024 * 1024;
const CATALOG_LIMIT: u64 = 16 * 1024 * 1024;
const DOCUMENT_COUNT: usize = 8;

#[derive(Deserialize)]
struct Catalog {
    tables: Vec<CatalogTable>,
}
#[derive(Deserialize)]
struct CatalogTable {
    schema: String,
    name: String,
    columns: Vec<CatalogColumn>,
}
#[derive(Deserialize)]
struct CatalogColumn {
    name: String,
    #[serde(default)]
    data_type: String,
    #[serde(default)]
    not_null: bool,
}
type CatalogColumns = BTreeMap<String, Option<Arc<Vec<String>>>>;
struct CatalogSnapshot {
    schema: Snapshot,
    qualified: BTreeMap<String, CatalogColumns>,
    unqualified: CatalogColumns,
}
impl CatalogSnapshot {
    fn columns(&self, reference: &[String]) -> Arc<Vec<String>> {
        let found = match reference {
            [table] => self.unqualified.get(table),
            [schema, table] => self.qualified.get(schema).and_then(|tables| tables.get(table)),
            _ => None,
        };
        found.and_then(Option::as_ref).cloned().unwrap_or_default()
    }
}
fn snapshot(catalog: Catalog) -> CatalogSnapshot {
    let mut result = Snapshot::default();
    let mut qualified: BTreeMap<String, CatalogColumns> = BTreeMap::new();
    let mut unqualified = BTreeMap::new();
    for (index, table) in catalog.tables.into_iter().enumerate() {
        let id = index as i64 + 1;
        // Built once per catalog, off the UI thread. Shared bounded lists avoid
        // scanning or copying unrelated columns for every query binding.
        let columns = Arc::new(table.columns.iter().take(2048).map(|c| c.name.clone()).collect());
        qualified.entry(table.schema.clone()).or_default()
            .entry(table.name.clone()).and_modify(|value| *value = None)
            .or_insert_with(|| Some(Arc::clone(&columns)));
        unqualified.entry(table.name.clone()).and_modify(|value| *value = None)
            .or_insert(Some(columns));
        if !result.schemas.iter().any(|s| s.name == table.schema) {
            result.schemas.push(Schema {
                id: result.schemas.len() as i64 + 1,
                name: table.schema.clone(),
                ..Default::default()
            });
        }
        result.tables.push(Table {
            id,
            schema: table.schema.clone(),
            name: table.name.clone(),
            ..Default::default()
        });
        for (number, column) in table.columns.into_iter().enumerate() {
            result.columns.push(Column {
                name: column.name,
                table_name: table.name.clone(),
                table_oid: id,
                class_kind: ColumnClassKind::OrdinaryTable,
                number: number as i64 + 1,
                schema_name: table.schema.clone(),
                type_id: 0,
                type_name: (!column.data_type.is_empty()).then_some(column.data_type),
                is_nullable: !column.not_null,
                is_primary_key: false,
                is_unique: false,
                default_expr: None,
                varchar_length: None,
                comment: None,
            });
        }
    }
    CatalogSnapshot { schema: result, qualified, unqualified }
}

fn read_frame(input: &mut impl BufRead) -> Result<Option<Value>, String> {
    let mut length = None;
    let mut total = 0;
    loop {
        // A bounded reader prevents an unterminated hostile header growing indefinitely.
        let mut bytes = Vec::new();
        let count = input
            .take(8193)
            .read_until(b'\n', &mut bytes)
            .map_err(|e| e.to_string())?;
        if count == 0 {
            return if total == 0 {
                Ok(None)
            } else {
                Err("Incomplete header".into())
            };
        }
        total += count;
        if count > 8192 || total > 8192 {
            return Err("Header too large".into());
        }
        let line = std::str::from_utf8(&bytes)
            .map_err(|_| "Invalid header")?
            .trim();
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':')
            && key.eq_ignore_ascii_case("Content-Length")
        {
            if length.is_some() {
                return Err("Duplicate content length".into());
            }
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "Invalid content length")?,
            );
        }
    }
    let length = length.ok_or("Missing content length")?;
    if length > FRAME_LIMIT {
        return Err("Frame too large".into());
    }
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| e.to_string())
}
fn write_frame(output: &mut impl Write, value: Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
    write!(output, "Content-Length: {}\r\n\r\n", bytes.len()).map_err(|e| e.to_string())?;
    output
        .write_all(&bytes)
        .and_then(|_| output.flush())
        .map_err(|e| e.to_string())
}
fn byte_position(text: &str, position: &Value) -> Option<usize> {
    let line = usize::try_from(position["line"].as_u64()?).ok()?;
    let character = usize::try_from(position["character"].as_u64()?).ok()?;
    let mut start = 0;
    for _ in 0..line {
        start += text.get(start..)?.find('\n')? + 1;
    }
    let segment = text
        .get(start..)?
        .split('\n')
        .next()?
        .trim_end_matches('\r');
    let mut utf16 = 0;
    for (offset, ch) in segment.char_indices() {
        if utf16 == character {
            return Some(start + offset);
        }
        utf16 += ch.len_utf16();
        if utf16 > character {
            return None;
        }
    }
    (utf16 == character).then_some(start + segment.len())
}
fn position(text: &str, byte: usize) -> Option<Value> {
    let prefix = text.get(..byte)?;
    let line = prefix.bytes().filter(|b| *b == b'\n').count();
    let character = prefix.rsplit('\n').next()?.encode_utf16().count();
    Some(json!({"line":line,"character":character}))
}
fn complete(snapshot: &CatalogSnapshot, text: &str, cursor: usize) -> Vec<Value> {
    // Incomplete editing states can panic inside upstream tree-sitter context
    // extraction (notably immediately after a semicolon). Preserve the LSP
    // session and local fallback instead of terminating all future completions.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        complete_inner(snapshot, text, cursor)
    }))
    .unwrap_or_default()
}
fn complete_inner(snapshot: &CatalogSnapshot, text: &str, cursor: usize) -> Vec<Value> {
    let ranges = pgls_statement_splitter::split(text).ranges;
    let Some(range) = ranges
        .into_iter()
        .find(|r| usize::from(r.start()) <= cursor && cursor <= usize::from(r.end()))
    else {
        return vec![];
    };
    let offset = usize::from(range.start());
    let statement = &text[offset..usize::from(range.end())];
    let scoped = scope::complete(snapshot, statement, cursor - offset);
    let known_scope = scoped.is_some();
    if let Some(items) = scoped.filter(|items| !items.is_empty()) {
        return items.into_iter().filter_map(|(label, range, replacement)| Some(json!({
            "label": label, "kind": 5, "detail": "Column in current query scope",
            "textEdit": {"range": {"start": position(text, offset + range.start)?, "end": position(text, offset + range.end)?}, "newText": replacement},
            "insertTextFormat": 1,
        }))).collect();
    }
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&pgls_treesitter_grammar::LANGUAGE.into())
        .is_err()
    {
        return vec![];
    }
    let Some(tree) = parser.parse(statement, None) else {
        return vec![];
    };
    pgls_completions::complete(pgls_completions::CompletionParams {
        position: ((cursor - offset) as u32).into(), schema: &snapshot.schema,
        text: statement.into(), tree: &tree,
    }).into_iter().take(50).filter_map(|item| {
        use pgls_completions::CompletionItemKind as Kind;
        if known_scope && matches!(item.kind, Kind::Column) { return None; }
        let kind = match item.kind { Kind::Table => 7, Kind::Column => 5, Kind::Function => 3, Kind::Keyword => 14, Kind::Schema => 9, _ => 12 };
        let mut value = json!({"label":item.label,"detail":item.detail.unwrap_or(item.description),"kind":kind,"sortText":item.sort_text,"preselect":item.preselected});
        if let Some(edit) = item.completion_text {
            let start = offset + usize::from(edit.range.start());
            let end = offset + usize::from(edit.range.end());
            if start > end || end > text.len() { return None; }
            value["textEdit"] = json!({"range":{"start":position(text,start)?,"end":position(text,end)?},"newText":edit.text});
            value["insertTextFormat"] = json!(if edit.is_snippet {2} else {1});
        }
        Some(value)
    }).collect()
}
fn serve(
    input: &mut impl BufRead,
    output: &mut impl Write,
    snapshot: &CatalogSnapshot,
) -> Result<(), String> {
    let mut documents = BTreeMap::<String, String>::new();
    let mut shutdown = false;
    while let Some(message) = read_frame(input)? {
        let method = message["method"].as_str().unwrap_or_default();
        let params = &message["params"];
        let uri = params["textDocument"]["uri"].as_str().unwrap_or_default();
        if method == "exit" {
            break;
        }
        let result = match method {
            "initialize" => Some(
                json!({"capabilities":{"positionEncoding":"utf-16","textDocumentSync":1,"completionProvider":{"triggerCharacters":[".","\""]}}}),
            ),
            "shutdown" => {
                shutdown = true;
                documents.clear();
                Some(Value::Null)
            }
            "textDocument/didOpen" | "textDocument/didChange" if !shutdown => {
                let text = if method.ends_with("didOpen") {
                    params["textDocument"]["text"].as_str()
                } else {
                    params["contentChanges"]
                        .as_array()
                        .and_then(|v| v.last())
                        .filter(|v| v.get("range").is_none())
                        .and_then(|v| v["text"].as_str())
                };
                if let Some(text) = text.filter(|text| text.len() <= DOCUMENT_LIMIT) {
                    if documents.contains_key(uri) || documents.len() < DOCUMENT_COUNT {
                        documents.insert(uri.into(), text.into());
                    }
                } else {
                    documents.remove(uri);
                }
                None
            }
            "textDocument/didClose" => {
                documents.remove(uri);
                None
            }
            "textDocument/completion" if !shutdown => {
                let items = documents
                    .get(uri)
                    .and_then(|text| {
                        byte_position(text, &params["position"])
                            .map(|cursor| complete(snapshot, text, cursor))
                    })
                    .unwrap_or_default();
                Some(json!({"isIncomplete":false,"items":items}))
            }
            _ => None,
        };
        if let Some(id) = message.get("id") {
            let response = if let Some(result) = result {
                json!({"jsonrpc":"2.0","id":id,"result":result})
            } else {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Unsupported method"}})
            };
            write_frame(output, response)?;
        }
    }
    Ok(())
}
pub fn run(path: &Path) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    file.take(CATALOG_LIMIT + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > CATALOG_LIMIT {
        return Err("Catalog exceeds 16 MiB".into());
    }
    let catalog = serde_json::from_slice(&data).map_err(|e| format!("Invalid catalog: {e}"))?;
    serve(
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
        &snapshot(catalog),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_index_shares_columns_and_preserves_ambiguity() {
        let schema = snapshot(serde_json::from_value(json!({"tables":[
            {"schema":"public", "name":"shared", "columns":[{"name":"public_id"}]},
            {"schema":"private", "name":"shared", "columns":[{"name":"private_id"}]},
            {"schema":"public", "name":"unique", "columns":[{"name":"id"}]}
        ]})).unwrap());
        assert!(schema.columns(&["shared".into()]).is_empty());
        assert_eq!(*schema.columns(&["public".into(), "shared".into()]), ["public_id"]);
        assert_eq!(*schema.columns(&["private".into(), "shared".into()]), ["private_id"]);
        let bare = schema.columns(&["unique".into()]);
        let qualified = schema.columns(&["public".into(), "unique".into()]);
        assert!(Arc::ptr_eq(&bare, &qualified));
        assert!(schema.columns(&["missing".into()]).is_empty());
        // Alias renaming must copy on write and leave the shared index intact.
        let query = "SELECT t.ren FROM public.unique t(renamed)";
        assert!(complete(&schema, query, 12).iter().any(|item| item["label"] == "renamed"));
        assert_eq!(*schema.columns(&["unique".into()]), ["id"]);
    }
    fn fixture() -> CatalogSnapshot {
        snapshot(serde_json::from_value(json!({"tables":[{"schema":"public","name":"customers","columns":[{"name":"customer_id","data_type":"int4","not_null":true},{"name":"display_name","data_type":"text"}]},{"schema":"billing","name":"invoices","columns":[{"name":"invoice_id"}]}]})).unwrap())
    }
    fn labels(query: &str) -> Vec<String> {
        let cursor = query.find('|').unwrap();
        let text = query.replace('|', "");
        complete(&fixture(), &text, cursor)
            .into_iter()
            .map(|v| v["label"].as_str().unwrap().to_string())
            .collect()
    }
    #[test]
    fn statement_end_failure_does_not_poison_next_completion() {
        let schema = fixture();
        let text = "SELECT c.dis FROM public.customers c;";
        assert!(complete(&schema, text, text.len()).is_empty());
        assert!(
            complete(&schema, text, 12)
                .iter()
                .any(|item| item["label"] == "display_name")
        );
    }

    #[test]
    fn schema_tables_and_alias_columns() {
        assert!(labels("SELECT * FROM public.cust|").contains(&"customers".into()));
        assert!(labels("SELECT c.dis| FROM public.customers c").contains(&"display_name".into()));
        assert!(!labels("SELECT c.dis| FROM public.customers c").contains(&"invoice_id".into()));
    }
    #[test]
    fn unicode_positions_and_statement_offsets() {
        let text = "-- 😀\nSELECT 1;\nSELECT c.dis FROM public.customers c";
        assert_eq!(byte_position(text, &json!({"line":0,"character":4})), None);
        assert_eq!(
            byte_position(text, &json!({"line":0,"character":5})),
            Some(7)
        );
        let cursor = text.find("c.dis").unwrap() + 5;
        let items = complete(&fixture(), text, cursor);
        let item = items.iter().find(|v| v["label"] == "display_name").unwrap();
        assert_eq!(item["textEdit"]["range"]["start"]["line"], 2);
        assert_eq!(item["textEdit"]["range"]["end"]["character"], 12);
    }
    #[test]
    fn cte_columns() {
        let cte = labels(
            "WITH selected AS (SELECT display_name FROM public.customers) SELECT s.dis| FROM selected s",
        );
        assert!(cte.contains(&"display_name".into()), "{cte:?}");
    }
    #[test]
    fn unqualified_scoped_columns_and_ambiguous_edits() {
        for query in [
            r#"WITH selected AS (SELECT display_name AS "title" FROM public.customers) SELECT "ti|" FROM selected"#,
            "WITH selected AS (SELECT display_name AS title FROM public.customers) SELECT ti| FROM selected",
            "SELECT ti| FROM (SELECT display_name AS title FROM public.customers) sub",
            "WITH selected AS (SELECT display_name AS title FROM public.customers) SELECT * FROM selected WHERE ti| = 'x'",
        ] {
            assert!(labels(query).contains(&"title".into()), "{query}");
        }
        let query = "WITH selected AS (SELECT display_name AS title FROM public.customers) SELECT * FROM selected WHERE EXISTS (SELECT ti| FROM billing.invoices)";
        let cursor = query.find('|').unwrap();
        assert!(
            scope::complete(&fixture(), &query.replace('|', ""), cursor)
                .unwrap()
                .iter()
                .any(|item| item.0 == "title")
        );
        for query in [
            "SELECT dis|(customer_id) FROM public.customers",
            "SELECT customer_id AS dis| FROM public.customers",
            "SELECT customer_id::dis| FROM public.customers",
        ] {
            let cursor = query.find('|').unwrap();
            assert!(
                scope::complete(&fixture(), &query.replace('|', ""), cursor).is_none(),
                "{query}"
            );
        }
        let query = "SELECT dis| FROM public.customers a CROSS JOIN public.customers b";
        let cursor = query.find('|').unwrap();
        let items = complete(&fixture(), &query.replace('|', ""), cursor);
        for source in ["a", "b"] {
            let item = items
                .iter()
                .find(|v| v["label"] == format!("{source}.display_name"))
                .unwrap();
            assert_eq!(
                item["textEdit"]["newText"],
                format!("\"{source}\".\"display_name\"")
            );
        }
        assert!(!items.iter().any(|v| v["label"] == "display_name"));
        assert!(labels("SELECT * FROM public.cust|").contains(&"customers".into()));
        assert!(scope::complete(&fixture(), "SELECT 'dis' FROM public.customers", 11).is_none());
    }

    #[test]
    fn correlated_aliases_respect_shadowing_and_boundaries() {
        for query in [
            "SELECT * FROM public.customers c WHERE EXISTS (SELECT c.dis| FROM billing.invoices i)",
            "SELECT (SELECT c.dis|) FROM public.customers c",
            "SELECT * FROM public.customers c WHERE EXISTS (SELECT 1 WHERE EXISTS (SELECT c.dis|))",
            "WITH selected AS (SELECT display_name AS title FROM public.customers) SELECT * FROM selected c WHERE EXISTS (SELECT c.ti|)",
            "WITH selected AS (SELECT display_name AS title FROM public.customers) SELECT * FROM selected c WHERE EXISTS (WITH selected AS (SELECT invoice_id FROM billing.invoices) SELECT c.ti| FROM selected)",
        ] {
            let items = labels(query);
            let expected = if query.starts_with("WITH") {
                "title"
            } else {
                "display_name"
            };
            assert!(items.contains(&expected.into()), "{query}: {items:?}");
        }
        for query in [
            "SELECT * FROM public.customers c WHERE EXISTS (SELECT c.dis| FROM billing.invoices c)",
            "SELECT * FROM public.customers c CROSS JOIN (SELECT c.dis|) sub",
            "WITH selected AS (SELECT c.dis|) SELECT * FROM public.customers c",
        ] {
            let cursor = query.find('|').unwrap();
            let found = scope::complete(&fixture(), &query.replace('|', ""), cursor);
            assert!(found.is_none_or(|items| items.is_empty()), "{query}");
            assert!(!labels(query).contains(&"display_name".into()), "{query}");
        }
    }

    #[test]
    fn relation_column_aliases_replace_only_declared_positions() {
        for (query, expected) in [
            (
                "SELECT t.ren| FROM public.customers AS t(renamed_id, title)",
                "renamed_id",
            ),
            (
                "SELECT t.dis| FROM public.customers AS t(customer_id)",
                "display_name",
            ),
            (
                "SELECT t.ti| FROM (SELECT customer_id, display_name FROM public.customers) AS t(customer_id, title)",
                "title",
            ),
            (
                "SELECT ti| FROM (SELECT customer_id, display_name FROM public.customers) AS t(customer_id, title)",
                "title",
            ),
            (
                "WITH renamed AS (SELECT * FROM public.customers AS t(customer_id, title)) SELECT r.ti| FROM renamed r",
                "title",
            ),
            (
                "SELECT t.ti| FROM (SELECT 1, 'hello') AS t(customer_id, title)",
                "title",
            ),
            (
                "SELECT t.ti| FROM (VALUES (1, 'hello'), (2, 'world')) AS t(customer_id, title)",
                "title",
            ),
            (
                "SELECT t.col| FROM (VALUES (1, 'hello')) AS t(customer_id)",
                "column2",
            ),
            (
                "WITH v AS (VALUES (1, 'hello')) SELECT v.col| FROM v",
                "column1",
            ),
        ] {
            assert!(
                labels(query).contains(&expected.into()),
                "{query}: {:?}",
                labels(query)
            );
        }
        for query in [
            "SELECT t.dis| FROM public.customers AS t(customer_id, title)",
            "SELECT t.dis| FROM (SELECT customer_id, display_name FROM public.customers) AS t(customer_id, title)",
        ] {
            assert!(!labels(query).contains(&"display_name".into()), "{query}");
        }
    }

    #[test]
    fn values_comments_do_not_create_columns_and_aliases_do_not_extend_arity() {
        let query = "SELECT t.col| FROM (VALUES (/* first */ 1, /* second */ 'a')) t";
        let found = labels(query);
        assert!(
            found.contains(&"column1".into()) && found.contains(&"column2".into()),
            "{found:?}"
        );
        assert!(!found.contains(&"column3".into()), "{found:?}");
        assert!(!labels("SELECT t.ex| FROM (SELECT 1) AS t(id, extra)").contains(&"extra".into()));
    }

    #[test]
    fn recursive_cte_columns_and_union_branches() {
        for query in [
            "WITH RECURSIVE walk(node_id) AS (SELECT 1 UNION ALL SELECT w.no| FROM walk w) SELECT * FROM walk",
            "WITH RECURSIVE walk(node_id) AS (VALUES (1) UNION ALL SELECT w.no| FROM walk w) SELECT * FROM walk",
            "WITH RECURSIVE customers(node_id) AS (SELECT 1 UNION ALL SELECT c.no| FROM customers c) SELECT * FROM customers",
            "WITH RECURSIVE walk(node_id) AS (SELECT 1 UNION ALL SELECT (SELECT w.no|) FROM walk w) SELECT * FROM walk",
            "WITH RECURSIVE walk AS (SELECT id AS node_id FROM public.customers c UNION ALL SELECT w.no| FROM walk w) SELECT * FROM walk",
            "WITH RECURSIVE walk AS (SELECT id AS node_id FROM public.customers c UNION ALL SELECT no| FROM walk) SELECT * FROM walk",
            "WITH RECURSIVE walk(node_id) AS (SELECT 1 UNION ALL SELECT node_id FROM walk) SELECT w.no| FROM walk w",
        ] {
            assert!(
                labels(query).contains(&"node_id".into()),
                "{query}: {:?}",
                labels(query)
            );
        }
        for query in [
            "SELECT c.dis| FROM billing.invoices c UNION ALL SELECT * FROM public.customers c",
            "SELECT * FROM public.customers c UNION ALL SELECT c.dis| FROM billing.invoices c",
            "SELECT * FROM public.customers c UNION ALL SELECT (SELECT c.dis|) FROM billing.invoices c",
            "WITH RECURSIVE customers(node_id) AS (SELECT 1 UNION ALL SELECT c.dis| FROM customers c) SELECT * FROM customers",
            "WITH walk(node_id) AS (SELECT 1 UNION ALL SELECT w.no| FROM walk w) SELECT * FROM walk",
            "WITH RECURSIVE walk(node_id) AS (SELECT w.no| FROM walk w UNION ALL SELECT 1) SELECT * FROM walk",
        ] {
            let found = labels(query);
            assert!(
                !found.contains(&"node_id".into()) && !found.contains(&"display_name".into()),
                "{query}: {found:?}"
            );
        }
    }

    #[test]
    fn comma_lateral_sources_preserve_visibility_and_projection() {
        for query in [
            "SELECT * FROM public.customers c, LATERAL (SELECT c.dis|) sub",
            "SELECT * FROM public.customers c, /* scope */ LATERAL /* query */ (SELECT dis|) sub",
            "SELECT sub.ti| FROM public.customers c, LATERAL (SELECT c.display_name AS title) sub",
            "SELECT * FROM public.customers c, LATERAL (SELECT c.display_name AS title) sub, LATERAL (SELECT sub.ti|) next",
            "SELECT * FROM public.customers c, LATERAL (SELECT (SELECT c.dis|)) sub",
        ] {
            let expected = if query.contains("ti|") {
                "title"
            } else {
                "display_name"
            };
            assert!(
                labels(query).contains(&expected.into()),
                "{query}: {:?}",
                labels(query)
            );
        }
        for query in [
            "SELECT * FROM billing.invoices i, LATERAL (SELECT c.dis|) sub, public.customers c",
            "SELECT * FROM billing.invoices i, LATERAL (SELECT dis|) sub, public.customers c",
            "SELECT * FROM public.customers c, LATERAL (SELECT c.dis| FROM billing.invoices c) sub",
            "SELECT * FROM public.customers c, (SELECT c.dis|) sub",
            "SELECT * FROM LATERAL (SELECT c.dis|) sub, public.customers c",
        ] {
            assert!(
                !labels(query).contains(&"display_name".into()),
                "{query}: {:?}",
                labels(query)
            );
        }
    }

    #[test]
    fn lateral_join_column_alias_lists() {
        for join in ["CROSS JOIN", "JOIN", "LEFT JOIN"] {
            let condition = if join == "CROSS JOIN" { "" } else { " ON true" };
            let query = format!("SELECT sub.ti| FROM public.customers c {join} LATERAL (SELECT 1 AS id, 'hello' AS original) AS sub(id, title){condition}");
            assert!(labels(&query).contains(&"title".into()), "{query}: {:?}", labels(&query));
            assert!(!labels(&query.replace("ti|", "ori|")).contains(&"original".into()));
        }
        for (query, expected) in [
            ("SELECT sub.ori| FROM public.customers c CROSS JOIN LATERAL (SELECT 1 AS id, 'hello' AS original) sub(renamed_id)", "original"),
            ("SELECT sub.\"Dis|\" FROM public.customers c LEFT JOIN LATERAL (SELECT 'hello') sub(\"Display Name\") ON true", "Display Name"),
            ("SELECT sub.ti| FROM public.customers c JOIN LATERAL (VALUES (1, 'hello')) sub(id, title) ON true", "title"),
            ("SELECT * FROM public.customers c CROSS JOIN LATERAL (SELECT c.display_name) sub(title) CROSS JOIN LATERAL (SELECT sub.ti|) next", "title"),
        ] {
            assert!(labels(query).contains(&expected.into()), "{query}: {:?}", labels(query));
        }
        assert!(!labels("SELECT sub.ex| FROM public.customers c CROSS JOIN LATERAL (SELECT 1) sub(id, extra)").contains(&"extra".into()));
    }

    #[test]
    fn lateral_joins_only_see_preceding_sources() {
        for query in [
            "SELECT * FROM public.customers c CROSS JOIN LATERAL (SELECT c.dis|) sub",
            "SELECT * FROM public.customers c LEFT JOIN LATERAL (SELECT dis|) sub ON true",
            "SELECT * FROM public.customers c JOIN LATERAL (SELECT (SELECT c.dis|)) sub ON true",
            "SELECT sub.ti| FROM public.customers c CROSS JOIN LATERAL (SELECT c.display_name AS title) sub",
            "SELECT sub.ti| FROM public.customers c LEFT JOIN LATERAL (SELECT c.display_name AS title) sub ON true",
            "SELECT * FROM public.customers c CROSS JOIN LATERAL (SELECT c.display_name AS title) sub CROSS JOIN LATERAL (SELECT sub.ti|) next",
        ] {
            let expected = if query.contains("ti|") {
                "title"
            } else {
                "display_name"
            };
            assert!(
                labels(query).contains(&expected.into()),
                "{query}: {:?}",
                labels(query)
            );
        }
        for query in [
            "SELECT * FROM billing.invoices i CROSS JOIN LATERAL (SELECT c.dis|) sub CROSS JOIN public.customers c",
            "SELECT * FROM billing.invoices i CROSS JOIN LATERAL (SELECT dis|) sub CROSS JOIN public.customers c",
            "SELECT * FROM public.customers c CROSS JOIN LATERAL (SELECT c.dis| FROM billing.invoices c) sub",
            "SELECT * FROM public.customers c CROSS JOIN (SELECT c.dis|) sub",
        ] {
            assert!(
                !labels(query).contains(&"display_name".into()),
                "{query}: {:?}",
                labels(query)
            );
        }
    }

    #[test]
    fn unqualified_correlated_columns_use_nearest_scope() {
        for query in [
            "SELECT (SELECT dis|) FROM public.customers c",
            "SELECT * FROM public.customers c WHERE EXISTS (SELECT 1 WHERE EXISTS (SELECT dis|))",
            "WITH selected AS (SELECT 42 AS display_name) SELECT (SELECT dis|) FROM selected",
        ] {
            assert!(labels(query).contains(&"display_name".into()), "{query}");
        }
        let query = "SELECT (SELECT dis| FROM (SELECT 1 AS display_name) inner_source) FROM public.customers a CROSS JOIN public.customers b";
        let found = labels(query);
        assert!(found.contains(&"display_name".into()));
        assert!(!found.iter().any(|name| name.ends_with(".display_name")));
        for query in [
            "SELECT * FROM public.customers c CROSS JOIN (SELECT dis|) sub",
            "WITH selected AS (SELECT dis|) SELECT * FROM public.customers c",
        ] {
            let cursor = query.find('|').unwrap();
            assert!(
                scope::complete(&fixture(), &query.replace('|', ""), cursor)
                    .is_some_and(|items| items.is_empty())
            );
            assert!(!labels(query).contains(&"display_name".into()));
        }
        let query = "SELECT (SELECT dis|) FROM public.customers a CROSS JOIN public.customers b";
        let found = labels(query);
        assert!(found.contains(&"a.display_name".into()));
        assert!(found.contains(&"b.display_name".into()));
        assert!(!found.contains(&"display_name".into()));
        let query = "SELECT (SELECT dis| FROM billing.invoices a) FROM public.customers a CROSS JOIN public.customers b";
        let cursor = query.find('|').unwrap();
        let found = scope::complete(&fixture(), &query.replace('|', ""), cursor).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "b.display_name");
        assert_eq!(found[0].2, "\"b\".\"display_name\"");
    }

    #[test]
    fn quoted_table_names() {
        assert!(
            labels("SELECT c.dis| FROM \"public\".\"customers\" c")
                .contains(&"display_name".into())
        );
    }
    #[test]
    fn quoted_column_token() {
        assert!(
            labels("SELECT c.\"dis|\" FROM public.customers c").contains(&"display_name".into())
        );
    }
    #[test]
    fn full_protocol_changes_and_close() {
        let mut input = Vec::new();
        for message in [
            json!({"id":1,"method":"initialize"}),
            json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///q.sql","text":"SELECT c.dis FROM public.customers c"}}}),
            json!({"id":2,"method":"textDocument/completion","params":{"textDocument":{"uri":"file:///q.sql"},"position":{"line":0,"character":12}}}),
            json!({"method":"textDocument/didChange","params":{"textDocument":{"uri":"file:///q.sql"},"contentChanges":[{"text":"SELECT * FROM billing.inv"}]}}),
            json!({"id":3,"method":"textDocument/completion","params":{"textDocument":{"uri":"file:///q.sql"},"position":{"line":0,"character":25}}}),
            json!({"method":"textDocument/didClose","params":{"textDocument":{"uri":"file:///q.sql"}}}),
            json!({"id":4,"method":"textDocument/completion","params":{"textDocument":{"uri":"file:///q.sql"},"position":{"line":0,"character":0}}}),
            json!({"id":5,"method":"shutdown"}),
            json!({"method":"exit"}),
        ] {
            write_frame(&mut input, message).unwrap();
        }
        let mut output = Vec::new();
        serve(&mut std::io::Cursor::new(input), &mut output, &fixture()).unwrap();
        let mut output = std::io::Cursor::new(output);
        assert!(read_frame(&mut output).unwrap().unwrap()["result"]["capabilities"].is_object());
        let first = read_frame(&mut output).unwrap().unwrap();
        assert!(
            first["result"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["label"] == "display_name")
        );
        let second = read_frame(&mut output).unwrap().unwrap();
        assert!(
            second["result"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["label"] == "invoices")
        );
        assert!(
            read_frame(&mut output).unwrap().unwrap()["result"]["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(read_frame(&mut output).unwrap().unwrap()["result"].is_null());
    }
    #[test]
    fn bounded_frames() {
        assert!(
            read_frame(&mut std::io::Cursor::new(
                b"Content-Length: 99999999\r\n\r\n"
            ))
            .is_err()
        );
        assert!(read_frame(&mut std::io::Cursor::new(vec![b'x'; 9000])).is_err());
    }
    #[test]
    fn cte_aliases_stars_chains_and_nested_scopes() {
        for (sql, expected) in [
            (
                "WITH selected AS (SELECT display_name AS title FROM public.customers) SELECT s.ti| FROM selected s",
                "title",
            ),
            (
                "WITH selected(renamed) AS (SELECT customer_id, display_name FROM public.customers) SELECT s.dis| FROM selected s",
                "display_name",
            ),
            (
                "WITH selected(renamed) AS (SELECT customer_id FROM public.customers) SELECT s.| FROM selected s",
                "renamed",
            ),
            (
                "WITH selected AS (SELECT * FROM public.customers), next AS (SELECT * FROM selected) SELECT n.dis| FROM next n",
                "display_name",
            ),
            (
                "WITH selected AS (SELECT c.* FROM public.customers c) SELECT s.dis| FROM selected s",
                "display_name",
            ),
            (
                "WITH selected(first) AS (SELECT 1, display_name FROM public.customers) SELECT s.dis| FROM selected s",
                "display_name",
            ),
            (
                "WITH selected(first) AS (SELECT * FROM public.customers z CROSS JOIN billing.invoices a) SELECT s.inv| FROM selected s",
                "invoice_id",
            ),
            (
                "SELECT sub.ti| FROM (SELECT display_name AS title FROM public.customers) sub",
                "title",
            ),
            (
                "SELECT c.display_name FROM public.customers c WHERE EXISTS (SELECT c.inv| FROM billing.invoices c)",
                "invoice_id",
            ),
        ] {
            let items = labels(sql);
            assert!(items.contains(&expected.into()), "{sql}: {items:?}");
        }
        assert!(labels("WITH selected AS (SELECT display_name AS title FROM public.customers) SELECT s.dis| FROM selected s").is_empty());
        assert!(labels("SELECT c.display_name FROM public.customers c WHERE EXISTS (SELECT c.dis| FROM billing.invoices c)").is_empty());
    }

    #[test]
    fn quoted_case_escape_and_complete_token_edit() {
        let schema = snapshot(serde_json::from_value(json!({"tables":[{"schema":"Case Space", "name":"Customer", "columns":[{"name":"Display Name"}, {"name":"a\"b"}]}]})).unwrap());
        for (query, expected, replacement) in [
            (
                r#"SELECT "c"."Dis|suffix" FROM "Case Space"."Customer" "c""#,
                "Display Name",
                r#""Display Name""#,
            ),
            (
                r#"SELECT c."a""|" FROM "Case Space"."Customer" c"#,
                "a\"b",
                r#""a""b""#,
            ),
        ] {
            let cursor = query.find('|').unwrap();
            let text = query.replace('|', "");
            let items = complete(&schema, &text, cursor);
            let item = items
                .iter()
                .find(|item| item["label"] == expected)
                .unwrap_or_else(|| panic!("{query}: {items:?}"));
            assert_eq!(item["textEdit"]["newText"], replacement);
            let range = &item["textEdit"]["range"];
            let start = byte_position(&text, &range["start"]).unwrap();
            let end = byte_position(&text, &range["end"]).unwrap();
            assert!(text[start..end].starts_with('"'));
            assert!(text[start..end].ends_with('"'));
            let mut applied = text.clone();
            applied.replace_range(start..end, replacement);
            assert!(
                applied.contains(&format!(".{replacement} FROM")),
                "{applied}"
            );
        }
        let query = r#"SELECT c.di FROM "Case Space".Customer c"#;
        assert!(complete(&schema, query, 11).is_empty());
    }
    #[test]
    fn unicode_identifiers_and_scope_bounds() {
        let schema = snapshot(
            serde_json::from_value(
                json!({"tables":[{"schema":"public","name":"顧客","columns":[{"name":"表示名"}]}]}),
            )
            .unwrap(),
        );
        let text = "SELECT 客.表 FROM public.顧客 客";
        let cursor = text.find("表 FROM").unwrap() + "表".len();
        let items = complete(&schema, text, cursor);
        let item = items.iter().find(|item| item["label"] == "表示名").unwrap();
        assert_eq!(item["textEdit"]["newText"], "\"表示名\"");
        assert_eq!(item["textEdit"]["range"]["start"]["character"], 9);
        assert_eq!(item["textEdit"]["range"]["end"]["character"], 10);
        // Unknown aliases and string/comment contents must not gain scoped edits.
        for marked in [
            "SELECT 'c.dis|' FROM public.customers c",
            "SELECT 1 /* c.dis| */ FROM public.customers c",
            "SELECT c.customer_id -- c.dis|\nFROM public.customers c",
        ] {
            let cursor = marked.find('|').unwrap();
            assert!(scope::complete(&fixture(), &marked.replace('|', ""), cursor).is_none());
        }
    }

    #[test]
    #[ignore = "manual offline scoped completion latency benchmark"]
    fn large_catalog_completion_latency() {
        let tables: Vec<_> = (0..5000).map(|i| json!({"schema":"public", "name":format!("table_{i}"), "columns":(0..20).map(|j| json!({"name":format!("column_{j}")})).collect::<Vec<_>>()})).collect();
        let schema = snapshot(serde_json::from_value(json!({"tables":tables})).unwrap());
        let text = "SELECT a.column_1 FROM public.table_4999 a JOIN public.table_2500 b ON a.column_0 = b.column_0 JOIN public.table_42 c ON c.column_0 = a.column_0";
        let cursor = text.find("column_1").unwrap() + "column_1".len();
        let mut samples = Vec::new();
        for i in 0..105 {
            let start = std::time::Instant::now();
            let items = complete(&schema, text, cursor);
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            assert!(items.iter().any(|item| item["label"] == "column_19"));
            if i >= 5 { samples.push(elapsed); }
        }
        samples.sort_by(f64::total_cmp);
        eprintln!("5000 tables/100000 columns, three-source completion: p50={:.3}ms p95={:.3}ms p99={:.3}ms", samples[50], samples[95], samples[99]);
    }

    #[test]
    #[ignore = "manual offline scoped completion latency benchmark"]
    fn scoped_completion_latency() {
        let tables: Vec<_> = (0..100).map(|i| json!({"schema":"public", "name":format!("table_{i}"), "columns":(0..20).map(|j| json!({"name":format!("column_{j}")})).collect::<Vec<_>>()})).collect();
        let schema = snapshot(serde_json::from_value(json!({"tables":tables})).unwrap());
        for (case, text, marker, expected) in [
            (
                "recursive CTE",
                "WITH RECURSIVE walk(\"Display Name\") AS (SELECT column_12 FROM public.table_42 UNION ALL SELECT w.\"Dis\" FROM walk w) SELECT * FROM walk",
                "Dis\" FROM",
                "Display Name",
            ),
            (
                "qualified CTE",
                "WITH selected AS (SELECT * FROM public.table_42), next AS (SELECT column_12 AS \"Display Name\" FROM selected) SELECT n.\"Dis\" FROM next n",
                "Dis\" FROM",
                "Display Name",
            ),
            (
                "bare correlated",
                "WITH selected AS (SELECT column_12 AS \"Display Name\" FROM public.table_42) SELECT (SELECT (SELECT \"Dis\")) FROM selected",
                "Dis\"))",
                "Display Name",
            ),
            (
                "ambiguous correlated",
                "WITH selected AS (SELECT column_12 AS \"Display Name\" FROM public.table_42) SELECT (SELECT \"Dis\") FROM selected a CROSS JOIN selected b",
                "Dis\")",
                "b.Display Name",
            ),
        ] {
            let cursor = text.find(marker).unwrap() + 3;
            let mut samples = Vec::new();
            for i in 0..55 {
                let start = std::time::Instant::now();
                let items = complete(&schema, text, cursor);
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                assert!(items.iter().any(|item| item["label"] == expected));
                if i >= 5 {
                    samples.push(elapsed);
                }
            }
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "{case} completion 100 tables/2000 columns: p50={:.3}ms p95={:.3}ms max={:.3}ms",
                samples[25], samples[47], samples[49]
            );
        }
    }
}
