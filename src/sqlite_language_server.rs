//! SQLite schema adapter for syntaqlite's real stdio language server.
//! Only cached names, type affinities and nullability are sent: never row data,
//! connection paths, credentials, default literals or executable view bodies.
use crate::{db::Object, sql_language_server::ServerSpec};
use std::path::PathBuf;

const SCHEMA_BYTE_LIMIT: usize = 2 * 1024 * 1024;
const OBJECT_LIMIT: usize = 10_000;

/// Resolve only an explicit override, an app-bundled executable, a local Cargo
/// development install, or PATH. The app never downloads tools on the UI thread.
pub fn discover_program() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("RUSTABLE_SQLITE_LSP") {
        return existing_program(PathBuf::from(path))
            .ok_or("RUSTABLE_SQLITE_LSP does not point to an existing executable file.".into());
    }
    let executable = if cfg!(windows) {
        "syntaqlite.exe"
    } else {
        "syntaqlite"
    };
    if let Ok(current) = std::env::current_exe()
        && let Some(parent) = current.parent()
        && let Some(program) = existing_program(parent.join(executable))
    {
        return Ok(program);
    }
    if cfg!(debug_assertions)
        && let Some(program) = existing_program(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target/lsp-tools/bin")
                .join(executable),
        )
    {
        return Ok(program);
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            if let Some(program) = existing_program(directory.join(executable)) {
                return Ok(program);
            }
        }
    }
    Err("SQLite language server is unavailable. Install syntaqlite-cli 0.13.3 with the mise-managed Cargo toolchain or set RUSTABLE_SQLITE_LSP.".into())
}
fn existing_program(path: PathBuf) -> Option<PathBuf> {
    if !path.is_file() {
        return None;
    }
    path.canonicalize().ok()
}

/// Called by the generic language-server worker, never by the rendering thread.
pub fn server_spec(objects: &[Object], program: PathBuf) -> Result<ServerSpec, String> {
    let ddl = schema_ddl(objects)?;
    let directory = tempfile::Builder::new()
        .prefix("rustable-sqlite-lsp-")
        .tempdir()
        .map_err(|error| format!("Create SQLite language-server workspace: {error}"))?;
    Ok(ServerSpec {
        program,
        args: vec![
            "--no-config".into(),
            "--sqlite-version".into(),
            rusqlite::version().into(),
            "lsp".into(),
        ],
        root: directory.path().to_owned(),
        env: Vec::new(),
        directory: Some(directory),
        initial_requests: vec![(
            "syntaqlite/setSessionContext".into(),
            serde_json::json!({"ddl": ddl}),
        )],
    })
}

