# Redis completion and release plan

Requested 2026-11-08. This is the bounded acceptance checklist for the final Redis
release for now, extending the existing Redis roadmap goal. It is not full Redis
Insight parity, a browser redesign, or the unrelated TablePlus parity goal.

## Deliverables, in order

- [x] Connection options: reuse existing verified OpenSSH transport for Redis;
  preserve the Redis TLS hostname across tunneling and own tunnel lifetime with
  its connection. Add client-certificate/key paths with worker-side loading,
  paired configuration validation and no saved key bytes. Cover applicable
  discovery/data credentials separately. Keep unsupported platform/topology
  combinations explicit rather than silently weakening identity verification.
- [x] Search: enrich the index catalog with existing index metadata; provide a
  practical create/drop index workflow with explicit destructive-action review.
  Compact schema/options controls and use the available editor/results space.
  Avoid implementing a general schema designer or every Search clause.
- [x] Analyze: opt-in bounded incremental database scan with progress/cancel,
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

Compact form verification passed at 1000×760, covering expanded certificate/key
inputs, scrolling through SSH controls and fixed actions. The file-picker test
confirms selecting an SSH identity leaves TLS identity paths intact
(`/tmp/redis-security-ui-test.log`). Existing configured client identities now
expand on first display; empty optional sections remain collapsed. Native compact
capture inspected at `target/redis-security-captures/connection-compact-expanded.png`;
text wraps and actions remain visible. Added a dedicated expanded-security CPU
workload at compact/large viewports with scrolling and a simulated idle time jump;
its release measurements are pending, not covered by the prior default-form pass.

### Connection milestone status

Functional connection milestone complete: verified OpenSSH settings/ownership,
preserved TLS host identity, paired worker-loaded client certificates, independent
Sentinel discovery/data configuration, live Cluster redirects, and normal/node/
streaming SSH paths. Windows SSH remains explicitly unsupported by the existing
transport. Live SSH failover/resharding composition and broader native/display
checks are not claimed; the release validation gates above remain open.

Expanded security-form release workload completed on the same Apple M4 Max/macOS
27.2 with mise Rust 1.99.1, default egui scale, 20 warmups and 160 samples at each
viewport. Compact 1000×760 p95: 0.074625 ms; large 1400×1080 p95: 0.081208 ms.
Raw p50/p95/p99/max and all samples: `target/redis-security-expanded-performance/`.
This is new-workload CPU evidence, not an input-to-display or comparative pass.
Together with the earlier default-form comparisons and compact functional/capture
checks, it completes the targeted connection validation for this milestone;
full-suite noise and release-native checks remain explicitly open.

Next: Search catalog metadata and reviewed create/drop actions. The current
catalog performs one FT._LIST exchange and fetches FT.INFO only after selection.
Enrichment must batch bounded metadata for a catalog page, cache it until explicit
refresh, preserve names when one index disappears or lacks metadata permission,
and keep schema/query loading independent. Create/drop must validate again on the
worker under safe mode; dropping an index must not implicitly delete its documents.

### Search catalog implementation checkpoint

The catalog now displays prefixes, field types, document count and field count
in the existing styled table. Pages contain at most 50 indexes; metadata stays
cached until explicit refresh. FT.INFO requests for a page are sent together
through the bounded RESP pipeline (1 MiB per reply, at most 50 MiB decoded input
per batch). Display strings are capped and schemas are not retained in catalog
cache. Per-index server errors remain local to their rows; transport/parser
failure closes the connection and is never replayed. Cluster catalogs use the
same seed as the existing keyless Search commands.

Exchange budget: previously names-only discovery cost one FT._LIST exchange.
Cold enriched discovery costs that exchange plus one metadata pipeline for the
requested page; a new page costs one pipeline, cached revisits zero exchanges.
Names returned by FT._LIST are needed to construct FT.INFO requests and the list
reply does not contain metadata, so the additional cold exchange is necessary.
No per-row sequential wait or per-frame network operation was introduced. Existing
Search query and selected-schema execution paths are unchanged. A protocol fixture
requires both FT.INFO commands to arrive before sending either reply; with an
80 ms simulated response delay, the batch completed in 91.25 ms. This measures the
metadata operation, not full native input-to-display or SSH latency.

`cargo check --locked` and eight Search tests pass; four explicit live/benchmark
cases remain opt-in. The separate disposable Redis 8.4 Search fixture also passed
real catalog metadata plus a missing-index error and subsequent PING. Evidence:
`/tmp/redis-search-catalog-tests.log`, `/tmp/redis-search-catalog-live.log`.
Create/drop management, native captures and final Search performance comparison
remain in progress; the Search milestone remains unchecked.

