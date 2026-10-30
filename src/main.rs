mod agents;
mod app;
mod blocking;
mod command;
mod config;
mod connect;
mod crypt;
mod extensions;
mod import;
mod llm_guide;
mod mcp;
mod passwords;
mod platform;
mod plugins;
mod routing;
mod state;
mod sync;
mod trust;
mod github;
mod ai;
mod update;
mod market;
mod log;
mod triage;
mod terminal;

use config::Config;

const HELP: &str = "valerie: a fast, configurable, agent-native browser

USAGE:
  valerie [URL...]        open the browser (URLs open like external links; forwarded if already running)
  valerie connect [app…] [--list] [--undo]
                          add Valerie's MCP server to the AI apps here (Claude Code, Codex, Cursor, Gemini CLI, …)
  valerie mcp-config      print the settings any other MCP app needs to use Valerie
  valerie mcp-stdio       stdio <-> HTTP bridge for MCP clients that only speak stdio
  valerie sync-setup      create a private GitHub repo for tab sync (needs the gh CLI)
  valerie allow-from-sync list plugins and AI programs another machine sent that wait to be allowed here;
                          `allow-from-sync <name or program>` allows one
  valerie import-passwords <csv> [vault]  import a Chrome/Arc/Safari/1Password CSV export into a vault
  valerie default-config  print every config option with its default
  valerie llm-guide       print the guide that lets an AI assistant customise Valerie
  valerie check-config    validate config.yaml and every plugin (prints exact errors)
  valerie strings         print Valerie's words, ready to translate into strings/<language>.yaml
  valerie call <tool> [json]  call a browser tool, e.g. valerie call list_tabs '{}'  
  valerie screenshot [--full] [--tab N] [file.png]  save a picture of a tab (the active one; --full for the whole
                          page), to file.png or a Valerie Capture <time>.png here
  valerie paths           show config and data locations
  valerie triage          something's wrong: your AI (Claude Code or Codex) helps find why, even if Valerie won't open";

fn check_config() -> anyhow::Result<()> {
    let (problems, plugins) = config_problems();
    if problems.is_empty() {
        println!("OK: config and {plugins} plugin(s) are valid");
        Ok(())
    } else {
        for p in &problems {
            eprintln!("error: {p}");
        }
        std::process::exit(1)
    }
}

