# v0.4.0 release verification

## Local validation

- 493 native tests passed; 114 fixture/manual tests ignored.
- Strict native all-target Clippy passed.
- Five packaging tests passed, including font notices.
- PostgreSQL, MySQL, and MariaDB foreign-key fixtures each passed against disposable local servers.
- Separate browser-package locked WASM check passed; existing browser-only warnings remain.
- TablePlus encrypted import tests include official RNCryptor vectors and a locally verified user-provided export. No private export is committed.

## Publication

Signed platform packages, matching WASM demo deployment, and public download
verification are pending. See RELEASES.md for the required publication gates.
