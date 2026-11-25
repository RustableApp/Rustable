//! Shared pointer/selection regressions. No windows or databases are opened.
use eframe::egui::{self, Context, Event, Modifiers, PointerButton as Button, Pos2, RawInput};

fn button(pos: Pos2, pressed: bool, button: Button) -> Event {
    Event::PointerButton {
        pos,
        pressed,
        button,
        modifiers: Modifiers::NONE,
    }
}

fn frame(ctx: &Context, time: f64, events: Vec<Event>) -> (u32, u32) {
    let _ = ctx.run(
        RawInput {
            time: Some(time),
            events,
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(500.0, 200.0),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for index in 0..2 {
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(index as f32 * 100.0, 0.0),
                        egui::vec2(100.0, 100.0),
                    );
                    ui.interact(rect, egui::Id::new(index), egui::Sense::click_and_drag());
                }
            });
        },
    );
    ctx.interaction_snapshot(|i| (i.press_count, i.click_count))
}

fn counts(clicks: &[(f64, f32, Button, Option<u32>)]) -> Vec<u32> {
    let ctx = Context::default();
    ctx.options_mut(|o| o.input_options.max_double_click_delay = 0.5);
    frame(&ctx, 0.0, vec![]);
    clicks
        .iter()
        .map(|&(time, x, button_kind, native)| {
            let pos = Pos2::new(x, 30.0);
            let mut events = vec![Event::PointerMoved(pos)];
            if let Some(count) = native {
                events.push(Event::PointerClickCount(count));
            }
            events.push(button(pos, true, button_kind));
            let (press, _) = frame(&ctx, time + 1.0, events);
            let (_, release) = frame(&ctx, time + 1.02, vec![button(pos, false, button_kind)]);
            assert_eq!(press, release, "release retains the press classification");
            release
        })
        .collect()
}

#[test]
fn every_gap_must_be_valid_and_sequences_do_not_stick_at_triple() {
    for (times, expected) in [
        (vec![0.0, 0.4, 0.8], vec![1, 2, 3]),
        (vec![0.0, 0.1, 0.8], vec![1, 2, 1]),
        (vec![0.0, 0.7, 0.8], vec![1, 1, 2]),
        (vec![0.0, 0.1, 0.2, 0.3, 0.4], vec![1, 2, 3, 4, 5]),
        (vec![0.0, 30.0, 30.4], vec![1, 1, 2]),
    ] {
        assert_eq!(
            counts(
                &times
                    .into_iter()
                    .map(|t| (t, 20.0, Button::Primary, None))
                    .collect::<Vec<_>>()
            ),
            expected
        );
    }
}

#[test]
fn position_button_and_adjacent_widget_changes_restart_sequences() {
    for (second, expected) in [(20.0, 2), (80.0, 1), (101.0, 1)] {
        let first = if second == 101.0 { 99.0 } else { 20.0 };
        assert_eq!(
            counts(&[
                (0.0, first, Button::Primary, None),
                (0.2, second, Button::Primary, None)
            ]),
            [1, expected]
        );
    }
    assert_eq!(
        counts(&[
            (0.0, 20.0, Button::Secondary, None),
            (0.2, 20.0, Button::Primary, None)
        ]),
        [1, 1]
    );
}

#[test]
fn native_counts_override_frame_timing_and_default_distance() {
    // AppKit is authoritative, even when the UI processes events late or the
    // user's system setting accepts a wider radius than egui's fallback.
    assert_eq!(
        counts(&[
            (0.0, 20.0, Button::Primary, Some(1)),
            (2.0, 40.0, Button::Primary, Some(2)),
            (4.0, 40.0, Button::Primary, Some(3))
        ]),
        [1, 2, 3]
    );
    assert_eq!(
        counts(&[
            (0.0, 20.0, Button::Primary, Some(1)),
            (0.1, 20.0, Button::Primary, Some(1))
        ]),
        [1, 1]
    );
    assert_eq!(
        counts(&[
            (0.0, 99.0, Button::Primary, Some(1)),
            (0.1, 101.0, Button::Primary, Some(2)),
            (0.2, 101.0, Button::Primary, Some(3))
        ]),
        [1, 1, 2]
    );
}

#[test]
fn focus_loss_and_drag_break_the_sequence() {
    for focus_loss in [false, true] {
        let ctx = Context::default();
        frame(&ctx, 0.0, vec![]);
        let pos = Pos2::new(20.0, 30.0);
        frame(
            &ctx,
            1.0,
            vec![Event::PointerMoved(pos), button(pos, true, Button::Primary)],
        );
        if focus_loss {
            frame(&ctx, 1.01, vec![Event::WindowFocused(false)]);
            frame(&ctx, 1.02, vec![Event::WindowFocused(true)]);
        } else {
            frame(&ctx, 1.01, vec![Event::PointerMoved(Pos2::new(60.0, 30.0))]);
            frame(
                &ctx,
                1.02,
                vec![button(Pos2::new(60.0, 30.0), false, Button::Primary)],
            );
        }
        assert_eq!(
            frame(
                &ctx,
                1.1,
                vec![Event::PointerMoved(pos), button(pos, true, Button::Primary)]
            )
            .0,
            1
        );
    }
}

