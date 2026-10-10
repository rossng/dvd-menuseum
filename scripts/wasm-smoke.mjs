// Headless smoke test of the wasm player, as the worker runs it (through
// web/src/player/abi.js, with the disc from discs.js), with the bundle's
// files read from disk and a simulated clock.
//
// usage: node scripts/wasm-smoke.mjs <bundle dir> [seconds] ['<time>:<input> ...']
// (WASM=<path> runs another build of the module; FRAMES=yuv takes frames as
// planar YCbCr, as the browser player does with WebGL 2, instead of RGBA.)
import { openAsBlob } from "node:fs";
import { readFile, readdir } from "node:fs/promises";
import path from "node:path";
import { abi } from "../web/src/player/abi.js";
import { discFromFiles } from "../web/src/player/discs.js";

const root = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..");
const [dir, secondsArg = "40", scriptArg = ""] = process.argv.slice(2);
const script = scriptArg.split(/\s+/).filter(Boolean).map((s) => {
  const [t, kind, a] = s.split(":");
  return { t: Number(t) * 1000, kind, a };
});

const { instance } = await WebAssembly.instantiate(await readFile(process.env.WASM ?? path.join(root, "target/wasm32-unknown-unknown/release/menuseum_wasm.wasm")), {});
const player = abi(instance.exports);
// The bundle's files, as the worker sees a local disc's.
const names = await readdir(path.join(dir, "VIDEO_TS"));
const files = await Promise.all(names.map(async (name) => ({ path: `VIDEO_TS/${name}`, file: await openAsBlob(path.join(dir, "VIDEO_TS", name)) })));
const disc = discFromFiles(files);
const sets = new Map(disc.sets.map((v) => [v.id, v.read]));
await player.load(disc);
player.frameFormat(process.env.FRAMES === "yuv");
player.start(0, false);

const counts = {};
let still = null, now = 0, decodeMs = 0;
const end = Number(secondsArg) * 1000;
// Outputs are converted (to RGBA, f32 samples) as they are popped, so time that too.
const next = () => {
  const t0 = performance.now();
  const out = player.next();
  decodeMs += performance.now() - t0;
  return out;
};
while (now < end) {
  while (script.length && script[0].t <= now) {
    const s = script.shift();
    const ok = player.input(s.kind, s.a);
    console.log(`${(now / 1000).toFixed(1)}s input ${s.kind}${s.a ? ` ${s.a}` : ""}: ${ok ? "ok" : "ignored"}`);
    if (ok) still = null;
  }
  const t0 = performance.now();
  while (player.step(now) === 1) {
    for (const [id, chunk, start, stop] of player.wanted()) player.putChunk(id, chunk, await sets.get(id)(start, stop));
  }
  decodeMs += performance.now() - t0;
  for (let out; (out = next()); ) {
    const { type, data, ...meta } = out;
    counts[type] = (counts[type] ?? 0) + 1;
    if (type === "still") {
      console.log(`${(now / 1000).toFixed(1)}s still ${meta.seconds}s at ${(meta.at / 1000).toFixed(1)}s`);
      if (meta.seconds !== 255) still = { epoch: meta.epoch, until: meta.at + meta.seconds * 1000 };
    } else if (type === "state") {
      console.log(`${(now / 1000).toFixed(1)}s state d=${meta.domain} vts=${meta.vts} pgc=${meta.pgc} cell=${meta.cell} btn=${meta.button}`);
    } else if (type === "placeholder" || type === "error" || type === "flush") {
      console.log(`${(now / 1000).toFixed(1)}s ${type} ${JSON.stringify(meta)}`);
    }
  }
  if (still && now >= still.until) {
    player.stillDone(still.epoch);
    still = null;
  }
  now += 40;
}
console.log(JSON.stringify(counts), `decode+convert ${(decodeMs / 1000).toFixed(1)}s for ${(end / 1000).toFixed(0)}s of playback`);
