//! Shared highlighting plus native editor layout and execution cues.
use eframe::egui::{self, Color32, FontId};
#[path = "sql_highlight.rs"]
mod sql_highlight;
pub use sql_highlight::{editor_layout, layout, preview, selection_style};

#[derive(Default)]
struct TargetComputer;
impl egui::cache::ComputerMut<(&str, usize, crate::sql::Dialect), Option<std::ops::Range<usize>>>
    for TargetComputer
{
    fn compute(
        &mut self,
        (sql, caret, dialect): (&str, usize, crate::sql::Dialect),
    ) -> Option<std::ops::Range<usize>> {
        let range = crate::sql::execution_range_for_dialect(sql, caret..caret, false, dialect)?;
        let text = &sql[range.clone()];
        let start = range.start + text.len() - text.trim_start().len();
        let end = range.end - (text.len() - text.trim_end().len());
        (start < end).then(|| sql[..start].chars().count()..sql[..end].chars().count())
    }
}

/// Fill a reserved paint slot behind TextEdit, after its current-frame caret is known.
/// A normal text selection replaces this cue because that selection is Run Current's target.
pub fn paint_execution_target(
    ui: &egui::Ui,
    slot: egui::layers::ShapeIdx,
    output: &egui::text_edit::TextEditOutput,
    sql: &str,
    cursor: std::ops::Range<usize>,
    dialect: crate::sql::Dialect,
) {
    if !cursor.is_empty() {
        return;
    }
    type Cache = egui::cache::FrameCache<Option<std::ops::Range<usize>>, TargetComputer>;
    let target = ui
        .ctx()
        .memory_mut(|m| m.caches.cache::<Cache>().get((sql, cursor.start, dialect)));
    let Some(target) = target else {
        return;
    };
    let color = if ui.visuals().dark_mode {
        Color32::from_white_alpha(9)
    } else {
        Color32::from_black_alpha(7)
    };
    let clip = output.text_clip_rect.intersect(ui.clip_rect());
    let mut shapes = Vec::new();
    let mut row_start = 0;
    for row in &output.galley.rows {
        let row_end = row_start + row.glyphs.len();
        let start = target.start.max(row_start);
        let end = target.end.min(row_end);
        if start < end {
            let local_start = start - row_start;
            let local_end = end - row_start;
            // Follow the statement's actual text span on each visual row.
            let x1 = output.galley_pos.x + row.pos.x + row.x_offset(local_start);
            let x2 = output.galley_pos.x + row.pos.x + row.x_offset(local_end);
            let rect = egui::Rect::from_min_max(
                egui::pos2(x1, output.galley_pos.y + row.pos.y),
                egui::pos2(x2, output.galley_pos.y + row.pos.y + row.size.y),
            )
            .intersect(clip);
            if rect.is_positive() {
                shapes.push(egui::Shape::rect_filled(rect, 0.0, color));
            }
        }
        row_start = row_end + usize::from(row.ends_with_newline);
    }
    ui.painter().set(slot, egui::Shape::Vec(shapes));
}

/// Database offsets are UTF-8 bytes; the editor paints Unicode character ranges.
pub fn error_token_range(sql: &str, offset: usize) -> Option<std::ops::Range<usize>> {
    if offset > sql.len() || !sql.is_char_boundary(offset) || sql.is_empty() { return None; }
    let chars: Vec<char> = sql.chars().collect();
    let start = sql[..offset].chars().count().min(chars.len() - 1);
    let mut end = start + 1;
    let first = chars[start];
    if matches!(first, '\'' | '"' | '`') {
        while end < chars.len() {
            if chars[end] == first {
                end += 1;
                if chars.get(end) == Some(&first) { end += 1; } else { break; }
            } else { end += 1; }
        }
    } else if first.is_alphanumeric() || first == '_' {
        while end < chars.len() && (chars[end].is_alphanumeric() || matches!(chars[end], '_' | '$')) { end += 1; }
    }
    Some(start..end)
}

