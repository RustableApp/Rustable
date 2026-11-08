//! Shared SQL/Redis control conventions. Preserve SQL's established geometry.
use eframe::egui::{self, Stroke};
pub(crate) const BUTTON_HEIGHT: f32 = 23.0;
pub(crate) fn base_spacing(spacing: &mut egui::Spacing) {
    spacing.item_spacing = egui::vec2(6.0, 3.0);
    spacing.button_padding = egui::vec2(8.0, 4.0);
    spacing.interact_size.y = BUTTON_HEIGHT;
}
/// Match the custom pagination controls across every interaction state.
pub(crate) fn rounded_action_buttons(ui: &mut egui::Ui) {
    let widgets = &mut ui.visuals_mut().widgets;
    for widget in [
        &mut widgets.noninteractive,
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(5);
    }
}

/// Same pill geometry, active inset and separators as the SQL table view switch.
pub(crate) fn segments<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    id: &str,
    current: &mut T,
    segments: &[(T, &str, f32)],
) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(segments.iter().map(|s| s.2).sum(), 28.0),
        egui::Sense::hover(),
    );
    ui.painter()
        .rect_filled(rect, 6.0, ui.visuals().widgets.inactive.weak_bg_fill);
    let mut x = rect.left();
    for (index, (view, title, width)) in segments.iter().enumerate() {
        let segment =
            egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(*width, rect.height()));
        let response = ui.interact(segment, ui.id().with((id, index)), egui::Sense::click());
        if response.clicked() {
            *current = *view;
        }
        if *current == *view {
            ui.painter()
                .rect_filled(segment.shrink(1.0), 5.0, ui.visuals().selection.bg_fill);
        }
        if index > 0 && *current != *view && *current != segments[index - 1].0 {
            ui.painter().line_segment(
                [
                    egui::pos2(x, rect.top() + 6.0),
                    egui::pos2(x, rect.bottom() - 6.0),
                ],
                Stroke::new(1.0_f32, ui.visuals().weak_text_color().gamma_multiply(0.7)),
            );
        }
        let color = if *current == *view {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().text_color()
        };
        ui.painter().text(
            segment.center(),
            egui::Align2::CENTER_CENTER,
            *title,
            egui::FontId::proportional(13.0),
            color,
        );
        x += width;
    }
}

/// Shared bounded report table: enclosed surface, distinct header, quiet striping,
/// aligned metrics and truncation with full-value tooltips. Callers own paging.
pub(crate) fn report_table(
    ui: &mut egui::Ui,
    id: &str,
    title: &str,
    headers: [&str; 3],
    rows: &[[String; 3]],
    numeric: [bool; 3],
    height: f32,
) {
    use egui_extras::{Column, TableBuilder};
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(6.0)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.strong(title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(format!("{} entries", rows.len()));
                });
            });
            ui.add_space(6.0);
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(12.0, 0.0);
                ui.visuals_mut().faint_bg_color = ui
                    .visuals()
                    .widgets
                    .inactive
                    .weak_bg_fill
                    .gamma_multiply(0.45);
                let numeric_width = (ui.available_width() * 0.19).clamp(72.0, 140.0);
                let trailing_detail = headers[2] == "Command"
                    || headers[2] == "Slots / node ID"
                    || headers[2] == "Value / rule";
                let peak = if headers[1] == "Memory" {
                    rows.iter()
                        .filter_map(|r| r[1].parse::<u64>().ok())
                        .max()
                        .unwrap_or(1)
                        .max(1)
                } else {
                    1
                };
                let header = egui::Rect::from_min_size(
                    ui.next_widget_position(),
                    egui::vec2(ui.available_width(), 32.0),
                );
                ui.painter()
                    .rect_filled(header, 3.0, ui.visuals().faint_bg_color);
                ui.painter().line_segment(
                    [header.left_bottom(), header.right_bottom()],
                    ui.visuals().widgets.noninteractive.bg_stroke,
                );
                TableBuilder::new(ui)
                    .id_salt(id)
                    .striped(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .max_scroll_height(height)
                    .min_scrolled_height(0.0)
                    .column(if trailing_detail {
                        Column::initial(170.0).clip(true)
                    } else {
                        Column::remainder().at_least(100.0).clip(true)
                    })
                    .column(Column::exact(numeric_width).clip(true))
                    .column(if trailing_detail {
                        Column::remainder().clip(true)
                    } else {
                        Column::exact(numeric_width.max(100.0)).clip(true)
                    })
                    .header(32.0, |mut row| {
                        for (i, header) in headers.iter().enumerate() {
                            row.col(|ui| {
                                ui.with_layout(
                                    if numeric[i] {
                                        egui::Layout::right_to_left(egui::Align::Center)
                                    } else {
                                        egui::Layout::left_to_right(egui::Align::Center)
                                    },
                                    |ui| {
                                        ui.label(
                                            egui::RichText::new(*header)
                                                .size(12.0)
                                                .strong()
                                                .color(ui.visuals().weak_text_color()),
                                        );
                                    },
                                );
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(32.0, rows.len(), |mut row| {
                            let index = row.index();
                            for (i, value) in rows[index].iter().enumerate() {
                                row.col(|ui| {
                                    ui.with_layout(
                                        if numeric[i] {
                                            egui::Layout::right_to_left(egui::Align::Center)
                                        } else {
                                            egui::Layout::left_to_right(egui::Align::Center)
                                        },
                                        |ui| {
                                            if i == 1 && headers[i] == "Memory" {
                                                let r = ui.max_rect();
                                                let fraction = value.parse::<u64>().unwrap_or(0)
                                                    as f32
                                                    / peak as f32;
                                                let bar = egui::Rect::from_min_size(
                                                    egui::pos2(r.left(), r.bottom() - 3.0),
                                                    egui::vec2(r.width() * fraction, 2.0),
                                                );
                                                ui.painter().rect_filled(
                                                    bar,
                                                    1.0,
                                                    ui.visuals()
                                                        .selection
                                                        .bg_fill
                                                        .gamma_multiply(0.6),
                                                );
                                            }
                                            let value_text = if numeric[i] && headers[i] == "Memory"
                                            {
                                                human_bytes(value)
                                            } else {
                                                value.clone()
                                            };
                                            ui.add(
                                                egui::Label::new(if numeric[i] {
                                                    egui::RichText::new(value_text)
                                                } else {
                                                    egui::RichText::new(value_text).monospace()
                                                })
                                                .truncate(),
                                            )
                                            .on_hover_text(value);
                                        },
                                    );
                                });
                            }
                        })
                    });
            });
            if rows.is_empty() {
                ui.add_space(12.0);
                ui.weak("No entries in this snapshot.");
                ui.add_space(12.0);
            }
        });
}
fn human_bytes(value: &str) -> String {
    let Ok(bytes) = value.parse::<u64>() else {
        return value.into();
    };
    if bytes >= 1048576 {
        format!("{:.2} MiB", bytes as f64 / 1048576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

pub(crate) fn table_header(ui: &mut egui::Ui, label: &str) {
    let rect = ui.max_rect();
    ui.painter()
        .rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
    ui.add(egui::Label::new(egui::RichText::new(label).strong().size(12.0)).truncate());
}
pub(crate) fn table_surface<R>(
    ui: &mut egui::Ui,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(6.0)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            content(ui)
        })
}
