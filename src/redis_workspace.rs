//! Redis-native UI; network and decoding work remain on a dedicated worker.
use crate::{
    redis_session::{KeyPage, KeyRow, Session, Snapshot},
    redis_settings::Settings,
};
use eframe::egui;
use std::sync::mpsc;

enum Job {
    Scan(u64, Vec<u8>),
    Inspect(Vec<u8>),
    Execute(String, bool),
    Save(Vec<u8>, Vec<u8>, Vec<u8>, bool),
    Delete(Vec<u8>, bool),
    Expire(Vec<u8>, Option<u64>, bool),
    Database(u32),
}
enum Reply {
    Connected(u32),
    Keys(KeyPage, Vec<KeyRow>),
    Value(Snapshot),
    Output(String),
    Changed(&'static str),
}
pub(crate) struct Workspace {
    sender: mpsc::SyncSender<Job>,
    receiver: mpsc::Receiver<Result<Reply, (String, bool)>>,
    busy: bool,
    pub connected: bool,
    database: u32,
    database_draft: u32,
    keys: Vec<Vec<u8>>,
    rows: std::collections::HashMap<Vec<u8>, (String, String, i64)>,
    commands_visible: bool,
    inspector_visible: bool,
    filters_visible: bool,
    database_picker: bool,
    item_search: String,
    field_search: String,
    cursor: u64,
    scan_started: bool,
    pattern: String,
    scan_pattern: Vec<u8>,
    key_limit: bool,
    selected_key: Option<Vec<u8>>,
    snapshot: Option<Snapshot>,
    value_editor: String,
    preview: String,
    output: String,
    command: String,
    error: Option<String>,
    notice: Option<String>,
    read_only: bool,
    ttl_seconds: u64,
    delete_confirmation: bool,
}
impl Workspace {
    pub fn new(mut settings: Settings, password: String, ctx: egui::Context) -> Self {
        let database = settings.database;
        let (sender, jobs) = mpsc::sync_channel(1);
        let (responses, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut session = match Session::connect(&settings, &password) {
                Ok(session) => {
                    let _ = responses.send(Ok(Reply::Connected(settings.database)));
                    ctx.request_repaint();
                    session
                }
                Err(error) => {
                    let _ = responses.send(Err((error, false)));
                    ctx.request_repaint();
                    Session::disconnected()
                }
            };
            while let Ok(job) = jobs.recv() {
                let result = match job {
                    Job::Scan(cursor, pattern) => {
                        session.scan(cursor, &pattern, 200).and_then(|page| {
                            let rows =
                                session.key_rows(&page.keys[..page.keys.len().min(10000)])?;
                            Ok(Reply::Keys(page, rows))
                        })
                    }
                    Job::Inspect(key) => session.inspect(&key).map(Reply::Value),
                    Job::Execute(text, safe) => session
                        .execute(&text, safe)
                        .map(|values| Reply::Output(render_values(&values))),
                    Job::Save(key, original, value, safe) => session
                        .replace_string(&key, &original, &value, safe)
                        .map(|_| Reply::Changed("Value saved.")),
                    Job::Delete(key, safe) => session.delete(&key, safe).map(|deleted| {
                        Reply::Changed(if deleted {
                            "Key deleted. Refresh keys to update the list."
                        } else {
                            "The key was already absent."
                        })
                    }),
                    Job::Expire(key, ttl, safe) => session.expire(&key, ttl, safe).map(|changed| {
                        Reply::Changed(if changed {
                            "Expiry updated."
                        } else {
                            "Key no longer exists or expiry was unchanged."
                        })
                    }),
                    Job::Database(database) => {
                        let candidate = Settings {
                            database,
                            ..settings.clone()
                        };
                        Session::connect(&candidate, &password).map(|connection| {
                            session = connection;
                            settings = candidate;
                            Reply::Connected(database)
                        })
                    }
                };
                if responses
                    .send(result.map_err(|error| (error, session.is_connected())))
                    .is_err()
                {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Self {
            sender,
            receiver,
            busy: true,
            connected: false,
            database,
            database_draft: database,
            keys: vec![],
            rows: Default::default(),
            commands_visible: false,
            inspector_visible: true,
            filters_visible: false,
            database_picker: false,
            item_search: String::new(),
            field_search: String::new(),
            cursor: 0,
            scan_started: false,
            pattern: "*".into(),
            scan_pattern: b"*".to_vec(),
            key_limit: false,
            selected_key: None,
            snapshot: None,
            value_editor: String::new(),
            preview: String::new(),
            output: String::new(),
            command: "PING".into(),
            error: None,
            notice: None,
            read_only: true,
            ttl_seconds: 3600,
            delete_confirmation: false,
        }
    }
    pub fn busy(&self) -> bool {
        self.busy || self.value_dirty()
    }
    fn value_dirty(&self) -> bool {
        self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.kind == "string"
                && !snapshot.truncated
                && match &snapshot.value {
                    redis::Value::BulkString(bytes) => {
                        self.value_editor != crate::redis_commands::quote(bytes)
                    }
                    _ => false,
                }
        })
    }
    fn send(&mut self, job: Job) {
        if self.busy {
            return;
        }
        if self.value_dirty() && !matches!(&job, Job::Save(..)) {
            self.error = Some("Save or discard the edited value before changing keys, databases, or running another operation.".into());
            return;
        }
        let selected = match &job {
            Job::Inspect(key) => Some(key.clone()),
            _ => None,
        };
        match self.sender.try_send(job) {
            Ok(()) => {
                if let Some(key) = selected {
                    self.selected_key = Some(key);
                }
                self.busy = true;
                self.error = None;
                self.notice = None;
            }
            Err(_) => {
                self.error = Some("Redis worker is unavailable. Reopen this connection.".into())
            }
        }
    }
    fn refresh(&mut self) {
        if self.value_dirty() {
            self.error = Some("Save or discard the edited value before refreshing keys.".into());
            return;
        }
        self.selected_key = None;
        self.keys.clear();
        self.rows.clear();
        self.cursor = 0;
        self.scan_started = false;
        self.key_limit = false;
        self.scan_pattern = self.pattern.as_bytes().to_vec();
        self.send(Job::Scan(0, self.scan_pattern.clone()));
    }
    fn poll(&mut self) {
        while let Ok(reply) = self.receiver.try_recv() {
            self.busy = false;
            match reply {
                Err((error, connected)) => {
                    self.error = Some(error);
                    self.connected = connected;
                }
                Ok(Reply::Connected(database)) => {
                    self.connected = true;
                    self.database = database;
                    self.database_draft = database;
                    self.snapshot = None;
                    self.refresh();
                }
                Ok(Reply::Keys(page, rows)) => {
                    for row in rows {
                        self.rows.insert(
                            row.key,
                            (grid_preview(&row.value, &row.kind), row.kind, row.ttl_ms),
                        );
                    }
                    self.cursor = page.cursor;
                    self.scan_started = true;
                    let mut bytes = self.keys.iter().map(Vec::len).sum::<usize>();
                    let mut known: std::collections::HashSet<_> =
                        self.keys.iter().cloned().collect();
                    for key in page.keys {
                        if known.contains(&key) {
                            continue;
                        }
                        if self.keys.len() >= 10000
                            || bytes.saturating_add(key.len()) > 16 * 1024 * 1024
                        {
                            self.key_limit = true;
                            break;
                        }
                        bytes += key.len();
                        known.insert(key.clone());
                        self.keys.push(key);
                    }
                    self.keys.sort();
                }
                Ok(Reply::Value(snapshot)) => {
                    self.rows.insert(
                        snapshot.key.clone(),
                        (
                            grid_preview(&snapshot.value, &snapshot.kind),
                            snapshot.kind.clone(),
                            snapshot.ttl_ms,
                        ),
                    );
                    self.value_editor = match &snapshot.value {
                        redis::Value::BulkString(bytes) => crate::redis_commands::quote(bytes),
                        _ => String::new(),
                    };
                    self.preview = render_inspection(&snapshot);
                    self.snapshot = Some(snapshot);
                }
                Ok(Reply::Output(output)) => self.output = output,
                Ok(Reply::Changed(notice)) => {
                    if let Some(snapshot) = &self.snapshot {
                        let key = snapshot.key.clone();
                        self.snapshot = None;
                        self.send(Job::Inspect(key));
                    }
                    self.notice = Some(notice.into());
                }
            }
        }
    }
    /// Returns true when the user asks to open the connection manager.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        title: &str,
        color: egui::Color32,
        pane_sizes: &mut crate::pane_sizes::PaneSizes,
        preferences_dirty: &mut bool,
    ) -> bool {
        self.poll();
        let mut manager = false;
        egui::TopBottomPanel::top("redis-toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                crate::icons::toolbar_group(ui, 3, |ui| {
                    manager = crate::icons::button(ui, "plug", crate::icons::TOOLBAR_BUTTON, false)
                        .on_hover_text("Connections")
                        .clicked();
                    if crate::icons::button(ui, "database", crate::icons::TOOLBAR_BUTTON, false)
                        .on_hover_text("Database selection")
                        .clicked()
                    {
                        self.database_picker = !self.database_picker;
                    }
                    if crate::icons::button(ui, "file-code", crate::icons::TOOLBAR_BUTTON, false)
                        .on_hover_text("Redis commands")
                        .clicked()
                    {
                        self.commands_visible = !self.commands_visible;
                    }
                });
                ui.add_space(4.0);
                let status_width =
                    (ui.available_width() - crate::icons::toolbar_group_width(3) - 12.0).max(40.0);
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(status_width, 28.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 14.0, color);
                ui.painter().with_clip_rect(rect.shrink(8.0)).text(
                    rect.left_center() + egui::vec2(12.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    format!("Redis  ·  {title}  ·  db{}", self.database),
                    egui::FontId::monospace(12.0),
                    egui::Color32::WHITE,
                );
                ui.add_space(4.0);
                crate::icons::toolbar_group(ui, 3, |ui| {
                    if crate::icons::button(ui, "refresh-cw", crate::icons::TOOLBAR_BUTTON, false)
                        .on_hover_text("Refresh keys")
                        .clicked()
                        && !self.busy
                    {
                        self.refresh();
                    }
                    if crate::icons::button(ui, "panel-bottom", crate::icons::TOOLBAR_BUTTON, false)
                        .on_hover_text("Toggle commands")
                        .clicked()
                    {
                        self.commands_visible = !self.commands_visible;
                    }
                    if crate::icons::button(ui, "panel-right", crate::icons::TOOLBAR_BUTTON, false)
                        .on_hover_text("Toggle row details")
                        .clicked()
                    {
                        self.inspector_visible = !self.inspector_visible;
                    }
                });
            });
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            if let Some(notice) = &self.notice {
                ui.label(notice);
            }
            if !self.connected && !self.busy && ui.button("Reconnect").clicked() {
                self.send(Job::Database(self.database));
            }
        });
        let keys_id = egui::Id::new((ctx.viewport_id(), "redis-databases"));
        crate::pane_sizes::restore(ctx, keys_id, pane_sizes.redis_keys, 220.0, 0);
        egui::SidePanel::left(keys_id)
            .width_range(160.0..=360.0)
            .show(ctx, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    crate::app::sidebar_section_button(ui, "Items", true);
                });
                ui.add_space(4.0);
                crate::app::sidebar_search(ui, &mut self.item_search, "Search for item…");
                ui.add_space(4.0);
                let database_name = format!("db{}", self.database);
                let (heading, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 22.0),
                    egui::Sense::hover(),
                );
                ui.painter().text(
                    heading.left_center(),
                    egui::Align2::LEFT_CENTER,
                    "Databases (1)",
                    egui::FontId::proportional(11.0),
                    egui::Color32::from_rgb(125, 137, 156),
                );
                if database_name.contains(&self.item_search.to_lowercase()) {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 22.0),
                        egui::Sense::click(),
                    );
                    crate::app::paint_sidebar_item(ui, rect, &database_name, true, false);
                    crate::icons::paint(
                        ui.painter(),
                        rect.left_center() + egui::vec2(14.0, 0.0),
                        "database",
                        13.0,
                        ui.visuals().selection.stroke.color,
                    );
                    if response.clicked() && !self.busy {
                        self.refresh();
                    }
                }
                if self.database_picker {
                    ui.horizontal(|ui| {
                        ui.label("Database");
                        ui.add(egui::DragValue::new(&mut self.database_draft).range(0..=u32::MAX));
                        if ui
                            .add_enabled(
                                !self.busy && self.database_draft != self.database,
                                egui::Button::new("Open"),
                            )
                            .clicked()
                        {
                            self.send(Job::Database(self.database_draft));
                        }
                    });
                }
            });
        *preferences_dirty |=
            crate::pane_sizes::record_drag(ctx, keys_id, &mut pane_sizes.redis_keys, 0);
        if self.inspector_visible {
            let id = egui::Id::new((ctx.viewport_id(), "redis-row-details"));
            crate::pane_sizes::restore(ctx, id, pane_sizes.inspector, 240.0, 0);
            egui::SidePanel::right(id)
                .default_width(240.0)
                .width_range(190.0..=420.0)
                .show(ctx, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.add_space(4.0);
                    crate::app::sidebar_search(ui, &mut self.field_search, "Search for field…");
                    ui.add_space(5.0);
                    egui::ScrollArea::vertical()
                        .id_salt("redis-inspector-scroll")
                        .show(ui, |ui| self.inspector(ui));
                });
            *preferences_dirty |=
                crate::pane_sizes::record_drag(ctx, id, &mut pane_sizes.inspector, 0);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(ctx.style().visuals.panel_fill))
            .show(ctx, |ui| {
                if self.commands_visible {
                    let commands_id = egui::Id::new((ctx.viewport_id(), "redis-command-pane"));
                    crate::pane_sizes::restore(
                        ctx,
                        commands_id,
                        pane_sizes.redis_commands,
                        220.0,
                        1,
                    );
                    egui::TopBottomPanel::bottom(commands_id)
                        .height_range(140.0..=600.0)
                        .resizable(true)
                        .show_inside(ui, |ui| {
                            ui.set_min_height(ui.available_height());
                            self.command_pane(ui);
                        });
                    *preferences_dirty |= crate::pane_sizes::record_drag(
                        ctx,
                        commands_id,
                        &mut pane_sizes.redis_commands,
                        1,
                    );
                }
                egui::TopBottomPanel::bottom("redis-grid-footer")
                    .frame(
                        egui::Frame::new()
                            .fill(ui.visuals().panel_fill)
                            .inner_margin(6),
                    )
                    .show_inside(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.selectable_label(true, "Data")
                                .on_hover_text("Redis key/value grid");
                            ui.separator();
                            ui.checkbox(&mut self.read_only, "Read-only");
                            if self.busy {
                                ui.spinner();
                            }
                            ui.label(format!("{} keys loaded", self.keys.len()));
                            if ui
                                .add(crate::icons::widget("search"))
                                .on_hover_text("Filter keys")
                                .clicked()
                            {
                                self.filters_visible = !self.filters_visible;
                            }
                            let more = !self.busy
                                && self.connected
                                && (!self.scan_started || self.cursor != 0)
                                && !self.key_limit;
                            if ui
                                .add_enabled(more, crate::icons::widget("chevron-right"))
                                .on_hover_text("Load more keys")
                                .clicked()
                            {
                                self.send(Job::Scan(self.cursor, self.scan_pattern.clone()));
                            }
                            if self.key_limit {
                                ui.weak("Limit reached; narrow the filter.");
                            }
                        });
                    });
                egui::Frame::new()
                    .fill(egui::Color32::from_gray(48))
                    .inner_margin(6)
                    .show(ui, |ui| {
                        ui.add_sized(
                            [ui.available_width(), 28.0],
                            egui::Button::new(format!("db{}", self.database))
                                .fill(egui::Color32::from_gray(76))
                                .corner_radius(14),
                        );
                    });
                if self.filters_visible {
                    ui.horizontal(|ui| {
                        ui.label("Key pattern");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.pattern)
                                .desired_width((ui.available_width() - 80.0).max(80.0)),
                        );
                        if ui
                            .add_enabled(!self.busy && self.connected, egui::Button::new("Apply"))
                            .clicked()
                        {
                            self.refresh();
                        }
                    });
                }
                ui.scope(|ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(ui.available_rect_before_wrap()));
                    self.key_grid(ui);
                });
            });
        if self.delete_confirmation {
            let mut delete = false;
            let modal =
                egui::Modal::new(egui::Id::new("redis-delete-confirmation")).show(ctx, |ui| {
                    ui.heading("Delete this key?");
                    if let Some(snapshot) = &self.snapshot {
                        ui.monospace(display_key(&snapshot.key));
                    }
                    ui.label("This deletes the entire value from Redis.");
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.delete_confirmation = false;
                        }
                        delete = ui
                            .add_enabled(
                                !self.busy && !self.read_only,
                                egui::Button::new("Delete key"),
                            )
                            .clicked();
                    });
                });
            if modal.should_close() {
                self.delete_confirmation = false;
            }
            if delete {
                self.delete_confirmation = false;
                if let Some(snapshot) = &self.snapshot {
                    self.send(Job::Delete(snapshot.key.clone(), self.read_only));
                }
            }
        }
        if self.busy() && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.error = Some(
                "Finish the Redis operation and save or discard value edits before closing.".into(),
            );
        }
        manager
    }
    fn inspector(&mut self, ui: &mut egui::Ui) {
        let mut job = None;
        if let Some(snapshot) = &self.snapshot {
            crate::app::compact_inspector_spacing(ui);
            let search = self.field_search.to_lowercase();
            for (name, kind, value) in [
                ("key", "bytes", display_key(&snapshot.key)),
                ("type", "Redis type", snapshot.kind.clone()),
                (
                    "length",
                    "elements / bytes",
                    snapshot.length.map_or("—".into(), |n| n.to_string()),
                ),
                (
                    "ttl",
                    "seconds",
                    match snapshot.ttl_ms {
                        -2 => "Key no longer exists".into(),
                        -1 => "No expiry".into(),
                        n => format!("{:.1} (at load)", n as f64 / 1000.0),
                    },
                ),
            ] {
                if search.is_empty() || name.contains(&search) {
                    inspector_field(ui, name, kind, &value);
                }
            }
            if snapshot.truncated {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "Partial preview. Use range/scan commands to inspect the remaining values.",
                );
            }
            if search.is_empty() || "value".contains(&search) {
                inspector_field_heading(ui, "value", &snapshot.kind);
                if snapshot.kind == "string" && !snapshot.truncated {
                    ui.add_enabled(
                        !self.busy && !self.read_only,
                        egui::TextEdit::multiline(&mut self.value_editor)
                            .code_editor()
                            .desired_rows(4)
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(3.0);
                    ui.weak("Quoted bytes · Save preserves expiry");
                    if ui.small_button("Discard edits").clicked()
                        && let redis::Value::BulkString(bytes) = &snapshot.value
                    {
                        self.value_editor = crate::redis_commands::quote(bytes);
                    }
                    if ui
                        .add_enabled(
                            !self.busy && !self.read_only,
                            egui::Button::new("Save value"),
                        )
                        .clicked()
                    {
                        match crate::redis_commands::parse(&self.value_editor) {
                            Ok(args) if args.len() == 1 && args[0].len() == 1 => {
                                if let redis::Value::BulkString(original) = &snapshot.value {
                                    job = Some(Job::Save(
                                        snapshot.key.clone(),
                                        original.clone(),
                                        args[0][0].clone(),
                                        self.read_only,
                                    ));
                                }
                            }
                            _ => self.error = Some(
                                "Enter exactly one quoted value, such as \"hello\" or \"\\xff\"."
                                    .into(),
                            ),
                        }
                    }
                } else {
                    egui::Frame::new()
                        .fill(ui.visuals().extreme_bg_color)
                        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                        .corner_radius(4.0)
                        .inner_margin(4.0)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.add(
                                egui::Label::new(egui::RichText::new(&self.preview).monospace())
                                    .wrap(),
                            );
                        });
                }
            }
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Refresh"))
                    .clicked()
                {
                    job = Some(Job::Inspect(snapshot.key.clone()));
                }
                if ui
                    .add_enabled(
                        !self.busy && !self.read_only,
                        egui::Button::new("Delete key…"),
                    )
                    .clicked()
                {
                    self.delete_confirmation = true;
                }
            });
            egui::CollapsingHeader::new("Expiry").show(ui, |ui| {
                ui.add_enabled_ui(
                    !self.busy && !self.read_only && snapshot.kind != "none",
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("TTL seconds");
                            ui.add(
                                egui::DragValue::new(&mut self.ttl_seconds)
                                    .range(1..=u64::MAX / 1000),
                            );
                            if ui.button("Set TTL").clicked() {
                                job = Some(Job::Expire(
                                    snapshot.key.clone(),
                                    Some(self.ttl_seconds * 1000),
                                    self.read_only,
                                ));
                            }
                            if ui.button("Remove expiry").clicked() {
                                job = Some(Job::Expire(snapshot.key.clone(), None, self.read_only));
                            }
                        });
                    },
                );
            });
        } else {
            ui.centered_and_justified(|ui| {
                ui.weak("Select a key to inspect its value and expiry.");
            });
        }
        if let Some(job) = job {
            self.send(job);
        }
    }
    fn command_pane(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Redis commands");
            ui.weak("One command per line · quoted arguments and \\xHH escapes");
            if ui
                .add_enabled(!self.busy && self.connected, egui::Button::new("Run"))
                .clicked()
            {
                self.send(Job::Execute(self.command.clone(), self.read_only));
            }
        });
        ui.add(
            egui::TextEdit::multiline(&mut self.command)
                .code_editor()
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        egui::ScrollArea::both()
            .id_salt("redis-command-output")
            .show(ui, |ui| {
                ui.monospace(&self.output);
            });
    }
    fn key_grid(&mut self, ui: &mut egui::Ui) {
        use egui_extras::{Column, TableBuilder};
        let mut inspect = None;
        let mut delete = None;
        TableBuilder::new(ui)
            .id_salt(("redis-key-grid", self.database))
            .striped(false)
            .resizable(true)
            .auto_shrink([false, false])
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(34.0))
            .column(Column::initial(200.0).at_least(90.0).clip(true))
            .column(Column::remainder().at_least(120.0).clip(true))
            .column(Column::initial(85.0).clip(true))
            .column(Column::initial(70.0).clip(true))
            .header(28.0, |mut header| {
                header.col(crate::app::paint_grid_corner);
                for name in ["key", "value", "type", "ttl"] {
                    header.col(|ui| {
                        crate::app::paint_column_header(ui, name, None);
                    });
                }
            })
            .body(|mut body| {
                let widths = body.widths().to_vec();
                crate::app::paint_grid_backdrop(body.ui_mut(), &widths);
                body.rows(22.0, self.keys.len(), |mut row| {
                    let index = row.index();
                    let key = &self.keys[index];
                    row.set_selected(self.selected_key.as_ref() == Some(key));
                    let metadata = self.rows.get(key);
                    row.col(|ui| {
                        ui.weak((index + 1).to_string());
                    });
                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(display_key(key)).monospace())
                                .truncate(),
                        );
                    });
                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(
                                    metadata.map_or("Select to inspect", |row| row.0.as_str()),
                                )
                                .monospace(),
                            )
                            .truncate(),
                        );
                    });
                    row.col(|ui| {
                        ui.monospace(metadata.map_or("—".into(), |row| row.1.to_ascii_uppercase()));
                    });
                    row.col(|ui| {
                        ui.monospace(metadata.map_or("—".into(), |row| {
                            if row.2 < 0 {
                                row.2.to_string()
                            } else {
                                (row.2 / 1000).to_string()
                            }
                        }));
                    });
                    let response = row.response();
                    if response.contains_pointer()
                        && response.ctx.input(|input| input.pointer.primary_pressed())
                        && !self.busy
                    {
                        inspect = Some(key.clone());
                    }
                    response.context_menu(|ui| {
                        if ui.button("Inspect key").clicked() {
                            inspect = Some(key.clone());
                            ui.close();
                        }
                        if ui.button("Copy key").clicked() {
                            ui.ctx().copy_text(display_key(key));
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                !self.read_only && !self.busy,
                                egui::Button::new("Delete key…"),
                            )
                            .clicked()
                        {
                            delete = Some(key.clone());
                            ui.close();
                        }
                    });
                });
            });
        if let Some(key) = inspect {
            self.inspector_visible = true;
            self.send(Job::Inspect(key));
        }
        if let Some(key) = delete {
            if self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.key == key)
            {
                self.delete_confirmation = true;
            } else {
                self.inspector_visible = true;
                self.send(Job::Inspect(key));
                self.notice = Some("Inspect this key, then choose Delete key to confirm.".into());
            }
        }
    }
}
fn display_key(key: &[u8]) -> String {
    if key.len() > 512 {
        return format!(
            "{}… ({} bytes)",
            crate::redis_commands::quote(&key[..512]),
            key.len()
        );
    }
    match std::str::from_utf8(key) {
        Ok(text) if !text.chars().any(char::is_control) => text.to_owned(),
        _ => crate::redis_commands::quote(key),
    }
}
fn inspector_field_heading(ui: &mut egui::Ui, name: &str, kind: &str) {
    ui.horizontal(|ui| {
        ui.add(
            egui::Label::new(
                egui::RichText::new(name)
                    .size(11.0)
                    .color(ui.visuals().weak_text_color()),
            )
            .truncate(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(kind)
                        .size(10.0)
                        .color(ui.visuals().weak_text_color()),
                )
                .truncate(),
            );
        });
    });
}
fn inspector_field(ui: &mut egui::Ui, name: &str, kind: &str, value: &str) {
    ui.push_id(name, |ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 44.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_height(44.0);
                inspector_field_heading(ui, name, kind);
                egui::Frame::new()
                    .fill(ui.visuals().extreme_bg_color)
                    .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                    .corner_radius(4.0)
                    .inner_margin(1.0)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            if crate::app::clipboard_button(ui).clicked() {
                                ui.ctx().copy_text(value.to_owned());
                            }
                            crate::app::inspector_value_preview(ui, value).on_hover_text(value);
                        });
                    });
            },
        );
    });
}

