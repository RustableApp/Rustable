// This exact fixture is also compiled into the base worktree by --redis.
#[cfg(test)]
mod redis_browser_benchmark {
    use super::*;
    #[test]
    #[ignore = "manual Redis browser CPU comparison"]
    fn redis_key_grid_frame_benchmark() {
        let ctx = egui::Context::default();
        crate::scroll_response::install(&ctx);
        let mut workspace = Workspace::new(
            Settings {
                host: String::new(),
                ..Default::default()
            },
            String::new(),
            ctx.clone(),
        );
        // Validation fails before any network access; wait outside measurement.
        assert!(workspace.receiver.recv().unwrap().is_err());
        workspace.busy = false;
        workspace.keys = (0..10000)
            .map(|i| format!("app:users:{i:05}").into_bytes())
            .collect();
        for key in &workspace.keys {
            workspace
                .rows
                .insert(key.clone(), ("Sample value".into(), "string".into(), -1));
        }
        let mut samples = Vec::new();
        for frame in 0..180 {
            workspace.selected_key = Some(workspace.keys[frame % 30].clone());
            let start = std::time::Instant::now();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 760.0),
                    )),
                    time: Some(frame as f64 / 120.0),
                    events: if frame % 3 == 0 {
                        vec![
                            egui::Event::PointerMoved(egui::pos2(600.0, 400.0)),
                            egui::Event::MouseWheel {
                                unit: egui::MouseWheelUnit::Point,
                                delta: egui::vec2(0.0, -18.0),
                                modifiers: Default::default(),
                            },
                        ]
                    } else {
                        vec![]
                    },
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| workspace.key_grid(ui));
                },
            );
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/grid", &samples);
    }
}

#[cfg(test)]
mod redis_inspector_benchmark {
    use super::*;
    #[test]
    #[ignore = "manual Redis inspector CPU comparison"]
    fn redis_hash_inspector_frame_benchmark() {
        let ctx = egui::Context::default();
        let mut workspace = Workspace::new(
            Settings {
                host: String::new(),
                ..Default::default()
            },
            String::new(),
            ctx.clone(),
        );
        assert!(workspace.receiver.recv().unwrap().is_err());
        workspace.busy = false;
        let snapshot = Snapshot {
            key: b"app:users:1001".to_vec(),
            kind: "hash".into(),
            ttl_ms: 3_600_000,
            length: Some(200),
            truncated: false,
            value: redis::Value::Array(
                (0..200)
                    .flat_map(|i| {
                        [
                            redis::Value::BulkString(format!("field_{i:03}").into_bytes()),
                            redis::Value::BulkString(format!("Sample value {i}").into_bytes()),
                        ]
                    })
                    .collect(),
            ),
        };
        workspace.preview = render_inspection(&snapshot);
        workspace.snapshot = Some(snapshot);
        let mut samples = Vec::new();
        for frame in 0..180 {
            let start = std::time::Instant::now();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 760.0),
                    )),
                    time: Some(frame as f64 / 120.0),
                    ..Default::default()
                },
                |ctx| {
                    egui::SidePanel::right("benchmark-inspector")
                        .exact_width(360.0)
                        .show(ctx, |ui| {
                            browser_style(ui);
                            egui::ScrollArea::vertical().show(ui, |ui| workspace.inspector(ui));
                        });
                },
            );
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/hash-inspector", &samples);
    }
}

#[cfg(test)]
mod redis_string_inspector_benchmark {
    use super::*;
    #[test]
    #[ignore = "manual Redis large string CPU comparison"]
    fn redis_string_inspector_frame_benchmark() {
        let ctx = egui::Context::default();
        let mut workspace = Workspace::new(
            Settings {
                host: String::new(),
                ..Default::default()
            },
            String::new(),
            ctx.clone(),
        );
        assert!(workspace.receiver.recv().unwrap().is_err());
        workspace.busy = false;
        let bytes = format!("{{\"message\":\"{}\"}}", "member ".repeat(8000)).into_bytes();
        workspace.value_editor = crate::redis_commands::quote(&bytes);
        let snapshot = Snapshot {
            key: b"cache:json".to_vec(),
            kind: "string".into(),
            ttl_ms: -1,
            length: Some(bytes.len() as u64),
            truncated: false,
            value: redis::Value::BulkString(bytes),
        };
        workspace.preview = render_inspection(&snapshot);
        workspace.snapshot = Some(snapshot);
        let mut samples = Vec::new();
        for frame in 0..180 {
            let start = std::time::Instant::now();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 760.0),
                    )),
                    time: Some(frame as f64 / 120.0),
                    ..Default::default()
                },
                |ctx| {
                    egui::SidePanel::right("benchmark-string-inspector")
                        .exact_width(360.0)
                        .show(ctx, |ui| {
                            browser_style(ui);
                            egui::ScrollArea::vertical().show(ui, |ui| workspace.inspector(ui));
                        });
                },
            );
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/string-inspector", &samples);
    }
}

