# Rustable v0.4.27

- Choose PostgreSQL TLS modes explicitly: Verify full, Verify CA, Require, Prefer, or Disabled. Muted descriptions explain each option before you select it.
- Fix TablePlus and TablePro imports that incorrectly turned PostgreSQL Require into Verify full, preventing connections to some private servers.
- Apply the selected TLS mode consistently to database connections, connection tests, cancellation, and the query monitor.

Existing saved connections keep their security settings: enabled TLS remains Verify full, and disabled TLS stays disabled. New connections still default to Verify full. Require encrypts traffic without verifying the server; Prefer may use an unencrypted connection. Unsupported TablePlus Allow and unknown modes require manual review.

Fixes [#8](https://github.com/RustableApp/Rustable/issues/8). The website demo is built from the same release tag.
