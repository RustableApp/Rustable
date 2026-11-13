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
    app
}

#[cfg(feature = "qa")]
impl Rustable {
    pub fn run_connection_tls_preview() -> eframe::Result {
        struct Preview {
            app: Rustable,
            frame: usize,
            menu: bool,
            started: std::time::Instant,
            requested: bool,
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
                        std::fs::write(path, data).expect("write native TLS screenshot");
                        let _ = tx.send(());
                    });
                }
                if self.menu && self.frame == 2 {
                    ctx.data_mut(|data| data.insert_temp(egui::Id::new("tls-preview-open"), true));
                }
                self.app.connections_window(ctx);
                if !self.requested
                    && self.frame >= 3
                    && (!self.menu
                        || self.started.elapsed() >= std::time::Duration::from_millis(600))
                {
                    self.requested = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
                self.frame += 1;
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
        let path = std::path::PathBuf::from(std::env::var_os("RUSTABLE_TLS_CAPTURE").unwrap());
        let light = std::env::var_os("RUSTABLE_TLS_LIGHT").is_some();
        let width = std::env::var("RUSTABLE_TLS_WIDTH")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(1280.0);
        eframe::run_native(
            "PostgreSQL TLS preview",
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
                Ok(Box::new(Preview {
                    app,
                    frame: 0,
                    started: std::time::Instant::now(),
                    requested: false,
                    menu: std::env::var_os("RUSTABLE_TLS_MENU").is_some(),
                    path,
                    saved: None,
                }))
            }),
        )
    }
}
