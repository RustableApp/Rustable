//! Bounded viewport cache for continuously scrolling disk-backed results.
use crate::db::Grid;
use eframe::egui;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

#[derive(Default)]
pub struct Pages {
    source: Weak<Grid>,
    page: Option<Arc<Grid>>,
    pub offset: usize,
    worker: Option<Worker>,
    queued: Option<(Arc<Grid>, usize, Option<usize>)>,
    error: Option<String>,
    focus: Option<(usize, usize)>,
    requested: Option<(usize, Option<usize>)>,
    defer_viewport: bool,
    pub rendering: bool,
    selection: Option<Arc<Grid>>,
    previews: Vec<bool>,
    selection_offset: usize,
    selection_rows: std::collections::BTreeSet<usize>,
    pub all_selected: bool,
    copy: Option<CopyWorker>,
    copy_error: Option<String>,
    copied_until: Option<web_time::Instant>,
}
struct CopyWorker {
    receive: mpsc::Receiver<Result<String, String>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for CopyWorker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn clipboard_result(source: &Grid, cancel: &AtomicBool) -> Result<String, String> {
    use std::fmt::Write;
    const LIMIT: usize = 32 * 1024 * 1024;
    let mut output = String::new();
    source.visit_rows(cancel, |index, row| {
        if index > 0 {
            output.push('\n');
        }
        for (column, value) in row.iter().enumerate() {
            if column > 0 {
                output.push('\t');
            }
            let borrowed = match value {
                crate::db::Value::Text(text) => Some(text.as_str()),
                crate::db::Value::Blob(bytes) => crate::db::readable_binary(bytes),
                _ => None,
            };
            if let Some(text) = borrowed {
                if text.len() > LIMIT.saturating_sub(output.len()) {
                    return Err(
                        "Selection exceeds the 32 MiB clipboard limit. Export the data instead."
                            .into(),
                    );
                }
                output.push_str(text);
            } else if let crate::db::Value::Blob(bytes) = value {
                if bytes.len().saturating_mul(2) > LIMIT.saturating_sub(output.len()) {
                    return Err(
                        "Selection exceeds the 32 MiB clipboard limit. Export the data instead."
                            .into(),
                    );
                }
                for chunk in bytes.chunks(8192) {
                    if cancel.load(Ordering::Relaxed) {
                        return Err("Cancelled".into());
                    }
                    for byte in chunk {
                        write!(output, "{byte:02x}").unwrap();
                    }
                }
            } else {
                output.push_str(&crate::db::clipboard_text(value));
            }
            if output.len() > LIMIT {
                return Err(
                    "Selection exceeds the 32 MiB clipboard limit. Export the data instead.".into(),
                );
            }
        }
        Ok(true)
    })?;
    Ok(output)
}

type LoadedWindow = Result<(usize, Arc<Grid>, Vec<bool>), String>;
struct Worker {
    receive: mpsc::Receiver<LoadedWindow>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Pages {
    pub fn copy_all(&mut self, source: &Arc<Grid>, ctx: &egui::Context) {
        if self.copy.is_some() {
            return;
        }
        self.copy_error = None;
        let (send, receive) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.copy = Some(CopyWorker {
            receive,
            cancel: cancel.clone(),
        });
        let source = source.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = clipboard_result(&source, &cancel);
            if send.send(result).is_ok() {
                ctx.request_repaint();
            }
        });
    }
    pub fn is_loading(&self) -> bool {
        self.worker.is_some()
    }
    pub fn grid(&self, source: &Arc<Grid>) -> Arc<Grid> {
        if self.source.ptr_eq(&Arc::downgrade(source)) {
            if !self.rendering
                && let Some(selection) = &self.selection
            {
                return selection.clone();
            }
            self.page.as_ref().unwrap_or(source).clone()
        } else {
            source.clone()
        }
    }
    pub fn is_preview(&self, index: usize) -> bool {
        self.previews.get(index).copied().unwrap_or(false)
    }
    pub fn pin_selection(
        &mut self,
        selected: Option<usize>,
        rows: &std::collections::BTreeSet<usize>,
    ) {
        if selected.is_none() && rows.is_empty() {
            self.selection = None;
            self.selection_rows.clear();
            return;
        }
        if selected.is_some()
            || self
                .page
                .as_ref()
                .is_some_and(|page| rows.len() != page.rows.len())
        {
            self.all_selected = false;
        }
        self.selection = self.page.clone();
        self.selection_offset = self.offset;
        if &self.selection_rows != rows {
            self.selection_rows.clone_from(rows);
        }
        if let Some(index) = selected {
            self.selection_rows.insert(index);
        }
    }
    pub fn selected_offset(&self) -> usize {
        if self.selection.is_some() {
            self.selection_offset
        } else {
            self.offset
        }
    }
    pub fn selected_global(&self, index: usize) -> bool {
        self.selection_outside_viewport()
            && index
                .checked_sub(self.selection_offset)
                .is_some_and(|local| self.selection_rows.contains(&local))
    }
    pub fn retained_selection(&self) -> impl Iterator<Item = usize> + '_ {
        let outside = self.selection_outside_viewport();
        self.selection_rows
            .iter()
            .copied()
            .filter(move |_| outside)
            .map(|index| index + self.selection_offset)
    }
    pub fn selection_outside_viewport(&self) -> bool {
        self.selection.as_ref().is_some_and(|selection| {
            self.page
                .as_ref()
                .is_none_or(|page| !Arc::ptr_eq(page, selection))
        })
    }
    pub fn viewport(
        &mut self,
        source: &Arc<Grid>,
        visible: std::ops::Range<usize>,
        ctx: &egui::Context,
    ) {
        if egui::Popup::is_any_open(ctx) {
            return;
        }
        if self.defer_viewport {
            self.defer_viewport = false;
            return;
        }
        if self.focus.is_some() || visible.is_empty() || !source.is_stored() || self.error.is_some()
        {
            return;
        }
        let end = self.offset + self.page.as_ref().map_or(0, |page| page.rows.len());
        // Refill before either edge reaches the viewport, keeping the current
        // window visible while the next one is read. The same 256-row bound
        // now includes look-ahead and look-behind, rather than only later rows.
        let missing = visible.start < self.offset || visible.end > end;
        let near_edge = (self.offset > 0 && visible.start < self.offset + 64)
            || (end < source.row_count() && visible.end.saturating_add(64) > end);
        if !missing && !near_edge {
            return;
        }
        // Don't restart a useful read for every wheel event. Only a jump beyond
        // its coverage supersedes it; request() coalesces those jumps.
        if let Some((start, _)) = self.requested
            && visible.start >= start
            && visible.end <= start.saturating_add(256)
        {
            return;
        }
        let start = (visible.start.saturating_sub(96) / 32 * 32)
            .min(source.row_count().saturating_sub(256));
        if start != self.offset || missing {
            self.request(source, start, None, ctx);
        }
    }

    fn request(
        &mut self,
        source: &Arc<Grid>,
        position: usize,
        full_row: Option<usize>,
        ctx: &egui::Context,
    ) {
        if self.requested == Some((position, full_row)) {
            return;
        }
        self.requested = Some((position, full_row));
        if let Some(worker) = &self.worker {
            worker.cancel.store(true, Ordering::Relaxed);
            self.queued = Some((source.clone(), position, full_row));
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(store) = source.stored.clone() {
            let (send, receive) = mpsc::channel();
            let cancel = Arc::new(AtomicBool::new(false));
            self.worker = Some(Worker {
                receive,
                cancel: cancel.clone(),
            });
            self.error = None;
            let source = source.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result =
                    store
                        .window(position, full_row, &cancel)
                        .map(|(start, rows, previews)| {
                            let mut page = (*source).clone();
                            page.rows = rows;
                            (start, Arc::new(page), previews)
                        });
                if send.send(result).is_ok() {
                    ctx.request_repaint();
                }
            });
        }
        #[cfg(target_arch = "wasm32")]
        let _ = (source, position, full_row, ctx);
    }
    pub fn select(
        &mut self,
        source: &Arc<Grid>,
        row: usize,
        column: usize,
        ctx: &egui::Context,
    ) -> Option<(usize, usize)> {
        if !source.is_stored() {
            return Some((row, column));
        }
        if self.worker.is_none()
            && let Some(page) = &self.page
            && row >= self.offset
            && row < self.offset + page.rows.len()
            && !self.is_preview(row - self.offset)
        {
            return Some((row - self.offset, column));
        }
        self.focus = Some((row, column));
        self.request(source, row, Some(row), ctx);
        None
    }
    /// Returns a selection after a search-triggered page arrives, and whether selection must reset.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        source: &Arc<Grid>,
    ) -> (Option<(usize, usize)>, bool) {
        let mut changed = false;
        let mut selected = None;
        if !self.source.ptr_eq(&Arc::downgrade(source)) {
            *self = Self {
                source: Arc::downgrade(source),
                ..Default::default()
            };
            if source.is_stored() {
                self.request(source, 0, None, ui.ctx());
            }
            changed = true;
        }
        let copied = self
            .copy
            .as_ref()
            .and_then(|copy| match copy.receive.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Clipboard worker stopped".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            });
        if let Some(result) = copied {
            self.copy = None;
            match result {
                Ok(text) => {
                    ui.ctx().copy_text(text);
                    self.copied_until =
                        Some(web_time::Instant::now() + std::time::Duration::from_secs(2));
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_secs(2));
                }
                Err(error) => self.copy_error = Some(error),
            }
        }
        if self.copy.is_some() {
            ui.weak("Copying all result rows…");
        } else if self
            .copied_until
            .is_some_and(|until| web_time::Instant::now() < until)
        {
            ui.weak("Copied all result rows");
        }
        if let Some(error) = self.copy_error.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(ui.visuals().error_fg_color, error);
                if ui.button("Dismiss").clicked() {
                    self.copy_error = None;
                }
            });
        }
        let reply = self
            .worker
            .as_ref()
            .and_then(|worker| match worker.receive.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Result page worker stopped".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            });
        if let Some(result) = reply {
            self.worker = None;
            self.requested = None;
            if let Some((source, position, full_row)) = self.queued.take() {
                self.request(&source, position, full_row, ui.ctx());
            } else {
                match result {
                    Ok((offset, page, previews)) => {
                        self.offset = offset;
                        self.page = Some(page);
                        self.previews = previews;
                        selected = self
                            .focus
                            .take()
                            .and_then(|(row, column)| row.checked_sub(offset).map(|r| (r, column)));
                        self.defer_viewport = selected.is_some();
                    }
                    Err(error) => {
                        self.error = Some(error);
                        self.focus = None;
                    }
                }
            }
        }
        if let Some(error) = self.error.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(ui.visuals().error_fg_color, &error);
                if ui.button("Retry loading rows").clicked() {
                    self.request(source, self.offset, None, ui.ctx());
                }
            });
        }
        (selected, changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Value, result_store::Collector};
    #[test]
    fn prefetches_both_edges_without_restarting_useful_reads() {
        let source = Arc::new({
            let mut collector = Collector::new(0);
            for row in 0..3000 {
                collector.push(vec![Value::Integer(row)]).unwrap();
            }
            let mut grid = Grid::default();
            collector.finish(&mut grid).unwrap();
            grid
        });
        let ctx = egui::Context::default();
        let mut cached = (*source).clone();
        cached.rows = vec![vec![Value::Integer(0)]; 256];
        let mut pages = Pages {
            page: Some(Arc::new(cached)),
            offset: 512,
            ..Default::default()
        };
        pages.viewport(&source, 640..660, &ctx);
        assert!(!pages.is_loading(), "centered viewport must settle at idle");
        pages.viewport(&source, 700..720, &ctx);
        assert_eq!(pages.requested, Some((576, None)));
        assert_eq!(pages.offset, 512, "keep current rows while prefetch runs");
        pages.viewport(&source, 710..730, &ctx);
        assert_eq!(pages.requested, Some((576, None)));
        assert!(
            pages.queued.is_none(),
            "wheel events must not starve the worker"
        );
        pages.viewport(&source, 520..540, &ctx);
        assert_eq!(
            pages.requested,
            Some((416, None)),
            "reverse scroll prefetches above"
        );
        pages.viewport(&source, 2000..2020, &ctx);
        assert_eq!(
            pages.requested,
            Some((1888, None)),
            "jump supersedes stale work"
        );
        assert_eq!(pages.queued.as_ref().unwrap().1, 1888);
    }

    #[test]
    fn copy_all_reads_complete_snapshot_and_preserves_clipboard_values() {
        let mut collector = Collector::new(0);
        for row in 0..10001 {
            collector
                .push(vec![
                    Value::Integer(row),
                    Value::Text("日本".into()),
                    Value::Blob(vec![0, 255]),
                ])
                .unwrap();
        }
        let mut grid = Grid::default();
        collector.finish(&mut grid).unwrap();
        let cancel = AtomicBool::new(false);
        let copied = clipboard_result(&grid, &cancel).unwrap();
        assert_eq!(copied.lines().count(), 10001);
        assert!(copied.ends_with("10000\t日本\t00ff"));
        cancel.store(true, Ordering::Relaxed);
        assert!(clipboard_result(&grid, &cancel).is_err());
    }
    #[test]
    fn copy_limit_is_checked_before_large_value_allocation() {
        let mut collector = Collector::new(0);
        collector
            .push(vec![Value::Text("x".repeat(33 * 1024 * 1024))])
            .unwrap();
        let mut grid = Grid::default();
        collector.finish(&mut grid).unwrap();
        assert!(
            clipboard_result(&grid, &AtomicBool::new(false))
                .unwrap_err()
                .contains("32 MiB")
        );
    }
}
