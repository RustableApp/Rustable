#!/usr/bin/env bash
# Render real vector search controls against a disposable ephemeral fixture.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture="rustable-vector-preview-$$"
trap 'docker rm -f "$fixture" >/dev/null 2>&1 || true' EXIT
docker run -d --name "$fixture" -p 127.0.0.1::6379 "${RUSTABLE_REDIS_PREVIEW_IMAGE:-redis:8.4-alpine}" redis-server --save '' --appendonly no --maxmemory 64mb --requirepass rustable-redis-test-only >/dev/null
port="$(docker port "$fixture" 6379/tcp | sed 's/.*://')"
ready=false
for attempt in {1..40}; do
 if docker exec -e REDISCLI_AUTH=rustable-redis-test-only "$fixture" redis-cli PING 2>/dev/null | rg -q '^PONG'; then ready=true; break; fi
 sleep 0.2
done
if [ "$ready" != true ]; then echo 'Search preview fixture failed to start' >&2; exit 1; fi
docker exec -i -e REDISCLI_AUTH=rustable-redis-test-only "$fixture" redis-cli >/dev/null <<'SEED'
VADD search:product:embeddings VALUES 3 1 0 0 product:00000:description
VADD search:product:embeddings VALUES 3 1 0.3 0.1 product:00001:description
VSETATTR search:product:embeddings product:00000:description '{"category":"Electronics"}'
VSETATTR search:product:embeddings product:00001:description '{"category":"Electronics"}' 
SEED
RUSTABLE_REDIS_PREVIEW_PORT="$port" python3 - <<'PY'
import os,subprocess,time
from pathlib import Path
out=Path(os.environ.get('RUSTABLE_VECTOR_CAPTURE_DIR','target/redis-vector-controls-captures')).resolve()
out.mkdir(parents=True,exist_ok=True)
expected=['vector-elements','vector-detail','vector-similar']
for name in expected: (out/(name+'.ppm')).unlink(missing_ok=True)
env=dict(os.environ,RUSTABLE_REDIS_PREVIEW='1',RUSTABLE_REDIS_VECTOR_CAPTURE='1',RUSTABLE_REDIS_CAPTURE=str(out))
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
