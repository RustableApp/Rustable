# Rustable v0.4.5

- SQL drafts survive closing their tabs and reconnecting to saved connections. Reopening SQL restores the connection's drafts, and connection windows remember their size.
- A cleaner connection manager with a first-launch engine picker, more compact forms, and clearer SSH settings. **Use SSH config** applies across the connection while still allowing explicit host, port, user and key overrides.
- Encrypted SSH private keys now prompt for their passphrase on macOS/Linux. Passphrases are used for the current connection only and are not saved. This does not add SSH server password authentication or Windows SSH parity.
- Cleaner Structure, Index and Trigger views with clipped long values, hover details and aligned columns.
- Correctly padded macOS app icons in both the bundle and running application.
- Local Rust panic logs and an opt-in prompt on the next launch. Users can review/edit reports before sending them to Rustable's US Sentry project. Reports are never uploaded automatically. Native crashes without a Rust panic still need the operating system crash report.
- A shorter, product-focused README with development instructions moved into separate documentation.

The reported background OpenGL crash has not yet been reproduced or confirmed fixed; this release adds diagnostics to preserve the original panic if it recurs.

The website WASM demo is built from this same release tag.
