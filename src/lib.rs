//! Shared Rustable application UI and platform services.
//! Native and browser entry points must host this UI rather than duplicate it.

mod app;
#[cfg(unix)]
pub mod matrix;
#[cfg(unix)]
mod matrix_connector;
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
mod redis_analysis;
#[cfg(not(target_arch = "wasm32"))]
mod redis_analysis_view;
mod redis_analytics;
#[cfg(not(target_arch = "wasm32"))]
mod redis_archive;
#[cfg(not(target_arch = "wasm32"))]
mod redis_archive_view;
#[cfg(not(target_arch = "wasm32"))]
mod redis_bulk_view;
mod redis_capabilities;
#[cfg(not(target_arch = "wasm32"))]
mod redis_cluster;
mod redis_command_editor;
mod redis_command_results;
mod redis_commands;
mod redis_completion;
#[cfg(any(test, target_arch = "wasm32"))]
mod redis_demo;
mod redis_diagnostics;
mod redis_edit;
mod redis_history;
mod redis_json_path;
#[cfg(not(target_arch = "wasm32"))]
mod redis_module_info;
#[cfg(not(target_arch = "wasm32"))]
mod redis_monitor;
mod redis_mutation;
mod redis_preferences;
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
mod redis_sentinel;
#[cfg(not(target_arch = "wasm32"))]
mod redis_session;
mod redis_settings;
#[cfg(not(target_arch = "wasm32"))]
mod redis_stream;
mod redis_timeseries_chart;
#[cfg(not(target_arch = "wasm32"))]
mod redis_topology;
mod redis_value;
mod redis_vector_element;
mod redis_vector_matches;
mod redis_vector_search;
#[cfg(not(target_arch = "wasm32"))]
mod redis_vectors;
mod redis_workspace;
mod result_chart;
mod result_edit;
mod result_pages;
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
mod solarized;
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
    // Keep macOS in charge of the packaged icon's sizing and appearance in
    // Finder, the Dock and the app switcher. Supplying pixels here replaces
    // NSApplication.applicationIconImage with an unprocessed bitmap. `None`
    // would select eframe's default icon; its empty IconData sentinel preserves
    // the native bundle icon instead.
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSBundle, NSString};
        let bundle = NSBundle::mainBundle();
        if bundle.bundlePath().to_string().ends_with(".app")
            && bundle.pathForResource_ofType(
                Some(&NSString::from_str("Rustable")),
                Some(&NSString::from_str("icns")),
            ).is_some()
        {
            return eframe::egui::IconData::default();
        }
    }
    // Unbundled `cargo run` still needs an icon; other platforms are unchanged.
    #[cfg(target_os = "macos")]
    let bytes = include_bytes!("../assets/rustable-macos-1024.png").as_slice();
    #[cfg(not(target_os = "macos"))]
    let bytes = include_bytes!("../assets/rustable-logo-1024.png").as_slice();
    eframe::icon_data::from_png_bytes(bytes)
        .expect("embedded Rustable application icon must be a valid PNG")
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_native() -> eframe::Result {
    run_native_mode(#[cfg(unix)] None)
}

#[cfg(not(target_arch = "wasm32"))]
fn run_native_mode(#[cfg(unix)] matrix_socket: Option<std::path::PathBuf>) -> eframe::Result {
    #[cfg(feature = "qa")]
    if std::env::var_os("RUSTABLE_NATIVE_PROFILE").is_some() {
        return session_windows::native_performance::run();
    }
    #[cfg(feature = "qa")]
    if std::env::var_os("RUSTABLE_TLS_CAPTURE").is_some() {
        return app::Rustable::run_connection_tls_preview();
    }
    crash_log::install();
    // Winit builds About/Hide/Quit labels from NSProcessInfo, not the window
    // title or bundle display name. Set this before it creates the native menu.
    #[cfg(target_os = "macos")]
    objc2_foundation::NSProcessInfo::processInfo()
        .setProcessName(&objc2_foundation::NSString::from_str("Rustable"));
    #[cfg(feature = "qa")]
    if std::env::var_os("RUSTABLE_RESULT_CAPTURE").is_some() {
        return app::Rustable::run_result_pages_preview();
    }
    #[cfg(feature = "qa")]
    if std::env::var_os("RUSTABLE_SHARE_CAPTURE").is_some() {
        return app::Rustable::run_share_preview();
    }
    #[cfg(feature = "qa")]
    if std::env::var_os("RUSTABLE_TABLE_EXPORT_CAPTURE").is_some() {
        return app::Rustable::run_table_export_preview();
    }
    #[cfg(feature = "qa")]
    if std::env::var_os("RUSTABLE_QUERY_CAPTURE").is_some() {
        return app::Rustable::run_query_layout_preview();
    }
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
    // Keep the directory alive for the entire event loop. Normal launches expose
    // setup details; individual SQL workspaces still require explicit opt-in.
    #[cfg(unix)]
    let matrix_socket = matrix::LaunchSocket::new(matrix_socket)
        .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
    #[cfg(unix)]
    let matrix_path = matrix_socket.path.clone();
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
        Box::new(move |cc| {
            #[allow(unused_mut)]
            let mut windows = session_windows::SessionWindows::new(cc);
            #[cfg(unix)]
            windows.attach_matrix(matrix::listen(&matrix_path, cc.egui_ctx.clone()).map_err(std::io::Error::other)?);
            Ok(Box::new(windows))
        }),
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

/// Open the normal connection manager with opt-in Matrix controls.
#[cfg(unix)]
pub fn run_matrix(path: std::path::PathBuf) -> eframe::Result {
    run_native_mode(Some(path))
}

/// Standalone disposable workspace used by the scripted tour.
#[cfg(unix)]
pub fn run_matrix_demo(path: std::path::PathBuf) -> eframe::Result {
    eframe::run_native(
        "Rustable Matrix Experiment",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_title("Rustable · Matrix experiment")
                .with_inner_size([1280.0, 820.0])
                .with_min_inner_size([760.0, 560.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            crate::scroll_response::install(&cc.egui_ctx);
            let bridge =
                matrix::listen(&path, cc.egui_ctx.clone()).map_err(std::io::Error::other)?;
            let mut app = app::Rustable::new_demo(cc);
            #[cfg(feature = "qa")]
            if let Some(fixture) = std::env::var_os("RUSTABLE_MATRIX_FIXTURE") {
                app = app.new_window(&cc.egui_ctx, session_windows::WindowRequest {
                    recover_sql: false, identity: None, title: "Matrix SQLite fixture".into(),
                    connection: session_windows::WindowConnection::Sqlite(fixture.into()),
                });
            }
            app.attach_matrix(bridge);
            #[cfg(feature = "qa")]
            if std::env::var_os("RUSTABLE_MATRIX_FIXTURE").is_some() { app.prepare_matrix_fixture_capture(); }
            #[cfg(feature = "qa")]
            if let Some(path) = std::env::var_os("RUSTABLE_MATRIX_CAPTURE") {
                cc.egui_ctx
                    .set_theme(if std::env::var_os("RUSTABLE_MATRIX_LIGHT").is_some() {
                        eframe::egui::Theme::Light
                    } else {
                        eframe::egui::Theme::Dark
                    });
                if std::env::var_os("RUSTABLE_MATRIX_NARROW").is_some() {
                    cc.egui_ctx
                        .send_viewport_cmd(eframe::egui::ViewportCommand::InnerSize(
                            eframe::egui::vec2(760.0, 760.0),
                        ));
                }
                return Ok(Box::new(matrix::Capture {
                    app,
                    path: path.into(),
                    requested: false,
                    saved: None,
                }));
            }
            Ok(Box::new(app))
        }),
    )
}
