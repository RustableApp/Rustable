//! MCP tool definitions and handlers. Every handler is read-only: it uses
//! [`ReadOnlySession`], which has no write path and always runs Safe mode.
use super::catalog::{self, Catalog};
use crate::{
    connection_profiles::{ConnectionKind, Profile, ProfileId},
    db::{Browse, Grid, Object, Value, read_only::ReadOnlySession},
};
use serde_json::{Map, Value as Json, json};
use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicBool},
};

/// Live connections kept for reuse. Older idle sessions are closed first.
const MAX_SESSIONS: usize = 4;
/// Response size budget so one call cannot flood the client's context.
const OUTPUT_BUDGET: usize = 256 * 1024;
const CELL_CHARACTERS: usize = 2_000;
const MAX_QUERY_ROWS: usize = 1_000;
const MAX_PAGE_ROWS: usize = 500;
const MAX_TABLES: usize = 1_000;

struct Entry {
    fingerprint: String,
    session: ReadOnlySession,
    /// Catalog for the selected schema/database; refreshed on demand or miss.
    objects: Option<Vec<Object>>,
    last_used: u64,
}

pub(super) struct Tools {
    catalog: Catalog,
    sessions: HashMap<ProfileId, Entry>,
    cancel: Arc<AtomicBool>,
    clock: u64,
}

pub(super) const NAMES: [&str; 5] =
    ["list_connections", "list_tables", "describe_table", "read_table", "query"];

pub(super) fn definitions() -> Json {
    let read_only = json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false
    });
    let connection = json!({
        "type": "string",
        "description": "Connection name (or id) from list_connections."
    });
    let table = json!({
        "type": "string",
        "description": "Table or view name from list_tables. PostgreSQL also accepts schema.table."
    });
    json!([
        {
            "name": "list_connections",
            "title": "List connections",
            "description": "List the SQL connections saved in Rustable (SQLite, PostgreSQL, MySQL, MariaDB). No passwords are returned. Call this first to find a connection name.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": read_only
        },
        {
            "name": "list_tables",
            "title": "List tables",
            "description": "List tables and views for a connection. PostgreSQL lists one schema and MySQL/MariaDB one database at a time; pass `schema` to switch. The result includes the available schema names.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "connection": connection,
                    "schema": {"type": "string", "description": "PostgreSQL schema or MySQL/MariaDB database to list. Defaults to the current one."},
                    "name_filter": {"type": "string", "description": "Case-insensitive substring to match table names."},
                    "refresh": {"type": "boolean", "description": "Reload the catalog from the server instead of the cached list.", "default": false}
                },
                "required": ["connection"],
                "additionalProperties": false
            },
            "annotations": read_only
        },
        {
            "name": "describe_table",
            "title": "Describe table",
            "description": "Show a table's or view's columns: type, nullability, default, primary key and generated columns. Optionally include its CREATE definition.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "connection": connection,
                    "table": table,
                    "include_definition": {"type": "boolean", "description": "Also return the CREATE statement (one extra round trip on server databases).", "default": false}
                },
                "required": ["connection", "table"],
                "additionalProperties": false
            },
            "annotations": read_only
        },
        {
            "name": "read_table",
            "title": "Read table rows",
            "description": "Read a bounded page of rows from a table or view, optionally sorted. Prefer this over `query` for sampling or paging a table: it fetches only the requested page.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "connection": connection,
                    "table": table,
                    "limit": {"type": "integer", "minimum": 1, "maximum": MAX_PAGE_ROWS, "default": 50},
                    "offset": {"type": "integer", "minimum": 0, "default": 0},
                    "order_by": {"type": "string", "description": "Column to sort by."},
                    "descending": {"type": "boolean", "default": false}
                },
                "required": ["connection", "table"],
                "additionalProperties": false
            },
            "annotations": read_only
        },
        {
            "name": "query",
            "title": "Run read-only SQL",
            "description": "Run read-only SQL (SELECT, WITH, VALUES, SHOW, EXPLAIN; several statements allowed). The database server enforces read-only mode, so writes, DDL and transaction changes are rejected. The server computes the full result, so use LIMIT and WHERE on large tables. Each result set is cut to `max_rows`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "connection": connection,
                    "sql": {"type": "string", "description": "SQL in the connection's dialect."},
                    "max_rows": {"type": "integer", "minimum": 1, "maximum": MAX_QUERY_ROWS, "default": 100}
                },
                "required": ["connection", "sql"],
                "additionalProperties": false
            },
            "annotations": read_only
        }
    ])
}

