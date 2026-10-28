//! Add Valerie's MCP server to the AI apps on this computer (Claude Code, Codex, Cursor, …) so their agents can use
//! the browser: `valerie connect`, the "Connect AI Apps" command, the welcome guide and `mcp.connect_ai_apps`.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// How an app keeps its MCP servers.
#[derive(Clone, Copy, PartialEq)]
enum Format {
    /// A JSON file with the servers under this key: { "command", "args" }.
    Json(&'static str),
    /// VS Code's mcp.json: "servers" with "type": "stdio".
    VsCode,
    /// GitHub Copilot CLI: "mcpServers" with "type": "local" and the tools it may use.
    Copilot,
    /// OpenCode: "mcp" with { "type": "local", "command": [exe, args…] }.
    OpenCode,
    /// Codex's config.toml: a [mcp_servers.valerie] table.
    CodexToml,
}

pub struct AiApp {
    pub id: &'static str,
    pub title: &'static str,
    format: Format,
    /// The settings file, from the home folder (or the app-data folder for `Data`).
    file: Loc,
    /// This folder exists when the app is installed (it makes it on first run).
    marker: Loc,
}

#[derive(Clone, Copy)]
enum Loc {
    Home(&'static str),
    /// macOS: ~/Library/Application Support, Linux: ~/.config, Windows: %APPDATA%.
    Data(&'static str),
}

pub const APPS: &[AiApp] = &[
    AiApp { id: "claude_code", title: "Claude Code", format: Format::Json("mcpServers"), file: Loc::Home(".claude.json"), marker: Loc::Home(".claude") },
    AiApp { id: "claude_desktop", title: "Claude Desktop", format: Format::Json("mcpServers"), file: Loc::Data("Claude/claude_desktop_config.json"), marker: Loc::Data("Claude") },
    AiApp { id: "codex", title: "Codex", format: Format::CodexToml, file: Loc::Home(".codex/config.toml"), marker: Loc::Home(".codex") },
    AiApp { id: "cursor", title: "Cursor", format: Format::Json("mcpServers"), file: Loc::Home(".cursor/mcp.json"), marker: Loc::Home(".cursor") },
    AiApp { id: "gemini", title: "Gemini CLI", format: Format::Json("mcpServers"), file: Loc::Home(".gemini/settings.json"), marker: Loc::Home(".gemini") },
    AiApp { id: "opencode", title: "OpenCode", format: Format::OpenCode, file: Loc::Home(".config/opencode/opencode.json"), marker: Loc::Home(".config/opencode") },
    AiApp { id: "vscode", title: "VS Code (Copilot)", format: Format::VsCode, file: Loc::Data("Code/User/mcp.json"), marker: Loc::Data("Code/User") },
    AiApp { id: "copilot_cli", title: "Copilot CLI", format: Format::Copilot, file: Loc::Home(".copilot/mcp-config.json"), marker: Loc::Home(".copilot") },
    AiApp { id: "windsurf", title: "Windsurf", format: Format::Json("mcpServers"), file: Loc::Home(".codeium/windsurf/mcp_config.json"), marker: Loc::Home(".codeium/windsurf") },
    AiApp {
        id: "cline",
        title: "Cline",
        format: Format::Json("mcpServers"),
        file: Loc::Data("Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"),
        marker: Loc::Data("Code/User/globalStorage/saoudrizwan.claude-dev"),
    },
    AiApp { id: "kiro", title: "Kiro", format: Format::Json("mcpServers"), file: Loc::Home(".kiro/settings/mcp.json"), marker: Loc::Home(".kiro") },
    AiApp { id: "lm_studio", title: "LM Studio", format: Format::Json("mcpServers"), file: Loc::Home(".lmstudio/mcp.json"), marker: Loc::Home(".lmstudio") },
    AiApp { id: "amp", title: "Amp", format: Format::Json("amp.mcpServers"), file: Loc::Home(".config/amp/settings.json"), marker: Loc::Home(".config/amp") },
];

/// Where an app stands with Valerie.
#[derive(Debug, PartialEq)]
pub enum Status {
    NotInstalled,
    NotConnected,
    Connected,
    /// Connected to a Valerie that has since moved (an older copy, another folder): connecting fixes it.
    Outdated,
    /// It has its own server called "valerie" that Valerie didn't add; left alone.
    Other,
}

/// The home folder (VALERIE_CONNECT_HOME stands in for it in tests, so they never touch the real apps).
fn home() -> PathBuf {
    std::env::var_os("VALERIE_CONNECT_HOME").map(PathBuf::from).unwrap_or_else(|| dirs::home_dir().unwrap_or_default())
}

fn resolve(loc: Loc) -> PathBuf {
    match loc {
        Loc::Home(p) => home().join(p),
        Loc::Data(p) if std::env::var_os("VALERIE_CONNECT_HOME").is_some() => home().join("AppData").join(p),
        Loc::Data(p) => {
            let base = if cfg!(target_os = "macos") { home().join("Library/Application Support") } else { dirs::config_dir().unwrap_or_else(|| home().join(".config")) };
            base.join(p)
        }
    }
}

impl AiApp {
    pub fn file(&self) -> PathBuf {
        resolve(self.file)
    }

    pub fn installed(&self) -> bool {
        resolve(self.marker).is_dir() || self.file().is_file()
    }
}

pub fn app(id: &str) -> Option<&'static AiApp> {
    APPS.iter().find(|a| a.id == id)
}

/// The program the apps should start: this copy of Valerie, as long as it's one that stays put.
pub fn exe() -> Result<String> {
    let exe = std::env::current_exe()?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let shown = exe.to_string_lossy().into_owned();
    if shown.contains("/AppTranslocation/") {
        bail!("move Valerie to Applications first (macOS is running it from a temporary copy)");
    }
    Ok(shown)
}

/// Whether this copy is the installed app (not a build being tested), so connecting by itself at start is safe.
pub fn is_installed_copy(exe: &str) -> bool {
    let home = dirs::home_dir().unwrap_or_default();
    if cfg!(target_os = "macos") {
        exe.starts_with("/Applications/") || exe.starts_with(&*home.join("Applications").to_string_lossy())
    } else {
        !exe.contains("/target/") && !exe.contains("\\target\\")
    }
}

/// Whether an existing "valerie" entry was added by Valerie (it starts `… mcp-stdio`).
fn ours(entry: &Value) -> bool {
    let words: Vec<&str> = match (&entry["command"], &entry["args"]) {
        (Value::Array(c), _) => c.iter().filter_map(Value::as_str).collect(),
        (Value::String(c), Value::Array(a)) => std::iter::once(c.as_str()).chain(a.iter().filter_map(Value::as_str)).collect(),
        _ => return false,
    };
    words.contains(&"mcp-stdio")
}

fn points_at(entry: &Value, exe: &str) -> bool {
    match &entry["command"] {
        Value::Array(c) => c.first().and_then(Value::as_str) == Some(exe),
        c => c.as_str() == Some(exe),
    }
}

fn entry(format: Format, exe: &str) -> Value {
    match format {
        Format::OpenCode => json!({ "type": "local", "command": [exe, "mcp-stdio"], "enabled": true }),
        Format::VsCode => json!({ "type": "stdio", "command": exe, "args": ["mcp-stdio"] }),
        Format::Copilot => json!({ "type": "local", "command": exe, "args": ["mcp-stdio"], "tools": ["*"] }),
        _ => json!({ "command": exe, "args": ["mcp-stdio"] }),
    }
}

fn servers_key(format: Format) -> &'static str {
    match format {
        Format::Json(k) => k,
        Format::VsCode => "servers",
        Format::Copilot => "mcpServers",
        Format::OpenCode => "mcp",
        Format::CodexToml => "",
    }
}

/// A JSON settings file, `{}` when there isn't one yet. Files with comments aren't touched (rewriting would drop them).
fn read_json(file: &Path) -> Result<Value> {
    match std::fs::read_to_string(file) {
        Ok(t) if t.trim().is_empty() => Ok(json!({})),
        Ok(t) => {
            let v: Value = serde_json::from_str(&t).with_context(|| format!("{} isn't plain JSON (comments?), so it was left alone", file.display()))?;
            if !v.is_object() {
                bail!("{} isn't a JSON object, so it was left alone", file.display());
            }
            Ok(v)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e).with_context(|| format!("couldn't read {}", file.display())),
    }
}

/// Write a settings file in one step, keeping a copy of how it was before Valerie first changed it.
fn write_file(file: &Path, text: &str) -> Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let backup = PathBuf::from(format!("{}.before-valerie", file.display()));
    if file.is_file() && !backup.exists() {
        std::fs::copy(file, &backup)?;
    }
    let tmp = PathBuf::from(format!("{}.valerie-tmp", file.display()));
    std::fs::write(&tmp, text)?;
    // Keep the file's permissions (some hold API keys and are private).
    if let Ok(m) = std::fs::metadata(file) {
        let _ = std::fs::set_permissions(&tmp, m.permissions());
    }
    std::fs::rename(&tmp, file).with_context(|| format!("couldn't write {}", file.display()))
}

