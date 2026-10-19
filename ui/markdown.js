// ---------- Markdown files: shown as a formatted document (pages.markdown) ----------
// Part of content.js (joined into the same function), so it shares cfg, post, tr and darkWanted. A .md file (or a page
// sent as text/markdown) is rebuilt as a document: GitHub-style Markdown, an outline, highlighted code, Source to see
// the text, and for files on this computer, Edit and live updates when the file changes. Built with createElement only.
const MD_EXT = /\.(md|markdown|mdown|mkdn|mkd|mdwn)$/i;
function isMarkdownDoc() {
  if (!cfg.pages || cfg.pages.markdown === false) return false;
  const type = (document.contentType || "").toLowerCase();
  if (type === "text/markdown" || type === "text/x-markdown") return true;
  let path = location.pathname;
  try { path = decodeURIComponent(path); } catch (_) {}
  return (type === "text/plain" || (location.protocol === "file:" && type !== "text/html")) && MD_EXT.test(path);
}

function mdEl(tag, cls, text) { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; }

// Entities in Markdown text: the common named ones and every numeric one.
const MD_ENT = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " ", copy: "©", reg: "®", trade: "™", hellip: "…", mdash: "—",
  ndash: "–", larr: "←", rarr: "→", uarr: "↑", darr: "↓", harr: "↔", times: "×", divide: "÷", deg: "°", middot: "·", bull: "•", laquo: "«", raquo: "»",
  lsquo: "‘", rsquo: "’", ldquo: "“", rdquo: "”", check: "✓", plusmn: "±", ne: "≠", le: "≤", ge: "≥", infin: "∞", euro: "€", pound: "£", yen: "¥", sect: "§", para: "¶", zwj: "‍", ensp: " ", emsp: " ", thinsp: " " };
function mdEntities(s) {
  return s.indexOf("&") < 0 ? s : s.replace(/&(#x[\da-f]+|#\d+|[a-z]+\d*);/gi, (all, e) => {
    if (e[0] !== "#") return MD_ENT[e] ?? all;
    const n = e[1] === "x" || e[1] === "X" ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10);
    return n > 0 && n < 0x110000 ? String.fromCodePoint(n) : all;
  });
}
const MD_EMOJI = { tada: "🎉", rocket: "🚀", sparkles: "✨", fire: "🔥", warning: "⚠️", white_check_mark: "✅", heavy_check_mark: "✔️", x: "❌",
  bug: "🐛", memo: "📝", books: "📚", book: "📖", bulb: "💡", star: "⭐", heart: "❤️", "+1": "👍", thumbsup: "👍", "-1": "👎", eyes: "👀", wave: "👋",
  zap: "⚡", lock: "🔒", key: "🔑", wrench: "🔧", hammer: "🔨", gear: "⚙️", package: "📦", art: "🎨", construction: "🚧", rotating_light: "🚨",
  boom: "💥", pencil: "✏️", pencil2: "✏️", link: "🔗", mag: "🔍", heavy_plus_sign: "➕", heavy_minus_sign: "➖", information_source: "ℹ️",
  question: "❓", exclamation: "❗", point_right: "👉", clipboard: "📋", calendar: "📅", chart_with_upwards_trend: "📈", computer: "💻",
  globe_with_meridians: "🌐", smile: "😄", joy: "😂", thinking: "🤔", pray: "🙏", muscle: "💪", raised_hands: "🙌", coffee: "☕", recycle: "♻️",
  arrow_right: "➡️", arrow_left: "⬅️", arrow_up: "⬆️", arrow_down: "⬇️", new: "🆕", hourglass: "⌛", trophy: "🏆", gift: "🎁", lipstick: "💄", truck: "🚚" };

// Where a link may go: no script or data addresses (images may be data: pictures).
function mdSafeUrl(u, image) {
  const s = (u || "").trim().replace(/[\u0000-\u001f\s]/g, "");
  if (/^(javascript|vbscript|file):/i.test(s) && !(location.protocol === "file:" && /^file:/i.test(s))) return null;
  if (/^data:/i.test(s)) return image && /^data:image\/(png|gif|jpe?g|webp|svg\+xml);/i.test(s) ? s : null;
  return mdEntities((u || "").trim());
}

