//! Saved connection manager. Credentials and unfinished forms stay in memory only.
use super::*;
use crate::connection_probe::{Event as ProbeEvent, Probe, ProbeField};
use crate::connection_profiles::{
    ConnectionKind, GroupId, PostgresProfile, Profile, ProfileId, SshOptions,
};
use crate::credential_store::{Operation as CredentialOperation, Worker as CredentialWorker};

#[derive(Default)]
pub(super) struct Manager {
    draft: Option<Profile>,
    password: String,
    generation: u64,
    focus_group_name: bool,
    search: String,
    scroll_to_profile: Option<ProfileId>,
    rename_group: Option<(GroupId, String)>,
    collapsed_groups: std::collections::HashSet<Option<GroupId>>,
    error: Option<String>,
    probe: Option<Probe>,
    probe_snapshot: Option<(Profile, String)>,
    probe_status: Vec<(ProbeField, FieldStatus)>,
    probe_message: Option<String>,
    credentials: Option<CredentialWorker>,
    credential_pending: Vec<CredentialPending>,
    credential_latest: std::collections::HashMap<ProfileId, u64>,
    credential_delete_retry: std::collections::HashSet<ProfileId>,
    password_revision: u64,
}
#[derive(Clone, Copy)]
enum CredentialKind {
    Read(u64),
    Write,
    Delete,
}
struct CredentialPending {
    token: u64,
    id: ProfileId,
    generation: u64,
    kind: CredentialKind,
}
#[derive(Clone, Debug, PartialEq)]
enum FieldStatus {
    Running,
    Success,
    Failed(String),
}
impl Manager {
    fn queue_credential(&mut self, id: ProfileId, operation: CredentialOperation) {
        let kind = match &operation {
            CredentialOperation::Read => CredentialKind::Read(self.password_revision),
            CredentialOperation::Write(_) => CredentialKind::Write,
            CredentialOperation::Delete => CredentialKind::Delete,
        };
        let worker = self.credentials.get_or_insert_with(CredentialWorker::new);
        let token = worker.request(id, operation);
        if !matches!(kind, CredentialKind::Read(_)) {
            self.credential_latest.insert(id, token);
        }
        self.credential_pending.push(CredentialPending {
            token,
            id,
            generation: self.generation,
            kind,
        });
    }
    fn loading_password(&self) -> bool {
        self.credential_pending.iter().any(|pending| pending.generation == self.generation && matches!(pending.kind, CredentialKind::Read(revision) if revision == self.password_revision))
    }

    fn invalidate_probe(&mut self) {
        self.probe = None;
        self.probe_snapshot = None;
        self.probe_status.clear();
        self.probe_message = None;
    }
    fn poll_probe(&mut self) {
        if self
            .probe_snapshot
            .as_ref()
            .is_some_and(|(profile, password)| {
                self.draft.as_ref() != Some(profile) || &self.password != password
            })
        {
            self.invalidate_probe();
            return;
        }
        while let Some(event) = self.probe.as_ref().and_then(Probe::poll) {
            self.apply_probe_event(event);
        }
    }
    fn apply_probe_event(&mut self, event: ProbeEvent) {
        let (fields, status) = match event {
            ProbeEvent::Running(fields) => (fields, FieldStatus::Running),
            ProbeEvent::Success(fields) => (fields, FieldStatus::Success),
            ProbeEvent::Failed(fields, message) => {
                self.probe_message = Some(message.clone());
                (fields, FieldStatus::Failed(message))
            }
            ProbeEvent::Finished => {
                self.probe = None;
                self.probe_status
                    .retain(|(_, status)| !matches!(status, FieldStatus::Running));
                if self.probe_message.is_none() {
                    self.probe_message = Some("Connection test succeeded".into());
                }
                return;
            }
        };
        for field in fields {
            if let Some((_, previous)) = self.probe_status.iter_mut().find(|(key, _)| *key == field)
            {
                *previous = status.clone();
            } else {
                self.probe_status.push((field, status.clone()));
            }
        }
    }
    fn start_probe(&mut self, ctx: &egui::Context) {
        self.invalidate_probe();
        if let Some(profile) = &self.draft {
            self.probe_snapshot = Some((profile.clone(), self.password.clone()));
            self.probe = Some(Probe::start(
                profile.connection.clone(),
                self.password.clone(),
                ctx.clone(),
            ));
        }
    }

    fn select(&mut self, profile: Profile) {
        self.invalidate_probe();
        self.generation = self.generation.wrapping_add(1);
        self.password.clear();
        self.password_revision = self.password_revision.wrapping_add(1);
        let load = profile.remember_password && profile.id.0 != 0;
        let id = profile.id;
        self.draft = Some(profile);
        self.error = None;
        if load {
            self.queue_credential(id, CredentialOperation::Read);
        }
    }
    pub(super) fn set_path(&mut self, generation: u64, key: bool, path: PathBuf) {
        if self.generation != generation {
            return;
        }
        if let Some(draft) = &mut self.draft {
            match &mut draft.connection {
                ConnectionKind::SQLite { path: target } if !key => *target = path,
                ConnectionKind::Postgres(config) if key => {
                    if let Some(ssh) = &mut config.ssh {
                        ssh.identity_file = Some(path);
                    }
                }
                _ => {}
            }
        }
    }
    fn dirty(&self, profiles: &crate::connection_profiles::ConnectionProfiles) -> bool {
        self.draft
            .as_ref()
            .is_some_and(|draft| profiles.profile(draft.id) != Some(draft))
    }
}

pub(super) fn migrate_recent(prefs: &mut Preferences) {
    if prefs.connections_migrated {
        return;
    }
    for path in &prefs.recent {
        if prefs.saved_connections.profiles.iter().any(|profile| matches!(&profile.connection, ConnectionKind::SQLite { path: existing } if existing == path)) { continue; }
        let mut profile = Profile::draft(
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            ConnectionKind::SQLite { path: path.clone() },
        );
        if let Some(style) = prefs.connection_styles.get(&path.display().to_string()) {
            profile.environment = style.environment.clone();
            profile.color = style.color;
        }
        let _ = prefs.saved_connections.add_profile(profile);
    }
    prefs.connections_migrated = true;
}

#[derive(Clone, Copy)]
enum Drag {
    Profile(ProfileId),
    Group(GroupId),
}
enum Action {
    Select(ProfileId),
    New(bool),
    Save(bool),
    OpenWindow,
    Discard,
    Delete(ProfileId),
    Duplicate(ProfileId),
    Move(ProfileId, Option<GroupId>, usize),
    MoveGroup(GroupId, usize),
    DeleteGroup(GroupId),
    RenameGroup(GroupId, String),
    AddGroup,
}

fn form_checkbox(ui: &mut egui::Ui, checked: &mut bool, text: &str) -> egui::Response {
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        for visual in [
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
        ] {
            visual.corner_radius = egui::CornerRadius::same(3);
        }
        ui.checkbox(checked, text)
    })
    .inner
}

fn form_card(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(12.0)
        .inner_margin(12.0)
}

fn form_section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(title).strong());
    ui.add_space(3.0);
}

fn form_row(ui: &mut egui::Ui, label: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([84.0, 28.0], egui::Label::new(RichText::new(label).weak()));
        ui.scope(|ui| {
            ui.set_min_width(ui.available_width());
            content(ui);
        });
    });
}

fn status_row(
    ui: &mut egui::Ui,
    label: &str,
    field: Option<ProbeField>,
    content: impl FnOnce(&mut egui::Ui),
) {
    let response = ui.scope(|ui| form_row(ui, label, content)).response;
    let status = field.and_then(|field| {
        ui.ctx()
            .data(|data| {
                data.get_temp::<Vec<(ProbeField, FieldStatus)>>(egui::Id::new(
                    "connection-probe-status",
                ))
            })
            .and_then(|states| {
                states
                    .into_iter()
                    .find(|(key, _)| *key == field)
                    .map(|(_, status)| status)
            })
    });
    if let Some(status) = status {
        let (color, icon, tooltip) = match status {
            FieldStatus::Running => (ui.visuals().hyperlink_color, "•", "Testing…".to_owned()),
            FieldStatus::Success => (Color32::from_rgb(45, 170, 95), "✓", "Verified".to_owned()),
            FieldStatus::Failed(message) => (ui.visuals().error_fg_color, "!", message),
        };
        let mut rect = response.rect;
        rect.min.x += 92.0;
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            6.0,
            egui::Stroke::new(1.2_f32, color),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            egui::pos2(response.rect.left() + 79.0, response.rect.center().y),
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::proportional(12.0),
            color,
        );
        response.on_hover_text(tooltip);
    }
}

fn form_text(
    ui: &mut egui::Ui,
    label: &str,
    text: &mut String,
    password: bool,
    field: Option<ProbeField>,
) -> bool {
    let mut changed = false;
    status_row(ui, label, field, |ui| {
        let field = egui::Frame::new()
            .fill(ui.visuals().extreme_bg_color)
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.add_sized(
                    [ui.available_width(), 18.0],
                    TextEdit::singleline(text)
                        .desired_width(ui.available_width())
                        .password(password)
                        .frame(false)
                        .margin(egui::Margin::ZERO),
                )
            });
        changed = field.inner.changed();
        if field.inner.has_focus() {
            ui.painter().rect_stroke(
                field.response.rect,
                6.0,
                ui.visuals().selection.stroke,
                egui::StrokeKind::Inside,
            );
        }
    });
    changed
}

#[derive(Clone)]
struct PortEdit {
    text: String,
    committed: u16,
}
fn form_port(ui: &mut egui::Ui, port: &mut u16, field: ProbeField, generation: u64) -> bool {
    let id = ui.make_persistent_id(("connection-port-text", format!("{field:?}"), generation));
    let mut edit = ui
        .ctx()
        .data_mut(|data| data.get_temp::<PortEdit>(id))
        .unwrap_or_else(|| PortEdit {
            text: port.to_string(),
            committed: *port,
        });
    if edit.committed != *port {
        edit.text = port.to_string();
        edit.committed = *port;
    }
    ui.scope(|ui| {
        ui.set_max_width(184.0);
        form_text(ui, "Port", &mut edit.text, false, Some(field));
    });
    let value = edit.text.parse::<u16>().ok().filter(|value| *value != 0);
    if let Some(value) = value {
        *port = value;
        edit.committed = value;
    }
    ui.ctx().data_mut(|data| data.insert_temp(id, edit));
    value.is_some()
}

impl Rustable {
    fn poll_credentials(&mut self) {
        let mut profiles_changed = false;
        while let Some(response) = self
            .connection_manager
            .credentials
            .as_ref()
            .and_then(CredentialWorker::poll)
        {
            let Some(index) = self
                .connection_manager
                .credential_pending
                .iter()
                .position(|pending| pending.token == response.token)
            else {
                continue;
            };
            let pending = self.connection_manager.credential_pending.remove(index);
            let manager = &mut self.connection_manager;
            match pending.kind {
                CredentialKind::Read(revision) => {
                    if pending.generation != manager.generation
                        || revision != manager.password_revision
                        || !manager.draft.as_ref().is_some_and(|profile| {
                            profile.id == pending.id && profile.remember_password
                        })
                    {
                        continue;
                    }
                    match response.result {
                        Ok(Some(password)) => {
                            manager.password = password;
                            manager.password_revision = manager.password_revision.wrapping_add(1);
                        }
                        Ok(None) => manager.error = Some(
                            "Saved password was not found. Enter it again to use this connection."
                                .into(),
                        ),
                        Err(error) => {
                            manager.error = Some(format!(
                                "Cannot read the secure credential store: {error}. Enter the password for this session."
                            ))
                        }
                    }
                }
                CredentialKind::Write | CredentialKind::Delete => {
                    if manager.credential_latest.get(&pending.id) != Some(&pending.token) {
                        continue;
                    }
                    if let Err(error) = response.result {
                        if matches!(pending.kind, CredentialKind::Write) {
                            if let Some(profile) = self
                                .prefs
                                .saved_connections
                                .profiles
                                .iter_mut()
                                .find(|profile| profile.id == pending.id)
                            {
                                profile.remember_password = false;
                                profiles_changed = true;
                            }
                            if let Some(draft) = &mut manager.draft
                                && draft.id == pending.id
                                && pending.generation == manager.generation
                            {
                                draft.remember_password = false;
                            }
                            manager.error = Some(format!(
                                "Password remains session-only; the secure store could not save it: {error}"
                            ));
                        } else {
                            manager.credential_delete_retry.insert(pending.id);
                            manager.error = Some(format!(
                                "The secure store could not remove the saved password: {error}. Save again with Remember password off to retry."
                            ));
                        }
                    } else if matches!(pending.kind, CredentialKind::Delete) {
                        manager.credential_delete_retry.remove(&pending.id);
                    }
                }
            }
        }
        if profiles_changed {
            self.profiles_changed();
        }
    }

