// The DVD player, off the main thread: loads a bundle and streams its VOB
// sectors with HTTP range requests, or reads a disc's VIDEO_TS from files
// the viewer picked; runs the wasm player ahead of presentation and posts
// its outputs to the page.
//
// Messages in:  {type: "open", bundle | files, seed, compat, yuv}
//                 files: [{path, file}] of a local disc (File objects)
//                 yuv: frames as planar YCbCr, not RGBA
//               {type: "presented", t}        presentation time (timeline ms)
//               {type: "input", kind, a, b}
//               {type: "still_done", epoch}
// Messages out: {type: "frame" | "audio" | "subpicture" | "buttons" | "still" |
//                "placeholder" | "flush" | "state" | "stopped" | "error", ...}
//               {type: "ready"} {type: "log", message}

import wasm from "../../pkg/menuseum_wasm.wasm?url"; // built by scripts/dev.sh

const KINDS = [null, "frame", "audio", "subpicture", "buttons", "still", "placeholder", "flush", "state", "stopped", "error"];

const INPUTS = { up: 0, down: 1, left: 2, right: 3, activate: 4, select: 5, menu: 6, pointer: 7, click: 8 };
const MENUS = { escape: 0, title: 1, root: 2, subpicture: 3, audio: 4, angle: 5, part: 6 };

let x = null;
let vobs = new Map(); // vobs id -> read(start, end)
let presented = 0;
let running = false;
let wake = null;
const inflight = new Set();

const ready = WebAssembly.instantiateStreaming(fetch(wasm), {}).then(({ instance }) => {
  x = instance.exports;
});

function copyIn(bytes) {
  const ptr = x.mm_alloc(bytes.length);
  new Uint8Array(x.memory.buffer, ptr, bytes.length).set(bytes);
  return [ptr, bytes.length];
}

function text() {
  return new TextDecoder().decode(new Uint8Array(x.memory.buffer, x.mm_result_ptr(), x.mm_result_len()));
}

function check(ok, what) {
  if (!ok) throw new Error(`${what}: ${text()}`);
}

async function fetchBytes(base, path, range) {
  const r = await fetch(new URL(path, base), range ? { headers: { Range: `bytes=${range[0]}-${range[1] - 1}` } } : {});
  if (!r.ok) throw new Error(`${path}: HTTP ${r.status}`);
  const buf = new Uint8Array(await r.arrayBuffer());
  if (range && r.status === 200) return buf.subarray(range[0], range[1]); // server ignored Range
  return buf;
}

function drain() {
  let kind;
  while ((kind = x.mm_next_output())) {
    const meta = JSON.parse(text());
    const len = x.mm_data_len();
    let data = null;
    if (len) data = x.memory.buffer.slice(x.mm_data_ptr(), x.mm_data_ptr() + len);
    postMessage({ type: KINDS[kind], ...meta, data }, data ? [data] : []);
  }
}

function fetchWanted() {
  check(x.mm_wanted(), "wanted");
  for (const [id, chunk, start, end] of JSON.parse(text())) {
    const key = `${id}:${chunk}`;
    if (inflight.has(key) || inflight.size >= 6) continue;
    inflight.add(key);
    vobs.get(id)(start, end)
      .then((bytes) => {
        const idp = copyIn(new TextEncoder().encode(id));
        const dp = copyIn(bytes);
        x.mm_put_chunk(...idp, chunk, ...dp);
        x.mm_free(...idp);
        x.mm_free(...dp);
      })
      .catch((e) => postMessage({ type: "error", message: String(e) }))
      .finally(() => {
        inflight.delete(key);
        kick();
      });
  }
}

// Step the player until it is ahead of presentation, waiting on fetches.
function run() {
  if (running) return;
  running = true;
  try {
    for (;;) {
      const step = x.mm_step(presented);
      drain();
      if (step === 1) {
        fetchWanted();
        break; // resumed by kick() when a chunk lands
      }
      // Read ahead a little even when idle, so fetches overlap playback.
      if (step === 0) fetchWanted();
      break;
    }
  } finally {
    running = false;
  }
}

function kick() {
  if (wake) return;
  wake = setTimeout(() => {
    wake = null;
    run();
  }, 0);
}

// A bundle on the web: its manifest lists the IFOs and menu VOB sets.
async function bundleDisc(url) {
  const base = new URL(url, location.href);
  const manifest = await (await fetch(new URL("bundle.json", base))).json();
  const ifos = manifest.files
    .map((f) => ({ name: f.name.split("/").pop(), path: f.name }))
    .filter((f) => f.name.endsWith(".IFO"))
    .map((f) => ({ name: f.name, read: () => fetchBytes(base, f.path) }));
  const sets = manifest.vobs.map((v) => ({ id: v.id, sectors: v.sectors, read: (a, b) => fetchBytes(base, v.file, [a, b]) }));
  return { provider: manifest.provider, ifos, sets };
}

