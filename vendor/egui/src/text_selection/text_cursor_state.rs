//! Text cursor changes/interaction, without modifying the text.

use epaint::text::{Galley, cursor::CCursor};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{NumExt as _, Rect, Response, Ui, epaint};

use super::CCursorRange;

/// The state of a text cursor selection.
///
/// Used for [`crate::TextEdit`] and [`crate::Label`].
#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct TextCursorState {
    ccursor_range: Option<CCursorRange>,
    #[cfg_attr(feature = "serde", serde(skip))]
    drag_selection: Option<(CCursorRange, u32)>,
}

impl From<CCursorRange> for TextCursorState {
    fn from(ccursor_range: CCursorRange) -> Self {
        Self {
            ccursor_range: Some(ccursor_range),
            drag_selection: None,
        }
    }
}

impl TextCursorState {
    pub fn is_empty(&self) -> bool {
        self.ccursor_range.is_none()
    }

    /// The currently selected range of characters.
    pub fn char_range(&self) -> Option<CCursorRange> {
        self.ccursor_range
    }

    /// The currently selected range of characters, clamped within the character
    /// range of the given [`Galley`].
    pub fn range(&self, galley: &Galley) -> Option<CCursorRange> {
        self.ccursor_range.map(|mut range| {
            range.primary = galley.clamp_cursor(&range.primary);
            range.secondary = galley.clamp_cursor(&range.secondary);
            range
        })
    }

    /// Sets the currently selected range of characters.
    pub fn set_char_range(&mut self, ccursor_range: Option<CCursorRange>) {
        self.ccursor_range = ccursor_range;
    }
}

impl TextCursorState {
    /// Handle clicking and/or dragging text.
    ///
    /// Returns `true` if there was interaction.
    pub fn pointer_interaction(
        &mut self,
        ui: &Ui,
        response: &Response,
        cursor_at_pointer: CCursor,
        galley: &Galley,
        is_being_dragged: bool,
    ) -> bool {
        let text = galley.text();

        let pressed = ui.input(|i| i.pointer.button_pressed(crate::PointerButton::Primary));
        if response.hovered() && pressed {
            let count = ui.ctx().interaction_snapshot(|i| i.press_count);
            if count >= 2 {
                let range = if count == 2 {
                    select_word_at(text, cursor_at_pointer)
                } else {
                    select_click_line_at(text, cursor_at_pointer, ui.ctx().os())
                };
                self.set_char_range(Some(range));
                self.drag_selection = Some((range, count));
            } else {
                self.drag_selection = None;
                if ui.input(|i| i.modifiers.shift) {
                    let mut range = self
                        .range(galley)
                        .unwrap_or(CCursorRange::one(cursor_at_pointer));
                    range.primary = cursor_at_pointer;
                    self.set_char_range(Some(range));
                } else {
                    self.set_char_range(Some(CCursorRange::one(cursor_at_pointer)));
                }
            }
            true
        } else if response.sense.senses_drag()
            && is_being_dragged
            && ui.input(|i| i.pointer.primary_down())
        {
            if let Some((anchor, count)) = self.drag_selection {
                let extent = if count == 2 {
                    select_word_at(text, cursor_at_pointer)
                } else {
                    select_click_line_at(text, cursor_at_pointer, ui.ctx().os())
                };
                let [start, end] = anchor.sorted_cursors();
                let [extent_start, extent_end] = extent.sorted_cursors();
                let range = if extent_start.index < start.index {
                    CCursorRange::two(end, extent_start)
                } else {
                    CCursorRange::two(
                        start,
                        if extent_end.index > end.index {
                            extent_end
                        } else {
                            end
                        },
                    )
                };
                self.set_char_range(Some(range));
            } else if let Some(mut range) = self.range(galley) {
                range.primary = cursor_at_pointer;
                self.set_char_range(Some(range));
            }
            true
        } else {
            if !ui.input(|i| i.pointer.primary_down()) {
                self.drag_selection = None;
            }
            false
        }
    }
}

