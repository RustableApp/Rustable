#!/bin/sh
set -eu
mkdir -p /run/fixture
cp /keys/host /run/fixture/host
cp /keys/client.pub /run/fixture/client.pub
chmod 600 /run/fixture/*
socat TCP-LISTEN:43318,bind=127.0.0.1,reuseaddr,fork TCP:host.docker.internal:43318 &
socat TCP-LISTEN:43319,bind=127.0.0.1,reuseaddr,fork TCP:host.docker.internal:43319 &
exec /usr/sbin/sshd -D -e
