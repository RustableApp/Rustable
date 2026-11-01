# Bundled fallback font

Noto Emoji provides monochrome outline glyphs for egui on native and WASM.
Unmodified font from https://github.com/google/fonts/tree/8b0a1d0f5983c89bc2b93f1b5fb55f9e252744b5/ofl/notoemoji
licensed under the SIL Open Font License 1.1 (NotoEmoji-OFL.txt).

Appended after egui's default faces; existing Latin typography is preserved.
This supplies individual emoji glyphs, not full emoji sequence shaping or color.

The browser additionally downloads the unmodified NotoSansCJKjp-Regular.otf from
https://github.com/notofonts/noto-cjk/tree/f8d157532fbfaeda587e826d4cd5b21a49186f7c/Sans/OTF/Japanese
as a separate, self-hosted asset after startup. It covers Japanese, Chinese and
Korean using Japanese regional glyph forms. Licensed under SIL OFL 1.1
(NotoSansCJK-OFL.txt). Size: 16,467,736 bytes; SHA-256:
68a3fc98800b2a27b371f2fb79991daf3633bd89309d4ffaa6946fd587f375b5.
It is not embedded in database/background-worker WASM or native packages.
