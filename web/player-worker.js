// The DVD player, off the main thread: loads a bundle, streams its VOB
// sectors with HTTP range requests, runs the wasm player ahead of
// presentation and posts its outputs to the page.
//
// Messages in:  {type: "open", bundle, seed, compat, yuv}  yuv: frames as planar YCbCr, not RGBA
//               {type: "presented", t}        presentation time (timeline ms)
//               {type: "input", kind, a, b}
//               {type: "still_done", epoch}
// Messages out: {type: "frame" | "audio" | "subpicture" | "buttons" | "still" |
//                "placeholder" | "flush" | "state" | "stopped" | "error", ...}
//               {type: "ready"} {type: "log", message}

const KINDS = [null, "frame", "audio", "subpicture", "buttons", "still", "placeholder", "flush", "state", "stopped", "error"];
const INPUTS = { up: 0, down: 1, left: 2, right: 3, activate: 4, select: 5, menu: 6, pointer: 7, click: 8 };
const MENUS = { escape: 0, title: 1, root: 2, subpicture: 3, audio: 4, angle: 5, part: 6 };

let x = null;
let base = null;
let files = new Map(); // vobs id -> URL
let presented = 0;
let running = false;
let wake = null;
const inflight = new Set();

const ready = WebAssembly.instantiateStreaming(fetch("pkg/menuseum_wasm.wasm"), {}).then(({ instance }) => {
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

async function fetchBytes(path, range) {
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
    fetchBytes(files.get(id), [start, end])
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

async function open(msg) {
  await ready;
  base = new URL(msg.bundle, location.href);
  const manifest = await (await fetch(new URL("bundle.json", base))).json();
  for (const f of manifest.files) {
    const name = f.name.split("/").pop();
    if (!name.endsWith(".IFO")) continue;
    const kind = name === "VIDEO_TS.IFO" ? 0 : parseInt(name.slice(4, 6), 10);
    const p = copyIn(await fetchBytes(f.name));
    check(x.mm_add_ifo(kind, ...p), name);
    x.mm_free(...p);
  }
  for (const v of manifest.vobs) {
    files.set(v.id, v.file);
    const p = copyIn(new TextEncoder().encode(v.id));
    check(x.mm_declare_vobs(...p, v.sectors), v.id);
    x.mm_free(...p);
  }
  // The page converts planar YCbCr frames on the GPU when it can.
  check(x.mm_frame_format(msg.yuv ? 1 : 0), "frame format");
  check(x.mm_start(msg.seed ?? 0, msg.compat ? 1 : 0, 800, 96), "start");
  postMessage({ type: "ready", provider: manifest.provider });
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