// ---------- the bits of HTML a README uses: kept, without scripts, styles or event handlers ----------
const MD_TAGS = new Set("a abbr b bdi bdo blockquote br caption center cite code dd del details dfn div dl dt em figcaption figure h1 h2 h3 h4 h5 h6 hr i img ins kbd li mark ol p picture pre q rp rt ruby s samp section small source span strike strong sub summary sup table tbody td tfoot th thead time tr tt u ul var video wbr".split(" "));
const MD_VOID = new Set(["br", "hr", "img", "source", "wbr"]);
const MD_DROP = new Set(["script", "style", "iframe", "object", "embed", "noscript", "template", "textarea", "select", "form", "input", "button", "link", "meta", "base", "svg", "math"]);
const MD_ATTRS = new Set(["href", "src", "srcset", "alt", "title", "width", "height", "align", "valign", "open", "media", "type", "colspan", "rowspan", "name", "id", "start", "reversed", "datetime", "dir", "lang", "controls", "muted", "loop", "poster", "cite"]);
function mdTag(raw) {
  const m = raw.match(/^<(\/?)([A-Za-z][\w-]*)([\s\S]*?)(\/?)>$/);
  if (!m) return null;
  const name = m[2].toLowerCase(), attrs = [];
  for (const a of m[3].matchAll(/([^\s"'>\/=]+)(?:\s*=\s*("[^"]*"|'[^']*'|[^\s"'>]+))?/g)) {
    const k = a[1].toLowerCase(); let v = a[2] == null ? "" : a[2].replace(/^["']|["']$/g, "");
    if (!MD_ATTRS.has(k)) continue;
    if (k === "href" || k === "src" || k === "poster" || k === "cite") { v = mdSafeUrl(v, k !== "href"); if (v == null) continue; }
    if (k === "srcset" && /(javascript|vbscript|data):/i.test(v)) continue;
    attrs.push([k, mdEntities(v)]);
  }
  return { close: !!m[1], name, attrs, selfClose: !!m[4] || MD_VOID.has(name) };
}
function mdMake(t) {
  const e = document.createElement(t.name === "center" ? "div" : t.name);
  if (t.name === "center") e.setAttribute("align", "center");
  for (const [k, v] of t.attrs) { try { e.setAttribute(k, v); } catch (_) {} }
  if (t.name === "a" && e.getAttribute("href") && !e.getAttribute("href").startsWith("#")) e.rel = "noopener";
  if (t.name === "video") e.controls = true;
  return e;
}
// An open-tag stack shared by everything rendered into one container, so `<details>` … `</details>` can wrap Markdown.
function mdHtmlInto(raw, stack, ctx) {
  const tag = mdTag(raw);
  if (!tag) return false;
  if (MD_DROP.has(tag.name)) { if (!tag.close && !tag.selfClose) ctx.dropping = tag.name; else if (tag.close && ctx.dropping === tag.name) ctx.dropping = null; return true; }
  if (!MD_TAGS.has(tag.name)) return true;
  if (tag.close) {
    for (let i = stack.length - 1; i > 0; i--) if (stack[i].tagName.toLowerCase() === (tag.name === "center" ? "div" : tag.name)) { stack.length = i; break; }
    return true;
  }
  const e = mdMake(tag);
  if (/^h[1-6]$/.test(tag.name)) ctx.headings.push(e);
  stack[stack.length - 1].append(e);
  if (!tag.selfClose) stack.push(e);
  return true;
}

// ---------- inline Markdown ----------
const MD_INLINE = new RegExp([
  /\\([!"#$%&'()*+,\-./:;<=>?@[\\\]^_`{|}~])/,                                         // 1 escape
  /(`+)([\s\S]*?[^`])\2(?!`)/,                                                            // 2,3 code
  /!\[((?:[^\[\]\\]|\\.|\[[^\]]*\])*)\]\(\s*(<[^>]*>|[^\s()]*(?:\([^\s()]*\)[^\s()]*)*)(?:\s+("[^"]*"|'[^']*'))?\s*\)/, // 4,5,6 image
  /\[((?:[^\[\]\\]|\\.|\[(?:[^\[\]]|\[[^\]]*\])*\])*)\]\(\s*(<[^>]*>|[^\s()]*(?:\([^\s()]*\)[^\s()]*)*)(?:\s+("[^"]*"|'[^']*'))?\s*\)/, // 7,8,9 link
  /\[\^([^\]\s]+)\]/,                                                                     // 10 footnote
  /(!?)\[((?:[^\[\]\\]|\\.|\[[^\]]*\])+)\](?:\[([^\]]*)\])?/,                             // 11,12,13 reference link
  /<(https?:\/\/[^>\s]+|mailto:[^>\s]+|[\w.+-]+@[\w-]+\.[\w.-]+)>/,                       // 14 autolink
  /(<!--[\s\S]*?-->|<\/?[A-Za-z][\w-]*(?:\s+[^<>]*?)?\s*\/?>)/,                           // 15 html
  /((?:https?:\/\/|www\.)[^\s<]*[^\s<.,:;"')\]!?*_~])/,                                    // 16 bare url
  /(\*\*\*|___)(?=\S)([\s\S]*?\S)\17/,                                                    // 17,18 strong em
  /(\*\*|__)(?=\S)([\s\S]*?\S)\19/,                                                       // 19,20 strong
  /(\*|_)(?=[^\s*_])([\s\S]*?[^\s\\])\21/,                                                // 21,22 em
  /(~~?)(?=\S)([\s\S]*?\S)\23/,                                                           // 23,24 strike
  /:([a-z0-9_+-]+):/,                                                                     // 25 emoji
  /( {2,}|\\)\n/,                                                                         // 26 hard break
].map((r) => r.source).join("|"), "g");
const mdUnangle = (u) => (u.startsWith("<") && u.endsWith(">") ? u.slice(1, -1) : u);
const mdUnquote = (t) => (t ? t.slice(1, -1) : "");
function mdText(stack, s, ctx) { if (!ctx.dropping && s) stack[stack.length - 1].append(mdEntities(s)); }
function mdLink(href, title, ctx) {
  const a = mdEl("a"), u = mdSafeUrl(href, false);
  if (u != null) a.setAttribute("href", u);
  if (title) a.title = mdEntities(title);
  if (u && !u.startsWith("#")) a.rel = "noopener";
  return a;
}
function mdImage(src, alt, title) {
  const im = mdEl("img"), u = mdSafeUrl(src, true);
  if (u != null) im.setAttribute("src", u);
  im.alt = mdEntities(alt.replace(/\\(.)/g, "$1")); if (title) im.title = mdEntities(title);
  im.loading = "lazy";
  return im;
}
function mdInline(text, into, ctx) {
  const stack = [into];
  let last = 0, m;
  const re = new RegExp(MD_INLINE.source, "g");   // its own, since this calls itself
  while ((m = re.exec(text))) {
    const at = m.index, top = () => stack[stack.length - 1];
    // `_` emphasis only at word edges (snake_case stays as it is).
    if ((m[21] === "_" || m[19] === "__" || m[17] === "___") && ((at > 0 && /\w/.test(text[at - 1])) || /\w/.test(text[re.lastIndex] || ""))) { re.lastIndex = at + 1; continue; }
    mdText(stack, text.slice(last, at), ctx);
    last = re.lastIndex;
    if (ctx.dropping && m[15] == null) continue;
    if (m[1] != null) mdText(stack, m[1], ctx);
    else if (m[2] != null) { let c = m[3]; if (/^ .*[^ ].* $/.test(c)) c = c.slice(1, -1); top().append(mdEl("code", null, c.replace(/\n/g, " "))); }
    else if (m[4] != null) top().append(mdImage(mdUnangle(m[5]), m[4], mdUnquote(m[6])));
    else if (m[7] != null) { const a = mdLink(mdUnangle(m[8]), mdUnquote(m[9]), ctx); mdInline(m[7], a, ctx); top().append(a); }
    else if (m[10] != null) {
      const id = m[10], n = ctx.noteOrder.indexOf(id) >= 0 ? ctx.noteOrder.indexOf(id) + 1 : ctx.notes[id] != null ? ctx.noteOrder.push(id) : 0;
      if (!n) { mdText(stack, m[0], ctx); continue; }
      const sup = mdEl("sup", "fnref"), a = mdEl("a", null, String(n)); a.href = "#fn-" + id; a.id = "fnref-" + id; sup.append(a); top().append(sup);
    }
    else if (m[11] != null) {
      const ref = ctx.refs[(m[13] || m[12]).trim().toLowerCase().replace(/\s+/g, " ")];
      if (!ref) {
        // Not a reference: keep the text, but still format what's inside the brackets.
        mdText(stack, m[11] + "[", ctx); mdInline(m[12], top(), ctx); mdText(stack, "]" + (m[13] != null ? "[" + m[13] + "]" : ""), ctx); continue;
      }
      if (m[11]) top().append(mdImage(ref.url, m[12], ref.title));
      else { const a = mdLink(ref.url, ref.title, ctx); mdInline(m[12], a, ctx); top().append(a); }
    }
    else if (m[14] != null) { const u = m[14], a = mdLink(/^(https?|mailto):/.test(u) ? u : "mailto:" + u, "", ctx); a.textContent = u.replace(/^mailto:/, ""); top().append(a); }
    else if (m[15] != null) { if (!m[15].startsWith("<!--") && !mdHtmlInto(m[15], stack, ctx)) mdText(stack, m[15], ctx); }
    else if (m[16] != null) { const a = mdLink(m[16].startsWith("www.") ? "https://" + m[16] : m[16], "", ctx); a.textContent = m[16]; top().append(a); }
    else if (m[17] != null) { const s = mdEl("strong"), e = mdEl("em"); s.append(e); mdInline(m[18], e, ctx); top().append(s); }
    else if (m[19] != null) { const s = mdEl("strong"); mdInline(m[20], s, ctx); top().append(s); }
    else if (m[21] != null) { const s = mdEl("em"); mdInline(m[22], s, ctx); top().append(s); }
    else if (m[23] != null) { const s = mdEl("del"); mdInline(m[24], s, ctx); top().append(s); }
    else if (m[25] != null) mdText(stack, MD_EMOJI[m[25]] || m[0], ctx);
    else if (m[26] != null) top().append(mdEl("br"));
  }
  mdText(stack, text.slice(last), ctx);
}

// ---------- code, highlighted ----------
const MD_KW = {
  js: "await break case catch class const continue debugger default delete do else export extends finally for from function get if import in instanceof let new of return set static super switch this throw try typeof var void while with yield async true false null undefined NaN interface type enum implements private public protected readonly as declare namespace abstract keyof satisfies",
  rust: "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while Some None Ok Err",
  py: "and as assert async await break class continue def del elif else except False finally for from global if import in is lambda None nonlocal not or pass raise return True try while with yield self match case",
  go: "break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var nil true false iota",
  sh: "if then else elif fi for while until do done case esac function in return export local readonly unset shift source alias echo printf cd exit set eval exec trap wait sudo",
  c: "auto break case char const continue default do double else enum extern float for goto if inline int long register return short signed sizeof static struct switch typedef union unsigned void volatile while bool true false nullptr NULL class namespace template typename public private protected virtual override new delete this using try catch throw final abstract extends implements import package interface boolean byte instanceof super synchronized fun val var when is object companion data sealed let guard func struct protocol extension init deinit self Self nil as some any where defer throws rethrows async await",
  rb: "alias and begin break case class def defined? do else elsif end ensure false for if in module next nil not or redo rescue retry return self super then true undef unless until when while yield require attr_accessor puts",
  sql: "select from where and or not insert into values update set delete create table index view drop alter add column primary key foreign references join left right inner outer full on group by order having limit offset as distinct union all case when then else end null is in like between exists returning with begin commit rollback default unique check",
};
const MD_LANG = { javascript: "js", jsx: "js", ts: "js", tsx: "js", typescript: "js", mjs: "js", cjs: "js", json: "json", jsonc: "js", json5: "js",
  rs: "rust", rust: "rust", python: "py", py: "py", go: "go", golang: "go", sh: "sh", bash: "sh", zsh: "sh", shell: "sh", console: "sh", fish: "sh",
  terminal: "sh", c: "c", h: "c", cpp: "c", "c++": "c", cc: "c", hpp: "c", java: "c", kotlin: "c", kt: "c", swift: "c", cs: "c", csharp: "c",
  objc: "c", dart: "c", scala: "c", php: "c", ruby: "rb", rb: "rb", sql: "sql", psql: "sql", mysql: "sql", sqlite: "sql", yaml: "conf",
  yml: "conf", toml: "conf", ini: "conf", conf: "conf", dotenv: "conf", env: "conf", html: "markup", xml: "markup", svg: "markup", vue: "markup",
  svelte: "markup", plist: "markup", css: "css", scss: "css", less: "css", diff: "diff", patch: "diff", lua: "lua", makefile: "sh", dockerfile: "sh" };
const MD_RULES = {};
function mdRules(lang) {
  if (MD_RULES[lang]) return MD_RULES[lang];
  const S = String.raw, str = S`"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'`, num = S`\b(?:0[xob][\da-fA-F_]+|\d[\d_]*(?:\.\d[\d_]*)?(?:e[+-]?\d+)?)[a-zA-Z\d]*\b`;
  const g = (parts) => new RegExp(parts.map(([k, r]) => `(?<${k}>${r})`).join("|"), "gm");
  let r;
  if (lang === "markup") r = g([["com", S`<!--[\s\S]*?-->`], ["tag", S`<\/?[\w:.-]+|\/?>`], ["attr", S`[\w:.-]+(?=\s*=)`], ["str", str]]);
  else if (lang === "css") r = g([["com", S`\/\*[\s\S]*?\*\/`], ["str", str], ["kw", S`@[\w-]+|!important`], ["key", S`[\w-]+(?=\s*:[^:{;]*[;}\n])`], ["num", S`#[\da-fA-F]{3,8}\b|-?\d*\.?\d+(?:px|em|rem|%|vh|vw|s|ms|deg|fr)?\b`], ["fn", S`[\w-]+(?=\()`]]);
  else if (lang === "conf") r = g([["com", S`(?:^|\s)[#;].*`], ["key", S`^[ \t]*(?:- )?(?:"[^"\n]*"|'[^'\n]*'|[\w.$@/-]+)(?=[ \t]*[:=](?:\s|$))|^\[[^\]\n]+\]`], ["str", str], ["num", num], ["kw", S`\b(?:true|false|null|yes|no|on|off|~)\b`]]);
  else if (lang === "json") r = g([["key", S`"(?:\\.|[^"\\\n])*"(?=\s*:)`], ["str", S`"(?:\\.|[^"\\\n])*"`], ["num", S`-?` + num], ["kw", S`\b(?:true|false|null)\b`]]);
  else {
    const line = { py: "#", sh: "#", rb: "#", sql: "--", lua: "--" }[lang] || "//";
    const com = [line === "//" ? S`\/\/.*` : line === "#" ? S`(?:^|[ \t])#(?!!).*|^#!.*` : S`--.*`];
    if (lang !== "py" && lang !== "sh" && lang !== "rb") com.unshift(S`\/\*[\s\S]*?\*\/`);
    const strs = lang === "py" ? S`"""[\s\S]*?"""|'''[\s\S]*?'''|[rbfu]{0,2}(?:${str})` : lang === "rust" ? S`b?r#*"[\s\S]*?"#*|b?"(?:\\.|[^"\\])*"|b?'(?:\\.|[^\\'])'` : lang === "js" ? str + S`|\x60(?:\\.|[^\x60\\])*\x60` : lang === "go" ? str + S`|\x60[^\x60]*\x60` : str;
    const parts = [["com", com.join("|")], ["str", strs]];
    if (lang === "sh") parts.push(["com", S`^[ \t]*[$%] (?=\S)`], ["var", S`\$\{[^}\n]*\}|\$[\w@#?*!$-]+`], ["fn", S`(?<=^[ \t]*(?:[$%] )?|[|;&][ \t]*)[A-Za-z_./][\w./-]*`]);
    if (lang === "rust") parts.push(["var", S`'[a-z_]\w*\b(?!')`], ["fn", S`\b[a-z_]\w*!(?=[\[(\{])`], ["attr", S`#!?\[[^\]\n]*\]`]);
    if (lang === "py" || lang === "js") parts.push(["attr", S`@[\w.]+`]);
    parts.push(["num", num], ["word", S`[A-Za-z_$][\w$]*\??`]);
    r = g(parts);
    r.kw = new Set(MD_KW[lang] ? (lang === "sql" ? MD_KW.sql.split(" ").flatMap((w) => [w, w.toUpperCase()]) : MD_KW[lang].split(" ")) : (MD_KW.js + " " + MD_KW.c).split(" "));
  }
  return (MD_RULES[lang] = r);
}
function mdHighlight(code, lang, into) {
  const l = MD_LANG[lang] || (MD_KW[lang] ? lang : null);
  if (!l || code.length > 200000) return into.append(code);
  if (l === "diff") {
    for (const line of code.split("\n")) {
      const c = /^(\+\+\+|---)/.test(line) ? "hd" : line[0] === "+" ? "add" : line[0] === "-" ? "del" : line.startsWith("@@") ? "hunk" : "";
      into.append(mdEl("span", "dl " + c, line || " "));
    }
    return;
  }
  const re = mdRules(l), kw = re.kw;
  let last = 0, m;
  re.lastIndex = 0;
  while ((m = re.exec(code))) {
    if (!m[0]) { re.lastIndex++; continue; }
    if (m.index > last) into.append(code.slice(last, m.index));
    last = re.lastIndex;
    let k = Object.keys(m.groups).find((g) => m.groups[g] != null);
    if (k === "word") {
      const w = m[0];
      k = kw && kw.has(w) ? "kw" : /^\s*\(/.test(code.slice(last, last + 3)) ? "fn" : /^[A-Z][a-z]\w*$/.test(w) && l !== "sh" && l !== "sql" ? "type" : null;
      if (!k) { into.append(w); continue; }
    }
    into.append(mdEl("span", "t-" + k, m[0]));
  }
  if (last < code.length) into.append(code.slice(last));
}

// ---------- blocks ----------
const MD_BULLET = /^( {0,3})([-+*])([ \t]+|$)(.*)$/, MD_ORDERED = /^( {0,3})(\d{1,9})([.)])([ \t]+|$)(.*)$/;
const MD_FENCE = /^( {0,3})(`{3,}|~{3,})[ \t]*([^`\s{]*)[^`]*$/, MD_HR = /^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$/, MD_ATX = /^ {0,3}(#{1,6})(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$/;
const MD_HTML_BLOCK = /^ {0,3}(?:<!--|<\/?(?:address|article|aside|blockquote|center|details|dialog|dd|div|dl|dt|fieldset|figcaption|figure|footer|h[1-6]|header|hr|li|main|nav|ol|p|picture|pre|section|summary|table|tbody|td|tfoot|th|thead|tr|ul|video|img|a|br|sub|sup|kbd|b|strong|em|i|span|code|source)(?:[\s/>]|$))/i;
const MD_TABLE_SEP = /^ {0,3}\|?[ \t]*:?-+:?[ \t]*(?:\|[ \t]*:?-+:?[ \t]*)*\|?[ \t]*$/;
const mdIndent = (s) => { let n = 0; for (const c of s) { if (c === " ") n++; else if (c === "\t") n += 4 - (n % 4); else break; } return n; };
function mdStrip(s, n) {
  let col = 0, i = 0;
  while (i < s.length && col < n && (s[i] === " " || s[i] === "\t")) { col += s[i] === "\t" ? 4 - (col % 4) : 1; i++; }
  return (col > n ? " ".repeat(col - n) : "") + s.slice(i);
}
function mdStartsBlock(line, inPara) {
  if (MD_ATX.test(line) || MD_FENCE.test(line) || MD_HR.test(line) || /^ {0,3}>/.test(line) || MD_HTML_BLOCK.test(line)) return true;
  const b = line.match(MD_BULLET); if (b && b[4].trim()) return true;
  const o = line.match(MD_ORDERED); return !!(o && o[5].trim() && (!inPara || o[2] === "1"));
}
function mdCells(line) {
  let s = line.trim(); if (s.startsWith("|")) s = s.slice(1); if (s.endsWith("|") && !s.endsWith("\\|")) s = s.slice(0, -1);
  const out = []; let cur = "", code = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (c === "\\" && s[i + 1] === "|") { cur += "|"; i++; continue; }
    if (c === "`") code ^= 1;
    if (c === "|" && !code) { out.push(cur.trim()); cur = ""; } else cur += c;
  }
  out.push(cur.trim());
  return out;
}
const MD_ALERTS = { note: ["Note", "ℹ"], tip: ["Tip", "💡"], important: ["Important", "❖"], warning: ["Warning", "⚠"], caution: ["Caution", "⛔"] };

function mdBlocks(lines, into, ctx) {
  const stack = [into], top = () => stack[stack.length - 1];
  let para = null;
  const flush = () => {
    if (!para) return;
    const text = para.join("\n").replace(/^[ \t]+/gm, "").replace(/[ \t]+$/, "");
    para = null;
    if (!text) return;
    const p = mdEl("p"); mdInline(text, p, ctx);
    if (p.childNodes.length) top().append(p);
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (!line.trim()) { flush(); continue; }
    let m;
    // Indented code (not inside a paragraph).
    if (!para && mdIndent(line) >= 4) {
      const code = [];
      while (i < lines.length && (mdIndent(lines[i]) >= 4 || !lines[i].trim())) code.push(mdStrip(lines[i++], 4));
      i--; while (code.length && !code[code.length - 1].trim()) code.pop();
      top().append(mdCode(code.join("\n"), "", ctx)); continue;
    }
    if ((m = line.match(MD_FENCE))) {
      flush();
      const fence = m[2], pad = m[1].length, code = [];
      for (i++; i < lines.length; i++) {
        const c = lines[i].match(/^ {0,3}(`{3,}|~{3,})[ \t]*$/);
        if (c && c[1][0] === fence[0] && c[1].length >= fence.length) break;
        code.push(mdStrip(lines[i], pad));
      }
      top().append(mdCode(code.join("\n"), mdEntities(m[3] || "").toLowerCase(), ctx)); continue;
    }
    if (para && /^ {0,3}(=+|-+)[ \t]*$/.test(line)) {
      const h = mdEl(line.trim()[0] === "=" ? "h1" : "h2"), text = para.join("\n").trim(); para = null;
      mdInline(text, h, ctx); ctx.headings.push(h); top().append(h); continue;
    }
    if ((m = line.match(MD_ATX))) { flush(); const h = mdEl("h" + m[1].length); mdInline(m[2] || "", h, ctx); ctx.headings.push(h); top().append(h); continue; }
    if (MD_HR.test(line)) { flush(); top().append(mdEl("hr")); continue; }
    if (/^ {0,3}>/.test(line)) {
      flush();
      const inner = [];
      while (i < lines.length && lines[i].trim() && (/^ {0,3}>/.test(lines[i]) || (inner.length && !mdStartsBlock(lines[i], true)))) inner.push(lines[i++].replace(/^ {0,3}> ?/, ""));
      i--;
      const alert = (inner[0] || "").trim().match(/^\[!(note|tip|important|warning|caution)\]$/i);
      if (alert) {
        const kind = alert[1].toLowerCase(), box = mdEl("div", "alert " + kind), title = mdEl("p", "alert-title");
        title.append(mdEl("span", "alert-icon", MD_ALERTS[kind][1]), tr(MD_ALERTS[kind][0]));
        box.append(title); mdBlocks(inner.slice(1), box, ctx); top().append(box);
      } else { const q = mdEl("blockquote"); mdBlocks(inner, q, ctx); top().append(q); }
      continue;
    }
    const bullet = line.match(MD_BULLET), ordered = !bullet && line.match(MD_ORDERED);
    if ((bullet || ordered) && (!para || mdStartsBlock(line, true))) {
      flush(); i = mdList(lines, i, top(), ctx) - 1; continue;
    }
    if (!para && line.includes("|") && i + 1 < lines.length && MD_TABLE_SEP.test(lines[i + 1]) && lines[i + 1].includes("-")) {
      const head = mdCells(line), aligns = mdCells(lines[i + 1]).map((c) => (c.startsWith(":") && c.endsWith(":") ? "center" : c.endsWith(":") ? "right" : c.startsWith(":") ? "left" : ""));
      if (head.length === aligns.length) {
        const wrap = mdEl("div", "table"), t = mdEl("table"), thead = mdEl("thead"), tbody = mdEl("tbody"), row = (cells, tag) => {
          const tr_ = mdEl("tr");
          head.forEach((_, k) => { const c = mdEl(tag); if (aligns[k]) c.style.textAlign = aligns[k]; mdInline(cells[k] || "", c, ctx); tr_.append(c); });
          return tr_;
        };
        thead.append(row(head, "th")); i += 2;
        while (i < lines.length && lines[i].trim() && !mdStartsBlock(lines[i], false)) tbody.append(row(mdCells(lines[i++]), "td"));
        i--;
        t.append(thead); if (tbody.childNodes.length) t.append(tbody); wrap.append(t); top().append(wrap); continue;
      }
    }
    if (!para && MD_HTML_BLOCK.test(line)) {
      const html = [];
      while (i < lines.length && lines[i].trim()) html.push(lines[i++]);
      i--;
      mdHtmlBlock(html.join("\n"), stack, ctx); continue;
    }
    if (para && mdStartsBlock(line, true)) { flush(); i--; continue; }
    (para = para || []).push(line);
  }
  flush();
}
// An HTML block: tags open and close containers in the shared stack; text in between is shown as it is.
function mdHtmlBlock(src, stack, ctx) {
  const re = /<!--[\s\S]*?-->|<\/?[A-Za-z][\w-]*(?:\s+[^<>]*?)?\s*\/?>/g;
  let last = 0, m;
  const text = (s) => { if (!ctx.dropping && s.trim()) mdInline(s.replace(/^\s+|\s+$/g, " "), stack[stack.length - 1], ctx); };
  while ((m = re.exec(src))) {
    text(src.slice(last, m.index)); last = re.lastIndex;
    if (!m[0].startsWith("<!--")) mdHtmlInto(m[0], stack, ctx);
  }
  text(src.slice(last));
}
function mdList(lines, i, into, ctx) {
  const first = lines[i].match(MD_BULLET), ordered = !first;
  const marker = first ? first[2] : lines[i].match(MD_ORDERED)[3];
  const list = mdEl(ordered ? "ol" : "ul");
  if (ordered) { const start = +lines[i].match(MD_ORDERED)[2]; if (start !== 1) list.start = start; }
  let loose = false;
  const items = [];
  while (i < lines.length) {
    const b = lines[i].match(MD_BULLET), o = !b && lines[i].match(MD_ORDERED);
    if (!(ordered ? o && o[3] === marker : b && b[2] === marker) || MD_HR.test(lines[i]) && !ordered) break;
    const pad = ordered ? o[1].length + o[2].length + 1 : b[1].length + 1, gap = (ordered ? o[4] : b[3]).length;
    const width = pad + (gap >= 1 && gap <= 4 ? gap : 1), body = [ordered ? o[5] : b[4]];
    let blank = false;
    for (i++; i < lines.length; i++) {
      const l = lines[i];
      if (!l.trim()) { body.push(""); blank = true; continue; }
      if (mdIndent(l) >= width) { body.push(mdStrip(l, width)); if (blank) loose = true; blank = false; continue; }
      if (blank || mdStartsBlock(l, true) || MD_BULLET.test(l) || MD_ORDERED.test(l)) break;
      body.push(l); // a lazy continuation of the item's paragraph
    }
    while (body.length && !body[body.length - 1].trim()) { body.pop(); }
    items.push(body);
    if (blank && i < lines.length) {
      const nb = lines[i].match(MD_BULLET), no = !nb && lines[i].match(MD_ORDERED);
      if (ordered ? no && no[3] === marker : nb && nb[2] === marker) loose = true;
    }
  }
  for (const body of items) {
    const li = mdEl("li"), task = body[0].match(/^\[([ xX])\][ \t]+(.*)$/);
    if (task) {
      li.className = "task"; body[0] = task[2];
      const box = mdEl("span", "check" + (task[1] === " " ? "" : " done")); box.setAttribute("role", "checkbox"); box.setAttribute("aria-checked", task[1] !== " ");
      li.append(box);
    }
    mdBlocks(body, li, ctx);
    // A tight list's items hold their text directly, not in paragraphs.
    if (!loose) for (const p of [...li.children]) if (p.tagName === "P") p.replaceWith(...p.childNodes);
    list.append(li);
  }
  into.append(list);
  return i;
}
function mdCode(code, lang, ctx) {
  const wrap = mdEl("div", "code"), pre = mdEl("pre"), c = mdEl("code"), bar = mdEl("div", "code-bar");
  if (lang) bar.append(mdEl("span", "lang", lang));
  const copy = mdEl("button", "copy", tr("Copy"));
  copy.onclick = () => {
    const done = () => { copy.textContent = tr("Copied"); setTimeout(() => (copy.textContent = tr("Copy")), 1300); };
    const old = () => { const t = mdEl("textarea"); t.value = code; t.style.cssText = "position:fixed;opacity:0"; document.body.append(t); t.select(); document.execCommand("copy"); t.remove(); done(); };
    if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(code).then(done, old); else old();
  };
  bar.append(copy);
  mdHighlight(code, lang, c);
  if (lang) c.className = "lang-" + lang.replace(/[^\w-]/g, "");
  pre.append(c); wrap.append(bar, pre);
  return wrap;
}

// Reference definitions, footnotes and front matter come out before the blocks are read.
function mdPrepare(src) {
  const ctx = { refs: {}, notes: {}, noteOrder: [], headings: [], meta: null, dropping: null };
  let lines = src.replace(/\r\n?/g, "\n").replace(/\u0000/g, "�").split("\n");
  if (/^(---|\+\+\+)\s*$/.test(lines[0] || "")) {
    const end = lines.findIndex((l, k) => k > 0 && k < 200 && /^(---|\+\+\+|\.\.\.)\s*$/.test(l));
    if (end > 0) {
      const body = lines.slice(1, end);
      if (body.every((l) => !l.trim() || /^[\w-]+\s*[:=]/.test(l) || /^\s/.test(l) || /^-\s/.test(l))) { ctx.meta = body; lines = lines.slice(end + 1); }
    }
  }
  const out = [];
  let fence = null;
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i], f = l.match(/^ {0,3}(`{3,}|~{3,})/);
    if (f) { if (!fence) fence = f[1]; else if (f[1][0] === fence[0] && f[1].length >= fence.length && !l.slice(l.indexOf(f[1]) + f[1].length).trim()) fence = null; out.push(l); continue; }
    if (fence) { out.push(l); continue; }
    let m;
    if ((m = l.match(/^ {0,3}\[\^([^\]\s]+)\]:[ \t]?(.*)$/))) {
      const body = [m[2]];
      while (i + 1 < lines.length && (mdIndent(lines[i + 1]) >= 4 || (!lines[i + 1].trim() && i + 2 < lines.length && mdIndent(lines[i + 2]) >= 4))) body.push(mdStrip(lines[++i], 4));
      ctx.notes[m[1]] = body; continue;
    }
    if ((m = l.match(/^ {0,3}\[([^\]^][^\]]*)\]:[ \t]*(<[^>]*>|\S+)(?:[ \t]+("[^"]*"|'[^']*'|\([^)]*\)))?[ \t]*$/)) && (!out.length || !out[out.length - 1].trim() || /^ {0,3}\[[^\]]+\]:/.test(lines[i - 1]))) {
      const key = m[1].trim().toLowerCase().replace(/\s+/g, " ");
      if (!ctx.refs[key]) ctx.refs[key] = { url: mdUnangle(m[2]), title: m[3] ? m[3].slice(1, -1) : "" };
      continue;
    }
    out.push(l);
  }
  return [out, ctx];
}
function mdSlug(text, used) {
  const base = text.trim().toLowerCase().replace(/[^\p{L}\p{N}\s_-]/gu, "").replace(/\s/g, "-") || "section";
  let s = base, n = 1;
  while (used.has(s)) s = base + "-" + n++;
  used.add(s);
  return s;
}
function mdRender(src) {
  const [lines, ctx] = mdPrepare(src), body = mdEl("article", "md");
  if (ctx.meta) {
    const box = mdEl("dl", "front");
    for (const l of ctx.meta) { const m = l.match(/^([\w-]+)\s*[:=]\s*(.*)$/); if (m && m[2].trim()) box.append(mdEl("dt", null, m[1]), mdEl("dd", null, m[2].replace(/^["']|["']$/g, ""))); }
    if (box.childNodes.length) body.append(box);
  }
  mdBlocks(lines, body, ctx);
  if (ctx.noteOrder.length) {
    const sec = mdEl("section", "footnotes"), ol = mdEl("ol");
    sec.append(mdEl("hr"));
    for (const id of ctx.noteOrder) {
      const li = mdEl("li"); li.id = "fn-" + id; mdBlocks(ctx.notes[id], li, ctx);
      const back = mdEl("a", "fnback", "↩"); back.href = "#fnref-" + id; back.title = tr("Back to the text");
      (li.lastElementChild && li.lastElementChild.tagName === "P" ? li.lastElementChild : li).append(" ", back);
      ol.append(li);
    }
    sec.append(ol); body.append(sec);
  }
  const used = new Set(), outline = [];
  for (const h of ctx.headings) {
    if (!h.isConnected && !body.contains(h)) continue;
    const text = h.textContent;
    h.id = h.id || mdSlug(text, used);
    const a = mdEl("a", "anchor", "#"); a.href = "#" + h.id; a.setAttribute("aria-label", tr("Link to this section"));
    h.prepend(a);
    if (+h.tagName[1] <= 3 && text.trim()) outline.push([+h.tagName[1], text.trim(), h.id]);
  }
  return { body, outline, words: (body.textContent.match(/\S+/g) || []).length };
}

// ---------- the viewer ----------
const MD_CSS = `
:root { color-scheme: light; --bg: #fcfbf9; --fg: #24222a; --dim: #6b6875; --faint: #9895a1; --line: rgba(20,16,32,.09); --soft: rgba(20,16,32,.035);
  --code: #f4f2ee; --bar: rgba(252,251,249,.82); --ac: var(--accent); --link: color-mix(in srgb, var(--accent) 72%, #1b1530);
  --t-kw: #cf222e; --t-str: #0a3069; --t-com: #6e7781; --t-num: #0550ae; --t-fn: #8250df; --t-type: #953800; --t-tag: #116329; --t-attr: #0550ae; --t-var: #953800; --t-key: #0550ae;
  --note: #0969da; --tip: #1a7f37; --important: #8250df; --warning: #9a6700; --caution: #cf222e; }
:root.dark { color-scheme: dark; --bg: #17161b; --fg: #e9e7ef; --dim: #a29fac; --faint: #75727e; --line: rgba(255,255,255,.09); --soft: rgba(255,255,255,.035);
  --code: #201f26; --bar: rgba(23,22,27,.8); --link: color-mix(in srgb, var(--accent) 80%, #fff);
  --t-kw: #ff7b72; --t-str: #a5d6ff; --t-com: #8b949e; --t-num: #79c0ff; --t-fn: #d2a8ff; --t-type: #ffa657; --t-tag: #7ee787; --t-attr: #79c0ff; --t-var: #ffa657; --t-key: #79c0ff;
  --note: #4493f8; --tip: #3fb950; --important: #ab7df8; --warning: #d29922; --caution: #f85149; }
html { background: var(--bg); scroll-padding-top: 64px; }
body { margin: 0; background: var(--bg); color: var(--fg); font: 16.5px/1.72 -apple-system, BlinkMacSystemFont, "Inter", "Segoe UI", system-ui, sans-serif;
  -webkit-font-smoothing: antialiased; text-rendering: optimizeLegibility; }
::selection { background: color-mix(in srgb, var(--accent) 32%, transparent); }
.vmd-bar { position: sticky; top: 0; z-index: 5; display: flex; align-items: center; gap: 10px; height: 46px; padding: 0 14px 0 18px;
  background: var(--bar); backdrop-filter: saturate(1.6) blur(16px); -webkit-backdrop-filter: saturate(1.6) blur(16px); border-bottom: 1px solid var(--line);
  font: 13px/1 -apple-system, system-ui, sans-serif; color: var(--dim); }
.vmd-bar .doc { width: 15px; height: 18px; flex: none; border-radius: 3px; border: 1.5px solid currentColor; position: relative; opacity: .7; }
.vmd-bar .doc::after { content: "M↓"; position: absolute; inset: 0; display: grid; place-items: center; font: 700 6.5px/1 -apple-system, system-ui; }
.vmd-bar .path { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.vmd-bar .path b { color: var(--fg); font-weight: 600; }
.vmd-bar .grow { flex: 1; }
.vmd-bar .time { white-space: nowrap; color: var(--faint); }
.vmd-bar button { all: unset; cursor: pointer; padding: 6px 11px; border-radius: 8px; color: var(--dim); font-weight: 550; white-space: nowrap; transition: background .12s, color .12s; }
.vmd-bar button:hover { background: var(--soft); color: var(--fg); box-shadow: inset 0 0 0 1px var(--line); }
.vmd-bar button.on { color: var(--fg); background: color-mix(in srgb, var(--accent) 16%, transparent); }
.vmd-live { width: 7px; height: 7px; border-radius: 50%; background: #3fb950; box-shadow: 0 0 0 3px rgba(63,185,80,.18); flex: none; }
.vmd-live.flash { animation: vmdflash 1s; } @keyframes vmdflash { 30% { box-shadow: 0 0 0 7px rgba(63,185,80,.3); } }
.vmd-toc { position: fixed; top: 86px; left: max(20px, calc(50vw - 380px - 290px)); width: 230px; max-height: calc(100vh - 120px); overflow-y: auto; scrollbar-width: none;
  font: 13px/1.45 -apple-system, system-ui, sans-serif; border-left: 1px solid var(--line); padding: 2px 0; }
.vmd-toc .title { font-weight: 650; font-size: 11px; letter-spacing: .07em; text-transform: uppercase; color: var(--faint); padding: 0 0 8px 14px; }
.vmd-toc a { display: block; padding: 4px 10px 4px 14px; margin-left: -1px; border-left: 2px solid transparent; color: var(--dim); text-decoration: none;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap; transition: color .12s, border-color .12s; }
.vmd-toc a.l2 { padding-left: 26px; } .vmd-toc a.l3 { padding-left: 38px; font-size: 12.5px; }
.vmd-toc a:hover { color: var(--fg); } .vmd-toc a.on { color: var(--fg); border-left-color: var(--accent); font-weight: 550; }
@media (max-width: 1240px) { .vmd-toc { left: auto; right: 14px; top: 54px; width: 270px; max-height: 70vh; padding: 12px 6px; border: 1px solid var(--line);
  border-radius: 12px; background: var(--bg); box-shadow: 0 18px 50px rgba(0,0,0,.18); } .vmd-toc a { border-left: 0; border-radius: 6px; } .vmd-toc a.on { background: var(--soft); }
  .vmd-toc:not(.open) { display: none; } }
@media (min-width: 1241px) { .vmd-toc.closed { display: none; } }
.vmd-source { max-width: 900px; margin: 0 auto; padding: 40px 32px 120px; font: 13px/1.65 ui-monospace, "SF Mono", Menlo, monospace; white-space: pre-wrap; word-wrap: break-word; color: var(--fg); }
.md { max-width: 760px; margin: 0 auto; padding: 56px 36px 160px; overflow-wrap: break-word; }
.md > :first-child, .md li > :first-child, .md blockquote > :first-child, .md .alert > :first-child { margin-top: 0; }
.md > :last-child, .md li > :last-child, .md blockquote > :last-child { margin-bottom: 0; }
.md h1, .md h2, .md h3, .md h4, .md h5, .md h6 { position: relative; line-height: 1.25; font-weight: 700; letter-spacing: -.018em; margin: 2em 0 .7em; color: var(--fg); }
.md h1 { font-size: 2.15em; letter-spacing: -.03em; margin-top: 1.2em; } .md h2 { font-size: 1.55em; padding-bottom: .35em; border-bottom: 1px solid var(--line); }
.md h3 { font-size: 1.24em; } .md h4 { font-size: 1.05em; } .md h5, .md h6 { font-size: .82em; text-transform: uppercase; letter-spacing: .06em; color: var(--dim); }
.md .anchor { position: absolute; left: -1.05em; width: 1em; text-align: center; color: var(--faint); text-decoration: none; font-weight: 500; opacity: 0; transition: opacity .12s; }
.md h1:hover .anchor, .md h2:hover .anchor, .md h3:hover .anchor, .md h4:hover .anchor, .md h5:hover .anchor, .md h6:hover .anchor, .md .anchor:focus { opacity: 1; }
.md p, .md ul, .md ol, .md dl, .md .table, .md .code, .md blockquote, .md .alert, .md details, .md figure { margin: 0 0 1.15em; }
.md a { color: var(--link); text-decoration: underline; text-decoration-color: color-mix(in srgb, var(--link) 35%, transparent); text-underline-offset: .18em; text-decoration-thickness: 1px; }
.md a:hover { text-decoration-color: currentColor; }
.md strong { font-weight: 650; } .md del { color: var(--dim); }
.md img, .md video { max-width: 100%; height: auto; border-radius: 6px; } .md p > img:only-child, .md p > a:only-child > img:only-child { border-radius: 10px; }
.md [align=center] img, .md a > img { border-radius: 0; } .md img[src*="shields.io"], .md img[src*="badge"] { border-radius: 3px; vertical-align: middle; }
.md ul, .md ol { padding-left: 1.6em; } .md li { margin: .28em 0; } .md li > ul, .md li > ol { margin: .28em 0 .4em; }
.md li::marker { color: var(--faint); } .md ol > li::marker { font-variant-numeric: tabular-nums; font-weight: 550; }
.md li.task { list-style: none; position: relative; } .md li.task > .check { position: absolute; left: -1.5em; top: .32em; width: 1.05em; height: 1.05em; box-sizing: border-box;
  border-radius: 5px; border: 1.5px solid var(--faint); } .md li.task > .check.done { background: var(--accent); border-color: var(--accent); }
.md li.task > .check.done::after { content: ""; position: absolute; left: .3em; top: .1em; width: .28em; height: .55em; border: solid #fff; border-width: 0 2px 2px 0; transform: rotate(45deg); }
.md blockquote { padding: .1em 0 .1em 1.1em; border-left: 3px solid color-mix(in srgb, var(--accent) 55%, transparent); color: var(--dim); }
.md .alert { padding: .8em 1.1em; border-radius: 10px; border: 1px solid color-mix(in srgb, var(--c) 28%, transparent); background: color-mix(in srgb, var(--c) 7%, transparent); }
.md .alert.note { --c: var(--note); } .md .alert.tip { --c: var(--tip); } .md .alert.important { --c: var(--important); } .md .alert.warning { --c: var(--warning); } .md .alert.caution { --c: var(--caution); }
.md .alert-title { display: flex; align-items: center; gap: .45em; font-weight: 650; color: var(--c); margin: 0 0 .35em !important; }
.md .alert > :last-child { margin-bottom: 0; }
.md code, .vmd-src { font-family: ui-monospace, "SF Mono", SFMono-Regular, Menlo, Consolas, monospace; }
.md :not(pre) > code { font-size: .85em; padding: .16em .42em; border-radius: 6px; background: var(--code); border: 1px solid var(--line); }
.md kbd { font: 600 .78em ui-monospace, "SF Mono", Menlo, monospace; padding: .12em .45em; border-radius: 6px; border: 1px solid var(--line); border-bottom-width: 2px; background: var(--soft); }
.md .code { position: relative; border-radius: 12px; background: var(--code); border: 1px solid var(--line); overflow: hidden; }
.md .code-bar { position: absolute; top: 0; right: 0; display: flex; align-items: center; gap: 4px; padding: 7px 8px; font: 11.5px/1 -apple-system, system-ui, sans-serif; }
.md .code-bar .lang { color: var(--faint); padding: 0 4px; transition: opacity .12s; }
.md .code-bar .copy { all: unset; cursor: pointer; padding: 5px 9px; border-radius: 7px; color: var(--dim); background: var(--bg); box-shadow: 0 0 0 1px var(--line); opacity: 0; transition: opacity .12s; }
.md .code:hover .copy, .md .copy:focus { opacity: 1; } .md .code:hover .lang { opacity: 0; }
.md pre { margin: 0; padding: 16px 18px; overflow-x: auto; font-size: 13.4px; line-height: 1.6; tab-size: 4; }
.md pre code { font-size: inherit; }
.md .t-kw { color: var(--t-kw); } .md .t-str { color: var(--t-str); } .md .t-com { color: var(--t-com); font-style: italic; } .md .t-num { color: var(--t-num); }
.md .t-fn { color: var(--t-fn); } .md .t-type { color: var(--t-type); } .md .t-tag { color: var(--t-tag); } .md .t-attr { color: var(--t-attr); } .md .t-var { color: var(--t-var); } .md .t-key { color: var(--t-key); }
.md .dl { display: block; margin: 0 -18px; padding: 0 18px; width: max-content; min-width: 100%; } .md .dl.add { background: rgba(46,160,67,.14); color: var(--tip); }
.md .dl.del { background: rgba(248,81,73,.13); color: var(--caution); } .md .dl.hunk { color: var(--t-num); } .md .dl.hd { color: var(--dim); font-weight: 600; }
.md .table { overflow-x: auto; border: 1px solid var(--line); border-radius: 10px; }
.md table { border-collapse: collapse; width: 100%; font-size: .94em; font-variant-numeric: tabular-nums; }
.md th, .md td { padding: .55em .95em; text-align: left; vertical-align: top; border-bottom: 1px solid var(--line); }
.md th + th, .md td + td { border-left: 1px solid var(--line); }
.md thead th { font-weight: 650; background: var(--soft); white-space: nowrap; } .md tbody tr:last-child td { border-bottom: 0; }
.md tbody tr:nth-child(even) td { background: var(--soft); }
.md hr { height: 1px; border: 0; background: var(--line); margin: 2.4em 0; }
.md details { padding: .55em 1em; border: 1px solid var(--line); border-radius: 10px; } .md details[open] > summary { margin-bottom: .6em; }
.md summary { cursor: pointer; font-weight: 600; } .md summary::marker { color: var(--faint); }
.md dl dt { font-weight: 650; } .md dl dd { margin: 0 0 .6em 1.2em; color: var(--dim); }
.md .front { display: grid; grid-template-columns: max-content 1fr; gap: .3em 1.2em; padding: .9em 1.1em; border-radius: 10px; background: var(--soft); border: 1px solid var(--line); font-size: .88em; margin-bottom: 2em; }
.md .front dt { color: var(--faint); font-weight: 550; } .md .front dd { margin: 0; color: var(--fg); }
.md sup.fnref a { text-decoration: none; font-size: .75em; padding: 0 .15em; font-weight: 600; } .md .footnotes { font-size: .9em; color: var(--dim); margin-top: 3em; }
.md .footnotes hr { margin: 0 0 1.2em; } .md .fnback { text-decoration: none; }
.md :target { scroll-margin-top: 64px; } .md li:target, .md h1:target, .md h2:target, .md h3:target, .md h4:target { animation: vmdtarget 1.6s; }
@keyframes vmdtarget { from { background: color-mix(in srgb, var(--accent) 22%, transparent); } }
@media (max-width: 700px) { body { font-size: 16px; } .md { padding: 32px 20px 120px; } .vmd-bar .time { display: none; } .md .anchor { display: none; } }
@media print { .vmd-bar, .vmd-toc { display: none !important; } .md { max-width: none; padding: 0; } .md .code-bar { display: none; } :root { --bg: #fff; } }`;

// Windows-1252's characters for the bytes 0x80–0x9f (what browsers use for "Latin-1"); "_" where it has none.
const MD_CP1252 = "€_‚ƒ„…†‡ˆ‰Š‹Œ_Ž__‘’“”•–—˜™š›œ_žŸ";
// A server that doesn't say the file is UTF-8 gets it read as Latin-1 ("âŒ˜" for "⌘"): read it again as UTF-8.
function mdUtf8(text) {
  if (/^utf-?8$/i.test(document.characterSet) || !/[\u00c2-\u00f4]/.test(text)) return text;
  const bytes = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i++) {
    const c = text.charCodeAt(i), b = c < 256 ? c : MD_CP1252.indexOf(String.fromCharCode(c));
    if (b < 0) return text;
    bytes[i] = c < 256 ? c : 0x80 + b;
  }
  try { return new TextDecoder("utf-8", { fatal: true }).decode(bytes); } catch (_) { return text; }
}
let mdShown = false, mdTheme = () => {}, mdUpdate = () => {};
function markdownViewer() {
  if (!isMarkdownDoc()) return;
  const start = () => {
    if (mdShown || !document.body) return;
    mdShown = true;
    const pre = document.body.querySelector("pre");
    let src = mdUtf8((pre || document.body).textContent);
    const local = location.protocol === "file:";
    let path = location.pathname;
    try { path = decodeURIComponent(path); } catch (_) {}
    const parts = path.split("/").filter(Boolean), file = parts.pop() || location.hostname, dir = local ? path.slice(0, path.length - file.length).replace(/^\/Users\/[^/]+/, "~") : location.hostname + "/" + parts.join("/") + (parts.length ? "/" : "");
    const key = "valerie-md:" + location.href.split("#")[0];
    const saved = (() => { try { return JSON.parse(sessionStorage.getItem(key) || "{}"); } catch (_) { return {}; } })();
    const keep = (k, v) => { saved[k] = v; try { sessionStorage.setItem(key, JSON.stringify(saved)); } catch (_) {} };
    const tocWanted = cfg.pages.markdown_outline !== false;

    document.documentElement.style.setProperty("--accent", cfg.accent || "#9d8cff");
    const css = mdEl("style"); css.id = "__valerie_markdown"; css.textContent = MD_CSS;
    (document.head || document.documentElement).append(css);
    const meta = mdEl("meta"); meta.name = "viewport"; meta.content = "width=device-width, initial-scale=1"; (document.head || document.documentElement).append(meta);

    const bar = mdEl("header", "vmd-bar"), p = mdEl("span", "path"), time = mdEl("span", "time");
    p.append(dir, mdEl("b", null, file)); p.title = local ? path : location.href;
    const live = local ? mdEl("span", "vmd-live") : null;
    if (live) live.title = tr("Updates when the file changes");
    const btn = (label, title, fn) => { const b = mdEl("button", null, tr(label)); b.title = tr(title); b.onclick = fn; return b; };
    const toc = mdEl("nav", "vmd-toc"), main = mdEl("main"), source = mdEl("pre", "vmd-source");
    const tocBtn = btn("Outline", "Show the document's headings", () => {
      const wide = innerWidth > 1240;
      if (wide) { toc.classList.toggle("closed"); keep("toc", !toc.classList.contains("closed")); } else toc.classList.toggle("open");
      tocBtn.classList.toggle("on", wide ? !toc.classList.contains("closed") : toc.classList.contains("open"));
    });
    const srcBtn = btn("Source", "Show the Markdown text", () => showSource(!saved.source));
    bar.append(mdEl("span", "doc"), p, mdEl("span", "grow"));
    if (live) bar.append(live);
    bar.append(time);
    if (tocWanted) bar.append(tocBtn);
    bar.append(srcBtn);
    // A file:// page can't send Valerie messages, so it asks for a valerie:// address Valerie answers instead.
    const tell = (what) => { const i = new Image(); i.src = "valerie://md/" + what + "?" + Date.now(); };
    if (local) bar.append(btn("Edit", "Open this file in your editor", () => tell("edit")));

    let current = null, outline = [];
    const draw = (text, keepScroll) => {
      const y = scrollY, r = mdRender(text);
      if (current) current.replaceWith(r.body); else main.append(r.body);
      current = r.body; outline = r.outline;
      time.textContent = tr(`${Math.max(1, Math.round(r.words / 230))} min read`);
      const h1 = r.body.querySelector("h1");
      document.title = (h1 ? h1.textContent.replace(/^#/, "").trim() : "") || file;
      toc.replaceChildren();
      if (outline.length > 1) {
        toc.append(mdEl("div", "title", tr("On this page")));
        const top = Math.min(...outline.map((o) => o[0]));
        for (const [lvl, text_, id] of outline) { const a = mdEl("a", "l" + (lvl - top + 1), text_); a.href = "#" + id; a.onclick = () => toc.classList.remove("open"); toc.append(a); }
      }
      tocBtn.hidden = outline.length < 2;
      source.textContent = text;
      if (keepScroll) scrollTo(0, y);
      spy();
    };
    const showSource = (on) => {
      keep("source", on); srcBtn.classList.toggle("on", on); srcBtn.textContent = tr(on ? "Formatted" : "Source");
      main.hidden = on; source.hidden = !on; toc.style.visibility = on ? "hidden" : "";
    };
    // The outline follows along: the heading nearest the top of the window is lit.
    let spyQueued = false;
    const spy = () => {
      if (spyQueued) return; spyQueued = true;
      requestAnimationFrame(() => {
        spyQueued = false;
        let on = outline[0] && outline[0][2];
        for (const [, , id] of outline) { const h = document.getElementById(id); if (h && h.getBoundingClientRect().top < 110) on = id; else if (h) break; }
        for (const a of toc.querySelectorAll("a")) a.classList.toggle("on", a.getAttribute("href") === "#" + on);
      });
    };
    addEventListener("scroll", spy, { passive: true });

    document.body.replaceChildren(bar, main, source);
    if (tocWanted) document.body.append(toc);
    document.body.removeAttribute("style");
    draw(src, false);
    showSource(!!saved.source);
    if (innerWidth > 1240 && saved.toc === false) toc.classList.add("closed");
    tocBtn.classList.toggle("on", innerWidth > 1240 && !toc.classList.contains("closed"));
    document.addEventListener("keydown", (e) => { if (e.key === "Escape" && toc.classList.contains("open")) toc.classList.remove("open"); });
    document.addEventListener("click", (e) => { if (toc.classList.contains("open") && !toc.contains(e.target) && e.target !== tocBtn) toc.classList.remove("open"); });
    if (location.hash) { const t = document.getElementById(decodeURIComponent(location.hash.slice(1))); if (t) t.scrollIntoView(); }
    else if (saved.y) scrollTo(0, saved.y);
    addEventListener("pagehide", () => keep("y", scrollY));

    const dark = matchMedia("(prefers-color-scheme: dark)");
    const theme = () => { const want = cfg.pages.markdown_theme || "auto"; document.documentElement.classList.toggle("dark", want === "dark" || (want === "auto" && (dark.matches || !!darkWanted()))); };
    theme(); dark.addEventListener("change", theme);
    mdTheme = theme;

    // Valerie watches a file on this computer and sends it again when it's saved (yours or your AI's edits).
    if (local && cfg.pages.markdown_live !== false) {
      mdUpdate = (text) => {
        if (text === src) return;
        src = text; draw(src, true);
        live.classList.remove("flash"); void live.offsetWidth; live.classList.add("flash");
      };
      tell("watch");
    } else if (live) live.remove();
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start); else start();
}
markdownViewer();
