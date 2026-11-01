#!/bin/sh
# Disposable fixture keys only. Known host trust comes from our generated key.
set -eu
key_dir=${1:?Pass a new absolute key directory}
case "$key_dir" in /*) ;; *) echo 'Use an absolute key directory' >&2; exit 1;; esac
if [ -e "$key_dir" ]; then echo 'Key directory must not exist' >&2; exit 1; fi
mkdir -m 700 "$key_dir"
ssh-keygen -q -t ed25519 -N '' -f "$key_dir/client"
ssh-keygen -q -t ed25519 -N '' -f "$key_dir/host"
awk '{print "[127.0.0.1]:43320 " $1 " " $2}' "$key_dir/host.pub" > "$key_dir/known_hosts"
