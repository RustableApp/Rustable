# MySQL/MariaDB release acceptance

Priority updated 2026-11-01: ship verified MySQL and MariaDB support. Unrelated
TablePlus UI parity, Redis and browser polish are not gates for this milestone.
Do not call support production-ready merely because the driver compiles.

## Release scope

Connections (including verified TLS and SSH), session reuse, schema/table/view
browsing, handwritten SQL and explicit transactions, cancellation, bounded
results, filtered exports, CSV import, and reviewed row insert/update/delete.
Publish the tested server versions and restrictions in release notes. Visual
schema administration and a full MySQL language server are not implemented and
must not be advertised as available.

Reviewed changes require a primary key, supported values/expressions and an
InnoDB target. Trigger visibility must be provable from supported direct grants;
role-only and unsupported wildcard grants currently fail closed. This is a
documented account limitation, not permission to bypass validation. Unsupported
cascade-trigger settings must remain rejected without changing server settings.

## Gates for the next release

- [x] Combined driver acceptance on MySQL 8.4 and MariaDB 11.4, including TLS,
  SSH, SQL modes, result fidelity, cancellation, import/export, reviewed writes,
  conflict rollback and unknown commit/rollback outcomes.
- [x] App-level reviewed-edit and CSV-import integration on both engines.
- [ ] Native MySQL connection, browse/query, insert/update/delete, rejected-write
  recovery, explicit transaction recovery and cancellation acceptance.
- [ ] Finish native MariaDB transaction/cancel acceptance. Native insert, update,
  delete and rejected-write recovery are already recorded in DRIVER_CAPABILITIES.
- [ ] Audit effective-privilege restrictions and test rejection paths; expose
  accurate limitations rather than expanding privileges or silently weakening
  review guarantees.
- [ ] Full local regression and strict native Clippy on the release candidate.
- [ ] Update README/release notes with actual capabilities and tested versions.
- [ ] One batched release build on macOS, Windows and Linux; verify signed,
  notarized drag-to-Applications DMG, downloads and updater feed before publishing.

Keep ordinary work on the development branch. Promote and tag only the checked
release candidate; avoid repeated production builds for individual fixes.

## Candidate evidence — 2026-11-01, source f0c4cd2

Both disposable fixture suites passed `cargo test --locked --lib db::mysql --
--ignored --test-threads=1`: 48 tests each. MySQL used ports 43316/43318 and
MariaDB 43317/43319, with Verify Full CA and the SSH gateway on 43320. Logs:
`/tmp/rustable-release-mysql.log` and `/tmp/rustable-release-mariadb.log`.
No production database was accessed.

`cargo test --locked --lib live_mysql_ -- --ignored --test-threads=1` passed
three tests on each engine: CSV review/import and failed mapping retention;
grid edit/review/save and conflict retention; and plaintext connection probes
not misreporting TLS success. Logs: `/tmp/rustable-release-app-mysql.log` and
`/tmp/rustable-release-app-mariadb.log`. Commands ran through mise.

These suites exercise the stated versions, not every supported version or all
native UI interactions. The optional partial-revokes environment branch was not
enabled in this combined run; previous dedicated evidence is in the driver ledger.
