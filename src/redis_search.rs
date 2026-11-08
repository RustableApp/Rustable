//! Read-only Redis Search with explicit, bounded field previews.
use eframe::egui;
const PAGE: usize = 50;
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Options {
    pub fields: Vec<Vec<u8>>,
    pub sort: Option<Vec<u8>>,
    pub descending: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Field {
    pub name: Vec<u8>,
    pub path: String,
    pub kind: String,
    pub sortable: bool,
}
struct Results {
    request: Request,
    total: usize,
    keys: Vec<Vec<u8>>,
    values: Vec<Vec<String>>,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Request {
    Aggregate(crate::redis_analytics::Query),
    Profile(Box<Request>),
    Indexes,
    Metadata(Vec<Vec<u8>>),
    Schema(Vec<u8>),
    Query {
        index: Vec<u8>,
        query: String,
        offset: usize,
        options: Options,
    },
}
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) enum Reply {
    Aggregate(crate::redis_analytics::Report),
    Profile {
        reply: Box<Reply>,
        profile: crate::redis_analytics::Profile,
    },
    Indexes(Vec<Vec<u8>>),
    Metadata(crate::redis_search_catalog::Rows),
    Schema(Vec<Field>),
    Results {
        request: Request,
        total: usize,
        keys: Vec<Vec<u8>>,
        values: Vec<Vec<String>>,
    },
}
#[derive(Default)]
pub(crate) struct View {
    pub open: bool,
    pub pending: bool,
    pub error: Option<String>,
    indexes: Vec<Vec<u8>>,
    catalog_loaded: bool,
    catalog_page: usize,
    metadata: std::collections::BTreeMap<Vec<u8>, Result<crate::redis_search_catalog::Metadata, String>>,
    metadata_attempted: std::collections::BTreeSet<usize>,
    index: Option<Vec<u8>>,
    schema: Vec<Field>,
    options: Options,
    query: String,
    results: Option<Results>,
    aggregate_mode: bool,
    aggregate: crate::redis_analytics::Draft,
    aggregate_report: Option<crate::redis_analytics::Report>,
    profile: Option<crate::redis_analytics::Profile>,
    profile_visible: bool,
    collapse_schema: bool,
}
pub(crate) enum Action {
    Request(Request),
    Inspect(Vec<u8>),
}
impl View {
    pub fn loaded_indexes(&self) -> &[Vec<u8>] { &self.indexes }
    #[cfg(feature = "qa")]
    pub fn preview_request(&mut self, step: usize) -> Request {
        self.open = true;
        if step == 0 {
            return Request::Indexes;
        }
        let index = b"idx:users".to_vec();
        self.index = Some(index.clone());
        if step == 1 {
            return Request::Schema(index);
        }
        if std::env::var_os("RUSTABLE_REDIS_ANALYTICS_CAPTURE").is_some() {
            self.query = "*".into();
            if step == 4 {
                self.aggregate_mode = false;
                return Request::Profile(Box::new(Request::Query {
                    index,
                    query: "*".into(),
                    offset: 0,
                    options: Options::default(),
                }));
            }
            self.aggregate_mode = true;
            self.aggregate = crate::redis_analytics::Draft {
                groups: vec![b"plan".to_vec()],
                metric: Some(b"monthly_spend".to_vec()),
                reducer: if step == 2 {
                    crate::redis_analytics::Reducer::Count
                } else {
                    crate::redis_analytics::Reducer::Sum
                },
            };
            let request =
                Request::Aggregate(self.aggregate.request(index, "*".into(), &self.schema));
            return if step == 5 {
                Request::Profile(Box::new(request))
            } else {
                request
            };
        }
        self.query = match step {
            3 => "nothingmatches",
            4 => "@name:[",
            _ => "*",
        }
        .into();
        self.options = Options {
            fields: vec![b"name".to_vec(), b"plan".to_vec(), b"country".to_vec()],
            sort: Some(b"name".to_vec()),
            descending: false,
        };
        Request::Query {
            index,
            query: self.query.clone(),
            offset: 0,
            options: self.options.clone(),
        }
    }
    pub fn needs_indexes(&self) -> bool { !self.catalog_loaded && !self.pending }
    pub fn begin(&mut self) {
        self.pending = true;
        self.error = None;
    }
    pub fn receive(&mut self, reply: Reply) {
        self.pending = false;
        self.error = None;
        match reply {
            Reply::Aggregate(report) => {
                self.aggregate_report = Some(report);
                self.aggregate_mode = true;
                self.profile = None;
                self.profile_visible = false;
            }
            Reply::Profile { reply, profile } => {
                self.receive(*reply);
                self.profile = Some(profile);
                self.profile_visible = true;
                self.collapse_schema = true;
            }
            Reply::Metadata(rows) => { self.metadata.extend(rows); }
            Reply::Indexes(indexes) => {
                self.metadata.clear(); self.metadata_attempted.clear(); self.catalog_page = 0;
                self.catalog_loaded = true;
                if self.index.as_ref().is_some_and(|i| !indexes.contains(i)) {
                    self.index = None;
                    self.schema.clear();
                    self.results = None;
                    self.aggregate = Default::default();
                    self.aggregate_report = None;
                    self.profile = None;
                    self.profile_visible = false;
                }
                self.indexes = indexes;
            }
            Reply::Schema(schema) => {
                self.options
                    .fields
                    .retain(|name| schema.iter().any(|field| &field.name == name));
                if self.options.sort.as_ref().is_some_and(|name| {
                    !schema
                        .iter()
                        .any(|field| field.sortable && &field.name == name)
                }) {
                    self.options.sort = None;
                }
                self.aggregate.reconcile(&schema);
                self.schema = schema;
            }
            Reply::Results {
                request,
                total,
                keys,
                values,
            } => {
                self.aggregate_mode = false;
                self.profile = None;
                self.profile_visible = false;
                self.results = Some(Results {
                    request,
                    total,
                    keys,
                    values,
                })
            }
        }
    }
    fn current_request(&self, index: Vec<u8>) -> Request {
        let query = if self.query.trim().is_empty() {
            "*".into()
        } else {
            self.query.clone()
        };
        if self.aggregate_mode {
            Request::Aggregate(self.aggregate.request(index, query, &self.schema))
        } else {
            Request::Query {
                index,
                query,
                offset: 0,
                options: self.options.clone(),
            }
        }
    }
    pub fn show(&mut self, ctx: &egui::Context, enabled: bool) -> Option<Action> {
        if !self.open {
            return None;
        }
        let mut open = true;
        let mut action = None;
        egui::Window::new("Search indexes").open(&mut open).default_width(720.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .frame(egui::Frame::window(&ctx.style()).inner_margin(16).corner_radius(10.0))
            .default_height(500.0).max_height(620.0).vscroll(true).collapsible(false).show(ctx, |ui| {
            action = self.contents(ui, enabled);
        });
        self.open = open;
        action
    }
    pub fn pane(&mut self, ui: &mut egui::Ui, enabled: bool) -> Option<Action> {
        if self.index.is_some() {
            return self.contents(ui, enabled);
        }
        let mut action = None;
        ui.horizontal(|ui| {
            ui.heading("Search indexes");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(enabled && !self.pending, crate::redis_workspace::action_button("refresh-cw", "Refresh")).clicked() {
                    action = Some(Action::Request(Request::Indexes));
                }
            });
        });
        ui.add_space(16.0);
        use egui_extras::{Column, TableBuilder};
        let start = self.catalog_page * crate::redis_search_catalog::PAGE_SIZE;
        let end = (start + crate::redis_search_catalog::PAGE_SIZE).min(self.indexes.len());
        crate::controls::table_surface(ui, |ui| {
        TableBuilder::new(ui).cell_layout(egui::Layout::left_to_right(egui::Align::Center)).id_salt("redis-index-catalog").striped(true)
            .column(Column::remainder().clip(true)).column(Column::remainder().clip(true))
            .column(Column::initial(120.0).clip(true)).column(Column::exact(80.0)).column(Column::exact(60.0)).column(Column::exact(90.0))
            .header(32.0, |mut row| { for title in ["Index name", "Prefixes", "Field types", "Documents", "Fields", ""] {
                row.col(|ui| { crate::controls::table_header(ui, title); });
            } })
            .body(|body| body.rows(32.0, end.saturating_sub(start), |mut row| {
                let index = &self.indexes[start + row.index()];
                let metadata = self.metadata.get(index);
                row.col(|ui| { let response = ui.monospace(label(index)); if let Some(Err(error)) = metadata { response.on_hover_text(error); } });
                let info = metadata.and_then(|value| value.as_ref().ok());
                row.col(|ui| { ui.add(egui::Label::new(info.map_or("—", |m| m.prefixes.as_str())).truncate()); });
                row.col(|ui| { ui.add(egui::Label::new(info.map_or("—", |m| m.types.as_str())).truncate()); });
                row.col(|ui| { ui.label(info.and_then(|m| m.documents).map_or_else(|| "—".into(), |n| n.to_string())); });
                row.col(|ui| { ui.label(info.map_or_else(|| "—".into(), |m| m.fields.to_string())); });
                row.col(|ui| {
                    if ui.add_enabled(enabled && !self.pending, crate::redis_workspace::action_button("search", "Query")).clicked() {
                        self.index = Some(index.clone()); self.schema.clear(); self.results = None;
                        self.options = Options::default(); self.aggregate = Default::default();
                        self.aggregate_report = None; self.profile = None; self.profile_visible = false;
                        action = Some(Action::Request(Request::Schema(index.clone())));
                    }
                });
            }));
        });
        ui.horizontal(|ui| {
            ui.weak(format!("{} indexes · {}–{} shown", self.indexes.len(), if end == 0 {0} else {start + 1}, end));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(enabled && !self.pending && end < self.indexes.len(), crate::redis_workspace::action_button("chevron-right", "Next")).clicked() { self.catalog_page += 1; }
                if ui.add_enabled(enabled && !self.pending && self.catalog_page > 0, crate::redis_workspace::action_button("chevron-left", "Previous")).clicked() { self.catalog_page -= 1; }
            });
        });
        let start = self.catalog_page * crate::redis_search_catalog::PAGE_SIZE;
        let end = (start + crate::redis_search_catalog::PAGE_SIZE).min(self.indexes.len());
        if action.is_none() && enabled && !self.pending && self.catalog_loaded && start < end && self.metadata_attempted.insert(self.catalog_page) {
            action = Some(Action::Request(Request::Metadata(self.indexes[start..end].to_vec())));
        }
        if self.pending { ui.spinner(); }
        if let Some(error) = &self.error { ui.colored_label(ui.visuals().error_fg_color, error); }
        if self.indexes.is_empty() && !self.pending { ui.weak("No search indexes loaded. Refresh to discover indexes on this connection."); }
        action
    }
    fn contents(&mut self, ui: &mut egui::Ui, enabled: bool) -> Option<Action> {
        let mut action = None;
            crate::redis_workspace::browser_style(ui);

            ui.add_enabled_ui(enabled && !self.pending, |ui| {
                ui.horizontal(|ui| {
                    if ui.add(crate::redis_workspace::action_button("chevron-left", "Indexes")).clicked() { self.index = None; }
                    ui.weak("/");
                    let before = self.index.clone();
                    egui::ComboBox::from_id_salt("search-index").width(240.0)
                        .selected_text(self.index.as_ref().map_or("Choose an index".into(), |i| label(i)))
                        .show_ui(ui, |ui| {
                            egui::ScrollArea::vertical().max_height(240.0).show_rows(ui, 23.0, self.indexes.len(), |ui, range| {
                                for i in range { let key = &self.indexes[i]; ui.selectable_value(&mut self.index, Some(key.clone()), label(key)); }
                            });
                        });
                    if before != self.index {
                        self.schema.clear(); self.results = None; self.options = Options::default();self.aggregate=Default::default();self.aggregate_report=None;self.profile=None;self.profile_visible=false;
                        action = self.index.clone().map(|i| Action::Request(Request::Schema(i)));
                    }
                    if crate::redis_value_editor::icon(ui, "refresh-cw", "Refresh indexes", true).clicked() { action = Some(Action::Request(Request::Indexes)); }
                    if ui.add_enabled(self.index.is_some(), egui::Button::new("Refresh schema")).clicked() && let Some(index) = &self.index {
                        action = Some(Action::Request(Request::Schema(index.clone())));
                    }
                });
                if !self.schema.is_empty() {
                    let close_schema=self.collapse_schema;self.collapse_schema=false;
                    egui::CollapsingHeader::new("View index schema").default_open(false).open(close_schema.then_some(false)).show(ui, |ui| {
                        use egui_extras::{Column, TableBuilder};
                        TableBuilder::new(ui).cell_layout(egui::Layout::left_to_right(egui::Align::Center)).id_salt("search-schema").striped(true).max_scroll_height(130.0)
                            .column(Column::remainder().clip(true)).column(Column::remainder().clip(true)).column(Column::exact(110.0))
                            .header(25.0, |mut r| { for s in ["FIELD", "PATH", "TYPE"] { r.col(|ui| { crate::controls::table_header(ui, s); }); } })
                            .body(|body| body.rows(23.0, self.schema.len(), |mut r| {
                                let field=&self.schema[r.index()];
                                for v in [label(&field.name),field.path.clone(),format!("{}{}",field.kind,if field.sortable {" ↕"}else{""})] { r.col(|ui| { ui.add(egui::Label::new(&v).truncate()).on_hover_text(v); }); }
                            }));
                    });
                }
                egui::Frame::group(ui.style()).inner_margin(12).show(ui, |ui| {
                ui.strong("Query editor");
                ui.separator();
                egui::CollapsingHeader::new("Query options").show(ui, |ui| {
                let before_mode=self.aggregate_mode;
                crate::controls::segments(ui,"search-query-mode",&mut self.aggregate_mode,&[(false,"Documents",94.0),(true,"Aggregate",94.0)]);
                if before_mode!=self.aggregate_mode {self.profile=None;self.profile_visible=false;}
                ui.add_space(6.0);
                if self.aggregate_mode {self.aggregate.show(ui,&self.schema);} else {
                ui.horizontal(|ui| {
                    ui.menu_button(format!("Fields · {}",self.options.fields.len()), |ui| {
                        ui.weak("Choose up to 8 preview fields"); ui.weak("Select small fields; use Inspect for large values.");
                        egui::ScrollArea::vertical().max_height(240.0).show_rows(ui,23.0,self.schema.len(),|ui,range| {
                            for i in range {
                                let field=&self.schema[i]; let mut selected=self.options.fields.contains(&field.name);
                                if ui.add_enabled(selected || self.options.fields.len()<8, egui::Checkbox::new(&mut selected,label(&field.name))).changed() {
                                    if selected {self.options.fields.push(field.name.clone());} else {self.options.fields.retain(|name|name!=&field.name);}
                                }
                            }
                        });
                    });
                    egui::ComboBox::from_id_salt("search-sort").selected_text(self.options.sort.as_ref().map_or("Unsorted".into(),|v|format!("Sort: {}",label(v)))).show_ui(ui,|ui| {
                        ui.selectable_value(&mut self.options.sort,None,"Unsorted");
                        egui::ScrollArea::vertical().max_height(240.0).show_rows(ui,23.0,self.schema.len(),|ui,range| {
                            for i in range {let field=&self.schema[i];if field.sortable {ui.selectable_value(&mut self.options.sort,Some(field.name.clone()),label(&field.name));}}
                        });
                    });
                    ui.add_enabled_ui(self.options.sort.is_some(),|ui| {ui.checkbox(&mut self.options.descending,"Descending");});
                });
                }
                });
                ui.add_space(8.0);
                let query_id = ui.make_persistent_id("search-query-editor");
                let shortcut = enabled && !self.pending && ui.memory(|m| m.has_focus(query_id)) && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
                crate::redis_command_editor::panel(ui, 10, |ui| {
                    ui.horizontal_top(|ui| {
                        let (gutter, _) = ui.allocate_exact_size(egui::vec2(32.0, 180.0), egui::Sense::hover());
                        let mut layouter = crate::redis_command_editor::layout;
                        let output = egui::TextEdit::multiline(&mut self.query).id(query_id).code_editor().frame(false).layouter(&mut layouter)
                            .hint_text("* or @field:value").char_limit(4096).desired_rows(10).desired_width(f32::INFINITY).show(ui);
                        crate::editor::paint_line_numbers(ui, gutter, &output, 1);
                    });
                });
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if (ui.add_enabled(self.index.is_some(), crate::icons::query_run_button("Run query", false, egui::Color32::from_rgb(50, 104, 224))).clicked() || shortcut)
                            && let Some(index) = &self.index {
                            action = Some(Action::Request(self.current_request(index.clone())));
                        }
                        if ui.add_enabled(self.index.is_some(), crate::redis_workspace::action_button("search", "Profile query")).clicked() && let Some(index) = &self.index {
                            action = Some(Action::Request(Request::Profile(Box::new(self.current_request(index.clone())))));
                        }
                    });
                });
                });
                ui.add_space(12.0);
                egui::Frame::group(ui.style()).inner_margin(12).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.strong("Results");
                ui.separator();
                ui.weak("50 rows per page · up to 10,000 rows · previews may be truncated");
                if self.profile.is_some() {
                    crate::controls::segments(ui,"search-result-mode",&mut self.profile_visible,&[(false,"Results",74.0),(true,"Profile",74.0)]);
                }
                if self.profile_visible {
                    if let Some(profile)=&self.profile {profile.show(ui);}
                } else if self.aggregate_mode {
                    if let Some(report)=&self.aggregate_report {
                        if let Some(query)=report.show(ui) {action=Some(Action::Request(Request::Aggregate(query)));}
                    } else {ui.weak("Choose grouping and a reducer, then run the aggregation.");}
                } else {
                if let Some(Results {request: Request::Query { index, query, offset, options }, total, keys, values}) = &self.results {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        for (title, available, next) in [("Previous", *offset > 0, offset.saturating_sub(PAGE)), ("Next", offset + keys.len() < *total && keys.len() == PAGE && *offset < 9950, offset + PAGE)] {
                            if ui.add_enabled(available, egui::Button::new(title)).clicked() {
                                action = Some(Action::Request(Request::Query { index: index.clone(), query: query.clone(), offset: next, options: options.clone() }));
                            }
                        }
                        ui.weak(format!("{} matches · {} shown", total, keys.len()));
                    });
                    ui.small(format!("Results for {} · {} · offset {}", label(index), query, offset));
                    ui.weak(if options.sort.is_some() {"Sorted results can change between pages; duplicate sort values may repeat or skip keys."}else{"Results can change between pages; unsorted searches may repeat or skip keys."});
                    if options != &self.options {ui.weak("Field or sort settings changed. Search again to apply them.");}
                    if keys.is_empty() { ui.add_space(16.0); ui.strong("No matching documents"); ui.weak("Try * to see indexed keys, or adjust the query."); }
                    use egui_extras::{Column, TableBuilder};
                    egui::ScrollArea::horizontal().id_salt("search-columns").show(ui,|ui| {
                        let mut table=TableBuilder::new(ui).cell_layout(egui::Layout::left_to_right(egui::Align::Center)).id_salt(("search-results",&options.fields)).striped(true).max_scroll_height(270.0).column(Column::initial(260.0).clip(true));
                        for _ in &options.fields {table=table.column(Column::initial(130.0).clip(true));}
                        table
                            .header(26.0, |mut row| {
                                row.col(|ui| {ui.weak("MATCHING KEY");});
                                for field in &options.fields {row.col(|ui| {ui.add(egui::Label::new(label(field)).truncate());});}
                            })
                            .body(|body| body.rows(28.0, keys.len(), |mut row| {
                                let i=row.index(); let key = &keys[i];
                                row.col(|ui| {ui.horizontal(|ui| {
                                    ui.add_sized([ui.available_width()-72.0,23.0],egui::Label::new(egui::RichText::new(label(key)).monospace()).truncate()).on_hover_text(label(key));
                                    if ui.button("Inspect").clicked() {action=Some(Action::Inspect(key.clone()));}
                                });});
                                for cell in &values[i] {row.col(|ui| {ui.add(egui::Label::new(cell).truncate()).on_hover_text(cell);});}
                            }));
                    });
                } else if self.index.is_some() && self.error.is_none() && !self.pending {
                    ui.set_min_height(220.0);
                    ui.add_space(70.0);
                    ui.vertical_centered(|ui| {
                        ui.heading("Query results");
                        ui.weak("Run a query to see matching documents.");
                    });
                } else if self.indexes.is_empty() && self.error.is_none() && !self.pending {
                    ui.add_space(16.0); ui.strong("No search indexes");
                    ui.weak("Refresh to discover existing indexes. This panel does not create or modify indexes.");
                }
                }
                });
            });
            if self.pending { ui.horizontal(|ui| { ui.spinner(); ui.label("Loading…"); }); }
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
                ui.weak("Check Search support and permissions, then retry. Your query is retained.");
            }

        action
    }

}
fn label(bytes: &[u8]) -> String {
    let text = crate::redis_commands::quote(&bytes[..bytes.len().min(512)]);
    if bytes.len() > 512 {
        format!("{text}…")
    } else {
        text
    }
}
#[cfg(not(target_arch = "wasm32"))]
impl Request {
    pub fn command(&self) -> Result<redis::Cmd, String> {
        let mut cmd;
        match self {
            Self::Metadata(_) => return Err("Catalog metadata requires the bounded batch path.".into()),
            Self::Indexes => cmd = redis::cmd("FT._LIST"),
            Self::Schema(index) => {
                validate_index(index)?;
                cmd = redis::cmd("FT.INFO");
                cmd.arg(index);
            }
            Self::Query { index, .. } => {
                cmd = redis::cmd("FT.SEARCH");
                cmd.arg(index);
                self.append_query(&mut cmd)?;
            }
            Self::Aggregate(q) => {
                cmd = redis::cmd("FT.AGGREGATE");
                cmd.arg(&q.index);
                self.append_query(&mut cmd)?;
            }
            Self::Profile(request) => {
                let (index, kind) = match &**request {
                    Self::Query { index, .. } => (index, "SEARCH"),
                    Self::Aggregate(q) => (&q.index, "AGGREGATE"),
                    _ => return Err("Only Search and aggregation queries can be profiled.".into()),
                };
                cmd = redis::cmd("FT.PROFILE");
                cmd.arg(index).arg(kind).arg("LIMITED").arg("QUERY");
                request.append_query(&mut cmd)?;
            }
        }
        Ok(cmd)
    }
    fn append_query(&self, cmd: &mut redis::Cmd) -> Result<(), String> {
        match self {
            Self::Aggregate(q) => q.append(cmd),
            Self::Query {
                index,
                query,
                offset,
                options,
            } => {
                validate_index(index)?;
                if query.len() > 4096 || query.trim().is_empty() || *offset > 9950 {
                    return Err(
                        "Query must contain 1–4,096 bytes; paging is limited to 10,000 matches."
                            .into(),
                    );
                }
                if options.fields.len() > 8
                    || options.fields.iter().any(|f| f.len() > 4096)
                    || options.sort.as_ref().is_some_and(|f| f.len() > 4096)
                {
                    return Err(
                        "Choose up to 8 fields; field names are limited to 4,096 bytes.".into(),
                    );
                }
                cmd.arg(query);
                if options.fields.is_empty() {
                    cmd.arg("NOCONTENT");
                } else {
                    cmd.arg("RETURN")
                        .arg(options.fields.len())
                        .arg(&options.fields);
                }
                if let Some(sort) = &options.sort {
                    cmd.arg("SORTBY").arg(sort).arg(if options.descending {
                        "DESC"
                    } else {
                        "ASC"
                    });
                }
                cmd.arg("LIMIT")
                    .arg(offset)
                    .arg(PAGE)
                    .arg("TIMEOUT")
                    .arg(1000)
                    .arg("DIALECT")
                    .arg(2);
                Ok(())
            }
            _ => Err("Expected a Search or aggregation query.".into()),
        }
    }
    pub fn decode(self, value: redis::Value) -> Result<Reply, String> {
        use redis::Value;
        if let Self::Aggregate(q) = self {
            return q.decode(value).map(Reply::Aggregate);
        }
        if let Self::Profile(request) = self {
            let description = match &*request {
                Self::Query { index, query, .. } => {
                    format!("Documents · {} · {query}", label(index))
                }
                Self::Aggregate(q) => format!("Aggregation · {} · {}", label(&q.index), q.query),
                _ => return Err("Invalid profile request.".into()),
            };
            let Value::Array(mut parts) = value else {
                return Err("Unexpected profile response.".into());
            };
            if parts.len() != 2 {
                return Err("Expected query results and a profile report.".into());
            }
            let details = parts.pop().unwrap();
            if !matches!(details, Value::Array(_)) {
                return Err("Unexpected profile details.".into());
            }
            let profile = crate::redis_analytics::profile(details, description);
            let reply = request.decode(parts.pop().unwrap())?;
            return Ok(Reply::Profile {
                reply: Box::new(reply),
                profile,
            });
        }
        let Value::Array(values) = value else {
            return Err("Unexpected Search response.".into());
        };
        match self {
            Self::Aggregate(_) | Self::Profile(_) | Self::Metadata(_) => Err("Unexpected query decoder.".into()),
            Self::Indexes => {
                if values.len() > 1000 {
                    return Err("More than 1,000 indexes returned; use the command editor to narrow inspection.".into());
                }
                let mut indexes = Vec::with_capacity(values.len());
                let mut byte_count = 0;
                for v in values {
                    let b: Vec<u8> = bytes(v)?;
                    byte_count += b.len();
                    if byte_count > 1024 * 1024 {
                        return Err("Index names exceed the 1 MiB display limit.".into());
                    }
                    indexes.push(b);
                }
                indexes.sort();
                Ok(Reply::Indexes(indexes))
            }
            Self::Schema(_) => {
                let attrs = values
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .find(|p| crate::redis_value::text(&p[0]) == "attributes")
                    .map(|p| &p[1]);
                let Some(Value::Array(attrs)) = attrs else {
                    return Err("Missing Search index schema.".into());
                };
                if attrs.len() > 1000 {
                    return Err("Schema exceeds 1,000 fields.".into());
                }
                let mut rows = Vec::new();
                for attr in attrs {
                    if let Value::Array(parts) = attr {
                        let get = |key: &str| {
                            parts
                                .windows(2)
                                .find(|p| crate::redis_value::text(&p[0]) == key)
                                .map_or(String::new(), |p| {
                                    crate::redis_value::text(&p[1]).chars().take(512).collect()
                                })
                        };
                        let raw_name = parts
                            .windows(2)
                            .find(|p| crate::redis_value::text(&p[0]) == "attribute")
                            .ok_or("Missing field name")?;
                        let name = bytes(raw_name[1].clone())?;
                        if name.len() > 4096 {
                            return Err("Search schema field name exceeds 4,096 bytes.".into());
                        }
                        rows.push(Field {
                            name,
                            path: get("identifier"),
                            kind: get("type"),
                            sortable: parts.iter().enumerate().any(|(i, p)| {
                                crate::redis_value::text(p) == "SORTABLE"
                                    && (i == 0
                                        || !matches!(
                                            crate::redis_value::text(&parts[i - 1]).as_str(),
                                            "identifier" | "attribute" | "type"
                                        ))
                            }),
                        });
                    }
                }
                Ok(Reply::Schema(rows))
            }
            Self::Query { ref options, .. } => {
                let mut values = values.into_iter();
                let total: usize =
                    redis::from_redis_value(values.next().ok_or("Missing result count")?)
                        .map_err(|e| e.to_string())?;
                let mut keys = Vec::new();
                let mut documents = Vec::new();
                let mut byte_count = 0;
                while let Some(value) = values.next() {
                    let key: Vec<u8> = bytes(value)?;
                    byte_count += key.len();
                    if byte_count > 1024 * 1024 || keys.len() == PAGE {
                        return Err("Search keys exceed the bounded page limit.".into());
                    }
                    let mut cells = vec!["—".into(); options.fields.len()];
                    if !options.fields.is_empty() {
                        match values.next().ok_or("Missing document fields")? {
                            Value::Nil => {}
                            Value::Array(fields) => {
                                let (pairs, rest) = fields.as_chunks::<2>();
                                if !rest.is_empty() {
                                    return Err("Malformed document fields.".into());
                                }
                                for pair in pairs {
                                    let name = bytes(pair[0].clone())?;
                                    if let Some(i) = options.fields.iter().position(|f| f == &name)
                                    {
                                        cells[i] = match &pair[1] {
                                            Value::BulkString(b) => preview(b),
                                            Value::SimpleString(t) => preview(t.as_bytes()),
                                            Value::Nil => "(nil)".into(),
                                            _ => {
                                                return Err("Unexpected Search field value.".into());
                                            }
                                        };
                                    }
                                }
                            }
                            _ => return Err("Unexpected Search fields.".into()),
                        }
                    }
                    keys.push(key);
                    documents.push(cells);
                }
                Ok(Reply::Results {
                    request: self,
                    total,
                    keys,
                    values: documents,
                })
            }
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn preview(bytes: &[u8]) -> String {
    let prefix = &bytes[..bytes.len().min(4096)];
    let text = match std::str::from_utf8(prefix) {
        Ok(s) => s.to_owned(),
        Err(_) => crate::redis_commands::quote(prefix),
    };
    if prefix.len() < bytes.len() {
        format!("{text}… [truncated]")
    } else {
        text
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn bytes(value: redis::Value) -> Result<Vec<u8>, String> {
    match value {
        redis::Value::BulkString(b) => Ok(b),
        redis::Value::SimpleString(s) => Ok(s.into_bytes()),
        _ => Err("Expected a Search string.".into()),
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn validate_index(index: &[u8]) -> Result<(), String> {
    if index.len() > 4096 {
        Err("Index name exceeds 4,096 bytes.".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn catalog_revisits_reuse_empty_and_populated_results() {
        let mut view = super::View::default();
        assert!(view.needs_indexes());
        view.begin();
        assert!(!view.needs_indexes());
        view.receive(super::Reply::Indexes(vec![]));
        assert!(!view.needs_indexes(), "an empty catalog is still a loaded catalog");
        view.receive(super::Reply::Indexes(vec![b"idx:users".to_vec()]));
        assert!(!view.needs_indexes());
    }

    use super::*;
    #[test]
    fn query_and_schema_decoding_are_bounded_and_binary_safe() {
        use redis::Value::{Array, BulkString, Int};
        let request = Request::Query {
            index: b"index with space".to_vec(),
            query: "@name:avery".into(),
            offset: 50,
            options: Options::default(),
        };
        let args: Vec<Vec<u8>> = redis::from_redis_value(
            redis::Parser::new()
                .parse_value(&mut std::io::Cursor::new(
                    request.command().unwrap().get_packed_command(),
                ))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(args[1], b"index with space");
        assert_eq!(args[3], b"NOCONTENT");
        assert_eq!(args[5], b"50");
        let Reply::Results { total, keys, .. } = request
            .clone()
            .decode(Array(vec![Int(51), BulkString(vec![0, 255])]))
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(total, 51);
        assert_eq!(keys, vec![vec![0, 255]]);
        assert!(request.clone().decode(Array(vec![Int(0); 52])).is_err());
        let Request::Query { index, .. } = request else {
            unreachable!()
        };
        assert!(
            Request::Query {
                index,
                query: "x".repeat(4097),
                offset: 0,
                options: Default::default(),
            }
            .command()
            .is_err()
        );
        let field = |s: &str| BulkString(s.as_bytes().to_vec());
        let Reply::Schema(rows) = Request::Schema(b"idx".to_vec())
            .decode(Array(vec![
                field("attributes"),
                Array(vec![Array(vec![
                    field("identifier"),
                    field("$.name"),
                    field("attribute"),
                    field("name"),
                    field("type"),
                    field("TEXT"),
                    field("SORTABLE"),
                ])]),
            ]))
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(rows[0].name, b"name");
        assert_eq!(rows[0].path, "$.name");
        assert!(rows[0].sortable);
    }
    #[test]
    fn fields_are_ordered_missing_safe_and_bounded() {
        use redis::Value::{Array, BulkString, Int, Nil};
        let request = Request::Query {
            index: b"i".to_vec(),
            query: "*".into(),
            offset: 0,
            options: Options {
                fields: vec![b"second".to_vec(), b"first".to_vec(), b"missing".to_vec()],
                ..Default::default()
            },
        };
        let Reply::Results { values, .. } = request
            .clone()
            .decode(Array(vec![
                Int(2),
                BulkString(b"one".to_vec()),
                Array(vec![
                    BulkString(b"first".to_vec()),
                    BulkString(vec![255; 9000]),
                    BulkString(b"second".to_vec()),
                    BulkString(b"1234567890123456789".to_vec()),
                ]),
                BulkString(b"expired".to_vec()),
                Nil,
            ]))
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(values[0][0], "1234567890123456789");
        assert!(values[0][1].ends_with("[truncated]"));
        assert!(values[0][1].len() < 17000);
        assert_eq!(values[0][2], "—");
        assert_eq!(values[1], vec!["—"; 3]);
        assert!(
            request
                .decode(Array(vec![Int(1), BulkString(b"key".to_vec())]))
                .is_err()
        );
    }
    #[test]
    fn refresh_drops_deleted_index_and_query_results() {
        let mut view = View {
            index: Some(b"gone".to_vec()),
            pending: true,
            ..Default::default()
        };
        view.receive(Reply::Indexes(vec![b"other".to_vec()]));
        assert!(view.index.is_none());
        assert!(!view.pending);
    }
    #[test]
    fn search_settles_at_idle() {
        let ctx = egui::Context::default();
        let mut view = View {
            open: true,
            ..Default::default()
        };
        let mut delay = std::time::Duration::ZERO;
        for frame in 0..20 {
            let out = ctx.run(
                egui::RawInput {
                    time: Some(frame as f64),
                    ..Default::default()
                },
                |ctx| {
                    assert!(view.show(ctx, true).is_none());
                },
            );
            delay = out.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        }
        assert!(delay > std::time::Duration::from_millis(500));
    }
    fn analytics_fixture(profile: bool) -> View {
        use crate::redis_analytics::{Draft, Profile, Query, Reducer, Report};
        let request = Query {
            index: b"idx:users".to_vec(),
            query: "*".into(),
            groups: vec![b"plan".to_vec()],
            metric: Some(b"spend".to_vec()),
            reducer: Reducer::Sum,
            ..Default::default()
        };
        View {
            open: true,
            index: Some(b"idx:users".to_vec()),
            indexes: vec![b"idx:users".to_vec()],
            query: "*".into(),
            aggregate_mode: true,
            schema: vec![
                Field {
                    name: b"plan".to_vec(),
                    path: "plan".into(),
                    kind: "TAG".into(),
                    sortable: true,
                },
                Field {
                    name: b"spend".to_vec(),
                    path: "spend".into(),
                    kind: "NUMERIC".into(),
                    sortable: true,
                },
            ],
            aggregate: Draft {
                groups: request.groups.clone(),
                metric: request.metric.clone(),
                reducer: Reducer::Sum,
            },
            aggregate_report: Some(Report {
                request,
                columns: vec!["Plan".into(), "Count".into(), "Sum spend".into()],
                rows: (0..50)
                    .map(|i| vec![format!("Group {i}"), "10".into(), "120".into()])
                    .collect(),
                more: true,
            }),
            profile: profile.then(|| Profile {
                description: "Aggregation · idx:users · *".into(),
                client_ms: 1.5,
                truncated: false,
                rows: (0..150)
                    .map(|i| {
                        [
                            format!("Shards [1] / Result processors [{i}] / Time"),
                            "0.125".into(),
                        ]
                    })
                    .collect(),
            }),
            profile_visible: profile,
            collapse_schema: profile,
            ..Default::default()
        }
    }
    #[test]
    fn profile_window_stays_inside_viewport_and_settles() {
        let ctx = egui::Context::default();
        let mut view = analytics_fixture(true);
        let mut delay = std::time::Duration::ZERO;
        for frame in 0..60 {
            let out = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 760.0),
                    )),
                    time: Some(frame as f64),
                    ..Default::default()
                },
                |ctx| {
                    assert!(view.show(ctx, true).is_none());
                },
            );
            delay = out.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        }
        let rect = ctx
            .memory(|m| m.area_rect(egui::Id::new("Search indexes")))
            .unwrap();
        assert!(
            rect.left() >= 0.0
                && rect.right() <= 1200.0
                && rect.top() >= 0.0
                && rect.bottom() <= 760.0,
            "{rect:?}"
        );
        assert!(delay > std::time::Duration::from_millis(500));
    }
    #[test]
    #[ignore = "manual aggregation and profile CPU workloads"]
    fn redis_analytics_frame_benchmark() {
        for profile in [false, true] {
            let ctx = egui::Context::default();
            let mut view = analytics_fixture(profile);
            let mut samples = vec![];
            for frame in 0..180 {
                if frame % 20 == 0 {
                    view.query.push('a');
                }
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
                        view.show(ctx, true);
                    },
                );
                let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
                if frame >= 20 {
                    samples.push(start.elapsed().as_secs_f64() * 1000.0);
                }
            }
            samples.sort_by(f64::total_cmp);
            crate::ui_benchmark::report(
                if profile {
                    "redis/query-profile"
                } else {
                    "redis/aggregation"
                },
                &samples,
            );
        }
    }
    #[test]
    #[ignore = "manual Search UI CPU workload"]
    fn redis_search_frame_benchmark() {
        search_benchmark(false);
    }
    #[test]
    #[ignore = "manual rich Search UI workload"]
    fn redis_search_fields_frame_benchmark() {
        search_benchmark(true);
    }
    fn search_benchmark(fields: bool) {
        let ctx = egui::Context::default();
        let mut view = View {
            open: true,
            indexes: vec![b"idx:users".to_vec()],
            index: Some(b"idx:users".to_vec()),
            schema: vec![Field {
                name: b"name".to_vec(),
                path: "$.name".into(),
                kind: "TEXT".into(),
                sortable: true,
            }],
            query: "*".into(),
            ..Default::default()
        };
        if fields {
            view.options = Options {
                fields: vec![b"name".to_vec(), b"plan".to_vec(), b"country".to_vec()],
                sort: Some(b"name".to_vec()),
                descending: false,
            };
        }
        view.receive(Reply::Results {
            request: Request::Query {
                index: b"idx:users".to_vec(),
                query: "*".into(),
                offset: 0,
                options: view.options.clone(),
            },
            values: vec![
                if fields {
                    vec!["Avery Chen".into(), "Pro".into(), "Canada".into()]
                } else {
                    vec![]
                };
                50
            ],
            total: 10000,
            keys: (0..50)
                .map(|i| format!("app:users:{i}").into_bytes())
                .collect(),
        });
        let mut samples = vec![];
        for frame in 0..180 {
            if fields && frame % 20 == 0 {
                view.query.push('a');
            }
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
                    view.show(ctx, true);
                },
            );
            let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
            if frame >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report(
            if fields {
                "redis/search-fields"
            } else {
                "redis/search"
            },
            &samples,
        );
    }
}
