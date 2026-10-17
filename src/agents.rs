//! Headless agents: run the user's own Claude Code, Codex or OpenCode CLI against Valerie's MCP server.
use anyhow::{bail, Result};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};


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
    pub def: crate::config::AgentDef,
    pub prompt: String,
    pub space: String,
    /// Edit Valerie's own config and plugins instead of browsing.
    pub customize: bool,
    /// Give up after this long.
    pub timeout: Duration,
}

#[derive(Clone)]
pub struct Finished {
    pub name: String,
    pub ok: bool,
    pub log: PathBuf,
    pub summary: String,
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

fn customize_preamble(task: &str) -> String {
    format!(
        "You are customising the user's Valerie web browser. The current folder is Valerie's config folder. Read AGENTS.md \
         here first: it explains the settings file, plugins, commands and how to validate. Make the change the user asks for \
         by editing config.yaml and/or plugins/ (smallest change that works; make new behaviour switchable with a setting). \
         Then run the check-config command from AGENTS.md and fix any errors until it passes. If the change needs a new \
         plugin, first search the plugin marketplace as AGENTS.md explains; if a plugin already does it, install that \
         instead and say so. Finish with a short summary of what you changed and how to turn it off; if you made a new \
         plugin, end by saying it's something others might like and that they can share it in the plugin marketplace.\n\nThe user's request: {task}"
    )
}

/// Run one agent to completion (blocking). Output goes to a log file under the data dir.
pub fn run(r: Run) -> Finished {
    let dir = crate::config::data_dir().join("agents");
    let work = dir.join("work");
    let _ = std::fs::create_dir_all(&work);
    let log = dir.join(format!("{}-{}.log", crate::state::now(), r.name.replace(|c: char| !c.is_ascii_alphanumeric(), "-")));
    let finished = |ok: bool, summary: String| Finished { name: r.name.clone(), ok, log: log.clone(), summary };
    let r_name = r.name.clone();
    let timeout = r.timeout;
    let result = (|| -> Result<bool> {
        let prompt = if r.customize {
            customize_preamble(&r.prompt)
        } else {
            format!("{}\n\n{}", crate::ai::control_note(&r.def), preamble(&r.space, &r.prompt))
        };
        let cwd = if r.customize { crate::config::config_dir() } else { work.clone() };
        let mode = if r.customize { crate::ai::Mode::Customize } else { crate::ai::Mode::Browser };
        let (mut cmd, _) = crate::ai::build(&r.def, mode, &prompt, None, false, &cwd, &crate::ai::Opts::default())?;
        let file = std::fs::File::create(&log)?;
        use std::io::Write;
        writeln!(&file, "# {} ({})\n# {}\n", r.name, if r.def.cli.is_empty() { "command" } else { &r.def.cli }, r.prompt)?;
        let mut child = cmd.current_dir(&cwd).stdin(Stdio::null()).stdout(file.try_clone()?).stderr(file).spawn()?;
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
            if start.elapsed() > timeout {
                let _ = child.kill();
                forget();
                bail!("timed out after {} minutes", timeout.as_secs() / 60);
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
