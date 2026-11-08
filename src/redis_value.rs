//! Bounded, preformatted inspector pages. Formatting happens on the Redis worker.
#[cfg(target_arch = "wasm32")]
use crate::redis_demo as redis;
use crate::redis_session::Snapshot;
use eframe::egui;

pub(crate) const PAGE_SIZE: u64 = 200;
pub(crate) const STRING_PAGE: u64 = 65536;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Cursor {
    Scan(u64),
    Offset(u64),
    Stream(Vec<u8>),
    Lex(Option<Vec<u8>>),
    Timestamp { start: u64, end: Option<u64> },
}
impl Cursor {
    pub fn first(kind: &str, pattern: &[u8]) -> Self {
        if matches!(kind, "hash" | "set") || (kind == "zset" && pattern != b"*") {
            Self::Scan(0)
        } else if kind == "vectorset" {
            Self::Lex(None)
        } else if kind == "TSDB-TYPE" {
            Self::Timestamp { start: 0, end: None }
        } else if kind == "stream" {
            Self::Stream(b"-".to_vec())
        } else {
            Self::Offset(0)
        }
    }
    pub fn offset(&self) -> u64 {
        if let Self::Offset(n) = self { *n } else { 0 }
    }
}
pub(crate) fn timestamp_range(from: &str, to: &str) -> Result<Cursor, String> {
    let parse = |value: &str| value.trim().parse::<u64>().ok().filter(|n| *n <= i64::MAX as u64).ok_or("Enter nonnegative Unix timestamps in milliseconds.");
    let start = if from.trim().is_empty() { 0 } else { parse(from)? };
    let end = if to.trim().is_empty() { None } else { Some(parse(to)?) };
    if end.is_some_and(|end| end < start) { return Err("The end timestamp must be at or after the start.".into()); }
    Ok(Cursor::Timestamp { start, end })
}