/// The lines of Codex's config.toml without the [mcp_servers.valerie] table (and its sub-tables), and that table.
fn split_codex(toml: &str) -> (Vec<&str>, Vec<&str>) {
    let (mut rest, mut ours) = (Vec::new(), Vec::new());
    let mut inside = false;
    for line in toml.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            let name = t.trim_start_matches('[').trim_end_matches(']').trim();
            inside = name == "mcp_servers.valerie" || name.starts_with("mcp_servers.valerie.");
        }
        if inside { ours.push(line) } else { rest.push(line) }
    }
    (rest, ours)
}

fn codex_entry(table: &[&str]) -> Option<Value> {
    if table.is_empty() {
        return None;
    }
    let get = |k: &str| table.iter().find_map(|l| l.split_once('=').filter(|(a, _)| a.trim() == k).map(|(_, b)| b.trim().to_string()));
    let command = get("command").and_then(|c| serde_json::from_str::<Value>(&c).ok()).unwrap_or(Value::Null);
    let args = get("args").and_then(|a| serde_json::from_str::<Value>(&a).ok()).unwrap_or(json!([]));
    Some(json!({ "command": command, "args": args }))
}

/// The "valerie" entry the app has now, if any.
fn current(a: &AiApp) -> Result<Option<Value>> {
    let file = a.file();
    if a.format == Format::CodexToml {
        let toml = std::fs::read_to_string(&file).unwrap_or_default();
        return Ok(codex_entry(&split_codex(&toml).1));
    }
    let v = read_json(&file)?;
    Ok(v.get(servers_key(a.format)).and_then(|s| s.get("valerie")).cloned())
}

