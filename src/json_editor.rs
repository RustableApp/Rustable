//! Multi-line JSON viewer/editor for cell values.
//!
//! Formatting re-indents the original tokens instead of round-tripping through
//! `serde_json::Value`: numbers keep their exact digits (no f64 rounding), keys
//! keep their order, and duplicate keys survive. Validation uses `IgnoredAny`,
//! which checks syntax without converting values.
use crate::db::Value;
use eframe::egui::{self, Color32, FontId, RichText, TextFormat, text::LayoutJob};

/// Values above this size stay read-only, matching the inline editor limit.
pub(crate) const EDIT_LIMIT: usize = 64 * 1024;
/// Larger values are not laid out at all; they can still be copied.
const VIEW_LIMIT: usize = 1024 * 1024;

pub(crate) fn is_json_type(data_type: &str) -> bool {
    matches!(data_type.trim().to_ascii_lowercase().as_str(), "json" | "jsonb")
}

/// Text that is a JSON object or array, for columns without a JSON type
/// (SQLite TEXT, MariaDB's LONGTEXT alias, query expressions). Called while
/// rendering, so only values within the edit limit are fully parsed.
pub(crate) fn looks_like_json(text: &str) -> bool {
    let trimmed = text.trim_start();
    (trimmed.starts_with('{') || trimmed.starts_with('['))
        && (text.len() > EDIT_LIMIT || validate(text).is_ok())
}

pub(crate) fn validate(text: &str) -> Result<(), String> {
    serde_json::from_str::<serde::de::IgnoredAny>(text)
        .map(|_| ())
        .map_err(|error| format!("Line {}, column {}: {}", error.line(), error.column(), message(&error)))
}

fn message(error: &serde_json::Error) -> String {
    let text = error.to_string();
    // serde_json appends " at line X column Y"; the position is shown separately.
    match text.rfind(" at line ") {
        Some(index) => text[..index].to_owned(),
        None => text,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Open,
    Close,
    Colon,
    Comma,
    String,
    Space,
    Other,
}

fn tokens(text: &str) -> Vec<(Kind, &str)> {
    let bytes = text.as_bytes();
    let mut result = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        let (kind, end) = match bytes[start] {
            b'{' | b'[' => (Kind::Open, start + 1),
            b'}' | b']' => (Kind::Close, start + 1),
            b':' => (Kind::Colon, start + 1),
            b',' => (Kind::Comma, start + 1),
            b'"' => {
                let mut end = start + 1;
                while end < bytes.len() && bytes[end] != b'"' {
                    end += if bytes[end] == b'\\' { 2 } else { 1 };
                }
                (Kind::String, (end + 1).min(bytes.len()))
            }
            b' ' | b'\t' | b'\n' | b'\r' => {
                let mut end = start + 1;
                while end < bytes.len() && matches!(bytes[end], b' ' | b'\t' | b'\n' | b'\r') {
                    end += 1;
                }
                (Kind::Space, end)
            }
            _ => {
                let mut end = start + 1;
                while end < bytes.len()
                    && !matches!(bytes[end], b'{' | b'[' | b'}' | b']' | b':' | b',' | b'"' | b' ' | b'\t' | b'\n' | b'\r')
                {
                    end += 1;
                }
                (Kind::Other, end)
            }
        };
        // Token boundaries are ASCII, so slicing never splits a UTF-8 sequence.
        result.push((kind, &text[start..end]));
        start = end;
    }
    result
}

pub(crate) fn pretty(text: &str) -> Result<String, String> {
    validate(text)?;
    let tokens: Vec<_> = tokens(text).into_iter().filter(|(kind, _)| *kind != Kind::Space).collect();
    let mut output = String::with_capacity(text.len() + text.len() / 4);
    let mut depth = 0usize;
    let newline = |output: &mut String, depth: usize| {
        output.push('\n');
        for _ in 0..depth {
            output.push_str("  ");
        }
    };
    for (index, (kind, token)) in tokens.iter().enumerate() {
        match kind {
            Kind::Open => {
                output.push_str(token);
                if tokens.get(index + 1).is_some_and(|(next, _)| *next == Kind::Close) {
                    continue;
                }
                depth += 1;
                newline(&mut output, depth);
            }
            Kind::Close => {
                if index > 0 && tokens[index - 1].0 != Kind::Open {
                    depth = depth.saturating_sub(1);
                    newline(&mut output, depth);
                }
                output.push_str(token);
            }
            Kind::Comma => {
                output.push(',');
                newline(&mut output, depth);
            }
            Kind::Colon => output.push_str(": "),
            _ => output.push_str(token),
        }
    }
    Ok(output)
}