fn text<'a>(args: &'a Json, key: &str) -> Result<Option<&'a str>, String> {
    match args.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(value)) => Ok(Some(value)),
        Some(_) => Err(format!("`{key}` must be a string.")),
    }
}
fn required<'a>(args: &'a Json, key: &str) -> Result<&'a str, String> {
    text(args, key)?.ok_or_else(|| format!("`{key}` is required."))
}
fn flag(args: &Json, key: &str) -> Result<bool, String> {
    match args.get(key) {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("`{key}` must be true or false.")),
    }
}
fn count(args: &Json, key: &str, default: usize, min: usize, max: usize) -> Result<usize, String> {
    match args.get(key) {
        None | Some(Json::Null) => Ok(default),
        Some(value) => value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| (min..=max).contains(value))
            .ok_or_else(|| format!("`{key}` must be an integer from {min} to {max}.")),
    }
}

fn truncate(text: &str) -> Json {
    match text.char_indices().nth(CELL_CHARACTERS) {
        None => Json::String(text.to_owned()),
        Some((end, _)) => {
            let rest = text[end..].chars().count();
            Json::String(format!("{}… [{rest} more characters]", &text[..end]))
        }
    }
}

pub(super) fn cell(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Integer(value) => json!(value),
        Value::Real(value) if value.is_finite() => json!(value),
        Value::Real(value) => Json::String(value.to_string()),
        Value::Text(value) => truncate(value),
        Value::Blob(bytes) => match crate::db::readable_binary(bytes) {
            Some(text) => truncate(text),
            None => {
                let hex: String = bytes.iter().take(32).map(|b| format!("{b:02x}")).collect();
                let more = if bytes.len() > 32 { "…" } else { "" };
                Json::String(format!("<{} bytes: 0x{hex}{more}>", bytes.len()))
            }
        },
    }
}

/// Converts up to `max_rows` rows while staying inside the shared byte budget.
/// Reads spilled results from their local store without loading them all.
fn render(grid: &Grid, max_rows: usize, budget: &mut usize, cancel: &AtomicBool) -> Result<Json, String> {
    let mut rows = Vec::new();
    let mut cut = false;
    grid.visit_rows(cancel, |_, row| {
        if rows.len() >= max_rows {
            cut = true;
            return Ok(false);
        }
        let row = Json::Array(row.iter().map(cell).collect());
        let size = row.to_string().len() + 1;
        if size > *budget {
            cut = true;
            return Ok(false);
        }
        *budget -= size;
        rows.push(row);
        Ok(true)
    })?;
    let mut result = Map::new();
    result.insert("columns".into(), json!(grid.columns));
    result.insert("returned_rows".into(), json!(rows.len()));
    result.insert("rows".into(), Json::Array(rows));
    // Cut by max_rows or the response budget; drivers' own bounds are added by callers.
    result.insert("truncated".into(), json!(cut));
    Ok(Json::Object(result))
}

impl Tools {
    pub(super) fn new(catalog: Catalog, cancel: Arc<AtomicBool>) -> Self {
        Self { catalog, sessions: HashMap::new(), cancel, clock: 0 }
    }

    pub(super) fn call(&mut self, name: &str, args: &Json) -> Result<Json, String> {
        if !args.is_object() && !args.is_null() {
            return Err("Tool arguments must be an object.".into());
        }
        match name {
            "list_connections" => self.list_connections(),
            "list_tables" => self.list_tables(args),
            "describe_table" => self.describe_table(args),
            "read_table" => self.read_table(args),
            "query" => self.query(args),
            _ => Err(format!("Unknown tool {name:?}.")),
        }
    }

