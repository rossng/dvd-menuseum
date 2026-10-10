// A disc, drawn to a DVD's proportions in units of millimetres (120 mm
// across): a 15 mm hole, clear plastic with a stacking ring out to 33 mm,
// a mirror band to 42 mm, then the label (one side) or the data area (the
// other). The label is printed from the disc's catalog entry, in colours
// sampled from its menu; the data side's rainbow turns with --light.
//
// discMarkup(disc) -> HTML for a .disc with a .label and a .data face.

const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

const C = 60;          // centre
const HOLE = 7.5;
const STACK = 16.5;    // stacking ring
const MIRROR = 17.5;   // mirror band starts
const PRINT = 21;      // label print / data area starts
const RIM = 59.5;

// An annulus between two radii, as one path (with the even-odd rule).
const ring = (r0, r1) =>
  `M${C - r1} ${C}a${r1} ${r1} 0 1 0 ${2 * r1} 0a${r1} ${r1} 0 1 0 ${-2 * r1} 0z` +
  `M${C - r0} ${C}a${r0} ${r0} 0 1 0 ${2 * r0} 0a${r0} ${r0} 0 1 0 ${-2 * r0} 0z`;

// Split a title into at most two lines of similar length.
function lines(title) {
  if (title.length <= 14 || !title.includes(" ")) return [title];
  const words = title.split(" ");
  let best = null;
  for (let i = 1; i < words.length; i++) {
    const a = words.slice(0, i).join(" "), b = words.slice(i).join(" ");
    const worst = Math.max(a.length, b.length);
    if (!best || worst < best.worst) best = { lines: [a, b], worst };
  }
  return best.lines;
}

// The clear hub, the stacking ring and the mirror band, common to both faces.
function hub(id, codes) {
  return `
    <path d="${ring(HOLE, MIRROR)}" fill-rule="evenodd" fill="rgb(225 236 245 / 0.22)"/>
    <circle cx="${C}" cy="${C}" r="${HOLE + 0.2}" fill="none" stroke="rgb(255 255 255 / 0.55)" stroke-width="0.4"/>
    <circle cx="${C}" cy="${C}" r="${STACK}" fill="none" stroke="rgb(255 255 255 / 0.45)" stroke-width="0.9"/>
    <circle cx="${C}" cy="${C}" r="${STACK - 0.7}" fill="none" stroke="rgb(0 20 40 / 0.18)" stroke-width="0.3"/>
    <path d="${ring(MIRROR, PRINT)}" fill-rule="evenodd" fill="url(#${id}-mirror)"/>
    ${codes ? `<text class="codes"><textPath href="#${id}-codes" startOffset="0">${esc(codes)}</textPath></text>` : ""}`;
}

export function discMarkup(d) {
  const id = `d-${d.slug.replace(/[^a-z0-9-]/gi, "")}`;
  const [deep, light, vivid] = d.palette ?? ["#123a5c", "#2a5f88", "#5ad0ff"];
  const title = lines(d.title);
  // Fit the title to the chord of the print area at its height.
  const size = Math.min(title.length === 1 ? 11 : 8.5, ...title.map((l) => 84 / Math.max(l.length * 0.53, 1)));
  const top = title.length === 1 ? 34 : 26.5;
  const byline = [d.maker, d.year].filter(Boolean).join(" · ");
  const rim = [d.part, d.system, d.license].filter(Boolean).join("  ·  ");
  const codes = `${(d.provider || d.slug).toUpperCase()}  ${d.system ?? ""}  MENUSEUM  01`;
  const defs = `
    <linearGradient id="${id}-mirror" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#e3e9ef" stop-opacity="0.85"/><stop offset="0.45" stop-color="#b9c3cd" stop-opacity="0.75"/>
      <stop offset="0.55" stop-color="#dce3ea" stop-opacity="0.8"/><stop offset="1" stop-color="#aab5c1" stop-opacity="0.75"/>
    </linearGradient>
    <path id="${id}-codes" d="M${C - 19.3} ${C}a19.3 19.3 0 1 1 38.6 0a19.3 19.3 0 1 1 -38.6 0"/>`;
  return `
  <div class="disc">
    <div class="face label">
      <svg class="art" viewBox="0 0 120 120" aria-hidden="true">
        <defs>
          ${defs}
          <linearGradient id="${id}-print" x1="0.15" y1="1" x2="0.85" y2="0">
            <stop offset="0" stop-color="${deep}"/><stop offset="1" stop-color="${light}"/>
          </linearGradient>
          <radialGradient id="${id}-glow" cx="0.3" cy="0.15" r="0.75">
            <stop offset="0" stop-color="#fff" stop-opacity="0.35"/><stop offset="1" stop-color="#fff" stop-opacity="0"/>
          </radialGradient>
          <filter id="${id}-soft" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="2.2"/></filter>
          <clipPath id="${id}-clip"><path d="${ring(PRINT, RIM)}" clip-rule="evenodd"/></clipPath>
          <path id="${id}-rim" d="M${C - 53.5 * 0.94} ${C + 53.5 * 0.342}A53.5 53.5 0 0 0 ${C + 53.5 * 0.94} ${C + 53.5 * 0.342}"/>
        </defs>
        <g clip-path="url(#${id}-clip)">
          <rect width="120" height="120" fill="url(#${id}-print)"/>
          <g filter="url(#${id}-soft)" fill="none" stroke="${vivid}">
            <path d="M-10 92 C 25 60, 70 112, 130 58" stroke-width="13" opacity="0.45"/>
            <path d="M-10 104 C 35 78, 80 120, 130 76" stroke-width="7" opacity="0.35"/>
          </g>
          <g fill="none" stroke="#fff">
            <path d="M-10 87 C 25 55, 70 107, 130 53" stroke-width="0.5" opacity="0.7"/>
            <path d="M-10 100 C 35 74, 80 116, 130 72" stroke-width="0.35" opacity="0.5"/>
          </g>
          <rect width="120" height="120" fill="url(#${id}-glow)"/>
        </g>
        <path d="${ring(RIM - 0.4, RIM)}" fill-rule="evenodd" fill="rgb(255 255 255 / 0.35)"/>
        <text class="title" x="${C}" y="${top}" font-size="${size.toFixed(2)}">
          ${title.map((l, i) => `<tspan x="${C}" dy="${i ? (size * 1.08).toFixed(2) : 0}">${esc(l)}</tspan>`).join("")}
        </text>
        ${byline ? `<text class="byline" x="${C}" y="89">${esc(byline)}</text>` : ""}
        <text class="rim"><textPath href="#${id}-rim" startOffset="50%">${esc(rim)}</textPath></text>
        ${hub(id, "")}
      </svg>
      <div class="sheen"></div>
    </div>
    <div class="face data">
      <div class="rainbow"></div>
      <svg viewBox="0 0 120 120" aria-hidden="true"><defs>${defs}</defs>${hub(id, codes)}</svg>
    </div>
  </div>`;
}