/// Every mistake `valerie check-config` would report, and how many plugins loaded.
pub fn config_problems() -> (Vec<String>, usize) {
    let mut problems = Vec::new();
    match Config::load() {
        Ok(c) => {
            for (key, binding) in &c.keybindings {
                if let Some(b) = binding {
                    // A key sequence is shortcuts separated by spaces.
                    if key.split_whitespace().any(|k| k.parse::<muda::accelerator::Accelerator>().is_err()) || key.trim().is_empty() {
                        problems.push(format!("keybindings: {key:?} is not a valid shortcut"));
                    }
                    if b.command().is_none() {
                        problems.push(format!("keybindings: {key:?} has an unknown command {b:?}"));
                    }
                }
            }
            for s in &c.spaces {
                if let Some(p) = &s.profile {
                    if !c.profiles.iter().any(|x| &x.name == p) {
                        problems.push(format!("spaces: {:?} uses profile {p:?}, which isn't in `profiles`", s.name));
                    }
                }
            }
            if !["folder", "icons"].contains(&c.sidebar.compact_pinned.as_str()) {
                problems.push(format!("sidebar: compact_pinned is {:?}; use folder or icons", c.sidebar.compact_pinned));
            }
            if !(16.0..=120.0).contains(&c.sidebar.tab_height) {
                problems.push(format!("sidebar: tab_height is {}; use a number of pixels from 16 to 120", c.sidebar.tab_height));
            }
            if !["site", "tab"].contains(&c.tabs.zoom.as_str()) {
                problems.push(format!("tabs: zoom is {:?}; use site or tab", c.tabs.zoom));
            }
            if !["auto", "light", "dark", "sepia"].contains(&c.reader_view.theme.as_str()) {
                problems.push(format!("reader_view: theme is {:?}; use auto, light, dark or sepia", c.reader_view.theme));
            }
            if !["errors", "events", "verbose"].contains(&c.logs.level.as_str()) {
                problems.push(format!("logs: level is {:?}; use errors, events or verbose", c.logs.level));
            }
            if !["summary", "open", "hidden"].contains(&c.chats.steps.as_str()) {
                problems.push(format!("chats: steps is {:?}; use summary, open or hidden", c.chats.steps));
            }
            if !["float", "chats"].contains(&c.chats.from_space.as_str()) {
                problems.push(format!("chats: from_space is {:?}; use float or chats", c.chats.from_space));
            }
            if !["auto", "always", "never"].contains(&c.chats.worktrees.as_str()) {
                problems.push(format!("chats: worktrees is {:?}; use auto, always or never", c.chats.worktrees));
            }
            if !["ask", "always", "never"].contains(&c.mcp.connect_ai_apps.as_str()) {
                problems.push(format!("mcp: connect_ai_apps is {:?}; use ask, always or never", c.mcp.connect_ai_apps));
            }
            if !["block", "bar", "underline"].contains(&c.terminal_panel.cursor.as_str()) {
                problems.push(format!("terminal_panel: cursor is {:?}; use block, bar or underline", c.terminal_panel.cursor));
            }
            if !(4.0..=60.0).contains(&c.terminal_panel.font_size) {
                problems.push(format!("terminal_panel: font_size is {}; use 4 to 60 (pixels)", c.terminal_panel.font_size));
            }
            if !["top", "bottom"].contains(&c.tabs.new_tab_position.as_str()) {
                problems.push(format!("tabs: new_tab_position is {:?}; use top or bottom", c.tabs.new_tab_position));
            }
            if !["here", "editor"].contains(&c.pages.markdown_edit.as_str()) {
                problems.push(format!("pages: markdown_edit is {:?}; use here or editor", c.pages.markdown_edit));
            }
            if !["auto", "light", "dark"].contains(&c.pages.markdown_theme.as_str()) {
                problems.push(format!("pages: markdown_theme is {:?}; use auto, light or dark", c.pages.markdown_theme));
            }
            if !["off", "system", "always"].contains(&c.pages.dark_mode.as_str()) {
                problems.push(format!("pages: dark_mode is {:?}; use off, system or always", c.pages.dark_mode));
            }
            if !["none", "invert"].contains(&c.pages.dark_fallback.as_str()) {
                problems.push(format!("pages: dark_fallback is {:?}; use none or invert", c.pages.dark_fallback));
            }
            for (name, v) in [("camera", &c.permissions.camera), ("microphone", &c.permissions.microphone)] {
                if !["ask", "allow", "deny"].contains(&v.as_str()) {
                    problems.push(format!("permissions: {name} is {v:?}; use ask, allow or deny"));
                }
            }
            for (name, def) in c.agents.available.iter().filter_map(|(n, d)| d.as_ref().map(|d| (n, d))) {
                if def.command.is_empty() && ai::kind(if def.cli.is_empty() { name } else { &def.cli }).is_none() {
                    problems.push(format!("agents.available.{name}: cli {:?} isn't one Valerie knows (claude, codex, opencode, pi, cursor, hermes); use `command` for anything else", def.cli));
                }
            }
            let u = &c.agents.use_for;
            for (job, name) in [("default", &c.agents.default), ("chat", &u.chat), ("find", &u.find), ("tasks", &u.tasks), ("customize", &u.customize)] {
                if !name.is_empty() && !c.agents.available.get(name).is_some_and(Option::is_some) && ai::kind(name).is_none() {
                    problems.push(format!("agents: {job} uses {name:?}, which isn't under agents.available"));
                }
            }
            for t in c.all_tasks() {
                if t.schedule().is_none() {
                    problems.push(format!("tasks: {:?} has when {:?}; try \"every day at 8am\", \"weekdays at 9\", \"every 5 minutes\", \"when Valerie starts\" or \"only when I ask\"", t.name, t.when));
                }
                if !["", "page", "chat", "notify"].contains(&t.result.as_str()) {
                    problems.push(format!("tasks: {:?} has result {:?}; use page, chat or notify", t.name, t.result));
                }
                if !t.space.is_empty() && !c.spaces.iter().any(|s| s.name == t.space) {
                    problems.push(format!("tasks: {:?} lives in space {:?}, which isn't in `spaces`", t.name, t.space));
                }
                if t.want.trim().is_empty() {
                    problems.push(format!("tasks: {:?} needs `want` (what to do, in plain words)", t.name));
                }
                if !t.agent.is_empty() && !c.agents.available.get(&t.agent).is_some_and(Option::is_some) && ai::kind(&t.agent).is_none() {
                    problems.push(format!("tasks: {:?} uses agent {:?}, which isn't under agents.available", t.name, t.agent));
                }
            }
            for (name, a) in c.ai_actions.iter().filter_map(|(n, a)| a.as_ref().map(|a| (n, a))) {
                if !["page", "chat"].contains(&a.mode.as_str()) {
                    problems.push(format!("ai_actions.{name}: mode is {:?}; use page or chat", a.mode));
                }
                if !a.agent.is_empty() && !c.agents.available.get(&a.agent).is_some_and(Option::is_some) && ai::kind(&a.agent).is_none() {
                    problems.push(format!("ai_actions.{name}: agent {:?} isn't under agents.available", a.agent));
                }
            }
            if !["read", "ask", "full"].contains(&c.chats.access.as_str()) {
                problems.push(format!("chats: access is {:?}; use read, ask or full", c.chats.access));
            }
            if !["", "low", "medium", "high", "max"].contains(&c.chats.effort.as_str()) {
                problems.push(format!("chats: effort is {:?}; use low, medium, high, max or \"\"", c.chats.effort));
            }
            if !["", "space"].contains(&c.chats.theme.as_str()) && !c.theme_names().contains(&c.chats.theme) {
                problems.push(format!("chats: theme {:?} isn't a theme; use one of {} or \"space\"", c.chats.theme, c.theme_names().join(", ")));
            }
            if !["search", "best_match"].contains(&c.command_bar.top_result.as_str()) {
                problems.push(format!("command_bar: top_result is {:?}; use search or best_match", c.command_bar.top_result));
            }
            if !["ask", "allow", "deny"].contains(&c.notifications.websites.as_str()) {
                problems.push(format!("notifications: websites is {:?}; use ask, allow or deny", c.notifications.websites));
            }
            for (site, v) in &c.notifications.sites {
                if !["allow", "deny"].contains(&v.as_str()) {
                    problems.push(format!("notifications.sites: {site} is {v:?}; use allow or deny"));
                }
            }
            if !["window", "tab"].contains(&c.links.popups.as_str()) {
                problems.push(format!("links: popups is {:?}; use window or tab", c.links.popups));
            }
            if !platform::TERMINALS.iter().any(|t| t.eq_ignore_ascii_case(&c.terminal)) {
                problems.push(format!("terminal is {:?}; use one of {}", c.terminal, platform::TERMINALS.join(", ")));
            }
            if c.editor.trim().is_empty() {
                problems.push("editor is empty; use default, textedit, an app's name or a terminal editor like vim".into());
            }
            if !c.plugin_marketplace.index.starts_with("https://") && !c.plugin_marketplace.index.starts_with("file://") {
                problems.push(format!("plugin_marketplace: index is {:?}; it has to be an https:// address", c.plugin_marketplace.index));
            }
            if c.plugin_marketplace.repo.split('/').filter(|s| !s.is_empty()).count() != 2 {
                problems.push(format!("plugin_marketplace: repo is {:?}; write it as owner/name", c.plugin_marketplace.repo));
            }
            if !["auto", "ask", "off"].contains(&c.updates.install.as_str()) {
                problems.push(format!("updates: install is {:?}; use auto, ask or off", c.updates.install));
            }
            if !c.updates.feed.starts_with("https://") && !c.updates.feed.starts_with("file://") {
                problems.push(format!("updates: feed is {:?}; it has to be an https:// address", c.updates.feed));
            }
            if !["auto", "on", "off"].contains(&c.passwords.offer_passkeys.as_str()) {
                problems.push(format!("passwords: offer_passkeys is {:?}; use auto, on or off", c.passwords.offer_passkeys));
            }
            if !["reject", "hide", "off"].contains(&c.privacy.cookie_banners.as_str()) {
                problems.push(format!("privacy: cookie_banners is {:?}; use reject, hide or off", c.privacy.cookie_banners));
            }
            if !["serif", "sans"].contains(&c.reader_view.font.as_str()) {
                problems.push(format!("reader_view: font is {:?}; use serif or sans", c.reader_view.font));
            }
            if !["pin", "star", "favicons"].contains(&c.sidebar.pinned_folder_icon.as_str()) {
                problems.push(format!("sidebar: pinned_folder_icon is {:?}; use pin, star or favicons", c.sidebar.pinned_folder_icon));
            }
            for (name, site) in c.site_search.iter().filter_map(|(n, s)| s.as_ref().map(|s| (n, s))) {
                if !site.url.contains("%s") || !site.url.starts_with("http") {
                    problems.push(format!("site_search: {name:?} url must start with http and contain %s where the search goes"));
                }
                if site.keywords.iter().any(|k| k.trim().is_empty() || k.contains(' ')) {
                    problems.push(format!("site_search: {name:?} keywords must be single words"));
                }
            }
            for (site, profile) in c.site_profiles.iter().filter_map(|(k, v)| v.as_ref().map(|v| (k, v))) {
                if !c.profiles.iter().any(|x| &x.name == profile) {
                    problems.push(format!("site_profiles: {site:?} uses profile {profile:?}, which isn't in `profiles`"));
                }
            }
            for r in &c.routes {
                if !c.spaces.iter().any(|s| s.name == r.space) {
                    problems.push(format!("routes: {:?} sends links to space {:?}, which isn't in `spaces`", r.pattern, r.space));
                }
            }
            problems.extend(check_new_settings(&c));
        }
        Err(e) => problems.push(format!("config.yaml: {e:#}")),
    }
    problems.extend(config::unknown_keys(&std::fs::read_to_string(config::config_path()).unwrap_or_default()));
    if let Ok(here) = std::fs::read_to_string(config::this_mac_path()) {
        match serde_yaml::from_str::<serde_yaml::Value>(&here) {
            Ok(_) => problems.extend(config::unknown_keys(&here).into_iter().map(|u| format!("this-mac.yaml: {u}"))),
            Err(e) => problems.push(format!("this-mac.yaml: {e}")),
        }
    }
    let (plugins, errors) = plugins::load_all();
    problems.extend(errors.into_iter().map(|e| format!("plugin {e}")));
    (problems, plugins.len())
}

