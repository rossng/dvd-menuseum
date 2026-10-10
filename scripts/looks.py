#!/usr/bin/env python3
"""Write web/src/data/looks.json: how each disc in fixtures/discs.json looks
on the site, from its menu bundle. Its label's colours are sampled from the
menu a viewer first lands on, and its screen is drawn the shape of its first
picture before it plays.

usage: scripts/looks.py [--check] [--menuseum PATH] [slug ...]

  --check          fail if a disc's look is missing or out of date, rather
                   than writing them (needs neither the bundles nor the CLI)
  --menuseum PATH  the CLI that renders menus (default target/release/menuseum)
  slug ...         re-sample these discs even if they're up to date

A look records the archive it was sampled from (its Git LFS object id, which
its LFS pointer carries even where the archive isn't downloaded) and the
disc's `menu` options, and is re-sampled when either changes. Sampling needs
the bundles extracted (scripts/bundles.py).
"""

import argparse
import colorsys
import json
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

import bundles

ROOT = Path(__file__).resolve().parent.parent
DISCS = ROOT / "fixtures" / "discs.json"
BUNDLES = ROOT / "fixtures" / "bundles"
OUT = ROOT / "web" / "src" / "data" / "looks.json"


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


def sample(menuseum, slug, menu):
    """The palette of the last picture of a `menuseum render` run with the
    disc's scripted inputs, and whether its first picture is 16:9."""
    video_ts = BUNDLES / slug / "VIDEO_TS"
    if not video_ts.is_dir():
        raise SystemExit(f"{slug}: no bundle at {video_ts.parent}: run scripts/bundles.py")
    with tempfile.TemporaryDirectory() as tmp:
        cmd = [str(menuseum), "render", str(video_ts), "--out", tmp, "-i", menu.get("inputs", ""), "--settle", str(menu.get("settle", 12))]
        subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL)
        pngs = sorted(Path(tmp).glob("*.png"))
        if not pngs:
            raise SystemExit(f"{slug}: render produced no picture")
        colours = palette(pngs[-1])
    aspect = subprocess.run([str(menuseum), "aspect", str(video_ts)], check=True, capture_output=True, text=True).stdout.strip()
    return {"palette": colours, "widescreen": aspect == "16:9"}


def archive_id(slug):
    """The Git LFS object id of a bundle's archive (the SHA-256 of its
    contents): from its LFS pointer where it isn't downloaded, else from the
    stamp of its extracted copy, else by hashing it."""
    archive = BUNDLES / f"{slug}.tar.zst"
    if not archive.exists():
        raise SystemExit(f"{slug}: no bundle archive {archive.relative_to(ROOT)}")
    with open(archive, "rb") as f:
        head = f.read(200)
    if head.startswith(bundles.LFS_POINTER):
        return next(l.split(":", 1)[1] for l in head.decode().splitlines() if l.startswith("oid sha256:"))
    current, stamp = bundles.up_to_date(archive, BUNDLES / slug)
    return stamp["sha256"] if current else bundles.sha256(archive)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--menuseum", type=Path, default=ROOT / "target" / "release" / "menuseum")
    ap.add_argument("slugs", nargs="*")
    args = ap.parse_args()

    discs = json.loads(DISCS.read_text())["discs"]
    old = json.loads(OUT.read_text()) if OUT.exists() else {}
    looks, stale = {}, []
    for disc in discs:
        slug, menu = disc["slug"], disc.get("menu", {})
        oid = archive_id(slug)
        look = old.get(slug)
        if look and look.get("archive") == oid and look.get("menu", {}) == menu and slug not in args.slugs:
            looks[slug] = look
            continue
        stale.append(slug)
        if not args.check:
            looks[slug] = {**sample(args.menuseum, slug, menu), "archive": oid, **({"menu": menu} if menu else {})}
            print(f"{slug}: {' '.join(looks[slug]['palette'])}{' 16:9' if looks[slug]['widescreen'] else ''}", file=sys.stderr)
    if args.check:
        if stale or set(old) != set(looks):
            raise SystemExit(f"{OUT.relative_to(ROOT)} is out of date ({' '.join(stale) or 'discs removed'}): run scripts/looks.py")
        return
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(looks, indent=2) + "\n")


if __name__ == "__main__":
    main()
