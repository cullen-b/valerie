// Injected into every page as (function (CFG) { ... })(config). Pages can see and call this; it only talks to the
// browser through a whitelist of harmless messages.
if (window.__valerie || window.top !== window) return;
const post = (o) => { try { window.ipc.postMessage(JSON.stringify(o)); } catch (_) {} };
const PAGE_ACTIONS = new Set(["scroll_down", "scroll_up", "scroll_left", "scroll_right", "half_page_down", "half_page_up", "top", "bottom", "hints", "hints_new_tab", "insert", "find"]);
let cfg = CFG, mode = "normal", buffer = "", bufferTimer = 0;

// ---------- shadow-DOM UI host (hints, find bar, toasts) ----------
let shadow = null;
function ui() {
  if (shadow && shadow.host.isConnected) return shadow;
  const host = document.createElement("valerie-ui");
  host.style.cssText = "all: initial; position: fixed; inset: 0; pointer-events: none; z-index: 2147483647;";
  (document.documentElement || document.body).appendChild(host);
  shadow = host.attachShadow({ mode: "closed" });
  const st = document.createElement("style");
  st.textContent = `
    * { font: 600 11px/1 -apple-system, system-ui, sans-serif; }
    .hint { position: fixed; padding: 2px 4px; border-radius: 4px; background: ${cfg.accent || "#9d8cff"}; color: #fff;
            letter-spacing: .5px; box-shadow: 0 2px 6px rgba(0,0,0,.35); text-transform: uppercase; }
    .hint .typed { opacity: .45; }
    .toast { position: fixed; left: 50%; bottom: 28px; transform: translateX(-50%); padding: 9px 14px; border-radius: 10px;
             background: rgba(24,24,32,.92); color: #fff; font-size: 12.5px; font-weight: 500; transition: opacity .2s; box-shadow: 0 8px 24px rgba(0,0,0,.3); }
    .find { position: fixed; top: 12px; right: 16px; display: flex; gap: 6px; align-items: center; padding: 6px 8px; border-radius: 10px;
            background: rgba(24,24,32,.94); box-shadow: 0 8px 24px rgba(0,0,0,.3); pointer-events: auto; }
    .find input { all: unset; width: 220px; padding: 4px 6px; color: #fff; font: 13px -apple-system, system-ui; }
    .find span { color: rgba(255,255,255,.5); font-weight: 400; font-size: 11.5px; }`;
  shadow.appendChild(st);
  return shadow;
}

function toast(msg) {
  const t = document.createElement("div"); t.className = "toast"; t.textContent = msg; ui().appendChild(t);
  setTimeout(() => { t.style.opacity = "0"; setTimeout(() => t.remove(), 250); }, 1500);
}

// ---------- scrolling ----------
function scrollable(e) {
  if (!e || e === document.body || e === document.documentElement) return false;
  const s = getComputedStyle(e);
  return /(auto|scroll|overlay)/.test(s.overflowY + s.overflowX) && (e.scrollHeight > e.clientHeight + 4 || e.scrollWidth > e.clientWidth + 4);
}
function scroller() {
  for (let e = document.activeElement; e; e = e.parentElement) if (scrollable(e)) return e;
  const doc = document.scrollingElement || document.documentElement;
  if (doc.scrollHeight > innerHeight + 4) return doc;
  // App-like pages scroll an inner element: pick the largest visible one.
  let best = doc, area = 0;
  for (const e of document.querySelectorAll("div, main, section, article")) {
    if (!scrollable(e)) continue;
    const r = e.getBoundingClientRect(), a = Math.max(0, r.width) * Math.max(0, r.height);
    if (a > area) { area = a; best = e; }
  }
  return best;
}
const step = () => cfg.vim.scroll_step || 80;
const scrollBy = (x, y) => scroller().scrollBy({ left: x, top: y, behavior: "instant" });

