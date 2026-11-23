//! Chats from the user's other AI apps (Claude Code, Codex, OpenCode, T3 Code), read in place and never copied:
//! the newest few are listed, the rest are found by searching, and opening one continues it in Valerie through the
//! same app. Each app's files are its own private format, so anything that doesn't parse is skipped, never fatal.
use crate::ai::Msg;
use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The apps whose chats Valerie can list, and their names.
pub const SOURCES: &[(&str, &str)] = &[("claude", "Claude Code"), ("codex", "Codex"), ("opencode", "OpenCode"), ("t3", "T3 Code")];

pub fn source_title(id: &str) -> &'static str {
    SOURCES.iter().find(|s| s.0 == id).map(|s| s.1).unwrap_or("")
}

/// One chat in another app.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Found {
    pub source: &'static str,
    pub id: String,
    pub title: String,
    pub folder: String,
    /// Last change, in seconds.
    pub updated: u64,
    /// The words around a search match.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub snippet: String,
}

/// Bumped by each new search, so an older one still running stops.
pub static SEARCH: AtomicU64 = AtomicU64::new(0);

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn claude_projects() -> PathBuf {
    env_dir("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home().join(".claude")).join("projects")
}

fn codex_home() -> PathBuf {
    env_dir("CODEX_HOME").unwrap_or_else(|| home().join(".codex"))
}

fn opencode_db() -> PathBuf {
    env_dir("XDG_DATA_HOME").unwrap_or_else(|| home().join(".local").join("share")).join("opencode").join("opencode.db")
}

pub fn t3_home() -> PathBuf {
    env_dir("T3CODE_HOME").unwrap_or_else(|| home().join(".t3"))
}

fn t3_db() -> PathBuf {
    t3_home().join("userdata").join("statev2.sqlite")
}

/// Whether this computer has the app's chats at all.
pub fn available(source: &str) -> bool {
    match source {
        "claude" => claude_projects().is_dir(),
        "codex" => codex_home().join("sessions").is_dir(),
        "opencode" => opencode_db().is_file(),
        "t3" => t3_db().is_file(),
        _ => false,
    }
}

/// Folders whose chats aren't the user's own conversations: Valerie's agents (they're already Valerie chats) and
/// throwaway temp folders scripts run agents in.
pub fn skip_dirs() -> Vec<String> {
    let mut v = vec![crate::config::data_dir().to_string_lossy().into_owned(), std::env::temp_dir().to_string_lossy().into_owned()];
    // Valerie's usual data folder too, for a copy running with its own (VALERIE_DATA_DIR).
    if let Some(d) = dirs::data_dir() {
        v.push(d.join("Valerie").to_string_lossy().into_owned());
    }
    if cfg!(unix) {
        v.extend(["/tmp/".into(), "/private/tmp/".into(), "/private/var/folders/".into(), "/var/folders/".into()]);
    }
    v
}

/// A chat in one of these folders, or in a hidden folder in the home folder (~/.local/state/…, ~/.t3/worktrees/…,
/// where scripts and other apps run agents), isn't listed.
fn skipped(folder: &str, skip: &[String]) -> bool {
    let hidden = Path::new(folder).strip_prefix(home()).is_ok_and(|rest| rest.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')));
    hidden || skip.iter().map(|s| s.trim_end_matches(['/', '\\'])).any(|s| !s.is_empty() && (folder == s || folder.starts_with(&format!("{s}/")) || folder.starts_with(&format!("{s}\\"))))
}

/// The newest `limit` chats of one app.
pub fn recent(source: &str, limit: usize, skip: &[String]) -> Vec<Found> {
    let r = match source {
        "claude" => Ok(claude_recent(limit, skip)),
        "codex" => Ok(codex_recent(limit, skip)),
        "opencode" => opencode_list(limit, skip),
        "t3" => t3_list(limit),
        _ => Ok(vec![]),
    };
    r.unwrap_or_else(|e| {
        crate::log::line("other chats", format!("listing {source} chats: {e:#}"));
        vec![]
    })
}

/// Chats of one app whose messages mention `query` (any case), newest first. Stops early when a newer search starts.
pub fn search(source: &str, query: &str, limit: usize, skip: &[String], generation: u64) -> Vec<Found> {
    let q = query.trim();
    if q.is_empty() {
        return vec![];
    }
    let r = match source {
        "claude" => Ok(claude_search(q, limit, skip, generation)),
        "codex" => Ok(codex_search(q, limit, skip, generation)),
        "opencode" => opencode_search(q, limit, skip, generation),
        "t3" => t3_search(q, limit, generation),
        _ => Ok(vec![]),
    };
    r.unwrap_or_else(|e| {
        crate::log::line("other chats", format!("searching {source} chats: {e:#}"));
        vec![]
    })
}

/// One chat with its conversation.
pub fn load(source: &str, id: &str) -> Result<(Found, Vec<Msg>)> {
    match source {
        "claude" => {
            let path = claude_file(id).context("that Claude Code chat isn't on this computer anymore")?;
            claude_read(&path, None).context("couldn't read that Claude Code chat")
        }
        "codex" => {
            let path = codex_files().into_iter().map(|f| f.0).find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().contains(id)));
            codex_read(&path.context("that Codex chat isn't on this computer anymore")?, None).context("couldn't read that Codex chat")
        }
        "opencode" => opencode_load(id),
        "t3" => t3_load(id),
        _ => bail!("unknown app {source}"),
    }
}