pub fn schema_ddl(objects: &[Object]) -> Result<String, String> {
    let objects = objects
        .iter()
        .filter(|object| matches!(object.kind.as_str(), "table" | "view"))
        .collect::<Vec<_>>();
    if objects.len() > OBJECT_LIMIT {
        return Err("SQLite language-server catalog exceeds 10,000 objects.".into());
    }
    let mut ddl = String::new();
    for object in objects {
        if object.metadata_error.is_some() || object.columns.is_empty() {
            return Err(format!(
                "Column metadata is unavailable for {}. Refresh the schema before starting SQLite language support.",
                object.display_name()
            ));
        }
        // Views are represented by their known result columns, avoiding exposure
        // of literals inside their definitions. SQLite's affinity rules provide
        // useful completion types without interpreting arbitrary declared types.
        ddl.push_str("CREATE TABLE ");
        quote_into(&mut ddl, &object.name);
        ddl.push_str(" (");
        for (index, column) in object.columns.iter().enumerate() {
            if index != 0 {
                ddl.push_str(", ");
            }
            quote_into(&mut ddl, &column.name);
            ddl.push(' ');
            ddl.push_str(affinity(&column.data_type));
            if column.not_null {
                ddl.push_str(" NOT NULL");
            }
            if ddl.len() > SCHEMA_BYTE_LIMIT {
                return Err("SQLite language-server catalog exceeds 2 MiB.".into());
            }
        }
        ddl.push_str(");\n");
    }
    Ok(ddl)
}
fn quote_into(output: &mut String, identifier: &str) {
    output.push('"');
    for ch in identifier.chars() {
        if ch == '"' {
            output.push('"');
        }
        output.push(ch);
    }
    output.push('"');
}
fn affinity(declared: &str) -> &'static str {
    let upper = declared.to_ascii_uppercase();
    if upper.contains("INT") {
        "INTEGER"
    } else if upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT") {
        "TEXT"
    } else if upper.is_empty() || upper.contains("BLOB") {
        "BLOB"
    } else if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
        "REAL"
    } else {
        "NUMERIC"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Column;
    pub(super) fn object(name: &str, columns: &[(&str, &str)]) -> Object {
        Object {
            schema: None,
            display_name: None,
            metadata_error: None,
            name: name.into(),
            kind: "table".into(),
            ddl: "PRIVATE_VIEW_LITERAL".into(),
            columns: columns
                .iter()
                .map(|(name, data_type)| Column {
                    name: (*name).into(),
                    data_type: (*data_type).into(),
                    not_null: false,
                    default: "SECRET_DEFAULT".into(),
                    pk: false,
                    generated: false,
                })
                .collect(),
        }
    }
    #[test]
    #[ignore = "requires pinned syntaqlite installed through mise Cargo"]
    fn real_sqlite_lsp_completes_schema_and_reports_unknown_columns() {
        use crate::sql_language_server::{Document, Event, Worker};
        let program = discover_program().unwrap();
        let objects = vec![object(
            "customers",
            &[("id", "INTEGER"), ("name", "TEXT"), ("email", "TEXT")],
        )];
        let worker = Worker::start(eframe::egui::Context::default(), move || {
            server_spec(&objects, program)
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "LSP initialize timed out"
            );
            match worker.rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(Event::Ready) => break,
                Ok(Event::Error(error)) => panic!("{error}"),
                _ => {}
            }
        }
        worker.submit(Document {
            id: 17,
            version: 1,
            text: "SELECT na FROM customers;".into(),
            cursor: 9,
        });
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "LSP completion timed out"
            );
            match worker.rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(Event::Completions {
                    id: 17,
                    version: 1,
                    items,
                }) => {
                    assert!(
                        items.iter().any(|item| item["label"] == "name"),
                        "{items:?}"
                    );
                    break;
                }
                Ok(Event::Error(error)) => panic!("{error}"),
                _ => {}
            }
        }
        worker.submit(Document {
            id: 17,
            version: 2,
            text: "SELECT missing_column FROM customers;".into(),
            cursor: 21,
        });
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "LSP diagnostics timed out"
            );
            match worker.rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(Event::Diagnostics {
                    id: 17,
                    version: 2,
                    items,
                }) if items.iter().any(|item| {
                    item["message"]
                        .as_str()
                        .is_some_and(|message| message.contains("missing_column"))
                }) =>
                {
                    break;
                }
                Ok(Event::Error(error)) => panic!("{error}"),
                _ => {}
            }
        }
    }

    #[test]
    #[ignore = "manual release measurement with bundled syntaqlite"]
    fn completion_latency_baseline() {
        use crate::sql_language_server::{Document, Event, Worker};
        use std::time::{Duration, Instant};
        let program = discover_program().unwrap();
        let objects = (0..100)
            .map(|i| {
                object(
                    &format!("customers_{i}"),
                    &[("id", "INTEGER"), ("name", "TEXT"), ("email", "TEXT")],
                )
            })
            .collect::<Vec<_>>();
        let startup = Instant::now();
        let worker = Worker::start(eframe::egui::Context::default(), move || {
            server_spec(&objects, program)
        });
        loop {
            match worker
                .rx
                .recv_timeout(Duration::from_secs(10))
                .expect("startup timeout")
            {
                Event::Ready => break,
                Event::Error(error) => panic!("{error}"),
                _ => {}
            }
        }
        println!(
            "LSP startup and 100-table catalog: {:.3} ms",
            startup.elapsed().as_secs_f64() * 1000.0
        );
        let mut samples = Vec::new();
        for version in 0..55 {
            let started = Instant::now();
            worker.submit(Document {
                id: 1,
                version,
                text: format!("SELECT na FROM customers_{};", version % 100),
                cursor: 9,
            });
            loop {
                assert!(
                    started.elapsed() < Duration::from_secs(5),
                    "completion timeout"
                );
                match worker
                    .rx
                    .recv_timeout(Duration::from_secs(1))
                    .expect("completion stalled")
                {
                    Event::Completions {
                        version: received,
                        items,
                        ..
                    } if received == version => {
                        assert!(items.iter().any(|item| item["label"] == "name"));
                        if version >= 5 {
                            samples.push(started.elapsed().as_secs_f64() * 1000.0);
                        }
                        break;
                    }
                    Event::Error(error) => panic!("{error}"),
                    _ => {}
                }
            }
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "LSP submission-to-completion, 50 samples: p50 {:.3} ms, p95 {:.3} ms, p99 {:.3} ms",
            samples[24], samples[47], samples[49]
        );
    }

    #[test]
    fn catalog_is_quoted_bounded_and_excludes_values() {
        let catalog = vec![object(
            "odd\" table",
            &[
                ("column\"name", "INTEGER"),
                ("email", "VARCHAR(255)"),
                ("ratio", "DOUBLE"),
            ],
        )];
        let ddl = schema_ddl(&catalog).unwrap();
        assert!(ddl.contains("\"odd\"\" table\""));
        assert!(!ddl.contains("SECRET_DEFAULT"));
        assert!(!ddl.contains("PRIVATE_VIEW_LITERAL"));
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(&ddl).unwrap();
        let mut statement = db.prepare("SELECT * FROM \"odd\"\" table\"").unwrap();
        assert_eq!(statement.column_names(), ["column\"name", "email", "ratio"]);
        assert!(statement.query([]).unwrap().next().unwrap().is_none());
        let oversized = object("large", &[("x", "TEXT")]);
        assert!(schema_ddl(&vec![oversized; OBJECT_LIMIT + 1]).is_err());
        let mut unavailable = object("missing", &[]);
        unavailable.metadata_error = Some("no metadata".into());
        assert!(schema_ddl(&[unavailable]).is_err());
    }
    #[test]
    fn sqlite_affinity_order_matches_integer_and_text_precedence() {
        assert_eq!(affinity("FLOATING POINT"), "INTEGER");
        assert_eq!(affinity("CHARINT"), "INTEGER");
        assert_eq!(affinity("VARCHAR(300)"), "TEXT");
        assert_eq!(affinity("BOOLEAN"), "NUMERIC");
        assert_eq!(affinity(""), "BLOB");
    }
}