fn select_word_at(text: &str, ccursor: CCursor) -> CCursorRange {
    if ccursor.index == 0 {
        CCursorRange::two(ccursor, ccursor_next_word(text, ccursor))
    } else {
        let it = text.chars();
        let mut it = it.skip(ccursor.index - 1);
        if let Some(char_before_cursor) = it.next() {
            if let Some(char_after_cursor) = it.next() {
                if is_word_char(char_before_cursor) && is_word_char(char_after_cursor) {
                    let min = selection_previous_word(text, ccursor + 1);
                    let max = ccursor_next_word(text, min);
                    CCursorRange::two(min, max)
                } else if is_word_char(char_before_cursor) {
                    let min = selection_previous_word(text, ccursor);
                    let max = ccursor_next_word(text, min);
                    CCursorRange::two(min, max)
                } else if is_word_char(char_after_cursor) {
                    let max = ccursor_next_word(text, ccursor);
                    CCursorRange::two(ccursor, max)
                } else {
                    let min = selection_previous_word(text, ccursor);
                    let max = ccursor_next_word(text, ccursor);
                    CCursorRange::two(min, max)
                }
            } else {
                let min = selection_previous_word(text, ccursor);
                CCursorRange::two(min, ccursor)
            }
        } else {
            let max = ccursor_next_word(text, ccursor);
            CCursorRange::two(ccursor, max)
        }
    }
}

// Walk Unicode word boundaries without reversing/cloning the text on every
// double-click drag frame. Keyboard navigation retains its existing behavior.
fn selection_previous_word(text: &str, cursor: CCursor) -> CCursor {
    let mut start = 0;
    let mut boundary = 0;
    for word in text.split_word_bounds() {
        if start >= cursor.index {
            break;
        }
        let end = start + word.chars().count();
        if end < cursor.index && !all_word_chars(word) {
            boundary = end;
        }
        for (offset, ch) in word.chars().enumerate() {
            if ch == '.' && start + offset + 1 < cursor.index {
                boundary = boundary.max(start + offset + 1);
            }
        }
        start = end;
    }
    CCursor {
        index: boundary,
        prefer_next_row: true,
    }
}

/// AppKit selects a paragraph including its line ending, even on an empty line.
fn select_click_line_at(
    text: &str,
    cursor: CCursor,
    os: crate::os::OperatingSystem,
) -> CCursorRange {
    if os != crate::os::OperatingSystem::Mac {
        return select_line_at(text, cursor);
    }
    let mut chars = text.chars().enumerate().peekable();
    let mut start = 0;
    let mut end = 0;
    while let Some((index, ch)) = chars.next() {
        end = index + 1;
        if is_linebreak(ch) {
            if ch == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
                end = chars.next().unwrap().0 + 1;
            }
            if cursor.index < end {
                return CCursorRange::two(CCursor::new(start), CCursor::new(end));
            }
            start = end;
        }
    }
    CCursorRange::two(CCursor::new(start), CCursor::new(end))
}

fn select_line_at(text: &str, ccursor: CCursor) -> CCursorRange {
    if ccursor.index == 0 {
        CCursorRange::two(ccursor, ccursor_next_line(text, ccursor))
    } else {
        let it = text.chars();
        let mut it = it.skip(ccursor.index - 1);
        if let Some(char_before_cursor) = it.next() {
            if let Some(char_after_cursor) = it.next() {
                if (!is_linebreak(char_before_cursor)) && (!is_linebreak(char_after_cursor)) {
                    let min = ccursor_previous_line(text, ccursor + 1);
                    let max = ccursor_next_line(text, min);
                    CCursorRange::two(min, max)
                } else if !is_linebreak(char_before_cursor) {
                    let min = ccursor_previous_line(text, ccursor);
                    let max = ccursor_next_line(text, min);
                    CCursorRange::two(min, max)
                } else if !is_linebreak(char_after_cursor) {
                    let max = ccursor_next_line(text, ccursor);
                    CCursorRange::two(ccursor, max)
                } else {
                    let min = ccursor_previous_line(text, ccursor);
                    let max = ccursor_next_line(text, ccursor);
                    CCursorRange::two(min, max)
                }
            } else {
                let min = ccursor_previous_line(text, ccursor);
                CCursorRange::two(min, ccursor)
            }
        } else {
            let max = ccursor_next_line(text, ccursor);
            CCursorRange::two(ccursor, max)
        }
    }
}