    pub(super) fn connections_window(&mut self, ctx: &egui::Context) {
        if !self.connections {
            return;
        }
        if self.child_window {
            self.connections = false;
            self.manager_requested = true;
            return;
        }
        self.poll_credentials();
        if !self.connection_manager.credential_pending.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.connection_manager.poll_probe();
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new("connection-probe-status"),
                self.connection_manager.probe_status.clone(),
            )
        });
        let mut action = None;
        let mut keyboard_selection = None;
        let mut keyboard_connect = false;
        let mut pick = None;
        let mut test = false;
        let mut cancel_test = false;
        let mut ports_valid = true;
        let panel = egui::CentralPanel::default()
            .show(ctx, |ui| {
                let mut content = ui.available_rect_before_wrap();
                let width = content.width().min(1000.0);
                content.min.x += (content.width() - width) * 0.5;
                content.max.x = content.min.x + width;
                ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
                ui.spacing_mut().button_padding = egui::vec2(10.0, 5.0);
                ui.visuals_mut().widgets.inactive.corner_radius = egui::CornerRadius::same(8);
                ui.visuals_mut().widgets.hovered.corner_radius = egui::CornerRadius::same(8);
                ui.visuals_mut().widgets.active.corner_radius = egui::CornerRadius::same(8);
                ui.horizontal(|ui| {
                    ui.menu_button("+", |ui| {
                        if ui.button("SQLite connection").clicked() { action = Some(Action::New(false)); ui.close(); }
                        if ui.button("PostgreSQL connection").clicked() { action = Some(Action::New(true)); ui.close(); }
                        ui.separator();
                        if ui.add_enabled(self.pending.is_none(), egui::Button::new("Studio demo")).clicked() {
                            self.window_requests.push(crate::session_windows::WindowRequest { title: "Studio demo".into(), recover_sql: false, identity: None, connection: crate::session_windows::WindowConnection::Demo });
                            ui.close();
                        }
                        if !self.window_recovery.is_empty() {
                            ui.separator();
                            ui.checkbox(&mut self.recover_next_window, format!("Restore {} recovered SQL drafts on next connection", self.window_recovery.len()))
                                .on_hover_text("Drafts are restored without executing SQL.");
                        }
                    }).response.on_hover_text("New connection");
                    if ui.button("New group").clicked() { action = Some(Action::AddGroup); }
                    if let Some(id) = self.connection_search_navigation(ctx) {
                        action = Some(Action::Select(id));
                        keyboard_selection = Some(id);
                    }
                    if keyboard_selection.is_none()
                        && ctx.memory(|m| m.has_focus(egui::Id::new("connection-search")))
                        && !egui::Popup::is_any_open(ctx)
                        && ctx.input_mut(|input| input.modifiers.is_none()
                            && input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                        let manager = &self.connection_manager;
                        keyboard_connect = manager.draft.as_ref().is_some_and(|draft| {
                            self.prefs.saved_connections.matching_profiles(&manager.search).contains(&draft.id)
                                && (!manager.search.trim().is_empty() || !manager.collapsed_groups.contains(&draft.group))
                        });
                    }
                    connection_search(ui, &mut self.connection_manager.search);
                });
                if let Some(error) = self.error.clone() {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                        if ui.small_button("Dismiss").clicked() { self.error = None; }
                    });
                }
                if let Some(error) = &self.connection_manager.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                ui.separator();
                ui.horizontal_top(|ui| {
                    let list_width = if self.connection_manager.draft.is_some() { (ui.available_width() * 0.33).clamp(240.0, 320.0) } else { ui.available_width() };
                    ui.allocate_ui_with_layout(egui::vec2(list_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    // Reserve the split even when filtering leaves no rows to fill it.
                    ui.set_width(list_width);
                    ui.set_min_height(400.0);
                    let filtering = !self.connection_manager.search.trim().is_empty();
                    let visible: std::collections::HashSet<_> = self.prefs.saved_connections.matching_profiles(&self.connection_manager.search).into_iter().collect();
                    if filtering { ui.small("Clear search to reorder connections or groups."); }
                    egui::ScrollArea::vertical().id_salt("saved-connection-list").max_height((ctx.content_rect().height() - 110.0).max(200.0)).show(ui, |ui| {
                        let groups: Vec<_> = std::iter::once((None, "Ungrouped".to_owned()))
                            .chain(self.prefs.saved_connections.groups.iter().map(|group| (Some(group.id), group.name.clone()))).collect();
                        if visible.is_empty() && filtering { ui.weak("No matching connections."); }
                        if self.prefs.saved_connections.profiles.is_empty() && !filtering {
                            ui.add_space(18.0);
                            ui.heading("Your connections");
                            ui.weak("Use + to add a database connection.");
                            ui.add_space(12.0);
                        }
                        for (group_index, (group_id, name)) in groups.into_iter().enumerate() {
                            if filtering && !name.to_lowercase().contains(&self.connection_manager.search.trim().to_lowercase())
                                && !self.prefs.saved_connections.in_group(group_id).any(|profile| visible.contains(&profile.id)) { continue; }
                            let collapsed = self.connection_manager.collapsed_groups.contains(&group_id) && !filtering;

                            let editing = self.connection_manager.rename_group.as_ref().is_some_and(|(id, _)| Some(*id) == group_id);
                            let frame = egui::Frame::new().inner_margin(egui::Margin { left: 24, right: 4, top: 7, bottom: 5 });
                            let header = frame.show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                if editing {
                                    let (id, value) = self.connection_manager.rename_group.as_mut().unwrap();
                                    let response = ui.add(TextEdit::singleline(value).id(egui::Id::new(("rename-connection-group", id.0))).desired_width(ui.available_width()));
                                    if self.connection_manager.focus_group_name {
                                        self.connection_manager.focus_group_name = false;
                                        response.request_focus();
                                        response.scroll_to_me(Some(egui::Align::Center));
                                        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
                                            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(value.chars().count()))));
                                            state.store(ui.ctx(), response.id);
                                        }
                                    }
                                    let escape = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                                    let enter = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                                    if escape { self.connection_manager.rename_group = None; response.surrender_focus(); }
                                    else if enter || response.lost_focus() {
                                        action = Some(Action::RenameGroup(*id, value.clone()));
                                    }
                                    response
                                } else if let Some(id) = group_id.filter(|_| !filtering) {
                                    ui.dnd_drag_source(egui::Id::new(("connection-group", id.0)), Drag::Group(id), |ui| ui.strong(&name)).response
                                } else { ui.strong(&name) }

                            });
                            if header.response.contains_pointer() && egui::DragAndDrop::has_payload_of_type::<Drag>(ui.ctx()) {
                                ui.painter().rect_stroke(header.response.rect, 4.0, ui.visuals().selection.stroke, egui::StrokeKind::Inside);
                            }
                            let dropped = header.response.dnd_release_payload::<Drag>();
                            if let Some(dropped) = dropped.filter(|_| !filtering) {
                                match *dropped {
                                    Drag::Profile(id) => action = Some(Action::Move(id, group_id, usize::MAX)),
                                    Drag::Group(id) => action = Some(Action::MoveGroup(id, group_index.saturating_sub(1))),
                                }
                            }
                            ui.painter().text(egui::pos2(header.response.rect.left() + 10.0, header.response.rect.center().y), egui::Align2::CENTER_CENTER, if collapsed { "›" } else { "⌄" }, egui::FontId::proportional(14.0), ui.visuals().weak_text_color());
                            if !editing {
                            let heading_response = header.inner.interact(egui::Sense::click()) | header.response.interact(egui::Sense::click());
                            if heading_response.clicked() {
                                heading_response.request_focus();
                                if collapsed { self.connection_manager.collapsed_groups.remove(&group_id); } else { self.connection_manager.collapsed_groups.insert(group_id); }
                            }
                            if let Some(id) = group_id {
                                connection_menu(&heading_response, heading_response.has_focus()).show(|ui| {
                                    ui.add_enabled_ui(!filtering, |ui| {
                                        let index = group_index.saturating_sub(1);
                                        if connection_menu_action(ui, index > 0, "Move up").clicked() { action = Some(Action::MoveGroup(id, index - 1)); ui.close(); }
                                        if connection_menu_action(ui, index + 1 < self.prefs.saved_connections.groups.len(), "Move down").clicked() { action = Some(Action::MoveGroup(id, index + 1)); ui.close(); }
                                    });
                                    ui.separator();
                                    if connection_menu_action(ui, true, "Rename group").clicked() { self.connection_manager.rename_group = Some((id, name.clone())); self.connection_manager.focus_group_name = true; ui.close(); }
                                    if ui.button("Delete group · keep connections").clicked() { action = Some(Action::DeleteGroup(id)); ui.close(); }
                                });
                            }
                            }
                            if collapsed { continue; }
                            for (index, profile) in self.prefs.saved_connections.in_group(group_id).enumerate() {
                                if !visible.contains(&profile.id) { continue; }
                                let selected = self.connection_manager.draft.as_ref().is_some_and(|draft| draft.id == profile.id);
                                let render_row = |ui: &mut egui::Ui| connection_row(ui, profile, selected);
                                let response = if filtering { ui.scope(render_row) } else { ui.dnd_drag_source(egui::Id::new(("saved-connection", profile.id.0)), Drag::Profile(profile.id), render_row) };
                                let row_response = response.response.interact(egui::Sense::click()) | response.inner.clone();
                                if response.inner.clicked() || row_response.clicked() { row_response.request_focus(); action = Some(Action::Select(profile.id)); }
                                if !filtering && let Some(dropped) = response.response.dnd_release_payload::<Drag>()
                                    && let Drag::Profile(id) = *dropped {
                                    let after = ui.input(|input| input.pointer.interact_pos()).is_some_and(|pos| pos.y > response.response.rect.center().y);
                                    let old = self.prefs.saved_connections.in_group(group_id).position(|profile| profile.id == id);
                                    let mut target = index + usize::from(after);
                                    if old.is_some_and(|old| old < target) { target = target.saturating_sub(1); }
                                    action = Some(Action::Move(id, group_id, target));
                                }
                                if self.connection_manager.scroll_to_profile == Some(profile.id) {
                                    row_response.scroll_to_me(Some(egui::Align::Center));
                                    self.connection_manager.scroll_to_profile = None;
                                }
                                let keyboard_menu = row_response.has_focus() || (selected && ctx.memory(|m| m.has_focus(egui::Id::new("connection-search"))));
                                connection_menu(&row_response, keyboard_menu).show(|ui| {
                                    ui.add_enabled_ui(!filtering, |ui| {
                                        if connection_menu_action(ui, index > 0, "Move up").clicked() { action = Some(Action::Move(profile.id, group_id, index - 1)); ui.close(); }
                                        if connection_menu_action(ui, index + 1 < self.prefs.saved_connections.in_group(group_id).count(), "Move down").clicked() { action = Some(Action::Move(profile.id, group_id, index + 1)); ui.close(); }
                                        let destinations = group_id.is_some() || !self.prefs.saved_connections.groups.is_empty();
                                        let move_button = connection_menu_action(ui, destinations, "Move to group ⏵");
                                        if move_button.clicked() && move_button.has_focus() {
                                            ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("connection-destination-needs-focus"), true));
                                        }
                                        egui::containers::menu::SubMenu::default().show(ui, &move_button, |ui| {
                                            if connection_destination_action(ui, group_id.is_some(), "Ungrouped").clicked() { action = Some(Action::Move(profile.id, None, usize::MAX)); ui.close(); }
                                            for group in &self.prefs.saved_connections.groups {
                                                ui.push_id(group.id, |ui| {
                                                    if connection_destination_action(ui, group_id != Some(group.id), &group.name).clicked() { action = Some(Action::Move(profile.id, Some(group.id), usize::MAX)); ui.close(); }
                                                });
                                            }
                                        });
                                    });
                                    ui.separator();
                                    if connection_menu_action(ui, true, "Duplicate").clicked() { action = Some(Action::Duplicate(profile.id)); ui.close(); }
                                    if ui.button("Remove saved connection").clicked() { action = Some(Action::Delete(profile.id)); ui.close(); }
                                });
                            }
                            ui.add_space(6.0);
                        }
                    });
                    });
                    if self.connection_manager.draft.is_none() { return; }
                    ui.separator();
                    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), (ctx.content_rect().height() - 120.0).clamp(220.0, 900.0)), egui::Layout::top_down(egui::Align::Min), |ui| {
                    let Some(draft) = &mut self.connection_manager.draft else {
                        ui.weak("Choose a saved connection or add one.");
                        ui.add_space(12.0);
                        ui.weak("Drag connections onto group headings to organize them. Drag within a group to reorder.");
                        return;
                    };
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);
                    ui.spacing_mut().scroll.floating = false;
                    ui.spacing_mut().scroll.bar_inner_margin = 8.0;
                    ui.spacing_mut().interact_size.y = 28.0;
                    ui.visuals_mut().widgets.inactive.corner_radius = egui::CornerRadius::same(6);
                    ui.visuals_mut().widgets.hovered.corner_radius = egui::CornerRadius::same(6);
                    let form_height = (ctx.content_rect().height() - 160.0).clamp(180.0, 850.0);
                    let form = egui::ScrollArea::vertical().id_salt("connection-form").max_height(form_height).auto_shrink([false, true]).show(ui, |ui| {
                        let title = match draft.connection { ConnectionKind::SQLite { .. } => "SQLite Connection", ConnectionKind::Postgres(_) => "PostgreSQL Connection" };
                        ui.heading(title);
                        ui.add_space(8.0);
                        form_card(ui).show(ui, |ui| {
                        form_text(ui, "Name", &mut draft.name, false, None);
                        form_row(ui, "Group", |ui| {
                            egui::ComboBox::from_id_salt("profile-group").width(ui.available_width()).selected_text(draft.group.and_then(|id| self.prefs.saved_connections.groups.iter().find(|group| group.id == id)).map_or("Ungrouped", |group| group.name.as_str())).show_ui(ui, |ui| {
                                ui.selectable_value(&mut draft.group, None, "Ungrouped");
                                for group in &self.prefs.saved_connections.groups { ui.selectable_value(&mut draft.group, Some(group.id), &group.name); }
                            });
                        });
                        form_row(ui, "Environment", |ui| {
                            egui::ComboBox::from_id_salt("connection-environment").width((ui.available_width() - ui.spacing().interact_size.x - ui.spacing().item_spacing.x).max(80.0)).selected_text(&draft.environment).show_ui(ui, |ui| {
                                for name in ["Local", "Development", "Testing", "Staging", "Production"] { ui.selectable_value(&mut draft.environment, name.to_owned(), name); }
                            });
                            ui.color_edit_button_srgb(&mut draft.color).on_hover_text("Connection color");
                        });
                        });
                        ui.add_space(8.0);
                        form_card(ui).show(ui, |ui| {
                        form_section(ui, "Database");
                        match &mut draft.connection {
                            ConnectionKind::SQLite { path } => {
                                status_row(ui, "SQLite file", Some(ProbeField::File), |ui| {
                                    ui.add(egui::Label::new(path.file_name().unwrap_or_default().to_string_lossy()).truncate()).on_hover_text(path.display().to_string());
                                });
                                form_row(ui, "", |ui| { if ui.button("Choose file…").clicked() { pick = Some(false); } });
                            }
                            ConnectionKind::Postgres(config) => {
                                if ui.available_width() >= 500.0 {
                                    ui.horizontal(|ui| {
                                        let host_width = ui.available_width() - 192.0;
                                        ui.allocate_ui_with_layout(egui::vec2(host_width, 28.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                                            form_text(ui, "Host", &mut config.host, false, Some(ProbeField::Host));
                                        });
                                        ports_valid &= form_port(ui, &mut config.port, ProbeField::Port, self.connection_manager.generation);
                                    });
                                } else {
                                    form_text(ui, "Host", &mut config.host, false, Some(ProbeField::Host));
                                    ports_valid &= form_port(ui, &mut config.port, ProbeField::Port, self.connection_manager.generation);
                                }
                                form_text(ui, "Database", &mut config.database, false, Some(ProbeField::Database));
                                form_text(ui, "User", &mut config.user, false, Some(ProbeField::User));
                                if form_text(ui, "Password", &mut self.connection_manager.password, true, Some(ProbeField::Password)) { self.connection_manager.password_revision = self.connection_manager.password_revision.wrapping_add(1); }
                                form_row(ui, "", |ui| { form_checkbox(ui, &mut draft.remember_password, "Remember password").on_hover_text("Save in the system credential store when you Save or Open. Turning this off and saving removes the stored password."); });
                                form_section(ui, "Security");
                                status_row(ui, "TLS", Some(ProbeField::Tls), |ui| {
                                    egui::ComboBox::from_id_salt("profile-tls").width(ui.available_width()).selected_text(if config.tls { "Verify full" } else { "Disabled" }).show_ui(ui, |ui| {
                                        ui.selectable_value(&mut config.tls, true, "Verify full");
                                        ui.selectable_value(&mut config.tls, false, "Disabled (unencrypted)");
                                    });
                                });
                                let mut ssh = config.ssh.is_some();
                                form_row(ui, "SSH tunnel", |ui| { if form_checkbox(ui, &mut ssh, "Enabled").changed() { config.ssh = ssh.then(SshOptions::default); } });
                                if let Some(ssh) = &mut config.ssh {
                                    form_section(ui, "SSH gateway");
                                    form_text(ui, "Host", &mut ssh.host, false, Some(ProbeField::SshHost));
                                    status_row(ui, "Port", Some(ProbeField::SshPort), |ui| {
                                        let mut inherit = ssh.port == 0;
                                        if form_checkbox(ui, &mut inherit, "Use SSH config").changed() { ssh.port = if inherit { 0 } else { 22 }; }
                                    });
                                    if ssh.port != 0 { ports_valid &= form_port(ui, &mut ssh.port, ProbeField::SshPort, self.connection_manager.generation); }
                                    form_text(ui, "User", &mut ssh.user, false, Some(ProbeField::SshUser));
                                    status_row(ui, "Identity", Some(ProbeField::SshIdentity), |ui| {
                                        let mut use_agent = ssh.identity_file.is_none();
                                        if form_checkbox(ui, &mut use_agent, "Use config / SSH agent").changed() {
                                            if use_agent { ssh.identity_file = None; } else { pick = Some(true); }
                                        }
                                    });
                                    form_row(ui, "", |ui| {
                                        if ui.button("Choose key…").clicked() { pick = Some(true); }
                                        if let Some(path) = &ssh.identity_file {
                                            ui.add(egui::Label::new(path.file_name().unwrap_or_default().to_string_lossy()).truncate()).on_hover_text(path.display().to_string());
                                        }
                                    });
                                    ui.add(egui::Label::new(RichText::new("Host aliases use ~/.ssh/config. Blank user and automatic port inherit configuration. Selected keys are tried alongside configured keys. Gateway must already be trusted.").weak().small()).wrap());
                                }
                            }
                        }
                        });
                    });
                    #[cfg(test)]
                    ctx.data_mut(|data| data.insert_temp(egui::Id::new("connection-form-geometry"), (form.content_size, form.inner_rect)));
                    #[cfg(not(test))]
                    let _ = form;
                    ui.separator();
                    if let Some(message) = &self.connection_manager.probe_message {
                        ui.add(egui::Label::new(RichText::new(message).small()).truncate()).on_hover_text(message);
                    }
                    let valid = draft.validate().is_ok() && ports_valid;
                    if !ports_valid { ui.colored_label(ui.visuals().error_fg_color, "Port must be between 1 and 65535."); }
                    let password_loading = self.connection_manager.credential_pending.iter().any(|pending| pending.generation == self.connection_manager.generation && matches!(pending.kind, CredentialKind::Read(revision) if revision == self.connection_manager.password_revision));
                    if password_loading { ui.weak("Reading saved password…"); }
                    ui.horizontal(|ui| {
                        if ui.button("Discard").clicked() { action = Some(Action::Discard); }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_connect = valid && !password_loading && self.pending.is_none();
                        if ui.add_enabled(can_connect, egui::Button::new("Connect").fill(ui.visuals().selection.bg_fill))
                            .on_hover_text("Save and open connection · Enter from connection search").clicked()
                            || (can_connect && keyboard_connect) { action = Some(Action::Save(true)); }
                        if self.connection_manager.probe.is_some() {
                            if ui.button("Cancel test").clicked() { cancel_test = true; }
                        } else if ui.add_enabled(valid && !password_loading, egui::Button::new("Test")).clicked() { test = true; }
                        if ui.add_enabled(valid && !password_loading, egui::Button::new("Save")).clicked() { action = Some(Action::Save(false)); }
                        });
                    });
                    });
                });
                });
            });
        #[cfg(test)]
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new("connection-manager-geometry"),
                panel.response.rect,
            )
        });
        #[cfg(not(test))]
        let _ = panel;
        // Typing or changing transport invalidates highlights immediately and cancels the old worker.
        self.connection_manager.poll_probe();
        if !ports_valid {
            self.connection_manager.invalidate_probe();
        }
        if cancel_test {
            self.connection_manager.invalidate_probe();
        }
        if test {
            self.connection_manager.start_probe(ctx);
        }
        #[cfg(target_arch = "wasm32")]
        if pick.is_some() { self.error = Some("Connection file selection requires the desktop app.".into()); }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(key) = pick {
            let generation = self.connection_manager.generation;
            let tx = self.files_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let dialog = rfd::FileDialog::new();
                let dialog = if key {
                    dialog
                } else {
                    dialog
                        .add_filter("SQLite database", &["db", "sqlite", "sqlite3"])
                        .add_filter("All files", &["*"])
                };
                if let Some(path) = dialog.pick_file() {
                    let _ = tx.send(FileAction::ProfilePath(generation, key, path));
                    ctx.request_repaint();
                }
            });
        }
        if let Some(action) = action {
            let result = self.connection_action(action);
            if result.is_ok() && keyboard_selection.is_some()
                && self.connection_manager.draft.as_ref().map(|draft| draft.id) == keyboard_selection {
                self.connection_manager.scroll_to_profile = keyboard_selection;
            }
            if let Err(error) = result {
                self.connection_manager.error = Some(error);
            }
        }
    }

    fn connection_search_navigation(&self, ctx: &egui::Context) -> Option<ProfileId> {
        if !ctx.memory(|m| m.has_focus(egui::Id::new("connection-search")))
            || egui::Popup::is_any_open(ctx) { return None; }
        let forward = ctx.input_mut(|input| {
            if !input.modifiers.is_none() { return None; }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) { Some(true) }
            else if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) { Some(false) }
            else { None }
        })?;
        let manager = &self.connection_manager;
        let store = &self.prefs.saved_connections;
        let filtering = !manager.search.trim().is_empty();
        let matches: std::collections::HashSet<_> = store.matching_profiles(&manager.search).into_iter().collect();
        let ordered: Vec<_> = std::iter::once(None)
            .chain(store.groups.iter().map(|group| Some(group.id)))
            .filter(|group| filtering || !manager.collapsed_groups.contains(group))
            .flat_map(|group| store.in_group(group))
            .filter(|profile| matches.contains(&profile.id))
            .map(|profile| profile.id).collect();
        let current = manager.draft.as_ref().and_then(|draft| ordered.iter().position(|id| *id == draft.id));
        let index = match current {
            Some(index) if forward => index.saturating_add(1).min(ordered.len().saturating_sub(1)),
            Some(index) => index.saturating_sub(1),
            None if forward => 0,
            None => ordered.len().saturating_sub(1),
        };
        ordered.get(index).copied()
    }

    fn connection_action(&mut self, action: Action) -> Result<(), String> {
        self.connection_manager.error = None;
        match action {
            Action::Select(id) => {
                if self
                    .connection_manager
                    .draft
                    .as_ref()
                    .is_some_and(|draft| draft.id == id)
                {
                    return Ok(());
                }
                if self.connection_manager.dirty(&self.prefs.saved_connections) {
                    return Err("Save or discard the connection form before switching.".into());
                }
                let profile = self
                    .prefs
                    .saved_connections
                    .profile(id)
                    .ok_or("Connection no longer exists.")?
                    .clone();
                self.connection_manager.select(profile);
            }
            Action::New(postgres) => {
                if self.connection_manager.dirty(&self.prefs.saved_connections) {
                    return Err("Save or discard the connection form before adding another.".into());
                }
                self.connection_manager.select(Profile::draft(
                    String::new(),
                    if postgres {
                        ConnectionKind::Postgres(PostgresProfile::default())
                    } else {
                        ConnectionKind::SQLite {
                            path: PathBuf::new(),
                        }
                    },
                ));
            }
            Action::OpenWindow => {
                self.connection_action(Action::Save(false))?;
                let profile = self
                    .connection_manager
                    .draft
                    .as_ref()
                    .ok_or("Choose a connection first.")?;
                let connection = match &profile.connection {
                    ConnectionKind::SQLite { path } => {
                        crate::session_windows::WindowConnection::Sqlite(path.clone())
                    }
                    ConnectionKind::Postgres(config) => {
                        crate::session_windows::WindowConnection::Postgres(Box::new(
                            config.with_password(self.connection_manager.password.clone()),
                        ))
                    }
                };
                self.window_requests
                    .push(crate::session_windows::WindowRequest {
                        title: profile.name.clone(),
                        recover_sql: false,
                        identity: Some(ProfileIdentity::from(profile)),
                        connection,
                    });
            }
            Action::Save(open) => {
                if open {
                    return self.connection_action(Action::OpenWindow);
                }
                if self.connection_manager.loading_password() {
                    return Err(
                        "Wait for the saved password, or enter a password for this session.".into(),
                    );
                }
                let mut profile = self
                    .connection_manager
                    .draft
                    .clone()
                    .ok_or("Choose a connection first.")?;
                let was_remembered = self
                    .prefs
                    .saved_connections
                    .profile(profile.id)
                    .is_some_and(|profile| profile.remember_password);
                if profile.id.0 == 0 {
                    profile.id = self.prefs.saved_connections.add_profile(profile.clone())?;
                } else {
                    self.prefs
                        .saved_connections
                        .update_profile(profile.clone())?;
                }
                self.connection_manager.draft =
                    self.prefs.saved_connections.profile(profile.id).cloned();
                self.profiles_changed();
                if matches!(profile.connection, ConnectionKind::Postgres(_)) {
                    if profile.remember_password {
                        self.connection_manager.queue_credential(
                            profile.id,
                            CredentialOperation::Write(self.connection_manager.password.clone()),
                        );
                    } else if was_remembered
                        || self
                            .connection_manager
                            .credential_delete_retry
                            .contains(&profile.id)
                    {
                        self.connection_manager
                            .queue_credential(profile.id, CredentialOperation::Delete);
                    }
                }
            }
            Action::Discard => {
                let saved = self
                    .connection_manager
                    .draft
                    .as_ref()
                    .and_then(|draft| self.prefs.saved_connections.profile(draft.id))
                    .cloned();
                if let Some(saved) = saved {
                    self.connection_manager.select(saved);
                } else {
                    self.connection_manager.draft = None;
                    self.connection_manager.password.clear();
                    self.connection_manager.generation =
                        self.connection_manager.generation.wrapping_add(1);
                }
            }
            Action::Duplicate(id) => {
                if self.connection_manager.dirty(&self.prefs.saved_connections) {
                    return Err("Save or discard the connection form before duplicating.".into());
                }
                let new_id = self.prefs.saved_connections.duplicate_profile(id)?;
                let copy = self
                    .prefs
                    .saved_connections
                    .profile(new_id)
                    .expect("created duplicate")
                    .clone();
                self.connection_manager.select(copy);
                self.profiles_changed();
            }
            Action::Delete(id) => {
                let removed = self.prefs.saved_connections.remove_profile(id)?;
                if removed.remember_password
                    || self.connection_manager.credential_latest.contains_key(&id)
                {
                    self.connection_manager
                        .queue_credential(id, CredentialOperation::Delete);
                }
                self.profiles_changed();
                if self
                    .connection_manager
                    .draft
                    .as_ref()
                    .is_some_and(|draft| draft.id == id)
                {
                    self.connection_manager.draft = None;
                    self.connection_manager.password.clear();
                }
            }
            Action::AddGroup => {
                let mut name = "New group".to_owned();
                let mut suffix = 2;
                while self.prefs.saved_connections.groups.iter().any(|group| group.name == name) {
                    name = format!("New group {suffix}"); suffix += 1;
                }
                let id = self.prefs.saved_connections.add_group(name.clone())?;
                self.connection_manager.search.clear();
                self.connection_manager.rename_group = Some((id, name));
                self.connection_manager.focus_group_name = true;
            }
            Action::RenameGroup(id, name) => {
                self.prefs.saved_connections.rename_group(id, name)?;
                self.connection_manager.rename_group = None;
            }
            Action::DeleteGroup(id) => {
                self.prefs.saved_connections.remove_group(id)?;
                if let Some(draft) = &mut self.connection_manager.draft
                    && draft.group == Some(id)
                {
                    draft.group = None;
                }
            }
            Action::Move(id, group, index) => {
                if !self.connection_manager.search.trim().is_empty() {
                    return Err("Clear search before reordering connections.".into());
                }
                self.prefs
                    .saved_connections
                    .move_profile(id, group, index)?;
                if let Some(draft) = &mut self.connection_manager.draft
                    && draft.id == id
                {
                    draft.group = group;
                }
            }
            Action::MoveGroup(id, index) => {
                if !self.connection_manager.search.trim().is_empty() {
                    return Err("Clear search before reordering groups.".into());
                }
                self.prefs.saved_connections.move_group(id, index)?;
            }
        }
        Ok(())
    }
}

