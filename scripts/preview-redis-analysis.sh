#!/usr/bin/env bash
# Render real database-wide analysis UI against a disposable ephemeral fixture.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture="rustable-analysis-preview-$$"
trap 'docker rm -f "$fixture" >/dev/null 2>&1 || true' EXIT
docker run -d --name "$fixture" -p 127.0.0.1::6379 redis:8.4-alpine redis-server --save '' --appendonly no --maxmemory 64mb --requirepass rustable-redis-test-only >/dev/null
port="$(docker port "$fixture" 6379/tcp | sed 's/.*://')"
ready=false
for attempt in {1..40}; do
 if docker exec -e REDISCLI_AUTH=rustable-redis-test-only "$fixture" redis-cli PING 2>/dev/null | rg -q '^PONG'; then ready=true; break; fi
 sleep 0.2
done
if [ "$ready" != true ]; then echo 'Search preview fixture failed to start' >&2; exit 1; fi
python3 - <<'SEED' | docker exec -i -e REDISCLI_AUTH=rustable-redis-test-only "$fixture" redis-cli >/dev/null
for i in range(600):
 print(f'SET cache:item:{i} value-{i}' + (f' EX {3600 if i%2 else 86400}' if i%3 else ''))
for i in range(20):
 print(f'HSET users:{i} name User{i} plan Pro')
 print(f'RPUSH queue:{i} first second third')
for i in range(10): print(f'ZADD scores:{i} 100 first 200 second')
SEED
RUSTABLE_REDIS_ANALYSIS_QA=1 RUSTABLE_REDIS_PREVIEW_PORT="$port" mise exec -- cargo test --locked --lib live_database_analysis_and_partial_cancel -- --ignored --nocapture
RUSTABLE_REDIS_PREVIEW_PORT="$port" python3 - <<'PY'
import os,subprocess,time
from pathlib import Path
out=Path(os.environ.get('RUSTABLE_ANALYSIS_CAPTURE_DIR','target/redis-analysis-captures')).resolve()
out.mkdir(parents=True,exist_ok=True)
expected=['analyze']
for name in expected: (out/(name+'.ppm')).unlink(missing_ok=True)
env=dict(os.environ,RUSTABLE_REDIS_PREVIEW='1',RUSTABLE_REDIS_ANALYSIS_CAPTURE='1',RUSTABLE_REDIS_CAPTURE=str(out))
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
