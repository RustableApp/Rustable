# MySQL language-provider evaluation

`mysql_candidate.rs` is an observation harness for RainLib/lsp_sqls revision
`5a3c53065701cf74e334d8b70ba548f6657ad4e0`. It is not part of Rustable's runtime or
Cargo tests. Copy it into that checkout as `tests/rustable_acceptance.rs`, then run:

```sh
mise exec -- cargo test --locked --test rustable_acceptance -- --nocapture
```

The harness uses an injected fictional catalog, never opens a database, and
prints completion labels and parser diagnostics. A green test means the probe
ran, **not** that the provider meets acceptance. Inspect each observation against
the intended behavior: alias/CTE/derived column availability, UTF-16 cursor
handling, and no suggestions inside strings/comments. Mode-dependent SQL needs
separate session-mode inputs; the inspected candidate API has none.

See `docs/MYSQL-LANGUAGE-SERVER-RESEARCH.md` for results and the adoption decision.

`duck_mysql_candidate.rs` runs the same catalog/completion contexts against
`gentleeduck/duck-sqllsp` revision
`76a1a75dbb602820e89e339a7617049ea234bb0d`. Copy it into
`dsl-completion/tests/rustable_acceptance.rs` and run:

```sh
mise exec -- cargo test --locked -p dsl-completion --test rustable_acceptance -- --nocapture
```

This exercises the public `complete` entry point, including its derived-catalog
inference, using the MySql parser. The current manifest builds PostgreSQL's C
parser transitively even for this MySQL-only probe. Labels are reported with a
bounded sample because a failing context can produce hundreds of candidates.
