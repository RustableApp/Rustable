//! Cmd+/ (Ctrl+/ elsewhere) line-comment toggling for the SQL editor.
//!
//! Offsets are Unicode character indices, matching egui cursors. Every supported
//! dialect accepts `-- `; MySQL/MariaDB require whitespace after `--`, so the
//! inserted marker always carries a space. MySQL's `#` comments are recognised
//! when uncommenting.
use crate::sql::Dialect;
use std::ops::Range;

const MARKER: &str = "-- ";

/// Comment every selected line, or uncomment them when all non-blank selected
/// lines are already comments. Returns the new text and the adjusted selection.
pub fn toggle(sql: &str, selection: Range<usize>, dialect: Dialect) -> (String, Range<usize>) {
    let chars: Vec<char> = sql.chars().collect();
    let start = selection.start.min(chars.len());
    let end = selection.end.clamp(start, chars.len());
    let mut lines = Vec::new();
    lines.push(0..0);
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' {
            lines.last_mut().unwrap().end = i;
            lines.push(i + 1..i + 1);
        }
    }
    lines.last_mut().unwrap().end = chars.len();
    let line_of = |offset: usize| lines.partition_point(|line| line.start <= offset) - 1;
    let first = line_of(start);
    let mut last = line_of(end);
    // A selection ending at the start of a line does not include that line.
    if end > start && last > first && lines[last].start == end {
        last -= 1;
    }
    let indent = |line: &Range<usize>| {
        chars[line.clone()].iter().take_while(|c| matches!(c, ' ' | '\t')).count()
    };
    let blank = |line: &Range<usize>| chars[line.clone()].iter().all(|c| c.is_whitespace());
    let selected = &lines[first..=last];
    let mut targets: Vec<&Range<usize>> = selected.iter().filter(|line| !blank(line)).collect();
    if targets.is_empty() {
        targets = selected.iter().collect();
    }
    let markers: Vec<Option<usize>> = targets
        .iter()
        .map(|line| existing_marker(&chars[line.start + indent(line)..line.end], dialect))
        .collect();
    // (position, removed characters, inserted text), ascending by position.
    let edits: Vec<(usize, usize, &str)> = if markers.iter().all(Option::is_some) {
        targets
            .iter()
            .zip(markers)
            .map(|(line, len)| (line.start + indent(line), len.unwrap(), ""))
            .collect()
    } else {
        // Already-commented lines are left alone rather than double-commented.
        let column = targets.iter().map(|line| indent(line)).min().unwrap_or(0);
        targets
            .iter()
            .zip(&markers)
            .filter(|(_, marker)| marker.is_none())
            .map(|(line, _)| (line.start + column.min(line.len()), 0, MARKER))
            .collect()
    };
    let mut text = String::with_capacity(sql.len() + edits.len() * MARKER.len());
    let mut copied = 0;
    for (pos, removed, inserted) in &edits {
        text.extend(&chars[copied..*pos]);
        text.push_str(inserted);
        copied = pos + removed;
    }
    text.extend(&chars[copied..]);
    // An empty caret moves with text inserted at it; a selection grows to
    // include a marker inserted at its start.
    let map = |offset: usize, follow_insert: bool| {
        let mut mapped = offset;
        for (pos, removed, inserted) in &edits {
            if *removed > 0 {
                if offset >= pos + removed {
                    mapped -= removed;
                } else if offset > *pos {
                    mapped -= offset - pos;
                }
            } else if offset > *pos || (offset == *pos && follow_insert) {
                mapped += inserted.chars().count();
            }
        }
        mapped
    };
    let caret = start == end;
    (text, map(start, caret)..map(end, true))
}

