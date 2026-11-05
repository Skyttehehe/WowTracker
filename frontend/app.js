"use strict";

const CFG = Object.assign({ apiBase: "api", pollSeconds: 15, title: "WoW Tracker", locale: "da-DK", maxLevel: 60 }, window.WOWTRACKER || {});
const SVG_NS = "http://www.w3.org/2000/svg";
const W = 1500;
const H = 1000;

const POWER = {
  0: { name: "Mana", cls: "mana" },
  1: { name: "Rage", cls: "rage", scale: 10 },
  3: { name: "Energy", cls: "energy" },
  6: { name: "Runic Power", cls: "rage", scale: 10 },
};

let maps = {};             // uiMapID -> { name, parent, type, instance, min:[x,y], max:[x,y] }
let state = null;          // last /api/state
let fetchedAt = 0;         // Date.now() of that response
let selectedKey = null;
let journalFilter = "all";
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

const num = (n, digits = 0) =>
  n == null || !isFinite(n) ? "–" : Number(n).toLocaleString(CFG.locale, { maximumFractionDigits: digits, minimumFractionDigits: digits });
const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

function nowServer() {
  // Server time now, so "x minutes ago" does not depend on the viewer's clock.
  return state ? state.serverTime + (Date.now() - fetchedAt) / 1000 : Date.now() / 1000;
}

function ago(t) {
  if (!t) return "never";
  const d = Math.max(0, nowServer() - t);
  if (d < 60) return "just now";
  if (d < 3600) return `${Math.floor(d / 60)} min ago`;
  if (d < 86400) return `${Math.floor(d / 3600)} h ago`;
  const days = Math.floor(d / 86400);
  return days === 1 ? "1 day ago" : `${days} days ago`;
}

function hm(sec) {
  if (sec == null || !isFinite(sec)) return "–";
  const m = Math.max(0, Math.round(sec / 60));
  if (m < 60) return `${m}m`;
  return m % 60 ? `${Math.floor(m / 60)}h ${m % 60}m` : `${Math.floor(m / 60)}h`;
}

function clock(t) {
  const d = new Date(t * 1000);
  const time = d.toLocaleTimeString(CFG.locale, { hour: "2-digit", minute: "2-digit" });
  const today = new Date(nowServer() * 1000).toDateString() === d.toDateString();
  return today ? time : `${d.toLocaleDateString(CFG.locale, { day: "numeric", month: "numeric" })} ${time}`;
}

function money(copper, asText) {
  if (copper == null) return asText ? "" : "–";
  const g = Math.floor(copper / 10000);
  const sv = Math.floor((copper % 10000) / 100);
  const c = copper % 100;
  if (asText) return [g && `${g}g`, sv && `${sv}s`, c && `${c}c`].filter(Boolean).join(" ");
  const parts = [];
  if (g) parts.push(h("span", { class: "coin g" }, num(g)));
  if (g || sv) parts.push(h("span", { class: "coin s" }, sv));
  parts.push(h("span", { class: "coin c" }, c));
  return h("span", null, parts);
}

// ---------------------------------------------------------------------------
// Derived figures
// ---------------------------------------------------------------------------

/** Played seconds now: the saved figure plus time since, while online. */
function playedNow(c) {
  if (c.played == null) return null;
  return c.played + (c.online && c.savedAt ? Math.max(0, nowServer() - c.savedAt) : 0);
}

/** XP per played hour over the last played hour (or less, if that is all there is). */
function xpPerHour(c) {
  const samples = c.xpSamples || [];
  if (samples.length < 2) return null;
  const end = samples[samples.length - 1].played;
  let gained = 0;
  let from = end;
  for (let i = samples.length - 1; i > 0; i--) {
    const a = samples[i - 1];
    const b = samples[i];
    if (a.played < end - 3600) break;
    gained += a.level === b.level ? Math.max(0, b.xp - a.xp) : Math.max(0, a.xpMax - a.xp) + b.xp;
    from = a.played;
  }
  const span = end - from;
  return span >= 300 && gained > 0 ? (gained / span) * 3600 : null;
}