/// Shift+F10 anchors to the focused row rather than the last pointer position.
fn connection_menu(response: &egui::Response, keyboard_eligible: bool) -> egui::Popup<'_> {
    let ctx = &response.ctx;
    let key = response.id.with("keyboard-context-menu");
    let requested = keyboard_eligible && !egui::Popup::is_any_open(ctx)
        && ctx.input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, egui::Key::F10));
    if requested || response.secondary_clicked() {
        ctx.data_mut(|data| data.insert_temp(key, requested));
    }
    let mut popup = egui::Popup::context_menu(response);
    if ctx.data(|data| data.get_temp::<bool>(key).unwrap_or(false)) {
        popup = popup.anchor(response);
    }
    if requested {
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("connection-menu-needs-focus"), true));
        popup = popup.open_memory(egui::SetOpenCommand::Bool(true));
    }
    popup
}

fn connection_menu_action(ui: &mut egui::Ui, enabled: bool, label: &str) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new(label));
    if response.enabled() && ui.ctx().data_mut(|data| data.remove_temp::<bool>(egui::Id::new("connection-menu-needs-focus")).unwrap_or(false)) {
        response.request_focus();
    }
    response
}

fn connection_destination_action(ui: &mut egui::Ui, enabled: bool, label: &str) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new(label));
    // A submenu's first sizing pass is disabled. Keep this request separate from
    // its parent's focus request so later parent actions cannot consume it.
    if response.enabled() && ui.ctx().data_mut(|data| data.remove_temp::<bool>(egui::Id::new("connection-destination-needs-focus")).unwrap_or(false)) {
        response.request_focus();
    }
    response
}