pub(crate) fn compact(text: &str) -> Result<String, String> {
    validate(text)?;
    Ok(tokens(text).into_iter().filter(|(kind, _)| *kind != Kind::Space).map(|(_, token)| token).collect())
}

fn highlight(text: &str, dark: bool) -> LayoutJob {
    let color = |light: (u8, u8, u8), night: (u8, u8, u8)| {
        let (r, g, b) = if dark { night } else { light };
        Color32::from_rgb(r, g, b)
    };
    let plain = color((45, 55, 72), (218, 224, 235));
    let key = color((30, 94, 170), (110, 180, 255));
    let string = color((22, 128, 61), (130, 210, 140));
    let number = color((176, 92, 0), (240, 170, 90));
    let literal = color((150, 60, 170), (210, 150, 240));
    let punctuation = color((110, 120, 135), (150, 160, 175));
    let tokens = tokens(text);
    let mut job = LayoutJob::default();
    for (index, (kind, token)) in tokens.iter().enumerate() {
        let foreground = match kind {
            Kind::String => {
                let is_key = tokens[index + 1..]
                    .iter()
                    .find(|(kind, _)| *kind != Kind::Space)
                    .is_some_and(|(kind, _)| *kind == Kind::Colon);
                if is_key { key } else { string }
            }
            Kind::Other if matches!(*token, "true" | "false" | "null") => literal,
            Kind::Other => number,
            Kind::Space => plain,
            _ => punctuation,
        };
        job.append(token, 0.0, TextFormat { font_id: FontId::monospace(13.0), color: foreground, ..Default::default() });
    }
    job
}

#[derive(Default)]
struct Highlighter;
impl egui::cache::ComputerMut<(&str, bool), LayoutJob> for Highlighter {
    fn compute(&mut self, (text, dark): (&str, bool)) -> LayoutJob {
        highlight(text, dark)
    }
}