/// Checks for the settings that change Valerie's menus, sites, rules, window and the rest.
fn check_new_settings(c: &Config) -> Vec<String> {
    let mut p = Vec::new();
    let one_of = |p: &mut Vec<String>, what: String, v: &str, allowed: &[&str]| {
        if !allowed.contains(&v) {
            p.push(format!("{what} is {v:?}; use {}", allowed.join(", ")));
        }
    };
    one_of(&mut p, "updates: channel".into(), &c.updates.channel, &["stable", "edge"]);
    const PARTS: &[&str] = &["url", "favorites", "widgets", "space_name", "pinned", "kept", "today", "now_playing", "spaces"];
    for part in &c.sidebar.order {
        one_of(&mut p, "sidebar: order has".into(), part, PARTS);
    }
    const MENUS: &[&str] = &["Valerie", "Edit", "Tabs", "Spaces", "View", "Tools", "Plugins", "Window"];
    for m in &c.menu.hide {
        if !MENUS.iter().any(|x| x.eq_ignore_ascii_case(m)) {
            p.push(format!("menu: hide has {m:?}; menus are {}", MENUS.join(", ")));
        }
    }
    if c.menu.hide.iter().any(|m| m.eq_ignore_ascii_case("Valerie")) {
        p.push("menu: the Valerie menu can't be hidden (it holds Quit)".into());
    }
    if !c.menu.quit_shortcut.trim().is_empty() && c.menu.quit_shortcut.parse::<muda::accelerator::Accelerator>().is_err() {
        p.push(format!("menu: quit_shortcut {:?} isn't a shortcut (e.g. \"CmdOrCtrl+Q\", or \"\" for none)", c.menu.quit_shortcut));
    }
    for name in c.menu.commands.keys() {
        if command::Command::from_name(name).is_none() && !command::WITH_ARGS.iter().any(|(n, _)| n == name) {
            p.push(format!("menu: commands has {name:?}, which isn't a command (see `valerie llm-guide`)"));
        }
    }
    for (what, b) in [("back_button", &c.mouse.back_button), ("forward_button", &c.mouse.forward_button), ("middle_click_tab", &c.mouse.middle_click_tab)] {
        if b.as_ref().is_some_and(|b| b.command().is_none()) {
            p.push(format!("mouse: {what} isn't a command"));
        }
    }
    for (what, list) in [("dock_menu", &c.app.dock_menu), ("menu_bar_menu", &c.app.menu_bar_menu)] {
        if let Some(b) = list.iter().find(|b| b.command().is_none()) {
            p.push(format!("app: {what} has {b:?}, which isn't a command"));
        }
    }
    let icon = c.app.icon.trim();
    if !icon.is_empty() {
        let path = icon.strip_prefix("~/").map(|r| dirs::home_dir().unwrap_or_default().join(r)).unwrap_or_else(|| icon.into());
        if !path.exists() {
            p.push(format!("app: icon {icon:?} isn't a file"));
        }
    }
    for (key, b) in c.global_keybindings.iter().filter_map(|(k, b)| b.as_ref().map(|b| (k, b))) {
        if platform::hotkey_code(key).is_none() {
            p.push(format!("global_keybindings: {key:?} can't be a global shortcut (use modifiers and a letter, digit, F-key, arrow or Space)"));
        }
        if b.command().is_none() {
            p.push(format!("global_keybindings: {key:?} has an unknown command {b:?}"));
        }
    }
    for (i, r) in c.rules.iter().enumerate() {
        let kinds = [&r.block, &r.redirect, &r.hide, &r.header].iter().filter(|k| !k.trim().is_empty()).count();
        if kinds != 1 {
            p.push(format!("rules: rule {} needs exactly one of block, redirect, hide or header", i + 1));
        }
        if !r.redirect.trim().is_empty() && !r.to.starts_with("http") {
            p.push(format!("rules: rule {} redirects but `to` isn't an http(s) address", i + 1));
        }
        if !r.to.is_empty() && r.redirect.trim().is_empty() {
            p.push(format!("rules: rule {} has `to` without `redirect`", i + 1));
        }
        if !r.header.trim().is_empty() && routing::headers(std::slice::from_ref(r), "https://x/").is_empty() && r.site.is_empty() {
            p.push(format!("rules: rule {} header should look like \"Name: value\"", i + 1));
        }
        if !r.block.trim().is_empty() && blocking::user_block_rules(std::slice::from_ref(r)).0.is_empty() && blocking::user_block_rules(std::slice::from_ref(r)).1.is_empty() {
            p.push(format!("rules: rule {} block {:?} can't be used (it's a regular expression, matches everything, or uses an option Valerie can't do)", i + 1, r.block));
        }
    }
    const LINK_KEYS: &[&str] = &["middle_click", "cmd_click", "shift_click", "option_click", "cmd_shift_click", "option_shift_click"];
    const LINK_ACTIONS: &[&str] = &["background_tab", "new_tab", "split", "peek", "little_window", "default"];
    for (site, s) in c.sites.iter().filter_map(|(k, v)| v.as_ref().map(|v| (k, v))) {
        for (k, v) in [("camera", &s.camera), ("microphone", &s.microphone), ("notifications", &s.notifications)] {
            if let Some(v) = v {
                one_of(&mut p, format!("sites.{site}: {k}"), v, &["ask", "allow", "deny"]);
            }
        }
        if let Some(v) = &s.dark_mode {
            one_of(&mut p, format!("sites.{site}: dark_mode"), v, &["off", "always"]);
        }
        if let Some(v) = &s.popups {
            one_of(&mut p, format!("sites.{site}: popups"), v, &["window", "tab"]);
        }
        if let Some(v) = &s.cookie_banners {
            one_of(&mut p, format!("sites.{site}: cookie_banners"), v, &["reject", "hide", "off"]);
        }
        if s.zoom.is_some_and(|z| !(25.0..=500.0).contains(&z)) {
            p.push(format!("sites.{site}: zoom is a percent from 25 to 500"));
        }
        for (k, v) in s.links.iter().flatten() {
            one_of(&mut p, format!("sites.{site}: links key"), k, LINK_KEYS);
            one_of(&mut p, format!("sites.{site}: links.{k}"), v, LINK_ACTIONS);
        }
    }
    let lang = c.language.trim();
    if !lang.is_empty() && lang != "en" && !config::config_dir().join("strings").join(format!("{lang}.yaml")).exists() {
        p.push(format!("language is {lang:?} but there's no strings/{lang}.yaml in the config folder (`valerie strings` prints what to translate)"));
    } else if !lang.is_empty() && lang != "en" {
        let path = config::config_dir().join("strings").join(format!("{lang}.yaml"));
        if let Err(e) = serde_yaml::from_str::<std::collections::HashMap<String, String>>(&std::fs::read_to_string(&path).unwrap_or_default()) {
            p.push(format!("strings/{lang}.yaml: {e} (it should be \"English\": \"yours\" pairs)"));
        }
    }
    one_of(&mut p, "window: vibrancy".into(), &c.window.vibrancy, &["none", "sidebar", "window"]);
    one_of(&mut p, "window: traffic_lights".into(), &c.window.traffic_lights, &["auto", "show", "hide"]);
    if !(0.3..=1.0).contains(&c.window.opacity) {
        p.push(format!("window: opacity is {}; use a number from 0.3 to 1", c.window.opacity));
    }
    for r in &c.downloads.rules {
        if r.pattern.trim().is_empty() && r.site.trim().is_empty() {
            p.push("downloads: a rule needs `match` (like \"*.pdf\") or `site`".into());
        }
    }
    if let Some(name) = c.start_page.strip_prefix("plugin:") {
        if !plugins::plugins_dir().join(name.trim()).join("start.html").exists() {
            p.push(format!("start_page: plugin {:?} has no start.html", name.trim()));
        }
    } else {
        one_of(&mut p, "start_page".into(), &c.start_page, &["logo", "wordmark", "blank"]);
    }
    for k in c.agents.instructions_for.keys() {
        one_of(&mut p, "agents: instructions_for has".into(), k, &["chat", "find", "tasks", "customize"]);
    }
    let base = config::Config::merged(&std::fs::read_to_string(config::config_path()).unwrap_or_default()).unwrap_or_default();
    for s in &c.spaces {
        if let Some(over) = &s.settings {
            if let Some(Err(e)) = c.for_space(&base, &s.name) {
                p.push(format!("{e:#}"));
            }
            for u in config::unknown_keys(&serde_yaml::to_string(over).unwrap_or_default()) {
                p.push(format!("spaces: {:?} settings: {u}", s.name));
            }
        }
    }
    let ext = extensions::dir(&c.extensions);
    for e in std::fs::read_dir(&ext).into_iter().flatten().flatten().filter(|e| e.path().is_dir()) {
        if !extensions::find(&ext).iter().any(|f| f.dir == e.path()) {
            p.push(format!("extensions: {} has no readable manifest.json", e.file_name().to_string_lossy()));
        }
    }
    p
}

