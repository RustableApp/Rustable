fn pending_row_frame(ctx: &egui::Context, app: &mut Rustable, events: Vec<egui::Event>, time: f64) -> egui::FullOutput {
    ctx.run(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 500.0))),
        time: Some(time), events, ..Default::default()
    }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| app.data_grid(ui)); })
}

#[test]
fn pending_rows_are_green_selectable_and_reopen_from_text_or_padding() {
    for dark in [false, true] {
        for padding in [false, true] {
            let ctx = egui::Context::default();
            apply_theme(&ctx, if dark { Appearance::Dark } else { Appearance::Light });
            let mut app = app();
            app.connections = false;
            app.columns[0].pk = true;
            Arc::make_mut(&mut app.table).rows = vec![vec![Value::Integer(1), Value::Null, Value::Text("Existing".into())]];
            app.changes.push(Change::Cell { table: "t".into(), column: "name".into(),
                original: app.table.rows[0].clone(), original_columns: None, value: Value::Text("Changed".into()) });
            app.add_insert();
            let id = app.selected_insert().unwrap();
            assert!(app.finish_cell_edit());
            app.set_insert_value(id, "name", Some(Value::Text("New".into())));
            app.selected = None;
            app.row_selection.clear();
            let mut time = 30.0;
            let mut frame = |app: &mut Rustable, events| {
                time += 0.05;
                pending_row_frame(&ctx, app, events, time)
            };
            frame(&mut app, vec![]);
            let output = frame(&mut app, vec![]);
            let mut green = Color32::TRANSPARENT;
            let mut orange = Color32::TRANSPARENT;
            let _ = ctx.run(egui::RawInput::default(), |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                green = insert_fill(ui); orange = pending_fill(ui);
            }); });
            let green_cells: Vec<_> = output.shapes.iter().filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill == green => Some(rect.rect), _ => None,
            }).collect();
            assert_eq!(green_cells.len(), 3);
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == orange)), "existing changes stay orange");
            assert!(green.g() > green.r() && green.g() > green.b());
            let cell = *green_cells.last().unwrap();
            let point = if padding { egui::pos2(cell.right() - 4.0, cell.center().y) } else { egui::pos2(cell.left() + 14.0, cell.center().y) };
            // Restore the grid hit regions after the palette read above.
            frame(&mut app, vec![]);
            frame(&mut app, vec![]);
            let click = |pos, pressed| vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                pos, pressed, button: egui::PointerButton::Primary, modifiers: egui::Modifiers::NONE,
            }];
            frame(&mut app, click(point, true));
            let output = frame(&mut app, click(point, false));
            assert_eq!(app.selected_insert(), Some(id));
            assert_eq!(app.row_selection.cell_columns, Some((2, 2)));
            assert!(app.edit.is_none(), "single click only selects");
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect)
                if rect.stroke.width == 1.5 && rect.rect.contains(point))), "pending cell has the shared selection outline");
            frame(&mut app, click(point, true));
            frame(&mut app, click(point, false));
            assert_eq!(app.edit.as_ref().unwrap().insert_id, Some(id));
            assert_eq!(app.edit.as_ref().unwrap().column, "name");
            frame(&mut app, vec![]);
            frame(&mut app, vec![egui::Event::Text("Again ".into())]);
            let blank = egui::pos2(700.0, 350.0);
            frame(&mut app, click(blank, true));
            frame(&mut app, click(blank, false));
            frame(&mut app, vec![]);
            assert!(app.edit.is_none());
            assert_eq!(app.selected, None);
            assert_eq!(app.insert_value(id, "name"), Some(&Value::Text("Again New".into())));
            // A second edit after blur must not hit a retired TextEdit or lose the draft.
            for _ in 0..2 { for pressed in [true, false] { frame(&mut app, click(point, pressed)); } }
            assert_eq!(app.edit.as_ref().unwrap().insert_id, Some(id));
            assert_eq!(app.edit.as_ref().unwrap().text, "Again New");
            assert_eq!(app.insert_ids(), vec![id]);
        }
    }
}

