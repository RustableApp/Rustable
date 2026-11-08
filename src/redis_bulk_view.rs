//! Cached, virtualized per-key results for non-atomic Cluster bulk actions.
use eframe::egui;
pub(crate) struct View {
    rows: Vec<[String; 3]>,
    summary: String,
    stopped: Option<String>,
}
impl View {
    pub fn new(report: crate::redis_cluster::WriteReport<bool>) -> Self {
        use crate::redis_cluster::WriteState;
        let mut counts = [0usize; 4];
        let rows = report
            .outcomes
            .into_iter()
            .map(|row| {
                let (index, label) = match row.state {
                    WriteState::Confirmed(true) => (0, "Applied"),
                    WriteState::Confirmed(false) => (1, "Unchanged"),
                    WriteState::Unknown => (2, "Unconfirmed"),
                    WriteState::NotAttempted => (3, "Not attempted"),
                };
                counts[index] += 1;
                [
                    crate::redis_commands::quote(&row.key),
                    label.into(),
                    row.message,
                ]
            })
            .collect();
        Self {
            rows,
            summary: format!(
                "{} applied · {} unchanged · {} unconfirmed · {} not attempted",
                counts[0], counts[1], counts[2], counts[3]
            ),
            stopped: report.stopped,
        }
    }
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let mut open = true;
        let mut close = false;
        egui::Window::new("Bulk action results")
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .frame(egui::Frame::window(&ctx.style()).inner_margin(16).corner_radius(10.0))
            .open(&mut open).default_width(720.0)
            .collapsible(false).show(ctx, |ui| {
                crate::redis_workspace::browser_style(ui);
                ui.strong(&self.summary);
                if let Some(error) = &self.stopped { ui.colored_label(ui.visuals().warn_fg_color, error); }
                ui.weak("Unchanged means the key was absent or its expiry did not change. Inspect unconfirmed keys before retrying; successful changes are not rolled back.");
                ui.add_space(8.0);
                use egui_extras::{Column, TableBuilder};
                TableBuilder::new(ui).id_salt("redis-bulk-results").striped(true).max_scroll_height(360.0)
                    .column(Column::initial(240.0).clip(true)).column(Column::initial(120.0).clip(true)).column(Column::remainder().clip(true))
                    .header(25.0, |mut row| { for label in ["KEY", "OUTCOME", "DETAIL"] { row.col(|ui| { ui.weak(label); }); } })
                    .body(|body| { body.rows(25.0, self.rows.len(), |mut row| {
                        for cell in &self.rows[row.index()] { row.col(|ui| { ui.add(egui::Label::new(cell).truncate()).on_hover_text(cell); }); }
                    }); });
                ui.separator();
                close = ui.add(egui::Button::new("Close").min_size(egui::vec2(84.0, crate::controls::BUTTON_HEIGHT))).clicked();
            });
        open && !close
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "manual Redis bulk first-open/after-idle CPU benchmark"]
    fn redis_bulk_idle_frame_benchmark() {
        let mut first = Vec::new();
        let mut idle = Vec::new();
        for iteration in 0..45 {
            let ctx = egui::Context::default();
            let mut view = View {
                rows: (0..1000)
                    .map(|i| [format!("key:{i:06}"), "Applied".into(), String::new()])
                    .collect(),
                summary: "1000 applied · 0 unchanged · 0 unconfirmed · 0 not attempted".into(),
                stopped: None,
            };
            let frame = |time, events, view: &mut View| {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1200.0, 760.0),
                        )),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        assert!(view.show(ctx));
                    },
                );
                let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            };
            // Include sizing and the first visible frames, not an empty initial pass.
            let start = std::time::Instant::now();
            for n in 0..3 {
                frame(n as f64 / 120.0, vec![], &mut view);
            }
            let opening = start.elapsed().as_secs_f64() * 1000.0;
            for n in 3..30 {
                frame(n as f64 / 120.0, vec![], &mut view);
            }
            assert!(
                !ctx.has_requested_repaint(),
                "Idle results kept requesting redraws"
            );
            let start = std::time::Instant::now();
            frame(
                60.0,
                vec![
                    egui::Event::PointerMoved(egui::pos2(600.0, 400.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -24.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                &mut view,
            );
            let after_idle = start.elapsed().as_secs_f64() * 1000.0;
            if iteration >= 5 {
                first.push(opening);
                idle.push(after_idle);
            }
        }
        first.sort_by(f64::total_cmp);
        idle.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/bulk-first-open-three-frames", &first);
        crate::ui_benchmark::report("redis/bulk-first-scroll-after-idle", &idle);
    }
    #[test]
    #[ignore = "manual Redis bulk result CPU benchmark"]
    fn redis_bulk_results_frame_benchmark() {
        // The same presentation-only fixture is injected into the comparison
        // baseline; this adds coverage for the new interaction, not an old UI.
        for scroll in [false, true] {
            let mut view = View {
                rows: (0..1000)
                    .map(|i| {
                        [
                            format!("\"app:orders:{i:06}:binary\\x00\""),
                            ["Applied", "Unchanged", "Unconfirmed", "Not attempted"][i % 4].into(),
                            if i % 4 == 2 {
                                "Submitted outcome is unconfirmed. Inspect before retrying.".into()
                            } else {
                                String::new()
                            },
                        ]
                    })
                    .collect(),
                summary: "250 applied · 250 unchanged · 250 unconfirmed · 250 not attempted".into(),
                stopped: Some("Connection lost; uncertain writes were not retried.".into()),
            };
            let ctx = egui::Context::default();
            let mut samples = Vec::new();
            for frame in 0..200 {
                let events = if scroll {
                    vec![
                        egui::Event::PointerMoved(egui::pos2(600.0, 400.0)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, -24.0),
                            modifiers: egui::Modifiers::NONE,
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
                            egui::vec2(1200.0, 760.0),
                        )),
                        time: Some(frame as f64 / 120.0),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        assert!(view.show(ctx));
                    },
                );
                let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
                if frame >= 20 {
                    samples.push(start.elapsed().as_secs_f64() * 1000.0);
                }
            }
            samples.sort_by(f64::total_cmp);
            crate::ui_benchmark::report(
                if scroll {
                    "redis/bulk-results-scroll"
                } else {
                    "redis/bulk-results"
                },
                &samples,
            );
        }
    }
}
