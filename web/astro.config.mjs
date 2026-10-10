// The site is static: every page is rendered at build time from
// src/catalog.json (written by scripts/site.py). The menu bundles are too big
// to pass through the build; scripts/site.py places them beside its output,
// and in development they're served from fixtures/bundles/ by bundles() below.

import { createReadStream, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig } from "astro/config";

const BUNDLES = fileURLToPath(new URL("../fixtures/bundles/", import.meta.url));
const base = process.env.BASE_PATH || "/";

// Serve <base>bundles/ from fixtures/bundles/, answering the HTTP range
// requests the player streams VOB sectors with.
function bundles() {
  return {
    name: "menuseum-bundles",
    configureServer(server) {
      server.middlewares.use(`${base.replace(/\/$/, "")}/bundles/`, (req, res, next) => {
        const rel = decodeURIComponent(new URL(req.url, "http://x").pathname).replace(/^\/+/, "");
        if (rel.split("/").includes("..")) return next();
        const path = BUNDLES + rel;
        let size;
        try {
          const st = statSync(path);
          if (!st.isFile()) return next();
          size = st.size;
        } catch {
          return next();
        }
        res.setHeader("Accept-Ranges", "bytes");
        res.setHeader("Content-Type", path.endsWith(".json") ? "application/json" : "application/octet-stream");
        const m = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range ?? "");
        if (!m) {
          res.setHeader("Content-Length", size);
          return createReadStream(path).pipe(res);
        }
        const start = Number(m[1]);
        const end = Math.min(m[2] ? Number(m[2]) : size - 1, size - 1);
        if (start > end) {
          res.statusCode = 416;
          res.setHeader("Content-Range", `bytes */${size}`);
          return res.end();
        }
        res.statusCode = 206;
        res.setHeader("Content-Range", `bytes ${start}-${end}/${size}`);
        res.setHeader("Content-Length", end - start + 1);
        createReadStream(path, { start, end }).pipe(res);
      });
    },
  };
}

export default defineConfig({
  base,
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
    plugins: [bundles()],
    // The player's worker imports its wasm module's URL.
    worker: { format: "es" },
    // Small assets (the rarer alphabets' fonts) stay files, not data: URLs
    // the CSP would refuse.
    build: { assetsInlineLimit: 0 },
  },
});
