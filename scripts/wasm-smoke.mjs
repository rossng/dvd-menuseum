// Headless smoke test of the wasm player through the same C ABI the worker
// uses, with chunks read from the bundle on disk and a simulated clock.
//
// usage: node scripts/wasm-smoke.mjs <bundle dir> [seconds] ['<time>:<input> ...']
// (WASM=<path> runs another build of the module.)
import { readFile, open } from "node:fs/promises";
import path from "node:path";

const root = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..");
const [dir, secondsArg = "40", scriptArg = ""] = process.argv.slice(2);
const script = scriptArg.split(/\s+/).filter(Boolean).map((s) => {
  const [t, kind, a] = s.split(":");
  return { t: Number(t) * 1000, kind, a };
});
const INPUTS = { up: 0, down: 1, left: 2, right: 3, activate: 4, menu: 6 };
const KINDS = [null, "frame", "audio", "subpicture", "buttons", "still", "placeholder", "flush", "state", "stopped", "error"];

const { instance } = await WebAssembly.instantiate(await readFile(process.env.WASM ?? path.join(root, "target/wasm32-unknown-unknown/release/menuseum_wasm.wasm")), {});
const x = instance.exports;
const copyIn = (bytes) => {
  const p = x.mm_alloc(bytes.length);
  new Uint8Array(x.memory.buffer, p, bytes.length).set(bytes);
  return [p, bytes.length];
};
const text = () => new TextDecoder().decode(new Uint8Array(x.memory.buffer, x.mm_result_ptr(), x.mm_result_len()));
const manifest = JSON.parse(await readFile(path.join(dir, "bundle.json"), "utf8"));
for (const f of manifest.files.filter((f) => f.name.endsWith(".IFO"))) {
  const name = f.name.split("/").pop();
  const p = copyIn(await readFile(path.join(dir, f.name)));
  if (!x.mm_add_ifo(name === "VIDEO_TS.IFO" ? 0 : parseInt(name.slice(4, 6), 10), ...p)) throw new Error(text());
}
const files = new Map();
for (const v of manifest.vobs) {
  files.set(v.id, await open(path.join(dir, v.file)));
  if (!x.mm_declare_vobs(...copyIn(new TextEncoder().encode(v.id)), v.sectors)) throw new Error(text());
}
if (!x.mm_start(0, 0, 800, 96)) throw new Error(text());

const counts = {};
let still = null, now = 0, decodeMs = 0;
const end = Number(secondsArg) * 1000;
// Outputs are converted (to RGBA, f32 samples) as they are popped, so time that too.
const nextOutput = () => {
  const t0 = performance.now();
  const kind = x.mm_next_output();
  decodeMs += performance.now() - t0;
  return kind;
};
while (now < end) {
  while (script.length && script[0].t <= now) {
    const s = script.shift();
    const ok = x.mm_input(INPUTS[s.kind], s.kind === "menu" ? 0 : 0, 0);
    console.log(`${(now / 1000).toFixed(1)}s input ${s.kind}: ${ok ? "ok" : "ignored"}`);
    if (ok) still = null;
  }
  const t0 = performance.now();
  for (;;) {
    const step = x.mm_step(now);
    if (step !== 1) break;
    x.mm_wanted();
    for (const [id, chunk, start, stop] of JSON.parse(text())) {
      const buf = Buffer.alloc(stop - start);
      await files.get(id).read(buf, 0, buf.length, start);
      const idp = copyIn(new TextEncoder().encode(id)), dp = copyIn(buf);
      x.mm_put_chunk(...idp, chunk, ...dp);
      x.mm_free(...idp);
      x.mm_free(...dp);
    }
  }
  decodeMs += performance.now() - t0;
  let kind;
  while ((kind = nextOutput())) {
    const name = KINDS[kind], meta = JSON.parse(text());
    counts[name] = (counts[name] ?? 0) + 1;
    if (name === "still") {
      console.log(`${(now / 1000).toFixed(1)}s still ${meta.seconds}s at ${(meta.at / 1000).toFixed(1)}s`);
      if (meta.seconds !== 255) still = { epoch: meta.epoch, until: meta.at + meta.seconds * 1000 };
    } else if (name === "state") {
      console.log(`${(now / 1000).toFixed(1)}s state d=${meta.domain} vts=${meta.vts} pgc=${meta.pgc} cell=${meta.cell} btn=${meta.button}`);
    } else if (name === "placeholder" || name === "error" || name === "flush") {
      console.log(`${(now / 1000).toFixed(1)}s ${name} ${JSON.stringify(meta)}`);
    }
  }
  if (still && now >= still.until) {
    x.mm_still_done(still.epoch);
    still = null;
  }
  now += 40;
}
console.log(JSON.stringify(counts), `decode+convert ${(decodeMs / 1000).toFixed(1)}s for ${(end / 1000).toFixed(0)}s of playback`);