#[test]
fn pending_generated_fields_stay_readonly_on_double_click() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.connections = false;
    app.add_insert();
    let id = app.selected_insert().unwrap();
    assert!(app.finish_cell_edit());
    let mut time = 0.0;
    let mut frame = |app: &mut Rustable, events| { time += 0.05; pending_row_frame(&ctx, app, events, time) };
    frame(&mut app, vec![]);
    let output = frame(&mut app, vec![]);
    let point = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.text() == "GENERATED" => Some(text.pos + text.galley.size() * 0.5), _ => None,
    }).unwrap();
    for _ in 0..2 { for pressed in [true, false] {
        frame(&mut app, vec![egui::Event::PointerMoved(point), egui::Event::PointerButton {
            pos: point, pressed, button: egui::PointerButton::Primary, modifiers: egui::Modifiers::NONE,
        }]);
    } }
    assert_eq!(app.selected_insert(), Some(id));
    assert!(app.edit.is_none());
}

fn pending_key(key: egui::Key) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
}

#[test]
fn pending_keyboard_cursor_crosses_rows_and_enter_edits_without_mouse() {
    for loaded in [0, 1] {
        let ctx = egui::Context::default();
        let mut app = app();
        app.connections = false;
        app.columns[0].pk = true;
        if loaded > 0 { Arc::make_mut(&mut app.table).rows = vec![vec![Value::Integer(1), Value::Null, Value::Text("Existing".into())]]; }
        app.add_insert();
        let first = app.selected_insert().unwrap();
        app.add_insert();
        let second = app.selected_insert().unwrap();
        let mut time = 30.0;
        let mut frame = |app: &mut Rustable, events| { time += 0.05; pending_row_frame(&ctx, app, events, time) };
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        frame(&mut app, vec![pending_key(egui::Key::Enter)]);
        assert!(app.edit.is_none(), "Enter stages the initial draft");
        frame(&mut app, vec![pending_key(egui::Key::ArrowUp)]);
        assert_eq!(app.selected_insert(), Some(first));
        assert_eq!(app.row_selection.focused, Some(loaded));
        frame(&mut app, vec![pending_key(egui::Key::ArrowUp)]);
        assert_eq!(app.selected, Some(0));
        if loaded > 0 {
            assert_eq!(app.row_selection.rows.iter().copied().collect::<Vec<_>>(), vec![0]);
            frame(&mut app, vec![pending_key(egui::Key::ArrowDown)]);
            assert_eq!(app.selected_insert(), Some(first));
        }
        frame(&mut app, vec![pending_key(egui::Key::ArrowRight)]);
        assert_eq!(app.row_selection.cell_column, 1);
        frame(&mut app, vec![pending_key(egui::Key::Enter)]);
        assert!(app.edit.is_none(), "generated cells remain selectable but readonly");
        frame(&mut app, vec![pending_key(egui::Key::ArrowRight)]);
        assert_eq!(app.row_selection.cell_column, 2);
        frame(&mut app, vec![pending_key(egui::Key::Enter)]);
        assert_eq!(app.edit.as_ref().unwrap().insert_id, Some(first));
        assert_eq!(app.edit.as_ref().unwrap().column, "name");
        frame(&mut app, vec![]);
        frame(&mut app, vec![egui::Event::Text("First draft".into())]);
        frame(&mut app, vec![pending_key(egui::Key::Enter)]);
        assert!(app.edit.is_none());
        assert_eq!(app.insert_value(first, "name"), Some(&Value::Text("First draft".into())));
        frame(&mut app, vec![pending_key(egui::Key::ArrowDown)]);
        assert_eq!(app.selected_insert(), Some(second));
        frame(&mut app, vec![pending_key(egui::Key::Enter)]);
        assert_eq!(app.edit.as_ref().unwrap().insert_id, Some(second));
        frame(&mut app, vec![]);
        frame(&mut app, vec![pending_key(egui::Key::Escape)]);
        frame(&mut app, vec![pending_key(egui::Key::ArrowLeft)]);
        assert_eq!(app.row_selection.cell_column, 1, "Escape returns navigation to the grid");
        assert!(app.edit.is_none());
    }
}