pub fn paint_query_error(ui: &egui::Ui, output: &egui::text_edit::TextEditOutput, range: std::ops::Range<usize>) {
    let painter = ui.painter().with_clip_rect(output.text_clip_rect.intersect(ui.clip_rect()));
    let color = Color32::from_rgba_unmultiplied(220, 50, 65, 100);
    let mut row_start = 0;
    for row in &output.galley.rows {
        let row_end = row_start + row.glyphs.len();
        let start = range.start.max(row_start);
        let end = range.end.min(row_end);
        if start < end {
            let left = output.galley_pos.x + row.pos.x + row.x_offset(start - row_start);
            let right = output.galley_pos.x + row.pos.x + row.x_offset(end - row_start);
            let top = output.galley_pos.y + row.pos.y;
            painter.rect_filled(egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(right.max(left + 4.0), top + row.size.y)), 0.0, color);
        }
        row_start = row_end + usize::from(row.ends_with_newline);
    }
}

/// Character offsets match egui cursors (not UTF-8 byte offsets).
pub struct LineIndex {
    starts: Vec<usize>,
}
impl LineIndex {
    fn new(sql: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            sql.chars()
                .enumerate()
                .filter_map(|(i, c)| (c == '\n').then_some(i + 1)),
        );
        Self { starts }
    }
    pub fn line_count(&self) -> usize {
        self.starts.len()
    }
    pub fn line_column(&self, caret: usize) -> (usize, usize) {
        let line = self
            .starts
            .partition_point(|start| *start <= caret)
            .saturating_sub(1);
        (line + 1, caret - self.starts[line] + 1)
    }
}
#[derive(Default)]
struct LineComputer;
impl egui::cache::ComputerMut<&str, std::sync::Arc<LineIndex>> for LineComputer {
    fn compute(&mut self, sql: &str) -> std::sync::Arc<LineIndex> {
        std::sync::Arc::new(LineIndex::new(sql))
    }
}
pub fn line_index(ctx: &egui::Context, sql: &str) -> std::sync::Arc<LineIndex> {
    type Cache = egui::cache::FrameCache<std::sync::Arc<LineIndex>, LineComputer>;
    ctx.memory_mut(|m| m.caches.cache::<Cache>().get(sql))
}

/// The gutter is outside the horizontal scroll area; vertical positions come from
/// the editor's galley so blank lines and wrapped continuations stay aligned.
pub fn paint_line_numbers(
    ui: &egui::Ui,
    gutter: egui::Rect,
    output: &egui::text_edit::TextEditOutput,
    current_line: usize,
) {
    // ScrollArea may enforce a larger minimum viewport than our reserved gutter.
    // Follow the actual text viewport, not the requested gutter height.
    let clip = egui::Rect::from_min_max(
        egui::pos2(gutter.left(), output.text_clip_rect.top()),
        egui::pos2(gutter.right(), output.text_clip_rect.bottom()),
    )
    .intersect(ui.clip_rect());
    let painter = ui.painter().with_clip_rect(clip);
    let mut line = 1;
    let mut first_row = true;
    for row in &output.galley.rows {
        let top = output.galley_pos.y + row.pos.y;
        if top > clip.bottom() {
            break;
        }
        if first_row && top + row.size.y >= clip.top() {
            let color = if line == current_line {
                ui.visuals().text_color()
            } else {
                ui.visuals().weak_text_color()
            };
            let mut job = egui::text::LayoutJob::simple(
                line.to_string(), FontId::monospace(sql_highlight::EDITOR_FONT_SIZE), color, f32::INFINITY);
            job.sections[0].format.line_height = Some(sql_highlight::EDITOR_LINE_HEIGHT);
            let number = ui.fonts_mut(|fonts| fonts.layout_job(job));
            // Centering an independently laid-out label rounds differently from
            // the editor's explicit line height. Match its glyph baseline instead.
            let baseline_offset = row.glyphs.first().zip(number.rows[0].glyphs.first())
                .map_or(0.0, |(text, number)| text.pos.y - number.pos.y);
            painter.galley(
                egui::pos2(gutter.right() - 8.0 - number.size().x, top + baseline_offset),
                number,
                color,
            );
        }
        first_row = row.ends_with_newline;
        if row.ends_with_newline {
            line += 1;
        }
    }
}

