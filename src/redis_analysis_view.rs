//! On-demand analysis jobs and immutable, locally saved reports.
use crate::{
    redis_analysis::{self as analysis, Coverage, Progress, Report},
    redis_settings::Settings,
};
use eframe::egui;
use std::{
    io::{Read, Write},
    path::Path,
    sync::{Arc, atomic::Ordering, mpsc},
};
const MAX_FILE: usize = 1024 * 1024;
#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    version: u8,
    report: Report,
}
fn validate(report: &Report) -> Result<(), String> {
    if report.visited > analysis::MAX_KEYS
        || report.pages > analysis::MAX_PAGES
        || report.missing > report.visited
        || report.largest.len() > 15
        || report.namespaces.len() > 15
        || report.types.len() > 33
        || report.types.keys().any(|kind| kind.len() > 256)
        || report
            .largest
            .iter()
            .any(|key| key.name.len() > 4096 || key.kind.len() > 256)
        || report.namespaces.iter().any(|(name, _)| name.len() > 4096)
        || matches!(&report.coverage,Coverage::Failed(message) if message.len()>4096)
    {
        return Err("Report exceeds supported analysis bounds.".into());
    }
    Ok(())
}
fn read(path: &Path) -> Result<Report, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Choose a regular report file.".into());
    }
    let mut bytes = Vec::new();
    file.take((MAX_FILE + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FILE {
        return Err("Report exceeds 1 MiB.".into());
    }
    let saved: Saved = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if saved.version != 1 {
        return Err("Unsupported report version.".into());
    }
    validate(&saved.report)?;
    Ok(saved.report)
}
fn save(path: &Path, report: &Report) -> Result<(), String> {
    validate(report)?;
    let bytes = serde_json::to_vec(&Saved {
        version: 1,
        report: report.clone(),
    })
    .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FILE {
        return Err("Report exceeds 1 MiB.".into());
    }
    let resolved = crate::script_file::destination(path)?;
    let parent = resolved
        .parent()
        .ok_or("Report destination has no parent directory.")?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|()| file.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    file.persist(&resolved).map_err(|e| e.to_string())?;
    Ok(())
}
struct Prepared {
    report: Report,
    keys: Vec<[String; 3]>,
    namespaces: Vec<[String; 3]>,
    expiry: Vec<[String; 3]>,
    types: Vec<[String; 3]>,
}
impl Prepared {
    fn new(report: Report) -> Self {
        let summary =
            |name: String, t: &analysis::Totals| [name, t.keys.to_string(), format_bytes(t.bytes)];
        Self {
            keys: report
                .largest
                .iter()
                .map(|key| {
                    [
                        crate::redis_workspace::display_key(&key.name),
                        format_bytes(key.bytes),
                        key.kind.clone(),
                    ]
                })
                .collect(),
            namespaces: report
                .namespaces
                .iter()
                .map(|(name, t)| summary(crate::redis_workspace::display_key(name), t))
                .collect(),
            expiry: report
                .expiry
                .iter()
                .enumerate()
                .map(|(i, t)| summary(analysis::EXPIRY_LABELS[i].into(), t))
                .collect(),
            types: report
                .types
                .iter()
                .map(|(name, t)| summary(name.clone(), t))
                .collect(),
            report,
        }
    }
}
fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.2} MiB", bytes as f64 / 1048576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
enum Event {
    Report(Result<Report, String>),
    File(Result<Option<Report>, String>),
    Saved(Result<bool, String>),
}
pub(crate) struct View {
    settings: Settings,
    password: String,
    sentinel_password: String,
    progress: Option<Arc<Progress>>,
    receiver: Option<mpsc::Receiver<Event>>,
    data: Option<Prepared>,
    error: Option<String>,
    notice: Option<String>,
}
impl Drop for View {
    fn drop(&mut self) {
        if let Some(progress) = &self.progress {
            progress.cancel.store(true, Ordering::Release);
        }
    }
}
impl View {
    #[cfg(test)]
    pub(crate) fn prepare_benchmark(&mut self) {
        struct Fixture;
        impl analysis::Source for Fixture {
            fn page(&mut self,_:u64)->Result<(u64,Vec<Vec<u8>>),String>{Ok((0,(0..1000).map(|i|format!("namespace{}:key:{i:04}",i%16).into_bytes()).collect()))}
            fn samples(&mut self,keys:&[Vec<u8>])->Result<Vec<analysis::Sample>,String>{Ok(keys.iter().map(|_|analysis::Sample {bytes:Some(1024),ttl_ms:-1,kind:"string".into()}).collect())}
        }
        self.data=Some(Prepared::new(analysis::collect(&mut Fixture,0,&Progress::default())));
    }

