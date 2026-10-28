// Valerie's PDF viewer (pages.pdf_viewer): pdf.js draws the pages (text stays selectable and findable) and a layer on
// top holds your marks: text, a signature, arrows. Save writes them into a copy of the PDF with pdf-lib, which goes to
// your downloads. Valerie injects it into a PDF tab after ui/vendor/pdf.worker.js, pdf.js and pdf-lib.js, as
// (function (CFG, LOCAL) { ... })(page settings, the file's bytes in base64 for a PDF on this computer, else null).
if (window.__valeriePdf) return;
window.__valeriePdf = true;
const lib = globalThis.pdfjsLib, PDFLib = globalThis.PDFLib;
const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; };
const NS = "http://www.w3.org/2000/svg";
const svg = (tag, attrs) => { const e = document.createElementNS(NS, tag); for (const k in attrs) e.setAttribute(k, attrs[k]); return e; };
const tr = (t) => (CFG.strings && CFG.strings[t]) || t;
const toast = (m) => window.__valerie && window.__valerie.toast(m);
let fileName = "document.pdf";
try { fileName = decodeURIComponent(location.pathname.split("/").pop() || "") || fileName; } catch (_) {}
if (!/\.pdf$/i.test(fileName)) fileName += ".pdf";

const CSS = `
:root { color-scheme: light; --bg: #e7e5e1; --bar: rgba(250,249,247,.86); --fg: #24222a; --dim: #6b6875; --line: rgba(20,16,32,.1); --soft: rgba(20,16,32,.05);
  --accent: ${CFG.accent || "#9d8cff"}; --ink: #d43f3a; }
:root.dark { color-scheme: dark; --bg: #1b1a1f; --bar: rgba(30,29,35,.86); --fg: #e9e7ef; --dim: #a29fac; --line: rgba(255,255,255,.1); --soft: rgba(255,255,255,.06); }
html, body { margin: 0; background: var(--bg); color: var(--fg); font: 13px/1.3 -apple-system, BlinkMacSystemFont, system-ui, sans-serif; }
.vpdf-bar { position: sticky; top: 0; z-index: 20; display: flex; align-items: center; gap: 6px; height: 46px; padding: 0 12px 0 16px; box-sizing: border-box;
  background: var(--bar); backdrop-filter: saturate(1.6) blur(16px); -webkit-backdrop-filter: saturate(1.6) blur(16px); border-bottom: 1px solid var(--line); user-select: none; -webkit-user-select: none; }
.vpdf-bar .name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
.vpdf-bar .grow { flex: 1; } .vpdf-bar .info { color: var(--dim); white-space: nowrap; font-variant-numeric: tabular-nums; padding: 0 6px; }
.vpdf-bar .group { display: flex; gap: 2px; padding: 2px; border-radius: 9px; background: var(--soft); }
.vpdf-bar button { all: unset; cursor: pointer; padding: 6px 10px; border-radius: 7px; color: var(--dim); font-weight: 550; white-space: nowrap; }
.vpdf-bar button:hover { color: var(--fg); background: var(--soft); }
.vpdf-bar button.on { color: var(--fg); background: color-mix(in srgb, var(--accent) 22%, transparent); }
.vpdf-bar button.primary { background: var(--accent); color: #fff; } .vpdf-bar button.primary:hover { filter: brightness(1.08); }
.vpdf-bar button:disabled { opacity: .35; cursor: default; }
.vpdf-bar .zoom { min-width: 44px; text-align: center; }
.vpdf-bar input[type=color] { width: 22px; height: 22px; border: 0; padding: 0; background: none; cursor: pointer; }
.vpdf-pages { display: flex; flex-direction: column; align-items: center; gap: 18px; padding: 24px 16px 120px; }
.vpdf-page { position: relative; background: #fff; box-shadow: 0 1px 3px rgba(0,0,0,.18), 0 8px 28px rgba(0,0,0,.12); border-radius: 2px; flex: none; }
.vpdf-page canvas { position: absolute; inset: 0; display: block; }
.vpdf-marks { position: absolute; left: 0; top: 0; transform-origin: 0 0; z-index: 3; pointer-events: none; }
body.tool .vpdf-marks { pointer-events: auto; cursor: crosshair; }
body.tool-text .vpdf-marks { cursor: text; }
.vpdf-marks svg { position: absolute; inset: 0; overflow: visible; }
.mk { position: absolute; pointer-events: auto; cursor: move; }
.mk.sel { outline: 1.5px solid var(--accent); outline-offset: 3px; border-radius: 2px; }
.mk.text { white-space: pre; font-family: Helvetica, Arial, sans-serif; line-height: 1.2; min-width: 4px; outline-offset: 2px; }
.mk.text[contenteditable=true] { cursor: text; outline: 1.5px dashed var(--accent); }
.mk.sign img { width: 100%; height: 100%; display: block; pointer-events: none; }
.mk .grip { position: absolute; right: -6px; bottom: -6px; width: 10px; height: 10px; border-radius: 3px; background: var(--accent); cursor: nwse-resize; display: none; }
.mk.sel .grip { display: block; }
.vpdf-marks line.hit { stroke: transparent; stroke-width: 14; pointer-events: stroke; cursor: move; }
.vpdf-marks g.sel line.shaft, .vpdf-marks g.sel polygon { filter: drop-shadow(0 0 2px var(--accent)); }
.vpdf-pad { position: fixed; inset: 0; z-index: 50; display: grid; place-items: center; background: rgba(0,0,0,.35); }
.vpdf-pad .card { width: min(520px, calc(100vw - 32px)); border-radius: 14px; background: var(--bar); backdrop-filter: blur(20px); -webkit-backdrop-filter: blur(20px);
  box-shadow: 0 30px 90px rgba(0,0,0,.4); padding: 16px; display: flex; flex-direction: column; gap: 12px; }
.vpdf-pad .card b { font-size: 14px; } .vpdf-pad canvas { width: 100%; height: 190px; border-radius: 10px; background: #fff; cursor: crosshair; touch-action: none; }
.vpdf-pad .row { display: flex; gap: 8px; justify-content: flex-end; } .vpdf-pad .row .grow { flex: 1; }
.vpdf-pad button { all: unset; cursor: pointer; padding: 7px 13px; border-radius: 8px; background: var(--soft); font-weight: 600; }
.vpdf-pad button.primary { background: var(--accent); color: #fff; }
.vpdf-pad .hint { color: var(--dim); font-size: 12px; }
.textLayer { color-scheme: only light; position: absolute; text-align: initial; inset: 0; overflow: clip; opacity: 1; line-height: 1; letter-spacing: normal;
  word-spacing: normal; -webkit-text-size-adjust: none; text-size-adjust: none; forced-color-adjust: none; transform-origin: 0 0; z-index: 2;
  --min-font-size: 1; --text-scale-factor: calc(var(--total-scale-factor) * var(--min-font-size)); --min-font-size-inv: calc(1 / var(--min-font-size)); }
.textLayer :is(span, br) { color: transparent; position: absolute; white-space: pre; cursor: text; transform-origin: 0% 0%; -webkit-user-select: text; user-select: text; }
.textLayer > :not(.markedContent), .textLayer .markedContent span:not(.markedContent) { z-index: 1; --font-height: 0;
  font-size: calc(var(--text-scale-factor) * var(--font-height)); --scale-x: 1; --rotate: 0deg;
  transform: rotate(var(--rotate)) scaleX(var(--scale-x)) scale(var(--min-font-size-inv)); }
.textLayer .markedContent { display: contents; }
.textLayer ::selection { background: color-mix(in srgb, AccentColor, transparent 50%); color: transparent; }
.textLayer br::selection { background: transparent; }
.textLayer .endOfContent { display: block; position: absolute; inset: 100% 0 0; z-index: 0; cursor: default; user-select: none; -webkit-user-select: none; }
@media print { .vpdf-bar { display: none; } .vpdf-pages { padding: 0; gap: 0; } .vpdf-page { box-shadow: none; } }`;

