#!/usr/bin/env bash
# Disposable six-node Cluster; promotes replicas and stops original primaries.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture="rustable-cluster-failover-$$"
cleanup() {
 local result=$?
 if [[ "$result" != 0 ]]; then
  docker exec "$fixture" sh -c 'for log in /tmp/redis-*/redis.log; do tail -60 "$log"; done' || true
 fi
 docker rm -f "$fixture" >/dev/null 2>&1 || true
 return "$result"
}
trap cleanup EXIT
docker run -d --name "$fixture" -p 127.0.0.1:16600-16605:16600-16605 redis:7.4-alpine sh -c '
set -eu
for port in 16600 16601 16602 16603 16604 16605; do
 mkdir -p /tmp/redis-$port
 redis-server --logfile /tmp/redis-$port/redis.log --dir /tmp/redis-$port --port "$port" --bind 0.0.0.0 --requirepass rustable-cluster-test-only --masterauth rustable-cluster-test-only --cluster-enabled yes --cluster-config-file /tmp/redis-$port/nodes.conf --cluster-announce-ip 127.0.0.1 --cluster-announce-port "$port" --save "" --appendonly no --daemonize yes
done
exec sleep 3600' >/dev/null
for attempt in {1..40}; do
 if docker exec -e REDISCLI_AUTH=rustable-cluster-test-only "$fixture" redis-cli -p 16605 PING >/dev/null 2>&1; then break; fi
 sleep 0.2
done
docker exec -e REDISCLI_AUTH=rustable-cluster-test-only "$fixture" redis-cli --cluster create 127.0.0.1:16600 127.0.0.1:16601 127.0.0.1:16602 127.0.0.1:16603 127.0.0.1:16604 127.0.0.1:16605 --cluster-replicas 1 --cluster-yes
for attempt in {1..40}; do
 if docker exec -e REDISCLI_AUTH=rustable-cluster-test-only "$fixture" redis-cli -p 16600 CLUSTER INFO | tr -d '\r' | rg -q '^cluster_state:ok$'; then break; fi
 sleep 0.2
done
RUSTABLE_REDIS_CLUSTER_FAILOVER_QA=1 mise exec -- cargo test --locked --lib redis_session::tests::live_cluster_replica_promotion -- --include-ignored --nocapture --test-threads=1

if [[ -n "${RUSTABLE_REDIS_TOPOLOGY_CAPTURE:-}" ]]; then
 for i in {0..29}; do
  docker exec -e REDISCLI_AUTH=rustable-cluster-test-only "$fixture" redis-cli -c -p 16603 SET "routing:$i" value >/dev/null
 done
 RUSTABLE_REDIS_FAILOVER_CAPTURE=1 RUSTABLE_REDIS_PREVIEW=1 mise exec -- cargo run --features qa --bin rustable
fi
