// Shared selected-cell rendering fixture; also injected into the comparison base.
fn selected_editor_fixture(ctx: &egui::Context, row: usize) -> Rustable {
    let mut app = Rustable::with_launcher_preferences(ctx, Preferences::default());
    app.view = View::Data;
    app.browse.table = "customers".into();
    app.table = Arc::new(Grid {
        columns: vec!["name".into(), "plan".into()],
        rows: vec![vec![Value::Text("old".into()), Value::Text("Pro".into())]; 2],
        ..Default::default()
    });
    app.selected = app.row_selection.click(row, false, false, None);
    app.edit = draft();
    app.edit.as_mut().unwrap().row = app.table.rows[row].clone();
    app
}

#[test]
fn editing_cell_restores_its_stripe_without_erasing_selected_neighbors() {
    for dark in [false, true] {
        for row in [0, 1] {
            let ctx = egui::Context::default();
            apply_theme(&ctx, if dark { Appearance::Dark } else { Appearance::Light });
            let mut app = selected_editor_fixture(&ctx, row);
            let mut normal = Color32::TRANSPARENT;
            let mut selected = Color32::TRANSPARENT;
            let mut output = egui::FullOutput::default();
            for _ in 0..2 {
                output = ctx.run(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 500.0))),
                    ..Default::default()
                }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                    normal = if row.is_multiple_of(2) { ui.visuals().faint_bg_color } else { ui.visuals().panel_fill };
                    selected = ui.visuals().selection.bg_fill;
                    app.data_grid(ui);
                }); });
            }
            let (selection_index, selection) = output.shapes.iter().enumerate().find_map(|(index, shape)| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill == selected && rect.rect.width() > 700.0 => Some((index, rect.rect)),
                _ => None,
            }).expect("selected row remains painted across the viewport");
            let (cell_index, cell) = output.shapes.iter().enumerate().find_map(|(index, shape)| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill == normal && rect.rect.width() < 300.0 && rect.rect.width() > 40.0 && selection.contains_rect(rect.rect) => Some((index, rect.rect)),
                _ => None,
            }).expect("edited cell has a full neutral background");
            assert!(cell_index > selection_index, "neutral cell must cover the blue row fill");
            assert!(cell.right() < selection.right() - 100.0, "neighboring cells remain selected");
            assert_eq!(cell.top(), selection.top());
            assert_eq!(cell.bottom(), selection.bottom());
        }
    }
}

#[test]
#[ignore = "manual selected-cell editing CPU benchmark"]
fn selected_cell_edit_frame_benchmark() {
    for dark in [false, true] {
        let ctx = egui::Context::default();
        apply_theme(&ctx, if dark { Appearance::Dark } else { Appearance::Light });
        let mut app = selected_editor_fixture(&ctx, 1);
        let mut samples = Vec::new();
        for frame in 0..180 {
            let start = Instant::now();
            let output = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 500.0))),
                time: Some(frame as f64 / 120.0 + if frame >= 100 { 30.0 } else { 0.0 }),
                ..Default::default()
            }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| app.data_grid(ui)); });
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 20 { samples.push(start.elapsed().as_secs_f64() * 1000.0); }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report(&format!("grid/selected-cell-edit/{dark}"), &samples);
    }
}

