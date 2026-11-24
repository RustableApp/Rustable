# v0.4.39 release verification

Published at `2026-11-24T14:32:00Z`, GitHub release `456882425`:
[v0.4.39](https://github.com/RustableApp/Rustable/releases/tag/v0.4.39). The immutable tag points to
`962c6e8dea5da1862267375e93b25210ae8a6a42`, with matching Cargo version and
root/browser lockfiles. [Release notes](RELEASE-0.4.39.md) cover faster PostgreSQL
edit preparation, server-side guards and the bounded preparation cache.

Signed-release run `45412804215` passed on all platforms. Main CI `45412804210`
passed its platform test stages; its macOS release-cache warming step was still
running at publication. All packages came from the same release run and tag.

All six payload checksums and all seven uploaded asset digests/sizes matched.
macOS passed deep strict code signing, Gatekeeper assessment, app/DMG stapled-ticket
validation, DMG integrity, stable updater identity, Sparkle Ed25519 signature and
embedded release notes. The packaged executable reports v0.4.39 and lists 11 MCP
tools without opening a database. Windows x64 PE and Linux x64 ELF app/language
servers and bundled licenses were inspected. All seven versioned and latest
asset URLs returned HTTP 200; the latest updater feed matches the verified feed.

Local versioned checks passed: 942 library tests (269 opt-in tests ignored),
strict all-target QA Clippy, the separate locked browser-package WASM check
(85 existing warnings), five packaging tests and 14 website tests. Before the
version-only bump, 18 focused editing/structure tests and the release-mode editor,
browse and edit WAN regressions passed against a disposable PostgreSQL fixture.
See [EDIT-SERVER-CHECKS-PERFORMANCE.md](EDIT-SERVER-CHECKS-PERFORMANCE.md) for the
same-process baselines, protocol counts, cache invalidation, failure injection,
measurement scope and limits. At 30 ms simulated RTT, warm prepare/apply measured
92.88 ms versus 986.98 ms originally; cached preparation measured 0.223 ms.
These are driver measurements, excluding confirmation and post-save refresh.

Website content commit `fefa207` contains the exact release demo and updated
version labels. Cloudflare version `be37d400-7e7c-46fc-a92f-d87e53462436` serves it.
Six public version pages, BUILD.txt and WASM bytes match source; isolation/CSP and
indexing headers remain intact. Initial stale edge responses cleared during
propagation; the final checks used ordinary public URLs. Public WASM SHA-256:
`bc14a38b9169535f884b1fdff9fb83019ece534d7c95447fc43ce264ec22c076`.
Local and live startup and `SELECT 439 AS release_check` passed, showing one row
containing 439 after Cmd+Enter in Silent mode without a prompt. Local Wrangler
used compatibility date 2026-10-03 because its installed runtime predates the
production setting; production configuration was unchanged.

Native Windows/Linux GUI acceptance, Intel Mac, installed Sparkle old-to-new
upgrades, actual Touch ID authorization and native display/input performance
remain unverified. This release changes database worker paths, not rendering.
Raw release artifacts/logs are retained under `target/release-v0.4.39/`, with
performance evidence under `target/edit-server-checks/`.
