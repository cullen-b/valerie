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

/// mcp.web_tools: whether agents are offered web_search and fetch_page (kept in step with the settings by the app).
static WEB_TOOLS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_web_tools(on: bool) {
    WEB_TOOLS.store(on, std::sync::atomic::Ordering::Relaxed);
}

const WEB: &[&str] = &["web_search", "fetch_page"];

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

const INSTRUCTIONS: &str = "Valerie is the user's own web browser; these tools drive it.\n\
Reading the web: web_search finds pages (title, url and snippet from the user's search engine) and fetch_page reads any \
URL as clean Markdown (or text/HTML, optionally with its links). They run in a hidden tab with the user's logins and a \
real browser engine, so sign-in pages, JavaScript apps and bot-shy sites work, and they never touch the user's tabs or \
history. Use them whenever you need to search or read something online.\n\
Working in the user's tabs: list_tabs gives tab ids; page tools default to the active tab. read_page reads a tab \
(format: markdown), snapshot lists clickable elements with refs, then click / type / press_key by ref (snapshot again \
after the page changes). wait_for waits for text or a selector, screenshot shows the page, eval_js runs JavaScript. Open \
tabs with activate: false unless the user wants to watch. run_command does anything a keyboard shortcut can. When \
something fails, read_logs says why.";

/// Who's calling: the access level of the chat whose agent this is ("read" | "ask" | "full") and its id.
struct Caller {
    access: String,
    chat: String,
}

/// Tools that only look; everything else changes something and needs the chat's permission.
pub const LOOKING: &[&str] = &[
    "list_spaces", "list_tabs", "read_page", "snapshot", "screenshot", "wait_for", "read_chat", "list_chats", "search_history",
    "list_archived", "list_closed_tabs", "list_downloads", "site_state", "screenshot_ui", "read_ui", "read_logs", "get_settings",
    "list_pages", "read_console", "read_terminal", "web_search", "fetch_page",
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
            "instructions": INSTRUCTIONS
        })),
        "ping" => Ok(json!({})),
        // Some apps ask for these whatever the server offers; an empty list beats an error.
        "resources/list" => Ok(json!({ "resources": [] })),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        // `approve` is only for Valerie's own chats (Claude Code's permission prompts); it'd only confuse other agents.
        "tools/list" => Ok(json!({ "tools": tools().into_iter().filter(|t| !who.chat.is_empty() || t["name"] != "approve").collect::<Vec<_>>() })),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let args = loosen(name, params.get("arguments").cloned().filter(|a| !a.is_null()).unwrap_or(json!({})));
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
                // The terminal tools default to the calling chat's own shell.
                "read_terminal" | "run_in_terminal" => {
                    let mut a = args.clone();
                    a["_chat"] = json!(who.chat);
                    call(proxy, name, a)
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
    // Hints apps use to decide what needs the user's OK (MCP tool annotations).
    let mut hints = json!({ "readOnlyHint": LOOKING.contains(&name) });
    if ["close_tab", "delete_history", "clear_site_data"].contains(&name) {
        hints["destructiveHint"] = json!(true);
    }
    if WEB.contains(&name) {
        hints["openWorldHint"] = json!(true);
    }
    json!({ "name": name, "description": description, "annotations": hints,
            "inputSchema": { "type": "object", "properties": props, "required": required } })
}

/// Models don't all send the types a schema asks for (`"tab": "12"`, `"full_page": "true"`, a bare command name):
/// take what they meant.
fn loosen(name: &str, mut args: Value) -> Value {
    if let Value::String(s) = &args {
        args = serde_json::from_str(s).unwrap_or(json!({}));
    }
    let Some(obj) = args.as_object_mut() else { return json!({}) };
    if name == "run_command" {
        match obj.get("command").cloned() {
            Some(Value::String(c)) => {
                let c = serde_json::from_str::<Value>(&c).ok().filter(Value::is_object).unwrap_or(json!({ "cmd": c }));
                obj.insert("command".into(), c);
            }
            None if obj.contains_key("cmd") => {
                let c = Value::Object(std::mem::take(obj));
                obj.insert("command".into(), c);
            }
            _ => {}
        }
    }
    let Some(t) = tools().into_iter().find(|t| t["name"] == name) else { return args };
    let props = &t["inputSchema"]["properties"];
    for (k, v) in obj.iter_mut() {
        let want = props[k.as_str()]["type"].as_str().unwrap_or("");
        let fixed = match (want, &*v) {
            ("integer", Value::String(s)) => s.trim().parse::<i64>().ok().map(Value::from),
            ("integer", Value::Number(n)) if n.as_i64().is_none() => n.as_f64().map(|f| json!(f.round() as i64)),
            ("boolean", Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" | "1" => Some(json!(true)),
                "false" | "no" | "0" => Some(json!(false)),
                _ => None,
            },
            ("string", Value::Number(_) | Value::Bool(_)) => Some(json!(v.to_string())),
            _ => None,
        };
        if let Some(f) = fixed {
            *v = f;
        }
    }
    args
}