### Search index management checkpoint

Added inline create/drop controls using the existing button and icon conventions.
Creation supports HASH/JSON, one optional key prefix and 1–32 TEXT/TAG/NUMERIC/GEO
fields with aliases and sortable flags. JSON paths require aliases; duplicate
aliases and invalid bounds are rejected. Empty prefix explicitly means all keys.
Drop review displays the index name, explains query impact/document preservation,
and requires typing DROP. Commands use argument encoding; FT.DROPINDEX never sends
DD. Form drafts are cleared only on successful completion or explicit cancel.

Mutation jobs carry safe mode to the worker, which validates before constructing
or sending a command. Writes use the existing single-command path and never replay
an uncertain result. Successful operations update the cached name list locally;
the next visible metadata page uses the existing bounded batch. No preflight
existence query or automatic replacement of an existing index was introduced.
Validation borrows draft fields instead of cloning their text on every frame;
full drafts clone only on submit. Input sizes and field counts are bounded.

Nine targeted Search tests passed. The disposable Redis 8.4 fixture verified HASH
and JSON creation, safe-mode rejection of create/drop, duplicate-create errors,
and preservation of indexed documents after drop (`/tmp/redis-search-manage-live.log`).
Separate browser-package locked WASM check passed with platform dead-code warnings
(`/tmp/redis-search-manage-wasm.log`). Native Search captures, draft/error interaction
coverage and comparative UI measurements remain pending; Search is not marked complete.

### Search native and interaction checkpoint

The disposable native capture script `scripts/preview-redis-search.sh` captured
all six Search states at 1200×760 logical pixels: catalog, loaded metadata, inline
create, reviewed drop, selected schema and query. The catalog, create and drop
PNGs were visually inspected in `target/redis-search-management-captures/`.
Ten targeted Search tests pass (five explicitly ignored live/performance tests);
the new pointer interaction test covers draft retention after create failure and
requires both typed confirmation and unlocked editing before drop submission.
The performance runner's eight tests pass.

The shared Search page workload now prepares a loaded metadata cache instead of
accidentally dispatching requests to a disconnected fixture worker. The baseline
adapter marks the older names-only catalog loaded; neither measurement performs
network work. The richer catalog displays a bounded page of the same 200 index
names. A release comparison against `dc417bf` is running under
`target/redis-search-final-performance/` (log `/tmp/redis-search-final-performance.log`);
no performance pass is claimed yet. A separate create-eight-fields/drop-review
CPU workload was added and remains to be run. Light-theme native capture and
final release-wide platform checks remain open.

### Search performance decision

The five-pair same-machine release comparison finished (exit 1, explicitly a
regression result, not a pass). Search median p95 rose 0.063208 → 0.126250 ms
(+99.7%, +0.063042 ms). Median p50 was 0.057208 → 0.116709 ms; median p99
0.066042 → 0.135208 ms; worst recorded sample 0.087208 → 0.162667 ms.
All other measured scenarios passed. Full hardware/toolchain/sample metadata and
raw distributions are retained in `target/redis-search-final-performance/`.

Accepted narrowly for the requested richer catalog: the previous two-column
name/action table becomes six columns with prefixes, field types, document/field
counts and a reviewed drop action. Both use virtualized TableBuilder rows;
metadata is cached outside rendering and fetched once per bounded page. The
comparison keeps the same 200 index names, viewport and workload; candidate
paginates 50 names. This is additional useful visible work, not a hidden network
wait. Hiding the metadata behind per-index dialogs would defeat the requested
catalog; reducing row visibility just to pass would change the workload. Caching
formatted numeric labels would not remove the additional layout/hit-test cells;
replacing shared controls with custom painting is disproportionate at the
measured 0.063 ms cost and would risk interaction/accessibility consistency.
The 10% threshold remains unchanged. This is not a native FPS/input-latency claim.

The new release-only management workload ran from the exact Cargo-reported head
test executable (no rebuild), 20 warmups + 160 samples at 1000×760 with a simulated
idle gap. Create-eight-fields p50/p95/p99/max: 0.057917/0.067916/0.071750/0.071834 ms;
drop-review: 0.023041/0.028917/0.029250/0.029292 ms. Raw results and executable path
are in `target/redis-search-management-performance/`. These new interactions have
no prior implementation baseline; results are absolute CPU evidence only.

