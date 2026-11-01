//! Async browser-local query library. All writes commit atomically in IndexedDB.
use wasm_bindgen::prelude::*;
#[wasm_bindgen(module = "/src/browser_library.js")]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn library_request(json: &str) -> Result<JsValue, JsValue>;
}
pub(crate) async fn request(value: serde_json::Value) -> Result<serde_json::Value, String> {
    let response = library_request(&value.to_string()).await.map_err(|e| format!("Browser query storage: {}", crate::browser_error(e)))?;
    serde_json::from_str(&response.as_string().ok_or("Invalid query storage response")?).map_err(|e| e.to_string())
}
