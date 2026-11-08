#!/usr/bin/env bash
# Disposable native Redis preview; never reads saved connections or credentials.
set -euo pipefail
cd "$(dirname "$0")/.."
redis_preview_name="rustable-redis-preview-$$"
trap 'docker rm -f "$redis_preview_name" >/dev/null 2>&1 || true' EXIT
docker run -d --name "$redis_preview_name" -p 127.0.0.1:16379:6379 "${RUSTABLE_REDIS_PREVIEW_IMAGE:-redis:7.4-alpine}" \
    redis-server --save '' --appendonly no --requirepass rustable-redis-test-only >/dev/null
for attempt in {1..30}; do
    if docker exec -e REDISCLI_AUTH=rustable-redis-test-only "$redis_preview_name" redis-cli PING >/dev/null 2>&1; then break; fi
    sleep 0.2
done
docker exec -i -e REDISCLI_AUTH=rustable-redis-test-only "$redis_preview_name" redis-cli >/dev/null <<'DATA'
SET app "Rustable demo"
SET app:config:theme dark
SET app:config:locale en-US
HSET app:users:1001 name "Avery Chen" plan Pro country Canada
HSET app:users:1002 name "Sam Rivera" plan Team country Spain
HSET app:users:1003 name "Morgan Lee" plan Free country UK
SADD app:roles admin editor viewer
SET cache:homepage "Homepage HTML" EX 3600
SET cache:products:featured "[101,204,309]" EX 1800
SET cache:products:popular "[204,309]" EX 900
LPUSH queue:emails welcome receipt reminder
LPUSH queue:exports daily-report weekly-summary
ZADD leaderboard:weekly 4200 avery 3100 sam 2800 morgan
SET session:web:abc123 user-1001 EX 7200
SET session:web:def456 user-1002 EX 3600
SET session:mobile:ghi789 user-1003 EX 5400
XADD events:orders * order_id 901 status paid
DATA
python3 - "$redis_preview_name" <<'PYFIXTURE'
import json, subprocess, sys, os, math
commands = []
for i in range(1, 261): commands.append(f'RPUSH queue:emails "Welcome email #{i:03} — customer {1000+i}"')
for i in range(1, 301): commands.append(f'HSET app:settings:features feature:{i:03} "enabled"')
for i in range(1, 241): commands.append(f'ZADD leaderboard:weekly {2500+i*10} player:{i:03}')
for i in range(1, 8): commands.append(f'XADD events:orders * order_id {901+i} status shipped customer_id {1000+i}')
profile = json.dumps({"user": {"id": 1001, "name": "Avery Chen", "plan": "Pro"}, "roles": ["admin", "editor"], "settings": {"notifications": True, "locale": "en-CA"}})
commands.append('SET cache:profile ' + json.dumps(profile))
if os.environ.get('RUSTABLE_REDIS_VECTOR_CAPTURE'):
    for i in range(438): commands.append(f'VADD search:product:embeddings VALUES 3 1 {i/30} {i/100} product:{i:05}:description')
    commands.append('VSETATTR search:product:embeddings product:00000:description ' + json.dumps(json.dumps({"product_id": 9007199254740993, "name": "Travel camera", "category": "Electronics", "in_stock": True, "tags": ["travel", "photography"]})))
if os.environ.get('RUSTABLE_REDIS_TIMESERIES_CAPTURE'):
    commands.extend(['TS.CREATE telemetry:temperature RETENTION 86400000 LABELS site Chicago sensor office unit celsius', 'TS.CREATE telemetry:temperature:minute', 'TS.CREATERULE telemetry:temperature telemetry:temperature:minute AGGREGATION avg 60000'])
    for i in range(600): commands.append(f'TS.ADD telemetry:temperature {1700000000000+i*1000} {21+2*math.sin(i/30):.3f}')
subprocess.run(['docker','exec','-i','-e','REDISCLI_AUTH=rustable-redis-test-only',sys.argv[1],'redis-cli'], input='\n'.join(commands)+'\n', text=True, check=True, stdout=subprocess.DEVNULL)
PYFIXTURE
if [[ -n "${RUSTABLE_REDIS_SEARCH_CAPTURE:-}" ]]; then
    docker exec -e REDISCLI_AUTH=rustable-redis-test-only "$redis_preview_name" redis-cli FT.CREATE idx:users ON HASH PREFIX 1 app:users: SCHEMA name TEXT SORTABLE plan TAG country TAG monthly_spend NUMERIC SORTABLE >/dev/null
fi
if [[ -n "${RUSTABLE_REDIS_ANALYTICS_CAPTURE:-}" ]]; then
    docker exec -i -e REDISCLI_AUTH=rustable-redis-test-only "$redis_preview_name" redis-cli >/dev/null <<'ANALYTICS'
HSET app:users:1001 monthly_spend 120
HSET app:users:1002 monthly_spend 60
HSET app:users:1003 monthly_spend 0
HSET app:users:1004 name "Taylor Kim" plan Pro country Canada monthly_spend 90
HSET app:users:1005 name "Alex Jones" plan Pro country UK monthly_spend 150
HSET app:users:1006 name "Jules Martin" plan Team country France monthly_spend 40
ANALYTICS
fi
RUSTABLE_REDIS_PREVIEW=1 mise exec -- cargo run --features qa --bin rustable
