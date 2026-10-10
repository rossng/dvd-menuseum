#!/bin/sh
# Build the wasm player into web/pkg and the CLI that renders disc labels,
# write the catalog (scripts/site.py --catalog-only) and run the site's dev
# server, which serves the bundles from fixtures/bundles/ with the HTTP range
# requests the player streams with:
#   http://localhost:8000/                                  the archive
#   http://localhost:8000/play/<slug>/                      one disc
#   http://localhost:8000/play/local/?bundle=/bundles/<slug>/  any bundle, for
#     testing; any player page also takes ?seed=<n> and ?compat=1
# Bundles are extracted from their committed archives first (see
# scripts/bundles.py; make new ones with `menuseum bundle <disc> --out
# fixtures/bundles/<slug>` then `scripts/bundles.py pack <slug>`, and list
# them in web/discs.json). To try the site as it's deployed, build it with
# scripts/site.py and serve target/site with scripts/serve.py.
set -eu
cd "$(dirname "$0")/.."
python3 scripts/bundles.py
cargo build --release -p menuseum-wasm --target wasm32-unknown-unknown
mkdir -p web/pkg
cp target/wasm32-unknown-unknown/release/menuseum_wasm.wasm web/pkg/
[ "${1:-}" = "--build-only" ] && exit 0
cargo build --release -p menuseum-cli
[ -d web/node_modules ] || (cd web && npm ci)
python3 scripts/site.py --catalog-only
cd web
exec npx astro dev --port "${PORT:-8000}"
