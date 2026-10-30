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

static ASK_USER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_ask_user(on: bool) {
    ASK_USER.store(on, std::sync::atomic::Ordering::Relaxed);
}

const WEB: &[&str] = &["web_search", "fetch_page", "check_page", "map_site"];

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
Reading the web (no rate limits; hidden tabs with the user's logins, so sign-in pages and JavaScript apps work; the \
user's tabs and history aren't touched): web_search finds pages (recent, news and sites filter; read: N also reads the \
top results). fetch_page reads pages and PDFs as Markdown; give focus (your question) to get just the sections about it, word \
for word, and urls to read several at once. Prefer these to your other web tools.\n\
Building a site or app: check_page loads it (localhost or a file) at phone and desktop sizes and returns screenshots, \
console errors, broken images and anything sticking out past the screen, in one call. read_network lists a page's \
requests (status, time; bodies: true for API responses).\n\
Working out of sight: open_tab with hidden: true gives you a tab only you see, with the user's logins; use the page tools \
on it by its tab id, close_tab when done. map_site lists a site's pages (sitemap and links; read: N reads them).\n\
Need a person? Don't stop and tell the user to do something in their own time: ask_user hands them a page with your \
note and waits for them, then you carry on. Use it for anything only a person can or should do: make an account, sign \
in, a CAPTCHA or two-factor code, verify an email, accept terms, pay or pick a plan, create an API key, grant access, \
or choose between options (choices gives them buttons). Pass url to open the page for them, or tab for one you have \
open. Never ask them to type a password or key into the note box: they put it into the page. fetch_page and \
web_search return blocked with a tab when a page wants a person.\n\
Working in the user's tabs: list_tabs gives tab ids; page tools default to the active tab. read_page reads a tab \
(format: markdown), snapshot lists clickable elements with refs, then click / type / press_key by ref (snapshot again \
after the page changes). wait_for waits for text or a selector, screenshot shows the page (save_to writes a file), \
eval_js runs JavaScript. Open tabs with activate: false unless the user wants to watch. run_command does anything a \
keyboard shortcut can. When something fails, read_logs says why.";

/// Who's calling: the access level of the chat whose agent this is ("read" | "ask" | "full") and its id.
struct Caller {
    access: String,
    chat: String,
}

/// Tools that only look; everything else changes something and needs the chat's permission.
pub const LOOKING: &[&str] = &[
    "list_spaces", "list_tabs", "read_page", "snapshot", "screenshot", "wait_for", "read_chat", "list_chats", "search_history",
    "list_archived", "list_closed_tabs", "list_downloads", "site_state", "screenshot_ui", "read_ui", "read_logs", "get_settings",
    "list_pages", "read_console", "read_terminal", "web_search", "fetch_page", "check_page", "map_site", "read_network", "ask_user",
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
                // Valerie's own chats are named in the note the user sees.
                "ask_user" => {
                    let mut a = args.clone();
                    a["chat"] = json!(who.chat);
                    call_tool(name, a, proxy)
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
                Ok(v) if v.get("__content").is_some() => json!({ "content": v["__content"], "isError": false }),
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
        tool("web_search", "Search the web and get the results: title, url and snippet each, from the user's search engine in \
             a hidden tab (their tabs and history aren't touched; no rate limits). Filter by time (recent), to news or to \
             sites. With read: N it also reads the top N results and returns the parts of each about your question (focus), \
             so one call answers most research questions.", json!({
            "query": { "type": "string", "description": "What to search for; operators like \"exact phrase\" and -word work" },
            "limit": { "type": "integer", "description": "Most results to return (default 10, up to 30)" },
            "recent": { "type": "string", "enum": ["day", "week", "month", "year"], "description": "Only pages from this long ago" },
            "news": { "type": "boolean", "description": "News articles only" },
            "sites": { "type": "array", "items": { "type": "string" }, "description": "Only these sites, e.g. [\"github.com\", \"docs.rs\"]" },
            "read": { "type": "integer", "description": "Also read the top this many results (up to 5)" },
            "focus": { "type": "string", "description": "With read: what to pull out of each page (default: the query)" },
            "max_chars": { "type": "integer", "description": "With read: most characters per page (default 6000)" } }), &["query"]),
        tool("fetch_page", "Read web pages and PDFs as clean Markdown, like a scraper but in a real browser with the user's \
             logins, so JavaScript apps, sign-in pages (x.com, dashboards, docs behind a login), PDFs (marked ## Page N) and \
             local files work, with no rate limits. \
             Give focus (a question or keywords; \"quoted phrases\" count most) to get only the sections about it, word for word \
             under their headings, plus the page's outline: far smaller than the whole page. urls reads up to 10 at once. Runs \
             in hidden tabs; the user's tabs and history aren't touched.", json!({
            "url": { "type": "string", "description": "An address, or a file path like /Users/me/report.html" },
            "urls": { "type": "array", "items": { "type": "string" }, "description": "Several pages at once (instead of url)" },
            "focus": { "type": "string", "description": "Only the parts about this, e.g. \"rate limits for order writes\"" },
            "format": { "type": "string", "enum": ["markdown", "text", "html"], "description": "Default markdown" },
            "main_only": { "type": "boolean", "description": "Only the main content, without menus, headers and footers (default true)" },
            "links": { "type": "boolean", "description": "Also list the page's links" },
            "wait_for": { "type": "string", "description": "A CSS selector or text to wait for before reading (for slow pages)" },
            "max_chars": { "type": "integer", "description": "Longest content to return (default 40000)" } }), &[]),
        tool("map_site", "List a site's pages: from its sitemap and its links, under the address's folder (docs.rs/tokio/latest/ \
             lists just that part; under: \"/\" for the whole site). search keeps the ones about it; read: N reads the top N \
             (up to 20) like fetch_page with focus, so one call can read a docs section. Hidden tabs with the user's logins.", json!({
            "url": { "type": "string", "description": "Where to start, e.g. https://docs.example.com/api/" },
            "under": { "type": "string", "description": "Only addresses whose path starts with this (default: the url's folder)" },
            "search": { "type": "string", "description": "Only pages whose address or link text mentions these words, best first" },
            "limit": { "type": "integer", "description": "Most addresses to return (default 100, up to 1000)" },
            "read": { "type": "integer", "description": "Also read the first this many (up to 20)" },
            "focus": { "type": "string", "description": "With read: what to pull out of each page (default: search)" },
            "max_chars": { "type": "integer", "description": "With read: most characters per page (default 6000)" } }), &["url"]),
        tool("check_page", "Check a page you're building (a dev server like http://localhost:3000, or a file) at phone, tablet and \
             desktop sizes in one call: a screenshot at each size, console errors and warnings, failed files, broken images, \
             elements sticking out past the screen's edge (sideways scrolling), load time. Runs in a hidden tab, so nothing \
             on the user's screen moves.", json!({
            "url": { "type": "string", "description": "e.g. http://localhost:5173/settings or /Users/me/site/index.html" },
            "sizes": { "type": "array", "items": { "type": "string" }, "description": "phone, tablet, laptop, desktop or WIDTHxHEIGHT (default [phone, desktop])" },
            "full_page": { "type": "boolean", "description": "Screenshot the whole page, not just the first screen" },
            "screenshots": { "type": "boolean", "description": "false: just the checks, no pictures" },
            "save_to": { "type": "string", "description": "Save the screenshots in this folder and return their paths instead of the images" },
            "name": { "type": "string", "description": "With save_to: the files' name (name-phone.png …)" },
            "wait_for": { "type": "string", "description": "A CSS selector or text to wait for first" } }), &["url"]),
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
            "activate": { "type": "boolean", "description": "Switch to the tab (default false: open in background)" },
            "hidden": { "type": "boolean", "description": "A tab only you see (not in the user's sidebar or history; their logins work). \
                Work in it with its tab id; close_tab when done (it closes itself when unused for a while); activate_tab gives it to the user" } }), &["url"]),
        tool("open_link", "Open a URL the way an external link would: Air Traffic Control rules, then a little window. valerie://little opens an empty little window that asks where to go.",
             json!({ "url": { "type": "string" } }), &["url"]),
        tool("navigate", "Load a URL in an existing tab.", json!({ "url": { "type": "string" }, "tab": tab_prop() }), &["url"]),
        tool("activate_tab", "Switch to a tab (also switches space).", json!({ "tab": tab_prop() }), &["tab"]),
        tool("close_tab", "Close a tab.", json!({ "tab": tab_prop() }), &["tab"]),
        tool("ask_user", "Hand the user a page for anything only a person can do, then carry on: make an account, sign in, a \
             CAPTCHA or two-factor code, verify an email, accept terms, pay or pick a plan, create an API key, grant access, \
             a decision, or a step you can't work out. The page comes to the front (url opens it; a hidden tab comes into \
             their sidebar) with your note, a box to answer and a Done button (or your choices); this waits until they press \
             one and returns what they wrote and chose. A hidden tab goes back to you afterwards, past whatever they did. \
             Several steps? Ask once per step.", json!({
            "tab": tab_prop(),
            "url": { "type": "string", "description": "A page to open for the user instead of a tab you have, e.g. a sign-up or API keys page" },
            "note": { "type": "string", "description": "What you need, in a sentence, e.g. \"Please make a Resend account and create an API key, then press Done\"" },
            "choices": { "type": "array", "items": { "type": "string" }, "description": "Up to 6 short buttons in place of Done, e.g. [\"Free plan\", \"Pro plan\"]; the one pressed comes back as choice" },
            "who": { "type": "string", "description": "Your name as the user knows you, e.g. Claude Code" },
            "wait_seconds": { "type": "integer", "description": "Longest to wait (default 300, up to 600). If your tool calls time out sooner \
                (Codex: 60), pass less; it returns still_with_user and you call again with the same tab to keep waiting" } }), &["note"]),
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
                     "full_page": { "type": "boolean", "description": "The whole page, not just what's on screen (it's scrolled through once first, so lazy images load)" },
                     "save_to": { "type": "string", "description": "Save the PNG to this file (or folder) and return its path instead of the image" } }), &[]),
        tool("read_network", "List the page's requests since it loaded, oldest first: API calls (fetch, XHR) with method and \
             status, and scripts, styles and images with timing. failed_only shows what broke (status 400+ or no answer). \
             Reload the tab (history reload) to see its loading from the start.", json!({
            "tab": tab_prop(),
            "filter": { "type": "string", "description": "Only addresses (or response bodies) containing this, e.g. /api/" },
            "failed_only": { "type": "boolean" },
            "kinds": { "type": "array", "items": { "type": "string" }, "description": "e.g. [\"fetch\", \"xhr\"]; others: script, css, img, link, other" },
            "bodies": { "type": "boolean", "description": "Record API response and request bodies (first 4000 characters) from now on" },
            "limit": { "type": "integer", "description": "Latest this many (default 100)" } }), &[]),
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
    let ask = ASK_USER.load(std::sync::atomic::Ordering::Relaxed);
    all.into_iter().filter(|t| (web || !WEB.contains(&t["name"].as_str().unwrap_or(""))) && (ask || t["name"] != "ask_user")).collect()
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
    eval_for(proxy, tab, script, 30)
}

