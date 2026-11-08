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
