//! User confirmation is separate from the driver's enforced read-only mode.
use super::*;
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Mode {
    #[default]
    Silent,
    AlertAll,
    AlertWrites,
    PasswordAll,
    PasswordWrites,
}
impl Mode {
    const ALL: [Self; 5] = [
        Self::Silent,
        Self::AlertAll,
        Self::AlertWrites,
        Self::PasswordAll,
        Self::PasswordWrites,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Silent => "Silent Mode",
            Self::AlertAll => "Alert Mode 1",
            Self::AlertWrites => "Alert Mode 2",
            Self::PasswordAll => "Safe Mode 1",
            Self::PasswordWrites => "Safe Mode 2",
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::Silent => "Send without warnings",
            Self::AlertAll => "Warn before every query",
            Self::AlertWrites => "Warn except for SELECT / EXPLAIN / SHOW",
            Self::PasswordAll => "Authenticate before every query",
            Self::PasswordWrites => "Authenticate except for SELECT / EXPLAIN / SHOW",
        }
    }
    fn password(self) -> bool {
        matches!(self, Self::PasswordAll | Self::PasswordWrites)
    }
    fn requires(self, command: &Command, dialect: crate::sql::Dialect) -> bool {
        match command {
            Command::Query { sql, safe } => {
                !safe
                    && match self {
                        Self::Silent => false,
                        Self::AlertAll | Self::PasswordAll => true,
                        Self::AlertWrites | Self::PasswordWrites => !exempt(sql, dialect),
                    }
            }
            Command::Apply(_) | Command::ImportCsv { .. } => self != Self::Silent,
            // Internal catalog reads, browse and planning are not user execution.
            _ => false,
        }
    }
}

/// Conservative exemption across the entire script, not only its first word.
/// This is a confirmation preference, not a database permission boundary.
fn exempt(sql: &str, dialect: crate::sql::Dialect) -> bool {
    use sqlparser::tokenizer::{Token, Tokenizer};
    let pg = sqlparser::dialect::PostgreSqlDialect {};
    let sqlite = sqlparser::dialect::SQLiteDialect {};
    let mysql = crate::sql::mysql::Lexer(match dialect {
        crate::sql::Dialect::Mysql(m) => m,
        _ => Default::default(),
    });
    let lexer: &dyn sqlparser::dialect::Dialect = match dialect {
        crate::sql::Dialect::Postgres => &pg,
        crate::sql::Dialect::Sqlite => &sqlite,
        _ => &mysql,
    };
    if let crate::sql::Dialect::Mysql(mode) = dialect
        && crate::sql::mysql::executable_comments(sql, mode)
            .map_or(true, |comments| !comments.is_empty())
    {
        return false;
    }
    if sqlparser::parser::Parser::parse_sql(lexer, sql).is_err() {
        return false;
    }
    let Ok(tokens) = Tokenizer::new(lexer, sql).tokenize() else {
        return false;
    };
    let mut start = true;
    let mut seen = false;
    let mut show = false;
    for token in tokens {
        match token {
            Token::Whitespace(_) => {}
            Token::SemiColon => {
                start = true;
                show = false;
            }
            Token::Word(word) if word.quote_style.is_none() => {
                let word = word.value.to_ascii_uppercase();
                if start && !matches!(word.as_str(), "SELECT" | "SHOW" | "EXPLAIN") {
                    return false;
                }
                if start {
                    show = word == "SHOW";
                }
                if !show
                    && matches!(
                        word.as_str(),
                        "INSERT"
                            | "UPDATE"
                            | "DELETE"
                            | "MERGE"
                            | "INTO"
                            | "CALL"
                            | "COPY"
                            | "ALTER"
                            | "DROP"
                            | "CREATE"
                            | "TRUNCATE"
                    )
                {
                    return false;
                }
                start = false;
                seen = true;
            }
            _ if start => return false,
            _ => {}
        }
    }
    seen
}

