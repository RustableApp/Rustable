#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
if [[ "${1:-}" == "--release" ]]; then
    shift
    mise exec -- cargo build --release
    exec python3 scripts/matrix-demo.py --connections --binary target/release/rustable "$@"
fi
mise exec -- cargo build
exec python3 scripts/matrix-demo.py --connections "$@"
