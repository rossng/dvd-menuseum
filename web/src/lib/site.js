// What the pages know at build time: the catalog (written by
// scripts/site.py: web/discs.json joined with bundle facts, and `source`, the
// commit the site was built from) and where things are under the site's base.

import catalog from "../catalog.json";

export const { discs, source } = catalog;

// A path under the site's base, which is "/" locally and the repository's
// name on GitHub Pages.
export const url = (path = "") => `${import.meta.env.BASE_URL.replace(/\/?$/, "/")}${path}`;

// The shelf's stacks: discs with the same title (a film's PAL and NTSC
// discs, the discs of a set), in shelf order.
export const stacks = Object.values(Object.groupBy(discs, (d) => d.title));

// What tells a disc apart from the others in its stack: its part, its
// system, or both. Empty for a disc on its own.
export function variant(stack, d) {
  const varies = (k) => new Set(stack.map((x) => x[k])).size > 1;
  return ["part", "system"].filter(varies).map((k) => d[k]).join(" · ");
}

export const playUrl =(d) => url(`play/${encodeURIComponent(d.slug)}/`);
export const harnessUrl = (d) => `${url("harness/")}?bundle=${encodeURIComponent(url(d.bundle))}`;

// The deed of a Creative Commons licence name ("CC BY-SA 3.0"), if it is one.
export function licenceDeed(name) {
  const cc = /^CC ((?:BY|NC|SA|ND)(?:-(?:BY|NC|SA|ND))*) (\d\.\d)$/.exec(name ?? "");
  return cc && `https://creativecommons.org/licenses/${cc[1].toLowerCase()}/${cc[2]}/`;
}
