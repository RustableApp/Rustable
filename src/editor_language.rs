//! Per-session asynchronous language service; cached metadata only, never live SQL.
use crate::{
    db::Object,
    sql_language_server::{Document, Event, Worker},
};
use eframe::egui;
use serde_json::Value;

#[derive(Default)]
pub struct LanguageSession {
    worker: Option<Worker>,
    completion_worker: Option<Worker>,
    started: bool,
    version: u64,
    document: Option<Document>,
    pub status: String,
    pub diagnostics: Vec<Diagnostic>,
}
pub struct Diagnostic {
    pub range: std::ops::Range<usize>,
    pub message: String,
    pub error: bool,
}
impl LanguageSession {
    pub fn invalidate(&mut self) {
        *self = Self::default();
    }
    pub fn request_again(&mut self) {
        self.document = None;
    }
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        engine: &str,
        objects: &[Object],
        id: u64,
        sql: &str,
        cursor: usize,
    ) -> Option<Vec<Value>> {
        if !cfg!(target_arch = "wasm32") && !self.started && matches!(engine, "SQLite" | "PostgreSQL") {
            self.started = true;
            self.status = "Starting SQL language service…".into();
            let sqlite = engine == "SQLite";
            if !sqlite {
                let catalog = objects.to_vec();
                self.completion_worker = Some(Worker::start(ctx.clone(), move || {
                    crate::sql_language_server::postgres_completion_spec(
                        crate::sql_language_server::discover_postgres_program()
                            .ok_or("PostgreSQL completion helper is unavailable")?,
                        &catalog,
                    )
                }));
            }
            let objects = objects.to_vec();
            self.worker = Some(Worker::start(ctx.clone(), move || {
                if sqlite {
                    crate::sqlite_language_server::server_spec(
                        &objects,
                        crate::sqlite_language_server::discover_program()?,
                    )
                } else {
                    crate::sql_language_server::postgres_spec(crate::sql_language_server::discover_postgres_program().ok_or("PostgreSQL language server is unavailable; catalog suggestions remain available.")?)
                }
            }));
        }
        if self
            .document
            .as_ref()
            .is_none_or(|doc| doc.id != id || doc.text != sql || doc.cursor != cursor)
        {
            self.version = self.version.wrapping_add(1);
            self.diagnostics.clear();
            let doc = Document {
                id,
                version: self.version,
                text: sql.into(),
                cursor,
            };
            if let Some(worker) = &self.worker {
                worker.submit(doc.clone());
            }
            if let Some(worker) = &self.completion_worker {
                worker.submit(doc.clone());
            }
            self.document = Some(doc);
        }
        let mut completions = None;
        if let Some(worker) = &self.worker {
            for event in worker.rx.try_iter().take(64) {
                match event {
                    Event::Ready if engine != "PostgreSQL" => {
                        self.status = "SQL language service ready".into()
                    }
                    Event::Error(error) => self.status = format!("Catalog suggestions · {error}"),
                    Event::Completions {
                        id: response_id,
                        version,
                        items,
                    } if engine != "PostgreSQL" && self.matches(response_id, version) => {
                        completions = Some(items)
                    }
                    Event::Diagnostics {
                        id: response_id,
                        version,
                        items,
                    } if self.matches(response_id, version) => {
                        self.diagnostics = items
                            .iter()
                            .take(64)
                            .filter_map(|item| {
                                Some(Diagnostic {
                                    range: range(sql, item.get("range")?)?,
                                    message: item
                                        .get("message")?
                                        .as_str()?
                                        .chars()
                                        .take(2000)
                                        .collect(),
                                    error: item
                                        .get("severity")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(1)
                                        == 1,
                                })
                            })
                            .collect();
                    }
                    _ => {}
                }
            }
        }
        if let Some(worker) = &self.completion_worker {
            for event in worker.rx.try_iter().take(64) {
                match event {
                    Event::Ready => self.status = "PostgreSQL schema completion ready".into(),
                    Event::Error(error) => self.status = format!("Catalog suggestions · {error}"),
                    Event::Completions {
                        id: response_id,
                        version,
                        items,
                    } if self.matches(response_id, version) => completions = Some(items),
                    _ => {}
                }
            }
        }
        completions
    }
    fn matches(&self, id: u64, version: u64) -> bool {
        self.document
            .as_ref()
            .is_some_and(|doc| doc.id == id && doc.version == version)
    }
    pub fn paint(&self, ui: &egui::Ui, output: &egui::text_edit::TextEditOutput, sql: &str) {
        if self.document.as_ref().is_none_or(|doc| doc.text != sql) {
            return;
        }
        for diagnostic in &self.diagnostics {
            let start = output
                .galley
                .pos_from_cursor(egui::text::CCursor::new(diagnostic.range.start))
                .translate(output.galley_pos.to_vec2());
            let end = output
                .galley
                .pos_from_cursor(egui::text::CCursor::new(diagnostic.range.end))
                .translate(output.galley_pos.to_vec2());
            if !ui.clip_rect().intersects(start) {
                continue;
            }
            let right = if (start.top() - end.top()).abs() < 1.0 {
                end.left().max(start.left() + 6.0)
            } else {
                output.response.rect.right()
            };
            let color = if diagnostic.error {
                ui.visuals().error_fg_color
            } else {
                ui.visuals().warn_fg_color
            };
            ui.painter().line_segment(
                [start.left_bottom(), egui::pos2(right, start.bottom())],
                egui::Stroke::new(1.0_f32, color),
            );
            let rect = egui::Rect::from_min_max(start.min, egui::pos2(right, start.bottom()));
            ui.interact(
                rect,
                ui.id().with(("sql-diagnostic", diagnostic.range.start)),
                egui::Sense::hover(),
            )
            .on_hover_text(&diagnostic.message);
        }
    }
}