/// `valerie screenshot [--full] [--tab N] [--width W] [file.png]`: a tab's picture, written where you say.
fn screenshot(port: u16, args: &[String]) -> anyhow::Result<()> {
    let (mut a, mut file) = (serde_json::json!({}), None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--full" | "-f" => a["full_page"] = true.into(),
            "--tab" | "--width" => {
                let n: i64 = it.next().and_then(|v| v.parse().ok()).ok_or_else(|| anyhow::anyhow!("{arg} needs a number"))?;
                a[arg.trim_start_matches('-')] = n.into();
            }
            s if s.starts_with('-') => anyhow::bail!("usage: valerie screenshot [--full] [--tab N] [--width W] [file.png]"),
            s => file = Some(std::path::PathBuf::from(s)),
        }
    }
    let req = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "screenshot", "arguments": a } });
    let (status, body) = mcp::post_local(port, &mcp::load_or_create_token(), &req.to_string()).map_err(|_| anyhow::anyhow!("Valerie isn't running"))?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|_| anyhow::anyhow!("HTTP {status}: {body}"))?;
    let content = v["result"]["content"].as_array().cloned().unwrap_or_default();
    let Some(png) = content.iter().find(|c| c["type"] == "image").and_then(|c| mcp::unbase64(c["data"].as_str()?)) else {
        let why = content.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join(" ");
        anyhow::bail!("no picture: {}", if why.is_empty() { v["error"]["message"].as_str().unwrap_or("unknown error") } else { &why });
    };
    let file = file.unwrap_or_else(|| std::path::PathBuf::from(format!("Valerie Capture {}.png", platform::local_timestamp())));
    std::fs::write(&file, png)?;
    println!("{}", file.display());
    Ok(())
}

