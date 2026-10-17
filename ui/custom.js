// Added to each of Valerie's own windows: the user's CSS (ui/<window>.css in the config folder) and Valerie's words
// in their language (strings/<language>.yaml). Valerie calls __valerieUi.apply({ css, strings }) whenever they change.
window.__valerieUi = (() => {
  let strings = {}, patterns = [], observer = null;
  const mine = new WeakMap(); // text we wrote, so our own changes aren't translated again
  const ATTRS = ["placeholder", "title", "aria-label"];
  const escape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  function tr(text) {
    const t = text.trim();
    if (!t) return text;
    let out = strings[t];
    if (out == null) {
      for (const [re, to] of patterns) {
        const m = re.exec(t);
        if (m) { out = to; for (const g of m.slice(1)) out = out.replace("{}", g); break; }
      }
    }
    return out == null ? text : text.replace(t, out);
  }
  function translateText(n) {
    if (mine.get(n) === n.data) return;
    const t = tr(n.data);
    if (t !== n.data) { n.data = t; mine.set(n, t); }
  }
  function translateEl(e) {
    for (const a of ATTRS) {
      const v = e.getAttribute && e.getAttribute(a);
      if (v && mine.get(e) !== a + v) { const t = tr(v); if (t !== v) { e.setAttribute(a, t); mine.set(e, a + t); } }
    }
  }
  function walk(root) {
    if (!root) return;
    if (root.nodeType === 3) return translateText(root);
    if (root.nodeType !== 1 || root.tagName === "STYLE" || root.tagName === "SCRIPT") return;
    translateEl(root);
    const w = document.createTreeWalker(root, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
    for (let n = w.nextNode(); n; n = w.nextNode()) {
      if (n.nodeType === 3) { const p = n.parentNode; if (!p || (p.tagName !== "STYLE" && p.tagName !== "SCRIPT")) translateText(n); }
      else translateEl(n);
    }
  }
  function apply(d) {
    let st = document.getElementById("valerie-user-css");
    if (d.css) {
      if (!st) { st = document.createElement("style"); st.id = "valerie-user-css"; }
      // Last in <head>, so it wins over Valerie's own styles.
      (document.head || document.documentElement).appendChild(st);
      if (st.textContent !== d.css) st.textContent = d.css;
    } else if (st) st.remove();
    strings = d.strings || {};
    patterns = Object.keys(strings).filter((k) => k.includes("{}"))
      .map((k) => [new RegExp("^" + k.split("{}").map(escape).join("(.*?)") + "$"), strings[k]]);
    if (!Object.keys(strings).length) { if (observer) { observer.disconnect(); observer = null; } return; }
    const start = () => {
      walk(document.body);
      if (observer) return;
      observer = new MutationObserver((muts) => {
        for (const m of muts) {
          if (m.type === "characterData") translateText(m.target);
          else if (m.type === "attributes") translateEl(m.target);
          else m.addedNodes.forEach(walk);
        }
      });
      observer.observe(document.body, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ATTRS });
    };
    if (document.body) start(); else document.addEventListener("DOMContentLoaded", start);
  }
  return { apply, tr };
})();
