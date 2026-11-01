//! Browser host for Rustable's actual shared application, not a second UI.
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub async fn start(canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
    eframe::WebRunner::new().start(
        canvas,
        eframe::WebOptions::default(),
        Box::new(|cc| Ok(Box::new(rustable::Rustable::new_demo(cc)))),
    ).await
}

#[wasm_bindgen]
pub fn database_command(bytes: &[u8]) -> Vec<u8> {
    rustable::browser_database_command(bytes)
}

#[wasm_bindgen]
pub fn background_job(bytes: &[u8]) -> Vec<u8> {
    rustable::browser_background_job(bytes)
}
