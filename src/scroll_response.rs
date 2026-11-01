//! Small, immediate wheel steps; precise native trackpad movement stays unchanged.
//!
//! Egui 0.33 smooths point events >= 8 points as well as line/page events.
//! Fast macOS trackpad movement already includes OS momentum: smoothing it a
//! second time adds latency. Keep native deltas (including momentum) unchanged.
//! Clear egui's interpolation accumulator as well as replacing the delta so
//! there is neither residual movement nor invisible smoothing repaint work.
use eframe::egui::{self, Event, MouseWheelUnit, Vec2};

// Logical points per reported wheel line, not physical Retina pixels. Egui's
// default is 40; half that keeps a tick small without adding an animated tail.
const WHEEL_LINE_POINTS: f32 = 20.0;

pub fn install(ctx: &egui::Context) {
    ctx.options_mut(|options| options.input_options.line_scroll_speed = WHEEL_LINE_POINTS);
    ctx.on_begin_pass(
        "direct native scrolling",
        std::sync::Arc::new(|ctx| {
            let options = ctx.options(|options| options.input_options);
            ctx.input_mut(|input| {
                let delta = direct_delta(&input.events, options, input.viewport_rect().height());
                input.set_scroll_delta_without_smoothing(delta);
            });
        }),
    );
    #[cfg(not(target_arch = "wasm32"))]
    install_trace(ctx);
}

/// Optional hardware-input evidence. No SQL, cell contents or connection details.
#[cfg(not(target_arch = "wasm32"))]
fn install_trace(ctx: &egui::Context) {
    use std::{io::Write, sync::mpsc, time::Instant};
    let Some(path) = std::env::var_os("RUSTABLE_SCROLL_TRACE") else {
        return;
    };
    let file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(error) => {
            eprintln!("Cannot create scroll trace: {error}");
            return;
        }
    };
    let (sender, receiver) = mpsc::sync_channel::<serde_json::Value>(1024);
    std::thread::spawn(move || {
        let mut writer = std::io::BufWriter::new(file);
        for record in receiver {
            if serde_json::to_writer(&mut writer, &record).is_err()
                || writeln!(writer).is_err()
                || writer.flush().is_err()
            {
                break;
            }
        }
    });
    let origin = Instant::now();
    ctx.on_begin_pass(
        "scroll trace start",
        std::sync::Arc::new(move |ctx| {
            let id = egui::Id::new(("scroll-trace", ctx.viewport_id()));
            let events: Vec<_> = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|event| {
                        if let Event::MouseWheel {
                            unit,
                            delta,
                            modifiers,
                        } = event
                        {
                            Some(serde_json::json!({"unit":format!("{unit:?}"), "x":delta.x,
                    "y":delta.y, "shift":modifiers.shift, "alt":modifiers.alt,
                    "command":modifiers.command}))
                        } else {
                            None
                        }
                    })
                    .collect()
            });
            let now = origin.elapsed().as_secs_f64();
            let active = ctx.data_mut(|data| {
                if !events.is_empty() {
                    data.insert_temp(id.with("last-input"), now);
                }
                data.get_temp::<f64>(id.with("last-input"))
                    .is_some_and(|last| now - last < 0.5)
            });
            if active {
                let record = serde_json::json!({"elapsed_s":now, "pass":ctx.current_pass_index(),
                "pixels_per_point":ctx.pixels_per_point(), "events":events});
                ctx.data_mut(|data| data.insert_temp(id, (Instant::now(), record)));
            }
        }),
    );
    ctx.on_end_pass(
        "scroll trace end",
        std::sync::Arc::new(move |ctx| {
            let id = egui::Id::new(("scroll-trace", ctx.viewport_id()));
            if let Some((started, mut record)) = ctx.data_mut(|data| {
                let frame = data.get_temp::<(Instant, serde_json::Value)>(id);
                data.remove::<(Instant, serde_json::Value)>(id);
                frame
            }) {
                record["ui_pass_ms"] = (started.elapsed().as_secs_f64() * 1000.0).into();
                record["discard_requested"] = ctx.will_discard().into();
                // Never block rendering on disk I/O. A saturated diagnostic queue
                // produces gaps, so this trace is not a lossless input recorder.
                let _ = sender.try_send(record);
            }
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
            egui::vec2(0.0, -60.0)
        );
        for index in 1..30 {
            assert_eq!(frame(&ctx, vec![], index as f64 / 144.0), Vec2::ZERO);
        }
    }

    #[test]
    fn content_drag_scrolls_only_with_middle_button() {
        for button in [egui::PointerButton::Primary, egui::PointerButton::Secondary, egui::PointerButton::Middle] {
            let ctx = egui::Context::default();
            let mut offset = Vec2::ZERO;
            let mut render = |events| {
                let _ = ctx.run(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))), events, ..Default::default()
                }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                    offset = egui::ScrollArea::both().show(ui, |ui| { ui.allocate_space(egui::vec2(1000.0, 1000.0)); }).state.offset;
                }); });
            };
            render(vec![]); render(vec![]);
            let start = egui::pos2(180.0, 160.0);
            render(vec![Event::PointerMoved(start), Event::PointerButton { pos: start, button, pressed: true, modifiers: egui::Modifiers::NONE }]);
            render(vec![Event::PointerMoved(start - egui::vec2(40.0, 40.0))]);
            render(vec![Event::PointerMoved(start - egui::vec2(80.0, 80.0))]);
            if button == egui::PointerButton::Middle { assert!(offset.x > 0.0 && offset.y > 0.0, "{offset:?}"); }
            else { assert_eq!(offset, Vec2::ZERO, "{button:?}"); }
        }
    }

    #[test]
    fn wheel_does_not_schedule_invisible_smoothing_frames() {
        let ctx = egui::Context::default();
        install(&ctx);
        for i in 0..3 { frame(&ctx, vec![], i as f64 / 120.0); }
        frame(&ctx, vec![wheel(MouseWheelUnit::Line, egui::vec2(0.0,-3.0), egui::Modifiers::NONE)], 0.1);
        // egui may schedule the usual follow-up frame after input, but there
        // must not be an animation's worth of invisible repaint requests.
        for index in 0..30 {
            let output = ctx.run(egui::RawInput {
                time:Some(0.1 + (index+1) as f64/120.0), ..Default::default()
            }, |_| {});
            if index >= 2 {
                assert_ne!(output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
                    std::time::Duration::ZERO, "invisible wheel smoothing still requests repaint at idle frame {index}");
            }
        }
    }

    #[test]
    fn horizontal_wheel_steps_are_small_and_reverse_without_momentum() {
        let ctx = egui::Context::default();
        install(&ctx);
        for (index, lines) in [-1.0, 1.0, -3.0].into_iter().enumerate() {
            assert_eq!(
                frame(
                    &ctx,
                    vec![wheel(
                        MouseWheelUnit::Line,
                        egui::vec2(0.0, lines),
                        egui::Modifiers::SHIFT
                    )],
                    index as f64 / 120.0
                ),
                egui::vec2(lines * WHEEL_LINE_POINTS, 0.0)
            );
        }
        assert_eq!(frame(&ctx, vec![], 0.1), Vec2::ZERO);
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
