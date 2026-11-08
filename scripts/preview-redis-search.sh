#!/usr/bin/env bash
# Render real Search catalog/management UI against a disposable ephemeral fixture.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture="rustable-search-preview-$$"
trap 'docker rm -f "$fixture" >/dev/null 2>&1 || true' EXIT
docker run -d --name "$fixture" -p 127.0.0.1::6379 redis:8.4-alpine redis-server --save '' --appendonly no --maxmemory 64mb --requirepass rustable-redis-test-only >/dev/null
port="$(docker port "$fixture" 6379/tcp | sed 's/.*://')"
ready=false
for attempt in {1..40}; do
 if docker exec -e REDISCLI_AUTH=rustable-redis-test-only "$fixture" redis-cli PING 2>/dev/null | rg -q '^PONG'; then ready=true; break; fi
 sleep 0.2
done
if [ "$ready" != true ]; then echo 'Search preview fixture failed to start' >&2; exit 1; fi
docker exec -i -e REDISCLI_AUTH=rustable-redis-test-only "$fixture" redis-cli >/dev/null <<'DATA'
FT.CREATE idx:users ON HASH PREFIX 1 app:users: SCHEMA name TEXT SORTABLE plan TAG country TAG
HSET app:users:1001 name "Avery Chen" plan Pro country Canada
HSET app:users:1002 name "Sam Rivera" plan Team country Spain
HSET app:users:1003 name "Morgan Lee" plan Free country UK
DATA
RUSTABLE_REDIS_PREVIEW_PORT="$port" python3 - <<'PY'
import os,subprocess,time
from pathlib import Path
out=Path(os.environ.get('RUSTABLE_SEARCH_CAPTURE_DIR','target/redis-search-management-captures')).resolve()
out.mkdir(parents=True,exist_ok=True)
expected=['catalog','metadata','create-index','drop-review','schema','query']
for name in expected: (out/(name+'.ppm')).unlink(missing_ok=True)
env=dict(os.environ,RUSTABLE_REDIS_PREVIEW='1',RUSTABLE_REDIS_SEARCH_CAPTURE='1',RUSTABLE_REDIS_INDEX_MANAGEMENT_CAPTURE='1',RUSTABLE_REDIS_CAPTURE=str(out))
process=subprocess.Popen(['target/debug/rustable'],env=env)
try:
 deadline=time.monotonic()+40
 while not all((out/(name+'.ppm')).exists() for name in expected):
  if process.poll() is not None: raise RuntimeError('Native preview exited before capture')
  if time.monotonic()>deadline: raise TimeoutError('Native screenshot unavailable within 40 seconds')
  time.sleep(.2)
 for name in expected: subprocess.run(['sips','-s','format','png',str(out/(name+'.ppm')),'--out',str(out/(name+'.png'))],check=True,stdout=subprocess.DEVNULL)
 print(out)
finally:
 process.terminate()
 try: process.wait(timeout=5)
 except subprocess.TimeoutExpired: process.kill();process.wait()
PY