pub fn status(a: &AiApp, exe: &str) -> Status {
    if !a.installed() {
        return Status::NotInstalled;
    }
    match current(a) {
        Ok(None) | Err(_) => Status::NotConnected,
        Ok(Some(e)) if !ours(&e) => Status::Other,
        Ok(Some(e)) if points_at(&e, exe) => Status::Connected,
        Ok(Some(_)) => Status::Outdated,
    }
}

/// Add (or point at this copy) Valerie in one app. Returns false when it was already connected.
pub fn connect(a: &AiApp, exe: &str) -> Result<bool> {
    match status(a, exe) {
        Status::Connected => return Ok(false),
        Status::Other => bail!("{} already has its own server called valerie; left alone", a.title),
        _ => {}
    }
    if let Some(bin) = claude_cli(a) {
        // Claude Code rewrites ~/.claude.json all the time, so its own command makes the change.
        let _ = run(&bin, &["mcp", "remove", "--scope", "user", "valerie"]);
        run(&bin, &["mcp", "add", "--scope", "user", "valerie", "--", exe, "mcp-stdio"])?;
        return Ok(true);
    }
    let file = a.file();
    if a.format == Format::CodexToml {
        let toml = std::fs::read_to_string(&file).unwrap_or_default();
        let (rest, _) = split_codex(&toml);
        let mut out = rest.join("\n").trim_end().to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        // JSON strings are valid TOML basic strings.
        out.push_str(&format!("[mcp_servers.valerie]\ncommand = {}\nargs = [\"mcp-stdio\"]\n", json!(exe)));
        write_file(&file, &out)?;
        return Ok(true);
    }
    let mut v = read_json(&file)?;
    let key = servers_key(a.format);
    if !v[key].is_object() {
        v[key] = json!({});
    }
    v[key]["valerie"] = entry(a.format, exe);
    write_file(&file, &(serde_json::to_string_pretty(&v)? + "\n"))?;
    Ok(true)
}

