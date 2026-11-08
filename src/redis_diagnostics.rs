use eframe::egui;
#[derive(Clone)]
pub(crate) enum Request {
    Server,
    Cluster,
    TimeSeries(Vec<u8>),
    VectorSimilar { key: Vec<u8>, element: Vec<u8> },
    VectorElement { key: Vec<u8>, element: Vec<u8> },
    Keys(Vec<Vec<u8>>),
}
#[derive(Default)]
pub(crate) struct Report {
    pub vector_matches: Option<Box<crate::redis_vector_matches::Matches>>,
    pub vector_element: Option<Box<crate::redis_vector_element::Element>>,
    pub summary: Vec<(String, String)>,
    pub rows: Vec<[String; 3]>,
    pub namespaces: Vec<[String; 3]>,
    pub analysis: bool,
    pub topology: bool,
    pub module_key: Option<String>,
    pub footer: String,
}
impl Report {
    pub fn show(&self, ui: &mut egui::Ui) -> Option<Request> {
        if let Some(matches) = &self.vector_matches { return matches.show(ui); }
        if let Some(element) = &self.vector_element { element.show(ui); return None; }
        if let Some(key) = &self.module_key { ui.label(egui::RichText::new(key).monospace()); }
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
        ui.columns(2, |columns| {
            for (i, (name, value)) in self.summary.iter().enumerate() {
                egui::Frame::new()
                    .fill(columns[i % 2].visuals().faint_bg_color)
                    .corner_radius(6.0)
                    .inner_margin(12)
                    .show(&mut columns[i % 2], |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.weak(name);
                        ui.label(egui::RichText::new(value).size(18.0));
                    });
            }
        });
        ui.add_space(8.0);
        if self.analysis {
            ui.strong("Top namespaces · sampled memory");
            use egui_extras::{Column, TableBuilder};
            TableBuilder::new(ui).id_salt("redis-top-namespaces").striped(true).max_scroll_height(180.0)
                .column(Column::remainder()).column(Column::exact(120.0)).column(Column::exact(100.0))
                .header(28.0, |mut row| { for name in ["NAMESPACE", "BYTES", "KEYS"] { row.col(|ui| { ui.weak(name); }); } })
                .body(|body| body.rows(26.0, self.namespaces.len(), |mut row| { for value in &self.namespaces[row.index()] { row.col(|ui| { ui.add(egui::Label::new(value).truncate()); }); } }));
            ui.add_space(16.0);
        }
        ui.strong(if self.module_key.is_some() {
            "Properties, labels and compaction rules"
        } else if self.topology {
            "Cluster nodes"
        } else if self.analysis {
            "Top 15 keys · sampled memory"
        } else {
            "Slow log · latest 50 entries"
        });
        use egui_extras::{Column, TableBuilder};
        TableBuilder::new(ui)
            .id_salt("redis-diagnostics-table")
            .striped(true)
            .max_scroll_height(320.0)
            .column(Column::initial(if self.module_key.is_some() { 80.0 } else { 180.0 }).clip(true))
            .column(Column::initial(if self.module_key.is_some() { 220.0 } else if self.topology { 160.0 } else { 110.0 }).clip(true))
            .column(Column::remainder().clip(true))
            .header(28.0, |mut row| {
                for name in if self.module_key.is_some() {
                    ["SECTION", "NAME / DESTINATION", "VALUE / RULE"]
                } else if self.topology {
                    ["ENDPOINT", "ROLE / HEALTH", "SLOTS / NODE ID"]
                } else if self.analysis {
                    ["KEY", "BYTES", "NAMESPACE"]
                } else {
                    ["UNIX SECONDS", "DURATION µs", "COMMAND"]
                } {
                    row.col(|ui| {
                        ui.weak(name);
                    });
                }
            })
            .body(|body| {
                body.rows(26.0, if self.analysis { self.rows.len().min(15) } else { self.rows.len() }, |mut row| {
                    for value in &self.rows[row.index()] {
                        row.col(|ui| {
                            ui.add(
                                egui::Label::new(egui::RichText::new(value).monospace()).truncate(),
                            )
                            .on_hover_text(value);
                        });
                    }
                });
            });
        if self.rows.is_empty() {
            ui.weak("No entries returned.");
        }
        ui.label(egui::RichText::new(&self.footer).color(ui.visuals().weak_text_color()));
        None
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn server(values: Vec<redis::Value>) -> Result<Report, String> {
    let mut values = values.into_iter();
    let info: String = redis::from_redis_value(values.next().ok_or("Missing INFO response")?)
        .map_err(|e| e.to_string())?;
    let fields: std::collections::BTreeMap<_, _> = info
        .lines()
        .filter_map(|line| line.split_once(':'))
        .collect();
    let summary = [
        ("Redis version", "redis_version"),
        ("Memory used", "used_memory_human"),
        ("Connected clients", "connected_clients"),
        ("Operations / second", "instantaneous_ops_per_sec"),
        ("Evicted keys", "evicted_keys"),
        ("Keyspace hits", "keyspace_hits"),
    ]
    .into_iter()
    .map(|(label, key)| {
        (
            label.into(),
            fields.get(key).unwrap_or(&"—").trim().to_owned(),
        )
    })
    .collect();
    let rows = match values.next() {
        Some(redis::Value::Array(entries)) => entries
            .into_iter()
            .take(50)
            .filter_map(|entry| {
                let redis::Value::Array(parts) = entry else {
                    return None;
                };
                if parts.len() < 4 {
                    return None;
                }
                let command = if let redis::Value::Array(args) = &parts[3] {
                    args.iter()
                        .take(16)
                        .map(|a| {
                            crate::redis_value::text(a)
                                .chars()
                                .take(256)
                                .collect::<String>()
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    String::new()
                };
                Some([
                    crate::redis_value::text(&parts[1]),
                    crate::redis_value::text(&parts[2]),
                    command,
                ])
            })
            .collect(),
        _ => return Err("Unexpected slow log response.".into()),
    };
    Ok(Report{namespaces:vec![],summary,rows,analysis:false,topology:false,module_key:None,vector_element:None,vector_matches:None,footer:"Snapshot taken on request. Slow log measures server execution, not network or client latency. No automatic polling or server configuration changes.".into()})
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn keys(keys: &[Vec<u8>], values: Vec<redis::Value>) -> Result<Report, String> {
    let mut rows = Vec::new();
    let mut total = 0u64;
    let mut missing = 0usize;
    for (key, value) in keys.iter().zip(values) {
        if value == redis::Value::Nil {
            missing += 1;
            continue;
        }
        let bytes: u64 = redis::from_redis_value(value).map_err(|e| e.to_string())?;
        total = total.saturating_add(bytes);
        rows.push((key, bytes));
    }
    let count = rows.len();
    let mut namespaces = std::collections::BTreeMap::<String, (u64, u64)>::new();
    for (key, bytes) in &rows {
        let name = crate::redis_commands::quote(key.split(|b| *b == b':').next().unwrap_or_default());
        let entry = namespaces.entry(name).or_default();
        entry.0 = entry.0.saturating_add(*bytes); entry.1 += 1;
    }
    let mut namespaces: Vec<_> = namespaces.into_iter().collect();
    namespaces.sort_by_key(|(_, (bytes, _))| std::cmp::Reverse(*bytes));
    let namespaces = namespaces.into_iter().take(15).map(|(name, (bytes, count))| [name, bytes.to_string(), count.to_string()]).collect();
    rows.sort_by_key(|(_, size)| std::cmp::Reverse(*size));
    Ok(Report {
        namespaces,
        summary:vec![("Keys sampled".into(),count.to_string()),("Sampled memory".into(),format!("{:.2} MiB",total as f64/1048576.0)),("Names no longer present".into(),missing.to_string()),("Sampling".into(),"5 elements / key".into())],
        rows:rows.into_iter().take(50).map(|(key,size)|[crate::redis_commands::quote(key),size.to_string(),crate::redis_commands::quote(key.split(|b|*b==b':').next().unwrap_or_default())]).collect(),analysis:true,topology:false,module_key:None,vector_element:None,vector_matches:None,
        footer:"Estimates for up to 1,000 loaded keys only, not the whole database. Nested values use MEMORY USAGE SAMPLES 5. Shows the top 15 sampled keys and namespaces; keys may change during collection.".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn namespaces_include_keys_outside_the_top_key_list() {
        let names: Vec<_> = (0..100).map(|i| format!("app:{i}").into_bytes()).collect();
        let report = keys(&names, vec![redis::Value::Int(10); 100]).unwrap();
        assert_eq!(report.rows.len(), 50);
        assert_eq!(report.namespaces, vec![["\"app\"".to_string(), "1000".to_string(), "100".to_string()]]);
    }
    fn timeseries_fixture() -> Report {
        Report {
            module_key: Some("telemetry:temperature".into()),
            summary: ["Total samples", "Retention (ms)", "Memory (bytes)", "First timestamp (ms)", "Last timestamp (ms)", "Chunks"].into_iter().map(|name| (name.into(), "1700000000000".into())).collect(),
            rows: (0..132).map(|i| ["Label".into(), format!("label:{i:03}"), "x".repeat(259)]).collect(),
            footer: "Metadata at the last refresh. Bounded labels and compaction rules.".into(),
            ..Default::default()
        }
    }
    fn metadata_frame(ctx: &egui::Context, report: &Report, frame: usize, scroll: bool) -> egui::FullOutput {
        ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 760.0))),
            time: Some(frame as f64 / 120.0),
            events: if scroll { vec![egui::Event::PointerMoved(egui::pos2(350.0, 450.0)), egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, if frame % 40 < 20 { -20.0 } else { 20.0 }), modifiers: egui::Modifiers::NONE }] } else { vec![] },
            ..Default::default()
        }, |ctx| {
            egui::Window::new("Time-series metadata").fixed_pos(egui::pos2(20.0, 20.0)).default_width(640.0).show(ctx, |ui| {
                crate::redis_workspace::browser_style(ui);
                let _ = report.show(ui);
            });
        })
    }
    #[test]
    fn timeseries_metadata_settles_at_idle() {
        let report = timeseries_fixture();
        let ctx = egui::Context::default();
        for frame in 0..80 { let _ = metadata_frame(&ctx, &report, frame, false); }
        assert!(!ctx.has_requested_repaint());
    }
    #[test]
    #[ignore = "manual bounded time-series metadata scrolling CPU benchmark"]
    fn redis_timeseries_metadata_frame_benchmark() {
        let report = timeseries_fixture();
        let ctx = egui::Context::default();
        let mut samples = Vec::new();
        for frame in 0..180 {
            let start = std::time::Instant::now();
            let out = metadata_frame(&ctx, &report, frame, true);
            let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
            if frame >= 20 { samples.push(start.elapsed().as_secs_f64() * 1000.0); }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/timeseries-metadata-scroll", &samples);
    }
    #[test]
    #[ignore = "manual Redis diagnostics CPU benchmark"]
    fn redis_diagnostics_frame_benchmark() {
        let report = Report {
            namespaces: vec![],
            summary: vec![
                ("Keys sampled".into(), "1000".into()),
                ("Memory".into(), "12 MiB".into()),
            ],
            rows: (0..50)
                .map(|i| [format!("app:users:{i:05}"), "1024".into(), "app".into()])
                .collect(),
            analysis: true,
            topology: false,
            module_key: None,
            vector_element: None,
            vector_matches: None,
            footer: "Sampled keys only".into(),
        };
        let ctx = egui::Context::default();
        let mut samples = Vec::new();
        for frame in 0..180 {
            let start = std::time::Instant::now();
            let out = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 760.0),
                    )),
                    time: Some(frame as f64 / 120.0),
                    ..Default::default()
                },
                |ctx| {
                    egui::Window::new("Diagnostics")
                        .default_width(640.0)
                        .show(ctx, |ui| {
                            crate::redis_workspace::browser_style(ui);
                            let _ = report.show(ui);
                        });
                },
            );
            let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
            if frame >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/diagnostics", &samples);
    }
}