/// Like `eval`, letting a promise take up to `secs` (reading a long PDF).
fn eval_for(proxy: &EventLoopProxy<UserEvent>, tab: Option<i64>, script: &str, secs: u64) -> Result<Value, String> {
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
            if start.elapsed() > Duration::from_secs(secs) {
                return Err(format!("promise did not settle within {secs}s"));
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
// A PDF: Valerie's viewer (pages.pdf_viewer) reads its text, page by page.
if (document.contentType === 'application/pdf') {
  return (async () => {
    for (let i = 0; i < 50 && !window.__valeriePdfText && !window.__valeriePdfFailed; i++) await new Promise((r) => setTimeout(r, 100));
    if (!window.__valeriePdfText) throw new Error(window.__valeriePdfFailed ? 'this PDF could not be read: ' + window.__valeriePdfFailed
      : "PDFs can only be read with Valerie's PDF viewer on (pages.pdf_viewer: true)");
    const max = A.max_chars || 40000, r = await window.__valeriePdfText(max + 1);
    return { url: location.href, title: document.title, pdf_pages: r.pages, truncated: r.text.length > max, content: r.text.slice(0, max) };
  })();
}
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
  : md(root, { pre: false }).replace(/[\u200b-\u200d\ufeff]/g, '').replace(/[ \t]+\n/g, '\n').replace(/\n[ \t]+(?![-\d>|])/g, '\n').replace(/\n{3,}/g, '\n\n').replace(/ {2,}/g, ' ').trim();
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
// News-style results: a link holding "source / headline / summary / age" as lines, with no heading element.
if (!results.length) {
  for (const a of document.querySelectorAll('#search a[href], #rso a[href], main a[href]')) {
    const url = unwrap(a.getAttribute('href'));
    const lines = (a.innerText || '').split('\n').map((l) => l.trim()).filter(Boolean);
    if (!/^https?:/.test(url) || (own(url) && !url.includes('/goto?')) || seen.has(url) || lines.length < 2) continue;
    const title = lines[0].length >= 25 ? lines[0] : lines[1];
    if (title.length < 15) continue;
    seen.add(url);
    results.push({ title, url, snippet: lines.filter((l) => l !== title).join(' · ').slice(0, 400) });
    if (results.length >= (A.limit || 10)) break;
  }
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
    let mut out = f(tab);
    // A page that wants a person stays open, for ask_user.
    if let Ok(v) = &mut out {
        if v["blocked"].is_string() && ASK_USER.load(std::sync::atomic::Ordering::Relaxed) && call(proxy, "web_keep", json!({ "tab": tab })).is_ok() {
            v["tab"] = json!(tab);
            v["next"] = json!(format!("This page wants a person ({}). Call ask_user with tab {} and a short note, e.g. \"Please sign in, \
                then press Done\"; then read it with read_page (format: markdown) on that tab, and close_tab when finished.",
                v["blocked"].as_str().unwrap_or(""), tab.unwrap_or(0)));
            return out;
        }
    }
    let _ = call(proxy, "web_close", json!({ "tab": tab }));
    out
}

/// Whether the page is a wall only a person gets past: "captcha" (robot checks), "sign_in", or "".
const BLOCKED_JS: &str = r#"
const body = document.body ? document.body.innerText : "", words = body.split(/\s+/).filter(Boolean).length;
const title = (document.title || "").toLowerCase(), text = (title + " " + body.slice(0, 3000)).toLowerCase();
if (document.querySelector('iframe[src*="recaptcha"], iframe[src*="hcaptcha"], iframe[src*="challenges.cloudflare.com"], #challenge-form, #challenge-running, .cf-turnstile, .g-recaptcha, .h-captcha, #px-captcha')
    || (words < 600 && /just a moment|verify (that )?you are (a )?human|are you a robot|unusual traffic|press (&|and) hold|security check|checking your browser|captcha/.test(text))) return "captcha";
const pw = [...document.querySelectorAll('input[type="password"]')].some((i) => i.offsetParent !== null);
if (words < 400 && (pw || /^(sign in|log in|login)\b|\bsign in to\b|\blog in to\b/.test(title))) return "sign_in";
return "";
"#;

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

/// A path or address the way a browser wants it: "/Users/me/r.html" and "~/r.html" become file:// addresses.
fn as_address(u: &str) -> String {
    let u = u.trim();
    let home = || dirs::home_dir().unwrap_or_default().to_string_lossy().into_owned();
    match u.strip_prefix("~/") {
        Some(rest) => format!("file://{}/{rest}", home()),
        None if u.starts_with('/') => format!("file://{u}"),
        None => u.to_string(),
    }
}

/// Write a screenshot's PNG to `path` (a folder gets a timestamped name) and return where it went.
fn save_png(shot: &Value, path: &str) -> Result<String, String> {
    let png = shot["__image_png"].as_str().and_then(unbase64).ok_or("no picture came back")?;
    let mut p = PathBuf::from(path.strip_prefix("~/").map(|r| dirs::home_dir().unwrap_or_default().join(r)).unwrap_or_else(|| PathBuf::from(path)));
    if p.is_dir() || path.ends_with('/') {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        p = p.join(format!("valerie-{stamp}.png"));
    }
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, png).map_err(|e| format!("couldn't save {}: {e}", p.display()))?;
    Ok(p.to_string_lossy().into_owned())
}

/// One page read in a hidden tab (fetch_page), narrowed to `focus` when it's given.
fn fetch_one(proxy: &EventLoopProxy<UserEvent>, url: &str, a: &Value) -> Result<Value, String> {
    if url.trim().is_empty() {
        return Err("give a url (or urls)".into());
    }
    let focus_on = a["focus"].as_str().unwrap_or("").trim().to_string();
    let max = a["max_chars"].as_u64().unwrap_or(40_000) as usize;
    let mut args = a.clone();
    if !focus_on.is_empty() {
        // Read the whole page, then keep what's about the focus.
        args["max_chars"] = json!(2_000_000);
        args["format"] = json!("markdown");
    }
    let mut page = in_web_tab(proxy, json!({ "url": as_address(url) }), |tab| {
        let tab = wait_ready(proxy, tab)?;
        if eval(proxy, tab, "return document.contentType")? == "application/pdf" {
            // Valerie's PDF viewer downloads it and loads it first.
            poll(proxy, tab, "return !!(window.__valeriePdfText || window.__valeriePdfFailed)", 60, |v| v == &Value::Bool(true))?;
            return eval_for(proxy, tab, &with_args(&args, MARKDOWN_JS), 120);
        }
        settle(proxy, tab, a["wait_for"].as_str())?;
        let mut page = eval_for(proxy, tab, &with_args(&args, MARKDOWN_JS), 120)?;
        if let Some(b) = eval(proxy, tab, BLOCKED_JS)?.as_str().filter(|b| !b.is_empty()) {
            page["blocked"] = json!(b);
        }
        Ok(page)
    })
    .map_err(|e| if e.contains("timed out") { format!("{url} didn't load (no such site, or it's down)") } else { e })?;
    if !focus_on.is_empty() {
        let f = focus(page["content"].as_str().unwrap_or(""), &focus_on, max.min(40_000));
        page["content"] = f["content"].clone();
        page["truncated"] = f["truncated"].clone();
        page["focus"] = json!({ "query": focus_on, "sections_matched": f["matched"], "outline": f["outline"] });
    }
    Ok(page)
}

const STOP: &[&str] = &[
    "the", "and", "for", "are", "but", "not", "you", "all", "any", "can", "had", "her", "was", "one", "our", "out", "has", "have", "how",
    "what", "when", "where", "which", "who", "why", "with", "this", "that", "from", "they", "will", "would", "there", "their", "about",
    "does", "into", "than", "then", "them", "these", "those", "your", "its", "also", "just", "quote", "exactly", "full", "section",
    "text", "page", "tell", "find", "give", "list", "show", "every", "each", "some", "more", "most", "exact", "verbatim",
];

/// The parts of a Markdown page about `query`, word for word: sections (under their headings) scored by the query's
/// words they hold, rarer words counting more and "quoted phrases" most, kept in page order up to `max` characters.
pub fn focus(md: &str, query: &str, max: usize) -> Value {
    let q = query.to_lowercase();
    let phrases: Vec<&str> = q.split('"').skip(1).step_by(2).map(str::trim).filter(|p| !p.is_empty()).collect();
    let mut words: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() > 2 && !STOP.contains(w))
        .map(|w| if w.len() > 4 && w.ends_with('s') && !w.ends_with("ss") { w[..w.len() - 1].to_string() } else { w.to_string() })
        .collect();
    words.sort();
    words.dedup();
    // Sections at headings, long ones cut at paragraphs into ~1500-character pieces that keep their heading path.
    let (mut chunks, mut outline, mut path): (Vec<(String, String)>, Vec<String>, Vec<(usize, String)>) = (vec![], vec![], vec![]);
    let mut body = String::new();
    let flush = |chunks: &mut Vec<(String, String)>, path: &[(usize, String)], body: &mut String| {
        if !body.trim().is_empty() {
            chunks.push((path.iter().map(|(_, h)| h.as_str()).collect::<Vec<_>>().join(" › "), std::mem::take(body).trim().to_string()));
        }
        body.clear();
    };
    let mut fence = false;
    for line in md.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
        }
        let level = line.chars().take_while(|c| *c == '#').count();
        if !fence && (1..=6).contains(&level) && line[level..].starts_with(' ') {
            flush(&mut chunks, &path, &mut body);
            let title = line[level..].trim().to_string();
            path.retain(|(l, _)| *l < level);
            path.push((level, title.clone()));
            if outline.len() < 120 {
                outline.push(format!("{}{title}", "  ".repeat(level.saturating_sub(1))));
            }
            continue;
        }
        if !fence && line.trim().is_empty() && body.len() > 1500 {
            flush(&mut chunks, &path, &mut body);
        }
        body.push_str(line);
        body.push('\n');
    }
    flush(&mut chunks, &path, &mut body);
    let lower: Vec<String> = chunks.iter().map(|(h, b)| format!("{h}\n{b}").to_lowercase()).collect();
    let n = chunks.len().max(1) as f64;
    let df = |w: &str| lower.iter().filter(|c| c.contains(w)).count() as f64;
    let idf: Vec<f64> = words.iter().map(|w| (1.0 + n / (1.0 + df(w))).ln()).collect();
    let mut scored: Vec<(usize, f64)> = lower
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let head = chunks[i].0.to_lowercase();
            let mut s: f64 = words.iter().zip(&idf).map(|(w, idf)| {
                let tf = c.matches(w.as_str()).count() as f64;
                if tf == 0.0 { 0.0 } else { (1.0 + tf.ln()) * idf * if head.contains(w.as_str()) { 2.0 } else { 1.0 } }
            }).sum();
            s += phrases.iter().filter(|p| c.contains(*p)).count() as f64 * 5.0;
            (i, s / (1.0 + (c.len() as f64 / 3000.0)))
        })
        .filter(|(_, s)| *s > 0.0)
        .collect();
    let matched = scored.len();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (mut keep, mut used) = (Vec::new(), 0);
    for (i, _) in scored {
        let len = chunks[i].0.len() + chunks[i].1.len() + 8;
        if used + len > max && !keep.is_empty() {
            continue;
        }
        used += len;
        keep.push(i);
    }
    keep.sort();
    let content = if keep.is_empty() {
        md.chars().take(max).collect::<String>()
    } else {
        keep.iter().map(|&i| if chunks[i].0.is_empty() { chunks[i].1.clone() } else { format!("## {}\n\n{}", chunks[i].0, chunks[i].1) }).collect::<Vec<_>>().join("\n\n…\n\n")
    };
    let truncated = content.chars().count() >= max || keep.len() < matched;
    json!({ "content": content.chars().take(max).collect::<String>(), "matched": matched, "truncated": truncated, "outline": outline })
}

