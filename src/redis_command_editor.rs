//! Redis command editor with SQL-compatible execution shortcuts and cached color.
use eframe::egui::{self, Color32, FontId, TextFormat, text::LayoutJob};
#[derive(Default)]
pub(crate) struct Editor {
    cursor: std::ops::Range<usize>,
    suggestions: crate::redis_completion::Suggestions,
    selected: usize,
    dismissed: bool,
    focus_requested: bool,
    last_context: Option<(u64, usize, usize)>,
}
impl Editor {
    #[cfg(test)]
    pub(crate) fn show_capability_benchmark(&mut self, ui: &mut egui::Ui, text: &mut String, enabled: bool) -> Option<String> {
        let caps = crate::redis_capabilities::Capabilities { version: Some([6,2,0]), commands: [Some(false);4] };
        self.show_with_context(ui, text, enabled, &crate::redis_completion::Context { capabilities: Some(&caps), ..Default::default() })
    }
    pub fn focus(&mut self) {
        self.focus_requested = true;
    }
    pub fn toolbar(
        &mut self,
        ui: &mut egui::Ui,
        text: &str,
        enabled: bool,
        busy: bool,
    ) -> Option<String> {
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
    #[cfg(test)]
    pub fn show(&mut self, ui: &mut egui::Ui, text: &mut String, enabled: bool) -> Option<String> {
        self.show_with_context(
            ui,
            text,
            enabled,
            &crate::redis_completion::Context::default(),
        )
    }
    pub fn show_with_context(
        &mut self,
        ui: &mut egui::Ui,
        text: &mut String,
        enabled: bool,
        context: &crate::redis_completion::Context<'_>,
    ) -> Option<String> {
        let id = ui.make_persistent_id("redis-command-editor");
        if self.focus_requested {
            self.focus_requested = false;
            self.dismissed = false;
            let caret = text.chars().count();
            self.cursor = caret..caret;
            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(caret),
                )));
            state.store(ui.ctx(), id);
            ui.memory_mut(|m| m.request_focus(id));
        }
        let focused = ui.is_enabled() && ui.memory(|m| m.has_focus(id));
        let explicit =
            focused && ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Space));
        if explicit {
            self.dismissed = false;
            self.last_context = None;
        }
        if focused && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.dismissed = true;
        }
        let selection_before = self.selected;
        let mut accept = None;
        if focused && !self.dismissed && !self.suggestions.items.is_empty() {
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    self.dismissed = true;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    self.selected = (self.selected + 1) % self.suggestions.items.len();
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    self.selected = (self.selected + self.suggestions.items.len() - 1)
                        % self.suggestions.items.len();
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                {
                    accept = Some(self.selected);
                }
            });
        }
        if let Some(index) = accept {
            self.accept(ui.ctx(), id, text, index);
        }
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
        if run.is_some() {
            self.dismissed = true;
        }
        let can_run = enabled && !text.trim().is_empty() && text.len() <= 1024 * 1024;
        let height = (ui.available_height() * 0.55).clamp(54.0, 220.0);
        let mut anchor = ui.cursor().min;
        let mut changed = false;
        let mut has_focus = focused;
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
                        changed = output.response.changed();
                        has_focus = ui.is_enabled() && output.response.has_focus();
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
                        let caret = output
                            .galley
                            .pos_from_cursor(egui::text::CCursor::new(self.cursor.end));
                        anchor = output.galley_pos + caret.left_bottom().to_vec2();
                        crate::editor::paint_line_numbers(ui, gutter, &output, current_line);
                    });
                });
        });
        if (has_focus || focused) && self.cursor.is_empty() {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            text.hash(&mut h);
            context.capabilities.hash(&mut h);
            let stamp = (
                h.finish(),
                self.cursor.end,
                context.keys.len() + context.indexes.len(),
            );
            if self.last_context != Some(stamp) {
                self.suggestions =
                    crate::redis_completion::suggest_with_context(text, self.cursor.end, context);
                self.selected = 0;
                self.last_context = Some(stamp);
                if changed {
                    self.dismissed = false;
                }
            }
            if !self.dismissed
                && (!self.suggestions.items.is_empty() || !self.suggestions.signature.is_empty())
            {
                let bounds = ui.ctx().content_rect();
                let width = 480.0_f32.min(bounds.width() - 16.0);
                let pos = egui::pos2(
                    anchor
                        .x
                        .min(bounds.right() - width - 8.0)
                        .max(bounds.left() + 8.0),
                    (anchor.y + 4.0)
                        .min(bounds.bottom() - 280.0)
                        .max(bounds.top() + 8.0),
                );
                let mut clicked = None;
                egui::Area::new(id.with("intellisense"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(pos)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style())
                            .inner_margin(8)
                            .show(ui, |ui| {
                                ui.set_width(width);
                                let selected = self.suggestions.items.get(self.selected);
                                let signature =
                                    selected.map_or(self.suggestions.signature, |i| i.signature);
                                if !signature.is_empty() {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(signature).monospace().strong(),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(signature);
                                    ui.add(
                                        egui::Label::new(
                                            selected.map_or(self.suggestions.summary, |i| i.detail),
                                        )
                                        .wrap(),
                                    );
                                    let warning = selected.map_or(self.suggestions.warning, |i| i.warning);
                                    if !warning.is_empty() { ui.colored_label(ui.visuals().warn_fg_color, warning); }
                                    if !self.suggestions.argument.is_empty() {
                                        ui.weak(format!("Expected: {}", self.suggestions.argument));
                                    }
                                }
                                if !self.suggestions.items.is_empty() {
                                    ui.visuals_mut().clip_rect_margin = 0.0;
                                    let row_stride = 25.0 + ui.spacing().item_spacing.y;
                                    let mut list = egui::ScrollArea::vertical()
                                        .id_salt("redis-completions")
                                        .max_height(6.0 * row_stride - ui.spacing().item_spacing.y);
                                    if selection_before != self.selected || changed || explicit {
                                        list = list.vertical_scroll_offset(
                                            self.selected.saturating_sub(5) as f32 * row_stride,
                                        );
                                    }
                                    list.show_rows(
                                        ui,
                                        25.0,
                                        self.suggestions.items.len(),
                                        |ui, range| {
                                            for i in range {
                                                let item = &self.suggestions.items[i];
                                                let r = ui.add_sized(
                                                    [width, 25.0],
                                                    egui::Button::selectable(
                                                        i == self.selected,
                                                        egui::RichText::new(&item.label)
                                                            .monospace(),
                                                    )
                                                    .shortcut_text(
                                                        if item.detail.starts_with("Loaded") {
                                                            "name"
                                                        } else if item
                                                            .signature
                                                            .starts_with(&item.label)
                                                        {
                                                            "command"
                                                        } else {
                                                            "option"
                                                        },
                                                    ),
                                                );
                                                if r.clicked() {
                                                    clicked = Some(i);
                                                }
                                                r.on_hover_text(item.detail);
                                            }
                                        },
                                    );
                                    ui.separator();
                                    ui.weak("↑ ↓ Navigate   Tab / Enter Insert   Esc Dismiss");
                                }
                            });
                    });
                if let Some(index) = clicked {
                    self.accept(ui.ctx(), id, text, index);
                }
            }
        }
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.weak("One command per line · Ctrl+Space for suggestions · \\xHH for binary bytes");
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
    fn accept(&mut self, ctx: &egui::Context, id: egui::Id, text: &mut String, index: usize) {
        let Some(item) = self.suggestions.items.get(index) else {
            return;
        };
        let range = self.suggestions.replace.clone();
        if range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            return;
        }
        let caret = text[..range.start].chars().count() + item.insert.chars().count();
        text.replace_range(range, &item.insert);
        if let Some(mut state) = egui::text_edit::TextEditState::load(ctx, id) {
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(caret),
                )));
            state.store(ctx, id);
        }
        self.cursor = caret..caret;
        self.dismissed = false;
        self.last_context = None;
        self.suggestions = Default::default();
        ctx.memory_mut(|m| m.request_focus(id));
    }
}
/// Full-width editor/result surface with horizontal dividers only.
pub(crate) fn panel<R>(
    ui: &mut egui::Ui,
    padding: i8,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    let response = egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .inner_margin(padding)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            contents(ui)
        });
    let rect = response.response.rect;
    ui.painter()
        .line_segment([rect.left_top(), rect.right_top()], stroke);
    ui.painter()
        .line_segment([rect.left_bottom(), rect.right_bottom()], stroke);
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
pub(crate) fn layout(
    ui: &egui::Ui,
    text: &dyn egui::TextBuffer,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
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
        assert_eq!(
            highlight("", true).sections[0].format.font_id,
            FontId::monospace(14.0)
        );
    }
    #[test]
    fn typing_exact_command_resets_selection_and_accepts_exact_match() {
        let ctx = egui::Context::default();
        let mut editor = Editor::default();
        let mut text = "GE".to_owned();
        let (_, id) = frame(&ctx, &mut editor, &mut text, vec![], true, true);
        let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(2))));
        state.store(&ctx, id);
        frame(&ctx, &mut editor, &mut text, vec![], true, true);
        editor.selected = 2;
        frame(&ctx, &mut editor, &mut text, vec![egui::Event::Text("T".into())], true, true);
        assert_eq!(text, "GET");
        assert_eq!(editor.selected, 0);
        assert_eq!(editor.suggestions.items[0].label, "GET");
        frame(&ctx, &mut editor, &mut text, vec![egui::Event::Key {
            key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false,
            modifiers: egui::Modifiers::NONE,
        }], true, true);
        assert_eq!(text, "GET ");
    }

    #[test]
    fn completion_keyboard_acceptance_and_dismissal() {
        let ctx = egui::Context::default();
        let mut editor = Editor::default();
        let mut text = "HG".to_owned();
        let (_, id) = frame(&ctx, &mut editor, &mut text, vec![], true, true);
        let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(2),
            )));
        state.store(&ctx, id);
        frame(&ctx, &mut editor, &mut text, vec![], true, true);
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        assert!(!editor.suggestions.items.is_empty());
        let expected = editor.suggestions.items[1].insert.clone();
        frame(
            &ctx,
            &mut editor,
            &mut text,
            vec![key(egui::Key::ArrowDown)],
            true,
            true,
        );
        assert_eq!(editor.selected, 1);
        let action = frame(
            &ctx,
            &mut editor,
            &mut text,
            vec![key(egui::Key::Tab)],
            true,
            true,
        )
        .0;
        assert!(action.is_none());
        assert_eq!(text, expected);
        frame(
            &ctx,
            &mut editor,
            &mut text,
            vec![key(egui::Key::Escape)],
            true,
            true,
        );
        let before = text.clone();
        frame(
            &ctx,
            &mut editor,
            &mut text,
            vec![key(egui::Key::Enter)],
            true,
            true,
        );
        assert_eq!(text, format!("{before}\n"));
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
    fn capability_refresh_updates_suggestions_without_text_edits() {
        let ctx = egui::Context::default();
        let mut editor = Editor::default();
        let mut text = "LMP".to_owned();
        let old = crate::redis_capabilities::Capabilities { version: Some([6,2,0]), ..Default::default() };
        let new = crate::redis_capabilities::Capabilities { version: Some([7,2,0]), ..Default::default() };
        for frame in 0..6 {
            let caps = if frame < 3 { &old } else { &new };
            let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0,500.0))), ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let id = ui.make_persistent_id("redis-command-editor");
                    ui.memory_mut(|m| m.request_focus(id));
                    if let Some(mut state) = egui::text_edit::TextEditState::load(ctx, id) {
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(3))));
                        state.store(ctx,id);
                    }
                    editor.show_with_context(ui, &mut text, true, &crate::redis_completion::Context { capabilities: Some(caps), ..Default::default() });
                });
            });
            if frame >= 1 {
                let item = editor.suggestions.items.iter().find(|i| i.label == "LMPOP").unwrap();
                assert_eq!(item.warning.is_empty(), frame >= 3);
            }
            assert_eq!(text,"LMP");
        }
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
