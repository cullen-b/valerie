//! Built-in MCP server (Streamable HTTP, JSON responses) so agents can drive the browser.
//! Binds to 127.0.0.1, requires a bearer token, and rejects browser origins and foreign Host headers.
use crate::app::UserEvent;
use crate::command::{CATALOG, WITH_ARGS};
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

pub fn random_hex() -> String {
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

/// Compare two secrets without leaking how much of them matched.
pub fn same_secret(a: &str, b: &str) -> bool {
    eq_const(a, b)
}

fn eq_const(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn spawn(port: u16, token: String, proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        let server = match tiny_http::Server::http(("127.0.0.1", port)) {
            Ok(s) => s,
            Err(e) => return crate::log::line("mcp", format!("MCP server could not bind 127.0.0.1:{port}: {e}")),
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
    // A chat's agent says which access level its chat has; everyone else (your own MCP setup, `valerie call`) is full.
    let who = Caller {
        access: header(&rq, "X-Valerie-Access").unwrap_or("full").to_string(),
        chat: header(&rq, "X-Valerie-Chat").unwrap_or_default().to_string(),
    };
    let out = match msg {
        Value::Array(batch) => {
            let r: Vec<Value> = batch.into_iter().filter_map(|m| rpc(m, proxy, &who)).collect();
            if r.is_empty() { None } else { Some(Value::Array(r)) }
        }
        m => rpc(m, proxy, &who),
    };
    match out {
        Some(v) => respond(rq, 200, v.to_string()),
        None => respond(rq, 202, String::new()),
    }
}

/// Who's calling: the access level of the chat whose agent this is ("read" | "ask" | "full") and its id.
struct Caller {
    access: String,
    chat: String,
}

/// Tools that only look; everything else changes something and needs the chat's permission.
pub const LOOKING: &[&str] = &[
    "list_spaces", "list_tabs", "read_page", "snapshot", "screenshot", "wait_for", "read_chat", "list_chats", "search_history",
    "list_archived", "list_closed_tabs", "list_downloads", "site_state", "screenshot_ui", "read_ui", "read_logs", "get_settings",
    "list_pages", "read_console",
];

/// Check a tool call against the chat's access level, asking the user for "ask" chats.
fn allowed(name: &str, args: &Value, who: &Caller, proxy: &EventLoopProxy<UserEvent>) -> Result<(), String> {
    if LOOKING.contains(&name) || name == "approve" || who.access == "full" {
        return Ok(());
    }
    if who.access == "read" {
        return Err(format!("This chat is read-only, so `{name}` isn't allowed. Tell the user what you'd do instead; they can switch the chat to Ask first or Full access."));
    }
    match call_wait(proxy, "confirm", json!({ "chat": who.chat, "tool": name, "args": args }), 600)?["allowed"].as_bool() {
        Some(true) => Ok(()),
        _ => Err(format!("The user said no to `{name}`. Don't try it again; ask them what they'd like instead.")),
    }
}

fn rpc(msg: Value, proxy: &EventLoopProxy<UserEvent>, who: &Caller) -> Option<Value> {
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
            let started = Instant::now();
            let result = allowed(name, &args, who, proxy).and_then(|()| match name {
                // Claude Code's permission questions (--permission-prompt-tool): the user answers in Valerie.
                "approve" => {
                    let ok = call_wait(proxy, "confirm", json!({ "chat": who.chat, "tool": args["tool_name"], "args": args["input"] }), 600)?["allowed"].as_bool() == Some(true);
                    Ok(json!(if ok {
                        json!({ "behavior": "allow", "updatedInput": args["input"] }).to_string()
                    } else {
                        json!({ "behavior": "deny", "message": "The user said no." }).to_string()
                    }))
                }
                _ => call_tool(name, args.clone(), proxy),
            });
            log_call(name, &args, who, started.elapsed(), &result);
            Ok(match result {
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

/// One log line per tool call: who, which tool, how long it took and any error (never what was typed or the script).
fn log_call(name: &str, args: &Value, who: &Caller, took: Duration, result: &Result<Value, String>) {
    if name == "read_logs" {
        return;
    }
    let by = if who.chat.is_empty() { "outside agent".to_string() } else { format!("chat {}", who.chat) };
    let tab = args.get("tab").and_then(Value::as_i64).map(|t| format!(" tab {t}")).unwrap_or_default();
    let what = match name {
        "run_command" => format!(" {}", args["command"]["cmd"].as_str().unwrap_or("?")),
        "set_setting" | "get_settings" => format!(" {}", args["key"].as_str().unwrap_or("")),
        "show_page" => format!(" {}", args["name"].as_str().unwrap_or("")),
        _ => String::new(),
    };
    let ms = took.as_millis();
    let slow = if ms >= 5000 { " (slow)" } else { "" };
    match result {
        Ok(_) => crate::log::event("mcp", format!("{by}: {name}{what}{tab} {ms}ms{slow}")),
        Err(e) => crate::log::line("mcp", format!("{by}: {name}{what}{tab} failed after {ms}ms: {}", e.chars().take(300).collect::<String>())),
    }
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

pub fn tools() -> Vec<Value> {
    let commands: Vec<&str> = CATALOG.iter().map(|c| c.0).collect();
    let with_args: Vec<String> = WITH_ARGS.iter().map(|(n, a)| format!("{n} {a}")).collect();
    vec![
        tool("list_spaces", "List spaces (tab groups with their own profile and theme) and which is active.", json!({}), &[]),
        tool("list_chats", "List the user's chats with Valerie's agents (ids, titles, when they were last used).", json!({}), &[]),
        tool("read_chat", "Read a whole chat by id (the user may paste one as chat:<id> to share its context).",
             json!({ "id": { "type": "string" } }), &["id"]),
        tool("approve", "Ask the user to approve an action (used for permission prompts).",
             json!({ "tool_name": { "type": "string" }, "input": { "type": "object" } }), &["tool_name"]),
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
        tool("read_console", "Read what the page wrote to its console since it loaded, oldest first: console.log/info/warn/error/debug, \
             uncaught errors, failed promises and files that failed to load. Use it to check a page or app you're building.",
             json!({ "tab": tab_prop(), "level": { "type": "string", "enum": ["error", "warn", "info", "log", "debug"], "description": "Only this level (warn includes errors)" },
                     "limit": { "type": "integer", "default": 100 } }), &[]),
        tool("eval_js", "Run JavaScript in the page as a function body; `return` a JSON-serialisable value. Promises are awaited.",
             json!({ "tab": tab_prop(), "script": { "type": "string" } }), &["script"]),
        tool("run_command", &format!(
            "Run any browser command, exactly like a keybinding would. Pass {{\"cmd\": name, ...options}}. No-option commands: {}. \
             With options: {}.",
            commands.join(", "), with_args.join("; ")),
             json!({ "command": { "type": "object" } }), &["command"]),
        tool("search_history", "Search the pages the user visited (title or address contains the words), most recent first.",
             json!({ "query": { "type": "string" }, "limit": { "type": "integer", "default": 50 } }), &[]),
        tool("delete_history", "Remove pages from history: those whose title or address contains `query`, exactly `url`, or visited in the last `since_minutes`.",
             json!({ "query": { "type": "string" }, "url": { "type": "string" }, "since_minutes": { "type": "integer" } }), &[]),
        tool("list_archived", "List tabs Valerie archived (closed after a while unused), newest first; reopen one with run_command restore_archived.",
             json!({ "query": { "type": "string" }, "limit": { "type": "integer", "default": 50 } }), &[]),
        tool("list_closed_tabs", "List recently closed tabs, most recent first (reopen_closed_tab brings back the latest).", json!({}), &[]),
        tool("list_downloads", "List finished downloads: file name, where it was saved, the address it came from and when.", json!({}), &[]),
        tool("clear_site_data", "Clear a site's cookies, caches and storage (signs the user out of it). Always asks the user first.",
             json!({ "site": { "type": "string", "description": "e.g. example.com; defaults to the active tab's site" } }), &[]),
        tool("site_state", "What Valerie remembers about a site: zoom, dark mode, notifications, ad blocking, hidden elements, boosts, \
             plugins and per-site settings.", json!({ "site": { "type": "string", "description": "Defaults to the active tab's site" } }), &[]),
        tool("show_page", "Make or update a page and show it in a tab: a whole HTML document saved as pages/<name>.html in the \
             config folder and opened at valerie://page/<name>. Use it for anything worth seeing rather than reading: a dashboard, \
             a chart, a table, a little tool. It stays (the user finds it with \"open_page\" or the command bar) and can use the \
             browser itself: `await valerie.call(\"<tool>\", {…})` for these same tools (looking ones freely; list others in \
             <meta name=\"valerie-tools\" content=\"open_tab close_tab\">, the user approves once), `valerie.log(…)` into the log \
             (area page:<name>), `valerie.on(\"downloads\", fn)` when downloads change. Colours: var(--v-bg), --v-fg, --v-dim, \
             --v-line, --v-card, --v-accent. Only <script> tags run (no onclick=\"…\" attributes; use addEventListener); \
             scripts from https:// load.", json!({
            "name": { "type": "string", "description": "Letters, digits, - and _ (e.g. tab-report)" },
            "html": { "type": "string", "description": "The whole page" },
            "activate": { "type": "boolean", "description": "Switch to it (default true)" } }), &["name", "html"]),
        tool("list_pages", "List the pages at valerie://page/<name>: ones agents or the user made (pages/ in the config folder) and \
             the built-in downloads and history pages (a pages/downloads.html or history.html replaces those).", json!({}), &[]),
        tool("read_logs", "Read Valerie's log, oldest first: failures, messages the user saw, commands run, agents' tool calls with \
             how long they took, page crashes, and plugins' own messages (`valerie.log` in a plugin script, area `plugin:<name>`). \
             Read it first when something didn't work, including a plugin or page you just made.", json!({
            "area": { "type": "string", "description": "Only this area, e.g. sync, update, mcp, command, page, plugin (all plugins) or plugin:<name>" },
            "contains": { "type": "string", "description": "Only entries containing this text (any case)" },
            "since_minutes": { "type": "integer", "description": "Only the last this many minutes" },
            "limit": { "type": "integer", "default": 100 } }), &[]),
        tool("get_settings", "Read a setting or a section of Valerie's settings: the value in use, what the user set themselves, and \
             the reference text that says what it does and which values it takes. No key: the section names and the user's own file.",
             json!({ "key": { "type": "string", "description": "e.g. sidebar, or sidebar.tab_height" } }), &[]),
        tool("set_setting", "Change one setting (takes effect at once). Checked like `valerie check-config`: a wrong name or value \
             changes nothing and says why. The user can undo it with the undo_customization command.", json!({
            "key": { "type": "string", "description": "e.g. sidebar.tab_height (see get_settings)" },
            "value": { "description": "The new value: a string, number, true/false, list or object" },
            "this_computer": { "type": "boolean", "description": "Only on this computer (this-mac.yaml), not synced to the others" } }), &["key", "value"]),
        tool("screenshot_ui", "Capture Valerie's own interface as a PNG (to check a change to its look).",
             json!({ "part": { "type": "string", "enum": ["sidebar", "command_bar", "chats", "welcome", "plugins"], "default": "sidebar" },
                     "width": { "type": "integer" } }), &[]),
        tool("read_ui", "Read Valerie's own interface: its visible text and an outline of elements with their ids and classes \
             (the names to use in ui/*.css).", json!({ "part": { "type": "string", "enum": ["sidebar", "command_bar", "chats", "welcome", "plugins"], "default": "sidebar" } }), &[]),
    ]
}

/// Every tool's name (what plugin `permissions` can list).
pub fn tool_names() -> Vec<String> {
    tools().iter().filter_map(|t| t["name"].as_str().map(str::to_string)).filter(|n| n != "approve").collect()
}

/// Run a tool for a plugin's page script (off the main thread: tools wait for the browser).
pub fn run_tool(name: &str, args: Value, proxy: &EventLoopProxy<UserEvent>) -> Result<Value, String> {
    if name == "clear_site_data" {
        return call_wait(proxy, "clear_site_data", args, 600);
    }
    call_tool(name, args, proxy)
}

/// Send one operation to the UI thread and wait for its answer.
pub fn call(proxy: &EventLoopProxy<UserEvent>, op: &str, args: Value) -> Result<Value, String> {
    call_wait(proxy, op, args, 30)
}

/// Like `call`, waiting up to `secs` (asking the user takes longer than the browser does).
fn call_wait(proxy: &EventLoopProxy<UserEvent>, op: &str, args: Value, secs: u64) -> Result<Value, String> {
    let (tx, rx) = channel();
    proxy
        .send_event(UserEvent::Mcp(McpRequest { op: op.into(), args, reply: tx }))
        .map_err(|_| "browser is shutting down".to_string())?;
    rx.recv_timeout(Duration::from_secs(secs)).map_err(|_| "timed out waiting for the browser".to_string())?
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
        "list_spaces" | "list_tabs" | "open_tab" | "open_link" | "navigate" | "activate_tab" | "close_tab" | "read_chat" | "list_chats"
        | "search_history" | "delete_history" | "list_archived" | "list_closed_tabs" | "list_downloads" | "site_state" | "read_ui" => call(proxy, name, a),
        "clear_site_data" => call_wait(proxy, "clear_site_data", a, 600),
        "screenshot_ui" | "get_settings" | "list_pages" => call(proxy, name, a),
        "show_page" => {
            let page = a["name"].as_str().unwrap_or("").trim().to_string();
            let html = a["html"].as_str().unwrap_or("");
            if page.is_empty() || page.len() > 60 || !page.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Err("name must be letters, digits, - and _ (up to 60)".into());
            }
            if html.trim().is_empty() || html.len() > 4 << 20 {
                return Err("html must be a whole page, up to 4 MB".into());
            }
            crate::app::snapshot_config(&format!("page {page}"));
            let dir = crate::config::config_dir().join("pages");
            std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(dir.join(format!("{page}.html")), html)).map_err(|e| e.to_string())?;
            crate::log::event(&format!("page:{page}"), "saved by an agent");
            let shown = call(proxy, "open_page", json!({ "name": page, "activate": a["activate"].as_bool().unwrap_or(true) }))?;
            Ok(json!({ "url": shown["url"], "tab": shown["tab"], "file": dir.join(format!("{page}.html")),
                       "next": "Check it: screenshot the tab, and read_logs with area page:<name> for script errors." }))
        }
        "read_logs" => {
            let limit = a["limit"].as_u64().unwrap_or(100).clamp(1, 2000) as usize;
            let entries = crate::log::read(a["area"].as_str().unwrap_or(""), a["contains"].as_str().unwrap_or(""), a["since_minutes"].as_u64(), limit);
            Ok(json!({ "folder": crate::log::dir(), "entries": entries }))
        }
        "set_setting" => {
            let key = a["key"].as_str().unwrap_or("").trim().to_string();
            let path: Vec<&str> = key.split('.').filter(|k| !k.is_empty()).collect();
            if path.is_empty() {
                return Err("give the setting's key, e.g. sidebar.tab_height".into());
            }
            let value = match &a["value"] {
                Value::Null => return Err("give a value".into()),
                v => v.to_string(),
            };
            crate::app::snapshot_config(&format!("set {key}"));
            crate::config::change_setting(&path, &value, a["this_computer"].as_bool().unwrap_or(false)).map_err(|e| format!("{e:#}"))?;
            crate::log::event("settings", format!("{key} changed by an agent"));
            Ok(json!(format!("{key} is now {value}. The user can undo it with undo_customization.")))
        }
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
        "read_console" => {
            let js = with_args(&a, "const all = window.__valerie && window.__valerie.consoleLines ? window.__valerie.consoleLines() : []; \
                const keep = { error: ['error'], warn: ['error', 'warn'] }[A.level] || (A.level ? [A.level] : null); \
                const lines = all.filter((l) => !keep || keep.includes(l.level)); \
                return lines.slice(-(A.limit || 100)).map((l) => ({ level: l.level, text: l.text, seconds_ago: Math.round((Date.now() - l.at) / 1000) }));");
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

pub fn unbase64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for chunk in s.trim_end_matches('=').as_bytes().chunks(4) {
        let v: Vec<u8> = chunk.iter().map(|c| val(*c)).collect::<Option<_>>()?;
        let n = v.iter().enumerate().fold(0u32, |n, (i, x)| n | (*x as u32) << (18 - 6 * i));
        out.extend_from_slice(&n.to_be_bytes()[1..v.len()]);
    }
    Some(out)
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

    #[test]
    fn unbase64_round_trips() {
        for s in [&b""[..], b"f", b"fo", b"foo", b"foobar", &[0, 255, 7, 128, 64]] {
            assert_eq!(super::unbase64(&super::base64(s)).as_deref(), Some(s));
        }
    }

    #[test]
    fn dechunk_joins_chunks() {
        assert_eq!(super::dechunk(b"4\r\nWiki\r\n5;x=1\r\npedia\r\n0\r\n\r\n"), b"Wikipedia");
    }
}

/// Minimal HTTP/1.1 POST to the local MCP endpoint. Returns (status, body).
pub fn post_local(port: u16, token: &str, body: &str) -> std::io::Result<(u16, String)> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(400))?;
    // Long enough for the user to answer an approval question.
    s.set_read_timeout(Some(Duration::from_secs(660)))?;
    // A chat's agent carries its chat's access level (set by Valerie when it starts the agent).
    let mut extra = String::new();
    for (h, var) in [("X-Valerie-Access", "VALERIE_ACCESS"), ("X-Valerie-Chat", "VALERIE_CHAT")] {
        if let Some(v) = std::env::var(var).ok().map(|v| v.replace(['\r', '\n'], "")).filter(|v| !v.is_empty()) {
            extra.push_str(&format!("{h}: {v}\r\n"));
        }
    }
    write!(
        s,
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\n\
         {extra}Accept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut resp = Vec::new();
    s.read_to_end(&mut resp)?;
    let split = resp.windows(4).position(|w| w == b"\r\n\r\n").map_or(resp.len(), |i| i + 4);
    let head = String::from_utf8_lossy(&resp[..split]).to_ascii_lowercase();
    let status = head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let raw = &resp[split..];
    let body = if head.contains("transfer-encoding: chunked") { dechunk(raw) } else { raw.to_vec() };
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

/// Decode an HTTP chunked body (big responses, like screenshots, arrive this way).
fn dechunk(mut raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(eol) = raw.windows(2).position(|w| w == b"\r\n") {
        let size = std::str::from_utf8(&raw[..eol]).ok().and_then(|l| usize::from_str_radix(l.split(';').next()?.trim(), 16).ok());
        let Some(size) = size.filter(|n| *n > 0) else { break };
        let start = eol + 2;
        let end = (start + size).min(raw.len());
        out.extend_from_slice(&raw[start..end]);
        raw = &raw[(end + 2).min(raw.len())..];
    }
    out
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
pub fn stdio_bridge(port: u16, access: Option<String>, chat: Option<String>) -> anyhow::Result<()> {
    // Some CLIs start MCP servers with a scrubbed environment, so the access level also arrives as arguments.
    if let Some(a) = access {
        std::env::set_var("VALERIE_ACCESS", a);
    }
    if let Some(c) = chat {
        std::env::set_var("VALERIE_CHAT", c);
    }
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
