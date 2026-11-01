use eframe::egui;

/// Compact, fixed-order pane controls, independent of the toolbar's RTL layout.
pub(super) fn show(ui: &mut egui::Ui, activity: &mut bool, inspector: &mut bool) {
    crate::icons::toolbar_group(ui, 2, |ui| {
        for (index, visible) in [activity, inspector].into_iter().enumerate() {
            let name = if index == 0 { "activity log" } else { "row inspector" };
            let mut response = crate::icons::button(ui, if index == 0 { "panel-bottom" } else { "panel-right" }, crate::icons::TOOLBAR_BUTTON, *visible);
            if response.clicked() { *visible = !*visible; response.mark_changed(); }
            response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), *visible, name));
            response.on_hover_text(format!("{} {name}", if *visible { "Hide" } else { "Show" }));
        }
    });
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
