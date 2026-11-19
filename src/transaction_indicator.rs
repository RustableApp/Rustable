//! Transaction presentation inside the existing connection status capsule.
use super::*;

pub(super) fn guidance(state: db::TransactionState) -> &'static str {
    match state {
        db::TransactionState::Active => "\nTransaction open: COMMIT to keep changes or ROLLBACK to discard them. Switching SQL tabs does not end the transaction.",
        db::TransactionState::Failed => "\nTransaction failed: ROLLBACK required before continuing.",
        _ => "",
    }
}

pub(super) fn title(state: db::TransactionState, compact: bool) -> &'static str {
    match (state, compact) {
        (db::TransactionState::Failed, false) => "TRANSACTION FAILED",
        (db::TransactionState::Failed, true) => "TX FAILED",
        (_, false) => "TRANSACTION OPEN",
        (_, true) => "TX OPEN",
    }
}

pub(super) fn paint(ui: &egui::Ui, rect: egui::Rect, state: db::TransactionState, details: &str, timing: &str) {
    let failed = state == db::TransactionState::Failed;
    let dark = ui.visuals().dark_mode;
    let accent = if failed { ui.visuals().error_fg_color } else { ui.visuals().warn_fg_color };
    let background = match (dark, failed) {
        (true, false) => Color32::from_rgb(57, 42, 17),
        (false, false) => Color32::from_rgb(255, 242, 216),
        (true, true) => Color32::from_rgb(64, 26, 28),
        (false, true) => Color32::from_rgb(255, 231, 231),
    };
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, rect.height() / 2.0, background);
    painter.rect_stroke(rect, rect.height() / 2.0, Stroke::new(1.0_f32, accent.gamma_multiply(0.7)), egui::StrokeKind::Inside);
    let ink = if dark { accent } else { Color32::from_rgb(87, 48, 12) };
    crate::icons::paint(&painter, rect.left_center() + egui::vec2(16.0, 0.0), if failed { "circle-alert" } else { "lock" }, 14.0, accent);
    let show_timing = rect.width() >= 300.0 && !timing.is_empty();
    let end = rect.right() - if show_timing { 86.0 } else { 8.0 };
    let text = egui::Rect::from_min_max(rect.min + egui::vec2(30.0, 0.0), egui::pos2(end.max(rect.left() + 30.0), rect.bottom()));
    let labels = painter.with_clip_rect(text.intersect(rect));
    labels.text(text.left_top() + egui::vec2(0.0, 3.0), egui::Align2::LEFT_TOP,
        title(state, rect.width() < 250.0), egui::FontId::monospace(11.0), ink);
    let detail = if failed { if rect.width() < 190.0 { "ROLLBACK" } else { "ROLLBACK required" } } else if rect.width() >= 400.0 && !details.is_empty() {
        details
    } else if rect.width() < 190.0 { "Uncommitted" } else { "COMMIT / ROLLBACK" };
    labels.text(text.left_bottom() - egui::vec2(0.0, 3.0), egui::Align2::LEFT_BOTTOM,
        detail, egui::FontId::monospace(10.0), ui.visuals().text_color());
    if show_timing {
        painter.text(rect.right_center() - egui::vec2(11.0, 0.0), egui::Align2::RIGHT_CENTER,
            timing, egui::FontId::monospace(11.0), ui.visuals().text_color());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_rect(shape: &egui::Shape, title: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(text) if (text.galley.text() == title || text.galley.text() == title.replace("TRANSACTION", "TX")
                || (title == "ROLLBACK required" && text.galley.text() == "ROLLBACK")) =>
                Some(egui::Rect::from_min_size(text.pos, text.galley.size())),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_rect(shape, title)),
            _ => None,
        }
    }

    #[test]
    fn toolbar_tracks_real_transactions_across_queries_errors_tabs_commit_and_rollback() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        app.worker.send(Command::Open(None)).unwrap();
        app.worker.rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap().unwrap();
        for (sql, open) in [("SELECT 'BEGIN'", false), ("BEGIN", true), ("SELECT 1", true),
            ("SELECT * FROM missing_table", true), ("ROLLBACK", false), ("BEGIN", true), ("COMMIT", false),
            ("SAVEPOINT edit_scope", true), ("RELEASE edit_scope", false)] {
            app.worker.send(Command::Query { sql: sql.into(), safe: false }).unwrap();
            let result = app.worker.rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            let Payload::Query { report: result, .. } = result.unwrap() else { panic!("expected SQL result"); };
            assert_eq!(result.error.is_some(), sql.contains("missing_table"));
            app.new_tab();
            for width in [500.0, 760.0, 1280.0] {
                for dark in [true, false] {
                    apply_theme(&ctx, if dark { Appearance::Dark } else { Appearance::Light });
                    let output = ctx.run(egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 760.0))),
                        ..Default::default()
                    }, |ctx| {
                        app.toolbar(ctx);
                        assert!(ctx.available_rect().top() <= 46.0, "transaction status must not add a toolbar row");
                    });
                    let title = output.shapes.iter().find_map(|shape| text_rect(&shape.shape, "TRANSACTION OPEN"));
                    assert_eq!(title.is_some(), open, "{sql} at {width}/{dark}");
                    if let Some(rect) = title { assert!(rect.left() >= 0.0 && rect.right() <= width); }
                }
            }
        }
    }

    #[test]
    fn failed_transaction_remains_visible_while_busy_in_compact_and_wide_toolbars() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        app.pending = Some(Pending::Query(0));
        for width in [500.0, 760.0, 1280.0] {
            let output = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 760.0))),
                ..Default::default()
            }, |ctx| app.toolbar_with_transaction(ctx, db::TransactionState::Failed));
            for title in ["TRANSACTION FAILED", "ROLLBACK required"] {
                assert!(output.shapes.iter().any(|shape| text_rect(&shape.shape, title).is_some()));
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn transaction_status_survives_ai_takeover_and_clears_after_session_loss() {
        let ctx = egui::Context::default();
        let mut app = Rustable::with_optional_connection(&ctx, Preferences::default(), None, None);
        app.offer_matrix();
        app.matrix.as_mut().unwrap().frame_locked = true;
        for width in [500.0, 760.0, 1280.0] {
            for state in [db::TransactionState::Active, db::TransactionState::Disconnected, db::TransactionState::Idle] {
                for frame in 0..2 {
                let output = ctx.run(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 760.0))),
                    ..Default::default()
                }, |ctx| app.toolbar_with_transaction(ctx, state));
                if frame == 1 { assert_eq!(output.shapes.iter().any(|shape| text_rect(&shape.shape, "TRANSACTION OPEN").is_some()),
                    state == db::TransactionState::Active); }
                }
            }
        }
    }

    #[test]
    fn transaction_capsule_settles_without_repainting() {
        for state in [db::TransactionState::Active, db::TransactionState::Failed] {
            let ctx = egui::Context::default();
            for frame in 0..40 {
                let output = ctx.run(egui::RawInput {
                    time: Some(frame as f64), ..Default::default()
                }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(600.0, 32.0), egui::Sense::hover());
                    paint(ui, rect, state, "LOCAL · SQLite · test", "Ready");
                }); });
                if frame == 39 {
                    assert!(output.viewport_output.values().all(|v| v.repaint_delay > std::time::Duration::from_secs(1)));
                }
            }
        }
    }
}
