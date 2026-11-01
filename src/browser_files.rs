//! Browser file transfers; imported files are copies, never owned disk paths.
//! Never silently invoke native threads or filesystem dialogs in a browser.
use super::*;
use wasm_bindgen::prelude::*;
#[wasm_bindgen(module = "/src/browser_files.js")]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn pick_sql() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    fn download_sql(name: &str, sql: &str) -> Result<(), JsValue>;
    #[wasm_bindgen(catch)]
    fn download_bytes(name: &str, bytes: &[u8], mime: &str) -> Result<(), JsValue>;
}

impl Rustable {
    pub(super) fn browser_file_unavailable(&self, ctx: &egui::Context) {
        let _ = self.files_tx.send(FileAction::QueryLibraryError("Browser file access is not implemented yet. Use the desktop app for file import/export.".into()));
        ctx.request_repaint();
    }
    pub(super) fn preview_csv_dialog(&self, ctx: &egui::Context) { self.browser_file_unavailable(ctx); }
    pub(super) fn export_table_dialog(&self, ctx: &egui::Context, _format: db::ExportFormat) { self.browser_file_unavailable(ctx); }
    pub(super) fn script_dialog(&self, ctx: &egui::Context, save: bool) {
        let tx = self.files_tx.clone();
        let ctx = ctx.clone();
        if save {
            let tab = &self.tabs[self.tab];
            let name = tab.source_name.clone().unwrap_or_else(|| "query.sql".into());
            let result = if tab.sql.len() as u64 > crate::script_file::MAX_BYTES {
                Err("SQL editor files are limited to 8 MiB".into())
            } else { download_sql(&name, &tab.sql).map_err(|e| format!("SQL download failed: {e:?}")) };
            let _ = tx.send(match result { Ok(()) => FileAction::BrowserDownload(name), Err(error) => FileAction::QueryLibraryError(error) });
            ctx.request_repaint();
            return;
        }
        wasm_bindgen_futures::spawn_local(async move {
            let result = async {
                let value = pick_sql().await.map_err(|e| format!("SQL import failed: {e:?}"))?;
                if value.is_null() { return Ok(None); }
                let name = js_sys::Reflect::get(&value, &"name".into()).ok().and_then(|v| v.as_string()).ok_or("Missing SQL filename")?;
                let bytes = js_sys::Reflect::get(&value, &"bytes".into()).map_err(|_| "Missing SQL content")?;
                let sql = crate::script_file::decode(js_sys::Uint8Array::new(&bytes).to_vec())?;
                Ok::<_, String>(Some(FileAction::BrowserScript(name, sql)))
            }.await;
            match result {
                Ok(Some(action)) => { let _ = tx.send(action); }
                Ok(None) => {}
                Err(error) => { let _ = tx.send(FileAction::QueryLibraryError(error)); }
            }
            ctx.request_repaint();
        });
    }
    fn export_browser_results(&self, ctx: &egui::Context, format: db::ExportFormat) {
        use crate::browser_jobs::{self, Request, Response};
        if self.browser_export.borrow().is_some() { return; }
        let tx = self.files_tx.clone();
        let (name, mime) = match format { db::ExportFormat::Csv => ("results.csv", "text/csv;charset=utf-8"), db::ExportFormat::Json => ("results.json", "application/json") };
        match browser_jobs::start(Request::Export { grid: self.grid(), format }, ctx.clone(), move |reply| {
            let result = reply.and_then(|reply| match reply {
                Response::Exported(bytes) => download_bytes(name, &bytes, mime).map(|()| name.to_owned()).map_err(|e| format!("Download failed: {e:?}")),
                _ => Err("Unexpected export response".into()),
            });
            let _ = tx.send(FileAction::BrowserExportDone(result));
        }) {
            Ok(worker) => *self.browser_export.borrow_mut() = Some(worker),
            Err(error) => { let _ = self.files_tx.send(FileAction::QueryLibraryError(error)); ctx.request_repaint(); }
        }
    }
    pub(super) fn json_export_dialog(&self, ctx: &egui::Context) { self.export_browser_results(ctx, db::ExportFormat::Json); }
    pub(super) fn file_dialog(&self, ctx: &egui::Context, export: bool) {
        if export { self.export_browser_results(ctx, db::ExportFormat::Csv); } else { self.browser_file_unavailable(ctx); }
    }
    pub(super) fn choose_query_directory(&self, ctx: &egui::Context) { self.browser_file_unavailable(ctx); }
}
