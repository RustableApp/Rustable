// Full-page Redis tools share the connection toolbar and existing SQL controls.
impl Workspace {
    fn section_bar(&mut self, ctx: &egui::Context) {
        let before = self.section;
        egui::TopBottomPanel::top("redis-sections")
            .frame(egui::Frame::new().fill(ctx.style().visuals.panel_fill).inner_margin(8))
            .show(ctx, |ui| {
                browser_style(ui);
                ui.horizontal(|ui| {
                    let nav_width = 5.0 * 92.0 + 4.0 * ui.spacing().item_spacing.x;
                    let actions_width = if self.section == Section::Browse { 220.0 } else { 0.0 };
                    ui.add_space(((ui.available_width() - nav_width) * 0.5).min(ui.available_width() - nav_width - actions_width).max(0.0));
                    for (section, title) in [
                        (Section::Browse, "Browse"), (Section::Search, "Search"),
                        (Section::Workbench, "Workbench"), (Section::Analyze, "Analyze"),
                        (Section::PubSub, "Pub/Sub"),
                    ] {
                        let selected = self.section == section;
                        let response = ui.add(egui::Button::new(title).frame(false).min_size(egui::vec2(92.0, 30.0)));
                        if selected {
                            let r = response.rect;
                            ui.painter().line_segment([r.left_bottom(), r.right_bottom()], egui::Stroke::new(2.0_f32, ui.visuals().selection.bg_fill));
                        }
                        if response.clicked() { self.section = section; }
                    }
                    if self.section == Section::Browse {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add_enabled(self.connected && !self.busy && !self.read_only && self.edit.is_none(), action_button("plus", "Add key")).clicked() {
                                self.inspector_visible = true;
                                self.edit = Some(crate::redis_edit::Draft::new(crate::redis_mutation::Action::Create, vec![], "string".into()));
                            }
                            if ui.add_enabled(!self.busy, egui::Button::selectable(self.bulk_mode, "Bulk actions")).clicked() { self.bulk_mode = !self.bulk_mode; }
                        });
                    }
                });
            });
        if before != self.section && self.section == Section::Search && self.search.needs_indexes() && self.connected && !self.busy && self.edit.is_none() && !self.value_dirty() {
            self.search.open = false;
            self.search.begin();
            self.send(Job::Search(crate::redis_search::Request::Indexes));
        }
        if self.section == Section::PubSub {
            #[cfg(not(target_arch = "wasm32"))]
            { self.pubsub.open = false; }
        }
    }

    fn search_action(&mut self, action: crate::redis_search::Action) {
        if self.busy || !self.connected || self.value_dirty() || self.edit.is_some() { return; }
        match action {
            crate::redis_search::Action::Request(request) => {
                self.search.begin(); self.send(Job::Search(request));
            }
            crate::redis_search::Action::Mutate(mutation) => {
                match mutation.validate(self.read_only) {
                    Ok(()) => { self.search.begin(); self.send(Job::SearchMutation(mutation, self.read_only)); }
                    Err(error) => self.search.error = Some(error),
                }
            }
            crate::redis_search::Action::Inspect(key) => {
                self.search.open = false; self.section = Section::Browse;
                self.inspector_visible = true; self.send(Job::Inspect(key));
            }
        }
    }

    fn analysis_pane(&mut self, ui: &mut egui::Ui) {
        #[cfg(not(target_arch = "wasm32"))]
        self.analysis.poll(ui.ctx());
        egui::Frame::new().inner_margin(16).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.selectable_label(!self.analysis_server, "Database analysis").clicked() { self.analysis_server = false; }
                if ui.selectable_label(self.analysis_server, "Server / Slow log").clicked() { self.analysis_server = true; }
                if self.analysis_server { ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(!cfg!(target_arch = "wasm32") && self.connected && !self.busy && self.edit.is_none() && !self.value_dirty() && self.analysis_server, action_button("refresh-cw", "New report")).clicked() {
                        let request = if self.analysis_server { crate::redis_diagnostics::Request::Server }
                            else { crate::redis_diagnostics::Request::Keys(self.keys.iter().take(1000).cloned().collect()) };
                        self.diagnostics_request = Some(request.clone()); self.diagnostics_report = None;
                        self.diagnostics_open = false; self.send(Job::Diagnostics(request));
                    }
                }); }
            });
            ui.separator();
            #[cfg(not(target_arch = "wasm32"))]
            if !self.analysis_server { self.analysis.show(ui, self.database, self.connected); return; }
            #[cfg(target_arch = "wasm32")]
            {
                ui.horizontal(|ui| {
                    ui.heading(if self.analysis_server { "Server / Slow log" } else { "Database analysis" });
                    if !self.analysis_server {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            for (icon,label) in [("refresh-cw","New report"),("list-upload","Save report"),("folder","Open report")] {
                                ui.add_enabled(false,action_button(icon,label)).on_hover_text("Available in the desktop app with a Redis connection");
                            }
                        });
                    }
                });
                ui.add_space(16.0);
                ui.label("Connect to Redis in the desktop app to generate and save analysis reports.");
                ui.weak("This browser demo stores sample keys in memory; it has no Redis server memory, expiry scan or slow log to measure.");
                return;
            }
            #[cfg(not(target_arch = "wasm32"))]
            egui::ScrollArea::vertical().id_salt("redis-analysis-page").show(ui, |ui| {
                let report = self.diagnostics_report.as_ref().filter(|report| {
                    report.module_key.is_none() && !report.topology && report.vector_element.is_none() && report.vector_matches.is_none()
                        && report.analysis != self.analysis_server
                });
                if let Some(report) = report {
                    report.show(ui);
                } else {
                    ui.add_space(70.0);
                    ui.vertical_centered(|ui| {
                        ui.heading(if self.busy { "Generating report…" } else { "Understand your Redis data" });
                        ui.label("Generate a report to see memory estimates or inspect the server's slow log.");
                        ui.weak("Database analysis samples up to 1,000 loaded keys. Reports refresh only when requested.");
                    });
                }
            });
        });
    }
}

