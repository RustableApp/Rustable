//! Shared production-layout fixture for regression checks and native captures.
use super::*;

fn fixture(ctx: &egui::Context, count: usize) -> Rustable {
    let mut app = Rustable::with_optional_connection(ctx, Preferences::default(), None, None);
    app.connections = false;
    app.view = View::Query;
    app.inspector = false;
    app.prefs.sidebar_hidden = true;
    app.prefs.activity_visible = false;
    app.label = "Studio demo".into();
    app.engine = "SQLite".into();
    let tab = &mut app.tabs[0];
    tab.open = true;
    tab.sql = "-- Customer report\nSELECT id, name, plan FROM customers LIMIT 5;\n\n-- Order summary\nSELECT status, COUNT(*) AS orders FROM orders GROUP BY status;\n\n-- Recent activity\nSELECT * FROM events LIMIT 12;\n\n-- Update confirmation\nUPDATE customers SET active = 1 WHERE id = 42;".into();
    tab.cursor = 19..19;
    tab.results = (0..count)
        .map(|index| {
            Arc::new(if index == 3 {
                Grid {
                    affected: 1,
                    ..Default::default()
                }
            } else {
                Grid {
                    columns: vec!["id".into(), "name".into(), "plan".into()],
                    rows: [
                        "Avery Chen",
                        "Noah Williams",
                        "Sofia Martinez",
                        "Liam Anderson",
                        "Isla Thompson",
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(row, name)| {
                        vec![
                            Value::Integer((row + 1) as i64),
                            Value::Text(name.into()),
                            Value::Text(if row % 2 == 0 { "Pro" } else { "Starter" }.into()),
                        ]
                    })
                    .collect(),
                    ..Default::default()
                }
            })
        })
        .collect();
    tab.grid = tab.results.first().cloned().unwrap_or_default();
    app
}