#[test]
fn pending_row_tail_selects_full_row_then_arrows_select_cells() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.connections = false;
    app.add_insert();
    let id = app.selected_insert().unwrap();
    app.finish_cell_edit();
    app.selected = None;
    app.row_selection.clear();
    let mut time = 30.0;
    let mut frame = |app: &mut Rustable, events| { time += 0.05; pending_row_frame(&ctx, app, events, time) };
    frame(&mut app, vec![]); frame(&mut app, vec![]);
    let pos = egui::pos2(700.0, 44.0);
    for pressed in [true, false] {
        frame(&mut app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
            pos, pressed, button: egui::PointerButton::Primary, modifiers: egui::Modifiers::NONE,
        }]);
    }
    assert_eq!(app.selected_insert(), Some(id));
    assert_eq!(app.row_selection.cell_columns, None);
    frame(&mut app, vec![pending_key(egui::Key::ArrowRight)]);
    assert_eq!(app.row_selection.cell_columns, Some((1, 1)));
    assert_eq!(app.insert_ids(), vec![id]);
}

#[test]
fn pending_keyboard_scrolls_to_offscreen_row_and_column() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.connections = false;
    app.columns = (0..30).map(|n| Column { name: format!("field_{n}"), data_type: "TEXT".into(),
        not_null: false, default: String::new(), pk: n == 0, generated: false }).collect();
    Arc::make_mut(&mut app.table).columns = app.columns.iter().map(|c| c.name.clone()).collect();
    for _ in 0..30 { app.add_insert(); }
    let first = app.insert_ids()[0];
    let mut time = 0.0;
    let mut frame = |app: &mut Rustable, events| { time += 0.05; pending_row_frame(&ctx, app, events, time) };
    for _ in 0..20 { frame(&mut app, vec![]); }
    frame(&mut app, vec![pending_key(egui::Key::Enter)]);
    assert!(app.edit.is_none(), "editor remains {:?}", app.edit.as_ref().map(|e| (&e.column, e.focus)));
    let keys = (0..29).map(|_| pending_key(egui::Key::ArrowUp))
        .chain((0..29).map(|_| pending_key(egui::Key::ArrowRight))).collect();
    frame(&mut app, keys);
    for _ in 0..20 { frame(&mut app, vec![]); }
    let output = frame(&mut app, vec![]);
    assert_eq!(app.selected_insert(), Some(first));
    assert_eq!(app.row_selection.cell_column, 29);
    assert!(!app.row_selection.pending_cell_scroll);
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect)
        if rect.stroke.width == 1.5 && rect.rect.intersect(shape.clip_rect).width() > 20.0
            && rect.rect.intersect(shape.clip_rect).height() > 10.0)), "navigation reveals the selected draft cell");
    frame(&mut app, vec![pending_key(egui::Key::Enter)]);
    assert_eq!(app.edit.as_ref().unwrap().column, "field_29");
    assert_eq!(app.edit.as_ref().unwrap().insert_id, Some(first));
}

