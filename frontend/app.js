"use strict";

const CFG = Object.assign({ apiBase: "api", pollSeconds: 15, title: "WoW Tracker" }, window.WOWTRACKER || {});
const SVG_NS = "http://www.w3.org/2000/svg";
const W = 1500;
const H = 1000;

// Standard class colours, keyed by the locale-independent class token.
const CLASS_COLORS = {
  WARRIOR: "#C69B6D", PALADIN: "#F48CBA", HUNTER: "#AAD372", ROGUE: "#FFF468", PRIEST: "#FFFFFF",
  SHAMAN: "#0070DD", MAGE: "#3FC7EB", WARLOCK: "#8788EE", DRUID: "#FF7C0A",
};
const CLASSIFICATION = { elite: "Elite", rare: "Rare", rareelite: "Rare Elite", worldboss: "Boss", trivial: "Trivial", minus: "Minor" };
const INSTANCE = { party: "dungeon", raid: "raid", pvp: "battleground", arena: "arena", scenario: "scenario" };

let maps = {};             // uiMapID -> { name, parent, type, instance, min:[x,y], max:[x,y] }
let state = null;          // last /api/state
let fetchedAt = 0;         // Date.now() of that response
let selectedKey = null;
let view = "continent";
const art = {};            // uiMapID -> image URL | null (probed once)

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

function h(tag, attrs, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k === "class") el.className = v;
    else if (k === "style") el.setAttribute("style", v);
    else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children.flat()) {
    if (c == null || c === false) continue;
    el.append(c instanceof Node ? c : String(c));
  }
  return el;
}

function s(tag, attrs, ...children) {
  const el = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs || {})) if (v != null) el.setAttribute(k, v);
  for (const c of children.flat()) if (c != null) el.append(c instanceof Node ? c : String(c));
  return el;
}

const fmt = (n) => (n == null ? "?" : Number(n).toLocaleString("en-US"));

function nowServer() {
  // Server time now, so "x minutes ago" does not depend on the viewer's clock.
  return state ? state.serverTime + (Date.now() - fetchedAt) / 1000 : Date.now() / 1000;
}

function ago(t) {
  if (!t) return "unknown";
  const d = Math.max(0, nowServer() - t);
  if (d < 60) return "just now";
  if (d < 3600) return `${Math.floor(d / 60)} min ago`;
  if (d < 86400) return `${Math.floor(d / 3600)} h ${Math.floor((d % 3600) / 60)} min ago`;
  return `${Math.floor(d / 86400)} days ago`;
}

function duration(sec) {
  if (sec == null || sec < 0) return "";
  if (sec < 60) return `${Math.round(sec)} s`;
  return `${Math.floor(sec / 60)} min ${Math.round(sec % 60)} s`;
}

// ---------------------------------------------------------------------------
// Map geometry (UiMapAssignment bounds; x is north-south, y is east-west)
// ---------------------------------------------------------------------------

function toWorld(m, x, y) {
  return [m.max[0] - y * (m.max[0] - m.min[0]), m.max[1] - x * (m.max[1] - m.min[1])];
}

function toFrame(frame, wx, wy) {
  return [
    ((frame.max[1] - wy) / (frame.max[1] - frame.min[1])) * W,
    ((frame.max[0] - wx) / (frame.max[0] - frame.min[0])) * H,
  ];
}

/** Position of (mapID, x, y) inside `frame`, or null if on another continent. */
function project(frame, mapID, x, y) {
  const m = maps[mapID];
  if (!m || !m.min || m.instance !== frame.instance) return null;
  const [wx, wy] = toWorld(m, x, y);
  return toFrame(frame, wx, wy);
}

function continentOf(id) {
  let m = maps[id];
  let cur = id;
  for (let i = 0; m && i < 6; i++) {
    if (m.type === 2) return cur;
    if (!maps[m.parent]) break;
    cur = m.parent;
    m = maps[cur];
  }
  return id; // islands like Zephras Isle have no continent map
}

