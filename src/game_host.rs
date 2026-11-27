//! Playing the casino's games with friends (pages.game_invites). The host's casino tab runs every game; this is a small
//! web server behind a Cloudflare quick tunnel that hands friends' browsers the pictures of their seat that the tab
//! draws, and hands the tab their clicks and keys. Nothing else on this computer is reachable through it.

use crate::app::UserEvent;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Read};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tao::event_loop::EventLoopProxy;

const MAX_GUESTS: usize = 12;
/// A friend whose page hasn't asked for a picture in this long has left.
const GUEST_GONE: Duration = Duration::from_secs(30);
/// The casino tab sends pictures (or just checks in) at least every 15 seconds; this long without, it's closed.
const HOST_GONE: Duration = Duration::from_secs(90);

struct Guest {
    seen: Instant,
    /// The latest picture of this friend's seat: (version, {png, msg}).
    frame: (u64, Value),
    inputs: Vec<Instant>,
}

struct Room {
    session: u64,
    token: String,
    url: Option<String>,
    guests: HashMap<String, Guest>,
    version: u64,
    host_seen: Instant,
    server: Arc<tiny_http::Server>,
    tunnel: Option<std::process::Child>,
}

static ROOM: Mutex<Option<Room>> = Mutex::new(None);
static NEW_FRAME: Condvar = Condvar::new();
static SESSIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn tell(proxy: &EventLoopProxy<UserEvent>, v: Value) {
    let _ = proxy.send_event(UserEvent::GameHost(v));
}

/// Whether games are being hosted, and the link once the tunnel is up.
pub fn status() -> Value {
    match ROOM.lock().unwrap().as_ref() {
        Some(r) => json!({ "state": if r.url.is_some() { "ready" } else { "starting" }, "url": r.url.as_ref().map(|u| link(u, &r.token)), "guests": r.guests.keys().collect::<Vec<_>>() }),
        None => json!({ "state": "off" }),
    }
}

fn link(base: &str, token: &str) -> String {
    format!("{base}/g/{token}")
}

/// Start hosting: the server now, the tunnel in the background (`{ready: link}` or `{error}` arrives as a GameHost event).
pub fn start(proxy: EventLoopProxy<UserEvent>) -> Value {
    let mut room = ROOM.lock().unwrap();
    if room.is_some() {
        drop(room);
        return status();
    }
    let server = match tiny_http::Server::http(("127.0.0.1", 0)) {
        Ok(s) => Arc::new(s),
        Err(e) => return json!({ "error": format!("couldn't start the game server: {e}") }),
    };
    let Some(port) = server.server_addr().to_ip().map(|a| a.port()) else { return json!({ "error": "the game server has no port" }) };
    let session = SESSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let token = crate::mcp::random_hex()[..24].to_string();
    *room = Some(Room { session, token: token.clone(), url: None, guests: HashMap::new(), version: 0, host_seen: Instant::now(), server: server.clone(), tunnel: None });
    drop(room);
    crate::log::line("games", format!("hosting: game server on 127.0.0.1:{port}, opening a tunnel"));

    let p = proxy.clone();
    std::thread::spawn(move || {
        for rq in server.incoming_requests() {
            let p = p.clone();
            std::thread::spawn(move || handle(rq, &p));
        }
    });
    let p = proxy.clone();
    std::thread::spawn(move || match open_tunnel(port) {
        Ok((child, base)) => {
            let mut room = ROOM.lock().unwrap();
            match room.as_mut().filter(|r| r.session == session) {
                Some(r) => {
                    r.url = Some(base.clone());
                    r.tunnel = Some(child);
                    drop(room);
                    crate::log::line("games", format!("hosting: tunnel ready at {base}"));
                    tell(&p, json!({ "ready": link(&base, &token) }));
                }
                // Stopped while the tunnel was opening.
                None => {
                    let mut child = child;
                    let _ = child.kill();
                }
            }
        }
        Err(e) => {
            crate::log::line("games", format!("hosting: {e}"));
            stop_session(session);
            tell(&p, json!({ "error": e }));
        }
    });
    // Friends who closed their page leave; a closed casino tab ends hosting.
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(5));
        let mut room = ROOM.lock().unwrap();
        let Some(r) = room.as_mut().filter(|r| r.session == session) else { return };
        if r.host_seen.elapsed() > HOST_GONE {
            drop(room);
            stop_session(session);
            return tell(&proxy, json!({ "stopped": true }));
        }
        let gone: Vec<String> = r.guests.iter().filter(|(_, g)| g.seen.elapsed() > GUEST_GONE).map(|(id, _)| id.clone()).collect();
        for id in gone {
            r.guests.remove(&id);
            tell(&proxy, json!({ "leave": id }));
        }
    });
    json!({ "state": "starting" })
}

pub fn stop() {
    let session = ROOM.lock().unwrap().as_ref().map(|r| r.session);
    if let Some(s) = session {
        stop_session(s);
    }
}

