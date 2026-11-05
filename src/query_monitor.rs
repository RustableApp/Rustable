//! Separate PostgreSQL control connection: the busy query connection is never reused.
use super::*;

#[derive(Clone)]
struct RunningQuery {
    pid: i32,
    started: String,
    backend_started: String,
    user: String,
    elapsed: String,
    wait: String,
    sql: String,
}
#[cfg(not(target_arch = "wasm32"))]
type MonitorResult = Result<(Vec<RunningQuery>, Option<String>), String>;

#[derive(Default)]
pub(super) struct QueryMonitor {
    pub open: bool,
    rows: Vec<RunningQuery>,
    selected: Option<usize>,
    message: Option<String>,
    #[cfg(not(target_arch = "wasm32"))]
    reply: Option<mpsc::Receiver<MonitorResult>>,
}
impl QueryMonitor {
    fn replace_rows(&mut self, rows: Vec<RunningQuery>, current_pid: Option<i32>) {
        // Preserve an explicit selection only while it is still the same query.
        let previous = self.selected.and_then(|index| self.rows.get(index));
        self.selected = previous.and_then(|old| rows.iter().position(|row|
            row.pid == old.pid && row.started == old.started && row.backend_started == old.backend_started && row.sql == old.sql))
            .or_else(|| rows.iter().position(|row| Some(row.pid) == current_pid))
            .or_else(|| (current_pid.is_none() && rows.len() == 1).then_some(0));
        self.rows = rows;
    }
}
impl Rustable {
    pub(super) fn request_query_cancel(&mut self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.worker.postgres_control().is_some() {
            self.query_monitor.open = true;
            self.query_monitor.rows.clear();
            self.query_monitor.selected = None;
            self.refresh_query_monitor(ctx, None);
            return;
        }
        let _ = ctx;
        self.worker.cancel();
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn refresh_query_monitor(&mut self, ctx: &egui::Context, cancel: Option<RunningQuery>) {
        if self.query_monitor.reply.is_some() { return; }
        let Some(control) = self.worker.postgres_control() else { return; };
        self.query_monitor.message = None;
        let (tx, rx) = mpsc::channel();
        self.query_monitor.reply = Some(rx);
        let ctx = ctx.clone();
        let viewport = ctx.viewport_id();
        std::thread::spawn(move || {
            let result = monitor_request(control, cancel);
            let _ = tx.send(result);
            ctx.request_repaint_of(viewport);
        });
    }

    pub(super) fn query_monitor_window(&mut self, ctx: &egui::Context) {
        if !self.query_monitor.open { return; }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(reply) = &self.query_monitor.reply {
            let result = match reply.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("Query monitor stopped. Your database operation was not changed.".into())),
            };
            if let Some(result) = result {
                self.query_monitor.reply = None;
                match result {
                    Ok((rows, message)) => {
                        let current_pid = self.worker.postgres_control().map(|control| control.process_id);
                        self.query_monitor.replace_rows(rows, current_pid);
                        self.query_monitor.message = message;
                    }
                    Err(error) => self.query_monitor.message = Some(error),
                }
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let busy = self.query_monitor.reply.is_some();
        #[cfg(target_arch = "wasm32")]
        let busy = false;
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.query_monitor.open = false;
            return;
        }
        let confirm = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
        });
        let mut open = true;
        let mut refresh = false;
        let mut cancel = None;
        egui::Window::new("Running queries").open(&mut open).collapsible(false)
            .default_width(760.0).min_width(580.0).show(ctx, |ui| {
                egui::Frame::new().inner_margin(12).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong("Active queries");
                        ui.weak(format!("{}", self.query_monitor.rows.len()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            refresh = ui.add_enabled(!busy, egui::Button::new("Refresh")).clicked();
                            if busy { ui.spinner(); }
                        });
                    });
                    ui.add_space(8.0);
                    egui::Frame::new().fill(ui.visuals().extreme_bg_color)
                        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                        .corner_radius(6).inner_margin(8).show(ui, |ui| {
                        if self.query_monitor.rows.is_empty() {
                            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 180.0),
                                egui::Layout::top_down(egui::Align::Center), |ui| {
                                ui.add_space(58.0);
                                ui.strong(if busy { "Loading active queries…" } else { "No active queries" });
                                ui.add_space(4.0);
                                ui.weak(if busy { "Contacting the database" } else { "Refresh to check for running queries." });
                            });
                        } else {
                            TableBuilder::new(ui).striped(true).resizable(true).sense(egui::Sense::click())
                                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                                .min_scrolled_height(180.0).max_scroll_height(300.0)
                                .column(TableColumn::initial(64.0).at_least(48.0).clip(true))
                                .column(TableColumn::initial(92.0).at_least(60.0).clip(true))
                                .column(TableColumn::initial(92.0).at_least(60.0).clip(true))
                                .column(TableColumn::initial(110.0).at_least(70.0).clip(true))
                                .column(TableColumn::remainder().at_least(120.0).clip(true))
                                .header(30.0, |mut header| {
                                    for label in ["PID", "User", "Elapsed", "Waiting on", "Query"] {
                                        header.col(|ui| { ui.add(egui::Label::new(RichText::new(label).strong()).truncate()); });
                                    }
                                })
                                .body(|body| {
                                    body.rows(32.0, self.query_monitor.rows.len(), |mut row| {
                                        let index = row.index();
                                        let query = &self.query_monitor.rows[index];
                                        row.set_selected(self.query_monitor.selected == Some(index));
                                        for value in [query.pid.to_string(), query.user.clone(), query.elapsed.clone(),
                                            if query.wait.is_empty() { "—".into() } else { query.wait.clone() }, query.sql.clone()] {
                                            row.col(|ui| { ui.add(egui::Label::new(&value).truncate()).on_hover_text(&value); });
                                        }
                                        if row.response().clicked() { self.query_monitor.selected = Some(index); }
                                    });
                                });
                        }
                    });
                    ui.add_space(8.0);
                    ui.add(egui::Label::new(RichText::new("Visibility and cancellation depend on your PostgreSQL permissions.").weak().small()).wrap());
                    if let Some(message) = &self.query_monitor.message {
                        ui.add_space(8.0);
                        egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(6)
                            .inner_margin(10).show(ui, |ui| {
                                ui.add(egui::Label::new(message).wrap().selectable(true));
                            });
                    }
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(self.pending.is_some(), egui::Button::new("Cancel current operation"))
                            .on_hover_text("Cancel the operation running in this workspace").clicked() {
                            self.worker.cancel();
                            self.query_monitor.message = Some("Cancellation requested. Waiting for the database to acknowledge it.".into());
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add_enabled(!busy && self.query_monitor.selected.is_some(), crate::icons::shortcut_button("Cancel selected query", false, "Enter", Some(BLUE))).clicked() || (confirm && !busy) {
                                cancel = self.query_monitor.selected.and_then(|index| self.query_monitor.rows.get(index)).cloned();
                            }
                        });
                    });
                });
            });
        self.query_monitor.open = open;
        #[cfg(not(target_arch = "wasm32"))]
        if refresh || cancel.is_some() { self.refresh_query_monitor(ctx, cancel); }
        #[cfg(target_arch = "wasm32")]
        let _ = (refresh, cancel);
        if busy { ctx.request_repaint_after(std::time::Duration::from_millis(100)); }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn monitor_request(control: db::PostgresControl, cancel: Option<RunningQuery>) -> Result<(Vec<RunningQuery>, Option<String>), String> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|error| error.to_string())?;
    let mut config = db::postgres::async_client_config(&control.config, control.forwarded_port);
    config.application_name("Rustable query monitor").options("-c statement_timeout=5000 -c lock_timeout=1000");
    runtime.block_on(async {
        if control.config.tls {
            config.ssl_mode(tokio_postgres::config::SslMode::Require);
            let tls = postgres_native_tls::MakeTlsConnector::new(native_tls::TlsConnector::new().map_err(|error| error.to_string())?);
            monitor_connect(&config, tls, cancel).await
        } else {
            config.ssl_mode(tokio_postgres::config::SslMode::Disable);
            monitor_connect(&config, tokio_postgres::NoTls, cancel).await
        }
    })
}