pub fn tools() -> Vec<Value> {
    let commands: Vec<&str> = CATALOG.iter().map(|c| c.0).collect();
    let with_args: Vec<String> = WITH_ARGS.iter().map(|(n, a)| format!("{n} {a}")).collect();
    let web = WEB_TOOLS.load(std::sync::atomic::Ordering::Relaxed);
    let all = vec![
        tool("web_search", "Search the web and get the results: title, url and snippet for each, from the user's search \
             engine in a hidden tab (the user's tabs and history aren't touched). Read a result with fetch_page.", json!({
            "query": { "type": "string", "description": "What to search for; operators like site:example.com work" },
            "limit": { "type": "integer", "description": "Most results to return (default 10, up to 30)" } }), &["query"]),
        tool("fetch_page", "Read a web page as clean Markdown (like a scraper, but in a real browser with the user's logins, so \
             JavaScript apps and pages behind sign-in work). Loads in a hidden tab that closes afterwards; the user's tabs and \
             history aren't touched. Use links: true to also get every link on the page (to find more pages to read).", json!({
            "url": { "type": "string" },
            "format": { "type": "string", "enum": ["markdown", "text", "html"], "description": "Default markdown" },
            "main_only": { "type": "boolean", "description": "Only the main content, without menus, headers and footers (default true)" },
            "links": { "type": "boolean", "description": "Also list the page's links" },
            "wait_for": { "type": "string", "description": "A CSS selector or text to wait for before reading (for slow pages)" },
            "max_chars": { "type": "integer", "description": "Longest content to return (default 40000)" } }), &["url"]),
        tool("list_spaces", "List spaces (tab groups with their own profile and theme) and which is active.", json!({}), &[]),
        tool("list_chats", "List the user's chats with Valerie's agents (ids, titles, when they were last used).", json!({}), &[]),
        tool("read_chat", "Read a whole chat by id (the user may paste one as chat:<id> to share its context).",
             json!({ "id": { "type": "string" } }), &["id"]),
        tool("approve", "Ask the user to approve an action (used for permission prompts).",
             json!({ "tool_name": { "type": "string" }, "input": { "type": "object", "additionalProperties": true } }), &["tool_name"]),
        tool("list_tabs", "List tabs: favorites, pinned and today tabs, with ids, urls, titles, space and state.",
             json!({ "space": { "type": "string", "description": "Only this space" } }), &[]),
        tool("open_tab", "Open a URL (or search terms) in a new tab.", json!({
            "url": { "type": "string" }, "space": { "type": "string", "description": "Space name; defaults to the active space" },
            "activate": { "type": "boolean", "description": "Switch to the tab (default false: open in background)" } }), &["url"]),
        tool("open_link", "Open a URL the way an external link would: Air Traffic Control rules, then a little window. valerie://little opens an empty little window that asks where to go.",
             json!({ "url": { "type": "string" } }), &["url"]),
        tool("navigate", "Load a URL in an existing tab.", json!({ "url": { "type": "string" }, "tab": tab_prop() }), &["url"]),
        tool("activate_tab", "Switch to a tab (also switches space).", json!({ "tab": tab_prop() }), &["tab"]),
        tool("close_tab", "Close a tab.", json!({ "tab": tab_prop() }), &["tab"]),
        tool("history", "Go back, forward or reload a tab.", json!({
            "action": { "type": "string", "enum": ["back", "forward", "reload"] }, "tab": tab_prop() }), &["action"]),
        tool("read_page", "Read an open tab's URL, title and content: visible text (default), clean Markdown or HTML.", json!({
            "tab": tab_prop(), "max_chars": { "type": "integer", "default": 20000 },
            "format": { "type": "string", "enum": ["text", "markdown", "html"], "description": "Default text" } }), &[]),
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
        tool("screenshot", "Capture what the tab looks like as a PNG image: the visible viewport, or with full_page the whole page top to bottom.",
             json!({ "tab": tab_prop(), "width": { "type": "integer", "description": "Image width in points (default 1024; not used with full_page)" },
                     "full_page": { "type": "boolean", "description": "The whole page, not just what's on screen (it's scrolled through once first, so lazy images load)" } }), &[]),
        tool("read_console", "Read what the page wrote to its console since it loaded, oldest first: console.log/info/warn/error/debug, \
             uncaught errors, failed promises and files that failed to load. Use it to check a page or app you're building.",
             json!({ "tab": tab_prop(), "level": { "type": "string", "enum": ["error", "warn", "info", "log", "debug"], "description": "Only this level (warn includes errors)" },
                     "limit": { "type": "integer", "default": 100 } }), &[]),
        tool("read_terminal", "Read what a shell in Valerie's terminal panel shows (the last lines, as on screen): a dev server or \
             test run you or the user started there. Defaults to your chat's own terminal, else the one on screen; the reply \
             lists every open terminal.",
             json!({ "terminal": { "type": "integer", "description": "A terminal id from the list" },
                     "lines": { "type": "integer", "default": 200 } }), &[]),
        tool("run_in_terminal", "Start a command in a new shell in Valerie's terminal panel, where the user can see it and stop it \
             (Ctrl-C), e.g. a dev server or a watcher. It runs in your project folder unless you give one, and keeps going after \
             you answer; read its output with read_terminal. For quick commands, use your own shell instead.",
             json!({ "command": { "type": "string", "description": "One line, as you'd type it" },
                     "folder": { "type": "string", "description": "Where to run it (default: your project folder)" } }), &["command"]),
        tool("eval_js", "Run JavaScript in the page as a function body; `return` a JSON-serialisable value. Promises are awaited.",
             json!({ "tab": tab_prop(), "script": { "type": "string" } }), &["script"]),
        tool("run_command", &format!(
            "Run any browser command, exactly like a keybinding would. Pass {{\"cmd\": name, ...options}}. No-option commands: {}. \
             With options: {}.",
            commands.join(", "), with_args.join("; ")),
             json!({ "command": { "type": "object", "properties": { "cmd": { "type": "string", "description": "The command's name" } },
                                  "required": ["cmd"], "additionalProperties": true } }), &["command"]),
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
            "value": { "type": "string", "description": "The new value as you'd write it in the YAML file: 32, true, bottom, [a, b] or { x: 1 }" },
            "this_computer": { "type": "boolean", "description": "Only on this computer (this-mac.yaml), not synced to the others" } }), &["key", "value"]),
        tool("screenshot_ui", "Capture Valerie's own interface as a PNG (to check a change to its look).",
             json!({ "part": { "type": "string", "enum": ["sidebar", "command_bar", "chats", "welcome", "plugins"], "default": "sidebar" },
                     "width": { "type": "integer" } }), &[]),
        tool("read_ui", "Read Valerie's own interface: its visible text and an outline of elements with their ids and classes \
             (the names to use in ui/*.css).", json!({ "part": { "type": "string", "enum": ["sidebar", "command_bar", "chats", "welcome", "plugins"], "default": "sidebar" } }), &[]),
    ];
    all.into_iter().filter(|t| web || !WEB.contains(&t["name"].as_str().unwrap_or(""))).collect()
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

