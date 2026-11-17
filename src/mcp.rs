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

/// macOS's App Nap drops a background app to the lowest priority, where a busy computer can leave it unanswered for
/// minutes. Holding this activity for as long as Valerie runs keeps agents answered (mcp.answer_in_background).
#[cfg(target_os = "macos")]
fn stay_out_of_app_nap() {
    use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
    const USER_INITIATED_ALLOWING_IDLE_SYSTEM_SLEEP: u64 = 0x00FF_FFFF & !(1 << 20);
    let reason = objc2_foundation::NSString::from_str("Answering AI agents");
    unsafe {
        let info: Retained<AnyObject> = msg_send![class!(NSProcessInfo), processInfo];
        let activity: Retained<AnyObject> = msg_send![&*info, beginActivityWithOptions: USER_INITIATED_ALLOWING_IDLE_SYSTEM_SLEEP, reason: &*reason];
        std::mem::forget(activity);
    }
}

/// Set on a browser started by an agent's tool call: it opens with no window and no pages.
pub const STARTED_FOR_AGENT: &str = "VALERIE_STARTED_FOR_AGENT";
/// Set by Restart on the copy that replaces this one.
pub const RESTARTING: &str = "VALERIE_RESTARTING";

static CALLS_RUNNING: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LAST_CALL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A tool call running now or just finished, for telling when agents have stopped needing Valerie.
struct CallRunning;

impl CallRunning {
    fn begin() -> Self {
        CALLS_RUNNING.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self
    }
}

impl Drop for CallRunning {
    fn drop(&mut self) {
        LAST_CALL.store(crate::state::now(), std::sync::atomic::Ordering::Relaxed);
        CALLS_RUNNING.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Seconds since agents last used Valerie (0 while a call is running). Starting counts as use, so the call that
/// started Valerie gets to arrive.
pub fn idle_for(now: u64) -> u64 {
    if CALLS_RUNNING.load(std::sync::atomic::Ordering::Relaxed) > 0 {
        return 0;
    }
    let last = LAST_CALL.load(std::sync::atomic::Ordering::Relaxed);
    if last == 0 {
        LAST_CALL.store(now, std::sync::atomic::Ordering::Relaxed);
        return 0;
    }
    now.saturating_sub(last)
}

pub fn spawn(port: u16, token: String, proxy: EventLoopProxy<UserEvent>) {
    let for_agent = std::env::var_os(STARTED_FOR_AGENT).is_some();
    #[cfg(target_os = "macos")]
    if crate::config::Config::load().map_or(true, |c| c.mcp.answer_in_background) {
        stay_out_of_app_nap();
    }
    std::thread::spawn(move || {
        let server = match tiny_http::Server::http(("127.0.0.1", port)) {
            Ok(s) => s,
            // Two agents' calls started Valerie at once: the other one is answering, so this one isn't needed.
            Err(_) if for_agent => std::process::exit(0),
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
Use web_search and fetch_page alongside any other web tools you have. Reach for them for pages that need a login or JavaScript, \
PDFs, research across several pages, and anything another tool returned blocked, empty or thin: they have no quota and see what \
the user sees. Keep curl for APIs and raw files.\n\
Reading the web (no rate limits; hidden tabs with the user's logins, so sign-in pages and JavaScript apps work; the \
user's tabs and history aren't touched): web_search finds pages (recent, news and sites filter; read: N also reads the \
top results). fetch_page reads pages and PDFs as Markdown; give focus (your question) to get just the sections about it, word \
for word, and urls to read several at once.\n\
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
after the page changes); drag moves one onto another. wait_for waits for text or a selector, screenshot shows the page (save_to writes a file), \
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
    "list_pages", "list_layouts", "check_panel", "read_console", "read_terminal", "web_search", "fetch_page", "check_page", "map_site", "read_network", "ask_user",
];

/// The confirm tool that always reaches a person: "Always Allow" can't cover it.
pub const ASK_EACH_TIME: &str = "ask_each_time";

/// What the agent is about to do, if it could spend money (or turn this check off). mcp.ask_before_paying.
fn risky_action(name: &str, args: &Value, proxy: &EventLoopProxy<UserEvent>) -> Result<Option<String>, String> {
    let key = args["key"].as_str().map(|k| k.rsplit('+').next().unwrap_or(k).trim());
    let key_press = name == "press_key" && matches!(key, Some("Enter" | "Return" | "" | "Space" | "Spacebar"));
    let submit = name == "type" && args["submit"].as_bool() == Some(true);
    if name != "click" && name != "set_setting" && name != "type" && !key_press {
        return Ok(None);
    }
    // A config that doesn't load counts as the defaults, so the check stays on.
    let config = crate::config::Config::load().unwrap_or_else(|_| crate::config::Config::defaults());
    if !config.mcp.ask_before_paying {
        return Ok(None);
    }
    if name == "set_setting" {
        let key = args["key"].as_str().unwrap_or("").split('.').map(str::trim).filter(|k| !k.is_empty()).collect::<Vec<_>>().join(".");
        let guarded = key == "mcp" || key == "mcp.ask_before_paying" || key == "mcp.paying_words";
        return Ok(guarded.then(|| format!("change the {key} setting to {}", args["value"])));
    }
    let target = match name {
        "click" => json!({ "ref": args["ref"], "selector": args["selector"] }),
        "type" => json!({ "ref": args["ref"], "selector": args["selector"], "submits": submit }),
        _ => json!({ "submits": true }),
    };
    let found = eval(proxy, args.get("tab").and_then(Value::as_i64), &with_args(&target, &format!("{FIND_JS}\n{PAYING_JS}\nreturn pressTarget(find(), A.submits);")))?;
    let site = found["site"].as_str().unwrap_or("this page");
    if name == "type" && found["card_field"].as_bool() == Some(true) {
        return Ok(Some(format!("type into a card field on {site}")));
    }
    if name == "type" && !submit {
        return Ok(None);
    }
    let label = found["label"].as_str().unwrap_or("").trim();
    let action = if label.is_empty() { "click a button".to_string() } else { format!("click “{label}”") };
    if let Some(sign) = found["sign"].as_str().filter(|s| !s.is_empty()) {
        return Ok(Some(format!("{action} on {site}, a page that takes payment ({sign})")));
    }
    Ok(paying_word(found["text"].as_str().unwrap_or(""), &config.mcp.paying_words).map(|word| {
        if label.is_empty() { format!("{action} on {site} (it mentions “{word}”)") } else { format!("{action} on {site}") }
    }))
}

/// An agent's script, watched while it runs: anything it presses or sends is checked when it happens.
fn guarded_script(script: &str, words: &[String]) -> String {
    format!("const __vg = (() => {{ const WORDS = {};\n{PAYING_JS}\n{SCRIPT_GUARD_JS}\n}})();\nreturn __vg.run(function () {{\n{script}\n}});", json!(words))
}

fn run_script(script: &str, tab: Option<i64>, proxy: &EventLoopProxy<UserEvent>) -> Result<Value, String> {
    let config = crate::config::Config::load().unwrap_or_else(|_| crate::config::Config::defaults());
    if !config.mcp.ask_before_paying {
        return eval(proxy, tab, script);
    }
    let out = eval(proxy, tab, &guarded_script(script, &config.mcp.paying_words))?;
    match out.get("__blocked").and_then(|b| b.get(0)) {
        Some(first) => Err(format!(
            "Valerie stopped this script: it tried to {}. That needs the user's yes, and anything before it in the script already ran. {}",
            first["what"].as_str().unwrap_or("do something that looks like paying"),
            if first["kind"] == "request" {
                "Press the page's own button with the click tool so they can approve it, or hand it over with ask_user."
            } else {
                "Use the click tool on that button so they can approve it."
            }
        )),
        None => Ok(out.get("__value").cloned().unwrap_or(Value::Null)),
    }
}

/// The first of `words` in `text`: whole words, so "pay" doesn't match "display". Words with symbols or non-English
/// letters ("$", "購入") match anywhere, since those languages don't put spaces between words.
fn paying_word<'a>(text: &str, words: &'a [String]) -> Option<&'a str> {
    let spaced = |t: &str| {
        let mut out = String::from(" ");
        let mut prev_lower = false;
        for c in t.chars() {
            if prev_lower && c.is_uppercase() {
                out.push(' '); // payNow, checkoutBtn
            }
            prev_lower = c.is_lowercase();
            if c.is_alphanumeric() { out.extend(c.to_lowercase()) } else if !out.ends_with(' ') { out.push(' ') }
        }
        if !out.ends_with(' ') {
            out.push(' ');
        }
        out
    };
    let (normal, lower) = (spaced(text), text.to_lowercase());
    words.iter().map(String::as_str).find(|w| {
        let word = spaced(w);
        if word.trim().is_empty() || !w.is_ascii() { !w.trim().is_empty() && lower.contains(&w.trim().to_lowercase()) } else { normal.contains(&word) }
    })
}

fn ask_each_time(what: &str, name: &str, args: &Value, chat: &str, proxy: &EventLoopProxy<UserEvent>) -> Result<(), String> {
    let answer = call_wait(proxy, "confirm", json!({ "chat": chat, "tool": ASK_EACH_TIME, "args": { "what": what } }), 600)?;
    if answer["allowed"].as_bool() != Some(true) {
        return Err(format!("The user said no to this: {what}. Don't try it another way; ask them what they'd like instead."));
    }
    // They may have taken minutes to answer: the yes only covers what they were shown.
    match risky_action(name, args, proxy)? {
        Some(now) if now != what => Err(format!("The page changed while the user was deciding: they approved “{what}”, not “{now}”. Take a new snapshot.")),
        _ => Ok(()),
    }
}

/// A bug report goes out publicly under the user's name: they always see the exact text and say yes first.
fn confirm_report(args: &Value, chat: &str, proxy: &EventLoopProxy<UserEvent>) -> Result<(), String> {
    let title = args["title"].as_str().unwrap_or("").trim();
    let what = match args["issue"].as_u64() {
        Some(n) => format!("add this to issue #{n} on GitHub, from your account"),
        None => format!("post this bug report on GitHub, from your account: “{}”", title.chars().take(120).collect::<String>()),
    };
    let content = format!("{title}\n\n{}", args["body"].as_str().unwrap_or("").trim());
    let answer = call_wait(proxy, "confirm", json!({ "chat": chat, "tool": ASK_EACH_TIME, "args": { "what": what, "content": content } }), 600)?;
    match answer["allowed"].as_bool() {
        Some(true) => Ok(()),
        _ => Err("The user said no to sending the report. Ask what they'd like changed, or show them the text to post themselves.".into()),
    }
}

/// Check a tool call against the chat's access level, asking the user for "ask" chats.
fn allowed(name: &str, args: &Value, who: &Caller, proxy: &EventLoopProxy<UserEvent>) -> Result<(), String> {
    if LOOKING.contains(&name) || name == "approve" {
        return Ok(());
    }
    if who.access == "read" {
        return Err(format!("This chat is read-only, so `{name}` isn't allowed. Tell the user what you'd do instead; they can switch the chat to Ask first or Full access."));
    }
    if let Some(what) = risky_action(name, args, proxy)? {
        return ask_each_time(&what, name, args, &who.chat, proxy);
    }
    if name == "report_issue" {
        if who.chat.is_empty() {
            return Err("report_issue only works in a Valerie chat, where the user sees the report first. Show them the text instead.".into());
        }
        return confirm_report(args, &who.chat, proxy);
    }
    if who.access == "full" {
        return Ok(());
    }
    match call_wait(proxy, "confirm", json!({ "chat": who.chat, "tool": name, "args": args }), 600)?["allowed"].as_bool() {
        Some(true) => Ok(()),
        _ => Err(format!("The user said no to `{name}`. Don't try it again; ask them what they'd like instead.")),
    }
}

/// What every MCP app asks before using a tool. None of it needs the browser, so the stdio bridge answers it too: an agent
/// that starts while Valerie is closed or busy still gets its tools.
fn catalog_answer(method: &str, params: &Value, who: &Caller) -> Option<Value> {
    Some(match method {
        "initialize" => json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "valerie", "version": env!("CARGO_PKG_VERSION") },
            "instructions": INSTRUCTIONS
        }),
        "ping" => json!({}),
        // Some apps ask for these whatever the server offers; an empty list beats an error.
        "resources/list" => json!({ "resources": [] }),
        "resources/templates/list" => json!({ "resourceTemplates": [] }),
        "prompts/list" => json!({ "prompts": [] }),
        // `approve` is only for Valerie's own chats (Claude Code's permission prompts); it'd only confuse other agents.
        "tools/list" => json!({ "tools": tools().into_iter().filter(|t| !who.chat.is_empty() || t["name"] != "approve").collect::<Vec<_>>() }),
        _ => return None,
    })
}