    fn list_connections(&mut self) -> Result<Json, String> {
        let profiles = self.catalog.profiles()?;
        let connections: Vec<Json> = profiles
            .iter()
            .map(|profile| {
                json!({
                    "name": profile.name,
                    "id": profile.id.0,
                    "engine": catalog::engine(&profile.connection),
                    "environment": profile.environment,
                    "group": self.catalog.group_name(profile),
                    "target": catalog::target(&profile.connection),
                })
            })
            .collect();
        let mut result = json!({ "connections": connections });
        if profiles.is_empty() {
            result["note"] = json!(format!(
                "No SQL connections are saved{}. Add one in the Rustable app first.",
                self.catalog
                    .location()
                    .map(|path| format!(" in {}", path.display()))
                    .unwrap_or_default()
            ));
        }
        Ok(result)
    }

    /// Reuses a live session unless the saved profile changed since it opened.
    fn entry(&mut self, reference: &str) -> Result<&mut Entry, String> {
        let profile = self.catalog.resolve(reference)?;
        let fingerprint = serde_json::to_string(&profile.connection).map_err(|e| e.to_string())?;
        self.clock += 1;
        let stale = self.sessions.get(&profile.id).is_some_and(|entry| entry.fingerprint != fingerprint);
        if stale {
            self.sessions.remove(&profile.id);
        }
        if !self.sessions.contains_key(&profile.id) {
            while self.sessions.len() >= MAX_SESSIONS {
                let oldest = self.sessions.iter().min_by_key(|(_, entry)| entry.last_used).map(|(id, _)| *id);
                let Some(oldest) = oldest else { break };
                self.sessions.remove(&oldest);
            }
            let session = open(&profile, self.cancel.clone())?;
            self.sessions.insert(profile.id, Entry { fingerprint, session, objects: None, last_used: 0 });
        }
        let entry = self.sessions.get_mut(&profile.id).expect("session inserted above");
        entry.last_used = self.clock;
        Ok(entry)
    }

    fn list_tables(&mut self, args: &Json) -> Result<Json, String> {
        let connection = required(args, "connection")?;
        let schema = text(args, "schema")?.filter(|schema| !schema.is_empty());
        let filter = text(args, "name_filter")?.map(str::to_lowercase);
        let refresh = flag(args, "refresh")?;
        let entry = self.entry(connection)?;
        let switch = schema.is_some_and(|schema| {
            entry.session.schema_catalog().selected.as_deref() != Some(schema)
        });
        if let Some(schema) = schema.filter(|_| switch || refresh) {
            entry.objects = Some(entry.session.select_schema(schema)?);
        } else if refresh || entry.objects.is_none() {
            entry.objects = Some(entry.session.objects()?);
        }
        let objects = entry.objects.as_deref().unwrap_or_default();
        let matching: Vec<&Object> = objects
            .iter()
            .filter(|object| {
                filter.as_ref().is_none_or(|filter| object.display_name().to_lowercase().contains(filter))
            })
            .collect();
        let tables: Vec<Json> = matching
            .iter()
            .take(MAX_TABLES)
            .map(|object| {
                let mut item = json!({ "name": object.name, "kind": object.kind });
                if object.display_name() != object.name {
                    item["display_name"] = json!(object.display_name());
                }
                item
            })
            .collect();
        let catalog = entry.session.schema_catalog();
        let mut result = json!({
            "engine": entry.session.engine(),
            "tables": tables,
            "total": matching.len(),
            "truncated": matching.len() > MAX_TABLES,
        });
        if !catalog.names.is_empty() {
            result["schemas"] = json!(catalog.names);
            result["selected_schema"] = json!(catalog.selected);
        }
        Ok(result)
    }