/// Search-page address for `engine` (a search URL with %s): recency and news where the engine has them.
fn search_url(engine: &str, query: &str, recent: &str, news: bool) -> Option<String> {
    let q: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
    let mut u = engine.replace("%s", &q);
    let d = match recent { "day" | "d" | "24h" => "d", "week" | "w" => "w", "month" | "m" => "m", "year" | "y" => "y", _ => "" };
    if engine.contains("google.") {
        if !d.is_empty() {
            u.push_str(&format!("&tbs=qdr:{d}"));
        }
        if news {
            u.push_str("&tbm=nws");
        }
    } else if engine.contains("duckduckgo.") {
        if !d.is_empty() {
            u.push_str(&format!("&df={d}"));
        }
        if news {
            return None;
        }
    } else if !d.is_empty() || news {
        return None; // this engine's filters aren't known: Google does it
    }
    Some(u)
}

const GOOGLE: &str = "https://www.google.com/search?q=%s";
const DUCKDUCKGO: &str = "https://html.duckduckgo.com/html/?q=%s";

/// web_search: the user's engine (Google when it can't filter as asked), DuckDuckGo when that one blocks the search,
/// and with `read` the top results fetched too (narrowed to `focus`, or the query).
fn web_search(proxy: &EventLoopProxy<UserEvent>, a: &Value) -> Result<Value, String> {
    let query = a["query"].as_str().unwrap_or("").trim();
    if query.is_empty() {
        return Err("give a query".into());
    }
    let limit = a["limit"].as_u64().unwrap_or(10).clamp(1, 30);
    let sites: Vec<String> = match &a["sites"] {
        Value::Array(s) => s.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        Value::String(s) => s.split([',', ' ']).filter(|x| !x.is_empty()).map(str::to_string).collect(),
        _ => vec![],
    };
    let full = match sites.len() {
        0 => query.to_string(),
        1 => format!("{query} site:{}", sites[0]),
        _ => format!("{query} ({})", sites.iter().map(|s| format!("site:{s}")).collect::<Vec<_>>().join(" OR ")),
    };
    let (recent, news) = (a["recent"].as_str().unwrap_or(""), a["news"].as_bool() == Some(true));
    let mine = call(proxy, "search_engine", json!({}))?.as_str().unwrap_or(GOOGLE).to_string();
    let first = search_url(&mine, &full, recent, news).or_else(|| search_url(GOOGLE, &full, recent, news)).unwrap_or_default();
    let run = |url: &str| {
        in_web_tab(proxy, json!({ "url": url }), |tab| {
            let tab = wait_ready(proxy, tab)?;
            let mut r = poll(proxy, tab, &with_args(&json!({ "limit": limit }), SEARCH_JS), 8, |v| v["results"].as_array().is_some_and(|r| !r.is_empty()))?;
            if r["results"].as_array().is_none_or(Vec::is_empty) {
                if let Some(b) = eval(proxy, tab, BLOCKED_JS)?.as_str().filter(|b| !b.is_empty()) {
                    r["blocked"] = json!(b);
                }
            }
            Ok(r)
        })
    };
    let empty = |r: &Value| r["results"].as_array().is_none_or(Vec::is_empty);
    let mut r = run(&first)?;
    if empty(&r) && !news {
        if let Some(u) = search_url(DUCKDUCKGO, &full, recent, false) {
            let other = run(&u)?;
            if empty(&other) {
                let _ = call(proxy, "web_close", json!({ "tab": other["tab"] }));
            } else {
                // DuckDuckGo answered: the search page that wanted a person isn't needed.
                let _ = call(proxy, "web_close", json!({ "tab": r["tab"] }));
                r = other;
            }
        }
    }
    resolve_redirects(&mut r["results"]);
    r["query"] = json!(full);
    if empty(&r) {
        if r["blocked"].is_null() {
            r["note"] = json!("No results could be read; page_text shows what the search page said (a consent or robot check?).");
        }
        return Ok(r);
    }
    let read = a["read"].as_u64().unwrap_or(0).min(5) as usize;
    if read > 0 {
        let focus_on = a["focus"].as_str().filter(|f| !f.trim().is_empty()).unwrap_or(query).to_string();
        let opts = json!({ "focus": focus_on, "max_chars": a["max_chars"].as_u64().unwrap_or(6000) });
        let urls: Vec<String> = r["results"].as_array().into_iter().flatten().take(read).filter_map(|x| x["url"].as_str().map(str::to_string)).collect();
        let pages: Vec<Value> = std::thread::scope(|s| {
            let opts = &opts;
            let jobs: Vec<_> = urls.iter().map(|u| s.spawn(move || fetch_one(proxy, u, opts))).collect();
            jobs.into_iter().map(|j| j.join().unwrap_or(Err("failed".into()))).collect::<Vec<_>>()
        })
        .into_iter()
        .map(|p| p.unwrap_or_else(|e| json!({ "error": e })))
        .collect();
        if let Some(list) = r["results"].as_array_mut() {
            for (res, page) in list.iter_mut().zip(pages) {
                res["content"] = page.get("content").cloned().unwrap_or_else(|| page["error"].clone());
            }
        }
    }
    Ok(r)
}

