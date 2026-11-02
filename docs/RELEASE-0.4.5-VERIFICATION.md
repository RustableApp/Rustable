# v0.4.5 release verification

Source tag: `21fbe9c58851fa054c4124c7d32abb2dc6410dac`.
Signed release workflow: `41512461727`.

Local acceptance: 514 enabled native tests passed; 118 optional fixture/manual
checks ignored. Strict all-target Clippy passed. Separate browser-package locked
WASM check passed, with existing browser-only warnings. The actual OpenSSH
private-key helper passed correct-passphrase, wrong-passphrase and cancellation
checks using a disposable encrypted key.

Crash transport tests cover acknowledgement persistence, new-report detection,
report size limits, envelope/attachment bytes, crash-time release version,
HTTP failure and redirect handling. Sentry accepted synthetic setup events.
Native QA shows the editable consent prompt; declining it survives restart.
No real crash report was uploaded during release verification.

The original background OpenGL crash has not been reproduced or confirmed fixed.
Native Windows/Linux GUI acceptance, Intel macOS, and installed old-to-new
Sparkle update acceptance remain unverified. SSH key prompting is Unix-only.

## Packages and publication

Windows, Linux and browser artifacts came from workflow `41512461727`.
Its macOS job was cancelled by GitHub after runner acquisition failed due to
hosted ARM64 capacity. macOS recovery `41514913602` built successfully and Apple
accepted/stapled the app, but ticket validation lost its CloudKit connection
(NSURLErrorDomain -1005, stapler status 68).

Workflow-only commit `627532d` adds bounded status-68 stapler retries. Recovery
`41517758954` checked out the unchanged v0.4.5 tag and passed all macOS build,
signing and notarization steps. No successful Windows/Linux/WASM build was repeated.

All final SHA-256 checksums match. Windows ZIP CRC, PE executables, Linux ELF
executables and license notices passed. macOS strict/deep signature validation,
Gatekeeper acceptance, app and DMG stapled tickets, signed DMG integrity and the
Sparkle Ed25519 archive signature passed independently. The bundled icon matches
the padded source ICNS. Native Windows/Linux GUI and installed Sparkle update
acceptance remain outside this verification.

Published stable at `2026-11-02T20:41:34Z`:
https://github.com/RustableApp/Rustable/releases/tag/v0.4.5
All seven public latest-download links return HTTP 200. The public latest
macOS feed is byte-identical to the verified v0.4.5 appcast.

## Matching website deployment

Website content commit: `4a66298` (verification documentation: `28961e6`).
Cloudflare deployment: `2b950753-183f-40ae-b481-69a1dcf6402d`.
BUILD.txt reports v0.4.5 and the exact tag commit with clean tracked source.
Three embedding tests passed. Local CI artifact and live demo startup plus
`SELECT 45 AS release_check` passed. Live COOP/COEP and WASM CSP headers passed.
Public WASM SHA-256 matches the exact CI artifact:
`7c552a650de25496c5b17df9b097c02400e3c0a57e5049dc40b517b4f9db6148`.
