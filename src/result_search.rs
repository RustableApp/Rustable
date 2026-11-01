//! Literal search of loaded result cells, off the render thread.
use crate::db::{self, Grid, Value};
use eframe::egui;
use std::sync::{Arc, Weak, mpsc};

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
    worker: Option<mpsc::Receiver<Result<Found, String>>>,
    error: Option<String>,
    #[cfg(target_arch = "wasm32")]
    browser_worker: Option<crate::browser_jobs::Handle>,
    current: usize,
}
impl Found {
    fn matches(&self, grid: &Arc<Grid>, needle: &str, case: bool) -> bool {
        self.source.ptr_eq(&Arc::downgrade(grid))
            && self.needle == needle
            && self.case_sensitive == case
    }
}
pub(crate) fn scan(grid: &Arc<Grid>, needle: String, case_sensitive: bool) -> Found {
    let mut result = Found {
        source: Arc::downgrade(grid),
        needle,
        case_sensitive,
        cells: Vec::new(),
        limited: false,
    };
    if result.needle.is_empty() {
        return result;
    }
    let Ok(pattern) = regex::RegexBuilder::new(&regex::escape(&result.needle))
        .case_insensitive(!case_sensitive)
        .build()
    else {
        return result;
    };
    for (row, values) in grid.rows.iter().enumerate() {
        for (column, value) in values.iter().enumerate() {
            let rendered;
            let text = if let Value::Text(text) = value {
                text.as_str()
            } else {
                rendered = db::display(value);
                &rendered
            };
            if pattern.is_match(text) {
                if result.cells.len() == MATCH_LIMIT {
                    result.limited = true;
                    return result;
                }
                result.cells.push((row, column));
            }
        }
    }
    result
}
impl Search {
    pub fn has_focus(&self, ctx: &egui::Context) -> bool {
        self.open
            && self
                .field_id
                .is_some_and(|id| ctx.memory(|m| m.has_focus(id)))
    }
    pub fn show(&mut self, ui: &mut egui::Ui, grid: &Arc<Grid>) -> Option<Cell> {
        if !self.open {
            self.worker = None;
            #[cfg(target_arch = "wasm32")]
            {
                self.browser_worker = None;
            }
            return None;
        }
        let mut move_by = std::mem::take(&mut self.step);
        ui.horizontal_wrapped(|ui| {
            ui.label("Find in results");
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.needle)
                    .char_limit(1024)
                    .desired_width(180.0)
                    .hint_text("Loaded cells only"),
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
                "Search loaded values".into()
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
            self.worker = None;
            #[cfg(target_arch = "wasm32")]
            { self.browser_worker = None; }
            return None;
        }
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
        if let Some(receiver) = &self.worker {
            match receiver.try_recv() {
                Ok(Ok(found)) => {
                    // The status label was painted before consuming this reply.
                    ui.ctx().request_repaint();
                    if found.matches(grid, &self.needle, self.case_sensitive) {
                        self.current = 0;
                        target = found.cells.first().copied();
                        self.found = Some(found);
                    }
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
            // One in-flight scan per tab; input changes coalesce while it runs.
            // Retain only the existing Arc, and cap matching coordinates.
            let (tx, rx) = mpsc::sync_channel(1);
            let source = grid.clone();
            let needle = self.needle.clone();
            let case = self.case_sensitive;
            let ctx = ui.ctx().clone();
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::spawn(move || {
                let _ = tx.send(Ok(scan(&source, needle, case)));
                ctx.request_repaint();
            });
            self.worker = Some(rx);
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
                                Ok(found)
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
    use super::*;
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
