//! Bounded database analysis. Collection runs on a worker, never during rendering.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub(crate) const MAX_KEYS: usize = 100_000;
pub(crate) const MAX_NAME_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const BATCH: usize = 200;
pub(crate) const MAX_PAGES: usize = 1_000;
pub(crate) const EXPIRY_LABELS: [&str; 6] = [
    "No expiry",
    "Within 1 minute",
    "1–60 minutes",
    "1–24 hours",
    "Over 24 hours",
    "Unknown / changed",
];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Totals {
    pub keys: u64,
    pub bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Key {
    pub name: Vec<u8>,
    pub bytes: u64,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum Coverage {
    Complete,
    KeyLimit,
    NameLimit,
    PageLimit,
    TimeLimit,
    Cancelled,
    Failed(String),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Report {
    pub created_ms: u64,
    pub elapsed_ms: u64,
    pub database: u32,
    pub coverage: Coverage,
    pub visited: usize,
    pub missing: usize,
    pub pages: usize,
    pub total: Totals,
    pub types: BTreeMap<String, Totals>,
    pub expiry: [Totals; 6],
    pub namespaces: Vec<(Vec<u8>, Totals)>,
    pub largest: Vec<Key>,
}
#[derive(Default)]
pub(crate) struct Progress {
    pub cancel: AtomicBool,
    pub keys: AtomicUsize,
    pub pages: AtomicUsize,
}
pub(crate) struct Sample {
    pub bytes: Option<u64>,
    pub ttl_ms: i64,
    pub kind: String,
}
/// Each page has a bounded RESP reader; COUNT is only a server hint.
pub(crate) trait Source {
    fn page(&mut self, cursor: u64) -> Result<(u64, Vec<Vec<u8>>), String>;
    /// One pipeline/fan-out for independent metadata. Exactly one row per key.
    fn samples(&mut self, keys: &[Vec<u8>]) -> Result<Vec<Sample>, String>;
}
fn add(total: &mut Totals, bytes: u64) {
    total.keys = total.keys.saturating_add(1);
    total.bytes = total.bytes.saturating_add(bytes);
}
fn bucket(ttl: i64) -> usize {
    match ttl {
        -1 => 0,
        0..=60_000 => 1,
        60_001..=3_600_000 => 2,
        3_600_001..=86_400_000 => 3,
        86_400_001.. => 4,
        _ => 5,
    }
}
/// No values are retained. Deduplication and aggregate memory have fixed bounds.
/// SCAN is not a snapshot; even Complete only means cursor traversal finished.
pub(crate) fn collect(source: &mut impl Source, database: u32, progress: &Progress) -> Report {
    let started = web_time::Instant::now();
    let mut report = Report {
        created_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64,
        elapsed_ms: 0,
        database,
        coverage: Coverage::Complete,
        visited: 0,
        missing: 0,
        pages: 0,
        total: Totals::default(),
        types: BTreeMap::new(),
        expiry: Default::default(),
        namespaces: vec![],
        largest: vec![],
    };
    let mut seen = HashSet::new();
    let mut name_bytes = 0;
    let mut namespaces = BTreeMap::<Vec<u8>, Totals>::new();
    let mut cursor = 0;
    'scan: loop {
        if progress.cancel.load(Ordering::Acquire) {
            report.coverage = Coverage::Cancelled;
            break;
        }
        if started.elapsed().as_secs() >= 60 {
            report.coverage = Coverage::TimeLimit;
            break;
        }
        if report.pages >= MAX_PAGES {
            report.coverage = Coverage::PageLimit;
            break;
        }
        let (next, page) = match source.page(cursor) {
            Ok(page) => page,
            Err(error) => {
                report.coverage = Coverage::Failed(error.chars().take(1024).collect());
                break;
            }
        };
        report.pages += 1;
        progress.pages.store(report.pages, Ordering::Release);
        // Bound the page independently of SCAN's advisory COUNT.
        if page.len() > 10_000 || page.iter().any(|key| key.len() > 4096) {
            report.coverage =
                Coverage::Failed("Scan page exceeds the supported key count or name size.".into());
            break;
        }
        for chunk in page.chunks(BATCH) {
            if progress.cancel.load(Ordering::Acquire) {
                report.coverage = Coverage::Cancelled;
                break 'scan;
            }
            if started.elapsed().as_secs() >= 60 {
                report.coverage = Coverage::TimeLimit;
                break 'scan;
            }
            let mut keys = Vec::new();
            let mut limit = None;
            for key in chunk {
                if seen.contains(key) {
                    continue;
                }
                if seen.len() >= MAX_KEYS {
                    limit = Some(Coverage::KeyLimit);
                    break;
                }
                if name_bytes + key.len() > MAX_NAME_BYTES {
                    limit = Some(Coverage::NameLimit);
                    break;
                }
                name_bytes += key.len();
                seen.insert(key.clone());
                keys.push(key.clone());
            }
            if !keys.is_empty() {
                let samples = match source.samples(&keys) {
                    Ok(samples) if samples.len() == keys.len() => samples,
                    Ok(_) => {
                        report.coverage =
                            Coverage::Failed("Incomplete analysis metadata response.".into());
                        break 'scan;
                    }
                    Err(error) => {
                        report.coverage = Coverage::Failed(error.chars().take(1024).collect());
                        break 'scan;
                    }
                };
                for (name, sample) in keys.into_iter().zip(samples) {
                    report.visited += 1;
                    let Some(bytes) = sample.bytes else {
                        report.missing += 1;
                        continue;
                    };
                    // Metadata commands are pipelined, not an atomic snapshot. A key
                    // may disappear or change type between the independent replies.
                    let kind = sample.kind.chars().take(64).collect::<String>();
                    let namespace = name
                        .split(|b| *b == b':')
                        .next()
                        .unwrap_or_default()
                        .to_vec();
                    add(&mut report.total, bytes);
                    // Redis type names are server-provided; bound even malformed peers.
                    let kind = if report.types.len() >= 32 && !report.types.contains_key(&kind) {
                        "Other".into()
                    } else {
                        kind
                    };
                    add(report.types.entry(kind.clone()).or_default(), bytes);
                    add(&mut report.expiry[bucket(sample.ttl_ms)], bytes);
                    add(namespaces.entry(namespace).or_default(), bytes);
                    report.largest.push(Key { name, bytes, kind });
                    report
                        .largest
                        .sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.name.cmp(&b.name)));
                    report.largest.truncate(15);
                }
                progress.keys.store(report.visited, Ordering::Release);
            }
            if let Some(limit) = limit {
                report.coverage = limit;
                break 'scan;
            }
        }
        if next == 0 {
            break;
        }
        cursor = next;
    }
    report.namespaces = namespaces.into_iter().collect();
    report
        .namespaces
        .sort_by(|a, b| b.1.bytes.cmp(&a.1.bytes).then(a.0.cmp(&b.0)));
    report.namespaces.truncate(15);
    report.elapsed_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        pages: std::collections::VecDeque<(u64, Vec<Vec<u8>>)>,
        calls: usize,
        fail: bool,
    }
    impl Source for Fixture {
        fn page(&mut self, _: u64) -> Result<(u64, Vec<Vec<u8>>), String> {
            Ok(self.pages.pop_front().unwrap())
        }
        fn samples(&mut self, keys: &[Vec<u8>]) -> Result<Vec<Sample>, String> {
            self.calls += 1;
            assert!(keys.len() <= BATCH);
            if self.fail {
                return Err("Connection lost".into());
            }
            Ok(keys
                .iter()
                .map(|key| Sample {
                    bytes: (key != b"gone").then_some(10),
                    ttl_ms: if key == b"gone" { -2 } else { 60_000 },
                    kind: "string".into(),
                })
                .collect())
        }
    }
    #[test]
    fn deduplicates_scan_and_keeps_full_sample_totals_beyond_top_fifteen() {
        let keys: Vec<_> = (0..450).map(|n| format!("app:{n}").into_bytes()).collect();
        let mut source = Fixture {
            pages: [
                (1, keys.clone()),
                (0, [keys, vec![b"gone".to_vec()]].concat()),
            ]
            .into(),
            calls: 0,
            fail: false,
        };
        let progress = Progress::default();
        let report = collect(&mut source, 2, &progress);
        assert_eq!(report.coverage, Coverage::Complete);
        assert_eq!(report.visited, 451);
        assert_eq!(report.missing, 1);
        assert_eq!(report.total.bytes, 4500);
        assert_eq!(report.largest.len(), 15);
        assert_eq!(report.namespaces[0].1.keys, 450);
        assert_eq!(report.expiry[1].keys, 450);
        assert_eq!(source.calls, 4);
        assert_eq!(progress.keys.load(Ordering::Acquire), 451);
        let restored: Report =
            serde_json::from_slice(&serde_json::to_vec(&report).unwrap()).unwrap();
        assert_eq!(restored.coverage, report.coverage);
        assert_eq!(restored.largest, report.largest);
    }
    #[test]
    fn cancellation_and_failure_never_claim_complete() {
        let mut source = Fixture {
            pages: [(0, vec![b"key".to_vec()])].into(),
            calls: 0,
            fail: true,
        };
        let progress = Progress::default();
        progress.cancel.store(true, Ordering::Release);
        assert_eq!(
            collect(&mut source, 0, &progress).coverage,
            Coverage::Cancelled
        );
        assert_eq!(source.calls, 0);
        progress.cancel.store(false, Ordering::Release);
        assert!(matches!(
            collect(&mut source, 0, &progress).coverage,
            Coverage::Failed(_)
        ));
    }
    #[test]
    fn expiry_boundaries_and_disappeared_keys_are_distinct() {
        for (ttl, expected) in [
            (-2, 5),
            (-1, 0),
            (0, 1),
            (60_000, 1),
            (60_001, 2),
            (3_600_000, 2),
            (3_600_001, 3),
            (86_400_000, 3),
            (86_400_001, 4),
        ] {
            assert_eq!(bucket(ttl), expected);
        }
    }
    #[test]
    fn nonterminating_scan_and_large_names_stop_at_bounds() {
        struct Repeating {
            calls: usize,
            large_names: bool,
        }
        impl Source for Repeating {
            fn page(&mut self, _: u64) -> Result<(u64, Vec<Vec<u8>>), String> {
                self.calls += 1;
                let keys = if self.large_names {
                    (0..1000)
                        .map(|n| {
                            let mut key = format!("{}:{n}:", self.calls).into_bytes();
                            key.resize(4096, b'x');
                            key
                        })
                        .collect()
                } else {
                    vec![]
                };
                Ok((1, keys))
            }
            fn samples(&mut self, keys: &[Vec<u8>]) -> Result<Vec<Sample>, String> {
                Ok(keys
                    .iter()
                    .map(|_| Sample {
                        bytes: Some(1),
                        ttl_ms: -1,
                        kind: "string".into(),
                    })
                    .collect())
            }
        }
        let mut source = Repeating {
            calls: 0,
            large_names: false,
        };
        let report = collect(&mut source, 0, &Progress::default());
        assert_eq!(report.coverage, Coverage::PageLimit);
        assert_eq!(source.calls, MAX_PAGES);
        source.large_names = true;
        let report = collect(&mut source, 0, &Progress::default());
        assert_eq!(report.coverage, Coverage::NameLimit);
        assert_eq!(report.visited, MAX_NAME_BYTES / 4096);
        assert_eq!(report.total.keys, report.visited as u64);
    }
    #[test]
    fn cancellation_between_metadata_batches_keeps_collected_rows() {
        struct Cancelling<'a>(&'a Progress);
        impl Source for Cancelling<'_> {
            fn page(&mut self, _: u64) -> Result<(u64, Vec<Vec<u8>>), String> {
                Ok((0, (0..400).map(|n| n.to_string().into_bytes()).collect()))
            }
            fn samples(&mut self, keys: &[Vec<u8>]) -> Result<Vec<Sample>, String> {
                self.0.cancel.store(true, Ordering::Release);
                Ok(keys
                    .iter()
                    .map(|_| Sample {
                        bytes: Some(2),
                        ttl_ms: -1,
                        kind: "string".into(),
                    })
                    .collect())
            }
        }
        let progress = Progress::default();
        let report = collect(&mut Cancelling(&progress), 0, &progress);
        assert_eq!(report.coverage, Coverage::Cancelled);
        assert_eq!(report.visited, BATCH);
        assert_eq!(report.total.bytes, 400);
    }
}