/// What a developer checks on a page they're building, at each size: problems in its console, images that didn't load,
/// things sticking out past the screen's edge, and how long it took to load.
const CHECK_JS: &str = r#"
const W = document.documentElement.clientWidth;
const name = (e) => e.tagName.toLowerCase() + (e.id ? '#' + e.id : '') + [...e.classList].slice(0, 2).map((c) => '.' + c).join('');
const out = [];
if (document.documentElement.scrollWidth > W + 1 && document.body) {
  for (const e of document.body.querySelectorAll('*')) {
    const r = e.getBoundingClientRect();
    if (!r.width || r.right <= W + 1 || out.some((o) => o.el.contains(e))) continue;
    let p = e.parentElement, clipped = false;
    while (p && p !== document.body) { if (getComputedStyle(p).overflowX !== 'visible') { clipped = true; break; } p = p.parentElement; }
    if (!clipped) out.push({ el: e, text: name(e) + ' (' + Math.round(r.right - W) + 'px past the right edge)' });
    if (out.length >= 10) break;
  }
}
const lines = window.__valerie && window.__valerie.consoleLines ? window.__valerie.consoleLines() : [];
const nav = performance.getEntriesByType('navigation')[0];
return {
  viewport: innerWidth + 'x' + innerHeight,
  scrolls_sideways: document.documentElement.scrollWidth > W + 1,
  sticking_out: out.map((o) => o.text),
  console: lines.filter((l) => l.level === 'error' || l.level === 'warn').slice(-30).map((l) => l.level + ': ' + String(l.text).slice(0, 300)),
  broken_images: [...document.images].filter((i) => i.complete && i.naturalWidth === 0 && i.src).map((i) => i.src).slice(0, 10),
  load_ms: nav && nav.loadEventEnd ? Math.round(nav.loadEventEnd) : null,
  page_height: document.documentElement.scrollHeight,
};
"#;