// ---------- text helpers ----------

/// Seconds since 1970 from an ISO time like 2026-11-23T07:59:51.204Z (UTC); 0 if it isn't one.
pub fn iso_secs(s: &str) -> u64 {
    let n = |a: usize, b: usize| s.get(a..b).and_then(|x| x.parse::<i64>().ok());
    let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(se)) = (n(0, 4), n(5, 7), n(8, 10), n(11, 13), n(14, 16), n(17, 19)) else { return 0 };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    (days * 86400 + h * 3600 + mi * 60 + se).max(0) as u64
}

/// Words an app adds to a message itself (instructions, command output, reminders), not what the person typed.
fn is_wrapper(t: &str) -> bool {
    let t = t.trim_start();
    t.is_empty() || t.starts_with('<') || t.starts_with("[Request interrupted") || t.starts_with("# AGENTS.md instructions") || t.starts_with("Caveat: The messages below")
}

fn one_line(t: &str, max: usize) -> String {
    let s: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max {
        s
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}

/// The words around the first place `q` (lowercase) is in `text`, or None.
pub fn snippet(text: &str, q: &str) -> Option<String> {
    let at = text.to_ascii_lowercase().find(q)?;
    let mut a = at.saturating_sub(50);
    while !text.is_char_boundary(a) {
        a -= 1;
    }
    let mut b = (at + q.len() + 110).min(text.len());
    while !text.is_char_boundary(b) {
        b += 1;
    }
    let s = one_line(&text[a..b], 400);
    Some(format!("{}{s}{}", if a > 0 { "…" } else { "" }, if b < text.len() { "…" } else { "" }))
}

fn push(msgs: &mut Vec<Msg>, role: &str, text: &str, tools: Vec<String>, at: u64) {
    let text = text.trim();
    if text.is_empty() && tools.is_empty() {
        return;
    }
    // An app writes an answer in pieces (one per block or tool call); Valerie shows each answer as one message.
    if role == "assistant" {
        if let Some(m) = msgs.last_mut().filter(|m| m.role == "assistant") {
            if !text.is_empty() {
                if !m.text.is_empty() {
                    m.text.push_str("\n\n");
                }
                m.text.push_str(text);
            }
            m.tools.extend(tools);
            return;
        }
    }
    msgs.push(Msg { role: role.into(), text: text.into(), tools, at, ..Default::default() });
}

fn file_secs(p: &Path) -> u64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

/// Whether a file holds `q` (lowercase) anywhere, cheaply, before it's read properly.
fn file_mentions(p: &Path, q: &str) -> bool {
    let Ok(mut bytes) = std::fs::read(p) else { return false };
    bytes.make_ascii_lowercase();
    // Raw JSON escapes quotes and line breaks; a plain word search still finds the words.
    String::from_utf8_lossy(&bytes).contains(q)
}

// ---------- Claude Code: ~/.claude/projects/<folder>/<session>.jsonl ----------

fn claude_files() -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    for dir in std::fs::read_dir(claude_projects()).into_iter().flatten().flatten() {
        for f in std::fs::read_dir(dir.path()).into_iter().flatten().flatten() {
            let p = f.path();
            if p.extension().is_some_and(|x| x == "jsonl") {
                let t = f.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
                out.push((p, t));
            }
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

fn claude_file(id: &str) -> Option<PathBuf> {
    if id.is_empty() || id.contains(['/', '\\', '.']) {
        return None;
    }
    std::fs::read_dir(claude_projects()).into_iter().flatten().flatten().map(|d| d.path().join(format!("{id}.jsonl"))).find(|p| p.is_file())
}

fn claude_text(content: &Value, tools: &mut Vec<String>) -> String {
    match content {
        Value::String(s) if !is_wrapper(s) => s.clone(),
        Value::Array(parts) => {
            let mut out = Vec::new();
            for p in parts {
                match p["type"].as_str() {
                    Some("text") => {
                        let t = p["text"].as_str().unwrap_or("");
                        if !is_wrapper(t) {
                            out.push(t.to_string());
                        }
                    }
                    Some("tool_use") => tools.push(p["name"].as_str().unwrap_or("tool").to_string()),
                    _ => {}
                }
            }
            out.join("\n\n")
        }
        _ => String::new(),
    }
}

/// A Claude Code chat: its summary and conversation. With `query`, None unless a message mentions it (and the summary
/// carries the words around it).
fn claude_read(path: &Path, query: Option<&str>) -> Option<(Found, Vec<Msg>)> {
    let f = std::fs::File::open(path).ok()?;
    let id = path.file_stem()?.to_string_lossy().into_owned();
    let (mut title, mut first, mut folder, mut snip) = (String::new(), String::new(), String::new(), String::new());
    let mut msgs = Vec::new();
    let mut driven = false;
    for line in BufReader::new(f).lines().map_while(Result::ok) {
        // Most lines are big attachments (instructions, tool lists): only these are worth parsing.
        let user = line.contains("\"type\":\"user\"");
        let assistant = line.contains("\"type\":\"assistant\"");
        let named = line.contains("\"type\":\"custom-title\"");
        if !(user || assistant || named) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        match v["type"].as_str() {
            Some("custom-title") => title = v["customTitle"].as_str().unwrap_or("").to_string(),
            Some(role @ ("user" | "assistant")) if v["isSidechain"] != true && v["isMeta"] != true => {
                if folder.is_empty() {
                    folder = v["cwd"].as_str().unwrap_or("").to_string();
                    driven = by_another_app(&v);
                }
                let mut tools = Vec::new();
                let text = claude_text(&v["message"]["content"], &mut tools);
                if role == "user" && first.is_empty() && !text.trim().is_empty() {
                    first = text.clone();
                }
                if let (Some(q), true) = (query, snip.is_empty()) {
                    snip = snippet(&text, q).unwrap_or_default();
                }
                push(&mut msgs, role, &text, if role == "assistant" { tools } else { vec![] }, iso_secs(v["timestamp"].as_str().unwrap_or("")));
            }
            _ => {}
        }
    }
    if driven || first.is_empty() || (query.is_some() && snip.is_empty()) {
        return None;
    }
    if title.is_empty() {
        title = one_line(&first, 80);
    }
    Some((Found { source: "claude", id, title, folder, updated: file_secs(path), snippet: snip }, msgs))
}

/// Claude Code run by another app (T3 Code, or a script through its SDK): that app lists the chat itself.
fn driven(entrypoint: &str) -> bool {
    entrypoint.starts_with("sdk-") || entrypoint == "t3-code"
}

fn by_another_app(line: &Value) -> bool {
    line["entrypoint"].as_str().is_some_and(driven)
}

/// A string field's value straight from a raw JSON line (no escapes in it), without parsing the line.
fn raw_field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let at = line.find(&format!("\"{key}\":\""))? + key.len() + 4;
    line[at..].find('"').map(|end| &line[at..at + end])
}

/// A Claude Code chat's summary, read cheaply: the top of the file up to the first message, and the end for its title.
fn claude_summary(path: &Path, skip: &[String]) -> Option<Found> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let (mut folder, mut first) = (String::new(), String::new());
    let mut read = 0usize;
    for line in BufReader::new(&f).lines().map_while(Result::ok) {
        read += line.len();
        if read > 8_000_000 {
            break;
        }
        // Every line says which app started the chat: most are another app's, so they're dropped here, unparsed.
        if raw_field(&line, "entrypoint").is_some_and(driven) {
            return None;
        }
        if !line.contains("\"type\":\"user\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if v["type"] != "user" || v["isSidechain"] == true || v["isMeta"] == true {
            continue;
        }
        if folder.is_empty() {
            folder = v["cwd"].as_str().unwrap_or("").to_string();
            if by_another_app(&v) || skipped(&folder, skip) {
                return None;
            }
        }
        let text = claude_text(&v["message"]["content"], &mut Vec::new());
        if !text.trim().is_empty() {
            first = text;
            break;
        }
    }
    if first.is_empty() {
        return None;
    }
    // Its name (set by the person or Claude Code) is written again and again; the last one is near the end.
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(512 * 1024))).ok()?;
    let mut tail = Vec::new();
    f.read_to_end(&mut tail).ok()?;
    let tail = String::from_utf8_lossy(&tail);
    let title = tail
        .lines()
        .rev()
        .filter(|l| l.contains("\"type\":\"custom-title\""))
        .find_map(|l| serde_json::from_str::<Value>(l).ok()?["customTitle"].as_str().map(str::to_string))
        .unwrap_or_else(|| one_line(&first, 80));
    let id = path.file_stem()?.to_string_lossy().into_owned();
    Some(Found { source: "claude", id, title, folder, updated: file_secs(path), snippet: String::new() })
}

