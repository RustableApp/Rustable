// Shared input workload: identical production grid interactions on both revisions.
#[test]
#[ignore = "manual grid selection CPU benchmark"]
fn grid_selection_frame_benchmark() {
    let ctx = egui::Context::default();
    let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
    app.connections = false;
    app.view = View::Data;
    app.table = Arc::new(Grid {
        columns: vec!["id".into(), "name".into(), "plan".into()],
        rows: (0..500).map(|n| vec![Value::Integer(n), Value::Text(format!("Customer {n}")), Value::Text("Pro".into())]).collect(),
        ..Default::default()
    });
    let mut time = 0.0;
    let mut frame = |app: &mut Rustable, events, idle: bool| {
        time += if idle { 30.0 } else { 1.0 / 120.0 };
        let start = std::time::Instant::now();
        let output = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            time: Some(time), events, ..Default::default()
        }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| app.data_grid(ui)); });
        let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
        start.elapsed().as_secs_f64() * 1000.0
    };
    let pointer = |pos, pressed| vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
        pos, pressed, button: egui::PointerButton::Primary, modifiers: egui::Modifiers::NONE,
    }];
    frame(&mut app, vec![], false);
    let mut select = Vec::new();
    let mut clear = Vec::new();
    for iteration in 0..180 {
        let cell = egui::pos2(100.0, 50.0);
        frame(&mut app, pointer(cell, true), iteration == 100);
        frame(&mut app, pointer(cell, false), false);
        let select_ms = frame(&mut app, vec![egui::Event::Key {
            key: egui::Key::A, physical_key: None, pressed: true, repeat: false,
            modifiers: egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND,
        }], false);
        assert_eq!(app.row_selection.rows.len(), 500);
        let blank = egui::pos2(1150.0, 50.0);
        frame(&mut app, pointer(blank, true), false);
        let clear_ms = frame(&mut app, pointer(blank, false), false);
        if iteration >= 20 { select.push(select_ms); clear.push(clear_ms); }
    }
    for (name, mut samples) in [("select-all", select), ("row-tail-click", clear)] {
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report(&format!("grid/selection/{name}"), &samples);
    }
}
