// What every page shares: the aurora behind it, the light the discs reflect
// (following the pointer, or drifting), and the licences dialog.

const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

const AURORA = `
<svg viewBox="0 0 1600 1000" preserveAspectRatio="none">
  <defs>
    <linearGradient id="au-a" x1="0" x2="1">
      <stop offset="0" stop-color="#6ff0d0" stop-opacity="0"/><stop offset="0.3" stop-color="#6ff0d0" stop-opacity="0.5"/>
      <stop offset="0.65" stop-color="#7fd8ff" stop-opacity="0.55"/><stop offset="1" stop-color="#7fd8ff" stop-opacity="0"/>
    </linearGradient>
    <linearGradient id="au-b" x1="0" x2="1">
      <stop offset="0" stop-color="#9fe9ff" stop-opacity="0"/><stop offset="0.5" stop-color="#bff3ff" stop-opacity="0.4"/>
      <stop offset="1" stop-color="#9fe9ff" stop-opacity="0"/>
    </linearGradient>
    <linearGradient id="au-line" x1="0" x2="1">
      <stop offset="0" stop-color="#fff" stop-opacity="0"/><stop offset="0.45" stop-color="#fff" stop-opacity="0.85"/>
      <stop offset="1" stop-color="#fff" stop-opacity="0"/>
    </linearGradient>
    <filter id="au-wide" x="-10%" y="-50%" width="120%" height="200%"><feGaussianBlur stdDeviation="28"/></filter>
    <filter id="au-soft" x="-10%" y="-50%" width="120%" height="200%"><feGaussianBlur stdDeviation="7"/></filter>
    <radialGradient id="au-flare"><stop offset="0" stop-color="#fff" stop-opacity="0.9"/><stop offset="1" stop-color="#bfefff" stop-opacity="0"/></radialGradient>
  </defs>
  <g transform="translate(0 -150)"><g>
    <path d="M0 640 C 320 470 620 780 940 610 S 1420 430 1600 520 L1600 640 C 1400 560 1200 760 920 730 S 320 620 0 760 Z" fill="url(#au-a)" filter="url(#au-wide)"/>
    <path d="M0 655 C 320 500 620 790 940 630 S 1420 460 1600 545" fill="none" stroke="url(#au-b)" stroke-width="26" filter="url(#au-soft)"/>
    <g fill="none" stroke="url(#au-line)">
      <path d="M0 650 C 320 495 620 785 940 622 S 1420 452 1600 538" stroke-width="1.6"/>
      <path d="M0 668 C 330 520 630 795 945 642 S 1420 478 1600 560" stroke-width="0.9" opacity="0.7"/>
      <path d="M0 686 C 340 545 640 805 950 662 S 1420 505 1600 582" stroke-width="0.6" opacity="0.5"/>
      <path d="M0 704 C 350 570 650 815 955 682 S 1420 532 1600 604" stroke-width="0.5" opacity="0.35"/>
    </g>
  </g></g>
  <g>
    <path d="M0 300 C 400 180 760 420 1100 300 S 1500 200 1600 230 L1600 290 C 1450 270 1300 380 1080 380 S 380 260 0 380 Z" fill="url(#au-b)" filter="url(#au-wide)" opacity="0.55"/>
    <g fill="none" stroke="url(#au-line)" opacity="0.6">
      <path d="M0 330 C 400 210 760 440 1100 330 S 1500 230 1600 255" stroke-width="1"/>
      <path d="M0 346 C 410 230 770 452 1105 348 S 1500 252 1600 275" stroke-width="0.5"/>
    </g>
  </g>
  <circle cx="1180" cy="410" r="70" fill="url(#au-flare)" opacity="0.35"/>
  <circle cx="1240" cy="440" r="12" fill="url(#au-flare)" opacity="0.6"/>
  <circle cx="300" cy="530" r="40" fill="url(#au-flare)" opacity="0.25"/>
</svg>`;

export function aurora() {
  const div = document.createElement("div");
  div.className = "aurora";
  div.setAttribute("aria-hidden", "true");
  div.innerHTML = AURORA;
  document.body.prepend(div);
}

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

// The licences of everything the site serves. `link` opens it.
export function licences(link, discs) {
  const dialog = document.createElement("dialog");
  dialog.className = "glass";
  dialog.setAttribute("aria-labelledby", "licences-title");
  const row = (what, licence) => `<tr><td>${what}</td><td>${licence}</td></tr>`;
  dialog.innerHTML = `
    <div class="bar"><h2 id="licences-title">Licences</h2><button class="close" aria-label="Close">✕</button></div>
    <div class="body">
      <h3>Software</h3>
      <table>
        ${row("This website", `<a href="https://blueoakcouncil.org/license/1.0.0" rel="noopener">Blue Oak Model License 1.0.0</a>`)}
        ${row("DVD player (WebAssembly)", "GPL-2.0-or-later")}
        ${row("&nbsp;&nbsp;libdvdnav-derived navigation, © Håkan Hjort, Rich Wareham", "GPL-2.0-or-later")}
        ${row("&nbsp;&nbsp;oxideav decoders, © Karpelès Lab Inc.", "MIT")}
        ${row("&nbsp;&nbsp;bytemuck, memchr, itoa, thiserror, zmij, serde_core, serde_json", "MIT")}
        ${row("Open Sans typeface", `<a href="https://openfontlicense.org" rel="noopener">SIL Open Font License 1.1</a>`)}
      </table>
      <p><a href="NOTICE.md">Full notices</a></p>
      <h3>Discs</h3>
      <table>
        ${discs.map((d) => row(`<a href="${esc(d.source)}" rel="noopener">${esc(d.title)}</a> <span class="faint">${esc(d.system)}</span>`, esc(d.license))).join("")}
      </table>
    </div>`;
  document.body.append(dialog);
  dialog.querySelector(".close").addEventListener("click", () => dialog.close());
  dialog.addEventListener("click", (e) => e.target === dialog && dialog.close()); // the backdrop
  link.addEventListener("click", (e) => {
    e.preventDefault();
    dialog.showModal();
  });
}
