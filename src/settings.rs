//! Application-wide appearance and authentication preferences.
use super::*;

impl Rustable {
    pub(super) fn settings_shortcut(&mut self, ctx: &egui::Context) {
        let native =
            ctx.input(|input| input.focused) && crate::native_menu::take_settings_requested();
        let key =
            ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma));
        if (native || key) && !self.protection.modal_open() {
            self.settings_open = true;
        }
    }

    pub(super) fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = true;
        let width = (ctx.content_rect().width() - 64.0).clamp(280.0, 600.0);
        egui::Window::new("Settings").id(egui::Id::new((ctx.viewport_id(), "settings")))
            .open(&mut open).collapsible(false).resizable(false).default_width(width)
            .default_pos(egui::pos2((ctx.content_rect().width()-width)/2.0, ((ctx.content_rect().height()-if width < 560.0 {560.0} else {450.0})/2.0).max(20.0)))
            .show(ctx, |ui| {
                ui.set_width(width);
                egui::Frame::new().inner_margin(16.0).show(ui, |ui| {
                    ui.label(RichText::new("Appearance").size(15.0).strong());
                    ui.add_space(3.0);
                    ui.label(RichText::new("Choose a theme for all Rustable windows.").size(12.0).weak());
                    ui.add_space(14.0);
                    let columns = if ui.available_width() >= 530.0 { 5 } else { 3 };
                    let card_width = (ui.available_width() - 10.0*(columns-1) as f32)/columns as f32;
                    let before = self.prefs.appearance;
                    for choices in Appearance::ALL.chunks(columns) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            for &choice in choices {
                                if theme_card(ui, choice, self.prefs.appearance == choice, card_width).clicked() { self.prefs.appearance = choice; }
                            }
                        });
                        ui.add_space(8.0);
                    }
                    if before != self.prefs.appearance { self.shared_preferences_dirty = true; apply_theme(ctx, self.prefs.appearance); }
                    if self.prefs.appearance == Appearance::System { ui.label(RichText::new("Automatically follows your system appearance.").size(11.0).weak()); }
                    else { ui.label(RichText::new("Theme applies to every window.").size(11.0).weak()); }
                    ui.add_space(22.0);
                    ui.label(RichText::new("Security").size(15.0).strong());
                    ui.add_space(10.0);
                    egui::Frame::new().fill(ui.visuals().faint_bg_color).stroke(ui.visuals().widgets.noninteractive.bg_stroke).corner_radius(8).inner_margin(14.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let available = ui.available_width();
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0,36.0),egui::Sense::hover());
                            crate::icons::paint(ui.painter(),rect.center(),"lock",19.0,ui.visuals().text_color());
                            ui.allocate_ui_with_layout(egui::vec2((available-90.0).max(110.0), 36.0),egui::Layout::top_down(egui::Align::Min), |ui| {
                                ui.set_width((available-90.0).max(110.0));
                                ui.label(RichText::new("Unlock with Touch ID").size(13.0).strong());
                                ui.label(RichText::new("Authenticate before sending protected queries.").size(11.0).weak());
                            });
                            #[cfg(target_os = "macos")]
                            if security_switch(ui, &mut self.prefs.touch_id).changed() { self.shared_preferences_dirty = true; }
                            #[cfg(not(target_os = "macos"))]
                            ui.weak("macOS");
                        });
                        ui.add_space(10.0); ui.separator(); ui.add_space(8.0);
                        ui.label(RichText::new("You can also use your database password. Choose a protection mode from the lock menu in each connection.").size(12.0).weak());
                    });
                    ui.add_space(4.0);
                });
            });
        self.settings_open = open;
    }
}

