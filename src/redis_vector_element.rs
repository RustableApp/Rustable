//! Explicit, bounded inspection of a vector element; never fetched while paging.
use crate::redis_value::StringViews;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) const SCRIPT: &str = r#"
local values = redis.call('VEMB', KEYS[1], ARGV[1])
if not values then return redis.error_reply('Vector element no longer exists. Refresh the page.') end
if #values > 4096 then return redis.error_reply('Vector exceeds the 4096-dimension inspector limit') end
for _,v in ipairs(values) do
 if type(v) ~= 'string' or #v > 128 then return redis.error_reply('Invalid vector component') end
end
local attributes = redis.call('VGETATTR', KEYS[1], ARGV[1])
if attributes and #attributes > 65536 then return redis.error_reply('Vector attributes exceed the 64 KiB inspector limit') end
return {values, attributes or false}
"#;

pub(crate) struct Element {
    pub name: String,
    pub components: Vec<String>,
    pub attributes: Option<StringViews>,
}
impl Element {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn decode(value: redis::Value, name: &[u8]) -> Result<Self, String> {
        let (values, attributes): (Vec<Vec<u8>>, Option<Vec<u8>>) =
            redis::from_redis_value(value).map_err(|e| format!("Invalid vector element: {e}"))?;
        if values.is_empty()
            || values.len() > 4096
            || name.len() > 65536
            || attributes.as_ref().is_some_and(|a| a.len() > 65536)
        {
            return Err("Vector element exceeds inspector limits.".into());
        }
        let components = values
            .into_iter()
            .map(|v| {
                if v.len() > 128 {
                    return Err("Vector component exceeds inspector limits.".to_string());
                }
                let text =
                    String::from_utf8(v).map_err(|_| "Invalid vector component.".to_string())?;
                if !text.parse::<f64>().is_ok_and(f64::is_finite) {
                    return Err("Non-finite or invalid vector component.".into());
                }
                Ok(text)
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            name: crate::redis_commands::quote(name)
                .chars()
                .take(256)
                .collect(),
            components,
            attributes: attributes.map(|bytes| StringViews::new(&bytes, 0, true)),
        })
    }
}

impl Element {
    pub fn show(&self, ui: &mut eframe::egui::Ui) {
        use eframe::egui;
        use egui_extras::{Column, TableBuilder};
        ui.add(egui::Label::new(egui::RichText::new(&self.name).monospace()).truncate());
        ui.strong(format!(
            "Stored vector · {} dimensions",
            self.components.len()
        ));
        ui.weak("Approximate values after normalization and quantization.");
        TableBuilder::new(ui)
            .id_salt("vector-components")
            .striped(true)
            .max_scroll_height(200.0)
            .column(Column::initial(90.0))
            .column(Column::remainder())
            .header(28.0, |mut row| {
                row.col(|ui| {
                    ui.weak("DIMENSION");
                });
                row.col(|ui| {
                    ui.weak("VALUE");
                });
            })
            .body(|body| {
                body.rows(26.0, self.components.len(), |mut row| {
                    let index = row.index();
                    row.col(|ui| {
                        ui.monospace(index.to_string());
                    });
                    row.col(|ui| {
                        ui.monospace(&self.components[index]);
                    });
                })
            });
        if ui
            .add(crate::redis_workspace::action_button(
                "clipboard",
                "Copy vector",
            ))
            .clicked()
        {
            ui.ctx()
                .copy_text(format!("[{}]", self.components.join(", ")));
        }
        ui.add_space(8.0);
        ui.strong("JSON attributes");
        if let Some(attributes) = &self.attributes {
            if let Some(error) = &attributes.json_error {
                ui.colored_label(ui.visuals().warn_fg_color, error);
            }
            crate::redis_workspace::value_lines_styled(
                ui,
                "vector-attributes",
                &attributes.json,
                180.0,
                Some(&attributes.json_tokens),
            );
            if ui
                .add_enabled(
                    attributes.json_source.is_some(),
                    crate::redis_workspace::action_button("clipboard", "Copy attributes"),
                )
                .clicked()
            {
                ui.ctx()
                    .copy_text(attributes.json_source.clone().unwrap_or_default());
            }
        } else {
            ui.weak("No attributes stored for this element.");
        }
        ui.weak("Snapshot loaded on request. Refresh to load changes.");
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    fn reply(values: &[&str], attributes: Option<&[u8]>) -> redis::Value {
        redis::Value::Array(vec![
            redis::Value::Array(
                values
                    .iter()
                    .map(|v| redis::Value::BulkString(v.as_bytes().to_vec()))
                    .collect(),
            ),
            attributes.map_or(redis::Value::Nil, |v| redis::Value::BulkString(v.to_vec())),
        ])
    }
    #[test]
    fn decoder_rejects_unbounded_or_invalid_components() {
        for value in [
            reply(&[], None),
            reply(&["NaN"], None),
            reply(&["inf"], None),
            reply(&["bad"], None),
            reply(&["1"; 4097], None),
            reply(&["1"], Some(&vec![b' '; 65537])),
        ] {
            assert!(Element::decode(value, b"element").is_err());
        }
        let detail = Element::decode(
            reply(&["-0.25", "1e-6"], Some(br#"{"id":9007199254740993}"#)),
            &[0, 255],
        )
        .unwrap();
        assert_eq!(detail.components, ["-0.25", "1e-6"]);
        assert!(
            detail
                .attributes
                .unwrap()
                .json_source
                .unwrap()
                .contains("9007199254740993")
        );
    }
    #[test]
    #[ignore = "manual maximum-size vector detail CPU workload"]
    fn redis_vector_detail_frame_benchmark() {
        use eframe::egui;
        let attributes = format!("{{\"items\":[{}]}}", vec!["0"; 32000].join(","));
        let detail = Element::decode(
            reply(&["0.123456789"; 4096], Some(attributes.as_bytes())),
            b"product:00000:description",
        )
        .unwrap();
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
                        egui::Event::PointerMoved(egui::pos2(350.0, 200.0)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, if frame % 40 < 20 { -18.0 } else { 18.0 }),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ctx| {
                    egui::Window::new("Vector element")
                        .fixed_pos(egui::pos2(20.0, 20.0))
                        .default_width(640.0)
                        .show(ctx, |ui| {
                            crate::redis_workspace::browser_style(ui);
                            detail.show(ui);
                        });
                },
            );
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/vector-detail", &samples);
    }
}