fn size_of(s: &str) -> Option<(f64, f64)> {
    match s {
        "phone" | "mobile" => Some((390.0, 844.0)),
        "tablet" => Some((820.0, 1180.0)),
        "laptop" => Some((1280.0, 800.0)),
        "desktop" => Some((1440.0, 900.0)),
        _ => s.split_once(['x', 'X', '×']).and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?))),
    }
}

/// check_page: load a page (a dev server, a file) in a hidden tab and look at it at each size.
fn check_page(proxy: &EventLoopProxy<UserEvent>, a: &Value) -> Result<Value, String> {
    let url = as_address(a["url"].as_str().unwrap_or(""));
    if url.is_empty() {
        return Err("give a url (http://localhost:3000, a file path…)".into());
    }
    let names: Vec<String> = match &a["sizes"] {
        Value::Array(s) => s.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        Value::String(s) => s.split([',', ' ']).filter(|x| !x.is_empty()).map(str::to_string).collect(),
        _ => vec!["phone".into(), "desktop".into()],
    };
    let sizes: Vec<(String, (f64, f64))> = names.iter().take(6).map(|n| size_of(n).map(|s| (n.clone(), s)).ok_or(format!("{n:?} isn't a size: phone, tablet, laptop, desktop or WIDTHxHEIGHT"))).collect::<Result<_, _>>()?;
    let save = a["save_to"].as_str().filter(|p| !p.trim().is_empty());
    let pictures = a["screenshots"].as_bool() != Some(false);
    let (w0, h0) = sizes.first().map(|s| s.1).unwrap_or((1440.0, 900.0));
    in_web_tab(proxy, json!({ "url": url, "width": w0, "height": h0 }), |tab| {
        let tab = wait_ready(proxy, tab)?;
        settle(proxy, tab, a["wait_for"].as_str())?;
        let (mut report, mut images) = (Vec::new(), Vec::new());
        for (i, (name, (w, h))) in sizes.iter().enumerate() {
            if i > 0 {
                call(proxy, "web_size", json!({ "tab": tab, "width": w, "height": h }))?;
                std::thread::sleep(Duration::from_millis(500));
            }
            let mut r = eval(proxy, tab, CHECK_JS)?;
            r["size"] = json!(name);
            if pictures {
                let shot = if a["full_page"].as_bool() == Some(true) {
                    full_page(proxy, tab).map(|png| json!({ "__image_png": base64(&png) }))
                } else {
                    call(proxy, "screenshot", json!({ "tab": tab, "width": w.min(if save.is_some() { 1280.0 } else { 800.0 }) }))
                };
                match (shot, save) {
                    (Ok(s), Some(dir)) => r["screenshot"] = json!(save_png(&s, &format!("{}/{}-{name}.png", dir.trim_end_matches('/'), a["name"].as_str().unwrap_or("check")))?),
                    (Ok(s), None) => images.push(json!({ "type": "image", "data": s["__image_png"], "mimeType": "image/png" })),
                    (Err(e), _) => r["screenshot_error"] = json!(e),
                }
            }
            report.push(r);
        }
        let head = eval(proxy, tab, "return { url: location.href, title: document.title }")?;
        let text = json!({ "url": head["url"], "title": head["title"], "sizes": report });
        let mut content = vec![json!({ "type": "text", "text": serde_json::to_string_pretty(&text).unwrap_or_default() })];
        content.extend(images);
        Ok(json!({ "__content": content }))
    })
}

