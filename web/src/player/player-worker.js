// The DVD player, off the main thread: opens a disc (a menu bundle on the
// web, or a VIDEO_TS's files the viewer picked; see discs.js), runs the wasm
// player ahead of presentation, fetches the chunks it wants, and posts its
// outputs to the page.
//
// Messages in:  {type: "open", bundle | files, seed, compat, yuv}
//                 files: [{path, file}] of a local disc (File objects)
//                 yuv: frames as planar YCbCr, not RGBA
//               {type: "presented", t}        presentation time (timeline ms)
//               {type: "input", kind, a, b}
//               {type: "still_done", epoch}
// Messages out: the player's outputs (abi.js OUTPUTS: {type: "frame" |
//                 "audio" | ... | "error", ...}), {type: "ready", provider},
//                 {type: "log", message}

import wasm from "../../pkg/menuseum_wasm.wasm?url"; // built by scripts/dev.sh
import { abi } from "./abi.js";
import { bundleDisc, discFromFiles } from "./discs.js";

let player = null;
let sets = new Map(); // vobs id -> read(start, end)
let presented = 0;
let wake = null;
const inflight = new Set();

const ready = WebAssembly.instantiateStreaming(fetch(wasm), {}).then(({ instance }) => {
  player = abi(instance.exports);
});

const fail = (err) => postMessage({ type: "error", message: String(err) });

function drain() {
  for (let out; (out = player.next()); ) postMessage(out, out.data ? [out.data] : []);
}

function fetchWanted() {
  for (const [id, chunk, start, end] of player.wanted()) {
    const key = `${id}:${chunk}`;
    if (inflight.has(key) || inflight.size >= 6) continue;
    inflight.add(key);
    sets.get(id)(start, end)
      .then((bytes) => player.putChunk(id, chunk, bytes))
      .catch(fail)
      .finally(() => {
        inflight.delete(key);
        kick();
      });
  }
}

// Step the player ahead of presentation. It asks for chunks when it needs
// them, and reads ahead a little even when it doesn't, so that fetches
// overlap playback; kick() steps it again when one lands.
function run() {
  const step = player.step(presented);
  drain();
  if (step === 0 || step === 1) fetchWanted();
}

function kick() {
  if (wake) return;
  wake = setTimeout(() => {
    wake = null;
    run();
  }, 0);
}

async function open(msg) {
  await ready;
  const disc = msg.files ? discFromFiles(msg.files) : await bundleDisc(msg.bundle);
  await player.load(disc);
  sets = new Map(disc.sets.map((v) => [v.id, v.read]));
  // The page converts planar YCbCr frames on the GPU when it can.
  player.frameFormat(msg.yuv);
  player.start(msg.seed ?? 0, msg.compat);
  postMessage({ type: "ready", provider: disc.provider });
  kick();
}

onmessage = (e) => {
  const m = e.data;
  try {
    switch (m.type) {
      case "open":
        open(m).catch(fail);
        break;
      case "presented":
        presented = m.t;
        kick();
        break;
      case "input": {
        const ok = player.input(m.kind, m.a, m.b);
        if (m.kind !== "pointer") postMessage({ type: "log", message: `input ${m.kind}${m.a !== undefined ? " " + m.a : ""}: ${ok ? "ok" : "ignored"}` });
        drain();
        kick();
        break;
      }
      case "still_done":
        player.stillDone(m.epoch);
        kick();
        break;
    }
  } catch (err) {
    fail(err);
  }
};
