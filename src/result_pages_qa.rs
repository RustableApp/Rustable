//! Disposable complete-result UI checks; never opens a database connection.
use super::*;
fn fixture(ctx: &egui::Context) -> Rustable {
    let grid = std::thread::spawn(|| {
        let mut collector = db::result_store::Collector::new(1024);
        for row in 0..10021 {
            collector
                .push(vec![
                    Value::Integer(row),
                    Value::Text(format!("Customer {row} · 日本")),
                    Value::Text("Pro".into()),
                ])
                .unwrap();
        }
        let mut grid = Grid {
            columns: vec!["id".into(), "name".into(), "plan".into()],
            ..Default::default()
        };
        collector.finish(&mut grid).unwrap();
        Arc::new(grid)
    })
    .join()
    .unwrap();
    let mut app = Rustable::with_optional_connection(ctx, Preferences::default(), None, None);
    app.connections = false;
    app.view = View::Query;
    app.inspector = false;
    app.prefs.sidebar_hidden = true;
    app.prefs.activity_visible = false;
    app.label = "Disposable results fixture".into();
    app.engine = "SQLite".into();
    let tab = &mut app.tabs[0];
    tab.open = true;
    tab.sql = "-- Complete results, stored locally
SELECT id, name, plan FROM customers;"
        .into();
    tab.grid = grid.clone();
    tab.results = vec![grid];
    app
}
#[cfg(feature = "qa")]
impl Rustable {
    pub fn run_result_pages_preview() -> eframe::Result {
        struct Preview {
            app: Rustable,
            frame: usize,
            last: bool,
            path: std::path::PathBuf,
            saved: Option<std::sync::mpsc::Receiver<()>>,
        }
        impl eframe::App for Preview {
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
                if self.last && self.frame == 2 {
                    let tab = &mut self.app.tabs[0];
                    tab.result_pages
                        .select(&tab.grid, tab.grid.row_count() - 1, 1, ctx);
                }
                let loading_preview = std::env::var_os("RUSTABLE_RESULT_LOADING").is_some();
                if loading_preview && self.frame >= 80 {
                    // Force a cache miss only in this disposable screenshot fixture.
                    self.app.tabs[0].result_pages.offset = 50000;
                }
                self.app.toolbar(ctx);
                self.app.workspace(ctx);
                if self.frame >= 90
                    && self.saved.is_none()
                    && !self.app.grid().rows.is_empty()
                    && (loading_preview || !self.app.tabs[0].result_pages.is_loading())
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
                self.frame += 1;
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        let path = std::path::PathBuf::from(std::env::var_os("RUSTABLE_RESULT_CAPTURE").unwrap());
        let light = std::env::var_os("RUSTABLE_RESULT_LIGHT").is_some();
        let width = std::env::var("RUSTABLE_RESULT_WIDTH")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(1280.0);
        eframe::run_native(
            "Complete results preview",
            eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default().with_inner_size([width, 760.0]),
                persist_window: false,
                ..Default::default()
            },
            Box::new(move |cc| {
                crate::fonts::install(&cc.egui_ctx);
                let mut app = fixture(&cc.egui_ctx);
                app.prefs.appearance = if light {
                    Appearance::Light
                } else {
                    Appearance::Dark
                };
                apply_theme(&cc.egui_ctx, app.prefs.appearance);
                let grid = app.tabs[0].grid.clone();
                app.tabs[0].pins.add(grid, 0).unwrap();
                Ok(Box::new(Preview {
                    app,
                    frame: 0,
                    last: std::env::var_os("RUSTABLE_RESULT_LAST").is_some(),
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
    fn frame(ctx: &egui::Context, app: &mut Rustable, width: f32, time: f64) {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 760.0),
                )),
                time: Some(time.max(ctx.input(|input| input.time) + 1.0 / 120.0)),
                ..Default::default()
            },
            |ctx| app.workspace(ctx),
        );
        let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
    }
    fn wait_page(ctx: &egui::Context, app: &mut Rustable, width: f32, expected: usize) {
        let start = std::time::Instant::now();
        loop {
            frame(ctx, app, width, start.elapsed().as_secs_f64());
            if app.tabs[0].result_pages.offset == expected && !app.grid().rows.is_empty() {
                break;
            }
            assert!(start.elapsed().as_secs() < 5, "page worker stalled");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    #[test]
    fn search_navigation_uses_global_rows_and_result_switch_clears_page() {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx);
        wait_page(&ctx, &mut app, 760.0, 0);
        assert_eq!(app.grid().rows.len(), 256);
        let tab = &mut app.tabs[0];
        assert!(tab.result_pages.select(&tab.grid, 10020, 1, &ctx).is_none());
        wait_page(&ctx, &mut app, 760.0, 10020);
        assert_eq!(app.grid().rows[0][0], Value::Integer(10020));
        assert_eq!(app.selected, Some(0));
        assert_eq!(app.row_selection.cell_column, 1);
        assert!(app.query_edit_target().is_none());
        let grid = Arc::new(Grid {
            columns: vec!["id".into()],
            rows: vec![vec![Value::Integer(42)]],
            ..Default::default()
        });
        app.tabs[0].grid = grid.clone();
        app.tabs[0].results = vec![grid];
        frame(&ctx, &mut app, 760.0, 10.0);
        assert_eq!(app.tabs[0].result_pages.offset, 0);
        assert_eq!(app.grid().rows[0][0], Value::Integer(42));
    }
    #[test]
    fn wheel_scroll_crosses_cache_boundary_without_discarding_inspector_selection() {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx);
        wait_page(&ctx, &mut app, 760.0, 0);
        app.selected = app.row_selection.click(1, false, false, None);
        app.tabs[0]
            .result_pages
            .pin_selection(app.selected, &app.row_selection.rows);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(760.0, 760.0),
                )),
                events: vec![
                    egui::Event::PointerMoved(egui::pos2(400.0, 600.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -16000.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ctx| app.workspace(ctx),
        );
        let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
        let start = std::time::Instant::now();
        while app.tabs[0].result_pages.offset < 256 || app.tabs[0].result_pages.is_loading() {
            frame(
                &ctx,
                &mut app,
                760.0,
                ctx.input(|input| input.time) + 1.0 / 60.0,
            );
            assert!(
                start.elapsed().as_secs() < 5,
                "wheel scrolling did not load a later window"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(app.selected, Some(1));
        assert_eq!(
            app.grid().rows[1][0],
            Value::Integer(1),
            "inspector selection changed while scrolling"
        );
        assert!(app.tabs[0].result_pages.selection_outside_viewport());
    }

    #[test]
    fn end_key_reaches_final_result_and_select_all_is_logical() {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx);
        wait_page(&ctx, &mut app, 760.0, 0);
        app.selected = app.row_selection.click(0, false, false, None);
        app.tabs[0]
            .result_pages
            .pin_selection(app.selected, &app.row_selection.rows);
        ctx.memory_mut(|memory| memory.request_focus(app.tabs[0].result_grid_focus.unwrap()));
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(760.0, 760.0),
                )),
                events: vec![egui::Event::Key {
                    key: egui::Key::End,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ctx| app.workspace(ctx),
        );
        wait_page(&ctx, &mut app, 760.0, 10020);
        assert_eq!(
            app.grid().rows[app.selected.unwrap()][0],
            Value::Integer(10020)
        );
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }],
                modifiers: egui::Modifiers::COMMAND,
                ..Default::default()
            },
            |ctx| app.select_all_rows_shortcut(ctx, Some(0)),
        );
        assert!(app.tabs[0].result_pages.all_selected);
        assert!(app.row_selection.rows.len() <= 256);
        assert_eq!(app.tabs[0].grid.row_count(), 10021);
    }

    #[test]
    fn cache_miss_paints_visible_row_wide_skeletons() {
        fn skeleton(shape: &egui::Shape, clip: egui::Rect) -> bool {
            match shape {
                egui::Shape::Rect(rect) => rect.rect.intersect(clip).width() > 500.0
                    && (rect.rect.height() - 6.0).abs() < 0.1 && rect.fill.a() > 0,
                egui::Shape::Vec(shapes) => shapes.iter().any(|shape| skeleton(shape, clip)),
                _ => false,
            }
        }
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx);
        wait_page(&ctx, &mut app, 760.0, 0);
        app.tabs[0].result_pages.offset = 50000;
        let output = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(760.0, 760.0))),
            ..Default::default()
        }, |ctx| app.workspace(ctx));
        assert!(output.shapes.iter().any(|shape| skeleton(&shape.shape, shape.clip_rect)),
            "missing rows must have a visible placeholder spanning the row");
    }

    #[test]
    fn first_loaded_page_sizes_columns_from_values() {
        fn contains(shape: &egui::Shape, needle: &str) -> bool {
            match shape {
                egui::Shape::Text(text) => text.galley.text().contains(needle),
                egui::Shape::Vec(shapes) => shapes.iter().any(|shape| contains(shape, needle)),
                _ => false,
            }
        }
        for width in [500.0, 1280.0] {
            let ctx = egui::Context::default();
            let mut app = fixture(&ctx);
            wait_page(&ctx, &mut app, width, 0);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 760.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.workspace(ctx),
            );
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|shape| contains(&shape.shape, "Customer 0")),
                "first page kept header-only column widths"
            );
        }
    }
    #[test]
    fn closing_last_tab_releases_its_result_snapshot() {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx);
        wait_page(&ctx, &mut app, 760.0, 0);
        let snapshot = Arc::downgrade(app.tabs[0].grid.stored.as_ref().unwrap());
        app.close_tab(0);
        // Closing cancels prefetch without blocking the UI on its worker. The
        // worker may still own the snapshot until it observes cancellation.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while snapshot.strong_count() != 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(snapshot.upgrade().is_none(), "cancelled prefetch retained the closed result");
    }
    #[test]
    fn rapid_search_navigation_coalesces_to_latest_row() {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx);
        wait_page(&ctx, &mut app, 760.0, 0);
        let tab = &mut app.tabs[0];
        tab.result_pages.select(&tab.grid, 8000, 1, &ctx);
        tab.result_pages.select(&tab.grid, 9000, 1, &ctx);
        tab.result_pages.select(&tab.grid, 10, 1, &ctx);
        wait_page(&ctx, &mut app, 760.0, 10);
        assert_eq!(app.grid().rows[0][0], Value::Integer(10));
        assert_eq!(app.selected, Some(0));
    }
    #[test]
    #[ignore = "manual release-mode disk-backed result UI benchmark"]
    fn stored_results_frame_benchmark() {
        for (width, columns) in [(500.0, 3), (760.0, 3), (1280.0, 3), (1280.0, 120)] {
            let ctx = egui::Context::default();
            let mut app = fixture(&ctx);
            if columns == 120 {
                let grid = std::thread::spawn(move || {
                    let mut collector = db::result_store::Collector::new(1024);
                    for row in 0..10021 {
                        collector
                            .push(
                                (0..columns)
                                    .map(|column| {
                                        Value::Integer(if column == 0 {
                                            row
                                        } else {
                                            column as i64
                                        })
                                    })
                                    .collect(),
                            )
                            .unwrap();
                    }
                    let mut grid = Grid {
                        columns: (0..columns)
                            .map(|column| format!("Column {column}"))
                            .collect(),
                        ..Default::default()
                    };
                    collector.finish(&mut grid).unwrap();
                    Arc::new(grid)
                })
                .join()
                .unwrap();
                app.tabs[0].grid = grid.clone();
                app.tabs[0].results = vec![grid];
            }
            wait_page(&ctx, &mut app, width, 0);
            let mut samples = Vec::new();
            for index in 0..180 {
                let started = std::time::Instant::now();
                if index == 100 {
                    let tab = &mut app.tabs[0];
                    tab.result_pages.select(&tab.grid, 9000, 1, &ctx);
                }
                let events = if index % 4 == 0 {
                    vec![
                        egui::Event::PointerMoved(egui::pos2(width / 2.0, 600.0)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, if index < 100 { -240.0 } else { 240.0 }),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                } else {
                    vec![]
                };
                let time = (index as f64 / 120.0 + if index >= 100 { 30.0 } else { 0.0 })
                    .max(ctx.input(|input| input.time) + 1.0 / 120.0);
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 760.0),
                        )),
                        events,
                        time: Some(time),
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
            crate::ui_benchmark::report(
                &format!("query/continuous-results/{columns}/{width}"),
                &samples,
            );
        }
    }
}
