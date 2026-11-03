//! Shared highlighting plus native editor layout and execution cues.
use eframe::egui::{self, Color32, FontId};
#[path = "sql_highlight.rs"]
mod sql_highlight;
pub use sql_highlight::{editor_layout, layout, preview};

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
            painter.text(
                egui::pos2(gutter.right() - 8.0, top + row.size.y / 2.0),
                egui::Align2::RIGHT_CENTER,
                line.to_string(),
                FontId::monospace(14.0),
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