Search functional milestone complete: light-theme capture of all six states also
succeeded against the disposable fixture, with create and query views visually
inspected (`target/redis-search-management-light-captures/`). This completes the
functional milestone and targeted validation, including the explicitly accepted
rendering cost above; it does not close release-wide native/platform or full-suite
gates. Next milestone is bounded database-wide Analyze.

### Full catalog page correction and Analyze collector

Transport reuse review found that Search's 50-index page exceeded the bounded
reader's earlier 16-reply admission limit. The prior two-index fixture missed
this: the functional milestone evidence was insufficient for a full page. The
reader now admits up to 50 replies with an explicit 64 MiB total input budget
and unchanged 16 MiB individual maximum. Search remains one exchange per page,
with 1 MiB per FT.INFO reply (50 MiB maximum page input). A protocol test now
requires all 50 commands before sending any replies, retains a per-index server
error, verifies later replies and PING, and rejects over-budget batches before
sending bytes. It passed with the same 80 ms simulated delay
(`/tmp/redis-search-full-page-test.log`).

Analyze now has a worker-oriented collector/report model in `redis_analysis.rs`.
It deduplicates SCAN results, caps collection at 100,000 unique names / 16 MiB
of name bytes / 1,000 pages, checks cancellation and a 60-second collection
budget between reads, groups expiry buckets and types, and retains top 15 keys
and namespaces while totals include all measured keys. Complete means cursor
traversal finished, never snapshot consistency. Cancellation/errors preserve
partial measured results. Metadata requests are bounded to 200 keys in the
source interface; the transport implementation still needs to supply bounded
RESP reads and suitable pipelining. Five model tests passed, covering duplicates,
missing keys, expiry boundaries, partial cancellation, name/page bounds, and
serialization. Network integration, UI, saved-report storage and their fixtures
remain outstanding; Analyze is not marked complete.

### Analyze transport checkpoint

Session now implements the collector source: bounded SCAN (1 MiB response) and
one pipelined MEMORY USAGE SAMPLES 5 / PTTL / TYPE batch per up-to-200 new keys.
Metadata replies are limited to 16 KiB each. The opt-in bounded batch reader now
admits 1,000 replies within the unchanged 64 MiB aggregate input cap, avoiding
serialized sub-batches for 600 scalar replies. Its parser is shared with a bounded
receive API used by Cluster fan-out; ordinary unbounded callers are unchanged.
Cluster metadata redirects preserve the response limit. Protocol/transport
failures close affected connections; server errors are drained and reported,
never silently omitted from a supposedly complete batch.

Warm standalone collection takes one SCAN exchange followed by one metadata
pipeline exchange per 200 unique returned keys (COUNT is advisory, so a larger
page can require more metadata batches). Cluster retains independent-node fan-out
and existing bounded waves; topology redirects can add recovery exchanges. No
preflight PING or per-key sequential metadata is added. The UI integration will
create a dedicated report Session through existing verified settings so Cluster
scan state cannot overwrite Browse's cursor; initial authentication/discovery
costs apply once per explicitly requested report. That lifecycle is not wired yet.

`cargo check --locked` passed with temporary unused-code warnings for the not-yet
wired collector. The 200-key protocol test withheld all replies until all 600
commands arrived, then checked normal and server-error batches followed by PING
(`/tmp/redis-analysis-transport-tests.log`). Fourteen Cluster tests passed (four
live cases ignored), including expanded warm/cold normal/bounded fan-out and
1,000-command pipelines (`/tmp/redis-analysis-cluster-tests.log`). New bounded
Cluster SCAN test verifies a valid page and immediate rejection/connection stop
on an oversized declared response (`/tmp/redis-analysis-scan-test.log`). Existing
bounded-response cleanup regression also passed. Live Analyze integration, TLS
coverage, UI and storage remain required; this milestone is still incomplete.

### Analyze UI and local report integration checkpoint

The native Analyze page now starts a dedicated verified Session per explicit
report, with atomic progress/cancellation and a single-result worker channel.
Browse's cursor and connection are untouched. Cancellation is checked between
reads; the UI explains that an in-flight read must finish. Closing the workspace
sets cancellation without joining on the UI thread. Redraws are scheduled only
while report progress is active. Completion prepares table strings once.

