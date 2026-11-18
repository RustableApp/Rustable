//! UI-owned Matrix actions. No socket, file or database IO in these handlers.
use super::*;
use serde_json::{Value as Json, json};

impl Rustable {
    #[cfg(feature = "qa")]
    pub(crate) fn matrix_capture_ready(&self) -> bool {
        self.matrix
            .as_ref()
            .is_some_and(|bridge| bridge.owned.len() >= 3)
            && self.pending.is_none()
            && !self.tabs[self.tab].results.is_empty()
    }

    #[cfg(feature = "qa")]
    pub(crate) fn prepare_matrix_fixture_capture(&mut self) {
        let bridge = self.matrix.as_mut().unwrap();
        bridge.demo = false;
        bridge.enabled = false;
    }

    #[cfg(feature = "qa")]
    pub(crate) fn grant_matrix_fixture(&mut self, ctx: &egui::Context) {
        let key = egui::Id::new((ctx.viewport_id(), "matrix-fixture-granted"));
        if self.pending.is_none()
            && self.engine == "SQLite"
            && !ctx.data(|d| d.get_temp::<bool>(key).unwrap_or(false))
        {
            ctx.data_mut(|d| d.insert_temp(key, true));
            if let Some(bridge) = &mut self.matrix {
                bridge.enabled = true;
            }
        }
    }

    /// QA-only native capture of a real SQLite connection, including opt-in state.
    #[cfg(feature = "qa")]
    pub(crate) fn matrix_fixture_capture(&mut self, ctx: &egui::Context) {
        let Some(prefix) = std::env::var_os("RUSTABLE_MATRIX_CAPTURE") else {
            return;
        };
        if self.pending.is_some() || self.matrix.is_none() {
            return;
        }
        let key = egui::Id::new((ctx.viewport_id(), "matrix-fixture-capture"));
        let mut stage = ctx.data(|d| d.get_temp::<u8>(key).unwrap_or(0));
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let suffix = if stage == 1 { "disabled" } else { "active" };
            let path = format!(
                "{}-{suffix}.ppm",
                std::path::PathBuf::from(&prefix).display()
            );
            std::thread::spawn(move || {
                let mut bytes =
                    format!("P6\n{} {}\n255\n", image.width(), image.height()).into_bytes();
                for pixel in &image.pixels {
                    bytes.extend_from_slice(&pixel.to_array()[..3]);
                }
                std::fs::write(path, bytes).expect("write Matrix fixture capture");
            });
            if stage == 1 {
                assert_eq!(self.engine, "SQLite");
                self.safe = true;
                self.inspector = false;
                self.matrix.as_mut().unwrap().enabled = true;
                stage = 2;
                ctx.request_repaint();
            } else {
                stage = 4;
            }
        }
        if stage == 0 || (stage == 2 && self.matrix_capture_ready()) {
            stage += 1;
            ctx.request_repaint();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ctx.data_mut(|d| d.insert_temp(key, stage));
    }

    pub(crate) fn attach_matrix(&mut self, bridge: crate::matrix::Bridge) {
        self.inspector = false;
        self.matrix = Some(bridge);
    }

    #[cfg(test)]
    pub(crate) fn grant_matrix_for_test(&mut self) {
        assert!(self.supports_safe_mode() && self.pending.is_none());
        self.matrix.as_mut().unwrap().enabled = true;
    }

    pub(crate) fn offer_matrix(&mut self) {
        if self.matrix.is_none() && self.redis_workspace.is_none() {
            self.matrix = Some(crate::matrix::Bridge::window());
        }
    }

    pub(crate) fn matrix_id(&self) -> Option<u64> {
        self.matrix.as_ref().map(|bridge| bridge.id)
    }

    pub(crate) fn matrix_summary(&self) -> Option<Json> {
        let bridge = self.matrix.as_ref().filter(|bridge| bridge.enabled)?;
        self.supports_safe_mode().then(|| json!({"workspace_id":bridge.id,
            "connection":self.active_connection_name().chars().take(200).collect::<String>(),
            "engine":self.engine,"database":self.connected_database,"schema":self.schemas.selected,
            "safe_mode":self.safe_mode_active(),"production":self.is_production(),"busy":self.pending.is_some()}))
    }