#[cfg(feature = "qa")]
impl Rustable {
    pub fn run_query_layout_preview() -> eframe::Result {
        struct Preview {
            app: Rustable,
            frame: usize,
            assist: bool,
            path: std::path::PathBuf,
            saved: Option<std::sync::mpsc::Receiver<()>>,
        }
        impl eframe::App for Preview {
            fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
                if self.assist {
                    match self.frame {
                        3 => input.events.push(egui::Event::Text("(".into())),
                        4 => input.events.push(egui::Event::Key {
                            key: egui::Key::Enter,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        }),
                        5 => input.events.push(egui::Event::Text("SELECT 1".into())),
                        _ => {}
                    }
                }
            }

            fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
                if self.saved.as_ref().is_some_and(|rx| rx.try_recv().is_ok()) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                let screenshot = ctx.input(|input| {
                    input.events.iter().find_map(|event| {
                        if let egui::Event::Screenshot { image, .. } = event {
                            Some(image.clone())
                        } else {
                            None
                        }
                    })
                });
                if let Some(image) = screenshot {
                    let path = self.path.clone();
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.saved = Some(rx);
                    std::thread::spawn(move || {
                        let mut data =
                            format!("P6\n{} {}\n255\n", image.width(), image.height()).into_bytes();
                        for pixel in &image.pixels {
                            data.extend_from_slice(&pixel.to_array()[..3]);
                        }
                        std::fs::write(path, data).expect("write native query screenshot");
                        let _ = tx.send(());
                    });
                }
                self.app.toolbar(ctx);
                self.app.workspace(ctx);
                if self.frame == if self.assist { 8 } else { 3 } {
                    if self.assist {
                        assert_eq!(
                            self.app.tabs[0].sql,
                            "SELECT *\nFROM customers\n\tWHERE EXISTS (\n\t\tSELECT 1\n\t)"
                        );
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
                self.frame += 1;
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        let path = std::path::PathBuf::from(std::env::var_os("RUSTABLE_QUERY_CAPTURE").unwrap());
        let light = std::env::var_os("RUSTABLE_QUERY_LIGHT").is_some();
        let assist = std::env::var_os("RUSTABLE_EDITOR_ASSIST_CAPTURE").is_some();
        let width = std::env::var("RUSTABLE_QUERY_WIDTH")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(1280.0);
        eframe::run_native(
            "Query layout preview",
            eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default().with_inner_size([width, 760.0]),
                persist_window: false,
                ..Default::default()
            },
            Box::new(move |cc| {
                crate::fonts::install(&cc.egui_ctx);
                let result_count = std::env::var("RUSTABLE_QUERY_RESULT_COUNT")
                    .ok().and_then(|value| value.parse().ok()).unwrap_or(8);
                let mut app = fixture(&cc.egui_ctx, if assist { 0 } else { result_count });
                if assist {
                    app.tabs[0].sql = "SELECT *\nFROM customers\n\tWHERE EXISTS ".into();
                    let end = app.tabs[0].sql.chars().count();
                    app.tabs[0].cursor = end..end;
                    app.tabs[0].focus_editor = true;
                }
                app.prefs.appearance = if light {
                    Appearance::Light
                } else {
                    Appearance::Dark
                };
                apply_theme(&cc.egui_ctx, app.prefs.appearance);
                let grid = app.tabs[0].grid.clone();
                if !assist && result_count > 1 {
                    app.tabs[0].pins.add(grid, 0).unwrap();
                }
                Ok(Box::new(Preview {
                    app,
                    frame: 0,
                    assist,
                    path,
                    saved: None,
                }))
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_rects(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect, usize)>) {
        match shape {
            egui::Shape::Text(text) => out.push((
                text.galley.text().into(),
                egui::Rect::from_min_size(text.pos, text.galley.size()),
                text.galley.rows.len(),
            )),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    text_rects(shape, out);
                }
            }
            _ => {}
        }
    }
    fn frame(
        ctx: &egui::Context,
        app: &mut Rustable,
        width: f32,
        events: Vec<egui::Event>,
    ) -> Vec<(String, egui::Rect, usize)> {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 760.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.workspace(ctx),
        );
        let mut text = vec![];
        for clipped in output.shapes {
            text_rects(&clipped.shape, &mut text);
        }
        text
    }

    #[test]
    fn query_footer_tracks_text_origin_and_tabs_stay_single_line() {
        for light in [false, true] {
            for width in [500.0, 760.0, 1280.0] {
                let ctx = egui::Context::default();
                let mut app = fixture(&ctx, 128);
                apply_theme(
                    &ctx,
                    if light {
                        Appearance::Light
                    } else {
                        Appearance::Dark
                    },
                );
                app.tabs[0].sql = format!("{}SELECT 1;", "\n".repeat(999));
                let mut first_y = None;
                for index in 0..12 {
                    let texts = frame(&ctx, &mut app, width, vec![]);
                    let status = texts
                        .iter()
                        .find(|(s, _, _)| s.starts_with("Line "))
                        .unwrap()
                        .1;
                    let sql = texts
                        .iter()
                        .find(|(s, _, _)| s.ends_with("SELECT 1;"))
                        .unwrap()
                        .1;
                    assert!(
                        (status.left() - sql.left()).abs() < 1.0,
                        "counter must follow SQL text origin: {status:?} {sql:?}"
                    );
                    let tabs: Vec<_> = texts
                        .iter()
                        .filter(|(s, _, _)| s.starts_with("Result "))
                        .collect();
                    assert!(
                        !tabs.is_empty() && tabs.len() < 12,
                        "only visible tabs should be rendered"
                    );
                    for (_, rect, rows) in &tabs {
                        assert_eq!(*rows, 1);
                        assert!(rect.top() > status.bottom());
                    }
                    if index == 2 {
                        first_y = Some(tabs[0].1.top());
                    }
                    if index > 2 {
                        assert!(
                            (tabs[0].1.top() - first_y.unwrap()).abs() < 0.1,
                            "pane height drift"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn query_result_tabs_select_and_reveal_offscreen_results() {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx, 128);
        frame(&ctx, &mut app, 760.0, vec![]);
        let texts = frame(&ctx, &mut app, 760.0, vec![]);
        let pos = texts
            .iter()
            .find(|(s, _, _)| s.starts_with("Result 2 ·"))
            .unwrap()
            .1
            .center();
        for pressed in [true, false] {
            frame(
                &ctx,
                &mut app,
                760.0,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        assert_eq!(app.tabs[0].result_index, 1);
        assert!(Arc::ptr_eq(&app.tabs[0].grid, &app.tabs[0].results[1]));
        app.tabs[0].result_index = 127;
        for _ in 0..4 {
            frame(&ctx, &mut app, 760.0, vec![]);
        }
        let texts = frame(&ctx, &mut app, 760.0, vec![]);
        assert!(texts.iter().any(|(s, _, _)| s.starts_with("Result 128 ·")));
    }

    #[test]
    #[ignore = "manual query editor/results CPU benchmark"]
    fn query_editor_results_frame_benchmark() {
        for (count, width) in [(0, 1280.0), (4, 1280.0), (128, 760.0), (4, 500.0)] {
            let ctx = egui::Context::default();
            let mut app = fixture(&ctx, count);
            let mut samples = Vec::new();
            for index in 0..180 {
                if count > 0 {
                    app.tabs[0].result_index = index % count;
                }
                let started = std::time::Instant::now();
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 760.0),
                        )),
                        time: Some(index as f64 / 120.0 + if index >= 100 { 30.0 } else { 0.0 }),
                        ..Default::default()
                    },
                    |ctx| app.workspace(ctx),
                );
                let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
                if index >= 20 {
                    samples.push(started.elapsed().as_secs_f64() * 1000.0);
                }
            }
            samples.sort_by(f64::total_cmp);
            crate::ui_benchmark::report(&format!("query/editor-results/{count}/{width}"), &samples);
        }

        // Same input and text on both revisions; includes first input after idle.
        let ctx = egui::Context::default();
        let id = egui::Id::new("history-benchmark");
        let original = "SELECT id, name FROM customers;\n".repeat(200);
        let mut text = original.clone();
        let mut samples = Vec::new();
        for index in 0..180 {
            text.clone_from(&original);
            let time = index as f64 * 31.0;
            let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
            state.clear_undoer();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(0))));
            state.store(&ctx, id);
            ctx.memory_mut(|m| m.request_focus(id));
            for measured in [false, true] {
                let mut events = Vec::new();
                if measured {
                    events.push(egui::Event::Text("a".into()));
                    for modifiers in [egui::Modifiers::COMMAND, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT] {
                        for pressed in [true, false] {
                            events.push(egui::Event::Key { key: egui::Key::Z, physical_key: None, pressed, repeat: false, modifiers });
                        }
                    }
                }
                let started = std::time::Instant::now();
                let output = ctx.run(egui::RawInput {
                    events, time: Some(time + if measured { 30.0 } else { 0.0 }),
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                    ..Default::default()
                }, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        egui::TextEdit::multiline(&mut text).id(id).code_editor()
                            .layouter(&mut crate::editor::editor_layout).show(ui);
                    });
                });
                let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
                if measured && index >= 20 { samples.push(started.elapsed().as_secs_f64() * 1000.0); }
            }
            assert_eq!(text, format!("a{original}"));
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("query/editor-history", &samples);
    }
}
