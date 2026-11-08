//! Drafts are separate from live values. Opening an editor never writes data.
use crate::redis_mutation::{Action, Mutation};
use eframe::egui;
pub(crate) struct Draft {
    pub action: Action,
    pub keys: Vec<Vec<u8>>,
    key_labels: Vec<String>,
    pub kind: String,
    pub key_text: String,
    pub field: String,
    pub value: String,
    pub expected: Vec<u8>,
    pub length: u64,
    pub expiry: bool,
    pub ttl: u64,
    pub error: Option<String>,
}
fn bytes(text: &str) -> Result<Vec<u8>, String> {
    if text.starts_with('"') || text.starts_with('\'') {
        let parsed = crate::redis_commands::parse(text)?;
        if parsed.len() != 1 || parsed[0].len() != 1 {
            return Err("Enter one value, optionally quoted with byte escapes.".into());
        }
        Ok(parsed[0][0].clone())
    } else {
        Ok(text.as_bytes().to_vec())
    }
}
impl Draft {
    pub fn new(action: Action, keys: Vec<Vec<u8>>, kind: String) -> Self {
        let key_labels = keys
            .iter()
            .map(|key| {
                let mut label = crate::redis_commands::quote(&key[..key.len().min(512)]);
                if key.len() > 512 {
                    label.push_str(&format!("… ({} bytes)", key.len()));
                }
                label
            })
            .collect::<Vec<_>>();
        let key_text = key_labels.first().cloned().unwrap_or_default();
        Self {
            action,
            key_labels,
            keys,
            kind,
            key_text,
            field: String::new(),
            value: String::new(),
            expected: vec![],
            length: 0,
            expiry: false,
            ttl: 3600,
            error: None,
        }
    }
    pub fn request(&self) -> Result<Mutation, String> {
        if self.action == Action::Create && self.key_text.is_empty() {
            return Err("Enter a key name; use \"\" for an empty name.".into());
        }
        if self.action == Action::Rename && self.field.is_empty() {
            return Err("Enter the destination name; use \"\" for an empty name.".into());
        }
        let keys = if self.action == Action::Create {
            vec![bytes(&self.key_text)?]
        } else {
            self.keys.clone()
        };
        let ttl_ms = if self.expiry {
            Some(self.ttl.checked_mul(1000).ok_or("Expiry is too large.")?)
        } else {
            None
        };
        let request = Mutation {
            action: self.action,
            keys,
            kind: self.kind.clone(),
            field: bytes(&self.field)?,
            value: if self.kind == "ReJSON-RL" {
                self.value.as_bytes().to_vec()
            } else {
                bytes(&self.value)?
            },
            expected: self.expected.clone(),
            length: self.length,
            ttl_ms,
        };
        request.validate(false)?;
        Ok(request)
    }
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        enabled: bool,
        pending: bool,
        cluster: bool,
    ) -> (Option<Mutation>, bool) {
        let title = match self.action {
            Action::Create => "Create key",
            Action::Add => "Add item",
            Action::Update => "Edit item",
            Action::Remove => "Remove item",
            Action::Rename => "Rename key",
            Action::DeleteKeys => "Delete selected keys",
            Action::ExpireKeys => "Change expiry",
        };
        let mut open = true;
        let mut request = None;
        let mut cancel = false;
        egui::Window::new(title).enabled(!pending).anchor(egui::Align2::CENTER_CENTER,egui::Vec2::ZERO).frame(egui::Frame::window(&ctx.style()).inner_margin(16).corner_radius(10.0)).id(egui::Id::new("redis-edit-dialog")).open(&mut open).collapsible(false).resizable(true).default_width(440.0).show(ctx,|ui| {
            (request, cancel) = self.pane(ui, enabled, pending, cluster, false);
        });
        (request, (!open || cancel) && !pending)
    }
    pub fn pane(&mut self, ui: &mut egui::Ui, enabled: bool, pending: bool, cluster: bool, inline: bool) -> (Option<Mutation>, bool) {
        let title = match self.action {
            Action::Create => "Create key",
            Action::Add => "Add item",
            Action::Update => "Edit item",
            Action::Remove => "Remove item",
            Action::Rename => "Rename key",
            Action::DeleteKeys => "Delete selected keys",
            Action::ExpireKeys => "Change expiry",
        };

        let mut request = None;
        let mut cancel = false;
        ui.add_enabled_ui(!pending, |ui| {
            crate::redis_workspace::browser_style(ui);
            ui.spacing_mut().item_spacing=egui::vec2(8.0,10.0);

            ui.label(egui::RichText::new(if self.action==Action::Create {"Start with one value. Existing keys are never replaced."} else {"Changes apply only when you confirm below."}).color(ui.visuals().weak_text_color()));
            if self.action==Action::Create {
                ui.label("Key name");ui.add(egui::TextEdit::singleline(&mut self.key_text).margin(egui::vec2(6.0,4.0)).desired_width(f32::INFINITY).char_limit(65536));
                ui.horizontal(|ui| { ui.label("Type");egui::ComboBox::from_id_salt("redis-create-type").selected_text(crate::redis_workspace::type_label(&self.kind)).show_ui(ui,|ui| {
                    for kind in ["string","hash","list","set","zset","stream","ReJSON-RL"] {ui.selectable_value(&mut self.kind,kind.into(),if kind=="ReJSON-RL" {"JSON (module required)"} else {kind});}
                });});
            } else if self.keys.len()==1 {
                ui.label(egui::RichText::new(&self.key_text).monospace()).on_hover_text(&self.key_text);
            } else {
                ui.strong(format!("{} explicit key names",self.keys.len()));
                egui::ScrollArea::vertical().max_height(150.0).show_rows(ui,20.0,self.keys.len(),|ui,range| {for i in range {ui.add(egui::Label::new(egui::RichText::new(&self.key_labels[i]).monospace()).truncate());}});
            }
            if self.action==Action::Rename {
                ui.label("New name");ui.add(egui::TextEdit::singleline(&mut self.field).margin(egui::vec2(6.0,4.0)).desired_width(f32::INFINITY).char_limit(65536));
                ui.weak("Expiry is preserved. An existing destination is never overwritten.");
            } else if matches!(self.action,Action::Create|Action::Add|Action::Update) {
                if matches!(self.kind.as_str(),"hash"|"zset"|"stream") {
                    ui.label(if self.kind=="zset" {"Member"} else {"Field"});
                    ui.add_enabled(self.action!=Action::Update,egui::TextEdit::singleline(&mut self.field).margin(egui::vec2(6.0,4.0)).desired_width(f32::INFINITY).char_limit(65536));
                }
                ui.label(if self.kind=="zset" {"Score"} else {"Value"});
                ui.add(egui::TextEdit::multiline(&mut self.value).code_editor().desired_rows(5).desired_width(f32::INFINITY).char_limit(262144));
                if self.kind=="stream" {ui.weak("Appends one field/value pair with a server-generated entry ID.");}
                else if self.kind!="ReJSON-RL" {ui.weak("Plain text or quoted bytes, e.g. \"hello\\x00\". Empty values are allowed.");}
                else {ui.weak("A complete JSON document. Requires RedisJSON on the server.");}
            } else if self.action==Action::Remove {
                ui.label(egui::RichText::new(&self.field).monospace());
                ui.colored_label(ui.visuals().warn_fg_color,if self.kind=="stream" {"Removes this entry; the stream key remains."}else {"Removing the last item also removes its key."});
            }
            if matches!(self.action,Action::Create|Action::ExpireKeys) {
                ui.horizontal(|ui| {ui.checkbox(&mut self.expiry,"Expire after");ui.add_enabled(self.expiry,egui::DragValue::new(&mut self.ttl).range(1..=315360000).suffix(" seconds"));});
                if self.action==Action::ExpireKeys && !self.expiry {ui.weak("Removes expiry from the selected keys.");}
            }
            if self.action==Action::DeleteKeys {ui.colored_label(ui.visuals().warn_fg_color,"Deletes only the names listed above, including any current values at those names. This cannot be undone.");}
            if cluster && self.keys.len()>1 && matches!(self.action,Action::DeleteKeys|Action::ExpireKeys) {
                ui.colored_label(ui.visuals().warn_fg_color,"Keys in different Cluster slots change independently. Partial completion is possible; results show each key's outcome.");
            }
            if let Some(error)=&self.error {ui.colored_label(ui.visuals().error_fg_color,error);}
            if inline { ui.add_space((ui.available_height() - 48.0).max(0.0)); }
            ui.separator();
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(enabled,egui::Button::new(if self.action==Action::Update {"Save changes"}else {title}).min_size(egui::vec2(84.0,crate::controls::BUTTON_HEIGHT))).clicked() {match self.request() {Ok(r)=>request=Some(r),Err(e)=>self.error=Some(e)}}
                    cancel=ui.add(egui::Button::new("Cancel").min_size(egui::vec2(84.0,crate::controls::BUTTON_HEIGHT))).clicked();
                });
            });

        });
        (request, cancel && !pending)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drafts_preserve_binary_and_validate_before_submission() {
        let mut draft = Draft::new(Action::Create, vec![], "hash".into());
        assert!(
            draft.request().is_err(),
            "an untouched key-name field must not create an empty key accidentally"
        );
        draft.key_text = r#""""#.into();
        assert_eq!(draft.request().unwrap().keys[0], Vec::<u8>::new());
        draft.key_text = r#""a\x00\xff""#.into();
        draft.field = r#""\xff""#.into();
        draft.value = r#""\x00hello""#.into();
        let request = draft.request().unwrap();
        assert_eq!(request.keys[0], vec![b'a', 0, 255]);
        assert_eq!(request.field, vec![255]);
        assert_eq!(request.value, b"\0hello");
        draft.kind = "zset".into();
        draft.value = "NaN".into();
        assert!(draft.request().is_err());
        draft.kind = "ReJSON-RL".into();
        draft.value = "{broken".into();
        assert!(draft.request().is_err());
        draft.expiry = true;
        draft.ttl = u64::MAX;
        assert!(draft.request().is_err());
    }
    #[test]
    fn idle_drafts_do_not_submit_or_repaint_continuously() {
        let ctx = egui::Context::default();
        let mut draft = Draft::new(Action::Create, vec![], "string".into());
        let mut delay = std::time::Duration::ZERO;
        for i in 0..20 {
            let out = ctx.run(
                egui::RawInput {
                    time: Some(i as f64),
                    ..Default::default()
                },
                |ctx| {
                    assert!(draft.show(ctx, true, false, false).0.is_none());
                },
            );
            delay = out.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        }
        assert!(delay > std::time::Duration::from_secs(1));
    }
    #[test]
    #[ignore = "manual Redis draft interaction CPU benchmark"]
    fn redis_edit_dialog_frame_benchmark() {
        let ctx = egui::Context::default();
        let mut draft = Draft::new(
            Action::Update,
            vec![b"app:users:1001".to_vec()],
            "hash".into(),
        );
        draft.field = "profile".into();
        draft.value = "member ".repeat(8000);
        let mut samples = Vec::new();
        let mut changes = Vec::new();
        for frame in 0..180 {
            if frame % 20 == 0 {
                draft.value.push('x');
            }
            let start = std::time::Instant::now();
            let out = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 760.0),
                    )),
                    time: Some(frame as f64 / 120.0 + if frame >= 120 { 5.0 } else { 0.0 }),
                    ..Default::default()
                },
                |ctx| {
                    assert!(draft.show(ctx, true, false, false).0.is_none());
                },
            );
            let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            if frame >= 20 {
                samples.push(ms);
            }
            if frame % 20 == 0 {
                changes.push(ms);
            }
        }
        samples.sort_by(f64::total_cmp);
        changes.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/edit-dialog", &samples);
        crate::ui_benchmark::report("redis/edit-first-change", &changes);
    }
}