#[cfg(test)]
mod redis_browser_chrome_benchmark {
    use super::*;
    #[test]
    #[ignore = "manual Redis toolbar and browser CPU comparison"]
    fn redis_browser_chrome_frame_benchmark() {
        let ctx = egui::Context::default();
        let mut workspace = Workspace::new(
            Settings {
                host: String::new(),
                ..Default::default()
            },
            String::new(),
            ctx.clone(),
        );
        assert!(workspace.receiver.recv().unwrap().is_err());
        workspace.busy = false;
        workspace.connected = true;
        workspace.scan_started = true;
        workspace.keys = (0..10000)
            .map(|i| format!("app:users:{i:05}").into_bytes())
            .collect();
        for key in &workspace.keys {
            workspace
                .rows
                .insert(key.clone(), ("Sample value".into(), "string".into(), -1));
        }
        workspace.expanded.insert(b"app:".to_vec());
        workspace.expanded.insert(b"app:users:".to_vec());
        workspace.rebuild_tree();
        let mut panes = crate::pane_sizes::PaneSizes::default();
        let mut dirty = false;
        let mut samples = Vec::new();
        for frame in 0..180 {
            let start = std::time::Instant::now();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 760.0),
                    )),
                    time: Some(frame as f64 / 120.0),
                    ..Default::default()
                },
                |ctx| {
                    workspace.show(
                        ctx,
                        "Disposable fixture",
                        egui::Color32::GRAY,
                        &mut panes,
                        &mut dirty,
                    );
                },
            );
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/browser-chrome", &samples);
    }
}
#[cfg(test)]
mod redis_vector_inspector_benchmark {
    use super::*;
    #[test]
    #[ignore = "manual Redis inspector CPU comparison"]
    fn redis_vector_inspector_frame_benchmark() {
        let ctx = egui::Context::default();
        let mut workspace = Workspace::new(
            Settings {
                host: String::new(),
                ..Default::default()
            },
            String::new(),
            ctx.clone(),
        );
        assert!(workspace.receiver.recv().unwrap().is_err());
        workspace.busy = false;
        let snapshot = Snapshot {
            key: b"app:users:1001".to_vec(),
            kind: "vectorset".into(),
            ttl_ms: 3_600_000,
            length: Some(200),
            truncated: false,
            value: redis::Value::Array(
                (0..200)
                    .map(|i| redis::Value::BulkString(format!("product:{i:05}:description").into_bytes()))
                    .collect(),
            ),
        };
        workspace.preview = render_inspection(&snapshot);
        workspace.snapshot = Some(snapshot);
        let mut samples = Vec::new();
        for frame in 0..180 {
            let start = std::time::Instant::now();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 760.0),
                    )),
                    time: Some(frame as f64 / 120.0),
                    events: vec![egui::Event::PointerMoved(egui::pos2(1050.0, 650.0)), egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, if frame % 60 < 30 { -18.0 } else { 18.0 }), modifiers: egui::Modifiers::NONE }],
                    ..Default::default()
                },
                |ctx| {
                    egui::SidePanel::right("benchmark-inspector")
                        .exact_width(360.0)
                        .show(ctx, |ui| {
                            browser_style(ui);
                            egui::ScrollArea::vertical().show(ui, |ui| workspace.inspector(ui));
                        });
                },
            );
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/vector-inspector", &samples);
    }
}