async function readBytes() {
  if (LOCAL) {
    const bin = atob(LOCAL), out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }
  const r = await fetch(location.href, { credentials: "include" });
  if (!r.ok) throw new Error("HTTP " + r.status);
  return new Uint8Array(await r.arrayBuffer());
}

async function start() {
  let original, doc;
  try {
    original = await readBytes();
    if (String.fromCharCode(...original.subarray(0, 1024)).indexOf("%PDF") < 0) throw new Error("not a PDF");
    doc = await lib.getDocument({ data: original.slice(), isEvalSupported: false, useSystemFonts: true }).promise;
  } catch (e) {
    // Leave the PDF as macOS shows it.
    console.warn("valerie pdf viewer:", e);
    window.__valeriePdfFailed = String(e && e.message || e);
    return;
  }
  // Agents' fetch_page reads the PDF's text through this: "## Page N" then that page's lines, up to `max` characters.
  window.__valeriePdfText = async (max) => {
    let out = "";
    for (let n = 1; n <= doc.numPages && out.length < max; n++) {
      const items = (await (await doc.getPage(n)).getTextContent()).items;
      let text = "", lastY = null;
      for (const it of items) {
        const y = it.transform ? it.transform[5] : null, h = it.height || 10;
        if (lastY !== null && y !== null && Math.abs(lastY - y) > h * 1.8 && !text.endsWith("\n\n")) text += text.endsWith("\n") ? "\n" : "\n\n";
        text += it.str + (it.hasEOL ? "\n" : "");
        if (y !== null) lastY = y;
      }
      out += `## Page ${n}\n\n${text.replace(/[ \t]+\n/g, "\n").replace(/\n{3,}/g, "\n\n").trim()}\n\n`;
    }
    return { pages: doc.numPages, text: out };
  };

  // ---------- the page ----------
  const head = el("head"), body = document.createElement("body"), style = el("style");
  style.textContent = CSS;
  const meta = el("meta"); meta.name = "viewport"; meta.content = "width=device-width, initial-scale=1";
  head.append(style, meta, el("title", null, fileName));
  document.documentElement.replaceChildren(head, body);
  const dark = matchMedia("(prefers-color-scheme: dark)");
  const theme = () => document.documentElement.classList.toggle("dark", dark.matches);
  theme(); dark.addEventListener("change", theme);

  const bar = el("header", "vpdf-bar"), info = el("span", "info"), zoomLabel = el("button", "zoom");
  const btn = (label, title, fn, cls) => { const b = el("button", cls, tr(label)); b.title = tr(title); b.onclick = fn; return b; };
  const tools = {
    select: btn("Select", "Select and move marks (Esc)", () => setTool("select")),
    text: btn("Text", "Click the page to add text", () => setTool("text")),
    sign: btn("Sign", "Draw your signature, then click where it goes", () => setTool("sign")),
    arrow: btn("Arrow", "Drag on the page to draw an arrow", () => setTool("arrow")),
  };
  const color = el("input"); color.type = "color"; color.value = localStorage.getItem("valerie-pdf-color") || "#d43f3a"; color.title = tr("Colour for text and arrows");
  color.oninput = () => { localStorage.setItem("valerie-pdf-color", color.value); if (selected && selected.color) { snapshot(); selected.color = color.value; drawMarks(selected.page); } };
  const undoBtn = btn("Undo", "Undo (⌘Z)", () => undo());
  const saveBtn = btn("Save Copy", "Save a copy with your marks to your downloads", () => save(), "primary");
  const toolGroup = el("div", "group"); toolGroup.append(...Object.values(tools));
  const zoomGroup = el("div", "group");
  zoomGroup.append(btn("−", "Zoom out", () => setZoom(zoom / 1.2)), zoomLabel, btn("+", "Zoom in", () => setZoom(zoom * 1.2)));
  zoomLabel.title = tr("Fit the width of the window"); zoomLabel.onclick = () => setZoom(fitZoom());
  bar.append(el("span", "name", fileName), el("span", "grow"), info, zoomGroup, toolGroup, color, undoBtn, saveBtn);
  const pagesEl = el("main", "vpdf-pages");
  body.append(bar, pagesEl);

  // ---------- pages: drawn when they come near the screen, again after zooming ----------
  const pages = [];
  for (let n = 1; n <= doc.numPages; n++) {
    const page = await doc.getPage(n), vp1 = page.getViewport({ scale: 1 });
    const box = el("div", "vpdf-page"), marks = el("div", "vpdf-marks"), canvas = el("canvas"), text = el("div", "textLayer");
    marks.style.width = vp1.width + "px"; marks.style.height = vp1.height + "px";
    const layer = svg("svg", { width: vp1.width, height: vp1.height });
    marks.append(layer);
    box.append(canvas, text, marks);
    box.dataset.n = n;
    pagesEl.append(box);
    pages.push({ n, page, vp1, box, canvas, text, marks, layer, drawn: 0, list: [] });
  }
  const maxWidth = Math.max(...pages.map((p) => p.vp1.width));
  const fitZoom = () => Math.max(0.3, Math.min(2.2, (innerWidth - 48) / maxWidth));
  let zoom = Math.min(1.6, fitZoom());
  const sizeAll = () => {
    for (const p of pages) {
      p.box.style.width = Math.floor(p.vp1.width * zoom) + "px"; p.box.style.height = Math.floor(p.vp1.height * zoom) + "px";
      p.marks.style.transform = `scale(${zoom})`;
    }
    zoomLabel.textContent = Math.round(zoom * 100) + "%";
  };
  async function draw(p) {
    if (p.drawn === zoom) return;
    const z = (p.drawn = zoom), vp = p.page.getViewport({ scale: z }), dpr = window.devicePixelRatio || 1;
    const c = el("canvas"); c.width = Math.floor(vp.width * dpr); c.height = Math.floor(vp.height * dpr);
    c.style.width = Math.floor(vp.width) + "px"; c.style.height = Math.floor(vp.height) + "px";
    try {
      await p.page.render({ canvasContext: c.getContext("2d"), canvas: c, viewport: vp, transform: dpr !== 1 ? [dpr, 0, 0, dpr, 0, 0] : undefined }).promise;
    } catch (e) { if (p.drawn === z) p.drawn = 0; return; }
    if (p.drawn !== z) return;
    p.canvas.replaceWith(c); p.canvas = c;
    const t = el("div", "textLayer");
    t.style.setProperty("--total-scale-factor", z);
    try { await new lib.TextLayer({ textContentSource: p.page.streamTextContent(), container: t, viewport: vp }).render(); } catch (_) {}
    if (p.drawn === z) { p.text.replaceWith(t); p.text = t; }
  }
  const near = new Set();
  const io = new IntersectionObserver((entries) => {
    for (const e of entries) { const p = pages[+e.target.dataset.n - 1]; if (e.isIntersecting) { near.add(p); draw(p); } else near.delete(p); }
  }, { rootMargin: "900px 0px" });
  pages.forEach((p) => io.observe(p.box));
  const setZoom = (z) => {
    const mid = (scrollY + innerHeight / 2) / Math.max(1, document.documentElement.scrollHeight);
    zoom = Math.max(0.3, Math.min(4, z)); sizeAll();
    scrollTo(scrollX, mid * document.documentElement.scrollHeight - innerHeight / 2);
    near.forEach(draw);
  };
  sizeAll();
  const showPage = () => {
    let cur = 1;
    for (const p of pages) { if (p.box.getBoundingClientRect().top < innerHeight / 2) cur = p.n; else break; }
    info.textContent = `${cur} / ${doc.numPages}`;
  };
  addEventListener("scroll", showPage, { passive: true }); showPage();
  addEventListener("resize", () => { if (Math.abs(zoom - fitZoom()) < 0.02) return; });

  // ---------- marks ----------
  // Each mark lives in its page's own units (PDF points, from the top-left), so zooming just scales the layer.
  let tool = "select", selected = null, history = [], dirty = false, signature = localStorage.getItem("valerie-signature");
  const snapshot = () => { history.push(JSON.stringify(pages.map((p) => p.list))); if (history.length > 100) history.shift(); undoBtn.disabled = false; dirty = true; };
  const undo = () => {
    const last = history.pop(); if (!last) return;
    JSON.parse(last).forEach((list, i) => { pages[i].list = list.map((m) => ({ ...m, page: pages[i].n })); });
    selected = null; pages.forEach((p) => drawMarks(p.n)); undoBtn.disabled = !history.length;
  };
  undoBtn.disabled = true;
  const setTool = (t) => {
    tool = t; if (t !== "select") select(null);
    for (const k in tools) tools[k].classList.toggle("on", k === t);
    body.classList.toggle("tool", t !== "select"); body.classList.toggle("tool-text", t === "text");
    if (t === "sign" && !signature) signaturePad();
  };
  setTool("select");
  const pageOf = (m) => pages[m.page - 1];
  const select = (m) => { selected = m; for (const p of pages) drawMarks(p.n); };
  const pointIn = (p, e) => { const r = p.marks.getBoundingClientRect(); return [(e.clientX - r.left) / zoom, (e.clientY - r.top) / zoom]; };
  const arrowHead = (m) => {
    const ang = Math.atan2(m.y2 - m.y1, m.x2 - m.x1), L = 6 + m.width * 3.2, spread = 0.45;
    return [[m.x2, m.y2], [m.x2 - L * Math.cos(ang - spread), m.y2 - L * Math.sin(ang - spread)], [m.x2 - L * Math.cos(ang + spread), m.y2 - L * Math.sin(ang + spread)]];
  };
  function drawMarks(n) {
    const p = pages[n - 1];
    p.marks.querySelectorAll(".mk").forEach((e) => e.remove());
    p.layer.replaceChildren();
    for (const m of p.list) {
      if (m.type === "arrow") {
        const g = svg("g", {}), head = arrowHead(m), ang = Math.atan2(m.y2 - m.y1, m.x2 - m.x1), back = (6 + m.width * 3.2) * 0.7;
        g.append(svg("line", { class: "shaft", x1: m.x1, y1: m.y1, x2: m.x2 - back * Math.cos(ang), y2: m.y2 - back * Math.sin(ang), stroke: m.color, "stroke-width": m.width, "stroke-linecap": "round" }),
          svg("polygon", { points: head.map((q) => q.join(",")).join(" "), fill: m.color }));
        const hit = svg("line", { class: "hit", x1: m.x1, y1: m.y1, x2: m.x2, y2: m.y2 });
        g.append(hit);
        if (m === selected) g.setAttribute("class", "sel");
        hit.addEventListener("pointerdown", (e) => grab(e, m));
        p.layer.append(g);
        continue;
      }
      const d = el("div", "mk " + m.type + (m === selected ? " sel" : ""));
      d.style.left = m.x + "px"; d.style.top = m.y + "px";
      if (m.type === "text") {
        d.textContent = m.text; d.style.fontSize = m.size + "px"; d.style.color = m.color;
        d.ondblclick = () => editText(d, m);
      } else {
        d.style.width = m.w + "px"; d.style.height = m.h + "px";
        const im = el("img"); im.src = m.src; im.alt = tr("Signature"); d.append(im);
        const grip = el("span", "grip"); d.append(grip);
        grip.addEventListener("pointerdown", (e) => resize(e, m));
      }
      d.addEventListener("pointerdown", (e) => { if (d.isContentEditable) return; grab(e, m); });
      p.marks.append(d);
      m.el = d;
    }
  }
  // Drag a mark to move it (a tiny drag is just a click that selects it).
  function grab(e, m) {
    if (tool !== "select") setTool("select");
    e.preventDefault(); e.stopPropagation();
    const p = pageOf(m), [sx, sy] = pointIn(p, e), start = JSON.stringify(m.type === "arrow" ? [m.x1, m.y1, m.x2, m.y2] : [m.x, m.y]);
    let moved = false;
    select(m);
    const move = (ev) => {
      const [x, y] = pointIn(p, ev), dx = x - sx, dy = y - sy, o = JSON.parse(start);
      if (!moved && Math.hypot(dx, dy) * zoom < 3) return;
      if (!moved) { moved = true; history.push(JSON.stringify(pages.map((q) => q.list.map((k) => ({ ...k, ...(k === m ? (m.type === "arrow" ? { x1: o[0], y1: o[1], x2: o[2], y2: o[3] } : { x: o[0], y: o[1] }) : {}) }))))); undoBtn.disabled = false; dirty = true; }
      if (m.type === "arrow") { m.x1 = o[0] + dx; m.y1 = o[1] + dy; m.x2 = o[2] + dx; m.y2 = o[3] + dy; } else { m.x = o[0] + dx; m.y = o[1] + dy; }
      drawMarks(m.page);
    };
    const up = () => { removeEventListener("pointermove", move); removeEventListener("pointerup", up); };
    addEventListener("pointermove", move); addEventListener("pointerup", up);
  }
  function resize(e, m) {
    e.preventDefault(); e.stopPropagation(); snapshot();
    const p = pageOf(m), [sx] = pointIn(p, e), w0 = m.w, ratio = m.h / m.w;
    const move = (ev) => { const [x] = pointIn(p, ev); m.w = Math.max(30, w0 + x - sx); m.h = m.w * ratio; drawMarks(m.page); };
    const up = () => { removeEventListener("pointermove", move); removeEventListener("pointerup", up); };
    addEventListener("pointermove", move); addEventListener("pointerup", up);
  }
  function editText(d, m) {
    select(m); d = m.el;
    d.contentEditable = "true"; d.focus();
    const range = document.createRange(); range.selectNodeContents(d); const s = getSelection(); s.removeAllRanges(); s.addRange(range);
    // The text is kept as it's typed: a toolbar click doesn't take the focus away in WebKit, it just redraws.
    let changed = false;
    d.addEventListener("input", () => { if (!changed) { snapshot(); changed = true; } m.text = d.innerText.replace(/\n$/, ""); });
    const done = () => {
      d.removeEventListener("blur", done);
      if (!m.text.trim()) { const p = pageOf(m); p.list = p.list.filter((k) => k !== m); selected = null; }
      d.contentEditable = "false"; drawMarks(m.page);
    };
    d.addEventListener("blur", done);
    d.addEventListener("keydown", (e) => { e.stopPropagation(); if (e.key === "Escape" || (e.key === "Enter" && (e.metaKey || e.ctrlKey))) { e.preventDefault(); d.blur(); } });
  }
  // Tools: a click places text or the signature; a drag draws an arrow.
  for (const p of pages) {
    p.marks.addEventListener("pointerdown", (e) => {
      if (tool === "select" || e.target.closest(".mk") || e.target.classList.contains("hit")) return;
      e.preventDefault(); e.stopPropagation();   // the click that places a mark isn't a click away from it
      const [x, y] = pointIn(p, e);
      if (tool === "text") {
        snapshot();
        const m = { type: "text", page: p.n, x, y: y - 9, text: tr("Text"), size: 14, color: color.value };
        p.list.push(m); drawMarks(p.n); editText(m.el, m); setTool("select");
      } else if (tool === "sign") {
        if (!signature) return signaturePad();
        const img = new Image();
        img.onload = () => {
          snapshot();
          const w = 160, h = (w * img.naturalHeight) / Math.max(1, img.naturalWidth);
          const m = { type: "sign", page: p.n, x: x - w / 2, y: y - h / 2, w, h, src: signature };
          p.list.push(m); setTool("select"); select(m);
        };
        img.src = signature;
      } else if (tool === "arrow") {
        const m = { type: "arrow", page: p.n, x1: x, y1: y, x2: x, y2: y, color: color.value, width: 2.5 };
        let added = false;
        const move = (ev) => {
          const [x2, y2] = pointIn(p, ev); m.x2 = x2; m.y2 = y2;
          if (!added && Math.hypot(x2 - x, y2 - y) > 4) { snapshot(); p.list.push(m); added = true; }
          if (added) drawMarks(p.n);
        };
        const up = () => { removeEventListener("pointermove", move); removeEventListener("pointerup", up); if (added) select(m); };
        addEventListener("pointermove", move); addEventListener("pointerup", up);
      }
    });
  }
  document.addEventListener("pointerdown", (e) => { if (selected && !e.target.closest(".mk") && !e.target.classList.contains("hit") && !e.target.closest(".vpdf-bar")) select(null); });
  document.addEventListener("keydown", (e) => {
    const typing = e.target.isContentEditable || /^(INPUT|TEXTAREA)$/.test(e.target.tagName);
    if ((e.metaKey || e.ctrlKey) && !e.shiftKey && e.key.toLowerCase() === "z" && !typing) { e.preventDefault(); e.stopPropagation(); undo(); }
    else if ((e.key === "Backspace" || e.key === "Delete") && selected && !typing) {
      e.preventDefault(); e.stopPropagation(); snapshot();
      const p = pageOf(selected); p.list = p.list.filter((k) => k !== selected); selected = null; drawMarks(p.n);
    } else if (e.key === "Enter" && selected && selected.type === "text" && !typing) { e.preventDefault(); editText(selected.el, selected); }
    else if (e.key === "Escape" && !typing && (tool !== "select" || selected)) { e.preventDefault(); e.stopPropagation(); setTool("select"); select(null); }
  }, true);

  // ---------- signature pad: draw it once; it's remembered for PDFs from this site ----------
  function signaturePad() {
    const pad = el("div", "vpdf-pad"), card = el("div", "card"), c = el("canvas"), row = el("div", "row");
    card.append(el("b", null, tr("Draw your signature")), c, row);
    const hint = el("span", "hint grow", signature ? tr("Replaces the one you saved") : tr("Kept for PDFs from this site"));
    const clear = el("button", null, tr("Clear")), cancel = el("button", null, tr("Cancel")), use = el("button", "primary", tr("Use"));
    row.append(hint, clear, cancel, use);
    pad.append(card); body.append(pad);
    const dpr = window.devicePixelRatio || 1, r = c.getBoundingClientRect();
    c.width = r.width * dpr; c.height = r.height * dpr;
    const g = c.getContext("2d"); g.scale(dpr, dpr); g.lineCap = g.lineJoin = "round"; g.strokeStyle = "#1a1a2e"; g.lineWidth = 2.6;
    let drawing = false, any = false, last = null;
    c.onpointerdown = (e) => { drawing = true; any = true; last = [e.offsetX, e.offsetY]; c.setPointerCapture(e.pointerId); };
    c.onpointermove = (e) => {
      if (!drawing) return;
      const q = [e.offsetX, e.offsetY], mid = [(last[0] + q[0]) / 2, (last[1] + q[1]) / 2];
      g.beginPath(); g.moveTo(last[0], last[1]); g.quadraticCurveTo(last[0], last[1], mid[0], mid[1]); g.lineTo(q[0], q[1]); g.stroke(); last = q;
    };
    c.onpointerup = () => (drawing = false);
    clear.onclick = () => { g.clearRect(0, 0, c.width, c.height); any = false; };
    const close = () => { pad.remove(); if (!signature) setTool("select"); };
    cancel.onclick = close;
    pad.onpointerdown = (e) => { if (e.target === pad) close(); };
    use.onclick = () => {
      if (!any) return close();
      signature = trimmed(c);
      try { localStorage.setItem("valerie-signature", signature); } catch (_) {}
      pad.remove(); setTool("sign"); toast("Click where your signature goes");
    };
  }
  // The drawn signature, cut to its ink, as a PNG.
  function trimmed(c) {
    const g = c.getContext("2d"), { width: w, height: h } = c, px = g.getImageData(0, 0, w, h).data;
    let x0 = w, y0 = h, x1 = 0, y1 = 0;
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) if (px[(y * w + x) * 4 + 3] > 8) { x0 = Math.min(x0, x); y0 = Math.min(y0, y); x1 = Math.max(x1, x); y1 = Math.max(y1, y); }
    if (x1 <= x0) return c.toDataURL("image/png");
    const pad = 6, out = el("canvas");
    out.width = x1 - x0 + pad * 2; out.height = y1 - y0 + pad * 2;
    out.getContext("2d").drawImage(c, x0 - pad, y0 - pad, out.width, out.height, 0, 0, out.width, out.height);
    return out.toDataURL("image/png");
  }

  // ---------- save a copy: the marks go into the PDF's pages ----------
  const rgb = (hex) => { const n = parseInt(hex.slice(1), 16); return PDFLib.rgb(((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255); };
  const dataBytes = (url) => { const bin = atob(url.split(",")[1]), out = new Uint8Array(bin.length); for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i); return out; };
  // Text in a script Helvetica can't write (Chinese, emoji…) goes in as a picture of the text instead.
  const textImage = (m) => {
    const c = el("canvas"), g = c.getContext("2d"), k = 4, lines = m.text.split("\n"), font = `${m.size * k}px Helvetica, Arial, sans-serif`;
    g.font = font;
    c.width = Math.ceil(Math.max(...lines.map((l) => g.measureText(l).width)) + 4); c.height = Math.ceil(lines.length * m.size * 1.2 * k + 4);
    g.font = font; g.fillStyle = m.color; g.textBaseline = "top";
    lines.forEach((l, i) => g.fillText(l, 0, i * m.size * 1.2 * k + m.size * 0.1 * k));
    return [c.toDataURL("image/png"), c.width / k, c.height / k];
  };
  async function save() {
    if (!pages.some((p) => p.list.length)) return toast("Add text, a signature or an arrow first");
    saveBtn.disabled = true; saveBtn.textContent = tr("Saving…");
    try {
      const pdf = await PDFLib.PDFDocument.load(original, { ignoreEncryption: true });
      const font = await pdf.embedFont(PDFLib.StandardFonts.Helvetica);
      for (const p of pages) {
        if (!p.list.length) continue;
        const pg = pdf.getPage(p.n - 1), at = (x, y) => p.vp1.convertToPdfPoint(x, y);
        // A box from its top-left corner and size, in the page's own (unrotated, bottom-up) coordinates.
        const box = (x, y, w, h) => { const [ax, ay] = at(x, y + h), [bx, by] = at(x + w, y); return { x: Math.min(ax, bx), y: Math.min(ay, by), width: Math.abs(bx - ax), height: Math.abs(by - ay) }; };
        for (const m of p.list) {
          if (m.type === "text") {
            let fits = true;
            try { font.encodeText(m.text.replace(/\n/g, "")); } catch (_) { fits = false; }
            if (fits) {
              m.text.split("\n").forEach((line, i) => {
                const [x, y] = at(m.x, m.y + m.size * (0.93 + i * 1.2));
                pg.drawText(line, { x, y, size: m.size, font, color: rgb(m.color), rotate: p.page.rotate ? PDFLib.degrees(-p.page.rotate) : undefined });
              });
            } else {
              const [src, w, h] = textImage(m);
              pg.drawImage(await pdf.embedPng(dataBytes(src)), box(m.x, m.y, w, h));
            }
          } else if (m.type === "sign") {
            pg.drawImage(await pdf.embedPng(dataBytes(m.src)), box(m.x, m.y, m.w, m.h));
          } else if (m.type === "arrow") {
            const ang = Math.atan2(m.y2 - m.y1, m.x2 - m.x1), back = (6 + m.width * 3.2) * 0.7;
            const [x1, y1] = at(m.x1, m.y1), [x2, y2] = at(m.x2 - back * Math.cos(ang), m.y2 - back * Math.sin(ang));
            pg.drawLine({ start: { x: x1, y: y1 }, end: { x: x2, y: y2 }, thickness: m.width, color: rgb(m.color), lineCap: PDFLib.LineCapStyle.Round });
            // drawSvgPath flips y around its origin, so points go in as (x, -y) from (0, 0).
            const head = arrowHead(m).map(([x, y]) => at(x, y)), path = "M " + head.map(([x, y]) => `${x} ${-y}`).join(" L ") + " Z";
            pg.drawSvgPath(path, { x: 0, y: 0, color: rgb(m.color), borderWidth: 0 });
          }
        }
      }
      const out = await pdf.save();
      const url = URL.createObjectURL(new Blob([out], { type: "application/pdf" })), a = el("a");
      a.href = url; a.download = fileName.replace(/\.pdf$/i, "") + " (marked up).pdf";
      body.append(a); a.click(); a.remove();
      setTimeout(() => URL.revokeObjectURL(url), 60000);
      dirty = false;
    } catch (e) {
      console.error("valerie pdf save", e);
      toast("Couldn't save the PDF: " + (e && e.message ? e.message : e));
    }
    saveBtn.disabled = false; saveBtn.textContent = tr("Save Copy");
  }
  addEventListener("beforeunload", (e) => { if (dirty) { e.preventDefault(); e.returnValue = ""; } });
}
// A PDF opened from this computer may never send DOMContentLoaded, so its state is checked too.
let started = false;
const go = () => { if (started || document.readyState === "loading") return; started = true; start(); };
document.addEventListener("DOMContentLoaded", go); addEventListener("load", go);
(function poll() { go(); if (!started) setTimeout(poll, 100); })();