/// The site's own list of pages (robots.txt → sitemaps, gzipped or nested) and the page's links, same site only.
const MAP_JS: &str = r#"
return (async () => {
  const origin = location.origin, found = new Map();
  const add = (u, title) => {
    try {
      const x = new URL(String(u).trim(), location.href); x.hash = "";
      if (x.origin !== origin || !/^https?:$/.test(x.protocol)) return;
      const t = (title || "").replace(/\s+/g, " ").trim().slice(0, 120);
      if (!found.has(x.href) || (!found.get(x.href) && t)) found.set(x.href, t);
    } catch (_) {}
  };
  const get = async (u) => {
    try {
      const r = await fetch(u, { credentials: "include" });
      if (!r.ok) return "";
      if (/\.gz($|\?)/.test(u) && typeof DecompressionStream === "function") return await new Response(r.body.pipeThrough(new DecompressionStream("gzip"))).text();
      return await r.text();
    } catch (_) { return ""; }
  };
  const from = [], maps = [], seen = new Set();
  for (const m of (await get(origin + "/robots.txt")).matchAll(/^\s*sitemap:\s*(\S+)/gim)) maps.push(m[1]);
  // Docs sites often keep theirs in their own folder (/docs/sitemap.xml).
  const top = "/" + (location.pathname.split("/")[1] || "");
  maps.push(origin + A.under.replace(/\/?$/, "/") + "sitemap.xml", origin + top.replace(/\/?$/, "/") + "sitemap.xml", origin + "/sitemap.xml", origin + "/sitemap_index.xml");
  for (let n = 0; maps.length && n < 15 && found.size < 20000; ) {
    maps.sort((a, b) => b.includes(A.under) - a.includes(A.under));
    const u = maps.shift();
    if (seen.has(u)) continue;
    seen.add(u); n++;
    const xml = await get(u);
    if (!xml.includes("<loc")) continue;
    const doc = new DOMParser().parseFromString(xml, "application/xml"), index = doc.getElementsByTagName("sitemapindex").length > 0;
    for (const loc of doc.getElementsByTagName("loc")) index ? maps.push(loc.textContent.trim()) : add(loc.textContent, "");
    if (!index && !from.includes("sitemap")) from.push("sitemap");
  }
  for (const a of document.querySelectorAll("a[href]")) add(a.href, a.href.split("\x23")[0] === location.href.split("\x23")[0] ? "" : a.textContent);
  from.push("links");
  return { from, urls: [...found].map(([url, title]) => [url, title]) };
})();
"#;

