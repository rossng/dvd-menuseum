// The player page's behaviour (markup in components/Player.astro): starts the
// player on the page's bundle, or, on the local page, on the files of a folder
// the viewer chooses or drops on the screen, and wires up the remote.
//
// For testing, any player page takes ?bundle=<url> (play that bundle
// instead), ?seed=<n> (seed the VM's random numbers) and ?compat=1 (play with
// libdvdnav's quirks); any of them opens the Details panel.

import { createPlayer, KEYS } from "./player.js";
import { discMarkup } from "../lib/disc.js";
import { light } from "../lib/light.js";

const $ = (id) => document.getElementById(id);
const page = $("player").dataset;
const local = "local" in page;
const disc = { slug: "local", title: page.title }; // the slug only labels a local disc
const params = new URLSearchParams(location.search);
const testing = ["bundle", "seed", "compat"].some((k) => params.has(k));
const options = { seed: Number(params.get("seed") ?? 0), compat: params.get("compat") === "1" };
const stopLight = light($("insert"));

// What the status display calls each domain of the DVD virtual machine.
const DOMAINS = { 1: "First play", 2: "Title", 4: "Main menu", 8: "Title set menu" };
const hex = (a) => a.map((v) => v.toString(16).padStart(4, "0")).join(" ");
const log = (msg) => {
  const el = $("log");
  el.textContent += msg + "\n";
  if (el.textContent.length > 20000) el.textContent = el.textContent.slice(-15000);
  el.scrollTop = el.scrollHeight;
};

// The page's phase:
//   empty    the local page, until a disc is chosen: the screen picks one
//   loading  the disc is opening
//   ready    it's open: the screen (or OK) plays it
//   playing  the remote drives the player
// A local disc that won't open goes back to empty, saying why.
const PROMPTS = {
  empty: "Choose a DVD’s folder (the one holding VIDEO_TS, or VIDEO_TS itself), or drop it here",
  loading: "Loading…",
  ready: "Play",
};
let phase = null;
let player = null;
let aspect = null; // widescreen, as the player last said

function enter(next, prompt = PROMPTS[next]) {
  phase = next;
  const insert = $("insert");
  if (next === "playing") {
    // Leave the main thread to the player: nothing else moves while it runs.
    stopLight();
    document.querySelector(".aurora").classList.add("still");
    if (aspect !== null) $("screen").classList.toggle("wide", aspect);
    insert.classList.add("going");
    setTimeout(() => insert.remove(), 600);
    return;
  }
  insert.classList.toggle("loading", next === "loading");
  insert.setAttribute("aria-label", next === "empty" ? "Choose a DVD" : "Play");
  insert.querySelector(".disc").classList.add("spinning");
  $("cta").textContent = prompt;
}

const load = (source) => {
  enter("loading");
  player = createPlayer({ video: $("video"), overlay: $("overlay") }, { ...source, ...options }, {
    ready: (provider) => {
      if (testing) log(`opened: ${provider || "untitled disc"}, seed ${options.seed}${options.compat ? ", libdvdnav quirks" : ""}`);
      enter("ready");
    },
    log,
    error: (message) => {
      if (local && phase === "loading" && source.files) {
        player.terminate();
        player = null;
        enter("empty", `Couldn't read that disc (${message.replace(/^Error: /, "")}). Choose another folder.`);
      } else if (/HTTP|fetch/i.test(message)) $("where").textContent = "Couldn't read the disc";
    },
    stats: (text) => ($("stats").textContent = text),
    // Until playback starts the screen keeps the shape the page was built
    // with (the first picture's), rather than following the player as it
    // reads ahead.
    widescreen: (wide) => {
      aspect = wide;
      if (phase === "playing") $("screen").classList.toggle("wide", wide);
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
  });
  player.showRects($("rects").getAttribute("aria-pressed") === "true");
};

// A local disc takes the name of its folder (or bundle).
function rename(name) {
  disc.title = name && !/^VIDEO_TS$/i.test(name) ? name.replace(/[._]+/g, " ").trim() : "Your DVD";
  document.title = `${disc.title} · Menuseum`;
  $("insert-disc").innerHTML = discMarkup(disc);
  $("info").querySelector("h2").textContent = disc.title;
  $("where").textContent = disc.title;
}

// The files of a chosen or dropped folder, with their paths.
function insertLocal(files) {
  if (!files.length) return;
  const top = files[0].path.includes("/") ? files[0].path.split("/")[0] : null;
  if (!files.some((f) => /(^|\/)VIDEO_TS\.IFO$/i.test(f.path))) {
    return enter("empty", `There’s no VIDEO_TS.IFO in “${top ?? "those files"}”. Choose a DVD’s folder, or its VIDEO_TS folder.`);
  }
  rename(top);
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

const picker = Object.assign(document.createElement("input"), { type: "file", webkitdirectory: true, hidden: true });
if (local) {
  picker.addEventListener("change", () => {
    insertLocal([...picker.files].map((file) => ({ path: file.webkitRelativePath || file.name, file })));
    picker.value = "";
  });
  document.body.append(picker);

  const screen = $("insert");
  screen.addEventListener("dragover", (e) => {
    if (phase !== "empty") return;
    e.preventDefault();
    screen.classList.add("dragging");
  });
  screen.addEventListener("dragleave", () => screen.classList.remove("dragging"));
  screen.addEventListener("drop", async (e) => {
    e.preventDefault();
    screen.classList.remove("dragging");
    if (phase !== "empty") return;
    const entries = [...e.dataTransfer.items].map((i) => i.webkitGetAsEntry?.()).filter(Boolean);
    insertLocal(await walk(entries));
  });
}

$("insert").addEventListener("click", () => {
  if (phase === "empty") picker.click();
  else if (phase === "ready") {
    player.start();
    enter("playing");
  }
});

// Before playback starts, OK (or Enter) does what the screen does.
const send = (kind, a) => {
  if (phase !== "playing") return kind === "activate" && $("insert").click();
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
$("overlay").addEventListener("mousemove", (e) => phase === "playing" && player.input("pointer", ...player.pictureCoords(e)));
$("overlay").addEventListener("click", (e) => phase === "playing" && player.input("click", ...player.pictureCoords(e)));
$("rects").addEventListener("click", () => {
  const on = $("rects").getAttribute("aria-pressed") !== "true";
  $("rects").setAttribute("aria-pressed", on);
  player?.showRects(on);
});

if (testing) document.querySelector("details.more").open = true;
if (params.has("bundle")) {
  if (local) rename(params.get("bundle").split("/").filter(Boolean).pop());
  load({ bundle: params.get("bundle") });
} else if (local) enter("empty");
else load({ bundle: page.bundle });
