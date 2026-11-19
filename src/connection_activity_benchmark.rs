// Shared with the base revision by scripts/ui-performance.py.
#[test]
#[ignore]
fn connection_activity_frame_benchmark() {
    for width in [760.0, 1280.0] {
        for dark in [true, false] {
            for busy in [false, true] {
                let ctx = egui::Context::default();
                apply_theme(&ctx, if dark { Appearance::Dark } else { Appearance::Light });
                let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
                app.connections = false;
                app.engine = "PostgreSQL".into();
                app.label = "Production analytics".into();
                app.view = View::Query;
                #[cfg(unix)]
                app.offer_matrix();
                if busy { app.pending = Some(Pending::Query(0)); }
                let mut samples = Vec::new();
                for frame in 0..180 {
                    let time = frame as f64 / 60.0 + if frame >= 100 { 30.0 } else { 0.0 };
                    app.started = Instant::now() - std::time::Duration::from_secs_f64(time);
                    let start = Instant::now();
                    let output = ctx.run(egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 760.0))),
                        time: Some(time), ..Default::default()
                    }, |ctx| app.toolbar(ctx));
                    let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
                    let ms = start.elapsed().as_secs_f64()*1000.0;
                    if frame == 0 || frame == 100 { eprintln!("Activity {width}/{dark}/{busy} frame {frame}: {ms:.6}ms"); }
                    if frame >= 20 { samples.push(ms); }
                }
                samples.sort_by(f64::total_cmp);
                crate::ui_benchmark::report(&format!("connection/activity/{width}/{dark}/{busy}"), &samples);
            }
        }
    }
}

// Same real SQLite transaction on base and candidate; setup is not timed.
#[test]
#[ignore]
fn transaction_toolbar_frame_benchmark() {
    for width in [500.0, 760.0, 1280.0] {
        for dark in [true, false] {
            for busy in [false, true] {
                let ctx = egui::Context::default();
                apply_theme(&ctx, if dark { Appearance::Dark } else { Appearance::Light });
                let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
                app.worker.send(Command::Open(None)).unwrap();
                app.worker.rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap().unwrap();
                app.worker.send(Command::Query { sql: "BEGIN".into(), safe: false }).unwrap();
                app.worker.rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap().unwrap();
                assert_eq!(app.worker.transaction_state(), db::TransactionState::Active);
                app.connections = false;
                app.engine = "SQLite".into();
                app.label = "Transaction fixture".into();
                app.view = View::Query;
                #[cfg(unix)]
                app.offer_matrix();
                if busy { app.pending = Some(Pending::Query(0)); }
                let mut samples = Vec::new();
                for frame in 0..180 {
                    let time = frame as f64 / 60.0 + if frame >= 100 { 30.0 } else { 0.0 };
                    app.started = Instant::now() - std::time::Duration::from_secs_f64(time);
                    let start = Instant::now();
                    let output = ctx.run(egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 760.0))),
                        time: Some(time), ..Default::default()
                    }, |ctx| app.toolbar(ctx));
                    let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
                    let ms = start.elapsed().as_secs_f64() * 1000.0;
                    if frame == 0 || frame == 100 { eprintln!("Transaction {width}/{dark}/{busy} frame {frame}: {ms:.6}ms"); }
                    if frame >= 20 { samples.push(ms); }
                }
                samples.sort_by(f64::total_cmp);
                crate::ui_benchmark::report(&format!("connection/transaction/{width}/{dark}/{busy}"), &samples);
            }
        }
    }
}
