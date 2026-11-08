//! Identical workload copied to the comparison worktree by --redis-vectors.
#[test]
#[ignore = "manual vector controls CPU comparison"]
fn redis_vector_controls_frame_benchmark() {
    use eframe::egui;
    for scenario in ["results", "typing", "after-idle"] {
        let ctx = egui::Context::default();
        let mut draft =
            crate::redis_vector_search::Draft::new(b"vectors".to_vec(), b"source".to_vec());
        draft.prepare_benchmark(scenario != "results");
        let value = redis::Value::Array(
            (0..20)
                .flat_map(|i| {
                    [
                        redis::Value::BulkString(
                            format!("product:{i:04}:description").into_bytes(),
                        ),
                        redis::Value::BulkString(format!("{}", 1.0 - i as f64 / 40.0).into_bytes()),
                    ]
                })
                .collect(),
        );
        let matches =
            crate::redis_vector_matches::Matches::decode(value, b"vectors", b"source").unwrap();
        let mut samples = Vec::new();
        for frame in 0..180 {
            let start = std::time::Instant::now();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 760.0),
                    )),
                    time: Some(
                        frame as f64 / 120.0
                            + if scenario == "after-idle" && frame >= 100 {
                                30.0
                            } else {
                                0.0
                            },
                    ),
                    events: if scenario == "typing" {
                        vec![egui::Event::Text("1".into())]
                    } else {
                        vec![]
                    },
                    ..Default::default()
                },
                |ctx| {
                    egui::Window::new("Similar elements")
                        .fixed_pos(egui::pos2(20.0, 20.0))
                        .default_width(640.0)
                        .show(ctx, |ui| {
                            crate::redis_workspace::browser_style(ui);
                            if scenario != "results"
                                || crate::redis_vector_search::BENCHMARK_HAS_CONTROLS
                            {
                                assert!(draft.show(ui, true).is_none());
                            } else {
                                let _ = ui.add(crate::redis_workspace::action_button(
                                    "refresh-cw",
                                    "Refresh",
                                ));
                            }
                            let _ = matches.show(ui);
                        });
                },
            );
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report(&format!("redis/vector-controls/{scenario}"), &samples);
    }
}