/** Health and power: the newer of the combat log and the addon's save. */
function vitals(c) {
  // The game hides current health from addons but not the maximum.
  const live = c.live && c.live.hp != null && c.live.hpMax ? c.live : null;
  const saved = c.hp != null && c.hpMax ? c : null;
  if (live && (!saved || live.t >= (c.savedAt || 0))) return { v: live, t: live.t, source: "log" };
  if (saved) return { v: saved, t: c.savedAt, source: "addon" };
  return null;
}

function lastUpdate(c) {
  return Math.max(c.savedAt || 0, (c.live && c.live.t) || 0) || null;
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
// Header
// ---------------------------------------------------------------------------

function selected() {
  if (!state || !state.characters.length) return null;
  return state.characters.find((c) => c.key === selectedKey) || state.characters[0];
}

function renderHeader(c) {
  const $ = (id) => document.getElementById(id);
  if (!c) {
    $("avatar").textContent = "?";
    $("realm").textContent = "";
    $("name").textContent = CFG.title;
    $("desc").textContent = state ? "No characters yet" : "Loading…";
    $("pill").className = "pill";
    $("pill").lastChild.textContent = "Offline";
    $("seen").textContent = state && state.companionSeenAt ? `Companion seen ${ago(state.companionSeenAt)}` : "";
    return;
  }
  document.title = `${c.name} · ${CFG.title}`;
  $("avatar").textContent = (c.name || "?").slice(0, 1).toUpperCase();
  $("realm").textContent = [c.realm, c.faction].filter(Boolean).join(" · ");
  $("name").textContent = c.name;
  $("desc").textContent = [c.level != null ? `Level ${c.level}` : null, c.race, c.class].filter(Boolean).join(" ");
  $("pill").className = `pill${c.online ? " online" : ""}`;
  $("pill").lastChild.textContent = c.online ? "Online" : "Offline";
  $("seen").textContent = `Last update ${ago(lastUpdate(c))} · seen ${ago(state.companionSeenAt)}`;

  const nav = $("chars");
  const chars = state.characters;
  nav.replaceChildren(...(chars.length < 2 ? [] : chars.map((o) => h("button", {
    type: "button",
    "aria-pressed": String(o === c),
    onclick: () => { selectedKey = o.key; render(); },
  }, o.online ? `● ${o.name}` : o.name))));
}

// ---------------------------------------------------------------------------
// Progress and vitals
// ---------------------------------------------------------------------------

function renderProgress(c) {
  const el = document.getElementById("progress");
  if (!c || c.level == null) {
    el.replaceChildren(h("div", { class: "card-head" }, h("h2", null, "Progress")),
      h("p", { class: "empty-note" }, "Install the addon and type /wt sync in game."));
    return;
  }
  const max = CFG.maxLevel;
  const toGo = Math.max(0, max - c.level);
  const pct = c.xpMax ? (c.xp / c.xpMax) * 100 : 100;
  const restedEnd = c.xpMax ? clamp(((c.xp + (c.rested || 0)) / c.xpMax) * 100, 0, 100) : 0;
  const rate = xpPerHour(c);
  const eta = rate && c.xpMax ? ((c.xpMax - c.xp) / rate) * 3600 : null;

  // The journey to max level: one step per level.
  const steps = h("div", { class: "journey-steps", style: `grid-template-columns:repeat(${max},1fr)` });
  for (let i = 1; i <= max; i++) {
    const cls = i < c.level ? "done" : i === c.level ? "now" : null;
    const tip = i < c.level ? `Level ${i} · done` : i === c.level ? `Level ${i} · now, ${num(pct, 0)}%` : `Level ${i}`;
    steps.append(h("i", { class: cls, title: tip },
      i === c.level ? h("b", { style: `width:${pct}%` }) : null));
  }
  const ticks = h("div", { class: "journey-ticks", "aria-hidden": "true" });
  for (let i = 0; i <= max; i += 10) {
    const lv = Math.max(1, i);
    ticks.append(h("span", { style: `left:${((lv - 0.5) / max) * 100}%` }, lv));
  }
  const pips = h("div", { class: "journey", role: "img", "aria-label": `Level ${c.level} of ${max}` }, steps, ticks);

  el.replaceChildren(
    h("div", { class: "card-head" }, h("h2", null, "Progress"),
      h("span", { class: "muted" }, toGo ? `${toGo} level${toGo === 1 ? "" : "s"} to go` : "Max level")),
    h("div", { class: "level-row" },
      h("div", { class: "level-box" }, h("small", null, "Level"), h("b", null, c.level)),
      h("div", null,
        h("div", { class: "bar", title: `${pct.toFixed(1)}%` },
          h("span", { class: "rested", style: `width:${restedEnd}%` }),
          h("span", { class: "xp", style: `width:${pct}%` })),
        h("div", { class: "bar-meta" },
          h("span", null, c.xpMax ? `${num(c.xp)} / ${num(c.xpMax)} XP (${num(pct, 1)}%)` : "Max level"),
          c.rested ? h("span", { class: "rested-note" }, `${num(c.rested)} rested`) : null))),
    pips,
    h("div", { class: "stats" },
      stat("XP / hour", rate ? num(rate) : "–"),
      stat("Next level in", eta ? hm(eta) : "–"),
      stat("Gold", money(c.money)),
      stat("Deaths", num(c.deaths || 0))));
}

function stat(label, value) {
  return h("div", { class: "stat" }, h("small", null, label), h("b", null, value));
}

function renderVitals(c) {
  const el = document.getElementById("vitals");
  const badges = h("div", { class: "badges" },
    c && c.online && c.inCombat ? h("span", { class: "badge combat" }, "In combat") : null,
    c && c.resting ? h("span", { class: "badge resting" }, "Resting") : null);
  const head = h("div", { class: "card-head" }, h("h2", null, "Vitals"), badges);
  const vt = c && vitals(c);
  if (!vt) {
    el.replaceChildren(head, h("p", { class: "empty-note" },
      "No health data yet. The game hides it from addons, so it comes from the combat log: type /combatlog in game, then fight something."));
    return;
  }
  const { v } = vt;
  const rows = [bar("Health", v.hp, v.hpMax, "hp")];
  const pw = POWER[v.powerType];
  if (pw && v.powerMax && v.power != null) {
    // The combat log writes rage in tenths; the addon gets whole points.
    const k = vt.source === "log" ? pw.scale || 1 : 1;
    rows.push(bar(pw.name, Math.round(v.power / k), Math.round(v.powerMax / k), pw.cls));
  }
  const note = vt.source === "log"
    ? `From the game's combat log at your last combat action (${ago(vt.t)}). Shown as of the last update.`
    : `From the addon at the last save (${ago(vt.t)}).`;
  el.replaceChildren(head, ...rows, h("p", { class: "vitals-note" }, note));
}

function bar(label, cur, max, cls) {
  const pct = max ? clamp((cur / max) * 100, 0, 100) : 0;
  return h("div", { class: "vital" },
    h("div", { class: "vital-top" }, h("span", null, label), h("span", null, `${num(cur)} / ${num(max)}`)),
    h("div", { class: "bar" }, h("span", { class: cls, style: `width:${pct}%` })));
}

// ---------------------------------------------------------------------------
// Zone map
// ---------------------------------------------------------------------------

// The map keeps the same layer elements between renders so the marker can
// glide to its next position instead of jumping.
let mapLayers = null;
let markerAt = null;   // "character|frame" the marker was last placed on
let mapHover = [];     // [{ x, y, label }] in SVG units, for the tooltip
let mapScale = 1;      // SVG units per on-screen marker unit

function mapSvg() {
  const svg = document.getElementById("map");
  if (!mapLayers) {
    mapLayers = {
      base: s("g"),
      trail: s("g"),
      deaths: s("g"),
      marker: s("g", { class: "marker" },
        s("circle", { class: "ring", r: 14 }),
        s("circle", { class: "dot", r: 9, "stroke-width": 4 }),
        s("title")),
    };
    svg.replaceChildren(mapLayers.base, mapLayers.trail, mapLayers.deaths, mapLayers.marker);
    svg.addEventListener("pointermove", onMapHover);
    svg.addEventListener("pointerleave", () => { document.getElementById("map-tip").hidden = true; });
  }
  return svg;
}

function onMapHover(ev) {
  const svg = ev.currentTarget;
  const tip = document.getElementById("map-tip");
  const ctm = svg.getScreenCTM();
  if (!ctm || !mapHover.length) { tip.hidden = true; return; }
  const p = new DOMPoint(ev.clientX, ev.clientY).matrixTransform(ctm.inverse());
  let best = null;
  let bd = Infinity;
  for (const q of mapHover) {
    const d = (q.x - p.x) ** 2 + (q.y - p.y) ** 2;
    if (d < bd) { bd = d; best = q; }
  }
  const r = 18 * mapScale;
  if (!best || bd > r * r) { tip.hidden = true; return; }
  const box = svg.parentElement.getBoundingClientRect();
  const at = new DOMPoint(best.x, best.y).matrixTransform(ctm);
  tip.textContent = best.label;
  tip.hidden = false;
  tip.style.left = `${at.x - box.left}px`;
  tip.style.top = `${at.y - box.top}px`;
}

async function renderMap(c) {
  const svg = mapSvg();
  const $ = (id) => document.getElementById(id);
  const hasPos = c && c.mapID && maps[c.mapID] && maps[c.mapID].min;

  $("zone-name").textContent = (c && (c.zone || (maps[c.mapID] || {}).name)) || "Location";
  $("subzone").textContent = c ? c.subzone || (c.instance && c.instance !== "none" ? "Inside an instance" : "") : "";
  const coords = $("coords");
  coords.hidden = !(c && c.x != null && c.hasPosition !== false);
  if (!coords.hidden) coords.textContent = `${(c.x * 100).toFixed(1)}, ${(c.y * 100).toFixed(1)}`;
  const onMap = c ? (c.trail || []).filter((p) => p.mapID === c.mapID).length : 0;
  $("trail-note").textContent = c && hasPos
    ? `Trail: ${num(onMap)} recorded position${onMap === 1 ? "" : "s"} on this map.${c.positionSource === "combatLog" ? ` Position from the combat log, ${ago(c.positionAt)}.` : ""}`
    : "";

  if (!hasPos) {
    mapLayers.base.replaceChildren(s("text", { x: W / 2, y: H / 2, fill: "#8d95a3", "font-size": 34, "text-anchor": "middle" },
      c ? "No position yet. Move around in game, then /wt sync." : "Waiting for data…"));
    mapLayers.trail.replaceChildren();
    mapLayers.deaths.replaceChildren();
    mapLayers.marker.style.display = "none";
    markerAt = null;
    mapHover = [];
    svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
    svg.style.aspectRatio = "3 / 2";
    return;
  }

  // The zone's own map art when there is an image for it; otherwise the
  // continent, drawn schematically (or with its art, if present).
  const zoneArt = await probeArt(c.mapID);
  const frameId = zoneArt ? c.mapID : continentOf(c.mapID);
  const frame = maps[frameId];
  const artUrl = zoneArt || (await probeArt(frameId));
  if (selected() !== c) return; // selection changed while probing

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
  for (const z of zones) z.city = zones.some((o) => o !== z && o.w * o.h > z.w * z.h && inside(z, o));

  // Trail: break the line on long gaps or when leaving this frame.
  const segments = [];
  const hover = [];
  let seg = [];
  let lastT = null;
  for (const p of c.trail || []) {
    const pt = project(frame, p.mapID, p.x, p.y);
    if (!pt || (lastT && p.t - lastT > 900)) {
      if (seg.length > 1) segments.push(seg);
      seg = [];
    }
    if (pt) {
      seg.push(pt);
      const other = p.mapID !== c.mapID && maps[p.mapID] ? ` · ${maps[p.mapID].name}` : "";
      hover.push({ x: pt[0], y: pt[1], label: `${clock(p.t)} · ${(p.x * 100).toFixed(1)}, ${(p.y * 100).toFixed(1)}${other}` });
    }
    lastT = p.t;
  }
  if (seg.length > 1) segments.push(seg);
  const pos = project(frame, c.mapID, c.x, c.y);

  let vb = { x: 0, y: 0, w: W, h: H };
  const layer = s("g");
  if (artUrl) {
    layer.append(s("image", { href: artUrl, x: 0, y: 0, width: W, height: H, preserveAspectRatio: "none" }));
  } else {
    // Continent schematic, cropped to the land.
    const pts = zones.filter((z) => !z.city).map((z) => [z.cx, z.cy]).concat(segments.flat(), pos ? [pos] : []);
    if (pts.length) {
      const xs = pts.map((p) => p[0]);
      const ys = pts.map((p) => p[1]);
      const pad = 130;
      vb = { x: Math.min(...xs) - pad, y: Math.min(...ys) - pad / 2, w: 0, h: 0 };
      vb.w = Math.max(...xs) + pad - vb.x;
      vb.h = Math.max(...ys) + pad / 2 - vb.y;
      // Keep the frame's 3:2 shape so the card does not jump in height.
      const want = 1.5;
      if (vb.w / vb.h < want) { const nw = vb.h * want; vb.x -= (nw - vb.w) / 2; vb.w = nw; }
      else { const nh = vb.w / want; vb.y -= (nh - vb.h) / 2; vb.h = nh; }
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
  mapLayers.base.replaceChildren(layer);
  svg.setAttribute("viewBox", `${vb.x.toFixed(0)} ${vb.y.toFixed(0)} ${vb.w.toFixed(0)} ${vb.h.toFixed(0)}`);
  svg.style.aspectRatio = "3 / 2";
  const k = Math.max(vb.w / W, vb.h / H); // keep markers the same on-screen size
  mapScale = k;

  mapLayers.trail.replaceChildren(...segments.map((pts) => s("polyline", {
    class: `trail${artUrl ? "" : " on-schematic"}`, "stroke-width": 3 * k,
    points: pts.map((p) => p.map((v) => v.toFixed(1)).join(",")).join(" "),
  })));

  // Deaths with a recorded position on this map.
  const deaths = [];
  for (const e of c.journal || []) {
    if (e.kind !== "death" || !e.mapID || e.x == null || e.y == null) continue;
    const pt = project(frame, e.mapID, e.x, e.y);
    if (!pt) continue;
    const label = `Died at level ${e.level ?? "?"} · ${clock(e.t)}${e.subzone ? ` · ${e.subzone}` : ""}`;
    hover.push({ x: pt[0], y: pt[1], label });
    deaths.push(s("g", { class: "death-mark", transform: `translate(${pt[0].toFixed(1)} ${pt[1].toFixed(1)}) scale(${k})` },
      s("circle", { r: 9 }),
      s("path", { d: "M-4,-4L4,4M4,-4L-4,4" })));
  }
  mapLayers.deaths.replaceChildren(...deaths);
  mapHover = hover;

  const mk = mapLayers.marker;
  if (pos) {
    const stale = c.hasPosition === false;
    const at = `${c.key}|${frameId}`;
    const jump = markerAt !== at; // new character or map: no glide
    markerAt = at;
    mk.style.display = "";
    mk.setAttribute("class", `marker${c.online ? " online" : ""}${stale ? " stale" : ""}${jump ? " jump" : ""}`);
    mk.style.transform = `translate(${pos[0].toFixed(1)}px, ${pos[1].toFixed(1)}px) scale(${k})`;
    mk.lastChild.textContent = stale ? `${c.name}: last known position` : c.name;
    if (jump) {
      mk.getBoundingClientRect(); // apply the jump before turning the glide back on
      requestAnimationFrame(() => mk.classList.remove("jump"));
    }
  } else {
    mk.style.display = "none";
    markerAt = null;
  }
}

// ---------------------------------------------------------------------------
// Quest log
// ---------------------------------------------------------------------------

function renderQuests(c) {
  const el = document.getElementById("quests");
  const quests = (c && c.quests) || [];
  const ready = quests.filter((q) => q.complete).length;
  const list = h("div", { class: "scroll" });
  if (!quests.length) {
    list.append(h("p", { class: "empty-note" }, c ? "No quests recorded yet. They appear after the next /wt sync." : ""));
  }
  let header = null;
  for (const q of quests) {
    if (q.header !== header) {
      header = q.header;
      if (header) list.append(h("div", { class: "group" }, header));
    }
    const objectives = q.objectives || [];
    list.append(h("div", { class: `quest${q.complete ? " ready" : ""}${q.failed ? " failed" : ""}` },
      h("div", { class: "quest-top" }, h("b", null, q.title), q.level ? h("small", null, `Lv ${q.level}`) : null),
      q.complete ? h("p", { class: "quest-state" }, "Ready to turn in") : q.failed ? h("p", { class: "quest-state" }, "Failed") : null,
      objectives.map((o) => {
        const counted = o.need != null && o.need > 0;
        // Saves from before the addon understood "3/3 Fel Cone".
        const text = counted ? o.text.replace(/^\d+\s*\/\s*\d+\s+/, "") : o.text;
        const pct = counted ? clamp((o.have / o.need) * 100, 0, 100) : o.done ? 100 : 0;
        return h("div", { class: `obj${o.done ? " done" : ""}` },
          h("div", { class: "obj-top" }, h("span", null, text), counted ? h("span", null, `${o.have}/${o.need}`) : null),
          h("div", { class: "bar" }, h("span", { style: `width:${pct}%` })));
      })));
  }
  el.replaceChildren(
    h("div", { class: "card-head" }, h("h2", null, "Quest log"),
      h("span", { class: "muted" }, quests.length ? `${quests.length} active · ${ready} ready to turn in` : "")),
    h("div", { class: "scroll-box" }, list));
}

// ---------------------------------------------------------------------------
// Levelling curve
// ---------------------------------------------------------------------------

function renderCurve(c) {
  const el = document.getElementById("curve");
  const head = h("div", { class: "card-head" }, h("h2", null, "Levelling curve"), h("span", { class: "muted" }, "active play time"));
  const samples = (c && c.xpSamples) || [];
  if (samples.length < 2) {
    el.replaceChildren(head, h("p", { class: "empty-note" }, "The curve fills in as you play with the addon installed."));
    return;
  }
  const pts = samples.map((p) => [p.played / 3600, p.level + (p.xpMax ? p.xp / p.xpMax : 0)]);
  const x0 = pts[0][0];
  const x1 = Math.max(pts[pts.length - 1][0], x0 + 0.01);
  const lo = Math.floor(Math.min(...pts.map((p) => p[1])));
  const hi = Math.max(lo + 1, Math.ceil(Math.max(...pts.map((p) => p[1]))));
  const VW = 600, VH = 300, L = 30, R = 8, T = 10, B = 24;
  const sx = (x) => L + ((x - x0) / (x1 - x0)) * (VW - L - R);
  const sy = (y) => T + (1 - (y - lo) / (hi - lo)) * (VH - T - B);

  const axis = s("g", { class: "curve-axis" });
  const step = Math.max(1, Math.ceil((hi - lo) / 6));
  for (let lv = lo; lv <= hi; lv += step) {
    axis.append(s("line", { x1: L, x2: VW - R, y1: sy(lv), y2: sy(lv) }));
    axis.append(s("text", { x: L - 8, y: sy(lv) + 4, "text-anchor": "end" }, lv));
  }
  // Steps: the level rises inside a level, then jumps at a level-up.
  const line = pts.map(([x, y], i) => `${i ? "L" : "M"}${sx(x).toFixed(1)},${sy(y).toFixed(1)}`).join(" ");
  const area = `${line} L${sx(x1).toFixed(1)},${sy(lo).toFixed(1)} L${sx(x0).toFixed(1)},${sy(lo).toFixed(1)} Z`;
  const svg = s("svg", { viewBox: `0 0 ${VW} ${VH}`, role: "img", "aria-label": "Level against played time" },
    s("defs", null, s("linearGradient", { id: "curve-fill", x1: 0, y1: 0, x2: 0, y2: 1 },
      s("stop", { offset: "0", "stop-color": "#e3b45a", "stop-opacity": ".35" }),
      s("stop", { offset: "1", "stop-color": "#e3b45a", "stop-opacity": "0" }))),
    axis,
    s("path", { d: area, fill: "url(#curve-fill)" }),
    s("path", { d: line, class: "curve-line" }));
  const played = playedNow(c);
  el.replaceChildren(head, svg, h("p", { class: "curve-note" }, played != null ? `${num(played / 3600, 1)} h played` : ""));
}

// ---------------------------------------------------------------------------
// Journal
// ---------------------------------------------------------------------------

const FILTERS = [
  ["all", "All", null],
  ["levels", "Levels", ["level"]],
  ["quests", "Quests", ["questAccepted", "questDone"]],
  ["zones", "Zones", ["zone"]],
  ["deaths", "Deaths", ["death"]],
];

function describe(e) {
  const where = [e.subzone, e.zone].filter(Boolean).join(", ");
  switch (e.kind) {
    case "questDone":
      return ["✓", "done", e.title || "Quest", ["Completed", e.xp ? `${num(e.xp)} XP` : null, e.money ? money(e.money, true) : null].filter(Boolean).join(" · ")];
    case "questAccepted":
      return ["!", "", e.title || "Quest", "Accepted"];
    case "level":
      return ["★", "level", `Reached level ${e.level}`, e.zone || ""];
    case "zone":
      return ["⚑", "", `Entered ${e.zone}`, e.level ? `Level ${e.level}` : ""];
    case "death":
      return ["☠", "death", `Died at level ${e.level}`, where];
    case "login":
      return ["⏻", "", "Logged in", [e.zone, e.level ? `Level ${e.level}` : null].filter(Boolean).join(" · ")];
    default:
      return ["•", "", e.kind, ""];
  }
}

function renderJournal(c) {
  const el = document.getElementById("journal");
  const kinds = (FILTERS.find((f) => f[0] === journalFilter) || FILTERS[0])[2];
  const entries = ((c && c.journal) || []).filter((e) => !kinds || kinds.includes(e.kind)).slice().reverse();
  const chips = h("div", { class: "chips", role: "group", "aria-label": "Filter the journal" },
    FILTERS.map(([key, label]) => h("button", {
      type: "button",
      "aria-pressed": String(key === journalFilter),
      onclick: () => { journalFilter = key; renderJournal(selected()); },
    }, label)));
  const list = h("div", { class: "scroll" },
    entries.length ? entries.map((e) => {
      const [icon, cls, title, sub] = describe(e);
      return h("div", { class: "entry" },
        h("span", { class: `icon ${cls}`, "aria-hidden": "true" }, icon),
        h("div", null, h("b", null, title), sub ? h("small", null, sub) : null),
        h("time", { datetime: new Date(e.t * 1000).toISOString() }, clock(e.t)));
    }) : h("p", { class: "empty-note" }, c ? "Nothing here yet. Levels, quests, zones and deaths are recorded from now on." : ""));
  el.replaceChildren(h("div", { class: "card-head" }, h("h2", null, "Journal")), chips, h("div", { class: "scroll-box" }, list));
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

function renderFooter(c) {
  const el = document.getElementById("updated");
  el.textContent = c
    ? `Updates when you /reload, /wt sync or log out in game, and from the combat log while you fight. Last save ${ago(c.savedAt)}.${c.live ? ` Combat log ${ago(c.live.t)}.` : ""}`
    : "";
}

function render() {
  const c = selected();
  renderHeader(c);
  renderProgress(c);
  renderVitals(c);
  renderQuests(c);
  renderCurve(c);
  renderJournal(c);
  renderMap(c);
  renderFooter(c);
}

/** Only the "x ago" texts change between polls when nothing new arrived. */
function renderTimes() {
  const c = selected();
  renderHeader(c);
  renderVitals(c);
  renderFooter(c);
}

function banner(msg) {
  const b = document.getElementById("banner");
  b.hidden = !msg;
  b.textContent = msg || "";
}

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
    if (changed) render(); else renderTimes();
  } catch (e) {
    banner(`Cannot reach the tracker API (${e.message}). Showing the last data received.`);
  }
}

async function start() {
  document.title = CFG.title;
  try {
    maps = (await (await fetch("data/maps.json")).json()).maps;
  } catch {
    banner("Could not load data/maps.json.");
  }
  render();
  await poll();
  setInterval(poll, Math.max(5, CFG.pollSeconds) * 1000);
}

start();
