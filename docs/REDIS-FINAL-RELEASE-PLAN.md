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

### SSH transport checkpoint

Redis connection settings now reuse the existing SSH form and OpenSSH gateway
configuration. Ordinary, Cluster-node and Sentinel-discovery/data connections own
an SSH lease through the Redis connection; streaming transports own their lease
through stream drop. TCP forwarding preserves the configured Redis TLS hostname.
No extra warm-path Redis requests were added; SSH authentication/forwarding occurs
only when establishing a new connection. See `vendor/README.md` for the endpoint
and ownership APIs. Windows retains the existing explicit unsupported SSH result.

Verified on macOS: all seven `ssh_tunnel::tests` passed, including isolated live
sshd forwarding, unknown/changed host-key rejection, Redis PING through the
forwarded endpoint, and listener/private-directory cleanup when the Redis
connection drops (`/tmp/redis-ssh-tests.log`). The mutual-TLS fixture also passes
forwarded endpoint checks with correct/incorrect configured TLS hostnames
(`/tmp/redis-ssh-tls-identity.log`). These tests use temporary known-hosts files,
not the user's trust store. Live routed topology and streaming SSH coverage,
connection-form UI/performance checks and release checks remain open. The parent
connection milestone is still incomplete.

### Sentinel mutual-TLS handoff verification

The disposable Sentinel TLS fixture now requires client certificates on both
endpoints and supplies distinct discovery/data client identities. Sentinel uses
its data client identity for monitoring the primary. Its listener trusts the data
CA (also needed for outbound primary verification); server trust remains separate
between discovery and data. This does not claim certificate-subject ACL isolation.

The test exercises an actual discovered-primary PING and parses MONITOR and
SUBSCRIBE acknowledgements. It rejects wrong discovery/data server trust, wrong
discovery hostname, and missing client identity independently on either side for
both sessions and streams. `bash scripts/test-redis-sentinel-tls.sh` passed;
evidence: `/tmp/redis-sentinel-mtls.log`. The fixture removes only its own container
and temporary certificates. Cluster mTLS, SSH topology/stream integration and
connection UI checks remain open.

### Cluster mutual-TLS routing verification

Added an isolated three-primary TLS-only Cluster fixture requiring client
certificates on every node. The live test starts with empty routing, follows
MOVED responses, executes repeated INCR/GET on keys owned by all three primaries,
and cleans up the keys. New node connections reject missing/untrusted client
identities and a wrong TLS hostname; a subsequent valid routed GET proves the
rejected writes did not execute. `bash scripts/test-redis-cluster-mtls.sh` passed
(`/tmp/redis-cluster-mtls.log`). This covers real discovered connections rather
than only calling the node constructor against standalone Redis. SSH topology and
stream integration, UI/performance evidence and the remaining release gates are
still outstanding.

### Connection-form CPU comparison

Compared `6a6b930` with `cb9e130` using
`python3 scripts/ui-performance.py --base 6a6b930 --redis-connections --output target/redis-connection-security-performance`.
Apple M4 Max, macOS 27.2 arm64, mise-selected release toolchain recorded in
`environment.json`; serial builds, five alternating pairs, unchanged standalone
profile workload (20 warmups, 160 measured frames, default egui scale/manager
viewport). Redis profile median p95 was 0.0567 → 0.0579 ms (+2.1%, pass).
The full suite was inconclusive due to noisy vertical-grid tessellation.

A five-pair repeat using the same compiled binaries recorded hashes and raw
p50/p95/p99/max samples in
`target/redis-connection-security-performance-repeat-verified/`. Redis profile
p95 was 0.053584 → 0.056000 ms (+4.5%, pass). Several unrelated grid/sidebar cases
remained noisy, so this is not a whole-suite pass. An earlier repeat attempt
selected an older cached binary and failed the workload-presence check; its
incomplete output is retained but excluded. No thresholds or fixtures changed.
These results cover the default form only, not newly expanded SSH/client-identity
interactions or native input-to-display latency. Those checks remain outstanding;
no overall UI/release gate is checked off from this narrow evidence.

### SSH application-path verification

The isolated live SSH test now also enters through `Session::connect`, the node
constructor used by topology routing, and `Control::start` for MONITOR. It checks
actual responses and drops/stops each transport. A test-only thread-local override
supplies temporary SSH configuration/known-hosts files, restored even on unwind;
gateway selection, credentials, forwarding and strict verification still run
through production code. All seven SSH tests pass (`/tmp/redis-ssh-integration.log`).
The first fixture attempt exposed inherited nonblocking mode on its accepted mock
socket; setting that socket back to blocking fixed the fixture. No application
transport workaround was needed. This is not a live SSH failover/resharding claim.

Native QA build passed. Captured and inspected standalone Redis security options
in dark/light themes at 1200×1080 logical points (2400×2160 pixels):
`target/redis-security-captures/connection-dark.png` and `connection-light.png`.
SSH uses the existing SQL gateway controls and action sizing; no clipping in these
captures. Client-identity expansion, compact viewport and native interaction
coverage remain open. Both isolated preview processes were stopped afterwards.
