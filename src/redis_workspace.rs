//! Redis-native UI; network and decoding work remain on a dedicated worker.
use crate::{
    redis_session::{KeyPage, Session, Snapshot},
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
    Keys(KeyPage),
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
    cursor: u64,
    scan_started: bool,
    pattern: String,
    scan_pattern: Vec<u8>,
    key_limit: bool,
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
                        session.scan(cursor, &pattern, 200).map(Reply::Keys)
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
            cursor: 0,
            scan_started: false,
            pattern: "*".into(),
            scan_pattern: b"*".to_vec(),
            key_limit: false,
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
        match self.sender.try_send(job) {
            Ok(()) => {
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
        self.keys.clear();
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
                Ok(Reply::Keys(page)) => {
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
        pane_sizes: &mut crate::pane_sizes::PaneSizes,
        preferences_dirty: &mut bool,
    ) -> bool {
        self.poll();
        let mut manager = false;
        egui::TopBottomPanel::top("redis-toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                manager = ui
                    .add(crate::icons::widget("plug"))
                    .on_hover_text("Connections")
                    .clicked();
                ui.strong(title);
                ui.label("Redis");
                ui.separator();
                ui.label("Database");
                ui.add_enabled(
                    !self.busy,
                    egui::DragValue::new(&mut self.database_draft).range(0..=u32::MAX),
                );
                if ui
                    .add_enabled(
                        !self.busy && self.database_draft != self.database,
                        egui::Button::new("Open"),
                    )
                    .clicked()
                {
                    self.send(Job::Database(self.database_draft));
                }
                ui.separator();
                ui.checkbox(&mut self.read_only, "Read-only");
                if !self.connected && !self.busy && ui.button("Reconnect").clicked() {
                    self.send(Job::Database(self.database));
                }
                if self.busy {
                    ui.spinner();
                }
            });
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            if let Some(notice) = &self.notice {
                ui.label(notice);
            }
        });
        let keys_id = egui::Id::new((ctx.viewport_id(), "redis-key-browser"));
        crate::pane_sizes::restore(ctx, keys_id, pane_sizes.redis_keys, 280.0, 0);
        egui::SidePanel::left(keys_id).default_width(280.0).width_range(180.0..=600.0).resizable(true).show(ctx, |ui| {
            ui.set_min_width(ui.available_width());
                ui.heading("Keys");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.pattern).desired_width((ui.available_width()-65.0).max(40.0)).hint_text("Key pattern, e.g. user:*"));
                if ui.add_enabled(!self.busy && self.connected, egui::Button::new("Scan")).clicked() { self.refresh(); }
            });
            ui.weak(format!("{} keys loaded · incremental scan", self.keys.len()));
            let can_more = !self.busy && self.connected && (!self.scan_started || self.cursor != 0) && !self.key_limit;
            if ui.add_enabled(can_more, egui::Button::new("Load more keys")).clicked() { self.send(Job::Scan(self.cursor, self.scan_pattern.clone())); }
            if self.key_limit { ui.weak("Key browsing limit reached (10,000 keys / 16 MiB). Narrow the pattern and scan again."); }
            let mut inspect = None;
            egui::ScrollArea::vertical().show_rows(ui, 24.0, self.keys.len(), |ui, range| {
                for index in range {
                    let key = &self.keys[index];
                    let selected = self.snapshot.as_ref().is_some_and(|snapshot| &snapshot.key == key);
                    let label = display_key(key);
                    if ui.add_enabled(!self.busy, egui::Button::selectable(selected, egui::RichText::new(label).monospace()).truncate()).clicked() { inspect = Some(key.clone()); }
                }
            });
            if let Some(key) = inspect { self.send(Job::Inspect(key)); }
        });
        *preferences_dirty |=
            crate::pane_sizes::record_drag(ctx, keys_id, &mut pane_sizes.redis_keys, 0);
        egui::CentralPanel::default().show(ctx, |ui| {
            let commands_id = egui::Id::new((ctx.viewport_id(), "redis-command-pane"));
            crate::pane_sizes::restore(ctx, commands_id, pane_sizes.redis_commands, 260.0, 1);
            egui::TopBottomPanel::bottom(commands_id)
                .default_height(260.0)
                .height_range(140.0..=600.0)
                .resizable(true)
                .show_inside(ui, |ui| {
                    ui.set_min_height(ui.available_height());
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
                });
            *preferences_dirty |=
                crate::pane_sizes::record_drag(ctx, commands_id, &mut pane_sizes.redis_commands, 1);
            let mut job = None;
            if let Some(snapshot) = &self.snapshot {
                ui.heading(display_key(&snapshot.key));
                ui.horizontal(|ui| {
                    ui.label(format!("Type: {}", snapshot.kind));
                    if let Some(length) = snapshot.length {
                        ui.label(format!("Length: {length}"));
                    }
                    ui.label(match snapshot.ttl_ms {
                        -2 => "Key no longer exists".into(),
                        -1 => "No expiry".into(),
                        n => format!("TTL: {:.1}s (at load)", n as f64 / 1000.0),
                    });
                    if ui
                        .add_enabled(!self.busy, egui::Button::new("Refresh"))
                        .clicked()
                    {
                        job = Some(Job::Inspect(snapshot.key.clone()));
                    }
                });
                if snapshot.truncated {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "Partial preview. Use range/scan commands to inspect the remaining values.",
                    );
                }
                ui.add_enabled_ui(
                    !self.busy && !self.read_only && snapshot.kind != "none",
                    |ui| {
                        ui.horizontal(|ui| {
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
                            if ui.button("Delete key…").clicked() {
                                self.delete_confirmation = true;
                            }
                        });
                    },
                );
                if snapshot.kind == "string" && !snapshot.truncated {
                    ui.weak("Quoted byte string. Escapes preserve binary data.");
                    ui.add_enabled(
                        !self.busy && !self.read_only,
                        egui::TextEdit::multiline(&mut self.value_editor)
                            .code_editor()
                            .desired_rows(8)
                            .desired_width(f32::INFINITY),
                    );
                    if ui.button("Discard value edits").clicked()
                        && let redis::Value::BulkString(bytes) = &snapshot.value
                    {
                        self.value_editor = crate::redis_commands::quote(bytes);
                    }
                    if ui
                        .add_enabled(
                            !self.busy && !self.read_only,
                            egui::Button::new("Save string · preserve TTL"),
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
                    egui::ScrollArea::both().show(ui, |ui| {
                        ui.monospace(&self.preview);
                    });
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.weak("Select a key to inspect its value and expiry.");
                });
            }
            if let Some(job) = job {
                self.send(job);
            }
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
