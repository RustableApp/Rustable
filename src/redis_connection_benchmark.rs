// Identical standalone-profile workload injected into the comparison baseline.
#[test]
#[ignore = "manual Redis connection profile CPU comparison"]
fn redis_connection_profile_frame_benchmark() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.connection_action(Action::NewRedis).unwrap();
    app.connections = true;
    app.child_window = false;
    let mut samples = Vec::new();
    for frame in 0..180 {
        if let ConnectionKind::Redis(settings) =
            &mut app.connection_manager.draft.as_mut().unwrap().connection
        {
            settings.host = format!("redis-{}.example.test", frame % 10);
        }
        let start = std::time::Instant::now();
        let output = manager_frame(&ctx, &mut app, vec![]);
        if frame == 10 {
            assert!(painted_text(&output, "Redis Connection").is_some());
            assert!(painted_text(&output, "Host").is_some());
        }
        let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
        if frame >= 20 {
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    samples.sort_by(f64::total_cmp);
    crate::ui_benchmark::report("redis/connection-profile", &samples);
}

#[test]
#[ignore = "manual Sentinel connection form CPU measurements"]
fn redis_sentinel_profile_frame_benchmark() {
    for count in [1, 8] {
        let ctx = egui::Context::default();
        let mut app = app();
        app.connection_action(Action::NewRedis).unwrap();
        app.connections = true;
        app.child_window = false;
        if let ConnectionKind::Redis(settings) = &mut app.connection_manager.draft.as_mut().unwrap().connection {
            settings.sentinel = Some(crate::redis_settings::Sentinel { seeds: (0..count).map(|index| crate::redis_settings::SentinelSeed { host: format!("sentinel-{index}.example.test"), port: 26379 }).collect(), ..Default::default() });
        }
        let mut samples = Vec::new();
        for frame in 0..180 {
            let start = std::time::Instant::now();
            let output = manager_frame(&ctx, &mut app, vec![
                egui::Event::PointerMoved(egui::pos2(1000.0, 500.0)),
                egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, if frame % 40 < 20 { -20.0 } else { 20.0 }), modifiers: egui::Modifiers::NONE },
            ]);
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 20 { samples.push(start.elapsed().as_secs_f64() * 1000.0); }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report(&format!("redis/sentinel-profile/{count}-seeds-scroll"), &samples);
    }
}

#[test]
#[ignore = "manual Cluster seed form CPU measurements"]
fn redis_cluster_profile_frame_benchmark() {
    for count in [0, 7] {
        let ctx = egui::Context::default();
        let mut app = app();
        app.connection_action(Action::NewRedis).unwrap();
        app.connections = true;
        app.child_window = false;
        if let ConnectionKind::Redis(settings) = &mut app.connection_manager.draft.as_mut().unwrap().connection {
            settings.cluster = true;
            settings.cluster_seeds = (0..count).map(|index| crate::redis_settings::SentinelSeed { host: format!("seed-{index}.example.test"), port: 6379 }).collect();
        }
        let mut samples = Vec::new();
        for frame in 0..180 {
            let start = std::time::Instant::now();
            let output = manager_frame(&ctx, &mut app, vec![
                egui::Event::PointerMoved(egui::pos2(1000.0, 500.0)),
                egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, if frame % 40 < 20 { -20.0 } else { 20.0 }), modifiers: egui::Modifiers::NONE },
            ]);
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 20 { samples.push(start.elapsed().as_secs_f64() * 1000.0); }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report(&format!("redis/cluster-profile/{count}-alternates-scroll"), &samples);
    }
}

#[test]
#[ignore = "manual expanded Redis security form CPU measurements"]
fn redis_security_profile_frame_benchmark() {
    for (width, height) in [(1000.0, 760.0), (1400.0, 1080.0)] {
        let ctx = egui::Context::default();
        let mut app = app();
        app.connection_action(Action::NewRedis).unwrap();
        app.connections = true;
        app.child_window = false;
        let ConnectionKind::Redis(settings) = &mut app.connection_manager.draft.as_mut().unwrap().connection else { unreachable!() };
        settings.client_certificate = Some("/certificates/client.pem".into());
        settings.client_key = Some("/certificates/client-key.pem".into());
        settings.ssh = Some(SshOptions { host: "gateway.example.test".into(), ..Default::default() });
        let mut samples = Vec::new();
        for frame in 0..180 {
            let start = std::time::Instant::now();
            let output = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height))),
                time: Some(frame as f64 / 120.0 + if frame >= 100 { 30.0 } else { 0.0 }),
                events: vec![
                    egui::Event::PointerMoved(egui::pos2(width - 200.0, 500.0)),
                    egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, if frame % 40 < 20 { -25.0 } else { 25.0 }), modifiers: egui::Modifiers::NONE },
                ],
                ..Default::default()
            }, |ctx| app.connections_window(ctx));
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 20 { samples.push(start.elapsed().as_secs_f64() * 1000.0); }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report(&format!("redis/security-profile/{width}x{height}"), &samples);
    }
}
