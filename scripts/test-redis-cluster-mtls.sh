#!/usr/bin/env bash
# Disposable three-primary mutual-TLS Cluster, isolated from saved connections.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture="rustable-cluster-mtls-$$"
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
docker run -d --name "$fixture" -p 127.0.0.1:16430:16430 -p 127.0.0.1:16431:16431 -p 127.0.0.1:16432:16432 -v "$certs:/certs:ro" redis:7.4-alpine sh -c '
set -eu
for port in 16430 16431 16432; do
 mkdir -p /tmp/redis-$port
 redis-server --dir /tmp/redis-$port --port 0 --tls-port "$port" --bind 0.0.0.0 --tls-auth-clients yes --tls-cert-file /certs/server.crt --tls-key-file /certs/server.key --tls-client-cert-file /certs/client.crt --tls-client-key-file /certs/client.key --tls-ca-cert-file /certs/ca.crt --tls-cluster yes --tls-replication yes --requirepass rustable-mtls-test-only --masterauth rustable-mtls-test-only --cluster-enabled yes --cluster-config-file /tmp/redis-$port/nodes.conf --cluster-announce-ip 127.0.0.1 --cluster-announce-tls-port "$port" --save "" --appendonly no --maxmemory 64mb --daemonize yes
done
exec sleep 3600' >/dev/null
cli() { docker exec -e REDISCLI_AUTH=rustable-mtls-test-only "$fixture" redis-cli --tls --cacert /certs/ca.crt --cert /certs/client.crt --key /certs/client.key "$@"; }
ready=false
for attempt in {1..50}; do
 if cli -p 16432 PING 2>/dev/null | rg -q '^PONG'; then ready=true; break; fi
 sleep 0.2
done
if [ "$ready" != true ]; then echo 'Cluster TLS fixture did not start' >&2; exit 1; fi
cli --cluster create 127.0.0.1:16430 127.0.0.1:16431 127.0.0.1:16432 --cluster-replicas 0 --cluster-yes
ready=false
for attempt in {1..50}; do
 if cli -p 16430 CLUSTER INFO | tr -d '\r' | rg -q '^cluster_state:ok$'; then ready=true; break; fi
 sleep 0.2
done
if [ "$ready" != true ]; then echo 'Cluster TLS fixture did not converge' >&2; exit 1; fi
RUSTABLE_REDIS_CLUSTER_MTLS_QA=1 RUSTABLE_REDIS_MTLS_CERTS="$certs" mise exec -- cargo test --locked --lib redis_cluster::tests::live_mutual_tls_routes -- --ignored --nocapture --test-threads=1
