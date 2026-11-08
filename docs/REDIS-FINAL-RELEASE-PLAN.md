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
entries, so verify the implementation before adding work. The user deleted the old
paused goal and explicitly requested a new goal using this checklist and its scope
boundaries. A fresh persistent goal now tracks this batch. No implementation or
validation checkbox is complete yet for this new batch.

### Connection milestone — client identity plumbing

Implemented paired client-certificate/private-key path settings for Redis data
connections and independent Sentinel discovery. The existing shared TLS connector
loads PEM files on its worker with a 1 MiB per-file limit and passes them to the
Redis driver's existing verified TLS identity configuration. Cluster endpoints
inherit the data identity through cloned settings; Sentinel seed settings copy
only the discovery identity. No commands or sequential protocol exchanges were
added (cold/warm setup sequences unchanged; certificate authentication belongs to
the existing TLS handshake). Only filesystem paths serialize into profiles.
The connection forms expose optional certificate/key paths in a collapsed section.

Evidence so far: `cargo check --locked` passed; all four Redis settings tests
passed, including old-profile compatibility, paired identity validation, TLS
requirement, serialization and independently configured Sentinel identity.
Logs: `/tmp/redis-mtls-check.log`, `/tmp/redis-mtls-settings-tests.log`.
This is partial implementation: live mutual-TLS acceptance/rejection, UI capture
and performance checks are still required. SSH remains unimplemented. The parent
connection milestone remains unchecked until those requirements are verified.

### Live mutual-TLS verification

Added `scripts/test-redis-mtls.sh`: a disposable Redis 7.4 container, ephemeral
loopback port and separate test CAs; cleanup removes only its own container/files.
The live test passes valid client certificate authentication, repeated PING,
raw/node connector reuse and authentication on the dedicated streaming transport.
It rejects no identity, an untrusted client certificate, a mismatched private
key, wrong server hostname, untrusted server CA and missing key files on both
ordinary and streaming connections. An oversized PEM is rejected before connect.
The transport audit found Pub/Sub/MONITOR build TLS separately; that path now uses
the same bounded PEM reader and native TLS client identity support.

Verified: `bash scripts/test-redis-mtls.sh` passed (latest log
`/tmp/redis-mtls-live.log`). These checks do not yet prove an actual Sentinel
mTLS discovery/data handoff or routed Cluster mTLS behavior, nor SSH. Those and
connection-form screenshot/performance validation remain part of the open milestone.
