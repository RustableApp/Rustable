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
    layout_rows(ui, text, width, usize::MAX)
}

/// Compact highlighted preview with whole lines and an overflow ellipsis.
pub fn preview(ui: &egui::Ui, text: &str, width: f32) -> std::sync::Arc<egui::Galley> {
    layout_rows(ui, &text, width, 2)
}

fn layout_rows(ui: &egui::Ui, text: &dyn egui::TextBuffer, width: f32, max_rows: usize) -> std::sync::Arc<egui::Galley> {
    type Cache = egui::cache::FrameCache<LayoutJob, Highlighter>;
    let mut job = ui.ctx().memory_mut(|memory| {
        memory
            .caches
            .cache::<Cache>()
            .get((text.as_str(), ui.visuals().dark_mode))
    });
    job.wrap.max_width = width.max(1.0);
    job.wrap.max_rows = max_rows;
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
    fn preview_elides_long_sql_without_limiting_the_editor() {
        let ctx = egui::Context::default();
        let sql = "SELECT customer_id, customer_name, total_value FROM customers WHERE total_value > 100 ORDER BY customer_name;";
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let compact = preview(ui, sql, 120.0);
                assert_eq!(compact.rows.len(), 2);
                assert!(compact.elided);
                let full = layout(ui, &sql, 120.0);
                assert!(full.rows.len() > 2);
                assert!(!full.elided);
            });
        });
    }
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

