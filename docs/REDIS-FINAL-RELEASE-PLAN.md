# Redis completion and release plan

Requested 2026-11-08. This is the bounded acceptance checklist for the final Redis
release for now, extending the existing Redis roadmap goal. It is not full Redis
Insight parity, a browser redesign, or the unrelated TablePlus parity goal.

## Deliverables, in order

- [ ] Connection options: reuse existing verified OpenSSH transport for Redis;
  preserve the Redis TLS hostname across tunneling and own tunnel lifetime with
  its connection. Add client-certificate/key paths with worker-side loading,
  paired configuration validation and no saved key bytes. Cover applicable
  discovery/data credentials separately. Keep unsupported platform/topology
  combinations explicit rather than silently weakening identity verification.
- [ ] Search: enrich the index catalog with existing index metadata; provide a
  practical create/drop index workflow with explicit destructive-action review.
  Compact schema/options controls and use the available editor/results space.
  Avoid implementing a general schema designer or every Search clause.
- [ ] Analyze: opt-in bounded incremental database scan with progress/cancel,
  honest sampled/partial coverage, expiry buckets and locally saved reports.
  Make top-key/namespace rankings visible without oversized chart spacing.
  This is not continuous telemetry, exhaustive memory accounting or a scheduler.
- [ ] Workbench: bounded per-connection persistent automatic command history,
  privacy/clear controls, and distinguishable per-command result sections with
  status/timing/copy. Preserve saved snippets and keyboard execution conventions.
- [ ] IntelliSense: improve common advanced/count-dependent clauses and gate or
  annotate known version/module incompatibilities using cached capabilities.
  No per-keystroke network calls, external language server or exhaustive grammar.
- [ ] Vector search: support a supplied vector, attribute filter and bounded
  result/search settings in the existing similarity workflow. Preserve bounded
  transport and direct server background execution; no vector visualization suite.
- [ ] Editing: JSON path operations with explicit conflict/safety semantics and
  multi-entry creation for supported collections. Keep existing size bounds and
  retained drafts on failure. No general-purpose JSON IDE.
- [ ] Command workflows: evaluate explicit transaction and bounded blocking-command
  support on isolated sessions with cancellation and unknown-outcome handling.
  Never expose connection/protocol-changing commands through the pooled editor.
  Document a concrete safety/platform blocker if a workflow cannot responsibly
  ship in this release; do not label it implemented or remove guards for parity.
- [ ] Polish: Search, new-key forms, Pub/Sub composer alignment, Analyze table
  visibility. Preserve the current browser layout and default value previews.

## Validation and release gates

- [ ] Targeted disposable Cluster/Sentinel abrupt-loss and resharding coverage;
  bounded RESP TLS and batch-error cases. Never replay uncertain writes.
- [ ] Preserve established exchange budgets; count real sequential protocol
  dependencies and run applicable fixtures. SQL sequencing stays unchanged.
- [ ] Same-machine release UI comparisons for changed rendering/input paths,
  including meaningful new workloads. Investigate >10% repeatable p95 regressions;
  retain raw evidence and concrete justifications without weakening thresholds.
- [ ] Native screenshots in both themes and realistic sizes; exercise focus,
  copy feedback, draft retention and available platform/display checks. Record
  unavailable checks without treating them as passes.
- [ ] Full locked tests, strict Clippy, browser-package locked WASM build and
  relevant fixture runs. Update capabilities, roadmap and user-facing release notes.
- [ ] Follow docs/RELEASES.md: promote verified source, bump/tag the next available
  version, inspect signed/notarized artifacts and update feed, publish the release,
  update the website with the exact tagged WASM bundle, verify SQL and Redis-first
  startup, download links, provenance, headers and live checksums.

## Scope control and progress

Finish one functional milestone and its necessary tests before widening scope.
Reuse existing components, controls, stores, transport and test fixtures. Do not
spend repeated rounds adjusting cosmetic details that do not affect clarity or
usability. Report screenshots for visible changes and meaningful milestone status.
Do not release early merely because the remaining work is inconvenient.

Initial audit: working tree clean at `405149e`; latest desktop release v0.4.19.
Existing roadmap documents contain historical pending items superseded by later
entries, so verify the implementation before adding work. The existing persistent
Redis goal was reported paused by the goal API on this request; that API supports
creation and completion/pause/block status updates but no resume operation. The
user's instruction authorizes resuming; the runtime control must resume automatic
continuation. No implementation or validation checkbox is complete yet for this
new batch.
