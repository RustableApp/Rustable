//! Temporary explicit capability errors while browser file adapters are implemented.
//! Never silently invoke native threads or filesystem dialogs in a browser.
use super::*;
impl Rustable {
    pub(super) fn browser_file_unavailable(&self, ctx: &egui::Context) {
        let _ = self.files_tx.send(FileAction::QueryLibraryError("Browser file access is not implemented yet. Use the desktop app for file import/export.".into()));
        ctx.request_repaint();
    }
    pub(super) fn preview_csv_dialog(&self, ctx: &egui::Context) { self.browser_file_unavailable(ctx); }
    pub(super) fn export_table_dialog(&self, ctx: &egui::Context, _format: db::ExportFormat) { self.browser_file_unavailable(ctx); }
    pub(super) fn script_dialog(&self, ctx: &egui::Context, _save: bool) { self.browser_file_unavailable(ctx); }
    pub(super) fn json_export_dialog(&self, ctx: &egui::Context) { self.browser_file_unavailable(ctx); }
    pub(super) fn file_dialog(&self, ctx: &egui::Context, _export: bool) { self.browser_file_unavailable(ctx); }
    pub(super) fn choose_query_directory(&self, ctx: &egui::Context) { self.browser_file_unavailable(ctx); }
}