/// Scrolls through the page once (images that load as they come into view do), back to where it was, with Valerie's
/// own bits (find bar, toasts) hidden for the picture.
const BEFORE_FULL_PAGE: &str = r#"
const se = document.scrollingElement || document.documentElement, x0 = scrollX, y0 = scrollY;
const go = (y) => scrollTo({ left: x0, top: y, behavior: "instant" }), wait = (ms) => new Promise((r) => setTimeout(r, ms));
return (async () => {
  for (let y = 0, n = 0; y < se.scrollHeight && n < 60; y += innerHeight * 0.85, n++) { go(y); await wait(90); }
  go(se.scrollHeight); await wait(250); go(y0); await wait(150);
  document.querySelectorAll("valerie-ui").forEach((e) => (e.style.visibility = "hidden"));
  return true;
})();"#;

/// The whole page as a PNG, top to bottom.
pub fn full_page(proxy: &EventLoopProxy<UserEvent>, tab: Option<i64>) -> Result<Vec<u8>, String> {
    let _ = eval(proxy, tab, BEFORE_FULL_PAGE);
    let shot = call_wait(proxy, "screenshot", json!({ "tab": tab, "full_page": true }), 60);
    let _ = eval(proxy, tab, r#"document.querySelectorAll("valerie-ui").forEach((e) => (e.style.visibility = "")); return true;"#);
    let b64 = shot?["__image_png"].as_str().map(str::to_string).ok_or("no picture came back")?;
    unbase64(&b64).ok_or_else(|| "the picture came back damaged".to_string())
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

/// The page (or its main content with `A.main_only`) as Markdown, text or HTML; `A.links` adds every link on the page.
const MARKDOWN_JS: &str = r#"
const abs = (u) => { try { return new URL(u, location.href).href; } catch (e) { return ''; } };
const SKIP = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'SVG', 'CANVAS', 'IFRAME', 'OBJECT', 'EMBED', 'HEAD', 'BUTTON', 'SELECT', 'INPUT', 'TEXTAREA', 'DIALOG', 'VALERIE-UI']);
const CHROME = new Set(['NAV', 'FOOTER', 'ASIDE', 'HEADER']);
const CHROME_ROLES = /^(navigation|banner|contentinfo|complementary|search|menu|menubar|dialog|alertdialog)$/;
const len = (e) => (e && e.innerText || '').trim().length;
let root = document.body || document.documentElement;
if (A.main_only !== false) {
  const main = document.querySelector('main, [role=main]');
  const articles = [...document.querySelectorAll('article')].sort((a, b) => len(b) - len(a));
  if (main && len(main) > 200) root = main;
  else if (articles[0] && len(articles[0]) > 200) root = articles[0];
}
const hidden = (e) => e.hidden || e.getAttribute('aria-hidden') === 'true' || (e.checkVisibility ? !e.checkVisibility() : getComputedStyle(e).display === 'none');
const skip = (e) => SKIP.has(e.tagName) || hidden(e) || (A.main_only !== false && e !== root && (CHROME.has(e.tagName) && !e.closest('article') || CHROME_ROLES.test(e.getAttribute('role') || '')));
const inline = (s) => s.replace(/\s*\n\s*/g, ' ').trim();
function kids(e, ctx) { let out = ''; for (const c of e.childNodes) out += md(c, ctx); return out; }
function list(e, ctx) {
  let i = Number(e.getAttribute('start') || 1), out = '\n\n';
  for (const li of e.children) {
    if (li.tagName !== 'LI' || skip(li)) continue;
    const mark = e.tagName === 'OL' ? (i++) + '. ' : '- ';
    const body = kids(li, ctx).trim().replace(/\n{2,}/g, '\n').replace(/\n/g, '\n  ');
    if (body) out += mark + body + '\n';
  }
  return out + '\n';
}
function table(e, ctx) {
  const rows = [...e.rows].slice(0, 300).map((r) => [...r.cells].map((c) => inline(kids(c, ctx)).replace(/\|/g, '\\|')));
  if (!rows.length) return '';
  const n = Math.max(...rows.map((r) => r.length));
  if (n < 2) return '\n\n' + rows.map((r) => r.join(' ')).join('\n') + '\n\n';
  const line = (r) => '| ' + Array.from({ length: n }, (_, i) => r[i] || '').join(' | ') + ' |';
  return '\n\n' + [line(rows[0]), '|' + ' --- |'.repeat(n), ...rows.slice(1).map(line)].join('\n') + '\n\n';
}
function md(n, ctx) {
  if (n.nodeType === 3) return ctx.pre ? n.data : n.data.replace(/\s+/g, ' ');
  if (n.nodeType !== 1 || skip(n)) return '';
  const t = n.tagName;
  switch (t) {
    case 'H1': case 'H2': case 'H3': case 'H4': case 'H5': case 'H6': {
      const s = inline(kids(n, ctx)); return s ? '\n\n' + '#'.repeat(+t[1]) + ' ' + s + '\n\n' : '';
    }
    case 'BR': return '\n';
    case 'HR': return '\n\n---\n\n';
    case 'A': {
      const s = inline(kids(n, ctx)), h = n.getAttribute('href') || '';
      if (!s) return '';
      return /^(javascript:|#)/i.test(h) || !h ? s : '[' + s + '](' + abs(h) + ')';
    }
    case 'IMG': {
      const src = abs(n.currentSrc || n.getAttribute('src') || '');
      if (!src.startsWith('http') || (n.width && n.width < 24)) return '';
      return '![' + (n.getAttribute('alt') || '').trim() + '](' + src + ')';
    }
    case 'STRONG': case 'B': { const s = kids(n, ctx); return s.trim() ? '**' + s.trim() + '** ' : s; }
    case 'EM': case 'I': { const s = kids(n, ctx); return s.trim() ? '*' + s.trim() + '* ' : s; }
    case 'S': case 'DEL': { const s = kids(n, ctx); return s.trim() ? '~~' + s.trim() + '~~' : s; }
    case 'CODE': return ctx.pre ? n.textContent : '`' + n.textContent.trim() + '`';
    case 'PRE': {
      const lang = ((n.querySelector('code') || n).className.match(/language-([\w+-]+)/) || [])[1] || '';
      return '\n\n```' + lang + '\n' + n.innerText.replace(/\n+$/, '') + '\n```\n\n';
    }
    case 'BLOCKQUOTE': return '\n\n' + kids(n, ctx).trim().split('\n').map((l) => '> ' + l).join('\n') + '\n\n';
    case 'UL': case 'OL': return list(n, ctx);
    case 'TABLE': return table(n, ctx);
    case 'LI': return '\n- ' + kids(n, ctx).trim() + '\n';
    case 'DT': return '\n\n**' + inline(kids(n, ctx)) + '**\n';
    case 'P': case 'DIV': case 'SECTION': case 'ARTICLE': case 'MAIN': case 'HEADER': case 'FOOTER': case 'FIGURE': case 'FIGCAPTION':
    case 'DL': case 'DD': case 'ADDRESS': case 'DETAILS': case 'SUMMARY': case 'FORM': case 'FIELDSET': case 'NAV': case 'ASIDE':
      return '\n\n' + kids(n, ctx) + '\n\n';
    default: return kids(n, ctx);
  }
}
const max = A.max_chars || 40000;
const fmt = A.format || 'markdown';
let content = fmt === 'html' ? root.outerHTML : fmt === 'text' ? root.innerText
  : md(root, { pre: false }).replace(/[ \t]+\n/g, '\n').replace(/\n[ \t]+(?![-\d>|])/g, '\n').replace(/\n{3,}/g, '\n\n').replace(/ {2,}/g, ' ').trim();
const out = { url: location.href, title: document.title, truncated: content.length > max, content: content.slice(0, max) };
const desc = document.querySelector('meta[name=description], meta[property="og:description"]');
if (desc && desc.content) out.description = desc.content.trim();
if (A.links) {
  const seen = new Set(); out.links = [];
  for (const a of document.querySelectorAll('a[href]')) {
    const u = abs(a.getAttribute('href')).split('#')[0];
    if (!/^https?:/.test(u) || seen.has(u)) continue;
    seen.add(u); out.links.push({ text: inline(a.innerText || a.getAttribute('aria-label') || '').slice(0, 120), url: u });
    if (out.links.length >= 1000) break;
  }
}
return out;
"#;

/// Results on a search engine's page: links with a heading (Google, Bing, DuckDuckGo, Brave, Kagi and most others).
const SEARCH_JS: &str = r#"
const base = location.hostname.split('.').slice(-2).join('.');
const unwrap = (h) => {
  try {
    const u = new URL(h, location.href);
    if (u.pathname === '/url' && /google\./.test(u.hostname)) return u.searchParams.get('q') || u.searchParams.get('url') || '';
    if (u.pathname === '/goto' && /google\./.test(u.hostname)) return u.href; // opaque: Valerie follows it afterwards
    if (u.pathname.startsWith('/l/') && /duckduckgo\.com$/.test(u.hostname)) return u.searchParams.get('uddg') || '';
    if (u.pathname.startsWith('/ck/') && /bing\.com$/.test(u.hostname)) {
      const x = u.searchParams.get('u') || '';
      return x.startsWith('a1') ? atob(x.slice(2).replace(/-/g, '+').replace(/_/g, '/')) : '';
    }
    return u.href;
  } catch (e) { return ''; }
};
const own = (h) => { try { const n = new URL(h).hostname; return n === base || n.endsWith('.' + base); } catch (e) { return true; } };
const anchors = [...document.querySelectorAll('a[href]')].filter((a) =>
  a.querySelector('h1, h2, h3') || a.closest('h2, h3') || a.matches('a[data-testid=result-title-a], a.result__a, .snippet > a, a.__sri_title_link'));
const seen = new Set(), results = [];
for (const a of anchors) {
  const url = unwrap(a.getAttribute('href'));
  if (!/^https?:/.test(url) || (own(url) && !url.includes('/goto?')) || seen.has(url.split('#')[0])) continue;
  const title = (a.querySelector('h1, h2, h3') || a).innerText.trim().replace(/\s+/g, ' ');
  if (!title) continue;
  seen.add(url.split('#')[0]);
  let box = a;
  while (box.parentElement && box.parentElement !== document.body && anchors.filter((x) => box.parentElement.contains(x)).length === 1) box = box.parentElement;
  const snippet = (box.innerText || '').split('\n').map((l) => l.trim())
    .filter((l) => l.length > 2 && l !== title && l !== 'Web results' && !l.includes(' › ') && !/^https?:\/\//.test(l) && !url.includes(l))
    .join(' ').replace(/\s+/g, ' ').replace(/\s*Read more\b.*$|\s*Missing: .*$/, '').slice(0, 400);
  results.push({ title, url, snippet });
  if (results.length >= (A.limit || 10)) break;
}
return { engine: location.hostname, results, page_text: results.length ? undefined : (document.body ? document.body.innerText.slice(0, 1500) : '') };
"#;

/// Google hides where its results go behind /goto links; ask where each one redirects (in parallel, no cookies sent).
fn resolve_redirects(results: &mut Value) {
    let Some(list) = results.as_array_mut() else { return };
    std::thread::scope(|s| {
        for r in list.iter_mut().filter(|r| r["url"].as_str().is_some_and(|u| u.contains(".google.") && u.contains("/goto?"))) {
            s.spawn(move || {
                let url = r["url"].as_str().unwrap_or("").to_string();
                let out = std::process::Command::new("curl")
                    .args(["-s", "-o", if cfg!(windows) { "NUL" } else { "/dev/null" }, "-w", "%{redirect_url}", "--max-time", "6", "--proto", "=https", &url])
                    .output();
                let to = out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
                if to.starts_with("http") && !to.contains(".google.") {
                    r["url"] = json!(to);
                }
            });
        }
    });
}

/// Open a hidden tab (not in the sidebar or history), run `f` on it, and close it whatever happens.
fn in_web_tab(proxy: &EventLoopProxy<UserEvent>, open: Value, f: impl FnOnce(Option<i64>) -> Result<Value, String>) -> Result<Value, String> {
    let tab = call(proxy, "web_open", open)?["tab"].as_i64();
    let out = f(tab);
    let _ = call(proxy, "web_close", json!({ "tab": tab }));
    out
}

/// Wait until `js` returns something `done` says is ready (or `secs` pass) and return its last answer.
fn poll(proxy: &EventLoopProxy<UserEvent>, tab: Option<i64>, js: &str, secs: u64, done: impl Fn(&Value) -> bool) -> Result<Value, String> {
    let start = Instant::now();
    loop {
        let v = eval(proxy, tab, js)?;
        if done(&v) || start.elapsed() > Duration::from_secs(secs) {
            return Ok(v);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// After the load event, give pages that draw themselves with JavaScript a moment: until their text stops growing.
fn settle(proxy: &EventLoopProxy<UserEvent>, tab: Option<i64>, wait_for: Option<&str>) -> Result<(), String> {
    if let Some(w) = wait_for.filter(|w| !w.trim().is_empty()) {
        let js = with_args(&json!({ "w": w }), "try { if (document.querySelector(A.w)) return true; } catch (e) {} \
            return !!(document.body && document.body.innerText.includes(A.w));");
        poll(proxy, tab, &js, 15, |v| v == &Value::Bool(true))?;
    }
    let last = std::cell::Cell::new(-1i64);
    let steady = std::cell::Cell::new(0);
    poll(proxy, tab, "return document.body ? document.body.innerText.length : 0", 6, |v| {
        let n = v.as_i64().unwrap_or(0);
        steady.set(if n > 0 && n == last.get() { steady.get() + 1 } else { 0 });
        last.set(n);
        steady.get() >= 2
    })?;
    Ok(())
}

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
            let here = a["this_computer"].as_bool().unwrap_or(false);
            crate::app::snapshot_config(&format!("set {key}"));
            let value = match &a["value"] {
                Value::Null => return Err("give a value".into()),
                // Written as in the YAML file ("32", "true", "[a, b]"); if that doesn't fit, it's taken as plain text.
                Value::String(s) => match crate::config::change_setting(&path, s, here) {
                    Ok(()) => s.clone(),
                    Err(_) => {
                        let quoted = json!(s).to_string();
                        crate::config::change_setting(&path, &quoted, here).map_err(|e| format!("{e:#}"))?;
                        quoted
                    }
                },
                v => {
                    crate::config::change_setting(&path, &v.to_string(), here).map_err(|e| format!("{e:#}"))?;
                    v.to_string()
                }
            };
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
        "web_search" => {
            let query = a["query"].as_str().unwrap_or("").trim();
            if query.is_empty() {
                return Err("give a query".into());
            }
            let limit = a["limit"].as_u64().unwrap_or(10).clamp(1, 30);
            in_web_tab(proxy, json!({ "search": query }), |tab| {
                let tab = wait_ready(proxy, tab)?;
                let mut r = poll(proxy, tab, &with_args(&json!({ "limit": limit }), SEARCH_JS), 8, |v| v["results"].as_array().is_some_and(|r| !r.is_empty()))?;
                resolve_redirects(&mut r["results"]);
                r["query"] = json!(query);
                if r["results"].as_array().is_some_and(Vec::is_empty) {
                    r["note"] = json!("No results could be read; page_text shows what the search page said (a consent or robot check?).");
                }
                Ok(r)
            })
        }
        "fetch_page" => {
            let url = a["url"].as_str().unwrap_or("").trim();
            if url.is_empty() {
                return Err("give a url".into());
            }
            in_web_tab(proxy, json!({ "url": url }), |tab| {
                let tab = wait_ready(proxy, tab)?;
                settle(proxy, tab, a["wait_for"].as_str())?;
                eval(proxy, tab, &with_args(&a, MARKDOWN_JS))
            })
        }
        "read_page" => {
            let tab = wait_ready(proxy, tab)?;
            let format = a["format"].as_str().unwrap_or(if a["html"].as_bool() == Some(true) { "html" } else { "text" });
            if format == "markdown" {
                let mut o = a.clone();
                o["max_chars"] = json!(a["max_chars"].as_u64().unwrap_or(20000));
                return eval(proxy, tab, &with_args(&o, MARKDOWN_JS));
            }
            let js = with_args(&json!({ "max": a["max_chars"].as_u64().unwrap_or(20000), "html": format == "html" }),
                "const body = A.html ? document.documentElement.outerHTML : (document.body ? document.body.innerText : ''); \
                return { url: location.href, title: document.title, truncated: body.length > A.max, content: body.slice(0, A.max) };");
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
            if a["full_page"].as_bool() == Some(true) {
                return full_page(proxy, tab).map(|png| json!({ "__image_png": base64(&png) }));
            }
            call(proxy, "screenshot", json!({ "tab": tab, "width": a.get("width") }))
        }
        "eval_js" => {
            let tab = wait_ready(proxy, tab)?;
            eval(proxy, tab, a["script"].as_str().unwrap_or(""))
        }
        _ => Err(format!("unknown tool {name}; the tools are: {}", tool_names().join(", "))),
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
