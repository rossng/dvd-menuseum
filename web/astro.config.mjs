// The site is static: every page is rendered at build time from the catalog
// (catalog.mjs, imported by the pages as `virtual:catalog`). The menu bundles
// are too big to pass through the build: they're copied beside its output
// afterwards, and served from fixtures/bundles/ by `astro dev`.

import { constants, cpSync, rmSync } from "node:fs";
import { basename } from "node:path";
import { defineConfig } from "astro/config";
import sirv from "sirv";
import { BUNDLES, DISCS, LOOKS, catalog, shelf } from "./catalog.mjs";

const base = process.env.BASE_PATH || "/";

function catalogModule() {
  const id = "virtual:catalog";
  return {
    name: "menuseum-catalog",
    resolveId: (source) => (source === id ? `\0${id}` : undefined),
    load(source) {
      if (source !== `\0${id}`) return;
      this.addWatchFile(DISCS);
      this.addWatchFile(LOOKS);
      return `export default ${JSON.stringify(catalog())};`;
    },
  };
}

// <base>bundles/<slug>/: served by `astro dev` (sirv answers the HTTP range
// requests the player streams VOB sectors with), and copied into the built
// site (cloned, where the filesystem can).
const bundles = {
  name: "menuseum-bundles",
  hooks: {
    "astro:server:setup": ({ server }) => {
      server.middlewares.use(`${base.replace(/\/$/, "")}/bundles`, sirv(BUNDLES, { dev: true }));
    },
    "astro:build:done": ({ dir }) => {
      const out = new URL("bundles/", dir);
      rmSync(out, { recursive: true, force: true });
      for (const { slug } of shelf()) {
        cpSync(`${BUNDLES}${slug}`, new URL(`${slug}/`, out), {
          recursive: true,
          filter: (path) => !basename(path).startsWith("."),
          mode: constants.COPYFILE_FICLONE,
        });
      }
    },
  },
};

export default defineConfig({
  base,
  integrations: [bundles],
  // Every page gets a content security policy, as a <meta> element (GitHub
  // Pages can't send headers): everything comes from the site itself, with
  // Astro's hashes for its inline scripts and styles. Style attributes are
  // allowed: the shelf places its discs with them. Not applied by `astro dev`.
  security: {
    csp: {
      directives: [
        "default-src 'self'",
        "img-src 'self' data:",
        "font-src 'self'",
        "connect-src 'self'",
        "worker-src 'self'",
        "object-src 'none'",
        "base-uri 'self'",
        "form-action 'none'",
      ],
      styleDirective: {
        resources: [
          { resource: "'self'", kind: "element" },
          { resource: "'unsafe-inline'", kind: "attribute" },
        ],
      },
    },
  },
  // No Markdown code to highlight, and Shiki's inline styles defeat the CSP.
  markdown: { syntaxHighlight: false },
  vite: {
    plugins: [catalogModule()],
    // The player's worker imports its wasm module's URL.
    worker: { format: "es" },
    // Small assets (the rarer alphabets' fonts) stay files, not data: URLs
    // the CSP would refuse.
    build: { assetsInlineLimit: 0 },
  },
});