The page shows honest completion/partial status, generation time/database,
measured-key/memory totals, type and expiry tables, top namespaces and top 15
keys using shared styled report tables. Save/Open use worker-side native file
dialogs, a versioned bounded 1 MiB report format and atomic temporary-file
replacement. Reports contain names/statistics but no values or credentials.
Save completion reports success; cancelled dialogs do not claim success.
Failed collection/loading preserves the previous report. Seven targeted model
and UI/storage tests pass (`/tmp/redis-analysis-view-tests.log`), including idle
settling and file limits. `cargo check --locked` passed.

Native preview now waits for an actual report worker result before capturing
Analyze. QA build is running (`/tmp/redis-analysis-preview-build.log`). Still
required: disposable live report checks, screenshots in both themes, a meaningful
release UI comparison for the new report, cancellation under network delay, and
reviewing the browser-demo presentation (it still uses the older loaded-key
report path). Analyze remains unchecked until these are addressed.

### Analyze live fixture and native visual check

`scripts/preview-redis-analysis.sh` now creates its own ephemeral Redis 8.4
instance with 650 keys (600 strings, 20 hashes, 20 lists, 10 sorted sets), runs a
live collector assertion, and captures the native page after its dedicated worker
finishes. The test verifies exact counts, expiry total, multi-page traversal and
nonzero memory estimates. Cancelling after a real metadata batch preserves a
partial report and the connection still answers PING. It passed in both capture
runs (`/tmp/redis-analysis-live-preview.log`, `/tmp/redis-analysis-light-preview.log`).
This is between-batch cancellation evidence, not yet delayed-read cancellation.

Dark and light screenshots were inspected at 1200×760 logical pixels:
`target/redis-analysis-captures/analyze.png` and
`target/redis-analysis-light-captures/analyze.png`. Visual review caught rankings
below the fold and fallback icons. Rankings now come first, side by side on wide
windows; existing folder/list-upload/refresh icons identify the report actions.
Shared action buttons also now correctly resolve the previously requested trash
icon. No browser layout or default previews changed.

The release-mode comparison against pre-Analyze `188e34e` is running in
`target/redis-analysis-final-performance/` (process log
`/tmp/redis-analysis-final-performance.log`). Both sides now prepare the same
1,000 key names / 1 KiB-per-key sample outside measurement; the older report
displays its existing loaded-key analysis, while the candidate additionally
displays type/expiry and full-report controls. The baseline compatibility adapter
and fixture are retained with measurement metadata. Runner's eight tests pass.
No rendering-performance result is claimed until this run finishes.

### Analyze functional milestone completion

The delayed-network cancellation test passed
(`/tmp/redis-analysis-delayed-cancel.log`): cancellation is signalled while SCAN
is awaiting an 80 ms delayed reply. The worker drains that reply, emits no
metadata command, returns a cancelled report, and the connection answers PING.
This proves cancellation between network reads; it does not claim instantaneous
interruption of an in-flight socket/TLS read.

The browser demo no longer offers the nonfunctional loaded-key report workflow.
It uses the same report action labels with an explicit desktop-only state because
its in-memory sample data has no real Redis server memory or slow log to measure.
No fabricated server measurements are shown. The separate browser-package locked
WASM check passed (`/tmp/redis-analysis-browser-check.log`, 76 platform/dead-code
warnings; not warning-free). Live browser visual/provenance verification remains
a final release gate.

The five-pair release rendering comparison against `188e34e` finished in
`target/redis-analysis-final-performance/`: Analyze median p50/p95/p99 was
0.093958/0.101542/0.110209 → 0.113583/0.123000/0.134083 ms. Maximum observed
sample was 0.231833 → 0.233584 ms. p95 increased 21.1% (+0.021458 ms), a
regression explicitly accepted for the new expiry/type breakdown, report status
and controls alongside the requested rankings. It is not a threshold pass.
Prepared table strings and bounded/virtualized tables are already used. Removing
the additional breakdown would drop a requested feature; hiding all analysis
behind collapsible panels merely to reduce measured work would change the
workload. Custom painting or further micro-optimization at this measured cost
would duplicate shared controls and expand the polish scope. The unchanged 10%
threshold and full distributions remain recorded. One unrelated workspace case
was inconclusive from noise; other Redis pages passed. This does not prove native
presentation/input latency or close the release-wide performance gate.

Analyze is functionally complete with targeted collector/protocol/live/UI/storage
checks and inspected native screenshots. Shared TLS/Cluster failure fixtures and
full-suite/native-platform checks remain release-wide gates, not silently waived.
Next deliverable: bounded per-connection persistent Workbench history and distinct
per-command result sections.


### Workbench checkpoint — bounded persistent history

