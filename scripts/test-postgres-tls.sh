#!/usr/bin/env bash
# Disposable loopback fixture reproducing issue #8; never uses saved connections.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture="rustable-postgres-tls-$$"
certs="$(mktemp -d)"
trap 'docker rm -f "$fixture" >/dev/null 2>&1 || true; rm -rf "$certs"' EXIT
openssl req -x509 -newkey rsa:2048 -nodes -keyout "$certs/server.key" -out "$certs/server.crt" -days 3650 -subj /CN=wrong-host.example.test -addext subjectAltName=DNS:wrong-host.example.test
docker run -d --name "$fixture" --tmpfs /var/lib/postgresql/data -p 127.0.0.1:54329:5432 -e POSTGRES_PASSWORD=rustable_qa_only -e POSTGRES_DB=rustable_qa -v "$certs:/fixture:ro" postgres:17 sh -c 'cp /fixture/server.* /tmp/; chown postgres:postgres /tmp/server.*; chmod 600 /tmp/server.key; exec docker-entrypoint.sh postgres -c ssl=on -c ssl_cert_file=/tmp/server.crt -c ssl_key_file=/tmp/server.key'
ready=false
for attempt in {1..60}; do
 if docker exec "$fixture" pg_isready -U postgres >/dev/null 2>&1; then ready=true; break; fi
 sleep 0.5
done
if [ "$ready" != true ]; then echo 'PostgreSQL TLS fixture did not start' >&2; exit 1; fi
RUSTABLE_POSTGRES_TLS_QA=1 mise exec -- cargo test --lib explicit_tls_modes_accept_encrypt_only -- --ignored --nocapture --test-threads=1
RUSTABLE_POSTGRES_TLS_QA=1 mise exec -- cargo test --lib honors_encrypt_only_tls -- --ignored --nocapture --test-threads=1
