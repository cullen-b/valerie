//! Built-in MCP server (Streamable HTTP, JSON responses) so agents can drive the browser.
//! Binds to 127.0.0.1, requires a bearer token, and rejects browser origins and foreign Host headers.
use crate::app::UserEvent;
use crate::command::CATALOG;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::time::{Duration, Instant};
use tao::event_loop::EventLoopProxy;

pub struct McpRequest {
    pub op: String,
    pub args: Value,
    pub reply: Sender<Result<Value, String>>,
}

pub fn token_path() -> PathBuf {
    crate::config::data_dir().join("mcp-token")
}

fn random_hex() -> String {
    use std::hash::{BuildHasher, Hasher};
    // RandomState is seeded from the OS RNG; four 64-bit draws give a 256-bit token without extra crates.
    (0..4)
        .map(|i| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(i);
            format!("{:016x}", h.finish())
        })
        .collect()
}

pub fn load_or_create_token() -> String {
    let path = token_path();
    if let Ok(t) = std::fs::read_to_string(&path) {
        if t.trim().len() >= 32 {
            return t.trim().to_string();
        }
    }
    let t = random_hex();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::write(&path, &t);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    t
}

fn eq_const(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn spawn(port: u16, token: String, proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        let server = match tiny_http::Server::http(("127.0.0.1", port)) {
            Ok(s) => s,
            Err(e) => return eprintln!("valerie: MCP server could not bind 127.0.0.1:{port}: {e}"),
        };
        for rq in server.incoming_requests() {
            let (proxy, token) = (proxy.clone(), token.clone());
            std::thread::spawn(move || handle_http(rq, port, &token, &proxy));
        }
    });
}

fn header<'a>(rq: &'a tiny_http::Request, name: &str) -> Option<&'a str> {
    rq.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str())
}

fn respond(rq: tiny_http::Request, code: u16, body: String) {
    let mut r = tiny_http::Response::from_string(body).with_status_code(code);
    if let Ok(h) = tiny_http::Header::from_bytes("Content-Type", "application/json") {
        r = r.with_header(h);
    }
    let _ = rq.respond(r);
}

