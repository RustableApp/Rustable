# Importing TablePlus connections

In the native connection manager, choose **Import from TablePlus…**, select a
`.tableplusconnection` file, and enter its export password. Review the connections
and notices, select the entries to keep, then choose **Import selected**.
Nothing connects automatically. The current connection form is preserved.

The importer accepts RNCryptor v3 password-protected JSON exports and plaintext
JSON with TablePlus field names. It does not read TablePlus's local storage or
Keychain. Files are limited to 8 MiB and 1,000 records. Unsupported formats fail
without changing saved connections.

PostgreSQL, MySQL, MariaDB, SQLite and Redis are supported. Names, addresses,
ports, users, databases, environment labels and connection colors are imported.
Group names are preserved when present; linked nested groups become a flat path.
Exports that omit group names produce a notice and use Ungrouped.

Duplicates default to **Skip**, comparing names (case-insensitive) or complete
connection settings. **Keep both** assigns a new profile ID and a unique name.
Existing profiles are never overwritten. Model validation is atomic.

Database passwords are optional and unchecked by default. When requested,
explicit password values go through Rustable's existing OS credential store;
password-provider commands are never treated as passwords. Credentials are not
serialized into connection preferences. Credential-store failures are reported
by the connection manager. Export passwords and decrypted byte buffers are
cleared when dropped; they are never written to disk or logged.

SSH host/port/user settings are mapped for SQL connections. SSH passwords and
private keys are not imported. Configure the SSH agent or select a key manually.
Redis-over-SSH, Unix sockets and encrypted/remote SQLite entries are rejected.
Startup SQL and shell commands are never imported or executed.

TLS requires review: explicit mode 0 maps to disabled; other/missing modes enable
Rustable's certificate-verifying TLS. TablePlus modes are not a one-to-one match
for Rustable. Custom certificates, client keys, role/authenticator and advanced
driver options need manual setup. The preview reports these limitations.

## Format evidence and validation

- [TablePlus export documentation](https://tableplus.com/docs/gui-tools/manage-connections)
- [TablePlus export-format discussion and record example](https://github.com/TablePlus/TablePlus/issues/2018)
- [RNCryptor v3 specification](https://github.com/RNCryptor/RNCryptor-Spec/blob/master/RNCryptor-Spec-v3.md)

The cryptographic implementation authenticates before decrypting and is tested
against the upstream password vectors (including Unicode), wrong passwords and
tampered files. Parser tests cover engine mapping, unsupported records, duplicate
imports, groups/colors and credential exclusion from saved settings.

Verified decryption and parsing against a user-supplied TablePlus export on
2026-11-01. Its top-level array mixes connection records with group objects whose
`connections` and `groups` arrays contain descendants. The importer now traverses
that structure, preserving group paths, with a synthetic regression test.
The sample contained 10 connections: six importable and four blocked (two missing
hosts and two Redis SSH tunnels). Verification was preview-only: no profiles were
saved and no database connections were made. The private export is not a fixture
and is not included in the repository. Native picker interaction remains a manual
UI check; this verification exercises the same file reader, decryptor and parser.

To run an explicit local compatibility check, set `RUSTABLE_TEST_EXPORT` and
`RUSTABLE_TEST_EXPORT_PASSWORD` and run `cargo test --locked --lib
 tableplus_import::tests::local_export_compatibility -- --ignored --nocapture`.
The check prints counts and validation errors, not connection details or passwords.
