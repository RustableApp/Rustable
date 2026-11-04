# v0.4.11 release verification — publication pending

Source tag: `8b1daad44556a156f672d0fdc1cadc7fd53e61f2`.
Signed release workflow: `41886470946`; Build and test: `41886467961`.
Windows, Linux, and browser jobs passed. GitHub could not allocate a macOS ARM
runner and reported capacity constraints. The failed release job was retried;
alternate macOS 26 recovery run `41889186632` checks out the same release tag.
No desktop release has been published yet.

Local validation: 536 enabled tests passed, 123 optional tests ignored;
strict all-target Clippy and the separate locked browser package check passed.
Live PostgreSQL checks passed: seven driver integration tests, three transport,
statement-cleanup and reconnection tests, plus the uncached custom-type large-row
regression. At 30 ms simulated network RTT, median query latency improved from
124.4 ms to 82.1 ms. Failed writes are not automatically replayed.

Windows ZIP integrity and all three PE executables verified. Linux archive and
ELF executable checks passed. Native Windows/Linux GUI acceptance, macOS native
notification delivery and external-monitor hotplug remain unverified here.

Packaged browser startup and SELECT 411 AS release_check passed with macOS and
Windows user agents. Three website embedding tests passed. The deployed demo
started and returned the expected query value. Live BUILD.txt matches the clean
release tag; isolation/CSP headers passed. The public WASM matches the artifact:
`c59802d46e907a6be56199e1c0796c9b0915027a1125fe9cebd2bcf1fc2b036b`.
Website commit: `912afaf`. Cloudflare deployment:
`420eb815-e966-4714-9ce4-757b17c1576d`.

Remaining gates: macOS build/tests/signing/notarization, local package and
Sparkle signature verification, complete draft checksums, publication and all
public download links. Do not mark this release complete until these pass.
