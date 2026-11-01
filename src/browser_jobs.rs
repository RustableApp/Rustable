//! Browser background jobs reuse the native formatter and result-search algorithms.
use crate::{
    db::{Grid, transport::codec},
    sql::Dialect,
};
use bincode::Options;
use serde::{Deserialize, Serialize};
use std::{ops::Range, sync::Arc};

#[derive(Serialize, Deserialize)]
pub(crate) enum Request {
    Export { grid: Arc<Grid>, format: crate::db::ExportFormat },
    Format {
        sql: String,
        selection: Range<usize>,
        dialect: Dialect,
    },
    Search {
        grid: Arc<Grid>,
        needle: String,
        case_sensitive: bool,
    },
}
#[derive(Serialize, Deserialize)]
pub(crate) enum Response {
    Exported(Vec<u8>),
    Formatted(crate::sql_format::Formatted),
    Found(crate::result_search::Found),
}
pub(crate) fn execute(bytes: &[u8]) -> Vec<u8> {
    let result = codec()
        .deserialize::<Request>(bytes)
        .map_err(|e| e.to_string())
        .and_then(|request| match request {
            Request::Export { grid, format } => crate::db::browser_export::encode(&grid, format).map(Response::Exported),
            Request::Format {
                sql,
                selection,
                dialect,
            } => {
                crate::sql_format::format_document(sql, selection, dialect).map(Response::Formatted)
            }
            Request::Search {
                grid,
                needle,
                case_sensitive,
            } => Ok(Response::Found(crate::result_search::scan(
                &grid,
                needle,
                case_sensitive,
            ))),
        });
    codec().serialize(&result).unwrap_or_else(|e| {
        codec()
            .serialize(&Err::<Response, _>(e.to_string()))
            .expect("small error")
    })
}

use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{JsCast, prelude::*};
pub(crate) struct Handle {
    worker: web_sys::Worker,
    began: web_time::Instant,
    _message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _error: Closure<dyn FnMut(web_sys::ErrorEvent)>,
}
impl Handle {
    pub fn expired(&self) -> bool {
        self.began.elapsed().as_secs() >= 10
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.worker.terminate();
    }
}

pub(crate) fn start(
    request: Request,
    ctx: eframe::egui::Context,
    reply: impl FnMut(Result<Response, String>) + 'static,
) -> Result<Handle, String> {
    let bytes = codec().serialize(&request).map_err(|e| e.to_string())?;
    let options = web_sys::WorkerOptions::new();
    options.set_type(web_sys::WorkerType::Module);
    let worker = web_sys::Worker::new_with_options("./background-worker.js", &options)
        .map_err(|e| format!("Could not start browser worker: {}", crate::browser_error(e)))?;
    ctx.request_repaint_after(std::time::Duration::from_secs(10));
    let callback = Rc::new(RefCell::new(reply));
    let completed = callback.clone();
    let repaint = ctx.clone();
    let message = Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
        let result = crate::db::transport::message_bytes(event.data())
            .and_then(|bytes| codec().deserialize::<Result<Response, String>>(&bytes).map_err(|e| e.to_string()))
            .and_then(|r| r);
        completed.borrow_mut()(result);
        repaint.request_repaint();
    }) as Box<dyn FnMut(_)>);
    let error = Closure::wrap(Box::new(move |event: web_sys::ErrorEvent| {
        callback.borrow_mut()(Err(format!("Browser worker failed: {}", event.message())));
        ctx.request_repaint();
    }) as Box<dyn FnMut(_)>);
    worker.set_onmessage(Some(message.as_ref().unchecked_ref()));
    worker.set_onerror(Some(error.as_ref().unchecked_ref()));
    let handle = Handle {
        worker,
        began: web_time::Instant::now(),
        _message: message,
        _error: error,
    };
    handle
        .worker
        .post_message(&js_sys::Uint8Array::from(bytes.as_slice()))
        .map_err(|e| format!("Could not send job: {}", crate::browser_error(e)))?;
    Ok(handle)
}
