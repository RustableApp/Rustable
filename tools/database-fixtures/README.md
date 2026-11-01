# Disposable MySQL and MariaDB integration servers

Run from the Rustable repository root:

```sh
docker compose -f tools/database-fixtures/compose.yaml up -d --wait
RUSTABLE_MYSQL_TEST_PORT=43316 mise exec -- cargo test --lib sql::mysql::server_tests -- --ignored --nocapture
RUSTABLE_MYSQL_TEST_PORT=43317 mise exec -- cargo test --lib sql::mysql::server_tests -- --ignored --nocapture
```

These fixtures bind only loopback, use separate ports from developer databases,
and store database contents in tmpfs. Credentials in the compose file are public,
local-test-only values. Tests refuse other ports and use only `rustable_test`.
Stopping/recreating these containers discards their data. Do not use them for
anything you want to keep.

```sh
docker compose -f tools/database-fixtures/compose.yaml down
```

The first live contract test covers custom DELIMITER handling for stored routines,
Unicode, multiple result sets, exact integers beyond JavaScript's safe range,
SQL mode escaping, connection reuse, and rollback. It does not yet prove the
Rustable driver, GUI, TLS, cancellation, safe editing or metadata implementation.

Verified on MySQL 8.4.11 and MariaDB 11.4.13 on 2026-11-01. Images use release-line
tags so test output must record the actual server version on every run.

Value conversion checks (both text-query and prepared-statement protocols):

```sh
RUSTABLE_MYSQL_TEST_PORT=43316 mise exec -- cargo test --lib db::mysql_values -- --include-ignored
RUSTABLE_MYSQL_TEST_PORT=43317 mise exec -- cargo test --lib db::mysql_values -- --include-ignored
```

These compare unsigned BIGINT, exact DECIMAL, Unicode, binary bytes, NULL and
fractional/negative TIME values against Rustable's shared grid representation.
Empty-result column headers are checked without rerunning the query.
Unsigned values beyond i64 and DECIMAL are stored as exact text, avoiding f64
rounding. Text connections must request utf8mb4; invalid UTF-8 is rejected.
