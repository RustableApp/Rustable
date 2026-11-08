//! Native Pub/Sub UI. Retention and worker draining are independently bounded.
use crate::{
    redis_pubsub::{Event, Message, Request, Worker},
    redis_settings::Settings,
};
use eframe::egui;
use std::collections::VecDeque;
use std::time::Duration;
pub(crate) struct Publish {
    pub channel: Vec<u8>,
    pub payload: Vec<u8>,
}
struct Row {
    id: u64,
    received_at: String,
    channel: String,
    summary: String,
    message: Message,
}
pub(crate) struct View {
    pub open: bool,
    settings: Settings,
    password: zeroize::Zeroizing<String>,
    sentinel_password: zeroize::Zeroizing<String>,
    subscription: Option<Worker>,
    lookup: Option<Worker>,
    target: String,
    pattern: bool,
    channel_filter: String,
    channels: Vec<Vec<u8>>,
    channel_labels: Vec<String>,
    count: Option<u64>,
    count_channel: String,
    messages: VecDeque<Row>,
    received: u64,
    follow: bool,
    selected: Option<u64>,
    detail: Option<crate::redis_value::StringViews>,
    mode: crate::redis_value::StringMode,
    publish_channel: String,
    payload: String,
    hex: bool,
    pub publishing: bool,
    publish_status: String,
    error: String,
    reveal_channels: Option<bool>,
}
impl View {
    pub fn new(settings: Settings, password: String, sentinel_password: String) -> Self {
        Self {
            open: false,
            settings,
            password: zeroize::Zeroizing::new(password),
            sentinel_password: zeroize::Zeroizing::new(sentinel_password),
            subscription: None,
            lookup: None,
            target: "events:*".into(),
            pattern: true,
            channel_filter: "*".into(),
            channels: vec![],
            channel_labels: vec![],
            count: None,
            count_channel: String::new(),
            messages: VecDeque::new(),
            received: 0,
            follow: true,
            selected: None,
            detail: None,
            mode: Default::default(),
            publish_channel: "events:demo".into(),
            payload: String::new(),
            hex: false,
            publishing: false,
            publish_status: String::new(),
            error: String::new(),
            reveal_channels: None,
        }
    }
    fn worker(&self, request: Request, ctx: &egui::Context) -> Worker {
        Worker::start_with_sentinel(
            self.settings.clone(),
            self.password.to_string(),
            self.sentinel_password.to_string(),
            request,
            ctx.clone(),
        )
    }
    fn lookup(&mut self, request: Request, ctx: &egui::Context) {
        self.error.clear();
        if let Some(worker) = &self.lookup
            && !worker.done()
        {
            if let Err(error) = worker.request(request) {
                self.error = error;
            }
        } else {
            self.lookup = Some(self.worker(request, ctx));
        }
    }
    pub fn published(&mut self, result: Result<u64, String>) {
        self.publishing = false;
        self.publish_status = match result {
            Ok(count) => {
                format!("Redis reported {count} receivers. This is not a delivery acknowledgement.")
            }
            Err(error) => error,
        };
    }
    fn drain(&mut self, ctx: &egui::Context) {
        if let Some(worker) = &self.subscription {
            let mut n = 0;
            while n < 16 {
                let Ok(Event::Message(message)) = worker.receiver.try_recv() else {
                    break;
                };
                self.received += 1;
                let summary = crate::redis_commands::quote(
                    &message.preview[..message.preview.len().min(120)],
                );
                let channel = crate::redis_commands::quote(&message.channel);
                if self.messages.len() == 1000 {
                    self.messages.pop_front();
                }
                self.messages.push_back(Row {
                    id: self.received,
                    received_at: chrono::Local::now().format("%H:%M:%S").to_string(),
                    channel,
                    summary,
                    message,
                });
                n += 1;
            }
            if n == 16 {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        if let Some(worker) = &self.lookup {
            while let Ok(event) = worker.receiver.try_recv() {
                match event {
                    Event::Channels { channels, labels } => {
                        self.channels = channels;
                        self.channel_labels = labels;
                    }
                    Event::Count(count) => self.count = Some(count),
                    Event::Message(_) => {}
                }
            }
        }
    }
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        connected: bool,
        ready: bool,
        read_only: bool,
    ) -> Option<Publish> {
        self.drain(ctx);
        if !self.open {
            return None;
        }
        let mut open = true;
        let mut publish = None;
        egui::Window::new("Redis Pub/Sub").open(&mut open).anchor(egui::Align2::CENTER_CENTER,egui::Vec2::ZERO).default_width(820.0).default_height(540.0).max_height(650.0).vscroll(true).show(ctx,|ui|{
            publish = self.contents(ui, connected, ready, read_only, false);
        });
        if !open {
            if let Some(worker) = &self.subscription {
                worker.stop();
            }
            if let Some(worker) = &self.lookup {
                worker.stop();
            }
        }
        self.open = open;
        publish
    }
    pub fn pane(&mut self, ui: &mut egui::Ui, connected: bool, ready: bool, read_only: bool) -> Option<Publish> {
        self.drain(ui.ctx());
        let mut publish = None;
        egui::TopBottomPanel::bottom("redis-publish-composer").frame(egui::Frame::new().inner_margin(12)).show_inside(ui, |ui| {
            publish = self.publish_composer(ui, ready, read_only);
        });
        egui::ScrollArea::vertical().id_salt("pubsub-main").show(ui, |ui| {
            self.contents(ui, connected, ready, read_only, true);
        });
        publish
    }
    fn publish_composer(&mut self, ui: &mut egui::Ui, ready: bool, read_only: bool) -> Option<Publish> {
        let mut publish = None;
        ui.separator();
        let message_width = (ui.available_width() - 220.0 - 100.0 - 24.0).max(100.0);
        egui::Grid::new("publish-composer-fields").num_columns(3).spacing(egui::vec2(12.0, 6.0)).show(ui, |ui| {
            ui.weak("Channel name");
            ui.horizontal(|ui| { ui.weak("Message"); ui.checkbox(&mut self.hex, "Hex"); });
            ui.label("");
            ui.end_row();
            ui.vertical(|ui| {
                ui.set_width(220.0);
                ui.add(egui::TextEdit::singleline(&mut self.publish_channel).desired_width(f32::INFINITY).char_limit(8192));
            });
            ui.vertical(|ui| {
                ui.set_width(message_width);
                ui.add(egui::TextEdit::multiline(&mut self.payload).desired_rows(2).desired_width(f32::INFINITY).char_limit(131072).hint_text("Enter message"));
            });
            if ui.add_enabled(ready && !read_only && !self.publishing, crate::redis_workspace::action_button("arrow-right", "Publish")).on_hover_text("Publish to subscribers immediately").clicked() {
                match draft(&self.publish_channel, &self.payload, self.hex) {
                    Ok(request) => { self.publishing = true; self.publish_status = "Publishing…".into(); publish = Some(request); }
                    Err(error) => self.publish_status = error,
                }
            }
            ui.end_row();
        });
        if !self.publish_status.is_empty() { ui.label(&self.publish_status); }
        publish
    }
    fn subscription_controls(&mut self, ui: &mut egui::Ui, connected: bool, active: bool) {
        let context = ui.ctx().clone();
        let ctx = &context;
            ui.horizontal(|ui|{
                ui.add_enabled_ui(!active, |ui| { crate::controls::segments(ui,"subscription-mode",&mut self.pattern,&[(false,"Channel",76.0),(true,"Pattern",76.0)]); });
                ui.add_enabled(!active,egui::TextEdit::singleline(&mut self.target).desired_width(280.0).char_limit(8192));
                if !active && ui.add_enabled(connected,egui::Button::new("Subscribe")).clicked(){
                    match name(&self.target) {
                        Ok(target)=>{self.error.clear();self.messages.clear();self.received=0;self.selected=None;self.detail=None;self.subscription=Some(self.worker(Request::Subscribe{target,pattern:self.pattern},ctx));},
                        Err(error)=>self.error=error,
                    }
                }
                if active && ui.button("Unsubscribe").clicked(){self.subscription.as_ref().unwrap().stop();}
            });
    }
    fn contents(&mut self, ui: &mut egui::Ui, connected: bool, ready: bool, read_only: bool, embedded: bool) -> Option<Publish> {
        let context = ui.ctx().clone();
        let ctx = &context;
        let mut publish = None;
            crate::controls::base_spacing(ui.spacing_mut());
            crate::controls::rounded_action_buttons(ui);
            ui.weak("Live messages").on_hover_text("Pub/Sub spans databases and has no replay. Messages sent while disconnected are lost.");
            ui.add_space(6.0);
            let active=self.subscription.as_ref().is_some_and(|w|!w.done());
            if !self.messages.is_empty() { self.subscription_controls(ui, connected, active); }
            if let Some(worker)=&self.subscription {ui.add(egui::Label::new(worker.status()).wrap());ui.weak(format!("{} received · {} retained · {} dropped while busy",self.received,self.messages.len(),worker.dropped()));}
            ui.horizontal(|ui|{
                ui.checkbox(&mut self.follow,"Follow latest");
                if ui.add_enabled(!self.messages.is_empty(),egui::Button::new("Clear messages")).clicked(){self.messages.clear();self.detail=None;self.selected=None;self.received=0;}
                ui.weak("1,000 retained · 4 KiB previews");
            });
            ui.separator();
            if self.messages.is_empty(){
                let height = (ui.available_height() - 220.0).max(280.0);
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), height), egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.add_space((height * 0.3).max(12.0));
                    ui.heading(if active { "Listening for messages" } else { "No active subscription" });
                    ui.weak("Subscribe to a channel or pattern to see new messages.");
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        ui.add_space(((ui.available_width() - 660.0) * 0.5).max(0.0));
                        self.subscription_controls(ui, connected, active);
                    });
                });
            }else{
                use egui_extras::{Column, TableBuilder};
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    TableBuilder::new(ui).cell_layout(egui::Layout::left_to_right(egui::Align::Center)).id_salt("pubsub-messages").striped(true).max_scroll_height(320.0).stick_to_bottom(self.follow)
                        .column(Column::exact(110.0)).column(Column::initial(220.0).clip(true)).column(Column::remainder().clip(true))
                        .header(32.0, |mut row| { for name in ["Received (local)", "Channel", "Message"] { row.col(|ui| { crate::controls::table_header(ui, name); }); } })
                        .body(|body| body.rows(28.0, self.messages.len(), |mut table_row| {
                            let row = &self.messages[table_row.index()];
                            table_row.col(|ui| { ui.label(&row.received_at); });
                            table_row.col(|ui| {
                                if ui.selectable_label(self.selected == Some(row.id), &row.channel).clicked() {
                                    self.selected = Some(row.id);
                                    self.detail = Some(crate::redis_value::StringViews::new(&row.message.preview, 0, row.message.bytes == row.message.preview.len()));
                                }
                            });
                            table_row.col(|ui| { ui.add(egui::Label::new(egui::RichText::new(&row.summary).monospace()).truncate()); });
                        }));
                    ui.separator(); ui.weak(format!("{} messages retained · {} received", self.messages.len(), self.received));
                });
            }
            if let Some(detail)=&self.detail {
                ui.horizontal(|ui|{crate::controls::segments(ui,"pubsub-format",&mut self.mode,&[(crate::redis_value::StringMode::Text,"Text",50.0),(crate::redis_value::StringMode::Json,"JSON",52.0),(crate::redis_value::StringMode::Hex,"Hex",48.0)]);ui.weak("Selected message preview");});
                let lines=match self.mode {crate::redis_value::StringMode::Json=>&detail.json,crate::redis_value::StringMode::Hex=>&detail.hex,_=>&detail.text};
                if self.mode==crate::redis_value::StringMode::Json && let Some(error)=&detail.json_error {ui.add(egui::Label::new(error).wrap());}
                egui::ScrollArea::both().id_salt("pubsub-detail").max_height(140.0).show_rows(ui,18.0,lines.len(),|ui,range|{
                    for i in range {
                        if self.mode==crate::redis_value::StringMode::Json {
                            let mut job=egui::text::LayoutJob::default();
                            for token in &detail.json_tokens[i] {job.append(&lines[i][token.range.clone()],0.0,egui::TextFormat{font_id:egui::FontId::monospace(12.0),color:token.kind.color(ui.visuals().dark_mode),..Default::default()});}
                            ui.label(job);
                        }else{ui.monospace(&lines[i]);}
                    }
                });
            }
            ui.separator();
            egui::CollapsingHeader::new("Active channels").open(self.reveal_channels.take()).show(ui,|ui|{
                let lookup_active=self.lookup.as_ref().is_some_and(|w|w.busy());
                ui.horizontal(|ui|{
                    ui.label("Pattern");ui.add(egui::TextEdit::singleline(&mut self.channel_filter).desired_width(280.0).char_limit(8192));
                    if ui.add_enabled(connected&&!lookup_active,egui::Button::new("Refresh channels")).clicked(){match name(&self.channel_filter){Ok(pattern)=>{self.lookup(Request::Channels(pattern),ctx);self.count=None;self.error.clear();},Err(error)=>self.error=error}}
                });
                if let Some(worker)=&self.lookup {ui.add(egui::Label::new(worker.status()).wrap());}
                if let Some(count)=self.count{ui.add(egui::Label::new(format!("{}: {count} direct subscribers on this node; excludes patterns.", self.count_channel)).wrap());}
                let mut selected=None;
                egui::ScrollArea::vertical().id_salt("pubsub-channels").max_height(100.0).show_rows(ui,22.0,self.channels.len(),|ui,range|{for i in range{if ui.add_enabled(!lookup_active, egui::Button::selectable(false,&self.channel_labels[i]).truncate()).clicked(){selected=Some(i);}}});
                if let Some(i)=selected {
                    if !active {self.target=self.channel_labels[i].clone();self.pattern=false;}self.publish_channel=self.channel_labels[i].clone();self.count=None;
                    self.count_channel=crate::redis_commands::quote(&self.channels[i][..self.channels[i].len().min(120)]);self.lookup(Request::Count(self.channels[i].clone()),ctx);
                }
                ui.add(egui::Label::new("Up to 1,000 channels / 1 MiB per response. Narrow the pattern if the limit is exceeded. Channel names accept quotes and \\xHH escapes.").wrap());
            });
            if !embedded { publish = self.publish_composer(ui, ready, read_only); }
            if !self.error.is_empty(){ui.colored_label(ui.visuals().error_fg_color,&self.error);}

        publish
    }

}
fn name(text: &str) -> Result<Vec<u8>, String> {
    let mut commands = crate::redis_commands::parse(&format!("X {text}"))?;
    if commands.len() != 1 || commands[0].len() != 2 {
        return Err("Enter one channel/pattern; quote names containing spaces.".into());
    }
    let bytes = commands.pop().unwrap().pop().unwrap();
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err("Channel/pattern must contain 1–4,096 bytes.".into());
    }
    Ok(bytes)
}
fn draft(channel: &str, payload: &str, hex: bool) -> Result<Publish, String> {
    let channel = name(channel)?;
    let payload = if hex {
        let compact: Vec<_> = payload
            .bytes()
            .filter(|b| !b.is_ascii_whitespace())
            .collect();
        if !compact.len().is_multiple_of(2) {
            return Err("Hex payload needs pairs of digits.".into());
        }
        let mut bytes = Vec::with_capacity(compact.len() / 2);
        for pair in compact.as_chunks::<2>().0 {
            let digit = |b: u8| (b as char).to_digit(16).map(|n| n as u8);
            bytes.push(
                digit(pair[0]).ok_or("Invalid hexadecimal digit")? * 16
                    + digit(pair[1]).ok_or("Invalid hexadecimal digit")?,
            );
        }
        bytes
    } else {
        payload.as_bytes().to_vec()
    };
    if payload.len() > 65536 {
        return Err("Publish payload exceeds 64 KiB.".into());
    }
    Ok(Publish { channel, payload })
}

