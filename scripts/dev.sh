#!/bin/sh
# Build the wasm player into web/pkg and the CLI that renders disc labels,
# assemble the site in target/site (scripts/site.py, symlinked to web/ so
# edits are live) and serve it:
#   http://localhost:8000/                            the archive
#   http://localhost:8000/play.html?disc=<slug>       one disc
#   http://localhost:8000/harness.html?bundle=bundles/<slug>/   test harness
# Bundles are extracted from their committed archives first (see
# scripts/bundles.py; make new ones with `menuseum bundle <disc> --out
# fixtures/bundles/<slug>` then `scripts/bundles.py pack <slug>`, and list
# them in web/discs.json).
# scripts/serve.py answers the HTTP range requests the player streams with.
set -eu
cd "$(dirname "$0")/.."
python3 scripts/bundles.py
cargo build --release -p menuseum-wasm --target wasm32-unknown-unknown
mkdir -p web/pkg
cp target/wasm32-unknown-unknown/release/menuseum_wasm.wasm web/pkg/
[ "${1:-}" = "--build-only" ] && exit 0
cargo build --release -p menuseum-cli
python3 scripts/site.py
cd target/site
exec python3 ../../scripts/serve.py "${PORT:-8000}"
