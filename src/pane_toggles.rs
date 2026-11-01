use eframe::egui::{self, Stroke};

/// Compact, fixed-order pane controls, independent of the toolbar's RTL layout.
pub(super) fn show(ui: &mut egui::Ui, activity: &mut bool, inspector: &mut bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(78.0, 28.0), egui::Sense::hover());
    ui.painter().rect(
        rect,
        14.0,
        ui.visuals().widgets.inactive.weak_bg_fill,
        ui.visuals().widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    for (index, visible) in [activity, inspector].into_iter().enumerate() {
        let button = egui::Rect::from_min_size(
            rect.min + egui::vec2(2.0 + index as f32 * 37.0, 2.0),
            egui::vec2(37.0, 24.0),
        );
        let mut response = ui.interact(
            button,
            ui.id().with(("pane-toggle", index)),
            egui::Sense::click(),
        );
        if response.clicked() {
            *visible = !*visible;
            response.mark_changed();
        }
        let name = if index == 0 {
            "activity log"
        } else {
            "row inspector"
        };
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                ui.is_enabled(),
                *visible,
                name,
            )
        });
        let visuals = ui.style().interact(&response);
        if *visible || response.hovered() || response.has_focus() {
            ui.painter().rect_filled(
                button,
                12.0,
                if *visible {
                    ui.visuals().widgets.active.weak_bg_fill
                } else {
                    visuals.weak_bg_fill
                },
            );
        }
        let stroke = Stroke::new(1.3_f32, visuals.fg_stroke.color);
        let icon = egui::Rect::from_center_size(button.center(), egui::vec2(17.0, 13.0));
        ui.painter()
            .rect_stroke(icon, 2.0, stroke, egui::StrokeKind::Inside);
        if index == 0 {
            let y = icon.bottom() - 4.5;
            ui.painter().line_segment(
                [egui::pos2(icon.left(), y), egui::pos2(icon.right(), y)],
                stroke,
            );
            for x in [-5.0, 0.0, 5.0] {
                ui.painter().circle_filled(
                    egui::pos2(icon.center().x + x, icon.bottom() - 2.2),
                    0.7,
                    stroke.color,
                );
            }
        } else {
            let x = icon.right() - 5.0;
            ui.painter().line_segment(
                [egui::pos2(x, icon.top()), egui::pos2(x, icon.bottom())],
                stroke,
            );
            for y in [-3.0, 0.0, 3.0] {
                ui.painter().circle_filled(
                    egui::pos2(icon.right() - 2.4, icon.center().y + y),
                    0.7,
                    stroke.color,
                );
            }
        }
        response.on_hover_text(format!("{} {name}", if *visible { "Hide" } else { "Show" }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_buttons_toggle_independently_in_rtl_toolbar() {
        let ctx = egui::Context::default();
        let (mut activity, mut inspector) = (false, false);
        let mut bounds = egui::Rect::NOTHING;
        let mut frame = |events| {
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            bounds = ui
                                .scope(|ui| show(ui, &mut activity, &mut inspector))
                                .response
                                .rect;
                        });
                    });
                },
            );
            (bounds, activity, inspector)
        };
        let (rect, _, _) = frame(vec![]);
        for (index, expected) in [(0, (true, false)), (1, (true, true)), (0, (false, true))] {
            let pos = egui::pos2(rect.left() + 20.5 + index as f32 * 37.0, rect.center().y);
            frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            let (_, activity, inspector) = frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            assert_eq!((activity, inspector), expected);
        }
    }
}
