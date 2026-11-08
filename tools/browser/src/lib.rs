//! Browser host for Rustable's actual shared application, not a second UI.
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub async fn start(canvas: web_sys::HtmlCanvasElement, redis: bool) -> Result<(), JsValue> {
    eframe::WebRunner::new().start(
        canvas,
        eframe::WebOptions::default(),
        Box::new(move |cc| Ok(Box::new(if redis { rustable::Rustable::new_redis_demo(cc) } else { rustable::Rustable::new_demo(cc) }))),
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
