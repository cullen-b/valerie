//! AI in Valerie: the agent CLIs already on the user's computer (Claude Code, Codex, OpenCode, Pi, Cursor, Hermes, or
//! any command), run headless. They chat in the Chats space with full control of the browser, answer questions
//! about a page from find (with no tools at all), and stream their answers back as they write them.
use crate::config::AgentDef;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A CLI Valerie knows how to drive.
pub struct Kind {
    pub id: &'static str,
    pub title: &'static str,
    /// Executable names to look for, in order.
    pub bins: &'static [&'static str],
    /// Shell command that installs it (run in a terminal when it's missing).
    pub install: &'static str,
    /// The same in PowerShell, for Windows.
    pub install_windows: &'static str,
    /// Command that signs in (also runs in PowerShell on Windows).
    pub login: &'static str,
    /// Its own install page, for when installing fails.
    pub help: &'static str,
}

impl Kind {
    fn install_command(&self) -> &'static str {
        if cfg!(windows) {
            self.install_windows
        } else {
            self.install
        }
    }
}

pub const KINDS: &[Kind] = &[
    Kind {
        id: "claude",
        title: "Claude Code",
        bins: &["claude"],
        install: "curl -fsSL https://claude.ai/install.sh | bash",
        install_windows: "irm https://claude.ai/install.ps1 | iex",
        login: "claude auth login",
        help: "https://code.claude.com/docs/en/setup",
    },
    Kind {
        id: "codex",
        title: "Codex",
        bins: &["codex"],
        install: "curl -fsSL https://chatgpt.com/codex/install.sh | sh",
        install_windows: "irm https://chatgpt.com/codex/install.ps1 | iex",
        login: "codex login",
        help: "https://learn.chatgpt.com/docs/codex/cli",
    },
    Kind {
        id: "opencode",
        title: "OpenCode",
        bins: &["opencode"],
        install: "curl -fsSL https://opencode.ai/install | bash",
        // OpenCode has no Windows install script; its zip holds just opencode.exe. Only single quotes: this is passed on a command line.
        install_windows: r"$ProgressPreference = 'SilentlyContinue'; $d = Join-Path $HOME '.opencode\bin'; $z = Join-Path $env:TEMP 'opencode.zip'; New-Item -ItemType Directory -Force $d | Out-Null; irm https://github.com/anomalyco/opencode/releases/latest/download/opencode-windows-x64.zip -OutFile $z; Expand-Archive -Force $z $d; Remove-Item $z",
        login: "opencode auth login",
        help: "https://opencode.ai/docs/",
    },
    Kind {
        id: "pi",
        title: "Pi",
        bins: &["pi"],
        install: "npm install -g --ignore-scripts @earendil-works/pi-coding-agent",
        install_windows: "npm install -g --ignore-scripts @earendil-works/pi-coding-agent",
        login: "echo 'Type /login, pick your provider and sign in, then /quit.'; pi",
        help: "https://pi.dev",
    },
    Kind {
        id: "cursor",
        title: "Cursor",
        bins: &["cursor-agent", "agent"],
        install: "curl https://cursor.com/install -fsS | bash",
        install_windows: "irm 'https://cursor.com/install?win32=true' | iex",
        login: "cursor-agent login",
        help: "https://cursor.com/docs/cli/installation",
    },
    Kind {
        id: "hermes",
        title: "Hermes",
        bins: &["hermes"],
        install: "curl -fsSL https://raw.githubusercontent.com/NousResearch/hermes-agent/main/scripts/install.sh | bash",
        install_windows: "irm https://raw.githubusercontent.com/NousResearch/hermes-agent/main/scripts/install.ps1 | iex",
        login: "hermes setup",
        help: "https://github.com/NousResearch/hermes-agent",
    },
];

pub fn kind(id: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.id == id)
}

/// What the agent may do.
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// Full control of the browser (Valerie's MCP tools, or `valerie call` for CLIs without MCP).
    Browser,
    /// Answer from the page text it's given, with as little power as the CLI allows: no tools for Claude, Pi and
    /// OpenCode; read-only, offline and in an empty folder for Codex, Cursor and Hermes. The page text could be trying
    /// to trick it, and the answer is shown in Valerie's own panel, never inside the page.
    PageOnly,
    /// Edit Valerie's config folder.
    Customize,
}

/// How the CLI's output is laid out.
#[derive(Clone, Copy)]
pub enum Format {
    Claude,
    Codex,
    OpenCode,
    Cursor,
    Pi,
    Text,
}

/// One piece of an agent's answer as it arrives.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Ev {
    /// Text to append.
    Delta { text: String },
    /// A whole paragraph or message to append (separated from what came before).
    Block { text: String },
    /// The complete answer, used only if nothing streamed.
    Final { text: String },
    /// The agent used a tool ("open_tab", "Ran ls"…).
    Tool { text: String },
    /// A step started, or more about it arrived (matched by id): a tool call with what it was given.
    Step { step: Step },
    /// A step finished, with what it printed or returned.
    StepDone { id: String, output: String, error: bool },
    /// Reasoning to append.
    Thinking { text: String },
    /// How much of the model's context the conversation fills, and tokens written (zero when not reported).
    Usage { context: u64, output: u64 },
    /// Session id for continuing the conversation next turn.
    Session { id: String },
    Error { text: String },
}

/// One thing an agent did while answering, shown in the chat where it happened.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Step {
    /// The CLI's id for the call, so its result can find it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// The tool ("Edit", "Bash", "open_tab"…), or "thinking".
    pub name: String,
    /// What it worked on: a file, a command, an address.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// Where in the message's text it happened (UTF-16 units, as the chat page counts).
    #[serde(default)]
    pub at: usize,
    /// "" while it runs | "done" | "error" | "stopped".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub state: String,
    /// What it printed or returned (clipped), or the reasoning for "thinking".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output: String,
    /// An edit's text before and after.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub before: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub after: String,
    /// A to-do list the agent keeps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todos: Vec<Todo>,
    /// When it started (milliseconds since 1970) and how long it took.
    #[serde(default)]
    pub started_ms: u64,
    #[serde(default)]
    pub ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Todo {
    pub text: String,
    /// "pending" | "in_progress" | "completed"
    pub state: String,
}

/// Longest text kept for one step's output or edit.
const STEP_TEXT: usize = 12_000;

/// Keep the start and the end of long text (errors are usually at the end).
pub fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max / 3).collect();
    let tail: Vec<char> = s.chars().rev().take(max * 2 / 3).collect();
    format!("{head}\n… ({} characters left out) …\n{}", s.len() - max, tail.into_iter().rev().collect::<String>())
}

/// A shell command without the `bash -lc '…'` wrapper Codex puts around it.
fn bare_command(c: &str) -> String {
    let c = c.trim();
    for w in ["bash -lc ", "/bin/bash -lc ", "zsh -lc ", "/bin/zsh -lc ", "sh -c "] {
        if let Some(rest) = c.strip_prefix(w) {
            let r = rest.trim();
            let q = r.chars().next().filter(|q| *q == '\'' || *q == '"');
            return match q {
                Some(q) if r.len() > 1 && r.ends_with(q) => r[1..r.len() - 1].to_string(),
                _ => r.to_string(),
            };
        }
    }
    c.to_string()
}

/// A step from a tool call: what it worked on, an edit's before and after, a to-do list.
pub fn step_for(id: &str, name: &str, input: &Value) -> Step {
    let name = name.trim_start_matches("mcp__valerie__").to_string();
    let s = |k: &str| input[k].as_str().unwrap_or("").to_string();
    let first = |keys: &[&str]| keys.iter().map(|k| s(k)).find(|v| !v.is_empty()).unwrap_or_default();
    let mut step = Step { id: id.to_string(), ..Default::default() };
    match name.as_str() {
        "Bash" => step.detail = s("command"),
        "Edit" => {
            step.detail = s("file_path");
            step.before = clip(&s("old_string"), STEP_TEXT);
            step.after = clip(&s("new_string"), STEP_TEXT);
        }
        "MultiEdit" => {
            step.detail = s("file_path");
            let edits = input["edits"].as_array().cloned().unwrap_or_default();
            let join = |k: &str| edits.iter().filter_map(|e| e[k].as_str()).collect::<Vec<_>>().join("\n⋯\n");
            step.before = clip(&join("old_string"), STEP_TEXT);
            step.after = clip(&join("new_string"), STEP_TEXT);
        }
        "Write" => {
            step.detail = s("file_path");
            step.after = clip(&s("content"), STEP_TEXT);
        }
        "TodoWrite" => {
            step.todos = input["todos"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|t| Todo { text: t["content"].as_str().unwrap_or("").to_string(), state: t["status"].as_str().unwrap_or("pending").to_string() })
                .collect();
        }
        "Grep" => step.detail = [s("pattern"), s("path")].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" in "),
        "run_command" => step.detail = input["command"]["cmd"].as_str().unwrap_or("").to_string(),
        _ => step.detail = first(&["file_path", "notebook_path", "path", "pattern", "url", "query", "description", "command", "name", "key", "text", "selector"]),
    }
    if step.detail.is_empty() {
        if let Some(t) = input["tab"].as_u64() {
            step.detail = format!("tab {t}");
        }
    }
    step.detail = step.detail.lines().next().unwrap_or("").chars().take(300).collect();
    step.name = name;
    step
}

