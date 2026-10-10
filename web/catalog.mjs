// The catalog the pages are built from (as `virtual:catalog`, see
// astro.config.mjs): the discs of fixtures/discs.json, in shelf order, each
// joined with facts from its menu bundle (fixtures/bundles/<slug>/, extracted
// by scripts/bundles.py) and its look (src/data/looks.json, written by
// scripts/looks.py); and `source`, the commit the site is built from (the GPL
// source offer for the player). $DISCS, slugs separated by spaces, puts only
// those discs on the shelf.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const ROOT = new URL("../", import.meta.url);
const REPO = "https://github.com/rossng/dvd-menuseum";
export const DISCS = fileURLToPath(new URL("fixtures/discs.json", ROOT));
export const LOOKS = fileURLToPath(new URL("web/src/data/looks.json", ROOT));
export const BUNDLES = fileURLToPath(new URL("fixtures/bundles/", ROOT));

const json = (path) => JSON.parse(readFileSync(path, "utf8"));

// The discs on the shelf: all of them, or those $DISCS names.
export function shelf() {
  const all = json(DISCS).discs;
  const only = process.env.DISCS?.split(/\s+/).filter(Boolean);
  if (!only) return all;
  const unknown = only.filter((slug) => !all.some((d) => d.slug === slug));
  if (unknown.length) throw new Error(`$DISCS: not in fixtures/discs.json: ${unknown.join(" ")}`);
  return all.filter((d) => only.includes(d.slug));
}

// Where the source of this build is: the commit checked out.
function source() {
  try {
    const commit = process.env.GITHUB_SHA || execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
    return `${REPO}/tree/${commit}`;
  } catch {
    return REPO;
  }
}

export function catalog() {
  const looks = json(LOOKS);
  const discs = shelf().map(({ archive_id, image, notes, menu, ...disc }) => {
    const look = looks[disc.slug];
    if (!look) throw new Error(`${disc.slug}: no look in src/data/looks.json: run scripts/looks.py`);
    let manifest;
    try {
      manifest = json(`${BUNDLES}${disc.slug}/bundle.json`);
    } catch {
      throw new Error(`${disc.slug}: no bundle in fixtures/bundles/: run scripts/bundles.py`);
    }
    const names = manifest.files.map((f) => f.name.split("/").pop());
    return {
      ...disc,
      source: `https://archive.org/details/${archive_id}`,
      bundle: `bundles/${disc.slug}/`,
      provider: manifest.provider,
      bytes: manifest.files.reduce((n, f) => n + f.size, 0),
      title_sets: names.filter((n) => /^VTS_\d\d_0\.IFO$/.test(n)).length,
      palette: look.palette,
      widescreen: look.widescreen,
    };
  });
  return { source: source(), discs };
}
