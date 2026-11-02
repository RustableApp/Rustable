# v0.4.3 release verification

## Source and local validation

Tag commit: `79ef1a3a436707b7f8cea9fbcb2d9314cbd72c08`.
497 native tests passed, 114 fixture/manual tests ignored. Strict all-target
native Clippy and the separate browser-package locked WASM check passed.
Five packaging tests, twelve browser adapter tests and three website embedding
tests passed. Existing browser-only compiler warnings remain.

TablePro tests cover both JSON format versions, five supported engines, nested
group references and cycles, credential indexing, unsupported settings, file
limits, encrypted authentication, wrong passwords, truncation and tampering.
An independent Apple CryptoKit/CommonCrypto encrypted fixture passes. No
user-generated TablePro export has been supplied. Native menu inspection confirms
About Rustable, Hide Rustable and Quit Rustable; launcher screenshot confirms the
empty toolbar is gone. Full native file-picker acceptance was interrupted by UI
automation timeouts after file selection and is not claimed as verified.

## Packaging

Workflow: `41390646488`. Initial Windows, Linux and WASM jobs succeeded.
The first macOS attempt passed signature validation and Apple notarization, then
failed to download the stapling ticket because the Apple CloudKit connection was
lost (NSURLErrorDomain -1005, stapler exit 68). Only failed jobs were retried.
Windows ZIP CRC, three PE executables and license notices verified. Linux archive
contains three executable ELF files and the required license notices.

Attempt 2 completed successfully. All release checksums match. macOS strict/deep
code signature verification, Gatekeeper notarized acceptance, DMG integrity and
app/DMG stapled-ticket validation passed. The Sparkle ZIP Ed25519 signature was
independently verified against the bundled public key; the feed URL and archive
length match the immutable v0.4.3 asset. Bundle version is 0.4.3.

Published stable at 2026-11-02 05:28:32 UTC (November 1, 11:28 PM Chicago):
https://github.com/RustableApp/Rustable/releases/tag/v0.4.3
All seven public latest-download links return HTTP 200. The latest macOS update
feed points to v0.4.3.

## Website

Website content commit: `af24dbc`.
Cloudflare deployment: `b8a7db78-5947-49e0-9430-da2bc62a53bd`.
Live BUILD.txt reports 0.4.3, the exact tag commit and clean tracked source.
Live WASM SHA-256 matches the CI artifact:
`cb9c7f607dc815e043f1fc7e8334fa3d6389511f193cee3db7899872f3b15ea1`.
COOP/COEP and WASM CSP response headers verified. Exact CI and live demo startup
and `SELECT 43 AS release_check` returned the expected 43.

## Limits

Native Windows/Linux GUI acceptance, Intel macOS and installed old-to-new Sparkle
update acceptance remain unverified. v0.4.1 and v0.4.2 were cancelled unpublished
candidates; the stable desktop release before this version is v0.4.0.