// ---------- link hints ----------
let hints = null;
const CLICKABLE = "a[href], button, input:not([type=hidden]), select, textarea, summary, label[for], [role=button], [role=link], [role=tab], [role=menuitem], [role=checkbox], [onclick], [contenteditable=true], [tabindex]:not([tabindex='-1'])";
function visibleRect(e) {
  const r = e.getClientRects()[0];
  if (!r || r.width < 3 || r.height < 3 || r.bottom < 0 || r.right < 0 || r.top > innerHeight || r.left > innerWidth) return null;
  const s = getComputedStyle(e);
  if (s.visibility === "hidden" || s.opacity === "0") return null;
  const hit = document.elementFromPoint(Math.min(Math.max(r.left + r.width / 2, 0), innerWidth - 1), Math.min(Math.max(r.top + r.height / 2, 0), innerHeight - 1));
  if (hit && hit !== e && !e.contains(hit) && !hit.contains(e)) return null;
  return r;
}
function labels(n) {
  const chars = "asdfghjklqweruiopzxcvbnm";
  let out = [""];
  while (out.length < n) { const prefix = out.shift(); for (const c of chars) out.push(prefix + c); }
  return out.slice(0, n).sort();
}
function startHints(newTab) {
  const els = [...document.querySelectorAll(CLICKABLE)].map((e) => [e, visibleRect(e)]).filter((x) => x[1]);
  if (!els.length) return toast("No links");
  const root = ui(), ls = labels(els.length);
  hints = { newTab, typed: "", items: els.map(([e, r], i) => {
    const d = document.createElement("div"); d.className = "hint"; d.textContent = ls[i];
    d.style.left = Math.max(r.left - 2, 0) + "px"; d.style.top = Math.max(r.top - 2, 0) + "px";
    root.appendChild(d); return { e, label: ls[i], d };
  }) };
  mode = "hints";
}
function stopHints() { if (hints) hints.items.forEach((h) => h.d.remove()); hints = null; mode = "normal"; }
function hintKey(k) {
  if (k === "Escape") return stopHints();
  if (k === "Backspace") hints.typed = hints.typed.slice(0, -1);
  else if (k.length === 1) hints.typed += k.toLowerCase();
  const left = hints.items.filter((h) => h.label.startsWith(hints.typed));
  hints.items.forEach((h) => {
    const on = h.label.startsWith(hints.typed); h.d.style.display = on ? "" : "none";
    if (on) h.d.innerHTML = '<span class="typed">' + hints.typed + "</span>" + h.label.slice(hints.typed.length);
  });
  if (left.length === 1 && left[0].label === hints.typed) { const { e } = left[0]; const nt = hints.newTab; stopHints(); activate(e, nt); }
  else if (!left.length) stopHints();
}
function activate(e, newTab) {
  const a = e.closest("a[href]");
  if (newTab && a && /^https?:/.test(a.href)) return post({ t: "open", url: a.href });
  if (e.matches("input, textarea, select, [contenteditable=true]")) { e.focus(); if (e.select) try { e.select(); } catch (_) {} return; }
  e.focus({ preventScroll: true });
  e.click();
}

// ---------- find ----------
function openFind(initial) {
  const root = ui();
  let bar = root.querySelector(".find");
  if (!bar) {
    bar = document.createElement("div"); bar.className = "find";
    bar.innerHTML = '<input placeholder="Find in page"><span>↵ next · ⇧↵ prev · esc</span>';
    root.appendChild(bar);
    const inp = bar.querySelector("input");
    inp.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Enter") { e.preventDefault(); if (inp.value) window.find(inp.value, false, e.shiftKey, true); }
      if (e.key === "Escape") { bar.remove(); window.getSelection()?.removeAllRanges(); }
    }, true);
  }
  const inp = bar.querySelector("input"); if (initial) inp.value = initial;
  setTimeout(() => { inp.focus(); inp.select(); }, 0);
}

// ---------- key handling ----------
function editable(e) {
  if (!e) return false;
  if (e.isContentEditable) return true;
  if (e.tagName === "TEXTAREA" || e.tagName === "SELECT") return true;
  if (e.tagName === "INPUT") return !/^(button|submit|checkbox|radio|reset|range|color|file|image)$/i.test(e.type);
  return false;
}
function deepActive() { let a = document.activeElement; while (a && a.shadowRoot && a.shadowRoot.activeElement) a = a.shadowRoot.activeElement; return a; }
function excluded() { const h = location.hostname; return (cfg.vim.excluded_hosts || []).some((x) => h === x || h.endsWith("." + x)); }

