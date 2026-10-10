#!/bin/sh
# Build the wasm player into web/pkg and run the site's dev server, which
# serves the bundles from fixtures/bundles/ with the HTTP range requests the
# player streams with:
#   http://localhost:8000/                                     the archive
#   http://localhost:8000/play/<slug>/                         one disc
#   http://localhost:8000/play/local/?bundle=/bundles/<slug>/  any bundle, for
#     testing; any player page also takes ?seed=<n> and ?compat=1
# Bundles are extracted from their committed archives first (see
# scripts/bundles.py). To add a disc, list it in fixtures/discs.json, make its
# bundle with `menuseum bundle <disc> --out fixtures/bundles/<slug>` and
# `scripts/bundles.py pack <slug>`, and sample its look with scripts/looks.py.
# To try the site as it's deployed: `npm run build && npm run preview` in web/.
set -eu
cd "$(dirname "$0")/.."
python3 scripts/bundles.py
cargo build --release -p menuseum-wasm --target wasm32-unknown-unknown
mkdir -p web/pkg
cp target/wasm32-unknown-unknown/release/menuseum_wasm.wasm web/pkg/
[ "${1:-}" = "--build-only" ] && exit 0
[ -d web/node_modules ] || (cd web && npm ci)
cd web
exec npx astro dev --port "${PORT:-8000}"
