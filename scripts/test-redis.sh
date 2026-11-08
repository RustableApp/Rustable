#!/usr/bin/env bash
# Disposable standalone Redis 7.4 integration fixtures. Requires Docker + OpenSSL.
set -euo pipefail
cd "$(dirname "$0")/.."
redis_test_dir="$(mktemp -d)"
redis_plain_name="rustable-redis-test-$$"
redis_tls_name="rustable-redis-tls-test-$$"
redis_json_name="rustable-redis-json-test-$$"
cleanup() {
    docker rm -f "$redis_plain_name" "$redis_tls_name" "$redis_json_name" >/dev/null 2>&1 || true
    rm -rf "$redis_test_dir"
}
trap cleanup EXIT
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj '/CN=Rustable test CA'     -keyout "$redis_test_dir/ca.key" -out "$redis_test_dir/ca.crt" >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes -subj '/CN=127.0.0.1'     -keyout "$redis_test_dir/server.key" -out "$redis_test_dir/server.csr" >/dev/null 2>&1
cat > "$redis_test_dir/extensions.cnf" <<'CERT'
subjectAltName=IP:127.0.0.1
basicConstraints=CA:FALSE
keyUsage=digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
CERT
openssl x509 -req -days 2 -in "$redis_test_dir/server.csr"     -CA "$redis_test_dir/ca.crt" -CAkey "$redis_test_dir/ca.key" -CAcreateserial     -extfile "$redis_test_dir/extensions.cnf" -out "$redis_test_dir/server.crt" >/dev/null 2>&1
docker run -d --name "$redis_plain_name" -p 127.0.0.1:16379:6379 redis:7.4-alpine     redis-server --save '' --appendonly no --requirepass rustable-redis-test-only >/dev/null
docker run -d --name "$redis_tls_name" -p 127.0.0.1:16380:6379     -v "$redis_test_dir:/certs:ro" redis:7.4-alpine redis-server     --save '' --appendonly no --port 0 --tls-port 6379 --tls-auth-clients no     --tls-cert-file /certs/server.crt --tls-key-file /certs/server.key     --tls-ca-cert-file /certs/ca.crt --requirepass rustable-redis-test-only >/dev/null
docker run -d --name "$redis_json_name" -p 127.0.0.1:16381:6379 redis:8.2-alpine redis-server --save '' --appendonly no --requirepass rustable-redis-test-only >/dev/null
for redis_test_attempt in {1..30}; do
    if docker exec -e REDISCLI_AUTH=rustable-redis-test-only "$redis_json_name" redis-cli PING >/dev/null 2>&1 && docker exec -e REDISCLI_AUTH=rustable-redis-test-only "$redis_plain_name" redis-cli PING >/dev/null 2>&1         && docker exec -e REDISCLI_AUTH=rustable-redis-test-only "$redis_tls_name"             redis-cli --tls --cacert /certs/ca.crt -h 127.0.0.1 PING >/dev/null 2>&1; then
        break
    fi
    sleep 0.2
done
RUSTABLE_REDIS_JSON_PORT=16381 RUSTABLE_REDIS_TEST_CA="$redis_test_dir/ca.crt" mise exec -- cargo test --locked --lib redis_session:: -- --skip live_vector_pages --skip live_cluster_replica_promotion --skip live_cluster_reconnect --skip live_cluster_timeseries --include-ignored --nocapture --test-threads=1

RUSTABLE_REDIS_TEST_CA="$redis_test_dir/ca.crt" mise exec -- cargo test --locked --lib redis_monitor::transport_tests -- --include-ignored --nocapture --test-threads=1
RUSTABLE_REDIS_TEST_CA="$redis_test_dir/ca.crt" mise exec -- cargo test --locked --lib redis_pubsub::transport_tests -- --include-ignored --nocapture --test-threads=1
mise exec -- cargo test --locked --lib redis_archive::transport_tests -- --include-ignored --nocapture --test-threads=1

RUSTABLE_REDIS_TEST_CA="$redis_test_dir/ca.crt" mise exec -- cargo test --locked --lib redis_cluster::tests::discovered_nodes_preserve_tls_identity_and_never_fall_back_to_plaintext -- --include-ignored --nocapture --test-threads=1
