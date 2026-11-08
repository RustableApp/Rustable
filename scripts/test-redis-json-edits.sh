#!/usr/bin/env bash
# No saved connections; ephemeral disposable Redis with JSON support.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture="rustable-json-edits-$$"
trap 'docker rm -f "$fixture" >/dev/null 2>&1 || true' EXIT
docker run -d --name "$fixture" -p 127.0.0.1::6379 redis:8.4-alpine redis-server --save '' --appendonly no --requirepass rustable-json-test-only >/dev/null
for attempt in {1..40}; do
 if docker exec -e REDISCLI_AUTH=rustable-json-test-only "$fixture" redis-cli PING 2>/dev/null | rg -q '^PONG'; then break; fi
 sleep 0.2
done
port=$(docker port "$fixture" 6379/tcp)
export RUSTABLE_REDIS_JSON_EDIT_PORT=${port##*:}
RUSTABLE_REDIS_JSON_EDIT_QA=1 mise exec -- cargo test --locked --lib live_json_path_edits -- --ignored --nocapture --test-threads=1
