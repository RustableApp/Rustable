use super::*;

fn fixture(ctx: &egui::Context, count: usize) -> Rustable {
    let mut app = Rustable::with_optional_connection(ctx, Preferences::default(), None, None);
    app.pending = None;
    app.connections = false;
    app.engine = "PostgreSQL".into();
    app.safe = false;
    app.label = "Commerce · Local development".into();
    app.inspector = false;
    app.prefs.sidebar_hidden = false;
    app.prefs.activity_visible = false;
    app.objects = (0..count)
        .map(|index| {
            let name = [
                "customers",
                "orders",
                "payments",
                "products",
                "shipments",
                "subscriptions",
            ]
            .get(index)
            .map_or_else(|| format!("table_{index:04}"), |name| (*name).to_owned());
            Object {
                name: name.clone(),
                display_name: Some(name),
                schema: Some("public".into()),
                kind: "table".into(),
                columns: vec![],
                metadata_error: None,
                ddl: String::new(),
            }
        })
        .collect();
    app.browse.table = "customers".into();
    app.view = View::Data;
    app.table = Arc::new(Grid {
        columns: vec!["id".into(), "name".into(), "email".into(), "plan".into()],
        rows: [
            "Avery Chen",
            "Noah Williams",
            "Sofia Martinez",
            "Liam Anderson",
            "Isla Thompson",
        ]
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            vec![
                Value::Integer(i as i64 + 1),
                Value::Text(name.into()),
                Value::Text(format!(
                    "{}@example.com",
                    name.split_whitespace().next().unwrap().to_lowercase()
                )),
                Value::Text("Pro".into()),
            ]
        })
        .collect(),
        ..Default::default()
    });
    app.table_loaded = true;
    app.columns = app
        .table
        .columns
        .iter()
        .enumerate()
        .map(|(i, name)| Column {
            name: name.clone(),
            data_type: if i == 0 { "integer" } else { "text" }.into(),
            pk: i == 0,
            not_null: false,
            default: String::new(),
            generated: false,
        })
        .collect();
    app
}

fn pointer(
    pos: egui::Pos2,
    button: egui::PointerButton,
    modifiers: egui::Modifiers,
) -> Vec<egui::Event> {
    let mut events = vec![egui::Event::PointerMoved(pos)];
    events.extend([true, false].map(|pressed| egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers,
    }));
    events
}