fn call(port: u16, tool: &str, args: &str) -> anyhow::Result<()> {
    let arguments: serde_json::Value = serde_json::from_str(if args.trim().is_empty() { "{}" } else { args })?;
    let req = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": tool, "arguments": arguments } });
    let (status, body) = mcp::post_local(port, &mcp::load_or_create_token(), &req.to_string())
        .map_err(|_| anyhow::anyhow!("Valerie isn't running"))?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|_| anyhow::anyhow!("HTTP {status}: {body}"))?;
    let r = &v["result"];
    for c in r["content"].as_array().into_iter().flatten() {
        match c["type"].as_str() {
            Some("text") => println!("{}", c["text"].as_str().unwrap_or("")),
            Some("image") => {
                // Save images (screenshots) to a file so scripts and assistants can open them.
                let path = std::env::temp_dir().join(format!("valerie-{tool}-{}.png", state::now()));
                match mcp::unbase64(c["data"].as_str().unwrap_or("")).map(|png| std::fs::write(&path, png)) {
                    Some(Ok(())) => println!("[image saved to {}]", path.display()),
                    _ => println!("[{} image that could not be saved]", c["mimeType"].as_str().unwrap_or("")),
                }
            }
            _ => {}
        }
    }
    if r["isError"].as_bool() == Some(true) || v.get("error").is_some() {
        std::process::exit(1);
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let loaded = Config::load();
    log::init(loaded.as_ref().map_or(Config::defaults().logs.keep_days, |c| c.logs.keep_days));
    log::set_level(&loaded.as_ref().map_or(Config::defaults().logs.level, |c| c.logs.level.clone()));
    let config = loaded.unwrap_or_else(|e| {
        log::line("config", format!("config error, using defaults: {e:#}"));
        Config::defaults()
    });
    match args.first().map(String::as_str) {
        Some("-h" | "--help" | "help") => {
            println!("{HELP}");
            return Ok(());
        }
        Some("--version") | Some("version") => {
            println!("Valerie {}", app::version());
            return Ok(());
        }
        Some("mcp-stdio") => {
            let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
            return mcp::stdio_bridge(config.mcp.port, flag("--access"), flag("--chat"));
        }
        Some("sync-setup") => return sync::setup(),
        Some("allow-from-sync") => {
            let t = trust::load();
            match args.get(1..).map(|a| a.join(" ")).filter(|a| !a.is_empty()) {
                Some(what) if trust::allow_plugin(&what) => println!("Allowed the plugin {what} on this computer."),
                Some(what) if trust::allow_program(&what) => println!("Allowed the AI program `{what}` on this computer."),
                Some(what) => anyhow::bail!("nothing called {what:?} is waiting (run `valerie allow-from-sync` to see what is)"),
                None if t.held_plugins.is_empty() && t.held_programs.is_empty() => println!("Nothing from other machines is waiting."),
                None => {
                    for name in t.held_plugins.keys() {
                        println!("plugin {name}:");
                        for p in trust::powers(&trust::files_in(&plugins::plugins_dir().join(name))) {
                            println!("  {p}");
                        }
                    }
                    for p in &t.held_programs {
                        println!("AI program: {p}");
                    }
                    println!("\nAllow one with `valerie allow-from-sync <plugin name or the program exactly as shown>`.");
                }
            }
            return Ok(());
        }
        Some("connect") => return connect::cli(&args[1..]),
        Some("mcp-config") => {
            let token = mcp::load_or_create_token();
            let exe = connect::exe()?;
            let port = config.mcp.port;
            println!("Most apps (stdio; no token needed):\n{}\n", serde_json::to_string_pretty(&serde_json::json!({ "mcpServers": { "valerie": { "command": exe, "args": ["mcp-stdio"] } } }))?);
            println!("Apps that take a URL (Streamable HTTP):\n  url: http://127.0.0.1:{port}/mcp\n  header: Authorization: Bearer {token}\n");
            println!("Claude Code:\n  claude mcp add --scope user valerie -- \"{exe}\" mcp-stdio\n");
            println!("Or let Valerie add itself to every AI app it finds: valerie connect");
            return Ok(());
        }
        Some("import-passwords") => {
            let Some(file) = args.get(1) else { anyhow::bail!("usage: valerie import-passwords <export.csv> [vault]") };
            let vault = args.get(2).cloned().or_else(|| config.profiles.first().map(|p| p.name.clone())).unwrap_or_else(|| "Personal".into());
            let n = passwords::import_csv(std::path::Path::new(file), &vault)?;
            println!("Imported {n} passwords into the {vault} vault. Delete the CSV now: it holds your passwords in plain text.");
            return Ok(());
        }
        Some("llm-guide") => {
            print!("{}", llm_guide::guide());
            return Ok(());
        }
        Some("check-config") => return check_config(),
        Some("strings") => {
            print!("{}", llm_guide::strings_template());
            return Ok(());
        }
        Some("screenshot") => return screenshot(config.mcp.port, &args[1..]),
        Some("call") => {
            let Some(tool) = args.get(1) else { anyhow::bail!("usage: valerie call <tool> [json]") };
            return call(config.mcp.port, tool, args.get(2).map(String::as_str).unwrap_or("{}"));
        }
        // Which AI apps answer right now (installed, signed in, plan active).
        // The plugin marketplace from the command line (for people and their AI assistants); it has extensions too.
        Some("plugins") => {
            use anyhow::Context;
            let index = || market::fetch(&config.plugin_marketplace.index);
            let ext_dir = extensions::dir(&config.extensions);
            let tag = |e: &market::Entry| if e.is_extension() { "  [extension]" } else { "" };
            match args.get(1).map(String::as_str) {
                Some("search") => {
                    let q = args[2..].join(" ");
                    let all = index()?;
                    let hits = market::search(&all, &q);
                    if hits.is_empty() {
                        println!("No plugin or extension like that in the marketplace yet.");
                    }
                    for e in hits.iter().take(10) {
                        println!("{:<24} {}{}{}", e.name, e.description, tag(e), if e.programs.is_empty() { "" } else { "  [runs programs]" });
                    }
                }
                Some("install") => {
                    let name = args.get(2).context("usage: valerie plugins install <name> [--extension]")?;
                    let kind = args.iter().any(|a| a == "--extension").then_some(market::EXTENSION);
                    let all = index()?;
                    let e = market::find(&all, name, kind).with_context(|| format!("nothing called {name} in the marketplace"))?;
                    if !e.programs.is_empty() {
                        println!("{name} runs these on your computer:\n{}", e.programs.join("\n---\n"));
                    }
                    let dir = market::install(e, &plugins::plugins_dir(), &ext_dir)?;
                    let next = if e.is_extension() { "Valerie asks once before loading it; needs macOS 15.4 or later" } else { "Valerie picks it up by itself" };
                    println!("Installed the {} {name} in {} ({next}).", e.noun(), dir.display());
                }
                Some("share") => {
                    let name = args.get(2).context("usage: valerie plugins share <name> [--extension]")?;
                    let kind = args.iter().any(|a| a == "--extension").then_some(market::EXTENSION);
                    let (dir, _) = market::local(name, kind, &plugins::plugins_dir(), &ext_dir)
                        .with_context(|| format!("no plugin or extension called {name} in {} or {}", plugins::plugins_dir().display(), ext_dir.display()))?;
                    let token = github::token().context("sign in to GitHub first: run Set Up Sync in Valerie, or `gh auth login`")?;
                    let all = index().unwrap_or_default();
                    println!("Shared: {}", market::share(&dir, &config.plugin_marketplace.repo, &all, &token)?);
                }
                _ => {
                    let all = index()?;
                    let mine: Vec<String> = plugins::load_all().0.into_iter().map(|p| p.name).collect();
                    let exts: Vec<String> = extensions::find(&ext_dir).into_iter().map(|f| f.name).collect();
                    for e in &all {
                        let have = if e.is_extension() { exts.contains(&e.name) } else { mine.contains(&e.name) };
                        println!("{:<24} {}{}{}", e.name, e.description, tag(e), if have { "  (installed)" } else { "" });
                    }
                    println!("\nvalerie plugins search <words> | install <name> | share <name>  (add --extension when a plugin has the same name)");
                }
            }
            return Ok(());
        }
        Some("check-agents") => {
            let path = ai::search_path();
            for k in ai::KINDS {
                let status = if ai::find_bin(k, &path).is_none() {
                    "not installed".to_string()
                } else {
                    let (_, def) = config.agent_named(k.id);
                    match ai::probe(&def) {
                        Ok(()) => format!("works{}", if def.model.is_empty() { String::new() } else { format!(" ({})", def.model) }),
                        Err(e) => format!("doesn't answer: {e:#}"),
                    }
                };
                println!("{:<12} {status}", k.title);
            }
            return Ok(());
        }
        Some("effective-config") => {
            print!("# Every setting as it applies right now: Valerie's defaults with your config.yaml on top.\n{}", serde_yaml::to_string(&config)?);
            return Ok(());
        }
        Some("default-config") => {
            print!("{}", config::DEFAULT_YAML);
            return Ok(());
        }
        Some("triage") => return triage::run_cli(),
        Some("paths") => {
            println!("config: {}\ndata:   {}\nlogs:   {}", config::config_path().display(), config::data_dir().display(), log::dir().display());
            return Ok(());
        }
        _ => {}
    }
    let urls: Vec<String> = args.into_iter().filter(|a| !a.starts_with('-')).collect();
    if !urls.is_empty() && config.mcp.enabled && mcp::forward_open(config.mcp.port, &urls) {
        return Ok(());
    }
    log::line("app", format!("Valerie {} started", app::version()));
    app::run(config, urls)
}
