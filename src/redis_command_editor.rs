//! Redis command editor with SQL-compatible execution shortcuts and cached color.
use eframe::egui::{self, Color32, FontId, TextFormat, text::LayoutJob};
#[derive(Default)]
pub(crate) struct Editor {
    cursor: std::ops::Range<usize>,
}
impl Editor {
    pub fn toolbar(&mut self, ui: &mut egui::Ui, text: &str, enabled: bool, busy: bool) -> Option<String> {
        let mut run = None;
        let can_run = enabled && !text.trim().is_empty() && text.len() <= 1024 * 1024;
        if ui
            .add_enabled(
                can_run,
                crate::icons::query_run_button(
                    if self.cursor.is_empty() {
                        "Run current"
                    } else {
                        "Run selection"
                    },
                    busy,
                    Color32::from_rgb(50, 104, 224),
                ),
            )
            .on_hover_text(format!(
                "{} · current command or selected text",
                crate::icons::shortcut(ui.ctx(), false, "Enter")
            ))
            .clicked()
        {
            run = Some(false);
        }
        if ui
            .add_enabled(
                can_run,
                crate::icons::shortcut_button("Run all", true, "Enter", None),
            )
            .clicked()
        {
            run = Some(true);
        }
        run.and_then(|all| target(text, self.cursor.clone(), all))
    }
    pub fn show(&mut self, ui: &mut egui::Ui, text: &mut String, enabled: bool) -> Option<String> {
        let id = ui.make_persistent_id("redis-command-editor");
        let focused = ui.memory(|m| m.has_focus(id));
        let run = if focused {
            ui.input_mut(|i| {
                if i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::Enter,
                ) {
                    Some(true)
                } else if i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter) {
                    Some(false)
                } else {
                    None
                }
            })
        } else {
            None
        };
        let can_run = enabled && !text.trim().is_empty() && text.len() <= 1024 * 1024;
        let height = (ui.available_height() * 0.55).clamp(54.0, 220.0);
        panel(ui, 8, |ui| {
                crate::editor::selection_style(ui.visuals_mut());
                egui::ScrollArea::vertical()
                    .id_salt("redis-command-editor-scroll")
                    .max_height(height)
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            let (gutter, _) = ui.allocate_exact_size(
                                egui::vec2(
                                    (text.len().max(1).ilog10() as f32 * 8.5 + 16.5).max(32.0),
                                    90.0,
                                ),
                                egui::Sense::hover(),
                            );
                            let output = egui::TextEdit::multiline(text)
                                .id(id)
                                .code_editor()
                                .frame(false)
                                .desired_rows(5)
                                .min_size(egui::vec2(0.0, height))
                                .desired_width(ui.available_width())
                                .layouter(&mut layout)
                                .show(ui);
                            if let Some(cursor) = output.cursor_range {
                                self.cursor = cursor.as_sorted_char_range();
                            }
                            let visual_row = output
                                .galley
                                .layout_from_cursor(egui::text::CCursor::new(self.cursor.start))
                                .row;
                            let current_line = 1 + output
                                .galley
                                .rows
                                .iter()
                                .take(visual_row)
                                .filter(|row| row.ends_with_newline)
                                .count();
                            crate::editor::paint_line_numbers(ui, gutter, &output, current_line);
                        });
                    });
            });
        ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.weak("One command per line · quote values containing spaces · \\xHH for binary bytes");
        });
        if text.len() > 1024 * 1024 {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Commands exceed the 1 MiB execution limit.",
            );
        }
        if can_run {
            run.and_then(|all| target(text, self.cursor.clone(), all))
        } else {
            None
        }
    }
}
/// Full-width editor/result surface with horizontal dividers only.
pub(crate) fn panel<R>(ui: &mut egui::Ui, padding: i8, contents: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    let response = egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .inner_margin(padding)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            contents(ui)
        });
    let rect = response.response.rect;
    ui.painter().line_segment([rect.left_top(), rect.right_top()], stroke);
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], stroke);
    response
}
pub(crate) fn target(text: &str, cursor: std::ops::Range<usize>, all: bool) -> Option<String> {
    if all {
        return (!text.trim().is_empty()).then(|| text.to_owned());
    }
    let byte = |n| text.char_indices().nth(n).map_or(text.len(), |(i, _)| i);
    let start = byte(cursor.start);
    let end = byte(cursor.end);
    if start != end {
        return Some(text[start..end].to_owned());
    }
    let mut from = 0;
    let mut quote = None;
    let mut escaped = false;
    for (index, b) in text.bytes().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if b == b'\\' {
            escaped = true;
            continue;
        }
        if Some(b) == quote {
            quote = None;
        } else if quote.is_none() && matches!(b, b'\'' | b'"') {
            quote = Some(b);
        } else if quote.is_none() && matches!(b, b'\n' | b'\r') {
            if start <= index {
                let command = &text[from..index];
                return (!command.trim().is_empty()).then(|| command.to_owned());
            }
            from = index + 1;
        }
    }
    let command = &text[from..];
    (!command.trim().is_empty()).then(|| command.to_owned())
}
#[derive(Default)]
struct Highlighter;
impl egui::cache::ComputerMut<(&str, bool), LayoutJob> for Highlighter {
    fn compute(&mut self, (code, dark): (&str, bool)) -> LayoutJob {
        highlight(code, dark)
    }
}
pub(crate) fn layout(ui: &egui::Ui, text: &dyn egui::TextBuffer, width: f32) -> std::sync::Arc<egui::Galley> {
    type Cache = egui::cache::FrameCache<LayoutJob, Highlighter>;
    let mut job = ui.ctx().memory_mut(|m| {
        m.caches
            .cache::<Cache>()
            .get((text.as_str(), ui.visuals().dark_mode))
    });
    job.wrap.max_width = width.max(1.0);
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}
fn highlight(code: &str, dark: bool) -> LayoutJob {
    let plain = if dark {
        Color32::from_rgb(218, 224, 235)
    } else {
        Color32::from_rgb(45, 55, 72)
    };
    let keyword = if dark {
        Color32::from_rgb(77, 166, 255)
    } else {
        Color32::from_rgb(0, 102, 194)
    };
    let string = if dark {
        Color32::from_rgb(141, 202, 159)
    } else {
        Color32::from_rgb(37, 123, 79)
    };
    let number = if dark {
        Color32::from_rgb(185, 137, 238)
    } else {
        Color32::from_rgb(128, 67, 174)
    };
    let mut job = LayoutJob::default();
    let mut append = |text: &str, color| {
        job.append(
            text,
            0.0,
            TextFormat {
                font_id: FontId::monospace(14.0),
                line_height: Some(18.0),
                color,
                ..Default::default()
            },
        )
    };
    if code.is_empty() || code.len() > 65536 {
        append(code, plain);
        return job;
    }
    let bytes = code.as_bytes();
    let mut i = 0;
    let mut first = true;
    let mut tokens = 0;
    while i < bytes.len() {
        let start = i;
        if bytes[i].is_ascii_whitespace() {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                if matches!(bytes[i], b'\n' | b'\r') {
                    first = true;
                }
                i += 1;
            }
            append(&code[start..i], plain);
            continue;
        }
        let mut quote = None;
        let mut quoted = false;
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'\\' {
                i = (i + 2).min(bytes.len());
                continue;
            }
            if Some(b) == quote {
                quote = None;
            } else if quote.is_none() && matches!(b, b'\'' | b'"') {
                quote = Some(b);
                quoted = true;
            } else if quote.is_none() && b.is_ascii_whitespace() {
                break;
            }
            i += 1;
        }
        let word = &code[start..i];
        append(
            word,
            if first {
                keyword
            } else if quoted {
                string
            } else if word.parse::<f64>().is_ok() {
                number
            } else if [
                "MATCH",
                "COUNT",
                "TYPE",
                "EX",
                "PX",
                "NX",
                "XX",
                "GET",
                "WITHSCORES",
                "LIMIT",
                "BYLEX",
                "BYSCORE",
                "REV",
                "STREAMS",
                "BLOCK",
                "VALUES",
                "ELE",
                "EF",
                "FILTER",
            ]
            .iter()
            .any(|option| word.eq_ignore_ascii_case(option))
            {
                keyword
            } else {
                plain
            },
        );
        first = false;
        tokens += 1;
        if tokens >= 4096 {
            append(&code[i..], plain);
            break;
        }
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn execution_targets_preserve_unicode_quotes_and_escaped_newlines() {
        let text = "GET café\nSET key \"first\nsecond\"\nPING";
        assert_eq!(target(text, 5..5, false).unwrap(), "GET café");
        assert_eq!(
            target(text, 24..24, false).unwrap(),
            "SET key \"first\nsecond\""
        );
        assert_eq!(target(text, 4..8, false).unwrap(), "café");
        assert_eq!(target(text, 0..0, true).unwrap(), text);
        assert_eq!(
            target("SET a x\\\ny\nPING", 10..10, false).unwrap(),
            "SET a x\\\ny"
        );
        assert!(target("\nPING", 0..0, false).is_none());
    }
    #[test]
    fn highlighting_preserves_bytes_and_distinguishes_tokens() {
        let text = "SET café \"hello \\\"world\\\"\" EX 60\nGET café\nSET key \"two\nlines\"";
        for dark in [false, true] {
            let job = highlight(text, dark);
            assert_eq!(job.text, text);
            assert!(
                job.sections
                    .iter()
                    .map(|s| s.format.color)
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    >= 4
            );
            assert!(
                job.sections
                    .iter()
                    .all(|s| text.is_char_boundary(s.byte_range.start)
                        && text.is_char_boundary(s.byte_range.end))
            );
        }
        assert_eq!(highlight(&"x".repeat(65537), true).sections.len(), 1);
        assert_eq!(highlight("", true).sections[0].format.font_id, FontId::monospace(14.0));
    }
    fn frame(
        ctx: &egui::Context,
        editor: &mut Editor,
        text: &mut String,
        events: Vec<egui::Event>,
        enabled: bool,
        focus: bool,
    ) -> (Option<String>, egui::Id) {
        let mut action = None;
        let mut id = egui::Id::NULL;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 500.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    id = ui.make_persistent_id("redis-command-editor");
                    if focus {
                        ui.memory_mut(|m| m.request_focus(id));
                    }
                    action = editor.show(ui, text, enabled);
                });
            },
        );
        (action, id)
    }
    #[test]
    fn shortcuts_are_focused_guarded_and_do_not_insert_newlines() {
        let ctx = egui::Context::default();
        let mut editor = Editor::default();
        let mut text = "GET café\nPING".to_owned();
        let (_, id) = frame(&ctx, &mut editor, &mut text, vec![], true, true);
        let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(2),
            )));
        state.store(&ctx, id);
        let key = |shift| egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: if shift {
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT
            } else {
                egui::Modifiers::COMMAND
            },
        };
        assert_eq!(
            frame(&ctx, &mut editor, &mut text, vec![key(false)], true, true)
                .0
                .as_deref(),
            Some("GET café")
        );
        let original = text.clone();
        assert_eq!(
            frame(&ctx, &mut editor, &mut text, vec![key(true)], true, true)
                .0
                .as_deref(),
            Some(original.as_str())
        );
        assert!(
            frame(&ctx, &mut editor, &mut text, vec![key(false)], false, true)
                .0
                .is_none()
        );
        assert_eq!(text, original);
        let enter = egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        assert!(
            frame(&ctx, &mut editor, &mut text, vec![enter], true, true)
                .0
                .is_none()
        );
        assert_eq!(
            text.matches('\n').count(),
            original.matches('\n').count() + 1
        );

        ctx.memory_mut(|m| m.surrender_focus(id));
        assert!(
            frame(&ctx, &mut editor, &mut text, vec![key(false)], true, false)
                .0
                .is_none()
        );
    }
}
