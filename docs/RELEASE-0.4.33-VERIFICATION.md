# v0.4.33 release verification

Published at `2026-11-19T01:54:02Z`, GitHub release `450778623`:
[v0.4.33](https://github.com/RustableApp/Rustable/releases/tag/v0.4.33).

Source: immutable tag `v0.4.33`, commit
`97d7eb69cd04b16551121af0ccb6ef7bf29b9be7`.
[Release notes](RELEASE-0.4.33.md) cover the compact one-click AI connector,
transaction status in the existing connection capsule, saved-connection deletion
confirmation, and AI documentation.

Main CI run `44437349758` passed on macOS, Windows, Linux and browser.
Local locked native tests: 928 passed, 250 ignored. Strict all-target QA Clippy,
the separate locked browser check (87 existing warnings), five packaging-tool
tests and 14 website tests passed.

UI screenshots and performance evidence are recorded in [MATRIX.md](MATRIX.md)
and [UI-PERFORMANCE.md](UI-PERFORMANCE.md). Connector CPU comparisons passed;
transaction comparisons include noisy/inconclusive cases and a documented
bounded cost for visible transaction status. Saved-connection baseline timing
remains unverified after its earlier baseline build failure. These are not an
overall UI performance pass. No database driver or protocol sequencing changed;
no added database round trips. Database latency suites were not rerun.

Independent macOS validation passed deep strict code signing, Gatekeeper,
stapled app/DMG notarization, DMG signature and integrity, stable updater identity,
Sparkle Ed25519 signature and embedded release notes. The signed packaged MCP
adapter initialized with version 0.4.33 and listed 11 tools without opening a
database. Linux x64 binaries and bundled language servers/licenses were inspected.

Native Windows/Linux GUI acceptance, Intel Mac, installed Sparkle old-to-new
upgrade, 60/120 Hz input-to-present, mixed-DPI and monitor hot-plug behavior remain
unverified. MCP continues to require macOS/Linux and explicitly enabled SQL
workspaces; Windows/browser transport, Redis MCP and built-in chat are not included.

Website source commit `26cc38b` preserves the already deployed AI pages and
includes the exact tagged browser archive with unmodified tracked source.
Local Chrome startup and `SELECT 433 AS release_check;` passed. Local Wrangler
required a preview-only compatibility-date override to 2026-10-03; production
configuration was unchanged. Release artifacts and logs are retained under
`target/release-v0.4.33/`.

Signed-release run `44437386170` succeeded for all platforms and created the
complete seven-asset draft. All six payload checksums and seven uploaded asset
digests/sizes passed verification. Windows x64 PE and Linux x64 ELF architectures,
bundled language servers and licenses passed inspection. All seven versioned and
latest download URLs returned HTTP 200 after publication; the public latest
updater feed exactly matches the verified signed feed.

Cloudflare deployment/version `4da12b9a-9791-4f05-a8f4-c87f5cae06d1` serves the
matching demo. Live startup and `SELECT 433 AS release_check;` passed in the
in-app browser. The native Chrome live navigation remained blank; the same tagged
artifact passed locally in Chrome and publicly in the in-app browser. Six version
pages, the preserved AI page, BUILD.txt and public WASM bytes match website source.
Existing COOP/COEP, WASM CSP and demo indexing headers were verified unchanged.
Public WASM SHA-256:
`63c6c9bc548745d2e8ccb0869e3c702096793de21606cb85d7f89fbdb5f2d257`.
