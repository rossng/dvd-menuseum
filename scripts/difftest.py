#!/usr/bin/env python3
"""Differential test: compare `menuseum trace` with the libdvdnav oracle.

For each disc image and seed, a random input script is generated and fed to
both players; their event traces must be identical. Mismatches are written to
target/difftest/ with both traces and the script.

Usage: scripts/difftest.py [--seeds N] [--length N] [--max-events N] [disc.iso ...]
Default discs: every fixtures/discs/*/disc.iso.
Build first: cargo build --release && oracle/build.sh
"""

import argparse
import random
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OURS = ROOT / "target" / "release" / "menuseum"
ORACLE = ROOT / "target" / "oracle" / "oracle"
OUT = ROOT / "target" / "difftest"

MOVES = ["up", "down", "left", "right"]
MENUS = ["menu:root", "menu:title", "menu:escape", "menu:part", "menu:audio", "menu:subpicture"]


def script(seed: int, length: int) -> str:
    rng = random.Random(seed)
    out = []
    for _ in range(length):
        r = rng.random()
        if r < 0.45:
            out.append(rng.choice(MOVES))
        elif r < 0.85:
            out.append("activate")
        elif r < 0.93:
            out.append(f"select:{rng.randint(1, 8)}")
        else:
            out.append(rng.choice(MENUS))
    return " ".join(out)


def run(cmd: list[str]) -> list[str]:
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=1800)
    if p.returncode != 0:
        return [f"EXIT {p.returncode}: {p.stderr.strip()}"]
    return p.stdout.splitlines()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seeds", type=int, default=20)
    ap.add_argument("--first-seed", type=int, default=0)
    ap.add_argument("--length", type=int, default=30)
    ap.add_argument("--max-events", type=int, default=20000)
    ap.add_argument("--compat", action="store_true", help="pass --compat to menuseum")
    ap.add_argument("discs", nargs="*")
    args = ap.parse_args()

    discs = [Path(d) for d in args.discs] or sorted((ROOT / "fixtures" / "discs").glob("*/disc.iso"))
    OUT.mkdir(parents=True, exist_ok=True)
    failures = 0
    for disc in discs:
        name = disc.parent.name if disc.name == "disc.iso" else disc.stem
        for seed in range(args.first_seed, args.first_seed + args.seeds):
            inputs = script(seed, args.length)
            common = ["-i", inputs, "--max-events", str(args.max_events), "--seed", str(seed)]
            ours = run([str(OURS), "trace", str(disc), *common, *(["--compat"] if args.compat else [])])
            theirs = run([str(ORACLE), str(disc), *common])
            if ours == theirs:
                print(f"ok   {name} seed={seed} ({len(ours)} lines, {ours[-1] if ours else ''})")
                continue
            failures += 1
            base = OUT / f"{name}-{seed}"
            (base.with_suffix(".ours")).write_text("\n".join(ours) + "\n")
            (base.with_suffix(".oracle")).write_text("\n".join(theirs) + "\n")
            (base.with_suffix(".script")).write_text(inputs + "\n")
            first = next((i for i, (a, b) in enumerate(zip(ours, theirs)) if a != b), min(len(ours), len(theirs)))
            print(f"FAIL {name} seed={seed} at line {first + 1}")
            print(f"  ours:   {ours[first][:220] if first < len(ours) else '<end>'}")
            print(f"  oracle: {theirs[first][:220] if first < len(theirs) else '<end>'}")
    print(f"{failures} failure(s)")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
