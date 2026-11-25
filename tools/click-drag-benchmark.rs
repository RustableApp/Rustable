//! Standalone shared egui workload; link unchanged against base/candidate rlibs.
//! See docs/UI-PERFORMANCE.md. This intentionally uses only their common API.
use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput};
fn main() {
    for clicks in [2, 3] {
        let ctx = Context::default();
        ctx.set_os(egui::os::OperatingSystem::Mac);
        ctx.options_mut(|o| o.input_options.max_double_click_delay = 0.5);
        let mut sql = "SELECT café, value FROM customers WHERE id = 42;\n".repeat(200);
        let mut samples = Vec::new();
        for frame in 0..620 {
            let phase = frame % 20;
            let pos = if phase < 6 {
                Pos2::new(95.0, 35.0)
            } else {
                Pos2::new(
                    95.0 + (phase - 6) as f32 * 12.0,
                    35.0 + (phase - 6) as f32 * 5.0,
                )
            };
            let mut events = Vec::new();
            if (1..=4).contains(&phase) || (clicks == 3 && phase == 5) {
                // Double-click holds its second press; triple-click releases the
                // second then holds its third. Both drag until phase 15.
                let pressed = phase % 2 == 1;
                if !(phase == 4 && clicks == 2) {
                    events.push(Event::PointerMoved(pos));
                    events.push(Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    });
                }
            }
            if (6..=15).contains(&phase) {
                events.push(Event::PointerMoved(pos));
            }
            if phase == 15 {
                events.push(Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                });
            }
            let start = std::time::Instant::now();
            let output = ctx.run(
                RawInput {
                    events,
                    time: Some(frame as f64 * 0.04 + if frame >= 300 { 30.0 } else { 0.0 }),
                    screen_rect: Some(egui::Rect::from_min_size(
                        Pos2::ZERO,
                        egui::vec2(1000.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let output = egui::TextEdit::multiline(&mut sql)
                            .code_editor()
                            .desired_width(980.0)
                            .show(ui);
                        std::hint::black_box(output.cursor_range);
                    });
                },
            );
            std::hint::black_box(ctx.tessellate(output.shapes, output.pixels_per_point));
            if frame >= 20 && (6..=14).contains(&phase) {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        let n = samples.len();
        println!(
            "RUSTABLE_UI_BENCH {{\"name\":\"click/drag/{clicks}\",\"p50\":{},\"p95\":{},\"p99\":{},\"max\":{},\"samples\":{:?}}}",
            samples[n / 2],
            samples[n * 95 / 100],
            samples[n * 99 / 100],
            samples[n - 1],
            samples
        );
    }
}
