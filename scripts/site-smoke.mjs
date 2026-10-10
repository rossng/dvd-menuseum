#!/usr/bin/env node
// Smoke-test a built site (`npm run build` in web/) in headless Chrome: serve
// it, load every page, and fail on any exception, console error, failed
// request or content security policy violation. The disc pages must load
// their disc (the Play button lights), the local page likewise when given a
// bundle, and the first disc must play into its menus.
//
// usage: node scripts/site-smoke.mjs <site dir> [--base PATH] [--chrome PATH]
//   --base    the base path the site was built for (default $BASE_PATH or /)
//   --chrome  the browser (default $CHROME, else the usual install places)

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import path from "node:path";

const args = process.argv.slice(2);
const opt = (name, fallback) => (args.includes(name) ? args[args.indexOf(name) + 1] : fallback);
const dir = path.resolve(args.find((a, i) => !a.startsWith("--") && !args[i - 1]?.startsWith("--")) ?? "");
const base = opt("--base", process.env.BASE_PATH || "/").replace(/\/?$/, "/");
if (!existsSync(path.join(dir, "index.html"))) {
  console.error(`usage: site-smoke.mjs <site dir>: no index.html in ${dir}`);
  process.exit(2);
}

// ---- the site, served as `astro preview` serves it (answering range requests)

const web = path.resolve(path.dirname(new URL(import.meta.url).pathname), "../web");
const { preview } = await import(createRequire(path.join(web, "package.json")).resolve("astro"));
process.env.BASE_PATH = base; // read by astro.config.mjs
const server = await preview({ root: web, outDir: dir, server: { host: "127.0.0.1", port: 0 }, logLevel: "warn" });
const origin = `http://127.0.0.1:${server.port}`;

// ---- Chrome, driven over the DevTools protocol

const CANDIDATES = [
  "google-chrome", "google-chrome-stable", "chromium", "chromium-browser",
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/Applications/Chromium.app/Contents/MacOS/Chromium",
];
const onPath = (name) => (process.env.PATH ?? "").split(path.delimiter).map((d) => path.join(d, name)).find(existsSync);
const chromePath = opt("--chrome", process.env.CHROME) ?? CANDIDATES.map((c) => (c.includes("/") ? existsSync(c) && c : onPath(c))).find(Boolean);
if (!chromePath) {
  console.error("no Chrome or Chromium found: pass --chrome PATH");
  process.exit(2);
}
const profile = mkdtempSync(path.join(tmpdir(), "site-smoke-"));
const chrome = spawn(chromePath, [
  "--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`, "--no-first-run",
  "--no-default-browser-check", "--autoplay-policy=no-user-gesture-required", "about:blank",
], { stdio: ["ignore", "ignore", "pipe"] });
const browserWs = await new Promise((ok, fail) => {
  let err = "";
  chrome.on("error", fail);
  chrome.stderr.on("data", (d) => {
    err += d;
    const m = /DevTools listening on (ws:\/\/\S+)/.exec(err);
    if (m) ok(m[1]);
  });
  chrome.once("exit", () => fail(new Error(`${chromePath} exited:\n${err}`)));
});
const devtools = `http://${new URL(browserWs).host}`;

async function open(url) {
  const target = await (await fetch(`${devtools}/json/new?about:blank`, { method: "PUT" })).json();
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((ok) => (ws.onopen = ok));
  let id = 0;
  const pending = new Map(), problems = [];
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id) return pending.get(msg.id)?.(msg);
    const p = msg.params;
    if (msg.method === "Runtime.exceptionThrown") problems.push(`exception: ${p.exceptionDetails.exception?.description ?? p.exceptionDetails.text}`);
    if (msg.method === "Runtime.consoleAPICalled" && p.type === "error") problems.push(`console: ${p.args.map((a) => a.value ?? a.description).join(" ")}`);
    if (msg.method === "Log.entryAdded" && p.entry.level === "error") problems.push(`${p.entry.source}: ${p.entry.text} ${p.entry.url ?? ""}`);
  };
  const send = (method, params = {}) => new Promise((ok, fail) => {
    pending.set(++id, (msg) => (msg.error ? fail(new Error(`${method}: ${msg.error.message}`)) : ok(msg.result)));
    ws.send(JSON.stringify({ id, method, params }));
  });
  for (const domain of ["Runtime", "Log", "Page"]) await send(`${domain}.enable`);
  await send("Page.navigate", { url });
  const evaluate = async (expression) =>
    (await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true })).result.value;
  // Polls `expression` until it's truthy; returns its last value.
  const until = async (expression, seconds) => {
    let value;
    for (const end = Date.now() + seconds * 1000; Date.now() < end; await new Promise((ok) => setTimeout(ok, 250))) {
      value = await evaluate(expression).catch(() => undefined);
      if (value) return value;
    }
    return value;
  };
  return { evaluate, until, problems, close: () => ws.close() };
}

// ---- the pages

const discs = readdirSync(path.join(dir, "play")).filter((d) => d !== "local");
const pages = [
  {
    path: "",
    checks: [["the seal loads", "!!customElements.get('seal-of-slop')"],
             ["Open Sans loads", "document.fonts.ready.then(() => [...document.fonts].some((f) => f.family.includes('Open Sans') && f.status === 'loaded'))"]],
  },
  { path: "notices/", checks: [["the notices list the crates", "document.querySelectorAll('main li').length > 0"]] },
  { path: "play/local/", checks: [["the folder picker is ready", "!!document.querySelector('input[type=file]')"]] },
  ...discs.map((slug, i) => ({
    path: `play/${slug}/`,
    checks: [["the disc loads", "document.getElementById('cta').textContent === 'Play'"]],
    // The first disc plays: its status display shows where the VM is.
    play: i === 0,
  })),
  {
    // Testing: the local page, given a bundle.
    path: `play/local/?bundle=${encodeURIComponent(`${base}bundles/${discs[0]}/`)}`,
    checks: [["the disc loads", "document.getElementById('cta').textContent === 'Play'"]],
  },
];

let failed = 0;
for (const page of pages) {
  const tab = await open(`${origin}${base}${page.path}`);
  const report = [];
  for (const [what, expression] of page.checks) {
    if (!(await tab.until(expression, 30))) report.push(`${what}: no`);
  }
  if (page.play) {
    await tab.evaluate("document.getElementById('insert').click()");
    const where = await tab.until("/ — /.test(document.getElementById('where').textContent) && document.getElementById('where').textContent", 30);
    if (where) console.log(`  /${page.path}: playing, at “${where}”`);
    else report.push("it plays: no state from the player");
  }
  await new Promise((ok) => setTimeout(ok, 500)); // stragglers: late requests and errors
  report.push(...tab.problems);
  tab.close();
  console.log(`${report.length ? "FAIL" : "ok  "} /${page.path}`);
  for (const r of report) console.log(`       ${r.length > 300 ? `${r.slice(0, 300)}…` : r}`);
  failed += report.length > 0;
}

const exited = new Promise((ok) => chrome.once("exit", ok));
chrome.kill();
await exited;
await server.stop();
rmSync(profile, { recursive: true, force: true, maxRetries: 5 });
console.log(failed ? `${failed} of ${pages.length} pages failed` : `all ${pages.length} pages ok`);
process.exit(failed ? 1 : 0);