/// One hit target per connection: the badge, title and endpoint select the same row.
fn connection_row(ui: &mut egui::Ui, profile: &Profile, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 55.0), egui::Sense::click());
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            8.0,
            if selected {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.hovered.weak_bg_fill
            },
        );
    }
    let (badge, color, endpoint) = match &profile.connection {
        ConnectionKind::SQLite { path } => (
            "Sl",
            Color32::from_rgb(137, 72, 224),
            path.display().to_string(),
        ),
        ConnectionKind::Postgres(config) => (
            "Pg",
            Color32::from_rgb(0, 141, 204),
            match &config.ssh {
                Some(ssh) => format!("SSH: {} · {}", ssh.host, config.database),
                None => format!("{} · {}", config.host, config.database),
            },
        ),
    };
    let center = egui::pos2(rect.left() + 23.0, rect.center().y);
    ui.painter().circle_filled(center, 16.0, color);
    ui.painter().text(
        center,
        egui::Align2::CENTER_CENTER,
        badge,
        egui::FontId::proportional(14.0),
        Color32::WHITE,
    );
    let left = rect.left() + 47.0;
    let width = (rect.right() - left - 8.0).max(1.0);
    let foreground = if selected {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().text_color()
    };
    let title = ui.painter().layout_no_wrap(
        profile.name.clone(),
        egui::FontId::proportional(14.0),
        foreground,
    );
    let painter = ui.painter().with_clip_rect(egui::Rect::from_min_max(
        egui::pos2(left, rect.top()),
        rect.max,
    ));
    painter.galley(
        egui::pos2(left, rect.top() + 7.0),
        title.clone(),
        foreground,
    );
    let tag_x = left + title.size().x + 8.0;
    let tag = ui.painter().layout_no_wrap(
        profile.environment.to_lowercase(),
        egui::FontId::proportional(10.0),
        foreground,
    );
    if tag_x + tag.size().x + 10.0 < rect.right() {
        let tag_rect = egui::Rect::from_min_size(
            egui::pos2(tag_x, rect.top() + 8.0),
            tag.size() + egui::vec2(8.0, 4.0),
        );
        let tint = Color32::from_rgb(profile.color[0], profile.color[1], profile.color[2]);
        painter.rect_filled(tag_rect, 4.0, tint.gamma_multiply(0.3));
        painter.galley(tag_rect.min + egui::vec2(4.0, 2.0), tag, foreground);
    }
    let mut job = egui::text::LayoutJob::simple(
        endpoint.clone(),
        egui::FontId::proportional(12.0),
        if selected {
            foreground.gamma_multiply(0.8)
        } else {
            ui.visuals().weak_text_color()
        },
        width,
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    painter.galley(egui::pos2(left, rect.top() + 30.0), galley, foreground);
    response.on_hover_text(endpoint)
}