fn rpc(msg: Value, proxy: &EventLoopProxy<UserEvent>, who: &Caller) -> Option<Value> {
    let id = msg.get("id").cloned()?; // notifications get no response
    let method = msg["method"].as_str().unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    if let Some(result) = catalog_answer(method, &params, who) {
        return Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }
    let result = match method {
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let args = loosen(name, params.get("arguments").cloned().filter(|a| !a.is_null()).unwrap_or(json!({})));
            let started = Instant::now();
            let _running = CallRunning::begin();
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

/// How an action answers the page's alert, confirm and prompt.
fn with_dialog(mut props: Value) -> Value {
    props["dialog"] = json!({ "type": "string", "enum": ["accept", "dismiss"],
        "description": "How to answer a confirm or prompt the page shows meanwhile (default dismiss: Cancel)" });
    props["dialog_text"] = json!({ "type": "string", "description": "With dialog: accept, the answer to a prompt (default: the one it suggests)" });
    props
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
    let mut tool = json!({ "name": name, "description": description, "annotations": hints,
            "inputSchema": { "type": "object", "properties": props, "required": required } });
    // Claude Code hides most MCP tools behind a search; agents only reach for the web tools they can see.
    if ["web_search", "fetch_page"].contains(&name) {
        tool["_meta"] = json!({ "anthropic/alwaysLoad": true });
    }
    tool
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
        tool("web_search", "Search the web, with no quota. Get the results: title, url and snippet each, from the user's search engine in \
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
        tool("fetch_page", "Use this for a page that needs a login or JavaScript, or that another tool couldn't read. Read web pages and PDFs as clean Markdown, like a scraper but in a real browser with the user's \
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
        tool("snapshot", "List the page's interactive elements with refs for click/type: buttons, links, fields, editors and anything clickable, inside shadow DOM and same-origin frames too. Only what's shown on the page is listed (not what sits in a closed menu or section). Each has its label and, where it applies, href (same-site links as a path), value (a list for a multiple-choice <select>), checked, expanded, options (dropdowns), editor (a rich text or code editor), disabled, readonly, offscreen, and covered (what sits on top of it, e.g. an open dialog). other_site_frames lists embedded frames from another site, which these tools can't reach inside: open that address in its own tab to work in it. truncated: true means there were more than 500. Refs reset on every snapshot.",
             json!({ "tab": tab_prop(), "only_in_view": { "type": "boolean" } }), &[]),
        tool("click", "Click an element by snapshot ref or CSS selector (shadow DOM and same-origin frames included), the way a person would: on whatever is on top at its middle, then waits for the page to react and says if it went somewhere new, opened a new tab, or showed an alert, confirm or prompt (answered by dialog). Refuses what a person couldn't click: something covered (a dialog or banner), hidden (in a closed menu or section) or disabled; force: true clicks covered, hidden and aria-disabled ones anyway. double: true double-clicks; button: right right-clicks (opens the page's own context menu).", with_dialog(json!({
            "tab": tab_prop(), "ref": { "type": "string" }, "selector": { "type": "string" },
            "force": { "type": "boolean", "description": "Click even if something covers it or it isn't shown" },
            "double": { "type": "boolean", "description": "Double-click" },
            "button": { "type": "string", "enum": ["left", "right"], "description": "Default left" } })), &[]),
        tool("type", "Type text into an input, textarea, rich text box or code editor (CodeMirror, Monaco, online document editors) by ref or selector, or into whatever has the cursor. A ref on an editor's frame finds the editor inside it. Editors get the text pasted in as one edit, so whole documents with newlines work; clear: true replaces everything in the editor. Date, time, color, number and slider fields always get the whole value replaced (dates as 2026-11-15, times as 14:30). Says what the field holds afterwards if that isn't what was typed (a length limit or a format the page applies). keys: true sends the text key by key, for fields that only react to key presses (formatted phone or card fields). On a dropdown (<select>), text picks the option with that label or value; on a multiple-choice one it adds that option to the selection.", with_dialog(json!({
            "tab": tab_prop(), "ref": { "type": "string" }, "selector": { "type": "string" }, "text": { "type": "string" },
            "clear": { "type": "boolean", "description": "Replace existing value" }, "submit": { "type": "boolean", "description": "Submit the form / press Enter afterwards" },
            "keys": { "type": "boolean", "description": "Press each character as a key (keydown, keypress, input, keyup) instead of setting the text" } })), &["text"]),
        tool("hover", "Move the mouse onto an element by ref or selector, for menus and tooltips that open from scripts. A page's CSS :hover styles don't change (use check_page or eval_js to look at those), so a menu that only opens on :hover stays closed: click its items with force: true.", json!({
            "tab": tab_prop(), "ref": { "type": "string" }, "selector": { "type": "string" },
            "force": { "type": "boolean", "description": "Hover even if something covers it or it isn't shown" } }), &[]),
        tool("drag", "Drag an element (ref or selector) onto another (to_ref or to_selector) the way a person would with the mouse: press on the first, move across, let go on the second. Works for drag-and-drop lists, boards and drop zones, both kinds pages use (draggable elements and pointer-driven ones). Refuses when either is covered or hidden, like click; force: true drags anyway.", json!({
            "tab": tab_prop(), "ref": { "type": "string" }, "selector": { "type": "string" },
            "to_ref": { "type": "string" }, "to_selector": { "type": "string" },
            "force": { "type": "boolean", "description": "Drag even if something covers them or they aren't shown" } }), &[]),
        tool("upload_file", "Choose files for a file input (<input type=file>) by ref or selector, without the file picker. Files stay on this computer until the page sends them. Up to 20 MB in all.", json!({
            "tab": tab_prop(), "ref": { "type": "string" }, "selector": { "type": "string" },
            "paths": { "type": "array", "items": { "type": "string" }, "description": "Files on this computer" } }), &["paths"]),
        tool("press_key", "Press a key or shortcut on the focused element: Enter, Escape, ArrowDown, or with modifiers like cmd+a, ctrl+shift+z, mod+z (⌘ on Mac, Ctrl elsewhere). Select all, undo and redo work in editors that don't handle them. To put text in, use type, not paste.",
             with_dialog(json!({ "tab": tab_prop(), "key": { "type": "string" } })), &["key"]),
        tool("scroll", "Scroll the page, or with ref or selector and a direction the panel that element is in (a message list, a sidebar). \
             Without ref, a page that doesn't scroll itself (mail and chat apps) has its biggest scrolling panel scrolled instead. \
             ref or selector alone scrolls that element into view. Says where it got to (at_top, at_bottom) and if more loaded.", json!({
            "tab": tab_prop(), "direction": { "type": "string", "enum": ["up", "down", "top", "bottom"] }, "ref": { "type": "string" }, "selector": { "type": "string" } }), &[]),
        tool("wait_for", "Wait until a selector or text appears (and the page has loaded), inside shadow DOM and same-origin frames too.", json!({
            "tab": tab_prop(), "selector": { "type": "string" }, "text": { "type": "string" }, "timeout_ms": { "type": "integer", "default": 10000 } }), &[]),
        tool("screenshot", "Capture what the tab looks like as a PNG image: the visible viewport, or with full_page the whole page top to bottom.",
             json!({ "tab": tab_prop(), "width": { "type": "integer", "description": "Image width in points (default 1024; not used with full_page)" },
                     "full_page": { "type": "boolean", "description": "The whole page, not just what's on screen (it's scrolled through once first, so lazy images load)" },
                     "save_to": { "type": "string", "description": "Save the PNG to this file (or folder) and return its path instead of the image" } }), &[]),
        tool("read_network", "List the page's requests since it loaded, oldest first: API calls (fetch, XHR) with method and \
             status, and scripts, styles and images with timing. failed_only shows what broke (status 400+ or no answer). \
             The first read of a tab starts its recording (earlier requests come without status); reload the tab (history \
             reload) to see its loading from the start.", json!({
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
        tool("eval_js", "Run JavaScript in the page as a function body; `return` a JSON-serialisable value. Promises are awaited. \
             If it clicks, submits or sends something that looks like paying, Valerie stops it and says what to do instead. If the \
             page shows an alert, confirm or prompt meanwhile, it's answered by dialog and the result comes back as { result, dialogs }.",
             with_dialog(json!({ "tab": tab_prop(), "script": { "type": "string" } })), &["script"]),
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
        tool("list_layouts", "The layout on screen (layouts/<name>.yaml arranges the whole window), any mistake in its file, where \
             each of its blocks is, and every layout there is. Show one with run_command switch_layout { name }.", json!({}), &[]),
        tool("check_panel", "Load a plugin's panel out of sight for a moment: whether it works, its script errors, the tools it \
             called (none of them run) and what it shows. Use it after writing or changing a panel.",
             json!({ "panel": { "type": "string", "description": "<plugin> or <plugin>/<file>" }, "props": { "type": "object" } }), &["panel"]),
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
        tool("report_issue", "Send a bug report about Valerie to its GitHub issues (the triage.repo setting) from the user's GitHub \
             account, signing them in first if needed. The user sees the exact text and says yes before it's posted. Remove \
             anything private first. Returns the issue's address.", json!({
            "title": { "type": "string", "description": "Short title of the problem (not needed with issue)" },
            "body": { "type": "string", "description": "Markdown: what happened, steps to make it happen, what they expected, version and computer, the log lines that matter" },
            "issue": { "type": "integer", "description": "An existing issue's number: add the report there as a comment instead" } }), &["body"]),
        tool("screenshot_ui", "Capture Valerie's own interface as a PNG (to check a change to its look).",
             json!({ "part": { "type": "string", "description": "sidebar (default), command_bar, chats, welcome, plugins, little_bar (the front little window's top bar) or panel:<block id> for a layout's panel" },
                     "width": { "type": "integer" } }), &[]),
        tool("read_ui", "Read Valerie's own interface: its visible text and an outline of elements with their ids and classes \
             (the names to use in ui/*.css).", json!({ "part": { "type": "string", "description": "sidebar (default), command_bar, chats, welcome, plugins, little_bar (the front little window's top bar) or panel:<block id> for a layout's panel" } }), &[]),
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
    if let Some(what) = risky_action(name, &args, proxy)? {
        ask_each_time(&what, name, &args, "", proxy)?;
    }
    // Only a chat can show the user the report before it goes out.
    if name == "report_issue" {
        return Err("report_issue only works from a chat".into());
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

/// Tabs open now, by id, with the tab a hidden one was opened from.
fn open_tabs(proxy: &EventLoopProxy<UserEvent>) -> Option<Vec<Value>> {
    call(proxy, "list_tabs", json!({})).ok()?.as_array().cloned()
}

/// Tabs that weren't open before, opened from `tab` (a hidden one says where it came from), as sentences.
fn new_tabs(proxy: &EventLoopProxy<UserEvent>, tab: Option<i64>, before: &Option<Vec<Value>>) -> Vec<String> {
    let (Some(before), Some(now)) = (before, open_tabs(proxy)) else { return vec![] };
    now.iter()
        .filter(|t| !before.iter().any(|b| b["tab"] == t["tab"]))
        .filter(|t| t["section"] != "hidden" || t["opened_from"].as_i64() == tab)
        .map(|t| {
            let hidden = if t["section"] == "hidden" { "hidden " } else { "" };
            format!("It opened a new {hidden}tab: {} {} (use that tab id to work in it).", t["tab"], t["url"].as_str().unwrap_or(""))
        })
        .collect()
}

/// Start answering the page's alert, confirm and prompt for an action, as its `dialog` argument says.
fn watch_dialogs(proxy: &EventLoopProxy<UserEvent>, tab: Option<i64>, a: &Value) {
    let plan = json!({ "dialog": a["dialog"], "dialog_text": a["dialog_text"] });
    if let Err(e) = eval(proxy, tab, &with_args(&plan, &format!("{FIND_JS}\n{DIALOG_JS}"))) {
        crate::log::line("mcp", format!("couldn't watch the page's dialogs: {e}"));
    }
}

/// The page's alerts, confirms and prompts since `watch_dialogs`, as sentences (each said once).
fn dialogs_seen(proxy: &EventLoopProxy<UserEvent>, tab: Option<i64>, a: &Value) -> Vec<String> {
    let seen = eval(proxy, tab, DIALOGS_SEEN_JS).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let dismissing = a["dialog"] == "dismiss";
    seen.iter()
        .map(|d| {
            let said = d["message"].as_str().unwrap_or("").chars().take(300).collect::<String>();
            match (d["kind"].as_str().unwrap_or(""), &d["answered"]) {
                ("confirm", Value::Bool(true)) => format!("The page asked “{said}” (confirm) and got OK."),
                ("confirm", _) => format!("The page asked “{said}” (confirm) and got Cancel.{}",
                    if dismissing { "" } else { " To say OK, repeat this with dialog: \"accept\"." }),
                ("prompt", Value::String(v)) => format!("The page asked “{said}” (prompt) and got “{v}”."),
                ("prompt", _) => format!("The page asked “{said}” (prompt) and got Cancel.{}",
                    if dismissing { "" } else { " To answer it, repeat this with dialog: \"accept\" and dialog_text." }),
                _ => format!("The page showed an alert: “{said}” (it was closed with OK)."),
            }
        })
        .collect()
}

/// An action's result with what else happened added: text gets the sentences, anything else is wrapped.
fn with_notes(v: Value, notes: Vec<String>) -> Value {
    if notes.is_empty() {
        return v;
    }
    match v {
        Value::String(text) => json!(format!("{text}\n{}", notes.join("\n"))),
        other => json!({ "result": other, "notes": notes }),
    }
}

/// Run a click, typing or key press, then wait (up to 4s) for the page to react: until nothing has changed for a
/// moment, or a page it went to has loaded. Says where it ended up if the address changed, which tabs it opened and
/// what the page asked in dialogs meanwhile.
fn act(proxy: &EventLoopProxy<UserEvent>, a: &Value, script: &str) -> Result<Value, String> {
    let tab = match a.get("tab").and_then(Value::as_i64) {
        Some(t) => Some(t),
        None => call(proxy, "ensure_loaded", json!({ "tab": Value::Null }))?["tab"].as_i64(),
    };
    let before = open_tabs(proxy);
    watch_dialogs(proxy, tab, a);
    let _ = eval(proxy, tab, FRAMES_JS);
    let done = match eval(proxy, tab, &format!("{WATCH_JS}\n{script}")) {
        Ok(v) => v,
        Err(e) => {
            let notes = dialogs_seen(proxy, tab, a);
            return Err(if notes.is_empty() { e } else { format!("{e}\n{}", notes.join("\n")) });
        }
    };
    let start = Instant::now();
    let mut last = Value::Null;
    while start.elapsed() < Duration::from_millis(4000) {
        std::thread::sleep(Duration::from_millis(100));
        let check = &format!("{FINISH_ANIMATIONS_JS}\nconst m = window.__vgMut; \
            const busy = [...document.querySelectorAll('[aria-busy=true],[role=progressbar],[class*=spinner i],[class*=loading i]')].some((e) => e.getClientRects().length && getComputedStyle(e).visibility !== 'hidden'); \
            return {{ url: location.href, title: document.title, ready: document.readyState, quiet: m ? performance.now() - m.at : null, was: m ? m.url : null, was_title: m ? m.title : null, busy }};");
        let Ok(v) = eval_for(proxy, tab, check, 2) else { continue };
        let ready = v["ready"] == "complete";
        // A site that changes its address itself (GitHub, YouTube) sets the new title a moment later.
        let title_pending = v["was"].is_string() && v["was"] != v["url"] && v["was_title"] == v["title"] && start.elapsed() < Duration::from_millis(2000);
        let settled = match v["quiet"].as_f64() {
            Some(q) => q >= 300.0 && ready && v["busy"] != true && !title_pending,
            None => ready && start.elapsed() > Duration::from_millis(400),
        };
        last = v;
        if settled {
            break;
        }
    }
    let mut notes = dialogs_seen(proxy, tab, a);
    notes.extend(new_tabs(proxy, tab, &before));
    let moved = last["quiet"].is_null() || (last["was"].is_string() && last["was"] != last["url"]);
    let done = match (done, moved && last["url"].is_string()) {
        (Value::String(text), true) => {
            let title = last["title"].as_str().filter(|t| !t.is_empty() && last["was_title"].as_str() != Some(t));
            json!(format!("{text}\nThe page is now {}{}.", last["url"].as_str().unwrap_or(""), title.map(|t| format!(" “{t}”")).unwrap_or_default()))
        }
        (other, _) => other,
    };
    Ok(with_notes(done, notes))
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
    // Let a hidden page that waits on animation frames draw before it's read or pictured.
    if eval(proxy, tab, FRAMES_JS) == Ok(Value::Bool(true)) {
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(tab)
}

/// A file's type from its name, for upload_file (sites check it: an image uploader turns away "").
fn file_type(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "heic" => "image/heic",
        "pdf" => "application/pdf",
        "txt" | "md" | "typ" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "",
    }
}

/// Prefix page scripts with their arguments as a JS literal.
fn with_args(args: &Value, body: &str) -> String {
    format!("const A = {};\n{}", args, body)
}

const FIND_JS: &str = r#"
// The page, its open shadow roots and same-origin frames, in that order: sites built from web components or frames
// keep their buttons there.
function* roots(start) {
  const stack = [start || document];
  while (stack.length) {
    const root = stack.shift();
    yield root;
    for (const el of root.querySelectorAll('*')) {
      if (el.shadowRoot) stack.push(el.shadowRoot);
      if (el.tagName === 'IFRAME' || el.tagName === 'FRAME') { try { if (el.contentDocument) stack.push(el.contentDocument); } catch (err) {} }
    }
  }
}
function deepQuery(sel) {
  const first = document.querySelector(sel);
  if (first) return first;
  for (const r of roots()) { const e = r.querySelector(sel); if (e) return e; }
  return null;
}
function deepActive() {
  const real = realActive(), mine = window.__vgFocus;
  return (!real || real === document.body || real === document.documentElement) && mine && mine.isConnected ? mine : real;
}
// Pages in a background tab or an unfocused window don't take focus: Valerie tells the element itself (focus and
// focusin, blur on the last one) and remembers it, so keys and typing go there.
function focusOn(el) {
  if (!el || !el.focus) return;
  el.focus({ preventScroll: true });
  const prev = window.__vgFocus;
  if (realActive() === el) { window.__vgFocus = el; return; }
  if (prev === el && el.isConnected) return;
  const F = el.ownerDocument.defaultView.FocusEvent;
  if (prev && prev.isConnected) { prev.dispatchEvent(new F('blur', { relatedTarget: el })); prev.dispatchEvent(new F('focusout', { bubbles: true, composed: true, relatedTarget: el })); }
  window.__vgFocus = el;
  el.dispatchEvent(new F('focus', { relatedTarget: prev || null }));
  el.dispatchEvent(new F('focusin', { bubbles: true, composed: true, relatedTarget: prev || null }));
}
function realActive() {
  let a = document.activeElement;
  for (;;) {
    let inner = null;
    if (a && a.shadowRoot) inner = a.shadowRoot.activeElement;
    else if (a && a.contentDocument) { try { inner = a.contentDocument.activeElement; } catch (err) {} }
    if (!inner || inner === a) return a;
    a = inner;
  }
}
function find() {
  const e = A.ref ? deepQuery('[data-valerie-ref="' + CSS.escape(String(A.ref)) + '"]') : A.selector ? deepQuery(A.selector) : deepActive();
  if (!e) throw new Error(A.ref ? "no element with ref " + A.ref + " (take a new snapshot)" : "no element matches " + A.selector);
  return e;
}
function describe(e) {
  // A field by its name, not its options or what's typed in it.
  const field = /^(INPUT|TEXTAREA|SELECT)$/.test(e.tagName) && !/^(submit|button|reset|image)$/.test(e.type);
  const name = field ? e.getAttribute("aria-label") || (e.labels && e.labels[0] && e.labels[0].innerText) || e.placeholder || e.name || (e.id ? '#' + e.id : '')
    : e.innerText || e.value || e.getAttribute("aria-label");
  return (e.tagName.toLowerCase() + " " + String(name || "").trim().replace(/\s+/g, " ").slice(0, 80)).trim();
}
// Whether a person could see `e` at all: laid out, not hidden, not in a closed <details> or menu, with some size.
function rendered(e) {
  if (e.checkVisibility && !e.checkVisibility({ visibilityProperty: true, checkVisibilityCSS: true })) return false;
  const r = e.getBoundingClientRect();
  return e.getClientRects().length > 0 && r.width > 0 && r.height > 0;
}
// display: contents has no box of its own (checkVisibility says no) but its children show.
const drawn = (e) => !e.checkVisibility || e.checkVisibility() || getComputedStyle(e).display === 'contents';
// The page's text, then what's in open shadow roots and same-origin frames (innerText leaves those out).
function pageText() {
  const parts = [];
  for (const r of roots()) {
    if (r.nodeType === 9) {
      const f = r.defaultView && r.defaultView.frameElement;
      if (r.body && (r === document || !f || drawn(f))) parts.push(r.body.innerText);
      continue;
    }
    if (!drawn(r.host)) continue;
    for (const n of r.childNodes) {
      if (n.nodeType === 3) parts.push(n.data.trim());
      else if (n.nodeType === 1 && !/^(STYLE|SCRIPT|TEMPLATE|SLOT|LINK)$/.test(n.tagName) && drawn(n)) parts.push(n.innerText);
    }
  }
  return parts.filter(Boolean).join('\n\n');
}
// `a` is `b` or inside it, through shadow roots.
function within(a, b) { for (let n = a; n; n = n.parentNode || n.host) if (n === b) return true; return false; }
// What's at the middle of `e` if it isn't `e` (or part of it, or its label): null when a click there reaches `e`.
function coverAt(e) {
  const r = e.getBoundingClientRect();
  if (r.width === 0) return { hit: null, cover: null };
  const x = r.left + r.width / 2, y = r.top + r.height / 2, root = e.getRootNode();
  let hit = (root.elementFromPoint ? root : e.ownerDocument).elementFromPoint(x, y);
  while (hit && hit.shadowRoot) { const inner = hit.shadowRoot.elementFromPoint(x, y); if (!inner || inner === hit) break; hit = inner; }
  const label = hit && hit.closest && hit.closest('label');
  const ok = !hit || within(hit, e) || within(e, hit) || (label && label.control === e);
  return { x, y, hit, cover: ok ? null : hit };
}
// A sticky header or bar over the edge of the screen, not something over the page: scrolling gets past it.
function edgeBar(hit) {
  const W = hit.ownerDocument.defaultView;
  for (let n = hit; n && n.nodeType === 1; n = n.parentElement || (n.getRootNode() && n.getRootNode().host)) {
    const pos = W.getComputedStyle(n).position;
    if (pos === 'fixed' || pos === 'sticky') { const r = n.getBoundingClientRect(); return r.width * r.height < W.innerWidth * W.innerHeight * 0.4; }
  }
  return false;
}
// Says what's on top, for the agent: the open dialog, or the covering thing's words.
function coverName(hit) {
  const D = hit.ownerDocument, clean = (t) => String(t || '').trim().replace(/\s+/g, ' ').slice(0, 50);
  const d = hit.closest('[role=dialog],[role=alertdialog],[aria-modal=true],dialog') || D.querySelector('[role=dialog]:not([data-state=closed]),[role=alertdialog],[aria-modal=true],dialog[open]');
  if (d) {
    const by = d.getAttribute('aria-labelledby'), h = by && D.getElementById(by.split(' ')[0]) || d.querySelector('h1,h2,h3,[role=heading]');
    const name = clean(d.getAttribute('aria-label') || (h && h.innerText));
    return 'a dialog' + (name ? ' “' + name + '”' : '');
  }
  const text = clean(hit.innerText || hit.getAttribute('aria-label') || hit.getAttribute('title'));
  return text ? hit.tagName.toLowerCase() + ' “' + text + '”' : 'an overlay (an empty ' + hit.tagName.toLowerCase() + ')';
}
// Where a person's pointer would land on `e` (scrolled into view first, and clear of sticky bars), and what's on top.
function aim(e) {
  const view = e.ownerDocument.defaultView;
  const r = e.getBoundingClientRect();
  if (r.bottom <= 0 || r.top >= view.innerHeight || r.right <= 0 || r.left >= view.innerWidth) e.scrollIntoView({ block: 'center', inline: 'center' });
  let c = coverAt(e);
  if (c.cover && edgeBar(c.cover)) { e.scrollIntoView({ block: 'center', inline: 'center' }); c = coverAt(e); }
  return { view, x: c.x || 0, y: c.y || 0, hit: c.hit, target: c.hit && within(c.hit, e) ? c.hit : e, covered: c.cover };
}
"#;

/// How a page shows it takes payment, and what a click (or Enter on a form) would press, in every word a site might
/// describe it with. Hidden frames don't count: Stripe's script keeps invisible ones on every page of a site using it.
const PAYING_JS: &str = r#"
const CARD_FIELD = ['autocomplete*="cc-"', 'name*="cardnumber" i', 'name*="card_number" i', 'name*="card-number" i', 'name*="cvc" i', 'name*="cvv" i',
  'id*="cardnumber" i', 'id*="card_number" i', 'id*="card-number" i', 'id*="cvc" i', 'id*="cvv" i'].map((a) => 'input[' + a + ']').join(',');
const PAY_FRAME = /(^|\.)(stripe\.com|paypal\.com|adyen\.com|braintreegateway\.com|braintree-api\.com|squareup\.com|squarecdn\.com|checkout\.com|klarna\.com|affirm\.com|razorpay\.com|paddle\.com|recurly\.com|authorize\.net|mollie\.com|shopifyinc\.com|payments\.amazon\.com|pay\.google\.com)$/i;
const PAY_SITE = /^(checkout\.stripe\.com|buy\.stripe\.com|pay\.google\.com)$/i;
const WALLET = 'apple-pay-button,google-pay-button,gpay-button,.apple-pay-button,.gpay-button,.paypal-buttons,#paypal-button-container';
function shown(e) {
  const r = e.getBoundingClientRect();
  if (r.width < 20 || r.height < 10) return false;
  const s = getComputedStyle(e);
  return s.display !== 'none' && s.visibility !== 'hidden' && s.opacity !== '0';
}
function paymentSign() {
  if (PAY_SITE.test(location.hostname)) return 'a payment site';
  if ([...document.querySelectorAll(CARD_FIELD)].some(shown)) return 'card fields';
  const payFrame = (f) => { try { return PAY_FRAME.test(new URL(f.src, location.href).hostname) && shown(f); } catch (e) { return false; } };
  if ([...document.querySelectorAll('iframe[src]')].some(payFrame)) return 'a payment form';
  if ([...document.querySelectorAll(WALLET)].some(shown)) return 'a pay button';
  return '';
}
function pressTarget(hit, submits) {
  const pressable = 'button,a,summary,label,input,[role=button],[role=link],[role=menuitem],[role=option],[onclick]';
  const isButton = (e) => e.matches('button,a,summary,[role=button],[role=link],input[type=submit],input[type=button],input[type=image]');
  const form = hit.tagName === 'FORM' ? hit : hit.form;
  const submit = submits && form && !isButton(hit) ? form.querySelector('[type=submit],button:not([type=button]):not([type=reset]),input[type=image]') || form : hit;
  const nodes = [...new Set([submit, submit.closest(pressable) || submit])];
  const parts = [];
  for (const e of nodes) {
    parts.push(e.innerText, e.textContent);
    if (e.matches('input[type=submit],input[type=button],input[type=image]')) parts.push(e.value);
    for (const a of e.attributes) if (!/^(style|data-valerie-ref)$/.test(a.name)) parts.push(a.name.startsWith('data-') || a.name.startsWith('aria-') || /^(id|name|title|alt|href|action|formaction|value|onclick)$/.test(a.name) ? a.value : '');
    for (const i of e.querySelectorAll ? e.querySelectorAll('img[alt],[aria-label],[title]') : []) parts.push(i.getAttribute('alt'), i.getAttribute('aria-label'), i.getAttribute('title'));
    if (e.form) parts.push(e.form.getAttribute('action'), e.form.id, e.form.getAttribute('name'));
  }
  const c = nodes[nodes.length - 1];
  // A text editor (or its frame), or a long block of page text that isn't a button, says what's written, not what a press does.
  const editor = hit.isContentEditable || hit.tagName === 'TEXTAREA' || !!hit.querySelector('[contenteditable]:not([contenteditable=false]),textarea');
  const prose = !submit.closest(pressable) && (editor || (hit.innerText || '').length > 300);
  if (prose) parts.length = 0;
  const label = (c.innerText || c.value || c.getAttribute('aria-label') || c.getAttribute('title') || '').trim().replace(/\s+/g, ' ').slice(0, 80);
  return { site: location.hostname || location.href, label, text: parts.filter(Boolean).map((p) => String(p).slice(0, 1000)).join(' \n '), sign: paymentSign(), card_field: hit.matches(CARD_FIELD) };
}
"#;

/// A person's click, hover or drag, event by event, at the middle of the element, on whatever is on top there.
const POINTER_JS: &str = r#"
// What a person couldn't do to `e`: it's disabled, or not shown on the page.
function refuse(e, verb) {
  const ctl = e.closest('button,input,select,textarea');
  if (verb === 'click' && ctl && ctl.matches(':disabled')) throw new Error(describe(e) + ' is disabled, so clicking it does nothing. The page enables it once something else is done first (a required field, a choice); do that, then snapshot again.');
  if (A.force) return;
  if (verb === 'click' && e.closest('[aria-disabled=true]')) throw new Error(describe(e) + ' is marked disabled (aria-disabled), so the page will most likely ignore a click. Do what enables it first, or pass force: true if you\'re sure.');
  if (!rendered(e)) throw new Error(describe(e) + ' isn\'t shown on the page (it\'s hidden, inside a closed menu or section, or has no size), so a person couldn\'t ' + verb + ' it. Open what holds it first: click the menu button, tab or summary that shows it, then snapshot again. hover doesn\'t open menus that only show on CSS :hover; for those, force: true does it anyway.');
}
function coveredError(e, a, verb) { return new Error(describe(e) + ' is covered by ' + coverName(a.covered) + ', so a person couldn\'t ' + verb + ' it. Deal with what\'s on top first (close it, or click that instead), or pass force: true if you\'re sure.'); }
// Fires mouse and pointer events of one kind (`fire('mousedown', extra)`) at a point, as a mouse would.
function firer(v, x, y) {
  const base = { bubbles: true, cancelable: true, composed: true, view: v, clientX: x, clientY: y, screenX: x, screenY: y, button: 0 };
  return (t, type, extra) => t.dispatchEvent(type.startsWith('pointer')
    ? new v.PointerEvent(type, { ...base, pointerId: 1, pointerType: 'mouse', isPrimary: true, width: 1, height: 1, ...extra })
    : new v.MouseEvent(type, { ...base, ...extra }));
}
function arrive(fire, t, buttons) { for (const k of ['pointerover', 'pointerenter', 'mouseover', 'mouseenter', 'pointermove', 'mousemove']) fire(t, k, { bubbles: !k.endsWith('enter'), buttons }); }
function pointerAt(e, verb) {
  refuse(e, verb);
  const a = aim(e);
  if (a.covered && !A.force) throw coveredError(e, a, verb);
  const t = a.covered ? e : a.target, f = firer(t.ownerDocument.defaultView, a.x, a.y);
  const fire = (type, extra) => f(t, type, extra);
  arrive(f, t, 0);
  return { t, fire };
}
function clickOn(e) {
  const { t, fire } = pointerAt(e, 'click');
  const right = A.button === 'right', button = right ? 2 : 0, held = right ? 2 : 1;
  const press = (detail) => {
    const down = fire('pointerdown', { button, buttons: held }) && fire('mousedown', { button, buttons: held, detail });
    // Like a real press: focus moves unless the page stopped the mousedown.
    if (down && detail === 1) focusOn(t.closest('input,select,textarea,button,a[href],summary,[tabindex],[contenteditable]:not([contenteditable=false])'));
    if (right) fire('contextmenu', { button, buttons: held, detail });
    fire('pointerup', { button, buttons: 0 });
    fire('mouseup', { button, buttons: 0, detail });
    if (!right) fire('click', { buttons: 0, detail });
  };
  press(1);
  if (A.double) { press(2); fire('dblclick', { button, buttons: 0, detail: 2 }); }
  return (A.double ? 'double-clicked ' : right ? 'right-clicked ' : 'clicked ') + describe(e);
}
function hoverOn(e) { pointerAt(e, 'hover over'); return 'hovering over ' + describe(e); }
// Press on `src`, move across in steps, let go on `dst`. A draggable source also gets the page's drag-and-drop events,
// with one DataTransfer from dragstart to drop.
function dragOn(src, dst) {
  if (src.ownerDocument !== dst.ownerDocument) throw new Error('drag works within one page or frame: ' + describe(src) + ' and ' + describe(dst) + ' are in different ones');
  refuse(src, 'drag'); refuse(dst, 'drop onto');
  const a1 = aim(src);
  if (a1.covered && !A.force) throw coveredError(src, a1, 'drag');
  const a2 = aim(dst);
  if (a2.covered && !A.force) throw coveredError(dst, a2, 'drop onto');
  // Bringing the target into view can move the source.
  const c1 = coverAt(src), x1 = c1.x == null ? a1.x : c1.x, y1 = c1.y == null ? a1.y : c1.y, x2 = a2.x, y2 = a2.y;
  const D = src.ownerDocument, v = D.defaultView;
  const s = c1.hit && within(c1.hit, src) ? c1.hit : src, t = a2.covered ? dst : a2.target;
  const under = (x, y) => { let h = D.elementFromPoint(x, y); while (h && h.shadowRoot) { const i = h.shadowRoot.elementFromPoint(x, y); if (!i || i === h) break; h = i; } return h; };
  arrive(firer(v, x1, y1), s, 0);
  firer(v, x1, y1)(s, 'pointerdown', { buttons: 1 });
  firer(v, x1, y1)(s, 'mousedown', { buttons: 1, detail: 1 });
  // Checked after the press: some libraries make the item draggable on pointerdown.
  let dragEl = null;
  for (let n = s; n && n.nodeType === 1; n = n.parentElement || (n.getRootNode().host || null)) if (n.draggable) { dragEl = n; break; }
  const dt = new v.DataTransfer();
  const dragEvent = (el, type, x, y) => el.dispatchEvent(new v.DragEvent(type, { bubbles: true, cancelable: !/^(dragleave|dragend)$/.test(type), composed: true, view: v, clientX: x, clientY: y, screenX: x, screenY: y, buttons: 1, dataTransfer: dt }));
  const nudge = firer(v, x1 + 3, y1 + 3);
  nudge(s, 'pointermove', { buttons: 1 }); nudge(s, 'mousemove', { buttons: 1 });
  const html5 = !!dragEl && dragEvent(dragEl, 'dragstart', x1 + 3, y1 + 3);
  let over = s, accepted = false;
  const STEPS = 6;
  for (let i = 1; i <= STEPS; i++) {
    const x = x1 + (x2 - x1) * i / STEPS, y = y1 + (y2 - y1) * i / STEPS;
    const here = i === STEPS ? t : under(x, y) || t, f = firer(v, x, y);
    if (here !== over) {
      f(over, 'pointerout', { buttons: 1 }); f(over, 'mouseout', { buttons: 1 });
      for (const k of ['pointerover', 'pointerenter', 'mouseover', 'mouseenter']) f(here, k, { bubbles: !k.endsWith('enter'), buttons: 1 });
    }
    f(here, 'pointermove', { buttons: 1 }); f(here, 'mousemove', { buttons: 1 });
    if (html5) {
      dragEvent(dragEl, 'drag', x, y);
      if (here !== over) { dragEvent(here, 'dragenter', x, y); dragEvent(over, 'dragleave', x, y); }
      accepted = !dragEvent(here, 'dragover', x, y);
    }
    over = here;
  }
  if (html5) {
    if (accepted) dragEvent(t, 'drop', x2, y2);
    dragEvent(dragEl, 'dragend', x2, y2);
  }
  firer(v, x2, y2)(t, 'pointerup', { buttons: 0 });
  firer(v, x2, y2)(t, 'mouseup', { buttons: 0, detail: 1 });
  return 'dragged ' + describe(src) + ' onto ' + describe(dst) + (html5 && !accepted ? ', but it didn\'t take the drop (nothing there accepts dragged things), so nothing may have moved' : '');
}
"#;

/// Answers the page's alert, confirm and prompt while an agent's action is under way (until 5 seconds after it), as
/// `A.dialog` says, and notes each one for the result. Otherwise the page gets the real ones. Every window and
/// same-origin frame, once each.
const DIALOG_JS: &str = r#"
if (!window.__vgDialogs) Object.defineProperty(window, '__vgDialogs', { value: { until: 0, plan: {}, seen: [] } });
const S = window.__vgDialogs;
S.plan = { accept: A.dialog === 'accept', text: A.dialog_text };
S.until = Date.now() + 60000;
const wrap = (w) => {
  if (!w || w.__vgDialogsOn) return;
  Object.defineProperty(w, '__vgDialogsOn', { value: true });
  const real = { alert: w.alert, confirm: w.confirm, prompt: w.prompt }, live = () => Date.now() < S.until;
  const note = (o) => { if (S.seen.length < 20) S.seen.push(o); };
  const said = (m) => m === undefined ? '' : String(m);
  // A page that put its own dialogs in their place keeps them.
  if (![real.alert, real.confirm, real.prompt].every((f) => typeof f === 'function' && /\[native code\]/.test(Function.prototype.toString.call(f)))) return;
  w.alert = function alert(message) { if (!live()) return real.alert.apply(w, arguments); note({ kind: 'alert', message: said(message) }); };
  w.confirm = function confirm(message) {
    if (!live()) return real.confirm.apply(w, arguments);
    note({ kind: 'confirm', message: said(message), answered: S.plan.accept });
    return S.plan.accept;
  };
  w.prompt = function prompt(message, value) {
    if (!live()) return real.prompt.apply(w, arguments);
    const answer = S.plan.accept ? (S.plan.text != null ? String(S.plan.text) : said(value)) : null;
    note({ kind: 'prompt', message: said(message), answered: answer });
    return answer;
  };
};
wrap(window);
for (const r of roots()) if (r.nodeType === 9 && r.defaultView !== window) { try { wrap(r.defaultView); } catch (err) {} }
return true;
"#;

/// The dialogs answered since the last look (each reported once); they go on being answered for 5 more seconds.
const DIALOGS_SEEN_JS: &str = "const S = window.__vgDialogs; if (!S) return []; S.until = Date.now() + 5000; return S.seen.splice(0);";

/// Tabs out of sight never draw, so a menu or dialog's closing animation never ends and it stays on the page, often with
/// its overlay over everything. In those tabs, short animations (opening and closing ones, 1s or less) are finished
/// and their end announced. Longer ones (a toast's countdown) and scroll-driven ones are left alone.
const FINISH_ANIMATIONS_JS: &str = r#"
if (document.visibilityState === 'hidden' && document.getAnimations) {
  for (const a of document.getAnimations()) {
    try {
      const el = a.effect && a.effect.target, t = a.effect && a.effect.getComputedTiming();
      if (!el || a.playState !== 'running' || a.timeline !== document.timeline || !(t.endTime <= 1000)) continue;
      a.finish();
      const W = el.ownerDocument.defaultView;
      if (a.animationName) el.dispatchEvent(new W.AnimationEvent('animationend', { animationName: a.animationName, bubbles: true }));
      else if (a.transitionProperty) el.dispatchEvent(new W.TransitionEvent('transitionend', { propertyName: a.transitionProperty, bubbles: true }));
    } catch (err) {}
  }
}
"#;

/// In a tab the user has in the background, an agent's page still draws while it works there (content.js timerFrames;
/// tabs no one sees have it from the start). True when it was just turned on, and nudges the page to draw again.
const FRAMES_JS: &str = "if (document.visibilityState !== 'hidden' || !window.__valerie || !__valerie.timerFrames || !__valerie.timerFrames()) return false; dispatchEvent(new Event('resize')); return true;";

/// Notes DOM changes after an action, so `act` can wait until the page has reacted.
const WATCH_JS: &str = "if (!window.__vgMut) { const m = window.__vgMut = { at: 0 }; new MutationObserver(() => { m.at = performance.now(); }).observe(document, { subtree: true, childList: true, attributes: true, characterData: true }); } window.__vgMut.at = performance.now(); window.__vgMut.url = location.href; window.__vgMut.title = document.title;";

/// Key events with the modifiers a shortcut names ("cmd+a", "Ctrl+Shift+Z", "mod+z" = ⌘ on Mac, Ctrl elsewhere).
const KEYS_JS: &str = r#"
const MAC = /Mac|iPhone|iPad/.test(navigator.platform);
function keyEvent(spec) {
  const parts = String(spec).split('+');
  let key = parts.pop();
  if (key === '' && parts.length) { parts.pop(); key = '+'; }
  const mods = parts.map((m) => m.trim().toLowerCase());
  const has = (...names) => mods.some((m) => names.includes(m));
  const meta = has('cmd', 'command', 'meta', 'super', 'win') || (MAC && has('mod'));
  const ctrl = has('ctrl', 'control') || (!MAC && has('mod'));
  const alt = has('alt', 'option', 'opt'), shift = has('shift');
  if (/^(space|spacebar)$/i.test(key)) key = ' ';
  const named = { Enter: 13, Escape: 27, Tab: 9, Backspace: 8, Delete: 46, ' ': 32, ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40, Home: 36, End: 35, PageUp: 33, PageDown: 34 };
  const one = key.length === 1;
  if (one && /[a-z]/i.test(key)) key = shift ? key.toUpperCase() : key.toLowerCase();
  const code = one && /[a-z]/i.test(key) ? 'Key' + key.toUpperCase() : one && /\d/.test(key) ? 'Digit' + key : key === ' ' ? 'Space' : key;
  const keyCode = named[key] || (one ? key.toUpperCase().charCodeAt(0) : 0);
  return { key, code, keyCode, which: keyCode, metaKey: meta, ctrlKey: ctrl, altKey: alt, shiftKey: shift, bubbles: true, cancelable: true, plain: !mods.length, command: (MAC ? meta : ctrl) && !alt };
}
function pressOn(e, spec) {
  const o = keyEvent(spec);
  const K = e.ownerDocument.defaultView.KeyboardEvent;
  const go = e.dispatchEvent(new K('keydown', o));
  e.dispatchEvent(new K('keyup', o));
  // A shortcut the page leaves alone does what the keyboard would.
  const builtIn = o.command && { a: 'selectAll', z: o.shiftKey ? 'redo' : 'undo', y: 'redo' }[o.key.toLowerCase()];
  if (go && builtIn === 'selectAll') selectAllIn(e);
  else if (go && builtIn) e.ownerDocument.execCommand(builtIn);
  return { go, o };
}
// Select all inside a field or editor, never the whole page (what the browser does when the page has no focus).
function selectAllIn(e) {
  const D = e.ownerDocument;
  if (e.tagName === 'INPUT' || e.tagName === 'TEXTAREA') { try { e.setSelectionRange(0, e.value.length); } catch (err) { e.select(); } return; }
  if (!e.isContentEditable) return D.execCommand('selectAll');
  let root = e;
  while (root.parentElement && root.parentElement.isContentEditable) root = root.parentElement;
  const r = D.createRange();
  r.selectNodeContents(root);
  const sel = D.defaultView.getSelection();
  sel.removeAllRanges(); sel.addRange(r);
}
// Without focus the browser's editing commands do nothing: change a field's text at its cursor directly.
function editField(e, text, back, ahead, inputType) {
  const W = e.ownerDocument.defaultView, a = e.selectionStart, b = e.selectionEnd;
  if (a == null) {
    // Fields with no cursor (email, number) change at the end.
    const v = e.value, proto = e.tagName === 'TEXTAREA' ? W.HTMLTextAreaElement.prototype : W.HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(e, back < 0 ? v.slice(0, back) : v + text);
  } else e.setRangeText(text, Math.max(0, a + back), Math.min(e.value.length, b + ahead), 'end');
  e.dispatchEvent(new W.InputEvent('input', { bubbles: true, inputType, data: text || null }));
}
// Not requestAnimationFrame: hidden tabs never paint, so it would never fire.
const nextFrame = () => new Promise((r) => setTimeout(r, 50));
"#;

const PRESS_KEY_JS: &str = r#"
const e = deepActive() || document.body;
const { go, o } = pressOn(e, A.key);
const bare = !o.metaKey && !o.ctrlKey && !o.altKey;
const D = e.ownerDocument, W = D.defaultView;
const field = e.isContentEditable || e.tagName === 'TEXTAREA' || (e.tagName === 'INPUT' && !/^(checkbox|radio|button|submit|reset|file|range|color)$/.test(e.type));
// What the key would do if the page lets it: real keys do this themselves, made-up ones don't.
if (go && bare) {
  if (o.key === 'Tab') {
    const all = [...D.querySelectorAll('a[href],button,input:not([type=hidden]),select,textarea,summary,[tabindex],[contenteditable]:not([contenteditable=false])')]
      .filter((x) => x.tabIndex >= 0 && !x.disabled && x.getClientRects().length && getComputedStyle(x).visibility !== 'hidden');
    const i = all.indexOf(e), next = all[(i + (o.shiftKey ? -1 : 1) + all.length) % all.length];
    if (next) focusOn(next);
  } else if (o.key.length === 1 && field) {
    // execCommand can say yes and do nothing when the page has no focus.
    const before = e.isContentEditable ? e.innerText : e.value;
    D.execCommand('insertText', false, o.key);
    if ((e.isContentEditable ? e.innerText : e.value) === before && !e.isContentEditable) editField(e, o.key, 0, 0, 'insertText');
  } else if ((o.key === 'Backspace' || o.key === 'Delete') && field) {
    const before = e.value;
    D.execCommand(o.key === 'Backspace' ? 'delete' : 'forwardDelete');
    if (!e.isContentEditable && e.value === before) {
      const empty = e.selectionStart === e.selectionEnd;
      editField(e, '', o.key === 'Backspace' && empty ? -1 : 0, o.key === 'Delete' && empty ? 1 : 0, o.key === 'Backspace' ? 'deleteContentBackward' : 'deleteContentForward');
    }
  } else if (o.key === 'Enter' && (e.isContentEditable || e.tagName === 'TEXTAREA')) {
    D.execCommand(e.isContentEditable ? 'insertParagraph' : 'insertText', false, '\n');
  } else if ((o.key === 'Enter' && e.matches('button,a[href],summary,[role=button],[role=link],input[type=submit],input[type=button]')) || (o.key === ' ' && e.matches('button,summary,[role=button],input[type=checkbox],input[type=radio],[role=checkbox],[role=switch]'))) {
    e.click();
  } else if (o.key === 'Enter' && e.form && e.tagName === 'INPUT') {
    e.form.requestSubmit ? e.form.requestSubmit() : e.form.submit();
  }
}
return 'pressed ' + A.key + (e !== document.body ? ' on ' + describe(e) : '');
"#;

/// Writes into a field or a code/rich editor. Editors (CodeMirror, Monaco, ProseMirror, Typst's) take a paste event as
/// one edit, without auto-indent or bracket closing mangling the text; plain fields get their value set.
const TYPE_JS: &str = r#"
return (async () => {
const picked = find();
// Tag names, not instanceof: a field inside a frame has that frame's own HTMLInputElement.
const tag = (x) => (x && x.tagName) || '';
const field = (x) => /^(INPUT|TEXTAREA|SELECT)$/.test(tag(x)) || !!(x && x.isContentEditable);
const focused = deepActive();
// A ref often lands on an editor's frame or preview: write into the editor inside it, or the one with the cursor.
const e = field(picked) ? picked : field(focused) && picked.contains(focused) ? focused
  : picked.querySelector('textarea,[contenteditable]:not([contenteditable=false]),input:not([type=hidden])') || (field(focused) ? focused : null);
if (!e) throw new Error('nothing to type into in ' + describe(picked) + ': click into the text box or editor, then call type without a ref');
if (e.matches(':disabled')) throw new Error(describe(e) + ' is disabled, so it can\'t be typed into. The page enables it once something else is done first; do that, then snapshot again.');
if (e.readOnly && tag(e) !== 'SELECT') throw new Error(describe(e) + ' is read-only: the page shows this value but won\'t take typing. Look for an edit button, or another field for it.');
if (e !== focused) focusOn(e);
const name = describe(e);
let how = 'typed into ', after = '';
const W = e.ownerDocument.defaultView, D = e.ownerDocument;
const plain = tag(e) === 'INPUT' || tag(e) === 'TEXTAREA', old = plain ? e.value : null;
const setValue = (v) => Object.getOwnPropertyDescriptor(tag(e) === 'TEXTAREA' ? W.HTMLTextAreaElement.prototype : W.HTMLInputElement.prototype, 'value').set.call(e, v);
const changed = () => { e.dispatchEvent(new W.Event('input', { bubbles: true })); e.dispatchEvent(new W.Event('change', { bubbles: true })); };
// A script's value skips maxlength; a person's typing stops there.
const fit = (before, add) => e.maxLength >= 0 && (before + add).length > e.maxLength ? before + add.slice(0, Math.max(0, e.maxLength - before.length)) : before + add;
// Fields whose value is one whole thing: typing replaces it, written the way the browser takes it.
const FORMAT = { date: 'YYYY-MM-DD, e.g. 2026-11-15', time: 'HH:MM in 24-hour time, e.g. 14:30', 'datetime-local': 'YYYY-MM-DDTHH:MM, e.g. 2026-11-15T14:30',
  month: 'YYYY-MM, e.g. 2026-11', week: 'YYYY-Www, e.g. 2026-W46', color: '#rrggbb, e.g. #ff8800', range: 'a plain number, e.g. 70',
  number: 'a plain number, e.g. 42 or 3.5 (no commas, units or spaces)' };
const whole = tag(e) === 'INPUT' && FORMAT[e.type];
let root = e;
while (root.isContentEditable && root.parentElement && root.parentElement.isContentEditable) root = root.parentElement;
// Each character as a key press, for fields that only listen to keys: the page can take it over at any step.
async function typeKeys() {
  const K = W.KeyboardEvent, I = W.InputEvent;
  if (plain) { try { e.setSelectionRange(A.clear ? 0 : e.value.length, e.value.length); } catch (err) { if (A.clear) { setValue(''); changed(); } } }
  else if (A.clear) selectAllIn(e);
  const put = (t) => {
    const sel = W.getSelection();
    let r = sel.rangeCount && root.contains(sel.getRangeAt(0).commonAncestorContainer) ? sel.getRangeAt(0) : null;
    if (!r) { r = D.createRange(); r.selectNodeContents(root); r.collapse(false); }
    r.deleteContents();
    const n = t === '\n' ? D.createElement('br') : D.createTextNode(t);
    r.insertNode(n); r.setStartAfter(n); r.collapse(true);
    sel.removeAllRanges(); sel.addRange(r);
  };
  for (const ch of A.text) {
    const enter = ch === '\n', key = enter ? 'Enter' : ch, cc = enter ? 13 : ch.charCodeAt(0);
    const code = enter ? 'Enter' : /^[a-z]$/i.test(ch) ? 'Key' + ch.toUpperCase() : /^\d$/.test(ch) ? 'Digit' + ch : ch === ' ' ? 'Space' : '';
    const keyCode = enter ? 13 : /^[a-z\d ]$/i.test(ch) ? ch.toUpperCase().charCodeAt(0) : 0;
    const o = { key, code, keyCode, which: keyCode, shiftKey: ch !== ch.toLowerCase(), bubbles: true, cancelable: true, composed: true };
    if (e.dispatchEvent(new K('keydown', o)) && e.dispatchEvent(new K('keypress', { ...o, keyCode: cc, which: cc, charCode: cc })) && !(enter && tag(e) === 'INPUT')) {
      const inputType = enter ? (e.isContentEditable ? 'insertParagraph' : 'insertLineBreak') : 'insertText', data = enter ? null : ch;
      const full = plain && e.maxLength >= 0 && e.value.length - Math.abs((e.selectionEnd || 0) - (e.selectionStart || 0)) >= e.maxLength;
      if (!full && e.dispatchEvent(new I('beforeinput', { bubbles: true, cancelable: true, composed: true, inputType, data }))) {
        if (plain) editField(e, enter ? '\n' : ch, 0, 0, inputType);
        else { put(enter ? '\n' : ch); root.dispatchEvent(new I('input', { bubbles: true, composed: true, inputType, data })); }
      }
    }
    e.dispatchEvent(new K('keyup', o));
    await new Promise((r) => setTimeout(r, 0));
  }
}
if (tag(e) === 'SELECT') {
  const o = [...e.options].find((o) => o.value === A.text || o.text.trim() === A.text.trim());
  if (!o) throw new Error('no option ' + A.text + '; the options are: ' + [...e.options].map((o) => o.text.trim()).join(', '));
  // A multiple-choice list keeps what's chosen and adds this (clear starts over).
  if (e.multiple && A.clear) for (const x of e.options) x.selected = false;
  o.selected = true;
  changed();
  how = 'chose “' + o.text.trim() + '” in ';
  if (e.multiple) after = '; selected now: ' + [...e.selectedOptions].map((x) => '“' + x.text.trim() + '”').join(', ');
} else if (whole) {
  // Tried on a field of the same kind first, so a value the browser turns away doesn't blank the page's.
  const t = A.text.trim(), probe = D.createElement('input');
  probe.type = e.type;
  for (const k of ['min', 'max', 'step']) if (e.hasAttribute(k)) probe.setAttribute(k, e.getAttribute(k));
  probe.value = t;
  const ok = e.type === 'color' ? probe.value === t.toLowerCase() : e.type === 'range' ? t !== '' && isFinite(Number(t)) : probe.value !== '' || t === '';
  if (!ok) throw new Error('“' + A.text + '” isn\'t a value a ' + e.type + ' field takes, so ' + name + ' wasn\'t changed. Type it as ' + FORMAT[e.type] + '.');
  setValue(t);
  changed();
} else if (A.keys) {
  how = 'pressed the keys into ';
  await typeKeys();
} else if (e.isContentEditable || tag(e) === 'TEXTAREA') {
  const sel = W.getSelection();
  // Where the text goes: everything with clear, else the cursor if it's in this editor, else the end.
  if (e.isContentEditable && (A.clear || !sel.rangeCount || !root.contains(sel.anchorNode))) {
    const r = D.createRange();
    r.selectNodeContents(root);
    if (!A.clear) r.collapse(false);
    sel.removeAllRanges(); sel.addRange(r);
  }
  if (A.clear) {
    pressOn(e, 'mod+a');
    await nextFrame();
  }
  let pasted = false;
  try {
    const dt = new W.DataTransfer();
    dt.setData('text/plain', A.text);
    pasted = !e.dispatchEvent(new W.ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true }));
  } catch (err) {}
  if (pasted) how = 'pasted into ';
  else if (e.isContentEditable && (() => { const before = root.innerText; return !D.execCommand('insertText', false, A.text) || (A.text && root.innerText === before); })()) {
    // Only ever inside this editor, on a range of our own (a page without focus can drop the selection's).
    let r = sel.rangeCount && root.contains(sel.getRangeAt(0).commonAncestorContainer) ? sel.getRangeAt(0) : null;
    if (!r) { r = D.createRange(); r.selectNodeContents(root); if (!A.clear) r.collapse(false); }
    r.deleteContents();
    r.insertNode(D.createTextNode(A.text));
    root.dispatchEvent(new W.InputEvent('input', { bubbles: true, inputType: 'insertText', data: A.text }));
  }
  else if (!e.isContentEditable) {
    // A plain textarea (pages ignore a made-up paste): set its value the way React and friends notice.
    setValue(fit(A.clear ? '' : e.value, A.text));
    changed();
  }
  // clear means only the new text is left: if the editing command kept some of the old, put it in directly.
  const flat = (t) => String(t).replace(/\s+/g, ' ').trim();
  if (A.clear && !pasted && e.isContentEditable && flat(root.innerText) !== flat(A.text)) {
    root.textContent = A.text;
    root.dispatchEvent(new W.InputEvent('input', { bubbles: true, inputType: 'insertReplacementText', data: A.text }));
  }
  await nextFrame();
} else {
  setValue(fit(A.clear ? '' : e.value, A.text));
  changed();
}
// What the field holds now, if the page (a length limit, a format) or the browser made it something else.
if (plain && !how.startsWith('pasted')) {
  await nextFrame();
  const want = (A.clear || whole ? '' : old) + (whole ? A.text.trim() : A.text), now = e.value;
  if (e.type === 'color' ? now.toLowerCase() !== want.toLowerCase() : now !== want) {
    const show = (v) => !v ? 'nothing' : e.type === 'password' ? v.length + ' characters' : '“' + (v.length > 200 ? v.slice(0, 200) + '…' : v) + '”';
    const hint = A.keys || whole ? '' : '. If it only takes key presses (formatted fields often do), repeat with keys: true';
    after += now === old && want !== old ? ', but the field still holds ' + show(now) + ': it didn\'t take the text' + hint
      : ', and the field now holds ' + show(now) + (now ? '' : hint);
  }
}
if (A.submit) {
  const { go } = pressOn(e, 'Enter');
  if (go && e.form) (e.form.requestSubmit ? e.form.requestSubmit() : e.form.submit());
}
return how + name + after;
})();
"#;

/// A click, typing or key from a tool has passed its own check (and maybe the user): a script's guard still running mustn't stop it.
const END_SCRIPT_GUARDS_JS: &str = "if (window.__valerieGuards) [...window.__valerieGuards.live].forEach((finish) => finish());";

/// Stops what a script presses or sends if it looks like paying. It runs inside the page, so a script set on getting
/// around it could; it's for agents heading somewhere by mistake.
const SCRIPT_GUARD_JS: &str = r#"
// The same rules as paying_word in mcp.rs.
const spaced = (t) => ' ' + String(t).replace(/(\p{Ll})(\p{Lu})/gu, '$1 $2').toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim() + ' ';
const wordIn = (text, words) => {
  const normal = spaced(text), lower = String(text).toLowerCase();
  return words.find((w) => spaced(w).trim() && /^[\x00-\x7f]*$/.test(w) ? normal.includes(spaced(w)) : w.trim() && lower.includes(w.trim().toLowerCase()));
};
// `$` means a price on a button, but is everyday syntax in addresses and request bodies.
const requestWords = WORDS.filter((w) => w.trim() !== '$');
if (!window.__valerieGuards) Object.defineProperty(window, '__valerieGuards', { value: { live: new Set(), undo: [] }, enumerable: false });
const guards = window.__valerieGuards, blocked = [];
let live = true;
const stop = (kind, what) => {
  blocked.push({ kind, what });
  console.warn('Valerie stopped an agent script: it tried to ' + what);
  return new Error('Valerie stopped this: ' + what + ' needs the user\'s yes');
};
const checkPress = (hit, submits) => {
  if (!live || !hit || hit.nodeType !== 1) return;
  const t = pressTarget(hit, submits);
  if (t.sign || wordIn(t.text, WORDS)) throw stop('click', (t.label ? 'click “' + t.label + '”' : 'click a button') + ' on ' + t.site + (t.sign ? ', a page that takes payment (' + t.sign + ')' : ''));
};
const bodyText = (b) => typeof b === 'string' ? b : b instanceof URLSearchParams ? String(b) : b instanceof FormData ? [...b.entries()].map(([k, v]) => k + ' ' + (typeof v === 'string' ? v : '')).join(' ') : '';
const checkRequest = (method, url, body) => {
  method = String(method || 'GET').toUpperCase();
  if (!live || method === 'GET' || method === 'HEAD' || method === 'OPTIONS') return;
  const sign = paymentSign();
  if (sign || wordIn(String(url) + ' ' + bodyText(body).slice(0, 4000), requestWords)) throw stop('request', 'send a ' + method + ' to ' + String(url).slice(0, 80) + (sign ? ' from a page that takes payment (' + sign + ')' : ''));
};
const patch = (obj, name, wrap) => {
  if (!obj || typeof obj[name] !== 'function') return;
  const orig = obj[name], mine = wrap(orig);
  obj[name] = mine;
  guards.undo.push(() => { if (obj[name] === mine) obj[name] = orig; });
};
const PRESS_EVENTS = /^(click|mousedown|mouseup|pointerdown|pointerup|submit)$/;
const watch = (w) => {
  const pressing = (submits) => (orig) => function (...a) { checkPress(this, submits); return orig.apply(this, a); };
  patch(w.HTMLElement.prototype, 'click', pressing(false));
  patch(w.HTMLFormElement.prototype, 'submit', pressing(true));
  patch(w.HTMLFormElement.prototype, 'requestSubmit', pressing(true));
  patch(w.EventTarget.prototype, 'dispatchEvent', (orig) => function (ev, ...a) {
    if (ev && (PRESS_EVENTS.test(ev.type) || (ev.type === 'keydown' && ev.key === 'Enter'))) checkPress(this, ev.type !== 'click');
    return orig.call(this, ev, ...a);
  });
  patch(w, 'fetch', (orig) => function (input, init) {
    try {
      const isRequest = input && typeof input === 'object' && 'method' in input;
      checkRequest((init && init.method) || (isRequest && input.method), isRequest ? input.url : input, init && init.body);
    } catch (e) { return Promise.reject(e); }
    return orig.apply(this, arguments);
  });
  const opened = new WeakMap();
  patch(w.XMLHttpRequest.prototype, 'open', (orig) => function (method, url) { opened.set(this, [method, url]); return orig.apply(this, arguments); });
  patch(w.XMLHttpRequest.prototype, 'send', (orig) => function (body) { const o = opened.get(this) || []; checkRequest(o[0], o[1], body); return orig.apply(this, arguments); });
  patch(w.navigator, 'sendBeacon', (orig) => function (url, data) { checkRequest('POST', url, data); return orig.apply(this, arguments); });
};
watch(window);
for (const f of document.querySelectorAll('iframe')) { try { if (f.contentWindow.HTMLElement) watch(f.contentWindow); } catch (e) {} }
// Scripts can overlap: the last one out unwraps everything, newest first, so nothing stays wrapped.
const finish = () => {
  live = false;
  guards.live.delete(finish);
  if (!guards.live.size) guards.undo.splice(0).reverse().forEach((u) => u());
};
guards.live.add(finish);
const giveUp = setTimeout(finish, 30000);
// A moment longer than the script, for the setTimeout(() => button.click()) it leaves behind.
const settled = () => { clearTimeout(giveUp); setTimeout(finish, 1000); return blocked.length ? { __blocked: blocked } : null; };
return { run(script) {
  let out;
  try { out = script(); } catch (e) { const r = settled(); if (r) return r; throw e; }
  if (out && typeof out.then === 'function') return Promise.resolve(out).then((v) => settled() || { __value: v }, (e) => { const r = settled(); if (r) return r; throw e; });
  return settled() || { __value: out };
} };
"#;

/// Scrolls the page, the panel an element is in, or (when the page itself doesn't scroll, as in mail and chat apps)
/// its biggest scrolling panel; then waits a moment for more to load and says where it got to.
const SCROLL_JS: &str = r#"
const scrolls = (el) => el.scrollHeight > el.clientHeight + 1 && /(auto|scroll|overlay)/.test(el.ownerDocument.defaultView.getComputedStyle(el).overflowY);
const pageOf = (el) => el.ownerDocument.scrollingElement || el.ownerDocument.documentElement;
let box = null;
if (A.ref || A.selector) {
  const e = find();
  if (!A.direction) { e.scrollIntoView({ block: 'center', behavior: 'instant' }); return 'scrolled to ' + describe(e); }
  for (let n = e; n && n.nodeType === 1; n = n.parentElement || n.getRootNode().host || null) {
    if (n === pageOf(n) || n === n.ownerDocument.body) break;
    if (scrolls(n)) { box = n; break; }
  }
  if (!box) box = pageOf(e);
} else {
  const se = pageOf(document.documentElement), flow = (el) => el ? getComputedStyle(el).overflowY : 'visible';
  const viewport = flow(document.documentElement) !== 'visible' ? flow(document.documentElement) : flow(document.body);
  if (se.scrollHeight > innerHeight + 1 && !/hidden|clip/.test(viewport)) box = se;
  else {
    let most = 0;
    for (const r of roots()) for (const el of r.querySelectorAll('*')) {
      if (el.clientHeight < 50 || !scrolls(el)) continue;
      const q = el.getBoundingClientRect(), W = el.ownerDocument.defaultView;
      const seen = Math.max(0, Math.min(q.bottom, W.innerHeight) - Math.max(q.top, 0)) * Math.max(0, Math.min(q.right, W.innerWidth) - Math.max(q.left, 0));
      if (seen > most) { most = seen; box = el; }
    }
    if (!box) box = se;
  }
}
const page = box === pageOf(box), h = page ? box.ownerDocument.defaultView.innerHeight : box.clientHeight;
const go = { down: () => box.scrollBy({ top: h * 0.8, behavior: 'instant' }), up: () => box.scrollBy({ top: -h * 0.8, behavior: 'instant' }),
  top: () => box.scrollTo({ top: 0, behavior: 'instant' }), bottom: () => box.scrollTo({ top: box.scrollHeight, behavior: 'instant' }) }[A.direction || 'down'];
if (!go) throw new Error('direction is one of up, down, top, bottom');
const start = box.scrollHeight;
go();
return (async () => {
  // A feed adds more once its end comes into view: wait until its height stops changing (or a moment if it doesn't).
  let last = start, still = 0;
  for (let t = 0; t < 3000; t += 100) {
    await new Promise((r) => setTimeout(r, 100));
    if (box.scrollHeight !== last) { last = box.scrollHeight; still = 0; } else if (++still >= (last === start ? 6 : 3)) break;
  }
  const y = box.scrollTop, out = { scrollY: Math.round(y), scrollHeight: box.scrollHeight, viewport: Math.round(h), at_top: y <= 1, at_bottom: y + h >= box.scrollHeight - 2 };
  if (!page) {
    const label = box.getAttribute('aria-label');
    out.scrolled = box.tagName.toLowerCase() + (box.id ? '#' + box.id : box.classList[0] ? '.' + box.classList[0] : '') + (label ? ' “' + label.slice(0, 60) + '”' : '') + (box.ownerDocument !== document ? ' (in a frame)' : '');
  }
  if (box.scrollHeight > start) out.loaded_more = true;
  return out;
})();
"#;

const SNAPSHOT_JS: &str = r#"
const CLICKY = 'a[href],button,input:not([type=hidden]),select,textarea,summary,[role=button],[role=link],[role=tab],[role=menuitem],[role=menuitemcheckbox],[role=menuitemradio],[role=checkbox],[role=radio],[role=switch],[role=option],[role=treeitem],[role=textbox],[role=searchbox],[role=combobox],[role=slider],[role=spinbutton],[onclick],[tabindex]:not([tabindex="-1"])';
const clean = (t) => String(t || '').trim().replace(/\s+/g, ' ');
function nameOf(e) {
  const root = e.getRootNode(), byId = (id) => (root.getElementById ? root.getElementById(id) : e.ownerDocument.getElementById(id));
  let n = e.getAttribute('aria-label');
  if (!n && e.getAttribute('aria-labelledby')) n = e.getAttribute('aria-labelledby').split(/\s+/).map((id) => { const l = byId(id); return l ? l.innerText || l.textContent : ''; }).join(' ');
  if (!n && e.labels && e.labels.length) n = [...e.labels].map((l) => l.innerText || l.textContent).join(' ');
  const field = /^(INPUT|TEXTAREA|SELECT)$/.test(e.tagName);
  if (!n && field) n = e.placeholder || e.title || (/^(submit|button|reset)$/.test(e.type) ? e.value : '') || e.getAttribute('aria-placeholder') || e.name;
  if (!n && !field) n = (e.isContentEditable ? '' : e.innerText) || e.title || e.getAttribute('aria-placeholder') || [...e.querySelectorAll('img[alt],[title]')].map((i) => i.getAttribute('alt') || i.getAttribute('title')).join(' ');
  return clean(n).slice(0, 100);
}
document.querySelectorAll('[data-valerie-ref]').forEach((e) => e.removeAttribute('data-valerie-ref'));
for (const r of roots()) if (r !== document) r.querySelectorAll('[data-valerie-ref]').forEach((e) => e.removeAttribute('data-valerie-ref'));
let n = 0, scanned = 0, capped = false;
const out = [], closedFrames = [];
// Depth first in page order, into shadow roots and same-origin frames (ox, oy: where that frame sits on screen).
function visit(parent, ox, oy, inFrame) {
  for (const e of parent.children) {
    if (capped) return;
    if (++scanned > 40000) { capped = true; return; }
    const s = getComputedStyle(e);
    if (s.display === 'none') continue;
    const editor = e.isContentEditable && !(e.parentElement && e.parentElement.isContentEditable);
    const pointer = s.cursor === 'pointer' && !(e.parentElement && getComputedStyle(e.parentElement).cursor === 'pointer');
    const clicky = e.matches(CLICKY) || editor;
    if ((clicky || pointer) && s.visibility !== 'hidden') add(e, s, ox, oy, inFrame, editor, clicky);
    if (e.isContentEditable && !editor) continue;
    if (e.shadowRoot) visit(e.shadowRoot, ox, oy, inFrame);
    if (e.tagName === 'IFRAME' || e.tagName === 'FRAME') {
      let d = null; try { d = e.contentDocument; } catch (err) {}
      const fr = e.getBoundingClientRect();
      if (d && d.body) visit(d.body, ox + fr.left + e.clientLeft, oy + fr.top + e.clientTop, true);
      else if (fr.width > 60 && fr.height > 40 && e.src && /^https?:/.test(e.src)) closedFrames.push(e.src);
    }
    visit(e, ox, oy, inFrame);
  }
}
function add(e, s, ox, oy, inFrame, editor, clicky) {
  const r = e.getBoundingClientRect();
  if (r.width < 1 || r.height < 1) return;
  // Not drawn at all, e.g. inside a closed <details>.
  if (e.checkVisibility && !e.checkVisibility({ visibilityProperty: true, checkVisibilityCSS: true })) return;
  // A page's own 1px helper fields, hidden from everyone; real checkboxes hidden behind a styled label stay.
  if (r.width <= 1 && r.height <= 1 && !/^(checkbox|radio)$/.test(e.type) && (e.getAttribute('aria-hidden') === 'true' || e.tabIndex < 0)) return;
  // A pointer cursor alone counts only for something with words or a picture; else it's decoration.
  if (!clicky && !(e.innerText || '').trim() && !e.querySelector('svg,img') && !e.getAttribute('aria-label')) return;
  const top = { x: ox + r.left, y: oy + r.top };
  const inView = top.y + r.height > 0 && top.y < innerHeight && top.x + r.width > 0 && top.x < innerWidth;
  if (A.only_in_view && !inView) return;
  if (n >= 500) { capped = true; return; }
  const ref = String(++n);
  e.setAttribute('data-valerie-ref', ref);
  const o = { ref, tag: e.tagName.toLowerCase(), label: nameOf(e) };
  const role = e.getAttribute('role');
  if (role) o.role = role;
  if (e.type && e.tagName !== 'BUTTON' && e.tagName !== 'SELECT') o.type = e.type;
  if (editor) o.editor = true;
  if (e.tagName === 'SELECT') { o.value = e.multiple ? [...e.selectedOptions].map((x) => clean(x.text)) : clean(e.selectedOptions[0] && e.selectedOptions[0].text); o.options = [...e.options].slice(0, 30).map((x) => clean(x.text)); }
  else if (/^(checkbox|radio)$/.test(e.type)) o.checked = e.checked;
  else if (e.type === 'password') { if (e.value) o.value = '(filled)'; }
  else if (/^(INPUT|TEXTAREA)$/.test(e.tagName)) { if (e.value) o.value = e.value.slice(0, 100); }
  else if (editor) o.value = clean(e.innerText).slice(0, 100);
  for (const [k, attr] of [['checked', 'aria-checked'], ['expanded', 'aria-expanded'], ['selected', 'aria-selected'], ['pressed', 'aria-pressed']]) {
    const v = e.getAttribute(attr);
    if (v === 'true' || v === 'false') o[k] = v === 'true';
  }
  // Short: same-site links as a path, and no 2,000-character ad redirects (click by ref; read_page links has them whole).
  if (e.href) {
    let h = String(e.href);
    try { const u = new URL(h); if (u.origin === location.origin) h = u.pathname + u.search + u.hash; } catch (err) {}
    o.href = h.length > 150 ? h.slice(0, 150) + '…' : h;
  }
  if (e.disabled || e.getAttribute('aria-disabled') === 'true') o.disabled = true;
  if (e.readOnly === true && /^(INPUT|TEXTAREA)$/.test(e.tagName)) o.readonly = true;
  if (inFrame) o.in_frame = true;
  if (!inView) o.offscreen = true;
  else {
    const c = coverAt(e).cover;
    if (c && !edgeBar(c)) o.covered = coverName(c);
  }
  out.push(o);
}
visit(document.body || document.documentElement, 0, 0, false);
const result = { url: location.href, title: document.title, elements: out };
if (out.some((o) => o.covered)) result.note = 'covered says what sits on top of an element (a dialog, menu, banner or another link): deal with that first';
// Another site's frame (an embedded editor, a payment form, a video) can't be reached from here: open its address instead.
if (closedFrames.length) result.other_site_frames = [...new Set(closedFrames)].slice(0, 10);
if (capped) {
  result.truncated = true;
  result.note = (result.note ? result.note + '. ' : '') + 'There are more elements than this list holds: snapshot with only_in_view: true for what\'s on screen, or scroll and snapshot again';
}
return result;
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
const SKIP = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'SVG', 'CANVAS', 'OBJECT', 'EMBED', 'HEAD', 'BUTTON', 'SELECT', 'INPUT', 'TEXTAREA', 'DIALOG', 'VALERIE-UI']);
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
// A <slot> draws no box of its own; what's put in it is checked by itself.
const hidden = (e) => e.tagName !== 'SLOT' && (e.hidden || e.getAttribute('aria-hidden') === 'true' || (e.checkVisibility ? !e.checkVisibility() && getComputedStyle(e).display !== 'contents' : getComputedStyle(e).display === 'none'));
const skip = (e) => SKIP.has(e.tagName) || hidden(e) || (A.main_only !== false && e !== root && (CHROME.has(e.tagName) && !e.closest('article') || CHROME_ROLES.test(e.getAttribute('role') || '')));
const inline = (s) => s.replace(/\s*\n\s*/g, ' ').trim();
// An element with an open shadow root shows that instead of its children (they appear where its <slot>s are).
function kids(e, ctx) { let out = ''; for (const c of (e.shadowRoot || e).childNodes) out += md(c, ctx); return out; }
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
    case 'SLOT': {
      const put = n.assignedNodes({ flatten: true });
      if (!put.length) return kids(n, ctx);
      let out = ''; for (const c of put) out += md(c, ctx); return out;
    }
    case 'IFRAME': case 'FRAME': {
      let d = null; try { d = n.contentDocument; } catch (err) {}
      return d && d.body ? '\n\n' + kids(d.body, ctx) + '\n\n' : '';
    }
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
                let out = crate::platform::command("curl")
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
    // Raw files open as downloads, so the browser would only wait for nothing.
    if looks_like_a_file(url) {
        if let Some(mut page) = plain_file(url, if focus_on.is_empty() { max } else { 2_000_000 }) {
            keep_focus(&mut page, &focus_on, max);
            return Ok(page);
        }
    }
    let read = in_web_tab(proxy, json!({ "url": as_address(url) }), |tab| {
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
    .map_err(|e| if e.contains("timed out") { format!("{url} didn't finish loading in time: it may be down or very slow, or this computer is busy. Try once more before giving up on it.") } else { e });
    // A browser downloads what it can't show (raw .md, .ps1 and .json files) instead of opening it.
    let mut page = match read {
        Ok(p) if p["content"].is_string() => p,
        other => plain_file(url, if focus_on.is_empty() { max } else { 2_000_000 }).map_or(other, Ok)?,
    };
    keep_focus(&mut page, &focus_on, max);
    Ok(page)
}

fn keep_focus(page: &mut Value, focus_on: &str, max: usize) {
    if !focus_on.is_empty() {
        let f = focus(page["content"].as_str().unwrap_or(""), focus_on, max.min(40_000));
        page["content"] = f["content"].clone();
        page["truncated"] = f["truncated"].clone();
        page["focus"] = json!({ "query": focus_on, "sections_matched": f["matched"], "outline": f["outline"] });
    }
}

fn looks_like_a_file(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let ext = path.rsplit('/').next().and_then(|name| name.rsplit_once('.')).map_or("", |(_, e)| e).to_ascii_lowercase();
    url.starts_with("https://raw.githubusercontent.com/") || ["ps1", "sh", "bash", "py", "rs", "toml", "yaml", "yml", "json", "txt", "csv"].contains(&ext.as_str())
}

/// A text file at `url`, fetched without the browser (so without the user's logins).
fn plain_file(url: &str, max: usize) -> Option<Value> {
    let out = crate::platform::command("curl")
        .args(["-fsSL", "--max-time", "20", "--max-filesize", "20000000", "--proto", "=https,http", "--", url])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8(out.stdout).ok().filter(|t| !t.trim().is_empty() && !t.contains('\0'))?;
    let truncated = text.chars().count() > max;
    Some(json!({ "url": url, "content": text.chars().take(max).collect::<String>(), "truncated": truncated }))
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
        "report_issue" => {
            let config = crate::config::Config::load().unwrap_or_else(|_| crate::config::Config::defaults());
            if !config.triage.file_issues {
                return Err("Sending reports is off (the triage.file_issues setting). Show the user the text to post themselves.".into());
            }
            let notify = |s: crate::github::Step| {
                if let crate::github::Step::Code { code, url } = s {
                    let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(code.clone()));
                    let _ = proxy.send_event(UserEvent::Toast(format!("Paste the code {code} on GitHub (it's on your clipboard) to sign in")));
                    let _ = call(proxy, "open_tab", json!({ "url": url }));
                }
            };
            let sent = crate::github::sign_in(&config.sync.github_client_id, &notify).and_then(|token| {
                crate::triage::file_issue(&token, &config.triage.repo, a["title"].as_str().unwrap_or(""), a["body"].as_str().unwrap_or(""), a["issue"].as_u64())
            });
            match sent {
                Ok(url) => {
                    crate::log::event("triage", format!("bug report sent: {url}"));
                    Ok(json!({ "url": url }))
                }
                Err(e) => {
                    crate::log::line("triage", format!("couldn't send the bug report: {e:#}"));
                    Err(format!("Couldn't send it: {e:#}. Show the user the text to post themselves."))
                }
            }
        }
        "screenshot_ui" | "get_settings" | "list_pages" | "list_layouts" | "check_panel" => call(proxy, name, a),
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
                return Err("the network log is off (the mcp.network_log setting is never); set it to when_asked and reload the tab".into());
            }
            let rows = v["requests"].as_array().cloned().unwrap_or_default();
            let failed = rows.iter().filter(|r| r["error"].is_string() || r["status"].as_u64().is_some_and(|s| s >= 400)).count();
            let mut out = json!({ "requests": rows, "shown": rows.len(), "failed": failed });
            if v["started_now"] == Value::Bool(true) {
                out["note"] = json!("Recording started now: earlier requests are listed without status or method. Reload the tab (or repeat the action) and read again for full detail; it keeps recording through reloads.");
            } else if a["bodies"].as_bool() == Some(true) && v["had_bodies"] != Value::Bool(true) {
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
            let js = with_args(&json!({ "max": a["max_chars"].as_u64().unwrap_or(20000), "html": format == "html" }), &format!("{FIND_JS}\n\
                const body = A.html ? document.documentElement.outerHTML : pageText(); \
                return {{ url: location.href, title: document.title, truncated: body.length > A.max, content: body.slice(0, A.max) }};"));
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
            eval(proxy, tab, &with_args(&a, &format!("{FINISH_ANIMATIONS_JS}\n{FIND_JS}\n{SNAPSHOT_JS}")))
        }
        "click" => act(proxy, &a, &with_args(&a, &format!("{END_SCRIPT_GUARDS_JS}\n{FIND_JS}\n{POINTER_JS}\nreturn clickOn(find());"))),
        "type" => act(proxy, &a, &with_args(&a, &format!("{END_SCRIPT_GUARDS_JS}\n{FIND_JS}\n{KEYS_JS}\n{TYPE_JS}"))),
        "hover" => eval(proxy, tab, &with_args(&a, &format!("{FIND_JS}\n{POINTER_JS}\nreturn hoverOn(find());"))),
        "drag" => {
            if a["to_ref"].is_null() && a["to_selector"].is_null() {
                return Err("say where to drag it: to_ref (from snapshot) or to_selector".into());
            }
            let to = "const to = A.to_ref ? deepQuery('[data-valerie-ref=\"' + CSS.escape(String(A.to_ref)) + '\"]') : deepQuery(A.to_selector); \
                if (!to) throw new Error(A.to_ref ? 'no element with ref ' + A.to_ref + ' (take a new snapshot)' : 'no element matches ' + A.to_selector);";
            act(proxy, &a, &with_args(&a, &format!("{END_SCRIPT_GUARDS_JS}\n{FIND_JS}\n{POINTER_JS}\nconst from = find();\n{to}\nreturn dragOn(from, to);")))
        }
        "upload_file" => {
            let mut files = Vec::new();
            let mut total = 0;
            for path in a["paths"].as_array().map(Vec::as_slice).unwrap_or_default().iter().filter_map(Value::as_str) {
                let bytes = std::fs::read(path).map_err(|e| format!("can't read {path}: {e}"))?;
                total += bytes.len();
                if total > 20 << 20 {
                    return Err("those files are over 20 MB in all".into());
                }
                let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                files.push(json!({ "name": name, "data": base64(&bytes), "type": file_type(&name) }));
            }
            if files.is_empty() {
                return Err("paths lists no files".into());
            }
            let args = json!({ "ref": a["ref"], "selector": a["selector"], "files": files });
            let js = with_args(&args, &format!("{FIND_JS}\nconst e = find(); \
                if (e.tagName !== 'INPUT' || e.type !== 'file') throw new Error('that is not a file input: ' + describe(e)); \
                const W = e.ownerDocument.defaultView, list = new W.DataTransfer(); \
                for (const f of A.files) list.items.add(new W.File([Uint8Array.from(atob(f.data), (c) => c.charCodeAt(0))], f.name, {{ type: f.type }})); \
                e.files = list.files; e.dispatchEvent(new W.Event('input', {{bubbles: true}})); e.dispatchEvent(new W.Event('change', {{bubbles: true}})); \
                return 'chose ' + e.files.length + ' file(s) for ' + describe(e);"));
            eval(proxy, tab, &js)
        }
        "press_key" => act(proxy, &a, &with_args(&a, &format!("{END_SCRIPT_GUARDS_JS}\n{FIND_JS}\n{KEYS_JS}\n{PRESS_KEY_JS}"))),
        "scroll" => eval(proxy, tab, &with_args(&a, &format!("{FIND_JS}\n{SCROLL_JS}"))),
        "wait_for" => {
            let tab = wait_ready(proxy, tab)?;
            let timeout = Duration::from_millis(a["timeout_ms"].as_u64().unwrap_or(10_000));
            let js = with_args(&a, &format!("{FIND_JS}\nif (A.selector && !deepQuery(A.selector)) return false; \
                if (A.text && !pageText().includes(A.text)) return false; return true;"));
            let start = Instant::now();
            loop {
                if eval(proxy, tab, &js)? == Value::Bool(true) {
                    return Ok(json!("ready"));
                }
                if start.elapsed() > timeout {
                    let what = a["selector"].as_str().or(a["text"].as_str()).unwrap_or("the page");
                    return Err(format!("timed out after {}ms waiting for {what}", timeout.as_millis()));
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
            watch_dialogs(proxy, tab, &a);
            let out = run_script(a["script"].as_str().unwrap_or(""), tab, proxy);
            let notes = dialogs_seen(proxy, tab, &a);
            match out {
                // The script's own value stays as it was, beside what the page asked.
                Ok(v) if notes.is_empty() => Ok(v),
                Ok(v) => Ok(json!({ "result": v, "dialogs": notes })),
                Err(e) if notes.is_empty() => Err(e),
                Err(e) => Err(format!("{e}\n{}", notes.join("\n"))),
            }
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
    fn raw_files_are_told_from_pages() {
        for url in ["https://raw.githubusercontent.com/a/b/main/README.md", "https://x.com/install.ps1?v=1", "https://x.com/a.json"] {
            assert!(super::looks_like_a_file(url), "{url}");
        }
        for url in ["https://docs.rs/tokio/latest/tokio/", "https://x.com/a.html", "https://arxiv.org/abs/2406.09895"] {
            assert!(!super::looks_like_a_file(url), "{url}");
        }
    }

    #[test]
    fn a_plain_file_is_only_fetched_over_the_web() {
        assert!(super::plain_file("file:///etc/hosts", 100).is_none());
    }

    #[test]
    fn the_bridge_can_answer_what_apps_ask_first_without_the_browser() {
        use super::{catalog_answer, Caller};
        use serde_json::json;
        let who = Caller { access: "full".into(), chat: String::new() };
        assert!(catalog_answer("initialize", &json!({}), &who).unwrap()["instructions"].as_str().unwrap().contains("web_search"));
        let tools = catalog_answer("tools/list", &json!({}), &who).unwrap();
        let tools = tools["tools"].as_array().unwrap();
        let always = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap()["_meta"]["anthropic/alwaysLoad"] == json!(true);
        assert!(always("web_search") && always("fetch_page") && !always("click"));
        assert!(tools.iter().any(|t| t["name"] == "drag" && t["annotations"]["readOnlyHint"] == json!(false)), "drag changes the page");
        assert!(catalog_answer("tools/call", &json!({}), &who).is_none());
    }

    #[test]
    fn paying_words_are_whole_words_and_err_towards_asking() {
        let config = crate::config::Config::defaults();
        let words = &config.mcp.paying_words;
        for caught in ["Pay now", "PLACE ORDER", "Proceed to Checkout", "check out", "Buy", "payNow", "btn-checkout", "/cart/view", "Add to Bag",
            "Subscribe", "Upgrade to Pro", "Get it for $9.99", "Start free trial", "Confirm purchase", "Donate", "Jetzt kaufen", "購入する", "Payment"] {
            assert!(super::paying_word(caught, words).is_some(), "should ask about {caught:?}");
        }
        for fine in ["Display settings", "Save", "Payload", "Next", "Sign in", "Repay nothing", "Search", "Open menu", "Border"] {
            assert_eq!(super::paying_word(fine, words), None, "{fine:?}");
        }
    }

    #[test]
    fn a_guarded_script_keeps_its_words_and_stays_a_function_body() {
        let js = super::guarded_script("return 1 // a comment", &["pay".to_string()]);
        assert!(js.contains(r#"const WORDS = ["pay"];"#), "{js}");
        assert!(js.contains("function () {\nreturn 1 // a comment\n});"), "a trailing comment mustn't swallow the wrapper");
    }

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
        // A refusal (an older Valerie that doesn't know valerie://show) means this one should open by itself.
        matches!(post_local(port, &token, &req.to_string()), Ok((200, answer)) if !answer.contains("\"isError\":true"))
    })
}

/// `valerie mcp-stdio`: bridge for MCP clients that only speak stdio.
pub fn stdio_bridge(mcp: &crate::config::McpConfig, access: Option<String>, chat: Option<String>) -> anyhow::Result<()> {
    // Some CLIs start MCP servers with a scrubbed environment, so the access level also arrives as arguments.
    if let Some(a) = access {
        std::env::set_var("VALERIE_ACCESS", a);
    }
    if let Some(c) = chat {
        std::env::set_var("VALERIE_CHAT", c);
    }
    set_web_tools(mcp.web_tools);
    set_ask_user(mcp.ask_user);
    let who = Caller { access: "full".into(), chat: std::env::var("VALERIE_CHAT").unwrap_or_default() };
    let token = std::sync::Arc::new(load_or_create_token());
    let (port, start) = (mcp.port, mcp.start_when_needed);
    let stdout = std::sync::Arc::new(std::sync::Mutex::new(std::io::stdout()));
    let reply = |stdout: &std::sync::Mutex<std::io::Stdout>, text: &str| {
        let mut out = stdout.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(out, "{}", text.trim()).and_then(|()| out.flush());
    };
    let mut calls = Vec::new();
    let stdin = std::io::stdin();
    let mut line = String::new();
    while { line.clear(); stdin.read_line(&mut line)? > 0 } {
        let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else { continue };
        let Some(id) = msg.get("id").cloned() else { continue }; // notifications need no answer
        let empty = json!({});
        if let Some(result) = catalog_answer(msg["method"].as_str().unwrap_or(""), msg.get("params").unwrap_or(&empty), &who) {
            reply(&stdout, &json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string());
            continue;
        }
        // One at a time would leave every other call waiting behind a slow page or an ask_user.
        let (stdout, token, body) = (stdout.clone(), token.clone(), line.trim().to_string());
        calls.push(std::thread::spawn(move || {
            let answer = bridge_call(port, &token, &body, start)
                .unwrap_or_else(|why| json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": why } }).to_string());
            reply(&stdout, &answer);
        }));
    }
    for call in calls {
        let _ = call.join();
    }
    Ok(())
}

/// Send one call to the browser, starting it first if it's closed (mcp.start_when_needed).
fn bridge_call(port: u16, token: &str, body: &str, start: bool) -> Result<String, String> {
    let refused = |r: &std::io::Result<(u16, String)>| matches!(r, Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused);
    let mut sent = post_local(port, token, body);
    if refused(&sent) && start && start_browser() {
        for _ in 0..60 {
            std::thread::sleep(Duration::from_millis(500));
            sent = post_local(port, token, body);
            if !refused(&sent) {
                break;
            }
        }
    }
    match sent {
        Ok((200, answer)) => Ok(answer),
        Ok((status, _)) => Err(format!("Valerie answered with an error (HTTP {status})")),
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            Err("Valerie is not running and didn't start. Ask the user to open it.".to_string())
        }
        Err(e) => Err(format!("Valerie didn't answer: {e}. It may be busy; try again.")),
    }
}

/// Start the browser on its own, so it outlives the agent that needed it. Calls at the same moment start it once.
fn start_browser() -> bool {
    // Parallel calls start it once; a browser that has since quit (mcp.quit_when_unused_minutes) is started again.
    static TRIED: std::sync::Mutex<Option<(Instant, bool)>> = std::sync::Mutex::new(None);
    let mut tried = TRIED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, started)) = *tried {
        if at.elapsed() < Duration::from_secs(30) {
            return started;
        }
    }
    let started = (|| {
        let Ok(exe) = std::env::current_exe() else { return false };
        let mut browser = crate::platform::command(exe);
        browser.env(STARTED_FOR_AGENT, "1");
        browser.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut browser, 0);
        let started = browser.spawn();
        crate::log::line("mcp", match &started {
            Ok(_) => "started the browser for an agent".to_string(),
            Err(e) => format!("couldn't start the browser for an agent: {e}"),
        });
        started.is_ok()
    })();
    *tried = Some((Instant::now(), started));
    started
}