#[derive(Clone, Debug)]
pub(crate) struct PageRequest {
    pub key: Vec<u8>,
    pub kind: String,
    pub cursor: Cursor,
    pub pattern: Vec<u8>,
}
pub(crate) struct Inspection {
    pub snapshot: Snapshot,
    pub page: Page,
}
pub(crate) struct Page {
    pub cursor: Cursor,
    pub next: Option<Cursor>,
    pub pattern: Vec<u8>,
    pub display: Display,
}
#[derive(Default)]
pub(crate) struct Display {
    pub headers: Vec<&'static str>,
    pub rows: Vec<ValueRow>,
    pub strings: StringViews,
    pub timeseries_chart: crate::redis_timeseries_chart::Chart,
}
pub(crate) struct ValueRow {
    pub cells: Vec<String>,
    pub detail: Vec<String>,
    pub search: String,
}
#[derive(Default)]
pub(crate) struct StringViews {
    pub text: Vec<String>,
    pub json: Vec<String>,
    pub json_tokens: Vec<Vec<JsonToken>>,
    pub hex: Vec<String>,
    pub json_error: Option<String>,
    pub escaped: bool,
    pub text_source: String,
    pub quoted: String,
    pub json_source: Option<String>,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum StringMode {
    #[default]
    Text,
    Json,
    Hex,
    Edit,
}

pub(crate) fn text(value: &redis::Value) -> String {
    match value {
        redis::Value::BulkString(bytes) => match std::str::from_utf8(bytes) {
            Ok(s) if !s.chars().any(|c| c.is_control() && c != '\n' && c != '\t') => s.to_owned(),
            _ => crate::redis_commands::quote(bytes),
        },
        redis::Value::SimpleString(s) => s.clone(),
        redis::Value::Int(n) => n.to_string(),
        redis::Value::Double(n) => n.to_string(),
        redis::Value::Nil => "(nil)".into(),
        _ => "[nested value]".into(),
    }
}
// Cap individual line width so rendering an unbroken 64 KiB value stays cheap.
pub(crate) fn lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let mut piece = String::new();
        for (i, ch) in line.chars().enumerate() {
            if i > 0 && i % 256 == 0 {
                out.push(std::mem::take(&mut piece));
            }
            piece.push(ch);
        }
        out.push(piece);
    }
    out
}
struct JsonBuffer(Vec<u8>);
impl std::io::Write for JsonBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > 512 * 1024 {
            return Err(std::io::Error::other(
                "Formatted JSON exceeds the 512 KiB display limit.",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
// Validate without converting JSON number tokens to f64, then only change
// insignificant whitespace. Key order, number precision and escapes survive copy.
fn pretty_json(bytes: &[u8]) -> Result<String, String> {
    use serde::Deserialize;
    use std::io::Write;
    let mut parser = serde_json::Deserializer::from_slice(bytes);
    serde::de::IgnoredAny::deserialize(&mut parser).map_err(|e| e.to_string())?;
    parser.end().map_err(|e| e.to_string())?;
    let mut out = JsonBuffer(Vec::new());
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    let newline = |out: &mut JsonBuffer, depth: usize| -> std::io::Result<()> {
        out.write_all(b"\n")?;
        out.write_all(&[b' '; 256][..(depth * 2).min(256)])
    };
    for &byte in bytes {
        let result = if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            out.write_all(&[byte])
        } else {
            match byte {
                b'"' => {
                    quoted = true;
                    out.write_all(&[byte])
                }
                b'{' | b'[' => {
                    out.write_all(&[byte]).map_err(|e| e.to_string())?;
                    depth += 1;
                    newline(&mut out, depth)
                }
                b'}' | b']' => {
                    depth = depth.saturating_sub(1);
                    newline(&mut out, depth).map_err(|e| e.to_string())?;
                    out.write_all(&[byte])
                }
                b',' => {
                    out.write_all(&[byte]).map_err(|e| e.to_string())?;
                    newline(&mut out, depth)
                }
                b':' => out.write_all(b": "),
                b' ' | b'\n' | b'\r' | b'\t' => Ok(()),
                _ => out.write_all(&[byte]),
            }
        };
        result.map_err(|e| e.to_string())?;
    }
    String::from_utf8(out.0).map_err(|e| e.to_string())
}
impl StringViews {
    pub fn new(bytes: &[u8], offset: u64, complete: bool) -> Self {
        use std::fmt::Write;
        let plain = std::str::from_utf8(bytes)
            .ok()
            .filter(|s| !s.chars().any(|c| c.is_control() && c != '\n' && c != '\t'));
        let text = plain.map_or_else(|| crate::redis_commands::quote(bytes), str::to_owned);
        let parsed = if complete {
            pretty_json(bytes)
        } else {
            Err("JSON formatting requires the complete value (up to 64 KiB).".into())
        };
        let (json, json_error, json_source) = match parsed {
            Ok(s) => (lines(&s), None, Some(s)),
            Err(e) => (vec![], Some(e), None),
        };
        let hex = bytes
            .chunks(16)
            .enumerate()
            .map(|(i, chunk)| {
                let mut s = format!("{:08x}  ", offset + (i * 16) as u64);
                for j in 0..16 {
                    if let Some(b) = chunk.get(j) {
                        let _ = write!(s, "{b:02x} ");
                    } else {
                        s.push_str("   ");
                    }
                }
                s.push_str(" | ");
                for &b in chunk {
                    s.push(if (32..127).contains(&b) {
                        b as char
                    } else {
                        '.'
                    });
                }
                s
            })
            .collect();
        Self {
            text: lines(&text),
            json_tokens: json_source
                .as_ref()
                .map_or_else(Vec::new, |source| json_tokens(source, &json)),
            json,
            hex,
            json_error,
            escaped: plain.is_none(),
            text_source: text,
            quoted: if complete {
                crate::redis_commands::quote(bytes)
            } else {
                String::new()
            },
            json_source,
        }
    }
}
impl Display {
    pub fn new(snapshot: &Snapshot, cursor: &Cursor) -> Self {
        let mut display = Self::default();
        if let redis::Value::BulkString(bytes) = &snapshot.value {
            display.strings = StringViews::new(
                bytes,
                cursor.offset(),
                !snapshot.truncated && cursor.offset() == 0,
            );
            return display;
        }
        let redis::Value::Array(values) = &snapshot.value else {
            return display;
        };
        display.headers = match snapshot.kind.as_str() {
            "hash" => vec!["FIELD", "VALUE"],
            "zset" => vec!["MEMBER", "SCORE"],
            "list" => vec!["INDEX", "VALUE"],
            "set" => vec!["MEMBER"],
            "vectorset" => vec!["ELEMENT"],
            "stream" => vec!["ENTRY ID", "FIELDS"],
            "TSDB-TYPE" => vec!["TIMESTAMP (ms)", "VALUE"],
            _ => return display,
        };
        let raw: Vec<Vec<String>> = match snapshot.kind.as_str() {
            "hash" | "zset" => values
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| vec![text(&pair[0]), text(&pair[1])])
                .collect(),
            "list" => values
                .iter()
                .enumerate()
                .map(|(i, v)| vec![(cursor.offset() + i as u64).to_string(), text(v)])
                .collect(),
            "set" | "vectorset" => values.iter().map(|v| vec![text(v)]).collect(),
            "TSDB-TYPE" => values.iter().filter_map(|value| {
                let redis::Value::Array(pair) = value else { return None; };
                (pair.len() == 2).then(|| vec![text(&pair[0]), text(&pair[1])])
            }).collect(),
            "stream" => values
                .iter()
                .filter_map(|v| {
                    let redis::Value::Array(parts) = v else {
                        return None;
                    };
                    if parts.len() != 2 {
                        return None;
                    }
                    let redis::Value::Array(fields) = &parts[1] else {
                        return None;
                    };
                    let fields = fields
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|p| format!("{}: {}", text(&p[0]), text(&p[1])))
                        .collect::<Vec<_>>()
                        .join("\n");
                    Some(vec![text(&parts[0]), fields])
                })
                .collect(),
            _ => vec![],
        };
        if snapshot.kind == "TSDB-TYPE" { display.timeseries_chart = crate::redis_timeseries_chart::Chart::new(&raw); }
        display.rows = raw
            .into_iter()
            .map(|cells| {
                let detail = cells
                    .iter()
                    .zip(&display.headers)
                    .flat_map(|(value, heading)| {
                        let mut result = vec![heading.to_string()];
                        result.extend(lines(value));
                        result.push(String::new());
                        result
                    })
                    .collect();
                let search = cells.join("\n").to_lowercase();
                let cells = cells
                    .into_iter()
                    .map(|s| {
                        let mut short: String = s
                            .chars()
                            .take(256)
                            .map(|c| if c == '\n' { ' ' } else { c })
                            .collect();
                        if s.chars().count() > 256 {
                            short.push('…');
                        }
                        short
                    })
                    .collect();
                ValueRow {
                    cells,
                    detail,
                    search,
                }
            })
            .collect();
        display
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timeseries_range_validates_inclusive_bounds_without_precision_loss() {
        assert_eq!(timestamp_range("", "").unwrap(), Cursor::Timestamp { start: 0, end: None });
        assert_eq!(timestamp_range(" 123 ", "123").unwrap(), Cursor::Timestamp { start: 123, end: Some(123) });
        assert_eq!(timestamp_range("9007199254740993", "9223372036854775807").unwrap(), Cursor::Timestamp { start: 9007199254740993, end: Some(i64::MAX as u64) });
        for (start, end) in [("-1", ""), ("1.5", ""), ("2", "1"), ("", "9223372036854775808")] { assert!(timestamp_range(start, end).is_err()); }
    }
    #[test]
    fn json_preserves_numbers_order_escapes_and_rejects_invalid_input() {
        let input = br#"{"z":900719925474099312345678901234567890,"a":0.1234567890123456789,"s":"a\\b\"c"}"#;
        let views = StringViews::new(input, 0, true);
        let json = views.json_source.unwrap();
        assert!(json.contains("900719925474099312345678901234567890"));
        assert!(json.contains("0.1234567890123456789"));
        assert!(json.find("\"z\"").unwrap() < json.find("\"a\"").unwrap());
        assert_eq!(StringViews::new(b"{broken", 0, true).json.len(), 0);
        assert!(StringViews::new(b"{}", 65536, false).json_error.is_some());
        assert!(pretty_json(b"{} trailing").is_err());
        assert!(pretty_json(&[255]).is_err());
    }
    #[test]
    fn binary_hex_offsets_empty_and_long_lines_are_bounded() {
        let views = StringViews::new(&[0, 255, b'A', b'\n'], 65536, false);
        assert!(views.escaped);
        assert!(views.hex[0].starts_with("00010000  00 ff 41 0a"));
        assert_eq!(views.text_source, r#""\x00\xffA\n""#);
        assert_eq!(StringViews::new(b"", 0, true).text_source, "");
        assert!(
            lines(&"界".repeat(65536))
                .iter()
                .all(|line| line.chars().count() <= 256)
        );
        let input = format!(
            "{}{}{}",
            "[".repeat(100),
            "0,".repeat(3000) + "0",
            "]".repeat(100)
        );
        assert!(
            pretty_json(input.as_bytes())
                .unwrap_err()
                .contains("512 KiB")
        );
    }
    #[test]
    fn structured_rows_keep_original_indices_and_full_details() {
        let snapshot = Snapshot {
            key: b"list".to_vec(),
            kind: "list".into(),
            ttl_ms: -1,
            length: Some(500),
            truncated: false,
            value: redis::Value::Array(vec![redis::Value::BulkString(vec![b'x'; 1000])]),
        };
        let display = Display::new(&snapshot, &Cursor::Offset(200));
        assert_eq!(display.rows[0].cells[0], "200");
        assert!(display.rows[0].cells[1].ends_with('…'));
        assert!(display.rows[0].detail.iter().any(|line| line.len() == 256));
        assert_eq!(display.rows[0].search.matches('x').count(), 1000);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsonKind {
    Punctuation,
    Key,
    String,
    Number,
    Literal,
}
impl JsonKind {
    pub fn color(self, dark: bool) -> egui::Color32 {
        let rgb = match (dark, self) {
            (true, Self::Key) => [126, 194, 231],
            (true, Self::String) => [157, 205, 157],
            (true, Self::Number) => [233, 187, 126],
            (true, Self::Literal) => [193, 163, 231],
            (true, Self::Punctuation) => [165, 176, 192],
            (false, Self::Key) => [26, 91, 145],
            (false, Self::String) => [39, 112, 57],
            (false, Self::Number) => [145, 80, 16],
            (false, Self::Literal) => [120, 57, 156],
            (false, Self::Punctuation) => [79, 89, 105],
        };
        egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2])
    }
}
pub(crate) struct JsonToken {
    pub range: std::ops::Range<usize>,
    pub kind: JsonKind,
}
// Tokenize once on the worker, then intersect spans with bounded display lines.
// A string spanning several display lines retains its token kind and escapes.
fn json_tokens(source: &str, lines: &[String]) -> Vec<Vec<JsonToken>> {
    let bytes = source.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let kind = match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                    } else if bytes[i] == b'"' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                if bytes
                    .get(i..)
                    .unwrap_or_default()
                    .iter()
                    .copied()
                    .find(|b| !b.is_ascii_whitespace())
                    == Some(b':')
                {
                    JsonKind::Key
                } else {
                    JsonKind::String
                }
            }
            b'-' | b'0'..=b'9' => {
                i += 1;
                while i < bytes.len()
                    && matches!(bytes[i], b'0'..=b'9' | b'e' | b'E' | b'.' | b'+' | b'-')
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
                i += 1;
                JsonKind::Punctuation
            }
        };
        spans.push(JsonToken {
            range: start..i,
            kind,
        });
    }
    let mut offset = 0;
    let mut first = 0;
    lines
        .iter()
        .map(|line| {
            let end = offset + line.len();
            while first < spans.len() && spans[first].range.end <= offset {
                first += 1;
            }
            let mut tokens: Vec<JsonToken> = Vec::new();
            for span in spans[first..].iter().take_while(|s| s.range.start < end) {
                let range = span.range.start.max(offset) - offset..span.range.end.min(end) - offset;
                if let Some(last) = tokens.last_mut()
                    && last.kind == span.kind
                {
                    last.range.end = range.end;
                } else {
                    tokens.push(JsonToken {
                        range,
                        kind: span.kind,
                    });
                }
            }
            offset = end;
            if bytes.get(offset) == Some(&b'\n') {
                offset += 1;
            }
            tokens
        })
        .collect()
}

#[cfg(test)]
mod highlight_tests {
    use super::*;
    #[test]
    fn syntax_spans_preserve_unicode_escapes_and_wrapped_strings() {
        let source = format!(
            r#"{{"{}": "a\\b\"c", "number": -1.2e+30, "enabled": true, "empty": null}}"#,
            "é".repeat(300)
        );
        let formatted = pretty_json(source.as_bytes()).unwrap();
        let lines = lines(&formatted);
        let tokens = json_tokens(&formatted, &lines);
        for (line, spans) in lines.iter().zip(&tokens) {
            let restored: String = spans.iter().map(|s| &line[s.range.clone()]).collect();
            assert_eq!(&restored, line);
        }
        assert!(tokens.iter().flatten().any(|s| s.kind == JsonKind::Key));
        assert!(tokens.iter().flatten().any(|s| s.kind == JsonKind::String));
        assert!(tokens.iter().flatten().any(|s| s.kind == JsonKind::Number));
        assert!(tokens.iter().flatten().any(|s| s.kind == JsonKind::Literal));
    }
}
