// Presentation only: the player runs in player-worker.js. Frames, audio,
// subpictures and button info arrive placed on a timeline (ms); the audio
// clock maps the timeline to wall time, freezing it during stills and
// stalls. Used by the player page (page.js).
//
// const p = createPlayer({video, overlay, placeholder}, {bundle | files, seed, compat}, hooks)
//   files: [{path, file}], a local disc's VIDEO_TS (see player-worker.js)
//   hooks (all optional): ready(provider), state(st), log(msg), stats(text), error(msg),
//                         widescreen(bool), placeholder(text | null)
//   p.start()                    begin playback (call from a user gesture)
//   p.input(kind, a, b)          up/down/left/right/activate, menu <name>, pointer/click x y
//   p.showRects(bool)            outline the buttons
//   p.terminate()                stop the worker, before start() (a disc that won't open)
//   p.pictureCoords(event)       pointer position in picture pixels

export function createPlayer(els, opts, hooks = {}) {
  const { video, overlay } = els;
  // Frames arrive as planar YCbCr and are converted to RGB in a shader
  // where WebGL 2 is available, else as RGBA converted by the decoder.
  const yuv = yuvRenderer(video);
  const vctx = yuv ? null : video.getContext("2d"), octx = overlay.getContext("2d");
  const log = (msg) => hooks.log?.(msg);
  const worker = new Worker(new URL("./player-worker.js", import.meta.url), { type: "module" });
  const actx = new AudioContext();

  let epoch = 0;
  let frames = [], audio = [], spus = [], buttonSets = [], stills = [], placeholders = [];
  let state = null, liveButton = 0, stopped = false, rects = false;
  let base = null;        // AudioContext time (s) at which the timeline was at 0
  let frozenAt = null;    // timeline ms while the clock is held
  let holding = null;     // why: "still" or "stall"
  let stillTimer = null;
  let lastFrame = null;   // {at, duration}
  let sources = [];       // scheduled audio: {node, epoch}
  let audioUntil = -Infinity; // timeline ms where the audio received so far ends
  let shown = 0, dropped = 0, stalls = 0, lastPresentedPost = 0;
  let shownPlaceholder = null;

  // The video clock. currentTime moves in steps of several ms and is read
  // at uneven moments, so a frame due near a refresh would land on that
  // refresh or the next at random: uneven cadence, seen as judder. Instead
  // map the refresh's own timestamp onto the context's clock through a
  // smoothed offset, and aim at when the refresh reaches the screen. Video
  // also lags by the audio output latency, so it shows what is being heard.
  let offset = null;      // currentTime minus performance.now(), smoothed (ms)
  let latency = null;     // currentTime minus the time being heard, smoothed (ms)
  let refresh = 1000 / 60, lastTs = null, grid = null, clock = 0, samples = 0;
  function tick(ts) {
    const k = 1 / Math.min(++samples, 100); // a running mean at first, then slow
    if (lastTs !== null && ts - lastTs < 100) refresh += (ts - lastTs - refresh) * 0.05;
    lastTs = ts;
    // rAF timestamps wander a ms or so either side of the refreshes: follow
    // a smoothed grid of refreshes instead.
    const due = grid === null ? ts : grid + refresh * Math.max(1, Math.round((ts - grid) / refresh));
    grid = Math.abs(ts - due) > 4 ? ts : due + (ts - due) * 0.05;
    const sample = actx.currentTime * 1000 - performance.now();
    if (offset !== null && Math.abs(sample - offset) > 50) samples = 1; // the context was suspended
    offset = samples === 1 ? sample : offset + (sample - offset) * k;
    const o = actx.getOutputTimestamp?.();
    const l = o?.performanceTime > 0
      ? actx.currentTime * 1000 - (o.contextTime * 1000 + performance.now() - o.performanceTime)
      : (actx.outputLatency ?? 0) * 1000;
    const lc = Math.min(Math.max(l, 0), 500);
    latency = latency === null ? lc : latency + (lc - latency) * k;
    clock = grid + refresh + offset - latency;
  }

  // Phase lock. Where frames last p/q refreshes (25 fps on 60 Hz: 12/5),
  // the moments they fall due sit on a lattice of spacing refresh/q against
  // the refreshes, and a frame due right on a refresh boundary lands on one
  // side or the other at the mercy of the clock's last bit of jitter. So
  // nudge the video clock to keep the lattice midway between refreshes.
  // Moving the nudge by a whole spacing moves one frame to the next refresh,
  // so it only does that past a spacing either way: frames drifting against
  // the display (29.97 fps on 60 Hz) then slip by one refresh once, cleanly.
  let nudge = 0;
  function lock(late, duration) {
    const r = duration / refresh;
    let q = 1;
    while (q <= 12 && Math.abs(q * r - Math.round(q * r)) > 0.03) q++;
    if (q > 12) return;
    const s = refresh / q;
    nudge += (s / 2 - (((late % s) + s) % s)) * 0.1;
    if (nudge > s) nudge -= s;
    else if (nudge < -s) nudge += s;
  }
  const now = () => (frozenAt !== null ? frozenAt : clock - base * 1000 + nudge);
  function freeze(at, why) {
    if (frozenAt === null) [frozenAt, holding] = [at, why];
  }
  function unfreeze() {
    if (frozenAt !== null) {
      base = actx.currentTime - frozenAt / 1000;
      [frozenAt, holding] = [null, null];
    }
    clearTimeout(stillTimer);
    stillTimer = null;
  }

  function insertSorted(list, item) {
    let i = list.length;
    while (i > 0 && list[i - 1].at > item.at) i--;
    list.splice(i, 0, item);
  }

  worker.onmessage = (e) => {
    const m = e.data;
    if (m.epoch !== undefined && m.epoch < epoch) return;
    switch (m.type) {
      case "ready":
        hooks.ready?.(m.provider);
        break;
      case "frame":
        insertSorted(frames, m);
        break;
      case "audio":
        insertSorted(audio, m);
        audioUntil = Math.max(audioUntil, m.at + (m.frames / m.rate) * 1000);
        break;
      case "subpicture":
        insertSorted(spus, m);
        break;
      case "buttons":
        insertSorted(buttonSets, m);
        break;
      case "still":
        insertSorted(stills, m);
        break;
      case "placeholder":
        insertSorted(placeholders, m);
        log(`stripped: title ${m.title}, chapter ${m.chapter}`);
        break;
      case "flush":
        epoch = m.epoch;
        frames = frames.filter((f) => f.epoch >= epoch);
        audio = audio.filter((a) => a.epoch >= epoch);
        audioUntil = Math.max(-Infinity, ...audio.map((a) => a.at + (a.frames / a.rate) * 1000));
        spus = spus.filter((s) => s.epoch >= epoch);
        buttonSets = buttonSets.filter((b) => b.epoch >= epoch);
        stills = [];
        placeholders = placeholders.filter((p) => p.epoch >= epoch);
        for (const s of sources) if (s.epoch < epoch) s.node.stop();
        sources = sources.filter((s) => s.epoch >= epoch);
        if (base !== null) unfreeze();
        break;
      case "state":
        state = m;
        liveButton = m.button;
        hooks.widescreen?.(m.widescreen);
        hooks.state?.(m);
        break;
      case "stopped":
        stopped = true;
        log("stopped");
        break;
      case "error":
        log(`error: ${m.message}`);
        hooks.error?.(m.message);
        break;
      case "log":
        log(m.message);
        break;
    }
  };

  // ---- overlay ----

  let drawn = null;
  function drawOverlay(t) {
    const spu = [...spus].reverse().find((s) => s.at <= t && (s.until === null || s.until > t)) ?? null;
    const set = [...buttonSets].reverse().find((b) => b.at <= t) ?? null;
    // Forget subpictures and button sets superseded long ago.
    if (spus.length > 16) spus = spus.slice(-16);
    if (buttonSets.length > 16) buttonSets = buttonSets.slice(-16);
    const key = [spu, set, liveButton, rects];
    if (drawn && key.every((k, i) => k === drawn[i])) return;
    drawn = key;
    const w = state?.width ?? 720, h = state?.height ?? 576;
    if (overlay.width !== w || overlay.height !== h) [overlay.width, overlay.height] = [w, h];
    octx.clearRect(0, 0, w, h);
    const buttons = set?.buttons ?? [];
    if (spu) {
      const px = new Uint8Array(spu.data);
      const img = octx.createImageData(spu.width, spu.height);
      const sel = buttons[liveButton - 1];
      const hl = sel?.selected;
      for (let row = 0; row < spu.height; row++) {
        const y = spu.y + row;
        const inRow = hl && y >= sel.y0 && y <= sel.y1;
        for (let col = 0; col < spu.width; col++) {
          const v = px[row * spu.width + col];
          const x = spu.x + col;
          let ci = spu.color[v], alpha = spu.contrast[v];
          if (inRow && x >= sel.x0 && x <= sel.x1) [ci, alpha] = [hl.color[v], hl.contrast[v]];
          const c = spu.palette[ci & 15], o = (row * spu.width + col) * 4;
          img.data[o] = c[0];
          img.data[o + 1] = c[1];
          img.data[o + 2] = c[2];
          img.data[o + 3] = alpha * 17;
        }
      }
      octx.putImageData(img, spu.x, spu.y);
    }
    if (rects) {
      buttons.forEach((b, i) => {
        octx.strokeStyle = i + 1 === liveButton ? "#ff0" : "#0f0";
        octx.strokeRect(b.x0 + 0.5, b.y0 + 0.5, b.x1 - b.x0, b.y1 - b.y0);
        octx.fillStyle = octx.strokeStyle;
        octx.fillText(String(i + 1), b.x0 + 3, b.y0 + 11);
      });
    }
  }

  // ---- main loop ----

  function frame(ts) {
    requestAnimationFrame(frame);
    tick(ts);
    if (base === null) {
      // Prebuffer before starting the clock: 8 frames, or half a second of
      // anything. The worker decodes only so far ahead of what has been
      // shown, so a menu of a few frames over a long stretch of audio would
      // never reach 8.
      const first = frames[0] ?? placeholders[0] ?? stills[0];
      const ahead = Math.max(frames.at(-1)?.at ?? -Infinity, audio.at(-1)?.at ?? -Infinity) - (first?.at ?? 0);
      if (!first || (frames.length < 8 && ahead < 500 && !placeholders.length && !stills.length && !stopped)) return;
      base = actx.currentTime - first.at / 1000 + 0.05;
    }
    const t = now();
    if (performance.now() - lastPresentedPost > 30) {
      worker.postMessage({ type: "presented", t });
      lastPresentedPost = performance.now();
    }

    // Stills hold the clock at their position.
    const still = stills[0];
    if (still && still.at <= t && frozenAt === null) {
      stills.shift();
      freeze(still.at, "still");
      log(`still ${still.seconds === 255 ? "∞" : still.seconds + "s"}`);
      if (still.seconds !== 255) {
        stillTimer = setTimeout(() => {
          unfreeze();
          worker.postMessage({ type: "still_done", epoch: still.epoch });
        }, still.seconds * 1000);
      }
    }

    // Video: show the newest frame that is due.
    let img = null;
    while (frames.length && frames[0].at <= t) {
      if (img) dropped++;
      img = frames.shift();
    }
    if (img) {
      if (frozenAt === null) lock(t - img.at, img.duration);
      const { width, height } = img;
      if (video.width !== width || video.height !== height) [video.width, video.height] = [width, height];
      if (img.format === "yuv") yuv.draw(img);
      else vctx.putImageData(new ImageData(new Uint8ClampedArray(img.data), width, height), 0, 0);
      lastFrame = img;
      shown++;
    }

    // Placeholders for stripped cells.
    const ph = placeholders.find((p) => p.at <= t && t < p.at + p.duration) ?? null;
    if (ph !== shownPlaceholder) {
      shownPlaceholder = ph;
      hooks.placeholder?.(ph && `Title ${ph.title}, chapter ${ph.chapter}\n(not included in this archive)`);
    }
    placeholders = placeholders.filter((p) => t < p.at + p.duration);

    // Stall: out of frames and audio mid-playback (not in a still or
    // placeholder) — hold the clock until more arrive. A menu can be a
    // single picture over a long stretch of audio, which isn't a stall.
    if (frozenAt === null && !frames.length && !ph && !stopped && lastFrame && t > Math.max(lastFrame.at + lastFrame.duration, audioUntil) + 100) {
      freeze(t, "stall");
      stalls++;
    } else if (holding === "stall" && (frames.length || audioUntil > t)) {
      unfreeze();
    }

    // Audio: schedule chunks shortly before they are due.
    if (frozenAt === null) {
      while (audio.length && audio[0].at <= t + 250) {
        const a = audio.shift();
        const when = base + a.at / 1000;
        if (when < actx.currentTime - 0.02) continue; // too late
        const pcm = new Float32Array(a.data);
        const buf = actx.createBuffer(2, a.frames, a.rate);
        const l = buf.getChannelData(0), r = buf.getChannelData(1);
        for (let i = 0; i < a.frames; i++) {
          l[i] = pcm[2 * i];
          r[i] = pcm[2 * i + 1];
        }
        const node = actx.createBufferSource();
        node.buffer = buf;
        node.connect(actx.destination);
        node.start(Math.max(when, actx.currentTime));
        sources.push({ node, epoch: a.epoch });
        node.onended = () => (sources = sources.filter((s) => s.node !== node));
      }
    }

    drawOverlay(t);
    hooks.stats?.(
      `t ${(t / 1000).toFixed(2)}s${holding ? ` (${holding})` : ""} · frames shown ${shown}, dropped ${dropped}, buffered ${frames.length} · stalls ${stalls}`,
    );
  }

  worker.postMessage({
    type: "open",
    // The bundle's URL is resolved here, against the page, not the worker.
    ...(opts.files ? { files: opts.files } : { bundle: new URL(opts.bundle.replace(/\/?$/, "/"), location.href).href }),
    seed: opts.seed ?? 0,
    compat: !!opts.compat,
    yuv: !!yuv,
  });

  return {
    start() {
      actx.resume();
      requestAnimationFrame(frame);
    },
    input(kind, a, b) {
      worker.postMessage({ type: "input", kind, a, b });
    },
    terminate() {
      worker.terminate();
      actx.close();
    },
    showRects(on) {
      rects = on;
      drawn = null;
    },
    pictureCoords(e) {
      const r = overlay.getBoundingClientRect();
      return [Math.round(((e.clientX - r.left) / r.width) * overlay.width), Math.round(((e.clientY - r.top) / r.height) * overlay.height)];
    },
  };
}

