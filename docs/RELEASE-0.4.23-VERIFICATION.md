# v0.4.23 release verification

Published November 12, 2026 at 16:13:49 UTC. Release ID `443195419`.

## Source and validation

- Immutable tag `v0.4.23`: `815e901282a3620c812e8feebab1bd3c2e4d42df`.
- Fixes macOS title-bar resize stretching by anchoring cached OpenGL view contents at their original scale and painting resize events promptly. Includes the already-merged native fallback-font sharing improvement.
- The user confirmed stretching is gone in the disposable native preview. Settled native screenshots were reviewed. Synchronous painting alone was insufficient; adding AppKit TopLeft contents placement resolved the observed issue.
- Local locked tests: 794 passed, 206 ignored. Strict all-target Clippy, the cleaned-up native build and four-frame OpenGL context-recovery probe passed. Separate browser-package locked WASM check passed with 78 existing warnings.
- Changed renderer files pass rustfmt. Repository-wide `cargo fmt --check` reports existing formatting differences in untouched application files; it is not recorded as a pass or addressed by unrelated reformatting.
- Final release-mode UI comparison against `44ad36b`: 18 metrics passed, one noisy tessellation metric remains inconclusive, with no repeatable above-threshold regression. Raw results are retained in `target/macos-resize-final-performance/`; timings, scope and native limitations are in [UI-PERFORMANCE.md](UI-PERFORMANCE.md). No database/protocol sequencing changed.
- [Main CI](https://github.com/RustableApp/Rustable/actions/runs/43251104365) passed every platform and browser job.

## Package provenance and publication

The [original signed-release workflow](https://github.com/RustableApp/Rustable/actions/runs/43251104591) completed Windows, Linux and the matching demo. Its macOS 15 job remained queued without a runner for more than ten minutes. After the successful jobs finished, the workflow was canceled to avoid duplicate builds.

The documented [macOS-only recovery](https://github.com/RustableApp/Rustable/actions/runs/43252998949) on macOS 26 passed all tests, packaging, signing and notarization. Every combined artifact checks out the same immutable tag above. Windows/Linux/demo were not rebuilt. A complete draft was created manually from those artifacts, verified, then published; no published versioned asset was replaced.

- All six payload SHA-256 checksums passed. All seven GitHub upload digests, including SHA256SUMS, match the local verified files.
- Mac deep/strict code signature, stable signing team/bundle identity/updater key/feed URL, Gatekeeper acceptance, app and DMG notarization tickets, and DMG integrity passed.
- Sparkle Ed25519 signature verifies against the packaged public key. Its version-specific URL and embedded Markdown release notes match this release.
- Windows ZIP integrity and all three x64 PE executables passed inspection. Linux contains all three x64 ELF executables. Portable packages include license manifests.
- All seven public latest-release download URLs return HTTP 200; the public updater feed is byte-identical to the verified signed asset.

## Matching website

- Website commit `9c3de10`; Cloudflare version `4fb93086-4754-4aa3-9f37-ebb02c820f9e`.
- Deployed the exact tagged CI demo archive; live BUILD.txt reports 0.4.23, source `815e901282a3620c812e8feebab1bd3c2e4d42df`, and unmodified tracked files.
- Live WASM SHA-256: `481a4c232bb3ec43e2f319f3b4b0e3047f3044ef71efb08d6e9d1c04a438bbaa`, matching the release artifact.
- All 14 website tests passed. All six live landing pages match the deployed source, including updated version labels. Demo isolation and WASM CSP headers passed.
- Live in-app browser startup and `SELECT 423 AS release_check;` returning 423 were visually verified.
- Local evidence, scripts, downloaded artifacts, upload metadata and deployment logs are retained in `target/release-v0.4.23/`.

Mixed-DPI movement, monitor reconnection, secondary-window animations, light-theme resize, 60/120 Hz input-to-present timing, missed-frame and idle-redraw counts remain unverified. Native Windows/Linux GUI acceptance, Intel Mac and installed old-to-new Sparkle installation remain outside this release's verified scope. No broader responsiveness or platform-parity claim is inferred from CPU timings or package integrity.