#[derive(Default)]
pub(super) struct Protection {
    pub mode: Mode,
    password: Zeroizing<String>,
    connecting_password: Zeroizing<String>,
    pending: Option<(Command, Pending)>,
    query_run: Option<DestructiveRun>,
    entered: Zeroizing<String>,
    error: Option<String>,
    preview: String,
    confirmation_armed: bool,
    #[cfg(target_os = "macos")]
    biometric: Option<Biometric>,
}
impl Protection {
    pub(super) fn defer_query(&mut self, run: DestructiveRun) {
        self.query_run = Some(run);
    }
    #[cfg(unix)]
    pub(super) fn query_tab(&self) -> Option<u64> {
        self.query_run.as_ref().map(|run| run.tab_id)
    }
    pub fn modal_open(&self) -> bool {
        self.pending.is_some()
    }
    pub fn connecting(&mut self, command: &Command) {
        if command.replaces_connection() {
            self.connecting_password = Zeroizing::new(match command {
                Command::OpenPostgres(config) => config.password.clone(),
                Command::OpenMysql(config) => config.password.clone(),
                _ => String::new(),
            });
        }
    }
    pub fn connection_failed(&mut self) {
        self.connecting_password = Zeroizing::default();
    }
    fn password_matches(&self) -> bool {
        !self.password.is_empty() && *self.entered == *self.password
    }
    pub fn connected(&mut self, production: bool) {
        self.password = std::mem::take(&mut self.connecting_password);
        self.mode = if production {
            Mode::AlertWrites
        } else {
            Mode::Silent
        };
        self.clear();
    }
    pub(super) fn clear(&mut self) {
        self.pending = None;
        self.query_run = None;
        self.entered = Zeroizing::default();
        self.preview.clear();
        self.confirmation_armed = false;
        self.error = None;
        #[cfg(target_os = "macos")]
        {
            self.biometric = None;
        }
    }
}
impl Rustable {
    pub(super) fn protect_command(
        &mut self,
        command: Command,
        pending: Pending,
    ) -> Option<(Command, Pending)> {
        self.protect_command_with_preview(command, pending, false)
    }

    pub(super) fn protect_command_with_preview(
        &mut self,
        command: Command,
        pending: Pending,
        preview: bool,
    ) -> Option<(Command, Pending)> {
        if self.protection.modal_open() {
            return None;
        }
        let dialect = self
            .session_dialect
            .unwrap_or_else(|| crate::sql::Dialect::for_engine(&self.engine));
        if !preview && !self.protection.mode.requires(&command, dialect) {
            return Some((command, pending));
        }
        self.protection.preview = match &command {
            Command::Query { sql, .. } => sql.clone(),
            Command::Apply(plan) => plan
                .statements
                .iter()
                .map(|s| crate::review_sql::readable_dialect(&s.sql, &s.parameters, dialect))
                .collect::<Vec<_>>()
                .join("\n\n"),
            Command::ImportCsv { request, .. } => {
                let sql = match dialect {
                    crate::sql::Dialect::Mysql(_) => request.mysql_sql(),
                    crate::sql::Dialect::Postgres => request.postgres_sql(),
                    crate::sql::Dialect::Sqlite => request.sql(),
                };
                let sql = match sql {
                    Ok(sql) => sql,
                    Err(error) => {
                        self.error = Some(error);
                        return None;
                    }
                };
                format!("-- Import every source row using the reviewed mapping.\n{sql}")
            }
            _ => unreachable!(),
        };
        if let Some((end, _)) = self.protection.preview.char_indices().nth(32_768) {
            self.protection.preview.truncate(end);
            self.protection
                .preview
                .push_str("\n… Preview shortened; the complete reviewed operation will be sent.");
        }
        self.protection.confirmation_armed = false;
        self.protection.pending = Some((command, pending));
        self.protection.entered = Zeroizing::default();
        self.protection.error = None;
        None
    }

