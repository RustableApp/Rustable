//! Immutable launch details captured once when the local MCP listener starts.
use eframe::egui;
use serde_json::json;
use std::path::Path;

pub(crate) struct Connector {
    pub command: String,
    pub arguments: String,
    pub configuration: String,
}

impl Connector {
    pub fn new(command: &Path, socket: &Path) -> Result<Self, String> {
        let command = command
            .to_str()
            .ok_or("MCP executable path must be valid UTF-8")?;
        let socket = socket
            .to_str()
            .ok_or("MCP socket path must be valid UTF-8")?;
        let args = json!(["mcp", "--socket", socket]);
        let configuration = serde_json::to_string_pretty(&json!({
            "mcpServers": { "rustable-matrix": { "command": command, "args": args } }
        }))
        .map_err(|error| error.to_string())?;
        Ok(Self {
            command: command.into(),
            arguments: args.to_string(),
            configuration,
        })
    }

    pub fn show(&self, ctx: &egui::Context) -> bool {
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new((ctx.viewport_id(), "matrix-connector")))
            .frame(egui::Frame::popup(&ctx.style()).inner_margin(16).corner_radius(14))
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width()-64.0).clamp(240.0, 620.0));
                ui.set_max_height((ctx.content_rect().height()-96.0).max(180.0));
                ui.horizontal(|ui| {
                    ui.strong("Connect an AI agent");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui.button("Close").clicked();
                    });
                });
                ui.add_space(8.0);
                egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-150.0).max(120.0))
                    .show(ui, |ui| {
                        ui.label("Add Rustable as a local MCP server in your AI agent.");
                        ui.weak("Transport: stdio · Server name: rustable-matrix");
                        ui.add_space(12.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.strong("JSON configuration");
                            crate::redis_value_editor::copy(ui, "matrix-config", Some("Copy configuration"), true, || self.configuration.clone());
                        });
                        ui.add_space(6.0);
                        egui::Frame::new().fill(ui.visuals().extreme_bg_color)
                            .inner_margin(10).corner_radius(6).show(ui, |ui| {
                                ui.set_width(ui.available_width().max(100.0));
                                code(ui, &self.configuration);
                            });
                        ui.add_space(8.0);
                        let manual_qa = cfg!(feature = "qa") && std::env::var_os("RUSTABLE_MATRIX_CONNECTOR_MANUAL_QA").is_some();
                        egui::CollapsingHeader::new("Manual setup").default_open(manual_qa).show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.strong("Command");
                                crate::redis_value_editor::copy(ui, "matrix-command", Some("Copy command"), true, || self.command.clone());
                            });
                            code(ui, &self.command);
                            ui.add_space(8.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.strong("Arguments (JSON array)");
                                crate::redis_value_editor::copy(ui, "matrix-args", Some("Copy arguments"), true, || self.arguments.clone());
                            });
                            code(ui, &self.arguments);
                        });
                        ui.separator();
                        ui.label("Then open the intended database and choose AI → Enable AI control.");
                        ui.weak("Your agent can read and edit enabled SQL tabs and run queries using that connection’s permissions and Safe mode.");
                        ui.add_space(8.0);
                        ui.weak("Keep Rustable open. This configuration belongs to the current session; copy it again after restarting Rustable.");
                    });
            });
        close || modal.should_close()
    }
}