/// Length of a leading line-comment marker plus one following space, if any.
fn existing_marker(text: &[char], dialect: Dialect) -> Option<usize> {
    let space = |at: usize| usize::from(text.get(at) == Some(&' '));
    match text {
        ['#', ..] if matches!(dialect, Dialect::Mysql(_)) => Some(1 + space(1)),
        ['-', '-', rest @ ..] => {
            // MySQL treats `--x` as an operator sequence, not a comment.
            let comment = !matches!(dialect, Dialect::Mysql(_))
                || rest.first().is_none_or(|c| c.is_whitespace() || c.is_control());
            comment.then(|| 2 + space(2))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PG: Dialect = Dialect::Postgres;
    fn mysql() -> Dialect {
        Dialect::Mysql(Default::default())
    }
    fn chars(s: &str, byte: usize) -> usize {
        s[..byte].chars().count()
    }

    #[test]
    fn caret_line_round_trips() {
        let sql = "SELECT 1;\nSELECT 2;";
        let (commented, cursor) = toggle(sql, 12..12, PG);
        assert_eq!(commented, "SELECT 1;\n-- SELECT 2;");
        assert_eq!(cursor, 15..15);
        let (restored, cursor) = toggle(&commented, cursor, PG);
        assert_eq!(restored, sql);
        assert_eq!(cursor, 12..12);
    }

    #[test]
    fn selection_comments_lines_at_shared_indent_and_skips_blank_lines() {
        let sql = "SELECT a,\n    b\n\n  FROM t;\nSELECT 2;";
        let end = sql.find("SELECT 2").unwrap();
        let (commented, cursor) = toggle(sql, 0..end, PG);
        assert_eq!(commented, "-- SELECT a,\n--     b\n\n--   FROM t;\nSELECT 2;");
        // Selection ending at a line start excludes that line, and still covers
        // the same lines so a second press uncomments them.
        assert_eq!(cursor, 0..commented.find("SELECT 2").unwrap());
        assert_eq!(toggle(&commented, cursor, PG).0, sql);
    }

    #[test]
    fn indented_block_keeps_marker_at_minimum_indent() {
        let sql = "  WHERE a = 1\n    AND b = 2";
        let (commented, cursor) = toggle(sql, 3..sql.len(), PG);
        assert_eq!(commented, "  -- WHERE a = 1\n  --   AND b = 2");
        assert_eq!(toggle(&commented, cursor, PG).0, sql);
    }

    #[test]
    fn mixed_selection_comments_only_uncommented_lines() {
        let sql = "select * from something;\n\n-- select * from test;";
        let (commented, cursor) = toggle(sql, 0..sql.len(), PG);
        assert_eq!(commented, "-- select * from something;\n\n-- select * from test;");
        assert_eq!(cursor, 0..commented.len());
        // Every line is now a comment, so the next press uncomments them all.
        let (uncommented, _) = toggle(&commented, cursor, PG);
        assert_eq!(uncommented, "select * from something;\n\nselect * from test;");
        // Indented existing comments keep the block aligned.
        let (indented, _) = toggle("  -- a\n  b", 0..10, PG);
        assert_eq!(indented, "  -- a\n  -- b");
    }

    #[test]
    fn uncomments_markers_without_spaces() {
        assert_eq!(toggle("--SELECT 1;", 0..0, PG).0, "SELECT 1;");
        assert_eq!(toggle("--SELECT 1;", 0..0, Dialect::Sqlite).0, "SELECT 1;");
    }

    #[test]
    fn mysql_recognises_hash_and_requires_whitespace_after_dashes() {
        assert_eq!(toggle("# SELECT 1;", 0..0, mysql()).0, "SELECT 1;");
        assert_eq!(toggle("-- SELECT 1;", 0..0, mysql()).0, "SELECT 1;");
        assert_eq!(toggle("--", 0..0, mysql()).0, "");
        // `--1` is arithmetic in MySQL, so the line gets commented instead.
        assert_eq!(toggle("--1", 0..0, mysql()).0, "-- --1");
        // `#` is not a comment in PostgreSQL.
        assert_eq!(toggle("# x", 0..0, PG).0, "-- # x");
        // The inserted marker is valid MySQL comment syntax.
        let (commented, _) = toggle("SELECT 1;", 0..0, mysql());
        assert!(crate::sql::mysql::ranges(&commented, Default::default(), true).unwrap().is_empty());
    }

    #[test]
    fn blank_caret_line_and_unicode_offsets() {
        assert_eq!(toggle("", 0..0, PG), ("-- ".into(), 3..3));
        let sql = "SELECT '😀';\nSELECT 'é';";
        let start = chars(sql, sql.find("'é'").unwrap());
        let (commented, cursor) = toggle(sql, start..start + 3, PG);
        assert_eq!(commented, "SELECT '😀';\n-- SELECT 'é';");
        assert_eq!(commented.chars().skip(cursor.start).take(cursor.len()).collect::<String>(), "'é'");
        assert_eq!(toggle(&commented, cursor, PG).0, sql);
    }

    #[test]
    fn crlf_lines_round_trip() {
        let sql = "SELECT 1;\r\nSELECT 2;\r\n";
        let (commented, cursor) = toggle(sql, 0..chars(sql, sql.len()), PG);
        assert_eq!(commented, "-- SELECT 1;\r\n-- SELECT 2;\r\n");
        assert_eq!(toggle(&commented, cursor, PG).0, sql);
    }
}
