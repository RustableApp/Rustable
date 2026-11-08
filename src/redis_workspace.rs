//! Redis-native UI; network and decoding work remain on a dedicated worker.
#[cfg(target_arch = "wasm32")]
use crate::redis_demo as redis;
use crate::redis_value::{Cursor, Inspection, Page, PageRequest, StringMode};
use crate::{
    redis_session::{KeyPage, KeyRow, Session, Snapshot},
    redis_settings::Settings,
};
use eframe::egui;
use std::sync::mpsc;

enum Job {
    #[cfg(not(target_arch = "wasm32"))]
    Bulk(crate::redis_mutation::Mutation, bool, std::sync::Arc<std::sync::atomic::AtomicBool>),
    #[cfg(not(target_arch = "wasm32"))]
    Archive(crate::redis_archive_view::Action),
    #[cfg(not(target_arch = "wasm32"))]
    Publish(crate::redis_pubsub_view::Publish, bool),
    Search(crate::redis_search::Request),
    SearchMutation(crate::redis_search_manage::Mutation, bool),
    Diagnostics(crate::redis_diagnostics::Request),
    Mutate(crate::redis_mutation::Mutation, bool),
    Scan(u64, Vec<u8>, String),
    Rows(Vec<Vec<u8>>),
    Inspect(Vec<u8>),
    ValuePage(PageRequest, Navigation),
    Execute(String, bool),
    Save(Vec<u8>, Vec<u8>, Vec<u8>, bool),
    Delete(Vec<u8>, bool),
    Expire(Vec<u8>, Option<u64>, bool),
    Database(u32),
    #[cfg(not(target_arch = "wasm32"))]
    Reconnect,
}
#[derive(Clone, Copy)]
enum Navigation {
    Next,
    Previous,
    Search,
}
enum Reply {
    #[cfg(not(target_arch = "wasm32"))]
    Bulk(crate::redis_bulk_view::View, bool),
    #[cfg(not(target_arch = "wasm32"))]
    Archive(crate::redis_archive_view::Reply),
    #[cfg(not(target_arch = "wasm32"))]
    Published(u64),
    Search(crate::redis_search::Reply),
    Diagnostics(Box<crate::redis_diagnostics::Report>),
    Mutation(crate::redis_mutation::Mutation, u64),
    Connected(u32),
    #[cfg(not(target_arch = "wasm32"))]
    Recovering(String),
    #[cfg(not(target_arch = "wasm32"))]
    Recovered,
    #[cfg(not(target_arch = "wasm32"))]
    RecoveryFailed(String),
    Keys(KeyPage),
    Rows(Vec<KeyRow>),
    Value(Box<Inspection>),
    Output(crate::redis_command_results::Report),
    Changed(&'static str),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Section {
    #[default]
    Browse,
    Search,
    Workbench,
    Analyze,
    PubSub,
}
pub(crate) struct Workspace {
    #[cfg(not(target_arch = "wasm32"))]
    sender: mpsc::SyncSender<Job>,
    #[cfg(target_arch = "wasm32")]
    demo: Session,
    #[cfg(target_arch = "wasm32")]
    responses: mpsc::Sender<Result<Reply, (String, bool)>>,
    receiver: mpsc::Receiver<Result<Reply, (String, bool)>>,
    busy: bool,
    pub connected: bool,
    database: u32,
    database_draft: u32,
    cluster: bool,
    keys: Vec<Vec<u8>>,
    rows: std::collections::HashMap<Vec<u8>, (String, String, i64)>,
    section: Section,
    analysis_server: bool,
    #[cfg(not(target_arch = "wasm32"))]
    analysis: crate::redis_analysis_view::View,
    commands_visible: bool,
    inspector_visible: bool,
    tree_view: bool,
    separator: String,
    expanded: std::collections::HashSet<Vec<u8>>,
    tree: Vec<TreeRow>,
    kind: String,
    scan_kind: String,
    exact_key: String,
    auto_scan: bool,
    browse_busy: bool,
    pending: Option<Job>,
    refresh_pending: bool,
    visible_indices: Vec<usize>,
    preview_queue: std::collections::VecDeque<Vec<u8>>,
    database_picker: bool,
    exact_mode: bool,
    cursor: u64,
    scan_started: bool,
    pattern: String,
    scan_pattern: Vec<u8>,
    key_limit: bool,
    selected_key: Option<Vec<u8>>,
    snapshot: Option<Snapshot>,
    value_editor: String,
    value_page: Option<Page>,
    value_history: Vec<Cursor>,
    pending_navigation: Option<Navigation>,
    value_search: String,
    time_range: [String; 2],
    time_chart: bool,
    time_chart_focus: bool,
    value_matches: Vec<usize>,
    selected_value: Option<usize>,
    string_mode: StringMode,
    preview: String,
    output: String,
    command_results: crate::redis_command_results::Report,
    command_result_heights: Vec<f32>,
    command_result_width: f32,
    command: String,
    command_editor: crate::redis_command_editor::Editor,
    error: Option<String>,
    notice: Option<String>,
    read_only: bool,
    ttl_seconds: u64,
    delete_confirmation: bool,
    edit: Option<crate::redis_edit::Draft>,
    #[cfg(not(target_arch = "wasm32"))]
    bulk_results: Option<crate::redis_bulk_view::View>,
    #[cfg(not(target_arch = "wasm32"))]
    bulk_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    search: crate::redis_search::View,
    #[cfg(not(target_arch = "wasm32"))]
    monitor: crate::redis_monitor::View,
    #[cfg(not(target_arch = "wasm32"))]
    pubsub: crate::redis_pubsub_view::View,
    #[cfg(not(target_arch = "wasm32"))]
    archive: crate::redis_archive_view::View,
    diagnostics_open: bool,
    diagnostics_report: Option<Box<crate::redis_diagnostics::Report>>,
    diagnostics_request: Option<crate::redis_diagnostics::Request>,
    preferences_loaded: bool,
    snippets: Vec<crate::redis_preferences::Snippet>,
    snippets_dirty: bool,
    snippet_name: String,
    command_history: std::collections::VecDeque<String>,
    history_profile: Option<u64>,
    history_recording: bool,
    history_dirty: bool,
    bulk_mode: bool,
    bulk_selected: std::collections::BTreeSet<Vec<u8>>,
}
#[cfg(not(target_arch = "wasm32"))]
impl Drop for Workspace {
    fn drop(&mut self) {
        if let Some(cancel) = &self.bulk_cancel {
            cancel.store(true, std::sync::atomic::Ordering::Release);
        }
    }
}
impl Workspace {
    #[cfg(all(test, not(target_arch = "wasm32")))]
    fn prepare_analysis_benchmark(&mut self) { self.analysis.prepare_benchmark(); }

