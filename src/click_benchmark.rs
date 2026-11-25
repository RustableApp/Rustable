// Copied unchanged into the baseline by scripts/ui-performance.py.
#[test]
#[ignore = "release-mode shared click-path CPU comparison"]
fn shared_click_frame_benchmark() {
    for view in ["editor", "connections"] {
        let ctx = egui::Context::default();
        ctx.set_os(egui::os::OperatingSystem::Mac);
        crate::context_click::install(&ctx);
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        app.connections = true;
        for index in 0..20 {
            app.connection_action(Action::New(false)).unwrap();
            let draft = app.connection_manager.draft.as_mut().unwrap();
            draft.name = format!("Click fixture {index:02}");
            draft.connection = ConnectionKind::SQLite {
                path: PathBuf::from(":memory:"),
            };
            app.connection_action(Action::Save(false)).unwrap();
        }
        let mut sql = "SELECT alpha, beta FROM customers;\nSELECT gamma FROM orders;\n".repeat(20);
        let mut point = egui::pos2(90.0, 18.0);
        let mut samples = Vec::new();
        for frame in 0..180 {
            let phase = frame % 16;
            let mut events = Vec::new();
            if matches!(phase, 1..=6) {
                events.push(egui::Event::PointerMoved(point));
                events.push(egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: phase % 2 == 1,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let input = egui::RawInput {
                events,
                time: Some(frame as f64 * 0.08 + if frame >= 100 { 30.0 } else { 0.0 }),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 900.0),
                )),
                ..Default::default()
            };
            let start = std::time::Instant::now();
            let output = ctx.run(input, |ctx| {
                if view == "connections" {
                    app.connections_window(ctx);
                } else {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let output = egui::TextEdit::multiline(&mut sql)
                            .code_editor()
                            .layouter(&mut crate::editor::editor_layout)
                            .desired_width(1300.0)
                            .show(ui);
                        std::hint::black_box(output.cursor_range);
                    });
                }
            });
            std::hint::black_box(ctx.tessellate(output.shapes.clone(), output.pixels_per_point));
            if frame >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            if view == "connections" {
                point = painted_text(&output, "Click fixture 00").expect("visible fixture row");
                // Requests contain only disposable profiles; the fixture never opens them.
                app.window_requests.clear();
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report(&format!("click/{view}"), &samples);
    }
}