fn claude_recent(limit: usize, skip: &[String]) -> Vec<Found> {
    claude_files().into_iter().filter_map(|(p, _)| claude_summary(&p, skip)).take(limit).collect()
}

fn claude_search(q: &str, limit: usize, skip: &[String], generation: u64) -> Vec<Found> {
    let q = q.to_ascii_lowercase();
    let mut out = Vec::new();
    for (p, _) in claude_files() {
        if out.len() >= limit || SEARCH.load(Ordering::Relaxed) != generation {
            break;
        }
        if file_mentions(&p, &q) {
            if let Some((f, _)) = claude_read(&p, Some(&q)).filter(|r| !skipped(&r.0.folder, skip)) {
                out.push(f);
            }
        }
    }
    out
}

// ---------- Codex: ~/.codex/sessions/<year>/<month>/<day>/rollout-<time>-<id>.jsonl ----------

fn codex_files() -> Vec<(PathBuf, u64)> {
    fn walk(d: &Path, out: &mut Vec<(PathBuf, u64)>, depth: u8) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() && depth < 4 {
                walk(&p, out, depth + 1);
            } else if p.extension().is_some_and(|x| x == "jsonl") && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("rollout-")) {
                let t = file_secs(&p);
                out.push((p, t));
            }
        }
    }
    let mut out = Vec::new();
    walk(&codex_home().join("sessions"), &mut out, 0);
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