Saved connection profile IDs scope automatic command history through the existing
shared preference store. Each history retains at most 100 entries and 256 KiB;
the 16 most recently updated connections are retained. Unsaved connections and
the WASM demo remain session-only. Recording is a global setting independent of
history eviction; Clear removes the current connection's history. Authentication
and administrative credential commands are excluded, including rejected attempts.
Ordinary command values can still contain sensitive text; the controls disclose
this. Named snippets and execution shortcuts are unchanged. No database requests
were added (zero cold/warm exchange impact).

Verified: five matching history tests passed, including saved-profile isolation,
serialization/reload, clearing and disabled recording
(`/tmp/redis-history-integration-tests.log`). Two preference tests passed, covering
legacy loading, eviction and preserving recording opt-out across eviction/reload
(`/tmp/redis-history-preferences-tests.log`). UI screenshots and release performance
comparisons will cover the completed Workbench milestone together. Distinct
per-command results remain unimplemented, so the milestone remains unchecked.

### Workbench checkpoint — individual execution results

The editor now receives a bounded report with separate command headers, completion
status, elapsed query time and per-result copy controls. Successful replies survive
a later failure; remaining commands are labelled Not run and are never sent.
The entire batch is still validated before the first command, including Cluster
slot validation. Existing recovery delivers the report before reconnecting and
never replays the batch. Query timing excludes display formatting.

The editor now uses the existing 1 MiB bounded-response transport per command,
formats/releases each reply immediately, and shares a 256 KiB successful-output
budget across at most 100 commands. Error explanations are separately bounded.
Exchange count remains one sequential exchange per executed command, on both cold
and warm established sessions (connection setup and Cluster redirects unchanged).
Sentinel READONLY and uncertain-write connection-loss explanations remain explicit.

Two report model tests passed (partial failure and shared display bounds), plus
two execution tests for demo and a disposable local RESP peer. The peer proves
upfront rejection sends no writes, earlier output survives server errors, later
writes are not sent, PING remains aligned, and an oversized declared reply closes
the connection without allocating the declared body. Logs:
`/tmp/redis-command-results-tests.log`, `/tmp/redis-command-report-tests.log`.
This is an implementation checkpoint, not milestone completion: native screenshots,
interaction/performance validation, browser compilation and affected topology
regressions remain to be run. UI result layout has not yet been visually verified.

### Workbench visual and browser checkpoint

Fixed the report module's WASM value alias after the browser-package check exposed
it. The locked wasm32 browser-package check passed (78 existing/platform dead-code
warnings, not a warning-free build). Native QA build passed. Added an ephemeral-port
`preview-redis-workbench.sh` fixture and inspected real native captures at 1200×760
logical points in both themes:
`target/redis-workbench-captures/command-editor.png` and
`target/redis-workbench-light-captures/command-editor.png`.
Per-command copy icons now share the header row; the common copy-feedback helper
is reused. This visual capture is not an automated proof of click/focus behavior.

The shared Workbench benchmark now supplies identical 100 reply values on both
revisions: the old version renders combined output; the candidate renders sections.
Setup is excluded from frame measurements. An explicit baseline adapter preserves
that comparison. All eight performance-runner unit checks passed. The actual
release comparison remains pending; do not infer a performance pass from captures
or fixture checks. Logs: `/tmp/redis-workbench-browser-check.log`,
`/tmp/redis-workbench-preview-build.log`, `/tmp/redis-workbench-preview.log`,
`/tmp/redis-workbench-light-preview.log`, `/tmp/redis-workbench-runner-tests.log`.

### Workbench transport audit follow-up

A review during the release comparison's baseline compilation found that the
Cluster bounded-query entry point still rejected every command except VSIM.
Removed that obsolete restriction; routing continues to use command_slot and the
existing MOVED/ASK implementation. Also removed the bounded-path special error
message so a lost reply consistently explains that a submitted write may have
completed and was not replayed. Added a disposable protocol-peer regression for
GET, server-error draining, PING and a lost INCR reply with router shutdown.
This new test has not yet run: avoid competing test/build work while performance
comparison session 47793 is live. The rendering fixture and UI implementation were
not changed during the comparison; the transport-only correction was made before
candidate compilation and noted in its environment artifact.

Performance run: `target/redis-workbench-performance`, baseline `a495acc`, five
paired runs with `--redis-pages --redis-editor`. Still in progress; no timing or
pass claim yet. The milestone remains open for that result, the new Cluster test,
additional interaction checks and affected Sentinel coverage.
