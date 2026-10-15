//! Headless agents: run the user's own Claude Code, Codex or OpenCode CLI against Valerie's MCP server.
use anyhow::{bail, Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// Running agent processes by run name, so they can be stopped from the command bar.
static RUNNING: std::sync::Mutex<Vec<(String, u32)>> = std::sync::Mutex::new(Vec::new());

pub fn stop(name: &str) -> bool {
    let pid = RUNNING.lock().ok().and_then(|r| r.iter().find(|(n, _)| n == name).map(|(_, p)| *p));
    let Some(pid) = pid else { return false };
    #[cfg(unix)]
    let ok = Command::new("kill").arg(pid.to_string()).status().is_ok_and(|s| s.success());
    #[cfg(windows)]
    let ok = Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).status().is_ok_and(|s| s.success());
    ok
}

pub struct Run {
    pub name: String,
    pub agent: String,
    pub model: String,
    pub prompt: String,
    pub space: String,
}

#[derive(Clone)]
pub struct Finished {
    pub name: String,
    pub ok: bool,
    pub log: PathBuf,
    pub summary: String,
}

/// GUI apps get a minimal PATH, so look where CLIs are usually installed too.
fn search_path() -> String {
    let home = dirs::home_dir().unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    for extra in [".local/bin", ".bun/bin", ".npm-global/bin", ".cargo/bin", ".opencode/bin"] {
        dirs.push(home.join(extra));
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    std::env::join_paths(dirs).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

fn find(bin: &str, path: &str) -> Option<PathBuf> {
    std::env::split_paths(path).map(|d| d.join(bin)).find(|p| p.is_file())
}

fn preamble(space: &str, task: &str) -> String {
    format!(
        "You are a headless agent working inside the user's Valerie browser through the `valerie` MCP tools. \
         Open the tabs you need in the space named \"{space}\" (open_tab with space=\"{space}\"), work in the background, \
         and do not close or change the user's other tabs unless the task asks for it. Do not submit purchases, \
         payments or irreversible forms; stop and report instead. Finish with a short plain-text summary of what you found or did.\n\n\
         Task: {task}"
    )
}

fn command(agent: &str, model: &str, prompt: &str, exe: &Path, path: &str) -> Result<Command> {
    let bin = find(agent, path).with_context(|| format!("`{agent}` CLI not found on PATH"))?;
    let exe = exe.to_string_lossy().to_string();
    let mut c = Command::new(bin);
    match agent {
        "claude" => {
            let mcp = json!({ "mcpServers": { "valerie": { "command": exe, "args": ["mcp-stdio"] } } });
            c.args(["-p", prompt, "--mcp-config", &mcp.to_string(), "--strict-mcp-config", "--allowedTools", "mcp__valerie"]);
        }
        "codex" => {
            c.args(["exec", "--skip-git-repo-check", "-s", "read-only"])
                .args(["-c", &format!("mcp_servers.valerie.command={}", serde_json::to_string(&exe)?)])
                .args(["-c", r#"mcp_servers.valerie.args=["mcp-stdio"]"#])
                .arg(prompt);
        }
        "opencode" => {
            let cfg = json!({ "mcp": { "valerie": { "type": "local", "command": [exe, "mcp-stdio"], "enabled": true } } });
            c.args(["run", prompt]).env("OPENCODE_CONFIG_CONTENT", cfg.to_string());
        }
        other => bail!("unknown agent `{other}` (use claude, codex or opencode)"),
    }
    if !model.is_empty() {
        c.args([if agent == "claude" { "--model" } else { "-m" }, model]);
    }
    c.env("PATH", path);
    Ok(c)
}

/// Run one agent to completion (blocking). Output goes to a log file under the data dir.
pub fn run(r: Run) -> Finished {
    let dir = crate::config::data_dir().join("agents");
    let work = dir.join("work");
    let _ = std::fs::create_dir_all(&work);
    let log = dir.join(format!("{}-{}.log", crate::state::now(), r.name.replace(|c: char| !c.is_ascii_alphanumeric(), "-")));
    let finished = |ok: bool, summary: String| Finished { name: r.name.clone(), ok, log: log.clone(), summary };
    let r_name = r.name.clone();
    let result = (|| -> Result<bool> {
        let exe = std::env::current_exe()?;
        let path = search_path();
        let mut cmd = command(&r.agent, &r.model, &preamble(&r.space, &r.prompt), &exe, &path)?;
        let file = std::fs::File::create(&log)?;
        use std::io::Write;
        writeln!(&file, "# {} ({})\n# {}\n", r.name, r.agent, r.prompt)?;
        let mut child = cmd.current_dir(&work).stdin(Stdio::null()).stdout(file.try_clone()?).stderr(file).spawn()?;
        if let Ok(mut r) = RUNNING.lock() {
            r.push((r_name.clone(), child.id()));
        }
        let forget = || {
            if let Ok(mut r) = RUNNING.lock() {
                r.retain(|(n, _)| n != &r_name);
            }
        };
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                forget();
                return Ok(status.success());
            }
            if start.elapsed() > TIMEOUT {
                let _ = child.kill();
                forget();
                bail!("timed out after {} minutes", TIMEOUT.as_secs() / 60);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    })();
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    let tail: String = text.trim().chars().rev().take(400).collect::<Vec<_>>().into_iter().rev().collect();
    match result {
        Ok(ok) => finished(ok, tail),
        Err(e) => finished(false, format!("{e:#}")),
    }
}

/// Current local time as "HH:MM".
pub fn local_hhmm() -> String {
    Command::new("date").arg("+%H:%M").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}