pub fn ccursor_next_word(text: &str, ccursor: CCursor) -> CCursor {
    CCursor {
        index: next_word_boundary_char_index(text, ccursor.index),
        prefer_next_row: false,
    }
}

fn ccursor_next_line(text: &str, ccursor: CCursor) -> CCursor {
    CCursor {
        index: next_line_boundary_char_index(text.chars(), ccursor.index),
        prefer_next_row: false,
    }
}

pub fn ccursor_previous_word(text: &str, ccursor: CCursor) -> CCursor {
    let num_chars = text.chars().count();
    let reversed: String = text.graphemes(true).rev().collect();
    CCursor {
        index: num_chars
            - next_word_boundary_char_index(&reversed, num_chars - ccursor.index).min(num_chars),
        prefer_next_row: true,
    }
}

fn ccursor_previous_line(text: &str, ccursor: CCursor) -> CCursor {
    let num_chars = text.chars().count();
    CCursor {
        index: num_chars
            - next_line_boundary_char_index(text.chars().rev(), num_chars - ccursor.index),
        prefer_next_row: true,
    }
}

fn next_word_boundary_char_index(text: &str, cursor_ci: usize) -> usize {
    let mut word_ci = 0;
    for word in text.split_word_bounds() {

        // We consider `.` a word boundary.
        // At least that's how Mac works when navigating something like `www.example.com`.
        for (dot_ci_offset, chr) in word.chars().enumerate() {
            let dot_ci = word_ci + dot_ci_offset;
            if chr == '.' && cursor_ci < dot_ci {
                return dot_ci;
            }
        }

        // Splitting considers contiguous whitespace as one word, such words must be skipped,
        // this handles cases for example ' abc' (a space and a word), the cursor is at the beginning
        // (before space) - this jumps at the end of 'abc' (this is consistent with text editors
        // or browsers)
        if cursor_ci < word_ci && !all_word_chars(word) {
            return word_ci;
        }
        word_ci += word.chars().count();
    }

    word_ci
}

fn all_word_chars(text: &str) -> bool {
    text.chars().all(is_word_char)
}

fn next_line_boundary_char_index(it: impl Iterator<Item = char>, mut index: usize) -> usize {
    let mut it = it.skip(index);
    if let Some(_first) = it.next() {
        index += 1;

        if let Some(second) = it.next() {
            index += 1;
            for next in it {
                if is_linebreak(next) != is_linebreak(second) {
                    break;
                }
                index += 1;
            }
        }
    }
    index
}

pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_linebreak(c: char) -> bool {
    c == '\r' || c == '\n'
}

/// Accepts and returns character offset (NOT byte offset!).
pub fn find_line_start(text: &str, current_index: CCursor) -> CCursor {
    // We know that new lines, '\n', are a single byte char, but we have to
    // work with char offsets because before the new line there may be any
    // number of multi byte chars.
    // We need to know the char index to be able to correctly set the cursor
    // later.
    let chars_count = text.chars().count();

    let position = text
        .chars()
        .rev()
        .skip(chars_count - current_index.index)
        .position(|x| x == '\n');

    match position {
        Some(pos) => CCursor::new(current_index.index - pos),
        None => CCursor::new(0),
    }
}

pub fn byte_index_from_char_index(s: &str, char_index: usize) -> usize {
    for (ci, (bi, _)) in s.char_indices().enumerate() {
        if ci == char_index {
            return bi;
        }
    }
    s.len()
}