/// Take Valerie out of one app (only an entry Valerie added). Returns false when there was nothing to take out.
pub fn disconnect(a: &AiApp) -> Result<bool> {
    let file = a.file();
    match current(a)? {
        Some(e) if ours(&e) => {}
        _ => return Ok(false),
    }
    if let Some(bin) = claude_cli(a) {
        run(&bin, &["mcp", "remove", "--scope", "user", "valerie"])?;
        return Ok(true);
    }
    if a.format == Format::CodexToml {
        let toml = std::fs::read_to_string(&file)?;
        write_file(&file, &(split_codex(&toml).0.join("\n").trim_end().to_string() + "\n"))?;
        return Ok(true);
    }
    let mut v = read_json(&file)?;
    if let Some(s) = v.get_mut(servers_key(a.format)).and_then(Value::as_object_mut) {
        s.remove("valerie");
    }
    write_file(&file, &(serde_json::to_string_pretty(&v)? + "\n"))?;
    Ok(true)
}

/// The `claude` program, when connecting Claude Code for real (tests edit the file instead).
fn claude_cli(a: &AiApp) -> Option<PathBuf> {
    if a.id != "claude_code" || std::env::var_os("VALERIE_CONNECT_HOME").is_some() {
        return None;
    }
    crate::ai::find_bin(crate::ai::kind("claude")?, &crate::ai::search_path())
}

fn run(bin: &Path, args: &[&str]) -> Result<()> {
    let out = std::process::Command::new(bin).args(args).env("PATH", crate::ai::search_path()).stdin(std::process::Stdio::null()).output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
        bail!("{} said: {}", bin.display(), err.lines().find(|l| !l.trim().is_empty()).unwrap_or("it failed").trim());
    }
    Ok(())
}

/// Every app with where it stands, for the welcome guide and `valerie connect --list`.
pub fn list(exe: &str) -> Vec<Value> {
    APPS.iter()
        .map(|a| {
            let s = status(a, exe);
            json!({ "id": a.id, "title": a.title, "installed": s != Status::NotInstalled,
                    "status": format!("{s:?}").to_lowercase(), "file": a.file() })
        })
        .collect()
}

/// Connect every installed app that isn't yet (or points at an old copy). Returns what changed and what failed.
pub fn connect_all(exe: &str) -> (Vec<&'static str>, Vec<String>) {
    let (mut done, mut failed) = (Vec::new(), Vec::new());
    for a in APPS.iter().filter(|a| a.installed()) {
        match connect(a, exe) {
            Ok(true) => done.push(a.title),
            Ok(false) => {}
            Err(e) => failed.push(format!("{}: {e:#}", a.title)),
        }
    }
    (done, failed)
}

