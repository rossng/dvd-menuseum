// The player page's behaviour (markup in components/Player.astro): starts the
// player on the page's bundle, or, on the local page, on the files of a folder
// the viewer chooses or drops on the screen, and wires up the remote.

import { createPlayer, KEYS } from "./player.js";
import { discMarkup } from "../lib/disc.js";
import { light } from "../lib/light.js";

const $ = (id) => document.getElementById(id);
const page = $("player").dataset;
const local = "local" in page;
const disc = { slug: "local", title: page.title }; // the slug only labels a local disc
const stopLight = light($("insert"));
$("insert").querySelector(".disc").classList.add("spinning");

// What the status display calls each domain of the DVD virtual machine.
const DOMAINS = { 1: "First play", 2: "Title", 4: "Main menu", 8: "Title set menu" };
const hex = (a) => a.map((v) => v.toString(16).padStart(4, "0")).join(" ");
const log = (msg) => {
  const el = $("log");
  el.textContent += msg + "\n";
  if (el.textContent.length > 20000) el.textContent = el.textContent.slice(-15000);
  el.scrollTop = el.scrollHeight;
};

let player = null;
let ready = false;
let started = false;
let aspect = null; // widescreen, as the player last said
const load = (source) => (player = createPlayer(
  { video: $("video"), overlay: $("overlay") },
  source,
  {
    ready: () => {
      ready = true;
      $("insert").classList.remove("loading");
      $("cta").textContent = "Play";
    },
    log,
    error: (message) => {
      if (local && !ready) {
        $("cta").textContent = `Couldn't read that disc (${message.replace(/^Error: /, "")}). Choose another folder.`;
        player?.terminate();
        player = null;
        $("insert").classList.remove("loading");
      } else if (/HTTP|fetch/i.test(message)) $("where").textContent = "Couldn't read the disc";
    },
    stats: (text) => ($("stats").textContent = text),
    // Until playback starts the screen keeps the shape the page was built
    // with (the first picture's), rather than following the player as it
    // reads ahead.
    widescreen: (wide) => {
      aspect = wide;
      if (started) $("screen").classList.toggle("wide", wide);
    },
    placeholder: (text) => {
      $("notice").style.display = text ? "grid" : "none";
      if (text) $("notice").textContent = text;
    },
    state: (st) => {
      const vts = st.domain === 8 || st.domain === 2 ? ` ${st.vts}` : "";
      $("where").textContent = `${disc.title} — ${DOMAINS[st.domain] ?? "?"}${vts}`;
      $("btn").textContent = st.button ? `Button ${st.button}` : "";
      $("state").innerHTML =
        `domain ${DOMAINS[st.domain]} · vts ${st.vts} · pgc ${st.pgc} · pg ${st.pg} · cell ${st.cell} · button ${st.button}<br>` +
        `SPRM ${hex(st.sprm)}<br>GPRM ${hex(st.gprm)}`;
    },
  },
));

if (!local) load({ bundle: page.bundle });

// A local disc: the files of a chosen or dropped folder, with their paths.
// The disc takes the folder's name.
function insertLocal(files) {
  if (!files.length) return;
  const top = files[0].path.includes("/") ? files[0].path.split("/")[0] : "those files";
  const named = /^(VIDEO_TS|those files)$/i.test(top) ? null : top;
  if (!files.some((f) => /(^|\/)VIDEO_TS\.IFO$/i.test(f.path))) {
    $("cta").textContent = `There’s no VIDEO_TS.IFO in “${top}”. Choose a DVD’s folder, or its VIDEO_TS folder.`;
    return;
  }
  disc.title = named ? named.replace(/[._]+/g, " ").trim() : "Your DVD";
  document.title = `${disc.title} · Menuseum`;
  $("insert-disc").innerHTML = discMarkup(disc);
  $("insert").querySelector(".disc").classList.add("spinning");
  $("info").querySelector("h2").textContent = disc.title;
  $("where").textContent = disc.title;
  $("insert").classList.add("loading");
  $("insert").setAttribute("aria-label", "Play");
  $("cta").textContent = "Loading…";
  load({ files });
}

// Every file under the dropped entries.
async function walk(entries) {
  const out = [];
  const visit = async (e) => {
    if (e.isFile) out.push({ path: e.fullPath.replace(/^\//, ""), file: await new Promise((ok, fail) => e.file(ok, fail)) });
    else if (e.isDirectory) {
      const reader = e.createReader();
      for (let batch; (batch = await new Promise((ok, fail) => reader.readEntries(ok, fail))).length; ) {
        for (const child of batch) await visit(child);
      }
    }
  };
  for (const e of entries) await visit(e);
  return out;
}

let picker = null;
if (local) {
  picker = Object.assign(document.createElement("input"), { type: "file", webkitdirectory: true, hidden: true });
  picker.addEventListener("change", () => {
    insertLocal([...picker.files].map((file) => ({ path: file.webkitRelativePath || file.name, file })));
    picker.value = "";
  });
  document.body.append(picker);

  const screen = $("insert");
  screen.addEventListener("dragover", (e) => {
    if (player) return;
    e.preventDefault();
    screen.classList.add("dragging");
  });
  screen.addEventListener("dragleave", () => screen.classList.remove("dragging"));
  screen.addEventListener("drop", async (e) => {
    e.preventDefault();
    screen.classList.remove("dragging");
    if (player) return;
    const entries = [...e.dataTransfer.items].map((i) => i.webkitGetAsEntry?.()).filter(Boolean);
    insertLocal(await walk(entries));
  });
}

$("insert").addEventListener("click", () => {
  if (local && !player) return picker.click();
  if ($("insert").classList.contains("loading") || started) return;
  started = true;
  // Leave the main thread to the player: nothing else moves while it runs.
  stopLight();
  document.querySelector(".aurora").classList.add("still");
  player.start();
  if (aspect !== null) $("screen").classList.toggle("wide", aspect);
  $("insert").classList.add("going");
  setTimeout(() => $("insert").remove(), 600);
});

// Before playback starts, OK (or Enter) presses play.
const send = (kind, a) => {
  if (!started) return kind === "activate" && $("insert").click();
  player.input(kind, a);
};
for (const b of document.querySelectorAll("[data-input]")) {
  const [kind, a] = b.dataset.input.split(" ");
  b.addEventListener("click", () => send(kind, a));
}
addEventListener("keydown", (e) => {
  if (document.querySelector("dialog[open]") || e.target.closest?.("a, summary, input")) return;
  if (e.target.closest?.("button") && (e.key === "Enter" || e.key === " ")) return; // the button's own click
  const k = KEYS[e.key];
  if (k) {
    send(...k);
    e.preventDefault();
  }
});
$("overlay").addEventListener("mousemove", (e) => started && player.input("pointer", ...player.pictureCoords(e)));
$("overlay").addEventListener("click", (e) => started && player.input("click", ...player.pictureCoords(e)));
$("rects").addEventListener("click", () => {
  const on = $("rects").getAttribute("aria-pressed") !== "true";
  $("rects").setAttribute("aria-pressed", on);
  player.showRects(on);
});