#[cfg(test)]
mod redis_command_pane_benchmark {
    use super::*;
    #[test]
    #[ignore = "manual command editor CPU comparison"]
    fn redis_command_pane_frame_benchmark() {
        for count in [20, 200] {
            for editing in [false, true] {
                let ctx = egui::Context::default();
                let mut workspace = Workspace::new(Settings {host:String::new(),..Default::default()},String::new(),ctx.clone());
                assert!(workspace.receiver.recv().unwrap().is_err());
                workspace.busy=false;
                workspace.connected=true;
                workspace.command=(0..count).map(|i|format!("GET \"app:users:{i:04}\"\n")).collect::<String>();
                workspace.command.push('0');
                workspace.output="1) PONG\n2) dark".into();
                let mut samples=Vec::new();
                for frame in 0..180 {
                    if editing {workspace.command.pop(); workspace.command.push(char::from(b'0'+(frame%10) as u8));}
                    let start=std::time::Instant::now();
                    let output=ctx.run(egui::RawInput {screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1000.0,440.0))),time:Some(frame as f64/120.0),..Default::default()},|ctx| {
                        egui::CentralPanel::default().show(ctx,|ui| {browser_style(ui);workspace.command_pane(ui);});
                    });
                    let _=ctx.tessellate(output.shapes,output.pixels_per_point);
                    if frame>=20 {samples.push(start.elapsed().as_secs_f64()*1000.0);}
                }
                samples.sort_by(f64::total_cmp);
                crate::ui_benchmark::report(&format!("redis/command-editor/{count}/{editing}"),&samples);
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod redis_tool_pages_benchmark {
    use super::*;
    #[test]
    #[ignore = "manual full-page tool CPU comparison"]
    fn redis_tool_pages_frame_benchmark() {
        for page in ["search", "analysis", "pubsub", "workbench-history", "json-edit"] {
            let ctx = egui::Context::default();
            let mut workspace = Workspace::new(Settings { host: String::new(), ..Default::default() }, String::new(), ctx.clone());
            assert!(workspace.receiver.recv().unwrap().is_err());
            workspace.busy = false; workspace.connected = true;
            workspace.scan_started = true; workspace.auto_scan = false;
            match page {
                "search" => {
                    workspace.search.receive(crate::redis_search::Reply::Indexes((0..200).map(|i|format!("idx:catalog:{i:04}").into_bytes()).collect()));
                    workspace.search.prepare_catalog_benchmark();
                    workspace.search.open = true;
                }
                "analysis" => {
                    workspace.diagnostics_open = true;
                    workspace.diagnostics_request = Some(crate::redis_diagnostics::Request::Keys(vec![]));
                    workspace.diagnostics_report = Some(Box::new(crate::redis_diagnostics::Report {
                        analysis: true,
                        summary: vec![("Keys sampled".into(), "1000".into()), ("Sampled memory".into(), "10 MiB".into())],
                        rows: (0..50).map(|i| [format!("app:users:{i:04}"), (10000-i*100).to_string(), format!("namespace:{}",i%4)]).collect(),
                        ..Default::default()
                    }));
                }
                "workbench-history" => {
                    workspace.section = Section::Workbench;
                    workspace.command_history = (0..100).map(|i| format!("GET app:users:{i:04}")).collect();
                }
                "json-edit" => {
                    workspace.section = Section::Browse;
                    workspace.inspector_visible = true;
                    workspace.selected_key = Some(b"fixture:json".to_vec());
                    let mut draft = crate::redis_edit::Draft::new(crate::redis_mutation::Action::Update, vec![b"fixture:json".to_vec()], "ReJSON-RL".into());
                    draft.value = serde_json::to_string_pretty(&serde_json::json!({"users": (0..30).map(|i| format!("User {i}")).collect::<Vec<_>>()})).unwrap();
                    workspace.snapshot = Some(Snapshot { key: b"fixture:json".to_vec(), kind: "ReJSON-RL".into(), ttl_ms: -1, length: Some(draft.value.len() as u64), truncated: false, value: redis::Value::BulkString(draft.value.as_bytes().to_vec()) });
                    workspace.edit = Some(draft);
                }
                _ => workspace.pubsub.open = true,
            }
            let mut sizes = crate::pane_sizes::PaneSizes::default();
            let mut dirty = false;
            let mut samples = Vec::new();
            for frame in 0..180 {
                let start = std::time::Instant::now();
                let output = ctx.run(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0,760.0))),
                    time: Some(frame as f64/120.0 + if frame >= 100 {30.0} else {0.0}),
                    ..Default::default()
                }, |ctx| { workspace.show(ctx, "Fixture", egui::Color32::BLUE, &mut sizes, &mut dirty); });
                let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
                if frame >= 20 { samples.push(start.elapsed().as_secs_f64()*1000.0); }
            }
            samples.sort_by(f64::total_cmp);
            crate::ui_benchmark::report(&format!("redis/page/{page}"), &samples);
        }
    }
}

#[cfg(test)]
mod redis_completion_ui_benchmark {
    use eframe::egui;
    #[test]
    #[ignore = "manual focused command completion UI comparison"]
    fn redis_completion_frame_benchmark() {
        for scenario in ["commands", "arguments", "after-idle"] {
            let ctx=egui::Context::default();
            let mut editor=crate::redis_command_editor::Editor::default();
            let mut text=if scenario=="arguments" {"SET app:theme dark N"} else {"HG"}.to_owned();
            let mut samples=Vec::new();
            for frame in 0..180 {
                let start=std::time::Instant::now();
                let output=ctx.run(egui::RawInput {
                    screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1200.0,760.0))),
                    time:Some(frame as f64/120.0+if scenario=="after-idle" && frame>=100 {30.0}else{0.0}),
                    events:if frame>0 && frame%2==0 {vec![egui::Event::Key{key:egui::Key::ArrowDown,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers::NONE}]}else{vec![]},
                    ..Default::default()
                },|ctx| {
                    egui::CentralPanel::default().show(ctx,|ui| {
                        let id=ui.make_persistent_id("redis-command-editor");ui.memory_mut(|m|m.request_focus(id));
                        if let Some(mut state)=egui::text_edit::TextEditState::load(ctx,id) {
                            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(text.chars().count()))));state.store(ctx,id);
                        }
                        editor.show(ui,&mut text,true);
                    });
                });
                let _=ctx.tessellate(output.shapes,output.pixels_per_point);
                if frame>=20 {samples.push(start.elapsed().as_secs_f64()*1000.0);}
            }
            samples.sort_by(f64::total_cmp);crate::ui_benchmark::report(&format!("redis/completion/{scenario}"),&samples);
        }
    }
}
