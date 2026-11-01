# PostgreSQL generated data changes

The grid can stage updates, inserts and deletes on ordinary/partitioned PostgreSQL
tables with primary keys. Saves use one transaction, exactly the reviewed bound
statements, and require each generated write to affect exactly one row.

Updates and deletes use the **original primary key**, including all components of
composite keys. An optimistic full-row check runs immediately before each write,
locks the matching row with `FOR UPDATE`, and rejects stale data. Primary-key
predicates use equality so PostgreSQL can use the key index. Earlier writes'
trigger effects are checked before later writes; failures roll back the entire
save. Table locks and reviewed metadata fingerprints reject changed table
structure/constraints/triggers. Pending edits remain available after failure.

Parameters are bound as text or NULL and explicitly cast by PostgreSQL using
catalog-derived, fully quoted schema/type identities. This preserves exact NUMERIC
values and delegates validation to the server. BYTEA values use a bound hex
representation. The review displays values inline; execution remains parameterized.

Stored base types, enums, domains, arrays, ranges and multiranges can be staged.
Table browsing uses server text conversion where no native decoder exists, preserving
array dimensions, escaping and NULL elements. Sorting uses the original typed
column. Domain constraints remain enforced. Reviewed type fingerprints include
nested type dependencies, enum labels and domain constraints. Concurrency checks
use native equality for known scalar types and text representations for others.
Direct composite and pseudo types are refused by the planner. Views and tables
without primary keys remain read-only. Generated/identity fields are omitted from
inserts and cannot be edited. Omitted fields use server defaults; explicit NULL
remains NULL. SQL-result arrays of supported scalar, enum and domain elements
are decoded with dimensions, lower bounds, escaping and NULL elements preserved.
Unknown element encodings and interval results still require explicit text casts
for readable display; table browsing automatically uses server text projection.

Schema changes are not generated for PostgreSQL yet. No claims of full PostgreSQL
or TablePlus parity are implied by these data editing capabilities.

Reproducible live regression tests use only the disposable fixture on localhost
port 54329; they refuse to run without explicit opt-in:

```sh
RUSTABLE_POSTGRES_QA=1 mise exec rust@stable -- cargo test db::postgres::changes -- --include-ignored
```

Coverage includes exact decimals, bytea, date/time/UUID/JSON/XML, nonfinite floats,
composite-key edits, defaults/NULL, concurrent modifications, trigger changes,
zero-row trigger suppression, invalid casts, changed schema, and cancellation
while waiting for a locked row. No production connections are used by these tests.

### Interval result display

SQL results decode PostgreSQL `interval` wire values (including domains and array elements) into independently signed months, days and time, e.g. `+14 mons -2 days +01:01:01.1234`. Formatting preserves all microseconds without floating-point conversion or assuming a fixed month/day duration. Explicit component signs allow the displayed text to be parsed with any `IntervalStyle`. The decoder follows upstream [interval_send](https://doxygen.postgresql.org/backend_2utils_2adt_2timestamp_8c.html) and the [date/time type semantics](https://www.postgresql.org/docs/18/datatype-datetime.html). Live tests compare re-parsed scalar wire bytes, not merely PostgreSQL's interval equality (which can equate distinct month/day representations).

### Reviewed structure changes

PostgreSQL tables now support staged add/rename column, data type, nullability and typed constant-default changes, plus create/drop ordinary indexes. The Structure/Index UI uses the same undo/redo and Cmd+S review flow as row editing. Source table identity, columns, constraints, triggers, indexes and recursive type metadata are fingerprinted. Apply takes table locks, checks fingerprints and new target-type definitions, then executes the exact reviewed statements in one transaction. Tables without primary keys and tables containing composite columns can use structure operations even though their row-edit capabilities differ.

Type declarations are parsed as a restricted type-name/modifier/array grammar, resolved against PostgreSQL's catalog, and emitted with quoted schema/type names. PostgreSQL validates modifiers; defaults are typed escaped literals because DDL does not support ordinary bind parameters. Type changes use assignment conversions, without injecting an explicit cast that could silently truncate values. Required custom USING expressions remain a manual SQL workflow.

Limits: data edits and structure edits must be saved separately. Generated/identity type/default modifications, constraint-backed index deletion, column deletion, constraint editing, expression defaults, and mixed schema/data batches are not implemented here. Rename operations are ordered after other column operations so staged edits can refer to their original column names. The current property dialogs and read-only inspector still need the reference's inline editing UX.

Validation: a dedicated live fixture covers no-PK/composite tables, quoted identifiers/defaults, arrays and custom types, rename plus alter, index creation/removal, stale source and nested target-type metadata, unsafe narrowing failures, and all-or-nothing rollback. Native QA verified add-column staging, Cmd+Z/Shift+Cmd+Z undo/redo, review, commit and automatic refreshed structure. Screenshots: `screenshots/postgres-structure-review.png` and `screenshots/postgres-structure-saved.png`.
