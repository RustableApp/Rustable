#!/usr/bin/env bash
# Disposable mutual-TLS fixture; binds an automatically assigned loopback port.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture="rustable-mtls-$$"
certs="$(mktemp -d)"
cleanup() { docker rm -f "$fixture" >/dev/null 2>&1 || true; rm -rf "$certs"; }
trap cleanup EXIT
for authority in ca other-ca; do
 openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=Rustable $authority QA" -keyout "$certs/$authority.key" -out "$certs/$authority.crt" >/dev/null 2>&1
done
for identity in server client untrusted; do
 openssl req -newkey rsa:2048 -nodes -subj "/CN=$identity" -keyout "$certs/$identity.key" -out "$certs/$identity.csr" >/dev/null 2>&1
 authority=ca
 if [ "$identity" = untrusted ]; then authority=other-ca; fi
 printf 'basicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\n' > "$certs/extensions.cnf"
 if [ "$identity" = server ]; then
  printf 'subjectAltName=IP:127.0.0.1\nextendedKeyUsage=serverAuth\n' >> "$certs/extensions.cnf"
 else
  printf 'extendedKeyUsage=clientAuth\n' >> "$certs/extensions.cnf"
 fi
 openssl x509 -req -days 2 -in "$certs/$identity.csr" -CA "$certs/$authority.crt" -CAkey "$certs/$authority.key" -CAcreateserial -extfile "$certs/extensions.cnf" -out "$certs/$identity.crt" >/dev/null 2>&1
done
docker run -d --name "$fixture" -p 127.0.0.1::6379 -v "$certs:/certs:ro" redis:7.4-alpine redis-server --save '' --appendonly no --port 0 --tls-port 6379 --tls-auth-clients yes --tls-cert-file /certs/server.crt --tls-key-file /certs/server.key --tls-ca-cert-file /certs/ca.crt --requirepass rustable-mtls-test-only >/dev/null
port="$(docker port "$fixture" 6379/tcp | sed 's/.*://')"
ready=false
for attempt in {1..50}; do
 if docker exec -e REDISCLI_AUTH=rustable-mtls-test-only "$fixture" redis-cli --tls --cacert /certs/ca.crt --cert /certs/client.crt --key /certs/client.key -h 127.0.0.1 PING 2>/dev/null | rg -q '^PONG'; then ready=true; break; fi
 sleep 0.2
done
if [ "$ready" != true ]; then echo 'Mutual TLS fixture did not become ready' >&2; exit 1; fi
RUSTABLE_REDIS_MTLS_QA=1 RUSTABLE_REDIS_MTLS_CERTS="$certs" RUSTABLE_REDIS_MTLS_PORT="$port" mise exec -- cargo test --locked --lib redis_session::tests::live_mutual_tls_client_identity -- --ignored --nocapture --test-threads=1
