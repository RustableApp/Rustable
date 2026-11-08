# Rustable v0.4.20

- Redis connections now support verified SSH tunnels and client-certificate authentication, including separate Sentinel discovery credentials.
- Search gains a richer index catalog and reviewed index creation/deletion. Vector similarity search accepts supplied vectors, attribute filters and bounded search settings.
- Analyze can scan the database on demand with progress, cancellation, expiry distribution and saved reports. Top namespaces and top 15 keys stay prominent; reports clearly identify partial coverage.
- Workbench adds bounded per-connection history with privacy controls and separate command results. Completion understands more advanced command clauses and can show cached server-version/module hints.
- Edit individual JSON paths inline with conflict checks, or create collections with multiple entries. Failed edits retain their drafts, and existing keys are never replaced by creation.
- Refined report scrolling and Pub/Sub composer alignment while retaining Rustable's shared controls and virtualized tables.

Redis connections require the desktop app. Module features require compatible servers and permissions; the browser demo is an in-memory subset. Transactions and blocking commands remain unavailable. Database analysis is bounded and is not a point-in-time snapshot. Windows built-in SSH remains unsupported.

The website demo is built from the same release tag.
