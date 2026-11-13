//! A fixed-height, virtualized strip for the current run and pinned snapshots.
use super::*;

pub(super) const STRIP_HEIGHT: f32 = 36.0;
const TAB_WIDTH: f32 = 168.0;
const TAB_HEIGHT: f32 = 28.0;
const GAP: f32 = 4.0;

impl Rustable {
    pub(super) fn query_result_tabs(&mut self, ui: &mut egui::Ui) {
        let tab = &mut self.tabs[self.tab];
        let count = tab.pins.items.len() + tab.results.len();
        let selected = tab
            .selected_pin
            .unwrap_or(tab.pins.items.len() + tab.result_index);
        let active_id = ui.make_persistent_id(("visible-result", tab.id));
        let reveal = ui.ctx().data_mut(|data| {
            let previous = data.get_temp::<usize>(active_id);
            data.insert_temp(active_id, selected);
            previous != Some(selected)
        });
        let mut choose = None;
        let mut close = None;
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                egui::ScrollArea::horizontal()
                    .id_salt(("query-result-tabs", tab.id))
                    .auto_shrink([false, false])
                    .max_height(TAB_HEIGHT)
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show_viewport(ui, |ui, viewport| {
                        let stride = TAB_WIDTH + GAP;
                        let width = (count as f32 * stride - GAP).max(0.0);
                        let (strip, _) = ui.allocate_exact_size(
                            egui::vec2(width, TAB_HEIGHT),
                            egui::Sense::hover(),
                        );
                        let rect_for = |index: usize| {
                            egui::Rect::from_min_size(
                                strip.min + egui::vec2(index as f32 * stride, 0.0),
                                egui::vec2(TAB_WIDTH, TAB_HEIGHT),
                            )
                        };
                        if reveal && count > 0 {
                            ui.scroll_to_rect_animation(
                                rect_for(selected.min(count - 1)),
                                None,
                                egui::style::ScrollAnimation::none(),
                            );
                        }
                        let first = (viewport.left() / stride).floor().max(0.0) as usize;
                        let end = ((viewport.right() / stride).ceil() as usize).min(count);
                        for index in first..end {
                            let (name, grid) = if let Some(pin) = tab.pins.items.get(index) {
                                (format!("Pinned {}", pin.id), &pin.grid)
                            } else {
                                let result = index - tab.pins.items.len();
                                (format!("Result {}", result + 1), &tab.results[result])
                            };
                            let detail = if grid.columns.is_empty() {
                                format!("{} affected", grid.affected)
                            } else {
                                format!(
                                    "{}{} rows",
                                    grid.rows.len(),
                                    if grid.truncated { "+" } else { "" }
                                )
                            };
                            let label = format!("{name} · {detail}");
                            let response = ui
                                .push_id(index, |ui| {
                                    ui.put(rect_for(index), |ui: &mut egui::Ui| {
                                        let (rect, response) = ui.allocate_exact_size(
                                            egui::vec2(TAB_WIDTH, TAB_HEIGHT),
                                            egui::Sense::click(),
                                        );
                                        paint_workspace_tab(
                                            ui,
                                            rect,
                                            &response,
                                            &label,
                                            selected == index,
                                            false,
                                            12.0,
                                        );
                                        response.widget_info(|| {
                                            egui::WidgetInfo::selected(
                                                egui::WidgetType::SelectableLabel,
                                                ui.is_enabled(),
                                                selected == index,
                                                &label,
                                            )
                                        });
                                        response
                                    })
                                })
                                .inner
                                .on_hover_text(&label);
                            if response.clicked() {
                                choose = Some(index);
                            }
                            if index < tab.pins.items.len() {
                                response.context_menu(|ui| {
                                    if ui.button("Close pinned result").clicked() {
                                        close = Some(index);
                                        ui.close();
                                    }
                                });
                            }
                        }
                    });
            });
        if let Some(index) = close {
            tab.remove_pin(index);
        } else if let Some(index) = choose {
            if let Some(pin) = tab.pins.items.get(index) {
                if !Arc::ptr_eq(&tab.grid, &pin.grid) { tab.result_search.cancel(); }
                tab.selected_pin = Some(index);
                tab.grid = pin.grid.clone();
            } else {
                tab.result_index = index - tab.pins.items.len();
                tab.selected_pin = None;
                if !Arc::ptr_eq(&tab.grid, &tab.results[tab.result_index]) { tab.result_search.cancel(); }
                tab.grid = tab.results[tab.result_index].clone();
            }
        }
        if close.is_some() || choose.is_some() {
            self.row_selection.clear();
            self.selected = None;
        }
    }
}
