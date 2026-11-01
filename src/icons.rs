//! Shared Lucide artwork, cached at the display pixel density. See assets/icons/lucide/LICENSE.
use eframe::egui;
pub const TOOLBAR_BUTTON: egui::Vec2 = egui::vec2(37.0, 24.0);
pub const fn toolbar_group_width(count: usize) -> f32 { count as f32 * TOOLBAR_BUTTON.x + 4.0 }
pub fn paint(painter: &egui::Painter, center: egui::Pos2, name: &str, size: f32, color: egui::Color32) {
    let (uri, bytes): (&str, &'static [u8]) = match name {
        "clipboard" => ("bytes://lucide/clipboard.svg", include_bytes!("../assets/icons/lucide/clipboard.svg")),
        "check" => ("bytes://lucide/check.svg", include_bytes!("../assets/icons/lucide/check.svg")),
        "chevron-down" => ("bytes://lucide/chevron-down.svg", include_bytes!("../assets/icons/lucide/chevron-down.svg")),
        "chevron-left" => ("bytes://lucide/chevron-left.svg", include_bytes!("../assets/icons/lucide/chevron-left.svg")),
        "chevron-right" => ("bytes://lucide/chevron-right.svg", include_bytes!("../assets/icons/lucide/chevron-right.svg")),
        "circle-alert" => ("bytes://lucide/circle-alert.svg", include_bytes!("../assets/icons/lucide/circle-alert.svg")),
        "circle-stop" => ("bytes://lucide/circle-stop.svg", include_bytes!("../assets/icons/lucide/circle-stop.svg")),
        "clock" => ("bytes://lucide/clock.svg", include_bytes!("../assets/icons/lucide/clock.svg")),
        "code" => ("bytes://lucide/code.svg", include_bytes!("../assets/icons/lucide/code.svg")),
        "command" => ("bytes://lucide/command.svg", include_bytes!("../assets/icons/lucide/command.svg")),
        "database" => ("bytes://lucide/database.svg", include_bytes!("../assets/icons/lucide/database.svg")),
        "eye" => ("bytes://lucide/eye.svg", include_bytes!("../assets/icons/lucide/eye.svg")),
        "file-code" => ("bytes://lucide/file-code.svg", include_bytes!("../assets/icons/lucide/file-code.svg")),
        "file-plus" => ("bytes://lucide/file-plus.svg", include_bytes!("../assets/icons/lucide/file-plus.svg")),
        "folder" => ("bytes://lucide/folder.svg", include_bytes!("../assets/icons/lucide/folder.svg")),
        "key" => ("bytes://lucide/key.svg", include_bytes!("../assets/icons/lucide/key.svg")),
        "panel-bottom" => ("bytes://lucide/panel-bottom.svg", include_bytes!("../assets/icons/lucide/panel-bottom.svg")),
        "panel-left" => ("bytes://lucide/panel-left.svg", include_bytes!("../assets/icons/lucide/panel-left.svg")),
        "panel-right" => ("bytes://lucide/panel-right.svg", include_bytes!("../assets/icons/lucide/panel-right.svg")),
        "pin" => ("bytes://lucide/pin.svg", include_bytes!("../assets/icons/lucide/pin.svg")),
        "plug" => ("bytes://lucide/plug.svg", include_bytes!("../assets/icons/lucide/plug.svg")),
        "plus" => ("bytes://lucide/plus.svg", include_bytes!("../assets/icons/lucide/plus.svg")),
        "refresh-cw" => ("bytes://lucide/refresh-cw.svg", include_bytes!("../assets/icons/lucide/refresh-cw.svg")),
        "search" => ("bytes://lucide/search.svg", include_bytes!("../assets/icons/lucide/search.svg")),
        "sheet" => ("bytes://lucide/sheet.svg", include_bytes!("../assets/icons/lucide/sheet.svg")),
        "sliders-horizontal" => ("bytes://lucide/sliders-horizontal.svg", include_bytes!("../assets/icons/lucide/sliders-horizontal.svg")),
        "upload" => ("bytes://lucide/upload.svg", include_bytes!("../assets/icons/lucide/upload.svg")),
        "x" => ("bytes://lucide/x.svg", include_bytes!("../assets/icons/lucide/x.svg")),
        _ => return,
    };
    let ctx = painter.ctx();
    egui_extras::install_image_loaders(ctx);
    ctx.include_bytes(uri, bytes);
    let pixels = (size * ctx.pixels_per_point()).ceil() as u32;
    // Consult egui's cache each frame so it keeps the texture alive.
    if let Ok(egui::load::TexturePoll::Ready { texture }) = ctx.try_load_texture(uri, egui::TextureOptions::LINEAR, egui::load::SizeHint::Width(pixels)) {
        painter.image(texture.id, egui::Rect::from_center_size(center, egui::Vec2::splat(size)), egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), color);
    }
}
pub fn button(ui: &mut egui::Ui, name: &str, size: egui::Vec2, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let visuals = ui.style().interact_selectable(&response, selected);
    if selected || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 12.0, visuals.weak_bg_fill);
    }
    let label = match name {
        "plug" => "Open connections", "database" => "Open database", "command" => "Find anything",
        "refresh-cw" => "Refresh", "sliders-horizontal" => "Options", "search" => "Search",
        "chevron-left" => "Previous", "chevron-right" => "Next", "plus" => "Add",
        "x" => "Close", "pin" => "Pin result", "panel-bottom" => "SQL activity",
        "panel-right" => "Row inspector", _ => name,
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    paint(ui.painter(), rect.center(), name, 18.0, visuals.fg_stroke.color);
    response
}