#[cfg(feature = "qa")]
impl View {
    pub fn preview(&mut self, ctx: &egui::Context, step: usize) -> bool {
        self.open = true;
        match step {
            0 => true,
            1 => {
                if self.subscription.is_none() {
                    self.pattern = false;
                    self.target = "events:demo".into();
                    self.subscription = Some(self.worker(
                        Request::Subscribe {
                            target: b"events:demo".to_vec(),
                            pattern: false,
                        },
                        ctx,
                    ));
                }
                self.subscription.as_ref().unwrap().status() == "Subscribed"
            }
            2 => {
                let Some(row) = self.messages.front() else {
                    return false;
                };
                self.selected = Some(row.id);
                self.mode = crate::redis_value::StringMode::Json;
                self.detail = Some(crate::redis_value::StringViews::new(
                    &row.message.preview,
                    0,
                    true,
                ));
                true
            }
            3 => {
                self.detail = None;
                self.reveal_channels = Some(true);
                if self.lookup.is_none() {
                    self.channel_filter = "events:*".into();
                    self.lookup = Some(self.worker(Request::Channels(b"events:*".to_vec()), ctx));
                }
                !self.channels.is_empty()
            }
            4 => {
                self.reveal_channels = Some(false);

                self.payload = "{\n  \"event\": \"order.updated\",\n  \"id\": 42\n}".into();
                true
            }
            _ => {

                if let Some(worker) = &self.subscription {
                    worker.stop();
                    worker.done()
                } else {
                    true
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_publish_payloads_are_binary_safe_and_bounded() {
        assert_eq!(
            name("\"hello world\\x00\\xff\"").unwrap(),
            b"hello world\0\xff"
        );
        assert!(name("x\nPUBLISH x y").is_err());
        assert!(name("\"\"").is_err());
        assert_eq!(
            draft("events:x", "00 ff 0d 0a", true).unwrap().payload,
            b"\0\xff\r\n"
        );
        assert!(draft("events:x", "0ff", true).is_err());
        assert!(draft("events:x", "zz", true).is_err());
        assert!(draft("events:x", &"x".repeat(65537), false).is_err());
    }
    #[test]
    fn idle_pubsub_window_settles_within_viewport() {
        let ctx = egui::Context::default();
        let mut view = View::new(Settings::default(), String::new(), String::new());
        view.open = true;
        view.channels = vec![vec![b'x'; 4096]; 1000];
        view.channel_labels = view
            .channels
            .iter()
            .map(|c| crate::redis_commands::quote(c))
            .collect();
        view.reveal_channels = Some(true);
        let mut last = None;
        for i in 0..60 {
            last = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 760.0),
                    )),
                    time: Some(i as f64 / 120.0),
                    ..Default::default()
                },
                |ctx| {
                    assert!(view.show(ctx, true, true, true).is_none());
                },
            ));
        }
        let rect = ctx
            .memory(|m| m.area_rect(egui::Id::new("Redis Pub/Sub")))
            .unwrap();
        assert!(rect.width() <= 1200.0 && rect.height() <= 760.0, "{rect:?}");
        assert!(
            last.unwrap().viewport_output[&egui::ViewportId::ROOT].repaint_delay
                > Duration::from_millis(500)
        );
    }
}

