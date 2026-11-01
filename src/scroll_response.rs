//! Deliver native wheel/trackpad movement in its originating frame.
//!
//! Egui 0.33 smooths point events >= 8 points as well as line/page events.
//! Fast macOS trackpad movement already includes OS momentum: smoothing it a
//! second time adds latency. Keep native deltas (including momentum) unchanged.
//! Egui's private accumulator may request residual repaints, but must never
//! produce additional visible movement after the last native event.
use eframe::egui::{self, Event, MouseWheelUnit, Vec2};

pub fn install(ctx: &egui::Context) {
    ctx.on_begin_pass(
        "direct native scrolling",
        std::sync::Arc::new(|ctx| {
            let options = ctx.options(|options| options.input_options);
            ctx.input_mut(|input| {
                input.smooth_scroll_delta =
                    direct_delta(&input.events, options, input.viewport_rect().height());
            });
        }),
    );
}

fn direct_delta(events: &[Event], options: egui::InputOptions, viewport_height: f32) -> Vec2 {
    events.iter().fold(Vec2::ZERO, |total, event| {
        let Event::MouseWheel {
            unit,
            delta,
            modifiers,
        } = event
        else {
            return total;
        };
        // Preserve egui's zoom path; raw_scroll_delta itself also includes zoom,
        // so copying it directly would incorrectly pan while zooming.
        if modifiers.matches_any(options.zoom_modifier) {
            return total;
        }
        let mut delta = match unit {
            MouseWheelUnit::Point => *delta,
            MouseWheelUnit::Line => *delta * options.line_scroll_speed,
            MouseWheelUnit::Page => *delta * viewport_height,
        };
        let horizontal = modifiers.matches_any(options.horizontal_scroll_modifier);
        let vertical = modifiers.matches_any(options.vertical_scroll_modifier);
        if horizontal && !vertical {
            delta = egui::vec2(delta.x + delta.y, 0.0);
        }
        if vertical && !horizontal {
            delta = egui::vec2(0.0, delta.x + delta.y);
        }
        total + delta
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wheel(unit: MouseWheelUnit, delta: Vec2, modifiers: egui::Modifiers) -> Event {
        Event::MouseWheel {
            unit,
            delta,
            modifiers,
        }
    }
    fn frame(ctx: &egui::Context, events: Vec<Event>, time: f64) -> Vec2 {
        let mut observed = Vec2::ZERO;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                time: Some(time),
                ..Default::default()
            },
            |ctx| {
                observed = ctx.input(|i| i.smooth_scroll_delta);
            },
        );
        observed
    }

    #[test]
    fn wheel_movement_is_complete_in_first_frame_with_no_tail() {
        let ctx = egui::Context::default();
        install(&ctx);
        assert_eq!(
            frame(
                &ctx,
                vec![wheel(
                    MouseWheelUnit::Line,
                    egui::vec2(0.0, -3.0),
                    egui::Modifiers::NONE
                )],
                0.0
            ),
            egui::vec2(0.0, -120.0)
        );
        for index in 1..30 {
            assert_eq!(frame(&ctx, vec![], index as f64 / 144.0), Vec2::ZERO);
        }
    }

    #[test]
    fn native_trackpad_momentum_is_not_delayed_or_multiplied() {
        let ctx = egui::Context::default();
        install(&ctx);
        // Includes large finger movement and diminishing native momentum.
        for (index, movement) in [36.0, 24.0, 12.0, 7.0, 2.0, 0.25, 0.0]
            .into_iter()
            .enumerate()
        {
            let delta = egui::vec2(movement * 0.5, -movement);
            assert_eq!(
                frame(
                    &ctx,
                    vec![wheel(MouseWheelUnit::Point, delta, egui::Modifiers::NONE)],
                    index as f64 / 144.0
                ),
                delta
            );
        }
        assert_eq!(frame(&ctx, vec![], 0.1), Vec2::ZERO);
    }

    #[test]
    fn modifiers_page_units_and_zoom_keep_their_meaning() {
        let ctx = egui::Context::default();
        install(&ctx);
        assert_eq!(
            frame(
                &ctx,
                vec![wheel(
                    MouseWheelUnit::Page,
                    egui::vec2(0.0, -1.0),
                    egui::Modifiers::SHIFT
                )],
                0.0
            ),
            egui::vec2(-600.0, 0.0)
        );
        assert_eq!(
            frame(
                &ctx,
                vec![wheel(
                    MouseWheelUnit::Point,
                    egui::vec2(12.0, -2.0),
                    egui::Modifiers::ALT
                )],
                0.1
            ),
            egui::vec2(0.0, 10.0)
        );
        assert_eq!(
            frame(
                &ctx,
                vec![wheel(
                    MouseWheelUnit::Point,
                    egui::vec2(0.0, 3.0),
                    egui::Modifiers::COMMAND
                )],
                0.2
            ),
            Vec2::ZERO
        );
        assert_ne!(ctx.input(|input| input.zoom_delta()), 1.0);
    }
}