/// map_site: a site's pages under a folder, optionally narrowed to `search` and read.
fn map_site(proxy: &EventLoopProxy<UserEvent>, a: &Value) -> Result<Value, String> {
    let start = url::Url::parse(a["url"].as_str().unwrap_or("").trim()).map_err(|_| "give a full address, like https://docs.example.com/guide/")?;
    if !matches!(start.scheme(), "http" | "https") {
        return Err("map_site works on http(s) sites".into());
    }
    let under = match a["under"].as_str().map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => if u.starts_with('/') { u.to_string() } else { format!("/{u}") },
        // "/docs/intro.html" → "/docs/"; "/docs/api" → "/docs/api".
        None => {
            let p = start.path();
            let last = p.rsplit('/').next().unwrap_or("");
            if last.contains('.') { p[..p.len() - last.len()].to_string() } else { p.to_string() }
        }
    };
    let inside = |u: &str| url::Url::parse(u).is_ok_and(|x| x.path().starts_with(&under) || format!("{}/", x.path()) == under);
    let links = |url: &str| in_web_tab(proxy, json!({ "url": url }), |tab| {
        let tab = wait_ready(proxy, tab)?;
        settle(proxy, tab, None)?;
        eval_for(proxy, tab, &with_args(&json!({ "under": under }), MAP_JS), 60)
    });
    let first = links(start.as_str())?;
    let mut from: Vec<Value> = first["from"].as_array().cloned().unwrap_or_default();
    let mut found: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut take = |list: &Value, found: &mut Vec<(String, String)>| {
        for p in list.as_array().into_iter().flatten() {
            let (u, t) = (p[0].as_str().unwrap_or(""), p[1].as_str().unwrap_or(""));
            if inside(u) && seen.insert(u.to_string()) {
                found.push((u.to_string(), t.to_string()));
            }
        }
    };
    take(&first["urls"], &mut found);
    // No sitemap for this part: follow the links one step further.
    if found.len() < 30 && !from.iter().any(|f| f == "sitemap") {
        let next: Vec<String> = found.iter().map(|(u, _)| u.clone()).filter(|u| u.trim_end_matches('/') != start.as_str().trim_end_matches('/')).take(8).collect();
        let more: Vec<Value> = std::thread::scope(|s| {
            let jobs: Vec<_> = next.iter().map(|u| s.spawn(|| links(u).unwrap_or(Value::Null))).collect();
            jobs.into_iter().map(|j| j.join().unwrap_or(Value::Null)).collect()
        });
        for m in &more {
            take(&m["urls"], &mut found);
        }
        if !more.is_empty() {
            from.push(json!("links two steps in"));
        }
    }
    let total = found.len();
    let words: Vec<String> = a["search"].as_str().unwrap_or("").to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 1).map(str::to_string).collect();
    if words.is_empty() {
        found.sort();
    } else {
        let score = |(u, t): &(String, String)| { let h = format!("{} {}", u.to_lowercase(), t.to_lowercase()); words.iter().filter(|w| h.contains(w.as_str())).count() };
        found.retain(|p| score(p) > 0);
        found.sort_by_key(|p| std::cmp::Reverse(score(p)));
    }
    let matched = found.len();
    found.truncate(a["limit"].as_u64().unwrap_or(100).clamp(1, 1000) as usize);
    let mut out = json!({ "site": start.origin().ascii_serialization(), "under": under, "found": total, "from": from,
        "urls": found.iter().map(|(u, t)| if t.is_empty() { json!({ "url": u }) } else { json!({ "url": u, "title": t }) }).collect::<Vec<_>>() });
    if !words.is_empty() {
        out["matching_search"] = json!(matched);
    }
    let read = a["read"].as_u64().unwrap_or(0).min(20) as usize;
    if read > 0 {
        let focus_on = a["focus"].as_str().or(a["search"].as_str()).unwrap_or("").to_string();
        let opts = json!({ "focus": focus_on, "max_chars": a["max_chars"].as_u64().unwrap_or(6000) });
        let urls: Vec<String> = found.iter().take(read).map(|(u, _)| u.clone()).collect();
        let mut pages = Vec::new();
        for chunk in urls.chunks(5) {
            pages.extend(std::thread::scope(|s| {
                let opts = &opts;
                let jobs: Vec<_> = chunk.iter().map(|u| s.spawn(move || fetch_one(proxy, u, opts).unwrap_or_else(|e| json!({ "url": u, "error": e })))).collect();
                jobs.into_iter().map(|j| j.join().unwrap_or(Value::Null)).collect::<Vec<_>>()
            }));
        }
        out["pages"] = json!(pages);
    }
    Ok(out)
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
        "web_search" => web_search(proxy, &a),
        "fetch_page" => match a["urls"].as_array().filter(|u| !u.is_empty()) {
            // Several pages at once, side by side in their own hidden tabs.
            Some(urls) => {
                let urls: Vec<String> = urls.iter().filter_map(|u| u.as_str().map(str::to_string)).take(10).collect();
                let out: Vec<Value> = std::thread::scope(|s| {
                    let a = &a;
                    let jobs: Vec<_> = urls.iter().map(|u| s.spawn(move || fetch_one(proxy, u, a).unwrap_or_else(|e| json!({ "url": u, "error": e })))).collect();
                    jobs.into_iter().map(|j| j.join().unwrap_or(Value::Null)).collect()
                });
                Ok(json!(out))
            }
            None => fetch_one(proxy, a["url"].as_str().unwrap_or(""), &a),
        },
        "check_page" => check_page(proxy, &a),
        "map_site" => map_site(proxy, &a),
        "ask_user" => {
            let mut a = a;
            // A page that isn't open yet (a sign-up page, a billing page) opens as the user's own tab.
            if let Some(url) = a["url"].as_str().filter(|u| !u.trim().is_empty() && a["tab"].is_null()) {
                if !ASK_USER.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err("asking the user is off (the mcp.ask_user setting)".into());
                }
                if a["note"].as_str().is_none_or(|n| n.trim().is_empty()) {
                    return Err("say what you need the user to do (note), e.g. \"Make an account, then press Done\"".into());
                }
                a["tab"] = call(proxy, "open_tab", json!({ "url": url }))?["tab"].clone();
            }
            let tab = a["tab"].clone();
            let wait = a["wait_seconds"].as_u64().unwrap_or(300).clamp(5, 600);
            match call_wait(proxy, "ask_user", a, wait) {
                Err(e) if e.starts_with("timed out") => Ok(json!({ "done": false, "still_with_user": true, "tab": tab,
                    "next": "The user hasn't pressed Done yet. Call ask_user again with this tab (no note or url) to keep waiting." })),
                r => r,
            }
        }
        "read_network" => {
            let mut o = a.clone();
            o["failed"] = a["failed_only"].clone();
            let v = eval(proxy, tab, &with_args(&o, "return window.__valerie && __valerie.network ? __valerie.network(A) : null;"))?;
            if v.is_null() {
                return Err("this page keeps no network log (one of Valerie's own pages, or still loading)".into());
            }
            if v["on"] != Value::Bool(true) {
                return Err("the network log is off (the mcp.network_log setting); turn it on and reload the tab".into());
            }
            let rows = v["requests"].as_array().cloned().unwrap_or_default();
            let failed = rows.iter().filter(|r| r["error"].is_string() || r["status"].as_u64().is_some_and(|s| s >= 400)).count();
            let mut out = json!({ "requests": rows, "shown": rows.len(), "failed": failed });
            if a["bodies"].as_bool() == Some(true) && v["had_bodies"] != Value::Bool(true) {
                out["note"] = json!("Bodies are recorded from now on: repeat the action (or reload the tab) and read again.");
            }
            Ok(out)
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
            let shot = if a["full_page"].as_bool() == Some(true) {
                full_page(proxy, tab).map(|png| json!({ "__image_png": base64(&png) }))?
            } else {
                call(proxy, "screenshot", json!({ "tab": tab, "width": a.get("width") }))?
            };
            match a["save_to"].as_str().filter(|p| !p.trim().is_empty()) {
                Some(path) => save_png(&shot, path).map(|p| json!({ "saved": p })),
                None => Ok(shot),
            }
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
    fn focus_keeps_the_sections_about_the_question_in_page_order() {
        let md = "# API\n\nIntro text.\n\n## Rate limits\n\nWrite budget: 10 orders per second per shard.\n\n## Fees\n\nMakers pay nothing.\n\n## Errors\n\n429 means the rate limit was hit.";
        let f = super::focus(md, "What are the rate limits for order writes?", 10_000);
        let c = f["content"].as_str().unwrap();
        assert!(c.contains("10 orders per second") && c.contains("429"), "{c}");
        assert!(!c.contains("Makers pay nothing"), "{c}");
        assert!(c.find("Write budget").unwrap() < c.find("429").unwrap());
        assert!(c.contains("## API › Rate limits"));
        assert_eq!(f["outline"][1], "  Rate limits");
        // Nothing matches: the start of the page instead.
        assert!(super::focus(md, "zebra", 20)["content"].as_str().unwrap().starts_with("# API"));
    }

    #[test]
    fn search_urls_filter_where_the_engine_can() {
        let g = super::search_url(super::GOOGLE, "nfl injuries", "week", true).unwrap();
        assert!(g.ends_with("q=nfl+injuries&tbs=qdr:w&tbm=nws"), "{g}");
        assert!(super::search_url("https://kagi.com/search?q=%s", "x", "day", false).is_none());
        assert!(super::search_url("https://kagi.com/search?q=%s", "x", "", false).is_some());
        assert_eq!(super::size_of("phone"), Some((390.0, 844.0)));
        assert_eq!(super::size_of("1024x768"), Some((1024.0, 768.0)));
        assert!(super::as_address("/tmp/a.html").starts_with("file:///tmp"));
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
