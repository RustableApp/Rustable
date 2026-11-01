//! Use egui's same scroll interpolation on desktop and in the WASM workspace.
//! Keep diagnostics separate from input so tracing never changes scroll behavior.
use eframe::egui;
#[cfg(any(test, not(target_arch = "wasm32")))]
use eframe::egui::Event;
#[cfg(test)]
use eframe::egui::{MouseWheelUnit, Vec2};

pub fn install(_ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    install_trace(_ctx);
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_matches_browser_smoothing_at_each_refresh_rate() {
        for hz in [60.0, 120.0, 144.0] {
            for unit in [MouseWheelUnit::Point, MouseWheelUnit::Line, MouseWheelUnit::Page] {
                for modifiers in [egui::Modifiers::NONE, egui::Modifiers::SHIFT, egui::Modifiers::COMMAND] {
                    let desktop = egui::Context::default();
                    install(&desktop);
                    let browser = egui::Context::default();
                    let mut total = Vec2::ZERO;
                    let mut moving_frames = 0;
                    for frame in 0..180 {
                        let input = egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                            time: Some(frame as f64 / hz),
                            events: if frame == 3 { vec![Event::MouseWheel { unit, delta: egui::vec2(0.0, -1.0), modifiers }] } else { vec![] },
                            ..Default::default()
                        };
                        let mut actual = Vec2::ZERO;
                        let mut expected = Vec2::ZERO;
                        let _ = desktop.run(input.clone(), |ctx| actual = ctx.input(|i| i.smooth_scroll_delta));
                        let _ = browser.run(input, |ctx| expected = ctx.input(|i| i.smooth_scroll_delta));
                        assert_eq!(actual, expected);
                        total += actual;
                        moving_frames += usize::from(actual != Vec2::ZERO);
                        if frame == 179 { assert_eq!(actual, Vec2::ZERO, "must settle at idle"); }
                    }
                    if unit == MouseWheelUnit::Line && modifiers == egui::Modifiers::NONE {
                        assert!((total.y + 40.0).abs() < 0.01);
                        assert!(moving_frames > 1, "wheel ticks should be interpolated");
                    }
                }
            }
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

}