#[cfg(test)]
mod benchmarks {
    use super::*;
    fn message(index: usize, large: bool) -> Message {
        let preview = if large {
            vec![b'x'; 4096]
        } else {
            format!("{{\"id\":{index},\"paid\":true,\"total\":129.95}}").into_bytes()
        };
        Message {
            channel: format!("events:orders:{index:05}").into_bytes(),
            bytes: preview.len(),
            preview,
        }
    }
    #[test]
    #[ignore = "manual Pub/Sub CPU workloads"]
    fn redis_pubsub_frame_benchmark() {
        for scenario in 0..4 {
            let ctx = egui::Context::default();
            let mut view = View::new(Settings::default(), String::new(), String::new());
            view.open = true;
            let (worker, sender) = crate::redis_pubsub::fixture_worker();
            view.subscription = Some(worker);
            for i in 0..1000 {
                sender.try_send(Event::Message(message(i, false))).ok();
                view.drain(&ctx);
            }
            if scenario == 2 {

                view.payload = "x".repeat(65535);
            }
            let (lookup, lookup_sender) = crate::redis_pubsub::fixture_worker();
            if scenario == 3 {
                view.lookup = Some(lookup);
                view.reveal_channels = Some(true);
            }
            let mut samples = Vec::new();
            for i in 0..180 {
                if scenario == 3 && i % 10 == 0 {
                    let channels: Vec<Vec<u8>> = (0..1000)
                        .map(|n| format!("events:{i}:{n:04}:{}", "x".repeat(900)).into_bytes())
                        .collect();
                    let labels = channels
                        .iter()
                        .map(|c| crate::redis_commands::quote(c))
                        .collect();
                    lookup_sender
                        .try_send(Event::Channels { channels, labels })
                        .unwrap();
                }

                if scenario == 1 {
                    for j in 0..16 {
                        sender
                            .try_send(Event::Message(message(i * 16 + j, true)))
                            .ok();
                    }
                }
                let started = std::time::Instant::now();
                if scenario == 0 && i % 20 == 0 {
                    let row = &view.messages[i % view.messages.len()];
                    view.selected = Some(row.id);
                    view.detail = Some(crate::redis_value::StringViews::new(
                        &row.message.preview,
                        0,
                        true,
                    ));
                    view.mode = if i % 40 == 0 {
                        crate::redis_value::StringMode::Json
                    } else {
                        crate::redis_value::StringMode::Hex
                    };
                }
                if scenario == 2 {
                    view.payload.pop();
                    view.payload.push(if i % 2 == 0 { 'a' } else { 'b' });
                }
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1200.0, 760.0),
                        )),
                        time: Some(i as f64 / 120.0),
                        ..Default::default()
                    },
                    |ctx| {
                        view.show(ctx, true, true, false);
                    },
                );
                let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
                if i >= 20 {
                    samples.push(started.elapsed().as_secs_f64() * 1000.0);
                }
            }
            samples.sort_by(f64::total_cmp);
            crate::ui_benchmark::report(
                [
                    "redis/pubsub-messages",
                    "redis/pubsub-incoming",
                    "redis/pubsub-publish-64k",
                    "redis/pubsub-channel-refresh",
                ][scenario],
                &samples,
            );
        }
    }
}