/// Codex's own names for its chats (session_index.jsonl), by id.
fn codex_titles() -> std::collections::HashMap<String, String> {
    let text = std::fs::read_to_string(codex_home().join("session_index.jsonl")).unwrap_or_default();
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| Some((v["id"].as_str()?.to_string(), v["thread_name"].as_str()?.to_string())))
        .collect()
}

fn codex_read(path: &Path, query: Option<&str>) -> Option<(Found, Vec<Msg>)> {
    let f = std::fs::File::open(path).ok()?;
    let (mut id, mut folder, mut first, mut snip) = (String::new(), String::new(), String::new(), String::new());
    let mut msgs = Vec::new();
    for line in BufReader::new(f).lines().map_while(Result::ok) {
        if !(line.contains("\"session_meta\"") || line.contains("\"type\":\"message\"")) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        let p = &v["payload"];
        match (v["type"].as_str(), p["type"].as_str()) {
            (Some("session_meta"), _) if id.is_empty() => {
                // Subagents (their source is an object), `codex exec` scripts and other apps' runs aren't the person's chats.
                if p["source"].as_str().is_none_or(|s| s == "exec") || p["originator"] == "T3 Code" {
                    return None;
                }
                id = p["id"].as_str().or(p["session_id"].as_str()).unwrap_or("").to_string();
                folder = p["cwd"].as_str().unwrap_or("").to_string();
            }
            (Some("response_item"), Some("message")) => {
                let role = p["role"].as_str().unwrap_or("");
                if role != "user" && role != "assistant" {
                    continue;
                }
                let text = p["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c["text"].as_str())
                    .filter(|t| !is_wrapper(t))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if role == "user" && first.is_empty() && !text.trim().is_empty() {
                    first = text.clone();
                }
                if let (Some(q), true) = (query, snip.is_empty()) {
                    snip = snippet(&text, q).unwrap_or_default();
                }
                push(&mut msgs, role, &text, vec![], iso_secs(v["timestamp"].as_str().unwrap_or("")));
            }
            _ => {}
        }
    }
    if id.is_empty() || first.is_empty() || (query.is_some() && snip.is_empty()) {
        return None;
    }
    let title = codex_titles().remove(&id).unwrap_or_else(|| one_line(&first, 80));
    Some((Found { source: "codex", id, title, folder, updated: file_secs(path), snippet: snip }, msgs))
}

fn codex_recent(limit: usize, skip: &[String]) -> Vec<Found> {
    codex_files().into_iter().filter_map(|(p, _)| codex_read(&p, None).map(|r| r.0)).filter(|f| !skipped(&f.folder, skip)).take(limit).collect()
}

fn codex_search(q: &str, limit: usize, skip: &[String], generation: u64) -> Vec<Found> {
    let q = q.to_ascii_lowercase();
    let mut out = Vec::new();
    for (p, _) in codex_files() {
        if out.len() >= limit || SEARCH.load(Ordering::Relaxed) != generation {
            break;
        }
        if file_mentions(&p, &q) {
            if let Some((f, _)) = codex_read(&p, Some(&q)).filter(|r| !skipped(&r.0.folder, skip)) {
                out.push(f);
            }
        }
    }
    out
}

// ---------- SQLite apps (OpenCode, T3 Code): opened read-only, while the app itself may be writing ----------

fn open_db(path: &Path) -> Result<Connection> {
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    c.busy_timeout(std::time::Duration::from_secs(3))?;
    Ok(c)
}

fn has_table(c: &Connection, name: &str) -> bool {
    c.query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1", [name], |_| Ok(())).is_ok()
}

/// A LIKE pattern matching `q` anywhere (with `\` as the escape).
fn like(q: &str) -> String {
    format!("%{}%", q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"))
}

// OpenCode keeps chats in two layouts: `session_v2` / `session_message` (OpenCode 2) and the older `session` /
// `message` / `part`.

fn opencode_v2_text(kind: &str, data: &Value) -> (String, Vec<String>) {
    if kind == "user" {
        return (data["text"].as_str().unwrap_or("").to_string(), vec![]);
    }
    let mut text = Vec::new();
    let mut tools = Vec::new();
    for c in data["content"].as_array().into_iter().flatten() {
        match c["type"].as_str() {
            Some("text") => text.push(c["text"].as_str().unwrap_or("").to_string()),
            Some("tool") | Some("tool-call") => tools.push(c["name"].as_str().or(c["tool"].as_str()).unwrap_or("tool").to_string()),
            _ => {}
        }
    }
    (text.join("\n\n"), tools)
}

/// OpenCode's two layouts: (sessions, where a session's messages are, whether it's the newer one).
const OPENCODE_LAYOUTS: &[(&str, &str, bool)] = &[("session_v2", "session_message", true), ("session", "part", false)];

/// OpenCode's chats the person started, newest first (scripts' and Valerie's folders left out).
fn opencode_sessions(c: &Connection, table: &str, limit: usize, skip: &[String]) -> Result<Vec<(String, String, String, u64)>> {
    let mut st = c.prepare(&format!("SELECT id, COALESCE(title, ''), directory, time_updated FROM {table} WHERE parent_id IS NULL AND time_archived IS NULL ORDER BY time_updated DESC"))?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?)))?;
    Ok(rows.flatten().filter(|r| !skipped(&r.2, skip)).take(limit).map(|(a, b, c, t)| (a, b, c, (t / 1000).max(0) as u64)).collect())
}