function probeArt(id) {
  if (id in art) return Promise.resolve(art[id]);
  const exts = ["webp", "jpg", "png"];
  return new Promise((resolve) => {
    const tryNext = (i) => {
      if (i >= exts.length) { art[id] = null; return resolve(null); }
      const url = `maps/${id}.${exts[i]}`;
      const img = new Image();
      img.onload = () => { art[id] = url; resolve(url); };
      img.onerror = () => tryNext(i + 1);
      img.src = url;
    };
    tryNext(0);
  });
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

function selected() {
  if (!state || !state.characters.length) return null;
  return state.characters.find((c) => c.key === selectedKey) || state.characters[0];
}

async function renderMap(c) {
  const svg = document.getElementById("map");
  const caption = document.getElementById("map-caption");
  svg.replaceChildren();

  if (!c || !c.mapID || !maps[c.mapID] || !maps[c.mapID].min) {
    caption.textContent = c ? "No position recorded yet" : "";
    svg.append(s("text", { x: W / 2, y: H / 2, class: "zone", fill: "#9a9fa8", "font-size": 28, "text-anchor": "middle" },
      c ? "No position yet. Move around in game, then /wt sync." : "Waiting for data…"));
    return;
  }

  const frameId = view === "zone" ? c.mapID : continentOf(c.mapID);
  const frame = maps[frameId];
  caption.textContent = frame.name;

  const artUrl = await probeArt(frameId);
  if (selected() !== c) return; // selection changed while probing

  // Zone maps on the same continent. Their rectangles overlap on purpose (each
  // zone map shows some of its surroundings), so the schematic draws zone
  // names at their centres rather than boxes.
  const inside = (a, b) => a.x >= b.x && a.y >= b.y && a.x + a.w <= b.x + b.w && a.y + a.h <= b.y + b.h;
  const zones = Object.entries(maps)
    .filter(([, m]) => m.type === 3 && m.min && m.instance === frame.instance)
    .map(([id, m]) => {
      const [x1, y1] = toFrame(frame, m.max[0], m.max[1]);
      const [x2, y2] = toFrame(frame, m.min[0], m.min[1]);
      const z = { id: Number(id), m, x: Math.min(x1, x2), y: Math.min(y1, y2), w: Math.abs(x2 - x1), h: Math.abs(y2 - y1) };
      z.cx = z.x + z.w / 2;
      z.cy = z.y + z.h / 2;
      return z;
    });
  // A city (Stormwind, Orgrimmar...) is a map that sits entirely inside a zone map.
  for (const z of zones) z.city = zones.some((o) => o !== z && o.w * o.h > z.w * z.h && inside(z, o));

  // Trail: break the line on long gaps or when leaving this continent.
  const segments = [];
  let seg = [];
  let lastT = null;
  for (const p of c.trail || []) {
    const pt = project(frame, p.mapID, p.x, p.y);
    if (!pt || (lastT && p.t - lastT > 900)) {
      if (seg.length > 1) segments.push(seg);
      seg = [];
    }
    if (pt) seg.push(pt);
    lastT = p.t;
  }
  if (seg.length > 1) segments.push(seg);
  const pos = project(frame, c.mapID, c.x, c.y);

  // Visible area: the whole map when there is art (it covers exactly the
  // frame) or in zone view; for the continent schematic, just the land.
  let vb = { x: 0, y: 0, w: W, h: H };
  const layer = s("g");
  if (artUrl) {
    layer.append(s("image", { href: artUrl, x: 0, y: 0, width: W, height: H, preserveAspectRatio: "none" }));
  } else if (view === "zone") {
    layer.append(s("rect", { class: "zone-bg", x: 0, y: 0, width: W, height: H }));
    const grid = s("g", { class: "grid" });
    for (let i = 1; i < 10; i++) {
      grid.append(s("line", { x1: (i * W) / 10, y1: 0, x2: (i * W) / 10, y2: H }));
      grid.append(s("line", { x1: 0, y1: (i * H) / 10, x2: W, y2: (i * H) / 10 }));
      grid.append(s("text", { x: (i * W) / 10 + 6, y: 24 }, String(i * 10)));
      grid.append(s("text", { x: 8, y: (i * H) / 10 - 6 }, String(i * 10)));
    }
    layer.append(grid);
    const frameRect = { x: 0, y: 0, w: W, h: H };
    for (const z of zones) {
      if (z.id === frameId || !z.city || !inside(z, frameRect)) continue;
      layer.append(s("g", { class: `zone city${z.id === c.mapID ? " here" : ""}` },
        s("rect", { x: z.x, y: z.y, width: z.w, height: z.h, rx: 6 }),
        s("text", { x: z.cx, y: z.cy }, z.m.name)));
    }
    layer.append(s("text", { class: "watermark", x: W - 30, y: H - 30 }, frame.name));
  } else {
    const pts = zones.filter((z) => !z.city).map((z) => [z.cx, z.cy]).concat(segments.flat(), pos ? [pos] : []);
    if (pts.length) {
      const xs = pts.map((p) => p[0]);
      const ys = pts.map((p) => p[1]);
      const pad = 130;
      vb = { x: Math.min(...xs) - pad, y: Math.min(...ys) - pad / 2, w: 0, h: 0 };
      vb.w = Math.max(...xs) + pad - vb.x;
      vb.h = Math.max(...ys) + pad / 2 - vb.y;
    }
    const cur = zones.find((z) => z.id === c.mapID);
    if (cur) layer.append(s("rect", { class: "zone-outline", x: cur.x, y: cur.y, width: cur.w, height: cur.h, rx: 8 }));
    for (const z of zones) {
      const here = z.id === c.mapID;
      layer.append(s("g", { class: `zone-label${z.city ? " city" : ""}${here ? " here" : ""}` },
        s("circle", { cx: z.cx, cy: z.cy, r: z.city ? 4 : 5 }),
        s("text", { x: z.cx, y: z.cy + (z.city ? 20 : 26) }, z.m.name),
        s("title", null, z.m.name)));
    }
  }
  svg.append(layer);
  svg.setAttribute("viewBox", `${vb.x.toFixed(0)} ${vb.y.toFixed(0)} ${vb.w.toFixed(0)} ${vb.h.toFixed(0)}`);
  svg.style.aspectRatio = `${vb.w} / ${vb.h}`;
  const k = Math.max(vb.w / W, vb.h / H); // keep markers the same on-screen size

  for (const pts of segments) {
    svg.append(s("polyline", { class: "trail", "stroke-width": 3.5 * k, points: pts.map((p) => p.map((v) => v.toFixed(1)).join(",")).join(" ") }));
    svg.append(s("circle", { class: "trail-start", cx: pts[0][0], cy: pts[0][1], r: 5 * k }));
  }

  if (pos) {
    const color = CLASS_COLORS[c.classFile] || "#e8b64c";
    const stale = c.hasPosition === false;
    svg.append(s("g", { class: `marker${c.online ? " online" : ""}${stale ? " stale" : ""}` },
      s("circle", { class: "ring", cx: pos[0], cy: pos[1], r: 16 * k, stroke: color }),
      s("circle", { class: "dot", cx: pos[0], cy: pos[1], r: 11 * k, fill: color, "stroke-width": 4 * k }),
      s("title", null, stale ? `${c.name}: last known position` : c.name)));
    const right = pos[0] < vb.x + vb.w - 260 * k;
    svg.append(s("text", {
      class: "marker-label", x: pos[0] + (right ? 22 : -22) * k, y: pos[1] + 8 * k,
      "text-anchor": right ? "start" : "end", "font-size": 24 * k, "stroke-width": 5 * k,
    }, c.name));
  }
}

function unitBlock(u, heading, extra) {
  const name = u.name || (u.isPlayer ? "Unknown player" : "Unknown (hidden in combat)");
  const level = u.level == null ? "" : u.level === -1 ? "Level ??" : `Level ${u.level}`;
  const cls = CLASSIFICATION[u.classification];
  const reaction = u.reaction == null ? "" : u.reaction <= 3 ? "hostile" : u.reaction >= 5 ? "friendly" : "";
  const bits = [level, cls, u.creatureType, u.dead ? "dead" : null].filter(Boolean).join(" · ");
  return h("div", { class: "block" },
    h("h3", null, heading),
    h("p", { class: reaction }, name, cls && cls !== "Trivial" && cls !== "Minor" ? h("span", { class: "elite" }, " ◆") : null),
    bits ? h("p", { class: "sub" }, bits) : null,
    u.hpPct != null ? h("div", { class: "bar", title: `${u.hpPct}% health` }, h("span", { class: "hp", style: `width:${Math.max(0, Math.min(100, u.hpPct))}%` })) : null,
    extra || null);
}

function renderPanel(c) {
  const panel = document.getElementById("panel");
  if (!state) return;
  if (!c) {
    panel.replaceChildren(h("div", { class: "empty" },
      h("p", null, "No characters yet."),
      h("p", null, "Install the WowTracker addon, set ", h("code", null, "/wt endpoint <ip:port>"),
        ", start the companion, then type ", h("code", null, "/wt sync"), " in game.")));
    return;
  }

  const color = CLASS_COLORS[c.classFile] || "var(--text)";
  const desc = [c.level != null ? `Level ${c.level}` : null, c.race, c.class].filter(Boolean).join(" ");
  const status = c.online
    ? h("span", { class: "chip online" }, "Online")
    : h("span", { class: "chip" }, c.status === "offline" ? `Logged out ${ago(c.savedAt)}` : `Offline · seen ${ago(c.savedAt)}`);
  const combat = c.online && c.inCombat ? h("span", { class: "chip combat" }, "In combat") : null;

  const who = h("div", { class: "who" },
    h("h2", { style: `color:${color}` }, c.name),
    h("p", null, [desc, c.realm].filter(Boolean).join(" · ")),
    h("div", null, status, combat));

  // XP
  let xp;
  if (c.xpMax) {
    const pct = (c.xp / c.xpMax) * 100;
    const restedEnd = Math.min(100, ((c.xp + (c.rested || 0)) / c.xpMax) * 100);
    xp = h("div", { class: "block" },
      h("h3", null, "Experience"),
      h("div", { class: "bar", title: `${pct.toFixed(1)}%` },
        h("span", { class: "rested", style: `width:${restedEnd}%` }),
        h("span", { class: "xp", style: `width:${pct}%` })),
      h("p", { class: "sub" }, `${fmt(c.xp)} / ${fmt(c.xpMax)} XP (${pct.toFixed(1)}%)`,
        c.rested ? ` · ${fmt(c.rested)} rested` : "", c.resting ? " · resting" : ""));
  } else if (c.level != null) {
    xp = h("div", { class: "block" }, h("h3", null, "Experience"), h("p", null, "Max level"));
  }

  // Location
  const inInstance = c.instance && c.instance !== "none";
  const coords = c.x != null && c.hasPosition !== false ? `${(c.x * 100).toFixed(1)}, ${(c.y * 100).toFixed(1)}` : null;
  const lastMap = maps[c.mapID];
  const where = h("div", { class: "block" },
    h("h3", null, "Location"),
    h("p", null, c.zone || "Unknown"),
    h("p", { class: "sub" }, [c.subzone, coords].filter(Boolean).join(" · ") || " "),
    inInstance ? h("p", { class: "sub" }, `In a ${INSTANCE[c.instance] || "instance"}. The map shows the last outdoor position${lastMap ? ` (${lastMap.name})` : ""}.`) : null);

  // Target / fight
  let fight = null;
  if (c.online && c.inCombat) {
    fight = c.target ? unitBlock(c.target, "Fighting") : h("div", { class: "block" }, h("h3", null, "Fighting"), h("p", null, "Something (no target)"));
  } else if (c.target && c.online) {
    fight = unitBlock(c.target, "Target");
  }
  let last = null;
  const lf = c.lastFight;
  if (lf && lf.startedAt && !(c.online && c.inCombat)) {
    const when = h("p", { class: "sub" }, `${ago(lf.endedAt || lf.startedAt)}${lf.endedAt ? ` · lasted ${duration(lf.endedAt - lf.startedAt)}` : ""}`);
    last = lf.target ? unitBlock(lf.target, "Last fight", when) : h("div", { class: "block" }, h("h3", null, "Last fight"), h("p", null, "Unknown opponent"), when);
  }

  // Session
  const session = h("div", { class: "block" },
    h("h3", null, "Session"),
    h("dl", { class: "kv" },
      h("dt", null, "Logged in"), h("dd", null, c.loginAt ? ago(c.loginAt) : "?"),
      h("dt", null, "XP this session"), h("dd", null, fmt(c.xpGainedSession || 0)),
      h("dt", null, "Data saved"), h("dd", null, ago(c.savedAt))));

  panel.replaceChildren(...[who, xp, where, fight, last, session].filter(Boolean));
}

function renderChars() {
  const nav = document.getElementById("chars");
  const chars = state ? state.characters : [];
  if (chars.length < 2) { nav.replaceChildren(); return; }
  const cur = selected();
  nav.replaceChildren(...chars.map((c) => h("button", {
    type: "button",
    "aria-pressed": String(c === cur),
    onclick: () => { selectedKey = c.key; render(); },
  }, c.online ? `● ${c.name}` : c.name)));
}

function renderFooter() {
  const el = document.getElementById("updated");
  if (!state) { el.textContent = ""; return; }
  const c = selected();
  el.textContent = c
    ? `Updates when you /reload, /wt sync or log out in game. Last save ${ago(c.savedAt)}.`
    : "";
}

function render() {
  const c = selected();
  renderChars();
  renderPanel(c);
  renderMap(c);
  renderFooter();
}

function banner(msg) {
  const b = document.getElementById("banner");
  b.hidden = !msg;
  b.textContent = msg || "";
}

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

async function poll() {
  try {
    const base = CFG.apiBase.replace(/\/+$/, "");
    const res = await fetch(`${base}/state`, { cache: "no-store" });
    if (!res.ok) throw new Error(`API answered ${res.status}`);
    const next = await res.json();
    const changed = JSON.stringify(next.characters) !== JSON.stringify(state && state.characters);
    state = next;
    fetchedAt = Date.now();
    banner(null);
    if (changed) render(); else { renderPanel(selected()); renderFooter(); }
  } catch (e) {
    banner(`Cannot reach the tracker API (${e.message}). Showing the last data received.`);
  }
}

async function start() {
  document.title = CFG.title;
  document.getElementById("title").textContent = CFG.title;

  for (const btn of document.querySelectorAll(".seg button")) {
    btn.addEventListener("click", () => {
      view = btn.dataset.view;
      for (const b of document.querySelectorAll(".seg button")) b.setAttribute("aria-pressed", String(b === btn));
      renderMap(selected());
    });
  }

  try {
    maps = (await (await fetch("data/maps.json")).json()).maps;
  } catch {
    banner("Could not load data/maps.json.");
  }
  await poll();
  setInterval(poll, Math.max(5, CFG.pollSeconds) * 1000);
}

start();
