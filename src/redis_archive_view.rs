//! Reviewed import/export; immutable payloads and file I/O stay off the UI path.
use crate::redis_archive::{self as archive, Archive, Collision, Expiry, Review, State};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
pub(crate) struct Prepared {
    pub archive: Arc<Archive>,
    rows: Vec<[String; 4]>,
    skipped: Vec<(String, String)>,
}
impl Prepared {
    fn new(data: Archive, skipped: Vec<(Vec<u8>, String)>) -> Self {
        let rows = data
            .entries
            .iter()
            .map(|e| {
                [
                    crate::redis_commands::quote(&e.key),
                    e.kind.clone(),
                    format!("{} B", e.dump.len()),
                    archive::expiry_label(e.expires_at_ms),
                ]
            })
            .collect();
        Self {
            archive: Arc::new(data),
            rows,
            skipped: skipped
                .into_iter()
                .map(|(key, why)| (crate::redis_commands::quote(&key), why))
                .collect(),
        }
    }
}
pub(crate) enum Action {
    Export {
        keys: Vec<Vec<u8>>,
        database: u32,
        cancel: Arc<AtomicBool>,
    },
    Review {
        data: Arc<Prepared>,
        database: u32,
    },
    Import {
        data: Arc<Prepared>,
        database: u32,
        collision: Collision,
        expiry: Expiry,
        read_only: bool,
        cancel: Arc<AtomicBool>,
    },
}
pub(crate) enum Reply {
    Prepared(Arc<Prepared>),
    Reviewed(Review),
    Imported(Report),
}
pub(crate) struct Report {
    rows: Vec<[String; 3]>,
    counts: [usize; 6],
    stopped: Option<String>,
}
pub(crate) fn execute(
    action: Action,
    session: &mut crate::redis_session::Session,
    current_database: u32,
) -> Result<Reply, String> {
    let expected = match &action {
        Action::Export { database, .. }
        | Action::Review { database, .. }
        | Action::Import { database, .. } => *database,
    };
    archive::require_database(expected, current_database)?;
    match action {
        Action::Export {
            keys,
            database,
            cancel,
        } => {
            let report = session.export_archive(&keys, database, &cancel)?;
            Ok(Reply::Prepared(Arc::new(Prepared::new(
                report.archive,
                report.skipped,
            ))))
        }
        Action::Review { data, database } => session
            .review_archive(&data.archive, database)
            .map(Reply::Reviewed),
        Action::Import {
            data,
            collision,
            expiry,
            read_only,
            cancel,
            ..
        } => {
            let report =
                session.import_archive(&data.archive, collision, expiry, read_only, &cancel)?;
            let mut counts = [0; 6];
            let rows = report
                .outcomes
                .into_iter()
                .map(|r| {
                    let (index, label) = match r.state {
                        State::Restored => (0, "Restored"),
                        State::Exists => (1, "Existing · skipped"),
                        State::Expired => (2, "Expired · skipped"),
                        State::Error => (3, "Error"),
                        State::Unknown => (4, "Unconfirmed"),
                        State::NotAttempted => (5, "Not attempted"),
                    };
                    counts[index] += 1;
                    [
                        crate::redis_commands::quote(&r.key),
                        label.into(),
                        r.message,
                    ]
                })
                .collect();
            Ok(Reply::Imported(Report {
                rows,
                counts,
                stopped: report.stopped,
            }))
        }
    }
}
enum FileEvent {
    Loaded(Result<Arc<Prepared>, String>),
    Saved(Result<PathBuf, String>),
    Cancelled,
}
pub(crate) struct View {
    pub open: bool,
    pub busy: bool,
    file_pending: bool,
    importing: bool,
    data: Option<Arc<Prepared>>,
    review: Option<Review>,
    report: Option<Report>,
    collision: Collision,
    expiry: Expiry,
    confirm_replace: bool,
    cancel: Arc<AtomicBool>,
    files_tx: mpsc::SyncSender<FileEvent>,
    files_rx: mpsc::Receiver<FileEvent>,
    status: String,
    error: String,
}
impl Default for View {
    fn default() -> Self {
        let (files_tx, files_rx) = mpsc::sync_channel(2);
        Self {
            open: false,
            busy: false,
            file_pending: false,
            importing: false,
            data: None,
            review: None,
            report: None,
            collision: Default::default(),
            expiry: Default::default(),
            confirm_replace: false,
            cancel: Arc::new(AtomicBool::new(false)),
            files_tx,
            files_rx,
            status: String::new(),
            error: String::new(),
        }
    }
}
impl Drop for View {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl View {
    pub fn can_start(&self) -> bool {
        !self.busy && !self.file_pending
    }
    fn reset(&mut self) {
        self.data = None;
        self.review = None;
        self.report = None;
        self.error.clear();
        self.status.clear();
        self.confirm_replace = false;
        self.collision = Collision::Skip;
        self.expiry = Expiry::Preserve;
    }
    fn start(&mut self) {
        self.busy = true;
        self.error.clear();
        self.cancel = Arc::new(AtomicBool::new(false));
    }
    pub fn export(&mut self, keys: Vec<Vec<u8>>, database: u32) -> Action {
        self.reset();
        self.importing = false;
        self.open = true;
        self.start();
        self.status = "Capturing selected keys…".into();
        Action::Export {
            keys,
            database,
            cancel: self.cancel.clone(),
        }
    }
    pub fn load(&mut self, ctx: &egui::Context) {
        if self.file_pending || self.busy {
            return;
        }
        self.error.clear();
        self.open = true;
        self.file_pending = true;
        self.status = "Choose an archive…".into();
        let tx = self.files_tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let event = if let Some(path) = rfd::FileDialog::new()
                .add_filter("Rustable Redis archive", &["rredis"])
                .pick_file()
            {
                FileEvent::Loaded(Archive::read(&path).map(|a| Arc::new(Prepared::new(a, vec![]))))
            } else {
                FileEvent::Cancelled
            };
            let _ = tx.send(event);
            ctx.request_repaint();
        });
    }
    fn save(&mut self, ctx: &egui::Context) {
        let Some(data) = self.data.clone() else {
            return;
        };
        self.file_pending = true;
        self.error.clear();
        self.status = "Choose where to save the reviewed archive…".into();
        let tx = self.files_tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let event = if let Some(path) = rfd::FileDialog::new()
                .add_filter("Rustable Redis archive", &["rredis"])
                .set_file_name("redis-keys.rredis")
                .save_file()
            {
                // The native save dialog obtains overwrite confirmation. The
                // immutable reviewed bytes, not a re-read source, are saved.
                FileEvent::Saved(data.archive.save(&path, true).map(|()| path))
            } else {
                FileEvent::Cancelled
            };
            let _ = tx.send(event);
            ctx.request_repaint();
        });
    }
    pub fn failed(&mut self, error: String) {
        self.busy = false;
        self.error = error;
        self.status.clear();
    }
    pub fn complete(&mut self, reply: Reply) {
        self.busy = false;
        match reply {
            Reply::Prepared(data) => {
                self.status = format!(
                    "Captured {} keys; {} skipped. Review before saving.",
                    data.rows.len(),
                    data.skipped.len()
                );
                self.data = Some(data);
            }
            Reply::Reviewed(review) => {
                self.status = format!("Target checked: {} existing keys.", review.existing);
                self.review = Some(review);
            }
            Reply::Imported(report) => {
                self.status =
                    "Import finished. Inspect the per-key results before retrying any entry."
                        .into();
                self.report = Some(report);
                self.review = None;
            }
        }
    }
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        name: &str,
        database: u32,
        ready: bool,
        read_only: bool,
    ) -> Option<Action> {
        while let Ok(event) = self.files_rx.try_recv() {
            self.file_pending = false;
            match event {
                FileEvent::Loaded(Ok(data)) => {
                    self.reset();
                    self.importing = true;
                    self.data = Some(data);
                    self.review = None;
                    self.status =
                        "Archive loaded. Review the target database before importing.".into();
                }
                FileEvent::Loaded(Err(error)) => {
                    self.error = error;
                    self.status =
                        "Could not load the new archive; previous review retained.".into();
                }
                FileEvent::Saved(Err(error)) => self.error = error,
                FileEvent::Saved(Ok(path)) => self.status = format!("Saved {}", path.display()),
                FileEvent::Cancelled => self.status = "File selection cancelled.".into(),
            }
        }
        if !self.open {
            return None;
        }
        if self.busy && self.importing && read_only {
            self.cancel.store(true, Ordering::Release);
        }
        let mut action = None;
        let mut open = true;
        egui::Window::new("Redis import / export").open(&mut open).anchor(egui::Align2::CENTER_CENTER,egui::Vec2::ZERO).default_width(900.0).default_height(520.0).max_height(650.0).vscroll(true).show(ctx,|ui|{
            crate::controls::base_spacing(ui.spacing_mut());crate::controls::rounded_action_buttons(ui);
            ui.strong(if self.data.is_none() && !self.busy {"Import / export"}else if self.importing{"Review import"}else{"Review export"});
            ui.add(egui::Label::new(format!("{} · database {}",name,database)).wrap());
            if !self.status.is_empty(){ui.add(egui::Label::new(&self.status).wrap());}
            if !self.error.is_empty(){ui.add(egui::Label::new(egui::RichText::new(&self.error).color(ui.visuals().error_fg_color)).wrap());}
            let idle=ready&&!self.busy&&!self.file_pending;
            ui.horizontal(|ui|{
                if ui.add_enabled(self.can_start(),egui::Button::new("Open archive…")).clicked(){self.load(ctx);}
                if ui.add_enabled(self.busy,egui::Button::new("Stop after batch")).clicked(){self.cancel.store(true,Ordering::Release);self.status="Stopping after the submitted batch finishes…".into();}
                if self.file_pending{ui.weak("File dialog / disk operation in progress…");}
            });
            if let Some(data)=self.data.clone(){
                ui.add_space(5.0);ui.weak(format!("{} keys · source db{} · created {}",data.rows.len(),data.archive.database,archive::expiry_label(Some(data.archive.created_at_ms))));
                if self.importing{
                    ui.add_enabled_ui(!self.busy && !self.file_pending, |ui| { ui.horizontal(|ui|{
                        ui.label("Existing keys");let before=self.collision;
                        crate::controls::segments(ui,"archive-collision",&mut self.collision,&[(Collision::Skip,"Skip",58.0),(Collision::Replace,"Replace",76.0)]);
                        if before!=self.collision{self.confirm_replace=false;}
                        ui.add_space(8.0);ui.label("Expiry");crate::controls::segments(ui,"archive-expiry",&mut self.expiry,&[(Expiry::Preserve,"Keep deadlines",128.0),(Expiry::Persistent,"Remove expiry",124.0)]);
                    });
                    if self.collision==Collision::Replace{ui.checkbox(&mut self.confirm_replace,"Replace any existing target keys at execution time, including changes since this review.");}
                    if self.expiry==Expiry::Persistent{ui.add(egui::Label::new("Removing expiry also restores entries whose archived deadlines have already passed.").wrap());}
                    });
                    if let Some(review)=&self.review{
                        let expired=review.expired;
                        ui.weak(format!("Review snapshot: {} existing · {} missing · {} archived deadlines passed",review.existing,review.targets.len()-review.existing,expired));
                        if review.database!=database{ui.colored_label(ui.visuals().error_fg_color,"Database changed — review this target again.");}
                    }
                }
                ui.separator();
                review_table(ui,&data,self.review.as_ref().filter(|r|r.database==database),self.importing,if self.report.is_some(){90.0}else{240.0});
                if !data.skipped.is_empty(){egui::CollapsingHeader::new(format!("{} skipped during export",data.skipped.len())).show(ui,|ui|{egui::ScrollArea::vertical().max_height(100.0).show_rows(ui,22.0,data.skipped.len(),|ui,range|{for i in range{ui.horizontal(|ui|{ui.add(egui::Label::new(&data.skipped[i].0).truncate());ui.weak(&data.skipped[i].1);});}});});}
                ui.horizontal(|ui|{
                    if self.importing{
                        if ui.add_enabled(idle,egui::Button::new("Review target")).clicked(){self.review=None;self.start();self.status="Checking target keys…".into();action=Some(Action::Review{data:data.clone(),database});}
                        let reviewed=self.review.as_ref().is_some_and(|r|r.database==database);
                        let confirmed=self.collision!=Collision::Replace||self.confirm_replace;
                        if ui.add_enabled(idle&&reviewed&&confirmed&&!read_only,egui::Button::new("Import reviewed keys")).clicked(){self.start();self.report=None;self.status="Importing…".into();action=Some(Action::Import{data:data.clone(),database,collision:self.collision,expiry:self.expiry,read_only,cancel:self.cancel.clone()});}
                        if read_only{ui.weak("Read-only mode blocks imports.");}
                    }else if ui.add_enabled(!self.busy&&!self.file_pending&&!data.rows.is_empty(),egui::Button::new("Save archive as…")).clicked(){self.save(ctx);}
                });
            }
            if let Some(report)=&self.report{
                ui.separator();ui.strong(format!("{} restored · {} existing · {} expired · {} errors · {} unconfirmed · {} not attempted",report.counts[0],report.counts[1],report.counts[2],report.counts[3],report.counts[4],report.counts[5]));
                if let Some(reason)=&report.stopped{ui.add(egui::Label::new(reason).wrap());}
                egui::ScrollArea::both().id_salt("archive-outcomes").max_height(160.0).show_rows(ui,22.0,report.rows.len(),|ui,range|{for i in range{ui.horizontal(|ui|{ui.monospace(&report.rows[i][0]);ui.label(&report.rows[i][1]);ui.weak(&report.rows[i][2]);});}});
            }
            ui.add_space(5.0);ui.add(egui::Label::new("Archives contain raw Redis data and require compatible Redis/module versions. Export batches are separate snapshots. Import policies are enforced again at execution; completed writes are not rolled back or automatically retried.").wrap());
        });
        if !open {
            self.cancel.store(true, Ordering::Release);
        }
        self.open = open;
        action
    }
}

