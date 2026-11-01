#!/bin/sh
# Disposable loopback fixture credentials only. Never use these for deployment.
set -eu
cert_dir=${1:?Pass a new certificate directory}
if [ -e "$cert_dir" ]; then
    echo 'Certificate directory must not exist' >&2
    exit 1
fi
mkdir -p "$cert_dir"
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -sha256 \
    -subj '/CN=Rustable fixture CA' -keyout "$cert_dir/ca-key.pem" -out "$cert_dir/ca.pem"
openssl req -newkey rsa:2048 -nodes -sha256 -subj '/CN=localhost' \
    -keyout "$cert_dir/server-key.pem" -out "$cert_dir/server.csr"
cat > "$cert_dir/server.ext" <<'EXT'
basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
subjectAltName=DNS:localhost
EXT
openssl x509 -req -days 2 -sha256 -in "$cert_dir/server.csr" \
    -CA "$cert_dir/ca.pem" -CAkey "$cert_dir/ca-key.pem" -CAcreateserial \
    -extfile "$cert_dir/server.ext" -out "$cert_dir/server.pem"
# Database containers run under distinct UIDs; this key is a disposable fixture.
chmod 644 "$cert_dir/server-key.pem"
chmod 600 "$cert_dir/ca-key.pem"