    pub(super) fn protection_menu(&mut self, ui: &mut egui::Ui, width: Option<f32>) {
        let active = self.safe || self.protection.mode != Mode::Silent;
        let response = ui
            .add_enabled_ui(self.has_connection() && self.pending.is_none(), |ui| {
                crate::icons::button(
                    ui,
                    if active { "lock" } else { "lock-open" },
                    egui::vec2(
                        width.unwrap_or(crate::icons::TOOLBAR_BUTTON.x),
                        crate::icons::TOOLBAR_BUTTON.y,
                    ),
                    active,
                )
            })
            .inner
            .on_hover_text(format!(
                "Query protection · {}",
                self.protection.mode.label()
            ));
        let popup = egui::Popup::menu(&response);
        #[cfg(feature = "qa")]
        let popup = if std::env::var("RUSTABLE_PROTECTION_CAPTURE").as_deref() == Ok("menu") {
            popup.open(true)
        } else {
            popup
        };
        popup.show(|ui| {
            ui.set_width((ui.ctx().content_rect().width() - 32.0).clamp(270.0, 650.0));
            self.protection_options(ui);
        });
    }
    fn protection_options(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 2.0;
        for mode in Mode::ALL {
            if matches!(mode, Mode::AlertAll | Mode::PasswordAll) {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
            }
            let available = !mode.password()
                || !self.protection.password.is_empty()
                || (cfg!(target_os = "macos") && self.prefs.touch_id);
            let selected = self.protection.mode == mode && !self.safe;
            let label = format!("{} — {}", mode.label(), mode.description());
            let response = ui
                .add_enabled_ui(available, |ui| {
                    let color = ui.visuals().text_color();
                    let galley = ui.painter().layout(
                        label.clone(),
                        egui::FontId::proportional(13.0),
                        color,
                        (ui.available_width() - 62.0).max(160.0),
                    );
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), (galley.size().y + 16.0).max(34.0)),
                        egui::Sense::click(),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::SelectableLabel,
                            ui.is_enabled(),
                            selected,
                            &label,
                        )
                    });
                    if response.hovered() || response.has_focus() {
                        ui.painter().rect_filled(
                            rect,
                            5.0,
                            ui.visuals().widgets.hovered.weak_bg_fill,
                        );
                    }
                    if selected {
                        crate::icons::paint(
                            ui.painter(),
                            egui::pos2(rect.left() + 12.0, rect.center().y),
                            "check",
                            15.0,
                            color,
                        );
                    }
                    let icon = egui::pos2(rect.left() + 36.0, rect.center().y);
                    if mode == Mode::Silent {
                        crate::icons::paint(ui.painter(), icon, "lock-open", 17.0, color);
                    } else {
                        let accent = if mode.password() {
                            ui.visuals().error_fg_color
                        } else {
                            ui.visuals().warn_fg_color
                        };
                        if matches!(mode, Mode::AlertAll | Mode::PasswordAll) {
                            ui.painter().circle_filled(icon, 6.0, accent);
                        } else {
                            ui.painter().circle_stroke(
                                icon,
                                6.0,
                                egui::Stroke::new(1.6_f32, accent),
                            );
                        }
                    }
                    ui.painter().galley(
                        egui::pos2(rect.left() + 54.0, rect.center().y - galley.size().y / 2.0),
                        galley,
                        color,
                    );
                    response
                })
                .inner;
            if response
                .on_disabled_hover_text(
                    "Requires a database password or Touch ID enabled in Settings.",
                )
                .clicked()
            {
                self.protection.mode = mode;
                self.safe = false;
                self.status = format!("{}: {}", mode.label(), mode.description());
                ui.close();
            }
        }
    }

    #[cfg(test)]
    pub(super) fn confirm_changes_for_test(&mut self) {
        if !self.protection.modal_open() {
            self.save_changes(false);
            return;
        }
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| self.protection_confirmation(ctx));
        let _ = ctx.run(egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Enter, physical_key: None, pressed: true,
                repeat: false, modifiers: egui::Modifiers::COMMAND,
            }], ..Default::default()
        }, |ctx| self.protection_confirmation(ctx));
    }

    #[cfg(test)]
    pub(super) fn protected_plan_for_test(&self) -> &ChangePlan {
        match &self.protection.pending {
            Some((Command::Apply(plan), _)) => plan,
            _ => panic!("expected a pending apply confirmation"),
        }
    }

    pub(super) fn protection_confirmation(&mut self, ctx: &egui::Context) {
        if !self.protection.modal_open() {
            return;
        }
        let name = self.active_connection_name().to_owned();
        let password = self.protection.mode.password();
        let can_send = !self.safe || !matches!(self.protection.pending, Some((Command::Apply(_), _)));
        let submit_shortcut = self.protection.confirmation_armed && ctx.input(|input| {
            input.events.iter().any(|event| matches!(event,
                egui::Event::Key { key: egui::Key::Enter, pressed: true, repeat: false, modifiers, .. }
                    if modifiers.command && !modifiers.shift && !modifiers.alt
            ))
        });
        self.protection.confirmation_armed = true;
        let mut send = false;
        let mut cancel = false;
        #[cfg(target_os = "macos")]
        if let Some(auth) = &self.protection.biometric
            && let Ok(success) = auth.result.try_recv()
        {
            send = success;
            if !success {
                self.protection.error = Some("Touch ID did not authorize this request. Try again or enter your database password.".into());
            }
            self.protection.biometric = None;
        }
        let modal = egui::Modal::new(egui::Id::new((ctx.viewport_id(), "query-protection")))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .inner_margin(20.0)
                    .corner_radius(10),
            )
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 88.0).clamp(260.0, 640.0));
                ui.label(
                    RichText::new(if password {
                        "Safe Mode is enabled"
                    } else {
                        "Confirm query execution"
                    })
                    .size(18.0)
                    .strong(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{name} · {}", self.engine))
                        .size(12.0)
                        .weak(),
                );
                ui.add_space(12.0);
                ui.label("Send these statements to the database?");
                ui.add_space(8.0);
                egui::Frame::new()
                    .fill(ui.visuals().extreme_bg_color)
                    .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                    .inner_margin(12.0)
                    .corner_radius(7)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("SQL preview").size(11.0).weak());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if clipboard_button(ui)
                                        .on_hover_text("Copy SQL preview")
                                        .clicked()
                                    {
                                        ui.ctx().copy_text(self.protection.preview.clone());
                                    }
                                },
                            );
                        });
                        ui.add_space(4.0);
                        egui::ScrollArea::both()
                            .max_height((ctx.content_rect().height() - 400.0).clamp(90.0, 280.0))
                            .show(ui, |ui| {
                                ui.set_min_height(90.0);
                                ui.add(
                                    egui::Label::new(crate::editor::layout(
                                        ui,
                                        &self.protection.preview.as_str(),
                                        ui.available_width(),
                                    ))
                                    .selectable(true),
                                );
                            });
                    });
                if password {
                    ui.add_space(14.0);
                    ui.label(RichText::new("Database password").size(12.0).strong());
                    ui.add_space(4.0);
                    egui::Frame::new()
                        .fill(ui.visuals().extreme_bg_color)
                        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                        .inner_margin(egui::Margin::symmetric(9, 7))
                        .corner_radius(5)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut *self.protection.entered)
                                    .password(true)
                                    .frame(false)
                                    .hint_text("Enter your database password")
                                    .desired_width(f32::INFINITY),
                            );
                        });
                }
                if let Some(error) = &self.protection.error {
                    ui.add_space(8.0);
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(10.0);
                rounded_action_buttons(ui);
                ui.horizontal(|ui| {
                    #[cfg(target_os = "macos")]
                    if password
                        && self.prefs.touch_id
                        && ui
                            .add_enabled(
                                self.protection.biometric.is_none(),
                                egui::Button::new(if self.protection.biometric.is_some() {
                                    "Waiting for Touch ID…"
                                } else {
                                    "Use Touch ID"
                                })
                                .min_size(egui::vec2(0.0, crate::controls::BUTTON_HEIGHT)),
                            )
                            .clicked()
                    {
                        match Biometric::start(ctx.clone()) {
                            Ok(auth) => self.protection.biometric = Some(auth),
                            Err(error) => self.protection.error = Some(error),
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let button = crate::icons::primary_shortcut_button(
                            ui, "Send", false, "Enter",
                        );
                        if ui
                            .add_enabled(can_send && (!password || !self.protection.entered.is_empty()), button)
                            .clicked() || (can_send && submit_shortcut && (!password || !self.protection.entered.is_empty()))
                        {
                            if !password || self.protection.password_matches() {
                                send = true;
                            } else {
                                self.protection.error =
                                    Some("Incorrect database password. Nothing was sent.".into());
                                self.protection.entered = Zeroizing::default();
                            }
                        }
                        cancel = ui
                            .add(
                                egui::Button::new("Cancel")
                                    .min_size(egui::vec2(70.0, crate::controls::BUTTON_HEIGHT)),
                            )
                            .clicked();
                    });
                });
            });
        if cancel || modal.should_close() {
            self.protection.clear();
        } else if send && can_send {
            let command = self.protection.pending.take();
            let run = self.protection.query_run.take();
            self.protection.clear();
            if let Some(run) = run {
                self.execute_query_authorized(run);
            } else if let Some((command, pending)) = command {
                self.dispatch_authorized(command, pending);
            }
        }
    }
}

