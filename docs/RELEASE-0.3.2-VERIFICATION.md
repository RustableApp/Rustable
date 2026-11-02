# v0.3.2 release verification

## Local validation

- 475 native tests passed; 111 fixture/manual tests ignored.
- Strict native all-target Clippy passed; four focused Redis demo tests passed
  again after its iterator lint correction.
- Five packaging tests passed for the icon bundling change.
- Optimized native icon-fix build passed. A fresh local macOS bundle names
  Rustable.icns in Info.plist and includes the exact repository icon asset.
- Browser-package locked WASM check passed (existing browser warnings remain).
- Redis demo browser acceptance includes hash inspection/commands, a successful
  write, and retained data after switching Redis → SQLite → Redis.

## Published release

Published stable v0.3.2 at 2026-11-02 00:31:08 UTC (November 1 locally).
GitHub release ID: 430955383.
https://github.com/RustableApp/Rustable/releases/tag/v0.3.2
All seven public latest-download URLs returned HTTP 200. The public macOS
update feed points to v0.3.2. Live demo startup and SELECT 32 AS release_check
passed with the expected result 32.

Native Windows/Linux GUI acceptance, Intel macOS and installed old-to-new Sparkle
update acceptance remain unverified. Redis demo is a bounded in-memory subset;
real Redis server support is unchanged from v0.3.1.

## CI and artifact checks

Release tag v0.3.2 is commit `78287aff0efdcb4bde19a2366d8ef9e5dc13bfbc`,
workflow run `41365358630`. All platform builds and draft creation passed.
The Linux archive contains three ELF executables with executable modes and
license notices.

The macOS DMG passed integrity verification. The app passed strict code-signature
verification and Gatekeeper accepted it as notarized. Stapled app and DMG tickets
validated. Bundle version is 0.3.2, CFBundleIconFile names Rustable.icns, and icon
bytes match the source asset. The Sparkle ZIP signature verified with the installed
app's public key; its feed uses the immutable v0.3.2 URL and correct archive length.

The exact CI WASM demo passed local startup, SQLite SELECT 32 AS release_check
and Redis PING → PONG. Three embedding tests passed. Source provenance is clean.
WASM SHA-256: `739c2cf7a49d024bf89b110d85968494ba1365a28b478d6fe610154f7384d4cf`.
Website content commit: `f338b75`.

Windows ZIP CRC, three PE executables and license notices verified. All six
content assets downloaded from the draft match its SHA256SUMS manifest.
Website deployed as Cloudflare version `6d52199b-f2ab-4e64-8816-aa03840908c3`.
Live WASM checksum and clean v0.3.2 provenance match CI; COOP/COEP and WASM CSP
response headers verified.