// Bytes [start, end) of files laid end to end, as a title VOB set's are.
function spanning(parts) {
  return async (start, end) => {
    const out = new Uint8Array(end - start);
    let at = 0;
    for (const f of parts) {
      const a = Math.max(start, at), b = Math.min(end, at + f.size);
      if (a < b) out.set(new Uint8Array(await f.slice(a - at, b - at).arrayBuffer()), a - start);
      at += f.size;
    }
    return out;
  };
}

// A whole disc from the viewer's own files: every IFO (or its backup) and
// every VOB set, the film included. Nothing is uploaded.
function localDisc(list) {
  const byName = new Map();
  for (const { path, file } of list) {
    const name = path.split("/").pop().toUpperCase();
    // Prefer a VIDEO_TS directory's copy over a stray one beside it.
    if (!byName.has(name) || /(^|\/)VIDEO_TS\//i.test(path)) byName.set(name, file);
  }
  const ifos = [];
  const sets = [];
  const add = (id, parts) => {
    if (parts.length) sets.push({ id, sectors: Math.floor(parts.reduce((n, f) => n + f.size, 0) / 2048), read: spanning(parts) });
  };
  const stems = ["VIDEO_TS", ...[...byName.keys()].filter((n) => /^VTS_\d\d_0\.(IFO|BUP)$/.test(n)).map((n) => n.slice(0, 8))];
  for (const stem of new Set(stems)) {
    const ifo = byName.get(`${stem}.IFO`) ?? byName.get(`${stem}.BUP`);
    if (ifo) ifos.push({ name: `${stem}.IFO`, read: async () => new Uint8Array(await ifo.arrayBuffer()) });
    const menu = byName.get(`${stem}.VOB`);
    if (stem === "VIDEO_TS") {
      add("vmgm", menu ? [menu] : []);
      continue;
    }
    const n = parseInt(stem.slice(4, 6), 10);
    add(`menu-${n}`, menu ? [menu] : []);
    const title = [];
    for (let i = 1; i <= 9 && byName.has(`VTS_${stem.slice(4, 6)}_${i}.VOB`); i++) title.push(byName.get(`VTS_${stem.slice(4, 6)}_${i}.VOB`));
    add(`title-${n}`, title);
  }
  if (!ifos.some((f) => f.name === "VIDEO_TS.IFO")) throw new Error("no VIDEO_TS.IFO among those files");
  return { provider: null, ifos, sets };
}

async function open(msg) {
  await ready;
  const disc = msg.files ? localDisc(msg.files) : await bundleDisc(msg.bundle);
  for (const f of disc.ifos) {
    const kind = f.name === "VIDEO_TS.IFO" ? 0 : parseInt(f.name.slice(4, 6), 10);
    const p = copyIn(await f.read());
    check(x.mm_add_ifo(kind, ...p), f.name);
    x.mm_free(...p);
  }
  for (const v of disc.sets) {
    vobs.set(v.id, v.read);
    const p = copyIn(new TextEncoder().encode(v.id));
    check(x.mm_declare_vobs(...p, v.sectors), v.id);
    x.mm_free(...p);
  }
  // The page converts planar YCbCr frames on the GPU when it can.
  check(x.mm_frame_format(msg.yuv ? 1 : 0), "frame format");
  check(x.mm_start(msg.seed ?? 0, msg.compat ? 1 : 0, 800, 96), "start");
  postMessage({ type: "ready", provider: disc.provider });
  kick();
}

onmessage = (e) => {
  const m = e.data;
  try {
    switch (m.type) {
      case "open":
        open(m).catch((err) => postMessage({ type: "error", message: String(err) }));
        break;
      case "presented":
        presented = m.t;
        kick();
        break;
      case "input": {
        const a = m.kind === "menu" ? MENUS[m.a] : m.a ?? 0;
        const ok = x.mm_input(INPUTS[m.kind], a, m.b ?? 0);
        if (m.kind !== "pointer") postMessage({ type: "log", message: `input ${m.kind}${m.a !== undefined ? " " + m.a : ""}: ${ok ? "ok" : "ignored"}` });
        drain();
        kick();
        break;
      }
      case "still_done":
        x.mm_still_done(m.epoch);
        kick();
        break;
    }
  } catch (err) {
    postMessage({ type: "error", message: String(err) });
  }
};
