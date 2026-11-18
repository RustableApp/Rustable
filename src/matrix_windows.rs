//! Routing is local UI state only. Never reconnect, select a database, or replay
//! commands to find a target. A hidden/closed/revoked workspace is not a fallback.
use super::*;
use serde_json::json;

impl SessionWindows {
    pub(crate) fn attach_matrix(&mut self, bridge: crate::matrix::Bridge) {
        self.root.offer_matrix_connection(bridge.connector.as_ref());
        self.matrix = Some(bridge);
        #[cfg(feature = "qa")]
        if let Some(path) = std::env::var_os("RUSTABLE_MATRIX_FIXTURE") {
            // Explicit QA-only fixture: never selects a saved connection.
            self.crash_prompt = None; // Keep unrelated prior-session prompts out of fixture captures.
            self.windows.push(SessionWindow {
                id: egui::ViewportId::from_hash_of("matrix-fixture"),
                title: "Matrix SQLite fixture".into(),
                size_key: None,
                initial_size: if std::env::var_os("RUSTABLE_MATRIX_NARROW").is_some() {
                    [760.0, 760.0]
                } else {
                    [1280.0, 820.0]
                },
                request: Some(WindowRequest {
                    recover_sql: false,
                    identity: None,
                    title: "Matrix SQLite fixture".into(),
                    connection: WindowConnection::Sqlite(path.into()),
                }),
                app: None,
                parked: vec![],
                active_slot: 0,
                closed: false,
                connected: false,
            });
        }
    }

