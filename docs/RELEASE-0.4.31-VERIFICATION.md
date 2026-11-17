# v0.4.31 release verification

Published November 17, 2026 at `2026-11-17T20:47:09Z` as GitHub release
`449135819`: [v0.4.31](https://github.com/RustableApp/Rustable/releases/tag/v0.4.31).
The user explicitly authorized publication without the matching live demo and
without further icon changes. All seven versioned and latest asset URLs returned
HTTP 200; the public latest updater feed exactly matches the verified signed feed.

The web demo remains v0.4.30: Cloudflare rejected the existing private ops token
(code 9109), and no saved Wrangler OAuth login was available. The prepared
v0.4.31 website bundle remains committed for later deployment. No credentials
were printed or committed.

Source: immutable tag `v0.4.31`, commit
`e9718bf44c902965e9f25ff6b3a560bc52ef8e80`.
[Release notes](RELEASE-0.4.31.md) cover editor indentation/undo, bulk CSV/JSON
export, table actions, and styled destructive review with SQL highlighting.

[Main CI](https://github.com/RustableApp/Rustable/actions/runs/44162056889) and
[signed release](https://github.com/RustableApp/Rustable/actions/runs/44162061582)
succeeded on all platforms. The original macOS 15 jobs waited for runner
capacity. A macOS 26 recovery run (`44162858760`) was cancelled while still
queued once the original runner started; no recovery artifact was used.

## Package verification

All six payload checksums passed; all seven uploaded asset sizes and digests
match downloaded files. macOS deep strict signature validation, Gatekeeper,
stapled app/DMG notarization, DMG signature and integrity passed. Bundle identity,
team `624KJV575U`, updater URL and Ed25519 public key match the prior release.
The Sparkle signature verifies against the exact ZIP, with immutable v0.4.31
URL, correct length and embedded release notes. ARM64 Mac, x64 Windows PE and
x64 Linux ELF architectures, portable archive integrity, bundled licenses and
Mac icon passed. The release is explicitly bound to the existing tag and source;
the tag has not moved.

## Correctness and performance

Local preflight: 873 release tests passed, 234 opt-in tests skipped. Strict
all-target QA Clippy passed. The separate locked browser package check passed
with 83 existing warnings; five packaging-tool tests passed. Disposable SQLite
export/action tests, MySQL/MariaDB export tests and all five PostgreSQL release
latency tests passed during implementation. One debug spooling timing failed;
both baseline and candidate release comparisons passed without weakened thresholds.

Full timings, raw artifact paths and limitations are in
[UI-PERFORMANCE.md](UI-PERFORMANCE.md). Editor and sidebar comparisons passed;
20-table confirmation styling was inconclusive, and the richer 2000-table
confirmation costs .003500 ms more at p95 (+14.7%). That measured cost was
accepted for clearer action context and cached syntax highlighting, confined to
an open confirmation dialog. Native dark/light screenshots were inspected.
Physical input-to-present latency, 60/120 Hz, mixed-DPI/multiple windows,
Intel Mac, native Windows/Linux GUI and installed Sparkle old-to-new upgrades
remain unverified.

## Matching website prepared, deployment blocked

Website commit `152ef77` contains the exact tagged browser archive, all six
version labels and updated capability text. The draft's browser archive is
byte-identical to the locally tested CI artifact. BUILD.txt identifies v0.4.31,
the release commit and unmodified tracked source. All 14 website tests passed.
Local Chrome startup and `SELECT 431 AS release_check;` passed, with a second
one-row result tab from the second statement. Safari local startup showed a
blank canvas; Safari acceptance is not claimed. The installed local Wrangler
runtime required a command-line-only compatibility date of 2026-10-03;
production configuration remains 2026-10-31.

Live website deployment and its query/provenance/header/checksum checks remain
outstanding. The user explicitly overrode the matching-demo publication requirement
in [RELEASES.md](RELEASES.md). Public desktop downloads and updater checks passed.
Logs, assets, checks and provenance are retained in `target/release-v0.4.31/`.
