#!/usr/bin/env python3
"""Write web/src/data/notices.json: the third-party crates compiled into the
web player (menuseum-wasm, for wasm32), with the licence each is used under
and its notice, for the site's Licences dialog and notices page.

usage: scripts/notices.py [--check] [--cargo PATH]

  --check       fail if the file is out of date, rather than writing it
  --cargo PATH  the cargo to run (default cargo)

Follows normal dependencies only, and not into proc macros, which run at
build time and leave nothing in the module. A crate offering MIT is used
under MIT. Its notice is the copyright lines of its licence file (with its
authors, when the file has none) and the file's text; crates with the same
text share it.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "web" / "src" / "data" / "notices.json"
ROOT_CRATE = "menuseum-wasm"
TARGET = "wasm32-unknown-unknown"
# Licence files to read, by the licence a crate is used under.
FILES = {"MIT": ["LICENSE-MIT", "LICENSE-MIT.md", "LICENSE-MIT.txt", "LICENSE", "LICENSE.md", "LICENSE.txt"]}
GENERIC = ["LICENSE", "LICENSE.md", "LICENSE.txt", "COPYING"]


def chosen(expr):
    """The licence a crate is used under: MIT if its expression offers it."""
    if "MIT" in re.split(r"[\s()/]+|\bOR\b|\bAND\b", expr or ""):
        return "MIT"
    if not expr:
        raise SystemExit("a crate has no licence expression")
    return expr


def notice(pkg, licence):
    """(copyright lines, licence text without them) from the crate's licence file."""
    folder = Path(pkg["manifest_path"]).parent
    for name in FILES.get(licence, GENERIC):
        path = folder / name
        if path.is_file():
            text = path.read_text()
            if licence == "MIT" and "Permission is hereby granted" not in text:
                continue
            break
    else:
        raise SystemExit(f"{pkg['name']}: no {licence} licence file in {folder}")
    lines = text.splitlines()
    copyright = [l.strip() for l in lines if re.match(r"\s*Copyright\b", l, re.I) and "notice" not in l.lower()]
    body = [l for l in lines if l.strip() not in copyright]
    # Paragraphs, each on one line: the files wrap the same words differently.
    paragraphs = [" ".join(p.split()) for p in re.split(r"\n\s*\n", "\n".join(body))]
    paragraphs = [p for p in paragraphs if p and p not in ("MIT License", "The MIT License (MIT)", "The MIT License")]
    return copyright, "\n\n".join(paragraphs)


def crates(cargo):
    meta = json.loads(subprocess.run(
        [cargo, "metadata", "--format-version", "1", "--locked", "--filter-platform", TARGET],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout)
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    members = set(meta["workspace_members"])
    proc_macro = lambda p: all("proc-macro" in t["kind"] for t in p["targets"])
    start = next(p["id"] for p in meta["packages"] if p["name"] == ROOT_CRATE)
    seen, todo = {start}, [start]
    while todo:
        for dep in nodes[todo.pop()]["deps"]:
            if not any(k["kind"] is None for k in dep["dep_kinds"]) or dep["pkg"] in seen:
                continue
            if proc_macro(packages[dep["pkg"]]):
                continue
            seen.add(dep["pkg"])
            todo.append(dep["pkg"])
    return [packages[i] for i in seen if i not in members]


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--cargo", default="cargo")
    args = ap.parse_args()

    groups = {}
    for pkg in sorted(crates(args.cargo), key=lambda p: p["name"]):
        licence = chosen(pkg["license"])
        copyright, text = notice(pkg, licence)
        local = Path(pkg["manifest_path"]).is_relative_to(ROOT / "vendor")
        groups.setdefault((licence, text), []).append({
            "name": pkg["name"],
            "version": pkg["version"],
            "offered": pkg["license"],
            "copyright": copyright,
            # A licence file without a copyright line: who wrote it, instead.
            **({} if copyright else {"authors": [a.split(" <")[0] for a in pkg["authors"]]}),
            **({"note": "vendored, with local changes"} if local else {}),
        })
    licences = [{"licence": l, "text": t, "crates": c} for (l, t), c in groups.items()]
    out = json.dumps({"_comment": "Written by scripts/notices.py; don't edit.", "licences": licences}, indent=2, ensure_ascii=False) + "\n"
    if args.check:
        if not OUT.exists() or OUT.read_text() != out:
            raise SystemExit(f"{OUT.relative_to(ROOT)} is out of date: run scripts/notices.py")
        return
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(out)
    names = [c["name"] for g in licences for c in g["crates"]]
    print(f"notices: {len(names)} crates: {', '.join(names)}", file=sys.stderr)


if __name__ == "__main__":
    main()
