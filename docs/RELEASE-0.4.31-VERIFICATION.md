# v0.4.31 candidate verification

Verified November 17, 2026. **Draft only; not published.** GitHub draft
`449135819` contains seven verified assets. Publication is blocked by the
matching website deployment: Cloudflare rejected the existing private ops token
(code 9109), and no saved Wrangler OAuth login was available. The user was asked
to restore the existing deployment credential. The public stable release and
live demo remain v0.4.30. No credentials were printed or committed.

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
Mac icon passed. The draft is explicitly bound to the existing tag and source;
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

Live deployment, live query/provenance/header/checksum checks, publication and
public download/updater checks remain outstanding. Per [RELEASES.md](RELEASES.md),
the desktop draft stays unpublished until the matching live demo is verified.
Logs, assets, checks and provenance are retained in `target/release-v0.4.31/`.
