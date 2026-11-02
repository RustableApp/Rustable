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
#[cfg(not(target_arch = "wasm32"))]
mod tableplus_import;
mod credential_store;
mod db;
mod editor;
mod editor_language;
mod find;
mod fonts;
mod icons;
mod native_menu;
mod pane_sizes;
mod result_chart;
mod result_pins;
mod result_search;
mod review_sql;
mod script_file;
mod scroll_response;
mod redis_commands;
mod redis_settings;
#[cfg(not(target_arch = "wasm32"))]
mod redis_session;
mod redis_workspace;
#[cfg(any(test, target_arch = "wasm32"))]
mod redis_demo;
#[cfg(target_arch = "wasm32")]
use redis_demo as redis_session;
mod session_windows;
mod sql;
mod sql_format;
mod sql_language_server;
mod sqlite_language_server;
mod ssh_tunnel;
mod updates;

#[cfg(not(target_arch = "wasm32"))]
fn application_icon() -> eframe::egui::IconData {
    // eframe sets the macOS Dock icon at runtime, overriding the bundle icon.
    // Always supply our artwork, including when launched through cargo run.
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/rustable-logo-1024.png"))
        .expect("embedded Rustable application icon must be a valid PNG")
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_native() -> eframe::Result {
    // Winit builds About/Hide/Quit labels from NSProcessInfo, not the window
    // title or bundle display name. Set this before it creates the native menu.
    #[cfg(target_os = "macos")]
    objc2_foundation::NSProcessInfo::processInfo()
        .setProcessName(&objc2_foundation::NSString::from_str("Rustable"));
    #[cfg(feature = "qa")]
    if std::env::var_os("RUSTABLE_UI_PREVIEW").is_some() {
        return eframe::run_native("Rustable UI Preview", eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default().with_icon(application_icon()).with_title("Rustable UI Preview").with_inner_size([1280.0, 820.0]),
            vsync: true, ..Default::default()
        }, Box::new(|cc| {
            crate::scroll_response::install(&cc.egui_ctx);
            Ok(Box::new(app::Rustable::new_demo(cc)))
        }));
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
                .with_title("Connections — Rustable")
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
    let message = error.as_string().or_else(|| {
        js_sys::Reflect::get(&error, &"message".into()).ok().and_then(|value| value.as_string())
    }).filter(|message| !message.trim().is_empty()).unwrap_or_else(|| "Browser operation failed".into());
    message.chars().take(500).collect()
}
