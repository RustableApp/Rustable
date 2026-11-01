//! Browser edition: real egui/WASM UI and worker-owned SQLite/WASM sample data.
use eframe::egui::{self, Color32, RichText};
use serde::Deserialize;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::prelude::*;
#[path = "../../../src/sql_highlight.rs"]
mod sql_highlight;

#[wasm_bindgen(module = "/web/bridge.js")]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn request(
        sql: String,
        caret: usize,
        selection_end: usize,
        all: bool,
        reset: bool,
    ) -> Result<JsValue, JsValue>;
    fn cancel();
}
#[derive(Default, Deserialize)]
struct ResultSet {
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
    truncated: bool,
}
#[derive(Default, Deserialize)]
struct Reply {
    #[serde(default)]
    results: Vec<ResultSet>,
    #[serde(default)]
    tables: Vec<String>,
    #[serde(default)]
    elapsed: f64,
    #[serde(default)]
    error: Option<String>,
}
struct BrowserApp {
    sql: String,
    caret: usize,
    selection_end: usize,
    inbox: Rc<RefCell<Option<Reply>>>,
    generation: Rc<Cell<u64>>,
    busy: bool,
    reply: Reply,
    active: usize,
    table: Option<String>,
    selected: Option<usize>,
    history: Vec<String>,
    show_history: bool,
}
impl BrowserApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
        cc.egui_ctx.style_mut(|style| {
            style.spacing.button_padding = egui::vec2(12.0, 6.0);
        });
        let mut app = Self {
            sql: "SELECT * FROM customers LIMIT 50;".into(),
            caret: 0,
            selection_end: 0,
            inbox: Rc::default(),
            generation: Rc::default(),
            busy: false,
            reply: Reply::default(),
            active: 0,
            table: Some("customers".into()),
            selected: None,
            history: Vec::new(),
            show_history: false,
        };
        app.run(&cc.egui_ctx, true, false);
        app
    }
    fn run(&mut self, ctx: &egui::Context, all: bool, reset: bool) {
        if self.busy {
            return;
        }
        self.busy = true;
        let inbox = self.inbox.clone();
        let ctx = ctx.clone();
        let generation = self.generation.clone();
        let ticket = generation.get() + 1;
        generation.set(ticket);
        let sql = self.sql.clone();
        let caret = self.caret;
        let end = self.selection_end;
        if !reset {
            self.history.insert(0, sql.clone());
            self.history.truncate(30);
        }
        wasm_bindgen_futures::spawn_local(async move {
            let result = request(sql, caret, end, all, reset).await;
            let reply = match result {
                Ok(value) => serde_json::from_str(&value.as_string().unwrap_or_default())
                    .unwrap_or_else(|e| Reply {
                        error: Some(e.to_string()),
                        ..Default::default()
                    }),
                Err(error) => Reply {
                    error: Some(error.as_string().unwrap_or_else(|| {
                        "Browser database failed. Reset the demo to try again.".into()
                    })),
                    ..Default::default()
                },
            };
            if generation.get() == ticket {
                *inbox.borrow_mut() = Some(reply);
                ctx.request_repaint();
            }
        });
    }
}
impl eframe::App for BrowserApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if let Some(reply) = self.inbox.borrow_mut().take() {
            self.busy = false;
            self.active = 0;
            self.selected = None;
            self.reply.tables = reply.tables;
            self.reply.results = reply.results;
            self.reply.error = reply.error;
            self.reply.elapsed = reply.elapsed;
        }
        let mut run = None;
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::Enter,
            )
        }) {
            run = Some(true);
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
            run = Some(false);
        }
        let compact = ctx.content_rect().width() < 650.0;
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Rustable").strong().size(20.0));
                ui.colored_label(Color32::from_rgb(87, 190, 153), "SQLite · browser sandbox");
                if compact {
                    ui.menu_button("Tables", |ui| {
                        for table in self.reply.tables.clone() {
                            if ui.button(&table).clicked() {
                                self.sql = format!(
                                    "SELECT * FROM \"{}\" LIMIT 500;",
                                    table.replace('"', "\"\"")
                                );
                                self.table = Some(table);
                                run = Some(true);
                                ui.close();
                            }
                        }
                    });
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button("Reset demo")
                        .on_hover_text("Recreate disposable sample data. Your SQL text stays here.")
                        .clicked()
                    {
                        cancel();
                        self.busy = false;
                        self.run(ctx, true, true);
                    }
                    ui.hyperlink_to("Download desktop ↗", "https://rustable.app/#main");
                });
            });
        });
        if !compact {
            egui::SidePanel::left("objects").default_width(175.0).min_width(120.0).show(ctx, |ui| {
            ui.horizontal(|ui| { ui.selectable_value(&mut self.show_history, false, "Items"); ui.selectable_value(&mut self.show_history, true, "History"); });
            ui.separator();
            if self.show_history {
                for sql in self.history.clone() {
                    let galley = sql_highlight::preview(ui, &sql, ui.available_width() - 12.0);
                    if ui.add(egui::Button::new(galley).frame(false)).clicked() { self.sql = sql; self.table = None; }
                }
            } else {
                ui.weak("Sample database");
                for table in self.reply.tables.clone() {
                    if ui.selectable_label(self.table.as_ref() == Some(&table), table.clone()).clicked() {
                        self.sql = format!("SELECT * FROM \"{}\" LIMIT 500;", table.replace('"', "\"\""));
                        self.table = Some(table); run = Some(true);
                    }
                }
                ui.add_space(16.0);
                ui.weak("Real SQLite. Try SELECT, JOIN, INSERT or UPDATE. Changes live only in this tab.");
            }
        });
        }
        if ctx.content_rect().width() > 900.0 {
            egui::SidePanel::right("inspector")
                .default_width(210.0)
                .show(ctx, |ui| {
                    ui.strong("Row inspector");
                    ui.separator();
                    if let Some(set) = self.reply.results.get(self.active)
                        && let Some(row) = self.selected.and_then(|n| set.rows.get(n))
                    {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for (name, value) in set.columns.iter().zip(row) {
                                ui.weak(name);
                                ui.add(egui::Label::new(value).wrap());
                                ui.add_space(9.0);
                            }
                        });
                    } else {
                        ui.add_space(80.0);
                        ui.weak("No row selected");
                    }
                });
        }
        egui::TopBottomPanel::top("editor").resizable(true).default_height(215.0).min_height(130.0).show(ctx, |ui| {
            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, width| sql_highlight::layout(ui,text,width);
            egui::ScrollArea::vertical().max_height((ui.available_height()-44.0).max(50.0)).show(ui, |ui| {
                let output = egui::TextEdit::multiline(&mut self.sql).font(egui::TextStyle::Monospace).code_editor().desired_width(f32::INFINITY).desired_rows(6).layouter(&mut layouter).show(ui);
                if let Some(cursor) = output.cursor_range { let r = cursor.as_sorted_char_range(); self.caret = r.start; self.selection_end = r.end; }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.busy { if ui.button("Cancel & reset").clicked() { cancel(); self.generation.set(self.generation.get()+1); self.busy = false; self.reply.error = Some("Cancelled. The disposable database will be recreated on the next query.".into()); } }
                else {
                    if ui.add(egui::Button::new("Run current").fill(Color32::from_rgb(0,105,220))).clicked() { run=Some(false); }
                    if ui.button("Run all").clicked() { run=Some(true); }
                }
            });
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if self.busy {
                    ui.add(egui::Spinner::new().size(14.0));
                    ui.label("Running…");
                } else {
                    ui.label(format!("{:.1} ms", self.reply.elapsed));
                }
                if let Some(set) = self.reply.results.get(self.active) {
                    ui.label(format!(
                        "{} rows{}",
                        set.rows.len(),
                        if set.truncated {
                            " · display capped"
                        } else {
                            ""
                        }
                    ));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak("Rust + egui / WASM");
                });
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(error) = &self.reply.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            if self.reply.results.len() > 1 {
                ui.horizontal(|ui| {
                    for i in 0..self.reply.results.len() {
                        if ui
                            .selectable_value(&mut self.active, i, format!("Result {}", i + 1))
                            .changed()
                        {
                            self.selected = None;
                        }
                    }
                });
            }
            if let Some(set) = self.reply.results.get(self.active) {
                if set.columns.is_empty() {
                    ui.label("Statement completed.");
                } else {
                    egui::ScrollArea::horizontal().show(ui, |ui| {
                        let mut table = egui_extras::TableBuilder::new(ui)
                            .id_salt((&set.columns, self.active))
                            .striped(true)
                            .resizable(true)
                            .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
                        for (index, name) in set.columns.iter().enumerate() {
                            let chars = set
                                .rows
                                .iter()
                                .take(100)
                                .filter_map(|row| row.get(index))
                                .map(|v| v.chars().count())
                                .max()
                                .unwrap_or(0)
                                .max(name.chars().count());
                            table = table.column(
                                egui_extras::Column::initial(
                                    (chars as f32 * 8.0 + 24.0).clamp(70.0, 260.0),
                                )
                                .at_least(60.0),
                            );
                        }
                        table
                            .header(28.0, |mut header| {
                                for name in &set.columns {
                                    header.col(|ui| {
                                        ui.strong(name);
                                    });
                                }
                            })
                            .body(|body| {
                                body.rows(26.0, set.rows.len(), |mut row| {
                                    let n = row.index();
                                    row.set_selected(self.selected == Some(n));
                                    for value in &set.rows[n] {
                                        row.col(|ui| {
                                            if ui
                                                .add(
                                                    egui::Label::new(
                                                        RichText::new(value).monospace(),
                                                    )
                                                    .truncate()
                                                    .sense(egui::Sense::click()),
                                                )
                                                .clicked()
                                            {
                                                self.selected = Some(n);
                                            }
                                        });
                                    }
                                });
                            });
                    });
                }
            }
        });
        if let Some(all) = run {
            self.run(ctx, all, false);
        }
    }
}
#[wasm_bindgen]
pub async fn start(canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(|cc| Ok(Box::new(BrowserApp::new(cc)))),
        )
        .await
}
