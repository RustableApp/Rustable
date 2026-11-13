//! Literal search of complete result snapshots, off the render thread.
use crate::db::{Grid, Value};
use eframe::egui;
use std::sync::{Arc, Weak, mpsc, atomic::{AtomicBool, Ordering}};

const MATCH_LIMIT: usize = 100_000;
type Cell = (usize, usize);
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Found {
    #[serde(skip)]
    source: Weak<Grid>,
    needle: String,
    case_sensitive: bool,
    cells: Vec<Cell>,
    limited: bool,
}
#[derive(Default)]
pub struct Search {
    pub open: bool,
    pub focus: bool,
    pub step: isize,
    field_id: Option<egui::Id>,
    needle: String,
    case_sensitive: bool,
    found: Option<Found>,
    worker: Option<SearchWorker>,
    error: Option<String>,
    #[cfg(target_arch = "wasm32")]
    browser_worker: Option<crate::browser_jobs::Handle>,
    current: usize,
}
struct SearchWorker {
    receiver: mpsc::Receiver<Result<Option<Found>, String>>,
    cancel: Arc<AtomicBool>,
    source: Weak<Grid>,
    needle: String,
    case_sensitive: bool,
}
impl SearchWorker {
    fn matches(&self, grid: &Arc<Grid>, needle: &str, case: bool) -> bool {
        self.source.ptr_eq(&Arc::downgrade(grid))
            && self.needle == needle && self.case_sensitive == case
    }
}
impl Drop for SearchWorker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Found {
    fn matches(&self, grid: &Arc<Grid>, needle: &str, case: bool) -> bool {
        self.source.ptr_eq(&Arc::downgrade(grid))
            && self.needle == needle
            && self.case_sensitive == case
    }
}
#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) fn scan(grid: &Arc<Grid>, needle: String, case_sensitive: bool) -> Found {
    scan_until_cancelled(grid, needle, case_sensitive, || false).unwrap()
}
#[cfg(any(test, target_arch = "wasm32"))]
fn scan_until_cancelled(
    grid: &Arc<Grid>, needle: String, case_sensitive: bool,
    cancelled: impl FnMut() -> bool,
) -> Option<Found> {
    scan_checked(grid, needle, case_sensitive, cancelled)
        .ok()
        .flatten()
}
fn scan_checked(
    grid: &Arc<Grid>,
    needle: String,
    case_sensitive: bool,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Option<Found>, String> {
    if cancelled() {
        return Ok(None);
    }
    let mut result = Found {
        source: Arc::downgrade(grid),
        needle,
        case_sensitive,
        cells: Vec::new(),
        limited: false,
    };
    if result.needle.is_empty() {
        return Ok(Some(result));
    }
    let Ok(pattern) = regex::RegexBuilder::new(&regex::escape(&result.needle))
        .case_insensitive(!case_sensitive)
        .build()
    else {
        return Ok(Some(result));
    };
    let mut rendered = String::new();
    let mut stopped = false;
    grid.visit_rows(&AtomicBool::new(false), |row, values| {
        if cancelled() {
            stopped = true;
            return Ok(false);
        }
        // Check once per bounded group, keeping checkpoint bookkeeping out
        // of the numeric-cell hot loop. Large text still checks individually.
        for (chunk_index, chunk) in values.chunks(32).enumerate() {
            if chunk_index != 0 && cancelled() {
                stopped = true;
                return Ok(false);
            }
            for (offset, value) in chunk.iter().enumerate() {
                let text = if let Value::Text(text) = value {
                    if text.len() > 64 * 1024 && cancelled() {
                        stopped = true;
                        return Ok(false);
                    }
                    text.as_str()
                } else {
                    searchable_text(value, &mut rendered)
                };
                if pattern.is_match(text) {
                    if result.cells.len() == MATCH_LIMIT {
                        result.limited = true;
                        return Ok(false);
                    }
                    result.cells.push((row, chunk_index * 32 + offset));
                }
            }
        }
        Ok(true)
    })?;
    Ok(if stopped || cancelled() {
        None
    } else {
        Some(result)
    })
}
// Only scalar representations use the scratch buffer. Text is borrowed and NULL
// is static, so even a scan of huge text/blob values keeps scratch space small.
fn searchable_text<'a>(value: &'a Value, scratch: &'a mut String) -> &'a str {
    use std::fmt::Write;
    match value {
        Value::Text(text) => text,
        Value::Null => "NULL",
        _ => {
            scratch.clear();
            match value {
                Value::Integer(value) => write!(scratch, "{value}"),
                Value::Real(value) => write!(scratch, "{value}"),
                Value::Blob(bytes) => write!(scratch, "BLOB · {} bytes", bytes.len()),
                _ => unreachable!(),
            }.expect("writing into String");
            scratch
        }
    }
}

impl Search {
    /// Stop work even when this tab will no longer render (closed/replaced).
    pub fn cancel(&mut self) {
        self.worker = None;
        self.found = None;
        self.error = None;
        #[cfg(target_arch = "wasm32")]
        { self.browser_worker = None; }
    }

    fn cancel_obsolete(&mut self, grid: &Arc<Grid>) {
        if let Some(worker) = &self.worker
            && !worker.matches(grid, &self.needle, self.case_sensitive)
        {
            worker.cancel.store(true, Ordering::Relaxed);
            // Native scans acknowledge cancellation before starting another,
            // keeping at most one in-flight scan per open search.
            #[cfg(target_arch = "wasm32")]
            { self.worker = None; self.browser_worker = None; }
        }
        if self.found.as_ref().is_some_and(|found|
            !found.matches(grid, &self.needle, self.case_sensitive))
        {
            self.found = None;
            self.error = None;
        }
    }
    pub fn has_focus(&self, ctx: &egui::Context) -> bool {
        self.open
            && self
                .field_id
                .is_some_and(|id| ctx.memory(|m| m.has_focus(id)))
    }
    pub fn show(&mut self, ui: &mut egui::Ui, grid: &Arc<Grid>) -> Option<Cell> {
        if !self.open {
            self.cancel();
            return None;
        }
        let mut move_by = std::mem::take(&mut self.step);
        ui.horizontal_wrapped(|ui| {
            ui.label("Find in results");
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.needle)
                    .char_limit(1024)
                    .desired_width(180.0)
                    .hint_text("All result cells"),
            );
            self.field_id = Some(field.id);
            if field.changed() {
                self.error = None;
            }
            if self.focus {
                field.request_focus();
                self.focus = false;
            }
            if field.has_focus() || field.lost_focus() {
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter)) {
                    move_by = -1;
                }
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                    move_by = 1;
                }
                if move_by != 0 {
                    // Single-line TextEdit surrenders focus on Enter by default.
                    // Keep repeated next/previous navigation owned by this field.
                    field.request_focus();
                }
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                    self.open = false;
                }
            }
            if ui
                .checkbox(&mut self.case_sensitive, "Match case")
                .changed()
            {
                self.error = None;
            }
            let ready = self
                .found
                .as_ref()
                .filter(|f| f.matches(grid, &self.needle, self.case_sensitive));
            let enabled = ready.is_some_and(|f| !f.cells.is_empty());
            if ui
                .add_enabled(enabled, crate::icons::widget("chevron-left"))
                .on_hover_text("Previous match · Shift+Enter")
                .clicked()
            {
                move_by = -1;
            }
            if ui
                .add_enabled(enabled, crate::icons::widget("chevron-right"))
                .on_hover_text("Next match · Enter")
                .clicked()
            {
                move_by = 1;
            }
            ui.label(if self.needle.is_empty() {
                "Search result values".into()
            } else if let Some(found) = ready {
                if found.cells.is_empty() {
                    "No matches".into()
                } else {
                    format!(
                        "{} of {}{} cells",
                        self.current + 1,
                        found.cells.len(),
                        if found.limited { "+" } else { "" }
                    )
                }
            } else if self.error.is_some() {
                "Search unavailable".into()
            } else {
                "Searching…".into()
            });
            if ui
                .add(crate::icons::widget("x"))
                .on_hover_text("Close result search · Escape")
                .clicked()
            {
                self.open = false;
            }
        });
        if !self.open {
            self.cancel();
            return None;
        }
        self.cancel_obsolete(grid);
        #[cfg(target_arch = "wasm32")]
        if self
            .browser_worker
            .as_ref()
            .is_some_and(|worker| worker.expired())
        {
            self.worker = None;
            self.browser_worker = None;
            self.error = Some("Search timed out. Change the search to try again.".into());
        }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        let mut target = None;
        if let Some(worker) = &self.worker {
            match worker.receiver.try_recv() {
                Ok(Ok(Some(found))) => {
                    // The status label was painted before consuming this reply.
                    ui.ctx().request_repaint();
                    if found.matches(grid, &self.needle, self.case_sensitive) {
                        self.current = 0;
                        target = found.cells.first().copied();
                        self.found = Some(found);
                    }
                    self.worker = None;
                }
                Ok(Ok(None)) => {
                    self.worker = None;
                }
                Ok(Err(error)) => {
                    self.worker = None;
                    self.error = Some(error);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.error =
                        Some("Search worker stopped. Change the search to try again.".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        #[cfg(target_arch = "wasm32")]
        if self.worker.is_none() {
            self.browser_worker = None;
        }
        if self
            .found
            .as_ref()
            .is_some_and(|f| f.matches(grid, &self.needle, self.case_sensitive))
        {
            let found = self.found.as_ref().unwrap();
            if move_by != 0 && !found.cells.is_empty() {
                self.current = (self.current as isize + move_by)
                    .rem_euclid(found.cells.len() as isize) as usize;
                target = found.cells.get(self.current).copied();
            }
        } else if self.worker.is_none() && self.open && self.error.is_none() {
            if self.needle.is_empty() {
                // Opening/clearing search needs no background thread or grid lease.
                self.found = Some(Found {
                    source: Arc::downgrade(grid), needle: String::new(),
                    case_sensitive: self.case_sensitive, cells: Vec::new(), limited: false,
                });
                return None;
            }
            // One in-flight scan per tab; changed input cancels the old scan.
            // Retain only the existing Arc, and cap matching coordinates.
            let (tx, rx) = mpsc::sync_channel(1);
            let source = grid.clone();
            let needle = self.needle.clone();
            let case = self.case_sensitive;
            let ctx = ui.ctx().clone();
            let cancel = Arc::new(AtomicBool::new(false));
            self.worker = Some(SearchWorker {
                receiver: rx,
                cancel: cancel.clone(),
                source: Arc::downgrade(&source),
                needle: needle.clone(),
                case_sensitive: case,
            });
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::spawn(move || {
                let result = scan_checked(&source, needle, case, || cancel.load(Ordering::Relaxed));
                drop(source);
                // A closed search no longer needs a wakeup. Obsolete searches
                // still wake their owner to launch the latest coalesced request.
                if tx.send(result).is_ok() {
                    ctx.request_repaint();
                }
            });
            #[cfg(target_arch = "wasm32")]
            {
                use crate::browser_jobs::{self, Request, Response};
                let identity = Arc::downgrade(&source);
                self.browser_worker = match browser_jobs::start(
                    Request::Search {
                        grid: source,
                        needle,
                        case_sensitive: case,
                    },
                    ctx,
                    move |reply| {
                        let result = reply.and_then(|reply| match reply {
                            Response::Found(mut found) => {
                                found.source = identity.clone();
                                Ok(Some(found))
                            }
                            _ => Err("Unexpected search response".into()),
                        });
                        let _ = tx.try_send(result);
                    },
                ) {
                    Ok(handle) => Some(handle),
                    Err(error) => {
                        self.worker = None;
                        self.error = Some(error);
                        None
                    }
                };
            }
        }
        if target.is_some() {
            ui.ctx().request_repaint();
        }
        target.filter(|_| self.open)
    }
}
#[cfg(test)]
mod tests {
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn search_finds_rows_beyond_first_disk_page() {
        let mut collector = crate::db::result_store::Collector::new(128);
        for i in 0..10001 {
            collector
                .push(vec![crate::db::Value::Text(format!("row-{i}"))])
                .unwrap();
        }
        let mut grid = crate::db::Grid::default();
        collector.finish(&mut grid).unwrap();
        let grid = std::sync::Arc::new(grid);
        let found = super::scan_checked(&grid, "row-10000".into(), true, || false)
            .unwrap()
            .unwrap();
        assert_eq!(found.cells, [(10000, 0)]);
    }

    use super::*;
    #[test]
    fn cancelled_scan_stops_before_compilation_or_remaining_cells() {
        let grid = Arc::new(Grid {
            rows: vec![vec![Value::Text("match".into()); 10_000]],
            ..Default::default()
        });
        assert!(scan_until_cancelled(&grid, "match".into(), false, || true).is_none());
        let mut checks = 0;
        let found = scan_until_cancelled(&grid, "match".into(), false, || {
            checks += 1;
            checks == 18
        });
        assert!(found.is_none());
        assert_eq!(checks, 18, "cancelled scan must not traverse remaining cells");
        let complete = scan_until_cancelled(&grid, "MATCH".into(), false, || false).unwrap();
        assert_eq!(complete.cells.len(), 10_000);
    }

    fn pending_worker(grid: &Arc<Grid>, needle: &str) -> (SearchWorker, mpsc::SyncSender<Result<Option<Found>, String>>) {
        let (tx, receiver) = mpsc::sync_channel(1);
        (SearchWorker {
            receiver, cancel: Arc::new(AtomicBool::new(false)),
            source: Arc::downgrade(grid), needle: needle.into(), case_sensitive: false,
        }, tx)
    }

    #[test]
    fn close_drop_and_obsolete_requests_signal_cancellation() {
        let grid = Arc::new(Grid::default());
        let (worker, _sender) = pending_worker(&grid, "old");
        let cancelled = worker.cancel.clone();
        let mut search = Search {
            open: true, needle: "old".into(), worker: Some(worker),
            ..Default::default()
        };
        search.cancel_obsolete(&grid);
        assert!(!cancelled.load(Ordering::Relaxed));
        search.needle = "new".into();
        search.cancel_obsolete(&grid);
        assert!(cancelled.load(Ordering::Relaxed));
        #[cfg(not(target_arch = "wasm32"))]
        assert!(search.worker.is_some(), "wait for cancellation before starting another scan");
        search.cancel();
        assert!(search.worker.is_none());
        for (other_grid, case) in [(Arc::new(Grid::default()), false), (grid.clone(), true)] {
            let (worker, _sender) = pending_worker(&grid, "old");
            let cancelled = worker.cancel.clone();
            search.worker = Some(worker);
            search.needle = "old".into();
            search.case_sensitive = case;
            search.cancel_obsolete(&other_grid);
            assert!(cancelled.load(Ordering::Relaxed));
        }
        let (worker, _sender) = pending_worker(&grid, "old");
        let cancelled = worker.cancel.clone();
        search.worker = Some(worker);
        drop(search);
        assert!(cancelled.load(Ordering::Relaxed));
    }

    #[test]
    fn cancelled_worker_releases_grid_and_does_not_publish_partial_matches() {
        let grid = Arc::new(Grid {
            rows: vec![vec![Value::Text("x".repeat(1024 * 1024))]],
            ..Default::default()
        });
        let weak = Arc::downgrade(&grid);
        let (worker, sender) = pending_worker(&grid, "x");
        let cancel = worker.cancel.clone();
        let (go, wait) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            wait.recv().unwrap();
            let result = scan_until_cancelled(&grid, "x".into(), false,
                || cancel.load(Ordering::Relaxed));
            assert!(result.is_none());
            drop(grid);
            assert!(sender.send(Ok(result)).is_err());
        });
        drop(worker);
        go.send(()).unwrap();
        handle.join().unwrap();
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn empty_search_does_not_start_a_worker() {
        let grid = Arc::new(Grid::default());
        let mut search = Search { open: true, ..Default::default() };
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| { search.show(ui, &grid); });
        });
        assert!(search.worker.is_none());
        assert!(search.found.as_ref().unwrap().cells.is_empty());
        assert_eq!(Arc::strong_count(&grid), 1);
    }

    #[test]
    fn closed_search_releases_match_coordinates() {
        let grid = Arc::new(Grid::default());
        let mut search = Search { found: Some(scan(&grid, "x".into(), false)), ..Default::default() };
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| { search.show(ui, &grid); });
        });
        assert!(search.found.is_none());
    }

    #[test]
    fn search_is_literal_unicode_aware_and_matches_display_values() {
        let grid = Arc::new(Grid {
            rows: vec![
                vec![
                    Value::Text("Äpfel a.b".into()),
                    Value::Integer(42),
                    Value::Null,
                ],
                vec![Value::Text("axb".into())],
            ],
            ..Default::default()
        });
        assert_eq!(scan(&grid, "äPFEL".into(), false).cells, [(0, 0)]);
        assert!(scan(&grid, "äPFEL".into(), true).cells.is_empty());
        assert_eq!(scan(&grid, "a.b".into(), false).cells, [(0, 0)]);
        assert_eq!(scan(&grid, "42".into(), false).cells, [(0, 1)]);
        assert_eq!(scan(&grid, "NULL".into(), false).cells, [(0, 2)]);
        assert!(scan(&grid, String::new(), false).cells.is_empty());
    }
    #[test]
    fn reused_scalar_buffer_matches_display_without_retaining_large_values() {
        let values = [
            Value::Integer(i64::MIN), Value::Integer(i64::MAX), Value::Integer(0),
            Value::Real(-0.0), Value::Real(f64::MIN), Value::Real(f64::MAX),
            Value::Real(f64::from_bits(1)), Value::Real(f64::NAN),
            Value::Real(f64::INFINITY), Value::Real(f64::NEG_INFINITY),
            Value::Blob(vec![0; 1024 * 1024]), Value::Blob(Vec::new()),
            Value::Text("日本語".repeat(100_000)), Value::Null, Value::Integer(7),
        ];
        let mut scratch = String::new();
        for value in &values {
            assert_eq!(searchable_text(value, &mut scratch), crate::db::display(value));
            assert!(scratch.capacity() <= 1024, "only bounded scalar formatting retained");
        }
        let capacity = scratch.capacity();
        let buffer = scratch.as_ptr();
        for value in [Value::Integer(123), Value::Null, Value::Real(1.25)] {
            assert_eq!(searchable_text(&value, &mut scratch), crate::db::display(&value));
            assert_eq!(scratch.capacity(), capacity);
            assert_eq!(scratch.as_ptr(), buffer, "reuse the existing formatting allocation");
        }
        let grid = Arc::new(Grid { rows: vec![values.to_vec()], ..Default::default() });
        for needle in ["NULL", "BLOB", "bytes", "-0", "NaN", "inf", "日本語", "7"] {
            let expected: Vec<_> = values.iter().enumerate().filter_map(|(column, value)|
                crate::db::display(value).contains(needle).then_some((0, column))).collect();
            assert_eq!(scan(&grid, needle.into(), true).cells, expected);
        }
    }

    #[test]
    fn match_coordinates_are_bounded() {
        let grid = Arc::new(Grid {
            rows: vec![vec![Value::Integer(1); MATCH_LIMIT + 1]],
            ..Default::default()
        });
        let found = scan(&grid, "1".into(), false);
        assert_eq!(found.cells.len(), MATCH_LIMIT);
        assert!(found.limited);
    }

    #[test]
    fn asynchronous_search_selects_first_cell_and_enter_wraps_matches() {
        let ctx = egui::Context::default();
        let grid = Arc::new(Grid {
            rows: vec![
                vec![Value::Text("match".into())],
                vec![Value::Text("match".into())],
            ],
            ..Default::default()
        });
        let mut search = Search {
            open: true,
            focus: true,
            needle: "match".into(),
            ..Default::default()
        };
        let mut frame = |events: Vec<egui::Event>| {
            let mut target = None;
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        target = search.show(ui, &grid);
                    });
                },
            );
            target
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(cell) = frame(vec![]) {
                assert_eq!(cell, (0, 0));
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let enter = |modifiers| egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        assert_eq!(frame(vec![enter(egui::Modifiers::NONE)]), Some((1, 0)));
        frame(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(frame(vec![enter(egui::Modifiers::NONE)]), Some((0, 0)));
        frame(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(frame(vec![enter(egui::Modifiers::SHIFT)]), Some((1, 0)));
    }

    #[test]
    fn result_identity_and_search_options_reject_stale_matches() {
        let grid = Arc::new(Grid::default());
        let found = scan(&grid, "abc".into(), false);
        assert!(found.matches(&grid, "abc", false));
        assert!(!found.matches(&Arc::new(Grid::default()), "abc", false));
        assert!(!found.matches(&grid, "xyz", false));
        assert!(!found.matches(&grid, "abc", true));
    }
}

#[cfg(test)]
#[path = "result_search_performance.rs"]
mod performance;