/// LSP positions count UTF-16 code units, egui counts Unicode scalar values.
pub fn position(text: &str, value: &Value) -> Option<usize> {
    let line = value.get("line")?.as_u64()? as usize;
    let column = value.get("character")?.as_u64()? as usize;
    let (mut row, mut units, mut index) = (0, 0, 0);
    for ch in text.chars() {
        if row == line && units == column {
            return Some(index);
        }
        if ch == '\n' {
            if row == line {
                return None;
            }
            row += 1;
            units = 0;
        } else {
            units += ch.len_utf16();
        }
        index += 1;
    }
    (row == line && units == column).then_some(index)
}
pub fn range(text: &str, value: &Value) -> Option<std::ops::Range<usize>> {
    let start = position(text, value.get("start")?)?;
    let end = position(text, value.get("end")?)?;
    (start <= end).then_some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf16_positions_reject_surrogate_midpoints_and_out_of_bounds() {
        let text = "😀a\nSELECT";
        assert_eq!(
            position(text, &serde_json::json!({"line":0,"character":2})),
            Some(1)
        );
        assert_eq!(
            position(text, &serde_json::json!({"line":1,"character":3})),
            Some(6)
        );
        assert_eq!(
            position(text, &serde_json::json!({"line":0,"character":1})),
            None
        );
        assert_eq!(
            position(text, &serde_json::json!({"line":2,"character":0})),
            None
        );
    }
    #[test]
    #[ignore = "requires built private PostgreSQL language helper"]
    fn postgres_session_starts_empty_then_completes_loaded_alias() {
        let ctx = egui::Context::default();
        let objects = vec![Object {
            name: "public.completion_customers".into(),
            schema: Some("public".into()),
            display_name: Some("completion_customers".into()),
            kind: "table".into(),
            ddl: String::new(),
            metadata_error: None,
            columns: vec![crate::db::Column {
                name: "customer_name".into(),
                data_type: "text".into(),
                not_null: true,
                default: String::new(),
                pk: false,
                generated: false,
            }],
        }];
        let mut session = LanguageSession::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while session.status != "PostgreSQL schema completion ready" {
            session.update(&ctx, "PostgreSQL", &objects, 1, "", 0);
            assert!(std::time::Instant::now() < deadline, "{}", session.status);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let sql = "SELECT c.cust FROM public.completion_customers c;";
        // A full-query paste puts the cursor after its semicolon first.
        loop {
            if session
                .update(&ctx, "PostgreSQL", &objects, 1, sql, sql.len())
                .is_some()
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "{}", session.status);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        loop {
            if let Some(items) = session.update(&ctx, "PostgreSQL", &objects, 1, sql, 13)
                && items.iter().any(|item| item["label"] == "customer_name")
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "{}", session.status);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        for (id, source, marker, label) in [
            (
                2,
                "SELECT c.cust FROM \"public\".\"completion_customers\" c;",
                "cust FROM",
                "customer_name",
            ),
            (
                3,
                "WITH selected AS (SELECT customer_name AS \"Display Name\" FROM public.completion_customers) SELECT s.\"Dis\" FROM selected s;",
                "Dis\" FROM",
                "Display Name",
            ),
            (
                4,
                "WITH selected AS (SELECT customer_name AS \"Display Name\" FROM public.completion_customers) SELECT \"Dis\" FROM selected;",
                "Dis\" FROM",
                "Display Name",
            ),
            (
                5,
                "WITH selected AS (SELECT customer_name AS \"Display Name\" FROM public.completion_customers) SELECT * FROM selected s WHERE EXISTS (SELECT s.\"Dis\");",
                "Dis\")",
                "Display Name",
            ),
            (
                6,
                "WITH selected AS (SELECT customer_name AS \"Display Name\" FROM public.completion_customers) SELECT (SELECT \"Dis\") FROM selected s;",
                "Dis\")",
                "Display Name",
            ),
            (
                7,
                "WITH selected AS (SELECT customer_name AS \"Display Name\" FROM public.completion_customers) SELECT (SELECT \"Dis\") FROM selected a CROSS JOIN selected b;",
                "Dis\")",
                "b.Display Name",
            ),
            (
                8,
                "WITH selected AS (SELECT customer_name AS \"Display Name\" FROM public.completion_customers) SELECT * FROM selected s CROSS JOIN LATERAL (SELECT s.\"Dis\") sub;",
                "Dis\")",
                "Display Name",
            ),
            (
                9,
                "WITH selected AS (SELECT customer_name AS \"Display Name\" FROM public.completion_customers) SELECT * FROM selected s, /* source */ LATERAL /* query */ (SELECT s.\"Dis\") sub;",
                "Dis\")",
                "Display Name",
            ),
            (
                10,
                "WITH RECURSIVE walk(\"Display Name\") AS (SELECT customer_name FROM public.completion_customers UNION ALL SELECT w.\"Dis\" FROM walk w) SELECT * FROM walk;",
                "Dis\" FROM",
                "Display Name",
            ),
            (
                11,
                "SELECT t.\"Dis\" FROM (VALUES (1, 'hello')) AS t(id, \"Display Name\");",
                "Dis\" FROM",
                "Display Name",
            ),
            (
                12,
                "SELECT sub.\"Dis\" FROM public.completion_customers c LEFT JOIN LATERAL (SELECT c.customer_name) AS sub(\"Display Name\") ON true;",
                "Dis\" FROM",
                "Display Name",
            ),
        ] {
            let cursor = source.find(marker).unwrap() + if id == 2 { 4 } else { 3 };
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            let items = loop {
                if let Some(items) =
                    session.update(&ctx, "PostgreSQL", &objects, id, source, cursor)
                    && items.iter().any(|item| item["label"] == label)
                {
                    break items;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "{}: {source}",
                    session.status
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            let mut completion = crate::completion::Completion::default();
            completion.refresh(
                source,
                cursor,
                &objects,
                &[],
                false,
                crate::sql::Dialect::Postgres,
            );
            completion.receive_server(source, cursor, &items);
            let mut applied = source.to_owned();
            completion.apply(&mut applied, label);
            let expected = if id == 2 {
                source.replacen("c.cust FROM", "c.\"customer_name\" FROM", 1)
            } else if id == 7 {
                source.replace("\"Dis\"", "\"b\".\"Display Name\"")
            } else {
                source.replace("\"Dis\"", "\"Display Name\"")
            };
            assert_eq!(applied, expected);
        }
    }

    #[test]
    fn stale_versions_and_other_tabs_are_rejected() {
        let session = LanguageSession {
            document: Some(Document {
                id: 4,
                version: 9,
                text: "x".into(),
                cursor: 1,
            }),
            ..Default::default()
        };
        assert!(session.matches(4, 9));
        assert!(!session.matches(4, 8));
        assert!(!session.matches(3, 9));
    }
}
