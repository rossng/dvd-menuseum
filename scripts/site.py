#!/usr/bin/env python3
"""Build the website: write web/src/catalog.json describing the discs listed
in web/discs.json, including colours sampled from each disc's menu for its
printed label; build the pages from it (astro build); and place the bundles
beside them.

usage: scripts/site.py [--out DIR] [--copy] [--menuseum PATH] [--catalog-only]

  --out DIR        where to build the site (default target/site)
  --copy           copy the bundles rather than symlinking them (for deployment)
  --menuseum PATH  the CLI that renders menus (default target/release/menuseum)
  --catalog-only   only write the catalog (for `npm run dev` in web/)

Expects the bundles extracted (scripts/bundles.py), web/pkg/ built
(scripts/dev.sh --build-only) and web/node_modules installed (npm ci).
Palettes are cached in target/palettes/, keyed on the bundle and render
options, so only changed discs are re-rendered. The base path the site is
served under comes from $BASE_PATH (default /).

web/src/catalog.json           web/discs.json joined with bundle facts, and
                               `source`, the commit the site was built from
                               (the GPL source offer for the player)
DIR/
  index.html, play/<slug>/ ... the pages
  bundles/<slug>/              the menu bundles the player streams
"""

import argparse
import colorsys
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WEB = ROOT / "web"
BUNDLES = ROOT / "fixtures" / "bundles"
PALETTE_CACHE = ROOT / "target" / "palettes"
REPO = "https://github.com/rossng/dvd-menuseum"
CATALOG = WEB / "src" / "catalog.json"


def place(src, dst, copy):
    if copy:
        if src.is_dir():
            shutil.copytree(src, dst, ignore=shutil.ignore_patterns(".*"))
        else:
            shutil.copy2(src, dst)
    else:
        dst.symlink_to(src)


def read_png(path):
    """Pixels of a PNG from `menuseum render`: 8-bit RGB, unfiltered rows."""
    data = path.read_bytes()
    pos, idat = 8, b""
    while pos < len(data):
        n = int.from_bytes(data[pos : pos + 4], "big")
        kind, body = data[pos + 4 : pos + 8], data[pos + 8 : pos + 8 + n]
        if kind == b"IHDR":
            width, height = int.from_bytes(body[:4], "big"), int.from_bytes(body[4:8], "big")
            assert body[8:10] == bytes([8, 2]), "expected 8-bit RGB"
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    raw = zlib.decompress(idat)
    stride = width * 3 + 1
    assert all(raw[y * stride] == 0 for y in range(height)), "expected unfiltered rows"
    return width, height, lambda x, y: raw[y * stride + 1 + x * 3 : y * stride + 4 + x * 3]


def hex_hls(h, l, s):
    return "#" + "".join(f"{round(c * 255):02x}" for c in colorsys.hls_to_rgb(h, l, s))


def palette(png):
    """Three label colours from a menu picture: a deep and a lighter shade of
    its commonest colour, for the print, and its most vivid colour, for
    highlights."""
    width, height, px = read_png(png)
    buckets = {}
    for y in range(0, height, 4):
        for x in range(0, width, 4):
            r, g, b = px(x, y)
            key = (r >> 4, g >> 4, b >> 4)
            n, sr, sg, sb = buckets.get(key, (0, 0, 0, 0))
            buckets[key] = (n + 1, sr + r, sg + g, sb + b)
    top = sorted(buckets.values(), reverse=True)[:32]
    colours = [(n, colorsys.rgb_to_hls(sr / n / 255, sg / n / 255, sb / n / 255)) for n, sr, sg, sb in top]
    _, (h, l, s) = colours[0]
    _, (ah, al, as_) = max(colours, key=lambda c: c[1][2] * min(c[1][1], 1 - c[1][1]) * c[0] ** 0.5)
    if as_ < 0.2:  # a colourless menu: highlight in aqua
        ah, as_ = 0.55, 0.7
    if s < 0.15:  # a black or grey menu: print in a deep shade of its highlight
        h, s = ah, 0.4
    return [
        hex_hls(h, min(max(l, 0.1), 0.26), min(s, 0.7)),
        hex_hls(h, min(max(l, 0.1), 0.26) + 0.14, min(s, 0.7)),
        hex_hls(ah, min(max(al, 0.5), 0.68), max(as_, 0.55)),
    ]


