//! "Report a Problem": the user's own AI walks them through what went wrong with a playbook (triage.md) and a few
//! facts about this copy of Valerie, in a chat or, when Valerie won't open, in a terminal (`valerie triage`).

use crate::config::{self, Config};

pub const PLAYBOOK: &str = include_str!("triage.md");

/// What the helper needs to know about this copy: version, computer, update channel, plugins.
pub fn facts(c: &Config) -> String {
    let (plugins, plugin_errors) = crate::plugins::load_all();
    let names: Vec<String> = plugins.iter().map(|p| if p.manifest.enabled { p.name.clone() } else { format!("{} (off)", p.name) }).collect();
    let mut f = format!(
        "Facts about this copy of Valerie:\n- Version: {}\n- Computer: {} {}\n- Update channel: {}\n- AI used for chats: {}\n- Plugins: {}\n",
        crate::app::version(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        c.updates.channel,
        c.agent_for("chat").0,
        if names.is_empty() { "none".to_string() } else { names.join(", ") },
    );
    for e in plugin_errors.iter().take(5) {
        f.push_str(&format!("- Plugin problem: {e}\n"));
    }
    f
}

/// The hidden instructions for a "Report a Problem" chat (it reaches the log and settings through Valerie's tools).
pub fn chat_brief(c: &Config) -> String {
    let send = if c.triage.file_issues {
        format!(
            "Send a bug report with the report_issue tool (title and body in Markdown; issue: <number> to add it as a \
             comment on an existing issue). It goes to {} from their GitHub account, and they see the exact text and \
             say yes before anything is posted. Give them the link it returns. If they'd rather not, or it fails, show \
             them the text to post themselves.",
            c.triage.repo
        )
    } else {
        "Show them the text and let them send it themselves (never post anything for them).".to_string()
    };
    format!(
        "{PLAYBOOK}\nIn this chat you reach everything through Valerie's tools: read_logs for the log, get_settings and \
         set_setting for settings, site_state for a site, screenshot and read_page for a page, read_ui and screenshot_ui \
         for Valerie's own window, and open_tab plus read_page for the issues page. {send}\n\n{}",
        facts(c)
    )
}

/// Send a bug report to `repo` ("owner/name") from the user's GitHub account: a new issue, or a comment on issue
/// `number` when one already covers it. Returns its address.
pub fn file_issue(token: &str, repo: &str, title: &str, body: &str, number: Option<u64>) -> anyhow::Result<String> {
    use anyhow::{bail, Context};
    let (title, body) = (title.trim(), body.trim());
    if body.is_empty() || (number.is_none() && title.is_empty()) {
        bail!("give the report a title and a body");
    }
    if !repo.contains('/') {
        bail!("triage.repo should look like owner/name");
    }
    let body = format!("{body}\n\n<sub>Sent from Valerie {} with Report a Problem.</sub>", crate::app::version());
    let (path, payload) = match number {
        Some(n) => (format!("/repos/{repo}/issues/{n}/comments"), serde_json::json!({ "body": body })),
        None => (format!("/repos/{repo}/issues"), serde_json::json!({ "title": title.chars().take(200).collect::<String>(), "body": body })),
    };
    let (code, r) = crate::github::api(token, "POST", &path, Some(&payload))?;
    r["html_url"]
        .as_str()
        .filter(|_| (200..300).contains(&code))
        .map(str::to_string)
        .with_context(|| format!("GitHub didn't take it: {}", r["message"].as_str().unwrap_or("no reason given")))
}

/// `valerie triage`: start the user's AI in a terminal with the playbook, for when Valerie itself won't open.
pub fn run_cli() -> anyhow::Result<()> {
    let c = Config::load().unwrap_or_else(|e| {
        eprintln!("(your config has a problem, so the defaults are used here: {e:#})");
        Config::defaults()
    });
    let prompt = format!(
        "{PLAYBOOK}\nValerie may not be running, so work with files and commands:\n- Logs: {logs} (one file a day, \
         newest last; times in UTC)\n- Settings: {config} (the user's changes; every option is described in \
         reference/default-config.yaml next to it). `{exe} check-config` checks them and `{exe} effective-config` prints what applies.\n\
         - Data: {data}\n- Source code: https://github.com/cullen-b/valerie (clone it if reading the code would help find \
         the bug). If the `gh` command is installed you can search issues with it. Show them the report and let them \
         send it themselves (never post anything for them).\n\nStart by asking what went wrong.\n\n{facts}",
        logs = crate::log::dir().display(),
        config = config::config_path().display(),
        data = config::data_dir().display(),
        exe = std::env::current_exe().map(|e| e.display().to_string()).unwrap_or_else(|_| "valerie".into()),
        facts = facts(&c),
    );
    let (name, def) = c.agent_for("customize");
    let cli = if def.cli.is_empty() { name.clone() } else { def.cli.clone() };
    let bin = crate::ai::kind(&cli).and_then(|k| crate::ai::find_bin(k, &crate::ai::search_path()));
    // Claude Code and Codex start a conversation from a prompt given as their first argument.
    match bin.filter(|_| matches!(cli.as_str(), "claude" | "codex")) {
        Some(bin) => {
            println!("Starting {cli} to help with what went wrong…");
            let (program, first_args) = crate::ai::program_for(&bin, &crate::ai::search_path());
            let prompt = crate::ai::prompt_for(&program, &prompt);
            let status = std::process::Command::new(&program).args(first_args).arg(prompt.as_ref()).current_dir(config::config_dir()).status()?;
            if !status.success() {
                anyhow::bail!("{cli} ended with {status}");
            }
        }
        None => {
            let file = config::data_dir().join("triage-prompt.md");
            std::fs::create_dir_all(config::data_dir())?;
            std::fs::write(&file, &prompt)?;
            println!(
                "Valerie couldn't start Claude Code or Codex here. Open your AI assistant and paste in the contents of\n  {}\n\
                 then tell it what went wrong.",
                file.display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn refuses_a_report_without_text_before_calling_github() {
        assert!(super::file_issue("t", "o/r", "", "steps", None).is_err());
        assert!(super::file_issue("t", "o/r", "Title", "  ", None).is_err());
        assert!(super::file_issue("t", "nope", "Title", "body", None).is_err());
    }
}