    pub(crate) fn pause_matrix(&mut self, changed_connection: bool) {
        if let Some(mut bridge) = self.matrix.take() {
            if self.matrix_running(&bridge) {
                self.worker.cancel();
            }
            if self
                .destructive_confirm
                .as_ref()
                .is_some_and(|run| bridge.running_tab == Some(run.tab_id))
            {
                self.destructive_confirm = None;
            }
            bridge.revoke();
            if let Some(edit) = &mut bridge.edit {
                edit.streaming = false;
            }
            if changed_connection {
                bridge.owned.clear();
                bridge.edit = None;
                bridge.running_tab = None;
            }
            bridge.activity = if changed_connection {
                "Connection or schema changed · AI control is off"
            } else {
                "You have control · AI paused"
            }
            .into();
            self.matrix = Some(bridge);
        }
    }

    pub(crate) fn handle_matrix(&mut self, request: crate::matrix::Request) {
        let Some(mut bridge) = self.matrix.take() else {
            let _ = request
                .reply
                .try_send(Err("AI control is unavailable in this workspace".into()));
            return;
        };
        self.check_matrix_edit(&mut bridge);
        let result = if request.expires < std::time::Instant::now() {
            Err("Request expired before execution".into())
        } else if request.name == "list_workspaces" {
            if bridge.enabled && self.supports_safe_mode() {
                Ok(
                    json!({"workspaces":[{"workspace_id":bridge.id,"connection":self.active_connection_name(),"engine":self.engine,"safe_mode":self.safe_mode_active()}]}),
                )
            } else {
                Ok(json!({"workspaces":[]}))
            }
        } else if request.arguments["workspace_id"].as_u64() != Some(bridge.id) {
            Err("Workspace target expired or changed. Ask the user to enable the intended workspace and list it again; do not switch targets automatically.".into())
        } else if !bridge.enabled {
            Err("The user took over. Ask them to enable AI control in this window.".into())
        } else {
            self.matrix_action(&mut bridge, &request.name, &request.arguments)
        };
        if !matches!(
            request.name.as_str(),
            "workspace" | "inspect_query" | "list_workspaces"
        ) {
            bridge.activity = match &result {
                Ok(_) => match request.name.as_str() {
                    "begin_query_edit" | "append_query_edit" => "AI · updating this query".into(),
                    "finish_query_edit" | "replace_query" => "AI · edit complete".into(),
                    "revert_query_edit" => "AI · edit undone".into(),
                    _ => format!("AI · {}", request.name.replace('_', " ")),
                },
                Err(error) => format!("Paused · {}", error.chars().take(100).collect::<String>()),
            };
        }
        let _ = request.reply.try_send(result);
        self.matrix = Some(bridge);
    }