    fn describe_table(&mut self, args: &Json) -> Result<Json, String> {
        let connection = required(args, "connection")?;
        let table = required(args, "table")?;
        let include_definition = flag(args, "include_definition")?;
        let entry = self.entry(connection)?;
        let object = find(entry, table)?;
        let columns: Vec<Json> = object
            .columns
            .iter()
            .map(|column| {
                let mut item = json!({
                    "name": column.name,
                    "type": column.data_type,
                    "nullable": !column.not_null,
                });
                if !column.default.is_empty() {
                    item["default"] = json!(column.default);
                }
                if column.pk {
                    item["primary_key"] = json!(true);
                }
                if column.generated {
                    item["generated"] = json!(true);
                }
                item
            })
            .collect();
        let mut result = json!({ "name": object.name, "kind": object.kind, "columns": columns });
        if let Some(schema) = &object.schema {
            result["schema"] = json!(schema);
        }
        if let Some(error) = &object.metadata_error {
            result["metadata_error"] = json!(error);
        }
        if include_definition {
            result["definition"] = if object.ddl.trim().is_empty() {
                match entry.session.definition(&object.name) {
                    Ok(sql) => json!(sql),
                    Err(error) => json!(format!("Unavailable: {error}")),
                }
            } else {
                json!(object.ddl)
            };
        }
        Ok(result)
    }

    fn read_table(&mut self, args: &Json) -> Result<Json, String> {
        let connection = required(args, "connection")?;
        let table = required(args, "table")?;
        let limit = count(args, "limit", 50, 1, MAX_PAGE_ROWS)?;
        let offset = count(args, "offset", 0, 0, usize::MAX)?;
        let order_by = text(args, "order_by")?.filter(|column| !column.is_empty());
        let descending = flag(args, "descending")?;
        let cancel = self.cancel.clone();
        let entry = self.entry(connection)?;
        let object = find(entry, table)?;
        if let Some(column) = order_by
            && !object.columns.is_empty()
            && !object.columns.iter().any(|c| c.name == column)
        {
            return Err(format!("{} has no column {column:?}. Call describe_table for its columns.", object.name));
        }
        let page = entry.session.browse(&Browse {
            table: object.name.clone(),
            limit,
            offset,
            sort: order_by.map(|column| (column.to_owned(), descending)),
            ..Browse::default()
        })?;
        let mut budget = OUTPUT_BUDGET;
        let mut result = render(&page.grid, limit, &mut budget, &cancel)?;
        result["table"] = json!(object.name);
        result["offset"] = json!(offset);
        result["has_more"] = json!(page.grid.truncated);
        if let Some(total) = page.grid.total_rows {
            result["total_rows"] = json!(total.rows);
            result["total_is_estimate"] = json!(total.estimated);
        }
        Ok(result)
    }

    fn query(&mut self, args: &Json) -> Result<Json, String> {
        let connection = required(args, "connection")?;
        let sql = required(args, "sql")?;
        if sql.trim().is_empty() {
            return Err("`sql` is empty.".into());
        }
        let max_rows = count(args, "max_rows", 100, 1, MAX_QUERY_ROWS)?;
        let cancel = self.cancel.clone();
        let entry = self.entry(connection)?;
        let report = entry.session.query(sql);
        let mut budget = OUTPUT_BUDGET;
        let mut results = Vec::new();
        for grid in &report.grids {
            if grid.columns.is_empty() {
                continue;
            }
            let mut result = render(grid, max_rows, &mut budget, &cancel)?;
            result["row_count"] = json!(grid.row_count());
            if grid.truncated {
                result["truncated"] = json!(true);
            }
            results.push(result);
        }
        let mut output = json!({
            "results": results,
            "elapsed_ms": report.elapsed.as_millis() as u64,
        });
        if let Some(error) = report.error {
            if results.is_empty() {
                return Err(error);
            }
            output["error"] = json!(error);
        }
        Ok(output)
    }
}