/// The text of a tool's result (Claude gives a string or a list of blocks).
fn result_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|p| match p["type"].as_str() {
                Some("text") => p["text"].as_str().unwrap_or("").to_string(),
                Some("image") => "[image]".into(),
                _ => String::new(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// GUI apps get a minimal PATH, so look where CLIs are usually installed too.
pub fn search_path() -> String {
    let home = dirs::home_dir().unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    for extra in [".local/bin", ".bun/bin", ".npm-global/bin", ".cargo/bin", ".opencode/bin", ".hermes/bin"] {
        dirs.push(home.join(extra));
    }
    // Where the Windows installers put them: they add these to PATH, but only for programs started afterwards.
    if cfg!(windows) {
        for (base, sub) in [("LOCALAPPDATA", r"Programs\OpenAI\Codex\bin"), ("LOCALAPPDATA", "cursor-agent"), ("APPDATA", "npm"), ("ProgramFiles", r"Git\cmd")] {
            if let Some(b) = std::env::var_os(base) {
                dirs.push(PathBuf::from(b).join(sub));
            }
        }
        // Where Codex's installer really puts it, when CODEX_INSTALL_DIR moved the folder it adds to PATH.
        let codex_home = std::env::var_os("CODEX_HOME").map_or_else(|| home.join(".codex"), PathBuf::from);
        dirs.push(codex_home.join(r"packages\standalone\current\bin"));
    }
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        dirs.push(d); // so agents and scripts can run `valerie …`
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    // Store apps' commands come first on most PATHs, but a packaged Codex can be denied its own helpers: a real install wins.
    if cfg!(windows) {
        dirs.sort_by_key(|d| is_store_dir(d));
    }
    std::env::join_paths(dirs).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn find_bin(kind: &Kind, path: &str) -> Option<PathBuf> {
    // Windows programs carry an extension: codex.exe, or agent.cmd from npm and Cursor's installer.
    let names = |b: &str| if cfg!(windows) { ["exe", "cmd", "bat"].map(|e| format!("{b}.{e}")).to_vec() } else { vec![b.to_string()] };
    kind.bins.iter().find_map(|b| std::env::split_paths(path).find_map(|d| names(b).iter().map(|n| d.join(n)).find(|p| is_program(p))))
}

/// Store apps' commands (WindowsApps) are special links that `is_file` can't follow, though they run fine.
fn is_program(p: &Path) -> bool {
    p.is_file() || (cfg!(windows) && std::fs::symlink_metadata(p).is_ok_and(|m| !m.is_dir()))
}

fn is_store_dir(d: &Path) -> bool {
    d.to_string_lossy().to_lowercase().contains("windowsapps")
}

/// What to start for `bin`, and the arguments that go first. Windows refuses to hand a .cmd a multi-line argument (every
/// new chat's prompt is one), so npm's launchers are skipped by starting what they would.
pub fn program_for(bin: &Path, path: &str) -> (PathBuf, Vec<PathBuf>) {
    let direct = || (bin.to_path_buf(), vec![]);
    if !is_batch(bin) {
        return direct();
    }
    let Some(target) = std::fs::read_to_string(bin).ok().and_then(|text| npm_shim_target(&text)) else { return direct() };
    let dir = bin.parent().unwrap_or(Path::new(""));
    let target = dir.join(target.replace('\\', std::path::MAIN_SEPARATOR_STR));
    if !target.is_file() {
        return direct();
    }
    match target.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref() {
        Some("exe") => (target, vec![]),
        Some("js" | "cjs" | "mjs") => {
            let node = std::iter::once(dir.to_path_buf()).chain(std::env::split_paths(path)).map(|d| d.join("node.exe")).find(|p| p.is_file());
            node.map_or_else(direct, |node| (node, vec![target]))
        }
        _ => direct(),
    }
}

/// The script an npm launcher starts, relative to the launcher's folder.
fn npm_shim_target(text: &str) -> Option<String> {
    let (at, prefix) = ["\"%dp0%\\", "\"%~dp0\\"].iter().filter_map(|p| Some((text.rfind(p)?, *p))).max_by_key(|(at, _)| *at)?;
    let rest = &text[at + prefix.len()..];
    Some(rest[..rest.find('"')?].to_string())
}

fn is_batch(program: &Path) -> bool {
    program.extension().is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"))
}

/// A prompt `program` can be given as an argument: a .cmd that isn't npm's can't take line breaks.
pub fn prompt_for<'a>(program: &Path, prompt: &'a str) -> std::borrow::Cow<'a, str> {
    if is_batch(program) && prompt.contains(['\r', '\n']) {
        crate::log::line("agents", format!("{} can't take a multi-line prompt: sending it as one line", program.display()));
        return prompt.replace(['\r', '\n'], " ").into();
    }
    prompt.into()
}

/// How an agent without MCP support drives the browser: Valerie's own command line.
fn cli_tools_note(exe: &str) -> String {
    let tools: Vec<String> = crate::mcp::tools().iter().filter_map(|t| t["name"].as_str().map(str::to_string)).collect();
    format!(
        "Control the browser by running shell commands of the form `\"{exe}\" call <tool> '<json arguments>'` (it prints \
         the result). Tools: {}. Start with `\"{exe}\" call list_tabs '{{}}'`.",
        tools.join(", ")
    )
}

/// How one run may behave, beyond what the agent definition says.
#[derive(Clone, Debug)]
pub struct Opts {
    /// In the browser and for the CLI's own tools: "read" (only look) | "ask" (the user approves each action) | "full".
    pub access: String,
    /// The chat this run belongs to (so Valerie can show which chat is asking).
    pub chat: String,
    /// Reasoning effort: "" (the CLI's default) | "low" | "medium" | "high" | "max".
    pub effort: String,
    /// Propose a plan and wait instead of acting.
    pub plan: bool,
    /// Keep the user's own MCP servers, plugins and skills alongside Valerie's tools.
    pub own_tools: bool,
    /// Images to show the agent with the prompt (Codex takes them as files; others are told the paths).
    pub images: Vec<PathBuf>,
}

impl Default for Opts {
    fn default() -> Self {
        Opts { access: "full".into(), chat: String::new(), effort: String::new(), plan: false, own_tools: true, images: vec![] }
    }
}

/// Build the command for one turn. `session` continues an earlier conversation where the CLI supports it.
pub fn build(def: &AgentDef, mode: Mode, prompt: &str, session: Option<&str>, stream: bool, work: &Path, o: &Opts) -> Result<(Command, Format)> {
    let path = search_path();
    let exe = std::env::current_exe()?.to_string_lossy().to_string();
    let model = def.model.as_str();
    // Plan mode never acts, whatever the chat's access.
    let access = if o.plan || mode == Mode::PageOnly { "read" } else { o.access.as_str() };
    let effort = o.effort.as_str();
    if !def.command.is_empty() {
        if !crate::trust::program_allowed(&crate::trust::load(), &def.command) {
            anyhow::bail!("this AI's program came from another machine through sync and hasn't been allowed here: run \"Allow Plugins from Other Machines\" from the command bar");
        }
        // Any program: `{prompt}` and `{model}` in its arguments are filled in; it prints the answer.
        let mut c = crate::platform::command(&def.command[0]);
        c.args(def.command[1..].iter().map(|a| a.replace("{prompt}", prompt).replace("{model}", model)));
        c.env("PATH", &path).env("VALERIE_ACCESS", access).env("VALERIE_CHAT", &o.chat).current_dir(work);
        return Ok((c, Format::Text));
    }
    let k = kind(&def.cli).with_context(|| format!("unknown agent CLI `{}` (use claude, codex, opencode, pi, cursor or hermes, or `command`)", def.cli))?;
    let bin = find_bin(k, &path).with_context(|| format!("{} isn't installed: run \"Set Up {}\" from the command bar", k.title, k.title))?;
    let (program, first_args) = program_for(&bin, &path);
    let prompt = prompt_for(&program, prompt);
    let prompt = prompt.as_ref();
    let mut c = crate::platform::command(&program);
    c.args(first_args);
    // Some CLIs take their folder from PWD (inherited from whoever started Valerie) rather than the real one.
    c.current_dir(work).env("PWD", work);
    // Valerie's MCP server, told this run's access level (some CLIs start MCP servers with a scrubbed environment).
    let mcp_args = json!(["mcp-stdio", "--access", access, "--chat", o.chat]);
    let mcp_json = || json!({ "mcpServers": { "valerie": { "command": exe, "args": mcp_args } } });
    let format = match k.id {
        "claude" => {
            c.args(["-p", prompt]);
            if stream {
                c.args(["--output-format", "stream-json", "--verbose", "--include-partial-messages"]);
            }
            match mode {
                Mode::PageOnly => {
                    c.args(["--tools", "", "--strict-mcp-config"]);
                }
                Mode::Customize => {
                    let check = format!("Bash({exe} check-config)");
                    let call = format!("Bash({exe} call:*)");
                    // The plugin marketplace: look and install, but sharing is the user's call.
                    let search = format!("Bash({exe} plugins search:*)");
                    let install = format!("Bash({exe} plugins install:*)");
                    c.args(["--mcp-config", &mcp_json().to_string(), "--strict-mcp-config", "--permission-mode", "acceptEdits"]);
                    c.args(["--allowedTools", "Read", "Edit", "Write", "Glob", "Grep", &check, &call, &search, &install, "mcp__valerie"]);
                }
                Mode::Browser => {
                    c.args(["--mcp-config", &mcp_json().to_string()]);
                    if !o.own_tools {
                        c.arg("--strict-mcp-config");
                    }
                    // Valerie's tools check access themselves; Claude's own tools follow the chat's access.
                    match access {
                        _ if o.plan => {
                            c.args(["--permission-mode", "plan", "--allowedTools", "mcp__valerie"]);
                        }
                        "full" => {
                            c.arg("--dangerously-skip-permissions");
                        }
                        "ask" => {
                            c.args(["--permission-prompts", "host", "--permission-prompt-tool", "mcp__valerie__approve", "--allowedTools", "mcp__valerie"]);
                        }
                        // Read only: Claude's reading tools and Valerie's (which refuse to act); anything else is turned down.
                        _ => {
                            c.args(["--permission-mode", "dontAsk", "--allowedTools", "mcp__valerie", "Read", "Glob", "Grep", "WebSearch", "WebFetch"]);
                        }
                    }
                }
            }
            if !effort.is_empty() {
                c.args(["--effort", effort]);
            }
            if let Some(s) = session {
                c.args(["--resume", s]);
            }
            if !model.is_empty() {
                c.args(["--model", model]);
            }
            Format::Claude
        }
        "codex" => {
            c.arg("exec");
            if stream {
                c.arg("--json");
            }
            c.arg("--skip-git-repo-check");
            match (mode, access) {
                (Mode::Customize, _) => {
                    c.args(["-s", "workspace-write"]);
                }
                (Mode::Browser, "full") => {
                    c.arg("--dangerously-bypass-approvals-and-sandbox");
                }
                (Mode::Browser, "ask") => {
                    c.args(["-s", "workspace-write"]);
                }
                _ => {
                    c.args(["-s", "read-only"]);
                }
            }
            if mode == Mode::PageOnly {
                // Read-only sandbox (its commands can't write or reach the network), no web search, none of the user's MCP servers.
                c.args(["-c", r#"web_search="disabled""#, "-c", "mcp_servers={}"]);
            } else {
                c.args(["-c", &format!("mcp_servers.valerie.command={}", serde_json::to_string(&exe)?)]);
                c.args(["-c", &format!("mcp_servers.valerie.args={}", mcp_args)]);
            }
            if !effort.is_empty() {
                let e = if effort == "max" { "high" } else { effort };
                c.args(["-c", &format!("model_reasoning_effort=\"{e}\"")]);
            }
            if !model.is_empty() {
                c.args(["-m", model]);
            }
            for i in &o.images {
                c.arg("-i").arg(i);
            }
            match session {
                Some(s) => c.args(["resume", s, prompt]),
                None => c.arg(prompt),
            };
            Format::Codex
        }
        "opencode" => {
            // The folder is the process's own. OpenCode 2 talks to a shared background service unless told otherwise,
            // and that service wouldn't see this run's settings (Valerie's tools, the chat's access).
            let v2 = opencode_major(&bin) >= 2;
            // OpenCode only says on stderr, with its logs, why it's waiting (see watch_stderr).
            c.args(["run", "--print-logs", prompt]);
            if v2 {
                c.arg("--standalone");
                // It refuses anything it would have to ask about; with full access, the settings below decide.
                if access == "full" {
                    c.arg("--auto");
                }
            }
            if stream {
                c.args(["--format", "json"]);
            }
            let cfg = match mode {
                // OpenCode's free models refuse a request with no tools (it then retries forever), so the tools stay but every
                // use needs a "yes" a non-interactive run can't give: it's refused.
                Mode::PageOnly => json!({ "mcp": {}, "permission": { "*": "ask" } }),
                // Changing Valerie means editing its settings file; the only command it may run is the settings check.
                Mode::Customize => json!({
                    "mcp": { "valerie": { "type": "local", "command": [exe, "mcp-stdio", "--access", "full"], "enabled": true } },
                    "permission": { "edit": "allow", "external_directory": "deny",
                                    "bash": { "*": "deny", format!("{exe} check-config*"): "allow", "valerie check-config*": "allow",
                                              // OpenCode matches arguments word by word: "x *" covers "x anything".
                                              format!("{exe} plugins search *"): "allow", format!("{exe} plugins install *"): "allow",
                                              "valerie plugins search *": "allow", "valerie plugins install *": "allow" } },
                }),
                _ => {
                    // `opencode run` can't stop to ask, so "ask" and "read" both mean its own edits and commands are off.
                    let own = if access == "full" { "allow" } else { "deny" };
                    json!({
                        "mcp": { "valerie": { "type": "local", "command": [exe, "mcp-stdio", "--access", access, "--chat", o.chat], "enabled": true } },
                        "permission": { "edit": own, "bash": own },
                    })
                }
            };
            c.env("OPENCODE_CONFIG_CONTENT", cfg.to_string());
            if let Some(s) = session {
                c.args(["--session", s]);
            }
            // OpenCode 2 takes the reasoning effort on the model ("provider/model#high"); older ones as --variant.
            match (v2, model.is_empty(), effort.is_empty()) {
                (true, false, false) if !model.contains('#') => c.args(["-m", &format!("{model}#{effort}")]),
                (_, false, _) => c.args(["-m", model]),
                _ => &mut c,
            };
            if !v2 && !effort.is_empty() {
                c.args(["--variant", effort]);
            }
            Format::OpenCode
        }
        "cursor" => {
            c.args(["-p", "--trust"]);
            if stream {
                c.args(["--output-format", "stream-json", "--stream-partial-output"]);
            }
            if mode == Mode::PageOnly {
                // Ask mode is read-only; the sandbox keeps its commands in the (empty) working folder and offline.
                c.args(["--mode", "ask", "--sandbox", "enabled"]);
            } else {
                // Cursor reads MCP servers from the workspace folder.
                c.arg("--approve-mcps");
                let dir = work.join(".cursor");
                std::fs::create_dir_all(&dir)?;
                std::fs::write(dir.join("mcp.json"), mcp_json().to_string())?;
                match access {
                    "full" => {
                        c.arg("--force");
                    }
                    "ask" => {
                        c.args(["--sandbox", "enabled"]);
                    }
                    _ => {
                        c.args(["--mode", "ask"]);
                    }
                }
            }
            if let Some(s) = session {
                c.args(["--resume", s]);
            }
            match (model.is_empty(), effort.is_empty()) {
                (false, true) => {
                    c.args(["--model", model]);
                }
                (false, false) => {
                    c.args(["--model", &format!("{model}[effort={effort}]")]);
                }
                _ => {}
            }
            c.arg(prompt);
            Format::Cursor
        }
        "pi" => {
            c.args(["-p"]);
            if stream {
                c.args(["--mode", "json"]);
            }
            match access {
                _ if mode == Mode::PageOnly => {
                    c.arg("--no-tools");
                }
                // Looking only: Pi's read tools, plus bash for `valerie call` (read-only by Valerie's access check).
                "read" => {
                    c.args(["--tools", "read,grep,find,ls,bash"]);
                }
                _ => {}
            }
            if !model.is_empty() {
                c.args(["--model", model]);
            }
            c.arg(prompt);
            if stream { Format::Pi } else { Format::Text }
        }
        "hermes" => {
            if mode == Mode::PageOnly {
                // Only the harmless "clarify" toolset, and no AGENTS.md / memory / rules injected.
                c.args(["-t", "clarify", "--ignore-rules"]);
            }
            if !effort.is_empty() {
                c.args(["--reasoning", effort]);
            }
            // Picked from Valerie's list as "provider::model".
            match model.split_once("::") {
                Some((prov, m)) => {
                    c.args(["--provider", prov, "-m", m]);
                }
                None if !model.is_empty() => {
                    c.args(["-m", model]);
                }
                None => {}
            }
            c.args(["-z", prompt]);
            Format::Text
        }
        _ => unreachable!(),
    };
    // `valerie call` (Pi, Hermes, scripts) reads these to tell Valerie which chat and access level it's acting for.
    c.env("PATH", &path).env("VALERIE_ACCESS", access).env("VALERIE_CHAT", &o.chat).current_dir(work);
    Ok((c, format))
}

/// How this agent reaches the browser: MCP tools, or Valerie's command line for CLIs without per-run MCP.
pub fn control_note(def: &AgentDef) -> String {
    let exe = std::env::current_exe().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|_| "valerie".into());
    if matches!(def.cli.as_str(), "pi" | "hermes") || !def.command.is_empty() {
        cli_tools_note(&exe)
    } else {
        "Use the `valerie` MCP tools to see and control the browser.".to_string()
    }
}

/// Where and when the agent is running and what the chat's access lets it do, for both chat preambles.
fn situation(access: &str) -> String {
    let os = if cfg!(target_os = "macos") { "macOS" } else if cfg!(windows) { "Windows" } else { "Linux" };
    let (date, _, day, time) = crate::platform::local_day();
    let access = match access {
        "read" => " This chat is read-only: you can look (pages, tabs, settings, files) but not click, type, open tabs, edit \
                   files or change anything else, so say what you'd do instead.",
        "ask" => " The user approves each action you take in this chat, so do only what the request needs.",
        _ => "",
    };
    format!(" The computer runs {os}; it's {day} ({date}), {time} local time when this chat started.{access}")
}

/// The prompt preamble for an agent with browser control.
pub fn browser_preamble(def: &AgentDef, context: &str, offer_share: bool, scratch: &Path, access: &str) -> String {
    let how = control_note(def);
    let guide = crate::config::config_dir().join("AGENTS.md");
    let share = if offer_share {
        " Once you've made a plugin or browser extension and it works, ask the user whether they'd like to share it in \
         the public marketplace so others can use it; only if they say yes, share it as the guide explains."
    } else {
        ""
    };
    format!(
        "You are the assistant built into the user's Valerie web browser. You can see and control their browser. {how} \
         Open any tabs you need in the background and don't close or change the user's tabs unless they ask. Never submit \
         purchases, payments or other irreversible forms without the user saying so in this conversation; stop and ask \
         instead. When something in Valerie didn't work, read its log with read_logs; to change one of Valerie's settings, \
         use get_settings and set_setting. To add something no setting covers (like changing how a site behaves), make a \
         Valerie plugin (it works on every platform; a browser extension only on macOS 15.4+): read the guide at {} \
         first, and search the marketplace before making a new one.{share}{situation} Your working folder, {}, is a scratch \
         folder the user never looks in: save files they should keep in their Downloads folder (or where they say) and give \
         the full path. Answer in plain, short Markdown.{context}",
        guide.display(),
        scratch.display(),
        situation = situation(access),
        context = if context.is_empty() {
            String::new()
        } else {
            format!(
                "\n\n{context}\"This page\" or \"this tab\" means the active one. The list is from when the chat started; \
                 call list_tabs for the tabs as they are now."
            )
        }
    )
}

/// The prompt preamble for a chat working in a project folder: a coding agent that can also use the browser.
pub fn coding_preamble(def: &AgentDef, folder: &Path, access: &str) -> String {
    let how = control_note(def);
    format!(
        "You are a coding agent working in the project folder {}. The user talks to you from the chat built into their \
         Valerie web browser, which shows your steps, edits and commands as you go. Work as you would in a terminal: read \
         the code, make the change, run the tests or the build, and say plainly what you did and what's left. You can also \
         see and control the browser ({how}): use it to check what you build (open the dev server in a background tab, \
         take a screenshot, read the page, read_console for its errors). Don't close or change the user's own tabs unless \
         they ask, and never submit purchases, payments or other irreversible forms without the user saying so here. \
         \"This page\" means the user's active tab (marked in list_tabs).{} Answer in plain, short Markdown.",
        folder.display(),
        situation(access)
    )
}

/// Turn one line of CLI output into answer pieces.
pub fn parse(format: Format, line: &str) -> Vec<Ev> {
    let line = line.trim_end();
    if line.is_empty() {
        return vec![];
    }
    if let Format::Text = format {
        return vec![Ev::Delta { text: format!("{line}\n") }];
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else { return vec![] };
    let s = |p: &Value| p.as_str().map(str::to_string);
    let mut out = Vec::new();
    match format {
        // A subagent's own steps and words stay inside its one step.
        Format::Claude if v["parent_tool_use_id"].is_string() => {}
        Format::Claude => match v["type"].as_str() {
            Some("system") if v["subtype"] == "init" => out.extend(s(&v["session_id"]).map(|id| Ev::Session { id })),
            Some("stream_event") => {
                let e = &v["event"];
                if e["type"] == "content_block_delta" && e["delta"]["type"] == "text_delta" {
                    out.extend(s(&e["delta"]["text"]).map(|text| Ev::Delta { text }));
                } else if e["type"] == "content_block_delta" && e["delta"]["type"] == "thinking_delta" {
                    out.extend(s(&e["delta"]["thinking"]).map(|text| Ev::Thinking { text }));
                } else if e["type"] == "content_block_start" && e["content_block"]["type"] == "tool_use" {
                    // ToolSearch is Claude loading tool definitions, not something it did.
                    let b = &e["content_block"];
                    if let Some(n) = s(&b["name"]).filter(|n| n != "ToolSearch") {
                        out.push(Ev::Step { step: step_for(b["id"].as_str().unwrap_or(""), &n, &json!({})) });
                    }
                } else if e["type"] == "message_start" {
                    // A new message after tool use: keep paragraphs apart.
                    out.push(Ev::Block { text: String::new() });
                }
            }
            // The whole message once a block is complete: the tool's input is all there now.
            Some("assistant") => {
                let m = &v["message"];
                for b in m["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "tool_use" && b["name"] != "ToolSearch") {
                    out.push(Ev::Step { step: step_for(b["id"].as_str().unwrap_or(""), b["name"].as_str().unwrap_or("tool"), &b["input"]) });
                }
                let u = &m["usage"];
                let context = ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"].iter().filter_map(|k| u[*k].as_u64()).sum::<u64>();
                if context > 0 {
                    out.push(Ev::Usage { context, output: 0 });
                }
            }
            Some("user") => {
                for b in v["message"]["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "tool_result") {
                    let id = b["tool_use_id"].as_str().unwrap_or("").to_string();
                    out.push(Ev::StepDone { id, output: clip(&result_text(&b["content"]), STEP_TEXT), error: b["is_error"] == true });
                }
            }
            Some("result") => {
                if let Some(o) = v["usage"]["output_tokens"].as_u64() {
                    out.push(Ev::Usage { context: 0, output: o });
                }
                if v["is_error"] == true {
                    out.push(Ev::Error { text: s(&v["result"]).unwrap_or_else(|| "the agent failed".into()) });
                } else {
                    out.extend(s(&v["result"]).map(|text| Ev::Final { text }));
                }
            }
            _ => {}
        },
        Format::Cursor => match v["type"].as_str() {
            Some("system") => out.extend(s(&v["session_id"]).map(|id| Ev::Session { id })),
            // With partial output each delta carries a timestamp; the closing whole message doesn't.
            Some("assistant") if v.get("timestamp_ms").is_some() => {
                for c in v["message"]["content"].as_array().into_iter().flatten() {
                    out.extend(s(&c["text"]).map(|text| Ev::Delta { text }));
                }
            }
            Some("tool_call") if v["subtype"] == "started" => {
                let name = v["tool_call"].as_object().and_then(|o| o.keys().next().cloned()).unwrap_or_else(|| "tool".into());
                out.push(Ev::Tool { text: name.trim_end_matches("ToolCall").to_string() });
            }
            Some("result") => {
                if v["is_error"] == true {
                    out.push(Ev::Error { text: s(&v["result"]).unwrap_or_else(|| "the agent failed".into()) });
                } else {
                    out.extend(s(&v["result"]).map(|text| Ev::Final { text }));
                }
            }
            _ => {}
        },
        Format::Codex => {
            let item = &v["item"];
            let id = item["id"].as_str().unwrap_or("").to_string();
            match (v["type"].as_str(), item["type"].as_str()) {
                (Some("thread.started"), _) => out.extend(s(&v["thread_id"]).map(|id| Ev::Session { id })),
                (Some("item.started"), Some("command_execution")) => {
                    out.push(Ev::Step { step: Step { id, name: "Bash".into(), detail: bare_command(item["command"].as_str().unwrap_or("")), ..Default::default() } })
                }
                (Some("item.completed"), Some("command_execution")) => {
                    let failed = item["exit_code"].as_i64().is_some_and(|c| c != 0) || item["status"] == "failed";
                    out.push(Ev::StepDone { id, output: clip(item["aggregated_output"].as_str().unwrap_or(""), STEP_TEXT), error: failed })
                }
                (Some("item.started"), Some("mcp_tool_call")) => {
                    let name = item["tool"].as_str().unwrap_or("tool");
                    out.push(Ev::Step { step: step_for(&id, name, &item["arguments"]) })
                }
                (Some("item.completed"), Some("mcp_tool_call")) => {
                    let output = result_text(&item["result"]["content"]);
                    let error = item["status"] == "failed" || item["error"].is_object();
                    let output = if output.is_empty() { item["error"]["message"].as_str().unwrap_or("").to_string() } else { output };
                    out.push(Ev::StepDone { id, output: clip(&output, STEP_TEXT), error })
                }
                (Some("item.started"), Some("web_search")) => {
                    out.push(Ev::Step { step: Step { id, name: "WebSearch".into(), detail: item["query"].as_str().unwrap_or("").into(), ..Default::default() } })
                }
                (Some("item.completed"), Some("web_search")) => out.push(Ev::StepDone { id, output: String::new(), error: false }),
                (Some("item.completed"), Some("file_change")) => {
                    let files: Vec<&str> = item["changes"].as_array().into_iter().flatten().filter_map(|c| c["path"].as_str()).collect();
                    let state = if item["status"] == "failed" { "error" } else { "done" };
                    out.push(Ev::Step { step: Step { id, name: "Edit".into(), detail: files.join(", "), state: state.into(), ..Default::default() } })
                }
                (Some("item.started" | "item.updated" | "item.completed"), Some("todo_list")) => {
                    let todos = item["items"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|t| Todo { text: t["text"].as_str().unwrap_or("").into(), state: if t["completed"] == true { "completed" } else { "pending" }.into() })
                        .collect();
                    out.push(Ev::Step { step: Step { id, name: "TodoWrite".into(), todos, state: "done".into(), ..Default::default() } })
                }
                (Some("item.completed"), Some("reasoning")) => out.extend(s(&item["text"]).map(|text| Ev::Thinking { text })),
                (Some("item.completed"), Some("agent_message")) => out.extend(s(&item["text"]).map(|text| Ev::Block { text })),
                (Some("turn.completed"), _) => out.push(Ev::Usage { context: 0, output: v["usage"]["output_tokens"].as_u64().unwrap_or(0) }),
                (Some("error") | Some("turn.failed"), _) => {
                    let raw = s(&v["message"]).or_else(|| s(&v["error"]["message"])).unwrap_or_else(|| "the agent failed".into());
                    // Codex wraps API errors as JSON inside the message.
                    let msg = serde_json::from_str::<Value>(&raw).ok().and_then(|e| s(&e["error"]["message"])).unwrap_or(raw);
                    out.push(Ev::Error { text: msg });
                }
                _ => {}
            }
        }
        Format::OpenCode => {
            out.extend(s(&v["sessionID"]).map(|id| Ev::Session { id }));
            match v["type"].as_str() {
                Some("text") => out.extend(s(&v["part"]["text"]).map(|text| Ev::Block { text })),
                Some("tool_use") => out.extend(s(&v["part"]["tool"]).map(|t| Ev::Tool { text: t.trim_start_matches("valerie_").to_string() })),
                Some("error") => out.push(Ev::Error {
                    text: s(&v["error"]["data"]["message"]).or_else(|| s(&v["error"]["message"])).unwrap_or_else(|| "the agent failed".into()),
                }),
                _ => {}
            }
        }
        Format::Pi => match v["type"].as_str() {
            Some("message_update") => {
                let e = &v["assistantMessageEvent"];
                if e["type"] == "text_delta" {
                    out.extend(s(&e["delta"]).map(|text| Ev::Delta { text }));
                }
            }
            Some("tool_execution_start") => out.extend(s(&v["toolName"]).map(|text| Ev::Tool { text })),
            _ => {}
        },
        Format::Text => {}
    }
    out
}

/// Running agent processes by run id (a chat id, or "page:<tab>"), so they can be stopped.
static RUNNING: std::sync::Mutex<Vec<(String, u32)>> = std::sync::Mutex::new(Vec::new());

pub fn stop(run: &str) -> bool {
    let pid = RUNNING.lock().ok().and_then(|r| r.iter().find(|(n, _)| n == run).map(|(_, p)| *p));
    pid.is_some_and(kill_process)
}

fn kill_process(pid: u32) -> bool {
    #[cfg(unix)]
    let ok = crate::platform::command("kill").arg(pid.to_string()).status().is_ok_and(|s| s.success());
    #[cfg(windows)]
    let ok = crate::platform::command("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).status().is_ok_and(|s| s.success());
    ok
}

const FREE_LIMIT: &str = "OpenCode's free AI has reached its usage limit for now. Try again later, or pick another AI.";

/// Read an agent's stderr to the end, leaving out OpenCode's log lines. When OpenCode says its free models are at their
/// limit it keeps retrying without a word, so stop it there.
fn watch_stderr(err: Option<std::process::ChildStderr>, limited: std::sync::Arc<std::sync::atomic::AtomicBool>, pid: Option<u32>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut kept = String::new();
        let Some(err) = err else { return kept };
        for line in BufReader::new(err).lines().map_while(Result::ok) {
            if line.contains("Free usage limit reached") && !limited.swap(true, std::sync::atomic::Ordering::Relaxed) {
                crate::log::line("agents", "OpenCode's free models are at their usage limit");
                if let Some(pid) = pid {
                    kill_process(pid);
                }
            }
            if !line.starts_with("timestamp=") {
                kept.push_str(&line);
                kept.push('\n');
            }
        }
        kept
    })
}

/// Run one turn to the end (blocking), handing each piece of the answer to `on` as it arrives.
pub fn stream(run: &str, mut cmd: Command, format: Format, timeout: Duration, on: &dyn Fn(Ev)) -> Result<()> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().context("couldn't start the agent")?;
    if let Ok(mut r) = RUNNING.lock() {
        r.push((run.to_string(), child.id()));
    }
    let limited = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let errs = watch_stderr(child.stderr.take(), limited.clone(), Some(child.id()));
    let start = Instant::now();
    let mut got = false;
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            for ev in parse(format, &line) {
                got |= matches!(ev, Ev::Delta { .. } | Ev::Block { .. } | Ev::Final { .. });
                on(ev);
            }
            if start.elapsed() > timeout {
                let _ = child.kill();
                on(Ev::Error { text: format!("stopped after {} minutes", timeout.as_secs() / 60) });
                break;
            }
        }
    }
    let status = child.wait();
    if let Ok(mut r) = RUNNING.lock() {
        r.retain(|(n, _)| n != run);
    }
    let stderr = errs.join().unwrap_or_default();
    if limited.load(std::sync::atomic::Ordering::Relaxed) && !got {
        on(Ev::Error { text: FREE_LIMIT.into() });
    } else if !status.is_ok_and(|s| s.success()) && !got {
        let tail: String = stderr.lines().rev().filter(|l| !l.trim().is_empty()).take(4).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
        on(Ev::Error { text: if tail.is_empty() { "the agent stopped without answering".into() } else { tail } });
    }
    Ok(())
}

