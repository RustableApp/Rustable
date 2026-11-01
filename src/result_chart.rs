//! Charts of loaded results only; no SQL or persisted result payloads.
use crate::db::{self, Grid, Value};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use std::sync::{Arc, Weak};

#[derive(Default, Clone, Copy, PartialEq)]
enum Kind {
    #[default]
    Bar,
    Line,
    Pie,
}
const COLORS: [Color32; 8] = [
    Color32::from_rgb(45, 155, 240),
    Color32::from_rgb(245, 160, 55),
    Color32::from_rgb(65, 185, 135),
    Color32::from_rgb(190, 115, 225),
    Color32::from_rgb(240, 105, 125),
    Color32::from_rgb(65, 190, 205),
    Color32::from_rgb(190, 180, 75),
    Color32::from_rgb(135, 155, 200),
];
#[derive(Default)]
pub struct Chart {
    source: Weak<Grid>,
    columns: Vec<String>,
    kind: Kind,
    x: usize,
    ys: Vec<usize>,
    numeric: Vec<usize>,
    series: Vec<Vec<Option<f64>>>,
    pie_y: Option<usize>,
    pie_values: Vec<Option<f64>>,
}
fn number(value: &Value) -> Option<f64> {
    let value = match value {
        Value::Integer(v) => *v as f64,
        Value::Real(v) => *v,
        Value::Text(v) => v.trim().parse().ok()?,
        _ => return None,
    };
    value.is_finite().then_some(value)
}
impl Chart {
    fn prepare(&mut self, grid: &Arc<Grid>) {
        if self.source.ptr_eq(&Arc::downgrade(grid)) {
            return;
        }
        self.source = Arc::downgrade(grid);
        self.numeric = (0..grid.columns.len())
            .filter(|&column| {
                grid.rows
                    .iter()
                    .any(|r| r.get(column).and_then(number).is_some())
            })
            .collect();
        if self.columns != grid.columns {
            self.columns = grid.columns.clone();
            self.pie_y = None;
            self.x = 0;
            self.ys = self
                .numeric
                .iter()
                .copied()
                .find(|&c| c != self.x)
                .or_else(|| self.numeric.first().copied())
                .into_iter()
                .collect();
        }
        self.ys.retain(|c| self.numeric.contains(c));
        if self.pie_y.is_none_or(|c| !self.numeric.contains(&c)) {
            self.pie_y = self
                .ys
                .first()
                .copied()
                .or_else(|| self.numeric.first().copied());
        }
        self.rebuild(grid);
    }
    fn rebuild(&mut self, grid: &Grid) {
        self.pie_values = self
            .pie_y
            .map(|column| {
                grid.rows
                    .iter()
                    .map(|r| r.get(column).and_then(number))
                    .collect()
            })
            .unwrap_or_default();
        self.series = self
            .ys
            .iter()
            .map(|&column| {
                grid.rows
                    .iter()
                    .map(|r| r.get(column).and_then(number))
                    .collect()
            })
            .collect();
    }
    pub fn show(&mut self, ui: &mut egui::Ui, grid: &Arc<Grid>) {
        self.prepare(grid);
        if grid.columns.is_empty() || grid.rows.is_empty() {
            ui.weak("Run a query returning rows to draw a chart.");
            return;
        }
        let previous = (self.ys.clone(), self.pie_y);
        ui.horizontal_wrapped(|ui| {
            for (kind, label) in [(Kind::Bar, "Bar"), (Kind::Line, "Line"), (Kind::Pie, "Pie")] {
                ui.selectable_value(&mut self.kind, kind, label);
            }
            ui.separator();
            ui.label(if self.kind == Kind::Pie {
                "Labels"
            } else {
                "X axis"
            });
            egui::ComboBox::from_id_salt("chart-x")
                .selected_text(&grid.columns[self.x])
                .width(120.0)
                .show_ui(ui, |ui| {
                    for (i, name) in grid.columns.iter().enumerate() {
                        ui.selectable_value(&mut self.x, i, name);
                    }
                });
            if self.kind == Kind::Pie {
                ui.label("Value");
                egui::ComboBox::from_id_salt("pie-value")
                    .selected_text(
                        self.pie_y
                            .map(|c| grid.columns[c].as_str())
                            .unwrap_or("Choose column"),
                    )
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for &column in &self.numeric {
                            ui.selectable_value(
                                &mut self.pie_y,
                                Some(column),
                                &grid.columns[column],
                            );
                        }
                    });
            } else {
                ui.menu_button("Y axis", |ui| {
                    if self.numeric.is_empty() {
                        ui.weak("No numeric columns in this result.");
                    }
                    for &column in &self.numeric {
                        let mut selected = self.ys.contains(&column);
                        if ui
                            .add_enabled(
                                selected || self.ys.len() < 8,
                                egui::Checkbox::new(&mut selected, &grid.columns[column]),
                            )
                            .changed()
                        {
                            if selected {
                                self.ys.push(column);
                            } else {
                                self.ys.retain(|&c| c != column);
                            }
                        }
                    }
                });
            }
            ui.weak(format!(
                "{} loaded rows{}",
                grid.rows.len(),
                if grid.truncated {
                    "+ · result limit reached"
                } else {
                    ""
                }
            ));
        });
        if previous != (self.ys.clone(), self.pie_y) {
            self.rebuild(grid);
        }
        if (self.kind == Kind::Pie && self.pie_y.is_none())
            || (self.kind != Kind::Pie && self.ys.is_empty())
        {
            ui.weak("Choose a numeric value column to draw a chart.");
            return;
        }
        ui.horizontal_wrapped(|ui| {
            let skipped = if self.kind == Kind::Pie {
                self.pie_values.iter().filter(|v| v.is_none()).count()
            } else {
                for (i, &column) in self.ys.iter().enumerate() {
                    ui.colored_label(COLORS[i], &grid.columns[column]);
                }
                self.series.iter().flatten().filter(|v| v.is_none()).count()
            };
            if skipped > 0 {
                ui.weak(format!("{skipped} NULL or nonnumeric values omitted"));
            }
        });
        let size = Vec2::new(
            ui.available_width().max(1.0),
            ui.available_height().max(1.0),
        );
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
        if rect.width() < 140.0 || rect.height() < 100.0 {
            return;
        }
        if self.kind == Kind::Pie {
            self.pie(ui, grid, rect, &response);
        } else {
            self.cartesian(ui, grid, rect, &response);
        }
    }
    fn cartesian(&self, ui: &egui::Ui, grid: &Grid, rect: Rect, response: &egui::Response) {
        let painter = ui.painter().with_clip_rect(rect);
        let plot = Rect::from_min_max(
            rect.min + Vec2::new(68.0, 12.0),
            rect.max - Vec2::new(12.0, 40.0),
        );
        let Some((scale, low, high)) = bounds(&self.series) else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No finite numeric values",
                egui::FontId::proportional(13.0),
                ui.visuals().weak_text_color(),
            );
            return;
        };
        let y = |v: f64| {
            plot.bottom()
                - (((v / scale - low) / (high - low)).clamp(0.0, 1.0) as f32) * plot.height()
        };
        let step = plot.width() / grid.rows.len() as f32;
        let numeric_x: Option<Vec<f64>> = if self.kind == Kind::Line {
            grid.rows
                .iter()
                .map(|r| r.get(self.x).and_then(number))
                .collect()
        } else {
            None
        };
        let x_bounds = numeric_x
            .as_ref()
            .and_then(|xs| bounds(&[xs.iter().copied().map(Some).collect()]));
        let x = |row: usize| {
            if let (Some(xs), Some((scale, low, high))) = (&numeric_x, x_bounds) {
                plot.left()
                    + (((xs[row] / scale - low) / (high - low)).clamp(0.0, 1.0) as f32)
                        * plot.width()
            } else {
                plot.left() + (row as f32 + 0.5) * step
            }
        };
        let weak = ui.visuals().weak_text_color();
        for tick in 0..=4 {
            let fraction = tick as f64 / 4.0;
            let normalized = low + (high - low) * fraction;
            let py = plot.bottom() - fraction as f32 * plot.height();
            painter.hline(
                plot.x_range(),
                py,
                Stroke::new(1.0_f32, weak.gamma_multiply(0.2)),
            );
            painter.text(
                Pos2::new(plot.left() - 7.0, py),
                egui::Align2::RIGHT_CENTER,
                format_number(normalized * scale),
                egui::FontId::monospace(10.0),
                weak,
            );
        }
        let mut nearest: Option<(f32, usize, usize)> = None;
        let mut bars = egui::Mesh::default();
        for (series, values) in self.series.iter().enumerate() {
            let color = COLORS[series];
            let mut path = Vec::new();
            for (row, value) in values.iter().enumerate() {
                let Some(value) = value else {
                    if !path.is_empty() {
                        painter.add(egui::Shape::line(
                            std::mem::take(&mut path),
                            Stroke::new(1.5_f32, color),
                        ));
                    }
                    continue;
                };
                let point = Pos2::new(x(row), y(*value));
                if self.kind == Kind::Bar {
                    let width = step * 0.8 / self.series.len() as f32;
                    let left = plot.left() + row as f32 * step + step * 0.1 + series as f32 * width;
                    let bar = Rect::from_two_pos(
                        Pos2::new(left, y(0.0)),
                        Pos2::new(left + width, point.y),
                    );
                    bars.add_colored_rect(bar, color);
                    if response
                        .hover_pos()
                        .is_some_and(|p| bar.expand(1.0).contains(p))
                    {
                        nearest = Some((0.0, row, series));
                    }
                } else {
                    path.push(point);
                    if values.len() <= 200 {
                        painter.circle_filled(point, 2.0, color);
                    }
                    if let Some(mouse) = response.hover_pos() {
                        let distance = mouse.distance_sq(point);
                        if distance < 225.0 && nearest.is_none_or(|(best, _, _)| distance < best) {
                            nearest = Some((distance, row, series));
                        }
                    }
                }
            }
            if !path.is_empty() {
                painter.add(egui::Shape::line(path, Stroke::new(1.5_f32, color)));
            }
        }
        if !bars.is_empty() {
            painter.add(egui::Shape::mesh(bars));
        }
        // Labels stay bounded even for thousands of loaded rows.
        let stride = (grid.rows.len() / ((plot.width() / 95.0) as usize).max(1)).max(1);
        for row in (0..grid.rows.len()).step_by(stride) {
            let label = grid.rows[row]
                .get(self.x)
                .map(|v| db::preview(v, 12))
                .unwrap_or_default();
            painter.text(
                Pos2::new(x(row), plot.bottom() + 12.0),
                egui::Align2::CENTER_TOP,
                label,
                egui::FontId::proportional(10.0),
                weak,
            );
        }
        if let Some((_, row, series)) = nearest {
            self.tooltip(response, grid, row, self.ys[series]);
        }
    }
    fn tooltip(&self, response: &egui::Response, grid: &Grid, row: usize, column: usize) {
        response.clone().on_hover_ui(|ui| {
            ui.label(format!("Row {}", row + 1));
            for index in [self.x, column] {
                if let Some(value) = grid.rows[row].get(index) {
                    ui.label(format!(
                        "{}: {}",
                        grid.columns[index],
                        db::preview(value, 256)
                    ));
                }
            }
        });
    }
    fn pie(&self, ui: &mut egui::Ui, grid: &Grid, rect: Rect, response: &egui::Response) {
        let painter = ui.painter().with_clip_rect(rect);
        let values = &self.pie_values;
        if values.iter().flatten().any(|&v| v < 0.0) {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Pie charts require nonnegative values. Choose another value column.",
                egui::FontId::proportional(13.0),
                ui.visuals().weak_text_color(),
            );
            return;
        }
        let max = values.iter().flatten().copied().fold(0.0, f64::max);
        if max == 0.0 {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No positive values to draw",
                egui::FontId::proportional(13.0),
                ui.visuals().weak_text_color(),
            );
            return;
        }
        let total: f64 = values.iter().flatten().map(|v| v / max).sum();
        let (plot, legend) = pie_layout(rect);
        let center = plot.center();
        let radius = plot.width().min(plot.height()) * 0.43;
        let mut angle = -std::f64::consts::FRAC_PI_2;
        let hovered = response
            .hover_pos()
            .filter(|p| p.distance(center) <= radius)
            .map(|p| {
                let a = ((p.y - center.y) as f64).atan2((p.x - center.x) as f64);
                if a < -std::f64::consts::FRAC_PI_2 {
                    a + std::f64::consts::TAU
                } else {
                    a
                }
            });
        let mut mesh = egui::Mesh::default();
        for (row, value) in values.iter().enumerate() {
            let Some(value) = value.filter(|v| *v > 0.0) else {
                continue;
            };
            let end = angle + (value / max / total) * std::f64::consts::TAU;
            let steps = (((end - angle) * 32.0).ceil() as usize).max(1);
            let base = mesh.vertices.len() as u32;
            let color = COLORS[row % COLORS.len()];
            mesh.colored_vertex(center, color);
            for step in 0..=steps {
                let a = angle + (end - angle) * step as f64 / steps as f64;
                mesh.colored_vertex(
                    center + Vec2::new(a.cos() as f32, a.sin() as f32) * radius,
                    color,
                );
                if step > 0 {
                    mesh.add_triangle(base, base + step as u32, base + step as u32 + 1);
                }
            }
            if hovered.is_some_and(|a| a >= angle && a < end) {
                self.tooltip(response, grid, row, self.pie_y.unwrap());
            }
            angle = end;
        }
        painter.add(egui::Shape::mesh(mesh));
        let rows = values
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.is_some_and(|v| v > 0.0).then_some(i))
            .collect::<Vec<_>>();
        ui.scope_builder(egui::UiBuilder::new().max_rect(legend), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(legend));
            egui::ScrollArea::vertical()
                .id_salt("pie-legend")
                .auto_shrink([false, false])
                .show_rows(ui, 22.0, rows.len(), |ui, visible| {
                    for index in visible {
                        let row = rows[index];
                        let label = grid.rows[row]
                            .get(self.x)
                            .map(|v| db::preview(v, 48))
                            .unwrap_or_default();
                        let value = grid.rows[row]
                            .get(self.pie_y.unwrap())
                            .map(|v| db::preview(v, 48))
                            .unwrap_or_default();
                        let percent = values[row].unwrap() / max / total * 100.0;
                        let response = ui
                            .horizontal(|ui| {
                                ui.colored_label(COLORS[row % COLORS.len()], "●");
                                ui.add(
                                    egui::Label::new(format!("{label}: {value} ({percent:.1}%)"))
                                        .truncate(),
                                );
                            })
                            .response;
                        self.tooltip(&response, grid, row, self.pie_y.unwrap());
                    }
                });
        });
    }
}
fn pie_layout(rect: Rect) -> (Rect, Rect) {
    if rect.width() >= 420.0 {
        let split = rect.right() - 200.0;
        (
            Rect::from_min_max(rect.min, Pos2::new(split - 12.0, rect.bottom())),
            Rect::from_min_max(Pos2::new(split, rect.top()), rect.max),
        )
    } else {
        let split = rect.top() + rect.height() * 0.65;
        (
            Rect::from_min_max(rect.min, Pos2::new(rect.right(), split - 8.0)),
            Rect::from_min_max(Pos2::new(rect.left(), split), rect.max),
        )
    }
}
// Normalize before taking the range, avoiding overflow for extreme finite data.
fn bounds(series: &[Vec<Option<f64>>]) -> Option<(f64, f64, f64)> {
    let mut found = false;
    let scale = series
        .iter()
        .flatten()
        .flatten()
        .fold(0.0_f64, |max, &v| {
            found = true;
            max.max(v.abs())
        })
        .max(1.0);
    if !found {
        return None;
    }
    let mut low = 0.0_f64;
    let mut high = 0.0_f64;
    for &value in series.iter().flatten().flatten() {
        low = low.min(value / scale);
        high = high.max(value / scale);
    }
    if low == high {
        high = low + 1.0;
    }
    Some((scale, low, high))
}
fn format_number(value: f64) -> String {
    if value.abs() >= 1e6 || (value != 0.0 && value.abs() < 0.001) {
        format!("{value:.1e}")
    } else {
        format!("{value:.2}")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pie_selection_is_independent_and_resets_for_new_columns() {
        let mut chart = Chart::default();
        let mut grid = Arc::new(Grid {
            columns: vec!["label".into(), "revenue".into(), "cost".into()],
            rows: vec![vec![
                Value::Text("Jan".into()),
                Value::Integer(12),
                Value::Integer(8),
            ]],
            ..Default::default()
        });
        chart.prepare(&grid);
        chart.ys = vec![1, 2];
        chart.pie_y = Some(2);
        chart.rebuild(&grid);
        assert_eq!(chart.series, [vec![Some(12.0)], vec![Some(8.0)]]);
        assert_eq!(chart.pie_values, [Some(8.0)]);
        let same_shape = Arc::new((*grid).clone());
        chart.prepare(&same_shape);
        assert_eq!(chart.pie_y, Some(2));
        Arc::make_mut(&mut grid).columns[2] = "unrelated".into();
        chart.prepare(&grid);
        assert_eq!(chart.pie_y, Some(1));
        assert_eq!(chart.pie_values, [Some(12.0)]);
    }

    #[test]
    fn pie_legend_renders_labels_values_and_percentages_at_both_layouts() {
        let grid = Arc::new(Grid {
            columns: vec!["month".into(), "cost".into()],
            rows: vec![
                vec![Value::Text("Jan".into()), Value::Integer(1)],
                vec![Value::Text("Feb".into()), Value::Integer(3)],
            ],
            ..Default::default()
        });
        for width in [380.0, 800.0] {
            let ctx = egui::Context::default();
            let mut chart = Chart {
                kind: Kind::Pie,
                ..Default::default()
            };
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 500.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| chart.show(ui, &grid));
                },
            );
            let texts = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some(t.galley.text()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(texts.contains(&"Jan: 1 (25.0%)"), "{texts:?}");
            assert!(texts.contains(&"Feb: 3 (75.0%)"), "{texts:?}");
        }
    }
    #[test]
    fn numeric_conversion_and_extreme_ranges() {
        assert_eq!(number(&Value::Text("12.50".into())), Some(12.5));
        for value in [
            Value::Null,
            Value::Real(f64::INFINITY),
            Value::Text("NaN".into()),
            Value::Text("hello".into()),
        ] {
            assert!(number(&value).is_none());
        }
        let (scale, low, high) = bounds(&[vec![Some(-f64::MAX), Some(f64::MAX)]]).unwrap();
        assert!(scale.is_finite() && (high - low).is_finite());
        assert_eq!((low, high), (-1.0, 1.0));
        assert!(bounds(&[vec![None]]).is_none());
    }
    #[test]
    fn cache_tracks_source_and_preserves_same_shape_choices() {
        let mut chart = Chart::default();
        let make = |value| {
            Arc::new(Grid {
                columns: vec!["label".into(), "value".into()],
                rows: vec![vec![Value::Text("a".into()), Value::Integer(value)]],
                ..Default::default()
            })
        };
        let first = make(1);
        chart.prepare(&first);
        assert_eq!(chart.ys, [1]);
        assert_eq!(chart.series, [vec![Some(1.0)]]);
        let second = make(99);
        chart.prepare(&second);
        assert_eq!(chart.series, [vec![Some(99.0)]]);
        let weak = Arc::downgrade(&second);
        drop(second);
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn all_chart_kinds_render_finite_geometry() {
        let ctx = egui::Context::default();
        let grid = Arc::new(Grid {
            columns: vec!["label".into(), "value".into()],
            rows: vec![
                vec![Value::Text("a".into()), Value::Integer(2)],
                vec![Value::Text("b".into()), Value::Integer(5)],
            ],
            ..Default::default()
        });
        let mut chart = Chart::default();
        for kind in [Kind::Bar, Kind::Line, Kind::Pie] {
            chart.kind = kind;
            let out = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 500.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| chart.show(ui, &grid));
                },
            );
            assert!(!out.shapes.is_empty());
            for primitive in ctx.tessellate(out.shapes, out.pixels_per_point) {
                if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                    assert!(
                        mesh.vertices
                            .iter()
                            .all(|v| v.pos.x.is_finite() && v.pos.y.is_finite())
                    );
                }
            }
        }
    }
    #[test]
    fn pie_rejects_negative_values_and_nulls_do_not_become_zero() {
        let ctx = egui::Context::default();
        let grid = Arc::new(Grid {
            columns: vec!["label".into(), "value".into()],
            rows: vec![
                vec![Value::Text("a".into()), Value::Integer(-2)],
                vec![Value::Text("b".into()), Value::Null],
            ],
            ..Default::default()
        });
        let mut chart = Chart {
            kind: Kind::Pie,
            ..Default::default()
        };
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 500.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| chart.show(ui, &grid));
            },
        );
        assert_eq!(chart.series, [vec![Some(-2.0), None]]);
        assert!(output.shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text().contains("require nonnegative"))));
    }
    #[test]
    #[ignore = "manual loaded-result chart rendering benchmark"]
    fn chart_render_latency() {
        let ctx = egui::Context::default();
        let grid = Arc::new(Grid {
            columns: (0..9).map(|i| format!("value{i}")).collect(),
            rows: (0..5000)
                .map(|row| {
                    (0..9)
                        .map(|column| Value::Integer((row + column) % 1000))
                        .collect()
                })
                .collect(),
            ..Default::default()
        });
        let mut chart = Chart::default();
        chart.prepare(&grid);
        chart.ys = (1..9).collect();
        chart.rebuild(&grid);
        for kind in [Kind::Bar, Kind::Line, Kind::Pie] {
            chart.kind = kind;
            let mut samples = Vec::new();
            for run in 0..25 {
                let start = std::time::Instant::now();
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            Vec2::new(1200.0, 650.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| chart.show(ui, &grid));
                    },
                );
                let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
                assert!(!primitives.is_empty());
                if run >= 5 {
                    samples.push(start.elapsed().as_secs_f64() * 1000.0);
                }
            }
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "chart {} 5000 rows, {} series, CPU frame+tessellation p50={:.2}ms p95={:.2}ms",
                if kind == Kind::Bar {
                    "bar"
                } else if kind == Kind::Line {
                    "line"
                } else {
                    "pie"
                },
                if kind == Kind::Pie { 1 } else { 8 },
                samples[9],
                samples[18]
            );
        }
    }
}