#[cfg(target_os = "macos")]
struct Biometric {
    context: objc2::rc::Retained<objc2_local_authentication::LAContext>,
    result: std::sync::mpsc::Receiver<bool>,
}
#[cfg(target_os = "macos")]
impl Biometric {
    fn start(ctx: egui::Context) -> Result<Self, String> {
        use objc2_foundation::{NSError, NSString};
        use objc2_local_authentication::{LAContext, LAPolicy};
        // SAFETY: fresh context per operation; no authentication is cached or
        // reused. The callback sends only a boolean and requests a repaint.
        unsafe {
            let context = LAContext::new();
            let policy = LAPolicy::DeviceOwnerAuthenticationWithBiometrics;
            context.canEvaluatePolicy_error(policy).map_err(|_| "Touch ID is unavailable. Use your database password or configure Touch ID in macOS Settings.".to_owned())?;
            context.setLocalizedFallbackTitle(Some(&NSString::from_str("")));
            context.setTouchIDAuthenticationAllowableReuseDuration(0.0);
            let (send, result) = std::sync::mpsc::channel();
            let viewport = ctx.viewport_id();
            let reply =
                block2::RcBlock::new(move |success: objc2::runtime::Bool, _error: *mut NSError| {
                    let _ = send.send(success.as_bool());
                    ctx.request_repaint_of(viewport);
                });
            context.evaluatePolicy_localizedReason_reply(
                policy,
                &NSString::from_str("authorize sending the reviewed statements to your database"),
                &reply,
            );
            Ok(Self { context, result })
        }
    }
}
#[cfg(target_os = "macos")]
impl Drop for Biometric {
    fn drop(&mut self) {
        unsafe {
            self.context.invalidate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settle(app: &mut Rustable) {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while app.pending.is_some() {
            assert!(Instant::now() < deadline);
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    fn fixture() -> Rustable {
        let mut app = Rustable::with_preferences(&egui::Context::default(), Preferences::default());
        settle(&mut app);
        app.connections = false;
        app.view = View::Query;
        app
    }
    fn query(app: &mut Rustable, sql: &str) {
        app.tabs[0].sql = sql.into();
        app.tabs[0].cursor = 0..0;
        app.run_query(true);
    }
    #[test]
    fn exemptions_cover_every_statement_and_dialect() {
        for dialect in [
            crate::sql::Dialect::Sqlite,
            crate::sql::Dialect::Postgres,
            crate::sql::Dialect::Mysql(Default::default()),
        ] {
            for sql in [
                "SELECT 1",
                "/* delete */ SELECT 'UPDATE x'; SELECT 2",
                "EXPLAIN SELECT 1",
            ] {
                assert!(exempt(sql, dialect), "{sql}");
            }
            for sql in [
                "SELECT 1; DELETE FROM t WHERE id=1",
                "WITH x AS (DELETE FROM t RETURNING *) SELECT * FROM x",
                "EXPLAIN ANALYZE DELETE FROM t WHERE id=1",
                "SELECT * INTO new_table FROM t",
                "SELECT (",
                "CALL mutate()",
                "",
            ] {
                assert!(!exempt(sql, dialect), "{sql}");
            }
        }
        assert!(!exempt(
            "/*! DELETE FROM t */ SELECT 1",
            crate::sql::Dialect::Mysql(Default::default())
        ));
        assert!(exempt(
            "SHOW CREATE TABLE t",
            crate::sql::Dialect::Mysql(Default::default())
        ));
        assert!(exempt(
            "SHOW TABLES",
            crate::sql::Dialect::Mysql(Default::default())
        ));
    }
    #[test]
    fn modes_gate_all_writes_and_password_modes_do_not_exempt_mixed_scripts() {
        for mode in Mode::ALL {
            for (sql, read) in [
                ("SELECT 1", true),
                ("UPDATE t SET n=2 WHERE id=1", false),
                ("SELECT 1; DELETE FROM t", false),
            ] {
                let needs = mode.requires(
                    &Command::Query {
                        sql: sql.into(),
                        safe: false,
                    },
                    crate::sql::Dialect::Sqlite,
                );
                assert_eq!(
                    needs,
                    mode != Mode::Silent
                        && (!read || matches!(mode, Mode::AlertAll | Mode::PasswordAll))
                );
            }
            assert_eq!(
                mode.requires(
                    &Command::Apply(Default::default()),
                    crate::sql::Dialect::Sqlite
                ),
                mode != Mode::Silent
            );
        }
    }
    #[test]
    fn confirmation_cancel_retains_edits_and_no_history_or_write_is_sent() {
        let mut app = fixture();
        query(
            &mut app,
            "CREATE TABLE protection_probe(id INTEGER PRIMARY KEY); INSERT INTO protection_probe VALUES(1)",
        );
        settle(&mut app);
        app.protection.mode = Mode::AlertWrites;
        let history = app.prefs.history.clone();
        let executed = app.tabs[0].executed_sql.clone();
        query(&mut app, "DELETE FROM protection_probe WHERE id=1");
        assert!(app.protection.modal_open());
        assert!(app.pending.is_none());
        assert_eq!(app.prefs.history, history);
        assert_eq!(app.tabs[0].executed_sql, executed);
        let ctx = egui::Context::default();
        for shift in [false, true] {
            let modifiers = egui::Modifiers {
                shift,
                ..egui::Modifiers::COMMAND
            };
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: true,
                        repeat: true,
                        modifiers,
                    }],
                    modifiers,
                    ..Default::default()
                },
                |ctx| app.workspace_shortcuts(ctx),
            );
            assert!(app.pending.is_none());
            assert!(app.protection.modal_open());
        }
        app.protection.clear();
        query(&mut app, "SELECT count(*) FROM protection_probe WHERE id=1");
        settle(&mut app);
        assert_eq!(app.tabs[0].grid.rows[0][0], Value::Integer(1));
        query(&mut app, "DELETE FROM protection_probe WHERE id=1");
        let (command, pending) = app.protection.pending.take().unwrap();
        app.protection.clear();
        app.dispatch_authorized(command, pending);
        settle(&mut app);
        query(&mut app, "SELECT count(*) FROM protection_probe WHERE id=1");
        settle(&mut app);
        assert_eq!(app.tabs[0].grid.rows[0][0], Value::Integer(0));
    }
    #[test]
    fn silent_runs_and_internal_read_only_still_rejects_writes() {
        let mut app = fixture();
        query(&mut app, "DELETE FROM customers WHERE id=1");
        assert!(app.pending.is_some());
        assert!(!app.protection.modal_open());
        settle(&mut app);
        app.safe = true;
        app.protection.mode = Mode::PasswordAll;
        query(&mut app, "DROP TABLE customers");
        settle(&mut app);
        assert!(app.tabs[0].result_error.is_some());
        assert!(!app.protection.modal_open());
    }
    #[test]
    fn reconnect_resets_mode_and_credentials_and_password_is_not_persisted() {
        let mut protection = Protection::default();
        protection.connecting(&Command::OpenPostgres(crate::connection::PostgresConfig {
            password: "fixture-only".into(),
            ..Default::default()
        }));
        protection.connected(true);
        assert_eq!(protection.mode, Mode::AlertWrites);
        assert_eq!(&**protection.password, "fixture-only");
        protection.mode = Mode::PasswordAll;
        protection.connecting(&Command::Open(None));
        protection.connected(false);
        assert_eq!(protection.mode, Mode::Silent);
        assert!(protection.password.is_empty());
        let mut app = fixture();
        app.protection.password = Zeroizing::new("fixture-secret".into());
        assert!(
            !serde_json::to_string(&app.prefs)
                .unwrap()
                .contains("fixture-secret")
        );
    }
    #[test]
    fn protection_modes_allow_local_row_edits() {
        let mut app = fixture();
        app.view = View::Data;
        app.browse.table = "customers".into();
        app.columns = app
            .objects
            .iter()
            .find(|o| o.name == "customers")
            .unwrap()
            .columns
            .clone();
        for mode in Mode::ALL {
            app.protection.mode = mode;
            assert!(app.can_edit_rows(), "{mode:?}");
        }
    }
    #[test]
    fn saves_and_explicit_previews_share_one_confirmation() {
        for mode in [Mode::Silent, Mode::AlertWrites] {
            for preview in [false, true] {
                let mut app = fixture();
                query(&mut app, "CREATE TABLE save_probe(id INTEGER PRIMARY KEY)");
                settle(&mut app);
                app.protection.mode = mode;
                app.changes.push(Change::InsertRow {
                    id: 999, table: "save_probe".into(),
                    fields: vec![("id".into(), Value::Integer(7))],
                });
                if preview { app.preview_changes(); } else { app.save_changes(false); }
                settle(&mut app);
                assert!(app.review.is_none());
                let prompts = preview || mode != Mode::Silent;
                assert_eq!(app.protection.modal_open(), prompts);
                assert_eq!(app.changes.len(), usize::from(prompts));
                if prompts {
                    assert!(app.protection.preview.contains("INSERT"));
                    app.confirm_changes_for_test();
                    settle(&mut app);
                }
                assert!(app.error.is_none(), "{:?}", app.error);
                assert!(app.changes.is_empty());
                assert!(!app.protection.modal_open());
                query(&mut app, "SELECT id FROM save_probe");
                settle(&mut app);
                assert_eq!(app.tabs[0].grid.rows[0][0], Value::Integer(7));
            }
        }
    }

