//! Small, bounded index definitions and explicit index-only removal.
use eframe::egui;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Kind { #[default] Text, Tag, Numeric, Geo }
impl Kind {
    fn label(self) -> &'static str { match self { Self::Text => "TEXT", Self::Tag => "TAG", Self::Numeric => "NUMERIC", Self::Geo => "GEO" } }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Field { pub path: String, pub alias: String, pub kind: Kind, pub sortable: bool }
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Create { pub name: String, pub json: bool, pub prefix: String, pub fields: Vec<Field> }
impl Default for Create {
    fn default() -> Self { Self { name: String::new(), json: false, prefix: String::new(), fields: vec![Field::default()] } }
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Mutation { Create(Create), Drop(Vec<u8>) }
impl Mutation {
    pub fn validate(&self, read_only: bool) -> Result<(), String> {
        if read_only { return Err("Unlock editing before managing search indexes.".into()); }
        let name = match self { Self::Create(draft) => draft.name.as_bytes(), Self::Drop(name) => name };
        if name.is_empty() || name.len() > 4096 { return Err("Index name must contain 1–4,096 bytes.".into()); }
        if let Self::Create(draft) = self { draft.validate_fields()?; }
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn command(&self, read_only: bool) -> Result<redis::Cmd, String> {
        self.validate(read_only)?;
        match self {
            Self::Drop(name) => { let mut cmd = redis::cmd("FT.DROPINDEX"); cmd.arg(name); Ok(cmd) } // Never DD: documents survive.
            Self::Create(draft) => {
                let mut cmd = redis::cmd("FT.CREATE");
                cmd.arg(&draft.name).arg("ON").arg(if draft.json { "JSON" } else { "HASH" })
                    .arg("PREFIX").arg(1).arg(&draft.prefix).arg("SCHEMA");
                for field in &draft.fields {
                    cmd.arg(&field.path);
                    if !field.alias.is_empty() { cmd.arg("AS").arg(&field.alias); }
                    cmd.arg(field.kind.label());
                    if field.sortable { cmd.arg("SORTABLE"); }
                }
                Ok(cmd)
            }
        }
    }
}
impl Create {
    pub fn validate(&self, read_only: bool) -> Result<(), String> {
        if read_only { return Err("Unlock editing before managing search indexes.".into()); }
        if self.name.is_empty() || self.name.len() > 4096 { return Err("Index name must contain 1–4,096 bytes.".into()); }
        self.validate_fields()
    }
    fn validate_fields(&self) -> Result<(), String> {
            if self.prefix.len() > 4096 || self.fields.is_empty() || self.fields.len() > 32 { return Err("Use a prefix up to 4 KiB and 1–32 fields.".into()); }
            let mut aliases = std::collections::HashSet::new();
            for field in &self.fields {
                if field.path.is_empty() || field.path.len() > 4096 || field.alias.len() > 128 { return Err("Each field needs a path up to 4 KiB; aliases may use up to 128 bytes.".into()); }
                if self.json && (!field.path.starts_with('$') || field.alias.trim().is_empty()) { return Err("JSON fields need a $ path and an alias.".into()); }
                let alias = if field.alias.is_empty() { &field.path } else { &field.alias };
                if !aliases.insert(alias) { return Err("Field names/aliases must be unique.".into()); }
                if field.sortable && field.kind == Kind::Geo { return Err("GEO fields cannot be sortable.".into()); }
            }
        Ok(())
    }
    pub fn form(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Index name"); ui.add(egui::TextEdit::singleline(&mut self.name).char_limit(4096).desired_width(240.0).hint_text("idx:users"));
            ui.selectable_value(&mut self.json, false, "Hash"); ui.selectable_value(&mut self.json, true, "JSON");
        });
        ui.horizontal(|ui| { ui.label("Key prefix"); ui.add(egui::TextEdit::singleline(&mut self.prefix).char_limit(4096).desired_width(300.0).hint_text("users:")); ui.weak("Empty matches all keys"); });
        ui.add_space(8.0);
        let mut remove = None;
        for (index, field) in self.fields.iter_mut().enumerate() {
            ui.push_id(index, |ui| { ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut field.path).char_limit(4096).desired_width(180.0).hint_text(if self.json { "$.name" } else { "Field name" }));
                ui.add(egui::TextEdit::singleline(&mut field.alias).char_limit(128).desired_width(140.0).hint_text(if self.json { "Alias (required)" } else { "Alias (optional)" }));
                egui::ComboBox::from_id_salt("kind").selected_text(field.kind.label()).width(90.0).show_ui(ui, |ui| {
                    for kind in [Kind::Text, Kind::Tag, Kind::Numeric, Kind::Geo] { ui.selectable_value(&mut field.kind, kind, kind.label()); }
                });
                ui.checkbox(&mut field.sortable, "Sortable");
                if crate::redis_value_editor::icon(ui, "x", "Remove field", true).clicked() { remove = Some(index); }
            }); });
        }
        if let Some(index) = remove { self.fields.remove(index); }
        if ui.add_enabled(self.fields.len() < 32, crate::redis_workspace::action_button("plus", "Add field")).clicked() { self.fields.push(Field::default()); }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    #[test]
    fn safe_mode_and_definition_limits_are_enforced_before_commands() {
        let mut draft = Create { name: "idx:users".into(), prefix: "users:".into(), ..Default::default() };
        draft.fields[0].path = "name".into();
        assert!(Mutation::Create(draft.clone()).command(true).is_err());
        assert!(Mutation::Create(draft.clone()).command(false).is_ok());
        draft.fields.push(draft.fields[0].clone());
        assert!(Mutation::Create(draft.clone()).command(false).is_err());
        draft.fields.pop(); draft.json = true;
        assert!(Mutation::Create(draft.clone()).command(false).is_err());
        draft.fields[0].path = "$.name".into(); draft.fields[0].alias = "name".into();
        assert!(Mutation::Create(draft).command(false).is_ok());
        let drop = Mutation::Drop(b"index with spaces".to_vec());
        assert!(drop.command(true).is_err());
        let packed = drop.command(false).unwrap().get_packed_command();
        assert!(!packed.windows(2).any(|bytes| bytes == b"DD"));
        let args: Vec<Vec<u8>> = redis::from_redis_value(redis::Parser::new().parse_value(packed.as_slice()).unwrap()).unwrap();
        assert_eq!(args, vec![b"FT.DROPINDEX".to_vec(), b"index with spaces".to_vec()]);
    }
}
