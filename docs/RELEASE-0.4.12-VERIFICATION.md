# v0.4.12 release verification

Published stable release: https://github.com/RustableApp/Rustable/releases/tag/v0.4.12.
Source tag: `67d34f859bd95fb529900d624e94d0152d00df7d`.
Signed-release workflow `41900941251` and CI `41900917563` both succeeded across
all platforms. Linux dependency installation and macOS runner allocation delayed
packaging; neither required a fallback package.

The initial unpublished candidate `f89d5a7` / workflow `41900340336` was cancelled
when Linux exposed a platform-dependent text-width assertion in the Run toolbar
test. The corrected test checks the button background geometry and explicitly
covers Mac, Windows and Linux. No release existed when its tag was updated. All
published packages and the deployed browser bundle come from `67d34f8`; no
artifacts from the cancelled candidate were reused.

## Validation

- Local suite: 536 enabled tests passed, 125 opt-in tests ignored. Strict
  all-target Clippy and the separate locked browser package check passed.
- Live PostgreSQL: seven integration, three browse, three transport/reconnection
  and two custom-type tests passed. Coverage includes schema changes, large
  uncached enum results, bounded buffering, pagination, cancellation, safe mode,
  explicit transactions and uncertain write outcomes without automatic replay.
- MySQL and MariaDB fixture checks passed for ordinary wildcard browsing and
  invisible-column alignment through browsing, export and edit preconditions.
- At 30 ms simulated RTT, editor medians were 86.1 ms for the describe-first
  path and 44.7 ms for the new path; the actual simple-query reference was
  43.7 ms. Table initial-load medians fell from about 175 ms to 87.9 ms;
  refresh from about 90 ms to 51.3 ms, with revisits at 52.8 ms. These are
  controlled fixture measurements, not measurements of a user's SSH endpoint.
- The separate opt-in catalog latency benchmark still missed its existing 0.65
  ratio target (175 ms / 257 ms, approximately 0.68). That catalog path retains
  the general describe-first API and was not optimized here. Its threshold was
  not weakened; not all optional performance checks pass.

All six payload hashes matched SHA256SUMS. Windows ZIP integrity and all three
x64 PE executables passed; Linux archive and x64 ELF checks passed. Native
packages include their dependency license manifests.

macOS deep/strict codesign, Gatekeeper, app and DMG stapled tickets, DMG integrity,
and Sparkle Ed25519 signature passed. The signature verifies against the installed
app's updater public key. The appcast embeds the exact release notes with Markdown
format. All seven latest-release asset URLs returned HTTP 200; the public appcast
is byte-identical to the verified draft feed.

## Matching website demo

Website commit: `65d7681`. Cloudflare version:
`7e6a7571-36c5-40d8-aa3a-eb2715f87ffe`.
Live BUILD.txt identifies version 0.4.12, source `67d34f8`, and clean tracked files.
The deployed WASM SHA-256 matches the release artifact:
`f92e60fec50457581ce6dfa02fca4db41a794135b9c33a1756554c8e0d07f20a`.

Packaged demo startup and SELECT 412 passed with macOS and Windows user agents.
A running recursive SQLite query showed the spinner inside Run, a stable button
position and available Cancel control. The live demo started and returned 412.
No page errors were observed. Three embedding tests and public isolation/CSP
headers passed. The draft browser archive matched the tested workflow artifact.

Native Windows/Linux GUI acceptance, installed old-to-new Sparkle installation,
and visual rendering of the actual native update prompt were not exercised.
Package checks and embedded-feed validation do not establish those behaviors.
