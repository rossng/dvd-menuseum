#!/bin/sh
# Build the wasm player into web/pkg and serve the repository root, so the
# harness is at http://localhost:8000/web/?bundle=/fixtures/bundles/<slug>/
# Bundles are extracted from their committed archives first (see
# scripts/bundles.py; make new ones with `menuseum bundle <disc> --out
# fixtures/bundles/<slug>` then `scripts/bundles.py pack <slug>`).
# scripts/serve.py answers the HTTP range requests the player streams with.
set -eu
cd "$(dirname "$0")/.."
python3 scripts/bundles.py
cargo build --release -p menuseum-wasm --target wasm32-unknown-unknown
mkdir -p web/pkg
cp target/wasm32-unknown-unknown/release/menuseum_wasm.wasm web/pkg/
[ "${1:-}" = "--build-only" ] && exit 0
exec python3 scripts/serve.py "${PORT:-8000}"
