//! Keep egui's compact default faces, adding system Unicode coverage last.
use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::OnceLock;

const FALLBACK: &str = "system-unicode-fallback";

#[cfg(not(target_arch = "wasm32"))]
fn fallback() -> Option<Arc<FontData>> {
    static FONT: OnceLock<Option<Arc<FontData>>> = OnceLock::new();
    FONT.get_or_init(|| {
        #[cfg(target_os = "macos")]
        let candidates = [
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
        ];
        #[cfg(target_os = "windows")]
        let candidates = [
            "C:/Windows/Fonts/YuGothM.ttc",
            "C:/Windows/Fonts/msgothic.ttc",
        ];
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let candidates = [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        ];
        candidates.iter().find_map(|path| {
            std::fs::read(path)
                .ok()
                .map(|bytes| Arc::new(FontData::from_owned(bytes)))
        })
    })
    .clone()
}

fn definitions(font: Option<Arc<FontData>>) -> FontDefinitions {
    let mut definitions = FontDefinitions::default();
    let emoji = "noto-emoji-fallback";
    definitions.font_data.insert(emoji.into(), Arc::new(FontData::from_static(
        include_bytes!("../assets/fonts/NotoEmoji.ttf"),
    )));
    if let Some(font) = &font {
        definitions.font_data.insert(FALLBACK.into(), font.clone());
    }
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let fonts = definitions.families.entry(family).or_default();
        fonts.push(emoji.into());
        if font.is_some() { fonts.push(FALLBACK.into()); }
    }
    definitions
}

#[cfg(not(target_arch = "wasm32"))]
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions(None));
    let ctx = ctx.clone();
    // Font file I/O happens once off the render thread. Glyphs are rasterized on demand.
    std::thread::spawn(move || {
        if let Some(font) = fallback() {
            ctx.set_fonts(definitions(Some(font)));
            ctx.request_repaint();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_emoji_covers_both_families() {
        let definitions = definitions(None);
        let defaults = FontDefinitions::default();
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            assert_eq!(definitions.families[&family][0], defaults.families[&family][0]);
        }
        let ctx = egui::Context::default();
        ctx.set_fonts(definitions);
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for font in [egui::FontId::monospace(24.0), egui::FontId::proportional(24.0)] {
                    ui.fonts_mut(|fonts| assert!(fonts.has_glyphs(&font, "🦀😀🚀")));
                    ui.label(egui::RichText::new("🦀😀🚀").font(font));
                }
            });
        });
        assert!(!ctx.tessellate(output.shapes, output.pixels_per_point).is_empty());
        // Exercise text layout and atlas upload as well as glyph lookup.
        assert!(!output.textures_delta.set.is_empty());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn system_fallback_covers_cjk_without_replacing_latin_faces() {
        let definitions = definitions(Some(fallback().expect("macOS Unicode fallback font")));
        let defaults = FontDefinitions::default();
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            assert_eq!(
                definitions.families[&family][0],
                defaults.families[&family][0]
            );
        }
        let ctx = egui::Context::default();
        ctx.set_fonts(definitions);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            ctx.fonts_mut(|fonts| {
                for font in [
                    egui::FontId::monospace(13.0),
                    egui::FontId::proportional(13.0),
                ] {
                    assert!(fonts.has_glyphs(&font, "日本中文한국어"));
                }
            });
        });
    }
}

/// Browser builds share the bundled emoji fallback; OS font files are unavailable.
#[cfg(target_arch = "wasm32")]
pub fn install(ctx: &egui::Context) { ctx.set_fonts(definitions(None)); }