/// Open a terminal that installs the CLI if it's missing, then signs in: the one step a user does per agent.
/// Whether an agent CLI answers right now (installed, signed in, its plan active): a one-line question, no tools.
pub fn probe(def: &AgentDef) -> Result<()> {
    let work = crate::config::data_dir().join("agents").join("probe");
    std::fs::create_dir_all(&work)?;
    let (mut cmd, _) = build(def, Mode::PageOnly, "Reply with exactly the word OK and nothing else.", None, false, &work, &Opts::default())?;
    let out = run_with_timeout(cmd.current_dir(&work), Duration::from_secs(90))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if out.status.success() && text.to_uppercase().contains("OK") {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr).to_string() + &text;
    let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it didn't answer").trim().chars().take(200).collect::<String>();
    bail!("{last}")
}

/// Free models OpenCode offers without an account, best first; anything else ending in "-free" comes after.
/// (Nemotron 3 tests best in the community comparisons; the newer flash models answered well when checked.)
pub const FREE_MODELS: &[&str] = &["nemotron-3-ultra-free", "deepseek-v4.1-flash-free", "glm-5.3-flash-free", "mimo-v2.6-flash-free", "beam-free"];

/// The model and reasoning effort a CLI uses when Valerie doesn't name one, read from its own settings
/// (empty when it doesn't say, which means the CLI decides).
pub fn cli_defaults(cli: &str) -> (String, String) {
    let home = dirs::home_dir().unwrap_or_default();
    let read = |rel: &str| std::fs::read_to_string(home.join(rel)).unwrap_or_default();
    let json = |rel: &str| serde_json::from_str::<Value>(&read(rel)).unwrap_or(Value::Null);
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    match cli {
        "claude" => {
            let j = json(".claude/settings.json");
            let model = s(&j["model"]);
            // Effort is kept per full model id ("claude-opus-5-5"); the setting may be an alias ("opus").
            let effort = j["modelSettings"].as_object().and_then(|m| {
                m.iter().find(|(id, _)| !model.is_empty() && (id.as_str() == model || id.contains(&format!("-{model}-")))).map(|(_, v)| s(&v["effortLevel"]))
            });
            (model, effort.unwrap_or_default())
        }
        "codex" => {
            let toml = read(".codex/config.toml");
            let key = |k: &str| {
                toml.lines().take_while(|l| !l.trim_start().starts_with('[')).find_map(|l| {
                    let (a, b) = l.split_once('=')?;
                    (a.trim() == k).then(|| b.trim().trim_matches('"').to_string())
                })
            };
            (key("model").unwrap_or_default(), key("model_reasoning_effort").unwrap_or_default())
        }
        "opencode" => {
            let j = [".config/opencode/opencode.json", ".config/opencode/config.json"].iter().map(|f| json(f)).find(|j| !j.is_null()).unwrap_or(Value::Null);
            (s(&j["model"]), String::new())
        }
        "cursor" => {
            let j = json(".cursor/cli-config.json");
            let m = &j["model"];
            (if m["displayName"].is_string() { s(&m["displayName"]) } else { s(m) }, String::new())
        }
        "hermes" => {
            let y: serde_yaml::Value = serde_yaml::from_str(&read(".hermes/config.yaml")).unwrap_or(serde_yaml::Value::Null);
            let m = &y["model"];
            let name = m.as_str().or_else(|| m["default"].as_str()).or_else(|| m["model"].as_str()).unwrap_or("");
            (name.to_string(), String::new())
        }
        _ => (String::new(), String::new()),
    }
}

