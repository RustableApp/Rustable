//! Cached SQL highlighting: tokenization runs only when text or appearance changes.
use eframe::egui::{self, Color32, FontId, TextFormat, text::LayoutJob};

#[derive(Default)]
struct Highlighter;
impl egui::cache::ComputerMut<(&str, bool), LayoutJob> for Highlighter {
    fn compute(&mut self, (code, dark): (&str, bool)) -> LayoutJob {
        highlight(code, dark)
    }
}

pub fn layout(
    ui: &egui::Ui,
    text: &dyn egui::TextBuffer,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    type Cache = egui::cache::FrameCache<LayoutJob, Highlighter>;
    let mut job = ui.ctx().memory_mut(|memory| {
        memory
            .caches
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
    let identifier = if dark {
        Color32::from_rgb(224, 132, 176)
    } else {
        Color32::from_rgb(162, 55, 108)
    };
    let number = if dark {
        Color32::from_rgb(185, 137, 238)
    } else {
        Color32::from_rgb(128, 67, 174)
    };
    let comment = if dark {
        Color32::from_rgb(138, 149, 170)
    } else {
        Color32::from_rgb(116, 128, 147)
    };
    let mut job = LayoutJob::default();
    if code.is_empty() {
        // Empty LayoutJobs have a zero-height galley row. Retain the editor
        // font metrics so the first caret and line number share a full baseline.
        job.append(
            "",
            0.0,
            TextFormat {
                font_id: FontId::monospace(13.0),
                color: plain,
                ..Default::default()
            },
        );
    }
    let mut index = 0;
    let bytes = code.as_bytes();
    while index < bytes.len() {
        let start = index;
        let mut color = plain;
        if code[index..].starts_with("--") {
            index = code[index..]
                .find('\n')
                .map(|n| index + n)
                .unwrap_or(code.len());
            color = comment;
        } else if code[index..].starts_with("/*") {
            index = code[index + 2..]
                .find("*/")
                .map(|n| index + n + 4)
                .unwrap_or(code.len());
            color = comment;
        } else if matches!(bytes[index], b'\'' | b'"' | b'`' | b'[') {
            let open = bytes[index];
            let close = if open == b'[' { b']' } else { open };
            index += 1;
            while index < bytes.len() {
                if bytes[index] == close {
                    index += 1;
                    if index < bytes.len() && bytes[index] == close && open != b'[' {
                        index += 1;
                    } else {
                        break;
                    }
                } else {
                    index += 1;
                }
            }
            color = if open == b'\'' { string } else { identifier };
        } else if bytes[index].is_ascii_digit() {
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'.')
            {
                index += 1;
            }
            color = number;
        } else if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            if KEYWORDS.contains(&code[start..index].to_ascii_uppercase().as_str()) {
                color = keyword;
            }
        } else {
            index += code[index..].chars().next().unwrap().len_utf8();
        }
        job.append(
            &code[start..index],
            0.0,
            TextFormat {
                font_id: FontId::monospace(13.0),
                color,
                ..Default::default()
            },
        );
    }
    job
}

const KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "AND",
    "OR",
    "NOT",
    "NULL",
    "AS",
    "JOIN",
    "LEFT",
    "RIGHT",
    "INNER",
    "OUTER",
    "CROSS",
    "ON",
    "USING",
    "GROUP",
    "BY",
    "ORDER",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "ASC",
    "DESC",
    "DISTINCT",
    "UNION",
    "ALL",
    "WITH",
    "RECURSIVE",
    "INSERT",
    "INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE",
    "CREATE",
    "ALTER",
    "DROP",
    "TABLE",
    "VIEW",
    "INDEX",
    "TRIGGER",
    "BEGIN",
    "END",
    "COMMIT",
    "ROLLBACK",
    "SAVEPOINT",
    "RELEASE",
    "PRIMARY",
    "KEY",
    "FOREIGN",
    "REFERENCES",
    "UNIQUE",
    "DEFAULT",
    "CHECK",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "IN",
    "IS",
    "LIKE",
    "BETWEEN",
    "EXISTS",
    "RETURNING",
    "PRAGMA",
    "EXPLAIN",
    "OVER",
    "PARTITION",
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "ROUND",
    "COALESCE",
    "CAST",
    "INTEGER",
    "TEXT",
    "REAL",
    "BLOB",
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn highlighting_preserves_unicode_and_incomplete_input() {
        for sql in [
            "SELECT '日本;é', 42; -- SELECT\n",
            "SELECT 'unfinished",
            "/* unfinished 日本",
            "SELECT [odd name], \"a\"\"b\", `field`;",
            "SELECT 'it''s fine';",
        ] {
            let job = highlight(sql, false);
            assert_eq!(job.text, sql);
            assert_eq!(job.sections.last().unwrap().byte_range.end, sql.len());
        }
    }
    #[test]
    fn keywords_identifiers_and_numbers_have_distinct_reference_colors() {
        let sql = "SELECT \"name\", 42";
        let job = highlight(sql, true);
        let color = |token: &str| {
            job.sections
                .iter()
                .find(|section| &sql[section.byte_range.clone()] == token)
                .unwrap()
                .format
                .color
        };
        assert_eq!(color("SELECT"), Color32::from_rgb(77, 166, 255));
        assert_eq!(color("\"name\""), Color32::from_rgb(224, 132, 176));
        assert_eq!(color("42"), Color32::from_rgb(185, 137, 238));
    }

    #[test]
    fn strings_and_comments_are_not_keywords() {
        let job = highlight("SELECT 'SELECT' -- SELECT", false);
        assert_ne!(job.sections[0].format.color, job.sections[2].format.color);
        assert_ne!(
            job.sections[0].format.color,
            job.sections.last().unwrap().format.color
        );
    }
}

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
            // Full-line shading for standalone statements; exact text spans when
            // two statements share a line, so the other query is never implied.
            let x1 = if row.glyphs[..local_start]
                .iter()
                .all(|g| g.chr.is_whitespace())
            {
                clip.left()
            } else {
                output.galley_pos.x + row.pos.x + row.x_offset(local_start)
            };
            let x2 = if row.glyphs[local_end..]
                .iter()
                .all(|g| g.chr.is_whitespace())
            {
                clip.right()
            } else {
                output.galley_pos.x + row.pos.x + row.x_offset(local_end)
            };
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
                FontId::monospace(13.0),
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
    fn empty_editor_first_line_has_full_font_height_and_unclipped_number() {
        let ctx = egui::Context::default();
        let mut text = String::new();
        let output = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    let (gutter, _) =
                        ui.allocate_exact_size(egui::vec2(40.0, 200.0), egui::Sense::hover());
                    let editor = egui::TextEdit::multiline(&mut text)
                        .layouter(&mut layout)
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