fn stop_session(session: u64) {
    let mut room = ROOM.lock().unwrap();
    if room.as_ref().is_some_and(|r| r.session == session) {
        let mut r = room.take().unwrap();
        r.server.unblock();
        if let Some(mut c) = r.tunnel.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        crate::log::line("games", "hosting stopped");
    }
    NEW_FRAME.notify_all();
}

/// The casino tab's latest pictures: `{frames: {guest id: {png, msg}}}`. Also its "still here".
pub fn frames(args: &Value) -> Value {
    let mut room = ROOM.lock().unwrap();
    let Some(r) = room.as_mut() else { return json!({ "state": "off" }) };
    r.host_seen = Instant::now();
    if let Some(frames) = args["frames"].as_object() {
        for (id, f) in frames {
            if let Some(g) = r.guests.get_mut(id) {
                r.version += 1;
                g.frame = (r.version, f.clone());
            }
        }
        NEW_FRAME.notify_all();
    }
    json!({ "state": "ok" })
}

fn reply(rq: tiny_http::Request, code: u16, kind: &str, body: String) {
    let mut r = tiny_http::Response::from_string(body).with_status_code(code);
    for (k, v) in [("Content-Type", kind), ("Cache-Control", "no-store"), ("X-Content-Type-Options", "nosniff"), ("Referrer-Policy", "no-referrer")] {
        if let Ok(h) = tiny_http::Header::from_bytes(k, v) {
            r = r.with_header(h);
        }
    }
    let _ = rq.respond(r);
}

fn json_reply(rq: tiny_http::Request, code: u16, v: Value) {
    reply(rq, code, "application/json", v.to_string());
}

/// Friends' requests: GET /g/<token> (the page), POST /g/<token>/join, GET /g/<token>/frame?id=&since=,
/// POST /g/<token>/input?id=, POST /g/<token>/leave?id=.
fn handle(mut rq: tiny_http::Request, proxy: &EventLoopProxy<UserEvent>) {
    let full = rq.url().to_string();
    let (path, query) = full.split_once('?').unwrap_or((&full, ""));
    let param = |name: &str| query.split('&').find_map(|kv| kv.strip_prefix(name).and_then(|v| v.strip_prefix('='))).unwrap_or("").to_string();
    let mut parts = path.trim_start_matches('/').splitn(3, '/');
    let (g, token, action) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let ok = g == "g" && ROOM.lock().unwrap().as_ref().is_some_and(|r| crate::mcp::same_secret(token, &r.token));
    if !ok {
        return reply(rq, 404, "text/plain; charset=utf-8", "This game has ended, or the link is wrong.".into());
    }
    let mut body = String::new();
    let _ = rq.as_reader().take(2048).read_to_string(&mut body);
    let id = param("id");
    match (rq.method(), action) {
        (tiny_http::Method::Get, "") => reply(rq, 200, "text/html; charset=utf-8", include_str!("../ui/casino-guest.html").into()),
        (tiny_http::Method::Post, "join") => {
            let mut room = ROOM.lock().unwrap();
            let Some(r) = room.as_mut() else { return json_reply(rq, 404, json!({})) };
            if r.guests.len() >= MAX_GUESTS {
                return json_reply(rq, 429, json!({ "error": "This game is full." }));
            }
            let id = crate::mcp::random_hex()[..16].to_string();
            r.guests.insert(id.clone(), Guest { seen: Instant::now(), frame: (0, Value::Null), inputs: vec![] });
            drop(room);
            let night = serde_json::from_str::<Value>(&body).ok().and_then(|v| v["night"].as_bool()).unwrap_or(false);
            tell(proxy, json!({ "join": id, "night": night }));
            json_reply(rq, 200, json!({ "id": id }))
        }
        // Waits up to 20 seconds for a picture newer than `since`.
        (tiny_http::Method::Get, "frame") => {
            let since: u64 = param("since").parse().unwrap_or(0);
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut room = ROOM.lock().unwrap();
            loop {
                let Some(g) = room.as_mut().and_then(|r| r.guests.get_mut(&id)) else { return json_reply(rq, 410, json!({ "gone": true })) };
                g.seen = Instant::now();
                if g.frame.0 > since {
                    let (v, f) = g.frame.clone();
                    drop(room);
                    return json_reply(rq, 200, json!({ "v": v, "png": f["png"], "msg": f["msg"] }));
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    drop(room);
                    return json_reply(rq, 200, json!({ "v": since }));
                }
                room = NEW_FRAME.wait_timeout(room, left).unwrap().0;
            }
        }
        (tiny_http::Method::Post, "input") => {
            let mut room = ROOM.lock().unwrap();
            let Some(g) = room.as_mut().and_then(|r| r.guests.get_mut(&id)) else { return json_reply(rq, 410, json!({ "gone": true })) };
            // At most 20 clicks and keys a second from one friend.
            g.inputs.retain(|t| t.elapsed() < Duration::from_secs(1));
            if g.inputs.len() >= 20 {
                return json_reply(rq, 429, json!({}));
            }
            g.inputs.push(Instant::now());
            g.seen = Instant::now();
            drop(room);
            let Ok(v) = serde_json::from_str::<Value>(&body) else { return json_reply(rq, 400, json!({})) };
            let input = match v["type"].as_str() {
                Some("click") => json!({ "input": id, "type": "click", "x": v["x"].as_f64().unwrap_or(-1.0), "y": v["y"].as_f64().unwrap_or(-1.0) }),
                Some("key") => json!({ "input": id, "type": "key", "k": v["k"].as_str().unwrap_or("").chars().take(12).collect::<String>() }),
                _ => return json_reply(rq, 400, json!({})),
            };
            tell(proxy, input);
            json_reply(rq, 200, json!({}))
        }
        (tiny_http::Method::Post, "leave") => {
            let gone = ROOM.lock().unwrap().as_mut().and_then(|r| r.guests.remove(&id)).is_some();
            if gone {
                tell(proxy, json!({ "leave": id }));
            }
            json_reply(rq, 200, json!({}))
        }
        _ => json_reply(rq, 404, json!({})),
    }
}