    #[cfg(any(test, feature = "qa", target_arch = "wasm32"))]
    pub fn new(settings: Settings, password: String, ctx: egui::Context) -> Self {
        Self::new_with_sentinel(settings, password, String::new(), ctx)
    }
    pub fn new_with_sentinel(mut settings: Settings, password: String, sentinel_password: String, ctx: egui::Context) -> Self {
        let database = settings.database;
        #[cfg(not(target_arch = "wasm32"))]
        let analysis = crate::redis_analysis_view::View::new(settings.clone(), password.clone(), sentinel_password.clone());
        let cluster = settings.cluster;
        #[cfg(not(target_arch = "wasm32"))]
        let monitor = crate::redis_monitor::View::new(settings.clone(), password.clone(), sentinel_password.clone());
        #[cfg(not(target_arch = "wasm32"))]
        let pubsub = crate::redis_pubsub_view::View::new(settings.clone(), password.clone(), sentinel_password.clone());
        #[cfg(not(target_arch = "wasm32"))]
        let (sender, jobs) = mpsc::sync_channel(1);
        let (responses, receiver) = mpsc::channel();
        #[cfg(target_arch = "wasm32")]
        let demo = {
            let _ = (&mut settings, &password, &sentinel_password, &ctx);
            responses.send(Ok(Reply::Connected(database))).unwrap();
            Session::new()
        };
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(move || {
            let mut session = match Session::connect_with_sentinel(&settings, &password, &sentinel_password) {
                Ok(session) => {
                    let _ = responses.send(Ok(Reply::Connected(settings.database)));
                    ctx.request_repaint();
                    session
                }
                Err(error) => {
                    let _ = responses.send(Err((error, false)));
                    ctx.request_repaint();
                    Session::disconnected()
                }
            };
            while let Ok(job) = jobs.recv() {
                let recover = (settings.sentinel.is_some() || settings.cluster) && session.is_connected() && !matches!(&job, Job::Database(_) | Job::Reconnect);
                let result = match job {
                    Job::Bulk(mutation, safe, cancel) => session.bulk_mutate(&mutation, safe, &cancel).map(|report| Reply::Bulk(crate::redis_bulk_view::View::new(report), session.is_connected())),
                    Job::Archive(action) => {
                        crate::redis_archive_view::execute(action, &mut session, settings.database)
                            .map(Reply::Archive)
                    }
                    Job::Publish(request, safe) => session
                        .publish(&request.channel, &request.payload, safe)
                        .map(Reply::Published),
                    Job::Scan(cursor, pattern, kind) => session
                        .scan_filtered(cursor, &pattern, 200, &kind)
                        .map(Reply::Keys),
                    Job::Rows(keys) => session.key_rows(&keys).map(Reply::Rows),
                    Job::Inspect(key) => session.inspect_view(&key).map(Box::new).map(Reply::Value),
                    Job::ValuePage(request, _) => {
                        session.value_page(&request).map(Box::new).map(Reply::Value)
                    }
                    Job::Search(request) => session.search(request).map(Reply::Search),
                    Job::SearchMutation(mutation, safe) => session.search_mutate(mutation, safe).map(Reply::Search),
                    Job::Diagnostics(request) => session
                        .diagnostics(&request)
                        .map(Box::new)
                        .map(Reply::Diagnostics),
                    Job::Mutate(mutation, safe) => session
                        .mutate(&mutation, safe)
                        .map(|count| Reply::Mutation(mutation, count)),
                    Job::Execute(text, safe) => session
                        .execute_report(&text, safe)
                        .map(Reply::Output),
                    Job::Save(key, original, value, safe) => session
                        .replace_string(&key, &original, &value, safe)
                        .map(|_| Reply::Changed("Value saved.")),
                    Job::Delete(key, safe) => session.delete(&key, safe).map(|deleted| {
                        Reply::Changed(if deleted {
                            "Key deleted. Refresh keys to update the list."
                        } else {
                            "The key was already absent."
                        })
                    }),
                    Job::Expire(key, ttl, safe) => session.expire(&key, ttl, safe).map(|changed| {
                        Reply::Changed(if changed {
                            "Expiry updated."
                        } else {
                            "Key no longer exists or expiry was unchanged."
                        })
                    }),
                    Job::Reconnect => Ok(match session.reconnect(&settings, &password, &sentinel_password) {
                        Ok(()) => Reply::Recovered,
                        Err(error) => Reply::RecoveryFailed(error),
                    }),
                    Job::Database(database) => {
                        let candidate = Settings {
                            database,
                            ..settings.clone()
                        };
                        Session::connect_with_sentinel(&candidate, &password, &sentinel_password).map(|connection| {
                            session = connection;
                            settings = candidate;
                            Reply::Connected(database)
                        })
                    }
                };
                // Reconnect the transport once; never retain or replay the completed job.
                // Startup and explicit connection changes remain user-controlled.
                if recover && !session.is_connected() {
                    let original = match result {
                        Err(error) => error,
                        Ok(reply) => {
                            if responses.send(Ok(reply)).is_err() { break; }
                            "Redis connection lost. Review the operation results before retrying any write.".into()
                        }
                    };
                    if responses.send(Ok(Reply::Recovering(original.clone()))).is_err() { break; }
                    ctx.request_repaint();
                    let restored = session.reconnect(&settings, &password, &sentinel_password)
                        .map(|()| Reply::Recovered)
                        .map_err(|error| (format!("{original}\nAutomatic reconnect failed: {error}"), false));
                    if responses.send(restored).is_err() { break; }
                    ctx.request_repaint();
                    continue;
                }
                if responses
                    .send(result.map_err(|error| (error, session.is_connected())))
                    .is_err()
                {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            sender,
            #[cfg(target_arch = "wasm32")]
            demo,
            #[cfg(target_arch = "wasm32")]
            responses,
            receiver,
            busy: true,
            connected: false,
            database,
            database_draft: database,
            cluster,
            keys: vec![],
            rows: Default::default(),
            section: Section::Browse,
            analysis_server: false,
            #[cfg(not(target_arch = "wasm32"))]
            analysis,
            commands_visible: cfg!(target_arch = "wasm32"),
            inspector_visible: false,
            tree_view: true,
            separator: ":".into(),
            expanded: Default::default(),
            tree: vec![],
            kind: String::new(),
            scan_kind: String::new(),
            exact_key: String::new(),
            auto_scan: false,
            browse_busy: false,
            pending: None,
            refresh_pending: false,
            visible_indices: vec![],
            preview_queue: Default::default(),
            database_picker: false,
            exact_mode: false,
            cursor: 0,
            scan_started: false,
            pattern: "*".into(),
            scan_pattern: b"*".to_vec(),
            key_limit: false,
            selected_key: None,
            snapshot: None,
            value_editor: String::new(),
            value_page: None,
            value_history: vec![],
            pending_navigation: None,
            value_search: String::new(),
            time_range: Default::default(),
            time_chart: false,
            time_chart_focus: false,
            value_matches: vec![],
            selected_value: None,
            string_mode: StringMode::Text,
            preview: String::new(),
            command_results: Default::default(),
            command_result_heights: Vec::new(),
            command_result_width: 0.0,
            output: if cfg!(target_arch = "wasm32") {
                "In-memory Redis demo · Rust/WASM. Data stays in this tab and resets on reload. Run INFO for supported commands; key searches support * and ?.".into()
            } else {
                String::new()
            },
            command_editor: Default::default(),
            command: if cfg!(target_arch = "wasm32") {
                "INFO"
            } else {
                "PING"
            }
            .into(),
            error: None,
            notice: None,
            read_only: true,
            ttl_seconds: 3600,
            delete_confirmation: false,
            edit: None,
            #[cfg(not(target_arch = "wasm32"))]
            bulk_results: None,
            #[cfg(not(target_arch = "wasm32"))]
            bulk_cancel: None,
            search: Default::default(),
            #[cfg(not(target_arch = "wasm32"))]
            monitor,
            #[cfg(not(target_arch = "wasm32"))]
            pubsub,
            #[cfg(not(target_arch = "wasm32"))]
            archive: Default::default(),
            diagnostics_open: false,
            diagnostics_report: None,
            diagnostics_request: None,
            preferences_loaded: false,
            snippets: vec![],
            snippets_dirty: false,
            snippet_name: String::new(),
            command_history: Default::default(),
            history_profile: None,
            history_recording: true,
            history_dirty: false,
            bulk_mode: false,
            bulk_selected: Default::default(),
        }
    }
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))] // Native multiwindow synchronization.
    pub fn set_history_profile(&mut self, profile: Option<u64>) {
        if self.history_profile != profile {
            self.history_profile = profile;
            self.command_history.clear(); self.history_recording = true; self.history_dirty = false;
            self.preferences_loaded = false;
        }
    }
    pub fn reset_preferences(&mut self) {
        self.preferences_loaded = false;
    }
    pub fn load_preferences(&mut self, preferences: &crate::redis_preferences::Preferences) {
        if self.preferences_loaded {
            return;
        }
        let mut preferences = preferences.clone();
        preferences.sanitize();
        self.tree_view = preferences.tree;
        self.separator = preferences.separator;
        self.snippets = preferences.snippets;
        self.history_recording = preferences.record_history;
        if let Some(profile) = self.history_profile {
            let history = preferences.histories.remove(&profile).unwrap_or_default();
            self.command_history = history.commands;
            self.history_dirty = false;
        }
        self.preferences_loaded = true;
        self.rebuild_tree();
    }
    pub fn store_preferences(
        &mut self,
        preferences: &mut crate::redis_preferences::Preferences,
    ) -> bool {
        let changed = preferences.tree != self.tree_view
            || preferences.separator != self.separator
            || self.snippets_dirty || self.history_dirty;
        if changed {
            if self.history_dirty {
                preferences.record_history = self.history_recording;
                if let Some(profile) = self.history_profile {
                    preferences.histories.insert(profile, crate::redis_history::History {
                        commands: self.command_history.clone(),
                        updated_ms: chrono::Utc::now().timestamp_millis().max(0) as u64,
                    });
                    preferences.limit_profiles();
                }
                self.history_dirty = false;
            }
            preferences.tree = self.tree_view;
            preferences.separator = self.separator.clone();
            if self.snippets_dirty {
                preferences.snippets = self.snippets.clone();
                self.snippets_dirty = false;
            }
        }
        changed
    }
    #[cfg(all(feature = "qa", not(target_arch = "wasm32")))]
    pub(crate) fn preview_commands(&mut self) -> bool {
        if self.busy || !self.connected { return false; }
        self.auto_scan = false;
        self.preview_queue.clear();
        self.commands_visible = true;
        self.section = Section::Workbench;
        self.inspector_visible = false;
        self.command = "GET app:config:theme\nHGETALL app:users:1001\nZRANGE leaderboard:weekly 0 5 WITHSCORES\nSCAN 0 MATCH \"app:*\" COUNT 100".into();
        if let Ok(text)=std::env::var("RUSTABLE_REDIS_COMPLETION_CAPTURE") {
            self.command=text; self.command_editor.focus();
        }
        self.send(Job::Execute("PING\nGET app:config:theme".into(), true));
        true
    }
    #[cfg(all(feature = "qa", not(target_arch = "wasm32")))]
    pub(crate) fn preview_vectors(&mut self, step: usize) -> bool {
        if self.busy || !self.connected { return false; }
        if self.selected_key.as_deref() != Some(b"search:product:embeddings") {
            if !self.keys.iter().any(|key| key == b"search:product:embeddings") { return false; }
            self.auto_scan = false;
            self.send(Job::Inspect(b"search:product:embeddings".to_vec()));
            return false;
        }
        self.preview_queue.clear();
        if self.snapshot.is_none() { return false; }
        if step > 0 {
            let key = b"search:product:embeddings".to_vec();
            let element = b"product:00000:description".to_vec();
            let request = if step == 1 { crate::redis_diagnostics::Request::VectorElement { key, element } } else { crate::redis_diagnostics::Request::VectorSimilar { key, element } };
            self.diagnostics_request = Some(request.clone());
            self.diagnostics_report = None;
            self.diagnostics_open = true;
            self.send(Job::Diagnostics(request));
        }
        true
    }
    #[cfg(all(feature = "qa", not(target_arch = "wasm32")))]
    pub(crate) fn preview_timeseries(&mut self, step: usize) -> bool {
        if self.busy || !self.connected { return false; }
        if self.selected_key.as_deref() != Some(b"telemetry:temperature") {
            if !self.keys.iter().any(|key| key == b"telemetry:temperature") { return false; }
            self.auto_scan = false;
            self.send(Job::Inspect(b"telemetry:temperature".to_vec()));
            return false;
        }
        if self.snapshot.is_none() { return false; }
        self.auto_scan = false;
        self.preview_queue.clear();
        if step == 3 {
            self.diagnostics_open = false;
            self.time_chart = true;
            self.time_chart_focus = true;
        } else if step == 2 {
            let request = crate::redis_diagnostics::Request::TimeSeries(b"telemetry:temperature".to_vec());
            self.diagnostics_open = true;
            self.diagnostics_report = None;
            self.diagnostics_request = Some(request.clone());
            self.send(Job::Diagnostics(request));
        } else if step == 1 {
            self.time_range = ["1700000150000".into(), "1700000350000".into()];
            self.send(Job::ValuePage(PageRequest { key: b"telemetry:temperature".to_vec(), kind: "TSDB-TYPE".into(), cursor: crate::redis_value::timestamp_range(&self.time_range[0], &self.time_range[1]).unwrap(), pattern: b"*".to_vec() }, Navigation::Search));
        }
        true
    }
    #[cfg(all(feature = "qa", not(target_arch = "wasm32")))]
    pub(crate) fn preview_recovery(&mut self, step: usize) -> bool {
        if self.busy || !self.connected { return false; }
        if self.selected_key.as_deref() != Some(b"cache:profile") {
            if !self.keys.iter().any(|key| key == b"cache:profile") { return false; }
            self.auto_scan = false;
            self.send(Job::Inspect(b"cache:profile".to_vec()));
            return false;
        }
        if self.snapshot.is_none() { return false; }
        self.read_only = false;
        self.auto_scan = false;
        self.preview_queue.clear();
        if step == 0 {
            self.string_mode = StringMode::Edit;
            self.value_editor = crate::redis_commands::quote(br#"{"user":"Avery Chen","plan":"Team","note":"Unsaved draft"}"#);
        } else {
            self.connection_failed("Connection lost after submitting a write. It may have completed; verify the stored value before retrying.".into(), false);
            self.connection_recovered();
        }
        true
    }
    pub fn busy(&self) -> bool {
        self.busy || self.value_dirty()
    }
    fn value_dirty(&self) -> bool {
        self.edit.as_ref().is_some_and(|e| e.action == crate::redis_mutation::Action::Update && e.kind == "ReJSON-RL") || self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.kind == "string"
                && !snapshot.truncated
                && match &snapshot.value {
                    redis::Value::BulkString(bytes) => {
                        if let Some(page) = &self.value_page {
                            self.value_editor != page.display.strings.quoted
                        } else {
                            self.value_editor != crate::redis_commands::quote(bytes)
                        }
                    }
                    _ => false,
                }
        })
    }
    fn send(&mut self, job: Job) {
        if self.busy {
            if self.browse_busy && matches!(job, Job::Inspect(_)) && !self.value_dirty() {
                if let Job::Inspect(key) = &job {
                    self.selected_key = Some(key.clone());
                }
                self.pending = Some(job);
            }
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let keep_error = matches!(&job, Job::Reconnect);
        #[cfg(target_arch = "wasm32")]
        let keep_error = false;
        let preserves_draft = matches!(&job, Job::Save(..)) || matches!(&job, Job::Mutate(request, _) if self.edit.as_ref().is_some_and(|e| e.action == crate::redis_mutation::Action::Update && e.kind == "ReJSON-RL" && request.action == e.action && request.keys == e.keys));
        #[cfg(not(target_arch = "wasm32"))]
        let preserves_draft = preserves_draft || matches!(&job, Job::Reconnect);
        if self.value_dirty() && !preserves_draft {
            self.error = Some("Save or discard the edited value before changing keys, databases, or running another operation.".into());
            return;
        }
        if let Job::Execute(text, _) = &job
            && self.history_recording && crate::redis_history::eligible(text)
            && self.command_history.back() != Some(text)
        {
            self.command_history.push_back(text.clone());
            crate::redis_history::trim(&mut self.command_history);
            self.history_dirty = true;
        }
        self.browse_busy = matches!(&job, Job::Scan(..) | Job::Rows(..));
        let navigation = if let Job::ValuePage(_, nav) = &job {
            Some(*nav)
        } else {
            None
        };
        let selected = match &job {
            Job::Inspect(key) => {
                self.value_page = None;
                self.value_history.clear();
                self.value_search.clear();
                self.time_range = Default::default();
                self.value_matches.clear();
                self.selected_value = None;
                self.string_mode = StringMode::Text;
                Some(key.clone())
            }
            _ => None,
        };
        #[cfg(target_arch = "wasm32")]
        let sent = {
            let result = match job {
                Job::Scan(cursor, pattern, kind) => self
                    .demo
                    .scan_filtered(cursor, &pattern, 200, &kind)
                    .map(Reply::Keys),
                Job::Rows(keys) => self.demo.key_rows(&keys).map(Reply::Rows),
                Job::Inspect(key) => self.demo.inspect_view(&key).map(Box::new).map(Reply::Value),
                Job::ValuePage(request, _) => self
                    .demo
                    .value_page(&request)
                    .map(Box::new)
                    .map(Reply::Value),
                Job::Search(request) => self.demo.search(request).map(Reply::Search),
                Job::SearchMutation(mutation, safe) => mutation.validate(safe).and_then(|_| Err("Index management requires a desktop Redis Search connection.".into())),
                Job::Diagnostics(request) => self
                    .demo
                    .diagnostics(&request)
                    .map(Box::new)
                    .map(Reply::Diagnostics),
                Job::Mutate(mutation, safe) => self
                    .demo
                    .mutate(&mutation, safe)
                    .map(|count| Reply::Mutation(mutation, count)),
                Job::Execute(text, safe) => self
                    .demo
                    .execute_report(&text, safe)
                    .map(Reply::Output),
                Job::Save(key, original, value, safe) => self
                    .demo
                    .replace_string(&key, &original, &value, safe)
                    .map(|_| Reply::Changed("Value saved.")),
                Job::Delete(key, safe) => self
                    .demo
                    .delete(&key, safe)
                    .map(|_| Reply::Changed("Key deleted. Refresh keys to update the list.")),
                Job::Expire(key, ttl, safe) => self
                    .demo
                    .expire(&key, ttl, safe)
                    .map(|_| Reply::Changed("Expiry updated.")),
                Job::Database(database) => self
                    .demo
                    .select(database)
                    .map(|_| Reply::Connected(database)),
            };
            self.responses.send(result.map_err(|e| (e, true)))
        };
        #[cfg(not(target_arch = "wasm32"))]
        let sent = self.sender.try_send(job);
        match sent {
            Ok(()) => {
                self.pending_navigation = navigation;
                if let Some(key) = selected {
                    self.selected_key = Some(key);
                }
                self.busy = true;
                if !keep_error { self.error = None; }
                self.notice = None;
            }
            Err(_) => {
                self.error = Some("Redis worker is unavailable. Reopen this connection.".into())
            }
        }
    }
    fn refresh(&mut self) {
        if self.value_dirty() {
            self.error = Some("Save or discard the edited value before refreshing keys.".into());
            return;
        }
        if self.busy && !self.browse_busy {
            return;
        }
        self.scan_pattern = self.pattern.as_bytes().to_vec();
        self.scan_kind = self.kind.clone();
        if self.busy {
            if self.browse_busy {
                self.refresh_pending = true;
                self.auto_scan = false;
                self.pending = None;
            }
            return;
        }
        self.begin_scan();
    }
    fn begin_scan(&mut self) {
        self.refresh_pending = false;
        self.auto_scan = true;
        self.keys.clear();
        self.tree.clear();
        self.visible_indices.clear();
        self.preview_queue.clear();
        self.rows.clear();
        self.cursor = 0;
        self.scan_started = false;
        self.key_limit = false;
        self.send(Job::Scan(
            0,
            self.scan_pattern.clone(),
            self.scan_kind.clone(),
        ));
    }
    fn connection_failed(&mut self, error: String, connected: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.archive.busy {
            self.archive.failed(error.clone());
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.pubsub.publishing {
            self.pubsub.published(Err(error.clone()));
        }
        self.pending_navigation = None;
        if self.search.pending {
            self.search.pending = false;
            self.search.error = Some(error.clone());
        }
        if let Some(edit) = &mut self.edit {
            edit.error = Some(error.clone());
        }
        self.error = Some(error);
        self.auto_scan = false;
        self.refresh_pending = false;
        self.connected = connected;
        self.pending = None;
        self.notice = None;
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn connection_recovered(&mut self) {
        self.connected = true;
        self.notice = Some("Reconnected. The failed operation was not replayed. Displayed data may be stale; refresh after reviewing your drafts and any uncertain write.".into());
        // Keep drafts, results and the original error. Do not reuse a scan
        // cursor from another node or trigger background reads here.
        self.cursor = 0;
        self.auto_scan = false;
        self.preview_queue.clear();
        self.pending = None;
    }
    fn poll(&mut self) {
        while let Ok(reply) = self.receiver.try_recv() {
            self.busy = false;
            self.browse_busy = false;
            if self.refresh_pending && matches!(&reply, Ok(Reply::Keys(_) | Reply::Rows(_))) {
                self.begin_scan();
                continue;
            }
            match reply {
                Err((error, connected)) => {
                    self.connection_failed(error, connected);
                }
                #[cfg(not(target_arch = "wasm32"))]
                Ok(Reply::Archive(reply)) => {
                    self.archive.complete(reply);
                }
                #[cfg(not(target_arch = "wasm32"))]
                Ok(Reply::Published(count)) => {
                    self.pubsub.published(Ok(count));
                }
                #[cfg(not(target_arch = "wasm32"))]
                Ok(Reply::Recovering(error)) => {
                    self.connection_failed(error, false);
                    self.busy = true;
                    self.notice = Some(if self.cluster { "Reconnecting through known Cluster nodes…" } else { "Reconnecting to the Sentinel primary…" }.into());
                }
                #[cfg(not(target_arch = "wasm32"))]
                Ok(Reply::RecoveryFailed(error)) => {
                    let original = self.error.clone().unwrap_or_else(|| "Redis connection closed.".into());
                    self.connection_failed(original, false);
                    self.notice = Some(format!("Reconnect failed: {error}"));
                }
                #[cfg(not(target_arch = "wasm32"))]
                Ok(Reply::Recovered) => {
                    self.connection_recovered();
                }
                Ok(Reply::Connected(database)) => {
                    self.connected = true;
                    self.database = database;
                    self.database_draft = database;
                    self.search = Default::default();
                    self.diagnostics_open = false;
                    self.diagnostics_request = None;
                    self.diagnostics_report = None;
                    self.snapshot = None;
                    self.selected_key = None;
                    self.bulk_selected.clear();
                    self.edit = None;
                    self.expanded.clear();
                    self.refresh();
                }
                Ok(Reply::Rows(rows)) => {
                    for row in rows {
                        self.rows.insert(
                            row.key,
                            (grid_preview(&row.value, &row.kind), row.kind, row.ttl_ms),
                        );
                    }
                }
                Ok(Reply::Keys(mut page)) => {
                    self.cursor = page.cursor;
                    self.scan_started = true;
                    let mut bytes = self.keys.iter().map(Vec::len).sum::<usize>();
                    page.keys.sort();
                    page.keys.dedup();
                    page.keys
                        .retain(|key| self.keys.binary_search(key).is_err());
                    for key in page.keys {
                        if self.keys.len() >= 10000
                            || bytes.saturating_add(key.len()) > 16 * 1024 * 1024
                        {
                            self.key_limit = true;
                            break;
                        }
                        bytes += key.len();
                        self.preview_queue.push_back(key.clone());
                        self.keys.push(key);
                    }
                    self.keys.sort();
                    self.rebuild_tree();
                    if self.cursor == 0 || self.key_limit {
                        self.auto_scan = false;
                    }
                }
                Ok(Reply::Value(inspection)) => {
                    let snapshot = inspection.snapshot;
                    match self.pending_navigation.take() {
                        Some(Navigation::Next) => {
                            if self.value_history.len() >= 512 {
                                self.value_history.remove(0);
                            }
                            if let Some(page) = &self.value_page {
                                self.value_history.push(page.cursor.clone());
                            }
                        }
                        Some(Navigation::Previous) => {
                            self.value_history.pop();
                        }
                        Some(Navigation::Search) => self.value_history.clear(),
                        None => {}
                    }
                    self.value_page = Some(inspection.page);
                    self.selected_value = None;
                    self.update_value_matches(&snapshot.kind);
                    if self.keys.binary_search(&snapshot.key).is_ok() {
                        self.rows.insert(
                            snapshot.key.clone(),
                            (
                                grid_preview(&snapshot.value, &snapshot.kind),
                                snapshot.kind.clone(),
                                snapshot.ttl_ms,
                            ),
                        );
                    }
                    self.value_editor = match &snapshot.value {
                        redis::Value::BulkString(bytes) => crate::redis_commands::quote(bytes),
                        _ => String::new(),
                    };
                    self.preview = if snapshot.kind == "none" {
                        "Key no longer exists.".into()
                    } else {
                        String::new()
                    };
                    if snapshot.kind == "ReJSON-RL" {
                        self.string_mode = StringMode::Json;
                    }
                    self.snapshot = Some(snapshot);
                }
                Ok(Reply::Search(reply)) => self.search.receive(reply),
                Ok(Reply::Diagnostics(report)) => {
                    self.diagnostics_report = Some(report);
                    self.diagnostics_open = true;
                }
                #[cfg(not(target_arch = "wasm32"))]
                Ok(Reply::Bulk(report, connected)) => {
                    self.connected = connected;
                    self.bulk_cancel = None;
                    self.edit = None;
                    self.bulk_results = Some(report);
                    self.snapshot = None;
                    self.value_page = None;
                    self.value_editor.clear();
                    if connected { self.refresh(); }
                }
                Ok(Reply::Mutation(mutation, count)) => {
                    use crate::redis_mutation::Action;
                    self.edit = None;
                    let verb = match mutation.action {
                        Action::Create => "created",
                        Action::DeleteKeys => "deleted",
                        Action::Rename => "renamed",
                        _ => "updated",
                    };
                    self.notice = Some(format!("{count} key(s) {verb}."));
                    let focus = match mutation.action {
                        Action::Rename => Some(mutation.field.clone()),
                        Action::DeleteKeys | Action::ExpireKeys => None,
                        _ => mutation.keys.first().cloned(),
                    };
                    let rescan = matches!(
                        mutation.action,
                        Action::Create | Action::Rename | Action::DeleteKeys | Action::ExpireKeys
                    );
                    self.snapshot = None;
                    self.value_page = None;
                    self.value_editor.clear();
                    if rescan {
                        self.bulk_selected.clear();
                        self.refresh();
                    }
                    if let Some(key) = focus {
                        self.send(Job::Inspect(key));
                    }
                }
                Ok(Reply::Output(report)) => { self.output = report.text(); self.command_results = report; self.command_result_heights.clear(); },
                Ok(Reply::Changed(notice)) => {
                    if let Some(snapshot) = &self.snapshot {
                        let key = snapshot.key.clone();
                        self.snapshot = None;
                        self.send(Job::Inspect(key));
                    }
                    self.notice = Some(notice.into());
                }
            }
        }
    }
    /// Returns true when the user asks to open the connection manager.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        title: &str,
        color: egui::Color32,
        pane_sizes: &mut crate::pane_sizes::PaneSizes,
        preferences_dirty: &mut bool,
    ) -> bool {
        self.poll();
        if self.search.open { self.section = Section::Search; self.search.open = false; }
        #[cfg(not(target_arch = "wasm32"))]
        if self.pubsub.open { self.section = Section::PubSub; self.pubsub.open = false; }
        if self.diagnostics_open && matches!(self.diagnostics_request, Some(crate::redis_diagnostics::Request::Server | crate::redis_diagnostics::Request::Keys(_))) {
            self.section = Section::Analyze;
            self.analysis_server = matches!(self.diagnostics_request, Some(crate::redis_diagnostics::Request::Server));
            self.diagnostics_open = false;
        }
        let mut manager = false;
        egui::TopBottomPanel::top("redis-toolbar")
            .exact_height(44.0)
            .show(ctx, |ui| {
                browser_style(ui);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    crate::icons::toolbar_group(ui, 3, |ui| {
                        manager =
                            crate::icons::button(ui, "plug", crate::icons::TOOLBAR_BUTTON, false)
                                .on_hover_text("Connections")
                                .clicked();
                        if crate::icons::button(ui, "database", crate::icons::TOOLBAR_BUTTON, false)
                            .on_hover_text("Database selection")
                            .clicked()
                        {
                            self.database_picker = !self.database_picker;
                        }
                        if crate::icons::button(
                            ui,
                            "file-code",
                            crate::icons::TOOLBAR_BUTTON,
                            false,
                        )
                        .on_hover_text("Redis commands")
                        .clicked()
                        {
                            self.section = Section::Workbench;
                        }
                    });
                    let status_width = (ui.available_width()
                        - crate::icons::toolbar_group_width(4)
                        - ui.spacing().item_spacing.x)
                        .max(40.0);
                    let (rect, _) = ui
                        .allocate_exact_size(egui::vec2(status_width, 28.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        rect.left_center() + egui::vec2(8.0, 0.0),
                        4.0,
                        if self.connected {
                            egui::Color32::from_rgb(92, 195, 156)
                        } else {
                            color
                        },
                    );
                    ui.painter().with_clip_rect(rect.shrink(8.0)).text(
                        rect.left_center() + egui::vec2(22.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        format!("{title}   /   db{}", self.database),
                        egui::FontId::proportional(13.0),
                        ui.visuals().text_color(),
                    );
                    crate::icons::toolbar_group(ui, 4, |ui| {
                        if ui.add_enabled_ui(!self.busy && self.connected, |ui| {
                            crate::icons::button(ui, if self.read_only { "lock" } else { "lock-open" }, crate::icons::TOOLBAR_BUTTON, self.read_only)
                        }).inner.on_hover_text(if self.read_only {
                            "Read-only commands on. Click to allow writes."
                        } else {
                            "Writes allowed. Click to enable read-only commands."
                        }).clicked() {
                            self.read_only = !self.read_only;
                        }
                        if crate::icons::button(
                            ui,
                            "refresh-cw",
                            crate::icons::TOOLBAR_BUTTON,
                            false,
                        )
                        .on_hover_text("Refresh keys")
                        .clicked()
                            && (!self.busy || self.browse_busy)
                        {
                            self.refresh();
                        }
                        if crate::icons::button(
                            ui,
                            "panel-bottom",
                            crate::icons::TOOLBAR_BUTTON,
                            self.commands_visible,
                        )
                        .on_hover_text("Toggle commands")
                        .clicked()
                        {
                            self.commands_visible = !self.commands_visible;
                        }
                        if crate::icons::button(
                            ui,
                            "panel-right",
                            crate::icons::TOOLBAR_BUTTON,
                            self.inspector_visible,
                        )
                        .on_hover_text("Toggle row details")
                        .clicked()
                        {
                            self.inspector_visible = !self.inspector_visible;
                        }
                    });
                });
            });
        self.section_bar(ctx);
        egui::TopBottomPanel::bottom("redis-utilities")
            .frame(egui::Frame::new().fill(ctx.style().visuals.panel_fill).inner_margin(8))
            .show(ctx, |ui| {
                browser_style(ui);
                ui.horizontal(|ui| {
                    if ui.selectable_label(self.commands_visible && self.section == Section::Browse, "CLI").clicked() {
                        self.section = Section::Browse;
                        self.commands_visible = !self.commands_visible;
                    }
                    if ui.selectable_label(self.section == Section::Workbench, "Workbench").clicked() { self.section = Section::Workbench; }
                    #[cfg(not(target_arch = "wasm32"))]
                    if ui.button("Profiler").clicked() { self.monitor.open = true; }
                });
            });
        let mut inline_edit_result = None;
        if self.error.is_some() || self.notice.is_some() || (!self.connected && !self.busy) {
            egui::TopBottomPanel::top("redis-notices").show(ctx, |ui| {
                browser_style(ui);
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                if let Some(notice) = &self.notice {
                    ui.label(notice);
                }
                if !self.connected && !self.busy && ui.button("Reconnect").clicked() {
                    #[cfg(not(target_arch = "wasm32"))]
                    self.send(Job::Reconnect);
                    #[cfg(target_arch = "wasm32")]
                    self.send(Job::Database(self.database));
                }
            });
        }
        if self.database_picker {
            egui::Window::new("Select Redis database")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    browser_style(ui);
                    if self.cluster {ui.label("Redis Cluster uses database zero.");return;}
                    ui.horizontal(|ui| {
                        ui.label("Database index");
                        ui.add(egui::DragValue::new(&mut self.database_draft).range(0..=u32::MAX));
                        if ui
                            .add_enabled(!self.busy, egui::Button::new("Open database"))
                            .clicked()
                        {
                            self.send(Job::Database(self.database_draft));
                            self.database_picker = false;
                        }
                    });
                });
        }
        if self.section == Section::Browse {
            egui::TopBottomPanel::top("redis-browse-controls")
                .frame(egui::Frame::NONE.fill(ctx.style().visuals.panel_fill))
                .show(ctx, |ui| { browser_style(ui); self.browser_controls(ui); });
        if self.inspector_visible && (self.snapshot.is_some() || self.selected_key.is_some() || self.edit.as_ref().is_some_and(|e| e.action == crate::redis_mutation::Action::Create)) {
            let id = egui::Id::new((ctx.viewport_id(), "redis-row-details"));
            let width = (ctx.content_rect().width() * 0.5).max(320.0);
            crate::pane_sizes::restore(ctx, id, pane_sizes.inspector, width, 0);
            egui::SidePanel::right(id)
                .default_width(width)
                .width_range(300.0..=(ctx.content_rect().width() - 300.0).max(300.0))
                .frame(
                    egui::Frame::new()
                        .fill(ctx.style().visuals.panel_fill)
                        .inner_margin(16),
                )
                .show(ctx, |ui| {
                    browser_style(ui);
                    ui.set_min_width(ui.available_width());
                    egui::ScrollArea::vertical()
                        .id_salt("redis-inspector-scroll")
                        .show(ui, |ui| {
                            if let Some(edit) = &mut self.edit
                                && edit.action == crate::redis_mutation::Action::Create {
                                ui.heading("New key");
                                ui.add_space(16.0);
                                inline_edit_result = Some(edit.pane(ui, !self.busy && self.connected && !self.read_only, self.busy, self.cluster, true));
                            } else { self.inspector(ui); }
                        });
                });
            *preferences_dirty |=
                crate::pane_sizes::record_drag(ctx, id, &mut pane_sizes.inspector, 0);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(ctx.style().visuals.panel_fill))
            .show(ctx, |ui| {
                browser_style(ui);
                if self.commands_visible {
                    let commands_id = egui::Id::new((ctx.viewport_id(), "redis-command-pane"));
                    crate::pane_sizes::restore(
                        ctx,
                        commands_id,
                        pane_sizes.redis_commands,
                        220.0,
                        1,
                    );
                    egui::TopBottomPanel::bottom(commands_id)
                        .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill))
                        .height_range(140.0..=600.0)
                        .resizable(true)
                        .show_inside(ui, |ui| {
                            ui.set_min_height(ui.available_height());
                            self.command_pane(ui);
                        });
                    *preferences_dirty |= crate::pane_sizes::record_drag(
                        ctx,
                        commands_id,
                        &mut pane_sizes.redis_commands,
                        1,
                    );
                }
                egui::TopBottomPanel::bottom("redis-grid-footer")
                    .frame(
                        egui::Frame::new()
                            .fill(ui.visuals().panel_fill)
                            .inner_margin(6),
                    )
                    .show_inside(ui, |ui| {
                        ui.horizontal(|ui| {
                            if self.busy {
                                ui.spinner();
                            }
                            ui.label(format!("{} keys loaded", self.keys.len()));
                            let more = !self.busy
                                && self.connected
                                && (!self.scan_started || self.cursor != 0)
                                && !self.key_limit;
                            if ui
                                .add_enabled(more, egui::Button::new("Continue scan"))
                                .on_hover_text("Load more keys")
                                .clicked()
                            {
                                self.auto_scan = true;
                            }
                            if self.auto_scan && ui.button("Pause scan").clicked() {
                                self.auto_scan = false;
                            }
                            ui.weak(if self.key_limit {
                                "Partial"
                            } else if self.scan_started && self.cursor == 0 {
                                "Scan complete"
                            } else if self.auto_scan {
                                "Scanning…"
                            } else {
                                "Scan paused"
                            });
                            if self.key_limit {
                                ui.weak("Limit reached; narrow the filter.");
                            }
                        });
                    });
                self.browser_list_toolbar(ui);
                ui.scope(|ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(ui.available_rect_before_wrap()));
                    if self.tree_view {
                        self.key_tree(ui);
                    } else {
                        self.key_grid(ui);
                    }
                });
            });
        } else {
            if self.section == Section::Workbench {
                egui::SidePanel::left("redis-workbench-history").default_width(230.0).width_range(180.0..=400.0)
                    .show(ctx, |ui| {
                        browser_style(ui);
                        ui.add_space(8.0); ui.strong("Query history");
                        ui.weak(if self.history_profile.is_some() { "Saved connection · last 100 commands" } else { "This session · last 100 commands" });
                        if ui.checkbox(&mut self.history_recording, "Record history").on_hover_text("Applies to all Redis connections; existing history remains until cleared").changed() { self.history_dirty = true; }
                        ui.weak("Command text may contain sensitive values.");
                        if ui.add_enabled(!self.command_history.is_empty(), action_button("trash-2", "Clear history")).clicked() {
                            self.command_history.clear(); self.history_dirty = true;
                        }
                        ui.separator();
                        egui::ScrollArea::vertical().show_rows(ui, 44.0, self.command_history.len(), |ui, range| {
                            for i in range {
                                let command = &self.command_history[self.command_history.len() - 1 - i];
                                let preview: String = command.lines().next().unwrap_or_default().chars().take(100).collect();
                                if ui.add_sized([ui.available_width(), 40.0], egui::Button::new(egui::RichText::new(preview).monospace()).frame(false).truncate()).clicked() { self.command = command.clone(); self.command_editor.focus(); }
                            }
                        });
                        if self.command_history.is_empty() { ui.weak("Executed commands appear here."); }
                    });
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(ctx.style().visuals.panel_fill))
                .show(ctx, |ui| {
                    browser_style(ui);
                    match self.section {
                        Section::Workbench => self.command_pane(ui),
                        Section::Search => {
                            egui::Frame::new().inner_margin(16).show(ui, |ui| {
                                egui::ScrollArea::vertical().id_salt("redis-search-page").show(ui, |ui| {
                                    self.search.writable = !self.read_only; if let Some(action) = self.search.pane(ui, self.connected && !self.busy && self.edit.is_none() && !self.value_dirty()) { self.search_action(action); }
                                });
                            });
                        }
                        Section::Analyze => self.analysis_pane(ui),
                        Section::PubSub => {
                            #[cfg(not(target_arch = "wasm32"))]
                            egui::Frame::new().inner_margin(16).show(ui, |ui| {
                                if let Some(request) = self.pubsub.pane(ui, self.connected, self.connected && !self.busy && !self.value_dirty(), self.read_only) { self.send(Job::Publish(request, self.read_only)); }
                            });
                            #[cfg(target_arch = "wasm32")]
                            ui.label("Pub/Sub is available in the native app.");
                        }
                        Section::Browse => {}
                    }
                });
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.monitor.show(ctx, self.connected);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(action) = self.archive.show(
            ctx,
            title,
            self.database,
            self.connected && !self.busy,
            self.read_only,
        ) {
            self.send(Job::Archive(action));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.section != Section::PubSub && let Some(request) = self.pubsub.show(
            ctx,
            self.connected,
            self.connected && !self.busy,
            self.read_only,
        ) {
            self.send(Job::Publish(request, self.read_only));
        }
        if self.section != Section::Search && let Some(action) = self.search.show(ctx, self.connected && !self.busy && self.edit.is_none()) { self.search_action(action); }
        if self.diagnostics_open {
            let mut open = true;
            let mut refresh = false;
            let mut followup = None;
            egui::Window::new(if matches!(self.diagnostics_request, Some(crate::redis_diagnostics::Request::Cluster)) { "Redis cluster topology" } else if matches!(self.diagnostics_request, Some(crate::redis_diagnostics::Request::TimeSeries(_))) { "Time-series metadata" } else if matches!(self.diagnostics_request, Some(crate::redis_diagnostics::Request::VectorElement { .. })) { "Vector element" } else if matches!(self.diagnostics_request, Some(crate::redis_diagnostics::Request::VectorSimilar { .. })) { "Similar elements" } else { "Redis diagnostics" })
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .frame(
                    egui::Frame::window(&ctx.style())
                        .inner_margin(16)
                        .corner_radius(10.0),
                )
                .collapsible(false)
                .open(&mut open)
                .default_width(if matches!(self.diagnostics_request, Some(crate::redis_diagnostics::Request::Cluster)) { 800.0 } else { 640.0 })
                .resizable(true)
                .show(ctx, |ui| {
                    browser_style(ui);
                    refresh = ui
                        .add_enabled(!self.busy, action_button("refresh-cw", "Refresh"))
                        .clicked();
                    if let Some(report) = &self.diagnostics_report {
                        followup = ui.add_enabled_ui(!self.busy, |ui| report.show(ui)).inner;
                    } else if self.busy {
                        ui.spinner();
                    }
                    if let Some(error) = &self.error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                });
            self.diagnostics_open = open;
            if let Some(request) = followup {
                self.diagnostics_request = Some(request.clone());
                self.diagnostics_report = None;
                self.send(Job::Diagnostics(request));
            } else if refresh && let Some(request) = self.diagnostics_request.clone() {
                self.send(Job::Diagnostics(request));
            }
        }
        if let Some(edit) = &mut self.edit {
            let (request, cancel) = if let Some(result) = inline_edit_result {
                result
            } else if edit.action == crate::redis_mutation::Action::Create || (edit.action == crate::redis_mutation::Action::Update && edit.kind == "ReJSON-RL") {
                (None, false)
            } else { edit.show(ctx, !self.busy && self.connected && !self.read_only, self.busy, self.cluster) };
            if cancel {
                self.edit = None;
            }
            if let Some(request) = request {
                #[cfg(not(target_arch = "wasm32"))]
                if self.cluster && matches!(request.action, crate::redis_mutation::Action::DeleteKeys | crate::redis_mutation::Action::ExpireKeys)
                    && crate::redis_cluster::crosses_slots(&request.keys) {
                    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    self.bulk_cancel = Some(cancel.clone());
                    self.bulk_results = None;
                    self.send(Job::Bulk(request, self.read_only, cancel));
                } else { self.send(Job::Mutate(request, self.read_only)); }
                #[cfg(target_arch = "wasm32")]
                self.send(Job::Mutate(request, self.read_only));
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(cancel) = &self.bulk_cancel {
                if self.busy {
                    egui::Window::new("Applying bulk changes").collapsible(false).show(ctx, |ui| {
                        crate::redis_workspace::browser_style(ui);
                        ui.label("Finishing submitted keys; results will show each outcome.");
                        let stopping = cancel.load(std::sync::atomic::Ordering::Acquire);
                        if ui.add_enabled(!stopping, egui::Button::new(if stopping { "Stopping after current batch…" } else { "Stop after current batch" })).clicked() {
                            cancel.store(true, std::sync::atomic::Ordering::Release);
                        }
                    });
                } else { self.bulk_cancel = None; }
            }
            if let Some(view) = &mut self.bulk_results && !view.show(ctx) { self.bulk_results = None; }
        }
        if self.delete_confirmation {
            let mut delete = false;
            let modal =
                egui::Modal::new(egui::Id::new("redis-delete-confirmation")).show(ctx, |ui| {
                    ui.heading("Delete this key?");
                    if let Some(snapshot) = &self.snapshot {
                        ui.monospace(display_key(&snapshot.key));
                    }
                    ui.label("This deletes the entire value from Redis.");
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.delete_confirmation = false;
                        }
                        delete = ui
                            .add_enabled(
                                !self.busy && !self.read_only,
                                egui::Button::new("Delete key"),
                            )
                            .clicked();
                    });
                });
            if modal.should_close() {
                self.delete_confirmation = false;
            }
            if delete {
                self.delete_confirmation = false;
                if let Some(snapshot) = &self.snapshot {
                    self.send(Job::Delete(snapshot.key.clone(), self.read_only));
                }
            }
        }
        if self.busy() && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.error = Some(
                "Finish the Redis operation and save or discard value edits before closing.".into(),
            );
        }
        // One bounded job at a time; user selection takes priority over discovery.
        if !self.busy
            && self.connected
            && !self.value_dirty()
            && self.edit.is_none()
            && self.section == Section::Browse
        {
            if let Some(job) = self.pending.take() {
                self.send(job);
            } else if self.error.is_none() {
                // Retain the existing 200-key pipeline budget. Publishing names
                // separately must not turn scrolling into extra network waits.
                let count = self.preview_queue.len().min(200);
                if count != 0 {
                    let keys = self.preview_queue.drain(..count).collect();
                    self.send(Job::Rows(keys));
                } else if self.auto_scan
                    && !self.key_limit
                    && (!self.scan_started || self.cursor != 0)
                {
                    self.send(Job::Scan(
                        self.cursor,
                        self.scan_pattern.clone(),
                        self.scan_kind.clone(),
                    ));
                }
            }
        }
        manager
    }
    fn browser_controls(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().inner_margin(10).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("redis-search-mode")
                            .width(100.0)
                            .selected_text(if self.exact_mode {
                                "Exact key"
                            } else {
                                "Key pattern"
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.exact_mode, false, "Key pattern");
                                ui.selectable_value(&mut self.exact_mode, true, "Exact key");
                            });
                        let width = (ui.available_width()
                            - if self.exact_mode { 86.0 } else { 210.0 })
                        .max(80.0);
                        let input = ui.add_sized(
                            [width, crate::controls::BUTTON_HEIGHT],
                            egui::TextEdit::singleline(if self.exact_mode {
                                &mut self.exact_key
                            } else {
                                &mut self.pattern
                            })
                            .hint_text(if self.exact_mode {
                                "Key name or quoted bytes"
                            } else {
                                "Search keys, e.g. app:users:*"
                            })
                            .margin(egui::vec2(6.0, 4.0)),
                        );
                        if !self.exact_mode {
                            egui::ComboBox::from_id_salt("redis-type")
                                .width(110.0)
                                .selected_text(if self.kind.is_empty() {
                                    "All types"
                                } else {
                                    type_label(&self.kind)
                                })
                                .show_ui(ui, |ui| {
                                    for kind in [
                                        "",
                                        "string",
                                        "hash",
                                        "list",
                                        "set",
                                        "zset",
                                        "stream",
                                        "ReJSON-RL",
                                        "TSDB-TYPE",
                                        "vectorset",
                                    ] {
                                        ui.selectable_value(
                                            &mut self.kind,
                                            kind.to_owned(),
                                            if kind.is_empty() {
                                                "All types"
                                            } else {
                                                type_label(kind)
                                            },
                                        );
                                    }
                                });
                        }
                        let run = ui
                            .add_enabled(
                                self.connected && (!self.busy || self.browse_busy),
                                action_button(
                                    "search",
                                    if self.exact_mode {
                                        "Open key"
                                    } else {
                                        "Search"
                                    },
                                )
                                .min_size(egui::vec2(70.0, crate::controls::BUTTON_HEIGHT)),
                            )
                            .clicked()
                            || (input.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                        if run {
                            input.request_focus();
                            if self.exact_mode {
                                match exact_key_bytes(&self.exact_key) {
                                    Ok(key) => {
                                        self.inspector_visible = true;
                                        self.send(Job::Inspect(key));
                                    }
                                    Err(error) => self.error = Some(error),
                                }
                            } else {
                                self.refresh();
                            }
                        }
                    });
        });
    }
    fn browser_list_toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().inner_margin(8).show(ui, |ui| {
                    ui.horizontal(|ui| {

                        ui.weak(format!("{} keys loaded", self.keys.len()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            crate::controls::segments(ui, "redis-browser-view", &mut self.tree_view,
                                &[(false, "List", 52.0), (true, "Tree", 56.0)]);
                            ui.menu_button("Actions", |ui| {
                                ui.menu_button("Tree settings", |ui| {
                                    ui.label("Namespace separator");
                                    if ui.add(egui::TextEdit::singleline(&mut self.separator).desired_width(60.0).char_limit(8)).changed() { self.rebuild_tree(); }
                                    if ui.button("Collapse all").clicked() { self.expanded.clear(); self.rebuild_tree(); }
                                });
                                if ui
                                    .add_enabled(
                                        self.connected && !self.busy,
                                        egui::Button::new("Search indexes…"),
                                    )
                                    .clicked()
                                {
                                    self.section = Section::Search;
                                    self.search.open = false;
                                    self.search.begin();
                                    self.send(Job::Search(crate::redis_search::Request::Indexes));
                                    ui.close();
                                }
                                ui.separator();
                                #[cfg(not(target_arch = "wasm32"))]
                                if ui.button("Live command profiler…").clicked() {
                                    self.monitor.open = true;
                                    ui.close();
                                }
                                #[cfg(not(target_arch = "wasm32"))]
                                if ui.button("Pub/Sub…").clicked() { self.section = Section::PubSub; self.pubsub.open = false; ui.close(); }
                                #[cfg(not(target_arch = "wasm32"))]
                                {
                                    if ui.button("Import / export…").clicked() { self.archive.open = true; ui.close(); }
                                    let selected = self.bulk_mode && !self.bulk_selected.is_empty();
                                    let count = if selected { self.bulk_selected.len() } else { self.keys.len() };
                                    if ui.add_enabled(self.connected && !self.busy && self.archive.can_start() && (1..=1000).contains(&count), egui::Button::new(if selected { "Export selected keys…" } else { "Export loaded keys…" })).on_hover_text("Export up to 1,000 keys. Narrow the filter or select a smaller set.").clicked() {
                                        let keys = if selected { self.bulk_selected.iter().cloned().collect() } else { self.keys.clone() };
                                        let action = self.archive.export(keys,self.database); self.send(Job::Archive(action)); ui.close();
                                    }
                                }
                                if ui.add_enabled(self.connected && !self.busy, egui::Button::new("Cluster topology…")).clicked() {
                                    let request = crate::redis_diagnostics::Request::Cluster;
                                    self.diagnostics_request = Some(request.clone());
                                    self.diagnostics_report = None;
                                    self.diagnostics_open = true;
                                    self.send(Job::Diagnostics(request));
                                    ui.close();
                                }
                                for (label, analysis) in [
                                    ("Server & slow log…", false),
                                    ("Analyze loaded keys…", true),
                                ] {
                                    if ui
                                        .add_enabled(
                                            !self.busy
                                                && self.connected
                                                && (!analysis || !self.keys.is_empty()),
                                            egui::Button::new(label),
                                        )
                                        .clicked()
                                    {
                                        let request = if analysis {
                                            crate::redis_diagnostics::Request::Keys(
                                                self.keys.iter().take(1000).cloned().collect(),
                                            )
                                        } else {
                                            crate::redis_diagnostics::Request::Server
                                        };
                                        self.diagnostics_request = Some(request.clone());
                                        self.diagnostics_report = None;
                                        self.section = Section::Analyze;
                                        self.analysis_server = !analysis;
                                        self.diagnostics_open = false;
                                        self.send(Job::Diagnostics(request));
                                        ui.close();
                                    }
                                }
                                ui.separator();
                                if ui
                                    .add_enabled(
                                        !self.busy && !self.read_only && self.edit.is_none() && !self.value_dirty(),
                                        egui::Button::new("Create key…"),
                                    )
                                    .clicked()
                                {
                                    self.edit = Some(crate::redis_edit::Draft::new(
                                        crate::redis_mutation::Action::Create,
                                        vec![],
                                        "string".into(),
                                    ));
                                    ui.close();
                                }
                                ui.separator();
                                ui.checkbox(&mut self.bulk_mode, "Select keys for bulk actions");
                                ui.weak("Up to 1,000 explicit, loaded key names");
                                if ui.button("Select first 1,000 loaded keys").clicked() {
                                    self.bulk_mode = true;
                                    self.bulk_selected =
                                        self.keys.iter().take(1000).cloned().collect();
                                    ui.close();
                                }
                                if ui.button("Clear selection").clicked() {
                                    self.bulk_selected.clear();
                                    ui.close();
                                }
                                for (label, action) in [
                                    (
                                        "Delete selected…",
                                        crate::redis_mutation::Action::DeleteKeys,
                                    ),
                                    (
                                        "Change selected expiry…",
                                        crate::redis_mutation::Action::ExpireKeys,
                                    ),
                                ] {
                                    if ui
                                        .add_enabled(
                                            !self.busy
                                                && !self.read_only
                                                && self.edit.is_none() && !self.value_dirty()
                                                && !self.bulk_selected.is_empty(),
                                            egui::Button::new(label),
                                        )
                                        .clicked()
                                    {
                                        self.edit = Some(crate::redis_edit::Draft::new(
                                            action,
                                            self.bulk_selected.iter().cloned().collect(),
                                            String::new(),
                                        ));
                                        ui.close();
                                    }
                                }
                            });
                            if self.bulk_mode {
                                ui.weak(format!("{} selected", self.bulk_selected.len()));
                            }
                        });
                    });
        });
    }
    fn inspector(&mut self, ui: &mut egui::Ui) {
        #[cfg(test)]
        if self.value_page.is_none()
            && let Some(snapshot) = &self.snapshot
        {
            let cursor = Cursor::first(&snapshot.kind, b"*");
            let display = crate::redis_value::Display::new(snapshot, &cursor);
            let kind = snapshot.kind.clone();
            self.value_page = Some(Page {
                cursor,
                next: None,
                pattern: b"*".to_vec(),
                display,
            });
            self.update_value_matches(&kind);
        }
        let mut job = None;
        let mut close = false;
        let can_change = !self.busy && !self.value_dirty() && self.edit.is_none();
        if let Some(snapshot) = &self.snapshot {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);
            ui.horizontal_wrapped(|ui| {
                type_badge(ui, &snapshot.kind);
                ui.add(egui::Label::new(egui::RichText::new(display_key(&snapshot.key)).monospace().size(15.0)).wrap());
                if crate::redis_value_editor::icon(ui, "pencil", "Rename key", can_change && !self.read_only).clicked() {
                    self.edit = Some(crate::redis_edit::Draft::new(crate::redis_mutation::Action::Rename, vec![snapshot.key.clone()], snapshot.kind.clone()));
                }
                crate::redis_value_editor::copy(ui, "key", None, true, || display_key(&snapshot.key));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = crate::redis_value_editor::icon(ui, "x", "Close key (save or discard edits first)", can_change).clicked();
                });
            });
            ui.horizontal_wrapped(|ui| {
                ui.weak(if matches!(snapshot.kind.as_str(), "string" | "ReJSON-RL") { "Bytes" } else { "Length" });
                ui.label(snapshot.length.map_or("—".into(), |n| n.to_string()));
                ui.separator();
                ui.weak("TTL");
                ui.label(ttl_label(snapshot.ttl_ms)).on_hover_text("Expiry at the last refresh");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::redis_value_editor::icon(ui, "trash-2", "Delete key", can_change && !self.read_only).clicked() { self.delete_confirmation = true; }
                    if crate::redis_value_editor::icon(ui, "refresh-cw", "Refresh key", can_change).clicked() { job = Some(Job::Inspect(snapshot.key.clone())); }
                });
            });
            ui.separator();
        }
        if close {
            self.inspector_visible = false;
            self.selected_key = None;
            self.snapshot = None;
            self.value_page = None;
            return;
        }
        if self.snapshot.is_some() {
            self.value_inspector(ui, &mut job);
        }
        if let Some(snapshot) = &self.snapshot {
            ui.add_space(8.0);
            ui.separator();
            egui::CollapsingHeader::new("Expiry").show(ui, |ui| {
                ui.add_enabled_ui(
                    !self.busy && !self.read_only && snapshot.kind != "none",
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("TTL seconds");
                            ui.add(
                                egui::DragValue::new(&mut self.ttl_seconds)
                                    .range(1..=u64::MAX / 1000),
                            );
                            if ui.button("Set TTL").clicked() {
                                job = Some(Job::Expire(
                                    snapshot.key.clone(),
                                    Some(self.ttl_seconds * 1000),
                                    self.read_only,
                                ));
                            }
                            if ui.button("Remove expiry").clicked() {
                                job = Some(Job::Expire(snapshot.key.clone(), None, self.read_only));
                            }
                        });
                    },
                );
            });
        } else {
            ui.centered_and_justified(|ui| {
                ui.weak("Select a key to inspect its value and expiry.");
            });
        }
        if let Some(job) = job {
            self.send(job);
        }
    }
    fn update_value_matches(&mut self, kind: &str) {
        if let Some(page) = &self.value_page {
            let needle = self.value_search.to_lowercase();
            self.value_matches = page
                .display
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| {
                    matches!(kind, "hash" | "set" | "zset")
                        || needle.is_empty()
                        || row.search.contains(&needle)
                })
                .map(|(i, _)| i)
                .collect();
        }
    }
    fn value_inspector(&mut self, ui: &mut egui::Ui, job: &mut Option<Job>) {
        let dirty = self.value_dirty();
        let can_page = self.connected && !self.busy && !dirty;
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let Some(page) = &self.value_page else {
            ui.weak("Loading value…");
            return;
        };
        let collection = matches!(
            snapshot.kind.as_str(),
            "hash" | "list" | "set" | "zset" | "stream" | "TSDB-TYPE" | "vectorset"
        );
        let remote_search = matches!(snapshot.kind.as_str(), "hash" | "set" | "zset");
        let mut local_changed = false;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Value").size(15.0).strong());
            if self.read_only {
                ui.weak("Read-only");
            }
        });
        if snapshot.kind == "vectorset" { ui.weak("Elements in name order").on_hover_text("Binary names are ordered by their bytes. Requires Redis 8.4+ and permission to run EVAL_RO and VRANGE."); }
        if snapshot.kind == "TSDB-TYPE" {
            if ui.add_enabled(can_page, action_button("database", "Series info…")).clicked() {
                let request = crate::redis_diagnostics::Request::TimeSeries(snapshot.key.clone());
                self.diagnostics_request = Some(request.clone());
                self.diagnostics_report = None;
                self.diagnostics_open = true;
                *job = Some(Job::Diagnostics(request));
            }
            ui.weak("Time range · Unix milliseconds · blank means unbounded");
            ui.horizontal_wrapped(|ui| {
                for (label, value) in ["From", "Through"].into_iter().zip(&mut self.time_range) {
                    ui.vertical(|ui| {
                        ui.set_width(138.0);
                        ui.weak(label);
                        ui.add(egui::TextEdit::singleline(value).hint_text("Unbounded").desired_width(138.0));
                    });
                }
                if ui.add_enabled(can_page, action_button("search", "Apply range")).clicked() {
                    match crate::redis_value::timestamp_range(&self.time_range[0], &self.time_range[1]) {
                        Ok(cursor) => *job = Some(Job::ValuePage(PageRequest { key: snapshot.key.clone(), kind: snapshot.kind.clone(), cursor, pattern: b"*".to_vec() }, Navigation::Search)),
                        Err(error) => self.error = Some(error),
                    }
                }
            });
            let applied = self.value_history.first().unwrap_or(&page.cursor);
            if crate::redis_value::timestamp_range(&self.time_range[0], &self.time_range[1]).as_ref().ok() != Some(applied) {
                ui.weak("Range edits are not applied yet");
            }
        }
        if collection && !self.read_only && !matches!(snapshot.kind.as_str(), "TSDB-TYPE" | "vectorset") {
            ui.horizontal(|ui| {
                for (label, action) in [
                    ("Add…", crate::redis_mutation::Action::Add),
                    ("Edit…", crate::redis_mutation::Action::Update),
                    ("Remove…", crate::redis_mutation::Action::Remove),
                ] {
                    let supported = action == crate::redis_mutation::Action::Add
                        || (self.selected_value.is_some()
                            && !snapshot.truncated
                            && !(snapshot.kind == "stream"
                                && action == crate::redis_mutation::Action::Update));
                    if ui
                        .add_enabled(
                            can_page && !self.read_only && supported,
                            action_button(
                                match action {
                                    crate::redis_mutation::Action::Add => "plus",
                                    crate::redis_mutation::Action::Remove => "x",
                                    _ => "file-code",
                                },
                                label,
                            )
                            .min_size(egui::vec2(72.0, crate::controls::BUTTON_HEIGHT)),
                        )
                        .clicked()
                    {
                        self.edit = collection_draft(snapshot, page, self.selected_value, action);
                    }
                }
            });
        }
        if snapshot.truncated {
            ui.label(
                egui::RichText::new(if snapshot.kind == "ReJSON-RL" {
                    "Partial JSON document · read-only preview"
                } else if snapshot.kind == "string" {
                    "Partial value · read-only byte range"
                } else {
                    "Response limit reached. Some values are shortened; narrow the search."
                })
                .color(ui.visuals().warn_fg_color),
            );
        }
        if matches!(snapshot.kind.as_str(), "string" | "ReJSON-RL") {
            if snapshot.kind == "ReJSON-RL"
                && self.edit.is_none()
                && !snapshot.truncated
                && ui
                    .add_enabled(
                        can_page && !self.read_only,
                        action_button("pencil", "Edit JSON"),
                    )
                    .clicked()
                && let redis::Value::BulkString(bytes) = &snapshot.value
            {
                let mut draft = crate::redis_edit::Draft::new(
                    crate::redis_mutation::Action::Update,
                    vec![snapshot.key.clone()],
                    snapshot.kind.clone(),
                );
                draft.value = page
                    .display
                    .strings
                    .json_source
                    .as_ref()
                    .filter(|source| source.len() <= 65536)
                    .cloned()
                    .unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned());
                draft.expected = bytes.clone();
                self.edit = Some(draft);
            }
            if let Some(draft) = &mut self.edit
                && draft.action == crate::redis_mutation::Action::Update && draft.kind == "ReJSON-RL" {
                let mut layouter = crate::redis_value_editor::layout;
                ui.add_enabled(!self.busy && !self.read_only, egui::TextEdit::multiline(&mut draft.value)
                    .code_editor().layouter(&mut layouter).desired_rows(14).desired_width(f32::INFINITY));
                if let Some(error) = &draft.error { ui.colored_label(ui.visuals().error_fg_color, error); }
                let mut cancel = false;
                ui.horizontal(|ui| {
                    ui.weak("JSON document · Save preserves expiry");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::redis_value_editor::icon(ui, "check", "Save JSON document", self.connected && !self.busy && !self.read_only).clicked() {
                            match draft.request() { Ok(request) => *job = Some(Job::Mutate(request, self.read_only)), Err(error) => draft.error = Some(error) }
                        }
                        cancel = crate::redis_value_editor::icon(ui, "x", "Discard JSON edits", !self.busy).clicked();
                    });
                });
                if cancel { self.edit = None; }
                return;
            }
            ui.add_enabled_ui(!dirty, |ui| {
                let formats = [
                    (StringMode::Text, "Text", 48.0),
                    (StringMode::Json, "JSON", 52.0),
                    (StringMode::Hex, "Hex", 44.0),
                    (StringMode::Edit, "Edit", 44.0),
                ];
                crate::controls::segments(
                    ui,
                    "redis-value-format",
                    &mut self.string_mode,
                    if !snapshot.truncated && snapshot.kind == "string" {
                        &formats[..]
                    } else {
                        &formats[..3]
                    },
                );
            });
            if self.string_mode == StringMode::Edit && !snapshot.truncated {
                ui.add_enabled(
                    !self.busy && !self.read_only,
                    egui::TextEdit::multiline(&mut self.value_editor)
                        .code_editor()
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(3.0);
                ui.weak("Quoted bytes · Save preserves expiry");
                if ui.button("Discard edits").clicked()
                    && let redis::Value::BulkString(bytes) = &snapshot.value
                {
                    self.value_editor = crate::redis_commands::quote(bytes);
                }
                if ui
                    .add_enabled(
                        !self.busy && !self.read_only,
                        action_button("check", "Save value"),
                    )
                    .clicked()
                {
                    match crate::redis_commands::parse(&self.value_editor) {
                        Ok(args) if args.len() == 1 && args[0].len() == 1 => {
                            if let redis::Value::BulkString(original) = &snapshot.value {
                                *job = Some(Job::Save(
                                    snapshot.key.clone(),
                                    original.clone(),
                                    args[0][0].clone(),
                                    self.read_only,
                                ));
                            }
                        }
                        _ => {
                            self.error = Some(
                                "Enter exactly one quoted value, such as \"hello\" or \"\\xff\"."
                                    .into(),
                            )
                        }
                    }
                }
            } else {
                let views = &page.display.strings;
                let lines = match self.string_mode {
                    StringMode::Json => &views.json,
                    StringMode::Hex => &views.hex,
                    _ => &views.text,
                };
                if self.string_mode == StringMode::Json
                    && let Some(error) = &views.json_error
                {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
                if self.string_mode == StringMode::Text && views.escaped {
                    ui.weak("Binary/control bytes are escaped. Hex shows the exact bytes.");
                }
                value_lines_styled(
                    ui,
                    "redis-string-view",
                    lines,
                    340.0,
                    (self.string_mode == StringMode::Json).then_some(&views.json_tokens),
                );
                crate::redis_value_editor::copy(ui, "value", Some("Copy value"), self.string_mode != StringMode::Json || views.json_source.is_some(), || match self.string_mode {
                    StringMode::Json => views.json_source.clone().unwrap_or_default(),
                    StringMode::Hex => views.hex.join("\n"),
                    _ => views.text_source.clone(),
                });
                if let redis::Value::BulkString(bytes) = &snapshot.value {
                    if bytes.is_empty() {
                        ui.weak("Empty string");
                    } else {
                        ui.weak(format!(
                            "Bytes {}–{} of {}",
                            page.cursor.offset(),
                            page.cursor.offset() + bytes.len().saturating_sub(1) as u64,
                            snapshot.length.unwrap_or(0)
                        ));
                    }
                }
            }
        } else if collection {
            ui.horizontal(|ui| {
                let input = ui.add(
                    egui::TextEdit::singleline(&mut self.value_search)
                        .margin(egui::vec2(6.0, 4.0))
                        .char_limit(1024)
                        .hint_text(if remote_search {
                            if snapshot.kind == "hash" {
                                "Match fields, e.g. user:*"
                            } else {
                                "Match members, e.g. user:*"
                            }
                        } else {
                            "Filter elements on this page"
                        })
                        .desired_width(
                            (ui.available_width() - if remote_search { 115.0 } else { 20.0 })
                                .max(80.0),
                        ),
                );
                local_changed = input.changed() && !remote_search;
                if remote_search
                    && (ui
                        .add_enabled(!self.busy, action_button("search", "Search"))
                        .clicked()
                        || (input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))))
                {
                    let pattern = match collection_pattern(&self.value_search) {
                        Ok(pattern) => pattern,
                        Err(error) => {
                            self.error = Some(error);
                            return;
                        }
                    };
                    *job = Some(Job::ValuePage(
                        PageRequest {
                            key: snapshot.key.clone(),
                            kind: snapshot.kind.clone(),
                            cursor: Cursor::first(&snapshot.kind, &pattern),
                            pattern,
                        },
                        Navigation::Search,
                    ));
                }
            });
            if remote_search {
                ui.weak(format!("Matching {}", display_key(&page.pattern)));
                if snapshot.kind == "zset" {
                    ui.weak(if page.pattern == b"*" {
                        "Score order · ascending"
                    } else {
                        "Scan order · not sorted by score"
                    });
                }
            }
            if local_changed {
                let needle = self.value_search.to_lowercase();
                self.value_matches = page
                    .display
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| needle.is_empty() || row.search.contains(&needle))
                    .map(|(i, _)| i)
                    .collect();
                self.selected_value = None;
            }
            if snapshot.kind == "TSDB-TYPE" {
                let was_chart = self.time_chart;
                crate::controls::segments(ui, "timeseries-view", &mut self.time_chart, &[(false, "Table", 64.0), (true, "Chart", 64.0)]);
                if self.time_chart {
                    page.display.timeseries_chart.show(ui, &self.value_matches, !was_chart || std::mem::take(&mut self.time_chart_focus));
                }
            }
            let headers = &page.display.headers;
            if !headers.is_empty() && !(snapshot.kind == "TSDB-TYPE" && self.time_chart) {
                egui::Frame::new()
                    .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                    .corner_radius(6.0)
                    .inner_margin(6)
                    .show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 2.0);
                use egui_extras::{Column, TableBuilder};
                let height = (ui.available_height() - 120.0).clamp(120.0, 600.0);
                let mut table = TableBuilder::new(ui)
                    .id_salt(("redis-values", &snapshot.key, &page.pattern))
                    .max_scroll_height(height)
                    .min_scrolled_height(0.0)
                    .striped(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .sense(egui::Sense::click());
                for index in 0..headers.len() {
                    table = table.column(if snapshot.kind == "zset" && index == 0 {
                        Column::remainder().clip(true)
                    } else if snapshot.kind == "zset" {
                        Column::initial(100.0).at_least(70.0).clip(true)
                    } else if index + 1 == headers.len() {
                        Column::remainder().clip(true)
                    } else {
                        Column::initial(if snapshot.kind == "list" { 60.0 } else { 120.0 })
                            .at_least(50.0)
                            .clip(true)
                    });
                }
                table
                    .header(28.0, |mut header| {
                        for name in headers {
                            header.col(|ui| {
                                ui.weak(*name);
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(22.0, self.value_matches.len(), |mut row| {
                            let index = self.value_matches[row.index()];
                            row.set_selected(self.selected_value == Some(index));
                            for cell in &page.display.rows[index].cells {
                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(cell).monospace())
                                            .truncate(),
                                    );
                                });
                            }
                            if row.response().clicked() {
                                self.selected_value = Some(index);
                            }
                        });
                    });
                });
            }
            ui.weak(format!(
                "{} shown on this page · {} total in key",
                self.value_matches.len(),
                snapshot.length.unwrap_or(0)
            ));
            if self.value_matches.is_empty() {
                ui.label(if page.next.is_some() {
                    "No matches in this batch. Continue with Next."
                } else {
                    "No matching items on this page."
                });
            }
            if let Some(index) = self.selected_value
                && let Some(row) = page.display.rows.get(index)
            {
                ui.add_space(6.0);
                ui.strong("Selected item");
                if snapshot.kind == "vectorset" {
                    ui.horizontal_wrapped(|ui| {
                        for (similar, label) in [(false, "Inspect element…"), (true, "Find similar…")] {
                            if ui.add_enabled(can_page, action_button("search", label)).clicked()
                                && let redis::Value::Array(values) = &snapshot.value
                                && let Some(redis::Value::BulkString(element)) = values.get(index)
                            {
                                let request = if similar {
                                    crate::redis_diagnostics::Request::VectorSimilar { key: snapshot.key.clone(), element: element.clone() }
                                } else {
                                    crate::redis_diagnostics::Request::VectorElement { key: snapshot.key.clone(), element: element.clone() }
                                };
                                self.diagnostics_request = Some(request.clone());
                                self.diagnostics_report = None;
                                self.diagnostics_open = true;
                                *job = Some(Job::Diagnostics(request));
                            }
                        }
                    });
                }
                value_lines(ui, "redis-selected-value", &row.detail, 180.0);
                crate::redis_value_editor::copy(ui, "item", Some("Copy item"), true, || row.detail.join("\n"));
            }
        } else {
            ui.monospace(&self.preview);
        }
        if collection
            || (snapshot.kind == "string"
                && (page.next.is_some() || !self.value_history.is_empty()))
        {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        can_page && !self.value_history.is_empty(),
                        action_button("chevron-left", "Previous")
                            .min_size(egui::vec2(84.0, crate::controls::BUTTON_HEIGHT)),
                    )
                    .clicked()
                    && let Some(cursor) = self.value_history.last()
                {
                    *job = Some(Job::ValuePage(
                        PageRequest {
                            key: snapshot.key.clone(),
                            kind: snapshot.kind.clone(),
                            cursor: cursor.clone(),
                            pattern: page.pattern.clone(),
                        },
                        Navigation::Previous,
                    ));
                }
                if ui
                    .add_enabled(
                        can_page && page.next.is_some(),
                        action_button("chevron-right", "Next")
                            .min_size(egui::vec2(84.0, crate::controls::BUTTON_HEIGHT)),
                    )
                    .clicked()
                    && let Some(cursor) = &page.next
                {
                    *job = Some(Job::ValuePage(
                        PageRequest {
                            key: snapshot.key.clone(),
                            kind: snapshot.kind.clone(),
                            cursor: cursor.clone(),
                            pattern: page.pattern.clone(),
                        },
                        Navigation::Next,
                    ));
                }
                if page.next.is_none() {
                    ui.weak(if snapshot.truncated && snapshot.kind != "string" {
                        "Limited preview"
                    } else {
                        "End of results"
                    });
                }
            });
        }
    }
    fn command_pane(&mut self, ui: &mut egui::Ui) {
        if self.cluster {ui.weak("Key commands route to their primary. Server-level commands and command-line scans use the seed node; the key browser scans all primaries.");}
        let enabled = !self.busy && self.connected && self.edit.is_none();
        let mut command_to_run = self.command_editor.show_with_context(ui, &mut self.command, enabled, &crate::redis_completion::Context {keys:&self.keys,indexes:self.search.loaded_indexes(),snapshot:self.snapshot.as_ref()});
        ui.add_space(6.0);
        egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 0)).show(ui, |ui| {
        ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if let Some(command) = self.command_editor.toolbar(ui, &self.command, enabled, self.busy) {
                command_to_run = Some(command);
            }
            ui.separator();
            ui.menu_button("Command help", |ui| {
                for (name, template) in [
                    ("GET · read a string", "GET \"key\""),
                    ("HGET · read a field", "HGET \"key\" \"field\""),
                    ("SCAN · discover names", "SCAN 0 MATCH * COUNT 200"),
                    ("TTL · remaining seconds", "TTL \"key\""),
                    ("INFO · server information", "INFO"),
                    ("FT.AGGREGATE · count matches", "FT.AGGREGATE \"index\" * GROUPBY 0 REDUCE COUNT 0 AS count LIMIT 0 50 DIALECT 2"),
                    ("FT.PROFILE · query timing", "FT.PROFILE \"index\" SEARCH LIMITED QUERY * NOCONTENT LIMIT 0 50 DIALECT 2"),
                    ("JSON.GET · JSON document", "JSON.GET \"key\" $"),
                ] {
                    if ui.button(name).clicked() {
                        self.command = template.into();
                        ui.close();
                    }
                }
            });
            if self.section != Section::Workbench { ui.menu_button("History", |ui| {
                ui.weak(if self.history_profile.is_some() {"Saved connection · last 100 commands"} else {"This session · last 100 commands"});
                if ui.checkbox(&mut self.history_recording, "Record history").on_hover_text("Applies to all Redis connections; existing history remains until cleared").changed() { self.history_dirty = true; }
                if ui.add_enabled(!self.command_history.is_empty(), action_button("trash-2", "Clear history")).clicked() { self.command_history.clear(); self.history_dirty = true; }
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for command in self.command_history.iter().rev() {
                            if ui
                                .button(command.chars().take(80).collect::<String>())
                                .clicked()
                            {
                                self.command = command.clone(); self.command_editor.focus();
                                ui.close();
                            }
                        }
                    });
            });
            }
            ui.menu_button("Saved commands", |ui| {
                let mut remove = None;
                for (i, snippet) in self.snippets.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.button(&snippet.name).clicked() {
                            self.command = snippet.command.clone();
                            ui.close();
                        }
                        if ui.button("Remove").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    self.snippets.remove(i);
                    self.snippets_dirty = true;
                }
                ui.separator();
                ui.add(
                    egui::TextEdit::singleline(&mut self.snippet_name)
                        .hint_text("Name this command")
                        .char_limit(80),
                );
                ui.weak("Saved locally, including any values in the command.");
                if ui
                    .add_enabled(
                        !self.snippet_name.trim().is_empty()
                            && !self.command.trim().is_empty()
                            && self.command.len() <= 65536
                            && self.snippets.len() < 20,
                        egui::Button::new("Save current command"),
                    )
                    .clicked()
                {
                    self.snippets.push(crate::redis_preferences::Snippet {
                        name: self.snippet_name.trim().into(),
                        command: self.command.clone(),
                    });
                    self.snippets_dirty = true;
                    self.snippet_name.clear();
                    ui.close();
                }
            });
        });
        });
        });
        if let Some(command) = command_to_run {
            self.send(Job::Execute(command, self.read_only));
        }
        ui.add_space(8.0);
        crate::redis_command_editor::panel(ui, 0, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::Frame::new().inner_margin(10).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("Output");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        crate::redis_value_editor::copy(ui, "output", Some("Copy"), !self.output.is_empty(), || self.output.clone());
                    });
                });
            });
            let (divider, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), 1.0), egui::Sense::hover(),
            );
            ui.painter().line_segment(
                [divider.left_center(), divider.right_center()],
                ui.visuals().widgets.noninteractive.bg_stroke,
            );
            egui::Frame::new().inner_margin(10).show(ui, |ui| {
                egui::ScrollArea::both()
                    .id_salt("redis-command-output")
                    .max_height(ui.available_height().max(32.0))
                    .auto_shrink([false, self.section != Section::Workbench])
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if self.output.is_empty() {
                            ui.vertical_centered(|ui| {
                                ui.add_space((ui.available_height() * 0.3).max(24.0));
                                ui.heading("Command results");
                                ui.weak("Run a command to see its result.");
                            });
                        } else if !self.command_results.entries.is_empty() {
                            let width = ui.available_width();
                            if (self.command_result_width - width).abs() > 0.5 {
                                self.command_result_heights.clear();
                                self.command_result_width = width;
                            }
                            self.command_result_heights.resize(self.command_results.entries.len(), 0.0);
                            for (index, entry) in self.command_results.entries.iter().enumerate() {
                                let height = self.command_result_heights[index];
                                let bounds = egui::Rect::from_min_size(ui.next_widget_position(), egui::vec2(width, height));
                                if height > 0.0 && !ui.is_rect_visible(bounds) {
                                    ui.allocate_space(egui::vec2(width, height));
                                    continue;
                                }
                                let response = ui.push_id(index, |ui| {
                                    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), ui.id().with("result"), true)
                                        .show_header(ui, |ui| {
                                            ui.label(format!("{}. {}", index + 1, entry.command));
                                            ui.weak(format!("{} · {:.2} ms", entry.status.label(), entry.elapsed.as_secs_f64() * 1000.0));
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                crate::redis_value_editor::copy(ui, "command-result", None, true, || entry.output.clone());
                                            });
                                        }).body(|ui| { ui.monospace(&entry.output); });
                                    ui.separator();
                                });
                                self.command_result_heights[index] = response.response.rect.height();
                            }
                        } else {
                            ui.monospace(&self.output);
                        }
                    });
            });
        });
    }
    #[cfg(all(feature = "qa", not(target_arch = "wasm32")))]
    pub(crate) fn preview_bulk(&mut self, step: usize) -> bool {
        if self.busy || !self.scan_started || self.cursor != 0 { return false; }
        self.read_only = false;
        self.diagnostics_open = false;
        let mut keys: Vec<_> = (0..30).map(|i| format!("bulk:{i:02}").into_bytes()).collect();
        keys.push(b"bulk:missing".to_vec());
        if step == 0 {
            self.edit = Some(crate::redis_edit::Draft::new(crate::redis_mutation::Action::ExpireKeys, keys, String::new()));
        } else {
            self.edit = None;
            let mutation = crate::redis_mutation::Mutation {
                action: crate::redis_mutation::Action::ExpireKeys, keys, kind: String::new(),
                field: vec![], value: vec![], expected: vec![], length: 0, ttl_ms: None,
            };
            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            self.bulk_cancel = Some(cancel.clone());
            self.send(Job::Bulk(mutation, false, cancel));
        }
        true
    }
    #[cfg(all(feature = "qa", not(target_arch = "wasm32")))]
    pub(crate) fn preview_topology(&mut self, step: usize) -> bool {
        if self.busy || !self.connected { return false; }
        if !self.scan_started || self.cursor!=0 {self.auto_scan=true;return false;}
        if step==1 {self.diagnostics_open=false;self.expanded.insert(b"routing:".to_vec());self.rebuild_tree();self.send(Job::Inspect(b"routing:1".to_vec()));return true;}
        let request=crate::redis_diagnostics::Request::Cluster;
        self.diagnostics_request=Some(request.clone());
        self.diagnostics_report=None;
        self.diagnostics_open=true;
        self.send(Job::Diagnostics(request));
        true
    }
    #[cfg(all(feature = "qa", not(target_arch = "wasm32")))]
    pub(crate) fn preview_archive(&mut self, ctx: &egui::Context, step: usize) -> bool {
        if self.busy || !self.scan_started || self.cursor != 0 {
            return false;
        }
        match step {
            0 => {
                let mut keys = self.keys.clone();
                keys.push(b"archive:missing".to_vec());
                let action = self.archive.export(keys, self.database);
                self.send(Job::Archive(action));
                true
            }
            1 => self.archive.preview_load_export(ctx),
            2 => {
                if let Some(action) = self.archive.preview_review(self.database) {
                    self.send(Job::Archive(action));
                    true
                } else {
                    false
                }
            }
            3 => {
                self.archive.preview_options();
                true
            }
            4 => {
                self.read_only = false;
                if let Some(action) = self.archive.preview_import(self.database) {
                    self.send(Job::Archive(action));
                    true
                } else {
                    false
                }
            }
            _ => {
                self.read_only = true;
                self.archive.preview_clear_report();
                if let Some(action) = self.archive.preview_review(self.database) {
                    self.send(Job::Archive(action));
                    true
                } else {
                    false
                }
            }
        }
    }
    #[cfg(all(feature = "qa", not(target_arch = "wasm32")))]
    pub(crate) fn preview_pubsub(&mut self, ctx: &egui::Context, step: usize) -> bool {
        if self.busy || !self.scan_started || self.cursor != 0 {
            return false;
        }
        if !self.pubsub.preview(ctx, step) {
            return false;
        }
        if step == 1 {
            self.read_only = false;
            self.send(Job::Publish(
                crate::redis_pubsub_view::Publish {
                    channel: b"events:demo".to_vec(),
                    payload: br#"{"event":"order.created","id":42,"total":129.95,"paid":true}"#
                        .to_vec(),
                },
                false,
            ));
        }
        true
    }
    #[cfg(all(feature = "qa", not(target_arch = "wasm32")))]
    pub(crate) fn preview_monitor(&mut self, ctx: &egui::Context, step: usize) -> bool {
        if self.busy || !self.scan_started || self.cursor != 0 {
            return false;
        }
        if !self.monitor.preview(ctx, step) {
            return false;
        }
        if step == 1 {
            self.send(Job::Execute(
                "HGETALL app:users:1001\nHGETALL app:users:1002\nPING".into(),
                true,
            ));
        }
        true
    }
    #[cfg(feature = "qa")]
    pub(crate) fn preview_search(&mut self, step: usize) -> bool {
        if self.busy || !self.scan_started || self.cursor != 0 {
            return false;
        }
        if std::env::var_os("RUSTABLE_REDIS_INDEX_MANAGEMENT_CAPTURE").is_some() { self.read_only = false; }
        let request = self.search.preview_request(step);
        self.search.begin();
        self.send(Job::Search(request));
        true
    }
    #[cfg(feature = "qa")]
    pub(crate) fn preview_roadmap(&mut self, step: usize) -> bool {
        if self.busy || !self.scan_started || self.cursor != 0 {
            return false;
        }
        self.read_only = false;
        self.edit = None;
        self.diagnostics_open = false;
        self.commands_visible = false;
        match step {
            0 => {
                let mut draft = crate::redis_edit::Draft::new(
                    crate::redis_mutation::Action::Create,
                    vec![],
                    "hash".into(),
                );
                draft.key_text = "app:users:1004".into();
                draft.field = "name".into();
                draft.value = "Jordan Lee".into();
                draft.expiry = true;
                self.edit = Some(draft);
            }
            1 => {
                self.expanded.insert(b"app:".to_vec());
                self.expanded.insert(b"app:users:".to_vec());
                self.rebuild_tree();
                self.send(Job::Inspect(b"app:users:1001".to_vec()));
            }
            2 => {
                self.bulk_mode = true;
                self.bulk_selected = [
                    b"app:users:1001".to_vec(),
                    b"app:users:1002".to_vec(),
                    b"session:mobile:ghi789".to_vec(),
                ]
                .into_iter()
                .collect();
                self.edit = Some(crate::redis_edit::Draft::new(
                    crate::redis_mutation::Action::DeleteKeys,
                    self.bulk_selected.iter().cloned().collect(),
                    String::new(),
                ));
            }
            3 | 4 => {
                self.bulk_mode = false;
                let request = if step == 3 {
                    crate::redis_diagnostics::Request::Server
                } else {
                    crate::redis_diagnostics::Request::Keys(self.keys.clone())
                };
                self.diagnostics_report = None;
                self.diagnostics_request = Some(request.clone());
                self.diagnostics_open = true;
                self.send(Job::Diagnostics(request));
            }
            _ => {
                self.commands_visible = true;
                self.inspector_visible = false;
                self.command = "GET cache:profile\nHGET app:users:1001 name".into();
                self.send(Job::Execute(self.command.clone(), true));
            }
        }
        true
    }
    #[cfg(feature = "qa")]
    pub(crate) fn preview_roadmap_format(&mut self, step: usize) {
        if step == 1
            && self.edit.is_none()
            && let (Some(snapshot), Some(page)) = (&self.snapshot, &self.value_page)
        {
            self.selected_value = Some(0);
            self.edit = collection_draft(
                snapshot,
                page,
                Some(0),
                crate::redis_mutation::Action::Update,
            );
        }
    }
    #[cfg(feature = "qa")]
    pub(crate) fn preview_inspector(&mut self, step: usize) -> bool {
        if self.busy || !self.scan_started || self.cursor != 0 {
            return false;
        }
        self.expanded.clear();
        self.expanded.insert(match step {
            0 | 1 => b"queue:".to_vec(),
            2 => b"cache:".to_vec(),
            3 => b"leaderboard:".to_vec(),
            4 => b"events:".to_vec(),
            _ => b"app:".to_vec(),
        });
        if step == 5 {
            self.expanded.insert(b"app:settings:".to_vec());
        }
        self.rebuild_tree();
        if step == 1 {
            if let (Some(snapshot), Some(page)) = (&self.snapshot, &self.value_page)
                && let Some(cursor) = &page.next
            {
                self.send(Job::ValuePage(
                    PageRequest {
                        key: snapshot.key.clone(),
                        kind: snapshot.kind.clone(),
                        cursor: cursor.clone(),
                        pattern: page.pattern.clone(),
                    },
                    Navigation::Next,
                ));
            }
        } else {
            let key = match step {
                0 => b"queue:emails".as_slice(),
                2 => b"cache:profile",
                3 => b"leaderboard:weekly",
                4 => b"events:orders",
                _ => b"app:settings:features",
            };
            self.send(Job::Inspect(key.to_vec()));
        }
        true
    }
    #[cfg(feature = "qa")]
    pub(crate) fn preview_value_format(&mut self, step: usize) {
        if step == 2 {
            self.string_mode = StringMode::Json;
        }
        if step == 4 {
            self.selected_value = Some(0);
        }
    }
    #[cfg(feature = "qa")]
    pub(crate) fn preview_stage(&mut self, list: bool) -> bool {
        if self.busy || !self.scan_started || self.cursor != 0 {
            return false;
        }
        self.tree_view = !list;
        if list {
            self.pattern = "app:*".into();
            self.kind = "hash".into();
            self.refresh();
        } else {
            self.expanded.extend([
                b"app:".to_vec(),
                b"app:users:".to_vec(),
                b"app:config:".to_vec(),
                b"cache:".to_vec(),
                b"session:".to_vec(),
            ]);
            self.rebuild_tree();
            self.send(Job::Inspect(b"app:users:1001".to_vec()));
        }
        true
    }
    fn rebuild_tree(&mut self) {
        self.tree = tree_rows(&self.keys, self.separator.as_bytes(), &self.expanded);
    }
    fn key_tree(&mut self, ui: &mut egui::Ui) {
        let mut toggle = None;
        let mut inspect = None;
        self.visible_indices.clear();
        let width = ui.available_width();
        let key_width = (width * 0.48).max(180.0);
        let type_x = key_width;
        let ttl_x = key_width + 88.0;
        let (header, _) = ui.allocate_exact_size(egui::vec2(width, 32.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(header, 0.0, ui.visuals().faint_bg_color);
        for (x, text) in [
            (16.0, "KEY / NAMESPACE"),
            (type_x, "TYPE"),
            (ttl_x, "EXPIRES IN"),
        ] {
            ui.painter().text(
                header.left_center() + egui::vec2(x, 0.0),
                egui::Align2::LEFT_CENTER,
                text,
                egui::FontId::proportional(10.0),
                ui.visuals().weak_text_color(),
            );
        }
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical()
                .id_salt("redis-tree")
                .show_rows(ui, 32.0, self.tree.len(), |ui, range| {
                    for index in range {
                        let row = &self.tree[index];
                        let key = &self.keys[row.key];
                        let folder = row.prefix_len.is_some();
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 32.0),
                            egui::Sense::click(),
                        );
                        let selected = !folder && self.selected_key.as_ref() == Some(key);
                        if selected || response.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                0.0,
                                if selected {
                                    ui.visuals().selection.bg_fill.gamma_multiply(0.45)
                                } else {
                                    ui.visuals().widgets.hovered.weak_bg_fill
                                },
                            );
                        }
                        if selected {
                            ui.painter().rect_filled(
                                egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
                                0.0,
                                ui.visuals().selection.stroke.color,
                            );
                        }
                        let x = rect.left() + 16.0 + row.depth as f32 * 18.0;
                        let name_end = row
                            .prefix_len
                            .map_or(key.len(), |end| end.saturating_sub(self.separator.len()));
                        let name = display_key(&key[row.start..name_end.max(row.start)]);
                        let clip = egui::Rect::from_min_max(
                            rect.min,
                            egui::pos2(rect.left() + key_width - 12.0, rect.bottom()),
                        )
                        .intersect(ui.clip_rect());
                        let painter = ui.painter().with_clip_rect(clip);
                        if let Some(end) = row.prefix_len {
                            crate::icons::paint(
                                &painter,
                                egui::pos2(x + 6.0, rect.center().y),
                                if self.expanded.contains(&key[..end]) {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                },
                                12.0,
                                ui.visuals().weak_text_color(),
                            );
                            crate::icons::paint(
                                &painter,
                                egui::pos2(x + 26.0, rect.center().y),
                                "folder",
                                16.0,
                                egui::Color32::from_rgb(190, 164, 112),
                            );
                            ui.painter().text(
                                egui::pos2(rect.left() + type_x, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                format!("{} loaded", row.count),
                                egui::FontId::proportional(11.0),
                                ui.visuals().weak_text_color(),
                            );
                        } else {
                            self.visible_indices.push(row.key);
                            if self.bulk_mode {
                                let mark = egui::Rect::from_center_size(
                                    egui::pos2(x + 26.0, rect.center().y),
                                    egui::vec2(14.0, 14.0),
                                );
                                ui.painter().rect_stroke(
                                    mark,
                                    2.0,
                                    ui.visuals().widgets.inactive.fg_stroke,
                                    egui::StrokeKind::Inside,
                                );
                                if self.bulk_selected.contains(key) {
                                    crate::icons::paint(
                                        ui.painter(),
                                        mark.center(),
                                        "check",
                                        12.0,
                                        ui.visuals().text_color(),
                                    );
                                }
                            } else {
                                ui.painter().circle_filled(
                                    egui::pos2(x + 26.0, rect.center().y),
                                    3.0,
                                    self.rows
                                        .get(key)
                                        .map_or(egui::Color32::GRAY, |r| type_color(&r.1)),
                                );
                            }
                            if let Some((_, kind, ttl)) = self.rows.get(key) {
                                let badge = egui::Rect::from_center_size(
                                    egui::pos2(rect.left() + type_x + 30.0, rect.center().y),
                                    egui::vec2(60.0, 19.0),
                                );
                                ui.painter().rect_filled(
                                    badge,
                                    4.0,
                                    type_color(kind).gamma_multiply(0.15),
                                );
                                ui.painter().text(
                                    badge.center(),
                                    egui::Align2::CENTER_CENTER,
                                    type_label(kind).to_uppercase(),
                                    egui::FontId::proportional(9.0),
                                    type_color(kind),
                                );
                                ui.painter().text(
                                    egui::pos2(rect.left() + ttl_x, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    ttl_label(*ttl),
                                    egui::FontId::proportional(11.0),
                                    ui.visuals().weak_text_color(),
                                );
                            }
                        }
                        painter.text(
                            egui::pos2(x + 42.0, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            if name.is_empty() {
                                "(key at this prefix)"
                            } else {
                                &name
                            },
                            egui::FontId::monospace(12.0),
                            ui.visuals().text_color(),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::SelectableLabel,
                                ui.is_enabled(),
                                &name,
                            )
                        });
                        if response.clicked()
                            || (response.has_focus()
                                && ui.input(|i| {
                                    i.key_pressed(egui::Key::Enter)
                                        || i.key_pressed(egui::Key::Space)
                                }))
                        {
                            if let Some(end) = row.prefix_len {
                                toggle = Some(key[..end].to_vec());
                            } else if self.bulk_mode {
                                if !self.bulk_selected.remove(key)
                                    && self.bulk_selected.len() < 1000
                                {
                                    self.bulk_selected.insert(key.clone());
                                }
                            } else {
                                inspect = Some(key.clone());
                            }
                        }
                        if response.has_focus()
                            && let Some(end) = row.prefix_len
                        {
                            let open = self.expanded.contains(&key[..end]);
                            if ui.input(|i| {
                                (i.key_pressed(egui::Key::ArrowRight) && !open)
                                    || (i.key_pressed(egui::Key::ArrowLeft) && open)
                            }) {
                                toggle = Some(key[..end].to_vec());
                            }
                        }
                        response.context_menu(|ui| {
                            if !folder { crate::redis_value_editor::copy(ui, "tree-key", Some("Copy key"), true, || display_key(key)); }
                            if !folder && ui.button("Inspect key").clicked() {
                                inspect = Some(key.clone());
                                ui.close();
                            }
                        });
                        response.on_hover_text(if folder {
                            "Expand or collapse discovered keys".to_owned()
                        } else {
                            display_key(key)
                        });
                    }
                });
        });
        if self.tree.is_empty() {
            self.empty_browser(ui);
        }
        if let Some(prefix) = toggle {
            if !self.expanded.remove(&prefix) && self.expanded.len() < 10000 {
                self.expanded.insert(prefix);
            }
            self.rebuild_tree();
        }
        if let Some(key) = inspect {
            self.inspector_visible = true;
            self.send(Job::Inspect(key));
        }
    }
    fn empty_browser(&self, ui: &mut egui::Ui) {
        if self.busy {
            return;
        }
        ui.add_space(32.0);
        ui.vertical_centered(|ui| {
            if !self.connected {
                ui.strong("Connection unavailable");
                ui.weak("Reconnect to browse keys. Your filters are retained.");
            } else if self.scan_started && self.cursor == 0 {
                ui.strong("No matching keys");
                ui.weak("Try * or choose another key type, then search again.");
            } else {
                ui.strong("No matches in this batch");
                ui.weak("Continue scanning to check more keys, or adjust the filter.");
            }
        });
    }
    fn key_grid(&mut self, ui: &mut egui::Ui) {
        use egui_extras::{Column, TableBuilder};
        let mut inspect = None;
        let mut delete = None;
        self.visible_indices.clear();
        crate::app::with_grid_scrollbars(ui, |ui| {
            TableBuilder::new(ui)
                .wheel_before_layout(true)
                .min_scrolled_height(0.0)
                .id_salt(("redis-key-grid", self.database))
                .striped(false)
                .resizable(true)
                .auto_shrink([false, false])
                .sense(egui::Sense::click())
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::initial(100.0).clip(true))
                .column(Column::initial(260.0).at_least(90.0).clip(true))
                .column(Column::initial(100.0).clip(true))
                .column(Column::remainder().at_least(120.0).clip(true))
                .header(32.0, |mut header| {
                    for name in ["TYPE", "KEY", "EXPIRES IN", "VALUE PREVIEW"] {
                        header.col(|ui| {
                            ui.add_space(12.0);
                            ui.label(
                                egui::RichText::new(name)
                                    .size(10.0)
                                    .color(ui.visuals().weak_text_color()),
                            );
                        });
                    }
                })
                .body(|body| {
                    body.rows(22.0, self.keys.len(), |mut row| {
                        let index = row.index();
                        let key = &self.keys[index];
                        self.visible_indices.push(index);
                        let metadata = self.rows.get(key);
                        row.set_selected(
                            self.selected_key.as_ref() == Some(key)
                                || (self.bulk_mode && self.bulk_selected.contains(key)),
                        );
                        row.col(|ui| {
                            ui.add_space(10.0);
                            if let Some((_, kind, _)) = metadata {
                                type_badge(ui, kind);
                            } else {
                                ui.weak("—");
                            }
                        });
                        row.col(|ui| {
                            ui.add_space(12.0);
                            ui.add(
                                egui::Label::new(egui::RichText::new(display_key(key)).monospace())
                                    .truncate(),
                            );
                        });
                        row.col(|ui| {
                            ui.weak(metadata.map_or("—".into(), |row| ttl_label(row.2)));
                        });
                        row.col(|ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(
                                        metadata.map_or("Loading preview…", |row| row.0.as_str()),
                                    )
                                    .color(ui.visuals().weak_text_color()),
                                )
                                .truncate(),
                            );
                        });
                        let response = row.response();
                        if ((response.contains_pointer()
                            && response.ctx.input(|input| input.pointer.primary_pressed()))
                            || (response.has_focus()
                                && response.ctx.input(|input| {
                                    input.key_pressed(egui::Key::Enter)
                                        || input.key_pressed(egui::Key::Space)
                                })))
                            && (!self.busy || self.browse_busy)
                        {
                            if self.bulk_mode {
                                if !self.bulk_selected.remove(key)
                                    && self.bulk_selected.len() < 1000
                                {
                                    self.bulk_selected.insert(key.clone());
                                }
                            } else {
                                inspect = Some(key.clone());
                            }
                        }
                        response.context_menu(|ui| {
                            if ui.button("Inspect key").clicked() {
                                inspect = Some(key.clone());
                                ui.close();
                            }
                            crate::redis_value_editor::copy(ui, "list-key", Some("Copy key"), true, || display_key(key));
                            if ui
                                .add_enabled(
                                    !self.read_only && !self.busy,
                                    egui::Button::new("Delete key…"),
                                )
                                .clicked()
                            {
                                delete = Some(key.clone());
                                ui.close();
                            }
                        });
                    });
                });
        });
        if self.keys.is_empty() {
            self.empty_browser(ui);
        }
        if let Some(key) = inspect {
            self.inspector_visible = true;
            self.send(Job::Inspect(key));
        }
        if let Some(key) = delete {
            if self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.key == key)
            {
                self.delete_confirmation = true;
            } else {
                self.inspector_visible = true;
                self.send(Job::Inspect(key));
                self.notice = Some("Inspect this key, then choose Delete key to confirm.".into());
            }
        }
    }
}
struct TreeRow {
    key: usize,
    start: usize,
    prefix_len: Option<usize>,
    depth: usize,
    count: usize,
}
// Sorted ranges form virtual folders. Only expanded branches get flattened;
// depth/prefix limits keep pathological binary key names cheap and navigable.
fn tree_rows(
    keys: &[Vec<u8>],
    separator: &[u8],
    expanded: &std::collections::HashSet<Vec<u8>>,
) -> Vec<TreeRow> {
    fn append(
        keys: &[Vec<u8>],
        separator: &[u8],
        expanded: &std::collections::HashSet<Vec<u8>>,
        range: std::ops::Range<usize>,
        start: usize,
        depth: usize,
        rows: &mut Vec<TreeRow>,
    ) {
        let mut i = range.start;
        while i < range.end {
            let key = &keys[i];
            let split = if separator.is_empty() || depth >= 16 || start >= 512 {
                None
            } else {
                key[start..key.len().min(512)]
                    .windows(separator.len())
                    .position(|s| s == separator)
                    .map(|n| start + n + separator.len())
            };
            if let Some(end) = split {
                let prefix = &key[..end];
                let mut next = i + 1;
                while next < range.end && keys[next].starts_with(prefix) {
                    next += 1;
                }
                rows.push(TreeRow {
                    key: i,
                    start,
                    prefix_len: Some(end),
                    depth,
                    count: next - i,
                });
                if expanded.contains(prefix) {
                    append(keys, separator, expanded, i..next, end, depth + 1, rows);
                }
                i = next;
            } else {
                rows.push(TreeRow {
                    key: i,
                    start,
                    prefix_len: None,
                    depth,
                    count: 1,
                });
                i += 1;
            }
        }
    }
    let mut rows = Vec::new();
    append(keys, separator, expanded, 0..keys.len(), 0, 0, &mut rows);
    rows
}
fn collection_pattern(text: &str) -> Result<Vec<u8>, String> {
    if text.is_empty() {
        Ok(b"*".to_vec())
    } else {
        exact_key_bytes(text)
    }
}
fn exact_key_bytes(text: &str) -> Result<Vec<u8>, String> {
    if text.starts_with('"') || text.starts_with('\'') {
        let args = crate::redis_commands::parse(text)?;
        if args.len() != 1 || args[0].len() != 1 {
            return Err("Enter one key name or quoted byte string.".into());
        }
        Ok(args[0][0].clone())
    } else {
        Ok(text.as_bytes().to_vec())
    }
}
fn ttl_label(ttl: i64) -> String {
    match ttl {
        -2 => "Expired".into(),
        -1 => "No expiry".into(),
        n if n >= 86_400_000 => format!("{}d {}h", n / 86_400_000, (n / 3_600_000) % 24),
        n if n >= 3_600_000 => format!("{}h {}m", n / 3_600_000, (n / 60_000) % 60),
        n if n >= 60_000 => format!("{}m {}s", n / 60_000, (n / 1000) % 60),
        n => format!("{}s", n / 1000),
    }
}
pub(crate) fn display_key(key: &[u8]) -> String {
    if key.len() > 512 {
        return format!(
            "{}… ({} bytes)",
            crate::redis_commands::quote(&key[..512]),
            key.len()
        );
    }
    match std::str::from_utf8(key) {
        Ok(text) if !text.chars().any(char::is_control) => text.to_owned(),
        _ => crate::redis_commands::quote(key),
    }
}
fn value_lines(ui: &mut egui::Ui, id: &'static str, lines: &[String], height: f32) {
    value_lines_styled(ui, id, lines, height, None);
}
pub(crate) fn value_lines_styled(
    ui: &mut egui::Ui,
    id: &'static str,
    lines: &[String],
    height: f32,
    tokens: Option<&Vec<Vec<crate::redis_value::JsonToken>>>,
) {
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(6.0)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            egui::ScrollArea::both()
                .id_salt(id)
                .max_height(height)
                .show_rows(
                    ui,
                    ui.text_style_height(&egui::TextStyle::Monospace),
                    lines.len(),
                    |ui, range| {
                        for index in range {
                            if let Some(tokens) = tokens {
                                let mut job = egui::text::LayoutJob::default();
                                for token in &tokens[index] {
                                    job.append(
                                        &lines[index][token.range.clone()],
                                        0.0,
                                        egui::TextFormat {
                                            font_id: egui::TextStyle::Monospace.resolve(ui.style()),
                                            color: token.kind.color(ui.visuals().dark_mode),
                                            ..Default::default()
                                        },
                                    );
                                }
                                ui.add(egui::Label::new(job).extend());
                            } else {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&lines[index]).monospace(),
                                    )
                                    .extend(),
                                );
                            }
                        }
                    },
                );
        });
}
pub(crate) fn browser_style(ui: &mut egui::Ui) {
    crate::controls::base_spacing(ui.spacing_mut());
    crate::controls::rounded_action_buttons(ui);
}
pub(crate) fn type_label(kind: &str) -> &str {
    if kind == "vectorset" { "Vector set" } else if kind == "ReJSON-RL" { "JSON" } else if kind == "TSDB-TYPE" { "Time series" } else { kind }
}
fn type_color(kind: &str) -> egui::Color32 {
    match kind {
        "vectorset" => egui::Color32::from_rgb(127, 158, 230),
        "ReJSON-RL" => egui::Color32::from_rgb(165, 141, 231),
        "hash" => egui::Color32::from_rgb(165, 141, 231),
        "string" => egui::Color32::from_rgb(103, 183, 221),
        "set" | "zset" => egui::Color32::from_rgb(220, 170, 99),
        "list" => egui::Color32::from_rgb(103, 192, 158),
        "stream" => egui::Color32::from_rgb(222, 135, 153),
        "TSDB-TYPE" => egui::Color32::from_rgb(98, 190, 170),
        _ => egui::Color32::GRAY,
    }
}
fn type_badge(ui: &mut egui::Ui, kind: &str) {
    let color = type_color(kind);
    egui::Frame::new()
        .fill(color.gamma_multiply(0.15))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.spacing_mut().interact_size.y = 14.0;
            ui.label(
                egui::RichText::new(type_label(kind).to_uppercase())
                    .size(10.0)
                    .strong()
                    .color(color),
            );
        });
}
fn grid_preview(value: &redis::Value, kind: &str) -> String {
    if kind == "vectorset" && let redis::Value::Int(count) = value { return format!("{count} elements"); }
    if kind == "TSDB-TYPE" && let redis::Value::Array(sample) = value {
        let text = if sample.len() == 2 { format!("Latest: {} at {} ms", crate::redis_value::text(&sample[1]), crate::redis_value::text(&sample[0])) } else { "No samples".into() };
        return text.chars().take(256).collect();
    }
    if kind == "ReJSON-RL" && !matches!(value, redis::Value::BulkString(_)) {
        return "JSON document · open to inspect".into();
    }
    fn json(value: &redis::Value) -> serde_json::Value {
        match value {
            redis::Value::BulkString(bytes) => {
                serde_json::Value::String(match std::str::from_utf8(bytes) {
                    Ok(text) if !text.contains('\0') => text.to_owned(),
                    _ => crate::redis_commands::quote(bytes),
                })
            }
            redis::Value::Array(values) => {
                serde_json::Value::Array(values.iter().map(json).collect())
            }
            redis::Value::Int(n) => (*n).into(),
            redis::Value::SimpleString(text) => text.clone().into(),
            _ => serde_json::Value::Null,
        }
    }
    let text = if kind == "hash"
        && let redis::Value::Array(values) = value
    {
        let mut map = serde_json::Map::new();
        for pair in values.as_chunks::<2>().0 {
            map.insert(
                json(&pair[0]).as_str().unwrap_or("").to_owned(),
                json(&pair[1]),
            );
        }
        serde_json::Value::Object(map).to_string()
    } else if let redis::Value::BulkString(bytes) = value {
        match std::str::from_utf8(bytes) {
            Ok(text) if !text.chars().any(char::is_control) => text.to_owned(),
            _ => crate::redis_commands::quote(bytes),
        }
    } else {
        json(value).to_string()
    };
    let mut output: String = text.chars().take(256).collect();
    if text.chars().count() > 256 {
        output.push('…');
    }
    output
}
#[cfg(test)]
fn render_inspection(snapshot: &Snapshot) -> String {
    if matches!(snapshot.kind.as_str(), "hash" | "zset")
        && let redis::Value::Array(values) = &snapshot.value
    {
        let mut text = if snapshot.kind == "hash" {
            "Field → Value\n".to_owned()
        } else {
            "Member → Score\n".to_owned()
        };
        for pair in values.chunks(2) {
            let left = render_values(&pair[..1]);
            let right = render_values(&pair[1..]);
            text.push_str(&format!("{} → {}\n", left.trim_end(), right.trim_end()));
            if text.len() > 256 * 1024 {
                break;
            }
        }
        return limit_output(text);
    }
    render_values(std::slice::from_ref(&snapshot.value))
}
fn limit_output(mut output: String) -> String {
    if output.len() > 256 * 1024 {
        let mut end = 256 * 1024;
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        output.truncate(end);
        output.push_str("\n[output truncated]");
    }
    output
}
pub(crate) fn render_values(values: &[redis::Value]) -> String {
    fn append(value: &redis::Value, out: &mut String, depth: usize) {
        if out.len() > 256 * 1024 {
            return;
        }
        if depth > 32 {
            out.push_str("[nesting limit]\n");
            return;
        }
        match value {
            redis::Value::BulkString(bytes) => {
                out.push_str(&crate::redis_commands::quote(
                    &bytes[..bytes.len().min(65536)],
                ));
                if bytes.len() > 65536 {
                    out.push_str(" … [truncated]");
                }
                out.push('\n');
            }
            redis::Value::Array(values) | redis::Value::Set(values) => {
                if values.is_empty() {
                    out.push_str("(empty collection)\n");
                }
                for (i, value) in values.iter().take(1000).enumerate() {
                    if out.len() > 256 * 1024 {
                        break;
                    }
                    out.push_str(&format!("{}{}. ", "  ".repeat(depth), i + 1));
                    append(value, out, depth + 1);
                }
                if values.len() > 1000 {
                    out.push_str("[remaining elements omitted]\n");
                }
            }
            redis::Value::Map(values) => {
                if values.is_empty() {
                    out.push_str("(empty map)\n");
                }
                for (key, value) in values.iter().take(1000) {
                    if out.len() > 256 * 1024 {
                        break;
                    }
                    append(key, out, depth + 1);
                    append(value, out, depth + 1);
                }
            }
            redis::Value::SimpleString(value) => {
                out.push_str(&value.chars().take(65536).collect::<String>());
                out.push('\n');
            }
            redis::Value::Nil => out.push_str("(nil)\n"),
            redis::Value::Okay => out.push_str("OK\n"),
            redis::Value::Int(value) => out.push_str(&format!("{value}\n")),
            redis::Value::Double(value) => out.push_str(&format!("{value}\n")),
            redis::Value::Boolean(value) => out.push_str(&format!("{value}\n")),
            _ => out.push_str("[reply type unavailable in this view]\n"),
        }
    }
    let mut output = String::new();
    for value in values {
        append(value, &mut output, 0);
    }
    limit_output(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestResponses = mpsc::Sender<Result<Reply, (String, bool)>>;
    fn mock_workspace() -> (Workspace, TestResponses, mpsc::Receiver<Job>) {
        // Empty host fails validation before any connection attempt.
        let mut workspace = Workspace::new(
            Settings {
                host: String::new(),
                ..Default::default()
            },
            String::new(),
            egui::Context::default(),
        );
        assert!(workspace.receiver.recv().unwrap().is_err());
        let (responses, receiver) = mpsc::channel();
        let (sender, jobs) = mpsc::sync_channel(1);
        workspace.receiver = receiver;
        workspace.sender = sender;
        workspace.busy = false;
        workspace.connected = true;
        (workspace, responses, jobs)
    }
    #[test]
    fn history_is_persisted_per_profile_and_clear_and_recording_survive_reload() {
        let (mut workspace, _, jobs)=mock_workspace();
        let mut preferences=crate::redis_preferences::Preferences::default();
        workspace.set_history_profile(Some(11));workspace.load_preferences(&preferences);
        workspace.send(Job::Execute("GET account".into(),true));assert!(matches!(jobs.try_recv().unwrap(),Job::Execute(..)));workspace.busy=false;
        assert!(workspace.store_preferences(&mut preferences));
        let bytes=serde_json::to_vec(&preferences).unwrap();
        let mut restored:crate::redis_preferences::Preferences=serde_json::from_slice(&bytes).unwrap();
        workspace.set_history_profile(Some(22));workspace.load_preferences(&restored);assert!(workspace.command_history.is_empty());
        workspace.send(Job::Execute("GET other".into(),true));jobs.try_recv().unwrap();workspace.busy=false;workspace.store_preferences(&mut restored);
        workspace.set_history_profile(Some(11));workspace.load_preferences(&restored);assert_eq!(workspace.command_history.back().unwrap(),"GET account");
        workspace.history_recording=false;workspace.history_dirty=true;
        workspace.send(Job::Execute("GET private".into(),true));jobs.try_recv().unwrap();workspace.busy=false;
        assert_eq!(workspace.command_history.len(),1);
        workspace.command_history.clear();workspace.history_dirty=true;workspace.store_preferences(&mut restored);
        workspace.reset_preferences();workspace.load_preferences(&restored);
        assert!(!workspace.history_recording);assert!(workspace.command_history.is_empty());
        assert_eq!(restored.histories[&22].commands.back().unwrap(),"GET other");
        workspace.set_history_profile(None);workspace.load_preferences(&restored);
        workspace.send(Job::Execute("GET temporary".into(),true));jobs.try_recv().unwrap();workspace.busy=false;workspace.store_preferences(&mut restored);
        assert_eq!(restored.histories.len(),2); // Unsaved sessions do not acquire another profile's history.
    }
    #[test]
    fn recovered_transport_preserves_drafts_and_warning_without_background_work() {
        let (mut workspace, responses, jobs) = mock_workspace();
        workspace.command = "INCR counter".into();
        workspace.value_editor = "unsaved value".into();
        workspace.edit = Some(crate::redis_edit::Draft::new(crate::redis_mutation::Action::Create, vec![], "string".into()));
        workspace.keys = vec![b"existing".to_vec()];
        workspace.cursor = 99;
        workspace.pending = Some(Job::Inspect(b"queued".to_vec()));
        workspace.preview_queue.push_back(b"existing".to_vec());
        responses.send(Ok(Reply::Recovering("Write may have completed".into()))).unwrap();
        workspace.poll();
        assert!(workspace.busy);
        assert!(!workspace.connected);
        responses.send(Ok(Reply::Recovered)).unwrap();
        workspace.poll();
        assert!(workspace.connected);
        assert!(!workspace.busy);
        assert_eq!(workspace.command, "INCR counter");
        assert_eq!(workspace.value_editor, "unsaved value");
        assert!(workspace.edit.is_some());
        assert_eq!(workspace.error.as_deref(), Some("Write may have completed"));
        assert_eq!(workspace.keys, vec![b"existing".to_vec()]);
        assert_eq!(workspace.cursor, 0);
        assert!(!workspace.auto_scan);
        assert!(workspace.pending.is_none());
        assert!(workspace.preview_queue.is_empty());
        assert!(jobs.try_recv().is_err());
    }
    #[test]
    fn manual_recovery_keeps_original_warning_and_draft_on_failure() {
        let (mut workspace, responses, jobs) = mock_workspace();
        workspace.snapshot = Some(Snapshot { key: b"key".to_vec(), kind: "string".into(), ttl_ms: -1, length: Some(3), value: redis::Value::BulkString(b"old".to_vec()), truncated: false });
        workspace.value_editor = "new draft".into();
        workspace.error = Some("A write may have completed".into());
        workspace.connected = false;
        assert!(workspace.value_dirty());
        workspace.send(Job::Reconnect);
        assert!(matches!(jobs.try_recv().unwrap(), Job::Reconnect));
        assert_eq!(workspace.error.as_deref(), Some("A write may have completed"));
        responses.send(Ok(Reply::RecoveryFailed("unavailable".into()))).unwrap();
        workspace.poll();
        assert!(!workspace.connected);
        assert!(!workspace.busy);
        assert_eq!(workspace.value_editor, "new draft");
        assert_eq!(workspace.error.as_deref(), Some("A write may have completed"));
        assert_eq!(workspace.notice.as_deref(), Some("Reconnect failed: unavailable"));
    }
    #[test]
    fn cluster_worker_recovers_through_known_node_without_replaying_write() {
        use std::{io::{Read, Write}, net::TcpListener, time::Duration};
        fn command(socket: &mut std::net::TcpStream, parser: &mut redis::Parser) -> Vec<Vec<u8>> {
            loop {
                let command: Vec<Vec<u8>> = redis::from_redis_value(parser.parse_value(&mut *socket).unwrap()).unwrap();
                if command[0] == b"CLIENT" { socket.write_all(b"+OK\r\n").unwrap(); } else { return command; }
            }
        }
        fn node(role: &str, port: u16) -> String {
            format!("*10\r\n+id\r\n+{port:040x}\r\n+role\r\n+{role}\r\n+health\r\n+online\r\n+endpoint\r\n+127.0.0.1\r\n+port\r\n:{port}\r\n")
        }
        fn shard(start: u16, end: u16, port: u16) -> String {
            format!("*4\r\n+slots\r\n*2\r\n:{start}\r\n:{end}\r\n+nodes\r\n*1\r\n{}", node("master", port))
        }
        for replica in [false, true] {
        let seed = TcpListener::bind("127.0.0.1:0").unwrap();
        let other = TcpListener::bind("127.0.0.1:0").unwrap();
        let seed_port = seed.local_addr().unwrap().port();
        let other_port = other.local_addr().unwrap().port();
        assert!(crate::redis_cluster::command_slot(redis::cmd("INCR").arg("b")).unwrap().unwrap() < 8192);
        let seed_peer = std::thread::spawn(move || {
            let (mut socket, _) = seed.accept().unwrap();
            drop(seed);
            socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut parser = redis::Parser::new();
            assert_eq!(command(&mut socket, &mut parser)[0], b"PING");
            socket.write_all(b"+PONG\r\n").unwrap();
            assert_eq!(command(&mut socket, &mut parser), [b"CLUSTER".to_vec(), b"SHARDS".to_vec()]);
            if replica {
                write!(socket, "*1\r\n*4\r\n+slots\r\n*2\r\n:0\r\n:16383\r\n+nodes\r\n*2\r\n{}{}", node("master", seed_port), node("replica", other_port)).unwrap();
            } else {
                write!(socket, "*2\r\n{}{}", shard(0, 8191, seed_port), shard(8192, 16383, other_port)).unwrap();
            }
            assert_eq!(command(&mut socket, &mut parser), [b"INCR".to_vec(), b"b".to_vec()]);
            // Commit may have happened; lose both reply and original seed listener.
        });
        let alternate = std::thread::spawn(move || {
            let (mut socket, _) = other.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            assert_eq!(command(&mut socket, &mut redis::Parser::new()), [b"CLUSTER".to_vec(), b"SHARDS".to_vec()]);
            write!(socket, "*1\r\n{}", shard(0, 16383, other_port)).unwrap();
            assert_eq!(socket.read(&mut [0]).unwrap(), 0, "recovery must not replay or refresh");
        });
        let workspace = Workspace::new(Settings { host: "127.0.0.1".into(), port: seed_port, tls: false, cluster: true, ..Default::default() }, String::new(), egui::Context::default());
        let timeout = Duration::from_secs(4);
        assert!(matches!(workspace.receiver.recv_timeout(timeout).unwrap(), Ok(Reply::Connected(0))));
        workspace.sender.send(Job::Execute("INCR b\nINCR later".into(), false)).unwrap();
        assert!(matches!(workspace.receiver.recv_timeout(timeout).unwrap(), Ok(Reply::Recovering(error)) if error.contains("may have completed")));
        assert!(matches!(workspace.receiver.recv_timeout(timeout).unwrap(), Ok(Reply::Recovered)));
        drop(workspace);
        seed_peer.join().unwrap();
        alternate.join().unwrap();
        }
    }
    #[test]
    fn sentinel_worker_reconnects_once_without_resending_a_lost_write() {
        use std::{io::{Read, Write}, net::TcpListener, time::Duration};
        fn command(socket: &mut std::net::TcpStream, parser: &mut redis::Parser) -> Vec<Vec<u8>> {
            loop {
                let command: Vec<Vec<u8>> = redis::from_redis_value(parser.parse_value(&mut *socket).unwrap()).unwrap();
                if command[0] == b"CLIENT" { socket.write_all(b"+OK\r\n").unwrap(); } else { return command; }
            }
        }
        let seed = TcpListener::bind("127.0.0.1:0").unwrap();
        let data = TcpListener::bind("127.0.0.1:0").unwrap();
        let seed_port = seed.local_addr().unwrap().port();
        let data_port = data.local_addr().unwrap().port();
        let sentinel = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut socket, _) = seed.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                assert_eq!(command(&mut socket, &mut redis::Parser::new())[0], b"SENTINEL");
                write!(socket, "*2\r\n$9\r\n127.0.0.1\r\n${}\r\n{data_port}\r\n", data_port.to_string().len()).unwrap();
            }
        });
        let primary = std::thread::spawn(move || {
            for attempt in 0..2 {
                let (mut socket, _) = data.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut parser = redis::Parser::new();
                assert_eq!(command(&mut socket, &mut parser)[0], b"ROLE");
                assert_eq!(command(&mut socket, &mut parser)[0], b"SELECT");
                socket.write_all(b"*3\r\n$6\r\nmaster\r\n:0\r\n*0\r\n+OK\r\n").unwrap();
                if attempt == 0 {
                    assert_eq!(command(&mut socket, &mut parser), vec![b"INCR".to_vec(), b"counter".to_vec()]);
                    // Simulate a committed write with a lost response.
                } else {
                    assert_eq!(socket.read(&mut [0]).unwrap(), 0, "recovery must not replay the job or issue a refresh");
                }
            }
        });
        let workspace = Workspace::new_with_sentinel(Settings { tls: false, sentinel: Some(crate::redis_settings::Sentinel { tls: false, seeds: vec![crate::redis_settings::SentinelSeed { host: "127.0.0.1".into(), port: seed_port }], ..Default::default() }), ..Default::default() }, String::new(), String::new(), egui::Context::default());
        let timeout = Duration::from_secs(3);
        assert!(matches!(workspace.receiver.recv_timeout(timeout).unwrap(), Ok(Reply::Connected(0))));
        workspace.sender.send(Job::Execute("INCR counter\nINCR later".into(), false)).unwrap();
        assert!(matches!(workspace.receiver.recv_timeout(timeout).unwrap(), Ok(Reply::Recovering(error)) if error.contains("may have completed")));
        assert!(matches!(workspace.receiver.recv_timeout(timeout).unwrap(), Ok(Reply::Recovered)));
        drop(workspace);
        sentinel.join().unwrap();
        primary.join().unwrap();
    }
    #[test]
    #[ignore = "disposable Sentinel fixture via scripts/test-redis-sentinel.sh"]
    fn live_sentinel_workspace_recovers_after_primary_demotion_without_replay() {
        use std::time::{Duration, Instant};
        assert_eq!(std::env::var("RUSTABLE_REDIS_SENTINEL_QA").as_deref(), Ok("1"));
        let mut discovery = Session::raw_node(&Settings { host: "127.0.0.1".into(), port: 26410, tls: false, ..Default::default() }, "rustable-sentinel-test-only").unwrap();
        let target = |connection: &mut redis::Connection| -> (String, u16) {
            redis::cmd("SENTINEL").arg("get-master-addr-by-name").arg("rustable-qa").query(connection).unwrap()
        };
        let (host, port) = target(&mut discovery);
        assert_eq!(host, "127.0.0.1");
        assert!([16410, 16411, 16412].contains(&port));
        let mut old = Session::raw_node(&Settings { host, port, tls: false, ..Default::default() }, "rustable-data-test-only").unwrap();
        let key = format!("rustable:workspace-recovery:{}", std::process::id());
        redis::cmd("SET").arg(&key).arg(0).query::<()>(&mut old).unwrap();
        assert!(redis::cmd("WAIT").arg(1).arg(10000).query::<i64>(&mut old).unwrap() >= 1);
        let mut workspace = Workspace::new_with_sentinel(Settings {
            tls: false,
            sentinel: Some(crate::redis_settings::Sentinel { tls: false, service: "rustable-qa".into(), seeds: vec![crate::redis_settings::SentinelSeed { host: "127.0.0.1".into(), port: 26410 }], ..Default::default() }),
            ..Default::default()
        }, "rustable-data-test-only".into(), "rustable-sentinel-test-only".into(), egui::Context::default());
        assert!(matches!(workspace.receiver.recv_timeout(Duration::from_secs(6)).unwrap(), Ok(Reply::Connected(0))));
        workspace.connected = true;
        workspace.busy = false;
        workspace.command = "keep my editor text".into();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match redis::cmd("SENTINEL").arg("FAILOVER").arg("rustable-qa").query::<()>(&mut discovery) {
                Ok(()) => break,
                Err(error) if matches!(error.code(), Some("INPROG" | "NOGOODSLAVE")) && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
                Err(error) => panic!("Fixture failover rejected: {error}"),
            }
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        let new_port = loop {
            assert!(Instant::now() < deadline, "Sentinel did not demote the original primary");
            let (_, current) = target(&mut discovery);
            let role: redis::Value = match redis::cmd("ROLE").query(&mut old) {
                Ok(role) => role,
                Err(error) if error.is_io_error() => {
                    // Redis may close existing clients during demotion. Reopen only
                    // the fixture observer; leave the real workspace socket untouched.
                    old = Session::raw_node(&Settings { host: "127.0.0.1".into(), port, tls: false, ..Default::default() }, "rustable-data-test-only").unwrap();
                    continue;
                }
                Err(error) => panic!("Fixture ROLE check failed: {error}"),
            };
            if current != port && matches!(role, redis::Value::Array(ref values) if values.first() == Some(&redis::Value::BulkString(b"slave".to_vec()))) { break current; }
            assert!(Instant::now() < deadline, "Sentinel did not demote the original primary");
            std::thread::sleep(Duration::from_millis(100));
        };
        assert!([16410, 16411, 16412].contains(&new_port));
        workspace.sender.send(Job::Execute(format!("INCR {key}"), false)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            workspace.poll();
            if workspace.notice.as_deref().is_some_and(|notice| notice.starts_with("Reconnected.")) { break; }
            assert!(Instant::now() < deadline, "workspace did not recover: {:?}", workspace.error);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(workspace.connected);
        assert_eq!(workspace.command, "keep my editor text");
        let error = workspace.error.as_deref().unwrap();
        assert!(error.contains("not replayed") || error.contains("may have completed"), "{error}");
        let mut current = Session::raw_node(&Settings { host: "127.0.0.1".into(), port: new_port, tls: false, ..Default::default() }, "rustable-data-test-only").unwrap();
        assert_eq!(redis::cmd("GET").arg(&key).query::<i64>(&mut current).unwrap(), 0);
        workspace.sender.send(Job::Execute(format!("INCR {key}"), false)).unwrap();
        assert!(matches!(workspace.receiver.recv_timeout(Duration::from_secs(3)).unwrap(), Ok(Reply::Output(_))));
        assert_eq!(redis::cmd("GET").arg(&key).query::<i64>(&mut current).unwrap(), 1);
        redis::cmd("DEL").arg(&key).query::<()>(&mut current).unwrap();
    }
    #[test]
    fn bulk_window_close_cancels_remaining_batches() {
        let (mut workspace, _, _) = mock_workspace();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        workspace.bulk_cancel = Some(cancel.clone());
        drop(workspace);
        assert!(cancel.load(std::sync::atomic::Ordering::Acquire));
    }
    #[test]
    fn disconnected_bulk_result_does_not_refresh_or_replay() {
        let (mut workspace, responses, jobs) = mock_workspace();
        workspace.busy = true;
        workspace.bulk_cancel = Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)));
        let view = crate::redis_bulk_view::View::new(crate::redis_cluster::WriteReport {
            outcomes: vec![crate::redis_cluster::WriteOutcome { key: b"key".to_vec(), state: crate::redis_cluster::WriteState::Unknown, message: "Inspect before retrying".into() }],
            stopped: Some("Connection lost".into()),
        });
        responses.send(Ok(Reply::Bulk(view, false))).unwrap();
        workspace.poll();
        assert!(!workspace.connected);
        assert!(workspace.bulk_results.is_some());
        assert!(workspace.bulk_cancel.is_none());
        assert!(jobs.try_recv().is_err());
    }
    #[test]
    #[ignore = "manual new namespace tree CPU workload; no pre-feature tree baseline"]
    fn redis_namespace_tree_frame_benchmark() {
        let (mut w, _, _) = mock_workspace();
        w.keys = (0..10000)
            .map(|i| format!("app:users:{i:05}").into_bytes())
            .collect();
        w.expanded
            .extend([b"app:".to_vec(), b"app:users:".to_vec()]);
        for key in &w.keys {
            w.rows
                .insert(key.clone(), ("Sample".into(), "hash".into(), -1));
        }
        w.rebuild_tree();
        let ctx = egui::Context::default();
        crate::scroll_response::install(&ctx);
        let mut frames = Vec::new();
        let mut changes = Vec::new();
        for frame in 0..180 {
            let start = std::time::Instant::now();
            if frame % 20 == 0 {
                if !w.expanded.remove(b"app:users:".as_slice()) {
                    w.expanded.insert(b"app:users:".to_vec());
                }
                w.rebuild_tree();
            }
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 760.0),
                    )),
                    time: Some(frame as f64 / 120.0),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| w.key_tree(ui));
                },
            );
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            assert!(
                w.visible_indices.len() <= 30,
                "tree must remain virtualized"
            );
            if frame >= 20 {
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                frames.push(elapsed);
                if frame % 20 == 0 {
                    changes.push(elapsed);
                }
            }
        }
        frames.sort_by(f64::total_cmp);
        changes.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/tree", &frames);
        crate::ui_benchmark::report("redis/tree-toggle", &changes);
    }
    #[test]
    #[ignore = "manual Redis bulk selection CPU benchmark"]
    fn redis_bulk_selection_frame_benchmark() {
        let (mut w, _responses, _jobs) = mock_workspace();
        w.keys = (0..10000)
            .map(|i| format!("app:users:{i:05}").into_bytes())
            .collect();
        for key in &w.keys {
            w.rows
                .insert(key.clone(), ("value".into(), "string".into(), -1));
        }
        w.expanded.insert(b"app:".to_vec());
        w.expanded.insert(b"app:users:".to_vec());
        w.rebuild_tree();
        w.bulk_mode = true;
        w.bulk_selected = w.keys.iter().take(1000).cloned().collect();
        w.scan_started = true;
        let ctx = egui::Context::default();
        let mut sizes = crate::pane_sizes::PaneSizes::default();
        let mut dirty = false;
        let mut samples = Vec::new();
        for frame in 0..180 {
            if frame % 20 == 0 {
                let key = &w.keys[frame];
                if !w.bulk_selected.remove(key) {
                    w.bulk_selected.insert(key.clone());
                }
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
                    w.show(ctx, "Fixture", egui::Color32::GRAY, &mut sizes, &mut dirty);
                },
            );
            let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
            if frame >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/bulk-selection", &samples);
    }
    #[test]
    fn draft_editing_pauses_background_scan_and_close_resumes_it() {
        let (mut w, _responses, jobs) = mock_workspace();
        w.auto_scan = true;
        w.edit = Some(crate::redis_edit::Draft::new(
            crate::redis_mutation::Action::Create,
            vec![],
            "string".into(),
        ));
        let ctx = egui::Context::default();
        let mut panes = crate::pane_sizes::PaneSizes::default();
        let mut dirty = false;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            w.show(ctx, "Fixture", egui::Color32::GRAY, &mut panes, &mut dirty);
        });
        assert!(
            jobs.try_recv().is_err(),
            "typing in a draft must not compete with new scan jobs"
        );
        w.edit = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            w.show(ctx, "Fixture", egui::Color32::GRAY, &mut panes, &mut dirty);
        });
        assert!(matches!(jobs.try_recv().unwrap(), Job::Scan(..)));
    }
    #[test]
    fn inline_json_draft_blocks_navigation_but_allows_its_save() {
        let (mut w, _responses, jobs) = mock_workspace();
        let mut draft = crate::redis_edit::Draft::new(crate::redis_mutation::Action::Update, vec![b"json:key".to_vec()], "ReJSON-RL".into());
        draft.value = "{\"answer\":42}".into();
        let request = draft.request().unwrap();
        w.edit = Some(draft);
        w.send(Job::Inspect(b"other:key".to_vec()));
        assert!(jobs.try_recv().is_err());
        assert!(w.value_dirty());
        w.send(Job::Mutate(request, false));
        assert!(matches!(jobs.try_recv().unwrap(), Job::Mutate(..)));
    }
    #[test]
    fn search_navigation_does_not_start_a_request_during_an_edit() {
        let (mut w, _responses, jobs) = mock_workspace();
        w.edit = Some(crate::redis_edit::Draft::new(crate::redis_mutation::Action::Create, vec![], "string".into()));
        w.search_action(crate::redis_search::Action::Request(crate::redis_search::Request::Indexes));
        assert!(!w.search.pending);
        assert!(jobs.try_recv().is_err());
        assert!(w.edit.is_some());
    }
    #[test]
    fn search_pauses_discovery_and_failed_queries_retain_the_panel() {
        let (mut w, responses, jobs) = mock_workspace();
        w.auto_scan = true;
        w.search.open = true;
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            w.show(
                ctx,
                "Fixture",
                egui::Color32::GRAY,
                &mut Default::default(),
                &mut false,
            );
        });
        assert!(jobs.try_recv().is_err());
        w.search.begin();
        responses
            .send(Err(("Query syntax error".into(), true)))
            .unwrap();
        w.poll();
        assert!(!w.search.pending);
        assert_eq!(w.section, Section::Search);
        assert_eq!(w.search.error.as_deref(), Some("Query syntax error"));
    }
    #[test]
    fn list_row_can_be_opened_from_the_keyboard() {
        let (mut w, _responses, jobs) = mock_workspace();
        w.keys = vec![b"keyboard:key".to_vec()];
        let ctx = egui::Context::default();
        for frame in 0..12 {
            let key = if frame % 2 == 0 {
                egui::Key::Tab
            } else {
                egui::Key::Enter
            };
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 600.0),
                    )),
                    events: vec![
                        egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Default::default(),
                        },
                        egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: false,
                            repeat: false,
                            modifiers: Default::default(),
                        },
                    ],
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| w.key_grid(ui));
                },
            );
            if let Ok(Job::Inspect(key)) = jobs.try_recv() {
                assert_eq!(key, b"keyboard:key");
                return;
            }
        }
        panic!("Tab and Enter should open a key without pointer input");
    }
    #[test]
    fn tree_settles_at_idle_and_expansion_is_local() {
        let (mut w, _, jobs) = mock_workspace();
        w.keys = vec![b"app:users:1".to_vec(), b"app:users:2".to_vec()];
        w.rebuild_tree();
        let ctx = egui::Context::default();
        let mut final_delay = std::time::Duration::ZERO;
        for frame in 0..20 {
            let output = ctx.run(
                egui::RawInput {
                    time: Some(frame as f64),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| w.key_tree(ui));
                },
            );
            final_delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        }
        assert!(final_delay > std::time::Duration::from_secs(1));
        w.expanded.insert(b"app:".to_vec());
        w.rebuild_tree();
        assert_eq!(w.tree.len(), 2);
        assert!(jobs.try_recv().is_err());
    }

    fn test_inspection(kind: &str, cursor: Cursor, next: Option<Cursor>) -> Inspection {
        let snapshot = Snapshot {
            key: b"example".to_vec(),
            kind: kind.into(),
            ttl_ms: -1,
            length: Some(401),
            truncated: false,
            value: redis::Value::Array(vec![
                redis::Value::BulkString(b"Apple".to_vec()),
                redis::Value::BulkString(b"Banana".to_vec()),
            ]),
        };
        let display = crate::redis_value::Display::new(&snapshot, &cursor);
        Inspection {
            snapshot,
            page: Page {
                cursor,
                next,
                pattern: b"*".to_vec(),
                display,
            },
        }
    }
    #[test]
    fn failed_page_requests_preserve_navigation_and_success_commits_it() {
        let (mut w, responses, jobs) = mock_workspace();
        responses
            .send(Ok(Reply::Value(Box::new(test_inspection(
                "list",
                Cursor::Offset(0),
                Some(Cursor::Offset(200)),
            )))))
            .unwrap();
        w.poll();
        let request = PageRequest {
            key: b"example".to_vec(),
            kind: "list".into(),
            cursor: Cursor::Offset(200),
            pattern: b"*".to_vec(),
        };
        w.send(Job::ValuePage(request.clone(), Navigation::Next));
        jobs.recv().unwrap();
        responses.send(Err(("read failed".into(), true))).unwrap();
        w.poll();
        assert!(w.value_history.is_empty());
        assert_eq!(w.value_page.as_ref().unwrap().cursor, Cursor::Offset(0));
        w.send(Job::ValuePage(request, Navigation::Next));
        jobs.recv().unwrap();
        responses
            .send(Ok(Reply::Value(Box::new(test_inspection(
                "list",
                Cursor::Offset(200),
                None,
            )))))
            .unwrap();
        w.poll();
        assert_eq!(w.value_history, vec![Cursor::Offset(0)]);
        w.send(Job::ValuePage(
            PageRequest {
                key: b"example".to_vec(),
                kind: "list".into(),
                cursor: Cursor::Offset(0),
                pattern: b"*".to_vec(),
            },
            Navigation::Previous,
        ));
        jobs.recv().unwrap();
        responses
            .send(Ok(Reply::Value(Box::new(test_inspection(
                "list",
                Cursor::Offset(0),
                Some(Cursor::Offset(200)),
            )))))
            .unwrap();
        w.poll();
        assert!(w.value_history.is_empty());
    }
    #[test]
    fn switching_database_clears_key_scoped_diagnostics_actions() {
        let (mut workspace, responses, _jobs) = mock_workspace();
        workspace.diagnostics_open = true;
        workspace.diagnostics_request = Some(crate::redis_diagnostics::Request::VectorSimilar { key: b"old:key".to_vec(), element: b"member".to_vec() });
        workspace.diagnostics_report = Some(Box::default());
        responses.send(Ok(Reply::Connected(2))).unwrap();
        workspace.poll();
        assert_eq!(workspace.database, 2);
        assert!(!workspace.diagnostics_open);
        assert!(workspace.diagnostics_request.is_none());
        assert!(workspace.diagnostics_report.is_none());
    }
    #[test]
    fn page_filter_is_local_and_byte_patterns_are_explicit() {
        let (mut w, responses, _) = mock_workspace();
        responses
            .send(Ok(Reply::Value(Box::new(test_inspection(
                "list",
                Cursor::Offset(200),
                None,
            )))))
            .unwrap();
        w.poll();
        w.value_search = "BANANA".into();
        w.update_value_matches("list");
        assert_eq!(w.value_matches, vec![1]);
        w.update_value_matches("hash");
        assert_eq!(
            w.value_matches,
            vec![0, 1],
            "server-side drafts must not silently filter current results"
        );
        assert_eq!(collection_pattern(r#""\xff*""#).unwrap(), vec![255, b'*']);
        assert_eq!(collection_pattern("").unwrap(), b"*");
        assert!(collection_pattern("\"unterminated").is_err());
    }
    #[test]
    fn cached_string_original_still_guards_unsaved_binary_edits() {
        let (mut w, responses, jobs) = mock_workspace();
        let snapshot = Snapshot {
            key: b"binary".to_vec(),
            kind: "string".into(),
            ttl_ms: -1,
            length: Some(3),
            truncated: false,
            value: redis::Value::BulkString(vec![0, 255, b'"']),
        };
        let cursor = Cursor::Offset(0);
        let display = crate::redis_value::Display::new(&snapshot, &cursor);
        responses
            .send(Ok(Reply::Value(Box::new(Inspection {
                snapshot,
                page: Page {
                    cursor,
                    next: None,
                    pattern: b"*".to_vec(),
                    display,
                },
            }))))
            .unwrap();
        w.poll();
        assert!(!w.value_dirty());
        let original = w.value_editor.clone();
        w.value_editor = "changed".into();
        assert!(w.value_dirty());
        w.send(Job::Inspect(b"another".to_vec()));
        assert!(jobs.try_recv().is_err());
        assert!(w.error.as_ref().unwrap().contains("Save or discard"));
        w.value_editor = original;
        assert!(!w.value_dirty());
    }
    fn timeseries_ui_fixture() -> Workspace {
        let (mut workspace, responses, _) = mock_workspace();
        let snapshot = Snapshot { key: b"telemetry:temperature".to_vec(), kind: "TSDB-TYPE".into(), ttl_ms: -1, length: None, truncated: false,
            value: redis::Value::Array((0..200).map(|index| redis::Value::Array(vec![redis::Value::Int(1700000000000 + index * 1000), redis::Value::SimpleString(format!("{:.3}", 20.0 + index as f64 / 100.0))])).collect()) };
        let cursor = Cursor::Timestamp { start: 0, end: None };
        let display = crate::redis_value::Display::new(&snapshot, &cursor);
        responses.send(Ok(Reply::Value(Box::new(Inspection { snapshot, page: Page { cursor, next: Some(Cursor::Timestamp { start: 1700000199001, end: None }), pattern: b"*".to_vec(), display } })))).unwrap();
        workspace.poll();
        workspace
    }
    #[test]
    fn timeseries_inspector_settles_at_idle_with_unapplied_bounds() {
        let mut workspace = timeseries_ui_fixture();
        workspace.time_range = ["1700000001000".into(), "1700000100000".into()];
        let ctx = egui::Context::default();
        for frame in 0..40 {
            let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 760.0))), time: Some(frame as f64 + 60.0), ..Default::default() }, |ctx| {
                egui::SidePanel::right("timeseries-idle").exact_width(420.0).show(ctx, |ui| {
                    browser_style(ui);
                    egui::ScrollArea::vertical().show(ui, |ui| workspace.inspector(ui));
                });
            });
        }
        assert!(!ctx.has_requested_repaint(), "idle time-series inspector must stop repainting");
        assert!(!workspace.busy);
    }
    #[test]
    fn timeseries_chart_inspector_settles_after_focus_scroll() {
        let mut workspace = timeseries_ui_fixture();
        workspace.time_chart = true;
        workspace.time_chart_focus = true;
        let ctx = egui::Context::default();
        for frame in 0..120 {
            let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 760.0))), time: Some(frame as f64 / 30.0), ..Default::default() }, |ctx| {
                egui::SidePanel::right("timeseries-chart-idle").exact_width(420.0).show(ctx, |ui| {
                    browser_style(ui);
                    egui::ScrollArea::vertical().show(ui, |ui| workspace.inspector(ui));
                });
            });
        }
        assert!(!workspace.time_chart_focus);
        assert!(!ctx.has_requested_repaint());
        assert!(!workspace.busy);
    }
    #[test]
    #[ignore = "manual time-series range editing and scrolling CPU workload"]
    fn redis_timeseries_inspector_frame_benchmark() {
        let mut workspace = timeseries_ui_fixture();
        let ctx = egui::Context::default();
        let mut samples = Vec::new();
        for frame in 0..180 {
            workspace.time_range[0] = format!("{}", 1700000000000u64 + frame * 1000);
            let start = std::time::Instant::now();
            let output = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 760.0))), time: Some(frame as f64 / 120.0), events: vec![egui::Event::PointerMoved(egui::pos2(1000.0, 600.0)), egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, if frame % 40 < 20 { -20.0 } else { 20.0 }), modifiers: egui::Modifiers::NONE }], ..Default::default() }, |ctx| {
                egui::SidePanel::right("timeseries-benchmark").exact_width(420.0).show(ctx, |ui| {
                    browser_style(ui);
                    egui::ScrollArea::vertical().show(ui, |ui| workspace.inspector(ui));
                });
            });
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            if frame >= 20 { samples.push(start.elapsed().as_secs_f64() * 1000.0); }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/timeseries-range-scroll", &samples);
    }
    #[test]
    #[ignore = "manual new value-view interaction benchmark"]
    fn redis_value_modes_frame_benchmark() {
        let (mut w, responses, _) = mock_workspace();
        let bytes=serde_json::to_vec(&serde_json::json!({"items":(0..500).map(|i|format!("member {i} 日本語")).collect::<Vec<_>>()})).unwrap();
        let snapshot = Snapshot {
            key: b"json".to_vec(),
            kind: "string".into(),
            ttl_ms: -1,
            length: Some(bytes.len() as u64),
            truncated: false,
            value: redis::Value::BulkString(bytes),
        };
        let cursor = Cursor::Offset(0);
        let display = crate::redis_value::Display::new(&snapshot, &cursor);
        responses
            .send(Ok(Reply::Value(Box::new(Inspection {
                snapshot,
                page: Page {
                    cursor,
                    next: None,
                    pattern: b"*".to_vec(),
                    display,
                },
            }))))
            .unwrap();
        w.poll();
        let ctx = egui::Context::default();
        let mut samples = Vec::new();
        let mut first = Vec::new();
        for frame in 0..180 {
            w.string_mode = [StringMode::Text, StringMode::Json, StringMode::Hex][(frame / 20) % 3];
            let start = std::time::Instant::now();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 760.0),
                    )),
                    time: Some(frame as f64 / 120.0 + if frame >= 120 { 5.0 } else { 0.0 }),
                    ..Default::default()
                },
                |ctx| {
                    egui::SidePanel::right("value-modes")
                        .exact_width(420.0)
                        .show(ctx, |ui| {
                            browser_style(ui);
                            egui::ScrollArea::vertical().show(ui, |ui| w.inspector(ui));
                        });
                },
            );
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            if frame >= 20 {
                samples.push(elapsed);
            }
            if frame % 20 == 0 {
                first.push(elapsed);
            }
            assert!(
                !w.value_dirty(),
                "view switches must never modify editable bytes"
            );
        }
        samples.sort_by(f64::total_cmp);
        first.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/value-modes", &samples);
        crate::ui_benchmark::report("redis/value-mode-switch", &first);
    }

    #[test]
    fn namespace_tree_preserves_binary_empty_and_prefix_keys() {
        let mut keys = vec![
            b"app".to_vec(),
            b"app:".to_vec(),
            b"app:users:1".to_vec(),
            b"app:users:2".to_vec(),
            vec![255, b':', 0],
            vec![],
        ];
        keys.sort();
        let mut expanded = std::collections::HashSet::new();
        let rows = tree_rows(&keys, b":", &expanded);
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows.iter().find(|r| r.prefix_len == Some(4)).unwrap().count,
            3
        );
        expanded.extend([b"app:".to_vec(), b"app:users:".to_vec(), vec![255, b':']]);
        let rows = tree_rows(&keys, b":", &expanded);
        assert_eq!(
            rows.iter()
                .filter(|r| r.prefix_len.is_none())
                .map(|r| &keys[r.key])
                .collect::<Vec<_>>(),
            keys.iter().collect::<Vec<_>>()
        );
        assert_eq!(tree_rows(&keys, b"", &expanded).len(), keys.len());
        let keys = vec![b"a::b::c".to_vec()];
        let rows = tree_rows(&keys, b"::", &Default::default());
        assert_eq!(rows[0].prefix_len, Some(3));
    }
    #[test]
    fn sparse_duplicate_scan_and_refresh_discard_stale_results() {
        let (mut w, responses, jobs) = mock_workspace();
        w.refresh();
        assert!(matches!(jobs.recv().unwrap(), Job::Scan(0, _, _)));
        responses
            .send(Ok(Reply::Keys(KeyPage {
                cursor: 7,
                keys: vec![],
            })))
            .unwrap();
        w.poll();
        assert!(w.auto_scan);
        assert_eq!(w.cursor, 7);
        responses
            .send(Ok(Reply::Keys(KeyPage {
                cursor: 8,
                keys: vec![b"a:1".to_vec(), b"a:1".to_vec()],
            })))
            .unwrap();
        w.poll();
        assert_eq!(w.keys.len(), 1);
        w.expanded.insert(b"a:".to_vec());
        w.selected_key = Some(b"a:1".to_vec());
        w.send(Job::Scan(8, b"*".to_vec(), String::new()));
        jobs.recv().unwrap();
        w.pattern = "b:*".into();
        w.refresh();
        assert!(w.refresh_pending);
        w.pattern = "unsubmitted:*".into();
        responses
            .send(Ok(Reply::Keys(KeyPage {
                cursor: 0,
                keys: vec![b"stale".to_vec()],
            })))
            .unwrap();
        w.poll();
        assert!(w.keys.is_empty());
        assert!(w.expanded.contains(b"a:".as_slice()));
        assert_eq!(w.selected_key.as_deref(), Some(b"a:1".as_slice()));
        assert!(matches!(jobs.recv().unwrap(), Job::Scan(0, p, _) if p == b"b:*"));
    }
    #[test]
    fn page_enrichment_keeps_the_existing_batch_size_and_admission_limit() {
        let (mut w, responses, _) = mock_workspace();
        let keys: Vec<_> = (0..200)
            .map(|i| format!("app:{i:03}").into_bytes())
            .collect();
        responses
            .send(Ok(Reply::Keys(KeyPage {
                cursor: 7,
                keys: keys.clone(),
            })))
            .unwrap();
        w.poll();
        assert_eq!(w.keys, keys);
        assert_eq!(w.preview_queue.len(), 200);
        assert!(w.rows.is_empty(), "names are published before preview work");
        w.keys = (0..10000)
            .map(|i| format!("key:{i:05}").into_bytes())
            .collect();
        w.preview_queue.clear();
        responses
            .send(Ok(Reply::Keys(KeyPage {
                cursor: 0,
                keys: vec![b"overflow".to_vec()],
            })))
            .unwrap();
        w.poll();
        assert!(w.key_limit);
        assert!(w.preview_queue.is_empty());
    }
    #[test]
    fn selection_queues_during_scan_and_dirty_values_block_refresh() {
        let (mut w, _, jobs) = mock_workspace();
        w.refresh();
        jobs.recv().unwrap();
        w.send(Job::Inspect(b"chosen".to_vec()));
        assert!(matches!(&w.pending, Some(Job::Inspect(key)) if key == b"chosen"));
        w.busy = false;
        w.snapshot = Some(Snapshot {
            key: b"chosen".to_vec(),
            kind: "string".into(),
            ttl_ms: -1,
            length: Some(3),
            value: redis::Value::BulkString(b"old".to_vec()),
            truncated: false,
        });
        w.value_editor = "\"new\"".into();
        w.refresh();
        assert!(w.error.as_ref().unwrap().contains("Save or discard"));
        assert!(jobs.try_recv().is_err());
    }
    #[test]
    fn direct_lookup_is_binary_safe_and_does_not_scan() {
        assert_eq!(exact_key_bytes("a b").unwrap(), b"a b");
        assert_eq!(exact_key_bytes(r#""\xff\x00""#).unwrap(), vec![255, 0]);
        assert!(exact_key_bytes(r#""a" "b""#).is_err());
        let (mut w, _, jobs) = mock_workspace();
        w.send(Job::Inspect(exact_key_bytes("missing").unwrap()));
        assert!(matches!(jobs.recv().unwrap(), Job::Inspect(key) if key == b"missing"));
    }

    #[test]
    fn grid_previews_render_strings_hashes_and_binary_safely() {
        let bulk = |text: &str| redis::Value::BulkString(text.as_bytes().to_vec());
        assert_eq!(grid_preview(&bulk("hello"), "string"), "hello");
        assert_eq!(
            grid_preview(
                &redis::Value::Array(vec![bulk("name"), bulk("Alice")]),
                "hash"
            ),
            r#"{"name":"Alice"}"#
        );
        assert_eq!(
            grid_preview(&redis::Value::BulkString(vec![255, 0]), "string"),
            r#""\xff\x00""#
        );
        assert!(grid_preview(&bulk(&"x".repeat(300)), "string").ends_with('…'));
    }
    #[test]
    fn reply_previews_distinguish_empty_nil_and_preserve_binary() {
        assert_ne!(
            render_values(&[redis::Value::Nil]),
            render_values(&[redis::Value::Array(vec![])])
        );
        assert_eq!(
            render_values(&[redis::Value::BulkString(vec![255, 0])]),
            "\"\\xff\\x00\"\n"
        );
        let output = render_values(&[redis::Value::Array(vec![
            redis::Value::BulkString(vec![
                255;
                65536
            ]);
            20
        ])]);
        assert!(output.len() < 256 * 1024 + 100);
        assert!(output.contains("[output truncated]"));
    }
}

include!("redis_browser_benchmark.rs");

pub(crate) fn action_button(icon: &str, label: &'static str) -> egui::Button<'static> {
    let image = match icon {
        "chevron-left" => egui::include_image!("../assets/icons/lucide/chevron-left.svg"),
        "chevron-right" => egui::include_image!("../assets/icons/lucide/chevron-right.svg"),
        "clipboard" => egui::include_image!("../assets/icons/lucide/clipboard.svg"),
        "search" => egui::include_image!("../assets/icons/lucide/search.svg"),
        "plus" => egui::include_image!("../assets/icons/lucide/plus.svg"),
        "x" => egui::include_image!("../assets/icons/lucide/x.svg"),
        "file-code" => egui::include_image!("../assets/icons/lucide/file-code.svg"),
        "check" => egui::include_image!("../assets/icons/lucide/check.svg"),
        "pencil" => egui::include_image!("../assets/icons/lucide/pencil.svg"),
        "folder" => egui::include_image!("../assets/icons/lucide/folder.svg"),
        "list-upload" => egui::include_image!("../assets/icons/lucide/list-upload.svg"),
        "trash-2" => egui::include_image!("../assets/icons/lucide/trash-2.svg"),
        "arrow-right" => egui::include_image!("../assets/icons/lucide/arrow-right.svg"),
        _ => egui::include_image!("../assets/icons/lucide/refresh-cw.svg"),
    };
    egui::Button::image_and_text(
        egui::Image::new(image).fit_to_exact_size(egui::vec2(14.0, 14.0)),
        label,
    )
    .image_tint_follows_text_color(true)
    .min_size(egui::vec2(0.0, crate::controls::BUTTON_HEIGHT))
}

fn collection_draft(
    snapshot: &Snapshot,
    page: &Page,
    index: Option<usize>,
    action: crate::redis_mutation::Action,
) -> Option<crate::redis_edit::Draft> {
    use crate::redis_mutation::Action;
    let mut draft =
        crate::redis_edit::Draft::new(action, vec![snapshot.key.clone()], snapshot.kind.clone());
    draft.length = snapshot.length.unwrap_or(0);
    if action == Action::Add {
        return Some(draft);
    }
    let redis::Value::Array(rows) = &snapshot.value else {
        return None;
    };
    let i = index?;
    let raw = |value: &redis::Value| match value {
        redis::Value::BulkString(b) => b.clone(),
        _ => crate::redis_value::text(value).into_bytes(),
    };
    let (field, value) = match snapshot.kind.as_str() {
        "hash" | "zset" => (raw(rows.get(i * 2)?), raw(rows.get(i * 2 + 1)?)),
        "list" => (
            (page.cursor.offset() + i as u64).to_string().into_bytes(),
            raw(rows.get(i)?),
        ),
        "set" => {
            let v = raw(rows.get(i)?);
            (v.clone(), v)
        }
        "stream" => {
            let redis::Value::Array(parts) = rows.get(i)? else {
                return None;
            };
            (raw(parts.first()?), vec![])
        }
        _ => return None,
    };
    draft.field = crate::redis_commands::quote(&field);
    draft.value = crate::redis_commands::quote(&value);
    draft.expected = value;
    Some(draft)
}

include!("redis_layout.rs");

#[cfg(test)]
impl Workspace {
    fn prepare_results_benchmark(&mut self, values: Vec<redis::Value>) {
        let commands = (0..values.len()).map(|i| vec![b"GET".to_vec(), format!("app:users:{i:04}").into_bytes()]).collect::<Vec<_>>();
        let mut values = values.into_iter();
        self.command_results = crate::redis_command_results::execute(&commands, |_| Ok(values.next().unwrap()));
        self.output = self.command_results.text();
    }
}
