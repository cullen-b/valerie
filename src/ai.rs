//! AI in Valerie: the agent CLIs already on the user's computer (Claude Code, Codex, OpenCode, Pi, Cursor, Hermes, or
//! any command), run headless. They chat in the Chats space with full control of the browser, answer questions
//! about a page from find (with no tools at all), and stream their answers back as they write them.
use crate::config::AgentDef;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read};
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
    /// Shell command that signs in.
    pub login: &'static str,
}

pub const KINDS: &[Kind] = &[
    Kind { id: "claude", title: "Claude Code", bins: &["claude"], install: "curl -fsSL https://claude.ai/install.sh | bash", login: "claude auth login" },
    Kind { id: "codex", title: "Codex", bins: &["codex"], install: "npm install -g @openai/codex", login: "codex login" },
    Kind { id: "opencode", title: "OpenCode", bins: &["opencode"], install: "curl -fsSL https://opencode.ai/install | bash", login: "opencode auth login" },
    Kind {
        id: "pi",
        title: "Pi",
        bins: &["pi"],
        install: "npm install -g --ignore-scripts @earendil-works/pi-coding-agent",
        login: "echo 'Type /login, pick your provider and sign in, then /quit.'; pi",
    },
    Kind { id: "cursor", title: "Cursor", bins: &["cursor-agent", "agent"], install: "curl https://cursor.com/install -fsS | bash", login: "cursor-agent login" },
    Kind {
        id: "hermes",
        title: "Hermes",
        bins: &["hermes"],
        install: "curl -fsSL https://raw.githubusercontent.com/NousResearch/hermes-agent/main/scripts/install.sh | bash",
        login: "hermes setup",
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
    /// Session id for continuing the conversation next turn.
    Session { id: String },
    Error { text: String },
}

/// GUI apps get a minimal PATH, so look where CLIs are usually installed too.
pub fn search_path() -> String {
    let home = dirs::home_dir().unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    for extra in [".local/bin", ".bun/bin", ".npm-global/bin", ".cargo/bin", ".opencode/bin", ".hermes/bin"] {
        dirs.push(home.join(extra));
    }
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        dirs.push(d); // so agents and scripts can run `valerie …`
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    std::env::join_paths(dirs).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn find_bin(kind: &Kind, path: &str) -> Option<PathBuf> {
    kind.bins.iter().find_map(|b| std::env::split_paths(path).map(|d| d.join(b)).find(|p| p.is_file()))
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
}

impl Default for Opts {
    fn default() -> Self {
        Opts { access: "full".into(), chat: String::new(), effort: String::new(), plan: false, own_tools: true }
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
        // Any program: `{prompt}` and `{model}` in its arguments are filled in; it prints the answer.
        let mut c = Command::new(&def.command[0]);
        c.args(def.command[1..].iter().map(|a| a.replace("{prompt}", prompt).replace("{model}", model)));
        c.env("PATH", &path).env("VALERIE_ACCESS", access).env("VALERIE_CHAT", &o.chat).current_dir(work);
        return Ok((c, Format::Text));
    }
    let k = kind(&def.cli).with_context(|| format!("unknown agent CLI `{}` (use claude, codex, opencode, pi, cursor or hermes, or `command`)", def.cli))?;
    let bin = find_bin(k, &path).with_context(|| format!("{} isn't installed: run \"Set Up {}\" from the command bar", k.title, k.title))?;
    let mut c = Command::new(bin);
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
                    c.args(["--mcp-config", &mcp_json().to_string(), "--strict-mcp-config", "--permission-mode", "acceptEdits"]);
                    c.args(["--allowedTools", "Read", "Edit", "Write", "Glob", "Grep", &check, &call, "mcp__valerie"]);
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
            match session {
                Some(s) => c.args(["resume", s, prompt]),
                None => c.arg(prompt),
            };
            Format::Codex
        }
        "opencode" => {
            c.args(["run", prompt]).arg("--dir").arg(work);
            if stream {
                c.args(["--format", "json"]);
            }
            let cfg = match mode {
                Mode::PageOnly => json!({ "mcp": {}, "tools": { "*": false } }),
                // Changing Valerie means editing its settings file; the only command it may run is the settings check.
                Mode::Customize => json!({
                    "mcp": { "valerie": { "type": "local", "command": [exe, "mcp-stdio", "--access", "full"], "enabled": true } },
                    "permission": { "edit": "allow", "external_directory": "deny",
                                    "bash": { "*": "deny", format!("{exe} check-config*"): "allow", "valerie check-config*": "allow" } },
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
            if !effort.is_empty() {
                c.args(["--variant", effort]);
            }
            if let Some(s) = session {
                c.args(["--session", s]);
            }
            if !model.is_empty() {
                c.args(["-m", model]);
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

/// The prompt preamble for an agent with browser control.
pub fn browser_preamble(def: &AgentDef, context: &str) -> String {
    let how = control_note(def);
    format!(
        "You are the assistant built into the user's Valerie web browser. You can see and control their browser. {how} \
         Open any tabs you need in the background and don't close or change the user's tabs unless they ask. Never submit \
         purchases, payments or other irreversible forms without the user saying so in this conversation; stop and ask \
         instead. Answer in plain, short Markdown.\n\n{context}"
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
        Format::Claude => match v["type"].as_str() {
            Some("system") if v["subtype"] == "init" => out.extend(s(&v["session_id"]).map(|id| Ev::Session { id })),
            Some("stream_event") => {
                let e = &v["event"];
                if e["type"] == "content_block_delta" && e["delta"]["type"] == "text_delta" {
                    out.extend(s(&e["delta"]["text"]).map(|text| Ev::Delta { text }));
                } else if e["type"] == "content_block_start" && e["content_block"]["type"] == "tool_use" {
                    // ToolSearch is Claude loading tool definitions, not something it did.
                    let name = s(&e["content_block"]["name"]).filter(|n| n != "ToolSearch");
                    out.extend(name.map(|n| Ev::Tool { text: n.trim_start_matches("mcp__valerie__").to_string() }));
                } else if e["type"] == "message_start" {
                    // A new message after tool use: keep paragraphs apart.
                    out.push(Ev::Block { text: String::new() });
                }
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
        Format::Codex => match v["type"].as_str() {
            Some("thread.started") => out.extend(s(&v["thread_id"]).map(|id| Ev::Session { id })),
            Some("item.started") => match v["item"]["type"].as_str() {
                Some("mcp_tool_call") => out.extend(s(&v["item"]["tool"]).map(|text| Ev::Tool { text })),
                Some("command_execution") => out.extend(s(&v["item"]["command"]).map(|c| Ev::Tool { text: format!("Ran {c}") })),
                Some("web_search") => out.push(Ev::Tool { text: "Searched the web".into() }),
                _ => {}
            },
            Some("item.completed") if v["item"]["type"] == "agent_message" => out.extend(s(&v["item"]["text"]).map(|text| Ev::Block { text })),
            Some("error") | Some("turn.failed") => {
                let raw = s(&v["message"]).or_else(|| s(&v["error"]["message"])).unwrap_or_else(|| "the agent failed".into());
                // Codex wraps API errors as JSON inside the message.
                let msg = serde_json::from_str::<Value>(&raw).ok().and_then(|e| s(&e["error"]["message"])).unwrap_or(raw);
                out.push(Ev::Error { text: msg });
            }
            _ => {}
        },
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
    let Some(pid) = pid else { return false };
    #[cfg(unix)]
    let ok = Command::new("kill").arg(pid.to_string()).status().is_ok_and(|s| s.success());
    #[cfg(windows)]
    let ok = Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).status().is_ok_and(|s| s.success());
    ok
}

/// Run one turn to the end (blocking), handing each piece of the answer to `on` as it arrives.
pub fn stream(run: &str, mut cmd: Command, format: Format, timeout: Duration, on: &dyn Fn(Ev)) -> Result<()> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().context("couldn't start the agent")?;
    if let Ok(mut r) = RUNNING.lock() {
        r.push((run.to_string(), child.id()));
    }
    let mut err = child.stderr.take();
    let errs = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = err.as_mut() {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
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
    if !status.is_ok_and(|s| s.success()) && !got {
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
const FREE_MODELS: &[&str] = &["deepseek-v4.1-flash-free", "glm-5.3-flash-free", "nemotron-3-ultra-free", "mimo-v2.6-flash-free", "beam-free"];

/// For people without an AI subscription: install OpenCode (no Terminal needed) and find a free model that answers.
/// Returns that model ("opencode/…"). `step` hears what's happening.
pub fn set_up_free(step: &dyn Fn(&str)) -> Result<String> {
    let path = search_path();
    let k = kind("opencode").context("no OpenCode")?;
    if find_bin(k, &path).is_none() {
        step("Installing OpenCode…");
        let mut c = Command::new("/bin/bash");
        c.args(["-c", k.install]).env("PATH", &path).stdin(std::process::Stdio::null());
        let out = run_with_timeout(&mut c, Duration::from_secs(600))?;
        if !out.status.success() || find_bin(k, &search_path()).is_none() {
            bail!("couldn't install OpenCode: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("").trim());
        }
    }
    step("Finding a free model…");
    let bin = find_bin(k, &search_path()).context("OpenCode isn't installed")?;
    let mut c = Command::new(bin);
    c.args(["models", "opencode"]).env("PATH", &path).stdin(std::process::Stdio::null());
    let listed = String::from_utf8_lossy(&run_with_timeout(&mut c, Duration::from_secs(60))?.stdout).to_string();
    let free: Vec<&str> = listed.lines().map(str::trim).filter(|m| m.ends_with("-free")).collect();
    let mut order: Vec<String> = FREE_MODELS.iter().map(|m| format!("opencode/{m}")).filter(|m| free.contains(&m.as_str())).collect();
    let rest: Vec<String> = free.iter().map(|m| m.to_string()).filter(|m| !order.contains(m)).collect();
    order.extend(rest);
    if order.is_empty() {
        bail!("OpenCode lists no free models right now");
    }
    for m in order.iter().take(4) {
        step(&format!("Trying {}…", m.trim_start_matches("opencode/")));
        if probe(&AgentDef { cli: "opencode".into(), model: m.clone(), command: vec![] }).is_ok() {
            return Ok(m.clone());
        }
    }
    bail!("none of OpenCode's free models answered; try again in a minute")
}

/// Run a command, killing it after `limit`.
fn run_with_timeout(c: &mut Command, limit: Duration) -> Result<std::process::Output> {
    let mut child = c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn()?;
    let (mut so, mut se) = (child.stdout.take(), child.stderr.take());
    let read = |p: Option<&mut dyn std::io::Read>| {
        let mut buf = Vec::new();
        if let Some(p) = p {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    };
    let out_t = std::thread::spawn(move || read(so.as_mut().map(|p| p as &mut dyn std::io::Read)));
    let err_t = std::thread::spawn(move || read(se.as_mut().map(|p| p as &mut dyn std::io::Read)));
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            bail!("it took too long to answer");
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    Ok(std::process::Output { status, stdout: out_t.join().unwrap_or_default(), stderr: err_t.join().unwrap_or_default() })
}

pub fn set_up(k: &Kind) -> Result<()> {
    let path = search_path();
    let mut script = format!("export PATH=\"{path}\"\necho 'Setting up {} for Valerie'\n", k.title);
    if find_bin(k, &path).is_none() {
        script.push_str(&format!("echo 'Installing {}…'\n{}\n", k.title, k.install));
    }
    script.push_str(&format!("{}\necho\necho 'All set: you can close this window and go back to Valerie.'\n", k.login));
    #[cfg(target_os = "macos")]
    {
        let file = std::env::temp_dir().join(format!("valerie-set-up-{}.command", k.id));
        std::fs::write(&file, format!("#!/bin/bash\n{script}"))?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))?;
        Command::new("open").args(["-a", "Terminal"]).arg(&file).spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let file = std::env::temp_dir().join(format!("valerie-set-up-{}.sh", k.id));
        std::fs::write(&file, format!("#!/bin/bash\n{script}\nread -p 'Press Enter to close'"))?;
        Command::new("x-terminal-emulator").args(["-e", "bash"]).arg(&file).spawn()?;
    }
    #[cfg(windows)]
    {
        Command::new("cmd").args(["/C", "start", "cmd", "/K", k.login]).spawn()?;
    }
    Ok(())
}

// ---------------------------------------------------------------- chats

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Msg {
    pub role: String,
    #[serde(default)]
    pub text: String,
    /// Tools the agent used while writing this message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
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
    /// Set aside (still searchable); a new message brings it back.
    #[serde(default)]
    pub settled: bool,
    /// Hidden until this time (seconds), then it comes back marked unread.
    #[serde(default)]
    pub snoozed_until: u64,
    #[serde(default)]
    pub unread: bool,
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

/// Closing a chat moves it aside (it still syncs, and can be brought back by hand).
pub fn close_chat(dir: &Path, id: &str) {
    let closed = dir.join("closed");
    let _ = std::fs::create_dir_all(&closed);
    let _ = std::fs::rename(dir.join(format!("{id}.json")), closed.join(format!("{id}.json")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_cli() {
        let claude = r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}}"#;
        assert!(matches!(&parse(Format::Claude, claude)[..], [Ev::Delta { text }] if text == "hi"));
        let codex = r#"{"type":"item.completed","item":{"type":"agent_message","text":"done"}}"#;
        assert!(matches!(&parse(Format::Codex, codex)[..], [Ev::Block { text }] if text == "done"));
        let err = r#"{"type":"error","message":"{\"error\":{\"message\":\"bad model\"}}"}"#;
        assert!(matches!(&parse(Format::Codex, err)[..], [Ev::Error { text }] if text == "bad model"));
        let oc = r#"{"type":"text","sessionID":"s1","part":{"type":"text","text":"yo"}}"#;
        assert!(matches!(&parse(Format::OpenCode, oc)[..], [Ev::Session { .. }, Ev::Block { text }] if text == "yo"));
        let cursor = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"a"}]},"timestamp_ms":1}"#;
        assert!(matches!(&parse(Format::Cursor, cursor)[..], [Ev::Delta { text }] if text == "a"));
        let cursor_whole = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"a"}]}}"#;
        assert!(parse(Format::Cursor, cursor_whole).is_empty());
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