fn open(profile: &Profile, cancel: Arc<AtomicBool>) -> Result<ReadOnlySession, String> {
    let password = || -> Result<String, String> {
        if !profile.remember_password {
            return Ok(String::new());
        }
        crate::credential_store::read_blocking(profile.id, crate::credential_store::Purpose::Database)
            .map(Option::unwrap_or_default)
            .map_err(|error| format!("Could not read the saved password from the system credential store: {error}"))
    };
    let result = match &profile.connection {
        ConnectionKind::SQLite { path } => ReadOnlySession::sqlite(path, cancel),
        ConnectionKind::Postgres(config) => ReadOnlySession::postgres(&config.with_password(password()?), cancel),
        ConnectionKind::Mysql(config) | ConnectionKind::MariaDb(config) => {
            ReadOnlySession::mysql(&config.with_password(password()?), cancel)
        }
        ConnectionKind::Redis(_) => Err("Redis connections are not supported by the MCP server.".into()),
    };
    result.map_err(|error| {
        let server = !matches!(profile.connection, ConnectionKind::SQLite { .. });
        if server && !profile.remember_password {
            format!("{error}\nNo password is saved for {:?}. Connect once in Rustable with the password remembered so the MCP server can use it.", profile.name)
        } else {
            error
        }
    })
}