    #[cfg(feature = "qa")]
    pub fn preview(&mut self, ctx: &egui::Context, database: u32) -> bool {
        self.poll(ctx);
        if self.data.is_some() {
            return true;
        }
        if self.receiver.is_none() && self.error.is_none() {
            self.start(ctx, database);
        }
        false
    }
    pub fn new(settings: Settings, password: String, sentinel_password: String) -> Self {
        Self {
            settings,
            password,
            sentinel_password,
            progress: None,
            receiver: None,
            data: None,
            error: None,
            notice: None,
        }
    }
    fn start(&mut self, ctx: &egui::Context, database: u32) {
        if self.receiver.is_some() {
            return;
        }
        let mut settings = self.settings.clone();
        settings.database = database;
        let password = self.password.clone();
        let sentinel_password = self.sentinel_password.clone();
        let progress = Arc::new(Progress::default());
        self.progress = Some(progress.clone());
        let (tx, rx) = mpsc::sync_channel(1);
        self.receiver = Some(rx);
        self.error = None;
        self.notice = None;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = crate::redis_session::Session::connect_with_sentinel(
                &settings,
                &password,
                &sentinel_password,
            )
            .map(|mut session| analysis::collect(&mut session, database, &progress));
            let _ = tx.send(Event::Report(result));
            ctx.request_repaint();
        });
    }
    fn file(&mut self, ctx: &egui::Context, saving: bool) {
        if self.receiver.is_some() {
            return;
        }
        let report = if saving {
            self.data.as_ref().map(|data| data.report.clone())
        } else {
            None
        };
        let (tx, rx) = mpsc::sync_channel(1);
        self.receiver = Some(rx);
        self.error = None;
        self.notice = None;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let dialog =
                rfd::FileDialog::new().add_filter("Rustable analysis report", &["rreport"]);
            let event = if let Some(report) = report {
                Event::Saved(
                    dialog
                        .set_file_name("redis-analysis.rreport")
                        .save_file()
                        .map_or(Ok(false), |path| save(&path, &report).map(|()| true)),
                )
            } else {
                Event::File(
                    dialog
                        .pick_file()
                        .map_or(Ok(None), |path| read(&path).map(Some)),
                )
            };
            let _ = tx.send(event);
            ctx.request_repaint();
        });
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        let event = self.receiver.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(event) => Some(event),
            Err(mpsc::TryRecvError::Disconnected) => Some(Event::Report(Err(
                "Analysis worker stopped unexpectedly.".into(),
            ))),
            Err(mpsc::TryRecvError::Empty) => None,
        });
        if let Some(event) = event {
            self.receiver = None;
            self.progress = None;
            match event {
                Event::Report(Ok(report)) | Event::File(Ok(Some(report))) => {
                    self.data = Some(Prepared::new(report));
                    self.notice = None;
                }
                Event::Report(Err(error)) | Event::File(Err(error)) | Event::Saved(Err(error)) => {
                    self.error = Some(error)
                }
                Event::Saved(Ok(true)) => self.notice = Some("Report saved.".into()),
                Event::Saved(Ok(false)) => {}
                Event::File(Ok(None)) => {}
            }
        }
        if self.progress.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, database: u32, connected: bool) {
        self.poll(ui.ctx());
        ui.horizontal(|ui| {
            ui.heading("Database analysis");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        self.receiver.is_none() && connected,
                        crate::redis_workspace::action_button("refresh-cw", "New report"),
                    )
                    .clicked()
                {
                    self.start(ui.ctx(), database);
                }
                if ui
                    .add_enabled(
                        self.receiver.is_none() && self.data.is_some(),
                        crate::redis_workspace::action_button("list-upload", "Save report"),
                    )
                    .clicked()
                {
                    self.file(ui.ctx(), true);
                }
                if ui
                    .add_enabled(
                        self.receiver.is_none(),
                        crate::redis_workspace::action_button("folder", "Open report"),
                    )
                    .clicked()
                {
                    self.file(ui.ctx(), false);
                }
            });
        });
        ui.weak("On demand · up to 100,000 keys / 60 seconds · memory estimates sample 5 elements per key");
        if let Some(progress) = &self.progress {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!(
                    "{} keys measured · {} scan pages",
                    progress.keys.load(Ordering::Acquire),
                    progress.pages.load(Ordering::Acquire)
                ));
                let stopping = progress.cancel.load(Ordering::Acquire);
                if ui
                    .add_enabled(
                        !stopping,
                        crate::redis_workspace::action_button(
                            "x",
                            if stopping { "Cancelling…" } else { "Cancel" },
                        ),
                    )
                    .clicked()
                {
                    progress.cancel.store(true, Ordering::Release);
                }
                if stopping {
                    ui.weak("Finishing the current network read");
                }
            });
        }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if let Some(notice) = &self.notice {
            ui.label(notice);
        }
        let Some(data) = &self.data else {
            ui.add_space(28.0);
            ui.label("Generate a report across the database, or open a saved report.");
            ui.weak("Reports contain key names and estimates, never key values or credentials.");
            return;
        };
        let report = &data.report;
        ui.separator();
        let coverage = match &report.coverage {
            Coverage::Complete => "Traversal complete".into(),
            Coverage::KeyLimit => "Partial · key limit".into(),
            Coverage::NameLimit => "Partial · key-name memory limit".into(),
            Coverage::PageLimit => "Partial · scan-page limit".into(),
            Coverage::TimeLimit => "Partial · time limit".into(),
            Coverage::Cancelled => "Partial · cancelled".into(),
            Coverage::Failed(error) => format!("Partial · {error}"),
        };
        ui.label(format!(
            "{coverage} · db{} · {} keys · {} · {:.1}s",
            report.database,
            report.total.keys,
            format_bytes(report.total.bytes),
            report.elapsed_ms as f64 / 1000.0
        ));
        let created =
            chrono::DateTime::from_timestamp_millis(report.created_ms.min(i64::MAX as u64) as i64)
                .map(|date| date.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "Unknown date".into());
        ui.weak(format!("Generated {created}"));
        ui.weak(format!("{} names disappeared during collection. SCAN is not a snapshot; keys and expiry may change during the report.",report.missing));
        egui::ScrollArea::vertical()
            .id_salt("database-analysis-report")
            .show(ui, |ui| {
                let table = |ui: &mut egui::Ui,
                             id: &str,
                             title: &str,
                             headers: [&str; 3],
                             rows: &[[String; 3]],
                             numeric: [bool; 3]| {
                    // A table below the fold must not collapse to the remaining
                    // viewport height: the outer report owns that scrolling.
                    let body_height = (rows.len() as f32 * 32.0).min(260.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), body_height + 88.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| crate::controls::report_table(ui, id, title, headers, rows, numeric, body_height),
                    );
                };
                let rankings = |left: &mut egui::Ui, right: &mut egui::Ui| {
                    table(
                        left,
                        "analysis-namespaces",
                        "Top namespaces",
                        ["Namespace", "Keys", "Memory"],
                        &data.namespaces,
                        [false, true, true],
                    );
                    table(
                        right,
                        "analysis-largest",
                        "Top 15 keys",
                        ["Key", "Memory", "Type"],
                        &data.keys,
                        [false, true, false],
                    );
                };
                if ui.available_width() >= 950.0 {
                    ui.columns(2, |cols| {
                        let (left, right) = cols.split_at_mut(1);
                        rankings(&mut left[0], &mut right[0]);
                    });
                } else {
                    table(
                        ui,
                        "analysis-largest",
                        "Top 15 keys",
                        ["Key", "Memory", "Type"],
                        &data.keys,
                        [false, true, false],
                    );
                    ui.add_space(12.0);
                    table(
                        ui,
                        "analysis-namespaces",
                        "Top namespaces",
                        ["Namespace", "Keys", "Memory"],
                        &data.namespaces,
                        [false, true, true],
                    );
                }
                ui.add_space(12.0);
                ui.columns(2, |cols| {
                    table(
                        &mut cols[0],
                        "analysis-types",
                        "Data types",
                        ["Type", "Keys", "Memory"],
                        &data.types,
                        [false, true, true],
                    );
                    table(
                        &mut cols[1],
                        "analysis-expiry",
                        "Expiry distribution",
                        ["Remaining TTL", "Keys", "Memory"],
                        &data.expiry,
                        [false, true, true],
                    );
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_scroll_reaches_expiry_rows_below_rankings() {
        for width in [800.0, 1200.0] {
            let ctx = egui::Context::default();
            let mut view = View::new(Settings::default(), String::new(), String::new());
            view.prepare_benchmark();
            let mut reached = false;
            for frame in 0..60 {
                let output = ctx.run(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.0))),
                    time: Some(frame as f64 / 60.0),
                    events: if frame > 2 { vec![
                        egui::Event::PointerMoved(egui::pos2(width - 12.0, 500.0)),
                        egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, -80.0), modifiers: egui::Modifiers::NONE },
                    ] } else { vec![] },
                    ..Default::default()
                }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| view.show(ui, 0, true)); });
                fn visible(shape: &egui::Shape, clip: egui::Rect) -> bool {
                    match shape {
                        egui::Shape::Text(text) => text.galley.text() == analysis::EXPIRY_LABELS[5]
                            && clip.contains(text.pos + text.galley.size() * 0.5),
                        egui::Shape::Vec(shapes) => shapes.iter().any(|s| visible(s, clip)),
                        _ => false,
                    }
                }
                reached |= output.shapes.iter().any(|s| visible(&s.shape, s.clip_rect));
            }
            assert!(reached, "expiry rows must remain reachable at width {width}");
        }
    }
    fn fixture() -> Report {
        struct Sample;
        impl analysis::Source for Sample {
            fn page(&mut self, _: u64) -> Result<(u64, Vec<Vec<u8>>), String> {
                Ok((
                    0,
                    (0..400)
                        .map(|i| format!("namespace{}:key{i}", i % 20).into_bytes())
                        .collect(),
                ))
            }
            fn samples(&mut self, keys: &[Vec<u8>]) -> Result<Vec<analysis::Sample>, String> {
                Ok(keys
                    .iter()
                    .map(|_| analysis::Sample {
                        bytes: Some(512),
                        ttl_ms: 500,
                        kind: "string".into(),
                    })
                    .collect())
            }
        }
        analysis::collect(&mut Sample, 0, &Progress::default())
    }
    #[test]
    fn reports_roundtrip_and_invalid_files_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.rreport");
        let report = fixture();
        save(&path, &report).unwrap();
        let loaded = read(&path).unwrap();
        assert_eq!(loaded.largest, report.largest);
        assert_eq!(loaded.total, report.total);
        let mut bad = report.clone();
        bad.largest.push(bad.largest[0].clone());
        assert!(save(&path, &bad).is_err());
        assert_eq!(read(&path).unwrap().largest, report.largest); // Failed save preserves the old file.
        std::fs::write(&path, vec![b' '; MAX_FILE + 1]).unwrap();
        assert!(read(&path).unwrap_err().contains("1 MiB"));
        std::fs::write(&path, b"{\"version\":2}").unwrap();
        assert!(read(&path).is_err());
    }
    #[test]
    fn report_page_settles_and_failure_retains_previous_report() {
        let ctx = egui::Context::default();
        let mut view = View::new(Settings::default(), String::new(), String::new());
        view.data = Some(Prepared::new(fixture()));
        let mut delay = std::time::Duration::ZERO;
        for frame in 0..40 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 760.0),
                    )),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| view.show(ui, 0, true));
                },
            );
            delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        }
        assert!(delay > std::time::Duration::from_secs(1));
        assert!(view.receiver.is_none());
        let (tx, rx) = mpsc::sync_channel(1);
        view.receiver = Some(rx);
        tx.send(Event::Report(Err("Lost connection".into())))
            .unwrap();
        view.poll(&ctx);
        assert_eq!(view.data.as_ref().unwrap().report.total.keys, 400);
        assert!(view.error.is_some());
        let progress = Arc::new(Progress::default());
        view.progress = Some(progress.clone());
        drop(view);
        assert!(progress.cancel.load(Ordering::Acquire));
    }
}
