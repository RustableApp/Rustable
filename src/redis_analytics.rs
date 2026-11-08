//! Bounded aggregation and profile models, prepared by the Redis worker.
use eframe::egui;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum Reducer {
    #[default]
    Count,
    Sum,
    Average,
    Minimum,
    Maximum,
}
impl Reducer {
    pub fn label(self) -> &'static str {
        match self {
            Self::Count => "Count",
            Self::Sum => "Sum",
            Self::Average => "Average",
            Self::Minimum => "Minimum",
            Self::Maximum => "Maximum",
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn command(self) -> &'static str {
        match self {
            Self::Count => "COUNT",
            Self::Sum => "SUM",
            Self::Average => "AVG",
            Self::Minimum => "MIN",
            Self::Maximum => "MAX",
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Query {
    pub index: Vec<u8>,
    pub query: String,
    pub groups: Vec<Vec<u8>>,
    pub metric: Option<Vec<u8>>,
    pub reducer: Reducer,
    pub load: Vec<Vec<u8>>,
    pub offset: usize,
}
#[derive(Default)]
pub(crate) struct Draft {
    pub groups: Vec<Vec<u8>>,
    pub metric: Option<Vec<u8>>,
    pub reducer: Reducer,
}
pub(crate) struct Report {
    pub request: Query,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub more: bool,
}
pub(crate) struct Profile {
    pub rows: Vec<[String; 2]>,
    pub truncated: bool,
    pub client_ms: f64,
    pub description: String,
}
impl Draft {
    pub fn reconcile(&mut self, schema: &[crate::redis_search::Field]) {
        self.groups
            .retain(|name| schema.iter().any(|f| &f.name == name && f.kind != "VECTOR"));
        if self.metric.as_ref().is_some_and(|name| {
            !schema
                .iter()
                .any(|f| &f.name == name && f.kind == "NUMERIC")
        }) {
            self.metric = None;
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, schema: &[crate::redis_search::Field]) {
        ui.horizontal(|ui| {
            ui.menu_button(format!("Group by · {}", self.groups.len()), |ui| {
                ui.weak("Choose up to 3 fields; none summarizes all matches");
                egui::ScrollArea::vertical().max_height(220.0).show_rows(
                    ui,
                    23.0,
                    schema.len(),
                    |ui, range| {
                        for i in range {
                            let f = &schema[i];
                            let mut selected = self.groups.contains(&f.name);
                            if ui
                                .add_enabled(
                                    f.kind != "VECTOR" && (selected || self.groups.len() < 3),
                                    egui::Checkbox::new(&mut selected, label(&f.name)),
                                )
                                .changed()
                            {
                                if selected {
                                    self.groups.push(f.name.clone());
                                } else {
                                    self.groups.retain(|n| n != &f.name);
                                }
                            }
                        }
                    },
                );
            });
            egui::ComboBox::from_id_salt("aggregate-reducer")
                .selected_text(self.reducer.label())
                .show_ui(ui, |ui| {
                    for r in [
                        Reducer::Count,
                        Reducer::Sum,
                        Reducer::Average,
                        Reducer::Minimum,
                        Reducer::Maximum,
                    ] {
                        ui.selectable_value(&mut self.reducer, r, r.label());
                    }
                });
            if self.reducer != Reducer::Count {
                egui::ComboBox::from_id_salt("aggregate-metric")
                    .selected_text(
                        self.metric
                            .as_ref()
                            .map_or("Numeric field".into(), |n| label(n)),
                    )
                    .show_ui(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(220.0).show_rows(
                            ui,
                            23.0,
                            schema.len(),
                            |ui, range| {
                                for i in range {
                                    let f = &schema[i];
                                    if f.kind == "NUMERIC" {
                                        ui.selectable_value(
                                            &mut self.metric,
                                            Some(f.name.clone()),
                                            label(&f.name),
                                        );
                                    }
                                }
                            },
                        );
                    });
            }
        });
        ui.weak(
            "Count is always included. Non-sortable fields are loaded from matching documents.",
        );
    }
    pub fn request(
        &self,
        index: Vec<u8>,
        query: String,
        schema: &[crate::redis_search::Field],
    ) -> Query {
        let metric = if self.reducer == Reducer::Count {
            None
        } else {
            self.metric.clone()
        };
        let mut load = Vec::new();
        for name in self.groups.iter().chain(metric.iter()) {
            if !schema.iter().any(|f| &f.name == name && f.sortable) && !load.contains(name) {
                load.push(name.clone());
            }
        }
        Query {
            index,
            query,
            groups: self.groups.clone(),
            metric,
            reducer: self.reducer,
            load,
            offset: 0,
        }
    }
}
impl Report {
    pub fn show(&self, ui: &mut egui::Ui) -> Option<Query> {
        let mut next = None;
        ui.horizontal(|ui| {
            for (label, enabled, offset) in [
                (
                    "Previous",
                    self.request.offset > 0,
                    self.request.offset.saturating_sub(50),
                ),
                ("Next", self.more, self.request.offset + 50),
            ] {
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                    let mut q = self.request.clone();
                    q.offset = offset;
                    next = Some(q);
                }
            }
            ui.weak(format!(
                "{} groups shown · offset {}",
                self.rows.len(),
                self.request.offset
            ));
        });
        ui.add(
            egui::Label::new(format!(
                "Results for {} · {}",
                label(&self.request.index),
                self.request.query
            ))
            .truncate(),
        );
        hint(
            ui,
            "Ordered by count, then group values. Changes between requests can move groups between pages. Server timeout policy may return partial aggregates.",
        );
        if self.rows.is_empty() {
            ui.strong("No groups returned");
        }
        use egui_extras::{Column, TableBuilder};
        egui::ScrollArea::horizontal()
            .id_salt("aggregate-columns")
            .show(ui, |ui| {
                let mut table = TableBuilder::new(ui)
                    .id_salt(("aggregate-results", &self.columns))
                    .striped(true)
                    .max_scroll_height(270.0);
                for _ in &self.columns {
                    table = table.column(Column::initial(150.0).clip(true));
                }
                table
                    .header(26.0, |mut row| {
                        for col in &self.columns {
                            row.col(|ui| {
                                ui.add(egui::Label::new(col).truncate());
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(26.0, self.rows.len(), |mut row| {
                            for cell in &self.rows[row.index()] {
                                row.col(|ui| {
                                    ui.add(egui::Label::new(cell).truncate())
                                        .on_hover_text(cell);
                                });
                            }
                        })
                    });
            });
        next
    }
}
impl Profile {
    pub fn show(&self, ui: &mut egui::Ui) {
        ui.add(egui::Label::new(&self.description).truncate())
            .on_hover_text(&self.description);
        ui.strong(format!("Client elapsed · {:.2} ms", self.client_ms));
        hint(
            ui,
            "Server time fields are milliseconds; counts are operations. Client elapsed includes transport and decoding.",
        );
        use egui_extras::{Column, TableBuilder};
        TableBuilder::new(ui)
            .id_salt("search-profile")
            .striped(true)
            .max_scroll_height(220.0)
            .min_scrolled_height(80.0)
            .column(Column::remainder().clip(true))
            .column(Column::initial(180.0).clip(true))
            .header(26.0, |mut row| {
                for s in ["PROFILE DETAIL", "VALUE"] {
                    row.col(|ui| {
                        ui.weak(s);
                    });
                }
            })
            .body(|body| {
                body.rows(24.0, self.rows.len(), |mut row| {
                    for v in &self.rows[row.index()] {
                        row.col(|ui| {
                            ui.add(egui::Label::new(v).truncate()).on_hover_text(v);
                        });
                    }
                })
            });
        if self.rows.is_empty() {
            ui.weak("No profile details returned.");
        }
        if self.truncated {
            ui.weak("Profile display limited to 1,000 rows, 12 levels and bounded text.");
        }
        hint(
            ui,
            "Profiling adds server overhead. This report uses LIMITED details and refreshes only when requested.",
        );
    }
}
fn hint(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(egui::RichText::new(text).color(ui.visuals().weak_text_color())).wrap(),
    );
}
fn label(bytes: &[u8]) -> String {
    let b = &bytes[..bytes.len().min(512)];
    let mut s = crate::redis_commands::quote(b);
    if b.len() < bytes.len() {
        s.push('…');
    }
    s
}
#[cfg(not(target_arch = "wasm32"))]
fn property(name: &[u8]) -> Vec<u8> {
    let mut p = vec![b'@'];
    p.extend_from_slice(name);
    p
}
#[cfg(not(target_arch = "wasm32"))]
impl Query {
    fn aliases(&self) -> (Vec<u8>, Vec<u8>) {
        let mut count = b"__rustable_count".to_vec();
        while self.groups.contains(&count) {
            count.push(b'_');
        }
        let mut metric = b"__rustable_metric".to_vec();
        while self.groups.contains(&metric) {
            metric.push(b'_');
        }
        (count, metric)
    }
    pub fn append(&self, cmd: &mut redis::Cmd) -> Result<(), String> {
        if self.index.len() > 4096
            || self.query.len() > 4096
            || self.query.trim().is_empty()
            || self.offset > 9950
            || self.groups.len() > 3
            || self.load.len() > 4
            || self
                .groups
                .iter()
                .chain(self.load.iter())
                .chain(self.metric.iter())
                .any(|n| n.len() > 4096)
            || (self.reducer != Reducer::Count && self.metric.is_none())
        {
            return Err("Choose up to 3 group fields and a numeric field for the reducer. Query/name limits are 4,096 bytes; paging stops at 10,000 groups.".into());
        }
        if self
            .groups
            .iter()
            .enumerate()
            .any(|(i, g)| self.groups[..i].contains(g))
        {
            return Err("Group fields must be distinct.".into());
        }
        cmd.arg(&self.query).arg("TIMEOUT").arg(1000);
        if !self.load.is_empty() {
            cmd.arg("LOAD").arg(self.load.len());
            for f in &self.load {
                cmd.arg(property(f));
            }
        }
        cmd.arg("GROUPBY").arg(self.groups.len());
        for g in &self.groups {
            cmd.arg(property(g));
        }
        let (count, metric) = self.aliases();
        cmd.arg("REDUCE").arg("COUNT").arg(0).arg("AS").arg(&count);
        if self.reducer != Reducer::Count {
            cmd.arg("REDUCE")
                .arg(self.reducer.command())
                .arg(1)
                .arg(property(self.metric.as_ref().unwrap()))
                .arg("AS")
                .arg(&metric);
        }
        cmd.arg("SORTBY")
            .arg((self.groups.len() + 1) * 2)
            .arg(property(&count))
            .arg("DESC");
        for g in &self.groups {
            cmd.arg(property(g)).arg("ASC");
        }
        let limit = if self.offset == 9950 { 50 } else { 51 };
        cmd.arg("MAX")
            .arg(self.offset + limit)
            .arg("LIMIT")
            .arg(self.offset)
            .arg(limit)
            .arg("DIALECT")
            .arg(2);
        Ok(())
    }
    pub fn decode(self, value: redis::Value) -> Result<Report, String> {
        if self.reducer != Reducer::Count && self.metric.is_none() {
            return Err("Missing aggregation metric.".into());
        }
        let redis::Value::Array(values) = value else {
            return Err("Unexpected aggregation response.".into());
        };
        let mut values = values.into_iter();
        let _: usize = redis::from_redis_value(values.next().ok_or("Missing aggregation count")?)
            .map_err(|e| e.to_string())?;
        let (count, metric) = self.aliases();
        let mut names = self.groups.clone();
        names.push(count);
        let mut columns = self.groups.iter().map(|g| label(g)).collect::<Vec<_>>();
        columns.push("Count".into());
        if self.reducer != Reducer::Count {
            names.push(metric);
            columns.push(format!(
                "{} {}",
                self.reducer.label(),
                label(self.metric.as_ref().unwrap())
            ));
        }
        let mut rows = Vec::new();
        for v in values {
            if rows.len() == 51 {
                return Err("Aggregation page exceeds 51 rows.".into());
            }
            let mut cells = vec!["—".into(); names.len()];
            match v {
                redis::Value::Nil => {}
                redis::Value::Array(fields) => {
                    let (pairs, rest) = fields.as_chunks::<2>();
                    if !rest.is_empty() {
                        return Err("Malformed aggregation row.".into());
                    }
                    for pair in pairs {
                        let name = crate::redis_search::bytes(pair[0].clone())?;
                        if let Some(i) = names.iter().position(|n| n == &name) {
                            cells[i] = scalar(&pair[1]);
                        }
                    }
                }
                _ => return Err("Unexpected aggregation row.".into()),
            }
            rows.push(cells);
        }
        let more = rows.len() > 50 && self.offset < 9950;
        rows.truncate(50);
        Ok(Report {
            request: self,
            columns,
            rows,
            more,
        })
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn scalar(value: &redis::Value) -> String {
    match value {
        redis::Value::BulkString(b) => crate::redis_search::preview(b),
        redis::Value::SimpleString(s) => crate::redis_search::preview(s.as_bytes()),
        redis::Value::Nil => "(nil)".into(),
        redis::Value::Int(n) => n.to_string(),
        redis::Value::Double(n) => n.to_string(),
        redis::Value::Boolean(v) => v.to_string(),
        _ => "[nested value]".into(),
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn profile(value: redis::Value, description: String) -> Profile {
    fn visit(v: &redis::Value, path: String, depth: usize, steps: &mut usize, out: &mut Profile) {
        if out.rows.len() >= 1000 || depth > 12 || *steps >= 4096 {
            out.truncated = true;
            return;
        }
        *steps += 1;
        if let redis::Value::Array(values) = v {
            let (pairs, rest) = values.as_chunks::<2>();
            let paired = rest.is_empty()
                && pairs.iter().take(1001).all(|p| {
                    matches!(
                        p[0],
                        redis::Value::BulkString(_) | redis::Value::SimpleString(_)
                    )
                });
            if paired {
                for pair in pairs {
                    if out.rows.len() >= 1000 || *steps >= 4096 {
                        out.truncated = true;
                        break;
                    }
                    let name = scalar(&pair[0]).chars().take(120).collect::<String>();
                    visit(
                        &pair[1],
                        if path.is_empty() {
                            name
                        } else {
                            format!("{path} / {name}")
                        },
                        depth + 1,
                        steps,
                        out,
                    );
                }
            } else {
                for (i, v) in values.iter().enumerate() {
                    if out.rows.len() >= 1000 || *steps >= 4096 {
                        out.truncated = true;
                        break;
                    }
                    visit(v, format!("{path} [{}]", i + 1), depth + 1, steps, out);
                }
            }
        } else {
            out.rows.push([path, scalar(v)]);
        }
    }
    let mut report = Profile {
        rows: vec![],
        truncated: false,
        client_ms: 0.0,
        description,
    };
    visit(&value, String::new(), 0, &mut 0, &mut report);
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_limits_aliases_and_page_lookahead() {
        use redis::Value::{Array, BulkString, Int};
        let q = Query {
            index: b"idx".to_vec(),
            query: "*".into(),
            groups: vec![b"__rustable_count".to_vec()],
            ..Default::default()
        };
        let mut cmd = redis::cmd("FT.AGGREGATE");
        q.append(&mut cmd).unwrap();
        let row = || {
            Array(vec![
                BulkString(b"__rustable_count".to_vec()),
                BulkString(b"group".to_vec()),
                BulkString(b"__rustable_count_".to_vec()),
                Int(3),
            ])
        };
        let mut reply = vec![Int(51)];
        reply.extend((0..51).map(|_| row()));
        let result = q.clone().decode(Array(reply)).unwrap();
        assert!(result.more);
        assert_eq!(result.rows.len(), 50);
        assert_eq!(result.rows[0], vec!["group", "3"]);
        let mut invalid = q.clone();
        invalid.groups = vec![b"a".to_vec(); 4];
        assert!(invalid.append(&mut cmd).is_err());
        invalid = q.clone();
        invalid.offset = 10000;
        assert!(invalid.append(&mut cmd).is_err());
        invalid = q;
        invalid.reducer = Reducer::Sum;
        assert!(invalid.append(&mut cmd).is_err());
        assert!(invalid.decode(Array(vec![Int(0)])).is_err());
    }
    #[test]
    fn profile_tree_is_bounded_and_keeps_nested_details() {
        use redis::Value::{Array, BulkString, Int};
        let p = profile(
            Array(vec![
                BulkString(b"Shards".to_vec()),
                Array(vec![Array(vec![
                    BulkString(b"Total profile time".to_vec()),
                    BulkString(b"0.25".to_vec()),
                ])]),
            ]),
            "Fixture".into(),
        );
        assert_eq!(p.rows[0][1], "0.25");
        assert!(p.rows[0][0].contains("Total profile time"));
        assert!(!p.truncated);
        let p = profile(Array((0..20000).map(Int).collect()), "Wide".into());
        assert!(p.truncated);
        assert_eq!(p.rows.len(), 1000);
        let mut deep = Int(1);
        for _ in 0..30 {
            deep = Array(vec![deep]);
        }
        assert!(profile(deep, "Deep".into()).truncated);
    }
}