/// OpenCode's major version ("opencode v2.0.32" → 2; older ones print "1.0.3"), asked once.
fn opencode_major(bin: &Path) -> u32 {
    static MAJOR: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *MAJOR.get_or_init(|| {
        let out = crate::platform::command(bin).arg("--version").stdin(std::process::Stdio::null()).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        version_major(&out)
    })
}

fn version_major(text: &str) -> u32 {
    text.split(|c: char| !c.is_ascii_digit() && c != '.').find(|w| w.contains('.')).and_then(|w| w.split('.').next()?.parse().ok()).unwrap_or(1)
}

/// For people without an AI subscription: install OpenCode (no Terminal needed) and find a free model that answers.
/// Returns that model ("opencode/…"). `step` hears what's happening.
pub fn set_up_free(step: &dyn Fn(&str)) -> Result<String> {
    let path = search_path();
    let k = kind("opencode").context("no OpenCode")?;
    if find_bin(k, &path).is_none() {
        step("Installing OpenCode…");
        install_quietly(k)?;
    }
    step("Finding a free model…");
    let bin = find_bin(k, &search_path()).context("OpenCode isn't installed")?;
    let mut c = crate::platform::command(bin);
    // Every provider's models (OpenCode 2 can't list just one); its own free ones are "opencode/…-free".
    c.arg("models").env("PATH", &path).stdin(std::process::Stdio::null());
    let listed = String::from_utf8_lossy(&run_with_timeout(&mut c, Duration::from_secs(60))?.stdout).to_string();
    let free: Vec<&str> = listed.lines().map(str::trim).filter(|m| m.starts_with("opencode/") && m.ends_with("-free")).collect();
    let mut order: Vec<String> = FREE_MODELS.iter().map(|m| format!("opencode/{m}")).filter(|m| free.contains(&m.as_str())).collect();
    let rest: Vec<String> = free.iter().map(|m| m.to_string()).filter(|m| !order.contains(m)).collect();
    order.extend(rest);
    if order.is_empty() {
        bail!("OpenCode lists no free models right now");
    }
    let mut at_limit = false;
    for m in order.iter().take(4) {
        step(&format!("Trying {}…", m.trim_start_matches("opencode/")));
        match probe(&AgentDef { cli: "opencode".into(), model: m.clone(), command: vec![] }) {
            Ok(()) => return Ok(m.clone()),
            Err(e) => {
                at_limit |= e.to_string().contains(FREE_LIMIT);
                crate::log::line("setup", format!("free model {m} didn't answer: {e:#}"));
            }
        }
    }
    if at_limit {
        bail!("{FREE_LIMIT}");
    }
    bail!("none of OpenCode's free models answered; try again in a minute")
}

