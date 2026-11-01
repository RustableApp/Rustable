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
