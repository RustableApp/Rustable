//! Same action sequence and owned return values on both history implementations.
use super::*;
use std::{hint::black_box, time::Instant};

fn value(id: usize) -> Change {
    Change::Cell {
        original_columns: None,
        table: "fixture".into(),
        column: "payload".into(),
        original: vec![Value::Integer(id as i64)],
        value: Value::Text("x".repeat(16 * 1024)),
    }
}

#[test]
#[ignore = "release action CPU benchmark; run serially through ui-performance.py"]
fn change_history_action_benchmark() {
    let mut history = ChangeHistory::default();
    let mut changes = Vec::new();
    let mut record = Vec::new();
    for index in 0..180 {
        changes.push(value(index));
        let start = Instant::now();
        history.record(black_box(&changes));
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        if index >= 20 { record.push(elapsed); }
    }
    record.sort_by(f64::total_cmp);
    crate::ui_benchmark::report("history/record", &record);
    for redo in [false, true] {
        let mut times = Vec::new();
        for index in 0..180 {
            let start = Instant::now();
            let result = if redo { history.redo() } else { history.undo() }.unwrap();
            black_box(&result);
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(result.len(), if redo { index + 1 } else { 179 - index });
            if index >= 20 { times.push(elapsed); }
        }
        times.sort_by(f64::total_cmp);
        crate::ui_benchmark::report(if redo { "history/redo" } else { "history/undo" }, &times);
    }
}

#[test]
#[ignore = "run each revision in a separate process under /usr/bin/time -l"]
fn change_history_memory_benchmark() {
    let mut history = ChangeHistory::default();
    let mut changes = Vec::new();
    for index in 0..128 {
        changes.push(value(index));
        history.record(&changes);
    }
    assert_eq!(history.undo().unwrap().len(), 127);
    assert_eq!(history.redo().unwrap(), changes);
    black_box(&history);
    println!("History retained after 128 edits, each with 16 KiB text; undo/redo verified");
}