#[test]
#[ignore = "manual pending-row keyboard interaction CPU benchmark"]
fn pending_row_navigation_frame_benchmark() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.connections = false;
    for _ in 0..50 { app.add_insert(); }
    app.finish_cell_edit();
    app.selected = Some(0);
    app.row_selection.focus_pending(0);
    let mut time = 0.0;
    let mut frame = |app: &mut Rustable, events, idle| {
        time += if idle { 30.0 } else { 0.05 };
        let start = std::time::Instant::now();
        let output = pending_row_frame(&ctx, app, events, time);
        let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
        start.elapsed().as_secs_f64() * 1000.0
    };
    frame(&mut app, vec![], false);
    frame(&mut app, vec![], false);
    let pos = egui::pos2(245.0, 44.0);
    for pressed in [true, false] {
        frame(&mut app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
            pos, pressed, button: egui::PointerButton::Primary, modifiers: egui::Modifiers::NONE,
        }], false);
    }
    let mut samples = Vec::new();
    for iteration in 0..180 {
        let next = if iteration % 2 == 0 { 1 } else { 0 };
        let ms = frame(&mut app, vec![pending_key(if next == 1 { egui::Key::ArrowDown } else { egui::Key::ArrowUp })], iteration == 100);
        assert_eq!(app.selected, Some(next));
        frame(&mut app, vec![], false);
        frame(&mut app, vec![pending_key(egui::Key::Enter)], false);
        assert_eq!(app.edit.as_ref().and_then(|e| e.insert_id), Some(app.insert_ids()[next]));
        frame(&mut app, vec![], false);
        frame(&mut app, vec![pending_key(egui::Key::Escape)], false);
        assert!(app.edit.is_none());
        if iteration >= 20 { samples.push(ms); }
    }
    samples.sort_by(f64::total_cmp);
    crate::ui_benchmark::report("grid/pending/navigation", &samples);
}

#[test]
fn pending_labels_do_not_select_text_outside_editor() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.connections = false;
    app.add_insert();
    let id = app.selected_insert().unwrap();
    app.finish_cell_edit();
    app.set_insert_value(id, "name", Some(Value::Text("Draft text".into())));
    pending_row_frame(&ctx, &mut app, vec![], 0.0);
    let output = pending_row_frame(&ctx, &mut app, vec![], 0.1);
    let start = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.text() == "Draft text" => Some(text.pos + egui::vec2(2.0, 6.0)),
        _ => None,
    }).unwrap();
    let end = start + egui::vec2(45.0, 0.0);
    pending_row_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(start), egui::Event::PointerButton {
        pos: start, pressed: true, button: egui::PointerButton::Primary, modifiers: egui::Modifiers::NONE,
    }], 0.2);
    pending_row_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(end)], 0.3);
    pending_row_frame(&ctx, &mut app, vec![egui::Event::PointerButton {
        pos: end, pressed: false, button: egui::PointerButton::Primary, modifiers: egui::Modifiers::NONE,
    }], 0.4);
    let output = pending_row_frame(&ctx, &mut app, vec![egui::Event::Copy], 0.5);
    assert!(app.edit.is_none());
    assert!(!output.platform_output.commands.iter().any(|command| matches!(command,
        egui::OutputCommand::CopyText(text) if !text.is_empty())), "display labels must not create a text selection");
}

#[test]
fn pending_history_moves_selector_to_changed_field_in_both_directions() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.connections = false;
    app.add_insert();
    let first = app.selected_insert().unwrap();
    app.finish_cell_edit();
    app.set_insert_value(first, "name", Some(Value::Text("Draft".into())));
    app.add_insert();
    let second = app.selected_insert().unwrap();
    app.finish_cell_edit();
    app.set_insert_value(first, "name", Some(Value::Text("Changed".into())));
    // Deliberately select the other draft before each action.
    for (time, redo) in [(1.0, false), (2.0, true)] {
        app.selected = Some(1);
        app.row_selection.focus_pending(1);
        let events = [true, false].map(|pressed| egui::Event::Key {
            key: egui::Key::Z, physical_key: None, pressed, repeat: false,
            modifiers: egui::Modifiers { command: true, shift: redo, ..Default::default() },
        }).to_vec();
        let _ = ctx.run(egui::RawInput { time: Some(time), events, ..Default::default() }, |ctx| app.staged_history_shortcuts(ctx));
        assert_eq!(app.selected_insert(), Some(first));
        assert_eq!(app.row_selection.focused, Some(0));
        assert_eq!(app.row_selection.cell_columns, Some((2, 2)));
        assert_eq!(app.row_selection.pending_focus, Some(0));
        assert!(app.row_selection.pending_cell_scroll);
        assert_eq!(app.insert_ids(), vec![first, second]);
        assert_eq!(app.insert_value(first, "name"), Some(&Value::Text(if redo { "Changed" } else { "Draft" }.into())));
    }
}