fn grid_preview(value: &redis::Value, kind: &str) -> String {
    fn json(value: &redis::Value) -> serde_json::Value {
        match value {
            redis::Value::BulkString(bytes) => {
                serde_json::Value::String(match std::str::from_utf8(bytes) {
                    Ok(text) if !text.contains('\0') => text.to_owned(),
                    _ => crate::redis_commands::quote(bytes),
                })
            }
            redis::Value::Array(values) => {
                serde_json::Value::Array(values.iter().map(json).collect())
            }
            redis::Value::Int(n) => (*n).into(),
            redis::Value::SimpleString(text) => text.clone().into(),
            _ => serde_json::Value::Null,
        }
    }
    let text = if kind == "hash"
        && let redis::Value::Array(values) = value
    {
        let mut map = serde_json::Map::new();
        for pair in values.as_chunks::<2>().0 {
            map.insert(
                json(&pair[0]).as_str().unwrap_or("").to_owned(),
                json(&pair[1]),
            );
        }
        serde_json::Value::Object(map).to_string()
    } else if let redis::Value::BulkString(bytes) = value {
        match std::str::from_utf8(bytes) {
            Ok(text) if !text.chars().any(char::is_control) => text.to_owned(),
            _ => crate::redis_commands::quote(bytes),
        }
    } else {
        json(value).to_string()
    };
    let mut output: String = text.chars().take(256).collect();
    if text.chars().count() > 256 {
        output.push('…');
    }
    output
}
fn render_inspection(snapshot: &Snapshot) -> String {
    if matches!(snapshot.kind.as_str(), "hash" | "zset")
        && let redis::Value::Array(values) = &snapshot.value
    {
        let mut text = if snapshot.kind == "hash" {
            "Field → Value\n".to_owned()
        } else {
            "Member → Score\n".to_owned()
        };
        for pair in values.chunks(2) {
            let left = render_values(&pair[..1]);
            let right = render_values(&pair[1..]);
            text.push_str(&format!("{} → {}\n", left.trim_end(), right.trim_end()));
            if text.len() > 256 * 1024 {
                break;
            }
        }
        return limit_output(text);
    }
    render_values(std::slice::from_ref(&snapshot.value))
}
fn limit_output(mut output: String) -> String {
    if output.len() > 256 * 1024 {
        let mut end = 256 * 1024;
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        output.truncate(end);
        output.push_str("\n[output truncated]");
    }
    output
}
fn render_values(values: &[redis::Value]) -> String {
    fn append(value: &redis::Value, out: &mut String, depth: usize) {
        if out.len() > 256 * 1024 {
            return;
        }
        if depth > 32 {
            out.push_str("[nesting limit]\n");
            return;
        }
        match value {
            redis::Value::BulkString(bytes) => {
                out.push_str(&crate::redis_commands::quote(
                    &bytes[..bytes.len().min(65536)],
                ));
                if bytes.len() > 65536 {
                    out.push_str(" … [truncated]");
                }
                out.push('\n');
            }
            redis::Value::Array(values) | redis::Value::Set(values) => {
                if values.is_empty() {
                    out.push_str("(empty collection)\n");
                }
                for (i, value) in values.iter().take(1000).enumerate() {
                    if out.len() > 256 * 1024 {
                        break;
                    }
                    out.push_str(&format!("{}{}. ", "  ".repeat(depth), i + 1));
                    append(value, out, depth + 1);
                }
                if values.len() > 1000 {
                    out.push_str("[remaining elements omitted]\n");
                }
            }
            redis::Value::Map(values) => {
                if values.is_empty() {
                    out.push_str("(empty map)\n");
                }
                for (key, value) in values.iter().take(1000) {
                    if out.len() > 256 * 1024 {
                        break;
                    }
                    append(key, out, depth + 1);
                    append(value, out, depth + 1);
                }
            }
            redis::Value::SimpleString(value) => {
                out.push_str(&value.chars().take(65536).collect::<String>());
                out.push('\n');
            }
            redis::Value::Nil => out.push_str("(nil)\n"),
            redis::Value::Okay => out.push_str("OK\n"),
            redis::Value::Int(value) => out.push_str(&format!("{value}\n")),
            redis::Value::Double(value) => out.push_str(&format!("{value}\n")),
            redis::Value::Boolean(value) => out.push_str(&format!("{value}\n")),
            _ => out.push_str("[reply type unavailable in this view]\n"),
        }
    }
    let mut output = String::new();
    for value in values {
        append(value, &mut output, 0);
    }
    limit_output(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grid_previews_render_strings_hashes_and_binary_safely() {
        let bulk = |text: &str| redis::Value::BulkString(text.as_bytes().to_vec());
        assert_eq!(grid_preview(&bulk("hello"), "string"), "hello");
        assert_eq!(
            grid_preview(
                &redis::Value::Array(vec![bulk("name"), bulk("Alice")]),
                "hash"
            ),
            r#"{"name":"Alice"}"#
        );
        assert_eq!(
            grid_preview(&redis::Value::BulkString(vec![255, 0]), "string"),
            r#""\xff\x00""#
        );
        assert!(grid_preview(&bulk(&"x".repeat(300)), "string").ends_with('…'));
    }
    #[test]
    fn reply_previews_distinguish_empty_nil_and_preserve_binary() {
        assert_ne!(
            render_values(&[redis::Value::Nil]),
            render_values(&[redis::Value::Array(vec![])])
        );
        assert_eq!(
            render_values(&[redis::Value::BulkString(vec![255, 0])]),
            "\"\\xff\\x00\"\n"
        );
        let output = render_values(&[redis::Value::Array(vec![
            redis::Value::BulkString(vec![
                255;
                65536
            ]);
            20
        ])]);
        assert!(output.len() < 256 * 1024 + 100);
        assert!(output.contains("[output truncated]"));
    }
}