fn connection_search(ui: &mut egui::Ui, text: &mut String) {
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let editor_width = (ui.available_width() - 14.0 - 18.0 - 8.0).max(0.0);
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(14.0, 20.0), egui::Sense::hover());
                let center = rect.center() - egui::vec2(1.5, 1.5);
                let stroke = Stroke::new(1.3_f32, ui.visuals().weak_text_color());
                ui.painter().circle_stroke(center, 4.5, stroke);
                ui.painter().line_segment(
                    [center + egui::vec2(3.2, 3.2), center + egui::vec2(6.5, 6.5)],
                    stroke,
                );
                let search_response = ui.add_sized(
                    [editor_width, 20.0],
                    TextEdit::singleline(text)
                        .id(egui::Id::new("connection-search"))
                        .frame(false)
                        .margin(egui::Margin::ZERO)
                        .hint_text("Search connections…")
                        .desired_width(editor_width),
                );
                if search_response.has_focus() {
                    ui.ctx().memory_mut(|memory| memory.set_focus_lock_filter(search_response.id,
                        egui::EventFilter { vertical_arrows: true, horizontal_arrows: true, ..Default::default() }));
                }
                // Reserve this slot even while empty. Neither text-edit margins nor
                // button padding may grow a column/window during repeated layout.
                let (clear_rect, response) = ui.allocate_exact_size(
                    egui::vec2(18.0, 20.0),
                    if text.is_empty() {
                        egui::Sense::hover()
                    } else {
                        egui::Sense::click()
                    },
                );
                if !text.is_empty() {
                    let center = clear_rect.center();
                    for direction in [-1.0, 1.0] {
                        ui.painter().line_segment(
                            [
                                center + egui::vec2(-3.0, -3.0 * direction),
                                center + egui::vec2(3.0, 3.0 * direction),
                            ],
                            stroke,
                        );
                    }
                    if response.on_hover_text("Clear search").clicked() {
                        text.clear();
                    }
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Storage(std::collections::HashMap<String, String>);
    impl eframe::Storage for Storage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn flush(&mut self) {}
    }
    fn app() -> Rustable {
        let mut app = Rustable::with_preferences(&egui::Context::default(), Preferences::default());
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while app.pending.is_some() && Instant::now() < deadline {
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.pending.is_none());
        app
    }
    #[test]
    fn port_text_is_compact_left_aligned_and_invalid_input_is_retained() {
        let ctx = egui::Context::default();
        let mut host = "localhost".to_owned();
        let mut port = 5432;
        let mut host_rect = egui::Rect::NOTHING;
        let mut port_rect = egui::Rect::NOTHING;
        let mut port_id = egui::Id::NULL;
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_max_width(400.0);
                ui.spacing_mut().interact_size.y = 28.0;
                host_rect = ui
                    .scope(|ui| form_text(ui, "Host", &mut host, false, Some(ProbeField::Host)))
                    .response
                    .rect;
                port_rect = ui
                    .scope(|ui| {
                        port_id = ui.make_persistent_id((
                            "connection-port-text",
                            format!("{:?}", ProbeField::Port),
                            1_u64,
                        ));
                        assert!(form_port(ui, &mut port, ProbeField::Port, 1));
                    })
                    .response
                    .rect;
            });
        });
        assert!(
            (host_rect.height() - port_rect.height()).abs() < 1.0,
            "host {host_rect:?}, port {port_rect:?}"
        );
        let number = painted_text(&output, "5432").unwrap();
        assert!(
            number.x < port_rect.left() + 135.0,
            "numeric value should sit at the left input edge"
        );
        assert!(port_rect.width() <= 185.0, "port input should be compact");
        ctx.data_mut(|data| {
            data.insert_temp(
                port_id,
                PortEdit {
                    text: "999999".into(),
                    committed: 5432,
                },
            )
        });
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.scope(|ui| {
                    form_text(ui, "Host", &mut host, false, Some(ProbeField::Host));
                });
                ui.scope(|ui| {
                    assert!(!form_port(ui, &mut port, ProbeField::Port, 1));
                });
            });
        });
        assert_eq!(
            port, 5432,
            "invalid transient text must not change the connection port"
        );
        assert_eq!(
            ctx.data(|data| data.get_temp::<PortEdit>(port_id))
                .unwrap()
                .text,
            "999999"
        );
    }

    fn settle_credentials(app: &mut Rustable) {
        let start = std::time::Instant::now();
        while !app.connection_manager.credential_pending.is_empty() {
            app.poll_credentials();
            assert!(start.elapsed() < std::time::Duration::from_secs(3));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    #[test]
    fn remembering_and_forgetting_password_uses_secure_worker_and_never_preferences() {
        let mut app = app();
        app.connection_action(Action::New(true)).unwrap();
        app.connection_manager.draft.as_mut().unwrap().name = "Secure profile".into();
        app.connection_manager
            .draft
            .as_mut()
            .unwrap()
            .remember_password = true;
        app.connection_manager.password = "CREDENTIAL_SECRET".into();
        app.connection_action(Action::Save(false)).unwrap();
        settle_credentials(&mut app);
        let profile = app.connection_manager.draft.clone().unwrap();
        let id = profile.id;
        app.connection_manager.select(profile);
        assert!(app.connection_manager.loading_password());
        assert!(app.connection_action(Action::Save(true)).is_err());
        settle_credentials(&mut app);
        assert_eq!(app.connection_manager.password, "CREDENTIAL_SECRET");
        let mut storage = Storage::default();
        app.save_with_window_documents(&mut storage, vec![]);
        assert!(!storage.0["workspace"].contains("CREDENTIAL_SECRET"));
        app.connection_manager
            .draft
            .as_mut()
            .unwrap()
            .remember_password = false;
        app.connection_action(Action::Save(false)).unwrap();
        settle_credentials(&mut app);
        let mut remembered = app.prefs.saved_connections.profile(id).unwrap().clone();
        remembered.remember_password = true; // Explicitly verify the former key was deleted.
        app.connection_manager.select(remembered);
        settle_credentials(&mut app);
        assert!(app.connection_manager.password.is_empty());
        assert!(
            app.connection_manager
                .error
                .as_deref()
                .unwrap()
                .contains("not found")
        );
    }
    struct LockedCredentials;
    impl crate::credential_store::Backend for LockedCredentials {
        fn read(&self, _: &str) -> Result<Option<String>, String> {
            Err("locked".into())
        }
        fn write(&self, _: &str, _: &str) -> Result<(), String> {
            Err("locked".into())
        }
        fn delete(&self, _: &str) -> Result<(), String> {
            Err("locked".into())
        }
    }
    #[test]
    fn unavailable_keychain_keeps_password_session_only_and_delete_failure_is_retryable() {
        let mut app = app();
        app.connection_manager.credentials = Some(CredentialWorker::with_backend(
            std::sync::Arc::new(LockedCredentials),
        ));
        app.connection_action(Action::New(true)).unwrap();
        app.connection_manager.draft.as_mut().unwrap().name = "Locked store".into();
        app.connection_manager
            .draft
            .as_mut()
            .unwrap()
            .remember_password = true;
        app.connection_manager.password = "SESSION_ONLY".into();
        app.connection_action(Action::Save(false)).unwrap();
        settle_credentials(&mut app);
        let id = app.connection_manager.draft.as_ref().unwrap().id;
        assert!(
            !app.prefs
                .saved_connections
                .profile(id)
                .unwrap()
                .remember_password
        );
        assert!(
            !app.connection_manager
                .draft
                .as_ref()
                .unwrap()
                .remember_password
        );
        assert_eq!(app.connection_manager.password, "SESSION_ONLY");
        assert!(
            app.connection_manager
                .error
                .as_deref()
                .unwrap()
                .contains("session-only")
        );
        app.connection_manager
            .queue_credential(id, CredentialOperation::Delete);
        settle_credentials(&mut app);
        assert!(app.connection_manager.credential_delete_retry.contains(&id));
        app.connection_action(Action::Save(false)).unwrap();
        assert!(
            app.connection_manager
                .credential_pending
                .iter()
                .any(|pending| matches!(pending.kind, CredentialKind::Delete))
        );
        settle_credentials(&mut app);
    }
    struct DelayedCredentials {
        release: std::sync::Mutex<bool>,
        ready: std::sync::Condvar,
    }
    impl crate::credential_store::Backend for DelayedCredentials {
        fn read(&self, _: &str) -> Result<Option<String>, String> {
            let mut release = self.release.lock().unwrap();
            while !*release {
                release = self.ready.wait(release).unwrap();
            }
            Ok(Some("OLD_STORED_SECRET".into()))
        }
        fn write(&self, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn delete(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }
    #[test]
    fn delayed_secure_reads_cannot_overwrite_typed_password_or_another_profile() {
        for switch_profile in [false, true] {
            let mut app = app();
            let backend = std::sync::Arc::new(DelayedCredentials {
                release: std::sync::Mutex::new(false),
                ready: std::sync::Condvar::new(),
            });
            app.connection_manager.credentials =
                Some(CredentialWorker::with_backend(backend.clone()));
            let mut profile = Profile::draft(
                "Remembered".into(),
                ConnectionKind::Postgres(PostgresProfile::default()),
            );
            profile.id = ProfileId(123);
            profile.remember_password = true;
            app.connection_manager.select(profile);
            if switch_profile {
                app.connection_manager.select(Profile::draft(
                    "Other".into(),
                    ConnectionKind::Postgres(PostgresProfile::default()),
                ));
            }
            app.connection_manager.password = "TYPED_NOW".into();
            app.connection_manager.password_revision += 1;
            *backend.release.lock().unwrap() = true;
            backend.ready.notify_all();
            settle_credentials(&mut app);
            assert_eq!(app.connection_manager.password, "TYPED_NOW");
        }
    }

    #[test]
    fn probe_progress_is_field_specific_and_edits_invalidate_results() {
        let mut manager = Manager::default();
        manager.select(Profile::draft(
            "Test".into(),
            ConnectionKind::Postgres(PostgresProfile::default()),
        ));
        manager.probe_snapshot = Some((manager.draft.clone().unwrap(), manager.password.clone()));
        manager.apply_probe_event(ProbeEvent::Success(vec![
            ProbeField::SshHost,
            ProbeField::SshIdentity,
        ]));
        manager.apply_probe_event(ProbeEvent::Running(vec![
            ProbeField::Host,
            ProbeField::User,
            ProbeField::Password,
        ]));
        manager.apply_probe_event(ProbeEvent::Failed(
            vec![ProbeField::Password, ProbeField::User],
            "Authentication rejected (28P01)".into(),
        ));
        manager.apply_probe_event(ProbeEvent::Finished);
        assert!(
            manager
                .probe_status
                .contains(&(ProbeField::SshIdentity, FieldStatus::Success))
        );
        assert!(
            manager
                .probe_status
                .iter()
                .any(|(field, status)| *field == ProbeField::Password
                    && matches!(status, FieldStatus::Failed(_)))
        );
        assert_eq!(
            manager.probe_message.as_deref(),
            Some("Authentication rejected (28P01)")
        );
        manager.password = "changed".into();
        manager.poll_probe();
        assert!(manager.probe_status.is_empty());
        assert!(manager.probe_snapshot.is_none());
        assert!(manager.probe_message.is_none());
    }

    #[test]
    fn testing_sqlite_does_not_save_connect_or_queue_a_window() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("probe.sqlite");
        rusqlite::Connection::open(&path).unwrap();
        let mut app = app();
        app.connection_manager.select(Profile::draft(
            "Unsaved test".into(),
            ConnectionKind::SQLite { path },
        ));
        app.connection_manager
            .start_probe(&egui::Context::default());
        let start = std::time::Instant::now();
        while app.connection_manager.probe.is_some() {
            app.connection_manager.poll_probe();
            assert!(start.elapsed() < std::time::Duration::from_secs(3));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            app.connection_manager
                .probe_status
                .contains(&(ProbeField::File, FieldStatus::Success))
        );
        assert!(app.pending.is_none());
        assert!(app.take_window_requests().is_empty());
        assert!(app.prefs.saved_connections.profiles.is_empty());
        assert_eq!(app.connection_manager.draft.as_ref().unwrap().id.0, 0);
    }

    fn connect_saved_test_session(app: &mut Rustable) {
        app.connection_action(Action::Save(true)).unwrap();
        let request = app.take_window_requests().pop().unwrap();
        let identity = request.identity.unwrap();
        match request.connection {
            crate::session_windows::WindowConnection::Sqlite(path) => app.dispatch(
                Command::Open(Some(path.clone())),
                Pending::ConnectProfile {
                    recent: Some(path),
                    identity,
                },
            ),
            _ => panic!("SQLite identity fixture expected"),
        }
    }

    fn manager_frame(
        ctx: &egui::Context,
        app: &mut Rustable,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.connections_window(ctx),
        )
    }
    fn painted_text(output: &egui::FullOutput, value: &str) -> Option<egui::Pos2> {
        fn find(shape: &egui::Shape, value: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == value => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, value)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape, value))
    }
    fn manager_click(
        ctx: &egui::Context,
        app: &mut Rustable,
        pos: egui::Pos2,
        button: egui::PointerButton,
    ) {
        for pressed in [true, false] {
            let _ = manager_frame(
                ctx,
                app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
    #[test]
    fn new_group_is_created_and_named_inline() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.connections = true;
        manager_frame(&ctx, &mut app, vec![]);
        let output = manager_frame(&ctx, &mut app, vec![]);
        let button = painted_text(&output, "New group").unwrap();
        manager_click(&ctx, &mut app, button, egui::PointerButton::Primary);
        assert_eq!(app.prefs.saved_connections.groups.len(), 1);
        let id = app.prefs.saved_connections.groups[0].id;
        manager_frame(&ctx, &mut app, vec![]);
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new(("rename-connection-group", id.0)))));
        manager_frame(&ctx, &mut app, vec![egui::Event::Text("Production".into())]);
        manager_frame(&ctx, &mut app, vec![egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }]);
        assert_eq!(app.prefs.saved_connections.groups[0].name, "Production");
        assert!(app.connection_manager.rename_group.is_none());
        app.connection_action(Action::AddGroup).unwrap();
        manager_frame(&ctx, &mut app, vec![]);
        manager_frame(&ctx, &mut app, vec![egui::Event::Text("Discard this name".into())]);
        manager_frame(&ctx, &mut app, vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }]);
        assert_eq!(app.prefs.saved_connections.groups[1].name, "New group");
        assert!(app.connection_manager.rename_group.is_none());
    }
    #[test]
    fn search_enter_connects_only_visible_valid_selection() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.connection_action(Action::New(true)).unwrap();
        app.connection_manager.draft.as_mut().unwrap().name = "Keyboard connect".into();
        app.connection_action(Action::Save(false)).unwrap();
        let id = app.connection_manager.draft.as_ref().unwrap().id;
        app.connections = true;
        manager_frame(&ctx, &mut app, vec![]);
        let enter = |app: &mut Rustable| {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("connection-search")));
            manager_frame(&ctx, app, vec![egui::Event::Key {
                key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]);
        };
        app.connection_manager.search = "no matching profile".into();
        enter(&mut app);
        assert!(app.window_requests.is_empty());
        app.connection_manager.search.clear();
        app.connection_manager.collapsed_groups.insert(None);
        enter(&mut app);
        assert!(app.window_requests.is_empty());
        app.connection_manager.collapsed_groups.clear();
        app.connection_manager.draft.as_mut().unwrap().name.clear();
        enter(&mut app);
        assert!(app.window_requests.is_empty(), "invalid forms cannot connect");
        app.connection_manager.draft.as_mut().unwrap().name = "Keyboard connect".into();
        app.connection_manager.search = "Keyboard".into();
        app.connection_manager.credential_pending.push(CredentialPending {
            token: 0, id, generation: app.connection_manager.generation,
            kind: CredentialKind::Read(app.connection_manager.password_revision),
        });
        enter(&mut app);
        assert!(app.window_requests.is_empty(), "wait for pending saved password");
        app.connection_manager.credential_pending.clear();
        enter(&mut app);
        assert_eq!(app.window_requests.len(), 1);
        assert_eq!(app.window_requests[0].identity.as_ref().unwrap().id, id);
        assert_eq!(app.window_requests[0].title, "Keyboard connect");
    }
    #[test]
    fn restored_connection_keyboard_order_respects_groups_collapse_and_search() {
        let ctx = egui::Context::default();
        let mut app = app();
        let a = app.prefs.saved_connections.add_group("First group".into()).unwrap();
        let b = app.prefs.saved_connections.add_group("Second group".into()).unwrap();
        let mut ids = Vec::new();
        // Storage insertion order deliberately differs from display group order.
        for group in [Some(b), None, Some(a)] {
            let mut profile = Profile::draft("Same name".into(), ConnectionKind::Postgres(PostgresProfile::default()));
            profile.group = group;
            ids.push(app.prefs.saved_connections.add_profile(profile).unwrap());
        }
        app.prefs.saved_connections = serde_json::from_str(
            &serde_json::to_string(&app.prefs.saved_connections).unwrap()).unwrap();
        app.connections = true;
        manager_frame(&ctx, &mut app, vec![]);
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("connection-search")));
        manager_frame(&ctx, &mut app, vec![]);
        let down = || vec![egui::Event::Key { key: egui::Key::ArrowDown, physical_key: None,
            pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }];
        for expected in [ids[1], ids[2], ids[0]] {
            manager_frame(&ctx, &mut app, down());
            assert_eq!(app.connection_manager.draft.as_ref().unwrap().id, expected);
        }
        app.connection_manager.draft = None;
        app.connection_manager.collapsed_groups.insert(Some(a));
        for expected in [ids[1], ids[0]] {
            manager_frame(&ctx, &mut app, down());
            assert_eq!(app.connection_manager.draft.as_ref().unwrap().id, expected);
        }
        // Searching a collapsed group's name reveals and selects its member.
        app.connection_manager.search = "First group".into();
        manager_frame(&ctx, &mut app, down());
        assert_eq!(app.connection_manager.draft.as_ref().unwrap().id, ids[2]);
        assert!(app.window_requests.is_empty());
    }
    #[test]
    fn long_connection_list_keyboard_selection_scrolls_and_uses_profile_ids() {
        let ctx = egui::Context::default();
        let mut app = app();
        let mut ids = Vec::new();
        for index in 0..60 {
            app.connection_action(Action::New(true)).unwrap();
            app.connection_manager.draft.as_mut().unwrap().name = match index {
                1 | 2 => "Same display name".into(),
                59 => "Last connection".into(),
                _ => format!("Connection {index:02}"),
            };
            app.connection_action(Action::Save(false)).unwrap();
            ids.push(app.connection_manager.draft.as_ref().unwrap().id);
        }
        app.connection_manager.draft = None;
        app.connections = true;
        let visible = |output: &egui::FullOutput, label: &str| {
            output.shapes.iter().any(|clipped| {
                if let egui::Shape::Text(text) = &clipped.shape {
                    let center = text.pos + text.galley.size() * 0.5;
                    text.galley.text() == label && center.x < 400.0
                        && clipped.clip_rect.contains(center)
                        && (0.0..900.0).contains(&center.y)
                } else { false }
            })
        };
        let initial = manager_frame(&ctx, &mut app, vec![]);
        assert!(!visible(&initial, "Last connection"));
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("connection-search")));
        manager_frame(&ctx, &mut app, vec![]);
        for id in &ids {
            manager_frame(&ctx, &mut app, vec![egui::Event::Key {
                key: egui::Key::ArrowDown, physical_key: None, pressed: true,
                repeat: false, modifiers: egui::Modifiers::NONE,
            }]);
            assert_eq!(app.connection_manager.draft.as_ref().unwrap().id, *id);
        }
        // Scroll animation is advanced by rendered frames, not wall-clock sleeps.
        let mut output = manager_frame(&ctx, &mut app, vec![]);
        for _ in 0..60 { output = manager_frame(&ctx, &mut app, vec![]); }
        assert!(visible(&output, "Last connection"), "selected list row must be inside its clipped viewport");
        assert!(ctx.memory(|m| m.has_focus(egui::Id::new("connection-search"))));
        assert!(app.window_requests.is_empty());
    }
    #[test]
    fn search_arrows_select_visible_profiles_without_discarding_dirty_forms() {
        let ctx = egui::Context::default();
        let mut app = app();
        let mut ids = Vec::new();
        for name in ["Alpha", "Beta", "Gamma"] {
            app.connection_action(Action::New(true)).unwrap();
            app.connection_manager.draft.as_mut().unwrap().name = name.into();
            app.connection_action(Action::Save(false)).unwrap();
            ids.push(app.connection_manager.draft.as_ref().unwrap().id);
        }
        app.connection_manager.draft = None;
        app.connections = true;
        manager_frame(&ctx, &mut app, vec![]);
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("connection-search")));
        manager_frame(&ctx, &mut app, vec![]);
        let key = |key| vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }];
        manager_frame(&ctx, &mut app, key(egui::Key::ArrowDown));
        assert_eq!(app.connection_manager.draft.as_ref().unwrap().id, ids[0]);
        manager_frame(&ctx, &mut app, key(egui::Key::ArrowDown));
        assert_eq!(app.connection_manager.draft.as_ref().unwrap().id, ids[1]);
        manager_frame(&ctx, &mut app, key(egui::Key::ArrowUp));
        assert_eq!(app.connection_manager.draft.as_ref().unwrap().id, ids[0]);
        app.connection_manager.search = "Gamma".into();
        manager_frame(&ctx, &mut app, key(egui::Key::ArrowDown));
        assert_eq!(app.connection_manager.draft.as_ref().unwrap().id, ids[2]);
        app.connection_manager.draft.as_mut().unwrap().name = "Unsaved".into();
        app.connection_manager.search.clear();
        manager_frame(&ctx, &mut app, key(egui::Key::ArrowUp));
        assert_eq!(app.connection_manager.draft.as_ref().unwrap().id, ids[2]);
        assert_eq!(app.connection_manager.draft.as_ref().unwrap().name, "Unsaved");
        assert!(app.connection_manager.error.as_ref().unwrap().contains("Save or discard"));
        assert!(app.window_requests.is_empty());
        assert!(ctx.memory(|m| m.has_focus(egui::Id::new("connection-search"))));
    }
    #[test]
    fn selected_connection_menu_opens_from_keyboard_search_focus() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.connection_action(Action::New(true)).unwrap();
        app.connection_manager.draft.as_mut().unwrap().name = "Keyboard menu".into();
        app.connection_action(Action::Save(false)).unwrap();
        app.connections = true;
        manager_frame(&ctx, &mut app, vec![]);
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("connection-search")));
        manager_frame(&ctx, &mut app, vec![egui::Event::Key {
            key: egui::Key::F10, physical_key: None, pressed: true,
            repeat: false, modifiers: egui::Modifiers::SHIFT,
        }]);
        let output = manager_frame(&ctx, &mut app, vec![]);
        assert!(painted_text(&output, "Move to group ⏵").is_some());
        assert!(painted_text(&output, "Duplicate").is_some());
        assert!(egui::Popup::is_any_open(&ctx));
        assert!(app.take_window_requests().is_empty());
        manager_frame(&ctx, &mut app, vec![egui::Event::Key {
            key: egui::Key::Escape, physical_key: None, pressed: true,
            repeat: false, modifiers: egui::Modifiers::NONE,
        }]);
        assert!(!egui::Popup::is_any_open(&ctx));
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("connection-search")));
        manager_frame(&ctx, &mut app, vec![egui::Event::Key {
            key: egui::Key::F10, physical_key: None, pressed: true,
            repeat: false, modifiers: egui::Modifiers::SHIFT,
        }]);
        manager_frame(&ctx, &mut app, vec![]);
        manager_frame(&ctx, &mut app, vec![egui::Event::Key {
            key: egui::Key::Enter, physical_key: None, pressed: true,
            repeat: false, modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(app.prefs.saved_connections.profiles.len(), 2, "Enter must activate the focused Duplicate action");
        assert!(app.take_window_requests().is_empty());
    }

    #[test]
    fn keyboard_menu_can_move_profile_to_another_group() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.prefs.saved_connections.add_group("First destination".into()).unwrap();
        let group = app.prefs.saved_connections.add_group("Destination".into()).unwrap();
        app.connection_action(Action::New(true)).unwrap();
        app.connection_manager.draft.as_mut().unwrap().name = "Keyboard move".into();
        app.connection_action(Action::Save(false)).unwrap();
        let id = app.connection_manager.draft.as_ref().unwrap().id;
        app.connections = true;
        manager_frame(&ctx, &mut app, vec![]);
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("connection-search")));
        let key = |key, modifiers| vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }];
        manager_frame(&ctx, &mut app, key(egui::Key::F10, egui::Modifiers::SHIFT));
        manager_frame(&ctx, &mut app, vec![]);
        manager_frame(&ctx, &mut app, key(egui::Key::Enter, egui::Modifiers::NONE));
        manager_frame(&ctx, &mut app, vec![]);
        manager_frame(&ctx, &mut app, key(egui::Key::ArrowDown, egui::Modifiers::NONE));
        manager_frame(&ctx, &mut app, key(egui::Key::Enter, egui::Modifiers::NONE));
        assert_eq!(app.prefs.saved_connections.profile(id).unwrap().group, Some(group));
        assert_eq!(app.prefs.saved_connections.profiles.len(), 1);
        assert!(app.take_window_requests().is_empty());
    }

    #[test]
    fn context_menu_moves_preserve_profiles_without_connecting() {
        let ctx = egui::Context::default();
        let mut app = app();
        let first = app.prefs.saved_connections.add_group("First group".into()).unwrap();
        let second = app.prefs.saved_connections.add_group("Second group".into()).unwrap();
        let mut ids = Vec::new();
        for name in ["First profile", "Second profile"] {
            app.connection_action(Action::New(true)).unwrap();
            let draft = app.connection_manager.draft.as_mut().unwrap();
            draft.name = name.into();
            draft.group = Some(first);
            app.connection_action(Action::Save(false)).unwrap();
            ids.push(app.connection_manager.draft.as_ref().unwrap().id);
        }
        app.connection_manager.draft = None;
        app.connections = true;
        let _ = manager_frame(&ctx, &mut app, vec![]);
        for label in ["First group", "First profile"] {
            let output = manager_frame(&ctx, &mut app, vec![]);
            let target = painted_text(&output, label).unwrap();
            manager_click(&ctx, &mut app, target, egui::PointerButton::Secondary);
            let output = manager_frame(&ctx, &mut app, vec![]);
            let down = painted_text(&output, "Move down").unwrap();
            manager_click(&ctx, &mut app, down, egui::PointerButton::Primary);
        }
        assert_eq!(app.prefs.saved_connections.groups[0].id, second);
        assert_eq!(app.prefs.saved_connections.in_group(Some(first)).map(|p| p.id).collect::<Vec<_>>(), [ids[1], ids[0]]);
        let output = manager_frame(&ctx, &mut app, vec![]);
        let target = painted_text(&output, "First profile").unwrap();
        manager_click(&ctx, &mut app, target, egui::PointerButton::Secondary);
        let output = manager_frame(&ctx, &mut app, vec![]);
        let menu = painted_text(&output, "Move to group ⏵").unwrap();
        manager_click(&ctx, &mut app, menu, egui::PointerButton::Primary);
        let mut output = manager_frame(&ctx, &mut app, vec![]);
        // Inspect the popup layer before the same label in the background list.
        output.shapes.reverse();
        let destination = painted_text(&output, "Ungrouped").unwrap();
        manager_click(&ctx, &mut app, destination, egui::PointerButton::Primary);
        assert_eq!(app.prefs.saved_connections.profile(ids[0]).unwrap().group, None);
        assert_eq!(app.prefs.saved_connections.profiles.len(), 2);
        assert!(app.take_window_requests().is_empty());
        let restored: crate::connection_profiles::ConnectionProfiles = serde_json::from_str(&serde_json::to_string(&app.prefs.saved_connections).unwrap()).unwrap();
        assert_eq!(restored.groups[0].id, second);
        assert_eq!(restored.profile(ids[0]).unwrap().group, None);
    }

    #[test]
    fn secondary_click_menus_duplicate_rows_and_rename_groups_with_and_without_search() {
        for search in ["", "menu"] {
            let ctx = egui::Context::default();
            let mut app = app();
            let group = app
                .prefs
                .saved_connections
                .add_group("Menu group".into())
                .unwrap();
            app.connection_action(Action::New(true)).unwrap();
            app.connection_manager.draft.as_mut().unwrap().name = "Menu profile".into();
            app.connection_manager.draft.as_mut().unwrap().group = Some(group);
            app.connection_action(Action::Save(false)).unwrap();
            app.connection_manager.draft = None;
            app.connection_manager.search = search.into();
            app.connections = true;
            let _ = manager_frame(&ctx, &mut app, vec![]);
            let output = manager_frame(&ctx, &mut app, vec![]);
            let row = painted_text(&output, "Menu profile").unwrap();
            manager_click(&ctx, &mut app, row, egui::PointerButton::Secondary);
            let output = manager_frame(&ctx, &mut app, vec![]);
            let duplicate = painted_text(&output, "Duplicate")
                .expect("secondary row click must expose Duplicate");
            manager_click(&ctx, &mut app, duplicate, egui::PointerButton::Primary);
            assert_eq!(app.prefs.saved_connections.profiles.len(), 2);
            assert_eq!(
                app.connection_manager.draft.as_ref().unwrap().name,
                "Menu profile copy"
            );
            let output = manager_frame(&ctx, &mut app, vec![]);
            let header = painted_text(&output, "Menu group").unwrap();
            manager_click(&ctx, &mut app, header, egui::PointerButton::Secondary);
            let output = manager_frame(&ctx, &mut app, vec![]);
            let rename = painted_text(&output, "Rename group")
                .expect("secondary header click must expose Rename group");
            manager_click(&ctx, &mut app, rename, egui::PointerButton::Primary);
            assert_eq!(
                app.connection_manager
                    .rename_group
                    .as_ref()
                    .map(|(id, _)| *id),
                Some(group)
            );
        }
    }
    #[test]
    fn expanded_ssh_fits_large_screen_and_agent_checkbox_tracks_key_choice() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.connection_action(Action::New(true)).unwrap();
        if let ConnectionKind::Postgres(config) =
            &mut app.connection_manager.draft.as_mut().unwrap().connection
        {
            config.ssh = Some(SshOptions::default());
        }
        app.connections = true;
        let mut output = None;
        for _ in 0..5 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 952.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.connections_window(ctx),
            ));
        }
        let (content, rect) = ctx
            .data(|data| {
                data.get_temp::<(egui::Vec2, egui::Rect)>(egui::Id::new("connection-form-geometry"))
            })
            .unwrap();
        assert!(
            content.y <= rect.height() + 1.0,
            "expanded form needs scroll: {content:?} {rect:?}"
        );
        assert!(painted_text(&output.unwrap(), "Use config / SSH agent").is_some());
        let generation = app.connection_manager.generation;
        // Only a completed picker changes authentication. No callback on cancel leaves it intact.
        app.connection_manager
            .set_path(generation, true, "/tmp/test-key".into());
        let ConnectionKind::Postgres(config) =
            &app.connection_manager.draft.as_ref().unwrap().connection
        else {
            panic!()
        };
        assert_eq!(
            config.ssh.as_ref().unwrap().identity_file,
            Some(PathBuf::from("/tmp/test-key"))
        );
        let output = manager_frame(&ctx, &mut app, vec![]);
        let checkbox = painted_text(&output, "Use config / SSH agent").unwrap();
        manager_click(&ctx, &mut app, checkbox, egui::PointerButton::Primary);
        let ConnectionKind::Postgres(config) =
            &app.connection_manager.draft.as_ref().unwrap().connection
        else {
            panic!()
        };
        assert!(config.ssh.as_ref().unwrap().identity_file.is_none());
    }

    #[test]
    fn compact_connection_form_keeps_actions_visible_with_ssh_and_search() {
        for dark in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let mut app = app();
            app.connection_action(Action::New(true)).unwrap();
            app.connection_manager.draft.as_mut().unwrap().name = "Compact profile".into();
            if let ConnectionKind::Postgres(config) =
                &mut app.connection_manager.draft.as_mut().unwrap().connection
            {
                config.ssh = Some(SshOptions::default());
            }
            app.connection_manager.search = "compact".into();
            app.connections = true;
            let mut baseline = 0.0;
            for frame in 0..30 {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1000.0, 650.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.connections_window(ctx),
                );
                let rect = ctx
                    .data(|data| {
                        data.get_temp::<egui::Rect>(egui::Id::new("connection-manager-geometry"))
                    })
                    .unwrap();
                if frame == 3 {
                    baseline = rect.width();
                }
                if frame > 3 {
                    let (content, inner) = ctx
                        .data(|data| {
                            data.get_temp::<(egui::Vec2, egui::Rect)>(egui::Id::new(
                                "connection-form-geometry",
                            ))
                        })
                        .unwrap();
                    assert!(
                        content.y > inner.height(),
                        "small screen should scroll expanded SSH"
                    );
                    assert!(
                        content.x <= inner.width() + 1.0,
                        "form fields exceed reserved scrollbar gutter"
                    );
                    assert!(rect.width() <= baseline + 1.0);
                    for label in ["Discard", "Save", "Connect"] {
                        let pos = painted_text(&output, label)
                            .expect("fixed form action must be painted");
                        assert!(pos.y < 650.0 && pos.x < 1000.0, "{label} clipped: {pos:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn repeated_search_layout_does_not_expand_the_connection_window() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.connection_action(Action::New(true)).unwrap();
        app.connection_manager.draft.as_mut().unwrap().name = "Search profile".into();
        app.connection_action(Action::Save(false)).unwrap();
        app.connections = true;
        for _ in 0..3 {
            let _ = manager_frame(&ctx, &mut app, vec![]);
        }
        let baseline = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("connection-manager-geometry")))
            .unwrap()
            .width();
        let form_left = ctx.data(|data| data.get_temp::<(egui::Vec2, egui::Rect)>(
            egui::Id::new("connection-form-geometry"))).unwrap().1.left();
        for search in [
            "search".to_owned(),
            "very long query ".repeat(50),
            String::new(),
        ] {
            app.connection_manager.search = search;
            for _ in 0..20 {
                let _ = manager_frame(&ctx, &mut app, vec![]);
                let width = ctx
                    .data(|data| {
                        data.get_temp::<egui::Rect>(egui::Id::new("connection-manager-geometry"))
                    })
                    .unwrap()
                    .width();
                let left = ctx.data(|data| data.get_temp::<(egui::Vec2, egui::Rect)>(
                    egui::Id::new("connection-form-geometry"))).unwrap().1.left();
                assert!((left - form_left).abs() <= 1.0,
                    "search moved the form from {form_left} to {left}");
                assert!(
                    width <= baseline + 1.0,
                    "search grew window from {baseline} to {width}"
                );
            }
        }
    }
    #[test]
    fn duplicate_is_saved_without_connecting_clears_password_and_preserves_dirty_forms() {
        let mut app = app();
        app.connection_action(Action::New(true)).unwrap();
        app.connection_manager.draft.as_mut().unwrap().name = "Source".into();
        app.connection_action(Action::Save(false)).unwrap();
        let source = app.connection_manager.draft.as_ref().unwrap().id;
        let original_label = app.label.clone();
        app.connection_manager.password = "EPHEMERAL_DUPLICATE_PASSWORD".into();
        app.connection_manager.search = "source".into();
        app.connection_action(Action::Duplicate(source)).unwrap();
        let copy = app.connection_manager.draft.as_ref().unwrap();
        let copy_id = copy.id;
        assert_ne!(source, copy_id);
        assert_eq!(copy.name, "Source copy");
        assert_eq!(app.prefs.saved_connections.profile(copy_id), Some(copy));
        assert!(app.connection_manager.password.is_empty());
        assert_eq!(app.label, original_label);
        assert!(app.pending.is_none());
        assert_eq!(app.connection_manager.search, "source");
        app.connection_manager.draft.as_mut().unwrap().name = "Unfinished change".into();
        app.connection_manager.password = "KEEP_UNTIL_DISCARD".into();
        assert!(app.connection_action(Action::Duplicate(source)).is_err());
        assert_eq!(app.prefs.saved_connections.profiles.len(), 2);
        assert_eq!(
            app.connection_manager.draft.as_ref().unwrap().name,
            "Unfinished change"
        );
        assert_eq!(app.connection_manager.password, "KEEP_UNTIL_DISCARD");
        let mut storage = Storage::default();
        app.save_with_window_documents(&mut storage, vec![]);
        assert!(!storage.0["workspace"].contains("EPHEMERAL_DUPLICATE_PASSWORD"));
        assert!(!storage.0["workspace"].contains("KEEP_UNTIL_DISCARD"));
        let restored: Preferences = eframe::get_value(&storage, "workspace").unwrap();
        assert_eq!(
            restored.saved_connections.profile(copy_id).unwrap().name,
            "Source copy"
        );
    }
    #[test]
    fn filtering_preserves_draft_password_order_and_blocks_reorder() {
        let mut app = app();
        let group = app
            .prefs
            .saved_connections
            .add_group("Group".into())
            .unwrap();
        for name in ["One", "Two", "Three"] {
            app.connection_action(Action::New(true)).unwrap();
            app.connection_manager.draft.as_mut().unwrap().name = name.into();
            app.connection_manager.draft.as_mut().unwrap().group = Some(group);
            app.connection_action(Action::Save(false)).unwrap();
        }
        let original = app.prefs.saved_connections.profiles.clone();
        app.connection_manager.draft.as_mut().unwrap().name = "Still editing".into();
        app.connection_manager.password = "SEARCH_IS_NOT_CREDENTIAL_SEARCH".into();
        let draft = app.connection_manager.draft.clone();
        app.connection_manager.search = "tWo".into();
        assert_eq!(
            app.prefs
                .saved_connections
                .matching_profiles(&app.connection_manager.search),
            vec![original[1].id]
        );
        assert!(
            app.connection_action(Action::Move(original[2].id, Some(group), 0))
                .is_err()
        );
        assert!(app.connection_action(Action::MoveGroup(group, 0)).is_err());
        app.connection_manager.search.clear();
        assert_eq!(
            app.prefs.saved_connections.matching_profiles(""),
            original
                .iter()
                .map(|profile| profile.id)
                .collect::<Vec<_>>()
        );
        assert_eq!(app.prefs.saved_connections.profiles, original);
        assert_eq!(app.connection_manager.draft, draft);
        assert_eq!(
            app.connection_manager.password,
            "SEARCH_IS_NOT_CREDENTIAL_SEARCH"
        );
        assert!(
            app.prefs
                .saved_connections
                .matching_profiles("SEARCH_IS_NOT_CREDENTIAL_SEARCH")
                .is_empty()
        );
    }
    #[test]
    fn same_endpoint_profiles_keep_identity_styles_and_failed_connect_preserves_active_session() {
        fn settle(app: &mut Rustable) {
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            while app.pending.is_some() && Instant::now() < deadline {
                app.poll();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(app.pending.is_none());
        }
        let mut root = app();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shared.sqlite");
        rusqlite::Connection::open(&path).unwrap();
        let mut a = Profile::draft(
            "Production alias".into(),
            ConnectionKind::SQLite { path: path.clone() },
        );
        a.environment = "Production".into();
        a.color = [190, 20, 30];
        a.id = root.prefs.saved_connections.add_profile(a.clone()).unwrap();
        let mut b = Profile::draft(
            "Development alias".into(),
            ConnectionKind::SQLite { path: path.clone() },
        );
        b.environment = "Development".into();
        b.color = [20, 40, 190];
        b.id = root.prefs.saved_connections.add_profile(b.clone()).unwrap();
        root.prefs.connection_styles.insert(
            path.display().to_string(),
            ConnectionStyle {
                environment: "Legacy fallback".into(),
                color: [1, 2, 3],
            },
        );
        root.connection_manager.select(a.clone());
        connect_saved_test_session(&mut root);
        settle(&mut root);
        assert_eq!(root.active_profile.as_ref().unwrap().id, a.id);
        assert_eq!(root.active_connection_name(), "Production alias");
        assert_eq!(root.active_connection_style().color, a.color);
        root.connection_manager.select(b.clone());
        root.connection_action(Action::OpenWindow).unwrap();
        assert_eq!(root.active_connection_style().environment, "Production");
        let request = root.take_window_requests().pop().unwrap();
        let mut child = root.new_window(&egui::Context::default(), request);
        settle(&mut child);
        assert_eq!(child.active_profile.as_ref().unwrap().id, b.id);
        assert_eq!(child.active_connection_name(), "Development alias");
        assert_eq!(child.active_connection_style().color, b.color);
        root.connection_manager.draft.as_mut().unwrap().name = "Renamed development".into();
        root.connection_manager.draft.as_mut().unwrap().color = [30, 200, 40];
        root.connection_action(Action::Save(false)).unwrap();
        root.sync_window_profile(&mut child);
        assert_eq!(child.active_connection_name(), "Renamed development");
        assert_eq!(child.active_connection_style().color, [30, 200, 40]);
        assert_eq!(root.active_connection_name(), "Production alias");
        assert_eq!(root.active_connection_style().color, a.color);
        assert_eq!(
            root.prefs.connection_styles[&path.display().to_string()].environment,
            "Legacy fallback"
        );
        let mut storage = Storage::default();
        root.save_with_window_documents(&mut storage, vec![]);
        let prefs: Preferences = eframe::get_value(&storage, "workspace").unwrap();
        let mut restored = Rustable::with_preferences(&egui::Context::default(), prefs);
        settle(&mut restored);
        for (id, name, color) in [
            (a.id, "Production alias", a.color),
            (b.id, "Renamed development", [30, 200, 40]),
        ] {
            restored.connection_action(Action::Select(id)).unwrap();
            connect_saved_test_session(&mut restored);
            settle(&mut restored);
            assert_eq!(restored.active_profile.as_ref().unwrap().id, id);
            assert_eq!(restored.active_connection_name(), name);
            assert_eq!(restored.active_connection_style().color, color);
        }
        let mut failed = Profile::draft(
            "Cannot connect".into(),
            ConnectionKind::SQLite {
                path: directory.path().into(),
            },
        );
        failed.environment = "Wrong environment".into();
        root.connection_manager.select(failed);
        connect_saved_test_session(&mut root);
        assert_eq!(root.active_profile.as_ref().unwrap().id, a.id);
        settle(&mut root);
        assert!(root.error.is_some());
        assert_eq!(root.active_profile.as_ref().unwrap().id, a.id);
        assert_eq!(root.active_connection_name(), "Production alias");
        assert_eq!(root.active_connection_style().environment, "Production");
        // A failed reconnect of the same profile keeps the working endpoint and identity.
        root.connection_manager.select(a.clone());
        root.connection_manager.draft.as_mut().unwrap().connection = ConnectionKind::SQLite {
            path: directory.path().into(),
        };
        connect_saved_test_session(&mut root);
        settle(&mut root);
        assert!(root.error.is_some());
        assert_eq!(root.active_profile.as_ref().unwrap().id, a.id);
        assert_eq!(root.active_connection_style().color, a.color);
        assert_eq!(root.label, path.display().to_string());
        root.dispatch(
            Command::Open(Some(path.clone())),
            Pending::Connect(Some(path)),
        );
        settle(&mut root);
        assert!(root.active_profile.is_none());
        assert_eq!(
            root.active_connection_style().environment,
            "Legacy fallback"
        );
        // Removing a definition leaves its already-connected child accurately labeled.
        root.connection_action(Action::Delete(b.id)).unwrap();
        root.sync_window_profile(&mut child);
        assert_eq!(child.active_connection_name(), "Renamed development");
        assert_eq!(child.active_profile.as_ref().unwrap().id, b.id);
    }
    #[test]
    fn restored_profile_can_be_selected_by_clicking_its_draggable_row() {
        let mut original = app();
        original.connection_action(Action::New(true)).unwrap();
        original.connection_manager.draft.as_mut().unwrap().name = "Click restored profile".into();
        original.connection_action(Action::Save(false)).unwrap();
        let mut storage = Storage::default();
        original.save_with_window_documents(&mut storage, Vec::new());
        let ctx = egui::Context::default();
        let prefs = eframe::get_value(&storage, "workspace").unwrap();
        let mut restored = Rustable::with_preferences(&ctx, prefs);
        restored.connections = true;
        let frame = |app: &mut Rustable, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1400.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.connections_window(ctx),
            )
        };
        let _ = frame(&mut restored, vec![]);
        let output = frame(&mut restored, vec![]);
        fn position(shape: &egui::Shape) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == "Click restored profile" => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(position),
                _ => None,
            }
        }
        let point = output
            .shapes
            .iter()
            .find_map(|shape| position(&shape.shape))
            .expect("profile row painted");
        let _ = frame(
            &mut restored,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert!(restored.connection_manager.draft.is_none());
        let _ = frame(
            &mut restored,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(
            restored
                .connection_manager
                .draft
                .as_ref()
                .map(|profile| profile.name.as_str()),
            Some("Click restored profile")
        );
    }
    #[test]
    fn opening_new_window_queues_private_credentials_without_replacing_current_session() {
        let mut app = app();
        let original_label = app.label.clone();
        app.connection_action(Action::New(true)).unwrap();
        app.connection_manager.draft.as_mut().unwrap().name = "Separate session".into();
        app.connection_manager.password = "WINDOW_SECRET_ONLY".into();
        app.connection_action(Action::OpenWindow).unwrap();
        assert_eq!(app.label, original_label);
        assert!(app.pending.is_none());
        let requests = app.take_window_requests();
        assert_eq!(requests.len(), 1);
        assert!(
            matches!(&requests[0].connection, crate::session_windows::WindowConnection::Postgres(config) if config.password == "WINDOW_SECRET_ONLY")
        );
        assert!(app.take_window_requests().is_empty());
        app.connection_action(Action::Save(true)).unwrap();
        app.connection_action(Action::Save(true)).unwrap();
        let additional = app.take_window_requests();
        assert_eq!(
            additional.len(),
            2,
            "each Open creates a distinct native workspace request"
        );
        assert_eq!(
            additional[0].identity.as_ref().unwrap().id,
            additional[1].identity.as_ref().unwrap().id
        );
        assert!(app.pending.is_none(), "manager must never connect in place");
        let mut storage = Storage::default();
        app.save_with_window_documents(&mut storage, Vec::new());
        assert!(!storage.0["workspace"].contains("WINDOW_SECRET_ONLY"));
        assert_eq!(app.prefs.saved_connections.profiles.len(), 1);
    }
    #[test]
    fn saved_connection_survives_preferences_roundtrip_without_session_password() {
        let mut app = app();
        app.connection_action(Action::New(true)).unwrap();
        let draft = app.connection_manager.draft.as_mut().unwrap();
        draft.name = "Staging".into();
        draft.environment = "Staging".into();
        draft.color = [123, 45, 67];
        app.connection_manager.password = "NEVER_PERSIST_THIS".into();
        app.connection_action(Action::Save(false)).unwrap();
        let mut storage = Storage::default();
        eframe::set_value(&mut storage, "preferences", &app.prefs);
        assert!(!storage.0["preferences"].contains("NEVER_PERSIST_THIS"));
        let restored: Preferences = eframe::get_value(&storage, "preferences").unwrap();
        let profile = &restored.saved_connections.profiles[0];
        assert_eq!(profile.name, "Staging");
        assert_eq!(profile.environment, "Staging");
        assert_eq!(profile.color, [123, 45, 67]);
    }
    #[test]
    fn dirty_form_requires_explicit_discard_and_stale_file_dialog_cannot_modify_new_draft() {
        let mut app = app();
        app.connection_action(Action::New(false)).unwrap();
        let old_generation = app.connection_manager.generation;
        assert!(app.connection_action(Action::New(true)).is_err());
        app.connection_action(Action::Discard).unwrap();
        app.connection_action(Action::New(false)).unwrap();
        app.connection_manager
            .set_path(old_generation, false, "/tmp/stale.sqlite".into());
        assert!(
            matches!(&app.connection_manager.draft.as_ref().unwrap().connection, ConnectionKind::SQLite { path } if path.as_os_str().is_empty())
        );
    }
    #[test]
    fn legacy_migration_runs_once_and_respects_saved_removal() {
        let mut prefs = Preferences {
            recent: vec!["/tmp/example.sqlite".into()],
            ..Default::default()
        };
        migrate_recent(&mut prefs);
        assert_eq!(prefs.saved_connections.profiles.len(), 1);
        let id = prefs.saved_connections.profiles[0].id;
        prefs.saved_connections.remove_profile(id).unwrap();
        migrate_recent(&mut prefs);
        assert!(prefs.saved_connections.profiles.is_empty());
    }
    #[test]
    fn refreshing_query_metadata_does_not_replay_sql_or_replace_results() {
        let mut app = app();
        app.view = View::Query;
        app.safe = false;
        app.tabs[app.tab].sql = "DROP TABLE customers;".into();
        let original = app.tabs[app.tab].grid.clone();
        app.refresh_current_view();
        assert!(matches!(app.pending, Some(Pending::Export)));
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while app.pending.is_some() && Instant::now() < deadline {
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.pending.is_none());
        assert!(app.objects.iter().any(|object| object.name == "customers"));
        assert!(Arc::ptr_eq(&original, &app.tabs[app.tab].grid));
        assert_eq!(app.tabs[app.tab].sql, "DROP TABLE customers;");
    }
}