fn theme_card(ui: &mut egui::Ui, choice: Appearance, selected: bool, width: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 106.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            choice.label(),
        )
    });
    let outline = if selected {
        egui::Stroke::new(2.0_f32, ui.visuals().selection.bg_fill)
    } else {
        ui.style().interact(&response).bg_stroke
    };
    ui.painter().rect(
        rect,
        8.0,
        ui.visuals().extreme_bg_color,
        outline,
        egui::StrokeKind::Inside,
    );
    let preview = egui::Rect::from_min_max(
        rect.min + egui::vec2(7.0, 7.0),
        egui::pos2(rect.right() - 7.0, rect.top() + 69.0),
    );
    let dark = matches!(choice, Appearance::Dark | Appearance::SolarizedDark);
    let solar = choice.solarized();
    let background = match (dark, solar) {
        (true, true) => Color32::from_rgb(0, 43, 54),
        (false, true) => Color32::from_rgb(253, 246, 227),
        (true, false) => Color32::from_gray(32),
        (false, false) => Color32::from_gray(248),
    };
    let side = match (dark, solar) {
        (true, true) => Color32::from_rgb(7, 54, 66),
        (false, true) => Color32::from_rgb(238, 232, 213),
        (true, false) => Color32::from_gray(46),
        (false, false) => Color32::from_gray(229),
    };
    ui.painter().rect_filled(preview, 4.0, background);
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            preview.min,
            egui::pos2(preview.left() + preview.width() * 0.26, preview.bottom()),
        ),
        4.0,
        side,
    );
    ui.painter().rect_filled(
        egui::Rect::from_min_size(preview.min, egui::vec2(preview.width(), 10.0)),
        4.0,
        side,
    );
    for line in 0..4 {
        let color = if line % 2 == 0 {
            if solar {
                Color32::from_rgb(38, 139, 210)
            } else {
                Color32::from_rgb(106, 148, 213)
            }
        } else if dark {
            Color32::from_gray(103)
        } else {
            Color32::from_gray(160)
        };
        let x = preview.left() + preview.width() * 0.35;
        let y = preview.top() + 20.0 + line as f32 * 8.0;
        ui.painter().line_segment(
            [
                egui::pos2(x, y),
                egui::pos2(
                    x + preview.width() * if line % 2 == 0 { 0.48 } else { 0.33 },
                    y,
                ),
            ],
            egui::Stroke::new(2.5_f32, color),
        );
    }
    if choice == Appearance::System {
        let half =
            egui::Rect::from_min_max(egui::pos2(preview.center().x, preview.top()), preview.max);
        ui.painter().rect_filled(half, 4.0, Color32::from_gray(37));
        for line in 0..4 {
            let y = preview.top() + 20.0 + line as f32 * 8.0;
            ui.painter().line_segment(
                [
                    egui::pos2(half.left() + 4.0, y),
                    egui::pos2(half.right() - 6.0, y),
                ],
                egui::Stroke::new(2.5_f32, Color32::from_rgb(106, 148, 213)),
            );
        }
    }
    ui.painter().text(
        egui::pos2(rect.center().x, rect.bottom() - 18.0),
        egui::Align2::CENTER_CENTER,
        choice.label(),
        egui::FontId::proportional(11.0),
        ui.visuals().text_color(),
    );
    if selected {
        let center = egui::pos2(preview.right() - 7.0, preview.top() + 7.0);
        ui.painter()
            .circle_filled(center, 7.0, ui.visuals().selection.bg_fill);
        crate::icons::paint(
            ui.painter(),
            center,
            "check",
            11.0,
            ui.visuals().selection.stroke.color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(target_os = "macos")]
fn security_switch(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(34.0, 20.0), egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            ui.is_enabled(),
            *on,
            "Unlock Safe Mode with Touch ID",
        )
    });
    let fill = if *on {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };
    ui.painter().rect(
        rect,
        10.0,
        fill,
        ui.style().interact(&response).bg_stroke,
        egui::StrokeKind::Inside,
    );
    let center = egui::pos2(
        if *on {
            rect.right() - 10.0
        } else {
            rect.left() + 10.0
        },
        rect.center().y,
    );
    ui.painter().circle_filled(
        center,
        7.0,
        if *on {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().text_color()
        },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