/// Install an AI app with no window to look at. The error says why it failed, in plain words.
pub fn install_quietly(k: &Kind) -> Result<()> {
    let mut c = if cfg!(windows) { crate::platform::command("powershell") } else { crate::platform::command("/bin/bash") };
    if cfg!(windows) {
        c.args(["-NoProfile", "-OutputFormat", "Text", "-ExecutionPolicy", "Bypass", "-Command"]);
    } else {
        c.arg("-c");
    }
    // Nobody can answer Codex's installer, which asks to start Codex when it's done.
    c.arg(k.install_command()).env("PATH", search_path()).env("CODEX_NON_INTERACTIVE", "1");
    let out = run_with_timeout(&mut c, Duration::from_secs(600))?;
    let said = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        crate::log::line("setup", format!("{} installer said: {}", k.title, said.chars().rev().take(2000).collect::<Vec<_>>().into_iter().rev().collect::<String>()));
        let why = String::from_utf8_lossy(&out.stderr).lines().rfind(|l| !l.trim().is_empty()).unwrap_or("").trim().chars().take(200).collect::<String>();
        bail!("couldn't install {}: {}", k.title, if why.is_empty() { "the installer stopped" } else { &why });
    }
    if find_bin(k, &search_path()).is_none() {
        bail!("{} is installed. Restart Valerie so it can find it.", k.title);
    }
    Ok(())
}