fn layout(ui: &egui::Ui, text: &dyn egui::TextBuffer, width: f32) -> std::sync::Arc<egui::Galley> {
    type Cache = egui::cache::FrameCache<LayoutJob, Highlighter>;
    let mut job = ui.ctx().memory_mut(|memory| memory.caches.cache::<Cache>().get((text.as_str(), ui.visuals().dark_mode)));
    job.wrap.max_width = width.max(1.0);
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

/// Identity of the cell being edited; staged through the normal change list.
#[derive(Clone, Debug)]
pub(crate) struct Target {
    pub table: String,
    pub row: Vec<Value>,
    pub column: String,
    pub original: Value,
}

pub(crate) struct JsonEditor {
    pub target: Target,
    pub text: String,
    /// None when the value can only be viewed.
    pub writable: bool,
    pub nullable: bool,
    /// JSON column types must receive valid JSON; text columns only warn.
    pub strict: bool,
    pub null: bool,
    dirty: bool,
    focus: bool,
}

pub(crate) enum Outcome {
    Open,
    Close,
    Apply(Value),
}

impl JsonEditor {
    pub(crate) fn new(target: Target, current: &Value, writable: bool, nullable: bool, strict: bool) -> Self {
        let source = match current {
            Value::Text(text) => text.clone(),
            Value::Null => String::new(),
            other => crate::db::display(other),
        };
        let text = match pretty(&source) {
            Ok(formatted) if source.len() <= EDIT_LIMIT => formatted,
            _ => source,
        };
        let editable = writable && text.len() <= EDIT_LIMIT;
        Self {
            target,
            text,
            writable: editable,
            nullable,
            strict,
            null: matches!(current, Value::Null),
            dirty: false,
            focus: true,
        }
    }

    /// Value to stage for the current text. Valid JSON is always stored
    /// compact; formatting in the editor is for reading only.
    pub(crate) fn staged_value(&self) -> Result<Value, String> {
        if self.null {
            return Ok(Value::Null);
        }
        match validate(&self.text) {
            Ok(()) => Ok(Value::Text(compact(&self.text)?)),
            Err(error) if self.strict => Err(error),
            Err(_) => Ok(Value::Text(self.text.clone())),
        }
    }

    pub(crate) fn show(&mut self, ctx: &egui::Context) -> Outcome {
        let mut outcome = Outcome::Open;
        let validation = if self.null { Ok(()) } else { validate(&self.text) };
        let modal = egui::Modal::new(egui::Id::new("json-editor")).show(ctx, |ui| {
            let screen = ctx.content_rect();
            ui.set_width((screen.width() - 64.0).clamp(280.0, 820.0));
            ui.horizontal(|ui| {
                ui.strong(if self.writable { "Edit JSON" } else { "View JSON" });
                ui.label(RichText::new(&self.target.column).monospace().weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::app::clipboard_button(ui).on_hover_text("Copy JSON").clicked() {
                        ui.ctx().copy_text(self.text.clone());
                    }
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let can_format = self.writable && !self.null && validation.is_ok();
                if ui.add_enabled(can_format, egui::Button::new("Format")).on_hover_text("Indent with two spaces; values and key order are preserved").clicked()
                    && let Ok(text) = pretty(&self.text)
                {
                    self.dirty |= text != self.text;
                    self.text = text;
                }
                if ui.add_enabled(can_format, egui::Button::new("Compact")).on_hover_text("Show without whitespace (values are always saved compact)").clicked()
                    && let Ok(text) = compact(&self.text)
                {
                    self.dirty |= text != self.text;
                    self.text = text;
                }
                if self.nullable && self.writable {
                    let mut null = self.null;
                    if ui.checkbox(&mut null, "NULL").changed() {
                        self.null = null;
                        self.dirty = true;
                    }
                }
            });
            ui.add_space(4.0);
            let height = (screen.height() - 220.0).clamp(120.0, 560.0);
            egui::Frame::new()
                .fill(ui.visuals().extreme_bg_color)
                .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                .corner_radius(6)
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    egui::ScrollArea::both().max_height(height).min_scrolled_height(height).auto_shrink([false, false]).show(ui, |ui| {
                        if self.text.len() > VIEW_LIMIT {
                            ui.label(format!("This value is {} KiB. Copy it to view it elsewhere.", self.text.len() / 1024));
                            return;
                        }
                        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, width: f32| layout(ui, text, width);
                        let enabled = self.writable && !self.null;
                        let response = if enabled {
                            ui.add(egui::TextEdit::multiline(&mut self.text)
                                .id(egui::Id::new("json-editor-text"))
                                .code_editor()
                                .frame(false)
                                .desired_width(f32::INFINITY)
                                .desired_rows(12)
                                .layouter(&mut layouter))
                        } else {
                            // A &str buffer keeps selection and copying without allowing edits.
                            ui.add(egui::TextEdit::multiline(&mut self.text.as_str())
                                .id(egui::Id::new("json-editor-text"))
                                .code_editor()
                                .frame(false)
                                .desired_width(f32::INFINITY)
                                .desired_rows(12)
                                .layouter(&mut layouter))
                        };
                        if response.changed() {
                            self.dirty = true;
                        }
                        if self.focus {
                            response.request_focus();
                            self.focus = false;
                        }
                    });
                });
            ui.add_space(6.0);
            match &validation {
                Ok(()) if self.null => { ui.label(RichText::new("The value will be set to NULL.").weak()); }
                Ok(()) => { ui.label(RichText::new("✔ Valid JSON").color(ui.visuals().weak_text_color())); }
                Err(error) => {
                    let note = if self.strict { "" } else { " · this column stores text, so it can still be saved" };
                    ui.label(RichText::new(format!("{error}{note}")).color(ui.visuals().error_fg_color));
                }
            }
            if !self.writable {
                ui.label(RichText::new(if self.text.len() > EDIT_LIMIT {
                    "Read-only: values over 64 KiB must be edited with SQL."
                } else {
                    "Read-only: this result or column cannot be edited."
                }).weak());
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if self.writable {
                    let staged = self.staged_value();
                    let apply = ui.add_enabled(staged.is_ok(), egui::Button::new(format!("Apply · {}", crate::icons::shortcut(ui.ctx(), false, "Enter"))))
                        .on_hover_text("Stage this value as compact JSON. Changes are reviewed as SQL before saving.");
                    let shortcut = ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
                    if (apply.clicked() || shortcut) && let Ok(value) = staged {
                        outcome = if self.dirty { Outcome::Apply(value) } else { Outcome::Close };
                    }
                }
                if ui.button(if self.writable { "Cancel" } else { "Close" }).clicked() {
                    outcome = Outcome::Close;
                }
            });
        });
        if modal.should_close() && matches!(outcome, Outcome::Open) {
            outcome = Outcome::Close;
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_preserves_exact_numbers_key_order_duplicates_and_strings() {
        let source = r#"{"z":12345678901234567890.123456789,"a":[1,{},[],"x, y: {z}"],"a":-0.0e10,"u":"é\"q"}"#;
        let formatted = pretty(source).unwrap();
        assert_eq!(formatted, "{\n  \"z\": 12345678901234567890.123456789,\n  \"a\": [\n    1,\n    {},\n    [],\n    \"x, y: {z}\"\n  ],\n  \"a\": -0.0e10,\n  \"u\": \"\u{e9}\\\"q\"\n}");
        // Escapes are copied verbatim, never decoded.
        assert_eq!(pretty("[\"\\u0041\\n\"]").unwrap(), "[\n  \"\\u0041\\n\"\n]");
        assert_eq!(compact(&formatted).unwrap(), source);
        assert_eq!(pretty("[]").unwrap(), "[]");
        assert_eq!(pretty("  \"é\"  ").unwrap(), "\"é\"");
    }

    #[test]
    fn validation_reports_position_and_rejects_trailing_data() {
        let error = validate("{\n  \"a\": 1,\n}").unwrap_err();
        assert!(error.starts_with("Line 3, column 1:"), "{error}");
        assert!(validate("{} {}").is_err());
        assert!(pretty("{").is_err());
        assert!(looks_like_json(" {\"a\": [1]}"));
        assert!(!looks_like_json("\"text\""));
        assert!(!looks_like_json("{not json}"));
        assert!(is_json_type("JSONB") && is_json_type("json") && !is_json_type("jsonb[]"));
    }

    fn target() -> Target {
        Target { table: "t".into(), row: vec![Value::Integer(1)], column: "doc".into(), original: Value::Null }
    }

    #[test]
    fn values_are_shown_formatted_but_always_staged_compact() {
        let mut editor = JsonEditor::new(target(), &Value::Text(r#"{"a":1,"b":[true]}"#.into()), true, true, true);
        assert!(editor.text.contains('\n'));
        editor.text = editor.text.replace("true", "false");
        assert_eq!(editor.staged_value().unwrap(), Value::Text(r#"{"a":1,"b":[false]}"#.into()));
        // Stored pretty text and text typed with extra whitespace are compacted too.
        let mut editor = JsonEditor::new(target(), &Value::Text("{\n \"a\": 1\n}".into()), true, true, true);
        assert_eq!(editor.text, "{\n  \"a\": 1\n}");
        editor.text = "{\n    \"a\" :  2 ,\n \"s\": \" spaced  text \"\n}".into();
        assert_eq!(editor.staged_value().unwrap(), Value::Text(r#"{"a":2,"s":" spaced  text "}"#.into()));
    }

    #[test]
    fn strict_columns_require_valid_json_and_large_values_are_read_only() {
        let mut editor = JsonEditor::new(target(), &Value::Text("{}".into()), true, false, true);
        editor.text = "{".into();
        assert!(editor.staged_value().is_err());
        editor.strict = false;
        assert_eq!(editor.staged_value().unwrap(), Value::Text("{".into()));
        editor.null = true;
        assert_eq!(editor.staged_value().unwrap(), Value::Null);
        let large = format!("[{}]", vec!["1"; EDIT_LIMIT].join(","));
        assert!(!JsonEditor::new(target(), &Value::Text(large), true, true, true).writable);
    }
}