#[cfg(feature = "qa")]
impl Rustable {
    pub fn run_table_export_preview() -> eframe::Result {
        struct Preview {
            app: Rustable,
            frame: usize,
            path: PathBuf,
            mode: String,
            saved: Option<std::sync::mpsc::Receiver<()>>,
            started: Instant,
            captured: bool,
        }
        impl eframe::App for Preview {
            fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
                let target = match (self.mode.as_str(), self.frame) {
                    ("single", 2) => Some((
                        "customers",
                        egui::PointerButton::Secondary,
                        egui::Modifiers::NONE,
                    )),
                    (_, 2) if self.mode != "single" => Some((
                        "orders",
                        egui::PointerButton::Primary,
                        egui::Modifiers::COMMAND,
                    )),
                    (_, 3) if self.mode != "single" => Some((
                        "payments",
                        egui::PointerButton::Primary,
                        egui::Modifiers::COMMAND,
                    )),
                    (_, 4) if self.mode != "single" => Some((
                        "orders",
                        egui::PointerButton::Secondary,
                        egui::Modifiers::NONE,
                    )),
                    _ => None,
                };
                if let Some((name, button, modifiers)) = target {
                    let rect = ctx
                        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("export-row", name))))
                        .unwrap();
                    input.modifiers = modifiers;
                    input
                        .events
                        .extend(pointer(rect.center(), button, modifiers));
                }
            }
            fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
                if self.saved.as_ref().is_some_and(|rx| rx.try_recv().is_ok()) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                if let Some(image) = ctx.input(|input| {
                    input.events.iter().find_map(|event| {
                        if let egui::Event::Screenshot { image, .. } = event {
                            Some(image.clone())
                        } else {
                            None
                        }
                    })
                }) {
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.saved = Some(rx);
                    let path = self.path.clone();
                    std::thread::spawn(move || {
                        let mut data =
                            format!("P6\n{} {}\n255\n", image.width(), image.height()).into_bytes();
                        for pixel in &image.pixels {
                            data.extend_from_slice(&pixel.to_array()[..3]);
                        }
                        std::fs::write(path, data).unwrap();
                        let _ = tx.send(());
                    });
                }
                if self.frame == 6 && matches!(self.mode.as_str(), "dialog" | "schema") {
                    egui::Popup::close_all(ctx);
                    self.app.begin_sidebar_export();
                    if self.mode == "schema" {
                        let dialog = self.app.sidebar_export.as_mut().unwrap();
                        dialog.contents = Contents::SchemaAndData;
                        dialog.format = db::ExportFormat::Sql;
                        dialog.update_files();
                    }
                }
                if self.frame == 6 && self.mode == "truncate" {
                    egui::Popup::close_all(ctx);
                    self.app.sidebar_table_action(Action::Truncate);
                }
                self.app.toolbar(ctx);
                self.app.sidebar(ctx);
                self.app.workspace(ctx);
                self.app.sidebar_export_window(ctx);
                self.app.destructive_confirmation(ctx);
                if self.frame >= 10 && !self.captured && self.started.elapsed().as_millis() >= 700 {
                    self.captured = true;
                    assert_eq!(
                        self.app.sidebar_selection.names.len(),
                        if self.mode == "single" { 1 } else { 3 }
                    );
                    if matches!(self.mode.as_str(), "dialog" | "schema") {
                        assert_eq!(self.app.sidebar_export.as_ref().unwrap().count, 3);
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
                self.frame += 1;
                ctx.request_repaint_after(std::time::Duration::from_millis(40));
            }
        }
        let path = PathBuf::from(std::env::var_os("RUSTABLE_TABLE_EXPORT_CAPTURE").unwrap());
        let mode = std::env::var("RUSTABLE_TABLE_EXPORT_MODE").unwrap_or_else(|_| "dialog".into());
        let light = std::env::var_os("RUSTABLE_TABLE_EXPORT_LIGHT").is_some();
        let width = std::env::var("RUSTABLE_TABLE_EXPORT_WIDTH")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(1120.0);
        eframe::run_native(
            "Table export preview",
            eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default().with_inner_size([width, 720.0]),
                persist_window: false,
                ..Default::default()
            },
            Box::new(move |cc| {
                crate::fonts::install(&cc.egui_ctx);
                cc.egui_ctx
                    .data_mut(|d| d.insert_temp(egui::Id::new("table-export-capture"), true));
                let mut app = fixture(&cc.egui_ctx, 6);
                app.prefs.appearance = if light {
                    Appearance::Light
                } else {
                    Appearance::Dark
                };
                apply_theme(&cc.egui_ctx, app.prefs.appearance);
                Ok(Box::new(Preview {
                    app,
                    frame: 0,
                    path,
                    mode,
                    saved: None,
                    started: Instant::now(),
                    captured: false,
                }))
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
        match shape {
            egui::Shape::Text(t) => out.push((t.galley.text().to_owned(), t.pos)),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    text(shape, out);
                }
            }
            _ => (),
        }
    }
    fn frame(
        app: &mut Rustable,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> Vec<(String, egui::Pos2)> {
        let output = ctx.run(
            egui::RawInput {
                events,
                modifiers,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 720.0),
                )),
                ..Default::default()
            },
            |ctx| {
                app.sidebar(ctx);
                app.sidebar_export_window(ctx);
            },
        );
        let mut texts = Vec::new();
        for shape in &output.shapes {
            text(&shape.shape, &mut texts);
        }
        texts
    }

    #[test]
    fn sidebar_context_exports_multiple_tables_without_loading_them() {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx, 6);
        frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        let texts = frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        let point =
            |name: &str| texts.iter().find(|(s, _)| s == name).unwrap().1 + egui::vec2(10.0, 7.0);
        for name in ["orders", "payments"] {
            frame(
                &mut app,
                &ctx,
                pointer(
                    point(name),
                    egui::PointerButton::Primary,
                    egui::Modifiers::COMMAND,
                ),
                egui::Modifiers::COMMAND,
            );
        }
        assert_eq!(app.sidebar_selection.names.len(), 3);
        assert!(app.pending.is_none());
        assert_eq!(app.browse.table, "customers");
        frame(
            &mut app,
            &ctx,
            pointer(
                point("orders"),
                egui::PointerButton::Secondary,
                egui::Modifiers::NONE,
            ),
            egui::Modifiers::NONE,
        );
        let menu = frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        assert!(!menu.iter().any(|(s, _)| s == "Select all tables"));
        assert!(menu.iter().any(|(s, _)| s == "Truncate 3 Tables…"));
        assert!(menu.iter().any(|(s, _)| s == "Drop 3 Tables…"));
        let export = menu
            .iter()
            .find(|(s, _)| s == "Export 3 tables…")
            .expect("bulk context action")
            .1
            + egui::vec2(8.0, 7.0);
        frame(
            &mut app,
            &ctx,
            pointer(export, egui::PointerButton::Primary, egui::Modifiers::NONE),
            egui::Modifiers::NONE,
        );
        frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        assert_eq!(app.sidebar_export.as_ref().unwrap().count, 3);
        assert!(app.pending.is_none());
    }

    #[test]
    fn first_command_click_from_query_does_not_select_a_hidden_previous_table() {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx, 6);
        app.view = View::Query;
        frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        let labels = frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
        let point = labels
            .iter()
            .find(|(label, _)| label == "orders")
            .unwrap()
            .1
            + egui::vec2(10.0, 7.0);
        frame(
            &mut app,
            &ctx,
            pointer(
                point,
                egui::PointerButton::Primary,
                egui::Modifiers::COMMAND,
            ),
            egui::Modifiers::COMMAND,
        );
        assert_eq!(app.sidebar_selection.names, ["orders".to_owned()].into());
        assert!(app.pending.is_none());
    }

    #[test]
    fn export_format_tabs_preserve_selection_and_valid_contents() {
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx, 6);
        app.sidebar_selection.names.insert("customers".into());
        app.begin_sidebar_export();
        let selected = app.sidebar_export.as_ref().unwrap().count;
        let click = |app: &mut Rustable, label: &str| {
            for _ in 0..8 {
                frame(app, &ctx, vec![], egui::Modifiers::NONE);
            }
            let labels = frame(app, &ctx, vec![], egui::Modifiers::NONE);
            let position = labels
                .iter()
                .filter(|(text, _)| text == label)
                .max_by(|a, b| a.1.x.total_cmp(&b.1.x))
                .unwrap()
                .1
                + egui::vec2(5.0, 7.0);
            frame(
                app,
                &ctx,
                pointer(
                    position,
                    egui::PointerButton::Primary,
                    egui::Modifiers::NONE,
                ),
                egui::Modifiers::NONE,
            );
        };
        click(&mut app, "SQL");
        click(&mut app, "Schema only");
        let dialog = app.sidebar_export.as_ref().unwrap();
        assert!(dialog.contents == Contents::Schema);
        assert!(dialog.preview.ends_with(".sql"));
        click(&mut app, "JSON");
        let dialog = app.sidebar_export.as_ref().unwrap();
        assert!(dialog.contents == Contents::Data);
        assert_eq!(dialog.count, selected);
        assert!(dialog.preview.ends_with(".json"));
        click(&mut app, "Include SQL schema");
        assert!(app.sidebar_export.as_ref().unwrap().contents == Contents::SchemaAndData);
        click(&mut app, "CSV");
        click(&mut app, "Field names in the first row");
        let db::ExportFormat::CsvCustom(options) =
            app.sidebar_export.as_ref().unwrap().effective_format()
        else {
            panic!("CSV settings were not forwarded");
        };
        assert!(!options.header);
        click(&mut app, "All");
        click(&mut app, "SQL");
        let dialog = app.sidebar_export.as_ref().unwrap();
        assert_eq!(dialog.count, 6);
        assert!(dialog.single_file());
        assert_eq!(dialog.preview, "tables.sql");
    }

    #[test]
    #[ignore = "manual new sidebar export interaction CPU workload"]
    fn sidebar_export_frame_benchmark() {
        export_benchmark(Contents::Data);
    }

    #[test]
    #[ignore = "manual schema export interaction CPU workload"]
    fn sidebar_schema_export_frame_benchmark() {
        export_benchmark(Contents::SchemaAndData);
    }

    #[test]
    #[ignore = "manual SQL export interaction CPU workload"]
    fn sidebar_sql_export_frame_benchmark() {
        export_benchmark_format(Contents::SchemaAndData, db::ExportFormat::Sql);
    }

    fn export_benchmark(contents: Contents) {
        export_benchmark_format(contents, db::ExportFormat::Csv);
    }

    #[test]
    #[ignore = "manual CSV settings interaction CPU workload"]
    fn sidebar_csv_options_frame_benchmark() {
        export_benchmark_interaction(Contents::Data, db::ExportFormat::Csv, true);
    }

    fn export_benchmark_format(contents: Contents, format: db::ExportFormat) {
        export_benchmark_interaction(contents, format, false);
    }

    fn export_benchmark_interaction(
        contents: Contents,
        format: db::ExportFormat,
        csv_settings: bool,
    ) {
        for count in [20, 2000] {
            for editing in [false, true] {
                let ctx = egui::Context::default();
                let mut app = fixture(&ctx, count);
                app.sidebar_selection.names = app.objects.iter().map(|o| o.name.clone()).collect();
                app.begin_sidebar_export();
                app.sidebar_export.as_mut().unwrap().contents = contents;
                app.sidebar_export.as_mut().unwrap().format = format;
                app.sidebar_export.as_mut().unwrap().update_files();
                frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
                // Allow the centered modal to settle before locating the click target.
                for _ in 0..8 {
                    frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
                }
                let labels = frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
                let checkbox = labels
                    .iter()
                    .filter(|(label, _)| {
                        label
                            == if csv_settings {
                                "Field names in the first row"
                            } else {
                                "customers"
                            }
                    })
                    .max_by(|a, b| a.1.x.total_cmp(&b.1.x))
                    .unwrap()
                    .1
                    + egui::vec2(5.0, 7.0);
                let mut samples = Vec::new();
                for index in 0..180 {
                    let events = if editing {
                        pointer(
                            checkbox,
                            egui::PointerButton::Primary,
                            egui::Modifiers::NONE,
                        )
                    } else {
                        vec![]
                    };
                    let started = std::time::Instant::now();
                    let output = ctx.run(
                        egui::RawInput {
                            events,
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(1000.0, 720.0),
                            )),
                            time: Some(
                                index as f64 / 120.0 + if index >= 100 { 30.0 } else { 0.0 },
                            ),
                            ..Default::default()
                        },
                        |ctx| {
                            app.sidebar(ctx);
                            app.sidebar_export_window(ctx);
                        },
                    );
                    let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
                    if index >= 20 {
                        samples.push(started.elapsed().as_secs_f64() * 1000.0);
                    }
                    if editing && csv_settings {
                        assert_eq!(
                            app.sidebar_export.as_ref().unwrap().csv_options.header,
                            index % 2 != 0
                        );
                        assert_eq!(app.sidebar_export.as_ref().unwrap().count, count);
                    } else if editing {
                        assert_eq!(
                            app.sidebar_export.as_ref().unwrap().count,
                            if index % 2 == 0 { count - 1 } else { count }
                        );
                    }
                }
                samples.sort_by(f64::total_cmp);
                crate::ui_benchmark::report(
                    &format!(
                        "sidebar/{}-review/{count}/{editing}",
                        if csv_settings {
                            "csv-options"
                        } else if contents == Contents::Data {
                            "export"
                        } else {
                            "schema"
                        }
                    ),
                    &samples,
                );
            }
        }
    }
    #[test]
    #[ignore = "manual bounded destructive table review CPU workload"]
    fn sidebar_action_review_frame_benchmark() {
        for count in [20, 2000] {
            let ctx = egui::Context::default();
            let mut app = fixture(&ctx, count);
            app.sidebar_selection.names = app
                .objects
                .iter()
                .map(|object| object.name.clone())
                .collect();
            app.sidebar_table_action(Action::Drop);
            assert!(app.pending.is_none());
            let mut samples = Vec::new();
            for index in 0..180 {
                let started = Instant::now();
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1000.0, 720.0),
                        )),
                        time: Some(index as f64 / 120.0 + if index >= 100 { 30.0 } else { 0.0 }),
                        ..Default::default()
                    },
                    |ctx| app.destructive_confirmation(ctx),
                );
                let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
                if index >= 20 {
                    samples.push(started.elapsed().as_secs_f64() * 1000.0);
                }
            }
            samples.sort_by(f64::total_cmp);
            crate::ui_benchmark::report(&format!("sidebar/action-review/{count}"), &samples);
        }
    }
    #[test]
    #[ignore = "manual Command-click sidebar selection CPU workload"]
    fn sidebar_multiselect_frame_benchmark() {
        for count in [20, 2000] {
            let ctx = egui::Context::default();
            let mut app = fixture(&ctx, count);
            app.view = View::Query;
            frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
            let labels = frame(&mut app, &ctx, vec![], egui::Modifiers::NONE);
            let points: Vec<_> = ["orders", "payments"]
                .iter()
                .map(|name| {
                    labels.iter().find(|(label, _)| label == name).unwrap().1 + egui::vec2(8.0, 7.0)
                })
                .collect();
            let mut samples = Vec::new();
            for index in 0..180 {
                let started = Instant::now();
                let output = ctx.run(
                    egui::RawInput {
                        modifiers: egui::Modifiers::COMMAND,
                        events: pointer(
                            points[index % 2],
                            egui::PointerButton::Primary,
                            egui::Modifiers::COMMAND,
                        ),
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1000.0, 720.0),
                        )),
                        time: Some(index as f64 / 120.0 + if index >= 100 { 30.0 } else { 0.0 }),
                        ..Default::default()
                    },
                    |ctx| app.sidebar(ctx),
                );
                let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                if index == 0 || index == 100 {
                    eprintln!("sidebar multiselect {count} interaction {index}: {elapsed:.6}ms");
                }
                if index >= 20 {
                    samples.push(elapsed);
                }
                assert!(!app.sidebar_selection.names.contains("customers"));
                assert!(app.pending.is_none());
            }
            samples.sort_by(f64::total_cmp);
            crate::ui_benchmark::report(&format!("sidebar/multiselect/{count}"), &samples);
        }
    }
}
