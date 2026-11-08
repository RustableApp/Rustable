//! Opt-in native Redis browser QA; only the disposable loopback fixture is used.
use crate::{pane_sizes::PaneSizes, redis_settings::Settings, redis_workspace::Workspace};
use eframe::egui;

pub(crate) fn run() -> eframe::Result {
    eframe::run_native(
        "Rustable Redis Preview",
        eframe::NativeOptions {
            persist_window: false,
            viewport: egui::ViewportBuilder::default().with_inner_size([std::env::var("RUSTABLE_REDIS_PREVIEW_WIDTH").ok().and_then(|s|s.parse::<f32>().ok()).unwrap_or(1200.0).clamp(700.0,1800.0), 760.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            crate::scroll_response::install(&cc.egui_ctx);
            crate::fonts::install(&cc.egui_ctx);
            crate::app::apply_preview_theme(&cc.egui_ctx);
            Ok(Box::new(Preview {
                workspace: Workspace::new(
                    Settings {
                        host: "127.0.0.1".into(),
                        cluster: std::env::var_os("RUSTABLE_REDIS_TOPOLOGY_CAPTURE").is_some(),
                        port: if let Ok(port) = std::env::var("RUSTABLE_REDIS_PREVIEW_PORT") { port.parse().expect("disposable preview port") } else if std::env::var_os("RUSTABLE_REDIS_FAILOVER_CAPTURE").is_some() {16603} else if std::env::var_os("RUSTABLE_REDIS_TOPOLOGY_CAPTURE").is_some() {16400} else {16379},
                        tls: false,
                        ..Default::default()
                    },
                    if std::env::var_os("RUSTABLE_REDIS_TOPOLOGY_CAPTURE").is_some() {"rustable-cluster-test-only"} else {"rustable-redis-test-only"}.into(),
                    cc.egui_ctx.clone(),
                ),
                sizes: PaneSizes { redis_commands: std::env::var_os("RUSTABLE_REDIS_COMMAND_CAPTURE").map(|_| 420.0), ..Default::default() },
                stage: 0,
                inspector_capture: std::env::var_os("RUSTABLE_REDIS_INSPECTOR_CAPTURE").is_some()
                    || std::env::var_os("RUSTABLE_REDIS_LAYOUT_CAPTURE").is_some()
                    || std::env::var_os("RUSTABLE_REDIS_ROADMAP_CAPTURE").is_some()
                    || std::env::var_os("RUSTABLE_REDIS_SEARCH_CAPTURE").is_some()
                    || std::env::var_os("RUSTABLE_REDIS_MONITOR_CAPTURE").is_some()
                    || std::env::var_os("RUSTABLE_REDIS_PUBSUB_CAPTURE").is_some()
                    || std::env::var_os("RUSTABLE_REDIS_ARCHIVE_CAPTURE").is_some()
                    || std::env::var_os("RUSTABLE_REDIS_TOPOLOGY_CAPTURE").is_some()
                    || std::env::var_os("RUSTABLE_REDIS_RECOVERY_CAPTURE").is_some()
                    || std::env::var_os("RUSTABLE_REDIS_TIMESERIES_CAPTURE").is_some()
                    || std::env::var_os("RUSTABLE_REDIS_VECTOR_CAPTURE").is_some()
                    || std::env::var_os("RUSTABLE_REDIS_COMMAND_CAPTURE").is_some(),
                layout_capture: std::env::var_os("RUSTABLE_REDIS_LAYOUT_CAPTURE").is_some(),
                command_capture: std::env::var_os("RUSTABLE_REDIS_COMMAND_CAPTURE").is_some(),
                vector_capture: std::env::var_os("RUSTABLE_REDIS_VECTOR_CAPTURE").is_some(),
                timeseries_capture: std::env::var_os("RUSTABLE_REDIS_TIMESERIES_CAPTURE").is_some(),
                recovery_capture: std::env::var_os("RUSTABLE_REDIS_RECOVERY_CAPTURE").is_some(),
                topology_capture: std::env::var_os("RUSTABLE_REDIS_TOPOLOGY_CAPTURE").is_some(),
                archive_capture: std::env::var_os("RUSTABLE_REDIS_ARCHIVE_CAPTURE").is_some(),
                pubsub_capture: std::env::var_os("RUSTABLE_REDIS_PUBSUB_CAPTURE").is_some(),
                monitor_capture: std::env::var_os("RUSTABLE_REDIS_MONITOR_CAPTURE").is_some(),
                search_capture: std::env::var_os("RUSTABLE_REDIS_SEARCH_CAPTURE").is_some(),
                roadmap_capture: std::env::var_os("RUSTABLE_REDIS_ROADMAP_CAPTURE").is_some(),
                frames: 0,
                ready_since: None,
                capture: std::env::var_os("RUSTABLE_REDIS_CAPTURE").map(std::path::PathBuf::from),
            }))
        }),
    )
}
struct Preview {
    workspace: Workspace,
    sizes: PaneSizes,
    stage: u8,
    inspector_capture: bool,
    roadmap_capture: bool,
    search_capture: bool,
    monitor_capture: bool,
    pubsub_capture: bool,
    archive_capture: bool,
    topology_capture: bool,
    recovery_capture: bool,
    layout_capture: bool,
    command_capture: bool,
    vector_capture: bool,
    timeseries_capture: bool,
    frames: usize,
    ready_since: Option<std::time::Instant>,
    capture: Option<std::path::PathBuf>,
}
impl eframe::App for Preview {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if let Some(directory) = &self.capture {
            for event in ctx.input(|i| i.events.clone()) {
                if let egui::Event::Screenshot { image, .. } = event {
                    let path = directory.join(if self.layout_capture { ["browse-tree.ppm", "browse-list.ppm", "new-key.ppm", "search-indexes.ppm", "search-query.ppm", "workbench.ppm", "analyze.ppm", "pubsub.ppm", "json-view.ppm", "json-inline.ppm"][(self.stage/3) as usize] } else if self.command_capture { "command-editor.ppm" } else if self.vector_capture { ["vector-elements.ppm", "vector-detail.ppm", "vector-similar.ppm"][(self.stage/3) as usize] } else if self.timeseries_capture { ["timeseries-all.ppm", "timeseries-range.ppm", "timeseries-info.ppm", "timeseries-chart.ppm"][(self.stage/3) as usize] } else if self.recovery_capture { ["recovery-draft.ppm", "recovery-restored.ppm"][(self.stage/3) as usize] } else if self.topology_capture && std::env::var_os("RUSTABLE_REDIS_BULK_CAPTURE").is_some() { ["bulk-review.ppm","bulk-results.ppm"][(self.stage/3) as usize] } else if self.topology_capture { ["cluster-topology.ppm","cluster-key.ppm"][(self.stage/3) as usize] } else if self.archive_capture {
                        [
                            "archive-export.ppm",
                            "archive-loaded.ppm",
                            "archive-review.ppm",
                            "archive-replace.ppm",
                            "archive-results.ppm",
                            "archive-readonly.ppm",
                        ][(self.stage / 3) as usize]
                    } else if self.pubsub_capture {
                        [
                            "pubsub-ready.ppm",
                            "pubsub-messages.ppm",
                            "pubsub-json.ppm",
                            "pubsub-channels.ppm",
                            "pubsub-publish.ppm",
                            "pubsub-stopped.ppm",
                        ][(self.stage / 3) as usize]
                    } else if self.monitor_capture {
                        [
                            "monitor-ready.ppm",
                            "monitor-live.ppm",
                            "monitor-stopped.ppm",
                        ][(self.stage / 3) as usize]
                    } else if self.search_capture
                        && std::env::var_os("RUSTABLE_REDIS_ANALYTICS_CAPTURE").is_some()
                    {
                        [
                            "indexes.ppm",
                            "schema.ppm",
                            "aggregate-count.ppm",
                            "aggregate-sum.ppm",
                            "profile-documents.ppm",
                            "profile-aggregate.ppm",
                        ][(self.stage / 3) as usize]
                    } else if self.search_capture && std::env::var_os("RUSTABLE_REDIS_INDEX_MANAGEMENT_CAPTURE").is_some() {
                        ["catalog.ppm", "metadata.ppm", "create-index.ppm", "drop-review.ppm", "schema.ppm", "query.ppm"][(self.stage / 3) as usize]
                    } else if self.search_capture {
                        [
                            "indexes.ppm",
                            "schema.ppm",
                            "results.ppm",
                            "empty.ppm",
                            "error.ppm",
                            "retry.ppm",
                        ][(self.stage / 3) as usize]
                    } else if self.roadmap_capture {
                        [
                            "create-key.ppm",
                            "edit-field.ppm",
                            "bulk-delete.ppm",
                            "server-diagnostics.ppm",
                            "memory-analysis.ppm",
                            "commands.ppm",
                        ][(self.stage / 3) as usize]
                    } else if self.inspector_capture {
                        [
                            "list-first.ppm",
                            "list-next.ppm",
                            "json.ppm",
                            "sorted-set.ppm",
                            "stream.ppm",
                            "hash.ppm",
                        ][(self.stage / 3) as usize]
                    } else if self.stage == 2 {
                        "tree.ppm"
                    } else {
                        "filtered-list.ppm"
                    });
                    let directory = directory.clone();
                    std::thread::spawn(move || {
                        let mut data =
                            format!("P6\n{} {}\n255\n", image.width(), image.height()).into_bytes();
                        for pixel in &image.pixels {
                            data.extend_from_slice(&pixel.to_array()[..3]);
                        }
                        std::fs::create_dir_all(directory).unwrap();
                        std::fs::write(&path, data).unwrap();
                        eprintln!("Captured {}", path.display());
                    });
                    self.stage += 1;
                    self.frames = 0;
                    self.ready_since = None;
                }
            }
            if self.inspector_capture {
                let last_stage = if self.layout_capture {30} else if self.command_capture {3} else if self.vector_capture {9} else if self.timeseries_capture {12} else if self.topology_capture || self.recovery_capture {6} else if self.monitor_capture { 9 } else { 18 };
                if self.stage < last_stage
                    && self.stage.is_multiple_of(3)
                    && (if self.layout_capture { self.workspace.preview_layout(ctx, (self.stage/3) as usize) } else if self.command_capture { self.workspace.preview_commands() } else if self.vector_capture { self.workspace.preview_vectors((self.stage/3) as usize) } else if self.timeseries_capture { self.workspace.preview_timeseries((self.stage/3) as usize) } else if self.recovery_capture { self.workspace.preview_recovery((self.stage/3) as usize) } else if self.topology_capture && std::env::var_os("RUSTABLE_REDIS_BULK_CAPTURE").is_some() { self.workspace.preview_bulk((self.stage/3) as usize) } else if self.topology_capture { self.workspace.preview_topology((self.stage/3) as usize) } else if self.archive_capture {
                        self.workspace
                            .preview_archive(ctx, (self.stage / 3) as usize)
                    } else if self.pubsub_capture {
                        self.workspace
                            .preview_pubsub(ctx, (self.stage / 3) as usize)
                    } else if self.monitor_capture {
                        self.workspace
                            .preview_monitor(ctx, (self.stage / 3) as usize)
                    } else if self.search_capture {
                        self.workspace.preview_search((self.stage / 3) as usize)
                    } else if self.roadmap_capture {
                        self.workspace.preview_roadmap((self.stage / 3) as usize)
                    } else {
                        self.workspace.preview_inspector((self.stage / 3) as usize)
                    })
                {
                    self.stage += 1;
                }
                if self.stage < last_stage && self.stage % 3 == 1 && (!self.workspace.busy() || self.recovery_capture || (self.layout_capture && self.stage / 3 == 9)) {
                    if self.layout_capture {
                        // Full-page state is ready.
                    } else if self.command_capture {
                        // Command response is ready.
                    } else if self.vector_capture {
                        // Element page is ready.
                    } else if self.timeseries_capture {
                        // The sample page and its applied range are ready.
                    } else if self.recovery_capture {
                        // Draft/recovery state prepared through the real UI handlers.
                    } else if self.topology_capture {
                        // Topology response prepared by the worker.
                    } else if self.archive_capture {
                        // Review response has been prepared on the worker.
                    } else if self.pubsub_capture {
                        // Dedicated subscriber has its own worker.
                    } else if self.monitor_capture {
                        // Dedicated capture has its own worker.
                    } else if self.search_capture {
                        // Search response is already prepared by the worker.
                    } else if self.roadmap_capture {
                        self.workspace
                            .preview_roadmap_format((self.stage / 3) as usize);
                    } else {
                        self.workspace
                            .preview_value_format((self.stage / 3) as usize);
                    }
                    self.frames += 1;
                    if self.frames >= 4
                        && self
                            .ready_since
                            .get_or_insert_with(std::time::Instant::now)
                            .elapsed()
                            >= std::time::Duration::from_millis(500)
                    {
                        ctx.send_viewport_cmd(
                            egui::ViewportCommand::Screenshot(Default::default()),
                        );
                        self.stage += 1;
                    }
                }
                if self.stage < last_stage {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
            } else {
                if self.stage == 0 && self.workspace.preview_stage(false) {
                    self.stage = 1;
                }
                if self.stage == 3 && self.workspace.preview_stage(true) {
                    self.stage = 4;
                }
                if matches!(self.stage, 1 | 4) && !self.workspace.busy() {
                    self.frames += 1;
                    if self.frames >= 4
                        && self
                            .ready_since
                            .get_or_insert_with(std::time::Instant::now)
                            .elapsed()
                            >= std::time::Duration::from_millis(500)
                    {
                        ctx.send_viewport_cmd(
                            egui::ViewportCommand::Screenshot(Default::default()),
                        );
                        self.stage += 1;
                    }
                }
                if self.stage < 6 {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }
        self.workspace.show(
            ctx,
            "Disposable Redis fixture",
            egui::Color32::from_rgb(148, 58, 58),
            &mut self.sizes,
            &mut false,
        );
    }
}