/// `valerie connect [app…] [--list] [--undo]`.
pub fn cli(args: &[String]) -> Result<()> {
    let exe = exe()?;
    let undo = args.iter().any(|a| a == "--undo" || a == "--remove");
    let picked: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    for p in &picked {
        if app(p).is_none() {
            bail!("no AI app called {p}; one of: {}", APPS.iter().map(|a| a.id).collect::<Vec<_>>().join(", "));
        }
    }
    if args.iter().any(|a| a == "--list") {
        for a in APPS {
            println!("{:<15} {:<18} {:<14} {}", a.id, a.title, format!("{:?}", status(a, &exe)).to_lowercase(), a.file().display());
        }
        return Ok(());
    }
    let chosen: Vec<&AiApp> = APPS.iter().filter(|a| a.installed() && (picked.is_empty() || picked.iter().any(|p| *p == a.id))).collect();
    if chosen.is_empty() {
        println!("No AI apps found on this computer (looked for {}).", APPS.iter().map(|a| a.title).collect::<Vec<_>>().join(", "));
        return Ok(());
    }
    for a in chosen {
        let r = if undo { disconnect(a) } else { connect(a, &exe) };
        match r {
            Ok(true) if undo => println!("{}: took Valerie out of {}", a.title, a.file().display()),
            Ok(true) => println!("{}: added Valerie to {}", a.title, a.file().display()),
            Ok(false) if undo => println!("{}: Valerie wasn't there", a.title),
            Ok(false) => println!("{}: already connected", a.title),
            Err(e) => println!("{}: {e:#}", a.title),
        }
    }
    if !undo {
        println!("Restart those apps (or start a new session) and their agents can use Valerie: tools like web_search, fetch_page, open_tab and screenshot.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_home(f: impl FnOnce(&Path)) {
        let _g = crate::config::TEST_DIRS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("valerie-connect-{}", crate::mcp::random_hex()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("VALERIE_CONNECT_HOME", &dir);
        f(&dir);
        std::env::remove_var("VALERIE_CONNECT_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_apps_connect_keep_other_settings_and_undo() {
        with_home(|h| {
            std::fs::create_dir_all(h.join(".cursor")).unwrap();
            std::fs::write(h.join(".cursor/mcp.json"), r#"{"mcpServers":{"other":{"command":"x"}},"keep":1}"#).unwrap();
            let a = app("cursor").unwrap();
            assert_eq!(status(a, "/A/valerie"), Status::NotConnected);
            assert!(connect(a, "/A/valerie").unwrap());
            assert!(!connect(a, "/A/valerie").unwrap());
            let v = read_json(&a.file()).unwrap();
            assert_eq!(v["mcpServers"]["valerie"]["args"], json!(["mcp-stdio"]));
            assert_eq!(v["mcpServers"]["other"]["command"], "x");
            assert_eq!(v["keep"], 1);
            assert!(h.join(".cursor/mcp.json.before-valerie").is_file());
            assert_eq!(status(a, "/B/valerie"), Status::Outdated);
            assert!(disconnect(a).unwrap());
            assert!(read_json(&a.file()).unwrap()["mcpServers"].get("valerie").is_none());
            // Not installed: no folder, nothing written.
            assert_eq!(status(app("kiro").unwrap(), "/A/valerie"), Status::NotInstalled);
        });
    }

    #[test]
    fn leaves_files_with_comments_and_foreign_entries_alone() {
        with_home(|h| {
            std::fs::create_dir_all(h.join(".gemini")).unwrap();
            std::fs::write(h.join(".gemini/settings.json"), "{ // hi\n}").unwrap();
            assert!(connect(app("gemini").unwrap(), "/A/valerie").is_err());
            std::fs::write(h.join(".gemini/settings.json"), r#"{"mcpServers":{"valerie":{"command":"npx","args":["valerie-mcp"]}}}"#).unwrap();
            assert_eq!(status(app("gemini").unwrap(), "/A/valerie"), Status::Other);
            assert!(connect(app("gemini").unwrap(), "/A/valerie").is_err());
            assert!(!disconnect(app("gemini").unwrap()).unwrap());
        });
    }

    #[test]
    fn codex_toml_round_trips() {
        with_home(|h| {
            std::fs::create_dir_all(h.join(".codex")).unwrap();
            let before = "model = \"gpt-5\"\n\n[mcp_servers.other]\ncommand = \"x\"\n";
            std::fs::write(h.join(".codex/config.toml"), before).unwrap();
            let a = app("codex").unwrap();
            assert!(connect(a, "/A/valerie").unwrap());
            assert_eq!(status(a, "/A/valerie"), Status::Connected);
            assert!(connect(a, "/B/valerie").unwrap());
            let t = std::fs::read_to_string(a.file()).unwrap();
            assert_eq!(t.matches("[mcp_servers.valerie]").count(), 1);
            assert!(t.contains("command = \"/B/valerie\"") && t.contains("[mcp_servers.other]"));
            assert!(disconnect(a).unwrap());
            assert_eq!(std::fs::read_to_string(a.file()).unwrap(), before);
        });
    }

    #[test]
    fn opencode_and_vscode_use_their_own_shapes() {
        with_home(|h| {
            std::fs::create_dir_all(h.join(".config/opencode")).unwrap();
            std::fs::create_dir_all(h.join("AppData/Code/User")).unwrap();
            connect(app("opencode").unwrap(), "/A/valerie").unwrap();
            connect(app("vscode").unwrap(), "/A/valerie").unwrap();
            assert_eq!(read_json(&app("opencode").unwrap().file()).unwrap()["mcp"]["valerie"]["command"], json!(["/A/valerie", "mcp-stdio"]));
            assert_eq!(read_json(&app("vscode").unwrap().file()).unwrap()["servers"]["valerie"]["type"], "stdio");
            assert_eq!(status(app("opencode").unwrap(), "/A/valerie"), Status::Connected);
        });
    }
}
