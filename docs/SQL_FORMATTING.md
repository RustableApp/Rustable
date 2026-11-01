# SQL Beautify

The SQL editor's Beautify button and Cmd+I (Ctrl+I outside macOS) format the
selected text, or the full document if no text is selected. Formatting preserves
keyword case. It never runs SQL or writes a file. One Undo restores the preceding
SQL and selection; Redo reapplies the formatted text.

Formatting runs on a background thread, with one in-flight request per editor.
Typing remains available. If the document changes before the reply is applied,
the reply is discarded and a visible message asks the user to run Beautify again.
Closing, archiving or reconnecting an editor drops the reply receiver. Formatting
is limited to 1 MiB documents, 100,000 selected tokens and 128 nested parentheses;
output above 8 MiB is rejected. These are workload limits, not process RSS limits.

Implementation uses [sqlformat 0.5](https://docs.rs/sqlformat/0.5.0/sqlformat/)
and independently checks raw token streams using
[sqlparser 0.63](https://docs.rs/sqlparser/0.63.0/sqlparser/tokenizer/index.html).
The dialect is PostgreSQL or SQLite according to the connection. Whitespace may
change, but tokens, raw quoted contents and comments must compare equal, including
significant newlines between adjacent single-quoted strings. The entire document
is checked after replacing a selection to catch changes across selection edges.

The formatter receives temporary stand-ins for quoted values/identifiers and
comments, then their exact original bytes are restored. This preserves PostgreSQL
dollar-quoted function bodies, escaped strings, nested comments, SQLite bracket
identifiers, Unicode and CRLF comment terminators. Adjacent string literals stay
together with their original separators, preserving PostgreSQL newline-based
concatenation. Marker prefixes are absent from the input, and restoration uses
one regex pass rather than repeated whole-document replacement. Token locations
are mapped to UTF-8 byte offsets with one forward scan.

This is conservative formatting, not a semantic validation service. Constructs
that still fail the independent token comparison are rejected rather than
rewritten. The editor retains the original text and shows the reason. Complete
dialect formatting remains a parity gap; procedural body internals are preserved,
not reindented.

Verification: tests cover Unicode selection boundaries, unchanged unselected
prefix/suffix, incomplete quotes, significant string-separating newlines, dialect
literal/comment/operator preservation, stale asynchronous replies, and rendered
Cmd+I -> Cmd+Z -> Shift+Cmd+Z. Full suite: 335 passed, 35 opt-in tests ignored;
strict all-target/all-feature Clippy clean. Native QA verified Cmd+I formatting,
Cmd+Z restoring exact original SQL, and Shift+Cmd+Z restoring formatted SQL.
The rounded Beautify button and shortcut fit the right-aligned editor toolbar.
[Native screenshot](screenshots/sql-beautify.png). Both app bundles rebuilt,
including dependency license notices.

Dialect follow-up: 337 regular tests passed, 36 opt-in tests ignored. Added
required-success coverage for function bodies, comments, escaped and quoted text,
CRLF/Unicode and adjacent strings. A live in-memory SQLite comparison verifies
formatted and original values, including blobs and marker-like strings.

Release measurement (local Apple M4 Max): 105,780-byte PostgreSQL script, 2,000
statements, two warmups and seven measured runs. End-to-end formatting plus token
verification p50 28.30 ms, maximum 29.05 ms. This is worker CPU work, not UI
input-to-paint latency. Command: `mise exec rust@stable -- cargo test --locked
--release formatting_latency -- --ignored --nocapture`.
