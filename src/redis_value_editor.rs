use eframe::egui::{self, Color32, FontId, TextFormat, text::LayoutJob};

pub(crate) fn icon(ui: &mut egui::Ui, name: &str, hint: &str, enabled: bool) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        crate::icons::button(ui, name, egui::vec2(28.0, 28.0), false)
    })
    .inner
    .on_hover_text(hint)
}

pub(crate) fn copy(
    ui: &mut egui::Ui,
    salt: &str,
    label: Option<&'static str>,
    enabled: bool,
    value: impl FnOnce() -> String,
) -> egui::Response {
    let id = ui.make_persistent_id(("redis-copy-feedback", salt));
    let now = ui.input(|i| i.time);
    let copied = ui
        .data(|d| d.get_temp::<f64>(id))
        .is_some_and(|until| now < until);
    let response = if let Some(label) = label {
        let width=ui.painter().layout_no_wrap(label.into(),egui::TextStyle::Button.resolve(ui.style()),ui.visuals().text_color()).size().x+40.0;
        ui.add_enabled(
            enabled,
            crate::redis_workspace::action_button(
                if copied { "check" } else { "clipboard" },
                if copied { "Copied" } else { label },
            )
            .min_size(egui::vec2(
                width.max(80.0),
                crate::controls::BUTTON_HEIGHT,
            )),
        )
    } else {
        icon(
            ui,
            if copied { "check" } else { "clipboard" },
            if copied { "Copied" } else { "Copy key name" },
            enabled,
        )
    };
    if response.clicked() {
        ui.ctx().copy_text(value());
        ui.data_mut(|d| d.insert_temp(id, now + 1.5));
        ui.ctx().request_repaint();
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(1500));
    }
    response
}

#[derive(Default)]
struct JsonHighlighter;
impl egui::cache::ComputerMut<(&str, bool), LayoutJob> for JsonHighlighter {
    fn compute(&mut self, (text, dark): (&str, bool)) -> LayoutJob {
        highlight(text, dark)
    }
}

pub(crate) fn layout(
    ui: &egui::Ui,
    text: &dyn egui::TextBuffer,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    type Cache = egui::cache::FrameCache<LayoutJob, JsonHighlighter>;
    let mut job = ui.ctx().memory_mut(|m| {
        m.caches
            .cache::<Cache>()
            .get((text.as_str(), ui.visuals().dark_mode))
    });
    job.wrap.max_width = width.max(1.0);
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

fn highlight(text: &str, dark: bool) -> LayoutJob {
    use crate::redis_value::JsonKind;
    let mut job = LayoutJob::default();
    let mut append = |s: &str, color: Color32| {
        job.append(
            s,
            0.0,
            TextFormat {
                font_id: FontId::monospace(14.0),
                line_height: Some(18.0),
                color,
                ..Default::default()
            },
        )
    };
    if text.is_empty() || text.len() > 65536 {
        append(text, JsonKind::Punctuation.color(dark));
        return job;
    }
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut count = 0;
    while i < bytes.len() {
        let start = i;
        let kind = match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    let ch = text[i..].chars().next().unwrap();
                    i += ch.len_utf8();
                    if ch == '\\' {
                        if let Some(ch) = text[i..].chars().next() {
                            i += ch.len_utf8();
                        }
                    } else if ch == '"' {
                        break;
                    }
                }
                if bytes[i..].iter().find(|b| !b.is_ascii_whitespace()) == Some(&b':') {
                    JsonKind::Key
                } else {
                    JsonKind::String
                }
            }
            b'-' | b'0'..=b'9' => {
                i += 1;
                while i < bytes.len()
                    && matches!(bytes[i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
                {
                    i += 1;
                }
                JsonKind::Number
            }
            b't' | b'f' | b'n' => {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                JsonKind::Literal
            }
            _ => {
                i += text[i..].chars().next().unwrap().len_utf8();
                JsonKind::Punctuation
            }
        };
        append(&text[start..i], kind.color(dark));
        count += 1;
        if count == 4096 {
            append(&text[i..], JsonKind::Punctuation.color(dark));
            break;
        }
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copy_feedback_is_lazy_and_expires_without_idle_polling() {
        let ctx = egui::Context::default();
        let mut rect = egui::Rect::NOTHING;
        let mut copies = 0;
        let mut id = egui::Id::NULL;
        for frame in 0..80 {
            let events = if frame == 1 || frame == 2 {
                vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed: frame == 1,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            } else {
                vec![]
            };
            let output = ctx.run(
                egui::RawInput {
                    time: Some(frame as f64 / 10.0),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        id = ui.make_persistent_id(("redis-copy-feedback", "test"));
                        rect = copy(ui, "test", Some("Copy value"), true, || {
                            copies += 1;
                            "value".into()
                        })
                        .rect;
                    });
                },
            );
            if frame == 0 {
                assert_eq!(copies, 0);
            }
            if frame == 2 {
                assert_eq!(copies, 1);
                assert!(ctx.data(|d| d.get_temp::<f64>(id)).unwrap() > 0.2);
                assert!(
                    output
                        .platform_output
                        .commands
                        .iter()
                        .any(|c| matches!(c, egui::OutputCommand::CopyText(s) if s == "value"))
                );
            }
        }
        assert_eq!(copies, 1);
        assert!(!ctx.has_requested_repaint());
    }
    #[test]
    fn incomplete_json_and_unicode_drafts_remain_editable() {
        for text in [
            "",
            "{\"café\": \"猫",
            "{\"x\":\"\\",
            "🙂 : true",
            "{\"x\":12,\"yes\":true}",
        ] {
            let job = highlight(text, true);
            assert_eq!(job.text, text);
            assert!(
                job.sections
                    .iter()
                    .all(|s| text.is_char_boundary(s.byte_range.start)
                        && text.is_char_boundary(s.byte_range.end))
            );
        }
        assert_eq!(highlight(&"x".repeat(65537), false).sections.len(), 1);
    }
}
