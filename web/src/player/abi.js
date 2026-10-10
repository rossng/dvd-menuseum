// The wasm player's C ABI (crates/wasm), for player-worker.js and
// scripts/wasm-smoke.mjs: abi(exports) wraps the module's exports in calls
// that take strings and bytes.

// What mm_next_output returns, by its number.
export const OUTPUTS = [null, "frame", "audio", "subpicture", "buttons", "still", "placeholder", "flush", "state", "stopped", "error"];
export const INPUTS = { up: 0, down: 1, left: 2, right: 3, activate: 4, select: 5, menu: 6, pointer: 7, click: 8 };
export const MENUS = { escape: 0, title: 1, root: 2, subpicture: 3, audio: 4, angle: 5, part: 6 };

export function abi(x) {
  // The text the last call left (JSON, or an error message).
  const text = () => new TextDecoder().decode(new Uint8Array(x.memory.buffer, x.mm_result_ptr(), x.mm_result_len()));
  const check = (ok, what) => {
    if (!ok) throw new Error(`${what}: ${text()}`);
  };
  // Calls f(ptr, len) with `bytes` (or a string's UTF-8) copied into the module.
  function withBytes(bytes, f) {
    if (typeof bytes === "string") bytes = new TextEncoder().encode(bytes);
    const ptr = x.mm_alloc(bytes.length);
    new Uint8Array(x.memory.buffer, ptr, bytes.length).set(bytes);
    try {
      return f(ptr, bytes.length);
    } finally {
      x.mm_free(ptr, bytes.length);
    }
  }

  return {
    // Load a disc (see discs.js): its IFOs, and the VOB sets it can stream.
    async load(disc) {
      for (const f of disc.ifos) {
        const kind = f.name === "VIDEO_TS.IFO" ? 0 : parseInt(f.name.slice(4, 6), 10);
        const bytes = await f.read();
        check(withBytes(bytes, (p, n) => x.mm_add_ifo(kind, p, n)), f.name);
      }
      for (const v of disc.sets) check(withBytes(v.id, (p, n) => x.mm_declare_vobs(p, n, v.sectors)), v.id);
    },
    // Frames as planar YCbCr (for a WebGL 2 page) rather than RGBA.
    frameFormat: (yuv) => check(x.mm_frame_format(yuv ? 1 : 0), "frame format"),
    start: (seed, compat) => check(x.mm_start(seed, compat ? 1 : 0, 800, 96), "start"),
    // 0: ahead of `presentedMs`; 1: waiting on chunks (see wanted()).
    step: (presentedMs) => x.mm_step(presentedMs),
    // The chunks it wants: [[vobs id, chunk, start byte, end byte], ...].
    wanted() {
      check(x.mm_wanted(), "wanted");
      return JSON.parse(text());
    },
    putChunk: (id, chunk, bytes) => withBytes(id, (ip, il) => withBytes(bytes, (dp, dl) => x.mm_put_chunk(ip, il, chunk, dp, dl))),
    // The next output, {type, ...its fields, data: ArrayBuffer | null}, or null.
    next() {
      const kind = x.mm_next_output();
      if (!kind) return null;
      const out = JSON.parse(text());
      const len = x.mm_data_len();
      return { type: OUTPUTS[kind], ...out, data: len ? x.memory.buffer.slice(x.mm_data_ptr(), x.mm_data_ptr() + len) : null };
    },
    // up/down/left/right/activate, select n, menu <name>, pointer/click x y.
    input: (kind, a, b) => !!x.mm_input(INPUTS[kind], kind === "menu" ? MENUS[a] : a ?? 0, b ?? 0),
    stillDone: (epoch) => x.mm_still_done(epoch),
  };
}
