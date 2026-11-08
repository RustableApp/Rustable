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
docker run -d --name "$redis_plain_name" -p 127.0.0.1::6379 redis:7.4-alpine     redis-server --save '' --appendonly no --requirepass rustable-redis-test-only >/dev/null
docker run -d --name "$redis_tls_name" -p 127.0.0.1::6379     -v "$redis_test_dir:/certs:ro" redis:7.4-alpine redis-server     --save '' --appendonly no --port 0 --tls-port 6379 --tls-auth-clients no     --tls-cert-file /certs/server.crt --tls-key-file /certs/server.key     --tls-ca-cert-file /certs/ca.crt --requirepass rustable-redis-test-only >/dev/null
docker run -d --name "$redis_json_name" -p 127.0.0.1::6379 redis:8.2-alpine redis-server --save '' --appendonly no --requirepass rustable-redis-test-only >/dev/null
for redis_test_attempt in {1..30}; do
    if docker exec -e REDISCLI_AUTH=rustable-redis-test-only "$redis_json_name" redis-cli PING >/dev/null 2>&1 && docker exec -e REDISCLI_AUTH=rustable-redis-test-only "$redis_plain_name" redis-cli PING >/dev/null 2>&1         && docker exec -e REDISCLI_AUTH=rustable-redis-test-only "$redis_tls_name"             redis-cli --tls --cacert /certs/ca.crt -h 127.0.0.1 PING >/dev/null 2>&1; then
        break
    fi
    sleep 0.2
done
plain_endpoint="$(docker port "$redis_plain_name" 6379/tcp)"
tls_endpoint="$(docker port "$redis_tls_name" 6379/tcp)"
json_endpoint="$(docker port "$redis_json_name" 6379/tcp)"
export RUSTABLE_REDIS_TEST_PORT="${plain_endpoint##*:}"
export RUSTABLE_REDIS_TLS_PORT="${tls_endpoint##*:}"
export RUSTABLE_REDIS_JSON_PORT="${json_endpoint##*:}"
export RUSTABLE_REDIS_TEST_CA="$redis_test_dir/ca.crt"
# Select only the live cases owned by these fixtures. Other live cases require
# their own module, topology, certificate or seeded-data harnesses.
for test in \
 live_pagination_binary_empty_large_expiry_and_timeout \
 live_acl_username_and_passwordless_identity \
 live_verified_tls_rejects_untrusted_and_wrong_hostname \
 live_binary_types_expiry_guards_and_conflicts \
 live_filtered_scan_and_delayed_browser_latency \
 live_collection_pages_search_stream_ids_and_binary_strings \
 live_guarded_mutations_bulk_expiry_and_diagnostics \
 live_json_documents_preserve_ttl_and_reject_stale_edits \
 live_mutation_acl_failures_do_not_leave_partial_edits \
 live_search_indexes_schema_paging_and_errors \
 live_aggregations_and_query_profiles \
 live_timeseries_pagination_retains_samples_and_detects_type_changes \
 live_bounded_tls_batch_errors_and_overflow_close_without_replay; do
 mise exec -- cargo test --locked --lib "$test" -- --ignored --nocapture --test-threads=1
done
mise exec -- cargo test --locked --lib redis_monitor::transport_tests -- --include-ignored --nocapture --test-threads=1
mise exec -- cargo test --locked --lib redis_pubsub::transport_tests -- --include-ignored --nocapture --test-threads=1
mise exec -- cargo test --locked --lib redis_archive::transport_tests -- --include-ignored --nocapture --test-threads=1
mise exec -- cargo test --locked --lib redis_cluster::tests::discovered_nodes_preserve_tls_identity_and_never_fall_back_to_plaintext -- --include-ignored --nocapture --test-threads=1