/// Finds a catalog object, reloading the catalog once on a miss.
fn find(entry: &mut Entry, table: &str) -> Result<Object, String> {
    fn search<'a>(objects: &'a [Object], table: &str) -> Option<&'a Object> {
        let qualified = |object: &Object| {
            object.schema.as_ref().map(|schema| format!("{schema}.{}", object.display_name()))
        };
        objects
            .iter()
            .find(|o| o.name == table || o.display_name() == table || qualified(o).as_deref() == Some(table))
            .or_else(|| {
                objects.iter().find(|o| {
                    o.name.eq_ignore_ascii_case(table)
                        || o.display_name().eq_ignore_ascii_case(table)
                        || qualified(o).is_some_and(|name| name.eq_ignore_ascii_case(table))
                })
            })
    }
    if let Some(object) = entry.objects.as_deref().and_then(|objects| search(objects, table)) {
        return Ok(object.clone());
    }
    entry.objects = Some(entry.session.objects()?);
    entry
        .objects
        .as_deref()
        .and_then(|objects| search(objects, table))
        .cloned()
        .ok_or_else(|| {
            let schema = entry.session.schema_catalog().selected;
            format!(
                "No table or view named {table:?}{}. Call list_tables to see available names{}.",
                schema.map(|schema| format!(" in {schema}")).unwrap_or_default(),
                if entry.session.schema_catalog().names.is_empty() { "" } else { ", or pass `schema` to list_tables to switch" }
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection_profiles::{ConnectionProfiles, Profile};

    fn workspace(directory: &std::path::Path, database: &std::path::Path) -> std::path::PathBuf {
        #[derive(serde::Serialize)]
        struct Workspace<'a> {
            saved_connections: &'a ConnectionProfiles,
        }
        let mut profiles = ConnectionProfiles::default();
        profiles
            .add_profile(Profile::draft("Fixture".into(), ConnectionKind::SQLite { path: database.into() }))
            .unwrap();
        let mut entries = HashMap::new();
        entries.insert(
            "workspace".to_owned(),
            ron::to_string(&Workspace { saved_connections: &profiles }).unwrap(),
        );
        let path = directory.join("app.ron");
        std::fs::write(&path, ron::to_string(&entries).unwrap()).unwrap();
        path
    }

    fn tools() -> (tempfile::TempDir, std::path::PathBuf, Tools) {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("fixture.sqlite");
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE people(id INTEGER PRIMARY KEY, name TEXT NOT NULL DEFAULT 'anon', photo BLOB);
                 WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i < 6000)
                 INSERT INTO people(name, photo) SELECT 'person ' || i, x'00ff' FROM n;
                 CREATE VIEW names AS SELECT name FROM people;",
            )
            .unwrap();
        let storage = workspace(directory.path(), &database);
        let tools = Tools::new(Catalog::new(Some(storage), Vec::new()), Arc::new(AtomicBool::new(false)));
        (directory, database, tools)
    }

    #[test]
    fn lists_describes_reads_and_queries_sqlite() {
        let (_directory, _database, mut tools) = tools();
        let connections = tools.call("list_connections", &json!({})).unwrap();
        assert_eq!(connections["connections"][0]["name"], "Fixture");
        assert_eq!(connections["connections"][0]["engine"], "SQLite");

        let tables = tools.call("list_tables", &json!({"connection": "fixture"})).unwrap();
        let names: Vec<_> = tables["tables"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"people") && names.contains(&"names"), "{tables}");

        let described = tools
            .call("describe_table", &json!({"connection": "Fixture", "table": "people", "include_definition": true}))
            .unwrap();
        assert_eq!(described["columns"][0]["name"], "id");
        assert_eq!(described["columns"][0]["primary_key"], true);
        assert_eq!(described["columns"][1]["nullable"], false);
        assert!(described["definition"].as_str().unwrap().contains("CREATE TABLE"));

        let page = tools
            .call("read_table", &json!({"connection": "Fixture", "table": "people", "limit": 3, "order_by": "id", "descending": true}))
            .unwrap();
        assert_eq!(page["rows"].as_array().unwrap().len(), 3);
        assert_eq!(page["rows"][0][0], 6000);
        assert_eq!(page["rows"][0][2], "<2 bytes: 0x00ff>");
        assert_eq!(page["has_more"], true);

        // Large results spill to disk in the driver; only max_rows are returned.
        let result = tools.call("query", &json!({"connection": "Fixture", "sql": "SELECT id, name FROM people ORDER BY id", "max_rows": 5})).unwrap();
        let first = &result["results"][0];
        assert_eq!(first["row_count"], 6000);
        assert_eq!(first["returned_rows"], 5);
        assert_eq!(first["truncated"], true);
        assert_eq!(first["rows"][4], json!([5, "person 5"]));
    }

    #[test]
    fn writes_are_rejected_and_data_is_unchanged() {
        let (_directory, database, mut tools) = tools();
        for sql in [
            "DELETE FROM people",
            "UPDATE people SET name = 'x'",
            "DROP TABLE people",
            "PRAGMA query_only = OFF; DELETE FROM people",
            "SELECT 1; DELETE FROM people",
        ] {
            let outcome = tools.call("query", &json!({"connection": "Fixture", "sql": sql}));
            let rejected = outcome.as_ref().is_err() || outcome.as_ref().unwrap().get("error").is_some();
            assert!(rejected, "accepted {sql}: {outcome:?}");
        }
        let check = rusqlite::Connection::open(&database).unwrap();
        let count: i64 = check.query_row("SELECT count(*) FROM people WHERE name LIKE 'person %'", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 6000);
    }

    #[test]
    fn output_is_bounded_and_arguments_are_validated() {
        let (_directory, _database, mut tools) = tools();
        let result = tools
            .call("query", &json!({"connection": "Fixture", "sql": "SELECT printf('%.5000c', 'x') AS wide FROM people", "max_rows": 1000}))
            .unwrap();
        let first = &result["results"][0];
        assert!(first["returned_rows"].as_u64().unwrap() < 1000);
        assert_eq!(first["truncated"], true);
        assert!(result.to_string().len() <= OUTPUT_BUDGET + 4096);
        assert!(first["rows"][0][0].as_str().unwrap().contains("more characters"));

        assert!(tools.call("query", &json!({"connection": "Fixture", "sql": "SELECT 1", "max_rows": 0})).is_err());
        assert!(tools.call("read_table", &json!({"connection": "Fixture", "table": "missing"})).unwrap_err().contains("list_tables"));
        assert!(tools.call("read_table", &json!({"connection": "Fixture", "table": "people", "order_by": "nope"})).is_err());
        assert!(tools.call("query", &json!({"sql": "SELECT 1"})).unwrap_err().contains("connection"));
    }
}