/// Whether an app that's just been installed has a sign-in to use already: Codex shares the Codex app's.
pub fn already_signed_in(k: &Kind) -> bool {
    let args: &[&str] = match k.id {
        "codex" => &["login", "status"],
        _ => return false,
    };
    let Some(bin) = find_bin(k, &search_path()) else { return false };
    let mut c = crate::platform::command(bin);
    c.args(args).env("PATH", search_path());
    run_with_timeout(&mut c, Duration::from_secs(20)).is_ok_and(|o| o.status.success())
}

/// Run a command, killing it after `limit`.
fn run_with_timeout(c: &mut Command, limit: Duration) -> Result<std::process::Output> {
    let mut child = c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn()?;
    let mut so = child.stdout.take();
    let limited = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let err_t = watch_stderr(child.stderr.take(), limited.clone(), None);
    let read = |p: Option<&mut dyn std::io::Read>| {
        let mut buf = Vec::new();
        if let Some(p) = p {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    };
    let out_t = std::thread::spawn(move || read(so.as_mut().map(|p| p as &mut dyn std::io::Read)));
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if limited.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = child.kill();
            bail!("{FREE_LIMIT}");
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            bail!("it took too long to answer");
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    Ok(std::process::Output { status, stdout: out_t.join().unwrap_or_default(), stderr: err_t.join().unwrap_or_default().into_bytes() })
}

/// What a set-up script does, for bash. It installs if needed, signs in, and writes how that went to `status`:
/// install_failed | login_failed | done.
#[cfg_attr(windows, allow(dead_code))]
fn set_up_script_unix(k: &Kind, path: &str, status: &Path) -> String {
    use crate::platform::shell_quote as q;
    let found = k.bins.iter().map(|b| format!("command -v {b} >/dev/null")).collect::<Vec<_>>().join(" || ");
    let (title, install, login, help) = (k.title, k.install, k.login, k.help);
    format!(
        r#"export PATH={path}
status={status}
finish() {{ printf '%s' "$1" > "$status"; echo; echo "$2"; echo; read -r -p 'Press Enter to close this window. ' _; exit; }}
echo 'Setting up {title} for Valerie'
if ! {{ {found}; }}; then
  echo 'Installing {title}…'
  {install}
  hash -r
  {{ {found}; }} || finish install_failed "{title} didn't install. Valerie has a link to its download page, or get it here: {help}"
fi
{login}
[ $? -eq 0 ] || finish login_failed "{title} is installed, but signing in didn't finish. Go back to Valerie and press Try again."
finish done 'All set: go back to Valerie.'
"#,
        path = q(path),
        status = q(&status.to_string_lossy()),
    )
}

/// The same for PowerShell. Installers run in their own PowerShell because `irm … | iex` scripts can call `exit`.
#[cfg_attr(not(windows), allow(dead_code))]
fn set_up_script_windows(k: &Kind, path: &str, status: &Path) -> String {
    let q = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let found = k.bins.join(",");
    let (title, login, help) = (k.title, k.login, k.help);
    format!(
        r#"$ErrorActionPreference = 'Stop'
$env:Path = {path}
$env:CODEX_NON_INTERACTIVE = '1'
$status = {status}
$install = {install}
function Found {{ Get-Command {found} -ErrorAction SilentlyContinue }}
function Finish($state, $text) {{
  Set-Content -LiteralPath $status -Value $state -NoNewline -Encoding Ascii
  Write-Host ''; Write-Host $text; Write-Host ''
  Read-Host 'Press Enter to close this window' | Out-Null
  exit
}}
Write-Host 'Setting up {title} for Valerie'
if (-not (Found)) {{
  Write-Host 'Installing {title}...'
  try {{ & powershell -NoProfile -ExecutionPolicy Bypass -Command $install }} catch {{ Write-Host $_ }}
  $env:Path = [Environment]::GetEnvironmentVariable('Path', 'User') + ';' + $env:Path
  if (-not (Found)) {{ Finish 'install_failed' "{title} didn't install. Valerie has a link to its download page, or get it here: {help}" }}
}}
$signedIn = $true
$global:LASTEXITCODE = 0
try {{ {login}; if ($LASTEXITCODE -ne 0) {{ $signedIn = $false }} }} catch {{ Write-Host $_; $signedIn = $false }}
if (-not $signedIn) {{ Finish 'login_failed' "{title} is installed, but signing in didn't finish. Go back to Valerie and press Try again." }}
Finish 'done' 'All set: go back to Valerie.'
"#,
        path = q(path),
        status = q(&status.to_string_lossy()),
        install = q(k.install_windows),
    )
}

/// Install if needed and sign in, in a terminal window the user can see. Returns the file the script writes its result to.
pub fn set_up(k: &Kind) -> Result<PathBuf> {
    let path = search_path();
    let dir = crate::config::data_dir().join("setup");
    std::fs::create_dir_all(&dir)?;
    let status = dir.join(format!("{}.status", k.id));
    let _ = std::fs::remove_file(&status);
    #[cfg(target_os = "macos")]
    {
        let file = dir.join(format!("{}.command", k.id));
        std::fs::write(&file, format!("#!/bin/bash\n{}", set_up_script_unix(k, &path, &status)))?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))?;
        crate::platform::command("open").args(["-a", "Terminal"]).arg(&file).spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let file = dir.join(format!("{}.sh", k.id));
        std::fs::write(&file, format!("#!/bin/bash\n{}", set_up_script_unix(k, &path, &status)))?;
        // x-terminal-emulator only exists on Debian and its relatives.
        let terminals: &[(&str, &[&str])] = &[
            ("x-terminal-emulator", &["-e"]),
            ("gnome-terminal", &["--"]),
            ("ptyxis", &["--"]),
            ("kgx", &["-e"]),
            ("konsole", &["-e"]),
            ("xfce4-terminal", &["-x"]),
            ("alacritty", &["-e"]),
            ("kitty", &[]),
            ("foot", &[]),
            ("wezterm", &["start", "--"]),
            ("xterm", &["-e"]),
        ];
        if !terminals.iter().any(|(t, args)| crate::platform::command(t).args(*args).arg("bash").arg(&file).spawn().is_ok()) {
            bail!("no terminal app found");
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x10;
        let file = dir.join(format!("{}.ps1", k.id));
        // The mark tells Windows PowerShell it's UTF-8, for user names with accents.
        std::fs::write(&file, format!("\u{feff}{}", set_up_script_windows(k, &path, &status)))?;
        Command::new("powershell").args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(&file).creation_flags(CREATE_NEW_CONSOLE).spawn()?;
    }
    Ok(status)
}