fn handle_http(mut rq: tiny_http::Request, port: u16, token: &str, proxy: &EventLoopProxy<UserEvent>) {
    let host_ok = header(&rq, "Host").is_some_and(|h| h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}"));
    let origin_ok = header(&rq, "Origin").is_none_or(|o| o.starts_with("http://127.0.0.1") || o.starts_with("http://localhost"));
    if !host_ok || !origin_ok {
        return respond(rq, 403, r#"{"error":"forbidden"}"#.into());
    }
    let auth = header(&rq, "Authorization").and_then(|a| a.strip_prefix("Bearer ")).unwrap_or("");
    if !eq_const(auth.trim(), token) {
        return respond(rq, 401, r#"{"error":"missing or bad bearer token"}"#.into());
    }
    let path = rq.url().split('?').next().unwrap_or("").to_string();
    if path != "/mcp" && path != "/" {
        return respond(rq, 404, r#"{"error":"not found"}"#.into());
    }
    match rq.method() {
        tiny_http::Method::Post => {}
        tiny_http::Method::Delete => return respond(rq, 200, "{}".into()),
        _ => return respond(rq, 405, r#"{"error":"use POST"}"#.into()),
    }
    let mut body = String::new();
    if rq.as_reader().take(16 << 20).read_to_string(&mut body).is_err() {
        return respond(rq, 400, r#"{"error":"bad body"}"#.into());
    }
    let msg: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return respond(rq, 400, json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":e.to_string()}}).to_string()),
    };
    let out = match msg {
        Value::Array(batch) => {
            let r: Vec<Value> = batch.into_iter().filter_map(|m| rpc(m, proxy)).collect();
            if r.is_empty() { None } else { Some(Value::Array(r)) }
        }
        m => rpc(m, proxy),
    };
    match out {
        Some(v) => respond(rq, 200, v.to_string()),
        None => respond(rq, 202, String::new()),
    }
}

fn rpc(msg: Value, proxy: &EventLoopProxy<UserEvent>) -> Option<Value> {
    let id = msg.get("id").cloned()?; // notifications get no response
    let method = msg["method"].as_str().unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "valerie", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "You are controlling the user's Valerie browser. Tabs are identified by numeric ids from list_tabs. \
                Page tools default to the active tab. Use snapshot to get element refs, then click/type by ref. \
                Prefer opening new tabs in the background unless the user wants to watch."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Ok(match call_tool(name, args, proxy) {
                Ok(v) if v.get("__image_png").is_some() => {
                    json!({ "content": [{ "type": "image", "data": v["__image_png"], "mimeType": "image/png" }], "isError": false })
                }
                Ok(v) => json!({ "content": [{ "type": "text", "text": text(&v) }], "isError": false }),
                Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
            })
        }
        _ => Err(json!({ "code": -32601, "message": format!("unknown method {method}") })),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
    })
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        v => serde_json::to_string_pretty(v).unwrap_or_default(),
    }
}

fn tab_prop() -> Value {
    json!({ "type": "integer", "description": "Tab id from list_tabs. Defaults to the active tab." })
}

fn tool(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
    json!({ "name": name, "description": description,
            "inputSchema": { "type": "object", "properties": props, "required": required } })
}

fn tools() -> Vec<Value> {
    let commands: Vec<&str> = CATALOG.iter().map(|c| c.0).collect();
    vec![
        tool("list_spaces", "List spaces (tab groups with their own profile and theme) and which is active.", json!({}), &[]),
        tool("list_tabs", "List tabs: favorites, pinned and today tabs, with ids, urls, titles, space and state.",
             json!({ "space": { "type": "string", "description": "Only this space" } }), &[]),
        tool("open_tab", "Open a URL (or search terms) in a new tab.", json!({
            "url": { "type": "string" }, "space": { "type": "string", "description": "Space name; defaults to the active space" },
            "activate": { "type": "boolean", "description": "Switch to the tab (default false: open in background)" } }), &["url"]),
        tool("open_link", "Open a URL the way an external link would: Air Traffic Control rules, then a little window.",
             json!({ "url": { "type": "string" } }), &["url"]),
        tool("navigate", "Load a URL in an existing tab.", json!({ "url": { "type": "string" }, "tab": tab_prop() }), &["url"]),
        tool("activate_tab", "Switch to a tab (also switches space).", json!({ "tab": tab_prop() }), &["tab"]),
        tool("close_tab", "Close a tab.", json!({ "tab": tab_prop() }), &["tab"]),
        tool("history", "Go back, forward or reload a tab.", json!({
            "action": { "type": "string", "enum": ["back", "forward", "reload"] }, "tab": tab_prop() }), &["action"]),
        tool("read_page", "Read a page's URL, title and visible text (or HTML).", json!({
            "tab": tab_prop(), "max_chars": { "type": "integer", "default": 20000 }, "html": { "type": "boolean" } }), &[]),
        tool("snapshot", "List the page's interactive elements with refs for click/type. Refs reset on every snapshot.",
             json!({ "tab": tab_prop(), "only_in_view": { "type": "boolean" } }), &[]),
        tool("click", "Click an element by snapshot ref or CSS selector.", json!({
            "tab": tab_prop(), "ref": { "type": "string" }, "selector": { "type": "string" } }), &[]),
        tool("type", "Type text into an input/textarea/contenteditable by ref or selector.", json!({
            "tab": tab_prop(), "ref": { "type": "string" }, "selector": { "type": "string" }, "text": { "type": "string" },
            "clear": { "type": "boolean", "description": "Replace existing value" }, "submit": { "type": "boolean", "description": "Submit the form / press Enter afterwards" } }), &["text"]),
        tool("press_key", "Dispatch a key (e.g. Enter, Escape, ArrowDown) to the focused element.",
             json!({ "tab": tab_prop(), "key": { "type": "string" } }), &["key"]),
        tool("scroll", "Scroll the page or scroll an element into view.", json!({
            "tab": tab_prop(), "direction": { "type": "string", "enum": ["up", "down", "top", "bottom"] }, "ref": { "type": "string" } }), &[]),
        tool("wait_for", "Wait until a selector or text appears (and the page has loaded).", json!({
            "tab": tab_prop(), "selector": { "type": "string" }, "text": { "type": "string" }, "timeout_ms": { "type": "integer", "default": 10000 } }), &[]),
        tool("screenshot", "Capture what the tab looks like as a PNG image (the visible viewport).",
             json!({ "tab": tab_prop(), "width": { "type": "integer", "description": "Image width in points (default 1024)" } }), &[]),
        tool("eval_js", "Run JavaScript in the page as a function body; `return` a JSON-serialisable value. Promises are awaited.",
             json!({ "tab": tab_prop(), "script": { "type": "string" } }), &["script"]),
        tool("run_command", &format!(
            "Run any browser command, exactly like a keybinding would. Pass {{\"cmd\": name, ...args}}. No-arg commands: {}. \
             With args: switch_space {{index|name}}, new_space {{name}}, set_space_theme {{theme}}, go_to_tab {{index}}, toggle_pin {{tab}}, \
             toggle_favorite {{tab}}, move_tab {{tab, space?, pinned?, folder?, index?}}, rename_tab {{tab, title}}, split_view {{tab}}, \
             open_command_bar {{mode: new_tab|current}}, import {{source: arc|chrome|safari|firefox|brave|edge|vivaldi}}.",
            commands.join(", ")),
             json!({ "command": { "type": "object" } }), &["command"]),
    ]
}

/// Send one operation to the UI thread and wait for its answer.
pub fn call(proxy: &EventLoopProxy<UserEvent>, op: &str, args: Value) -> Result<Value, String> {
    let (tx, rx) = channel();
    proxy
        .send_event(UserEvent::Mcp(McpRequest { op: op.into(), args, reply: tx }))
        .map_err(|_| "browser is shutting down".to_string())?;
    rx.recv_timeout(Duration::from_secs(30)).map_err(|_| "timed out waiting for the browser".to_string())?
}

/// Evaluate a function body in a tab, awaiting promises.
fn eval(proxy: &EventLoopProxy<UserEvent>, tab: Option<i64>, script: &str) -> Result<Value, String> {
    let r = call(proxy, "eval", json!({ "tab": tab, "script": script }))?;
    if let Some(id) = r.get("pending").and_then(|p| p.as_str()) {
        let poll = format!("var r = window.__vres && window.__vres['{id}']; if (r) delete window.__vres['{id}']; return r || {{waiting: true}};");
        let start = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let v = call(proxy, "eval", json!({ "tab": tab, "script": poll }))?;
            let inner = v.get("ok").cloned().unwrap_or(Value::Null);
            if inner.get("waiting").is_none() {
                return unwrap_result(inner);
            }
            if start.elapsed() > Duration::from_secs(30) {
                return Err("promise did not settle within 30s".into());
            }
        }
    }
    unwrap_result(r)
}

