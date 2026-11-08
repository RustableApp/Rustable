#!/usr/bin/env bash
# Disposable Search fixture using an ephemeral loopback port.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture="rustable-search-catalog-$$"
trap 'docker rm -f "$fixture" >/dev/null 2>&1 || true' EXIT
docker run -d --name "$fixture" -p 127.0.0.1::6379 redis:8.4-alpine redis-server --save '' --appendonly no --maxmemory 64mb --requirepass rustable-search-test-only >/dev/null
port="$(docker port "$fixture" 6379/tcp | sed 's/.*://')"
ready=false
for attempt in {1..40}; do
 if docker exec -e REDISCLI_AUTH=rustable-search-test-only "$fixture" redis-cli PING 2>/dev/null | rg -q '^PONG'; then ready=true; break; fi
 sleep 0.2
done
if [ "$ready" != true ]; then echo 'Search fixture failed to start' >&2; exit 1; fi
RUSTABLE_REDIS_SEARCH_CATALOG_QA=1 RUSTABLE_REDIS_SEARCH_CATALOG_PORT="$port" mise exec -- cargo test --locked --lib redis_search_catalog::tests::live_catalog_metadata -- --ignored --nocapture
