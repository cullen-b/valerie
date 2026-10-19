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
    format!(
        "{PLAYBOOK}\nIn this chat you reach everything through Valerie's tools: read_logs for the log, get_settings and \
         set_setting for settings, site_state for a site, screenshot and read_page for a page, read_ui and screenshot_ui \
         for Valerie's own window, and open_tab plus read_page for the issues page.\n\n{}",
        facts(c)
    )
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
         the bug). If the `gh` command is installed you can search issues with it.\n\nStart by asking what went wrong.\n\n{facts}",
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
            let status = std::process::Command::new(bin).arg(&prompt).current_dir(config::config_dir()).status()?;
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
