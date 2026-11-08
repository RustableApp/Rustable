#!/usr/bin/env bash
# Disposable Redis 8.4 vector-set fixture; no saved profiles or user data.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture="rustable-redis-vectors-$$"
trap 'docker rm -f "$fixture" >/dev/null 2>&1 || true' EXIT
docker run -d --name "$fixture" -p 127.0.0.1:16382:6379 redis:8.4-alpine redis-server --save '' --appendonly no --requirepass rustable-vector-test-only >/dev/null
for attempt in {1..40}; do
 if docker exec -e REDISCLI_AUTH=rustable-vector-test-only "$fixture" redis-cli PING >/dev/null 2>&1; then break; fi
 sleep 0.2
done
RUSTABLE_REDIS_VECTOR_QA=1 mise exec -- cargo test --locked --lib redis_session::tests::live_vector_pages -- --include-ignored --nocapture --test-threads=1
