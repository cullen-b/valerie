// Injected into every page as (function (CFG) { ... })(config). Pages can see and call this; it only talks to the
// browser through a whitelist of harmless messages.
if (window.__valerie || window.top !== window) return;
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
    if (on) h.d.innerHTML = '<span class="typed">' + typed + "</span>" + h.label.slice(typed.length);
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
  if (reportedForm) return;
  if (pwFields().length || document.querySelector("input[autocomplete~=username], input[type=email]")) { reportedForm = true; post({ t: "login_form" }); }
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
if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", checkLogin); else checkLogin();
let loginTimer = 0;
new MutationObserver(() => { if (reportedForm || loginTimer) return; loginTimer = setTimeout(() => { loginTimer = 0; checkLogin(); }, 500); })
  .observe(document, { childList: true, subtree: true });

Object.defineProperty(window, "__valerie", {
  value: Object.freeze({
    configure: (c) => { if (c.edge && !cfg.edge) edgeArmedAt = performance.now() + 700; cfg = c; applyBoostCss(); }, toast, find: openFind, hints: startHints,
    accounts: (list, nonce) => { accounts = list; fillNonce = nonce; const f = document.activeElement; if (f instanceof HTMLInputElement && (f.type === "password" || f.type === "email")) showAccounts(f); },
    fill, offerSave,
  }),
  writable: false, configurable: false,
});
