// Injected into every page as (function (CFG) { ... })(config). Pages can see and call this; it only talks to the
// browser through a whitelist of harmless messages.
if (window.__valerie || window.top !== window) return;

// ---------- tabs opened in the background don't play audio/video until you first look at them ----------
// A tab Valerie hasn't shown yet is a hidden document. Until it becomes visible, play() is held (and resumed then);
// after that, media behaves normally (switching away doesn't pause anything).
(() => {
  if (CFG.tabs && CFG.tabs.autoplay_in_background) return;
  if (document.visibilityState === "visible") return;
  const realPlay = HTMLMediaElement.prototype.play;
  const waiting = new Map(); // element -> [resolve, reject] for the promise play() returned
  HTMLMediaElement.prototype.play = function () {
    if (document.visibilityState === "visible") return realPlay.call(this);
    return new Promise((resolve, reject) => waiting.set(this, [resolve, reject]));
  };
  // Media started by the `autoplay` attribute doesn't go through play(): pause it and remember it.
  const onPlay = (e) => {
    const m = e.target;
    if (document.visibilityState !== "visible" && m instanceof HTMLMediaElement && !waiting.has(m)) {
      m.pause();
      waiting.set(m, null);
    }
  };
  document.addEventListener("play", onPlay, true);
  const release = () => {
    if (document.visibilityState !== "visible") return;
    document.removeEventListener("visibilitychange", release);
    document.removeEventListener("play", onPlay, true);
    HTMLMediaElement.prototype.play = realPlay;
    for (const [m, p] of waiting) {
      const r = realPlay.call(m);
      if (p) r.then(p[0], p[1]); else r.catch(() => {});
    }
    waiting.clear();
  };
  document.addEventListener("visibilitychange", release);
})();
// Capture natives before page scripts run, so pages can't intercept what we send or fill.
const stringify = JSON.stringify;
const ipcPost = window.ipc ? window.ipc.postMessage.bind(window.ipc) : null;
const post = (o) => { try { (ipcPost || ((m) => window.ipc.postMessage(m)))(stringify(o)); } catch (_) {} };
const valueSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
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
    .hint { position: fixed; left: 0; top: 0; will-change: transform; padding: 2px 4px; border-radius: 4px; background: ${cfg.accent || "#9d8cff"}; color: #fff;
            letter-spacing: .5px; box-shadow: 0 2px 6px rgba(0,0,0,.35); text-transform: uppercase; }
    .hint .typed { opacity: .45; }
    .toast { position: fixed; left: 50%; bottom: 28px; transform: translateX(-50%); padding: 9px 14px; border-radius: 10px;
             background: rgba(24,24,32,.92); color: #fff; font-size: 12.5px; font-weight: 500; transition: opacity .2s; box-shadow: 0 8px 24px rgba(0,0,0,.3); }
    .menu { position: fixed; min-width: 220px; padding: 5px; border-radius: 10px; background: rgba(24,24,32,.97); pointer-events: auto;
            box-shadow: 0 10px 28px rgba(0,0,0,.35); }
    .menu div { padding: 8px 10px; border-radius: 7px; color: #fff; font-weight: 500; font-size: 12.5px; cursor: default; }
    .menu div:hover { background: ${cfg.accent || "#9d8cff"}; }
    .menu small { display: block; color: rgba(255,255,255,.55); font-weight: 400; margin-top: 2px; }
    .save { position: fixed; top: 12px; right: 16px; width: 300px; padding: 14px; border-radius: 12px; background: rgba(24,24,32,.97);
            box-shadow: 0 12px 32px rgba(0,0,0,.35); pointer-events: auto; color: #fff; }
    .save p { font-size: 13px; font-weight: 500; line-height: 1.35; margin: 0 0 10px; }
    .save button { all: unset; padding: 6px 11px; margin-right: 6px; border-radius: 7px; background: rgba(255,255,255,.1); color: #fff; font-size: 12px; }
    .save button.primary { background: ${cfg.accent || "#9d8cff"}; }
    .find { position: fixed; top: 12px; right: 16px; display: flex; gap: 6px; align-items: center; padding: 6px 10px 6px 8px; border-radius: 10px;
            background: rgba(24,24,32,.94); box-shadow: 0 8px 24px rgba(0,0,0,.3); pointer-events: auto; }
    .find.with-map { right: 78px; }
    .find input { all: unset; width: 220px; padding: 4px 6px; color: #fff; font: 13px -apple-system, system-ui; }
    .find .count { color: rgba(255,255,255,.5); font-weight: 400; font-size: 11.5px; min-width: 52px; text-align: right; }
    .minimap { position: fixed; top: 0; right: 0; width: 64px; height: 100vh; background: rgba(18,18,26,.82); pointer-events: auto;
               box-shadow: -1px 0 0 rgba(255,255,255,.06), -8px 0 24px rgba(0,0,0,.18); cursor: pointer; }`;
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
let innerScroller = null;
function scroller() {
  for (let e = document.activeElement; e; e = e.parentElement) if (scrollable(e)) return e;
  const doc = document.scrollingElement || document.documentElement;
  if (doc.scrollHeight > innerHeight + 4) return doc;
  // App-like pages scroll an inner element: pick the largest visible one (scanning is slow, so remember it).
  if (innerScroller && innerScroller.isConnected && scrollable(innerScroller)) return innerScroller;
  let best = doc, area = 0;
  for (const e of document.querySelectorAll("div, main, section, article")) {
    if (!scrollable(e)) continue;
    const r = e.getBoundingClientRect(), a = Math.max(0, r.width) * Math.max(0, r.height);
    if (a > area) { area = a; best = e; }
  }
  innerScroller = best === doc ? null : best;
  return best;
}
const step = () => cfg.vim.scroll_step || 80;
// Smooth scrolling: each key press adds to a target and the page eases toward it, so holding j glides.
const glide = { el: null, x: 0, y: 0, raf: 0, last: 0 };
function scrollBy(x, y) {
  const el = scroller();
  if (cfg.vim.smooth_scroll === false) return el.scrollBy({ left: x, top: y, behavior: "instant" });
  if (glide.el !== el) { endGlide(); glide.el = el; }
  glide.x += x; glide.y += y;
  if (!glide.raf) {
    // Pages with `scroll-behavior: smooth` would make each step async; we animate, so steps must be instant.
    glide.saved = el.style.scrollBehavior; el.style.scrollBehavior = "auto";
    glide.last = 0; glide.raf = requestAnimationFrame(glideFrame);
  }
}
function endGlide() {
  if (glide.raf) cancelAnimationFrame(glide.raf);
  if (glide.el && glide.saved !== undefined) glide.el.style.scrollBehavior = glide.saved;
  glide.x = glide.y = 0; glide.raf = 0; glide.saved = undefined;
}
function glideFrame(now) {
  // Frame timestamps can predate the key press, so treat the first frame as one normal frame.
  const dt = glide.last ? Math.max(1, Math.min(now - glide.last, 48)) : 16; glide.last = now;
  const f = 1 - Math.pow(0.002, dt / (cfg.vim.scroll_duration_ms || 150));
  const el = glide.el;
  // Whole-pixel steps (at least 1px), finishing exactly on the remaining distance.
  const step = (v) => Math.abs(v) <= 1.5 ? Math.round(v) : Math.round(v * f) || Math.sign(v);
  const dx = step(glide.x), dy = step(glide.y);
  const bx = el.scrollLeft, by = el.scrollTop;
  el.scrollBy({ left: dx, top: dy, behavior: "instant" });
  // Count what actually moved (browsers drop fractions); no movement at all means we hit an edge.
  const mx = el.scrollLeft - bx, my = el.scrollTop - by;
  glide.x = dx && !mx ? 0 : glide.x - mx;
  glide.y = dy && !my ? 0 : glide.y - my;
  if (Math.abs(glide.x) < 0.5 && Math.abs(glide.y) < 0.5) { glide.raf = 0; return endGlide(); }
  glide.raf = requestAnimationFrame(glideFrame);
}
// Holding a scroll key: continuous scrolling that starts gently and speeds up until the key is released.
const hold = { key: null, dx: 0, dy: 0, v: 0, acc: 0, last: 0, raf: 0, el: null };
function startHold(key, dx, dy) {
  if (hold.key) return;
  const el = scroller();
  Object.assign(hold, { key, dx, dy, v: cfg.vim.hold_scroll_speed || 700, acc: 0, last: 0, el });
  hold.saved = el.style.scrollBehavior; el.style.scrollBehavior = "auto";
  hold.raf = requestAnimationFrame(holdFrame);
}
function holdFrame(now) {
  const dt = (hold.last ? Math.max(1, Math.min(now - hold.last, 48)) : 16) / 1000; hold.last = now;
  hold.v = Math.min(cfg.vim.hold_scroll_max_speed || 6000, hold.v + (cfg.vim.hold_scroll_acceleration || 2400) * dt);
  hold.acc += hold.v * dt;
  const px = Math.trunc(hold.acc);
  if (px) {
    const el = hold.el, bx = el.scrollLeft, by = el.scrollTop;
    el.scrollBy({ left: px * hold.dx, top: px * hold.dy, behavior: "instant" });
    hold.acc -= px;
    if (el.scrollLeft === bx && el.scrollTop === by) return stopHold(false); // reached the edge
  }
  hold.raf = requestAnimationFrame(holdFrame);
}
function stopHold(coast = true) {
  if (!hold.key) return;
  cancelAnimationFrame(hold.raf);
  const el = hold.el, carry = coast && cfg.vim.smooth_scroll !== false ? hold.v * 0.06 : 0;
  if (el) el.style.scrollBehavior = hold.saved;
  const { dx, dy } = hold;
  Object.assign(hold, { key: null, raf: 0, el: null });
  // Ease to a stop rather than halting dead.
  if (carry > 1) scrollBy(carry * dx, carry * dy);
}
let holdTimer = 0;
const keysDown = new Set();
addEventListener("keydown", (e) => { if (e.key.length === 1) keysDown.add(e.key.toLowerCase()); }, true);
addEventListener("keyup", (e) => {
  const k = e.key.toLowerCase();
  keysDown.delete(k);
  if (hold.key && k === hold.key) stopHold();
}, true);
addEventListener("blur", () => keysDown.clear());
addEventListener("blur", () => stopHold(false));
document.addEventListener("visibilitychange", () => stopHold(false));

function scrollToEdge(top) {
  const s = scroller();
  endGlide();
  s.scrollTo({ top: top ? 0 : s.scrollHeight, behavior: cfg.vim.smooth_scroll === false ? "instant" : "smooth" });
}

// ---------- link hints ----------
// One pass over every element (including open shadow roots) reading only geometry, so the page lays out once;
// style and hit-test checks then run only for the few hundred candidates actually on screen.
let hints = null;
const CLICK_ROLES = new Set(["button", "link", "tab", "menuitem", "menuitemcheckbox", "menuitemradio", "checkbox", "radio", "switch",
  "option", "treeitem", "combobox", "textbox", "searchbox", "slider", "spinbutton", "gridcell", "row"]);
const FORM_TAGS = new Set(["INPUT", "TEXTAREA", "SELECT"]);
function clickableKind(el) {
  const tag = el.tagName;
  if (el.disabled || el.getAttribute("aria-disabled") === "true") return 0;
  if (tag === "A") return el.hasAttribute("href") || el.hasAttribute("onclick") ? 2 : 0;
  if (tag === "INPUT") return el.type === "hidden" ? 0 : 2;
  if (tag === "BUTTON" || tag === "SELECT" || tag === "TEXTAREA" || tag === "SUMMARY" || tag === "AREA") return 2;
  if (tag === "LABEL") return el.control ? 2 : 0;
  if ((tag === "VIDEO" || tag === "AUDIO") && el.controls) return 2;
  if (el.isContentEditable && !(el.parentElement && el.parentElement.isContentEditable)) return 2;
  const role = el.getAttribute("role");
  if (role && CLICK_ROLES.has(role)) return 2;
  if (el.hasAttribute("onclick") || el.hasAttribute("onmousedown") || el.hasAttribute("jsaction") || el.hasAttribute("aria-haspopup")) return 2;
  const ti = el.getAttribute("tabindex");
  if (ti !== null && +ti >= 0) return 2;
  return 1; // maybe: decided by `cursor: pointer` below
}
function hitsElement(el, r) {
  // Sample a few points: an overlay covering the centre (a play icon on a thumbnail) shouldn't hide the element.
  const root = el.getRootNode && el.getRootNode().elementFromPoint ? el.getRootNode() : document;
  const pts = [[r.left + r.width / 2, r.top + r.height / 2], [r.left + 3, r.top + 3], [r.right - 3, r.bottom - 3], [r.left + r.width / 2, r.top + 3]];
  for (const [x, y] of pts) {
    if (x < 0 || y < 0 || x >= innerWidth || y >= innerHeight) continue;
    const hit = root.elementFromPoint(x, y);
    if (!hit) continue;
    if (hit === el || el.contains(hit) || hit.contains(el)) return true;
    // A shadow host returned from the document: look inside it.
    if (hit.shadowRoot && hit.contains(el.getRootNode().host || el)) return true;
  }
  return false;
}
function collectHintables() {
  const vw = innerWidth, vh = innerHeight, found = [], seen = new Set();
  const onScreen = (r) => r.width >= 3 && r.height >= 3 && r.bottom >= 1 && r.right >= 1 && r.top <= vh - 1 && r.left <= vw - 1;
  // 1. Real controls (links, buttons, inputs, roles…): cheap tag/attribute test first, geometry only for matches.
  const roots = [document];
  for (let i = 0; i < roots.length; i++) {
    for (const el of roots[i].querySelectorAll("*")) {
      if (el.shadowRoot) roots.push(el.shadowRoot);
      if (clickableKind(el) !== 2) continue;
      const r = el.getBoundingClientRect();
      if (onScreen(r)) { found.push({ el, r, kind: 2 }); seen.add(el); }
    }
  }
  // 2. Things that only look clickable (cursor: pointer). Walk the tree but skip whole subtrees that are off-screen,
  //    so huge pages cost about as much as what's visible.
  const walk = (el) => {
    const r = el.getBoundingClientRect();
    const off = r.height > 0 && r.width > 0 && (r.bottom < 0 || r.top > vh || r.right < 0 || r.left > vw);
    if (off && getComputedStyle(el).position === "static") return;
    if (!seen.has(el) && onScreen(r) && clickableKind(el) === 1) found.push({ el, r, kind: 1 });
    for (let c = el.firstElementChild; c; c = c.nextElementSibling) walk(c);
    if (el.shadowRoot) for (let c = el.shadowRoot.firstElementChild; c; c = c.nextElementSibling) walk(c);
  };
  if (document.body) walk(document.body);
  const chosen = new Set(), out = [], styles = new Map();
  const style = (el) => { let s = styles.get(el); if (!s) { s = getComputedStyle(el); styles.set(el, s); } return s; };
  found.sort((a, b) => (a.el.compareDocumentPosition(b.el) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1));
  for (const c of found) {
    const s = style(c.el);
    if (s.visibility !== "visible" || +s.opacity < 0.05 || s.pointerEvents === "none") continue;
    if (c.kind === 1) {
      // The outermost `cursor: pointer` element that isn't a giant wrapper.
      if (s.cursor !== "pointer") continue;
      const p = c.el.parentElement;
      if (p && style(p).cursor === "pointer") continue;
      if (c.r.width * c.r.height > vw * vh * 0.4) continue;
    }
    // One hint per target: skip things nested inside an already-hinted link/button (form fields excepted).
    let dup = false;
    for (let a = c.el.parentElement, n = 0; a && n < 6; a = a.parentElement, n++) {
      if (chosen.has(a) && !FORM_TAGS.has(c.el.tagName)) { dup = true; break; }
    }
    if (dup || !hitsElement(c.el, c.r)) continue;
    chosen.add(c.el);
    out.push(c);
  }
  return out;
}
function labels(n) {
  const chars = (cfg.vim.hint_characters || "sadfjklewcmpgh").toLowerCase();
  let out = [""];
  while (out.length < n || (out.length === 1 && out[0] === "")) {
    const prefix = out.shift();
    for (const c of chars) out.push(prefix + c);
  }
  return out.slice(0, n);
}
function startHints(newTab) {
  const t0 = performance.now();
  const items = collectHintables();
  if (!items.length) return toast("Nothing to click");
  // Reading order: top to bottom, then left to right, so nearby things get similar labels.
  items.sort((a, b) => Math.round(a.r.top / 8) - Math.round(b.r.top / 8) || a.r.left - b.r.left);
  const root = ui(), ls = labels(items.length), frag = document.createDocumentFragment();
  hints = { newTab, typed: "", items: items.map(({ el, r }, i) => {
    const d = document.createElement("div"); d.className = "hint"; d.textContent = ls[i];
    d.style.transform = `translate(${Math.max(r.left - 2, 0)}px, ${Math.max(r.top - 2, 0)}px)`;
    frag.appendChild(d);
    return { e: el, label: ls[i], d };
  }), ms: performance.now() - t0 };
  root.appendChild(frag);
  mode = "hints";
}
function stopHints() { if (hints) for (const h of hints.items) h.d.remove(); hints = null; mode = "normal"; }
function hintKey(k) {
  if (k === "Escape") return stopHints();
  if (k === "Backspace") hints.typed = hints.typed.slice(0, -1);
  else if (k.length === 1) hints.typed += k.toLowerCase();
  else return;
  const typed = hints.typed;
  let left = 0, last = null;
  for (const h of hints.items) {
    const on = h.label.startsWith(typed);
    if (on) { left++; last = h; }
    if (on !== !h.hidden) { h.hidden = !on; h.d.style.display = on ? "" : "none"; }
    if (on) {
      // DOM building, not innerHTML: sites with Trusted Types (YouTube, Google) reject innerHTML.
      const t = document.createElement("span"); t.className = "typed"; t.textContent = typed;
      h.d.replaceChildren(t, h.label.slice(typed.length));
    }
  }
  if (left === 1 && last.label === typed) { const nt = hints.newTab; stopHints(); activate(last.e, nt); }
  else if (!left) stopHints();
}
// A real-looking click: frameworks (React, YouTube's Polymer, menus) often act on pointer/mouse down, not click.
function activate(e, newTab) {
  const a = e.closest("a[href]");
  if (newTab && a && /^https?:/.test(a.href)) return post({ t: "open", url: a.href });
  const tag = e.tagName;
  const textual = tag === "TEXTAREA" || e.isContentEditable || (tag === "INPUT" && !/^(button|submit|reset|checkbox|radio|file|image|color|range)$/i.test(e.type));
  userActed = true; // choosing a hint is deliberate, so it may focus a text box
  if (textual) { e.focus(); if (e.select) try { e.select(); } catch (_) {} return; }
  if (tag === "SELECT") { e.focus(); try { e.showPicker(); } catch (_) {} return; }
  const r = e.getBoundingClientRect();
  const x = r.left + Math.min(r.width / 2, 40), y = r.top + r.height / 2;
  const base = { bubbles: true, cancelable: true, composed: true, view: window, clientX: x, clientY: y, screenX: x, screenY: y, button: 0 };
  const ptr = { ...base, pointerId: 1, pointerType: "mouse", isPrimary: true };
  e.dispatchEvent(new PointerEvent("pointerover", ptr)); e.dispatchEvent(new MouseEvent("mouseover", base));
  e.dispatchEvent(new PointerEvent("pointerdown", { ...ptr, buttons: 1 })); e.dispatchEvent(new MouseEvent("mousedown", { ...base, buttons: 1 }));
  try { e.focus({ preventScroll: true }); } catch (_) {}
  e.dispatchEvent(new PointerEvent("pointerup", ptr)); e.dispatchEvent(new MouseEvent("mouseup", base));
  e.click();
}

// ---------- find: every match highlighted, a count, and a minimap of the whole page ----------
const fnd = { bar: null, input: null, count: null, items: [], cur: -1, map: null, sil: null, k: 1, timer: 0, onScroll: null, raf: 0 };
const canHighlight = typeof Highlight !== "undefined" && typeof CSS !== "undefined" && CSS.highlights;
function findScroller() { const s = scroller(); return s === document.body ? document.scrollingElement : s; }
function docY(rect, s) { return s === document.scrollingElement ? rect.top + s.scrollTop : rect.top - s.getBoundingClientRect().top + s.scrollTop; }
function ensureFindStyle() {
  if (document.getElementById("__valerie_find_style")) return;
  const st = document.createElement("style"); st.id = "__valerie_find_style";
  st.textContent = "::highlight(valerie-find){background-color:rgba(255,206,64,.55);color:inherit}::highlight(valerie-find-current){background-color:#ff9632;color:#000}";
  (document.head || document.documentElement).appendChild(st);
}
function searchPage(q) {
  const out = [], needle = q.toLowerCase(), s = findScroller();
  if (!needle || !document.body) return out;
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, { acceptNode(n) {
    const t = n.parentElement && n.parentElement.tagName;
    return !t || t === "SCRIPT" || t === "STYLE" || t === "NOSCRIPT" || t === "TEXTAREA" ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT;
  } });
  for (let n = walker.nextNode(); n && out.length < 3000; n = walker.nextNode()) {
    const text = n.data.toLowerCase();
    for (let i = text.indexOf(needle); i !== -1 && out.length < 3000; i = text.indexOf(needle, i + needle.length)) {
      const range = document.createRange(); range.setStart(n, i); range.setEnd(n, i + needle.length);
      const r = range.getBoundingClientRect();
      if (r.width || r.height) out.push({ range, y: docY(r, s), h: r.height });
    }
  }
  return out;
}
function paintHighlights() {
  if (!canHighlight) {
    const cur = fnd.items[fnd.cur];
    const sel = window.getSelection(); sel.removeAllRanges(); if (cur) sel.addRange(cur.range);
    return;
  }
  if (cfg.find && cfg.find.highlight_all === false) CSS.highlights.delete("valerie-find");
  else CSS.highlights.set("valerie-find", new Highlight(...fnd.items.map((m) => m.range)));
  const cur = fnd.items[fnd.cur];
  if (cur) CSS.highlights.set("valerie-find-current", new Highlight(cur.range)); else CSS.highlights.delete("valerie-find-current");
}
function goToMatch(i) {
  const n = fnd.items.length;
  if (!n) { fnd.cur = -1; fnd.count.textContent = fnd.input.value ? "No results" : ""; paintHighlights(); drawMinimap(); return; }
  fnd.cur = ((i % n) + n) % n;
  const m = fnd.items[fnd.cur], s = findScroller();
  const view = s === document.scrollingElement ? innerHeight : s.clientHeight;
  if (m.y < s.scrollTop + 40 || m.y + m.h > s.scrollTop + view - 40) {
    s.scrollTo({ top: Math.max(0, m.y - view / 2), behavior: cfg.vim.smooth_scroll === false ? "instant" : "smooth" });
  }
  fnd.count.textContent = (fnd.cur + 1) + " of " + n;
  paintHighlights(); drawMinimap();
}
function runSearch() {
  fnd.query = fnd.input.value;
  fnd.items = searchPage(fnd.query);
  // Start from the first match at or below the top of the screen, like other browsers.
  const s = findScroller(), top = s.scrollTop;
  const first = fnd.items.findIndex((m) => m.y >= top);
  goToMatch(first === -1 ? 0 : first);
}
// The minimap: a silhouette of the page's text and images, match marks, and the visible area.
function buildSilhouette() {
  if (!fnd.map) return;
  const s = findScroller(), dpr = devicePixelRatio || 1;
  const W = fnd.map.clientWidth, H = fnd.map.clientHeight, total = Math.max(s.scrollHeight, 1);
  fnd.k = Math.min(H / total, 0.25);
  const kx = W / Math.max(s.scrollWidth || innerWidth, 1);
  const off = document.createElement("canvas"); off.width = W * dpr; off.height = H * dpr;
  const g = off.getContext("2d"); g.scale(dpr, dpr);
  const blocks = document.body ? document.body.querySelectorAll("p,h1,h2,h3,h4,h5,h6,li,pre,blockquote,td,th,dt,dd,img,video,figure,input,button,textarea") : [];
  let n = 0;
  for (const el of blocks) {
    if (++n > 15000) break;
    const r = el.getBoundingClientRect();
    if (r.width < 2 || r.height < 2) continue;
    const y = docY(r, s) * fnd.k, x = (r.left + (s === document.scrollingElement ? s.scrollLeft : 0)) * kx;
    const media = /^(IMG|VIDEO|FIGURE|SVG)$/i.test(el.tagName), heading = /^H[1-6]$/.test(el.tagName);
    g.fillStyle = media ? "rgba(255,255,255,.14)" : heading ? "rgba(255,255,255,.55)" : "rgba(255,255,255,.28)";
    if (media) g.fillRect(x, y, Math.max(r.width * kx, 2), Math.max(r.height * fnd.k, 1));
    else {
      // Text reads as lines: one bar per line of the block, like a code minimap.
      const lineH = Math.max(r.height * fnd.k / Math.max(1, Math.round(r.height / 22)), 0.8);
      for (let ly = y; ly < y + r.height * fnd.k - 0.2; ly += lineH * 1.6) g.fillRect(x, ly, Math.max(r.width * kx * 0.92, 2), Math.min(lineH, 2));
    }
  }
  fnd.sil = off;
}
function drawMinimap() {
  if (!fnd.map) return;
  cancelAnimationFrame(fnd.raf);
  fnd.raf = requestAnimationFrame(() => {
    const c = fnd.map, dpr = devicePixelRatio || 1, W = c.clientWidth, H = c.clientHeight, s = findScroller();
    if (c.width !== W * dpr || c.height !== H * dpr) { c.width = W * dpr; c.height = H * dpr; }
    const g = c.getContext("2d"); g.setTransform(dpr, 0, 0, dpr, 0, 0); g.clearRect(0, 0, W, H);
    if (fnd.sil) g.drawImage(fnd.sil, 0, 0, W, H);
    const view = s === document.scrollingElement ? innerHeight : s.clientHeight;
    g.fillStyle = "rgba(255,255,255,.10)"; g.fillRect(0, s.scrollTop * fnd.k, W, Math.max(view * fnd.k, 6));
    g.strokeStyle = "rgba(255,255,255,.35)"; g.lineWidth = 1; g.strokeRect(0.5, s.scrollTop * fnd.k + 0.5, W - 1, Math.max(view * fnd.k, 6) - 1);
    // Matches as slim ticks on the right (like an editor's overview ruler) so the page silhouette stays readable.
    g.fillStyle = "rgba(255,206,64,.85)";
    for (const m of fnd.items) g.fillRect(W * 0.55, m.y * fnd.k - 0.75, W * 0.45, 1.5);
    const cur = fnd.items[fnd.cur];
    if (cur) { g.fillStyle = "#ff9632"; g.fillRect(0, cur.y * fnd.k - 1.5, W, 3); }
  });
}
function minimapScrollTo(e) {
  const s = findScroller(), r = fnd.map.getBoundingClientRect();
  const view = s === document.scrollingElement ? innerHeight : s.clientHeight;
  s.scrollTo({ top: Math.max(0, (e.clientY - r.top) / fnd.k - view / 2), behavior: "instant" });
}
function openFind(initial) {
  const root = ui();
  ensureFindStyle();
  if (!fnd.bar) {
    const minimap = !cfg.find || cfg.find.minimap !== false;
    fnd.bar = document.createElement("div"); fnd.bar.className = "find" + (minimap ? " with-map" : "");
    fnd.input = document.createElement("input"); fnd.input.placeholder = "Find in page";
    fnd.count = document.createElement("span"); fnd.count.className = "count";
    fnd.bar.append(fnd.input, fnd.count);
    root.appendChild(fnd.bar);
    fnd.input.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Enter") {
        e.preventDefault(); clearTimeout(fnd.timer);
        if (fnd.query !== fnd.input.value) runSearch(); else goToMatch(fnd.cur + (e.shiftKey ? -1 : 1));
      }
      if (e.key === "Escape") { e.preventDefault(); closeFind(); }
    }, true);
    fnd.input.addEventListener("input", () => { clearTimeout(fnd.timer); fnd.timer = setTimeout(runSearch, 90); });
    if (minimap) {
      fnd.map = document.createElement("canvas"); fnd.map.className = "minimap";
      root.appendChild(fnd.map);
      let dragging = false;
      fnd.map.addEventListener("mousedown", (e) => { dragging = true; minimapScrollTo(e); e.preventDefault(); });
      addEventListener("mousemove", (e) => { if (dragging) minimapScrollTo(e); }, true);
      addEventListener("mouseup", () => { dragging = false; }, true);
      // Draw the page silhouette after the find box is on screen, so typing never waits for it.
      setTimeout(() => { buildSilhouette(); drawMinimap(); }, 30);
    }
    fnd.onScroll = () => drawMinimap();
    addEventListener("scroll", fnd.onScroll, true);
    addEventListener("resize", () => { if (fnd.bar) { buildSilhouette(); runSearch(); } });
  }
  if (initial) fnd.input.value = initial;
  if (fnd.input.value) runSearch(); else drawMinimap();
  setTimeout(() => { fnd.input.focus(); fnd.input.select(); }, 0);
}
function closeFind() {
  if (!fnd.bar) return;
  fnd.bar.remove(); if (fnd.map) fnd.map.remove();
  removeEventListener("scroll", fnd.onScroll, true);
  if (canHighlight) { CSS.highlights.delete("valerie-find"); CSS.highlights.delete("valerie-find-current"); }
  // Leave the current match selected so ⌘C copies it, like other browsers.
  const cur = fnd.items[fnd.cur];
  if (cur) { const sel = window.getSelection(); sel.removeAllRanges(); sel.addRange(cur.range); }
  Object.assign(fnd, { bar: null, input: null, count: null, items: [], cur: -1, map: null, sil: null, query: "" });
}

// ---------- don't let pages grab focus before you've touched them (keeps vim keys working) ----------
let userActed = false;
addEventListener("mousedown", (e) => { if (e.isTrusted) userActed = true; }, true);
addEventListener("touchstart", (e) => { if (e.isTrusted) userActed = true; }, true);
document.addEventListener("focusin", (e) => {
  if (userActed || !cfg.vim.enabled || cfg.vim.block_autofocus === false || excluded()) return;
  const t = e.composedPath ? e.composedPath()[0] : e.target;
  if (editable(t)) t.blur();
}, true);

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
      case "top": return scrollToEdge(true);
      case "bottom": return scrollToEdge(false);
      case "hints": return startHints(false);
      case "hints_new_tab": return startHints(true);
      case "insert": mode = "insert"; return toast(cfg.escape_hints ? "-- INSERT -- (esc to leave)" : "-- INSERT --");
      case "find": return openFind();
    }
  }
  const cmd = typeof action === "string" ? { cmd: action === "yank_url" ? "copy_url" : action } : action;
  post({ t: "cmd", cmd });
}

window.addEventListener("keydown", (e) => {
  if (!cfg.vim.enabled || e.defaultPrevented && mode !== "hints") return;
  // Typing in Valerie's own UI (find box, prompts) lives in a closed shadow root the page can't see into.
  if (mode !== "hints" && inOurUi(e)) return;
  if (e.isTrusted && (e.key === "i" || e.key === "Tab" || e.key === "/")) userActed = true; // deliberate moves into fields
  if (mode === "hints") { e.preventDefault(); e.stopImmediatePropagation(); return hintKey(e.key); }
  if (e.metaKey || e.ctrlKey || e.altKey || e.isComposing) return;
  if (e.key === "Escape") {
    const a = deepActive();
    if (mode === "insert") { mode = "normal"; toast("-- NORMAL --"); }
    else if (!editable(a) && !buffer) post({ t: "cmd", cmd: { cmd: "close_peek" } });
    if (editable(a)) a.blur();
    buffer = ""; return;
  }
  if (mode === "insert" || editable(deepActive()) || excluded()) return;
  if (e.key.length !== 1) return;
  const b = cfg.vim.bindings, next = buffer + e.key;
  // Key repeat on a scroll key means it's being held: switch to accelerating continuous scrolling.
  const HOLDS = { scroll_down: [0, 1], scroll_up: [0, -1], scroll_left: [-1, 0], scroll_right: [1, 0] };
  if (!buffer && HOLDS[b[e.key]]) {
    e.preventDefault(); e.stopImmediatePropagation();
    if (cfg.vim.hold_to_accelerate === false) return run(b[e.key]);
    if (e.repeat || hold.key) return; // already gliding or holding
    // A tap glides one step; still down after hold_delay_ms means it's being held, without waiting for key repeat.
    run(b[e.key]);
    const key = e.key.toLowerCase(), dir = HOLDS[b[e.key]];
    clearTimeout(holdTimer);
    holdTimer = setTimeout(() => { if (keysDown.has(key)) startHold(key, ...dir); }, cfg.vim.hold_delay_ms ?? 180);
    return;
  }
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

// ---------- boosts (per-site CSS from the config dir; boost JS is run natively after load) ----------
// Plugin `matches` use the same rules as routes: hosts cover subdomains; patterns with "://" match the URL.
function globMatch(p, text) { return new RegExp("^" + p.toLowerCase().replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*") + "$").test(text.toLowerCase()); }
function pluginMatches(patterns) {
  if (!patterns || !patterns.length) return /^https?:/.test(location.href);
  const h = location.hostname;
  return patterns.some((p) => p.includes("://") ? globMatch(p, location.href) : globMatch(p, h) || globMatch("*." + p, h));
}
function applyBoostCss() {
  const h = location.hostname.toLowerCase(), b = cfg.boost_css || {};
  const css = Object.keys(b).filter((k) => h === k || h.endsWith("." + k)).map((k) => b[k])
    .concat((cfg.plugin_css || []).filter((p) => pluginMatches(p.matches)).map((p) => p.css)).join("\n");
  let st = document.getElementById("__valerie_boost");
  if (!css) { if (st) st.remove(); return; }
  if (!st) { st = document.createElement("style"); st.id = "__valerie_boost"; (document.head || document.documentElement).appendChild(st); }
  if (st.textContent !== css) st.textContent = css;
}
if (document.documentElement) applyBoostCss(); else document.addEventListener("DOMContentLoaded", applyBoostCss);

// ---------- reveal the hidden sidebar when the mouse touches the screen edge ----------
// Only a deliberate push toward the edge counts: when the sidebar hides, the page slides under a resting mouse.
let edgeAt = 0, edgeArmedAt = 0;
document.addEventListener("mousemove", (e) => {
  if (cfg.peek) { if (e.timeStamp - edgeAt > 400) { edgeAt = e.timeStamp; post({ t: "unpeek" }); } return; }
  if (!cfg.edge || e.timeStamp - edgeAt < 300 || performance.now() < edgeArmedAt) return;
  const hit = cfg.edge === "left" ? e.clientX <= 2 && e.movementX < 0 : e.clientX >= innerWidth - 3 && e.movementX > 0;
  if (hit) { edgeAt = e.timeStamp; post({ t: "edge" }); }
}, { passive: true });

// ---------- link gestures: middle / ⌘ / ⇧ / ⌥ clicks and Valerie's link menu ----------
const isMac = /Mac/.test(navigator.platform);
const linkAt = (e) => { const a = e.target.closest && e.target.closest("a[href]"); return a && /^https?:/.test(a.href) ? a : null; };
function gesture(e) {
  const L = cfg.links || {};
  if (e.button === 1) return L.middle_click;
  if (e.button !== 0 || (isMac && e.ctrlKey)) return null;
  const cmd = isMac ? e.metaKey : e.ctrlKey;
  const key = [cmd && "cmd", e.altKey && "option", e.shiftKey && "shift"].filter(Boolean).join("_");
  return key ? L[key + "_click"] : null;
}
function onLinkClick(e) {
  if (!e.isTrusted) return;
  const a = linkAt(e), action = a && gesture(e);
  if (!action || action === "default") return;
  e.preventDefault(); e.stopImmediatePropagation();
  post({ t: "open", url: a.href, mode: action });
}
document.addEventListener("click", onLinkClick, true);
document.addEventListener("auxclick", onLinkClick, true);
// Middle-click autoscroll / paste would otherwise start on mousedown.
document.addEventListener("mousedown", (e) => { if (e.isTrusted && e.button === 1 && linkAt(e) && (cfg.links || {}).middle_click !== "default") e.preventDefault(); }, true);

let linkMenu = null;
function hideLinkMenu() { if (linkMenu) { linkMenu.remove(); linkMenu = null; } }
function copyText(text) {
  const done = () => toast("Copied link");
  if (navigator.clipboard && navigator.clipboard.writeText) return navigator.clipboard.writeText(text).then(done, () => legacyCopy(text, done));
  legacyCopy(text, done);
}
function legacyCopy(text, done) {
  const t = document.createElement("textarea"); t.value = text; t.style.cssText = "position:fixed;opacity:0";
  document.documentElement.appendChild(t); t.select(); try { document.execCommand("copy"); done(); } catch (_) {} t.remove();
}
document.addEventListener("contextmenu", (e) => {
  if (!e.isTrusted || e.shiftKey || !(cfg.links || {}).context_menu) return;
  const a = linkAt(e);
  if (!a) return;
  e.preventDefault(); e.stopImmediatePropagation();
  hideLinkMenu(); hideMenu();
  const url = a.href;
  linkMenu = document.createElement("div"); linkMenu.className = "menu";
  const items = [
    ["Open Link in New Tab", "new_tab"], ["Open Link in Background", "background_tab"], ["Open in Split View", "split"],
    ["Peek", "peek"], ["Open in Little Window", "little_window"], ["Copy Link", "copy"],
  ];
  for (const [label, mode] of items) {
    const d = document.createElement("div"); d.textContent = label;
    d.addEventListener("mousedown", (ev) => {
      if (!ev.isTrusted || ev.button !== 0) return;
      ev.preventDefault(); hideLinkMenu();
      if (mode === "copy") copyText(url); else post({ t: "open", url, mode });
    });
    linkMenu.append(d);
  }
  ui().appendChild(linkMenu);
  const w = linkMenu.offsetWidth || 220, h = linkMenu.offsetHeight || 200;
  linkMenu.style.left = Math.min(e.clientX, innerWidth - w - 6) + "px";
  linkMenu.style.top = Math.min(e.clientY, innerHeight - h - 6) + "px";
}, true);
// Clicks inside our (closed) shadow UI only show the host element in the event path.
const inOurUi = (e) => e.composedPath && e.composedPath().some((n) => n.tagName === "VALERIE-UI");
for (const ev of ["mousedown", "scroll", "blur"]) addEventListener(ev, (e) => { if (linkMenu && !inOurUi(e)) hideLinkMenu(); }, true);
document.addEventListener("keydown", (e) => { if (e.key === "Escape") hideLinkMenu(); }, true);

// ---------- the pointer entering a page tells the sidebar to drop its hover states ----------
if (document.documentElement) document.documentElement.addEventListener("mouseenter", () => post({ t: "pointer" }));

// ---------- tell Valerie when this page gets focus (split view tracks the active pane) ----------
let focusedAt = 0;
const reportFocus = (e) => { if (!e.isTrusted || e.timeStamp - focusedAt < 200) return; focusedAt = e.timeStamp; post({ t: "focused" }); };
addEventListener("mousedown", reportFocus, true);
addEventListener("focus", reportFocus);

// ---------- media: mute and play/pause from Valerie, and what's playing for the sidebar mini player ----------
let tabMuted = false, lastMedia = null, pausedByUs = null;
document.addEventListener("play", (e) => {
  const m = e.target;
  if (!(m instanceof HTMLMediaElement)) return;
  if (tabMuted) m.muted = true;
  if (!m.muted || tabMuted) lastMedia = m;
  pausedByUs = null;
}, true);
document.addEventListener("volumechange", (e) => { if (tabMuted && e.target instanceof HTMLMediaElement && !e.target.muted) tabMuted = false; }, true);
function media(action) {
  const all = [...document.querySelectorAll("video, audio")];
  if (action === "mute") { tabMuted = !tabMuted; for (const m of all) m.muted = tabMuted; }
  else if (action === "toggle") {
    const m = lastMedia && lastMedia.isConnected ? lastMedia : all.find((x) => !x.paused);
    if (m && m.paused) m.play().catch(() => {});
    else if (m) { pausedByUs = m; m.pause(); }
  }
  report();
}
// "paused" only when Valerie paused it, so the mini player goes away when a video ends or you pause it on the page.
function mediaState() {
  if (!lastMedia || !lastMedia.isConnected) return null;
  return !lastMedia.paused ? "playing" : pausedByUs === lastMedia ? "paused" : null;
}

// ---------- a playing video pops out into picture-in-picture when you switch to another tab ----------
let autoPip = null;
document.addEventListener("visibilitychange", () => {
  if (document.hidden) {
    if (!(cfg.tabs && cfg.tabs.picture_in_picture_on_leave)) return;
    const v = [...document.querySelectorAll("video")].find((x) => !x.paused && !x.muted && x.readyState > 2 &&
      x.webkitSupportsPresentationMode && x.webkitSupportsPresentationMode("picture-in-picture") && x.webkitPresentationMode === "inline");
    if (v) { try { v.webkitSetPresentationMode("picture-in-picture"); autoPip = v; } catch (_) {} }
  } else if (autoPip) {
    try { if (autoPip.webkitPresentationMode === "picture-in-picture") autoPip.webkitSetPresentationMode("inline"); } catch (_) {}
    autoPip = null;
  }
});

// ---------- page metadata for the sidebar ----------
function favicon() {
  const links = [...document.querySelectorAll("link[rel~='icon'], link[rel='shortcut icon'], link[rel='apple-touch-icon']")];
  const svg = links.find((l) => /svg/.test(l.type || l.href));
  const best = svg || links.sort((a, b) => (parseInt(b.sizes?.value) || 16) - (parseInt(a.sizes?.value) || 16)).find((l) => (parseInt(l.sizes?.value) || 16) <= 64) || links[0];
  return best ? best.href : null;
}
let lastMeta = "";
function report() {
  const m = { t: "meta", favicon: favicon(), playing: [...document.querySelectorAll("video, audio")].some((v) => !v.paused && (!v.muted || tabMuted)),
    muted: tabMuted, media: mediaState(), url: location.href };
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

// ---------- reader view: the page's article as clean text ----------
let readerHost = null;
function articleRoot() {
  const a = document.querySelector("article");
  if (a && a.textContent.trim().length > 600) return a;
  // Otherwise the element holding the most paragraph text.
  const score = new Map();
  for (const p of document.querySelectorAll("p")) {
    const n = p.textContent.trim().length;
    if (n < 60 || !p.parentElement) continue;
    score.set(p.parentElement, (score.get(p.parentElement) || 0) + n);
    const gp = p.parentElement.parentElement;
    if (gp) score.set(gp, (score.get(gp) || 0) + n / 2);
  }
  let best = null, top = 0;
  for (const [el, s] of score) if (s > top) { top = s; best = el; }
  return top > 300 ? best : null;
}
function readerBlocks(root) {
  const out = [], SKIP = "nav, aside, footer, form, button, script, style, noscript, iframe, [aria-hidden=true], [role=navigation], [role=complementary]";
  const walk = (el) => {
    for (const c of el.children) {
      if (c.matches(SKIP) || (c.tagName !== "IMG" && !c.getClientRects().length)) continue;
      const tag = c.tagName;
      if (/^H[1-6]$/.test(tag)) out.push(["h" + Math.min(4, Math.max(2, +tag[1])), c.textContent]);
      else if (tag === "P" || tag === "BLOCKQUOTE" || tag === "PRE" || tag === "LI" || tag === "FIGCAPTION") out.push([tag.toLowerCase(), c.textContent]);
      else if (tag === "IMG") { if (c.currentSrc || c.src) out.push(["img", c.currentSrc || c.src, c.alt]); }
      else walk(c);
    }
  };
  walk(root);
  return out.filter(([k, t]) => k === "img" || (t && t.trim()));
}
function reader() {
  if (readerHost) { readerHost.remove(); readerHost = null; return; }
  const root = articleRoot();
  if (!root) return toast("No article on this page");
  const r = cfg.reader || {}, dark = matchMedia("(prefers-color-scheme: dark)").matches;
  const host = document.createElement("valerie-reader");
  host.style.cssText = `all: initial; position: fixed; inset: 0; z-index: 2147483646; overflow: auto; outline: none; display: block;
    background: ${dark ? "#1d1c21" : "#fbf8f2"};`;
  host.tabIndex = -1;
  const sh = host.attachShadow({ mode: "closed" }), st = document.createElement("style");
  const font = r.font === "sans" ? "-apple-system, system-ui, sans-serif" : '"New York", "Iowan Old Style", Charter, Georgia, serif';
  st.textContent = `
    main { max-width: ${r.width || 680}px; margin: 0 auto; padding: 56px 28px 140px; font: ${r.font_size || 19}px/1.65 ${font}; color: ${dark ? "#e6e3dc" : "#26231f"}; }
    .site { font: 600 12px/1 -apple-system, system-ui, sans-serif; letter-spacing: .6px; text-transform: uppercase; color: ${cfg.accent || "#9d8cff"}; margin-bottom: 14px; }
    h1 { font-size: 1.9em; line-height: 1.2; margin: 0 0 1.1em; } h2 { font-size: 1.35em; margin: 1.6em 0 .5em; } h3, h4 { font-size: 1.1em; margin: 1.4em 0 .4em; }
    p, li { margin: 0 0 1em; } ul { padding-left: 1.2em; } img { max-width: 100%; height: auto; border-radius: 8px; margin: .6em 0; }
    blockquote { margin: 1em 0; padding-left: 1em; border-left: 3px solid ${cfg.accent || "#9d8cff"}; opacity: .85; }
    pre { white-space: pre-wrap; font: .8em/1.5 ui-monospace, Menlo, monospace; padding: 12px; border-radius: 8px; background: ${dark ? "#2a2930" : "#efe9de"}; }
    .figcaption { font-size: .8em; opacity: .65; }`;
  const col = document.createElement("main"), site = document.createElement("div"), h1 = document.createElement("h1");
  site.className = "site"; site.textContent = location.hostname.replace(/^www\./, "");
  h1.textContent = ((root.querySelector("h1") || document.querySelector("h1") || {}).textContent || document.title).trim();
  col.append(site, h1);
  let list = null;
  for (const [k, v, alt] of readerBlocks(root)) {
    if (k !== "li") list = null;
    if (k === "img") { const im = document.createElement("img"); im.src = v; im.alt = alt || ""; col.append(im); continue; }
    const text = v.trim();
    if (/^h\d$/.test(k) && text === h1.textContent) continue;
    if (k === "li") { if (!list) { list = document.createElement("ul"); col.append(list); } const li = document.createElement("li"); li.textContent = text; list.append(li); continue; }
    const e = document.createElement(k === "figcaption" ? "p" : k);
    if (k === "figcaption") e.className = "figcaption";
    e.textContent = text; col.append(e);
  }
  sh.append(st, col);
  document.documentElement.append(host);
  readerHost = host; host.focus();
}
document.addEventListener("keydown", (e) => { if (readerHost && e.key === "Escape") { e.preventDefault(); e.stopPropagation(); reader(); } }, true);

// ---------- passwords: offer saved logins on focus, offer to save after submit ----------
let accounts = [], fillNonce = null, lastField = null, reportedForm = false, menu = null;
const visible = (e) => e && e.getClientRects().length > 0;
const pwFields = () => [...document.querySelectorAll("input[type=password]")].filter(visible);
function userFieldFor(pw) {
  const scope = (pw && pw.form) || document;
  let best = null;
  for (const i of scope.querySelectorAll("input")) {
    if (!/^(text|email|tel|)$/i.test(i.type) || !visible(i)) continue;
    if (!pw || i.compareDocumentPosition(pw) & Node.DOCUMENT_POSITION_FOLLOWING) best = i;
  }
  return best || document.querySelector("input[autocomplete=username], input[type=email]");
}
function checkLogin() {
  if (reportedForm || document.hidden) return;
  if (pwFields().length || document.querySelector("input[autocomplete~=username], input[type=email]")) { reportedForm = true; loginWatch.disconnect(); post({ t: "login_form" }); }
}
function hideMenu() { if (menu) { menu.remove(); menu = null; } }
function showAccounts(field) {
  hideMenu();
  if (!accounts.length) return;
  const r = field.getBoundingClientRect();
  menu = document.createElement("div"); menu.className = "menu";
  menu.style.left = Math.max(r.left, 4) + "px"; menu.style.top = r.bottom + 4 + "px";
  accounts.forEach((a, i) => {
    const d = document.createElement("div"); d.textContent = a.username || "(no username)";
    const sm = document.createElement("small"); sm.textContent = "Valerie · " + a.vault; d.append(sm);
    d.addEventListener("mousedown", (e) => { if (!e.isTrusted) return; e.preventDefault(); hideMenu(); post({ t: "fill", index: i, nonce: fillNonce }); });
    menu.append(d);
  });
  ui().appendChild(menu);
}
document.addEventListener("focusin", (e) => {
  const f = e.target;
  if (!(f instanceof HTMLInputElement)) return;
  lastField = f;
  if (!e.isTrusted || !accounts.length) return;
  if (f.type === "password" || f === userFieldFor(pwFields()[0]) || /username|email/.test(f.autocomplete) || f.type === "email") showAccounts(f);
}, true);
document.addEventListener("focusout", () => setTimeout(hideMenu, 150), true);
function setVal(el, v) {
  if (!el || v == null) return;
  el.focus(); valueSetter.call(el, v);
  el.dispatchEvent(new Event("input", { bubbles: true })); el.dispatchEvent(new Event("change", { bubbles: true }));
}
function fill(user, pw) {
  const pwField = lastField && lastField.type === "password" ? lastField : pwFields()[0];
  const userField = pwField ? userFieldFor(pwField) : lastField;
  if (userField && userField !== pwField) setVal(userField, user);
  if (pwField) setVal(pwField, pw);
}
// Multi-step logins ask for the username on one page and the password on the next.
document.addEventListener("input", (e) => {
  const f = e.target;
  if (e.isTrusted && f instanceof HTMLInputElement && f.type !== "password" && (f === userFieldFor(pwFields()[0]) || /username|email/.test(f.autocomplete) || f.type === "email")) {
    try { sessionStorage.setItem("__valerie_user", f.value); } catch (_) {}
  }
}, true);
let lastSent = "";
function captureCredential(scope) {
  const pw = [...((scope && scope.querySelectorAll) ? scope.querySelectorAll("input[type=password]") : pwFields())].find((p) => p.value);
  if (!pw) return;
  const uf = userFieldFor(pw);
  let username = uf && uf.value;
  if (!username) try { username = sessionStorage.getItem("__valerie_user") || ""; } catch (_) { username = ""; }
  const key = username + "\u0000" + pw.value;
  if (key === lastSent) return;
  lastSent = key;
  post({ t: "credential", username, password: pw.value });
}
document.addEventListener("submit", (e) => captureCredential(e.target), true);
document.addEventListener("click", (e) => {
  if (!e.isTrusted) return;
  const b = e.target.closest && e.target.closest("button, input[type=submit], [role=button]");
  if (b && pwFields().some((p) => p.value)) captureCredential(b.form || document);
}, true);
document.addEventListener("keydown", (e) => {
  const t = e.target;
  if (e.key === "Enter" && t instanceof HTMLInputElement && t.type === "password" && t.value) captureCredential(t.form || document);
}, true);
let offered = null;
function offerSave(host, username, nonce, vaults) {
  if (offered === nonce) return;
  offered = nonce;
  const box = document.createElement("div"); box.className = "save";
  const p = document.createElement("p"); p.textContent = "Save password" + (username ? " for " + username : "") + " on " + host + "?";
  box.append(p);
  const button = (label, cls, msg) => {
    const b = document.createElement("button"); b.textContent = label; if (cls) b.className = cls;
    b.addEventListener("click", (e) => { if (!e.isTrusted) return; box.remove(); if (msg) post(msg); });
    box.append(b);
  };
  button("Save", "primary", { t: "save_password", nonce, vault: vaults[0] });
  vaults.slice(1).forEach((v) => button("Save to " + v, "", { t: "save_password", nonce, vault: v }));
  button("Never", "", { t: "never_password", nonce });
  button("Not now", "", { t: "dismiss_password", nonce });
  ui().appendChild(box);
  setTimeout(() => box.remove(), 30000);
}
// Watch for login forms that appear later, but only while the tab is visible and until one is found.
let loginTimer = 0;
const loginWatch = new MutationObserver(() => { if (loginTimer) return; loginTimer = setTimeout(() => { loginTimer = 0; checkLogin(); }, 500); });
const watchLogin = () => {
  if (reportedForm || document.hidden) return loginWatch.disconnect();
  loginWatch.observe(document, { childList: true, subtree: true }); checkLogin();
};
watchLogin();
document.addEventListener("visibilitychange", watchLogin);

Object.defineProperty(window, "__valerie", {
  value: Object.freeze({
    closeFind, media, reader,
    configure: (c) => { if (c.edge && !cfg.edge) edgeArmedAt = performance.now() + 700; cfg = c; applyBoostCss(); }, toast, find: openFind, hints: startHints,
    accounts: (list, nonce) => { accounts = list; fillNonce = nonce; const f = document.activeElement; if (f instanceof HTMLInputElement && (f.type === "password" || f.type === "email")) showAccounts(f); },
    fill, offerSave,
  }),
  writable: false, configurable: false,
});
