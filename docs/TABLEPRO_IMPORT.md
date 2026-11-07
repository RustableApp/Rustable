# TablePro connection import

Use **Import… → From TablePro…** in the connection manager, then **Choose export
file…**. Select a connections export created in TablePro (`.tablepro` or
unencrypted JSON), not a database file or SQL dump. Unlock it locally if encrypted,
review the preview, and select which
connections to add. Existing profiles are never overwritten; duplicate handling
and explicit credential import use the same flow as TablePlus. Imported database
passwords are stored by the credential worker, never in preferences.

Supported: TablePro JSON format versions 1 and 2, PostgreSQL, MySQL, MariaDB,
SQLite and standalone Redis, nested groups, and TPRO version 1 encrypted exports
(PBKDF2-HMAC-SHA256, 600,000 iterations, AES-256-GCM). File size is bounded at 8 MiB;
decryption and parsing run off the UI thread. Authentication precedes JSON parsing.

Unsupported engines and advanced transport/authentication/certificate settings
are shown as unavailable in the preview. No tunnel command or startup command is
executed. SSH secret material, saved queries, tags, colors, credential profiles
and custom timeouts are not imported. SSH keys/agent configuration and TLS modes
must be reviewed before connecting. Environment variable references and Unix
sockets require manual setup. TLS modes requiring encryption use certificate
verification in Rustable; no insecure fallback is substituted.

Format reference: TableProApp/TablePro, `apple/Packages/TableProCore/Sources/TableProImport/`:
- `Bundle/ConnectionBundleV1.swift`
- `Bundle/ConnectionBundle+Coding.swift`
- `Bundle/ExportableConnection.swift`
- `ConnectionExportCrypto.swift`

https://github.com/TableProApp/TablePro/tree/main/apple/Packages/TableProCore/Sources/TableProImport

Validation covers all five engines, versioned group references, cycle rejection,
credential indexing, unsupported settings, encrypted authentication, wrong passwords,
tampering, truncation and size limits. `src/fixtures/tablepro-v1.tablepro` is a synthetic
export independently generated with Apple CryptoKit/CommonCrypto (passphrase
`fixture-only`, fixed salt 32 × 0x07 and nonce 12 × 0x08); it contains dummy settings only. A user-generated sample export has not yet
been supplied. Included in v0.4.3.
