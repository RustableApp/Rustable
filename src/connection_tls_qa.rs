//! Shared production-layout fixture for regression checks and native captures.
use super::*;

fn fixture(ctx: &egui::Context) -> Rustable {
    let mut app = Rustable::with_optional_connection(ctx, Preferences::default(), None, None);
    app.connections = true;
    let profile = crate::connection_profiles::PostgresProfile {
        host: "database.example.test".into(),
        tls: crate::connection::PostgresTlsMode::Require,
        ..Default::default()
    };
    app.connection_manager.draft = Some(crate::connection_profiles::Profile::draft(
        "Private PostgreSQL".into(),
        crate::connection_profiles::ConnectionKind::Postgres(profile),
    ));
    if let Ok(kind) = std::env::var("RUSTABLE_TLS_REDIS") {
        let mut settings = crate::redis_settings::Settings { host: "cache.example.test".into(), verify_tls: std::env::var_os("RUSTABLE_TLS_UNVERIFIED").is_none(), ..Default::default() };
        if kind == "sentinel" { settings.sentinel = Some(Default::default()); }
        if kind == "cluster" { settings.cluster = true; }
        if std::env::var_os("RUSTABLE_TLS_CERTIFICATES").is_some() {
            settings.root_certificate = Some("/certificates/root-ca.pem".into());
            settings.client_certificate = Some("/certificates/client.pem".into());
            settings.client_key = Some("/certificates/client-key.pem".into());
            if let Some(sentinel) = &mut settings.sentinel {
                sentinel.root_certificate = Some("/certificates/sentinel-ca.pem".into());
                sentinel.client_certificate = Some("/certificates/sentinel-client.pem".into());
                sentinel.client_key = Some("/certificates/sentinel-key.pem".into());
            }
        }
        if kind == "forward" { settings.port_forwards = vec![crate::redis_settings::PortForward { host: settings.host.clone(), port: 6379, local_port: 16379 }]; }
        app.connection_manager.draft = Some(crate::connection_profiles::Profile::draft("Forwarded cache".into(), ConnectionKind::Redis(Box::new(settings))));
    }
    if let Ok(engine) = std::env::var("RUSTABLE_TLS_MYSQL") {
        let config = crate::connection_profiles::MysqlProfile { root_certificate: Some("/certificates/root-ca.pem".into()), ..Default::default() };
        let connection = if engine == "mariadb" { ConnectionKind::MariaDb(config) } else { ConnectionKind::Mysql(config) };
        app.connection_manager.draft = Some(Profile::draft("Database preview".into(), connection));
    }
    #[cfg(feature = "qa")]
    if std::env::var_os("RUSTABLE_CLICK_INTERACTIVE").is_some() {
        for name in ["Click QA first", "Click QA second"] {
            let id = app.prefs.saved_connections.add_profile(Profile::draft(name.into(),
                ConnectionKind::SQLite { path: std::path::PathBuf::from(":memory:") })).unwrap();
            app.connection_manager.select(app.prefs.saved_connections.profile(id).unwrap().clone());
        }
    }
    app
}

#[cfg(feature = "qa")]
impl Rustable {
    pub fn run_connection_tls_preview() -> eframe::Result {
        struct Preview {
            app: Rustable,
            interactive: bool,
            native_count: u32,
            frame: usize,
            menu: bool,
            scroll: bool,
            started: std::time::Instant,
            requested: bool,
            path: std::path::PathBuf,
            saved: Option<std::sync::mpsc::Receiver<()>>,
        }
        impl eframe::App for Preview {
            fn persist_egui_memory(&self) -> bool { false }
            fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
                for event in &input.events {
                    if let egui::Event::PointerClickCount(count) = event { self.native_count = *count; }
                }
                if self.scroll && (2..6).contains(&self.frame) {
                    input.events.push(egui::Event::PointerMoved(egui::pos2(800.0, 500.0)));
                    input.events.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, -140.0), modifiers: egui::Modifiers::NONE });
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
                        std::fs::write(path, data).expect("write native TLS screenshot");
                        let _ = tx.send(());
                    });
                }
                if self.menu && self.frame == 2 {
                    ctx.data_mut(|data| data.insert_temp(egui::Id::new("tls-preview-open"), true));
                }
                self.app.connections_window(ctx);
                if !self.interactive && !self.requested
                    && self.frame >= 12
                    && (!self.menu
                        || self.started.elapsed() >= std::time::Duration::from_millis(600))
                {
                    self.requested = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
                self.frame += 1;
                if self.interactive {
                    egui::Window::new("Native input QA").anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0]).show(ctx, |ui| {
                        ui.label(format!("Native click count: {}", self.native_count));
                        ui.label(format!("Connection requests: {}", self.app.window_requests.len()));
                        ui.label(format!("SQL selection: {:?}", self.app.tabs[0].cursor));
                    });
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
            }
        }
        let path = std::path::PathBuf::from(std::env::var_os("RUSTABLE_TLS_CAPTURE").unwrap());
        let light = std::env::var_os("RUSTABLE_TLS_LIGHT").is_some();
        let width = std::env::var("RUSTABLE_TLS_WIDTH")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(1280.0);
        eframe::run_native(
            "Connection TLS preview",
            eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default().with_inner_size([width, 760.0]),
                persist_window: false,
                persistence_path: Some(path.with_extension(format!("state-{}", std::process::id()))),
                ..Default::default()
            },
            Box::new(move |cc| {
                crate::fonts::install(&cc.egui_ctx);
                let mut app = fixture(&cc.egui_ctx);
                app.prefs.appearance = if std::env::var_os("RUSTABLE_QUERY_SOLARIZED").is_some() {
                    if light { Appearance::SolarizedLight } else { Appearance::SolarizedDark }
                } else if light {
                    Appearance::Light
                } else {
                    Appearance::Dark
                };
                apply_theme(&cc.egui_ctx, app.prefs.appearance);
                Ok(Box::new(Preview {
                    app,
                    interactive: std::env::var_os("RUSTABLE_CLICK_INTERACTIVE").is_some(),
                    native_count: 0,
                    frame: 0,
                    started: std::time::Instant::now(),
                    requested: false,
                    menu: std::env::var_os("RUSTABLE_TLS_MENU").is_some(),
                    scroll: std::env::var_os("RUSTABLE_TLS_SCROLL").is_some(),
                    path,
                    saved: None,
                }))
            }),
        )
    }
}
