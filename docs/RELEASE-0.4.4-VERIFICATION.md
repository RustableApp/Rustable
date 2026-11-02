# v0.4.4 release verification

## Source and validation

Tag source: `4b156e433c89b2f6beec2b1f964ca7ff1ac73a9f`.
Workflow: `41466482717`.
504 enabled native tests passed; 116 fixture/manual tests ignored in the default suite.
Strict all-target native Clippy and separate browser-package locked WASM check passed.
Existing browser-only warnings remain.

DDL fixtures passed against disposable PostgreSQL, MySQL and MariaDB databases;
SQLite definition and modal-interaction tests passed. PostgreSQL's reconstructed
regular-table definition was successfully executed in its isolated test schema.
Ranking tests cover prefix priority, word boundaries, abbreviations, metadata,
Unicode and stable ties. Tab tests cover retained results, selection, view state,
empty tables, closing tabs, SQL-tab round trips and explicit refresh.

The exact CI WASM artifact starts, executes `SELECT 44 AS release_check`, opens
the SQLite DDL viewer and matches `rbp` to `revenue_by_plan`. Returning from SQL
restores the Structure view. Website embedding tests: three passed.

Windows ZIP CRC, three PE executables and license notices passed. Linux archive
contains three executable ELF files and required license notices.

## Website provenance

Website content commit: `3b45545`.
BUILD.txt reports version 0.4.4, the exact tag commit and clean tracked source.
WASM SHA-256: `512649499ef1de53fa48f8f3dd7874d3d37b0fe95309b56fe20c8d71d514a0cb`.

## Limits

PostgreSQL DDL is a structure preview, not a complete schema backup. Its displayed
SQL identifies omitted metadata; inherited, partitioned and foreign tables report
an explicit unsupported-preview error.
Native Windows/Linux GUI acceptance, Intel macOS and installed old-to-new Sparkle
update acceptance remain unverified.

## Published packages and deployment

Attempt 1 passed all builds and macOS app notarization, stapling and Gatekeeper
assessment, then lost the Apple CloudKit network connection while stapling the
DMG (NSURLErrorDomain -1005, stapler exit 68). Only failed jobs were retried.
Attempt 2 passed; Windows, Linux and WASM artifacts were reused from attempt 1.

All final SHA-256 checksums match. macOS strict/deep code signature verification,
Gatekeeper notarized acceptance, app and DMG stapled tickets, signed DMG and DMG
integrity passed locally. The Sparkle Ed25519 archive signature was independently
verified against the bundled public key. Bundle version, feed URL and archive
length match v0.4.4.

Published stable at 2026-11-02 15:51:03 UTC:
https://github.com/RustableApp/Rustable/releases/tag/v0.4.4
All seven public latest-download links return HTTP 200 and the latest updater
feed points to v0.4.4.

Cloudflare deployment: `a6cd14ee-eda5-455f-bdef-c8dcfd713341`.
Live demo provenance and WASM SHA-256 match the exact tagged artifact. COOP/COEP
and WASM CSP headers passed. Live startup and `SELECT 44 AS release_check` passed.