pub(crate) fn analysis_distribution(ui: &mut egui::Ui, rows: &[[String; 3]]) {
    // The report is bounded to 50 rows; these charts describe those rows only.
    let mut groups = std::collections::BTreeMap::<&str, (u64, u64)>::new();
    for row in rows.iter().take(50) {
        let item = groups.entry(&row[2]).or_default();
        item.0 = item.0.saturating_add(row[1].parse::<u64>().unwrap_or(0));
        item.1 += 1;
    }
    ui.add_space(12.0);
    ui.strong("Largest sampled keys · distribution by namespace");
    ui.weak("Charts cover the largest 50 keys in this report, not the entire database.");
    ui.columns(2, |columns| {
        for (index, title) in ["Memory", "Keys"].into_iter().enumerate() {
            let ui = &mut columns[index];
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 140.0), egui::Sense::hover());
            let center = rect.center();
            let radius = 50.0;
            let total: u64 = groups.values().map(|v| if index == 0 { v.0 } else { v.1 }).sum();
            ui.painter().circle_stroke(center, radius, egui::Stroke::new(12.0_f32, ui.visuals().faint_bg_color));
            let mut angle = -std::f32::consts::FRAC_PI_2;
            for (i, (_, counts)) in groups.iter().enumerate() {
                let value = if index == 0 { counts.0 } else { counts.1 };
                let span = value as f32 / total.max(1) as f32 * std::f32::consts::TAU;
                let steps = ((span * 24.0).ceil() as usize).max(2);
                let points = (0..=steps).map(|n| center + egui::Vec2::angled(angle + span * n as f32 / steps as f32) * radius).collect();
                let color = namespace_color(i, ui.visuals().dark_mode);
                ui.painter().add(egui::Shape::line(points, egui::Stroke::new(12.0_f32, color)));
                angle += span;
            }
            ui.painter().text(center - egui::vec2(0.0, 12.0), egui::Align2::CENTER_CENTER, title, egui::FontId::proportional(15.0), ui.visuals().weak_text_color());
            ui.painter().text(center + egui::vec2(0.0, 12.0), egui::Align2::CENTER_CENTER, if index == 0 { format!("{:.1} KiB", total as f64 / 1024.0) } else { total.to_string() }, egui::FontId::proportional(16.0), ui.visuals().text_color());
        }
    });
    ui.horizontal_wrapped(|ui| {
        for (i, (name, _)) in groups.iter().enumerate() {
            let color = namespace_color(i, ui.visuals().dark_mode);
            ui.colored_label(color, "●"); ui.label(*name);
        }
    });
    ui.add_space(16.0);
}


