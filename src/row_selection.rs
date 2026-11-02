use std::collections::BTreeSet;

#[derive(Default, Clone)]
pub(super) struct RowSelection {
    pub rows: BTreeSet<usize>,
    anchor: Option<usize>,
    pub pending_focus: Option<usize>,
    pub focused: Option<usize>,
    pub cell_column: usize,
    pub pending_cell_scroll: bool,
}
impl RowSelection {
    pub fn clear(&mut self) {
        self.rows.clear();
        self.anchor = None;
        self.pending_focus = None;
        self.focused = None;
        self.cell_column = 0;
        self.pending_cell_scroll = false;
    }
    pub fn click(
        &mut self,
        index: usize,
        shift: bool,
        command: bool,
        previous: Option<usize>,
    ) -> Option<usize> {
        self.focused = Some(index);
        if self.rows.is_empty()
            && let Some(previous) = previous
        {
            self.rows.insert(previous);
            self.anchor = Some(previous);
        }
        if shift {
            let anchor = self.anchor.unwrap_or(index);
            if !command {
                self.rows.clear();
            }
            self.rows.extend(anchor.min(index)..=anchor.max(index));
            self.anchor = Some(anchor);
        } else if command {
            if !self.rows.remove(&index) {
                self.rows.insert(index);
            }
            self.anchor = Some(index);
        } else {
            self.rows.clear();
            self.rows.insert(index);
            self.anchor = Some(index);
        }
        // Single-row mutation controls must never silently act on one of many rows.
        (self.rows.len() == 1).then(|| *self.rows.first().unwrap())
    }
    pub fn context_click(&mut self, index: usize) -> Option<usize> {
        if self.rows.contains(&index) {
            self.focused = Some(index);
            (self.rows.len() == 1).then_some(index)
        } else { self.click(index, false, false, None) }
    }

    pub fn select_all(&mut self, count: usize, focused: usize) -> Option<usize> {
        self.focused = (focused < count).then_some(focused);
        self.rows = (0..count).collect();
        self.anchor = (focused < count).then_some(focused);
        (count == 1).then_some(0)
    }

    pub fn move_vertical(
        &mut self,
        focused: usize,
        down: bool,
        shift: bool,
        count: usize,
    ) -> Option<usize> {
        if focused >= count {
            return None;
        }
        let next = if down {
            focused.saturating_add(1).min(count - 1)
        } else {
            focused.saturating_sub(1)
        };
        self.click(next, shift, false, Some(focused));
        self.pending_focus = Some(next);
        self.pending_cell_scroll = true;
        Some(next)
    }

    pub fn copy_indices(&self, clicked: usize) -> Vec<usize> {
        if self.rows.contains(&clicked) {
            self.rows.iter().copied().collect()
        } else {
            vec![clicked]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_click_preserves_selection_or_selects_outside_row() {
        let mut s = RowSelection::default();
        s.click(1, false, false, None);
        s.click(3, true, false, None);
        assert_eq!(s.context_click(2), None);
        assert_eq!(s.copy_indices(2), vec![1, 2, 3]);
        assert_eq!(s.focused, Some(2));
        assert_eq!(s.context_click(7), Some(7));
        assert_eq!(s.copy_indices(7), vec![7]);
    }
    #[test]
    fn arrows_extend_contract_and_stop_at_loaded_page_boundaries() {
        let mut s = RowSelection::default();
        s.click(2, false, false, None);
        assert_eq!(s.move_vertical(2, true, true, 5), Some(3));
        assert_eq!(s.copy_indices(2), vec![2, 3]);
        s.move_vertical(3, false, true, 5);
        assert_eq!(s.copy_indices(2), vec![2]);
        s.move_vertical(2, false, true, 5);
        assert_eq!(s.copy_indices(2), vec![1, 2]);
        s.move_vertical(1, false, false, 5);
        assert_eq!(s.copy_indices(0), vec![0]);
        assert_eq!(s.move_vertical(0, false, false, 5), Some(0));
        assert_eq!(s.move_vertical(4, true, false, 5), Some(4));
        assert_eq!(s.move_vertical(9, true, false, 5), None);
        s.clear();
        assert_eq!(s.pending_focus, None);
        assert_eq!(s.move_vertical(0, true, false, 0), None);
        assert!(s.rows.is_empty());
    }

    #[test]
    fn range_toggle_and_plain_click() {
        let mut s = RowSelection::default();
        assert_eq!(s.click(4, false, false, None), Some(4));
        assert_eq!(s.click(1, true, false, Some(4)), None);
        assert_eq!(s.copy_indices(2), vec![1, 2, 3, 4]);
        s.click(2, false, true, None);
        assert_eq!(s.copy_indices(1), vec![1, 3, 4]);
        assert_eq!(s.copy_indices(9), vec![9]);
        assert_eq!(s.click(8, false, false, None), Some(8));
        assert_eq!(s.click(8, false, true, Some(8)), None);
        assert!(s.rows.is_empty());
    }
    #[test]
    fn select_all_replaces_stale_indices_and_keeps_focused_anchor() {
        let mut s = RowSelection::default();
        s.click(99, false, false, None);
        assert_eq!(s.select_all(5, 2), None);
        assert_eq!(s.copy_indices(0), vec![0, 1, 2, 3, 4]);
        s.click(4, true, false, None);
        assert_eq!(s.copy_indices(2), vec![2, 3, 4]);
        assert_eq!(s.select_all(1, 0), Some(0));
        assert_eq!(s.select_all(0, 0), None);
        assert!(s.rows.is_empty());
    }

    #[test]
    fn range_extends_from_prior_single_selection_and_clear_resets_anchor() {
        let mut s = RowSelection::default();
        s.click(7, true, false, Some(5));
        assert_eq!(s.copy_indices(5), vec![5, 6, 7]);
        s.clear();
        assert_eq!(s.click(2, true, false, None), Some(2));
        s.click(5, true, true, None);
        assert_eq!(s.copy_indices(2), vec![2, 3, 4, 5]);
    }
}