pub fn widget(name: &'static str) -> impl egui::Widget {
    move |ui: &mut egui::Ui| button(ui, name, egui::vec2(24.0, 24.0), false)
}

/// Fixed outer geometry; borders are painted inside and never participate in layout.
pub fn toolbar_group<R>(ui: &mut egui::Ui, count: usize, contents: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(toolbar_group_width(count), 32.0), egui::Sense::hover());
    ui.painter().rect(rect, 16.0, ui.visuals().widgets.inactive.weak_bg_fill,
        ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    let inner = ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(2.0, 4.0)))
        .layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            contents(ui)
        }).inner;
    egui::InnerResponse { inner, response }
}

/// Shortcut keys are artwork, not font glyphs: identical on native and WASM.
/// Leave 12 points before the shortcut and 2 points between its keys.
pub fn shortcut_button(label: &'static str, shift: bool, key: &'static str, fill: Option<egui::Color32>) -> impl egui::Widget {
    move |ui: &mut egui::Ui| {
        egui_extras::install_image_loaders(ui.ctx());
        ui.scope(|ui| {
            ui.spacing_mut().icon_spacing = 2.0;
            let mut atoms = egui::Atoms::new(label);
            atoms.push_right(egui::Atom { size: Some(egui::vec2(8.0, 0.0)), ..Default::default() });
            let icon = |source| egui::Image::new(source).fit_to_exact_size(egui::vec2(14.0, 14.0));
            let mac = ui.ctx().os() == egui::os::OperatingSystem::Mac;
            if !mac { atoms.push_right("Ctrl"); }
            if shift { atoms.push_right(icon(egui::include_image!("../assets/icons/lucide/arrow-big-up.svg"))); }
            if mac { atoms.push_right(icon(egui::include_image!("../assets/icons/lucide/command.svg"))); }
            if key == "Enter" {
                atoms.push_right(icon(egui::include_image!("../assets/icons/lucide/corner-down-left.svg")));
            } else {
                atoms.push_right(key);
            }
            let button = egui::Button::new(atoms).image_tint_follows_text_color(fill.is_none());
            if let Some(fill) = fill {
                ui.visuals_mut().override_text_color = Some(egui::Color32::WHITE);
                ui.add(button.fill(fill))
            } else {
                ui.add(button)
            }
        }).inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_groups_share_exact_button_and_container_geometry() {
        for scale in [1.0, 1.5, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(scale);
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for count in [2, 3] {
                            let mut buttons = Vec::new();
                            let group = toolbar_group(ui, count, |ui| {
                                for _ in 0..count { buttons.push(button(ui, "database", TOOLBAR_BUTTON, false).rect); }
                            }).response.rect;
                            assert_eq!(group.height(), 32.0);
                            assert_eq!(group.width(), count as f32 * 37.0 + 4.0);
                            for (index, rect) in buttons.iter().enumerate() {
                                assert_eq!(rect.size(), TOOLBAR_BUTTON);
                                assert_eq!(rect.top() - group.top(), 4.0);
                                assert_eq!(rect.left() - group.left(), 2.0 + index as f32 * 37.0);
                            }
                        }
                    });
                });
            });
        }
    }
}