    #[test]
    fn confirmation_shortcut_requires_a_fresh_press_and_valid_password() {
        let mut app = fixture();
        app.protection.mode = Mode::PasswordAll;
        app.protection.password = Zeroizing::new("secret".into());
        query(&mut app, "SELECT 123");
        let ctx = egui::Context::default();
        let frame = |app: &mut Rustable, repeat: bool, shift: bool| {
            if !repeat {
                let _ = ctx.run(egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::Enter, physical_key: None, pressed: false,
                        repeat: false, modifiers: egui::Modifiers::COMMAND,
                    }], ..Default::default()
                }, |_| {});
            }
            let _ = ctx.run(egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Enter, physical_key: None, pressed: true, repeat,
                    modifiers: egui::Modifiers { shift, ..egui::Modifiers::COMMAND },
                }], ..Default::default()
            }, |ctx| app.protection_confirmation(ctx));
        };
        app.protection.entered = Zeroizing::new("secret".into());
        frame(&mut app, false, false); // Opening chord cannot approve.
        assert!(app.protection.modal_open());
        frame(&mut app, true, false);
        assert!(app.protection.modal_open());
        frame(&mut app, false, true);
        assert!(app.protection.modal_open());
        app.protection.entered = Zeroizing::new("wrong".into());
        frame(&mut app, false, false);
        assert!(app.protection.modal_open());
        assert!(app.protection.error.is_some());
        app.protection.entered = Zeroizing::new("secret".into());
        frame(&mut app, false, false);
        assert!(!app.protection.modal_open());
        settle(&mut app);
        assert_eq!(app.tabs[0].grid.rows[0][0], Value::Integer(123));
    }

    #[test]
    fn settings_shortcut_and_shared_security_preference() {
        let ctx = egui::Context::default();
        let mut app = fixture();
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Comma,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }],
                modifiers: egui::Modifiers::COMMAND,
                ..Default::default()
            },
            |ctx| app.settings_shortcut(ctx),
        );
        assert!(app.settings_open);
        app.prefs.touch_id = true;
        app.shared_preferences_dirty = true;
        app.publish_shared_preferences();
        let mut sibling = fixture();
        app.sync_window_preferences(&mut sibling);
        assert!(sibling.prefs.touch_id);
    }
}