/// Runs `cloudflared tunnel --url` for the port and waits for the public address it prints.
fn open_tunnel(port: u16) -> Result<(std::process::Child, String), String> {
    let bin = cloudflared()?;
    // An empty config of its own: cloudflared otherwise reads ~/.cloudflared/config.yaml, whose routes (for someone's
    // own tunnels) leave a quick tunnel answering 404.
    let config = crate::config::data_dir().join("bin").join("quick-tunnel.yml");
    if !config.exists() {
        let _ = std::fs::create_dir_all(config.parent().unwrap_or(&config));
        std::fs::write(&config, "# Valerie's game link: a quick tunnel with no settings of its own.\n").map_err(|e| e.to_string())?;
    }
    let mut child = crate::platform::command(&bin)
        .arg("tunnel")
        .arg("--config")
        .arg(&config)
        .args(["--no-autoupdate", "--url", &format!("http://127.0.0.1:{port}")])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't start cloudflared: {e}"))?;
    let err = child.stderr.take().ok_or("cloudflared has no output")?;
    let (tx, rx) = std::sync::mpsc::channel();
    // The address comes first; it works once a connection to Cloudflare is registered. Keeps reading after that, so
    // cloudflared never blocks on a full pipe.
    std::thread::spawn(move || {
        let mut found = None;
        for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
            if let Some(at) = line.find("https://") {
                let url: String = line[at..].chars().take_while(|c| !c.is_whitespace() && *c != '|').collect();
                if url.ends_with(".trycloudflare.com") && found.is_none() {
                    found = Some(url);
                }
            }
            if line.contains("Registered tunnel connection") {
                if let Some(url) = found.take() {
                    let _ = tx.send(url);
                }
            }
        }
    });
    match rx.recv_timeout(Duration::from_secs(45)) {
        Ok(url) => Ok((child, url)),
        Err(_) => {
            let _ = child.kill();
            Err("the tunnel didn't open (is this computer online?)".into())
        }
    }
}

/// cloudflared: one Valerie downloaded before, one already installed, or a fresh download from Cloudflare's releases.
fn cloudflared() -> Result<PathBuf, String> {
    let name = if cfg!(windows) { "cloudflared.exe" } else { "cloudflared" };
    let own = crate::config::data_dir().join("bin").join(name);
    if own.exists() {
        return Ok(own);
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    if let Some(found) = dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file()) {
        return Ok(found);
    }
    download(&own)?;
    Ok(own)
}

fn download(to: &std::path::Path) -> Result<(), String> {
    let arm = cfg!(target_arch = "aarch64");
    let asset = if cfg!(target_os = "macos") {
        if arm { "cloudflared-darwin-arm64.tgz" } else { "cloudflared-darwin-amd64.tgz" }
    } else if cfg!(windows) {
        "cloudflared-windows-amd64.exe"
    } else if arm {
        "cloudflared-linux-arm64"
    } else {
        "cloudflared-linux-amd64"
    };
    let dir = to.parent().ok_or("no folder for cloudflared")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let part = dir.join(format!("{asset}.part"));
    crate::log::line("games", format!("downloading {asset} for the game tunnel"));
    let url = format!("https://github.com/cloudflare/cloudflared/releases/latest/download/{asset}");
    let ok = crate::platform::command("curl").args(["-fsSL", "--max-time", "300", "--proto", "=https", "-o"]).arg(&part).arg(&url).status().is_ok_and(|s| s.success());
    if !ok {
        let _ = std::fs::remove_file(&part);
        return Err("couldn't download cloudflared, which the game link needs".into());
    }
    if asset.ends_with(".tgz") {
        let ok = crate::platform::command("tar").arg("-xzf").arg(&part).arg("-C").arg(dir).status().is_ok_and(|s| s.success());
        let _ = std::fs::remove_file(&part);
        if !ok {
            return Err("couldn't unpack cloudflared".into());
        }
    } else {
        std::fs::rename(&part, to).map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755));
    }
    let works = crate::platform::command(to).arg("--version").output().is_ok_and(|o| o.status.success());
    if !works {
        let _ = std::fs::remove_file(to);
        return Err("the downloaded cloudflared doesn't run".into());
    }
    Ok(())
}
