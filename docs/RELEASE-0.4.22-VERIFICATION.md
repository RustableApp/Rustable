# v0.4.22 release verification

Published November 9, 2026 at 21:29:51 UTC. Release ID `439666061`.

## Scope and verification limit

Packaged macOS builds preserve the bundle icon rather than replacing
NSApplication.applicationIconImage with a raw PNG. Unbundled runs retain their
fallback. The native bundle icon measured 206×206 visible pixels on a 256-pixel
canvas, matching Chrome, Notes, TablePlus and Ghostty through NSWorkspace's icon
renderer. All ten ICNS representations have the expected dimensions, and the
signed package references the exact checked-in ICNS via CFBundleIconFile.

**Dock and app-switcher visual acceptance remains unverified:** the Mac was
locked. This limitation was disclosed before the user explicitly requested
release. The release does not claim a completed visual comparison of the running
app. Inspection/measurement scripts and renders are in `/tmp/rustable-icon-review`.

## Source and checks

- Immutable tag `v0.4.22`: `66c8317b586f448733978a711bb4fd458bfc1eef`.
- [Main CI](https://github.com/RustableApp/Rustable/actions/runs/42701240797) and [signed release](https://github.com/RustableApp/Rustable/actions/runs/42701272619) passed every job.
- Local locked suite: 793 passed, 206 ignored. Strict all-target Clippy and release build passed. Separate browser-package locked WASM check passed with 78 existing warnings.
- No database or rendering/input hot path changed; only startup icon selection changed, so driver fixtures and frame benchmarks were not rerun.
- Six payload checksums, Mac deep/strict signature, Gatekeeper acceptance, app/DMG notarization tickets, DMG integrity, stable bundle/updater-key identity and Sparkle Ed25519 signature passed. Embedded updater notes match release notes.
- Windows ZIP integrity and three x64 PE executables, Linux x64 ELF executables and portable license manifests passed inspection.

## Matching website

- Website commit `0fa7295`; Cloudflare version `1a98a585-0808-4ba3-96fc-e0b5c326f594`.
- Staged demo files matched the exact release archive. Live BUILD.txt identifies 0.4.22, the tag commit above, and unmodified source.
- Live WASM SHA-256: `1c2126b67061b547c7d185fb2118014a0a52d66db874a4a575c332cdbcac00b8`.
- All 14 website tests passed. Local and live Chrome checks visually confirmed SELECT 422 returning 422 and Redis PING returning PONG, without page errors. Cross-origin isolation and CSP checks passed.
- All seven public latest-release downloads returned HTTP 200, and the public updater feed matched the verified asset.
- Package/deployment/live evidence: `/tmp/rustable-v0422/`, including release-verification.log, portable-verification.log, website-tests.log, deploy.log, live-final-verification.log, live-query.png and live-redis-workbench.png.

Native Windows/Linux GUI acceptance, Intel Mac and installed old-to-new Sparkle
installation remain outside this release's verified scope. Published versioned
assets were not replaced; v0.4.22 is a new immutable release.