    pub(super) fn matrix_frame(&mut self, ctx: &egui::Context) {
        let Some(mut bridge) = self.matrix.take() else {
            return;
        };
        if bridge.activity == "AI · run query" && self.pending.is_none() {
            bridge.activity = if self.destructive_confirm.is_some() {
                "Waiting for your confirmation".into()
            } else if self.tabs[self.tab].result_error.is_some() {
                "Query failed · inspect the result message".into()
            } else {
                "Query complete · ready for the next step".into()
            };
        }
        // Demo hosts read directly; real windows receive explicitly routed requests.
        let request = bridge
            .requests
            .as_ref()
            .and_then(|requests| requests.try_recv().ok());
        if let Some(request) = request {
            self.matrix = Some(bridge);
            self.handle_matrix(request);
            bridge = self.matrix.take().expect("handler restores controls");
        }
        self.check_matrix_edit(&mut bridge);
        if let Some(edit) = &mut bridge.edit
            && edit.streaming
            && (self.view != View::Query || self.tabs[self.tab].id != edit.tab_id)
        {
            edit.streaming = false;
        }
        let mut take_over = false;
        let mut undo_edit = None;
        let mut stop_edit = false;
        egui::TopBottomPanel::top("matrix-control").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong("Matrix");
                ui.weak(if bridge.demo { "Disposable demo" } else { "This workspace" });
                ui.label(if self.safe_mode_active() { "Safe mode on" } else { "Writes allowed" });
                ui.separator();
                ui.label(if bridge.enabled {
                    bridge.activity.as_str()
                } else {
                    "You have control · AI paused"
                });
                if bridge.enabled {
                    if ui.button("Take over").clicked() {
                        take_over = true;
                    }
                } else if ui.add_enabled(self.supports_safe_mode() && self.pending.is_none()
                    && !self.worker.transaction_state().blocks_replacement(), egui::Button::new("Enable AI control"))
                    .on_hover_text("Allow the connected MCP client to read and edit open SQL tabs, read data, and run SQL in this connection. SQL uses this connection’s existing privileges. Safe mode and normal confirmations still apply. Finish active work or transactions first.").clicked() {
                    bridge.enabled = true;
                    bridge.activity = "Ready for your AI client".into();
                }
            });
            if let Some(edit) = &bridge.edit {
                ui.horizontal_wrapped(|ui| {
                    ui.label(if edit.conflict { "Your changes kept · AI edit stopped" }
                        else if edit.streaming { "AI is editing SQL…" }
                        else if edit.finished { "AI edit complete" } else { "AI edit stopped" });
                    ui.weak(format!("{} characters changed", edit.inserted_chars));
                    if edit.streaming && ui.button("Stop edit").clicked() { stop_edit = true; }
                    if ui.add_enabled(!edit.conflict && self.pending.is_none(), egui::Button::new("Undo AI edit")).clicked() { undo_edit = Some(edit.id); }
                });
            }
        });
        if stop_edit && let Some(edit) = &mut bridge.edit {
            edit.streaming = false;
        }
        if let Some(id) = undo_edit {
            let _ =
                self.continue_matrix_edit(&mut bridge, "revert_query_edit", &json!({"edit_id":id}));
        }
        self.matrix = Some(bridge);
        if take_over {
            self.pause_matrix(false);
        }
    }

    fn matrix_running(&self, bridge: &crate::matrix::Bridge) -> bool {
        matches!(self.pending, Some(Pending::Query(index)) if self.tabs.get(index).is_some_and(|tab| bridge.running_tab == Some(tab.id)))
    }

    fn matrix_action(
        &mut self,
        bridge: &mut crate::matrix::Bridge,
        name: &str,
        args: &Json,
    ) -> Result<Json, String> {
        if !self.supports_safe_mode() {
            return Err("Open a SQLite, PostgreSQL, MySQL or MariaDB connection first.".into());
        }
        fn text<'a>(args: &'a Json, key: &str) -> Result<&'a str, String> {
            let value = args[key]
                .as_str()
                .ok_or_else(|| format!("Missing string {key}"))?;
            if value.len() > 32 * 1024 {
                return Err(format!("{key} exceeds 32 KiB"));
            }
            Ok(value)
        }
        if name == "workspace" {
            let mut tables = Vec::new();
            let mut remaining = 48 * 1024;
            for object in self.objects.iter().take(200) {
                let table = json!({"name":object.name.chars().take(256).collect::<String>(),
                    "kind":object.kind,"columns":object.columns.iter().take(50)
                        .map(|column|column.name.chars().take(128).collect::<String>()).collect::<Vec<_>>(),
                    "columns_truncated":object.columns.len() > 50});
                let size = table.to_string().len();
                if size > remaining {
                    break;
                }
                remaining -= size;
                tables.push(table);
            }
            return Ok(
                json!({"workspace_id":bridge.id,"connection":self.active_connection_name().chars().take(200).collect::<String>(),
                "engine":self.engine,"database":self.connected_database,"schema":self.schemas.selected,
                "production":self.is_production(),"transaction_state":self.worker.transaction_state(),
                "busy":self.pending.is_some(), "safe_mode":self.safe_mode_active(),
                "active_tab_id":if self.view == View::Query { Some(self.tabs[self.tab].id) } else { None },
                "active_view":if self.view == View::Query { "query" } else { "table" },
                "edit":bridge.edit.as_ref().map(|e|e.summary()),
                "tabs":self.tabs.iter().filter(|t| t.open).take(50).map(|t|json!({"id":t.id,
                    "title":t.source_name.as_ref().map(|name|name.chars().take(100).collect::<String>()),
                    "ai_owned":bridge.owned.contains(&t.id),"active":self.view == View::Query && t.id == self.tabs[self.tab].id})).collect::<Vec<_>>(),
                "catalog_truncated":tables.len() < self.objects.len(),"tables":tables}),
            );
        }
        if name == "cancel_query" {
            if self.matrix_running(bridge) {
                self.worker.cancel();
            }
            return Ok(json!({"cancel_requested":true}));
        }
        if name == "inspect_query" {
            let index = self.matrix_tab(args)?;
            let tab = &self.tabs[index];
            let grid = &tab.grid;
            let rows: Vec<Vec<String>> = grid
                .rows
                .iter()
                .take(20)
                .map(|row| {
                    row.iter()
                        .take(30)
                        .map(|v| match v {
                            Value::Text(t) => t.chars().take(200).collect(),
                            Value::Blob(b) => format!("<{} bytes>", b.len()),
                            _ => db::display(v),
                        })
                        .collect()
                })
                .collect();
            return Ok(
                json!({"tab_id":tab.id,"sql":tab.sql.chars().take(32768).collect::<String>(),"sql_truncated":tab.sql.len() > 32768,"running":matches!(self.pending,Some(Pending::Query(i)) if i == index),
                "selection":{"start":tab.cursor.start,"end":tab.cursor.end},
                "active":self.view == View::Query && index == self.tab,
                "edit":bridge.edit.as_ref().filter(|e|e.tab_id == tab.id).map(|e|e.summary()),
                "awaiting_confirmation":self.destructive_confirm.as_ref().is_some_and(|r|r.tab_id == tab.id),
                "error":tab.result_error,"result_sets":tab.results.len(),"columns":grid.columns.iter().take(30).map(|c|c.chars().take(200).collect::<String>()).collect::<Vec<_>>(),
                "preview":rows,"total_rows":grid.row_count(),"preview_only":true}),
            );
        }
        // Never navigate away from unfinished human work or interfere with a modal.
        if self.pending.is_some()
            || self.has_pending_edits()
            || self.edit.is_some()
            || self.destructive_confirm.is_some()
            || self.navigation_confirmation.is_some()
            || self.connections
            || self.review.is_some()
            || self.csv_preview.is_some()
            || self.csv_review.is_some()
            || self.json_editor.is_some()
            || self.rename.is_some()
            || self.add_column.is_some()
            || self.alter_column.is_some()
            || self.index_draft.is_some()
            || self.date_picker.is_some()
            || self.sidebar_export.is_some()
            || self.history
        {
            return Err(
                "Workspace busy or awaiting user input; retry after the user finishes.".into(),
            );
        }
        if matches!(
            name,
            "append_query_edit" | "finish_query_edit" | "revert_query_edit"
        ) {
            return self.continue_matrix_edit(bridge, name, args);
        }
        if bridge.edit.as_ref().is_some_and(|e| e.streaming) {
            return Err(
                "Finish or stop the current AI edit before opening, replacing or running SQL."
                    .into(),
            );
        }
        match name {
            "begin_query_edit" => {
                let index = self.matrix_tab(args)?;
                let start = args["start"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or("Missing start")?;
                let end = args["end"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or("Missing end")?;
                self.begin_matrix_edit(bridge, index, text(args, "expected_sql")?, start, end)
            }
            "open_query" => {
                bridge
                    .owned
                    .retain(|id| self.tabs.iter().any(|tab| tab.id == *id && tab.open));
                if bridge.owned.len() >= 16 {
                    return Err("Close an AI tab before opening more (limit 16).".into());
                }
                let sql = text(args, "sql")?;
                let title = text(args, "title")?.chars().take(60).collect::<String>();
                self.new_tab();
                let tab = &mut self.tabs[self.tab];
                tab.sql = sql.into();
                tab.source_name = Some(format!("AI · {title}"));
                bridge.owned.insert(tab.id);
                Ok(json!({"tab_id":tab.id}))
            }
            "replace_query" | "run_query" => {
                let index = self.matrix_tab(args)?;
                if self.tabs[index].sql != text(args, "expected_sql")? {
                    return Err("SQL changed. Inspect the tab; do not overwrite user edits.".into());
                }
                if name == "replace_query" {
                    let sql = text(args, "sql")?.to_owned();
                    let expected = text(args, "expected_sql")?;
                    let opened = self.begin_matrix_edit(
                        bridge,
                        index,
                        expected,
                        0,
                        expected.chars().count(),
                    )?;
                    self.continue_matrix_edit(
                        bridge,
                        "append_query_edit",
                        &json!({"edit_id":opened["edit_id"],"sequence":0,"text":sql}),
                    )?;
                    return self.continue_matrix_edit(
                        bridge,
                        "finish_query_edit",
                        &json!({"edit_id":opened["edit_id"]}),
                    );
                }
                self.activate_query(index);
                bridge.running_tab = Some(self.tabs[index].id);
                self.run_query(true);
                Ok(
                    json!({"tab_id":self.tabs[index].id,"started":self.pending.is_some(),"awaiting_confirmation":self.destructive_confirm.is_some()}),
                )
            }
            _ => Err(format!("Unknown tool {name}")),
        }
    }
    fn matrix_tab(&self, args: &Json) -> Result<usize, String> {
        let id = args["tab_id"].as_u64().ok_or("Missing tab_id")?;
        self.tabs
            .iter()
            .position(|t| t.id == id && t.open)
            .ok_or_else(|| "Tab was closed".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Rustable, crate::matrix::Bridge) {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        app.connections = false;
        app.inspector = false;
        app.engine = "SQLite".into();
        app.label = "Studio demo · in memory".into();
        let (_, requests) = mpsc::sync_channel(1);
        (
            app,
            crate::matrix::Bridge {
                requests: Some(requests),
                id: crate::matrix::Bridge::next_id(),
                demo: true,
                enabled: true,
                activity: String::new(),
                owned: Default::default(),
                running_tab: None,
                edit: None,
            },
        )
    }
    #[test]
    fn matrix_edits_requested_human_tabs_and_rejects_stale_sql() {
        let (mut app, mut bridge) = fixture();
        app.tabs[0].sql = "-- human draft".into();
        app.tabs[0].open = true;
        let human = app.tabs[0].id;
        let created = app
            .matrix_action(
                &mut bridge,
                "open_query",
                &json!({"title":"Example","sql":"SELECT 1"}),
            )
            .unwrap();
        let id = created["tab_id"].as_u64().unwrap();
        assert_eq!(app.tabs[0].sql, "-- human draft");
        assert!(
            app.matrix_action(
                &mut bridge,
                "replace_query",
                &json!({"tab_id":human,"expected_sql":"-- human draft","sql":"-- revised draft"})
            )
            .is_ok()
        );
        app.tabs.iter_mut().find(|t| t.id == id).unwrap().sql = "SELECT 2 -- user edit".into();
        assert!(
            app.matrix_action(
                &mut bridge,
                "run_query",
                &json!({"tab_id":id,"expected_sql":"SELECT 1"})
            )
            .unwrap_err()
            .contains("SQL changed")
        );
        assert!(app.pending.is_none());
    }
    #[test]
    fn matrix_streams_into_human_tab_with_unicode_and_one_undo_step() {
        let (mut app, mut bridge) = fixture();
        let original = "SELECT '🦀' AS name;";
        app.tabs[0].sql = original.into();
        app.tabs[0].source_name = Some("my-query.sql".into());
        app.tabs[0].saved_sql = Some(Arc::new(original.into()));
        app.view = View::Query;
        app.tabs[0].open = true;
        let id = app.tabs[0].id;
        let inspected = app
            .matrix_action(&mut bridge, "inspect_query", &json!({"tab_id":id}))
            .unwrap();
        assert_eq!(inspected["sql"], original);
        assert_eq!(
            app.matrix_action(&mut bridge, "workspace", &json!({}))
                .unwrap()["active_tab_id"],
            id
        );
        let edit = app
            .matrix_action(
                &mut bridge,
                "begin_query_edit",
                &json!({"tab_id":id,"expected_sql":original,"start":7,"end":10}),
            )
            .unwrap()["edit_id"]
            .clone();
        assert_eq!(
            app.tabs[0].sql, original,
            "begin doesn't erase the user's SQL"
        );
        let ctx = egui::Context::default();
        let editor = egui::Id::new("shared-editor-test");
        for (sequence, chunk) in ["'hello", " 🌍'"].into_iter().enumerate() {
            app.matrix_action(
                &mut bridge,
                "append_query_edit",
                &json!({"edit_id":edit,"sequence":sequence,"text":chunk}),
            )
            .unwrap();
            let mut holder = Some(bridge);
            super::super::matrix_edit::sync_editor(&mut holder, &mut app.tabs[0], &ctx, editor);
            bridge = holder.unwrap();
        }
        assert_eq!(app.tabs[0].sql, "SELECT 'hello 🌍' AS name;");
        assert!(app.tabs[0].dirty);
        assert_eq!(app.tabs[0].source_name.as_deref(), Some("my-query.sql"));
        assert!(
            app.matrix_action(
                &mut bridge,
                "run_query",
                &json!({"tab_id":id,"expected_sql":app.tabs[0].sql})
            )
            .is_err()
        );
        assert!(
            app.matrix_action(
                &mut bridge,
                "append_query_edit",
                &json!({"edit_id":edit,"sequence":1,"text":"duplicate"})
            )
            .is_err()
        );
        app.matrix_action(&mut bridge, "finish_query_edit", &json!({"edit_id":edit}))
            .unwrap();
        let state = egui::TextEdit::load_state(&ctx, editor).unwrap();
        let mut undo = state.undoer();
        let now = (
            egui::text::CCursorRange::one(egui::text::CCursor::new(app.tabs[0].cursor.start)),
            app.tabs[0].sql.clone(),
        );
        assert_eq!(
            undo.undo(&now).unwrap().1,
            original,
            "one undo restores the complete original"
        );
        app.matrix_action(&mut bridge, "revert_query_edit", &json!({"edit_id":edit}))
            .unwrap();
        assert_eq!(app.tabs[0].sql, original);
        assert!(!app.tabs[0].dirty);
    }

    #[test]
    fn matrix_stream_conflicts_switching_takeover_and_bounds() {
        for interrupted in ["typing", "switch", "takeover", "close"] {
            let (mut app, mut bridge) = fixture();
            app.view = View::Query;
            app.tabs[0].open = true;
            app.tabs[0].sql = "SELECT 1".into();
            let id = app.tabs[0].id;
            let edit = app
                .matrix_action(
                    &mut bridge,
                    "begin_query_edit",
                    &json!({"tab_id":id,"expected_sql":"SELECT 1","start":7,"end":8}),
                )
                .unwrap()["edit_id"]
                .clone();
            assert!(
                app.matrix_action(
                    &mut bridge,
                    "append_query_edit",
                    &json!({"edit_id":edit,"sequence":0,"text":"x".repeat(32768)})
                )
                .is_err()
            );
            app.matrix_action(
                &mut bridge,
                "append_query_edit",
                &json!({"edit_id":edit,"sequence":0,"text":"2"}),
            )
            .unwrap();
            match interrupted {
                "typing" => app.tabs[0].sql.push_str(" -- mine"),
                "switch" => app.new_tab(),
                "close" => app.tabs[0].open = false,
                _ => {
                    app.matrix = Some(bridge);
                    app.pause_matrix(false);
                    bridge = app.matrix.take().unwrap();
                }
            }
            let before = app.tabs[0].sql.clone();
            assert!(
                app.matrix_action(
                    &mut bridge,
                    "append_query_edit",
                    &json!({"edit_id":edit,"sequence":1,"text":"3"})
                )
                .is_err()
            );
            assert_eq!(app.tabs[0].sql, before);
            if interrupted == "typing" {
                assert!(
                    app.matrix_action(&mut bridge, "revert_query_edit", &json!({"edit_id":edit}))
                        .is_err()
                );
            }
        }
    }

    #[test]
    fn matrix_expired_and_paused_requests_do_not_mutate_workspace() {
        for paused in [false, true] {
            let (mut app, mut bridge) = fixture();
            let (send, requests) = mpsc::sync_channel(1);
            bridge.requests = Some(requests);
            bridge.enabled = !paused;
            let (reply, receive) = mpsc::sync_channel(1);
            send.send(crate::matrix::Request {
                name: "open_query".into(),
                arguments: json!({"title":"x","sql":"SELECT 1","workspace_id":bridge.id}),
                expires: if paused {
                    std::time::Instant::now() + std::time::Duration::from_secs(5)
                } else {
                    std::time::Instant::now() - std::time::Duration::from_secs(1)
                },
                reply,
            })
            .unwrap();
            app.matrix = Some(bridge);
            let count = app.tabs.len();
            let ctx = egui::Context::default();
            let _ = ctx.run(Default::default(), |ctx| app.matrix_frame(ctx));
            assert!(receive.try_recv().unwrap().is_err());
            assert_eq!(app.tabs.len(), count);
        }
    }
    fn wait(app: &mut Rustable) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.pending.is_some() {
            assert!(std::time::Instant::now() < deadline, "worker timed out");
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    #[test]
    fn matrix_runs_in_real_editor_and_keeps_safe_mode() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        wait(&mut app);
        app.connections = false;
        app.safe = true;
        let (_, mut bridge) = fixture();
        for (sql, rejected) in [
            ("SELECT COUNT(*) FROM customers", false),
            ("DELETE FROM customers", true),
        ] {
            let opened = app
                .matrix_action(
                    &mut bridge,
                    "open_query",
                    &json!({"title":"Test","sql":sql}),
                )
                .unwrap();
            let id = opened["tab_id"].as_u64().unwrap();
            let result = app
                .matrix_action(
                    &mut bridge,
                    "run_query",
                    &json!({"tab_id":id,"expected_sql":sql}),
                )
                .unwrap();
            assert_eq!(result["started"], true);
            wait(&mut app);
            let result = app
                .matrix_action(&mut bridge, "inspect_query", &json!({"tab_id":id}))
                .unwrap();
            assert_eq!(!result["error"].is_null(), rejected, "{result}");
            assert!(app.tabs[app.tab].open);
            assert!(app.safe);
        }
    }

    #[test]
    fn matrix_takeover_button_cancels_work_and_leaves_query_editable() {
        fn text_rect(shape: &egui::Shape) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == "Take over" => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(text_rect),
                _ => None,
            }
        }
        let ctx = egui::Context::default();
        let mut app = Rustable::with_preferences(&ctx, Preferences::default());
        wait(&mut app);
        app.connections = false;
        let (_, mut bridge) = fixture();
        let sql =
            "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n) SELECT SUM(x) FROM n";
        let id = app
            .matrix_action(
                &mut bridge,
                "open_query",
                &json!({"title":"Cancel me","sql":sql}),
            )
            .unwrap()["tab_id"]
            .clone();
        app.matrix_action(
            &mut bridge,
            "run_query",
            &json!({"tab_id":id,"expected_sql":sql}),
        )
        .unwrap();
        app.matrix = Some(bridge);
        let output = ctx.run(Default::default(), |ctx| app.matrix_frame(ctx));
        let pos = output
            .shapes
            .iter()
            .find_map(|s| text_rect(&s.shape))
            .unwrap()
            .center();
        let _ = ctx.run(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            },
            |ctx| app.matrix_frame(ctx),
        );
        assert!(!app.matrix.as_ref().unwrap().enabled);
        wait(&mut app);
        assert!(app.tabs[app.tab].result_error.is_some());
        assert_eq!(app.tabs[app.tab].sql, sql);
        assert!(app.tabs[app.tab].open);
    }

    #[test]
    fn matrix_status_bar_settles_at_idle() {
        let ctx = egui::Context::default();
        let (mut app, bridge) = fixture();
        app.matrix = Some(bridge);
        for index in 0..20 {
            let output = ctx.run(
                egui::RawInput {
                    time: Some(index as f64),
                    ..Default::default()
                },
                |ctx| app.matrix_frame(ctx),
            );
            if index == 19 {
                assert!(
                    output
                        .viewport_output
                        .values()
                        .all(|viewport| viewport.repaint_delay > std::time::Duration::ZERO)
                );
            }
        }
    }

    #[test]
    fn matrix_stream_edit_controls_settle_and_manual_run_waits() {
        let ctx = egui::Context::default();
        let (mut app, mut bridge) = fixture();
        app.view = View::Query;
        app.tabs[0].open = true;
        app.tabs[0].sql = "SELECT 1".into();
        let id = app.tabs[0].id;
        app.matrix_action(
            &mut bridge,
            "begin_query_edit",
            &json!({"tab_id":id,"expected_sql":"SELECT 1","start":7,"end":8}),
        )
        .unwrap();
        app.matrix = Some(bridge);
        app.run_query(true);
        assert!(app.pending.is_none());
        assert!(app.error.as_ref().unwrap().contains("stop the AI edit"));
        for index in 0..20 {
            let output = ctx.run(
                egui::RawInput {
                    time: Some(index as f64),
                    ..Default::default()
                },
                |ctx| app.matrix_frame(ctx),
            );
            if index == 19 {
                assert!(
                    output
                        .viewport_output
                        .values()
                        .all(|v| v.repaint_delay > std::time::Duration::ZERO)
                );
            }
        }
        assert!(
            app.matrix
                .as_ref()
                .unwrap()
                .edit
                .as_ref()
                .unwrap()
                .streaming
        );
    }

    #[test]
    #[ignore = "release CPU workload for streamed shared-tab edits"]
    fn matrix_stream_edit_frame_benchmark() {
        for width in [760.0, 1280.0] {
            let ctx = egui::Context::default();
            let (mut app, mut bridge) = fixture();
            app.view = View::Query;
            app.tabs[0].open = true;
            app.tabs[0].sql = "SELECT 1;\n".into();
            let id = app.tabs[0].id;
            let opened = app
                .matrix_action(
                    &mut bridge,
                    "begin_query_edit",
                    &json!({"tab_id":id,"expected_sql":"SELECT 1;\n","start":10,"end":10}),
                )
                .unwrap();
            let (send, requests) = mpsc::sync_channel(1);
            bridge.requests = Some(requests);
            let workspace = bridge.id;
            app.matrix = Some(bridge);
            let mut samples = Vec::new();
            for index in 0..180 {
                let (reply, receive) = mpsc::sync_channel(1);
                send.send(crate::matrix::Request { name:"append_query_edit".into(),
                    arguments:json!({"workspace_id":workspace,"edit_id":opened["edit_id"],"sequence":index,"text":format!("-- streamed line {index}: preserve the user's query\n")}),
                    expires:std::time::Instant::now()+std::time::Duration::from_secs(5),reply }).unwrap();
                let start = std::time::Instant::now();
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 760.0),
                        )),
                        time: Some(index as f64 / 120.0 + if index >= 100 { 30.0 } else { 0.0 }),
                        ..Default::default()
                    },
                    |ctx| {
                        app.matrix_frame(ctx);
                        app.toolbar(ctx);
                        app.workspace(ctx);
                    },
                );
                let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
                let ms = start.elapsed().as_secs_f64() * 1000.0;
                assert!(receive.try_recv().unwrap().is_ok());
                if index == 0 || index == 100 {
                    eprintln!("Stream {width}px frame {index}: {ms:.6}ms");
                }
                if index >= 20 {
                    samples.push(ms);
                }
            }
            samples.sort_by(f64::total_cmp);
            crate::ui_benchmark::report(&format!("matrix/stream-edit/{width}"), &samples);
        }
    }

    #[test]
    #[ignore = "manual release CPU benchmark for the new Matrix interaction"]
    fn matrix_frame_benchmark() {
        for width in [760.0, 1280.0] {
            let ctx = egui::Context::default();
            let (mut app, mut bridge) = fixture();
            let tab = app
                .matrix_action(
                    &mut bridge,
                    "open_query",
                    &json!({"title":"Customers","sql":"SELECT * FROM customers LIMIT 20;"}),
                )
                .unwrap()["tab_id"]
                .clone();
            let grid = Arc::new(Grid {
                columns: (0..30).map(|index| format!("column_{index}")).collect(),
                rows: vec![vec![Value::Text("value ".repeat(35)); 30]; 20],
                ..Default::default()
            });
            app.tabs[app.tab].grid = grid.clone();
            app.tabs[app.tab].results = vec![grid];
            let (send, requests) = mpsc::sync_channel(1);
            bridge.requests = Some(requests);
            app.matrix = Some(bridge);
            let mut samples = Vec::new();
            for index in 0..180 {
                let (reply, receive) = mpsc::sync_channel(1);
                send.try_send(crate::matrix::Request {
                    name: "inspect_query".into(),
                    arguments: json!({"tab_id":tab,"workspace_id":app.matrix.as_ref().unwrap().id}),
                    expires: std::time::Instant::now() + std::time::Duration::from_secs(5),
                    reply,
                })
                .unwrap();
                let start = std::time::Instant::now();
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 760.0),
                        )),
                        time: Some(index as f64 / 120.0 + if index >= 100 { 30.0 } else { 0.0 }),
                        ..Default::default()
                    },
                    |ctx| {
                        app.matrix_frame(ctx);
                        app.toolbar(ctx);
                        app.workspace(ctx);
                    },
                );
                let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                assert!(receive.try_recv().unwrap().is_ok());
                if index == 0 || index == 100 {
                    eprintln!(
                        "Matrix {width}px frame {index} CPU: {elapsed:.6}ms (0=cold, 100=after idle)"
                    );
                }
                if index >= 20 {
                    samples.push(elapsed);
                }
            }
            samples.sort_by(f64::total_cmp);
            crate::ui_benchmark::report(&format!("matrix/inspect/{width}"), &samples);
        }
    }
}
