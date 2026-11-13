//! Identical fixtures are copied into the comparison revision by ui-performance.py.
use super::*;

#[test]
#[ignore = "release CPU benchmark; run serially through ui-performance.py"]
fn sql_highlight_frame_benchmark() {
    use std::{hint::black_box, time::Instant};
    for (name, sql) in [
        ("query", "SELECT customer_id, customer_name, total_value FROM customers WHERE total_value > 100 ORDER BY customer_name;\n".repeat(200)),
        ("whitespace", format!("SELECT\n{}customer_name, 日本語 FROM customers;", " ".repeat(32_768))),
    ] {
        let job = highlight(&sql, true);
        println!("HIGHLIGHT_MEMORY {}", serde_json::json!({
            "fixture": name, "text_bytes": sql.len(), "sections": job.sections.len(),
            "retained_job_bytes": job.text.capacity() + job.sections.capacity() * std::mem::size_of::<egui::text::LayoutSection>(),
        }));
        let mut times = Vec::new();
        for sample in 0..180 {
            let start = Instant::now();
            black_box(highlight(black_box(&sql), true));
            if sample >= 20 { times.push(start.elapsed().as_secs_f64() * 1000.0); }
        }
        times.sort_by(f64::total_cmp);
        crate::ui_benchmark::report(&format!("sql/highlight/{name}"), &times);

        for mode in ["false", "true", "resize"] {
            let editing = mode == "true";
            let ctx = egui::Context::default();
            let mut times = Vec::new();
            let variants = [sql.clone(), format!("{sql} ")];
            for frame in 0..180 {
                let text = &variants[usize::from(editing && frame % 2 == 0)];
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                    time: Some(frame as f64 / 120.0 + if frame >= 100 { 30.0 } else { 0.0 }),
                    ..Default::default()
                };
                let start = Instant::now();
                let output = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let width = if mode == "resize" { 1000.0 + (frame % 20) as f32 * 5.0 } else { 1100.0 };
                        let galley = editor_layout(ui, &text.as_str(), width);
                        ui.painter().galley(egui::Pos2::ZERO, galley, egui::Color32::WHITE);
                    });
                });
                black_box(ctx.tessellate(output.shapes, output.pixels_per_point));
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                if frame == 0 || frame == 100 {
                    println!("SQL_LAYOUT_FIRST {}", serde_json::json!({
                        "fixture": name, "mode": mode, "frame": frame, "cpu_ms": elapsed,
                    }));
                }
                if frame >= 20 { times.push(elapsed); }
            }
            times.sort_by(f64::total_cmp);
            crate::ui_benchmark::report(&format!("sql/layout/{name}/{mode}"), &times);
        }
    }
}
