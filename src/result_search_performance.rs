//! Shared fixtures for completed scans and the loaded-result search panel.
use super::*;
use std::{hint::black_box, time::Instant};

#[test]
#[ignore = "release CPU benchmark; run serially through ui-performance.py"]
fn result_search_cpu_benchmark() {
    for (name, grid, needle, expected) in [
        ("text", Grid { rows: vec![vec![Value::Text("payload ".repeat(256))]; 5_000], ..Default::default() }, "absent", 0),
        ("numeric", Grid { rows: vec![vec![Value::Integer(42); 120]; 500], ..Default::default() }, "42", 60_000),
        ("large-cell", Grid { rows: vec![vec![Value::Text("x".repeat(16 * 1024 * 1024))]], ..Default::default() }, "absent", 0),
    ] {
        let grid = Arc::new(grid);
        let cancel = AtomicBool::new(false);
        let mut samples = Vec::new();
        for index in 0..180 {
            let start = Instant::now();
            let cancel = black_box(&cancel);
            let found = scan_until_cancelled(&grid, needle.into(), false,
                || cancel.load(Ordering::Relaxed)).unwrap();
            black_box(&found);
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(found.cells.len(), expected);
            if index >= 20 { samples.push(elapsed); }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report(&format!("search/scan/{name}"), &samples);
    }
    let ctx = egui::Context::default();
    let grid = Arc::new(Grid {
        rows: vec![vec![Value::Text("match".into()); 120]; 500], ..Default::default()
    });
    let mut search = Search {
        open: true, needle: "match".into(),
        found: Some(scan(&grid, "match".into(), false)), ..Default::default()
    };
    let mut samples = Vec::new();
    for frame in 0..180 {
        search.step = 1;
        let start = Instant::now();
        let output = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            time: Some(frame as f64 / 120.0 + if frame >= 100 { 30.0 } else { 0.0 }),
            ..Default::default()
        }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| { black_box(search.show(ui, &grid)); });
        });
        black_box(ctx.tessellate(output.shapes, output.pixels_per_point));
        if frame >= 20 { samples.push(start.elapsed().as_secs_f64() * 1000.0); }
    }
    samples.sort_by(f64::total_cmp);
    crate::ui_benchmark::report("search/panel-navigation", &samples);
}
