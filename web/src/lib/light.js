// The light the discs reflect: following the pointer, or drifting.

// --light, on `el`: the angle of the light its discs reflect, while `el`
// is .lit. Set there rather than on the root so that each change restyles
// only the discs. Between moves, the light drifts by a CSS animation (in
// site.css), which runs without the main thread. Returns a function that
// stops it.
export function light(el) {
  let idle;
  const move = (e) => {
    const angle = (e.clientX / innerWidth) * 220 - 110 + (e.clientY / innerHeight) * 80;
    el.style.setProperty("--light", `${angle.toFixed(1)}deg`);
    el.classList.add("lit");
    clearTimeout(idle);
    idle = setTimeout(() => el.classList.remove("lit"), 3000);
  };
  addEventListener("pointermove", move);
  return () => {
    clearTimeout(idle);
    removeEventListener("pointermove", move);
    el.classList.add("unlit");
  };
}