fn unwrap_result(r: Value) -> Result<Value, String> {
    if let Some(e) = r.get("error") {
        return Err(format!("page error: {}", text(e)));
    }
    Ok(r.get("ok").cloned().unwrap_or(Value::Null))
}

fn wait_ready(proxy: &EventLoopProxy<UserEvent>, tab: Option<i64>) -> Result<Option<i64>, String> {
    let t = call(proxy, "ensure_loaded", json!({ "tab": tab }))?;
    let tab = t["tab"].as_i64();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if let Ok(v) = eval(proxy, tab, "return document.readyState") {
            if v == "complete" || (v == "interactive" && start.elapsed() > Duration::from_secs(2)) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    Ok(tab)
}

/// Prefix page scripts with their arguments as a JS literal.
fn with_args(args: &Value, body: &str) -> String {
    format!("const A = {};\n{}", args, body)
}

const FIND_JS: &str = r#"
function find() {
  const e = A.ref ? document.querySelector('[data-valerie-ref="' + CSS.escape(String(A.ref)) + '"]') : A.selector ? document.querySelector(A.selector) : document.activeElement;
  if (!e) throw new Error(A.ref ? "no element with ref " + A.ref + " (take a new snapshot)" : "no element matches " + A.selector);
  return e;
}
function describe(e) { return (e.tagName.toLowerCase() + " " + (e.innerText || e.value || e.getAttribute("aria-label") || "").trim().slice(0, 80)).trim(); }
"#;

const SNAPSHOT_JS: &str = r#"
const sel = 'a[href],button,input:not([type=hidden]),select,textarea,summary,[role=button],[role=link],[role=tab],[role=menuitem],[role=checkbox],[role=option],[role=textbox],[role=combobox],[contenteditable=true],[onclick]';
document.querySelectorAll('[data-valerie-ref]').forEach((e) => e.removeAttribute('data-valerie-ref'));
let n = 0; const out = [];
for (const e of document.querySelectorAll(sel)) {
  const r = e.getBoundingClientRect();
  if (r.width < 1 || r.height < 1) continue;
  const s = getComputedStyle(e);
  if (s.visibility === 'hidden' || s.display === 'none') continue;
  const inView = r.bottom > 0 && r.top < innerHeight && r.right > 0 && r.left < innerWidth;
  if (A.only_in_view && !inView) continue;
  const ref = String(++n); e.setAttribute('data-valerie-ref', ref);
  const label = (e.getAttribute('aria-label') || e.innerText || e.value || e.title || e.placeholder || e.name || '').trim().replace(/\s+/g, ' ').slice(0, 100);
  const o = { ref, tag: e.tagName.toLowerCase(), label };
  if (e.getAttribute('role')) o.role = e.getAttribute('role');
  if (e.type && e.tagName !== 'BUTTON') o.type = e.type;
  if (e.href) o.href = e.href;
  if (e.disabled) o.disabled = true;
  if (!inView) o.offscreen = true;
  out.push(o);
  if (n >= 500) break;
}
return { url: location.href, title: document.title, elements: out };
"#;

fn call_tool(name: &str, a: Value, proxy: &EventLoopProxy<UserEvent>) -> Result<Value, String> {
    let tab = a.get("tab").and_then(|t| t.as_i64());
    match name {
        "list_spaces" | "list_tabs" | "open_tab" | "open_link" | "navigate" | "activate_tab" | "close_tab" => call(proxy, name, a),
        "history" => {
            let cmd = match a["action"].as_str() {
                Some("back") => "back",
                Some("forward") => "forward",
                _ => "reload",
            };
            call(proxy, "run", json!({ "cmd": cmd, "tab": tab }))
        }
        "run_command" => call(proxy, "run", a["command"].clone()),
        "read_page" => {
            let tab = wait_ready(proxy, tab)?;
            let js = with_args(&a, "const max = A.max_chars || 20000; const body = A.html ? document.documentElement.outerHTML : (document.body ? document.body.innerText : ''); \
                return { url: location.href, title: document.title, truncated: body.length > max, content: body.slice(0, max) };");
            eval(proxy, tab, &js)
        }
        "snapshot" => {
            let tab = wait_ready(proxy, tab)?;
            eval(proxy, tab, &with_args(&a, SNAPSHOT_JS))
        }
        "click" => {
            let js = with_args(&a, &format!("{FIND_JS}\nconst e = find(); e.scrollIntoView({{block: 'center'}}); \
                if (e.focus) e.focus({{preventScroll: true}}); \
                for (const t of ['pointerdown','mousedown','pointerup','mouseup']) e.dispatchEvent(new MouseEvent(t, {{bubbles: true, cancelable: true, view: window}})); \
                e.click(); return 'clicked ' + describe(e);"));
            eval(proxy, tab, &js)
        }
        "type" => {
            let js = with_args(&a, &format!("{FIND_JS}\nconst e = find(); e.focus(); \
                if (e.isContentEditable) {{ if (A.clear) document.execCommand('selectAll'); document.execCommand('insertText', false, A.text); }} \
                else {{ const proto = e instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : e instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype; \
                  Object.getOwnPropertyDescriptor(proto, 'value').set.call(e, (A.clear ? '' : e.value) + A.text); \
                  e.dispatchEvent(new Event('input', {{bubbles: true}})); e.dispatchEvent(new Event('change', {{bubbles: true}})); }} \
                if (A.submit) {{ const opts = {{key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true}}; \
                  const go = e.dispatchEvent(new KeyboardEvent('keydown', opts)); e.dispatchEvent(new KeyboardEvent('keyup', opts)); \
                  if (go && e.form) (e.form.requestSubmit ? e.form.requestSubmit() : e.form.submit()); }} \
                return 'typed into ' + describe(e);"));
            eval(proxy, tab, &js)
        }
        "press_key" => {
            let js = with_args(&a, "const e = document.activeElement || document.body; \
                const opts = {key: A.key, code: A.key, bubbles: true, cancelable: true}; \
                const go = e.dispatchEvent(new KeyboardEvent('keydown', opts)); e.dispatchEvent(new KeyboardEvent('keyup', opts)); \
                if (go && A.key === 'Enter' && e.form) (e.form.requestSubmit ? e.form.requestSubmit() : e.form.submit()); \
                return 'pressed ' + A.key;");
            eval(proxy, tab, &js)
        }
        "scroll" => {
            let js = with_args(&a, &format!("{FIND_JS}\nif (A.ref) {{ const e = find(); e.scrollIntoView({{block: 'center'}}); return 'scrolled to ' + describe(e); }} \
                const s = document.scrollingElement || document.documentElement; const h = innerHeight * 0.8; \
                ({{down: () => s.scrollBy(0, h), up: () => s.scrollBy(0, -h), top: () => s.scrollTo(0, 0), bottom: () => s.scrollTo(0, s.scrollHeight)}}[A.direction || 'down'])(); \
                return {{ scrollY: s.scrollTop, scrollHeight: s.scrollHeight, viewport: innerHeight }};"));
            eval(proxy, tab, &js)
        }
        "wait_for" => {
            let tab = wait_ready(proxy, tab)?;
            let timeout = Duration::from_millis(a["timeout_ms"].as_u64().unwrap_or(10_000));
            let js = with_args(&a, "if (A.selector && !document.querySelector(A.selector)) return false; \
                if (A.text && !(document.body && document.body.innerText.includes(A.text))) return false; return true;");
            let start = Instant::now();
            loop {
                if eval(proxy, tab, &js)? == Value::Bool(true) {
                    return Ok(json!("ready"));
                }
                if start.elapsed() > timeout {
                    return Err("timed out".into());
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        "screenshot" => {
            let tab = wait_ready(proxy, tab)?;
            call(proxy, "screenshot", json!({ "tab": tab, "width": a.get("width") }))
        }
        "eval_js" => {
            let tab = wait_ready(proxy, tab)?;
            eval(proxy, tab, a["script"].as_str().unwrap_or(""))
        }
        _ => Err(format!("unknown tool {name}")),
    }
}

pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
    }
}

/// Minimal HTTP/1.1 POST to the local MCP endpoint. Returns (status, body).
pub fn post_local(port: u16, token: &str, body: &str) -> std::io::Result<(u16, String)> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(400))?;
    s.set_read_timeout(Some(Duration::from_secs(60)))?;
    write!(
        s,
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut resp = String::new();
    s.read_to_string(&mut resp)?;
    let status = resp.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = resp.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    Ok((status, body))
}

/// If Valerie is already running, hand it the URLs (as external links) and return true.
pub fn forward_open(port: u16, urls: &[String]) -> bool {
    let token = load_or_create_token();
    urls.iter().all(|u| {
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "open_link", "arguments": { "url": u } } });
        matches!(post_local(port, &token, &req.to_string()), Ok((200, _)))
    })
}

/// `valerie mcp-stdio`: bridge for MCP clients that only speak stdio.
pub fn stdio_bridge(port: u16) -> anyhow::Result<()> {
    let token = load_or_create_token();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut line = String::new();
    while { line.clear(); stdin.read_line(&mut line)? > 0 } {
        if line.trim().is_empty() {
            continue;
        }
        let out = match post_local(port, &token, line.trim()) {
            Ok((200, body)) => Some(body),
            Ok(_) => None,
            Err(_) => serde_json::from_str::<Value>(&line).ok().and_then(|m| m.get("id").cloned()).map(|id| {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": "Valerie is not running" } }).to_string()
            }),
        };
        if let Some(o) = out {
            writeln!(stdout, "{}", o.trim())?;
            stdout.flush()?;
        }
    }
    Ok(())
}
