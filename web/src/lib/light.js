// The light the discs reflect: following the pointer, or drifting.

// --light, on `el`: the angle of the light its discs reflect. Set there
// rather than on the root so that each change restyles only the discs.
// Returns a function that stops it.
export function light(el) {
  let angle = 0, movedAt = -1e9, running = true;
  const move = (e) => {
    angle = (e.clientX / innerWidth) * 220 - 110 + (e.clientY / innerHeight) * 80;
    movedAt = performance.now();
  };
  addEventListener("pointermove", move);
  (function frame(t) {
    if (!running) return;
    const a = performance.now() - movedAt > 3000 ? Math.sin(t / 2600) * 90 : angle;
    el.style.setProperty("--light", `${a.toFixed(1)}deg`);
    requestAnimationFrame(frame);
  })(0);
  return () => {
    running = false;
    removeEventListener("pointermove", move);
  };
}