pub fn char_index_from_byte_index(input: &str, byte_index: usize) -> usize {
    for (ci, (bi, _)) in input.char_indices().enumerate() {
        if bi == byte_index {
            return ci;
        }
    }

    input.char_indices().last().map_or(0, |(i, _)| i + 1)
}

pub fn slice_char_range(s: &str, char_range: std::ops::Range<usize>) -> &str {
    assert!(
        char_range.start <= char_range.end,
        "Invalid range, start must be less than end, but start = {}, end = {}",
        char_range.start,
        char_range.end
    );
    let start_byte = byte_index_from_char_index(s, char_range.start);
    let end_byte = byte_index_from_char_index(s, char_range.end);
    &s[start_byte..end_byte]
}

/// The thin rectangle of one end of the selection, e.g. the primary cursor, in local galley coordinates.
pub fn cursor_rect(galley: &Galley, cursor: &CCursor, row_height: f32) -> Rect {
    let mut cursor_pos = galley.pos_from_cursor(*cursor);

    // Handle completely empty galleys
    cursor_pos.max.y = cursor_pos.max.y.at_least(cursor_pos.min.y + row_height);

    cursor_pos = cursor_pos.expand(1.5); // slightly above/below row

    cursor_pos
}

#[cfg(test)]
mod test {
    use crate::text_selection::text_cursor_state::next_word_boundary_char_index;

    #[test]
    fn selection_previous_word_preserves_word_boundaries() {
        use super::*;
        for text in [
            "abc.def.ghi",
            "SELECT café, value FROM customers;\n",
            "abc d3f g_h i-j",
            "❤️👍 skvělá knihovna 👍❤️",
            "hello   world",
            "don't stop",
            "a\r\nb",
        ] {
            for index in 0..=text.chars().count() {
                assert_eq!(
                    selection_previous_word(text, CCursor::new(index)),
                    ccursor_previous_word(text, CCursor::new(index)),
                    "{text:?} at {index}"
                );
            }
        }
    }

    #[test]
    fn test_next_word_boundary_char_index() {
        // ASCII only
        let text = "abc d3f g_h i-j";
        assert_eq!(next_word_boundary_char_index(text, 1), 3);
        assert_eq!(next_word_boundary_char_index(text, 3), 7);
        assert_eq!(next_word_boundary_char_index(text, 9), 11);
        assert_eq!(next_word_boundary_char_index(text, 12), 13);
        assert_eq!(next_word_boundary_char_index(text, 13), 15);
        assert_eq!(next_word_boundary_char_index(text, 15), 15);

        assert_eq!(next_word_boundary_char_index("", 0), 0);
        assert_eq!(next_word_boundary_char_index("", 1), 0);

        // ASCII only
        let text = "abc.def.ghi";
        assert_eq!(next_word_boundary_char_index(text, 1), 3);
        assert_eq!(next_word_boundary_char_index(text, 3), 7);
        assert_eq!(next_word_boundary_char_index(text, 7), 11);

        // Unicode graphemes, some of which consist of multiple Unicode characters,
        // !!! Unicode character is not always what is tranditionally considered a character,
        // the values below are correct despite not seeming that way on the first look,
        // handling of and around emojis is kind of weird and is not consistent across
        // text editors and browsers
        let text = "❤️👍 skvělá knihovna 👍❤️";
        assert_eq!(next_word_boundary_char_index(text, 0), 2);
        assert_eq!(next_word_boundary_char_index(text, 2), 3); // this does not skip the space between thumbs-up and 'skvělá'
        assert_eq!(next_word_boundary_char_index(text, 6), 10);
        assert_eq!(next_word_boundary_char_index(text, 9), 10);
        assert_eq!(next_word_boundary_char_index(text, 12), 19);
        assert_eq!(next_word_boundary_char_index(text, 15), 19);
        assert_eq!(next_word_boundary_char_index(text, 19), 20);
        assert_eq!(next_word_boundary_char_index(text, 20), 21);
    }
}
