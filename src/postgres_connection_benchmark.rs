// Same host-edit, scroll and after-idle workload on the base and candidate.
#[test]
#[ignore = "manual PostgreSQL connection form CPU comparison"]
fn postgres_connection_profile_frame_benchmark() {
    for (width, height) in [(1100.0, 760.0), (780.0, 600.0)] {
        let ctx = egui::Context::default();
        let mut app = app();
        app.connections = true;
        app.child_window = false;
        app.connection_manager.draft = Some(Profile::draft(
            "PostgreSQL".into(),
            ConnectionKind::Postgres(PostgresProfile::default()),
        ));
        let mut samples = Vec::new();
        for frame in 0..180 {
            if let ConnectionKind::Postgres(profile) =
                &mut app.connection_manager.draft.as_mut().unwrap().connection
            {
                profile.host = format!("database-{}.example.test", frame % 10);
            }
            let start = std::time::Instant::now();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, height),
                    )),
                    time: Some(frame as f64 / 120.0 + if frame >= 100 { 30.0 } else { 0.0 }),
                    events: vec![
                        egui::Event::PointerMoved(egui::pos2(width - 150.0, 500.0)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, if frame % 40 < 20 { -20.0 } else { 20.0 }),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ctx| app.connections_window(ctx),
            );
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report(
            &format!("postgres/connection-profile/{width}x{height}"),
            &samples,
        );
    }
}

#[test]
#[ignore = "manual PostgreSQL TLS popup CPU comparison"]
fn postgres_tls_menu_frame_benchmark() {
    for (width, height) in [(1100.0, 760.0), (780.0, 600.0)] {
        let ctx = egui::Context::default();
        let mut app = app();
        app.connections = true;
        app.child_window = false;
        app.connection_manager.draft = Some(Profile::draft("PostgreSQL".into(), ConnectionKind::Postgres(PostgresProfile::default())));
        let render = |app: &mut Rustable, frame: usize, events| ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height))),
            time: Some(frame as f64 / 120.0 + if frame >= 100 { 30.0 } else { 0.0 }),
            events, ..Default::default()
        }, |ctx| app.connections_window(ctx));
        for frame in 0..3 { render(&mut app, frame, vec![]); }
        let output = render(&mut app, 3, vec![]);
        let position = painted_text(&output, "Verify full").unwrap();
        for (frame, pressed) in [(4, true), (5, false)] {
            render(&mut app, frame, vec![egui::Event::PointerMoved(position), egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }]);
        }
        let mut samples = Vec::new();
        for frame in 6..186 {
            let start = std::time::Instant::now();
            let output = render(&mut app, frame, vec![]);
            assert!(painted_text(&output, "Disabled (unencrypted)").is_some() || painted_text(&output, "Disabled (unencrypted)\nTraffic is not encrypted.").is_some());
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 26 { samples.push(start.elapsed().as_secs_f64() * 1000.0); }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report(&format!("postgres/tls-menu/{width}x{height}"), &samples);
    }
}