#[cfg(test)]
mod line_tests {
    use super::*;
    #[test]
    fn database_error_tokens_use_character_ranges() {
        let sql = "SELECT '😀'; SELECT * \"public\".block";
        let start = sql.find('"').unwrap();
        let range = error_token_range(sql, start).unwrap();
        assert_eq!(sql.chars().skip(range.start).take(range.len()).collect::<String>(), "\"public\"");
        assert!(error_token_range(sql, sql.find('😀').unwrap() + 1).is_none());
        assert!(error_token_range(sql, sql.len() + 1).is_none());
        assert!(error_token_range("", 0).is_none());
        assert_eq!(error_token_range("SELKE", 0), Some(0..5));
    }

    #[test]
    fn empty_editor_first_line_has_full_font_height_and_unclipped_number() {
        let ctx = egui::Context::default();
        let mut text = String::new();
        let output = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    let (gutter, _) =
                        ui.allocate_exact_size(egui::vec2(40.0, 200.0), egui::Sense::hover());
                    let editor = egui::TextEdit::multiline(&mut text)
                        .layouter(&mut editor_layout)
                        .show(ui);
                    assert!(editor.galley.rows[0].size.y >= 13.0);
                    paint_line_numbers(ui, gutter, &editor, 1);
                });
            });
        });
        let number = output.shapes.iter().find(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "1")).unwrap();
        if let egui::Shape::Text(text) = &number.shape {
            assert!(text.pos.y >= number.clip_rect.top() - 0.5);
            assert!(text.pos.y + text.galley.size().y <= number.clip_rect.bottom() + 0.5);
        }
    }
    #[test]
    fn line_numbers_share_editor_baselines_and_font_metrics() {
        for scale in [1.0, 1.5, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(scale);
            let mut sql = "SELECT id,\n\n    customer_name\nFROM customers;".to_owned();
            let mut baselines = Vec::new();
            let output = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let gutter = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(40.0, 300.0));
                    let editor = egui::TextEdit::multiline(&mut sql).layouter(&mut editor_layout).show(ui);
                    for row in &editor.galley.rows {
                        baselines.push(row.glyphs.first().map(|glyph| {
                            (
                                editor.galley_pos.y + row.pos.y + glyph.pos.y,
                                glyph.font_height,
                                glyph.line_height,
                            )
                        }));
                    }
                    paint_line_numbers(ui, gutter, &editor, 1);
                });
            });
            for shape in output.shapes {
                if let egui::Shape::Text(text) = shape.shape
                    && let Ok(line) = text.galley.text().parse::<usize>()
                    && let Some((baseline, font_height, line_height)) = baselines[line - 1] {
                    let row = &text.galley.rows[0];
                    let glyph = &row.glyphs[0];
                    assert!((text.pos.y + row.pos.y + glyph.pos.y - baseline).abs() < 0.01, "scale {scale}, line {line}");
                    assert_eq!(glyph.font_height, font_height);
                    assert_eq!(glyph.line_height, line_height);
                }
            }
        }
    }

    #[test]
    fn logical_lines_handle_unicode_blank_lines_and_trailing_newline() {
        let lines = LineIndex::new("é🙂\n\nSELECT 1;\n");
        assert_eq!(lines.line_count(), 4);
        assert_eq!(lines.line_column(0), (1, 1));
        assert_eq!(lines.line_column(2), (1, 3));
        assert_eq!(lines.line_column(3), (2, 1));
        assert_eq!(lines.line_column(4), (3, 1));
        assert_eq!(lines.line_column(14), (4, 1));
        assert_eq!(LineIndex::new("").line_column(0), (1, 1));
    }
}

