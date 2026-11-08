#!/usr/bin/env bash
# Disposable Sentinel failover smoke fixture. Never reads saved connections.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture="rustable-sentinel-test-$$"
trap 'docker rm -f "$fixture" >/dev/null 2>&1 || true' EXIT
docker run -d --name "$fixture" \
 -p 127.0.0.1:16410:16410 -p 127.0.0.1:16411:16411 -p 127.0.0.1:16412:16412 \
 -p 127.0.0.1:26410:26410 -p 127.0.0.1:26411:26411 -p 127.0.0.1:26412:26412 \
 redis:7.4-alpine sh -c '
set -eu
for port in 16410 16411 16412; do
 mkdir -p /tmp/node-$port
 extra=""; if [ "$port" != 16410 ]; then extra="--replicaof 127.0.0.1 16410"; fi
 redis-server --port "$port" --bind 0.0.0.0 --requirepass rustable-data-test-only \
  --masterauth rustable-data-test-only --replica-announce-ip 127.0.0.1 --replica-announce-port "$port" \
  --dir /tmp/node-$port --save "" --appendonly no --maxmemory 64mb --daemonize yes $extra
done
for port in 26410 26411 26412; do
 cat > /tmp/sentinel-$port.conf <<CONFIG
port $port
bind 0.0.0.0
protected-mode yes
requirepass rustable-sentinel-test-only
sentinel monitor rustable-qa 127.0.0.1 16410 2
sentinel auth-pass rustable-qa rustable-data-test-only
sentinel down-after-milliseconds rustable-qa 1000
sentinel failover-timeout rustable-qa 10000
sentinel parallel-syncs rustable-qa 1
sentinel announce-ip 127.0.0.1
sentinel announce-port $port
daemonize yes
logfile /tmp/sentinel-$port.log
CONFIG
 redis-server /tmp/sentinel-$port.conf --sentinel
done
exec sleep 3600' >/dev/null
sentinel() { docker exec -e REDISCLI_AUTH=rustable-sentinel-test-only "$fixture" redis-cli --raw -p "$1" "${@:2}"; }
data() { docker exec -e REDISCLI_AUTH=rustable-data-test-only "$fixture" redis-cli --raw -p "$1" "${@:2}"; }
for attempt in {1..60}; do
 if sentinel 26410 SENTINEL CKQUORUM rustable-qa 2>/dev/null | rg -q '^OK'; then break; fi
 sleep 0.2
done
sentinel 26410 SENTINEL CKQUORUM rustable-qa | rg '^OK'
[[ "$(sentinel 26410 SENTINEL get-master-addr-by-name rustable-qa | tail -n 1)" == 16410 ]]
[[ "$(data 16410 ROLE | head -n 1)" == master ]]
[[ -z "$(sentinel 26410 SENTINEL get-master-addr-by-name unknown-service)" ]]
data 16410 SET sentinel:proof before-failover | rg '^OK$'
# WAIT uses the writer connection, so use a piped transaction-free CLI session.
ack="$(printf 'SET sentinel:proof before-failover\nWAIT 2 10000\n' |
 docker exec -i -e REDISCLI_AUTH=rustable-data-test-only "$fixture" redis-cli --raw -p 16410)"
[[ "$(printf '%s\n' "$ack" | tail -n 1)" == 2 ]]
[[ "$(data 16411 GET sentinel:proof)" == before-failover ]]
[[ "$(data 16412 GET sentinel:proof)" == before-failover ]]
RUSTABLE_REDIS_SENTINEL_QA=1 RUSTABLE_SENTINEL_EXPECTED_PRIMARY=16410 mise exec -- cargo test --locked --lib redis_sentinel::tests -- --skip tls_independent --include-ignored --nocapture --test-threads=1
sentinel 26410 SENTINEL FAILOVER rustable-qa | rg '^OK$'
promoted=""
for attempt in {1..100}; do
 promoted="$(sentinel 26410 SENTINEL get-master-addr-by-name rustable-qa | tail -n 1)"
 if [[ "$promoted" != 16410 && ( "$promoted" == 16411 || "$promoted" == 16412 ) ]]; then break; fi
 sleep 0.2
done
[[ "$promoted" == 16411 || "$promoted" == 16412 ]]
[[ "$(data "$promoted" ROLE | head -n 1)" == master ]]
[[ "$(data "$promoted" GET sentinel:proof)" == before-failover ]]
for attempt in {1..100}; do
 if [[ "$(data 16410 ROLE | head -n 1)" == slave ]]; then break; fi
 sleep 0.2
done
[[ "$(data 16410 ROLE | head -n 1)" == slave ]]
printf 'Sentinel fixture passed: primary 16410 -> %s; data retained; old primary demoted.\n' "$promoted"

RUSTABLE_REDIS_SENTINEL_QA=1 RUSTABLE_SENTINEL_EXPECTED_PRIMARY="$promoted" mise exec -- cargo test --locked --lib redis_sentinel::tests::live_ -- --include-ignored --nocapture --test-threads=1

# Exercise automatic recovery from a lost primary, not only requested failover.
# Confirm replication on the writing connection before shutting down our fixture node.
ack="$(printf 'SET sentinel:proof before-failover\nWAIT 2 10000\n' |
 docker exec -i -e REDISCLI_AUTH=rustable-data-test-only "$fixture" redis-cli --raw -p "$promoted")"
[[ "$(printf '%s\n' "$ack" | tail -n 1)" == 2 ]]
lost_primary="$promoted"
data "$lost_primary" SHUTDOWN NOSAVE
recovered=""
for attempt in {1..150}; do
 recovered="$(sentinel 26410 SENTINEL get-master-addr-by-name rustable-qa | tail -n 1)"
 if [[ "$recovered" != "$lost_primary" && ( "$recovered" == 16410 || "$recovered" == 16411 || "$recovered" == 16412 ) ]]; then break; fi
 sleep 0.2
done
[[ "$recovered" != "$lost_primary" && ( "$recovered" == 16410 || "$recovered" == 16411 || "$recovered" == 16412 ) ]]
[[ "$(data "$recovered" ROLE | head -n 1)" == master ]]
[[ "$(data "$recovered" GET sentinel:proof)" == before-failover ]]
RUSTABLE_REDIS_SENTINEL_QA=1 RUSTABLE_SENTINEL_EXPECTED_PRIMARY="$recovered" mise exec -- cargo test --locked --lib redis_sentinel::tests::live_ -- --include-ignored --nocapture --test-threads=1
printf 'Sentinel automatic recovery passed: stopped primary %s -> %s; data retained.\n' "$lost_primary" "$recovered"

RUSTABLE_REDIS_SENTINEL_QA=1 mise exec -- cargo test --locked --lib redis_workspace::tests::live_sentinel_workspace -- --include-ignored --nocapture --test-threads=1
