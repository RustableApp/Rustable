//! Ethan Schoonover's canonical sRGB Solarized palette and UI mapping.
//! https://ethanschoonover.com/solarized/#the-values
use eframe::egui::{Color32, Stroke, Visuals};

pub const BASE03: Color32 = Color32::from_rgb(0, 43, 54);
pub const BASE02: Color32 = Color32::from_rgb(7, 54, 66);
pub const BASE01: Color32 = Color32::from_rgb(88, 110, 117);
pub const BASE00: Color32 = Color32::from_rgb(101, 123, 131);
pub const BASE0: Color32 = Color32::from_rgb(131, 148, 150);
pub const BASE1: Color32 = Color32::from_rgb(147, 161, 161);
pub const BASE2: Color32 = Color32::from_rgb(238, 232, 213);
pub const BASE3: Color32 = Color32::from_rgb(253, 246, 227);
pub const YELLOW: Color32 = Color32::from_rgb(181, 137, 0);
pub const RED: Color32 = Color32::from_rgb(220, 50, 47);
pub const MAGENTA: Color32 = Color32::from_rgb(211, 54, 130);
pub const BLUE: Color32 = Color32::from_rgb(38, 139, 210);
pub const CYAN: Color32 = Color32::from_rgb(42, 161, 152);
pub const GREEN: Color32 = Color32::from_rgb(133, 153, 0);

// Read the palette from the local visuals, including cloned editor styles, without
// a context-data lock on each highlighted layout. Both backgrounds are exclusive
// to these built-in themes and remain unchanged by editor selection styling.
pub fn is_active(visuals: &Visuals) -> bool {
    matches!(visuals.extreme_bg_color, BASE03 | BASE3)
}

pub fn apply(v: &mut Visuals) {
    let (background, surface, text, emphasis, muted) = if v.dark_mode {
        (BASE03, BASE02, BASE0, BASE1, BASE01)
    } else {
        (BASE3, BASE2, BASE00, BASE01, BASE1)
    };
    v.panel_fill = background;
    v.window_fill = background;
    v.extreme_bg_color = background;
    v.faint_bg_color = surface;
    // Let interactive widgets use their selected/hovered foregrounds.
    v.override_text_color = None;
    v.weak_text_color = Some(muted);
    v.hyperlink_color = BLUE;
    v.warn_fg_color = YELLOW;
    v.error_fg_color = RED;
    v.window_stroke = Stroke::new(1.0_f32, muted);
    // Selection must stand apart from both zebra stripes and inactive segments.
    // Use the middle base tones with contrasting ink, retaining the official palette.
    v.selection.bg_fill = if v.dark_mode { BASE01 } else { BASE1 };
    v.selection.stroke = Stroke::new(1.0_f32, if v.dark_mode { BASE3 } else { BASE03 });
    v.text_cursor.stroke.color = emphasis;
    for widget in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        widget.bg_fill = surface;
        widget.weak_bg_fill = surface;
        widget.bg_stroke.color = muted;
        widget.fg_stroke.color = text;
    }
    v.widgets.hovered.bg_stroke.color = BLUE;
    v.widgets.hovered.fg_stroke.color = emphasis;
    v.widgets.active.bg_stroke.color = BLUE;
    v.widgets.active.fg_stroke.color = emphasis;
    v.widgets.open.fg_stroke.color = emphasis;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contrast(a: Color32, b: Color32) -> f32 {
        let luminance = |color: Color32| {
            let rgb = [color.r(), color.g(), color.b()].map(|value| {
                let value = f32::from(value) / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            });
            rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn selected_rows_and_segments_have_distinct_fills_and_readable_text() {
        for mut visuals in [Visuals::light(), Visuals::dark()] {
            apply(&mut visuals);
            for adjacent in [
                visuals.panel_fill,
                visuals.faint_bg_color,
                visuals.widgets.inactive.weak_bg_fill,
            ] {
                assert!(contrast(visuals.selection.bg_fill, adjacent) >= 2.0);
            }
            assert!(contrast(visuals.selection.bg_fill, visuals.selection.stroke.color) >= 4.5);
        }
    }
}
