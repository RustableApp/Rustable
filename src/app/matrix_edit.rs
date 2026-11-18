//! Shared SQL editing. Text stays in the ordinary editor; no database work here.
use super::*;
use serde_json::{Value as Json, json};

pub(crate) struct Edit {
    pub id: u64,
    pub tab_id: u64,
    pub before: String,
    pub applied: String,
    prefix: String,
    suffix: String,
    inserted: String,
    pub start: usize,
    pub inserted_chars: usize,
    pub sequence: u64,
    pub streaming: bool,
    pub conflict: bool,
    pub finished: bool,
    pub synced: bool,
    cursor_before: std::ops::Range<usize>,
    undo: Option<egui::text_edit::TextEditState>,
}

impl Edit {
    pub fn summary(&self) -> Json {
        json!({"edit_id":self.id,"tab_id":self.tab_id,"next_sequence":self.sequence,
            "streaming":self.streaming,"conflict":self.conflict,"finished":self.finished,
            "changed_range":{"start":self.start,"end":self.start + self.inserted_chars}})
    }
}

impl Rustable {
    pub(super) fn check_matrix_edit(&self, bridge: &mut crate::matrix::Bridge) {
        if let Some(edit) = &mut bridge.edit
            && !self
                .tabs
                .iter()
                .any(|t| t.open && t.id == edit.tab_id && t.sql == edit.applied)
        {
            edit.streaming = false;
            edit.conflict = true;
        }
    }

    pub(super) fn begin_matrix_edit(
        &mut self,
        bridge: &mut crate::matrix::Bridge,
        index: usize,
        expected: &str,
        start: usize,
        end: usize,
    ) -> Result<Json, String> {
        if bridge.edit.as_ref().is_some_and(|e| e.streaming) {
            return Err("Finish or stop the current AI edit first.".into());
        }
        let tab = &self.tabs[index];
        if expected.len() > 32 * 1024 || tab.sql != expected {
            return Err(
                "SQL changed or exceeds 32 KiB. Inspect the tab; do not overwrite user edits."
                    .into(),
            );
        }
        if tab.formatter.busy() {
            return Err("Wait for SQL formatting to finish.".into());
        }
        let offset = |n| {
            expected
                .char_indices()
                .map(|(i, _)| i)
                .chain(std::iter::once(expected.len()))
                .nth(n)
        };
        let begin = offset(start).ok_or("Invalid start character offset")?;
        let finish = offset(end)
            .filter(|&v| v >= begin)
            .ok_or("Invalid end character offset")?;
        let edit = Edit {
            id: crate::matrix::Bridge::next_id(),
            tab_id: tab.id,
            before: expected.into(),
            applied: expected.into(),
            prefix: expected[..begin].into(),
            suffix: expected[finish..].into(),
            inserted: String::new(),
            start,
            inserted_chars: 0,
            sequence: 0,
            streaming: true,
            conflict: false,
            finished: false,
            synced: true,
            cursor_before: tab.cursor.clone(),
            undo: None,
        };
        let result = edit.summary();
        bridge.edit = Some(Box::new(edit));
        self.activate_query(index);
        Ok(result)
    }

    pub(super) fn continue_matrix_edit(
        &mut self,
        bridge: &mut crate::matrix::Bridge,
        name: &str,
        args: &Json,
    ) -> Result<Json, String> {
        self.check_matrix_edit(bridge);
        let edit = bridge.edit.as_mut().ok_or("No AI edit to continue")?;
        if args["edit_id"].as_u64() != Some(edit.id) {
            return Err("Edit target expired. Inspect the tab again.".into());
        }
        if edit.conflict {
            return Err("The user changed or closed this tab. Streaming stopped; their changes were preserved.".into());
        }
        let index = self
            .tabs
            .iter()
            .position(|t| t.open && t.id == edit.tab_id)
            .ok_or("Tab was closed")?;
        if name == "revert_query_edit" {
            self.tabs[index].sql.clone_from(&edit.before);
            self.tabs[index].cursor = edit.cursor_before.clone();
            self.tabs[index].refresh_dirty();
            // Keep ordinary editor undo/redo history. This restoration will be
            // observed by TextEdit on its next frame.
            bridge.edit = None;
            return Ok(json!({"reverted":true}));
        }
        if !edit.streaming {
            return Err(
                "Edit already stopped or finished. Start a new edit after inspecting the tab."
                    .into(),
            );
        }
        if self.view != View::Query || self.tabs[self.tab].id != edit.tab_id {
            edit.streaming = false;
            return Err("The user switched away from the edited tab. Streaming stopped.".into());
        }
        if name == "append_query_edit" {
            if args["sequence"].as_u64() != Some(edit.sequence) {
                return Err(format!(
                    "Out-of-order chunk: expected sequence {}. Inspect before retrying.",
                    edit.sequence
                ));
            }
            let chunk = args["text"].as_str().ok_or("Missing text chunk")?;
            if edit.prefix.len() + edit.inserted.len() + chunk.len() + edit.suffix.len() > 32 * 1024
            {
                return Err("Edited SQL exceeds 32 KiB".into());
            }
            edit.inserted.push_str(chunk);
            edit.inserted_chars += chunk.chars().count();
            edit.sequence += 1;
        } else if name != "finish_query_edit" {
            return Err("Unknown editing action".into());
        }
        edit.applied = format!("{}{}{}", edit.prefix, edit.inserted, edit.suffix);
        let tab = &mut self.tabs[index];
        tab.sql.clone_from(&edit.applied);
        tab.refresh_dirty();
        tab.completion.dismiss();
        edit.synced = false;
        if name == "finish_query_edit" {
            edit.streaming = false;
            edit.finished = true;
        }
        Ok(edit.summary())
    }
}