    pub(super) fn poll_matrix(&mut self, ctx: &egui::Context) {
        let request = self
            .matrix
            .as_ref()
            .and_then(|bridge| bridge.requests.as_ref())
            .and_then(|requests| requests.try_recv().ok());
        let Some(request) = request else { return };
        if request.expires < std::time::Instant::now() {
            let _ = request
                .reply
                .try_send(Err("Request expired before execution".into()));
            return;
        }
        if request.name == "list_workspaces" {
            let workspaces: Vec<_> = self
                .windows
                .iter()
                .filter(|window| !window.closed)
                .filter_map(|window| window.app.as_ref()?.matrix_summary())
                .take(50)
                .collect();
            let _ = request.reply.try_send(Ok(json!({"workspaces":workspaces})));
            return;
        }
        if self.quitting
            || (self.close_confirmation.is_some()
                && !matches!(
                    request.name.as_str(),
                    "workspace" | "inspect_query" | "cancel_query"
                ))
        {
            let _ = request
                .reply
                .try_send(Err("Rustable is closing or awaiting confirmation".into()));
            return;
        }
        let id = request.arguments["workspace_id"].as_u64();
        if let Some(window) = self.windows.iter_mut().find(|window| {
            !window.closed
                && id.is_some()
                && window.app.as_ref().is_some_and(|app| app.matrix_id() == id)
        }) {
            if let Some(app) = &mut window.app {
                app.handle_matrix(request);
                ctx.request_repaint_of(window.id);
            }
        } else {
            let _ = request.reply.try_send(Err("Workspace is closed, inactive, or its permission changed. Ask the user to enable the intended workspace and list it again; do not switch targets automatically.".into()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };

    fn workspace(ctx: &egui::Context, enabled: bool) -> Box<Rustable> {
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Box::new(Rustable::new_demo(&cc));
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.close_work_blocked() {
            assert!(Instant::now() < deadline);
            app.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        app.offer_matrix();
        if enabled {
            app.grant_matrix_for_test();
        }
        app
    }
    fn window(ctx: &egui::Context, id: u64, enabled: bool) -> SessionWindow {
        SessionWindow {
            id: egui::ViewportId::from_hash_of(id),
            title: "Fixture".into(),
            size_key: None,
            initial_size: [1280.0, 820.0],
            request: None,
            app: Some(workspace(ctx, enabled)),
            parked: vec![],
            active_slot: 0,
            closed: false,
            connected: true,
        }
    }
    fn request(
        manager: &mut SessionWindows,
        ctx: &egui::Context,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let (send, receive) = mpsc::sync_channel(1);
        let (reply, result) = mpsc::sync_channel(1);
        let mut bridge = crate::matrix::Bridge::window();
        bridge.requests = Some(receive);
        manager.attach_matrix(bridge);
        send.send(crate::matrix::Request {
            name: name.into(),
            arguments,
            expires: Instant::now() + Duration::from_secs(5),
            reply,
        })
        .unwrap();
        manager.poll_matrix(ctx);
        result.try_recv().unwrap()
    }

    #[test]
    #[ignore = "release UI routing benchmark"]
    fn matrix_routing_benchmark() {
        let ctx = egui::Context::default();
        let mut manager = super::super::launcher_tests::test_manager(&ctx);
        manager.windows = (0..4).map(|id| window(&ctx, id, true)).collect();
        let id = manager.windows[3]
            .app
            .as_ref()
            .unwrap()
            .matrix_id()
            .unwrap();
        for name in ["list_workspaces", "workspace"] {
            let (send, receive) = mpsc::sync_channel(1);
            let mut bridge = crate::matrix::Bridge::window();
            bridge.requests = Some(receive);
            manager.attach_matrix(bridge);
            let mut samples = Vec::new();
            for index in 0..180 {
                let (reply, result) = mpsc::sync_channel(1);
                send.send(crate::matrix::Request {
                    name: name.into(),
                    arguments: json!({"workspace_id":id}),
                    expires: Instant::now() + Duration::from_secs(5),
                    reply,
                })
                .unwrap();
                let start = Instant::now();
                manager.poll_matrix(&ctx);
                let ms = start.elapsed().as_secs_f64() * 1000.0;
                assert!(result.try_recv().unwrap().is_ok());
                if index >= 20 {
                    samples.push(ms);
                }
            }
            samples.sort_by(f64::total_cmp);
            crate::ui_benchmark::report(&format!("matrix/routing/{name}/4-windows"), &samples);
        }
    }

    #[test]
    fn matrix_routes_exactly_and_only_lists_enabled_windows() {
        let ctx = egui::Context::default();
        let mut manager = super::super::launcher_tests::test_manager(&ctx);
        manager.windows = vec![window(&ctx, 1, true), window(&ctx, 2, false)];
        let id = manager.windows[0]
            .app
            .as_ref()
            .unwrap()
            .matrix_id()
            .unwrap();
        let hidden = manager.windows[1]
            .app
            .as_ref()
            .unwrap()
            .matrix_id()
            .unwrap();
        let listed = request(&mut manager, &ctx, "list_workspaces", json!({})).unwrap();
        assert_eq!(listed["workspaces"].as_array().unwrap().len(), 1);
        assert_eq!(listed["workspaces"][0]["workspace_id"], id);
        assert!(
            request(
                &mut manager,
                &ctx,
                "workspace",
                json!({"workspace_id":hidden})
            )
            .is_err()
        );
        let tab = request(
            &mut manager,
            &ctx,
            "open_query",
            json!({"workspace_id":id,"title":"Owned","sql":"SELECT 1"}),
        )
        .unwrap()["tab_id"]
            .clone();
        manager.windows[1]
            .app
            .as_mut()
            .unwrap()
            .grant_matrix_for_test();
        assert!(
            request(
                &mut manager,
                &ctx,
                "inspect_query",
                json!({"workspace_id":hidden,"tab_id":tab})
            )
            .is_err()
        );
        manager.windows[0].closed = true;
        assert!(
            request(
                &mut manager,
                &ctx,
                "run_query",
                json!({"workspace_id":id,"tab_id":tab,"expected_sql":"SELECT 1"})
            )
            .is_err()
        );
    }

    #[test]
    fn matrix_parking_and_takeover_invalidate_targets_without_fallback() {
        let ctx = egui::Context::default();
        let mut manager = super::super::launcher_tests::test_manager(&ctx);
        let mut window = window(&ctx, 1, true);
        let old_id = window.app.as_ref().unwrap().matrix_id().unwrap();
        window.parked.push(workspace(&ctx, false));
        window.switch_workspace(0);
        assert!(window.parked[0].matrix_summary().is_none());
        manager.windows.push(window);
        assert!(
            request(
                &mut manager,
                &ctx,
                "workspace",
                json!({"workspace_id":old_id})
            )
            .is_err()
        );
        manager.windows[0].switch_workspace(0);
        let app = manager.windows[0].app.as_mut().unwrap();
        app.grant_matrix_for_test();
        assert_ne!(app.matrix_id(), Some(old_id));
        assert!(
            request(
                &mut manager,
                &ctx,
                "workspace",
                json!({"workspace_id":old_id})
            )
            .is_err()
        );
        let id = manager.windows[0]
            .app
            .as_ref()
            .unwrap()
            .matrix_id()
            .unwrap();
        manager.windows[0].app.as_mut().unwrap().pause_matrix(false);
        manager.windows[0]
            .app
            .as_mut()
            .unwrap()
            .grant_matrix_for_test();
        assert!(request(&mut manager, &ctx, "workspace", json!({"workspace_id":id})).is_err());
    }
}
