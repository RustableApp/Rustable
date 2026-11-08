# Redis editor intelligence

The CLI and Workbench share a local completion provider and the SQL editor's
execution shortcuts. Type a command prefix or use Ctrl+Space. Up/Down selects,
Tab or Enter inserts, Escape dismisses. Command/Ctrl+Enter runs the current
command or selection; Shift+Command/Ctrl+Enter runs all commands. Running dismisses
completion. Choosing a history entry focuses the editor.

Suggestions replace the current token, including when the caret is inside it.
Quoted newlines, Unicode and escaped bytes remain part of their argument. Key and
field insertions use the same reversible quoting as command execution.

The bundled catalog has 392 core commands/subcommands from Redis 7.2.6 and 35
Rustable-authored module/vector command signatures. Command help displays syntax,
summary and expected argument names. A generated grammar follows mandatory,
optional, repeated and alternative arguments; trailing keyword options can be
entered in any order and mutually exclusive option groups are suppressed after use.
This is completion assistance, not a server-side syntax validator. Count-dependent
arguments and module signatures with more advanced clauses are not exhaustively
validated. Server version, module availability and ACL permissions still determine
what can execute; suggestions never bypass the command runner or safe mode.

Local context includes the sorted loaded key cache (up to 10,000 names), 1,000 already loaded Search
index names and 200 fields/members from the open inspector when its key matches the
command. It does not scan Redis or fetch fields while typing. Large editors over
64 KiB and commands over 64 preceding tokens skip completion. Menus are capped at
50 items and virtualized; suggestions are recomputed on edits/caret/context changes,
not every idle frame. Key lookup uses a binary prefix search rather than scanning the cache. Explicit Ctrl+Space refreshes cached suggestions.

Metadata provenance and BSD license are in `assets/redis/`. Regenerate the static
catalog with `python3 scripts/generate-redis-completions.py`. Desktop and browser
packaging include its notices. Do not substitute metadata from redis/docs, whose
current documentation license imposes different conditions, without reviewing it.

Performance and screenshot validation are recorded in UI-PERFORMANCE.md and the
Redis roadmap. This provider runs on native and WASM without an LSP process.

The unreleased completion pass adds exact key-count boundaries for script/function
calls, sorted-set union/intersection/difference and STORE variants, multi-pop
commands and intersection cardinality. WEIGHTS consumes one value per declared key;
subsequent options are suggested only after those values. Zero-key scripts switch
to ordinary arguments. These hints remain local and do not enable blocked commands.