fn code(ui: &mut egui::Ui, text: &str) {
    let mut job = egui::text::LayoutJob::simple(
        text.into(),
        egui::FontId::monospace(12.0),
        ui.visuals().text_color(),
        ui.available_width(),
    );
    job.wrap.break_anywhere = true;
    ui.add(egui::Label::new(job).wrap().selectable(true));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connector_config_preserves_executable_and_socket_arguments() {
        let command = Path::new("/Applications/Rustable \"Preview\".app/Contents/MacOS/rustable");
        let socket = Path::new("/tmp/private café/workspace.sock");
        let connector = Connector::new(command, socket).unwrap();
        let value: serde_json::Value = serde_json::from_str(&connector.configuration).unwrap();
        let server = &value["mcpServers"]["rustable-matrix"];
        assert_eq!(server["command"], command.to_str().unwrap());
        assert_eq!(
            server["args"],
            json!(["mcp", "--socket", socket.to_str().unwrap()])
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&connector.arguments).unwrap(),
            server["args"]
        );
        assert_eq!(server.as_object().unwrap().len(), 2);
    }
    fn texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(text) => out.push((
                text.galley.text().into(),
                egui::Rect::from_min_size(text.pos, text.galley.size()),
            )),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    texts(shape, out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn connector_copy_uses_exact_live_configuration_and_feedback_settles() {
        let ctx = egui::Context::default();
        let connector = Connector::new(
            Path::new("/Applications/Rustable Preview.app/Contents/MacOS/rustable"),
            Path::new("/tmp/private session/workspace.sock"),
        )
        .unwrap();
        let mut button = egui::Rect::NOTHING;
        let mut copied = false;
        let mut feedback = false;
        for frame in 0..80 {
            let events = if frame == 3 || frame == 4 {
                vec![
                    egui::Event::PointerMoved(button.center()),
                    egui::Event::PointerButton {
                        pos: button.center(),
                        button: egui::PointerButton::Primary,
                        pressed: frame == 3,
                        modifiers: Default::default(),
                    },
                ]
            } else {
                vec![]
            };
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(760.0, 760.0),
                    )),
                    time: Some(frame as f64 / 10.0),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    assert!(!connector.show(ctx));
                },
            );
            let mut labels = vec![];
            for shape in &output.shapes {
                texts(&shape.shape, &mut labels);
            }
            if frame == 2 {
                button = labels
                    .iter()
                    .find(|(s, _)| s == "Copy configuration")
                    .unwrap()
                    .1;
            }
            if frame == 4 {
                copied=output.platform_output.commands.iter().any(|command|matches!(command,egui::OutputCommand::CopyText(text) if text==&connector.configuration));
            }
            feedback |= labels.iter().any(|(s, _)| s == "Copied");
            if frame == 79 {
                assert!(labels.iter().any(|(s, _)| s == "Copy configuration"));
                assert!(
                    output
                        .viewport_output
                        .values()
                        .all(|v| v.repaint_delay > std::time::Duration::from_millis(100))
                );
            }
        }
        assert!(copied, "copy must emit the exact current configuration");
        assert!(feedback, "copy must show in-place confirmation");
    }

    #[test]
    #[ignore = "release CPU workload for the new MCP connection dialog"]
    fn matrix_connector_frame_benchmark() {
        for width in [500.0, 760.0] {
            let ctx = egui::Context::default();
            let connector = Connector::new(
                Path::new("/Applications/Rustable Preview.app/Contents/MacOS/rustable"),
                Path::new("/var/folders/private-rustable-session/workspace.sock"),
            )
            .unwrap();
            let mut samples = Vec::new();
            let mut button = egui::Rect::NOTHING;
            let mut copied = false;
            for frame in 0..180 {
                let events = if frame == 40 || frame == 41 {
                    vec![
                        egui::Event::PointerMoved(button.center()),
                        egui::Event::PointerButton {
                            pos: button.center(),
                            button: egui::PointerButton::Primary,
                            pressed: frame == 40,
                            modifiers: Default::default(),
                        },
                    ]
                } else {
                    vec![]
                };
                let start = std::time::Instant::now();
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 760.0),
                        )),
                        events,
                        time: Some(frame as f64 / 120.0 + if frame >= 100 { 30.0 } else { 0.0 }),
                        ..Default::default()
                    },
                    |ctx| {
                        connector.show(ctx);
                    },
                );
                if frame == 2 {
                    let mut labels = vec![];
                    for shape in &output.shapes {
                        texts(&shape.shape, &mut labels);
                    }
                    button = labels
                        .iter()
                        .find(|(s, _)| s == "Copy configuration")
                        .unwrap()
                        .1;
                }
                copied |= output.platform_output.commands.iter().any(|command|matches!(command,egui::OutputCommand::CopyText(text) if text==&connector.configuration));
                let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
                let ms = start.elapsed().as_secs_f64() * 1000.0;
                if frame == 0 || frame == 100 {
                    eprintln!("Connector {width}px frame {frame}: {ms:.6}ms");
                }
                if frame >= 20 {
                    samples.push(ms);
                }
            }
            assert!(copied);
            samples.sort_by(f64::total_cmp);
            crate::ui_benchmark::report(&format!("matrix/connector/{width}"), &samples);
        }
    }
}
