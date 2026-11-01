//! Session-only pinned result snapshots. Payloads are shared, never copied.
use crate::db::{Grid, Value};
use std::sync::Arc;

const MAX_PINS: usize = 16;
const MAX_BYTES: usize = 64 * 1024 * 1024;

pub struct Pin {
    pub id: u64,
    pub grid: Arc<Grid>,
    pub statement: usize,
    bytes: usize,
}
#[derive(Default)]
pub struct Pins {
    pub items: Vec<Pin>,
    next_id: u64,
}
impl Pins {
    pub fn find(&self, grid: &Arc<Grid>) -> Option<usize> {
        self.items
            .iter()
            .position(|pin| Arc::ptr_eq(&pin.grid, grid))
    }
    pub fn add(&mut self, grid: Arc<Grid>, statement: usize) -> Result<usize, &'static str> {
        if let Some(index) = self.find(&grid) {
            return Ok(index);
        }
        let bytes = retained_bytes(&grid);
        if self.items.len() >= MAX_PINS
            || self
                .items
                .iter()
                .map(|p| p.bytes)
                .sum::<usize>()
                .saturating_add(bytes)
                > MAX_BYTES
        {
            return Err(
                "Pinned results are limited to 16 results and 64 MiB per editor. Close a pinned result before pinning another.",
            );
        }
        self.next_id += 1;
        self.items.push(Pin {
            id: self.next_id,
            grid,
            statement,
            bytes,
        });
        Ok(self.items.len() - 1)
    }
    pub fn clear(&mut self) {
        self.items.clear();
    }
}
fn retained_bytes(grid: &Grid) -> usize {
    let mut bytes = std::mem::size_of::<Grid>()
        + grid.columns.capacity() * std::mem::size_of::<String>()
        + grid.rows.capacity() * std::mem::size_of::<Vec<Value>>();
    for column in &grid.columns {
        bytes = bytes.saturating_add(column.capacity());
    }
    for row in &grid.rows {
        bytes = bytes.saturating_add(row.capacity() * std::mem::size_of::<Value>());
        for value in row {
            bytes = bytes.saturating_add(match value {
                Value::Text(text) => text.capacity(),
                Value::Blob(blob) => blob.capacity(),
                _ => 0,
            });
        }
    }
    bytes
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pins_share_payload_deduplicate_and_release() {
        let grid = Arc::new(Grid::default());
        let weak = Arc::downgrade(&grid);
        let mut pins = Pins::default();
        assert_eq!(pins.add(grid.clone(), 3), Ok(0));
        assert_eq!(pins.add(grid.clone(), 3), Ok(0));
        assert_eq!(pins.items[0].statement, 3);
        assert!(Arc::ptr_eq(&pins.items[0].grid, &grid));
        drop(grid);
        assert!(weak.upgrade().is_some());
        pins.clear();
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn pin_count_and_allocated_capacity_are_bounded() {
        let mut pins = Pins::default();
        for _ in 0..MAX_PINS {
            pins.add(Arc::default(), 0).unwrap();
        }
        assert!(pins.add(Arc::default(), 0).is_err());
        pins.clear();
        let grid = Arc::new(Grid {
            rows: vec![vec![Value::Text(String::with_capacity(MAX_BYTES))]],
            ..Default::default()
        });
        assert!(pins.add(grid, 0).is_err());
        assert!(pins.items.is_empty());
    }
}