/// Keep one normal Cmd/Ctrl+Z step for the whole stream, preserving earlier undo.
/// Only touches text/layout state when a new chunk arrives, never on idle frames.
pub(super) fn sync_editor(
    bridge: &mut Option<crate::matrix::Bridge>,
    tab: &mut QueryTab,
    ctx: &egui::Context,
    editor_id: egui::Id,
) {
    let Some(edit) = bridge.as_mut().and_then(|b| b.edit.as_mut()) else {
        return;
    };
    if edit.tab_id != tab.id || edit.synced || edit.conflict || tab.sql != edit.applied {
        return;
    }
    let mut state = egui::TextEdit::load_state(ctx, editor_id).unwrap_or_default();
    let baseline = edit.undo.get_or_insert_with(|| {
        let mut undo = state.undoer();
        let range = egui::text::CCursorRange::two(
            egui::text::CCursor::new(edit.cursor_before.start),
            egui::text::CCursor::new(edit.cursor_before.end),
        );
        undo.add_undo(&(range, edit.before.clone()));
        // TextEditState::clone shares its history Arc. A fresh state keeps the
        // stream baseline independent from subsequent editor feed/undo calls.
        let mut baseline = egui::text_edit::TextEditState::default();
        baseline.set_undoer(undo);
        baseline
    });
    let caret = edit.start + edit.inserted_chars;
    let range = egui::text::CCursorRange::one(egui::text::CCursor::new(caret));
    let mut undo = baseline.undoer();
    undo.add_undo(&(range, edit.applied.clone()));
    state.set_undoer(undo);
    state.cursor.set_char_range(Some(range));
    state.store(ctx, editor_id);
    tab.cursor = caret..caret;
    tab.search.scroll = true;
    edit.synced = true;
}

pub(super) fn paint_edit(
    bridge: &Option<crate::matrix::Bridge>,
    tab: &QueryTab,
    ui: &egui::Ui,
    output: &egui::text_edit::TextEditOutput,
) {
    let Some(edit) = bridge.as_ref().and_then(|b| b.edit.as_ref()) else {
        return;
    };
    if edit.tab_id != tab.id || edit.conflict || tab.sql != edit.applied || edit.sequence == 0 {
        return;
    }
    let start = output
        .galley
        .pos_from_cursor(egui::text::CCursor::new(edit.start));
    let end = output
        .galley
        .pos_from_cursor(egui::text::CCursor::new(edit.start + edit.inserted_chars));
    let x = output.galley_pos.x - 4.0;
    let rect = egui::Rect::from_min_max(
        egui::pos2(x, output.galley_pos.y + start.top()),
        egui::pos2(x + 2.0, output.galley_pos.y + end.bottom()),
    );
    ui.painter()
        .rect_filled(rect, 1.0, ui.visuals().selection.bg_fill);
}

#[cfg(feature = "qa")]
impl Rustable {
    pub(crate) fn matrix_shared_edit_capture(
        &mut self,
        ctx: &egui::Context,
        prefix: &std::path::Path,
    ) {
        if self.pending.is_some() {
            return;
        }
        let key = egui::Id::new("matrix-shared-edit-capture");
        let mut stage = ctx.data(|d| d.get_temp::<u8>(key).unwrap_or(0));
        if stage == 0 {
            self.new_tab();
            self.inspector = false;
            self.tabs[self.tab].sql =
                "SELECT plan, COUNT(*) AS customers\nFROM customers\nGROUP BY plan;\n".into();
            self.tabs[self.tab].source_name = Some("Customer analysis.sql".into());
            self.tabs[self.tab].saved_sql = Some(Arc::new(self.tabs[self.tab].sql.clone()));
            stage = 1;
            ctx.request_repaint();
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image
            && matches!(stage, 2 | 4)
        {
            let suffix = if stage == 2 { "streaming" } else { "complete" };
            let path = format!("{}-{suffix}.ppm", prefix.display());
            std::thread::spawn(move || {
                let mut bytes =
                    format!("P6\n{} {}\n255\n", image.width(), image.height()).into_bytes();
                for pixel in &image.pixels {
                    bytes.extend_from_slice(&pixel.to_array()[..3]);
                }
                std::fs::write(path, bytes).expect("shared edit screenshot");
            });
            stage += 1;
        }
        if let Some(edit) = self.matrix.as_ref().and_then(|b| b.edit.as_ref())
            && ((stage == 1 && edit.streaming && edit.sequence >= 2)
                || (stage == 3 && edit.finished))
        {
            stage += 1;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            ctx.request_repaint();
        }
        ctx.data_mut(|d| d.insert_temp(key, stage));
    }
}