#[cfg(feature = "qa")]
impl Rustable {
    pub(super) fn prepare_protection_preview(&mut self) {
        match std::env::var("RUSTABLE_PROTECTION_CAPTURE").as_deref() {
            Ok("settings") => {
                self.settings_open = true;
            }
            Ok("password") | Ok("alert") => {
                self.protection.password = Zeroizing::new("disposable-preview".into());
                self.prefs.touch_id = true;
                self.protection.mode =
                    if std::env::var("RUSTABLE_PROTECTION_CAPTURE").as_deref() == Ok("password") {
                        Mode::PasswordWrites
                    } else {
                        Mode::AlertWrites
                    };
                let _ = self.protect_command(Command::Query { sql: "DELETE FROM \"public\".\"address\" WHERE (\"id\" = 8) OR (\"id\" = 9) OR (\"id\" = 10) OR (\"id\" = 11);".into(), safe: false }, Pending::Query(0));
            }
            Ok("menu") => {
                self.protection.password = Zeroizing::new("disposable-preview".into());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod rendering_tests {
    use super::*;
    #[test]
    fn send_uses_theme_foreground_in_all_appearances() {
        fn send_color(shape: &egui::Shape) -> Option<Color32> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == "Send" => Some(text.galley.job.sections[0].format.color),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(send_color),
                _ => None,
            }
        }
        for appearance in [Appearance::Light, Appearance::Dark, Appearance::SolarizedLight, Appearance::SolarizedDark] {
            let ctx = egui::Context::default();
            let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
            apply_theme(&ctx, appearance);
            app.protection.mode = Mode::AlertAll;
            assert!(app.protect_command(Command::Query { sql: "BEGIN;".into(), safe: false }, Pending::Query(0)).is_none());
            for frame in 0..3 {
                let output = ctx.run(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(760.0, 760.0))),
                    ..Default::default()
                }, |ctx| app.protection_confirmation(ctx));
                let color = output.shapes.iter().find_map(|shape| send_color(&shape.shape));
                if frame == 2 {
                    assert_eq!(color, Some(ctx.style().visuals.selection.stroke.color));
                }
            }
        }
    }
    #[test]
    fn password_validation_is_exact_and_clears_on_cancellation() {
        let mut protection = Protection::default();
        assert!(!protection.password_matches());
        protection.password = Zeroizing::new(" exact 東京 ".into());
        for value in ["", "exact 東京", " exact 東京", "wrong"] {
            protection.entered = Zeroizing::new(value.into());
            assert!(!protection.password_matches());
        }
        protection.entered = Zeroizing::new(" exact 東京 ".into());
        assert!(protection.password_matches());
        protection.clear();
        assert!(protection.entered.is_empty());
        assert!(!protection.password_matches());
        protection.connecting_password = Zeroizing::new("failed-connection".into());
        protection.connection_failed();
        assert!(protection.connecting_password.is_empty());
        assert_eq!(&**protection.password, " exact 東京 ");
    }
    #[test]
    #[ignore = "manual release query-protection interaction CPU benchmark"]
    fn protection_interaction_frame_benchmark() {
        for view in ["settings", "menu", "password", "password_large"] {
            let ctx = egui::Context::default();
            let mut app =
                Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
            app.engine = "PostgreSQL".into();
            app.label = "Disposable preview".into();
            app.connections = false;
            app.protection.password = Zeroizing::new("fixture-password".into());
            app.prefs.touch_id = true;
            if view == "settings" {
                app.settings_open = true;
            }
            if view.starts_with("password") {
                app.protection.mode = Mode::PasswordWrites;
                let _ = app.protect_command(
                    Command::Query {
                        sql: "DELETE FROM address WHERE (id=8) OR (id=9);"
                            .repeat(if view == "password_large" { 2000 } else { 1 }),
                        safe: false,
                    },
                    Pending::Query(0),
                );
            }
            let mut samples = Vec::new();
            for index in 0..180 {
                if view.starts_with("password") {
                    app.protection.entered = Zeroizing::new("x".repeat(index % 24));
                }
                let start = std::time::Instant::now();
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(760.0, 760.0),
                        )),
                        time: Some(index as f64 / 120.0 + if index >= 100 { 30.0 } else { 0.0 }),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            if view == "menu" {
                                app.protection_options(ui);
                            }
                        });
                        app.settings_window(ctx);
                        app.protection_confirmation(ctx);
                    },
                );
                let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
                if index >= 20 {
                    samples.push(start.elapsed().as_secs_f64() * 1000.0);
                }
            }
            samples.sort_by(f64::total_cmp);
            crate::ui_benchmark::report(&format!("protection/{view}"), &samples);
        }
    }
}
