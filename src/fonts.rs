//! Keep egui's compact default faces, adding system Unicode coverage last.
use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use std::sync::{Arc, OnceLock};

const FALLBACK: &str = "system-unicode-fallback";

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

fn definitions(font: Arc<FontData>) -> FontDefinitions {
    let mut definitions = FontDefinitions::default();
    definitions.font_data.insert(FALLBACK.into(), font);
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        definitions
            .families
            .entry(family)
            .or_default()
            .push(FALLBACK.into());
    }
    definitions
}

pub fn install(ctx: &egui::Context) {
    let ctx = ctx.clone();
    // Font file I/O happens once off the render thread. Glyphs are rasterized on demand.
    std::thread::spawn(move || {
        if let Some(font) = fallback() {
            ctx.set_fonts(definitions(font));
            ctx.request_repaint();
        }
    });
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn system_fallback_covers_cjk_without_replacing_latin_faces() {
        let definitions = definitions(fallback().expect("macOS Unicode fallback font"));
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
