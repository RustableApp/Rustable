//! Normalize macOS Control-click before egui derives click/drag state.
//! A context plugin also covers immediate child viewports, unlike App::raw_input_hook.
use eframe::egui::{self, Event, PointerButton, RawInput, ViewportId};
use std::collections::HashMap;

pub fn install(ctx: &egui::Context) {
    if ctx.os() == egui::os::OperatingSystem::Mac {
        #[cfg(target_os = "macos")]
        apply_double_click_interval(ctx, objc2_app_kit::NSEvent::doubleClickInterval());
        // egui registers each plugin type only once per shared context.
        ctx.add_plugin(ControlClick::default());
    }
}

#[cfg(any(target_os = "macos", test))]
fn apply_double_click_interval(ctx: &egui::Context, seconds: f64) {
    if seconds.is_finite() && seconds > 0.0 {
        ctx.options_mut(|options| options.input_options.max_double_click_delay = seconds);
    }
}

#[derive(Default)]
struct ControlClick {
    /// Remember the button chosen on press, even if Control is released first.
    primary_presses: HashMap<ViewportId, bool>,
}

impl egui::Plugin for ControlClick {
    fn debug_name(&self) -> &'static str { "macOS Control-click" }

    fn input_hook(&mut self, input: &mut RawInput) {
        self.primary_presses.retain(|id, _| input.viewports.contains_key(id));
        for event in &mut input.events {
            match event {
                Event::PointerButton { button, pressed, modifiers, .. }
                    if *button == PointerButton::Primary => {
                    let secondary = if *pressed {
                        self.primary_presses.insert(input.viewport_id, modifiers.ctrl);
                        modifiers.ctrl
                    } else {
                        self.primary_presses.remove(&input.viewport_id).unwrap_or(false)
                    };
                    if secondary { *button = PointerButton::Secondary; }
                }
                Event::WindowFocused(false) => { self.primary_presses.remove(&input.viewport_id); }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Modifiers, Pos2};

    fn click(pressed: bool, modifiers: Modifiers) -> Event {
        Event::PointerButton { pos: Pos2::new(20.0, 20.0), button: PointerButton::Primary, pressed, modifiers }
    }

    #[test]
    fn native_double_click_interval_accepts_normal_pace_and_rejects_slow_clicks() {
        for (gap, expected) in [(0.4, true), (0.6, false)] {
            let ctx = egui::Context::default();
            apply_double_click_interval(&ctx, 0.5);
            let mut doubled = false;
            for (time, pressed) in [(0.0, true), (0.02, false), (gap, true), (gap + 0.02, false)] {
                let _ = ctx.run(egui::RawInput { time: Some(time), events: vec![click(pressed, Modifiers::NONE)], ..Default::default() }, |ctx| {
                    doubled |= ctx.input(|input| input.pointer.button_double_clicked(PointerButton::Primary));
                });
            }
            assert_eq!(doubled, expected);
        }
    }

    #[test]
    fn control_click_opens_context_menu_without_primary_action() {
        for os in [egui::os::OperatingSystem::Mac, egui::os::OperatingSystem::Windows, egui::os::OperatingSystem::Nix] {
            let ctx = egui::Context::default();
            ctx.set_os(os);
            install(&ctx);
            let mut point = Pos2::ZERO;
            let mut primary = false;
            let mut menu = false;
            for phase in 0..4 {
                let mut events = vec![Event::PointerMoved(point)];
                if phase >= 2 {
                    events.push(Event::PointerButton { pos: point, button: PointerButton::Primary,
                        pressed: phase == 2, modifiers: if phase == 2 { Modifiers::CTRL } else { Modifiers::NONE } });
                }
                let _ = ctx.run(RawInput { events, ..Default::default() }, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let response = ui.button("Context target");
                        point = response.rect.center();
                        primary |= response.clicked();
                        response.context_menu(|ui| { menu = true; ui.label("Menu action"); });
                    });
                });
            }
            assert_eq!(menu, os == egui::os::OperatingSystem::Mac, "{os:?}");
            assert_eq!(primary, os != egui::os::OperatingSystem::Mac, "{os:?}");
            assert!(!ctx.input(|i| i.pointer.any_down()));
        }
    }

    #[test]
    fn pressing_control_after_mouse_down_does_not_change_button() {
        let mut plugin = ControlClick::default();
        let mut input = RawInput { events: vec![click(true, Modifiers::NONE), click(false, Modifiers::CTRL)], ..Default::default() };
        egui::Plugin::input_hook(&mut plugin, &mut input);
        assert!(input.events.iter().all(|e| matches!(e, Event::PointerButton { button: PointerButton::Primary, .. })));
    }

    #[test]
    fn child_viewport_state_is_independent_and_closed_windows_are_pruned() {
        use egui::Plugin;
        let child = ViewportId::from_hash_of("database");
        let mut input = RawInput::default();
        input.viewports.insert(child, Default::default());
        let mut plugin = ControlClick::default();
        input.events = vec![click(true, Modifiers::CTRL)];
        plugin.input_hook(&mut input);
        input.viewport_id = child;
        input.events = vec![click(true, Modifiers::NONE), click(false, Modifiers::NONE)];
        plugin.input_hook(&mut input);
        assert!(input.events.iter().all(|e| matches!(e, Event::PointerButton { button: PointerButton::Primary, .. })));
        input.viewport_id = ViewportId::ROOT;
        input.events = vec![click(false, Modifiers::NONE)];
        plugin.input_hook(&mut input);
        assert!(matches!(input.events[0], Event::PointerButton { button: PointerButton::Secondary, .. }));
        input.viewport_id = child;
        input.events = vec![click(true, Modifiers::CTRL)];
        plugin.input_hook(&mut input);
        input.viewports.remove(&child);
        input.events.clear();
        plugin.input_hook(&mut input);
        assert!(plugin.primary_presses.is_empty());
    }

    #[test]
    fn native_secondary_clicks_and_command_clicks_are_preserved() {
        use egui::Plugin;
        let mut plugin = ControlClick::default();
        let mut input = RawInput { events: vec![
            Event::PointerButton { pos: Pos2::ZERO, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::CTRL },
            click(true, Modifiers::MAC_CMD), click(false, Modifiers::MAC_CMD),
        ], ..Default::default() };
        let original = input.events.clone();
        plugin.input_hook(&mut input);
        assert_eq!(input.events, original);
        input.events = vec![Event::WindowFocused(false)];
        plugin.input_hook(&mut input);
        assert!(plugin.primary_presses.is_empty());
    }
    /// Identical frame workload with the normalizer disabled/enabled. Complements
    /// the base-commit suite with the new input path (including idle frames).
    #[test]
    #[ignore = "local release-mode context-click CPU benchmark"]
    fn context_click_frame_benchmark() {
        for pair in 0..5 {
            for enabled in if pair % 2 == 0 { [false, true] } else { [true, false] } {
                let ctx = egui::Context::default();
                ctx.set_os(egui::os::OperatingSystem::Mac);
                if enabled { install(&ctx); }
                let mut samples = Vec::new();
                for frame in 0..620 {
                    let events = match frame % 6 {
                        0 => vec![Event::PointerMoved(Pos2::new(20.0, 20.0))],
                        1 => vec![click(true, Modifiers::CTRL)],
                        2 => vec![click(false, Modifiers::NONE)],
                        _ => vec![],
                    };
                    let input = RawInput { events, time: Some(frame as f64 / 120.0),
                        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
                        ..Default::default() };
                    let start = std::time::Instant::now();
                    let output = ctx.run(input, |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| { ui.label("Input dispatch fixture"); });
                    });
                    std::hint::black_box(ctx.tessellate(output.shapes, output.pixels_per_point));
                    if frame >= 20 { samples.push(start.elapsed().as_secs_f64() * 1000.0); }
                }
                samples.sort_by(f64::total_cmp);
                crate::ui_benchmark::report(&format!("context-click/{pair}/{enabled}"), &samples);
            }
        }
    }

}
