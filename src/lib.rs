//! Shared Rustable application UI and platform services.
//! Native and browser entry points must host this UI rather than duplicate it.

mod app;
#[cfg(target_arch = "wasm32")]
mod browser_jobs;
pub use app::Rustable;
mod column_widths;
mod completion;
mod connection;
mod connection_probe;
mod connection_profiles;
mod credential_store;
mod db;
mod editor;
mod editor_language;
mod find;
mod fonts;
mod native_menu;
mod pane_sizes;
mod result_chart;
mod result_pins;
mod result_search;
mod review_sql;
mod script_file;
mod scroll_response;
mod session_windows;
mod sql;
mod sql_format;
mod sql_language_server;
mod sqlite_language_server;
mod ssh_tunnel;
mod updates;

#[cfg(not(target_arch = "wasm32"))]
pub fn run_native() -> eframe::Result {
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