#[cfg(test)]
mod indentation_tests {
    use super::*;
    use egui::text::{CCursor, CCursorRange};

    fn frame(
        ctx: &egui::Context,
        text: &mut String,
        events: Vec<egui::Event>,
        time: f64,
    ) -> CCursorRange {
        let mut cursor = None;
        let _ = ctx.run(
            egui::RawInput {
                events,
                time: Some(time),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let output = egui::TextEdit::multiline(text)
                        .id(egui::Id::new("indent-test"))
                        .code_editor()
                        .char_limit(1000)
                        .show(ui);
                    cursor = output.cursor_range;
                });
            },
        );
        cursor.unwrap_or_default()
    }

    fn select(ctx: &egui::Context, primary: usize, secondary: usize) -> CCursorRange {
        let id = egui::Id::new("indent-test");
        ctx.memory_mut(|m| m.request_focus(id));
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap();
        let range = CCursorRange::two(CCursor::new(secondary), CCursor::new(primary));
        state.cursor.set_char_range(Some(range));
        state.store(ctx, id);
        range
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            })
            .collect()
    }

    #[test]
    fn block_indentation_preserves_text_selection_and_immediate_history() {
        for reversed in [false, true] {
            let ctx = egui::Context::default();
            let original = "SELECT 'é🙂',\n  name\nFROM customers;";
            let mut text = original.to_owned();
            frame(&ctx, &mut text, vec![], 0.0);
            let end = original.chars().position(|c| c == 'F').unwrap();
            let before = if reversed {
                select(&ctx, 2, end)
            } else {
                select(&ctx, end, 2)
            };
            frame(&ctx, &mut text, vec![], 0.005);
            let after = frame(
                &ctx,
                &mut text,
                key(egui::Key::Tab, egui::Modifiers::NONE),
                0.01,
            );
            assert_eq!(text, "\tSELECT 'é🙂',\n\t  name\nFROM customers;");
            assert_eq!(
                after.primary.index,
                before.primary.index + if reversed { 1 } else { 2 }
            );
            assert_eq!(
                after.secondary.index,
                before.secondary.index + if reversed { 2 } else { 1 }
            );
            let indented = text.clone();
            let undone = frame(
                &ctx,
                &mut text,
                key(egui::Key::Z, egui::Modifiers::COMMAND),
                0.02,
            );
            assert_eq!(text, original);
            assert_eq!(undone, before);
            let redone = frame(
                &ctx,
                &mut text,
                key(
                    egui::Key::Z,
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                ),
                0.03,
            );
            assert_eq!(text, indented);
            assert_eq!(redone, after);
            frame(
                &ctx,
                &mut text,
                key(egui::Key::Tab, egui::Modifiers::NONE),
                0.04,
            );
            frame(
                &ctx,
                &mut text,
                key(egui::Key::Z, egui::Modifiers::COMMAND),
                0.05,
            );
            assert_eq!(text, indented);
            frame(
                &ctx,
                &mut text,
                key(egui::Key::Tab, egui::Modifiers::SHIFT),
                30.0,
            );
            assert_eq!(text, original);
            frame(
                &ctx,
                &mut text,
                key(egui::Key::Z, egui::Modifiers::COMMAND),
                30.01,
            );
            assert_eq!(text, indented);
        }
    }

    #[test]
    fn unindent_handles_partial_spaces_blank_lines_and_unicode() {
        for (original, expected) in [
            ("  é\n\t🙂\n\n x", "é\n🙂\n\nx"),
            ("a\nb", "a\nb"),
            (" ", ""),
        ] {
            let ctx = egui::Context::default();
            let mut text = original.to_owned();
            frame(&ctx, &mut text, vec![], 0.0);
            select(&ctx, original.chars().count(), 0);
            frame(&ctx, &mut text, vec![], 0.005);
            frame(
                &ctx,
                &mut text,
                key(egui::Key::Tab, egui::Modifiers::SHIFT),
                0.01,
            );
            assert_eq!(text, expected);
        }
    }

    #[test]
    fn tab_after_typing_undoes_only_the_indentation() {
        let ctx = egui::Context::default();
        let mut text = "a".to_owned();
        frame(&ctx, &mut text, vec![], 0.0);
        select(&ctx, 1, 1);
        frame(&ctx, &mut text, vec![], 0.005);
        frame(&ctx, &mut text, vec![egui::Event::Text("b".into())], 0.01);
        frame(
            &ctx,
            &mut text,
            key(egui::Key::Tab, egui::Modifiers::NONE),
            0.02,
        );
        assert_eq!(text, "ab\t");
        frame(
            &ctx,
            &mut text,
            key(egui::Key::Z, egui::Modifiers::COMMAND),
            0.03,
        );
        assert_eq!(text, "ab");
    }
    #[test]
    fn text_undo_ignores_navigation_and_redo_survives_selection() {
        let ctx = egui::Context::default();
        let mut text = "SELECT ".to_owned();
        frame(&ctx, &mut text, vec![], 0.0);
        select(&ctx, 7, 7);
        frame(&ctx, &mut text, vec![], 0.005);
        for (index, character) in ["n", "a", "m", "e"].into_iter().enumerate() {
            frame(
                &ctx,
                &mut text,
                {
                    let mut events = key(egui::Key::N, egui::Modifiers::NONE);
                    events.push(egui::Event::Text(character.into()));
                    events
                },
                0.01 + index as f64 * 0.01,
            );
        }
        frame(
            &ctx,
            &mut text,
            key(egui::Key::ArrowLeft, egui::Modifiers::NONE),
            0.06,
        );
        frame(&ctx, &mut text, vec![], 2.0);
        frame(
            &ctx,
            &mut text,
            key(egui::Key::Z, egui::Modifiers::COMMAND),
            2.01,
        );
        assert_eq!(text, "SELECT ");
        select(&ctx, 0, 3);
        frame(&ctx, &mut text, vec![], 4.0);
        frame(
            &ctx,
            &mut text,
            key(
                egui::Key::Z,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            ),
            4.01,
        );
        assert_eq!(text, "SELECT name");
    }

    #[test]
    fn paste_replacement_and_deletion_have_distinct_undo_steps() {
        let ctx = egui::Context::default();
        let mut text = "abc".to_owned();
        frame(&ctx, &mut text, vec![], 0.0);
        select(&ctx, 3, 0);
        frame(&ctx, &mut text, vec![], 0.005);
        frame(
            &ctx,
            &mut text,
            vec![egui::Event::Paste("é🙂".into())],
            0.01,
        );
        frame(&ctx, &mut text, vec![egui::Event::Text("xy".into())], 0.02);
        frame(
            &ctx,
            &mut text,
            key(egui::Key::Backspace, egui::Modifiers::NONE),
            0.03,
        );
        frame(
            &ctx,
            &mut text,
            key(egui::Key::Backspace, egui::Modifiers::NONE),
            0.04,
        );
        assert_eq!(text, "é🙂");
        for (index, expected) in ["é🙂xy", "é🙂", "abc"].into_iter().enumerate() {
            frame(
                &ctx,
                &mut text,
                key(egui::Key::Z, egui::Modifiers::COMMAND),
                0.05 + index as f64 * 0.01,
            );
            assert_eq!(text, expected);
        }
        for (index, expected) in ["é🙂", "é🙂xy", "é🙂"].into_iter().enumerate() {
            frame(
                &ctx,
                &mut text,
                key(
                    egui::Key::Z,
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                ),
                0.1 + index as f64 * 0.01,
            );
            assert_eq!(text, expected);
        }
    }

    #[test]
    fn new_edit_after_undo_clears_redo_and_pause_splits_typing() {
        let ctx = egui::Context::default();
        let mut text = String::new();
        frame(&ctx, &mut text, vec![], 0.0);
        select(&ctx, 0, 0);
        frame(&ctx, &mut text, vec![], 0.005);
        frame(
            &ctx,
            &mut text,
            vec![egui::Event::Text("first".into())],
            0.01,
        );
        frame(
            &ctx,
            &mut text,
            vec![egui::Event::Text("second".into())],
            30.0,
        );
        frame(
            &ctx,
            &mut text,
            key(egui::Key::Z, egui::Modifiers::COMMAND),
            30.01,
        );
        assert_eq!(text, "first");
        frame(
            &ctx,
            &mut text,
            vec![egui::Event::Paste("third".into())],
            30.02,
        );
        frame(
            &ctx,
            &mut text,
            key(
                egui::Key::Z,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            ),
            30.03,
        );
        assert_eq!(text, "firstthird");
    }

    #[test]
    fn composition_is_one_undo_step_even_after_a_pause() {
        let ctx = egui::Context::default();
        let mut text = "SELECT ".to_owned();
        frame(&ctx, &mut text, vec![], 0.0);
        select(&ctx, 7, 7);
        frame(&ctx, &mut text, vec![], 0.005);
        frame(
            &ctx,
            &mut text,
            vec![egui::Event::Ime(egui::ImeEvent::Enabled)],
            0.01,
        );
        frame(
            &ctx,
            &mut text,
            vec![egui::Event::Ime(egui::ImeEvent::Preedit("に".into()))],
            0.02,
        );
        frame(&ctx, &mut text, vec![], 3.0);
        frame(
            &ctx,
            &mut text,
            vec![egui::Event::Ime(egui::ImeEvent::Preedit("日本".into()))],
            3.01,
        );
        frame(
            &ctx,
            &mut text,
            vec![egui::Event::Ime(egui::ImeEvent::Commit("日本".into()))],
            3.02,
        );
        assert_eq!(text, "SELECT 日本");
        frame(
            &ctx,
            &mut text,
            key(egui::Key::Z, egui::Modifiers::COMMAND),
            3.03,
        );
        assert_eq!(text, "SELECT ");
    }

    #[test]
    fn bounded_block_indent_is_atomic_and_single_line_selection_is_preserved() {
        for (original, primary, secondary, expected) in [
            ("SELECT name", 9, 7, "\tSELECT name".to_owned()),
            ("  x", 3, 3, "  x\t".to_owned()),
            ("", 0, 0, "\t".to_owned()),
        ] {
            let ctx = egui::Context::default();
            let mut text = original.to_owned();
            frame(&ctx, &mut text, vec![], 0.0);
            select(&ctx, primary, secondary);
            frame(&ctx, &mut text, vec![], 0.005);
            frame(
                &ctx,
                &mut text,
                key(egui::Key::Tab, egui::Modifiers::NONE),
                0.01,
            );
            assert_eq!(text, expected);
        }
        let ctx = egui::Context::default();
        let original = format!("{}\nb", "a".repeat(997)); // 999 characters, two indents cannot fit.
        let mut text = original.clone();
        frame(&ctx, &mut text, vec![], 0.0);
        let selection = select(&ctx, 999, 0);
        frame(&ctx, &mut text, vec![], 0.005);
        let after = frame(
            &ctx,
            &mut text,
            key(egui::Key::Tab, egui::Modifiers::NONE),
            0.01,
        );
        assert_eq!(text, original);
        assert_eq!(after, selection);
    }

    #[test]
    #[ignore = "manual block indentation CPU benchmark; baseline deletes the selection"]
    fn block_indentation_frame_benchmark() {
        let ctx = egui::Context::default();
        let original = "SELECT 'é🙂', name;\n".repeat(40);
        let mut text = original.clone();
        frame(&ctx, &mut text, vec![], 0.0);
        select(&ctx, text.chars().count(), 0);
        frame(&ctx, &mut text, vec![], 0.005);
        let mut samples = Vec::new();
        for index in 0..180 {
            let started = std::time::Instant::now();
            frame(
                &ctx,
                &mut text,
                key(egui::Key::Tab, egui::Modifiers::NONE),
                index as f64 * 31.0 + 1.0,
            );
            frame(
                &ctx,
                &mut text,
                key(egui::Key::Tab, egui::Modifiers::SHIFT),
                index as f64 * 31.0 + 1.01,
            );
            if index >= 20 {
                samples.push(started.elapsed().as_secs_f64() * 1000.0);
            }
            assert_eq!(text, original);
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("editor/block-indentation", &samples);
    }
}