fn review_table(
    ui: &mut egui::Ui,
    data: &Prepared,
    review: Option<&Review>,
    importing: bool,
    height: f32,
) {
    use egui_extras::{Column, TableBuilder};
    let table = TableBuilder::new(ui)
        .striped(true)
        .min_scrolled_height(height)
        .max_scroll_height(height)
        .column(Column::remainder().clip(true))
        .column(Column::exact(80.0))
        .column(Column::exact(80.0))
        .column(Column::exact(180.0));
    let table = if importing {
        table.column(Column::exact(120.0))
    } else {
        table
    };
    table
        .header(23.0, |mut header| {
            for label in ["Key", "Type", "Stored size", "Archived expiry"] {
                header.col(|ui| {
                    ui.strong(label);
                });
            }
            if importing {
                header.col(|ui| {
                    ui.strong("Target");
                });
            }
        })
        .body(|body| {
            body.rows(23.0, data.rows.len(), |mut row| {
                let index = row.index();
                for text in &data.rows[index] {
                    row.col(|ui| {
                        ui.add(egui::Label::new(text).truncate());
                    });
                }
                if importing {
                    row.col(|ui| {
                        if let Some(target) = review.and_then(|r| r.targets.get(index)) {
                            ui.add(
                                egui::Label::new(if target.kind == "none" {
                                    "Missing"
                                } else {
                                    &target.kind
                                })
                                .truncate(),
                            )
                            .on_hover_text(&target.expiry);
                        } else {
                            ui.weak("Not checked");
                        }
                    });
                }
            });
        });
}

