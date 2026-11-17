#[test]
#[ignore = "manual pending-row rendering CPU benchmark"]
fn pending_rows_frame_benchmark() {
    for mode in ["idle", "editing"] { pending_row_benchmark_case(mode); }
}

#[test]
#[ignore = "manual new pending-row reediting interaction benchmark"]
fn pending_row_reopen_frame_benchmark() {
    pending_row_benchmark_case("reopen");
}

fn pending_row_benchmark_case(mode: &str) {
    let ctx = egui::Context::default();
    let mut app = app();
    app.connections = false;
    for row in 0..50 {
        app.add_insert();
        let id = app.selected_insert().unwrap();
        app.finish_cell_edit();
        app.set_insert_value(id, "name", Some(Value::Text(format!("Draft {row}"))));
    }
    let id = app.insert_ids()[0];
    app.selected = Some(0);
    if mode == "editing" { app.start_insert_edit(id, "name", false); }
    let mut time = 0.0;
    let mut frame = |app: &mut Rustable, events, idle| {
        time += if idle { 30.0 } else { 0.05 };
        let started = std::time::Instant::now();
        let output = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 600.0))),
            time: Some(time), events, ..Default::default()
        }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| app.data_grid(ui)); });
        let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
        started.elapsed().as_secs_f64() * 1000.0
    };
    frame(&mut app, vec![], false);
    let mut samples = Vec::new();
    for iteration in 0..180 {
        if mode == "reopen" {
            app.finish_cell_edit();
            // Retire the prior TextEdit before testing a fresh double-click.
            // This action is candidate-only: the baseline cannot reopen it.
            frame(&mut app, vec![], iteration == 100);
            frame(&mut app, vec![], false);
            let pos = egui::pos2(245.0, 44.0);
            for pressed in [true, false, true] {
                frame(&mut app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                    pos, pressed, button: egui::PointerButton::Primary, modifiers: egui::Modifiers::NONE,
                }], false);
            }
            let ms = frame(&mut app, vec![egui::Event::PointerButton {
                pos, pressed: false, button: egui::PointerButton::Primary, modifiers: egui::Modifiers::NONE,
            }], false);
            assert_eq!(app.edit.as_ref().map(|edit| edit.insert_id), Some(Some(id)));
            if iteration >= 20 { samples.push(ms); }
        } else {
            let ms = frame(&mut app, vec![], iteration == 100);
            if iteration >= 20 { samples.push(ms); }
        }
    }
    samples.sort_by(f64::total_cmp);
    crate::ui_benchmark::report(&format!("grid/pending/{mode}"), &samples);
}
