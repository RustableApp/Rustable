//! Targeted count-dependent argument states, bounded by the caller's token limit.
//! These are completion hints, never execution validation or server requests.
use super::{Edge, Token};
fn edges(spec: &[(&'static str, &'static str)]) -> Vec<Edge> {
    spec.iter()
        .map(|&(label, kind)| Edge {
            from: 0,
            to: 0,
            label,
            kind,
            slot: 0,
        })
        .collect()
}
fn one(label: &'static str, kind: &'static str) -> Vec<Edge> {
    edges(&[(label, kind)])
}
pub(super) fn expected(name: &str, args: &[Token]) -> Option<Vec<Edge>> {
    let script = matches!(
        name,
        "EVAL" | "EVAL_RO" | "EVALSHA" | "EVALSHA_RO" | "FCALL" | "FCALL_RO"
    );
    let store = matches!(name, "ZUNIONSTORE" | "ZINTERSTORE" | "ZDIFFSTORE");
    let pop = matches!(name, "LMPOP" | "ZMPOP" | "BLMPOP" | "BZMPOP");
    let blocking = matches!(name, "BLMPOP" | "BZMPOP");
    let cardinality = matches!(name, "SINTERCARD" | "ZINTERCARD");
    if !(script || store || pop || cardinality || matches!(name, "ZUNION" | "ZINTER" | "ZDIFF")) {
        return None;
    }
    let count_index = usize::from(script || store || blocking);
    if args.len() < count_index {
        return Some(one(
            if store {
                "destination"
            } else if blocking {
                "timeout"
            } else if name.starts_with("FCALL") {
                "function"
            } else if name.starts_with("EVALSHA") {
                "sha1"
            } else {
                "script"
            },
            if store { "key" } else { "string" },
        ));
    }
    let Some(count) = args
        .get(count_index)
        .and_then(|t| t.value.parse::<usize>().ok())
    else {
        return Some(one("numkeys", "integer"));
    };
    if count == 0 && !script {
        return Some(one("numkeys (positive)", "integer"));
    }
    let end = count_index.saturating_add(1).saturating_add(count);
    if args.len() < end {
        return Some(one("key", "key"));
    }
    if script {
        return Some(one("arg", "string"));
    }
    let tail = &args[end..];
    if pop {
        let direction = if name.contains("ZMPOP") {
            edges(&[("MIN", "token"), ("MAX", "token")])
        } else {
            edges(&[("LEFT", "token"), ("RIGHT", "token")])
        };
        if tail.is_empty() {
            return Some(direction);
        }
        if !direction
            .iter()
            .any(|e| e.label.eq_ignore_ascii_case(&tail[0].value))
        {
            return Some(vec![]);
        }
        return Some(match tail.len() {
            1 => one("COUNT", "token"),
            2 if tail[1].value.eq_ignore_ascii_case("COUNT") => one("count", "integer"),
            _ => vec![],
        });
    }
    if cardinality {
        return Some(match tail.len() {
            0 => one("LIMIT", "token"),
            1 if tail[0].value.eq_ignore_ascii_case("LIMIT") => one("limit", "integer"),
            _ => vec![],
        });
    }
    let weighted = !matches!(name, "ZDIFF" | "ZDIFFSTORE");
    let mut used_weights = false;
    let mut used_aggregate = false;
    let mut used_scores = false;
    let mut i = 0;
    while i < tail.len() {
        let option = tail[i].value.to_ascii_uppercase();
        i += 1;
        match option.as_str() {
            "WEIGHTS" if weighted && !used_weights => {
                used_weights = true;
                if tail.len() - i < count {
                    return Some(one("weight", "double"));
                }
                i += count;
            }
            "AGGREGATE" if weighted && !used_aggregate => {
                used_aggregate = true;
                if i == tail.len() {
                    return Some(edges(&[
                        ("SUM", "token"),
                        ("MIN", "token"),
                        ("MAX", "token"),
                    ]));
                }
                if !["SUM", "MIN", "MAX"]
                    .iter()
                    .any(|v| v.eq_ignore_ascii_case(&tail[i].value))
                {
                    return Some(vec![]);
                }
                i += 1;
            }
            "WITHSCORES" if !store && !used_scores => used_scores = true,
            _ => return Some(vec![]),
        }
    }
    let mut result = vec![];
    if weighted && !used_weights {
        result.extend(one("WEIGHTS", "token"));
    }
    if weighted && !used_aggregate {
        result.extend(one("AGGREGATE", "token"));
    }
    if !store && !used_scores {
        result.extend(one("WITHSCORES", "token"));
    }
    Some(result)
}
