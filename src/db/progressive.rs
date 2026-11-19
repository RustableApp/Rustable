//! Coalesced worker-to-UI result snapshots. Never queues rows on the UI thread.
use super::Grid;
use std::{
    cell::RefCell,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use web_time::Instant;

#[derive(Default)]
pub struct Mailbox {
    latest: Mutex<Option<Arc<Grid>>>,
}
impl Mailbox {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn take(&self) -> Option<Arc<Grid>> {
        self.latest.lock().unwrap().take()
    }
}
#[derive(Clone)]
struct Target {
    mailbox: Arc<Mailbox>,
    repaint: Arc<dyn Fn() + Send + Sync>,
}
thread_local! { static CURRENT: RefCell<Option<Target>> = const { RefCell::new(None) }; }
#[cfg(not(target_arch = "wasm32"))]
pub struct Scope(Option<Target>);
#[cfg(not(target_arch = "wasm32"))]
impl Scope {
    pub fn enter(mailbox: Arc<Mailbox>, repaint: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self(CURRENT.with(|slot| slot.replace(Some(Target { mailbox, repaint }))))
    }
}
#[cfg(not(target_arch = "wasm32"))]
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|slot| slot.replace(self.0.take()));
    }
}

pub struct Stream {
    target: Option<Target>,
    id: u64,
    last: Instant,
    first: bool,
}
impl Default for Stream {
    fn default() -> Self {
        Self {
            target: None,
            id: 0,
            last: Instant::now(),
            first: true,
        }
    }
}
impl Stream {
    pub fn new(grid: &mut Grid) -> Self {
        let target = CURRENT.with(|slot| slot.borrow().clone());
        if target.is_none() || grid.columns.is_empty() {
            return Self::default();
        }
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let mut stream = Self {
            target,
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            ..Self::default()
        };
        grid.stream_id = stream.id;
        stream.publish(grid.clone());
        stream
    }
    pub fn due(&self, rows: usize) -> bool {
        self.target.is_some()
            && ((self.first && rows > 0) || self.last.elapsed() >= Duration::from_millis(50))
    }
    pub fn publish(&mut self, mut grid: Grid) {
        let Some(target) = &self.target else {
            return;
        };
        grid.stream_id = self.id;
        grid.fetching = true;
        if grid.row_count() > 0 {
            self.first = false;
        }
        *target.mailbox.latest.lock().unwrap() = Some(Arc::new(grid));
        self.last = Instant::now();
        (target.repaint)();
    }
    pub fn update(&mut self, grid: &Grid) {
        if self.due(grid.rows.len()) {
            self.publish(grid.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Value, result_store::Collector};

    #[test]
    fn first_row_coalescing_spill_and_snapshot_isolation() {
        let mailbox = Arc::new(Mailbox::default());
        let _scope = Scope::enter(mailbox.clone(), Arc::new(|| {}));
        let mut grid = Grid {
            columns: vec!["id".into()],
            ..Default::default()
        };
        let mut collector = Collector::new(1);
        collector.start_stream(&mut grid);
        let header = mailbox.take().unwrap();
        assert!(header.fetching);
        assert_eq!(header.row_count(), 0);
        collector.push(vec![Value::Integer(1)]).unwrap();
        let first = mailbox.take().unwrap();
        assert!(first.is_stored());
        assert_eq!(first.row_count(), 1);
        // Readers can access a committed prefix while the writer stays open.
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut values = Vec::new();
        first
            .visit_rows(&cancel, |_, row| {
                values.push(row[0].clone());
                Ok(true)
            })
            .unwrap();
        assert_eq!(values, vec![Value::Integer(1)]);
        for i in 2..1000 {
            collector.push(vec![Value::Integer(i)]).unwrap();
        }
        collector.finish(&mut grid).unwrap();
        assert!(!grid.fetching);
        assert_eq!(grid.row_count(), 999);
        assert_eq!(grid.stream_id, first.stream_id);
        // A held snapshot must never include later rows, including Copy/Search readers.
        let mut count = 0;
        first
            .visit_rows(&cancel, |_, _| {
                count += 1;
                Ok(true)
            })
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            first
                .stored
                .as_ref()
                .unwrap()
                .window(0, None, &cancel)
                .unwrap()
                .1
                .len(),
            1
        );
        assert!(
            first
                .stored
                .as_ref()
                .unwrap()
                .window(1, None, &cancel)
                .unwrap()
                .1
                .is_empty()
        );
    }

    #[test]
    fn sqlite_publishes_before_completion_and_scope_does_not_leak() {
        use crate::db::{Command, Payload};
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        let cancel = Arc::new(AtomicBool::new(false));
        let mut connection = None;
        crate::db::handle(&mut connection, Command::Open(None), &cancel).unwrap();
        let mailbox = Arc::new(Mailbox::default());
        let observed = Arc::new(AtomicUsize::new(0));
        let _scope = Scope::enter(mailbox.clone(), {
            let mailbox = mailbox.clone();
            let observed = observed.clone();
            let cancel = cancel.clone();
            Arc::new(move || {
                if let Some(grid) = mailbox.take()
                    && grid.row_count() > 0
                {
                    observed.fetch_add(1, Ordering::Relaxed);
                    // Interrupt from the first published rows: execution must still be in flight.
                    cancel.store(true, Ordering::Relaxed);
                }
            })
        });
        let Payload::Query { report, .. } = crate::db::handle(&mut connection, Command::Query {
            sql: "WITH RECURSIVE x(v) AS (SELECT 1 UNION ALL SELECT v+1 FROM x WHERE v<1000000) SELECT v FROM x".into(), safe: true,
        }, &cancel).unwrap() else { panic!() };
        assert!(observed.load(Ordering::Relaxed) > 0);
        assert!(report.error.is_some());
        drop(_scope);
        cancel.store(false, Ordering::Relaxed);
        let Payload::Query { report, .. } = crate::db::handle(
            &mut connection,
            Command::Query {
                sql: "SELECT 42".into(),
                safe: true,
            },
            &cancel,
        )
        .unwrap() else {
            panic!()
        };
        assert!(report.error.is_none());
        assert_eq!(report.grids[0].stream_id, 0);
        assert!(mailbox.take().is_none());
    }
}
