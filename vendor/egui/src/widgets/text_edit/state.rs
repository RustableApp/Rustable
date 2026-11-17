use std::sync::Arc;

use crate::mutex::Mutex;

use crate::{
    Context, Id, Vec2,
    text_selection::{CCursorRange, TextCursorState},
};

/// Text history compares content, never cursor movement. Cursor snapshots are
/// still retained so undo restores the selection belonging to the edit.
#[derive(Clone, Debug, Default)]
pub struct TextEditUndoer(crate::util::undoer::Undoer<TextSnapshot>);

#[derive(Clone, Debug)]
struct TextSnapshot((CCursorRange, String));

impl PartialEq for TextSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.0.1 == other.0.1
    }
}

impl TextEditUndoer {
    pub fn has_undo(&self, current: &(CCursorRange, String)) -> bool {
        self.any_undo(|(_, text)| text != &current.1)
    }

    pub fn any_undo(&self, predicate: impl Fn(&(CCursorRange, String)) -> bool) -> bool {
        self.0.any_undo(|snapshot| predicate(&snapshot.0))
    }

    pub fn has_redo(&self, current: &(CCursorRange, String)) -> bool {
        self.0.has_redo(&TextSnapshot(current.clone()))
    }

    pub fn undo(&mut self, current: &(CCursorRange, String)) -> Option<&(CCursorRange, String)> {
        self.0
            .undo(&TextSnapshot(current.clone()))
            .map(|snapshot| &snapshot.0)
    }

    pub fn redo(&mut self, current: &(CCursorRange, String)) -> Option<&(CCursorRange, String)> {
        self.0
            .redo(&TextSnapshot(current.clone()))
            .map(|snapshot| &snapshot.0)
    }

    pub fn add_undo(&mut self, current: &(CCursorRange, String)) {
        let snapshot = TextSnapshot(current.clone());
        // An explicit edit boundary should remember its current selection even
        // when the text matches the preceding checkpoint.
        self.0.refresh_latest(&snapshot);
        self.0.add_undo(&snapshot);
    }

    pub(crate) fn feed_text(&mut self, time: f64, cursor: CCursorRange, text: &str) {
        self.0
            .feed_state(time, &TextSnapshot((cursor, text.to_owned())));
    }

    pub fn feed_state(&mut self, time: f64, current: &(CCursorRange, String)) {
        self.0.feed_state(time, &TextSnapshot(current.clone()));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditKind {
    Typing,
    Backspace,
    Delete,
    Action,
    Composition,
}

/// The text edit state stored between frames.
///
/// Attention: You also need to `store` the updated state.
/// ```
/// # egui::__run_test_ui(|ui| {
/// # let mut text = String::new();
/// use egui::text::{CCursor, CCursorRange};
///
/// let mut output = egui::TextEdit::singleline(&mut text).show(ui);
///
/// // Create a new selection range
/// let min = CCursor::new(0);
/// let max = CCursor::new(0);
/// let new_range = CCursorRange::two(min, max);
///
/// // Update the state
/// output.state.cursor.set_char_range(Some(new_range));
/// // Store the updated state
/// output.state.store(ui.ctx(), output.response.id);
/// # });
/// ```
#[derive(Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct TextEditState {
    /// Controls the text selection.
    pub cursor: TextCursorState,

    /// Wrapped in Arc for cheaper clones.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) undoer: Arc<Mutex<TextEditUndoer>>,

    /// Last coalescible edit: kind, resulting cursor and timestamp. No text copy.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) edit_group: Option<(EditKind, CCursorRange, f64)>,

    // If IME candidate window is shown on this text edit.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) ime_enabled: bool,

    // cursor range for IME candidate.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) ime_cursor_range: CCursorRange,

    // Text offset within the widget area.
    // Used for sensing and singleline text clipping.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) text_offset: Vec2,

    /// When did the user last press a key or click on the `TextEdit`.
    /// Used to pause the cursor animation when typing.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) last_interaction_time: f64,
}

impl TextEditState {
    pub fn load(ctx: &Context, id: Id) -> Option<Self> {
        ctx.data_mut(|d| d.get_persisted(id))
    }

    pub fn store(self, ctx: &Context, id: Id) {
        ctx.data_mut(|d| d.insert_persisted(id, self));
    }

    /// Inspect undo availability without cloning the retained text history.
    pub fn has_undo(&self, current: &(CCursorRange, String)) -> bool {
        self.undoer.lock().has_undo(current)
    }

    /// Cursor-only changes must not trap a cell editor's table-history handoff.
    pub fn has_text_undo(&self, current: &str) -> bool {
        self.undoer.lock().any_undo(|(_, text)| text != current)
    }

    pub fn has_redo(&self, current: &(CCursorRange, String)) -> bool {
        self.undoer.lock().has_redo(current)
    }

    pub fn undoer(&self) -> TextEditUndoer {
        self.undoer.lock().clone()
    }

    #[expect(clippy::needless_pass_by_ref_mut)] // Intentionally hide interiority of mutability
    pub fn set_undoer(&mut self, undoer: TextEditUndoer) {
        *self.undoer.lock() = undoer;
        self.edit_group = None;
    }

    pub fn clear_undoer(&mut self) {
        self.set_undoer(TextEditUndoer::default());
    }
}