/// Wait for a set-up window to finish; None if it never does (people close windows). The error is what went wrong, in plain words.
pub fn set_up_outcome(k: &Kind, status: &Path) -> Option<Result<(), String>> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(30 * 60) {
        if let Ok(result) = std::fs::read_to_string(status) {
            return Some(match result.trim() {
                "done" if find_bin(k, &search_path()).is_none() => Err(format!("{} is installed. Restart Valerie so it can find it.", k.title)),
                "done" => Ok(()),
                "login_failed" => Err(format!("{} is installed, but signing in didn't finish.", k.title)),
                _ => Err(format!("{} didn't install. The terminal window says why.", k.title)),
            });
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    None
}

// ---------------------------------------------------------------- chats

fn is_zero(n: &u64) -> bool {
    *n == 0
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Msg {
    pub role: String,
    #[serde(default)]
    pub text: String,
    /// Tools the agent used while writing this message (chats from before steps were kept).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// What the agent did while writing it, in order, each at the place in the text where it happened.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
    /// Files sent along with the user's message (pasted images and the like).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// How long the answer took (milliseconds), how full the model's context got, and tokens written.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ms: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub context: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Which agent wrote it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent: String,
    #[serde(default)]
    pub at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Chat {
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// The agent this chat talks to (a name under agents.available).
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub updated: u64,
    /// Each agent's own conversation id, so follow-ups continue where it left off.
    #[serde(default)]
    pub sessions: std::collections::BTreeMap<String, String>,
    /// Model, reasoning effort and access chosen in the composer ("" = the defaults).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effort: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub access: String,
    /// Plan first: the agent proposes, the user says go.
    #[serde(default)]
    pub plan: bool,
    /// Old "Settled" mark: such chats are now moved to Closed.
    #[serde(default, skip_serializing)]
    pub settled: bool,
    /// Hidden until this time (seconds), then it comes back marked unread.
    #[serde(default)]
    pub snoozed_until: u64,
    #[serde(default)]
    pub unread: bool,
    /// When it was closed (it sits in the Closed list until it's reopened).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub closed_at: u64,
    /// Extra instructions for this chat's agent the user doesn't see (a "Report a Problem" chat's playbook).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub brief: String,
    /// The project folder the agent works in ("" = Valerie's own folder for chats). Fixed once the chat starts: the
    /// agent's session belongs to the folder it started in.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub folder: String,
    /// The chat's own git worktree of that folder (chats.worktrees), where its agent works; its branch and the branch it
    /// started from. Empty: the agent works in the folder itself.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub worktree: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base: String,
    /// Tools the user allowed for the rest of this chat ("Always Allow" on an Ask-first question).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed: Vec<String>,
    /// Messages sent while the agent was busy, sent in order when it's done.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queue: Vec<String>,
    #[serde(default)]
    pub messages: Vec<Msg>,
}

impl Chat {
    pub fn new(agent: &str) -> Chat {
        let now = crate::state::now();
        let id = format!("{now}-{}", &crate::mcp::random_hex()[..6]);
        Chat { id, agent: agent.into(), created: now, updated: now, ..Default::default() }
    }

    /// The whole conversation as plain text (newest kept when it's too long), for sharing with another chat.
    pub fn transcript_all(&self, max_chars: usize) -> String {
        let mut out = String::new();
        for m in self.messages.iter().rev() {
            let line = format!("{}: {}\n\n", if m.role == "user" { "User" } else { "Assistant" }, m.text.trim());
            if out.len() + line.len() > max_chars {
                break;
            }
            out.insert_str(0, &line);
        }
        out
    }

    /// Earlier turns as plain text, for agents that can't continue a session themselves.
    pub fn transcript(&self, max_chars: usize) -> String {
        let mut out = String::new();
        for m in self.messages.iter().rev().skip(2) {
            let line = format!("{}: {}\n\n", if m.role == "user" { "User" } else { "Assistant" }, m.text.trim());
            if out.len() + line.len() > max_chars {
                break;
            }
            out.insert_str(0, &line);
        }
        out
    }
}

pub fn load_chats(dir: &Path) -> Vec<Chat> {
    let mut chats: Vec<Chat> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok().and_then(|s| serde_json::from_str(&s).ok()))
        .collect();
    chats.sort_by(|a, b| b.updated.cmp(&a.updated));
    chats
}

pub fn save_chat(dir: &Path, c: &Chat) {
    let _ = std::fs::create_dir_all(dir);
    if let Ok(s) = serde_json::to_string_pretty(c) {
        let _ = std::fs::write(dir.join(format!("{}.json", c.id)), s);
    }
}

/// A closed chat, as the Closed list shows it (the whole chat is read when it's reopened).
#[derive(Debug, Clone, Serialize)]
pub struct ClosedChat {
    pub id: String,
    pub title: String,
    pub folder: String,
    pub agent: String,
    pub closed: u64,
}

/// Closed chats, most recently closed first.
pub fn load_closed(dir: &Path) -> Vec<ClosedChat> {
    let mut out: Vec<ClosedChat> = std::fs::read_dir(dir.join("closed"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let c: Chat = serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()?;
            let title = if c.title.is_empty() { "New chat".to_string() } else { c.title };
            Some(ClosedChat { id: c.id, title, folder: c.folder, agent: c.agent, closed: if c.closed_at > 0 { c.closed_at } else { c.updated } })
        })
        .collect();
    out.sort_by(|a, b| b.closed.cmp(&a.closed));
    out
}

/// Bring a closed chat back into the list.
pub fn reopen_chat(dir: &Path, id: &str) -> Option<Chat> {
    let from = dir.join("closed").join(format!("{id}.json"));
    let mut c: Chat = serde_json::from_str(&std::fs::read_to_string(&from).ok()?).ok()?;
    c.closed_at = 0;
    c.settled = false;
    // Back at the top, and not closed again straight away by chats.auto_close_days.
    c.updated = crate::state::now();
    save_chat(dir, &c);
    let _ = std::fs::remove_file(from);
    Some(c)
}

/// Closing a chat moves it aside to the Closed list (it still syncs).
pub fn close_chat(dir: &Path, id: &str) {
    let closed = dir.join("closed");
    let _ = std::fs::create_dir_all(&closed);
    let _ = std::fs::rename(dir.join(format!("{id}.json")), closed.join(format!("{id}.json")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_prompts_say_where_and_what_the_agent_may_do() {
        let def = AgentDef { cli: "claude".into(), ..Default::default() };
        let b = browser_preamble(&def, "Tabs:\n- [active] Mail\n", false, Path::new("/scratch"), "read");
        assert!(b.contains("/scratch") && b.contains("read-only") && b.contains("list_tabs for the tabs as they are now"), "{b}");
        let c = coding_preamble(&def, Path::new("/proj"), "ask");
        assert!(c.contains("/proj") && c.contains("approves each action") && c.contains("irreversible"), "{c}");
    }

    /// Runs the real set-up script for this OS with a made-up app, and returns what it wrote to its status file and printed.
    fn quiet_kind(install: &'static str) -> Kind {
        Kind { id: "quiet-test", title: "Test App", bins: &["valerie-no-such-program"], install, install_windows: install, login: "", help: "" }
    }

    #[test]
    fn a_failed_quiet_install_says_so() {
        let e = install_quietly(&quiet_kind("echo no disk space >&2; exit 3")).unwrap_err();
        assert!(format!("{e:#}").contains("couldn't install Test App"), "{e:#}");
    }

    #[test]
    fn an_install_valerie_cant_find_asks_for_a_restart() {
        let e = install_quietly(&quiet_kind("exit 0")).unwrap_err();
        assert!(format!("{e:#}").contains("Restart Valerie"), "{e:#}");
    }

    fn run_set_up(name: &str, install: &'static str, login: &'static str) -> (String, String) {
        let dir = std::env::temp_dir().join(format!("valerie-set-up-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let k = Kind { id: "fake", title: "Fake App", bins: &["valerie-fake-app"], install, install_windows: install, login, help: "https://example.com/fake" };
        let status = dir.join("it's.status");
        let path = std::env::join_paths([dir.clone()].into_iter().chain(std::env::split_paths(&std::env::var_os("PATH").unwrap()))).unwrap();
        let path = path.to_string_lossy();
        let mut c = if cfg!(windows) {
            let file = dir.join("set-up.ps1");
            std::fs::write(&file, format!("\u{feff}{}", set_up_script_windows(&k, &path, &status))).unwrap();
            let mut c = Command::new("powershell");
            c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(file);
            c
        } else {
            let mut c = Command::new("bash");
            c.arg("-c").arg(set_up_script_unix(&k, &path, &status));
            c
        };
        let out = c.current_dir(&dir).stdin(Stdio::null()).output().unwrap();
        let result = (std::fs::read_to_string(&status).unwrap_or_default(), String::from_utf8_lossy(&out.stdout).into_owned());
        std::fs::remove_dir_all(&dir).unwrap();
        result
    }

    const FAKE_INSTALL: &str = if cfg!(windows) { "Set-Content valerie-fake-app.cmd '@echo off'" } else { "printf '#!/bin/sh\\n' > valerie-fake-app; chmod +x valerie-fake-app" };
    const FAIL: &str = if cfg!(windows) { "cmd /c exit 3" } else { "false" };
    const SUCCEED: &str = if cfg!(windows) { "cmd /c exit 0" } else { "true" };

    #[test]
    fn set_up_stops_with_the_download_page_when_the_install_fails() {
        let (status, printed) = run_set_up("install", FAIL, SUCCEED);
        assert_eq!(status, "install_failed", "{printed}");
        assert!(printed.contains("https://example.com/fake"), "{printed}");
        assert!(!printed.contains("All set"), "{printed}");
    }

    #[test]
    fn set_up_says_so_when_signing_in_fails() {
        let (status, printed) = run_set_up("login", FAKE_INSTALL, FAIL);
        assert_eq!(status, "login_failed", "{printed}");
        assert!(!printed.contains("All set"), "{printed}");
    }

    #[test]
    fn set_up_reports_done_only_after_both_steps_work() {
        let (status, printed) = run_set_up("done", FAKE_INSTALL, SUCCEED);
        assert_eq!(status, "done", "{printed}");
        assert!(printed.contains("All set"), "{printed}");
    }

    #[test]
    fn every_ai_app_can_install_on_windows() {
        for k in KINDS {
            assert!(!k.install_windows.contains("bash") && !k.install_windows.contains('"'), "{}", k.id);
            assert!(k.help.starts_with("https://"), "{}", k.id);
        }
    }

    #[test]
    fn parses_each_cli() {
        let claude = r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}}"#;
        assert!(matches!(&parse(Format::Claude, claude)[..], [Ev::Delta { text }] if text == "hi"));
        let codex = r#"{"type":"item.completed","item":{"type":"agent_message","text":"done"}}"#;
        assert!(matches!(&parse(Format::Codex, codex)[..], [Ev::Block { text }] if text == "done"));
        let err = r#"{"type":"error","message":"{\"error\":{\"message\":\"bad model\"}}"}"#;
        assert!(matches!(&parse(Format::Codex, err)[..], [Ev::Error { text }] if text == "bad model"));
        assert_eq!((super::version_major("opencode v2.0.32\n"), super::version_major("1.0.3"), super::version_major("")), (2, 1, 1));
        let oc = r#"{"type":"text","sessionID":"s1","part":{"type":"text","text":"yo"}}"#;
        assert!(matches!(&parse(Format::OpenCode, oc)[..], [Ev::Session { .. }, Ev::Block { text }] if text == "yo"));
        let cursor = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"a"}]},"timestamp_ms":1}"#;
        assert!(matches!(&parse(Format::Cursor, cursor)[..], [Ev::Delta { text }] if text == "a"));
        let cursor_whole = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"a"}]}}"#;
        assert!(parse(Format::Cursor, cursor_whole).is_empty());
    }

    #[test]
    fn claude_steps_carry_input_and_result() {
        let edit = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Edit","input":{"file_path":"/p/a.rs","old_string":"x","new_string":"y"}}],"usage":{"input_tokens":10,"cache_read_input_tokens":90}}}"#;
        match &parse(Format::Claude, edit)[..] {
            [Ev::Step { step }, Ev::Usage { context: 100, .. }] => {
                assert_eq!((step.id.as_str(), step.name.as_str(), step.detail.as_str()), ("t1", "Edit", "/p/a.rs"));
                assert_eq!((step.before.as_str(), step.after.as_str()), ("x", "y"));
            }
            other => panic!("{other:?}"),
        }
        let done = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"ok"}],"is_error":true}]}}"#;
        assert!(matches!(&parse(Format::Claude, done)[..], [Ev::StepDone { id, output, error: true }] if id == "t1" && output == "ok"));
        let think = r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hm"}}}"#;
        assert!(matches!(&parse(Format::Claude, think)[..], [Ev::Thinking { text }] if text == "hm"));
        // A subagent's words aren't the answer.
        let sub = r#"{"type":"stream_event","parent_tool_use_id":"t9","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"inner"}}}"#;
        assert!(parse(Format::Claude, sub).is_empty());
        let todos = step_for("t2", "TodoWrite", &json!({"todos":[{"content":"a","status":"completed"},{"content":"b","status":"in_progress"}]}));
        assert_eq!(todos.todos[1], Todo { text: "b".into(), state: "in_progress".into() });
        assert_eq!(step_for("t3", "mcp__valerie__open_tab", &json!({"url":"http://localhost:3000"})).detail, "http://localhost:3000");
    }

    #[test]
    fn npm_launchers_are_skipped() {
        use super::{npm_shim_target, program_for, prompt_for};
        use std::path::Path;
        // What npm's own cmd-shim writes for a script and for a native program.
        let script = "IF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n)\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\node_modules\\@openai\\codex\\bin\\codex.js\" %*\r\n";
        let native = "CALL :find_dp0\r\n\"%dp0%\\node_modules\\@x\\y\\y.exe\"   %*\r\n";
        assert_eq!(npm_shim_target(script).as_deref(), Some(r"node_modules\@openai\codex\bin\codex.js"));
        assert_eq!(npm_shim_target(native).as_deref(), Some(r"node_modules\@x\y\y.exe"));
        assert_eq!(npm_shim_target("@echo off\r\nagent.exe %*\r\n"), None);

        let dir = std::env::temp_dir().join(format!("valerie-shim-{}", std::process::id()));
        let tools = dir.join("tools");
        std::fs::create_dir_all(dir.join("node_modules/@openai/codex/bin")).unwrap();
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::write(dir.join("node_modules/@openai/codex/bin/codex.js"), "").unwrap();
        std::fs::write(tools.join("node.exe"), "").unwrap();
        let shim = dir.join("codex.cmd");
        std::fs::write(&shim, script).unwrap();
        let path = tools.to_string_lossy();
        assert_eq!(program_for(&shim, &path), (tools.join("node.exe"), vec![dir.join("node_modules/@openai/codex/bin/codex.js")]));
        // No node to run it with, or a launcher Valerie can't read: the launcher itself, with the prompt on one line.
        assert_eq!(program_for(&shim, "").0, shim);
        assert_eq!(prompt_for(&shim, "a\r\n\nb"), "a   b");
        assert_eq!(prompt_for(Path::new("codex.exe"), "a\nb"), "a\nb");
        std::fs::remove_dir_all(&dir).unwrap();

        assert!(super::is_store_dir(Path::new(r"C:\Users\a\AppData\Local\Microsoft\WindowsApps")));
        assert!(!super::is_store_dir(Path::new(r"C:\Users\a\AppData\Roaming\npm")));
    }

    #[cfg(windows)]
    #[test]
    fn a_multi_line_prompt_reaches_a_program_npm_installed() {
        let path = std::env::var("PATH").unwrap();
        if !std::env::split_paths(&path).any(|d| d.join("node.exe").is_file()) {
            return;
        }
        let dir = std::env::temp_dir().join(format!("valerie-npm-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(r"node_modules\x\bin")).unwrap();
        std::fs::write(dir.join(r"node_modules\x\bin\x.js"), "console.log(JSON.stringify(process.argv[2]))").unwrap();
        let shim = dir.join("x.cmd");
        std::fs::write(&shim, "@ECHO off\r\nSET dp0=%~dp0\r\n\"node\"  \"%dp0%\\node_modules\\x\\bin\\x.js\" %*\r\n").unwrap();
        let prompt = "line one\n\nline \"two\" & 100%";
        // The bug: Windows won't start the launcher with this prompt at all.
        assert!(std::process::Command::new(&shim).arg(prompt).output().is_err());
        let (program, first_args) = super::program_for(&shim, &path);
        let out = std::process::Command::new(program).args(first_args).arg(prompt).output().unwrap();
        assert_eq!(serde_json::from_slice::<String>(&out.stdout).unwrap(), prompt);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn codex_steps_carry_commands_and_output() {
        let start = r#"{"type":"item.started","item":{"id":"i1","type":"command_execution","command":"bash -lc 'cargo test'","status":"in_progress"}}"#;
        assert!(matches!(&parse(Format::Codex, start)[..], [Ev::Step { step }] if step.detail == "cargo test" && step.name == "Bash"));
        let end = r#"{"type":"item.completed","item":{"id":"i1","type":"command_execution","aggregated_output":"boom","exit_code":101}}"#;
        assert!(matches!(&parse(Format::Codex, end)[..], [Ev::StepDone { id, output, error: true }] if id == "i1" && output == "boom"));
        let files = r#"{"type":"item.completed","item":{"id":"i2","type":"file_change","changes":[{"path":"a.rs","kind":"update"},{"path":"b.rs","kind":"add"}],"status":"completed"}}"#;
        assert!(matches!(&parse(Format::Codex, files)[..], [Ev::Step { step }] if step.detail == "a.rs, b.rs" && step.state == "done"));
    }

    #[test]
    fn closed_chats_list_and_come_back() {
        let dir = std::env::temp_dir().join(format!("valerie-closed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut a = super::Chat::new("claude");
        a.title = "Fix login".into();
        a.folder = "/code/app".into();
        let mut b = super::Chat::new("codex");
        b.id.push('b');
        for (c, at) in [(&mut a, 100), (&mut b, 200)] {
            c.closed_at = at;
            c.settled = true;
            super::save_chat(&dir, c);
            super::close_chat(&dir, &c.id);
        }
        let list = super::load_closed(&dir);
        assert_eq!(list.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["New chat", "Fix login"]);
        assert_eq!(list[1].folder, "/code/app");
        let back = super::reopen_chat(&dir, &a.id).unwrap();
        assert!(back.closed_at == 0 && !back.settled && back.updated >= crate::state::now() - 5);
        assert_eq!(super::load_chats(&dir).len(), 1);
        assert_eq!(super::load_closed(&dir).len(), 1);
        assert!(super::reopen_chat(&dir, &a.id).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Talks to OpenCode's free model: `cargo test -- --ignored opencode_answers`.
    #[test]
    #[ignore]
    fn opencode_answers() {
        let def = AgentDef { cli: "opencode".into(), model: "opencode/deepseek-v4.1-flash-free".into(), command: vec![] };
        let o = Opts { effort: "high".into(), ..Default::default() };
        let dir = std::env::temp_dir();
        let (cmd, fmt) = build(&def, Mode::PageOnly, "Reply with just the word pong", None, true, &dir, &o).unwrap();
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        eprintln!("{args:?}");
        assert!(!args.contains(&"--dir".to_string()));
        let got = std::cell::RefCell::new(String::new());
        stream("test-oc", cmd, fmt, std::time::Duration::from_secs(120), &|ev| match ev {
            Ev::Block { text } | Ev::Delta { text } => got.borrow_mut().push_str(&text),
            Ev::Error { text } => panic!("{text}"),
            _ => {}
        })
        .unwrap();
        assert!(got.borrow().to_lowercase().contains("pong"), "{:?}", got.borrow());
    }

    #[test]
    fn clip_keeps_both_ends() {
        let long = format!("{}END", "a".repeat(100));
        let c = clip(&long, 30);
        assert!(c.starts_with("aaaaaaaaaa") && c.ends_with("END") && c.len() < long.len());
    }
}

#[cfg(test)]
mod free_ai_tests {
    /// Real network: installs OpenCode if missing and finds a free model that answers.
    #[test]
    #[ignore]
    fn free_ai_answers() {
        let model = super::set_up_free(&|s| eprintln!("step: {s}")).unwrap();
        eprintln!("model: {model}");
        assert!(model.starts_with("opencode/") && model.ends_with("-free"));
    }
}

#[cfg(test)]
mod cli_default_tests {
    /// Prints what each CLI on this machine uses by default (reads the user's own settings files).
    #[test]
    #[ignore]
    fn print_cli_defaults() {
        for k in super::KINDS {
            eprintln!("{:<9} {:?}", k.id, super::cli_defaults(k.id));
        }
    }
}
