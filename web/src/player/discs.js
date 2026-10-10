// A disc for the player (abi.js load()), from the files of its VIDEO_TS:
//   {provider, ifos: [{name, read()}], sets: [{id, sectors, read(start, end)}]}
// A menu bundle on the web is the same: its files, read with HTTP range
// requests. Shared by player-worker.js and scripts/wasm-smoke.mjs.

// Bytes [start, end) of files laid end to end, as a title VOB set's are.
function spanning(parts) {
  return async (start, end) => {
    const out = new Uint8Array(end - start);
    let at = 0;
    for (const f of parts) {
      const a = Math.max(start, at), b = Math.min(end, at + f.size);
      if (a < b) out.set(new Uint8Array(await f.slice(a - at, b - at).arrayBuffer()), a - start);
      at += f.size;
    }
    return out;
  };
}

// A disc from files: [{path, file}], where a file is a File or Blob (or
// anything with its size, slice() and arrayBuffer()). Every IFO (or its
// backup) and every VOB set, the film included if it's there.
export function discFromFiles(list, provider = null) {
  const byName = new Map();
  for (const { path, file } of list) {
    const name = path.split("/").pop().toUpperCase();
    // Prefer a VIDEO_TS directory's copy over a stray one beside it.
    if (!byName.has(name) || /(^|\/)VIDEO_TS\//i.test(path)) byName.set(name, file);
  }
  const ifos = [];
  const sets = [];
  const add = (id, parts) => {
    if (parts.length) sets.push({ id, sectors: Math.floor(parts.reduce((n, f) => n + f.size, 0) / 2048), read: spanning(parts) });
  };
  const stems = ["VIDEO_TS", ...[...byName.keys()].filter((n) => /^VTS_\d\d_0\.(IFO|BUP)$/.test(n)).map((n) => n.slice(0, 8))];
  for (const stem of new Set(stems)) {
    const ifo = byName.get(`${stem}.IFO`) ?? byName.get(`${stem}.BUP`);
    if (ifo) ifos.push({ name: `${stem}.IFO`, read: async () => new Uint8Array(await ifo.arrayBuffer()) });
    const menu = byName.get(`${stem}.VOB`);
    if (stem === "VIDEO_TS") {
      add("vmgm", menu ? [menu] : []);
      continue;
    }
    const n = parseInt(stem.slice(4, 6), 10);
    add(`menu-${n}`, menu ? [menu] : []);
    const title = [];
    for (let i = 1; i <= 9 && byName.has(`VTS_${stem.slice(4, 6)}_${i}.VOB`); i++) title.push(byName.get(`VTS_${stem.slice(4, 6)}_${i}.VOB`));
    add(`title-${n}`, title);
  }
  if (!ifos.some((f) => f.name === "VIDEO_TS.IFO")) throw new Error("no VIDEO_TS.IFO among those files");
  return { provider, ifos, sets };
}

// Bytes [start, end) of a file on the web, or all of it.
async function fetchBytes(url, range) {
  const r = await fetch(url, range ? { headers: { Range: `bytes=${range[0]}-${range[1] - 1}` } } : {});
  if (!r.ok) throw new Error(`${url.pathname.split("/").pop()}: HTTP ${r.status}`);
  const buf = await r.arrayBuffer();
  return range && r.status === 200 ? buf.slice(range[0], range[1]) : buf; // the server ignored Range
}

// As much of a File as discFromFiles uses, for a file on the web.
const remoteFile = (url, size, range = [0, size]) => ({
  size: range[1] - range[0],
  slice: (a, b) => remoteFile(url, size, [range[0] + a, range[0] + b]),
  arrayBuffer: () => fetchBytes(url, range[0] === 0 && range[1] === size ? null : range),
});

// A menu bundle at `url`: the files its manifest (bundle.json) lists.
export async function bundleDisc(url) {
  const base = new URL(url);
  const r = await fetch(new URL("bundle.json", base));
  if (!r.ok) throw new Error(`bundle.json: HTTP ${r.status}`);
  const manifest = await r.json();
  return discFromFiles(manifest.files.map((f) => ({ path: f.name, file: remoteFile(new URL(f.name, base), f.size) })), manifest.provider);
}