/// Lexical guard for typed delimiter pairing. Scans bytes without allocating tokens.
/// Only invoked for a delimiter edit, never on idle layout or ordinary typing.
pub fn parentheses_allowed(sql: &str, dialect: crate::sql::Dialect) -> bool {
    use crate::sql::Dialect;
    let b = sql.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'-'
            && b.get(i + 1) == Some(&b'-')
            && (!matches!(dialect, Dialect::Mysql(_))
                || b.get(i + 2).is_none_or(u8::is_ascii_whitespace))
            || b[i] == b'#' && matches!(dialect, Dialect::Mysql(_))
        {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            if i == b.len() {
                return false;
            }
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            let mut depth = 1;
            while i < b.len() && depth > 0 {
                if b[i] == b'/'
                    && b.get(i + 1) == Some(&b'*')
                    && matches!(dialect, Dialect::Postgres)
                {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if depth > 0 {
                return false;
            }
        } else if matches!(b[i], b'\'' | b'"' | b'`')
            || b[i] == b'[' && matches!(dialect, Dialect::Sqlite)
        {
            let quote = b[i];
            let end = if quote == b'[' { b']' } else { quote };
            let escapes = match dialect {
                Dialect::Mysql(mode) => {
                    !mode.no_backslash_escapes
                        && quote != b'`'
                        && !(quote == b'"' && mode.ansi_quotes)
                }
                Dialect::Postgres => {
                    quote == b'\''
                        && i > 0
                        && matches!(b[i - 1], b'e' | b'E')
                        && (i < 2 || !(b[i - 2].is_ascii_alphanumeric() || b[i - 2] == b'_'))
                }
                _ => false,
            };
            i += 1;
            let mut closed = false;
            while i < b.len() {
                if escapes && b[i] == b'\\' {
                    i = (i + 2).min(b.len());
                } else if b[i] == end {
                    i += 1;
                    if end != b']' && b.get(i) == Some(&end) {
                        i += 1;
                    } else {
                        closed = true;
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            if !closed {
                return false;
            }
        } else if b[i] == b'$'
            && matches!(dialect, Dialect::Postgres)
            && (i == 0
                || !(b[i - 1].is_ascii_alphanumeric()
                    || matches!(b[i - 1], b'_' | b'$')
                    || b[i - 1] >= 128))
        {
            let start = i;
            i += 1;
            if b.get(i) == Some(&b'$')
                || b.get(i)
                    .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_' || *c >= 128)
            {
                while b
                    .get(i)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c >= 128)
                {
                    i += 1;
                }
                if b.get(i) == Some(&b'$') {
                    i += 1;
                    let delimiter = &sql[start..i];
                    let Some(end) = sql[i..].find(delimiter) else {
                        return false;
                    };
                    i += end + delimiter.len();
                }
            }
        } else {
            i += 1;
        }
    }
    true
}

#[cfg(test)]
#[path = "editor_assist_tests.rs"]
mod assist_tests;
