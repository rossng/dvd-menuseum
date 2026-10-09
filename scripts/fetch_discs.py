#!/usr/bin/env python3
"""Fetch DVD-Video fixtures listed in fixtures/discs.json from archive.org.

By default only the menu data is fetched: VIDEO_TS.{IFO,BUP,VOB} and
VTS_nn_0.{IFO,BUP,VOB}. Files are pulled straight out of the remote disc image
with HTTP range requests against its ISO 9660 filesystem, so the (large) title
VOBs are never downloaded. --full additionally downloads the whole image, which
the libdvdnav oracle needs to play through title cells.

Output, per disc (all gitignored):
  fixtures/discs/<slug>/VIDEO_TS/...   menu files
  fixtures/discs/<slug>/listing.json   every VIDEO_TS file with size and LBA
  fixtures/discs/<slug>/SHA256SUMS
  fixtures/discs/<slug>/disc.iso       only with --full

Usage: scripts/fetch_discs.py [--full] [slug ...]
"""

import argparse
import hashlib
import json
import re
import subprocess
import sys
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "fixtures" / "discs.json"
OUT = ROOT / "fixtures" / "discs"
SECTOR = 2048
MENU_FILE = re.compile(r"^(VIDEO_TS|VTS_\d\d_0)\.(IFO|BUP|VOB)$")


class RemoteImage:
    def __init__(self, archive_id: str, image: str):
        self.url = f"https://archive.org/download/{archive_id}/{urllib.parse.quote(image)}"
        # Resolve the redirect to the storage node once.
        req = urllib.request.Request(self.url, method="HEAD")
        with urllib.request.urlopen(req) as r:
            self.url = r.url
            self.size = int(r.headers["Content-Length"])

    def read(self, offset: int, length: int) -> bytes:
        req = urllib.request.Request(
            self.url, headers={"Range": f"bytes={offset}-{offset + length - 1}"}
        )
        with urllib.request.urlopen(req) as r:
            if r.status != 206:
                raise RuntimeError(f"server ignored Range header (HTTP {r.status})")
            return r.read()

    def copy_to(self, offset: int, length: int, dest: Path, attempts: int = 5) -> str:
        h = hashlib.sha256()
        done = 0
        with open(dest, "wb") as f:
            for _ in range(attempts):
                req = urllib.request.Request(
                    self.url, headers={"Range": f"bytes={offset + done}-{offset + length - 1}"}
                )
                try:
                    with urllib.request.urlopen(req) as r:
                        while chunk := r.read(1 << 20):
                            f.write(chunk)
                            h.update(chunk)
                            done += len(chunk)
                except OSError as e:  # dropped connection: resume from where we got to
                    print(f"   retrying after {e}", file=sys.stderr)
                if done == length:
                    return h.hexdigest()
        raise RuntimeError(f"{dest}: got {done} bytes, want {length}")


def iso_dir(img: RemoteImage, lba: int, size: int):
    """Yield (name, lba, size, is_dir) for an ISO 9660 directory extent."""
    data = img.read(lba * SECTOR, size)
    pos = 0
    while pos < len(data):
        n = data[pos]
        if n == 0:  # records never span sectors; skip to the next one
            pos = (pos // SECTOR + 1) * SECTOR
            continue
        rec = data[pos : pos + n]
        name_len = rec[32]
        name = rec[33 : 33 + name_len]
        if name not in (b"\x00", b"\x01"):
            yield (
                name.decode("ascii").split(";")[0].rstrip("."),
                int.from_bytes(rec[2:6], "little"),
                int.from_bytes(rec[10:14], "little"),
                bool(rec[25] & 2),
            )
        pos += n


def list_video_ts(img: RemoteImage):
    pvd = img.read(16 * SECTOR, SECTOR)
    if pvd[1:6] != b"CD001" or pvd[0] != 1:
        raise RuntimeError("no ISO 9660 primary volume descriptor (UDF-only image?)")
    root = pvd[156:190]
    root_lba = int.from_bytes(root[2:6], "little")
    root_size = int.from_bytes(root[10:14], "little")
    for name, lba, size, is_dir in iso_dir(img, root_lba, root_size):
        if is_dir and name.upper() == "VIDEO_TS":
            return [
                {"name": n.upper(), "lba": l, "size": s}
                for n, l, s, d in iso_dir(img, lba, size)
                if not d
            ]
    raise RuntimeError("no VIDEO_TS directory")


def fetch(disc: dict, full: bool):
    dest = OUT / disc["slug"]
    vts = dest / "VIDEO_TS"
    vts.mkdir(parents=True, exist_ok=True)
    print(f"== {disc['slug']}", flush=True)

    img = RemoteImage(disc["archive_id"], disc["image"])
    files = list_video_ts(img)
    (dest / "listing.json").write_text(
        json.dumps({"image": disc["image"], "image_size": img.size, "files": files}, indent=2)
        + "\n"
    )

    sums = []
    for f in files:
        if not MENU_FILE.match(f["name"]):
            continue
        path = vts / f["name"]
        if path.exists() and path.stat().st_size == f["size"]:
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            print(f"   {f['name']:<14} {f['size']:>12,}  (cached)")
        else:
            print(f"   {f['name']:<14} {f['size']:>12,}", flush=True)
            digest = img.copy_to(f["lba"] * SECTOR, f["size"], path)
        sums.append(f"{digest}  VIDEO_TS/{f['name']}")
    (dest / "SHA256SUMS").write_text("\n".join(sums) + "\n")

    if full:
        print(f"   full image ({img.size:,} bytes) -> disc.iso", flush=True)
        subprocess.run(
            ["curl", "-fsSL", "-C", "-", "-o", str(dest / "disc.iso"), img.url], check=True
        )
        if (dest / "disc.iso").stat().st_size != img.size:
            raise RuntimeError("disc.iso size mismatch")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--full", action="store_true", help="also download the whole disc image")
    ap.add_argument("slugs", nargs="*", help="discs to fetch (default: all)")
    args = ap.parse_args()

    discs = json.loads(MANIFEST.read_text())["discs"]
    unknown = set(args.slugs) - {d["slug"] for d in discs}
    if unknown:
        sys.exit(f"unknown slug(s): {', '.join(sorted(unknown))}")
    failed = []
    for d in discs:
        if args.slugs and d["slug"] not in args.slugs:
            continue
        try:
            fetch(d, args.full)
        except Exception as e:  # keep going; report at the end
            print(f"   FAILED: {e}", file=sys.stderr)
            failed.append(d["slug"])
    if failed:
        sys.exit(f"failed: {', '.join(failed)}")


if __name__ == "__main__":
    main()
