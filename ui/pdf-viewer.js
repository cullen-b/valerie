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
.vpdf-reader { position: fixed; inset: 0; z-index: 40; overflow: auto; overscroll-behavior: contain; outline: none; background: var(--rbg); color: var(--rfg);
  --rdim: color-mix(in srgb, var(--rfg) 62%, transparent); --rline: color-mix(in srgb, var(--rfg) 13%, transparent); --rsoft: color-mix(in srgb, var(--rfg) 7%, transparent); }
html.reading { overflow: hidden; }
.rd-bar { position: sticky; top: 0; z-index: 2; display: flex; align-items: center; gap: 6px; height: 46px; padding: 0 12px 0 16px; box-sizing: border-box;
  background: color-mix(in srgb, var(--rbg) 86%, transparent); backdrop-filter: saturate(1.4) blur(16px); -webkit-backdrop-filter: saturate(1.4) blur(16px);
  font: 13px/1.3 -apple-system, BlinkMacSystemFont, system-ui, sans-serif; user-select: none; -webkit-user-select: none; }
.rd-bar .name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; } .rd-bar .grow { flex: 1; }
.rd-bar .info { color: var(--rdim); white-space: nowrap; font-variant-numeric: tabular-nums; padding: 0 6px; }
.vpdf-reader button { all: unset; cursor: pointer; padding: 6px 10px; border-radius: 7px; color: var(--rdim); font: 550 13px/1.3 -apple-system, BlinkMacSystemFont, system-ui, sans-serif; white-space: nowrap; }
.vpdf-reader button:hover, .vpdf-reader button[aria-expanded=true] { color: var(--rfg); background: var(--rsoft); }
.vpdf-reader button:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
.rd-bar .aa { font-family: Georgia, serif; font-size: 15px; font-weight: 500; letter-spacing: -.2px; }
.rd-col { max-width: var(--rwidth); margin: 0 auto; padding: 40px 28px 160px; font: var(--rsize)/var(--rlh) var(--rfont); text-wrap: pretty; overflow-wrap: break-word; }
.rd-col h1 { font-size: 1.85em; line-height: 1.18; margin: .4em 0 1em; letter-spacing: -.01em; text-wrap: balance; }
.rd-col h2 { font-size: 1.35em; line-height: 1.25; margin: 1.7em 0 .55em; text-wrap: balance; }
.rd-col h3 { font-size: 1.1em; line-height: 1.3; margin: 1.5em 0 .45em; }
.rd-col p { margin: 0 0 1em; content-visibility: auto; contain-intrinsic-size: auto 6em; } .rd-col ul { margin: 0 0 1em; padding-left: 1.3em; } .rd-col li { margin: 0 0 .35em; }
.rd-col li.num { list-style: none; margin-left: -1.3em; }
.rd-col .small, .rd-col .caption { font-size: .84em; color: var(--rdim); }
.rd-col figure { margin: 1.4em 0; } .rd-col figure canvas { display: block; max-width: 100%; height: auto; border-radius: 4px; background: #fff; }
.vpdf-reader[data-dark] .rd-col figure canvas { filter: brightness(.88); }
.rd-col .status { color: var(--rdim); font-style: italic; }
.rd-col .pg { display: block; margin: 2.2em 0 1.2em; border-top: 1px solid var(--rline); font: 11px/1 -apple-system, system-ui, sans-serif; color: var(--rdim); padding-top: 6px; text-align: right; font-variant-numeric: tabular-nums; }
.rd-panel { position: fixed; top: 50px; right: 12px; z-index: 3; width: 272px; padding: 14px; box-sizing: border-box; border-radius: 14px; display: flex; flex-direction: column; gap: 14px;
  background: color-mix(in srgb, var(--rbg) 92%, var(--rfg) 4%); border: 1px solid var(--rline); box-shadow: 0 18px 50px rgba(0,0,0,.28);
  font: 12px/1.3 -apple-system, BlinkMacSystemFont, system-ui, sans-serif; }
.rd-panel .lbl { color: var(--rdim); margin-bottom: 6px; }
.rd-panel .seg { display: flex; gap: 2px; padding: 2px; border-radius: 9px; background: var(--rsoft); }
.rd-panel .seg button { flex: 1; text-align: center; padding: 6px 4px; font-size: 12px; }
.rd-panel .seg button[aria-pressed=true] { color: var(--rfg); background: color-mix(in srgb, var(--accent) 24%, transparent); }
.rd-panel .themes { display: flex; justify-content: space-between; }
.rd-panel .themes button { width: 38px; height: 38px; padding: 0; border-radius: 50%; display: grid; place-items: center; font: 500 14px/1 Georgia, serif;
  box-shadow: inset 0 0 0 1px var(--rline); }
.rd-panel .themes button[aria-pressed=true] { box-shadow: 0 0 0 2px var(--rbg), 0 0 0 4px var(--accent); }
.rd-panel .size { display: flex; align-items: center; gap: 6px; } .rd-panel .size output { flex: 1; text-align: center; font-variant-numeric: tabular-nums; color: var(--rfg); }
.rd-panel .size button { padding: 6px 12px; background: var(--rsoft); }
.rd-panel .reset { align-self: flex-start; padding: 2px 0; font-size: 12px; } .rd-panel .reset:hover { background: none; text-decoration: underline; }
@media (max-width: 520px) { .rd-col { padding: 28px 18px 120px; } .rd-bar .name { display: none; } }
@media print { .vpdf-reader { position: static; overflow: visible; } html.reading .vpdf-bar, html.reading .vpdf-pages, .rd-bar, .rd-panel { display: none !important; } }
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
  let picking = false;   // one undo step per trip to the colour picker, not one per colour it passes through
  color.onchange = () => { picking = false; };
  color.oninput = () => { localStorage.setItem("valerie-pdf-color", color.value); if (selected && selected.color) { if (!picking) snapshot(); picking = true; selected.color = color.value; drawMarks(selected.page); } };
  const undoBtn = btn("Undo", /Mac/.test(navigator.platform) ? "Undo (⌘Z)" : "Undo (Ctrl+Z)", () => undo());
  const readerBtn = btn("Reader", "Read the text in one column, in the type and colours you like", () => openReader());
  const saveBtn = btn("Save Copy", "Save a copy with your marks to your downloads", () => save(), "primary");
  const toolGroup = el("div", "group"); toolGroup.append(...Object.values(tools));
  const zoomGroup = el("div", "group");
  zoomGroup.append(btn("−", "Zoom out", () => setZoom(zoom / 1.2)), zoomLabel, btn("+", "Zoom in", () => setZoom(zoom * 1.2)));
  zoomLabel.title = tr("Fit the width of the window"); zoomLabel.onclick = () => setZoom(fitZoom());
  bar.append(el("span", "name", fileName), el("span", "grow"), info, ...(CFG.pages && CFG.pages.pdf_reader === false ? [] : [readerBtn]), zoomGroup, toolGroup, color, undoBtn, saveBtn);
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
  const snapshot = (state = JSON.stringify(pages.map((p) => p.list))) => { history.push(state); if (history.length > 100) history.shift(); undoBtn.disabled = false; dirty = true; };
  const undo = () => {
    if (document.activeElement && document.activeElement.isContentEditable) document.activeElement.blur();   // ends an edit, so it's undone first
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
  function editText(d, m, fresh) {
    select(m); d = m.el;
    d.contentEditable = "true"; d.focus();
    const range = document.createRange(); range.selectNodeContents(d); const s = getSelection(); s.removeAllRanges(); s.addRange(range);
    // The text is kept as it's typed: a toolbar click doesn't take the focus away in WebKit, it just redraws.
    // An edit is one undo step; a new box's typing is part of placing it, which already is one.
    const start = m.text, now = () => d.innerText.replace(/\n$/, "");
    let changed = false;
    d.addEventListener("input", () => { if (!changed && !fresh) snapshot(); changed = true; m.text = now(); dirty = true; });
    const done = () => {
      d.removeEventListener("blur", done);
      const gone = !m.text.trim();
      if (gone) { const p = pageOf(m); p.list = p.list.filter((k) => k !== m); selected = null; }
      if (fresh && gone) { history.pop(); undoBtn.disabled = !history.length; }
      d.contentEditable = "false"; drawMarks(m.page);
    };
    d.addEventListener("blur", done);
    d.addEventListener("keydown", (e) => {
      e.stopPropagation();
      // ⌘Z (Ctrl+Z) with nothing typed left to undo in the box ends the edit and undoes the last change to the document.
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "z" && now() === start) {
        e.preventDefault(); if (changed && !fresh) history.pop();   // typing undone back to the start leaves no step
        d.blur(); undo(); return;
      }
      if (e.key === "Escape" || (e.key === "Enter" && (e.metaKey || e.ctrlKey))) { e.preventDefault(); d.blur(); } });
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
        p.list.push(m); drawMarks(p.n); editText(m.el, m, true); setTool("select");
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
    if (rdEl) { if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); if (rdPanel) closePanel(); else closeReader(); } return; }
    const typing = e.target.isContentEditable || e.target.tagName === "TEXTAREA" || (e.target.tagName === "INPUT" && e.target.type !== "color");
    if ((e.metaKey || e.ctrlKey) && !e.shiftKey && e.key.toLowerCase() === "z" && !typing) { e.preventDefault(); e.stopPropagation(); undo(); }
    else if ((e.key === "Backspace" || e.key === "Delete") && selected && !typing) {
      e.preventDefault(); e.stopPropagation(); snapshot();
      const p = pageOf(selected); p.list = p.list.filter((k) => k !== selected); selected = null; drawMarks(p.n);
    } else if (e.key === "Enter" && selected && selected.type === "text" && !typing) { e.preventDefault(); editText(selected.el, selected); }
    else if (e.key === "Escape" && !typing && (tool !== "select" || selected)) { e.preventDefault(); e.stopPropagation(); setTool("select"); select(null); }
  }, true);

  // ---------- reader (pages.pdf_reader): the PDF's text in one column, in the type and colours you pick ----------
  // Lines come from pdf.js's text in reading order; size, gaps and indents make them headings, paragraphs and lists.
  // Running headers, footers and page numbers are left out; pictures are cut from the pages (pages.pdf_reader_pictures).
  const THEMES = { light: ["#fbf8f2", "#26231f", "Light"], sepia: ["#f4ecd8", "#5b4636", "Sepia"], gray: ["#4a4a50", "#e7e5df", "Gray"],
    dark: ["#1d1c21", "#e3e0d9", "Dark"], black: ["#000000", "#b9b5ac", "Black"] };
  const FONTS = { serif: ['"New York", "Iowan Old Style", Charter, Georgia, serif', "Serif"], sans: ["-apple-system, BlinkMacSystemFont, system-ui, sans-serif", "Sans"],
    mono: ["ui-monospace, Menlo, Consolas, monospace", "Mono"] };
  const WIDTHS = [[560, "Narrow"], [680, "Medium"], [840, "Wide"]], SPACINGS = [[1.4, "Tight"], [1.65, "Normal"], [1.95, "Loose"]];
  const rdDefaults = () => { const r = CFG.reader || {}; return { theme: r.theme || "auto", font: r.font || "serif", size: r.font_size || 19, width: r.width || 680, spacing: r.line_height || 1.65 }; };
  let rd = rdDefaults();
  try { Object.assign(rd, JSON.parse(localStorage.getItem("valerie-pdf-reader") || "{}")); } catch (_) {}
  const rdSave = () => { try { localStorage.setItem("valerie-pdf-reader", JSON.stringify(rd)); } catch (_) {} };
  let rdEl = null, rdPanel = null, rdText = null;

  async function readText() {
    const lines = [];
    for (const p of pages) {
      const tc = await p.page.getTextContent();
      let cur = null;
      const flush = () => { if (cur && cur.text.trim()) lines.push(cur); cur = null; };
      for (const it of tc.items) {
        if (typeof it.str !== "string") continue;
        if (!it.str) { if (it.hasEOL) flush(); continue; }
        const t = it.transform, size = Math.hypot(t[2], t[3]) || it.height || 10, [x, y] = p.vp1.convertToViewportPoint(t[4], t[5]);
        if (cur && (Math.abs(y - cur.y) > Math.max(size, cur.size) * 0.5 || x < cur.x1 - size * 2)) flush();
        if (!cur) cur = { page: p.n, x, y, x1: x, size, text: "", H: p.vp1.height };
        else if (x - cur.x1 > size * 0.15 && !/\s$/.test(cur.text) && !/^\s/.test(it.str)) cur.text += " ";
        cur.text += it.str; cur.x1 = Math.max(cur.x1, x + (it.width || 0)); cur.size = Math.max(cur.size, size);
        if (it.hasEOL) flush();
      }
      flush();
    }
    // Running headers and footers: the same words (numbers aside) at the top or bottom of many pages, and page numbers.
    const key = (l) => l.text.replace(/\d+/g, "#").replace(/\s+/g, " ").trim().toLowerCase();
    const edge = (l) => l.y < l.H * 0.09 || l.y > l.H * 0.93;
    const seen = new Map();
    for (const l of lines) if (edge(l)) seen.set(key(l), (seen.get(key(l)) || new Set()).add(l.page));
    const pageNo = /^[\s\-–—|]*((page|p\.)\s*)?(\d+|[ivxlc]+)(\s*(of|\/)\s*\d+)?[\s\-–—|]*$/i;
    const body = lines.filter((l) => !(edge(l) && (pageNo.test(l.text) || (pages.length >= 2 && seen.get(key(l)).size >= Math.max(2, pages.length * 0.4)))));
    // The body text's size (by how much text is set in it) and its usual line spacing.
    const bySize = body.map((l) => [l.size, l.text.length]).sort((a, b) => a[0] - b[0]);
    let half = bySize.reduce((n, a) => n + a[1], 0) / 2, base = 12;
    for (const [sz, n] of bySize) if ((half -= n) <= 0) { base = sz; break; }
    const same = (a, b) => Math.abs(a - b) <= Math.max(a, b) * 0.12;
    const gaps = [];
    for (let i = 1; i < body.length; i++) { const a = body[i - 1], b = body[i]; if (a.page === b.page && same(a.size, base) && same(b.size, base) && b.y > a.y && b.y - a.y < base * 2.5) gaps.push(b.y - a.y); }
    gaps.sort((a, b) => a - b);
    const lead = gaps.length ? gaps[Math.floor(gaps.length / 2)] : base * 1.3;
    // A page's usual line length, so a short line that ends a sentence ends its paragraph.
    const widths = new Map();
    for (const l of body) if (same(l.size, base)) (widths.get(l.page) || widths.set(l.page, []).get(l.page)).push(l.x1 - l.x);
    const usual = new Map([...widths].map(([n, w]) => [n, w.sort((a, b) => a - b)[Math.floor(w.length / 2)]]));
    const bullet = /^([•●○◦▪■□►▸‣⁃∙·*–-]|\(?\d{1,3}[.)]|\(?[a-z]\))\s+/, ends = (t) => /[.!?:;"”’)\]]$/.test(t);
    const blocks = [];
    let b = null, prev = null;
    for (const l of body) {
      const t = l.text.replace(/\s+/g, " ").trim();
      let fresh = !b || !same(l.size, prev.size) || bullet.test(t);
      if (!fresh) {
        const gap = l.y - prev.y, pt = prev.text.trim();
        if (l.page !== prev.page || gap < -base) fresh = ends(pt);           // the next page or column carries on a sentence
        else if (gap > lead * 1.45) fresh = true;
        else if (ends(pt) && (prev.x1 < b.x1 - base * 3 || prev.x1 - prev.x < (usual.get(prev.page) || 0) * 0.7 || l.x > prev.x + base * 1.2)) fresh = true;   // short last line, or an indent
      }
      if (fresh) blocks.push((b = { lines: [], page: l.page, y: l.y - l.size, size: l.size, x1: 0, li: bullet.test(t) }));
      b.lines.push(t); b.x1 = Math.max(b.x1, l.x1); prev = l;
    }
    const join = (ls) => ls.reduce((s, t) => (!s ? t : /[A-Za-z]-$/.test(s) && /^[a-z]/.test(t) ? s.slice(0, -1) + t : s + " " + t), "");
    let titled = false;
    const out = blocks.map((b) => {
      const text = join(b.lines), r = b.size / base;
      let k = b.li ? "li" : "p";
      if (!b.li && text.length < 220 && r >= 1.12) k = !titled && r >= 1.45 && b.page <= 2 ? "h1" : r >= 1.3 ? "h2" : "h3";
      else if (r <= 0.86) k = "small";
      if (k === "h1") titled = true;
      if (/^(fig(ure)?|table|chart|exhibit|plate)\.?\s*[\dA-Z]+[.:\s]/i.test(text) && text.length < 500) k = "caption";
      return { k, text, page: b.page, y: b.y, num: b.li && /^\(?[\da-z]{1,3}[.)]/.test(text) };
    });
    const chars = new Map();
    for (const l of body) chars.set(l.page, (chars.get(l.page) || 0) + l.text.length);
    return { blocks: out, chars };
  }

  // Where a page's pictures are, from the drawing commands (a picture, or the whole page when it has almost no text).
  async function pictures(p, chars) {
    const W = p.vp1.width, H = p.vp1.height;
    if ((chars.get(p.n) || 0) < 40) return [{ x: 0, y: 0, w: W, h: H, whole: true }];
    const ol = await p.page.getOperatorList(), O = lib.OPS, U = lib.Util, found = [], stack = [];
    let m = [1, 0, 0, 1, 0, 0];
    for (let i = 0; i < ol.fnArray.length; i++) {
      const f = ol.fnArray[i], a = ol.argsArray[i];
      if (f === O.save) stack.push(m);
      else if (f === O.restore) m = stack.pop() || m;
      else if (f === O.transform) m = U.transform(m, a);
      else if (f === O.paintFormXObjectBegin) { stack.push(m); const mat = (a || []).find((x) => x && x.length === 6); if (mat) m = U.transform(m, Array.from(mat)); }
      else if (f === O.paintFormXObjectEnd) m = stack.pop() || m;
      else if (f === O.paintImageXObject || f === O.paintInlineImageXObject || f === O.paintImageXObjectRepeat) {
        const pts = [[0, 0], [1, 0], [0, 1], [1, 1]].map(([u, v]) => p.vp1.convertToViewportPoint(m[0] * u + m[2] * v + m[4], m[1] * u + m[3] * v + m[5]));
        const x0 = Math.max(0, Math.min(...pts.map((q) => q[0]))), y0 = Math.max(0, Math.min(...pts.map((q) => q[1])));
        const x1 = Math.min(W, Math.max(...pts.map((q) => q[0]))), y1 = Math.min(H, Math.max(...pts.map((q) => q[1])));
        found.push({ x: x0, y: y0, w: x1 - x0, h: y1 - y0 });
      }
    }
    // Pieces of one picture are joined; small ones, page backgrounds and logos in the margins are left out.
    const boxes = [];
    for (const f of found.filter((f) => f.w > 2 && f.h > 2)) {
      let g = { ...f };
      for (let i = boxes.length - 1; i >= 0; i--) {
        const o = boxes[i];
        if (g.x <= o.x + o.w + 3 && o.x <= g.x + g.w + 3 && g.y <= o.y + o.h + 3 && o.y <= g.y + g.h + 3) {
          const x = Math.min(g.x, o.x), y = Math.min(g.y, o.y);
          g = { x, y, w: Math.max(g.x + g.w, o.x + o.w) - x, h: Math.max(g.y + g.h, o.y + o.h) - y }; boxes.splice(i, 1);
        }
      }
      boxes.push(g);
    }
    return boxes.filter((g) => g.w >= 60 && g.h >= 40 && g.w * g.h < W * H * 0.85 && g.y + g.h > H * 0.1 && g.y < H * 0.9);
  }
  async function paintPicture(p, f, c) {
    const s = Math.min(3, 1400 / Math.max(1, f.w)) * Math.min(2, window.devicePixelRatio || 1), vp = p.page.getViewport({ scale: s });
    c.width = Math.max(1, Math.round(f.w * s)); c.height = Math.max(1, Math.round(f.h * s));
    try { await p.page.render({ canvasContext: c.getContext("2d"), canvas: c, viewport: vp, transform: [1, 0, 0, 1, -f.x * s, -f.y * s] }).promise; }
    catch (e) { console.warn("valerie pdf reader: picture", e); c.closest("figure")?.remove(); }
  }

  function applyReader() {
    if (!rdEl) return;
    const auto = rd.theme === "auto" || !THEMES[rd.theme], name = auto ? (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light") : rd.theme, [bg, fg] = THEMES[name];
    const st = rdEl.style;
    st.setProperty("--rbg", bg); st.setProperty("--rfg", fg); st.setProperty("--rfont", (FONTS[rd.font] || FONTS.serif)[0]);
    st.setProperty("--rsize", rd.size + "px"); st.setProperty("--rlh", rd.spacing); st.setProperty("--rwidth", rd.width + "px");
    const dark = name === "gray" || name === "dark" || name === "black";
    rdEl.toggleAttribute("data-dark", dark); st.colorScheme = dark ? "dark" : "light";
    if (rdPanel) rdPanel.refresh();
  }
  function closePanel() { if (!rdPanel) return; rdPanel.remove(); rdPanel = null; rdEl && rdEl.querySelector(".aa").setAttribute("aria-expanded", "false"); }
  function openPanel(anchor) {
    if (rdPanel) return closePanel();
    const panel = el("div", "rd-panel"), refreshers = [];
    panel.setAttribute("role", "dialog"); panel.setAttribute("aria-label", tr("Reader settings"));
    const row = (label, ...kids) => { const r = el("div"); r.append(el("div", "lbl", tr(label)), ...kids); panel.append(r); };
    const choose = (cls, items, prop) => {
      const g = el("div", cls);
      for (const [value, label, make] of items) {
        const b = make ? make() : el("button", null, tr(label));
        b.type = "button"; b.onclick = () => { rd[prop] = value; rdSave(); applyReader(); };
        refreshers.push(() => b.setAttribute("aria-pressed", String(rd[prop] === value)));
        g.append(b);
      }
      return g;
    };
    row("Theme", choose("themes", [["auto", "Auto"], ...Object.entries(THEMES).map(([k, v]) => [k, v[2]])].map(([k, label]) => [k, label, () => {
      const b = el("button", null, "Aa"), [bg, fg] = k === "auto" ? ["linear-gradient(135deg, #fbf8f2 50%, #1d1c21 50%)", "#8a8780"] : THEMES[k];
      b.style.background = bg; b.style.color = fg; b.title = tr(k === "auto" ? "Auto (follows light or dark mode)" : label); b.setAttribute("aria-label", b.title);
      return b;
    }]), "theme"));
    row("Font", choose("seg", Object.entries(FONTS).map(([k, v]) => [k, v[1], () => { const b = el("button", null, tr(v[1])); b.style.fontFamily = v[0]; return b; }]), "font"));
    const out = el("output"), minus = el("button", null, "A−"), plus = el("button", null, "A+"), size = el("div", "size");
    minus.setAttribute("aria-label", tr("Smaller text")); plus.setAttribute("aria-label", tr("Bigger text"));
    minus.style.fontSize = "12px"; plus.style.fontSize = "15px";
    const step = (d) => { rd.size = Math.max(12, Math.min(34, rd.size + d)); rdSave(); applyReader(); };
    minus.onclick = () => step(-1); plus.onclick = () => step(1);
    refreshers.push(() => { out.textContent = rd.size + " px"; });
    size.append(minus, out, plus); row("Size", size);
    row("Width", choose("seg", WIDTHS.map(([w, l]) => [w, l]), "width"));
    row("Line spacing", choose("seg", SPACINGS.map(([v, l]) => [v, l]), "spacing"));
    const reset = el("button", "reset", tr("Reset to your settings file"));
    reset.title = tr("reader_view in your Valerie settings");
    reset.onclick = () => { rd = rdDefaults(); try { localStorage.removeItem("valerie-pdf-reader"); } catch (_) {} applyReader(); };
    panel.append(reset);
    panel.refresh = () => refreshers.forEach((f) => f());
    rdPanel = panel; rdEl.append(panel); panel.refresh();
    anchor.setAttribute("aria-expanded", "true");
  }

  function closeReader() {
    if (!rdEl) return;
    const at = +(rdEl.dataset.page || 0);
    closePanel(); rdEl.remove(); rdEl = null; document.documentElement.classList.remove("reading");
    if (at && pages[at - 1]) pages[at - 1].box.scrollIntoView({ block: "start" });
  }
  async function openReader() {
    if (rdEl) return closeReader();
    setTool("select"); select(null);
    if (document.activeElement && document.activeElement.isContentEditable) document.activeElement.blur();
    const r = (rdEl = el("div", "vpdf-reader")), top = el("div", "rd-bar"), where = el("span", "info");
    r.tabIndex = -1;
    const aa = btn("Aa", "Type, size and colours", () => openPanel(aa), "aa"), back = btn("Pages", "Back to the PDF's pages (Esc)", () => closeReader());
    aa.setAttribute("aria-haspopup", "dialog"); aa.setAttribute("aria-expanded", "false");
    top.append(el("span", "name", fileName), el("span", "grow"), where, aa, back);
    const col = el("article", "rd-col"), status = el("p", "status", tr("Reading the PDF…"));
    status.setAttribute("aria-live", "polite");
    col.append(status);
    r.append(top, col);
    r.addEventListener("pointerdown", (e) => { if (rdPanel && !e.target.closest(".rd-panel, .aa")) closePanel(); });
    body.append(r); document.documentElement.classList.add("reading"); applyReader(); r.focus();
    // Open where you were reading.
    let from = 1;
    for (const p of pages) { if (p.box.getBoundingClientRect().top < innerHeight / 2) from = p.n; else break; }

    let data;
    try { data = await (rdText || (rdText = readText())); } catch (e) { console.warn("valerie pdf reader", e); status.textContent = tr("Couldn't read this PDF's text."); return; }
    if (rdEl !== r) return;
    status.remove();
    const marks = [];
    let page = 0, list = null;
    for (const b of data.blocks) {
      if (b.page !== page) {
        if (page) { const pg = el("span", "pg", tr("Page") + " " + b.page); pg.dataset.page = b.page; col.append(pg); marks.push(pg); }
        page = b.page; list = null;
      }
      if (b.k === "li") {
        if (!list) col.append((list = el("ul")));
        const li = el("li", b.num ? "num" : null, b.num ? b.text : b.text.replace(/^\S+\s+/, ""));
        li.dataset.page = b.page; li.dataset.y = b.y; list.append(li); continue;
      }
      list = null;
      const e = el(/^h\d$/.test(b.k) ? b.k : "p", b.k === "small" || b.k === "caption" ? b.k : null, b.text);
      e.dataset.page = b.page; e.dataset.y = b.y; col.append(e);
    }
    if (!data.blocks.length) col.append(el("p", "status", tr("This PDF has no text to read: its pages are pictures.")));
    // Which page you're reading, in the bar; the reader starts at the page you were on.
    const all = () => col.querySelectorAll("[data-page]");
    let queued = false;
    const track = () => {
      queued = false;
      let cur = data.blocks.length ? data.blocks[0].page : 1;
      for (const e of col.querySelectorAll(".pg")) { if (e.getBoundingClientRect().top < 120) cur = +e.dataset.page; else break; }
      r.dataset.page = cur; where.textContent = `${cur} / ${doc.numPages}`;
    };
    r.addEventListener("scroll", () => { if (!queued) { queued = true; requestAnimationFrame(track); } }, { passive: true });
    if (from > 1) { const first = [...all()].find((e) => +e.dataset.page >= from); if (first) r.scrollTop = first.getBoundingClientRect().top - 60; }
    track();

    // Pictures go in afterwards, page by page, each before the first text below it; each is drawn when it comes near.
    if (CFG.pages && CFG.pages.pdf_reader_pictures === false) return;
    const io = new IntersectionObserver((es) => {
      for (const x of es) if (x.isIntersecting) { io.unobserve(x.target); paintPicture(x.target._p, x.target._f, x.target.querySelector("canvas")); }
    }, { root: r, rootMargin: "800px 0px" });
    for (const p of pages) {
      let found;
      try { found = await pictures(p, data.chars); } catch (e) { console.warn("valerie pdf reader: pictures", e); continue; }
      if (rdEl !== r) return io.disconnect();
      for (const f of found) {
        const fig = el("figure"), c = el("canvas");
        c.style.width = Math.round(f.w * (f.whole ? 1 : 1.25)) + "px"; c.style.aspectRatio = `${f.w} / ${f.h}`;
        c.setAttribute("role", "img"); c.setAttribute("aria-label", tr("Picture from page") + " " + p.n);
        fig.append(c); fig._p = p; fig._f = f;
        const after = [...all()].find((e) => +e.dataset.page > p.n || (+e.dataset.page === p.n && e.dataset.y !== undefined && +e.dataset.y > f.y));
        const keepTop = r.scrollTop, anchorEl = [...all()].find((e) => e.getBoundingClientRect().top >= 46);
        const before = anchorEl && anchorEl.getBoundingClientRect().top;
        const spot = after ? (after.tagName === "LI" ? after.parentElement : after) : null;
        if (spot && spot.parentElement === col) col.insertBefore(fig, spot); else col.append(fig);
        if (anchorEl) r.scrollTop = keepTop + anchorEl.getBoundingClientRect().top - before;   // what you're reading stays put
        io.observe(fig);
      }
    }
  }
  if (!(CFG.pages && CFG.pages.pdf_reader === false)) window.__valeriePdfReader = openReader;
  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => applyReader());

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
