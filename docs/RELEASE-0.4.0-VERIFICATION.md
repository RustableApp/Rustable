# v0.4.0 release verification

## Local validation

- 493 native tests passed; 114 fixture/manual tests ignored.
- Strict native all-target Clippy passed.
- Five packaging tests passed, including font notices.
- PostgreSQL, MySQL, and MariaDB foreign-key fixtures each passed against disposable local servers.
- Separate browser-package locked WASM check passed; existing browser-only warnings remain.
- TablePlus encrypted import tests include official RNCryptor vectors and a locally verified user-provided export. No private export is committed.

## Publication

Published stable v0.4.0 at 2026-11-02 03:27:04 UTC (November 1, 9:27 PM Chicago).
https://github.com/RustableApp/Rustable/releases/tag/v0.4.0

Tag commit: `2200da98069c6753b8167a32951fb5c6e39009c6`.
Signed release workflow: `41381242334`; all platform builds and draft creation passed.
All seven public latest-download URLs return HTTP 200. The latest macOS updater
feed targets immutable v0.4.0 assets.

All downloaded content assets match SHA256SUMS. Windows ZIP CRC and three PE
executables verified. Linux archive contains three executable ELF files. Both
packages contain font/icon license notices, including Inter.

macOS DMG integrity, strict code signatures, Gatekeeper notarization acceptance,
and stapled app/DMG tickets passed. The bundle reports 0.4.0 and contains the
correct app icon and Inter notice. Sparkle ZIP signature verified independently
against the bundled public key; feed URL and archive length match.

## Website and matching WASM demo

Website commit: `187e7c0`.
Cloudflare deployment: `728724a0-781c-4180-9931-8a1534b71063`.
The live BUILD.txt identifies v0.4.0, the exact tag commit, and clean source.
Live WASM SHA-256 matches the release archive:
`cf7f58494efae7a63678c8f0112ab0572b3717a19e97de48fb7b56c73b681eed`.
COOP/COEP and WASM CSP response headers verified. Twelve browser adapter tests
and three website embedding tests passed.

Exact CI demo acceptance: startup with no SQL tab; explicit SQL editor creation;
SELECT 40 AS release_check returns 40; following orders.customer_id opens a new
filtered customers preview without pinning the source; Redis PING returns PONG.
Live startup and SELECT 40 AS release_check passed after deployment.

## Verification limits

Native Windows/Linux GUI acceptance, Intel macOS, and installed old-to-new
Sparkle update acceptance remain unverified. MySQL/MariaDB cross-database foreign
key navigation is not implemented. Browser-specific pre-existing compile
warnings remain; strict native Clippy passes.
