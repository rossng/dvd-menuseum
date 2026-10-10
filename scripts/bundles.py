#!/usr/bin/env python3
"""Menu bundles are committed as zstd-compressed tarballs in Git LFS and
played from extracted copies, which the web player streams with HTTP range
requests.

usage:
  scripts/bundles.py unpack [slug ...]   extract archives whose working copy is
                                         missing or out of date (the default)
  scripts/bundles.py pack [slug ...]     archive working copies, e.g. after
                                         `menuseum bundle <disc> --out fixtures/bundles/<slug>`

fixtures/bundles/<slug>.tar.zst   committed (LFS)
fixtures/bundles/<slug>/          working copy (gitignored), with a .archive
                                  stamp naming the archive it came from

zstd's long mode matters: menus often repeat the same encoded video, up to
~100 MB apart, which halves Elephants Dream. Decompression needs the same
window, so both sides pass --long.
"""

import hashlib
import json
import shutil
import subprocess
import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "fixtures" / "bundles"
LONG = "--long=27"
STAMP = ".archive"
LFS_POINTER = b"version https://git-lfs.github.com/spec/"


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(1 << 20):
            h.update(chunk)
    return h.hexdigest()


def up_to_date(archive, out):
    """Whether `out` was extracted from this exact archive. Size and mtime
    are checked first so an unchanged checkout doesn't rehash every archive."""
    try:
        stamp = json.loads((out / STAMP).read_text())
    except (OSError, ValueError):
        return False, None
    st = archive.stat()
    if [stamp.get("size"), stamp.get("mtime_ns")] == [st.st_size, st.st_mtime_ns]:
        return True, stamp
    return stamp.get("sha256") == sha256(archive), stamp


def unpack(slug):
    archive, out = ROOT / f"{slug}.tar.zst", ROOT / slug
    with open(archive, "rb") as f:
        if f.read(len(LFS_POINTER)) == LFS_POINTER:
            print(f"{slug}: archive is an LFS pointer; run `git lfs pull --exclude=\"\"`", file=sys.stderr)
            return False
    current, stamp = up_to_date(archive, out)
    st = archive.stat()
    if current:
        if stamp and stamp.get("mtime_ns") != st.st_mtime_ns:
            stamp.update(size=st.st_size, mtime_ns=st.st_mtime_ns)
            (out / STAMP).write_text(json.dumps(stamp) + "\n")
        return True
    tmp = ROOT / f".{slug}.tmp"
    shutil.rmtree(tmp, ignore_errors=True)
    tmp.mkdir()
    zstd = subprocess.Popen(["zstd", "-q", "-dc", LONG, str(archive)], stdout=subprocess.PIPE)
    with tarfile.open(fileobj=zstd.stdout, mode="r|") as tar:
        tar.extractall(tmp, filter="data")
    if zstd.wait():
        raise SystemExit(f"{slug}: zstd failed")
    stamp = {"sha256": sha256(archive), "size": st.st_size, "mtime_ns": st.st_mtime_ns}
    (tmp / STAMP).write_text(json.dumps(stamp) + "\n")
    shutil.rmtree(out, ignore_errors=True)
    tmp.rename(out)
    print(f"{slug}: extracted")
    return True


def pack(slug):
    src, archive = ROOT / slug, ROOT / f"{slug}.tar.zst"
    # Reproducible: sorted entries, no timestamps or owners, so an unchanged
    # bundle packs to an identical archive and LFS stores nothing new.
    zstd = subprocess.Popen(["zstd", "-q", "-19", LONG, "-T0", "-f", "-o", str(archive)], stdin=subprocess.PIPE)
    with tarfile.open(fileobj=zstd.stdin, mode="w|", format=tarfile.PAX_FORMAT) as tar:
        for path in sorted(p for p in src.rglob("*") if p.name != STAMP):
            info = tar.gettarinfo(path, path.relative_to(src).as_posix())
            info.mtime, info.uid, info.gid, info.uname, info.gname = 0, 0, 0, "", ""
            info.mode = 0o755 if info.isdir() else 0o644
            if info.isfile():
                with open(path, "rb") as f:
                    tar.addfile(info, f)
            else:
                tar.addfile(info)
    zstd.stdin.close()
    if zstd.wait():
        raise SystemExit(f"{slug}: zstd failed")
    st = archive.stat()
    stamp = {"sha256": sha256(archive), "size": st.st_size, "mtime_ns": st.st_mtime_ns}
    (src / STAMP).write_text(json.dumps(stamp) + "\n")
    total = sum(p.stat().st_size for p in src.rglob("*") if p.is_file() and p.name != STAMP)
    print(f"{slug}: {total / 1e6:.1f} MB -> {st.st_size / 1e6:.1f} MB")


def main():
    args = sys.argv[1:]
    cmd = args.pop(0) if args and args[0] in ("pack", "unpack") else "unpack"
    if cmd == "pack":
        slugs = args or sorted(p.name for p in ROOT.iterdir() if p.is_dir() and not p.name.startswith("."))
        for slug in slugs:
            pack(slug)
    else:
        slugs = args or sorted(p.name[: -len(".tar.zst")] for p in ROOT.glob("*.tar.zst"))
        if not all([unpack(slug) for slug in slugs]):
            sys.exit(1)


if __name__ == "__main__":
    main()
