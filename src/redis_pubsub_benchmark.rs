// Identical populated-table workload injected into both benchmark revisions.
#[test]
#[ignore = "manual bounded Pub/Sub table CPU comparison"]
fn redis_pubsub_table_frame_benchmark() {
    let ctx = egui::Context::default();
    let mut view = View::new(Settings::default(), String::new(), String::new());
    view.follow = false;
    view.received = 1000;
    for id in 0..1000 {
        let preview = format!("Message {id}").into_bytes();
        view.messages.push_back(Row {
            id, received_at: "12:30:45".into(), channel: "events:demo".into(),
            summary: format!("Message {id}"),
            message: Message { channel: b"events:demo".to_vec(), bytes: preview.len(), preview },
        });
    }
    let mut samples = Vec::new();
    for frame in 0..180 {
        let start = std::time::Instant::now();
        let output = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 760.0))),
            time: Some(frame as f64 / 120.0),
            events: vec![egui::Event::PointerMoved(egui::pos2(600.0, 300.0)),
                egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, if frame % 40 < 20 { -20.0 } else { 20.0 }), modifiers: egui::Modifiers::NONE }],
            ..Default::default()
        }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| { assert!(view.pane(ui, false, false, true).is_none()); }); });
        let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
        if frame >= 20 { samples.push(start.elapsed().as_secs_f64() * 1000.0); }
    }
    samples.sort_by(f64::total_cmp);
    crate::ui_benchmark::report("redis/pubsub-messages", &samples);
}
