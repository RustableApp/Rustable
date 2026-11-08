# Redis key browser

This records the initial browser milestone. Subsequent value-inspector and editing
work is documented in [REDIS-VALUE-INSPECTOR.md](REDIS-VALUE-INSPECTOR.md) and
[REDIS-ROADMAP.md](REDIS-ROADMAP.md), which supersede the initial feature limits.

The Redis workspace has namespace-tree and key-list views, a combined pattern/type
filter, and an exact-key search mode. Tree grouping defaults to `:`; change the
separator to match the application's naming convention. Expansion and selection
survive refreshes within the open workspace. Database changes clear that state.
They are not currently persisted across closing/reopening the connection.

Folders are virtual groups of **discovered** key names, not Redis directories.
Their counts say “loaded” even after a scan completes: SCAN is not a consistent
snapshot. Keys can expire or change while browsing. An empty batch is not the end
of a scan unless its cursor is zero. Repeated keys are deduplicated by raw bytes.
Keys can coexist with namespaces sharing their prefix, and binary names remain
byte-preserving. Exact search accepts literal text or quoted `\xHH` byte escapes.

Scanning starts automatically and can be paused/resumed. Pause stops subsequent
batches; an already submitted read finishes under the existing socket timeout.
Changing filters discards an outstanding discovery/preview result before the new
scan begins. Selection is queued ahead of further discovery; no extra connection
or health-check command is opened to make this work. Dirty values still require
save/discard before navigation, and write safety/unknown-outcome handling is intact.

The existing 10,000-key / 16 MiB key-name limit remains. Preview caches only contain
admitted keys, and enrichment retains the existing batches of up to 200 admitted keys per job. Tree
flattening is cached between changes, with only visible rows rendered. Pathological
names fall back to displaying the remaining suffix after 16 levels or 512 prefix
bytes. Expansion state is capped at 10,000 entries. These are retained-data bounds,
not Redis protocol response-byte bounds: SCAN COUNT is not a hard response limit.

Type filters use SCAN TYPE (Redis 6+). Unsupported servers/ACL restrictions surface
an error; no speculative capability/health probe or silent full scan is added.
JSON keys can be discovered by type, but the existing value inspector still does
not support RedisJSON. Rich collection editing, bulk operations, cluster/Sentinel,
and persisted browser preferences remain separate work.

## Protocol budget

On an established healthy standalone Redis connection, both first and repeated use:

| Operation | Before | After |
| --- | --- | --- |
| Publish a scan page's names, up to 200 keys | SCAN + TYPE/PTTL pipeline + value pipeline: 3 exchanges | SCAN: 1 exchange |
| Enrich up to 200 admitted page keys | Included above | TYPE/PTTL pipeline + value pipeline: 2 exchanges |
| Expand/collapse a discovered namespace | Unavailable | 0 exchanges |
| Apply a type filter | Unavailable | TYPE option in the same SCAN exchange |
| Exact-key inspection | Existing inspector: normally 4 exchanges | Same inspector, bypassing SCAN |

All-preview completion for the same page of up to 200 keys still costs three
exchanges. Names are published first, then the page is enriched in the background
before the next scan batch. This deliberately preserves page-level prefetching:
a visible-only alternative was rejected because scrolling would otherwise add
extra enrichment round trips. For oversized SCAN responses the original budget
of one SCAN plus two exchanges per 200-key chunk remains; only admitted keys are
enriched. We do not claim one exchange for full value previews. Connection setup,
authentication, TLS validation, PING/SELECT at connection creation, inspector
reads, and connection teardown are unchanged. A fresh normal RESP2 connection
adds three Redis protocol exchanges on both revisions: the driver's combined
AUTH/CLIENT SETINFO setup, then Rustable's PING, then SELECT. Thus names are
available after 4 protocol exchanges from a fresh connection instead of 6 for a
page of up to 200 keys; full enrichment remains 6. TCP establishment, optional
TLS negotiation, and legacy authentication fallback have their own unchanged
transport costs and are excluded from the established-connection measurements.
No cancellation worker or statement-cleanup exchange is introduced. Redis-rs 1.8.1's nontransactional
Pipeline::query sends the whole encoded batch through req_packed_commands before
reading replies. PostgreSQL and MySQL paths are untouched.

The protocol test withholds responses until complete metadata/value batches have
arrived, checking initial and repeated use and connection reuse after an error.
The disposable delayed-network test adds 15 ms per direction with overlapping
arrival deadlines, measures names and all-preview completion for the same 40 keys,
and checks the actual returned values. A local debug run measured names p50
42.93 ms / p95 46.42 ms, versus the old all-preview publication point p50 127.51 ms /
p95 138.17 ms (12 samples, including first use; no warmups). This measures the
transport/decoding operation, not input-to-display latency.

## Reproduce

```sh
mise exec -- cargo test --lib redis
bash scripts/test-redis.sh
python3 scripts/ui-performance.py --base ad686bfe8012a566046fcf6aed138920ef109524 --redis --output target/redis-ui-performance
mise exec -- cargo test --release --lib redis_namespace_tree_frame_benchmark -- --ignored --nocapture --test-threads=1
bash scripts/preview-redis-browser.sh
```