def menu_palette(menuseum, slug, bundle, opts):
    """The palette of the menu a viewer first lands on: the last picture of a
    `menuseum render` run with the disc's scripted inputs."""
    settle = str(opts.get("settle", 12))
    inputs = opts.get("inputs", "")
    stamp = bundle / ".archive"
    key = hashlib.sha256(json.dumps([(bundle / "bundle.json").read_text(), stamp.read_text() if stamp.exists() else "", settle, inputs]).encode()).hexdigest()[:16]
    cached = PALETTE_CACHE / f"{slug}-{key}.json"
    if cached.exists():
        return json.loads(cached.read_text())
    with tempfile.TemporaryDirectory() as tmp:
        cmd = [str(menuseum), "render", str(bundle / "VIDEO_TS"), "--out", tmp, "-i", inputs, "--settle", settle]
        subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL)
        pngs = sorted(Path(tmp).glob("*.png"))
        if not pngs:
            raise SystemExit(f"{slug}: render produced no picture")
        colours = palette(pngs[-1])
    PALETTE_CACHE.mkdir(parents=True, exist_ok=True)
    for old in PALETTE_CACHE.glob(f"{slug}-*.json"):
        old.unlink()
    cached.write_text(json.dumps(colours))
    print(f"{slug}: sampled palette {' '.join(colours)}", file=sys.stderr)
    return colours


def source():
    """Where the source of this build is: the commit checked out."""
    try:
        commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return REPO
    return f"{REPO}/tree/{commit}"


def facts(bundle):
    """What the catalog says about a bundle beyond web/discs.json."""
    manifest = json.loads((bundle / "bundle.json").read_text())
    names = [f["name"].split("/")[-1] for f in manifest["files"]]
    return {
        "provider": manifest.get("provider"),
        "bytes": sum(f["size"] for f in manifest["files"]),
        "title_sets": sum(1 for n in names if n.startswith("VTS_") and n.endswith(".IFO")),
        "menu_vobs": len(manifest["vobs"]),
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", type=Path, default=ROOT / "target" / "site")
    ap.add_argument("--copy", action="store_true")
    ap.add_argument("--menuseum", type=Path, default=ROOT / "target" / "release" / "menuseum")
    ap.add_argument("--catalog-only", action="store_true")
    args = ap.parse_args()

    discs = json.loads((WEB / "discs.json").read_text())["discs"]
    catalog = []
    for disc in discs:
        slug = disc["slug"]
        bundle = BUNDLES / slug
        if not (bundle / "bundle.json").exists():
            raise SystemExit(f"{slug}: no bundle at {bundle}: run scripts/bundles.py")
        entry = {k: v for k, v in disc.items() if k != "menu"}
        entry.update(facts(bundle), bundle=f"bundles/{slug}/", palette=menu_palette(args.menuseum, slug, bundle, disc.get("menu", {})))
        catalog.append(entry)
    text = json.dumps({"source": source(), "discs": catalog}, indent=2) + "\n"
    if not CATALOG.exists() or CATALOG.read_text() != text:  # unchanged, a dev server needn't reload
        CATALOG.write_text(text)
    if args.catalog_only:
        return

    if not (WEB / "pkg" / "menuseum_wasm.wasm").exists():
        raise SystemExit("web/pkg/menuseum_wasm.wasm is missing: run scripts/dev.sh --build-only")
    # Emptied rather than replaced, so a server running in it keeps working.
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    for old in out.iterdir():
        if old.is_dir() and not old.is_symlink():
            shutil.rmtree(old)
        else:
            old.unlink()
    subprocess.run(["npm", "run", "--silent", "build", "--", "--outDir", str(out)], cwd=WEB, check=True)
    (out / "bundles").mkdir()
    for disc in discs:
        place(BUNDLES / disc["slug"], out / "bundles" / disc["slug"], args.copy)
    total = sum(d["bytes"] for d in catalog)
    print(f"site: {len(catalog)} discs, {total / 1e6:.0f} MB of menus, in {os.path.relpath(out)}", file=sys.stderr)


if __name__ == "__main__":
    main()
