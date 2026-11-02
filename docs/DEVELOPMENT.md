# Development

Use [mise](https://mise.jdx.dev/) to install the toolchain and Cargo for dependencies. The Cargo lockfile is checked in.

```sh
mise run dev
mise run check
mise run build

# macOS app bundle: target/Rustable.app
mise run bundle
```

For an optimized local build:

```sh
mise exec rust@stable -- cargo run --release
```

## Isolated native QA

Build with `mise exec rust@stable -- cargo build --release --features qa` and copy the binary only to `target/Rustable QA.app/Contents/MacOS/rustable`. QA uses its own window title and persistence namespace.

Build normal releases separately with `mise exec rust@stable -- cargo build --release`. Both use the same Cargo output path, so do not copy the QA binary into the normal app bundle. Replace bundle executables atomically and restart only the intended app.

## Further reading

- [Driver capabilities](DRIVER_CAPABILITIES.md)
- [PostgreSQL editing](POSTGRES_EDITING.md)
- [Language servers](LANGUAGE_SERVERS.md)
- [Performance](PERFORMANCE.md)
- [Automatic updates](AUTOMATIC_UPDATES.md)

## Crash reports

Native builds save the first Rust panic (including a forced backtrace) to
`last-panic.log` in eframe's app storage directory. On macOS release builds this
is `~/Library/Application Support/Rustable/`. Development and QA builds use
separate app storage directories. Native faults without a Rust panic still need
the operating system crash report.

On the next launch, the user can review/edit the diagnostic text and explicitly
send it to Rustable's US Sentry project, or choose **Don't send**. There is no
automatic upload, telemetry SDK, query history collection, or session replay.
Home-directory paths are replaced before review; arbitrary panic messages may
still contain sensitive values, so the prompt explains this. The Sentry ingestion
DSN is public client configuration, not an administrative credential.

Sending runs off the UI thread, times out after 20 seconds, and does not follow
redirects. Only a successful HTTP response marks the report as sent. A SHA-256
acknowledgement suppresses repeat prompts for a sent/declined report without
deleting the local log. A different report prompts again. Retries use the same
event ID. Sentry groups reports by panic source location when available.

Run `cargo test --lib crash_log` for local tests. The ignored
`sentry_setup_delivery` test sends a clearly labelled synthetic event to the
real project; run it only when intentionally verifying live ingestion.