#[cfg(not(target_arch = "wasm32"))]
async fn monitor_connect<T>(config: &tokio_postgres::Config, tls: T, cancel: Option<RunningQuery>) -> Result<(Vec<RunningQuery>, Option<String>), String>
where T: tokio_postgres::tls::MakeTlsConnect<tokio_postgres::Socket>,
    T::Stream: Send + 'static, T::TlsConnect: Send,
    <T::TlsConnect as tokio_postgres::tls::TlsConnect<tokio_postgres::Socket>>::Future: Send,
{
    use std::time::Duration;
    let (client, connection) = tokio::time::timeout(Duration::from_secs(8), config.connect(tls)).await
        .map_err(|_| "Could not reach the database to inspect queries. The original query may still be running.".to_owned())?
        .map_err(|error| error.to_string())?;
    let task = tokio::spawn(connection);
    let result = tokio::time::timeout(Duration::from_secs(6), async {
        let mut message = None;
        if let Some(query) = cancel {
            let row = client.query_opt(
                "SELECT pg_cancel_backend(pid) FROM pg_stat_activity WHERE pid=$1 AND query_start::text=$2 AND backend_start::text=$3 AND query=$4 AND state='active' AND datname=current_database() AND pid<>pg_backend_pid()",
                &[&query.pid, &query.started, &query.backend_started, &query.sql]).await.map_err(|error| error.to_string())?;
            message = Some(match row {
                Some(row) if row.get::<_, bool>(0) => "Cancellation signal sent. Refresh to check whether the query has stopped.",
                Some(_) => "The server did not accept cancellation. Refresh the list and check permissions.",
                None => "That query has already ended or changed. No cancellation was sent.",
            }.into());
        }
        let rows = client.query(
            "SELECT pid, query_start::text, backend_start::text, COALESCE(usename,''), (clock_timestamp()-query_start)::text, COALESCE(wait_event_type || ': ' || wait_event,''), query FROM pg_stat_activity WHERE datname=current_database() AND state='active' AND pid<>pg_backend_pid() ORDER BY query_start LIMIT 200", &[])
            .await.map_err(|error| error.to_string())?;
        let rows = rows.into_iter().map(|row| RunningQuery { pid: row.get(0), started: row.get(1), backend_started: row.get(2), user: row.get(3), elapsed: row.get(4), wait: row.get(5), sql: row.get(6) }).collect();
        Ok((rows, message))
    }).await.map_err(|_| "Query monitor did not respond. The original query may still be running; no completion was assumed.".to_owned()).and_then(|result| result);
    task.abort();
    let _ = task.await;
    result
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    #[test]
    fn selection_tracks_query_identity_and_prefers_this_workspace() {
        let query = |pid, started: &str| RunningQuery { pid, started: started.into(), backend_started: "session".into(), user: String::new(), elapsed: String::new(), wait: String::new(), sql: "SELECT pg_sleep(10)".into() };
        let mut monitor = QueryMonitor::default();
        monitor.replace_rows(vec![query(1, "a"), query(2, "b")], Some(2));
        assert_eq!(monitor.selected, Some(1));
        monitor.selected = Some(0);
        monitor.replace_rows(vec![query(2, "b"), query(1, "a")], Some(2));
        assert_eq!(monitor.selected, Some(1), "keep explicitly selected query after sorting");
        monitor.replace_rows(vec![query(1, "new query")], Some(2));
        assert_eq!(monitor.selected, None, "never retarget cancellation to a replacement query");
        monitor.replace_rows(Vec::new(), Some(2));
        assert_eq!(monitor.selected, None);
    }

    #[test]
    #[ignore = "requires opted-in disposable PostgreSQL on port 54329"]
    fn monitor_lists_and_cancels_only_selected_query() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let config = crate::connection::PostgresConfig {
            host: "127.0.0.1".into(), port: 54329, database: "rustable_qa".into(),
            user: "postgres".into(), password: "rustable_qa_only".into(), tls: false, ssh: None,
        };
        let control = db::PostgresControl { process_id: 0, config: config.clone(), forwarded_port: Some(54329) };
        let mut client = ::postgres::Client::connect("host=127.0.0.1 port=54329 user=postgres password=rustable_qa_only dbname=rustable_qa", ::postgres::NoTls).unwrap();
        let pid: i32 = client.query_one("SELECT pg_backend_pid()", &[]).unwrap().get(0);
        assert_eq!(client.cancel_token().process_id(), pid, "startup metadata must identify the same backend");
        let thread = std::thread::spawn(move || client.batch_execute("SELECT pg_sleep(15) /* rustable monitor regression */"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let query = loop {
            let (rows, _) = monitor_request(control.clone(), None).unwrap();
            if let Some(row) = rows.into_iter().find(|row| row.pid == pid && row.sql.contains("monitor regression")) { break row; }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        let mut stale = query.clone();
        stale.sql.push_str(" stale");
        let (rows, message) = monitor_request(control.clone(), Some(stale)).unwrap();
        assert!(message.unwrap().contains("No cancellation"));
        assert!(rows.iter().any(|row| row.pid == pid));
        let (_, message) = monitor_request(control, Some(query)).unwrap();
        assert!(message.unwrap().contains("signal sent"));
        let error = thread.join().unwrap().unwrap_err();
        assert_eq!(error.code(), Some(&tokio_postgres::error::SqlState::QUERY_CANCELED));
    }
}
