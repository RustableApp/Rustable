//! Offline, bounded Redis grammar completion. No network requests while typing.
#[cfg(target_arch = "wasm32")]
use crate::redis_demo as redis;
use std::ops::Range;
#[derive(Default)]
pub(crate) struct Context<'a> {
    /// Sorted bytewise by the key browser.
    pub keys: &'a [Vec<u8>],
    pub indexes: &'a [Vec<u8>],
    pub snapshot: Option<&'a crate::redis_session::Snapshot>,
}
#[path = "redis_completion_catalog.rs"]
mod catalog;
pub(super) struct Edge {
    pub from: u16,
    pub to: u16,
    pub label: &'static str,
    pub kind: &'static str,
    pub slot: u16,
}
pub(super) struct Command {
    pub name: &'static str,
    pub summary: &'static str,
    pub signature: &'static str,
    pub group: &'static str,
    pub since: &'static str,
    pub edges: &'static [Edge],
}
#[derive(Clone)]
pub(crate) struct Item {
    pub label: String,
    pub insert: String,
    pub detail: &'static str,
    pub signature: &'static str,
}
#[derive(Default)]
pub(crate) struct Suggestions {
    pub items: Vec<Item>,
    pub replace: Range<usize>,
    pub signature: &'static str,
    pub summary: &'static str,
    pub argument: String,
}
struct Token {
    range: Range<usize>,
    value: String,
}
/// Tokenize the current command, respecting quoted newlines and escaped bytes.
fn tokens(text: &str, caret: usize) -> (Vec<Token>, Range<usize>, String) {
    let mut words = Vec::new();
    let mut start = None;
    let mut quote = None;
    let mut escape = false;
    let mut current_start = 0;
    for (i, c) in text.char_indices() {
        if i > caret && start.is_none() {
            break;
        }
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' {
            start.get_or_insert(i);
            escape = true;
            continue;
        }
        if Some(c) == quote {
            quote = None;
            continue;
        }
        if quote.is_none() && matches!(c, '\'' | '"') {
            start.get_or_insert(i);
            quote = Some(c);
            continue;
        }
        if quote.is_none() && matches!(c, ' ' | '\t' | '\n' | '\r') {
            if let Some(s) = start.take() {
                words.push(Token {
                    range: s..i,
                    value: decode(&text[s..i]),
                });
            }
            if i >= caret {
                break;
            }
            if matches!(c, '\n' | '\r') {
                words.clear();
                current_start = i + 1;
            }
        } else {
            start.get_or_insert(i);
        }
    }
    if let Some(s) = start {
        words.push(Token {
            range: s..text.len(),
            value: decode(&text[s..]),
        });
    }
    let at = words
        .iter()
        .position(|t| t.range.start <= caret && caret <= t.range.end);
    if let Some(i) = at {
        let range = words[i].range.clone();
        let prefix = decode(&text[range.start..caret]);
        words.truncate(i);
        (words, range, prefix)
    } else {
        (
            words
                .into_iter()
                .filter(|t| t.range.end <= caret && t.range.start >= current_start)
                .collect(),
            caret..caret,
            String::new(),
        )
    }
}
fn decode(s: &str) -> String {
    String::from_utf8_lossy(&decode_bytes(s)).into_owned()
}
fn decode_bytes(s: &str) -> Vec<u8> {
    // Close incomplete quotes for the existing binary-safe parser.
    let mut closed = s.to_owned();
    if s.starts_with('"') && !s.ends_with('"') {
        closed.push('"');
    }
    if s.starts_with('\'') && !s.ends_with('\'') {
        closed.push('\'');
    }
    crate::redis_commands::parse(&closed)
        .ok()
        .and_then(|c| c.into_iter().next())
        .and_then(|a| a.into_iter().next())
        .unwrap_or_else(|| s.trim_matches(['"', '\'']).as_bytes().to_vec())
}
fn closure(command: &Command, states: &mut Vec<u16>) {
    let mut i = 0;
    while i < states.len() && states.len() < 512 {
        let state = states[i];
        for e in command
            .edges
            .iter()
            .filter(|e| e.from == state && e.label.is_empty())
        {
            if !states.contains(&e.to) {
                states.push(e.to);
            }
        }
        i += 1;
    }
}
#[cfg(test)]
pub(crate) fn suggest(text: &str, caret_chars: usize, keys: &[Vec<u8>]) -> Suggestions {
    suggest_with_context(
        text,
        caret_chars,
        &Context {
            keys,
            ..Default::default()
        },
    )
}
pub(crate) fn suggest_with_context(
    text: &str,
    caret_chars: usize,
    context: &Context<'_>,
) -> Suggestions {
    if text.len() > 65536 {
        return Suggestions::default();
    }
    let caret = text
        .char_indices()
        .nth(caret_chars)
        .map_or(text.len(), |(i, _)| i);
    let (words, replace, prefix) = tokens(text, caret);
    if words.len() > 64 {
        return Suggestions::default();
    }
    let mut out = Suggestions {
        replace,
        ..Default::default()
    };
    let prefix_bytes = decode_bytes(&text[out.replace.start..caret]);
    let upper = prefix.to_ascii_uppercase();
    // Containers and their subcommands share the same completion surface.
    let lead = words
        .iter()
        .map(|t| t.value.to_ascii_uppercase())
        .collect::<Vec<_>>();
    let command = catalog::COMMANDS
        .iter()
        .filter(|c| {
            let names = c.name.split_whitespace();
            names.clone().count() <= lead.len() && names.zip(&lead).all(|(a, b)| a == b)
        })
        .max_by_key(|c| c.name.len());
    if words.is_empty() || command.is_some_and(|c| c.edges.iter().all(|e| e.label.is_empty())) {
        let stem = if lead.is_empty() {
            String::new()
        } else {
            format!("{} ", lead.join(" "))
        };
        for c in catalog::COMMANDS {
            if let Some(tail) = c.name.strip_prefix(&stem)
                && tail.starts_with(&upper)
                && tail != upper
            {
                out.items.push(Item {
                    label: tail.into(),
                    insert: format!("{tail} "),
                    detail: c.summary,
                    signature: c.signature,
                });
                if out.items.len() == 50 {
                    break;
                }
            }
        }
        if !out.items.is_empty() {
            return out;
        }
    }
    let Some(command) = command else {
        return out;
    };
    out.signature = command.signature;
    out.summary = command.summary;
    let _ = (command.group, command.since);
    let mut states = vec![0];
    closure(command, &mut states);
    let mut used_options = Vec::new();
    for token in words.iter().skip(command.name.split_whitespace().count()) {
        let mut next = Vec::new();
        let mut matched = Vec::new();
        for e in command.edges.iter().filter(|e| {
            !e.label.is_empty()
                && states.contains(&e.from)
                && (e.slot == 0 || !used_options.contains(&e.slot))
        }) {
            if e.kind != "token" || e.label.eq_ignore_ascii_case(&token.value) {
                next.push(e.to);
                if e.slot != 0 {
                    matched.push(e.slot);
                }
            }
        }
        used_options.extend(matched);
        next.sort_unstable();
        next.dedup();
        states = next;
        closure(command, &mut states);
    }
    let expected = command
        .edges
        .iter()
        .filter(|e| {
            !e.label.is_empty()
                && states.contains(&e.from)
                && (e.slot == 0 || !used_options.contains(&e.slot))
        })
        .collect::<Vec<_>>();
    let mut labels = Vec::new();
    for e in &expected {
        if !labels.contains(&e.label) {
            labels.push(e.label);
        }
        if e.kind == "token"
            && e.label.starts_with(&upper)
            && e.label != upper
            && !out.items.iter().any(|i| i.label == e.label)
        {
            out.items.push(Item {
                label: e.label.into(),
                insert: format!("{} ", e.label),
                detail: command.summary,
                signature: command.signature,
            });
        }
    }
    out.argument = labels.into_iter().take(8).collect::<Vec<_>>().join(" · ");
    if expected.iter().any(|e| e.kind == "key") {
        // The browser keeps loaded keys sorted; binary search avoids rescanning
        // the entire catalog on each edit and reaches names beyond the first page.
        let start = context
            .keys
            .partition_point(|key| key.as_slice() < prefix_bytes.as_slice());
        for key in context.keys[start..].iter().take(50) {
            if key.starts_with(&prefix_bytes) {
                let insert = crate::redis_commands::quote(key);
                out.items.push(Item {
                    label: insert.clone(),
                    insert: format!("{insert} "),
                    detail: "Loaded key · no additional server request",
                    signature: command.signature,
                });
                if out.items.len() >= 50 {
                    break;
                }
            }
        }
    }
    let mut add = |value: &[u8], detail| {
        if out.items.len() < 50 && value.starts_with(&prefix_bytes) {
            let quoted = crate::redis_commands::quote(value);
            if !out.items.iter().any(|i| i.label == quoted) {
                out.items.push(Item {
                    label: quoted.clone(),
                    insert: format!("{quoted} "),
                    detail,
                    signature: command.signature,
                });
            }
        }
    };
    if expected.iter().any(|e| e.label == "index") {
        for index in context.indexes.iter().take(1000) {
            add(index, "Loaded search index");
        }
    }
    if let Some(snapshot) = context.snapshot {
        let kind = match snapshot.kind.as_str() {
            "hash" => "field",
            "zset" | "set" => "member",
            "vectorset" => "element",
            _ => "",
        };
        let command_key = words
            .get(command.name.split_whitespace().count())
            .and_then(|t| crate::redis_commands::parse(&text[t.range.clone()]).ok())
            .and_then(|v| v.into_iter().next())
            .and_then(|v| v.into_iter().next());
        if !kind.is_empty()
            && command_key.as_deref() == Some(snapshot.key.as_slice())
            && expected.iter().any(|e| e.label == kind)
            && let redis::Value::Array(values) = &snapshot.value
        {
            let stride = if matches!(snapshot.kind.as_str(), "hash" | "zset") {
                2
            } else {
                1
            };
            for v in values.iter().step_by(stride).take(200) {
                match v {
                    redis::Value::BulkString(b) => add(b, "Loaded inspector field / member"),
                    redis::Value::SimpleString(s) => {
                        add(s.as_bytes(), "Loaded inspector field / member")
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(s: &str) -> Suggestions {
        suggest(
            s,
            s.chars().count(),
            &[b"app:config".to_vec(), b"hello world".to_vec()],
        )
    }
    #[test]
    fn commands_subcommands_and_grammar() {
        assert!(at("HG").items.iter().any(|i| i.label == "HGETALL"));
        assert!(at("CONFIG G").items.iter().any(|i| i.label == "GET"));
        assert!(
            at("GET app")
                .items
                .iter()
                .any(|i| i.insert == "\"app:config\" ")
        );
        assert!(at("SET app value N").items.iter().any(|i| i.label == "NX"));
        assert!(at("SET app value EX ").argument.contains("seconds"));
        assert!(
            at("SET app value EX 60 N")
                .items
                .iter()
                .any(|i| i.label == "NX")
        );
        assert!(
            !at("SET app value NX X")
                .items
                .iter()
                .any(|i| i.label == "XX")
        );
        assert!(at("JSON.GET ").signature.contains("path"));
        assert!(
            at("FT.SEARCH idx * L")
                .items
                .iter()
                .any(|i| i.label == "LIMIT")
        );
        assert!(at("GET x\nZR").items.iter().any(|i| i.label == "ZRANGE"));
    }
    #[test]
    fn loaded_context_is_scoped_and_binary_safe() {
        let snapshot = crate::redis_session::Snapshot {
            key: b"users".to_vec(),
            kind: "hash".into(),
            ttl_ms: -1,
            length: Some(1),
            truncated: false,
            value: redis::Value::Array(vec![
                redis::Value::BulkString(b"display name".to_vec()),
                redis::Value::BulkString(b"Avery".to_vec()),
            ]),
        };
        let indexes = vec![b"idx:users".to_vec()];
        let context = Context {
            snapshot: Some(&snapshot),
            indexes: &indexes,
            ..Default::default()
        };
        let s = "HGET users dis";
        let r = suggest_with_context(s, s.len(), &context);
        assert!(r.items.iter().any(|i| i.insert == "\"display name\" "));
        let s = "HGET other dis";
        assert!(suggest_with_context(s, s.len(), &context).items.is_empty());
        let s = "FT.SEARCH idx";
        assert!(
            suggest_with_context(s, s.len(), &context)
                .items
                .iter()
                .any(|i| i.insert == "\"idx:users\" ")
        );
        let r = suggest("GET ", 4, &[vec![b'a', 0, 255]]);
        assert_eq!(r.items[0].insert, "\"a\\x00\\xff\" ");
    }
    #[test]
    #[ignore = "manual bounded Redis completion typing CPU benchmark"]
    fn redis_intellisense_typing_benchmark() {
        let keys: Vec<_> = (0..1000)
            .map(|i| format!("app:users:{i:04}").into_bytes())
            .collect();
        let context = Context {
            keys: &keys,
            ..Default::default()
        };
        for (case, inputs) in [
            ("commands", ["HG", "GE"]),
            ("options", ["SET app value EX 60 N", "ZRANGE app 0 -1 W"]),
            ("keys-1000", ["GET app:users:09", "GET app:users:00"]),
        ] {
            let mut samples = Vec::new();
            for i in 0..550 {
                let text = inputs[i % 2];
                let start = std::time::Instant::now();
                let result = std::hint::black_box(suggest_with_context(
                    text,
                    text.chars().count(),
                    &context,
                ));
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                assert!(!result.items.is_empty(), "{text}");
                assert!(result.items.len() <= 50);
                if i >= 50 {
                    samples.push(elapsed);
                }
            }
            samples.sort_by(f64::total_cmp);
            crate::ui_benchmark::report(&format!("redis/intellisense-typing/{case}"), &samples);
        }
    }
    #[test]
    fn key_completion_reaches_the_whole_sorted_cache_and_binary_prefixes() {
        let keys: Vec<_> = (0..10000)
            .map(|i| format!("key:{i:05}").into_bytes())
            .collect();
        let s = "GET key:09999";
        assert_eq!(suggest(s, s.len(), &keys).items[0].insert, "\"key:09999\" ");
        let s = "GET \\xff";
        assert_eq!(
            suggest(s, s.len(), &[vec![255, b'a']]).items[0].insert,
            "\"\\xffa\" "
        );
        let s = "GET café\u{a0}name";
        assert_eq!(
            suggest(
                s,
                s.chars().count(),
                &["café\u{a0}name".as_bytes().to_vec()]
            )
            .items
            .len(),
            1
        );
    }
    #[test]
    fn replacement_preserves_suffix_and_unicode() {
        let s = "GET café\nHGETALL other";
        let r = suggest(s, 3, &[]);
        assert_eq!(&s[r.replace], "GET");
        let r = at("GET \"hello w");
        assert!(r.items.iter().any(|i| i.insert == "\"hello world\" "));
        let r = at("SET x \"line\nbreak\" N");
        assert!(r.items.iter().any(|i| i.label == "NX"));
        assert!(at(&"x".repeat(65537)).items.is_empty());
    }
}