fn opencode_list(limit: usize, skip: &[String]) -> Result<Vec<Found>> {
    let c = open_db(&opencode_db())?;
    let mut out: Vec<Found> = Vec::new();
    for (table, messages, _) in OPENCODE_LAYOUTS {
        if !has_table(&c, table) || !has_table(&c, messages) {
            continue;
        }
        for (id, mut title, folder, updated) in opencode_sessions(&c, table, limit, skip)? {
            if title.is_empty() {
                title = opencode_first_user(&c, table, &id).map(|t| one_line(&t, 80)).unwrap_or_default();
            }
            if !title.is_empty() && !out.iter().any(|f| f.id == id) {
                out.push(Found { source: "opencode", id, title, folder, updated, snippet: String::new() });
            }
        }
    }
    out.sort_by(|a, b| b.updated.cmp(&a.updated));
    out.truncate(limit);
    Ok(out)
}

/// Each chat is checked newest first, one indexed look each, so a common word finds its chats without reading the rest.
fn opencode_search(q: &str, limit: usize, skip: &[String], generation: u64) -> Result<Vec<Found>> {
    let c = open_db(&opencode_db())?;
    let q = q.to_ascii_lowercase();
    let pattern = like(&q);
    let mut out: Vec<Found> = Vec::new();
    for (table, messages, v2) in OPENCODE_LAYOUTS {
        if !has_table(&c, table) || !has_table(&c, messages) {
            continue;
        }
        let find = if *v2 {
            "SELECT type, data FROM session_message WHERE session_id = ?1 AND type IN ('user', 'assistant') AND data LIKE ?2 ESCAPE '\\' LIMIT 1"
        } else {
            "SELECT 'part', data FROM part WHERE session_id = ?1 AND data LIKE ?2 ESCAPE '\\' LIMIT 1"
        };
        let mut st = c.prepare(find)?;
        for (id, mut title, folder, updated) in opencode_sessions(&c, table, usize::MAX, skip)? {
            if out.len() >= limit || SEARCH.load(Ordering::Relaxed) != generation {
                break;
            }
            let Ok((kind, data)) = st.query_row(rusqlite::params![id, pattern], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) else { continue };
            let v: Value = serde_json::from_str(&data).unwrap_or_default();
            let text = if kind == "part" { v["text"].as_str().unwrap_or("").to_string() } else { opencode_v2_text(&kind, &v).0 };
            if title.is_empty() {
                title = opencode_first_user(&c, table, &id).map(|t| one_line(&t, 80)).unwrap_or_default();
            }
            // A match only in the agent's reasoning or a tool's output isn't one the person would recognise.
            let Some(snip) = snippet(&text, &q).or_else(|| snippet(&title, &q)) else { continue };
            if !title.is_empty() && !out.iter().any(|f| f.id == id) {
                out.push(Found { source: "opencode", id, title, folder, updated, snippet: snip });
            }
        }
    }
    out.sort_by(|a, b| b.updated.cmp(&a.updated));
    Ok(out)
}

