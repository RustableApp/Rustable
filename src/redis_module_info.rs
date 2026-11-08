//! Explicit, bounded module metadata. Never fetched on the normal browsing path.
use crate::redis_diagnostics::Report;

// EVAL_RO enforces read-only execution. The single key is declared for routing.
// TS.INFO may construct all metadata inside Redis, but only capped data crosses
// the wire. Do not use DEBUG: per-chunk details are outside this view's scope.
pub(crate) const TIMESERIES_SCRIPT: &str = r#"
local raw = redis.call('TS.INFO', KEYS[1])
local info = {}
for i=1,#raw,2 do
 local name=raw[i]
 if type(name)=='table' then name=name.ok end
 if type(name)=='string' then info[name]=raw[i+1] end
end
local clipped = 0
local function text(v)
  if type(v)=='table' then v=v.ok end
  if v == nil or v == false then return '-' end
  if type(v) == 'number' and math.abs(v) >= 9007199254740992 then
    return string.format('%.0f (approx.)',v)
  end
  local s=tostring(v)
  if #s > 256 then clipped=clipped+1; return string.sub(s,1,256)..'...' end
  return s
end
local summary = {}
for _,name in ipairs({'totalSamples','retentionTime','memoryUsage','firstTimestamp','lastTimestamp','chunkCount'}) do
  summary[#summary+1]={name,text(info[name])}
end
local rows = {}
for _,name in ipairs({'chunkSize','chunkType','duplicatePolicy','sourceKey'}) do
  rows[#rows+1]={'Property',name,text(info[name])}
end
local labels=info.labels or {}
for i=1,math.min(#labels,64) do rows[#rows+1]={'Label',text(labels[i][1]),text(labels[i][2])} end
local rules=info.rules or {}
for i=1,math.min(#rules,64) do
 local r=rules[i]
 rows[#rows+1]={'Rule',text(r[1]),text(r[3])..' / '..text(r[2])..' ms / alignment '..text(r[4])}
end
return {summary,rows,math.max(0,#labels-64),math.max(0,#rules-64),clipped}
"#;

pub(crate) fn timeseries(value: redis::Value, key: &[u8]) -> Result<Report, String> {
    type Metadata = (
        Vec<(String, Vec<u8>)>,
        Vec<(Vec<u8>, Vec<u8>, Vec<u8>)>,
        u64,
        u64,
        u64,
    );
    let (summary, rows, labels_omitted, rules_omitted, clipped): Metadata =
        redis::from_redis_value(value).map_err(|e| format!("Invalid time-series metadata: {e}"))?;
    if summary.len() != 6 || rows.len() > 132 {
        return Err("Time-series metadata exceeds the expected bounds.".into());
    }
    fn text(bytes: Vec<u8>) -> Result<String, String> {
        if bytes.len() > 1024 {
            return Err("Time-series metadata field exceeds the expected limit.".into());
        }
        Ok(crate::redis_value::text(&redis::Value::BulkString(bytes)))
    }
    let names = [
        ("totalSamples", "Total samples"),
        ("retentionTime", "Retention (ms)"),
        ("memoryUsage", "Memory (bytes)"),
        ("firstTimestamp", "First timestamp (ms)"),
        ("lastTimestamp", "Last timestamp (ms)"),
        ("chunkCount", "Chunks"),
    ];
    let mut result = Report {
        module_key: Some({
            let key = crate::redis_commands::quote(key);
            let mut short: String = key.chars().take(256).collect();
            if key.chars().count() > 256 {
                short.push('…');
            }
            short
        }),
        ..Default::default()
    };
    for ((name, value), (expected, label)) in summary.into_iter().zip(names) {
        if name != expected {
            return Err("Unexpected time-series metadata field.".into());
        }
        let value = text(value)?;
        result.summary.push((
            label.into(),
            if name == "retentionTime" && value == "0" {
                "No limit".into()
            } else {
                value
            },
        ));
    }
    for (category, name, value) in rows {
        result
            .rows
            .push([text(category)?, text(name)?, text(value)?]);
    }
    result.footer = format!(
        "Metadata at the last refresh. Up to 64 labels and 64 rules; text fields limited to 256 bytes. Omitted: {labels_omitted} labels, {rules_omitted} rules; {clipped} shortened fields. Requires TS.INFO and EVAL_RO permission. Large Lua numeric values are marked approximate."
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn string(value: &str) -> redis::Value {
        redis::Value::BulkString(value.as_bytes().to_vec())
    }
    fn response(rows: Vec<redis::Value>) -> redis::Value {
        redis::Value::Array(vec![
            redis::Value::Array(
                [
                    "totalSamples",
                    "retentionTime",
                    "memoryUsage",
                    "firstTimestamp",
                    "lastTimestamp",
                    "chunkCount",
                ]
                .into_iter()
                .map(|name| redis::Value::Array(vec![string(name), string("0")]))
                .collect(),
            ),
            redis::Value::Array(rows),
            redis::Value::Int(0),
            redis::Value::Int(0),
            redis::Value::Int(0),
        ])
    }
    #[test]
    fn metadata_checks_shape_field_limits_and_empty_keys() {
        let report = timeseries(response(vec![]), b"").unwrap();
        assert!(report.module_key.is_some());
        assert!(
            report
                .summary
                .contains(&("Retention (ms)".into(), "No limit".into()))
        );
        assert!(timeseries(redis::Value::Nil, b"key").is_err());
        let row = || redis::Value::Array(vec![string("Label"), string("name"), string("value")]);
        assert!(timeseries(response((0..133).map(|_| row()).collect()), b"key").is_err());
        assert!(
            timeseries(
                response(vec![redis::Value::Array(vec![
                    string("Label"),
                    string("name"),
                    string(&"x".repeat(1025))
                ])]),
                b"key"
            )
            .is_err()
        );
    }
}
