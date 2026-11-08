//! Bounded, worker-formatted approximate similarity results.
pub(crate) struct Match {
    pub element: Vec<u8>,
    pub label: String,
    pub score: String,
}
pub(crate) struct Matches {
    pub key: Vec<u8>,
    pub source: String,
    pub rows: Vec<Match>,
    pub limit: usize,
}
#[cfg(not(target_arch = "wasm32"))]
fn label(bytes: &[u8]) -> String {
    let quoted = crate::redis_commands::quote(bytes);
    let mut chars = quoted.chars();
    let mut short: String = chars.by_ref().take(256).collect();
    if chars.next().is_some() {
        short.push('…');
    }
    short
}
impl Matches {
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub fn decode(value: redis::Value, key: &[u8], source: &[u8]) -> Result<Self, String> {
        Self::decode_limit(value, key, source, 20)
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn decode_limit(value: redis::Value, key: &[u8], source: &[u8], limit: usize) -> Result<Self, String> {
        let redis::Value::Array(values) = value else {
            return Err("Invalid similarity response.".into());
        };
        if !(1..=100).contains(&limit) || values.len() > limit * 2 || !values.len().is_multiple_of(2) {
            return Err("Similarity response exceeds the requested result limit.".into());
        }
        let mut rows = Vec::with_capacity(values.len() / 2);
        let mut previous = f64::INFINITY;
        let mut names = std::collections::HashSet::new();
        let mut values = values.into_iter();
        while let Some(value) = values.next() {
            let element: Vec<u8> = redis::from_redis_value(value).map_err(|e| e.to_string())?;
            let score: f64 =
                redis::from_redis_value(values.next().unwrap()).map_err(|e| e.to_string())?;
            if element.len() > 65536
                || !score.is_finite()
                || !(-0.000001..=1.000001).contains(&score)
                || score > previous + 0.000001
                || !names.insert(element.clone())
            {
                return Err("Invalid, unordered or oversized similarity result.".into());
            }
            previous = score;
            rows.push(Match {
                label: label(&element),
                element,
                score: format!("{score:.4}"),
            });
        }
        Ok(Self {
            key: key.to_vec(),
            source: label(source),
            rows,
            limit,
        })
    }
    pub fn show(&self, ui: &mut eframe::egui::Ui) -> Option<crate::redis_diagnostics::Request> {
        use eframe::egui;
        use egui_extras::{Column, TableBuilder};
        ui.weak("Similar to");
        ui.add(egui::Label::new(egui::RichText::new(&self.source).monospace()).truncate());
        ui.weak(format!("Top {} approximate matches · higher scores mean more similar", self.limit));
        let mut action = None;
        TableBuilder::new(ui).cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .id_salt("vector-matches")
            .striped(true)
            .max_scroll_height(360.0)
            .column(Column::remainder().clip(true))
            .column(Column::exact(80.0))
            .column(Column::exact(90.0))
            .header(28.0, |mut row| {
                for name in ["ELEMENT", "SCORE", ""] {
                    row.col(|ui| {
                        crate::controls::table_header(ui, name);
                    });
                }
            })
            .body(|body| {
                body.rows(28.0, self.rows.len(), |mut row| {
                    let item = &self.rows[row.index()];
                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(&item.label).monospace())
                                .truncate(),
                        );
                    });
                    row.col(|ui| {
                        ui.monospace(&item.score);
                    });
                    row.col(|ui| {
                        if ui
                            .add(crate::redis_workspace::action_button("search", "Inspect"))
                            .clicked()
                        {
                            action = Some(crate::redis_diagnostics::Request::VectorElement {
                                key: self.key.clone(),
                                element: item.element.clone(),
                            });
                        }
                    });
                })
            });
        if self.rows.is_empty() {
            ui.weak("No matches. The key or source element may no longer exist.");
        }
        ui.weak("The source element may be included. Results reflect the last explicit search.");
        action
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    fn reply(rows: &[(&[u8], &str)]) -> redis::Value {
        redis::Value::Array(
            rows.iter()
                .flat_map(|(name, score)| {
                    [
                        redis::Value::BulkString(name.to_vec()),
                        redis::Value::BulkString(score.as_bytes().to_vec()),
                    ]
                })
                .collect(),
        )
    }
    #[test]
    fn scores_names_and_result_count_are_validated() {
        let result = Matches::decode(
            reply(&[(&[0, 255], "1.00000001"), (b"second", "0.5")]),
            b"key",
            b"source",
        )
        .unwrap();
        assert_eq!(result.rows[0].element, [0, 255]);
        assert_eq!(result.rows[1].score, "0.5000");
        for invalid in [
            reply(&[(b"a", "NaN")]),
            reply(&[(b"a", "2")]),
            reply(&[(b"a", "0.2"), (b"b", "0.8")]),
            reply(&[(b"a", "1"), (b"a", "1")]),
            reply(&[(&vec![0; 65537], "1")]),
            reply(&[(b"a".as_slice(), "1"); 21]),
        ] {
            assert!(Matches::decode(invalid, b"key", b"source").is_err());
        }
    }
    #[test]
    #[ignore = "manual similarity results CPU workload"]
    fn redis_vector_matches_frame_benchmark() {
        use eframe::egui;
        let names: Vec<_> = (0..20)
            .map(|i| format!("product:{i:03}:{}", "x".repeat(300)))
            .collect();
        let scores: Vec<_> = (0..20)
            .map(|i| format!("{}", 1.0 - i as f64 / 40.0))
            .collect();
        let rows: Vec<_> = names
            .iter()
            .zip(&scores)
            .map(|(name, score)| (name.as_bytes(), score.as_str()))
            .collect();
        let matches = Matches::decode(reply(&rows), b"vectors", b"source").unwrap();
        let ctx = egui::Context::default();
        let mut samples = Vec::new();
        for frame in 0..180 {
            let start = std::time::Instant::now();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 900.0),
                    )),
                    time: Some(frame as f64 / 120.0),
                    events: vec![
                        egui::Event::PointerMoved(egui::pos2(350.0, 250.0)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, if frame % 40 < 20 { -18.0 } else { 18.0 }),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ctx| {
                    egui::Window::new("Similar elements")
                        .fixed_pos(egui::pos2(20.0, 20.0))
                        .default_width(640.0)
                        .show(ctx, |ui| {
                            crate::redis_workspace::browser_style(ui);
                            let _ = matches.show(ui);
                        });
                },
            );
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/vector-matches", &samples);
    }
}