fn opencode_first_user(c: &Connection, table: &str, id: &str) -> Option<String> {
    if table == "session_v2" {
        let data: String = c.query_row("SELECT data FROM session_message WHERE session_id = ?1 AND type = 'user' ORDER BY seq LIMIT 1", [id], |r| r.get(0)).ok()?;
        serde_json::from_str::<Value>(&data).ok()?["text"].as_str().map(str::to_string)
    } else {
        let data: String = c
            .query_row(
                "SELECT p.data FROM message m JOIN part p ON p.message_id = m.id WHERE m.session_id = ?1 AND json_extract(m.data, '$.role') = 'user' \
                 AND json_extract(p.data, '$.type') = 'text' ORDER BY m.time_created, p.id LIMIT 1",
                [id],
                |r| r.get(0),
            )
            .ok()?;
        serde_json::from_str::<Value>(&data).ok()?["text"].as_str().map(str::to_string)
    }
}

fn opencode_load(id: &str) -> Result<(Found, Vec<Msg>)> {
    let c = open_db(&opencode_db())?;
    let mut msgs = Vec::new();
    let v2 = has_table(&c, "session_v2") && c.query_row("SELECT 1 FROM session_v2 WHERE id = ?1", [id], |_| Ok(())).is_ok();
    let table = if v2 { "session_v2" } else { "session" };
    let (title, folder, ms): (String, String, i64) =
        c.query_row(&format!("SELECT COALESCE(title, ''), directory, time_updated FROM {table} WHERE id = ?1"), [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).context("that OpenCode chat isn't on this computer anymore")?;
    if v2 {
        let mut st = c.prepare("SELECT type, data, time_created FROM session_message WHERE session_id = ?1 AND type IN ('user', 'assistant') ORDER BY seq")?;
        for (kind, data, at) in st.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?.flatten() {
            let (text, tools) = opencode_v2_text(&kind, &serde_json::from_str(&data).unwrap_or_default());
            push(&mut msgs, &kind, &text, tools, (at / 1000) as u64);
        }
    } else {
        let mut st = c.prepare(
            "SELECT json_extract(m.data, '$.role'), p.data, m.time_created FROM message m JOIN part p ON p.message_id = m.id WHERE m.session_id = ?1 ORDER BY m.time_created, p.id",
        )?;
        for (role, data, at) in st.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?.flatten() {
            let p: Value = serde_json::from_str(&data).unwrap_or_default();
            if p["synthetic"] == true {
                continue;
            }
            match p["type"].as_str() {
                Some("text") => push(&mut msgs, &role, p["text"].as_str().unwrap_or(""), vec![], (at / 1000) as u64),
                Some("tool") if role == "assistant" => push(&mut msgs, &role, "", vec![p["tool"].as_str().unwrap_or("tool").to_string()], (at / 1000) as u64),
                _ => {}
            }
        }
    }
    let title = if title.is_empty() { msgs.iter().find(|m| m.role == "user").map(|m| one_line(&m.text, 80)).unwrap_or_default() } else { title };
    Ok((Found { source: "opencode", id: id.into(), title, folder, updated: (ms / 1000).max(0) as u64, snippet: String::new() }, msgs))
}

// T3 Code: threads, and the messages its agents wrote, in its state database.

const T3_FROM: &str = "FROM orchestration_v2_projection_threads t LEFT JOIN projection_projects p ON p.project_id = t.project_id";
// Live threads the person started (a subagent's or delegate's thread has a parent).
const T3_LIVE: &str = "t.archived_at IS NULL AND t.deleted_at IS NULL AND json_extract(t.payload_json, '$.lineage.parentThreadId') IS NULL";
const T3_FOLDER: &str = "COALESCE(json_extract(t.payload_json, '$.worktreePath'), p.workspace_root, '')";

fn t3_threads(c: &Connection, limit: usize) -> Result<Vec<Found>> {
    let mut st = c.prepare(&format!("SELECT t.thread_id, t.title, {T3_FOLDER}, t.updated_at {T3_FROM} WHERE {T3_LIVE} ORDER BY t.updated_at DESC LIMIT ?1"))?;
    let rows = st.query_map([limit.min(i64::MAX as usize) as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?;
    Ok(rows.flatten().map(|(id, title, folder, at)| Found { source: "t3", id, title, folder, updated: iso_secs(&at), snippet: String::new() }).collect())
}

fn t3_list(limit: usize) -> Result<Vec<Found>> {
    t3_threads(&open_db(&t3_db())?, limit)
}

/// Threads newest first, one indexed look each, stopping once enough are found.
fn t3_search(q: &str, limit: usize, generation: u64) -> Result<Vec<Found>> {
    let c = open_db(&t3_db())?;
    let q = q.to_ascii_lowercase();
    let pattern = like(&q);
    let mut st = c.prepare(
        "SELECT json_extract(payload_json, '$.text') FROM orchestration_v2_projection_messages WHERE thread_id = ?1 AND streaming = 0 \
         AND role IN ('user', 'assistant') AND json_extract(payload_json, '$.text') LIKE ?2 ESCAPE '\\' LIMIT 1",
    )?;
    let mut out = Vec::new();
    for mut f in t3_threads(&c, usize::MAX)? {
        if out.len() >= limit || SEARCH.load(Ordering::Relaxed) != generation {
            break;
        }
        let hit: Option<String> = st.query_row(rusqlite::params![f.id, pattern], |r| r.get(0)).ok();
        if let Some(text) = hit.or_else(|| f.title.to_ascii_lowercase().contains(&q).then(|| f.title.clone())) {
            f.snippet = snippet(&text, &q).unwrap_or_default();
            out.push(f);
        }
    }
    Ok(out)
}

/// A T3 thread's finished messages, oldest first: (id, role, text, created).
pub fn t3_messages(thread: &str) -> Result<Vec<(String, String, String, String)>> {
    let c = open_db(&t3_db())?;
    let mut st = c.prepare(
        "SELECT message_id, role, COALESCE(json_extract(payload_json, '$.text'), ''), created_at FROM orchestration_v2_projection_messages \
         WHERE thread_id = ?1 AND streaming = 0 AND role IN ('user', 'assistant') ORDER BY created_at, message_id",
    )?;
    let rows = st.query_map([thread], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    Ok(rows.flatten().collect())
}

fn t3_load(id: &str) -> Result<(Found, Vec<Msg>)> {
    let c = open_db(&t3_db())?;
    let (title, folder, at): (String, String, String) =
        c.query_row(&format!("SELECT t.title, {T3_FOLDER}, t.updated_at {T3_FROM} WHERE {T3_LIVE} AND t.thread_id = ?1"), [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).context("that T3 Code thread isn't there anymore")?;
    let mut msgs = Vec::new();
    for (_, role, text, created) in t3_messages(id)? {
        push(&mut msgs, &role, &text, vec![], iso_secs(&created));
    }
    Ok((Found { source: "t3", id: id.into(), title, folder, updated: iso_secs(&at), snippet: String::new() }, msgs))
}

// ---------- T3 Code's own server and command-line tool ----------

/// The `t3` program, if T3 Code is installed.
pub fn t3_bin() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "t3.exe" } else { "t3" };
    let own = t3_home().join("bin").join(exe);
    if own.is_file() {
        return Some(own);
    }
    std::env::split_paths(&crate::ai::search_path()).map(|d| d.join(exe)).find(|p| p.is_file())
}

/// The address of T3 Code's running server (its web app), if it's running.
pub fn t3_origin() -> Option<String> {
    let text = std::fs::read_to_string(t3_home().join("userdata").join("server-runtime.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let (host, port) = (v["host"].as_str()?, v["port"].as_u64()?);
    let addr: std::net::SocketAddr = format!("{host}:{port}").parse().ok()?;
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)).ok()?;
    v["origin"].as_str().map(|o| o.trim_end_matches('/').to_string())
}

/// T3 Code's server, started in the background if it isn't running (T3 Code's own app may be closed).
pub fn t3_start() -> Result<String> {
    if let Some(o) = t3_origin() {
        return Ok(o);
    }
    let bin = t3_bin().context("T3 Code isn't installed")?;
    let mut c = crate::platform::command(&bin);
    c.args(["serve", "--no-browser"]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    c.env("PATH", crate::ai::search_path());
    c.spawn().context("couldn't start T3 Code")?;
    for _ in 0..60 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if let Some(o) = t3_origin() {
            return Ok(o);
        }
    }
    bail!("T3 Code didn't start")
}

/// A one-time link that signs this browser in to T3 Code's web app.
pub fn t3_pair_url() -> Result<String> {
    let bin = t3_bin().context("T3 Code isn't installed")?;
    let out = crate::platform::command(&bin).args(["pair", "--ttl", "2m", "--label", "Valerie"]).env("PATH", crate::ai::search_path()).output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().find_map(|l| l.trim().strip_prefix("Pairing URL:")).map(|u| u.trim().to_string()).context("T3 Code didn't give a sign-in link")
}

/// The id T3 Code's links use for this computer.
pub fn t3_environment() -> String {
    std::fs::read_to_string(t3_home().join("userdata").join("environment-id")).unwrap_or_default().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_times() {
        assert_eq!(iso_secs("1970-01-01T00:00:00Z"), 0);
        assert_eq!(iso_secs("2026-11-23T07:59:51.204Z"), 1795420791);
        assert_eq!(iso_secs("2000-03-01T00:00:00Z"), 951868800);
        assert_eq!(iso_secs("nope"), 0);
    }

    #[test]
    fn snippets_find_any_case() {
        let s = snippet("We talked about the Kalshi trading app yesterday", "kalshi").unwrap();
        assert!(s.contains("Kalshi trading"));
        assert!(snippet("nothing here", "kalshi").is_none());
        // Multi-byte text around the match doesn't split a character.
        let long = "é".repeat(80) + "needle" + &"ü".repeat(80);
        assert!(snippet(&long, "needle").unwrap().contains("needle"));
    }

    #[test]
    fn answers_in_pieces_become_one_message() {
        let mut m = Vec::new();
        push(&mut m, "user", "hi", vec![], 1);
        push(&mut m, "assistant", "one", vec![], 2);
        push(&mut m, "assistant", "", vec!["Bash".into()], 2);
        push(&mut m, "assistant", "two", vec![], 3);
        push(&mut m, "user", "   ", vec![], 4);
        assert_eq!(m.len(), 2);
        assert_eq!(m[1].text, "one\n\ntwo");
        assert_eq!(m[1].tools, vec!["Bash"]);
    }

    #[test]
    fn reads_a_claude_chat() {
        let dir = std::env::temp_dir().join(format!("valerie-history-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("abc.jsonl");
        let lines = [
            r#"{"type":"attachment","attachment":{"type":"instructions","content":"needle in the instructions"}}"#,
            r#"{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"},"cwd":"/code/app","timestamp":"2026-01-01T00:00:00Z"}"#,
            r#"{"type":"user","message":{"role":"user","content":"Fix the login bug"},"cwd":"/code/app","timestamp":"2026-01-01T00:00:01Z"}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Looking at auth.rs"},{"type":"tool_use","name":"Read"}]},"timestamp":"2026-01-01T00:00:02Z"}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"file text"}]},"timestamp":"2026-01-01T00:00:03Z"}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Fixed it."}]},"timestamp":"2026-01-01T00:00:04Z"}"#,
            r#"{"type":"custom-title","customTitle":"Login bug"}"#,
        ];
        std::fs::write(&p, lines.join("\n")).unwrap();
        let (f, msgs) = claude_read(&p, None).unwrap();
        assert_eq!((f.id.as_str(), f.title.as_str(), f.folder.as_str()), ("abc", "Login bug", "/code/app"));
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].text, "Looking at auth.rs\n\nFixed it.");
        // A word only in the app's own instructions isn't a match; one in the conversation is.
        assert!(claude_read(&p, Some("needle")).is_none());
        assert!(claude_read(&p, Some("auth.rs")).unwrap().0.snippet.contains("auth.rs"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_valerie_and_temp_folders() {
        let skip = vec!["/data/valerie".to_string(), "/tmp/".to_string()];
        assert!(skipped("/data/valerie/agents/chat", &skip));
        assert!(skipped("/tmp/ptx-monitor-1", &skip));
        assert!(!skipped("/code/app", &skip));
        assert!(skipped("/tmp", &skip));
        assert!(!skipped("/tmpfiles/app", &skip));
        let h = home().to_string_lossy().into_owned();
        assert!(skipped(&format!("{h}/.local/state/monitor"), &skip));
        assert!(!skipped(&format!("{h}/code/app"), &skip));
        assert_eq!(raw_field(r#"{"a":1,"entrypoint":"t3-code","cwd":"/x"}"#, "entrypoint"), Some("t3-code"));
        assert!(driven("sdk-ts") && driven("t3-code") && !driven("cli") && !driven("claude-desktop"));
    }
}

#[cfg(test)]
mod real_data {
    /// Reads this computer's own chats: `cargo test -- --ignored other_chats_here --nocapture`.
    #[test]
    #[ignore]
    fn other_chats_here() {
        let skip = super::skip_dirs();
        for (s, _) in super::SOURCES {
            let t = std::time::Instant::now();
            let r = super::recent(s, 20, &skip);
            println!("{s}: {} recent in {:?}", r.len(), t.elapsed());
            for f in r.iter().take(3) {
                println!("   {} | {} | {}", f.title, f.folder, f.updated);
            }
            let t = std::time::Instant::now();
            let hits = super::search(s, "worktree", 10, &skip, 0);
            println!("   search: {} hits in {:?}; first: {:?}", hits.len(), t.elapsed(), hits.first().map(|h| (&h.title, &h.snippet)));
            if let Some(f) = r.first() {
                let (_, msgs) = super::load(s, &f.id).unwrap();
                println!("   load {}: {} messages", f.id, msgs.len());
            }
        }
    }
}