fn text_frame(
    ctx: &Context,
    time: f64,
    events: Vec<Event>,
    text: &mut String,
) -> (egui::text::CCursorRange, Pos2, std::sync::Arc<egui::Galley>) {
    let mut result = None;
    let _ = ctx.run(
        RawInput {
            time: Some(time),
            events,
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(500.0, 200.0),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let output = egui::TextEdit::multiline(text)
                    .id_salt("native-click-text")
                    .code_editor()
                    .desired_width(450.0)
                    .show(ui);
                result = Some((
                    output
                        .cursor_range
                        .unwrap_or(egui::text::CCursorRange::one(egui::text::CCursor::new(0))),
                    output.galley_pos,
                    output.galley,
                ));
            });
        },
    );
    result.unwrap()
}

#[test]
fn word_and_line_selection_happen_on_press_and_drag_in_units() {
    for click_count in [2, 3] {
        let ctx = Context::default();
        ctx.set_os(egui::os::OperatingSystem::Mac);
        let mut text = "SELECT alpha FROM beta;\nSELECT gamma;".to_owned();
        let (_, origin, galley) = text_frame(&ctx, 0.0, vec![], &mut text);
        let point = |index| {
            origin
                + galley
                    .pos_from_cursor(egui::text::CCursor::new(index))
                    .center()
                    .to_vec2()
        };
        let pos = point(9);
        for count in 1..=click_count {
            let selection = text_frame(
                &ctx,
                count as f64,
                vec![
                    Event::PointerMoved(pos),
                    Event::PointerClickCount(count),
                    button(pos, true, Button::Primary),
                ],
                &mut text,
            )
            .0
            .as_sorted_char_range();
            if count == 2 {
                assert_eq!(&text[selection.clone()], "alpha");
            }
            if count == 3 {
                assert_eq!(&text[selection.clone()], "SELECT alpha FROM beta;\n");
            }
            if count < click_count {
                text_frame(
                    &ctx,
                    count as f64 + 0.02,
                    vec![button(pos, false, Button::Primary)],
                    &mut text,
                );
            }
        }
        let target = point(if click_count == 2 { 20 } else { 30 });
        let selection = text_frame(
            &ctx,
            click_count as f64 + 0.1,
            vec![Event::PointerMoved(target)],
            &mut text,
        )
        .0
        .as_sorted_char_range();
        assert_eq!(
            &text[selection.clone()],
            if click_count == 2 {
                "alpha FROM beta"
            } else {
                "SELECT alpha FROM beta;\nSELECT gamma;"
            }
        );
        let released = text_frame(
            &ctx,
            click_count as f64 + 0.12,
            vec![button(target, false, Button::Primary)],
            &mut text,
        )
        .0
        .as_sorted_char_range();
        assert_eq!(released, selection);
    }
}

#[test]
fn secondary_click_preserves_unicode_word_selection() {
    let ctx = Context::default();
    let mut text = "SELECT café FROM customers;".to_owned();
    let (_, origin, galley) = text_frame(&ctx, 0.0, vec![], &mut text);
    let point = |index| {
        origin
            + galley
                .pos_from_cursor(egui::text::CCursor::new(index))
                .center()
                .to_vec2()
    };
    for count in [1, 2] {
        text_frame(
            &ctx,
            count as f64,
            vec![
                Event::PointerMoved(point(9)),
                Event::PointerClickCount(count),
                button(point(9), true, Button::Primary),
            ],
            &mut text,
        );
        text_frame(
            &ctx,
            count as f64 + 0.02,
            vec![button(point(9), false, Button::Primary)],
            &mut text,
        );
    }
    let range = text_frame(
        &ctx,
        3.0,
        vec![
            Event::PointerMoved(point(20)),
            button(point(20), true, Button::Secondary),
        ],
        &mut text,
    )
    .0
    .as_sorted_char_range();
    assert_eq!(
        text.chars()
            .skip(range.start)
            .take(range.len())
            .collect::<String>(),
        "café"
    );
    text_frame(
        &ctx,
        3.02,
        vec![button(point(20), false, Button::Secondary)],
        &mut text,
    );
}

#[test]
fn native_first_event_can_already_be_double_or_triple() {
    for count in [2, 3] {
        assert_eq!(
            counts(&[
                (0.0, 20.0, Button::Primary, Some(count)),
                (3.0, 20.0, Button::Primary, Some(count))
            ]),
            [count, count]
        );
    }
    assert_eq!(
        counts(&[
            (0.0, 20.0, Button::Secondary, Some(1)),
            (0.1, 20.0, Button::Primary, Some(2))
        ]),
        [1, 1]
    );
}