#[cfg(feature = "qa")]
impl View {
    pub fn preview_load_export(&mut self, ctx: &egui::Context) -> bool {
        if !self.importing {
            let Some(data) = self.data.clone() else {
                return false;
            };
            self.importing = true;
            self.file_pending = true;
            let tx = self.files_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = (|| {
                    let mut data = Archive::decode(&data.archive.encode()?)?;
                    if let Some(entry) = data.entries.last_mut() {
                        entry.key.splice(0..0, b"imported:".iter().copied());
                    }
                    data.entries.push(archive::Entry {
                        key: b"imported:expired".to_vec(),
                        kind: "string".into(),
                        expires_at_ms: Some(1),
                        dump: data.entries[0].dump.clone(),
                    });
                    data.entries.push(archive::Entry {
                        key: b"imported:invalid-fixture".to_vec(),
                        kind: "string".into(),
                        expires_at_ms: None,
                        dump: b"intentional invalid fixture".to_vec(),
                    });
                    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
                    let path = directory.path().join("fixture.rredis");
                    data.save(&path, false)?;
                    Archive::read(&path).map(|a| Arc::new(Prepared::new(a, vec![])))
                })();
                let _ = tx.send(FileEvent::Loaded(result));
                ctx.request_repaint();
            });
        }
        !self.file_pending
    }
    pub fn preview_review(&mut self, database: u32) -> Option<Action> {
        let data = self.data.clone()?;
        self.review = None;
        self.start();
        Some(Action::Review { data, database })
    }
    pub fn preview_options(&mut self) {
        self.collision = Collision::Replace;
        self.confirm_replace = false;
    }
    pub fn preview_import(&mut self, database: u32) -> Option<Action> {
        let data = self.data.clone()?;
        self.collision = Collision::Skip;
        self.start();
        Some(Action::Import {
            data,
            database,
            collision: Collision::Skip,
            expiry: self.expiry,
            read_only: false,
            cancel: self.cancel.clone(),
        })
    }
    pub fn preview_clear_report(&mut self) {
        self.report = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prepared() -> Arc<Prepared> {
        Arc::new(Prepared::new(
            Archive {
                created_at_ms: 1700000000000,
                database: 2,
                entries: vec![archive::Entry {
                    key: b"binary\0\xff".to_vec(),
                    kind: "string".into(),
                    expires_at_ms: None,
                    dump: vec![1, 2, 3],
                }],
            },
            vec![],
        ))
    }
    #[test]
    fn database_changes_and_read_only_fail_before_connection_use() {
        let mut session = crate::redis_session::Session::disconnected();
        let data = prepared();
        let result = execute(
            Action::Import {
                data: data.clone(),
                database: 2,
                collision: Collision::Replace,
                expiry: Expiry::Persistent,
                read_only: false,
                cancel: Arc::new(AtomicBool::new(false)),
            },
            &mut session,
            3,
        );
        assert!(matches!(result,Err(error) if error.contains("database changed")));
        let result = execute(
            Action::Import {
                data,
                database: 3,
                collision: Collision::Replace,
                expiry: Expiry::Persistent,
                read_only: true,
                cancel: Arc::new(AtomicBool::new(false)),
            },
            &mut session,
            3,
        );
        assert!(matches!(result,Err(error) if error.contains("Read-only")));
    }
    #[test]
    fn reviewed_file_bytes_are_immutable_and_operations_cannot_overlap() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture.rredis");
        prepared().archive.save(&path, false).unwrap();
        let loaded = Arc::new(Prepared::new(Archive::read(&path).unwrap(), vec![]));
        std::fs::write(&path, b"replaced after review").unwrap();
        assert_eq!(loaded.archive.entries[0].dump, vec![1, 2, 3]);
        let mut view = View::default();
        assert!(view.can_start());
        view.file_pending = true;
        assert!(!view.can_start());
        view.file_pending = false;
        view.start();
        assert!(!view.can_start());
        let cancel = view.cancel.clone();
        drop(view);
        assert!(cancel.load(Ordering::Acquire));
    }
    #[test]
    fn cancelled_or_invalid_file_keeps_previous_review() {
        let ctx = egui::Context::default();
        let mut view = View::default();
        let data = prepared();
        view.data = Some(data.clone());
        for event in [
            FileEvent::Cancelled,
            FileEvent::Loaded(Err("invalid archive".into())),
        ] {
            view.file_pending = true;
            view.files_tx.send(event).unwrap();
            assert!(view.show(&ctx, "Fixture", 2, true, false).is_none());
            assert!(Arc::ptr_eq(view.data.as_ref().unwrap(), &data));
            assert!(!view.file_pending);
        }
        view.confirm_replace = true;
        view.files_tx
            .send(FileEvent::Loaded(Ok(prepared())))
            .unwrap();
        view.show(&ctx, "Fixture", 2, true, false);
        assert!(!view.confirm_replace);
        assert!(view.importing);
        assert!(!Arc::ptr_eq(view.data.as_ref().unwrap(), &data));
    }
    #[test]
    #[ignore = "manual archive review CPU workloads"]
    fn redis_archive_frame_benchmark() {
        for scenario in 0..3 {
            let ctx = egui::Context::default();
            let mut view = View::default();
            view.open = true;
            view.importing = true;
            let mut archive = Archive {
                created_at_ms: 1700000000000,
                database: 2,
                entries: vec![],
            };
            archive.entries = (0..1000)
                .map(|i| archive::Entry {
                    key: format!("archive:{i:04}:{}", "x".repeat(1000)).into_bytes(),
                    kind: "string".into(),
                    expires_at_ms: Some(1800000000000),
                    dump: vec![0; 60 * 1024],
                })
                .collect();
            let data = Arc::new(Prepared::new(archive, vec![]));
            view.data = Some(data.clone());
            if scenario == 1 {
                view.report = Some(Report {
                    rows: data
                        .rows
                        .iter()
                        .map(|r| [r[0].clone(), "Restored".into(), "".into()])
                        .collect(),
                    counts: [1000, 0, 0, 0, 0, 0],
                    stopped: None,
                });
            }
            let mut samples = Vec::new();
            for i in 0..180 {
                if scenario == 2 && i % 10 == 0 {
                    view.files_tx
                        .send(FileEvent::Loaded(Ok(data.clone())))
                        .unwrap();
                }
                let started = std::time::Instant::now();
                if i % 20 == 0 {
                    view.collision = if i % 40 == 0 {
                        Collision::Replace
                    } else {
                        Collision::Skip
                    };
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
                        view.show(ctx, "Fixture", 2, true, false);
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
                    "redis/archive-review",
                    "redis/archive-results",
                    "redis/archive-handoff",
                ][scenario],
                &samples,
            );
        }
    }
    #[test]
    fn review_stays_bounded_and_settles_at_idle() {
        let ctx = egui::Context::default();
        let mut view = View::default();
        view.open = true;
        view.importing = true;
        view.data = Some(prepared());
        for i in 0..60 {
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
                    assert!(view.show(ctx, "Fixture", 2, true, true).is_none());
                },
            );
            if i == 59 {
                assert!(
                    output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
                        > std::time::Duration::from_millis(500)
                );
            }
        }
        let rect = ctx
            .memory(|m| m.area_rect(egui::Id::new("Redis import / export")))
            .unwrap();
        assert!(rect.width() <= 1200.0 && rect.height() <= 760.0, "{rect:?}");
    }
}
