#!/usr/bin/env bash
# Disposable TLS-only Sentinel/data fixture with independent certificate authorities.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture="rustable-sentinel-tls-$$"
certs="$(mktemp -d)"
cleanup() { docker rm -f "$fixture" >/dev/null 2>&1 || true; rm -rf "$certs"; }
trap cleanup EXIT
for scope in data sentinel; do
 openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=Rustable $scope test CA" -keyout "$certs/$scope-ca.key" -out "$certs/$scope-ca.crt" >/dev/null 2>&1
 openssl req -newkey rsa:2048 -nodes -subj '/CN=127.0.0.1' -keyout "$certs/$scope.key" -out "$certs/$scope.csr" >/dev/null 2>&1
 cat > "$certs/extensions.cnf" <<'CERT'
subjectAltName=IP:127.0.0.1
basicConstraints=CA:FALSE
keyUsage=digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
CERT
 openssl x509 -req -days 2 -in "$certs/$scope.csr" -CA "$certs/$scope-ca.crt" -CAkey "$certs/$scope-ca.key" -CAcreateserial -extfile "$certs/extensions.cnf" -out "$certs/$scope.crt" >/dev/null 2>&1
done
docker run -d --name "$fixture" -p 127.0.0.1:16420:16420 -p 127.0.0.1:26420:26420 -v "$certs:/certs:ro" redis:7.4-alpine sh -c '
set -eu
redis-server --port 0 --tls-port 16420 --tls-auth-clients no --tls-cert-file /certs/data.crt --tls-key-file /certs/data.key --tls-ca-cert-file /certs/data-ca.crt --bind 0.0.0.0 --requirepass rustable-data-test-only --save "" --appendonly no --maxmemory 64mb --daemonize yes
cat > /tmp/sentinel.conf <<CONFIG
port 0
tls-port 26420
tls-auth-clients no
tls-cert-file /certs/sentinel.crt
tls-key-file /certs/sentinel.key
tls-ca-cert-file /certs/data-ca.crt
tls-replication yes
bind 0.0.0.0
requirepass rustable-sentinel-test-only
sentinel monitor rustable-tls-qa 127.0.0.1 16420 1
sentinel auth-pass rustable-tls-qa rustable-data-test-only
daemonize yes
logfile /tmp/sentinel.log
CONFIG
redis-server /tmp/sentinel.conf --sentinel
exec sleep 3600' >/dev/null
for attempt in {1..50}; do
 if docker exec -e REDISCLI_AUTH=rustable-sentinel-test-only "$fixture" redis-cli --tls --cacert /certs/sentinel-ca.crt -h 127.0.0.1 -p 26420 PING 2>/dev/null | rg -q '^PONG'; then break; fi
 sleep 0.2
done
RUSTABLE_REDIS_SENTINEL_TLS_QA=1 RUSTABLE_SENTINEL_TLS_CA="$certs/sentinel-ca.crt" RUSTABLE_SENTINEL_DATA_TLS_CA="$certs/data-ca.crt" mise exec -- cargo test --locked --lib redis_sentinel::tests::tls_independent -- --include-ignored --nocapture --test-threads=1