`--redis` transplants the exact same Redis grid benchmark source into the base
worktree. It compares 10,000 keys, selected rows and scrolling, plus a 200-field
hash inspector, at 1200×760, 20 warmup / 160 measured frames, five alternating paired
runs. The same inspector width and input values are used on each revision. The candidate's new
namespace-tree workload additionally measures expansion/collapse and virtualization;
there is no pre-feature tree baseline. CPU layout/tessellation is not GPU or
input-to-present performance.

For app-rendered captures, set `RUSTABLE_REDIS_CAPTURE=target/redis-captures` when
launching the preview. Its QA-only sequence expands fixture namespaces, inspects a
hash, captures the tree, applies the hash/prefix filters, and captures the list.
PPM output can be converted losslessly with `sips -s format png`. The preview only
connects to the disposable fixture at 127.0.0.1:16379 and does not load user settings.
Native keyboard/pointer, 60/120 Hz presentation, mixed-DPI, and multi-window checks
require an unlocked desktop; captures alone do not establish those checks.

## Validation record

- Redis unit checks: 20 passed (integration/benchmark tests excluded).
- Disposable Redis 7.4 plain/TLS suite: 8 passed, including filtered discovery,
  delayed transport, protocol batching, ACL/password identity, binary values,
  expiry, optimistic-write conflicts, timeouts, and certificate/hostname rejection.
- Existing PostgreSQL disposable latency suite: all 4 passed (editor, browse,
  catalog and completion); no SQL driver changes were made.
- Strict all-target Clippy with the QA feature passed. WASM library check passed
  with the repository's existing warnings.
- Native app-rendered tree/list captures are retained under
  `target/redis-browser-screenshots/verified/`. QA fixtures were removed afterward.
- Raw Redis integration and PostgreSQL results are in
  `target/redis-integration.log` and `target/redis-postgres-regression.log`.
- The first CPU comparison is retained in `target/redis-ui-performance/`; Redis
  grid p95 was 0.1232 → 0.1213 ms, but four SQL metrics were inconclusive due to
  noise. This preliminary run preceded the final visible-index allocation fix
  and the additional hash-inspector workload; it is not the final-code gate.

### Rendering performance and accepted inspector cost

Measured on Apple M4 Max, macOS 27.2, Rust 1.99.1, release/thin-LTO,
1200×760 logical points at the default egui scale, 20 warmups + 160 samples per
workload/run, five alternating pairs. Raw samples, environment, p99 and maxima are
retained in `target/redis-ui-performance-final/`. Measurements ran serially without
competing builds or benchmarks.

| Workload | Base p50 / p95 / p99 (ms) | Candidate p50 / p95 / p99 (ms) | Largest observed base / candidate stall (ms) |
| --- | --- | --- | --- |
| redis/grid | 0.120625 / 0.127875 / 0.135334 | 0.115375 / 0.124250 / 0.127250 | 0.148542 / 0.225125 |
| redis/hash-inspector | 0.025208 / 0.026083 / 0.027917 | 0.028459 / 0.030416 / 0.033541 | 0.028667 / 0.037458 |

Twenty of 21 comparison metrics passed, including the Redis key list and all
existing SQL UI workloads. The hash inspector is an **accepted regression**, not a
passing gate: median p95 increased 16.6%, from 0.026083 to 0.030416 ms (+0.004333 ms),
repeatable across the paired runs. The report retains its regression/exit-1 status;
thresholds and sample counts were not relaxed.

The concrete benefit is the requested visual polish: distinct, aligned field/value
columns, per-cell value tooltips, readable key metadata and consistent type badges,
replacing an undifferentiated quoted text dump. The old single text galley was
cheaper after its layout was cached. A full egui Grid was considered and replaced
with a virtualized table so offscreen hash fields are not laid out. Custom painting
could reduce widget overhead further, but would require manually reproducing cell
clipping, hover behavior and scrolling; that alternative remains future work.
The bounded layout and improved ability to read structured Redis data justify the
measured extra cost here; fitting a display frame budget alone is not the reason.

The new 10,000-key namespace-tree workload reports p50 0.031041 ms, p95 0.046584 ms,
p99 0.222042 ms and maximum 0.338750 ms; expansion/collapse p95 was 0.338750 ms.
It includes 160 measured frames and eight measured toggles, asserts visible-row
virtualization and has no pre-feature tree baseline. These small tail samples are
informational, not presentation-latency guarantees. Raw output is retained in
`target/redis-tree-performance.log`.

After this rendering comparison, only scan/preview job scheduling changed to
retain 200-key page prefetching, plus its admission-limit regression test. Rendering
methods and benchmark fixtures remained unchanged. Redis unit, strict Clippy and
WASM checks were rerun after that scheduling correction. Native interactive and
mixed-DPI/display-timing checks remain unverified because the desktop was locked.
