//! Shared Rustable application UI and platform services.
//! Native and browser entry points must host this UI rather than duplicate it.

mod app;
#[cfg(target_arch = "wasm32")]
mod browser_jobs;
#[cfg(target_arch = "wasm32")]
mod browser_library;
pub use app::Rustable;
mod column_widths;
mod completion;
mod connection;
mod connection_probe;
mod connection_profiles;
mod context_click;
mod controls;
#[cfg(not(target_arch = "wasm32"))]
mod crash_log;
mod credential_store;
mod date_picker;
mod db;
mod editor;
mod editor_language;
mod find;
mod fonts;
mod icons;
mod json_editor;
mod line_comment;
mod native_menu;
mod pane_sizes;
mod query_notification;
mod redis_analytics;
#[cfg(not(target_arch = "wasm32"))]
mod redis_archive;
#[cfg(not(target_arch = "wasm32"))]
mod redis_topology;
#[cfg(not(target_arch = "wasm32"))]
mod redis_cluster;
#[cfg(not(target_arch = "wasm32"))]
mod redis_sentinel;
#[cfg(not(target_arch = "wasm32"))]
mod redis_bulk_view;
#[cfg(not(target_arch = "wasm32"))]
mod redis_archive_view;
mod redis_commands;
mod redis_command_editor;
mod redis_completion;
mod redis_capabilities;
#[cfg(any(test, target_arch = "wasm32"))]
mod redis_demo;
mod redis_diagnostics;
mod redis_analysis;
#[cfg(not(target_arch = "wasm32"))]
mod redis_analysis_view;
mod redis_vector_element;
mod redis_vector_matches;
mod redis_vector_search;
#[cfg(not(target_arch = "wasm32"))]
mod redis_module_info;
#[cfg(not(target_arch = "wasm32"))]
mod redis_vectors;
mod redis_edit;
#[cfg(not(target_arch = "wasm32"))]
mod redis_monitor;
mod redis_mutation;
mod redis_json_path;
mod redis_preferences;
mod redis_history;
mod redis_command_results;
#[cfg(all(feature = "qa", not(target_arch = "wasm32")))]
mod redis_preview;
#[cfg(not(target_arch = "wasm32"))]
mod redis_pubsub;
#[cfg(not(target_arch = "wasm32"))]
mod redis_pubsub_frame;
#[cfg(not(target_arch = "wasm32"))]
mod redis_pubsub_view;
mod redis_search;
mod redis_search_catalog;
mod redis_search_manage;
#[cfg(not(target_arch = "wasm32"))]
mod redis_session;
mod redis_settings;
#[cfg(not(target_arch = "wasm32"))]
mod redis_stream;
mod redis_value;
mod redis_timeseries_chart;
mod redis_workspace;
mod result_chart;
mod result_edit;
mod result_pins;
mod result_search;
mod review_sql;
mod script_file;
mod scroll_response;
mod search;
#[cfg(not(target_arch = "wasm32"))]
mod tableplus_import;
#[cfg(not(target_arch = "wasm32"))]
mod tablepro_import;
#[cfg(target_arch = "wasm32")]
use redis_demo as redis_session;
mod session_windows;
mod sql;
mod sql_format;
mod sql_language_server;
mod sqlite_language_server;
mod ssh_prompt;
mod ssh_tunnel;
pub use ssh_prompt::helper as ssh_askpass_helper;
mod updates;

#[cfg(not(target_arch = "wasm32"))]
fn application_icon() -> eframe::egui::IconData {
    // eframe sets the macOS Dock icon at runtime, overriding the bundle icon.
    // Always supply our artwork, including when launched through cargo run.
    #[cfg(target_os = "macos")]
    let bytes = include_bytes!("../assets/rustable-macos-1024.png").as_slice();
    #[cfg(not(target_os = "macos"))]
    let bytes = include_bytes!("../assets/rustable-logo-1024.png").as_slice();
    eframe::icon_data::from_png_bytes(bytes)
        .expect("embedded Rustable application icon must be a valid PNG")
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_native() -> eframe::Result {
    crash_log::install();
    // Winit builds About/Hide/Quit labels from NSProcessInfo, not the window
    // title or bundle display name. Set this before it creates the native menu.
    #[cfg(target_os = "macos")]
    objc2_foundation::NSProcessInfo::processInfo()
        .setProcessName(&objc2_foundation::NSString::from_str("Rustable"));
    #[cfg(feature = "qa")]
    if std::env::var_os("RUSTABLE_SENTINEL_CAPTURE").is_some() {
        return app::Rustable::run_sentinel_preview();
    }
    #[cfg(feature = "qa")]
    if std::env::var_os("RUSTABLE_REDIS_PREVIEW").is_some() {
        return redis_preview::run();
    }
    #[cfg(feature = "qa")]
    if std::env::var_os("RUSTABLE_UI_PREVIEW").is_some() {
        return eframe::run_native(
            "Rustable UI Preview",
            eframe::NativeOptions {
                viewport: eframe::egui::ViewportBuilder::default()
                    .with_icon(application_icon())
                    .with_title("Rustable UI Preview")
                    .with_inner_size([1280.0, 820.0]),
                vsync: true,
                ..Default::default()
            },
            Box::new(|cc| {
                crate::scroll_response::install(&cc.egui_ctx);
                Ok(Box::new(app::Rustable::new_demo(cc)))
            }),
        );
    }
    eframe::run_native(
        if cfg!(feature = "qa") {
            "Rustable QA"
        } else if cfg!(debug_assertions) {
            "Rustable Development"
        } else {
            "Rustable"
        },
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_icon(application_icon())
                .with_title("Rustable")
                .with_inner_size(session_windows::LAUNCHER_SIZE)
                .with_min_inner_size([760.0, 560.0]),
            vsync: true,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(session_windows::SessionWindows::new(cc)))),
    )
}

#[cfg(target_arch = "wasm32")]
pub fn browser_database_command(bytes: &[u8]) -> Vec<u8> {
    db::browser::execute(bytes)
}

#[cfg(target_arch = "wasm32")]
pub fn browser_background_job(bytes: &[u8]) -> Vec<u8> {
    browser_jobs::execute(bytes)
}

/// Present browser errors without leaking implementation stack traces into UI.
#[cfg(target_arch = "wasm32")]
pub(crate) fn browser_error(error: wasm_bindgen::JsValue) -> String {
    let message = error
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(&error, &"message".into())
                .ok()
                .and_then(|value| value.as_string())
        })
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| "Browser operation failed".into());
    message.chars().take(500).collect()
}

#[cfg(test)]
mod ui_benchmark;

mod redis_value_editor;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod redis_vector_benchmark;
