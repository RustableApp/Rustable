//! Cached, bounded plotting of the loaded time-series page. No network work.
use eframe::egui::{self, Align2, FontId, Stroke};
#[derive(Default)]
pub(crate) struct Chart {
    points: Vec<Option<[f32; 2]>>,
    labels: Vec<String>,
    first: String,
    last: String,
    low: String,
    high: String,
}
impl Chart {
    pub fn new(rows: &[Vec<String>]) -> Self {
        let rows = &rows[..rows.len().min(crate::redis_value::PAGE_SIZE as usize)];
        let times: Vec<Option<u64>> = rows.iter().map(|r| r.first()?.parse().ok()).collect();
        let values: Vec<Option<f64>> = rows
            .iter()
            .map(|r| r.get(1)?.parse::<f64>().ok().filter(|v| v.is_finite()))
            .collect();
        let first = times.iter().flatten().copied().min().unwrap_or(0);
        let last = times.iter().flatten().copied().max().unwrap_or(first);
        let scale = values.iter().flatten().fold(1.0_f64, |s, v| s.max(v.abs()));
        let low = values
            .iter()
            .flatten()
            .map(|v| v / scale)
            .reduce(f64::min)
            .unwrap_or(0.0);
        let high = values
            .iter()
            .flatten()
            .map(|v| v / scale)
            .reduce(f64::max)
            .unwrap_or(0.0);
        Self {
            points: times
                .iter()
                .zip(&values)
                .map(|(t, v)| {
                    let (t, v) = ((*t)?, (*v)?);
                    Some([
                        if first == last {
                            0.5
                        } else {
                            (t.checked_sub(first)? as f64 / (last - first) as f64) as f32
                        },
                        if low == high {
                            0.5
                        } else {
                            ((v / scale - low) / (high - low)) as f32
                        },
                    ])
                })
                .collect(),
            labels: rows
                .iter()
                .map(|r| {
                    format!(
                        "{} ms\n{}",
                        r.first().map(String::as_str).unwrap_or("—"),
                        r.get(1).map(String::as_str).unwrap_or("—")
                    )
                })
                .collect(),
            first: first.to_string(),
            last: last.to_string(),
            low: format!("Min {:.4e}", low * scale),
            high: format!("Max {:.4e}", high * scale),
        }
    }
    pub fn show(&self, ui: &mut egui::Ui, visible: &[usize], focus: bool) {
        ui.weak("Loaded page only · text filter applies · Unix milliseconds");
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(120.0), 220.0),
            egui::Sense::hover(),
        );
        if focus {
            response.scroll_to_me(Some(egui::Align::Center));
        }
        let painter = ui.painter_at(rect);
        let plot = rect.shrink2(egui::vec2(10.0, 28.0));
        let weak = ui.visuals().weak_text_color();
        for fraction in [0.0, 0.5, 1.0] {
            painter.hline(
                plot.x_range(),
                plot.bottom() - plot.height() * fraction,
                Stroke::new(1.0_f32, weak.gamma_multiply(0.2)),
            );
        }
        let font = FontId::monospace(10.0);
        painter.text(
            rect.left_top(),
            Align2::LEFT_TOP,
            &self.high,
            font.clone(),
            weak,
        );
        painter.text(
            rect.right_top(),
            Align2::RIGHT_TOP,
            &self.low,
            font.clone(),
            weak,
        );
        painter.text(
            rect.left_bottom(),
            Align2::LEFT_BOTTOM,
            &self.first,
            font.clone(),
            weak,
        );
        painter.text(
            rect.right_bottom(),
            Align2::RIGHT_BOTTOM,
            &self.last,
            font,
            weak,
        );
        let color = ui.visuals().selection.bg_fill;
        let mut previous = None;
        let mut nearest = None;
        let mut drawn = 0;
        for &index in visible {
            let Some(Some([x, y])) = self.points.get(index) else {
                previous = None;
                continue;
            };
            let point = egui::pos2(
                plot.left() + x * plot.width(),
                plot.bottom() - y * plot.height(),
            );
            if let Some((prior_index, prior)) = previous
                && prior_index + 1 == index
            {
                painter.line_segment([prior, point], Stroke::new(1.5_f32, color));
            }
            painter.circle_filled(point, 2.0, color);
            previous = Some((index, point));
            drawn += 1;
            if let Some(mouse) = response.hover_pos() {
                let distance = mouse.distance_sq(point);
                if distance < 400.0 && nearest.is_none_or(|(best, _)| distance < best) {
                    nearest = Some((distance, index));
                }
            }
        }
        if drawn == 0 {
            painter.text(
                plot.center(),
                Align2::CENTER_CENTER,
                "No finite samples to plot",
                FontId::proportional(13.0),
                weak,
            );
        }
        if let Some((_, index)) = nearest {
            response.on_hover_text(&self.labels[index]);
        }
        ui.weak(format!(
            "{drawn} plotted · {} filtered or non-finite",
            self.points.len().saturating_sub(drawn)
        ));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Chart {
        Chart::new(
            &(0..200)
                .map(|i| {
                    vec![
                        (1700000000000u64 + i * 1000).to_string(),
                        (21.0 + (i as f64 / 30.0).sin()).to_string(),
                    ]
                })
                .collect::<Vec<_>>(),
        )
    }
    fn frame(ctx: &egui::Context, chart: &Chart, index: usize) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 760.0),
                )),
                time: Some(index as f64 / 120.0),
                ..Default::default()
            },
            |ctx| {
                egui::SidePanel::right("chart")
                    .exact_width(420.0)
                    .show(ctx, |ui| {
                        crate::redis_workspace::browser_style(ui);
                        chart.show(ui, &(0..200).collect::<Vec<_>>(), false);
                    });
            },
        )
    }
    #[test]
    fn chart_settles_and_caps_preparation() {
        let chart = fixture();
        let ctx = egui::Context::default();
        for i in 0..80 {
            let _ = frame(&ctx, &chart, i);
        }
        assert!(!ctx.has_requested_repaint());
        assert_eq!(
            Chart::new(&vec![vec!["0".into(), "NaN".into()]; 1000])
                .points
                .len(),
            200
        );
        assert_eq!(
            Chart::new(&[vec!["0".into(), "NaN".into()]]).points,
            vec![None]
        );
    }
    #[test]
    #[ignore = "manual loaded time-series chart CPU benchmark"]
    fn redis_timeseries_chart_frame_benchmark() {
        let chart = fixture();
        let ctx = egui::Context::default();
        let mut samples = Vec::new();
        for i in 0..180 {
            let start = std::time::Instant::now();
            let out = frame(&ctx, &chart, i);
            let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
            if i >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/timeseries-chart", &samples);
    }
    #[test]
    fn exact_timestamp_offsets_extreme_values_and_gaps() {
        let rows = vec![
            vec![(i64::MAX - 2).to_string(), "-1e308".into()],
            vec![(i64::MAX - 1).to_string(), "NaN".into()],
            vec![i64::MAX.to_string(), "1e308".into()],
        ];
        let chart = Chart::new(&rows);
        assert_eq!(chart.points, vec![Some([0.0, 0.0]), None, Some([1.0, 1.0])]);
        assert!(chart.labels[2].contains(&i64::MAX.to_string()));
        assert!(Chart::new(&[]).points.is_empty());
        assert_eq!(
            Chart::new(&[vec!["0".into(), "2".into()]]).points,
            vec![Some([0.5, 0.5])]
        );
    }
}
