// Presentation only: the player runs in player-worker.js. Frames, audio,
// subpictures and button info arrive placed on a timeline (ms); the audio
// clock maps the timeline to wall time, freezing it during stills and
// stalls. Shared by the site's player page and the test harness.
//
// const p = createPlayer({video, overlay, placeholder}, {bundle, seed, compat}, hooks)
//   hooks (all optional): ready(provider), state(st), log(msg), stats(text),
//                         widescreen(bool), placeholder(text | null)
//   p.start()                    begin playback (call from a user gesture)
//   p.input(kind, a, b)          up/down/left/right/activate, menu <name>, pointer/click x y
//   p.showRects(bool)            outline the buttons
//   p.pictureCoords(event)       pointer position in picture pixels

export function createPlayer(els, opts, hooks = {}) {
  const { video, overlay } = els;
  const vctx = video.getContext("2d"), octx = overlay.getContext("2d");
  const log = (msg) => hooks.log?.(msg);
  const worker = new Worker(new URL("player-worker.js", import.meta.url));
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
  let shown = 0, dropped = 0, stalls = 0, lastPresentedPost = 0;
  let shownPlaceholder = null;

  const now = () => (frozenAt !== null ? frozenAt : (actx.currentTime - base) * 1000);
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

  function frame() {
    requestAnimationFrame(frame);
    if (base === null) {
      // Prebuffer before starting the clock.
      const first = frames[0] ?? placeholders[0] ?? stills[0];
      if (!first || (frames.length < 8 && !placeholders.length && !stills.length && !stopped)) return;
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
      const { width, height } = img;
      if (video.width !== width || video.height !== height) [video.width, video.height] = [width, height];
      vctx.putImageData(new ImageData(new Uint8ClampedArray(img.data), width, height), 0, 0);
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

    // Stall: out of frames mid-playback (not in a still or placeholder) — hold
    // the clock until more arrive.
    if (frozenAt === null && !frames.length && !ph && !stopped && lastFrame && t > lastFrame.at + lastFrame.duration + 100) {
      freeze(t, "stall");
      stalls++;
    } else if (holding === "stall" && frames.length) {
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
    bundle: opts.bundle.endsWith("/") ? opts.bundle : opts.bundle + "/",
    seed: opts.seed ?? 0,
    compat: !!opts.compat,
  });

  return {
    start() {
      actx.resume();
      requestAnimationFrame(frame);
    },
    input(kind, a, b) {
      worker.postMessage({ type: "input", kind, a, b });
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

// Keyboard bindings shared by every page with a player.
export const KEYS = {
  ArrowUp: ["up"], ArrowDown: ["down"], ArrowLeft: ["left"], ArrowRight: ["right"],
  Enter: ["activate"], Escape: ["menu", "escape"],
  t: ["menu", "title"], r: ["menu", "root"], c: ["menu", "part"],
};
