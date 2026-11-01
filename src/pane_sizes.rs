//! User-chosen pane geometry, separate from egui's transient/clamped layout state.
use eframe::egui;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PaneSizes {
    pub sidebar: Option<f32>,
    pub inspector: Option<f32>,
    pub activity: Option<f32>,
    pub query_editor: Option<f32>,
}

pub fn restore(
    ctx: &egui::Context,
    id: egui::Id,
    preferred: Option<f32>,
    default: f32,
    axis: usize,
) {
    let size = preferred
        .filter(|size| size.is_finite() && *size > 0.0)
        .unwrap_or(default);
    let mut rect = egui::containers::panel::PanelState::load(ctx, id)
        .map_or(egui::Rect::ZERO, |state| state.rect);
    rect.max[axis] = rect.min[axis] + size;
    ctx.data_mut(|data| data.insert_persisted(id, egui::containers::panel::PanelState { rect }));
}

pub fn record_drag(
    ctx: &egui::Context,
    id: egui::Id,
    preferred: &mut Option<f32>,
    axis: usize,
) -> bool {
    let dragged = ctx
        .read_response(id.with("__resize"))
        .is_some_and(|response| response.dragged());
    let moved = ctx.input(|input| input.pointer.delta() != egui::Vec2::ZERO);
    if !dragged || !moved {
        return false;
    }
    let Some(state) = egui::containers::panel::PanelState::load(ctx, id) else {
        return false;
    };
    let size = state.rect.size()[axis];
    if !size.is_finite() || size <= 0.0 || *preferred == Some(size) {
        return false;
    }
    *preferred = Some(size);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_clamps_do_not_replace_preferred_sizes_or_create_preferences() {
        for axis in [0, 1] {
            for chosen in [None, Some(300.0)] {
                let ctx = egui::Context::default();
                let id = egui::Id::new(("pane", axis));
                let mut preferred = chosen;
                let mut actual = Vec::new();
                for limit in [400.0, 120.0, 400.0] {
                    let _ = ctx.run(egui::RawInput::default(), |ctx| {
                        restore(ctx, id, preferred, 180.0, axis);
                        egui::CentralPanel::default().show(ctx, |ui| {
                            if axis == 0 {
                                egui::SidePanel::left(id)
                                    .width_range(80.0..=limit)
                                    .show_inside(ui, |ui| {
                                        ui.set_min_width(ui.available_width());
                                    });
                            } else {
                                egui::TopBottomPanel::bottom(id)
                                    .height_range(80.0..=limit)
                                    .show_inside(ui, |ui| {
                                        ui.set_min_height(ui.available_height());
                                    });
                            }
                        });
                        assert!(!record_drag(ctx, id, &mut preferred, axis));
                    });
                    actual.push(
                        egui::containers::panel::PanelState::load(&ctx, id)
                            .unwrap()
                            .size()[axis],
                    );
                    assert_eq!(preferred, chosen);
                }
                assert!(
                    (actual[0] - chosen.unwrap_or(180.0)).abs() < 1.0,
                    "{actual:?}"
                );
                assert!(actual[1] <= 120.0, "{actual:?}");
                assert!((actual[2] - actual[0]).abs() < 1.0, "{actual:?}");
            }
        }
    }
}