// Draws planar YCbCr 4:2:0 frames on `canvas` with WebGL 2, converting
// exactly as the decoder's Picture::to_rgba does (integer BT.601 studio
// range, interlaced frames blended with the lines above and below), so the
// pixels match the RGBA path's. Returns null without WebGL 2.
function yuvRenderer(canvas) {
  const gl = canvas.getContext("webgl2", { alpha: false, antialias: false, depth: false, stencil: false });
  if (!gl) return null;
  const shader = (type, src) => {
    const s = gl.createShader(type);
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
    return s;
  };
  const VERTEX = `#version 300 es
    // One triangle covering the viewport.
    void main() { gl_Position = vec4(gl_VertexID == 1 ? 3.0 : -1.0, gl_VertexID == 2 ? 3.0 : -1.0, 0.0, 1.0); }`;
  const FRAGMENT = `#version 300 es
    precision highp float;
    precision highp int;
    precision highp usampler2D;
    uniform usampler2D Y, Cb, Cr;
    uniform int height;
    uniform bool interlaced;
    out vec4 rgba;
    int luma(int x, int y) { return int(texelFetch(Y, ivec2(x, y), 0).r); }
    void main() {
      int x = int(gl_FragCoord.x), y = height - 1 - int(gl_FragCoord.y);
      int l = luma(x, y);
      // (above + 2 * this + below) / 4
      if (interlaced && y > 0 && y + 1 < height) l = (luma(x, y - 1) + 2 * l + luma(x, y + 1) + 2) / 4;
      int u = int(texelFetch(Cb, ivec2(x / 2, y / 2), 0).r) - 128;
      int v = int(texelFetch(Cr, ivec2(x / 2, y / 2), 0).r) - 128;
      int c = (l - 16) * 298;
      ivec3 rgb = clamp((ivec3(c) + ivec3(409 * v, -100 * u - 208 * v, 516 * u) + 128) >> 8, 0, 255);
      rgba = vec4(vec3(rgb) / 255.0, 1.0);
    }`;
  function setup() {
    const prog = gl.createProgram();
    gl.attachShader(prog, shader(gl.VERTEX_SHADER, VERTEX));
    gl.attachShader(prog, shader(gl.FRAGMENT_SHADER, FRAGMENT));
    gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(prog));
    gl.useProgram(prog);
    const planes = ["Y", "Cb", "Cr"].map((name, i) => {
      gl.activeTexture(gl.TEXTURE0 + i);
      gl.bindTexture(gl.TEXTURE_2D, gl.createTexture());
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.uniform1i(gl.getUniformLocation(prog, name), i);
      return { w: 0, h: 0 };
    });
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    return { planes, height: gl.getUniformLocation(prog, "height"), interlaced: gl.getUniformLocation(prog, "interlaced") };
  }
  let st = setup();
  // A lost context (GPU reset, a backgrounded tab on mobile) comes back
  // empty: set up again when it is restored. Frames until then are skipped.
  canvas.addEventListener("webglcontextlost", (e) => {
    e.preventDefault();
    st = null;
  });
  canvas.addEventListener("webglcontextrestored", () => (st = setup()));
  return {
    draw({ width, height, interlaced, data }) {
      if (!st) return;
      const cw = (width + 1) >> 1, ch = (height + 1) >> 1;
      const bytes = new Uint8Array(data);
      const dims = [[width, height, 0], [cw, ch, width * height], [cw, ch, width * height + cw * ch]];
      st.planes.forEach((p, i) => {
        const [w, h, offset] = dims[i];
        const px = bytes.subarray(offset, offset + w * h);
        gl.activeTexture(gl.TEXTURE0 + i);
        if (p.w !== w || p.h !== h) {
          gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8UI, w, h, 0, gl.RED_INTEGER, gl.UNSIGNED_BYTE, px);
          [p.w, p.h] = [w, h];
        } else {
          gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, w, h, gl.RED_INTEGER, gl.UNSIGNED_BYTE, px);
        }
      });
      gl.uniform1i(st.height, height);
      gl.uniform1i(st.interlaced, interlaced ? 1 : 0);
      gl.viewport(0, 0, width, height);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
    },
  };
}

// Keyboard bindings shared by every page with a player.
export const KEYS = {
  ArrowUp: ["up"], ArrowDown: ["down"], ArrowLeft: ["left"], ArrowRight: ["right"],
  Enter: ["activate"], Escape: ["menu", "escape"],
  t: ["menu", "title"], r: ["menu", "root"], c: ["menu", "part"],
};