#[cfg(all(feature = "qa", not(target_arch = "wasm32")))]
impl Workspace {
    pub(crate) fn preview_analysis(&mut self, ctx: &egui::Context) -> bool {
        if !self.connected || self.busy { return false; }
        self.auto_scan = false; self.preview_queue.clear();
        self.section = Section::Analyze; self.analysis_server = false;
        self.commands_visible = false; self.diagnostics_open = false;
        self.analysis.preview(ctx, self.database)
    }
    pub(crate) fn preview_layout(&mut self, ctx: &egui::Context, step: usize) -> bool {
        if !self.connected || self.busy { return false; }
        if !self.scan_started || self.cursor != 0 { self.auto_scan = true; return false; }
        self.preview_queue.clear();
        self.auto_scan = false;
        self.commands_visible = false;
        self.inspector_visible = true;
        self.edit = None;
        self.read_only = true;
        self.diagnostics_open = false;
        match step {
            0 => {
                self.section = Section::Browse;
                self.tree_view = true;
                self.expanded.insert(b"leaderboard:".to_vec());
                self.expanded.insert(b"app:".to_vec());
                self.rebuild_tree();
                self.send(Job::Inspect(b"leaderboard:weekly".to_vec()));
            }
            1 => { self.section = Section::Browse; self.tree_view = false; self.inspector_visible = false; }
            2 => {
                self.section = Section::Browse; self.read_only = false;
                self.edit = Some(crate::redis_edit::Draft::new(crate::redis_mutation::Action::Create, vec![], "hash".into()));
            }
            3 | 4 => {
                self.section = Section::Search;
                let request = self.search.preview_request(step - 3);
                self.search.open = false; self.search.begin(); self.send(Job::Search(request));
            }
            5 => { return self.preview_commands(); }
            6 => {
                self.section = Section::Analyze; self.analysis_server = false;
                return self.analysis.preview(ctx, self.database);
            }
            8 => { self.section = Section::Browse; self.send(Job::Inspect(b"app:catalog:camera".to_vec())); self.string_mode = StringMode::Json; }
            9 => {
                self.section = Section::Browse; self.read_only = false;
                if let Some(snapshot) = &self.snapshot {
                    let mut draft = crate::redis_edit::Draft::new(crate::redis_mutation::Action::Update, vec![snapshot.key.clone()], snapshot.kind.clone());
                    if let redis::Value::BulkString(bytes) = &snapshot.value {
                        draft.expected = bytes.clone();
                        draft.value = self.value_page.as_ref().and_then(|p| p.display.strings.json_source.clone()).unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned());
                    }
                    self.edit = Some(draft);
                }
            }
            _ => { self.section = Section::PubSub; self.pubsub.open = false; }
        }
        true
    }
}


fn namespace_color(index: usize, dark: bool) -> egui::Color32 {
    egui::ecolor::Hsva::new((index as f32 * 0.618_034) % 1.0, 0.55, if dark { 0.85 } else { 0.65 }, 1.0).into()
}
