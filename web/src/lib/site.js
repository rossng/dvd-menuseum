// What the pages know at build time: the catalog (written by
// scripts/site.py: web/discs.json joined with bundle facts, and `source`, the
// commit the site was built from) and where things are under the site's base.

import catalog from "../catalog.json";

export const { discs, source } = catalog;

// A path under the site's base, which is "/" locally and the repository's
// name on GitHub Pages.
export const url = (path = "") => `${import.meta.env.BASE_URL.replace(/\/?$/, "/")}${path}`;

export const playUrl = (d) => url(`play/${encodeURIComponent(d.slug)}/`);
export const harnessUrl = (d) => `${url("harness/")}?bundle=${encodeURIComponent(url(d.bundle))}`;

// The deed of a Creative Commons licence name ("CC BY-SA 3.0"), if it is one.
export function licenceDeed(name) {
  const cc = /^CC ((?:BY|NC|SA|ND)(?:-(?:BY|NC|SA|ND))*) (\d\.\d)$/.exec(name ?? "");
  return cc && `https://creativecommons.org/licenses/${cc[1].toLowerCase()}/${cc[2]}/`;
}
