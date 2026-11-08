#!/usr/bin/env bash
# Disposable, isolated three-primary Redis cluster; never uses saved connections.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture="rustable-cluster-test-$$"
trap 'docker rm -f "$fixture" >/dev/null 2>&1 || true' EXIT
image=redis:7.4-alpine
if [[ "${RUSTABLE_REDIS_CLUSTER_MODULE_QA:-0}" == 1 ]]; then image=redis:8.2-alpine; fi
if [[ "${RUSTABLE_REDIS_CLUSTER_VECTOR_QA:-0}" == 1 ]]; then image=redis:8.4-alpine; fi
docker run -d --name "$fixture" -e RUSTABLE_REDIS_CLUSTER_MODULE_QA="${RUSTABLE_REDIS_CLUSTER_MODULE_QA:-0}" -e RUSTABLE_REDIS_CLUSTER_VECTOR_QA="${RUSTABLE_REDIS_CLUSTER_VECTOR_QA:-0}" -p 127.0.0.1:16400:16400 -p 127.0.0.1:16401:16401 -p 127.0.0.1:16402:16402 "$image" sh -c '
set -eu
module_args=""
if [ "$RUSTABLE_REDIS_CLUSTER_MODULE_QA" = 1 ]; then module_args="--loadmodule /usr/local/lib/redis/modules/redistimeseries.so"; fi
for port in 16400 16401 16402; do
 mkdir -p /tmp/redis-$port
 redis-server --dir /tmp/redis-$port --port "$port" --bind 0.0.0.0 --requirepass rustable-cluster-test-only --masterauth rustable-cluster-test-only --cluster-enabled yes --cluster-config-file /tmp/redis-$port/nodes.conf --cluster-announce-ip 127.0.0.1 --cluster-announce-port "$port" --save "" --appendonly no --daemonize yes $module_args
done
exec sleep 3600' >/dev/null
for attempt in {1..40}; do
 if docker exec -e REDISCLI_AUTH=rustable-cluster-test-only "$fixture" redis-cli -p 16402 PING >/dev/null 2>&1; then break; fi
 sleep 0.2
done
docker exec -e REDISCLI_AUTH=rustable-cluster-test-only "$fixture" redis-cli --cluster create 127.0.0.1:16400 127.0.0.1:16401 127.0.0.1:16402 --cluster-replicas 0 --cluster-yes
for attempt in {1..40}; do
 if docker exec -e REDISCLI_AUTH=rustable-cluster-test-only "$fixture" redis-cli -p 16400 CLUSTER INFO | tr -d '\r' | rg -q '^cluster_state:ok$'; then break; fi
 sleep 0.2
done
RUSTABLE_REDIS_CLUSTER_QA=1 mise exec -- cargo test --locked --lib redis_topology::tests -- --include-ignored --nocapture --test-threads=1 --skip redis_topology_frame_benchmark

RUSTABLE_REDIS_CLUSTER_QA=1 mise exec -- cargo test --locked --lib redis_cluster::tests -- --include-ignored --nocapture --test-threads=1 --skip discovered_nodes_preserve_tls_identity_and_never_fall_back_to_plaintext

if [[ "${RUSTABLE_REDIS_CLUSTER_MODULE_QA:-0}" == 1 ]]; then
 RUSTABLE_REDIS_CLUSTER_MODULE_QA=1 mise exec -- cargo test --locked --lib redis_session::tests::live_cluster_timeseries -- --include-ignored --nocapture --test-threads=1
fi

if [[ "${RUSTABLE_REDIS_CLUSTER_VECTOR_QA:-0}" == 1 ]]; then
 RUSTABLE_REDIS_CLUSTER_VECTOR_QA=1 mise exec -- cargo test --locked --lib redis_session::tests::live_cluster_vectors -- --include-ignored --nocapture --test-threads=1
fi

if [[ -n "${RUSTABLE_REDIS_TOPOLOGY_CAPTURE:-}" ]]; then
 # Integration cases intentionally delete routing keys. Restore deterministic QA data.
 for i in {0..29}; do
  docker exec -e REDISCLI_AUTH=rustable-cluster-test-only "$fixture" redis-cli -c -p 16400 SET "routing:$i" value >/dev/null
  printf -v key 'bulk:%02d' "$i"
  docker exec -e REDISCLI_AUTH=rustable-cluster-test-only "$fixture" redis-cli -c -p 16400 SET "$key" sample PX 600000 >/dev/null
 done
 RUSTABLE_REDIS_PREVIEW=1 mise exec -- cargo run --features qa --bin rustable
fi

# Last: deliberately stop the original seed; all preceding cases need all primaries.
RUSTABLE_REDIS_CLUSTER_RECOVERY_QA=1 mise exec -- cargo test --locked --lib redis_session::tests::live_cluster_reconnect -- --include-ignored --nocapture --test-threads=1