function run(action) {
  if (typeof action === "string" && PAGE_ACTIONS.has(action)) {
    const h = innerHeight;
    switch (action) {
      case "scroll_down": return scrollBy(0, step());
      case "scroll_up": return scrollBy(0, -step());
      case "scroll_left": return scrollBy(-step(), 0);
      case "scroll_right": return scrollBy(step(), 0);
      case "half_page_down": return scrollBy(0, h / 2);
      case "half_page_up": return scrollBy(0, -h / 2);
      case "top": { const s = scroller(); return s.scrollTo({ top: 0, behavior: "instant" }); }
      case "bottom": { const s = scroller(); return s.scrollTo({ top: s.scrollHeight, behavior: "instant" }); }
      case "hints": return startHints(false);
      case "hints_new_tab": return startHints(true);
      case "insert": mode = "insert"; return toast("-- INSERT -- (esc to leave)");
      case "find": return openFind();
    }
  }
  const cmd = typeof action === "string" ? { cmd: action === "yank_url" ? "copy_url" : action } : action;
  post({ t: "cmd", cmd });
}

window.addEventListener("keydown", (e) => {
  if (!cfg.vim.enabled || e.defaultPrevented && mode !== "hints") return;
  if (mode === "hints") { e.preventDefault(); e.stopImmediatePropagation(); return hintKey(e.key); }
  if (e.metaKey || e.ctrlKey || e.altKey || e.isComposing) return;
  if (e.key === "Escape") {
    if (mode === "insert") { mode = "normal"; toast("-- NORMAL --"); }
    const a = deepActive(); if (editable(a)) a.blur();
    buffer = ""; return;
  }
  if (mode === "insert" || editable(deepActive()) || excluded()) return;
  if (e.key.length !== 1) return;
  const b = cfg.vim.bindings, next = buffer + e.key;
  const keys = Object.keys(b).filter((k) => b[k] != null);
  clearTimeout(bufferTimer);
  if (b[next] != null && !keys.some((k) => k !== next && k.startsWith(next))) { buffer = ""; e.preventDefault(); e.stopImmediatePropagation(); return run(b[next]); }
  if (keys.some((k) => k.startsWith(next))) {
    buffer = next; e.preventDefault(); e.stopImmediatePropagation();
    bufferTimer = setTimeout(() => { if (b[buffer] != null) run(b[buffer]); buffer = ""; }, 700);
    return;
  }
  buffer = "";
  if (b[e.key] != null && !keys.some((k) => k !== e.key && k.startsWith(e.key))) { e.preventDefault(); e.stopImmediatePropagation(); run(b[e.key]); }
}, true);

// ---------- page metadata for the sidebar ----------
function favicon() {
  const links = [...document.querySelectorAll("link[rel~='icon'], link[rel='shortcut icon'], link[rel='apple-touch-icon']")];
  const svg = links.find((l) => /svg/.test(l.type || l.href));
  const best = svg || links.sort((a, b) => (parseInt(b.sizes?.value) || 16) - (parseInt(a.sizes?.value) || 16)).find((l) => (parseInt(l.sizes?.value) || 16) <= 64) || links[0];
  return best ? best.href : null;
}
let lastMeta = "";
function report() {
  const m = { t: "meta", favicon: favicon(), playing: [...document.querySelectorAll("video, audio")].some((v) => !v.paused && !v.muted), url: location.href };
  const s = JSON.stringify(m); if (s !== lastMeta) { lastMeta = s; post(m); }
}
for (const fn of ["pushState", "replaceState"]) {
  const orig = history[fn];
  history[fn] = function () { const r = orig.apply(this, arguments); setTimeout(report, 0); return r; };
}
addEventListener("popstate", report); addEventListener("hashchange", report);
document.addEventListener("play", report, true); document.addEventListener("pause", report, true); document.addEventListener("volumechange", report, true);
if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", report); else report();
addEventListener("load", report);

Object.defineProperty(window, "__valerie", {
  value: Object.freeze({ configure: (c) => { cfg = c; }, toast, find: openFind, hints: startHints }),
  writable: false, configurable: false,
});
