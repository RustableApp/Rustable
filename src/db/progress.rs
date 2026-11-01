//! Approximate lock-free observations of retained decoded result payloads.
//! These are NOT socket bytes, protocol bytes, total server rows, or a completion fraction.
use web_time::Instant;
use super::Value;
use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct Snapshot {
    pub active: bool,
    pub rows: u64,
    pub decoded_bytes: u64,
    pub elapsed: Duration,
}

pub(super) struct Progress {
    origin: Instant,
    started: AtomicU64,
    finished: AtomicU64,
    active: AtomicBool,
    rows: AtomicU64,
    decoded_bytes: AtomicU64,
}
impl Default for Progress {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            started: AtomicU64::new(0),
            finished: AtomicU64::new(0),
            active: AtomicBool::new(false),
            rows: AtomicU64::new(0),
            decoded_bytes: AtomicU64::new(0),
        }
    }
}
thread_local! {
    static CURRENT: RefCell<Option<Arc<Progress>>> = const { RefCell::new(None) };
}
impl Progress {
    fn now(&self) -> u64 {
        self.origin.elapsed().as_micros().min(u64::MAX as u128) as u64
    }
    pub(super) fn begin(self: &Arc<Self>) -> Operation {
        self.active.store(false, Ordering::Release);
        self.rows.store(0, Ordering::Relaxed);
        self.decoded_bytes.store(0, Ordering::Relaxed);
        self.started.store(self.now(), Ordering::Relaxed);
        self.finished.store(0, Ordering::Relaxed);
        self.active.store(true, Ordering::Release);
        let previous = CURRENT.with(|current| current.replace(Some(self.clone())));
        Operation {
            progress: self.clone(),
            previous,
        }
    }
    pub(super) fn snapshot(&self) -> Snapshot {
        let active = self.active.load(Ordering::Acquire);
        let end = if active {
            self.now()
        } else {
            self.finished.load(Ordering::Relaxed)
        };
        Snapshot {
            active,
            rows: self.rows.load(Ordering::Relaxed),
            decoded_bytes: self.decoded_bytes.load(Ordering::Relaxed),
            elapsed: Duration::from_micros(
                end.saturating_sub(self.started.load(Ordering::Relaxed)),
            ),
        }
    }
}
pub(super) struct Operation {
    progress: Arc<Progress>,
    previous: Option<Arc<Progress>>,
}
impl Drop for Operation {
    fn drop(&mut self) {
        self.progress
            .finished
            .store(self.progress.now(), Ordering::Relaxed);
        self.progress.active.store(false, Ordering::Release);
        CURRENT.with(|current| {
            current.replace(self.previous.take());
        });
    }
}
/// Called only after a row was successfully decoded and admitted to the result budget.
pub(super) fn decoded_row(values: &[Value]) {
    CURRENT.with(|current| {
        if let Some(progress) = current.borrow().as_ref() {
            let bytes = values
                .iter()
                .map(|value| match value {
                    Value::Null => 0,
                    Value::Integer(_) | Value::Real(_) => 8,
                    Value::Text(text) => text.len() as u64,
                    Value::Blob(blob) => blob.len() as u64,
                })
                .sum::<u64>();
            progress.decoded_bytes.fetch_add(bytes, Ordering::Relaxed);
            progress.rows.fetch_add(1, Ordering::Relaxed);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn telemetry_counts_payloads_resets_and_finishes_without_retaining_values() {
        let progress = Arc::new(Progress::default());
        assert!(!progress.snapshot().active);
        {
            let _operation = progress.begin();
            decoded_row(&[
                Value::Null,
                Value::Integer(7),
                Value::Real(1.0),
                Value::Text("日本".into()),
                Value::Blob(vec![1, 2, 3]),
            ]);
            let snapshot = progress.snapshot();
            assert!(snapshot.active);
            assert_eq!(snapshot.rows, 1);
            assert_eq!(snapshot.decoded_bytes, 25);
        }
        let complete = progress.snapshot();
        assert!(!complete.active);
        assert_eq!(complete.rows, 1);
        decoded_row(&[Value::Integer(1)]);
        assert_eq!(
            progress.snapshot().rows,
            1,
            "no operation means no observations"
        );
        let _operation = progress.begin();
        assert_eq!(progress.snapshot().rows, 0);
        assert_eq!(progress.snapshot().decoded_bytes, 0);
    }

    #[test]
    fn worker_sqlite_collection_publishes_real_rows_and_budget_truncation() {
        let progress = Arc::new(Progress::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let mut connection = None;
        super::super::handle(&mut connection, super::super::Command::Open(None), &cancel).unwrap();
        let _operation = progress.begin();
        let result = connection.as_mut().unwrap().execute(
            "SELECT 42, '日本', X'010203', NULL UNION ALL SELECT 7, 'x', X'', NULL",
            true,
            &cancel,
        );
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(progress.snapshot().rows, 2);
        assert_eq!(progress.snapshot().decoded_bytes, 26);
        assert_eq!(result.grids[0].rows.len(), 2);
        drop(_operation);
        let _operation = progress.begin();
        let result = connection.as_mut().unwrap().execute("WITH RECURSIVE x(v) AS (SELECT 1 UNION ALL SELECT v + 1 FROM x WHERE v < 6000) SELECT v FROM x", true, &cancel);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(result.grids[0].truncated);
        assert_eq!(progress.snapshot().rows, super::super::QUERY_LIMIT as u64);
        assert_eq!(
            progress.snapshot().decoded_bytes,
            (super::super::QUERY_LIMIT * 8) as u64
        );
    }
}
