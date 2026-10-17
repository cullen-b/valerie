mod agents;
mod app;
mod blocking;
mod command;
mod config;
mod import;
mod llm_guide;
mod mcp;
mod passwords;
mod platform;
mod plugins;
mod routing;
mod state;
mod sync;
mod github;
mod ai;
mod update;
mod market;

use config::Config;

const HELP: &str = "valerie: a fast, configurable, agent-native browser

USAGE:
  valerie [URL...]        open the browser (URLs open like external links; forwarded if already running)
  valerie mcp-config      print the command that connects Claude Code to the built-in MCP server
  valerie mcp-stdio       stdio <-> HTTP bridge for MCP clients that only speak stdio
  valerie sync-setup      create a private GitHub repo for tab sync (needs the gh CLI)
  valerie import-passwords <csv> [vault]  import a Chrome/Arc/Safari/1Password CSV export into a vault
  valerie default-config  print every config option with its default
  valerie llm-guide       print the guide that lets an AI assistant customise Valerie
  valerie check-config    validate config.yaml and every plugin (prints exact errors)
  valerie call <tool> [json]  call a browser tool, e.g. valerie call list_tabs '{}'  
  valerie paths           show config and data locations";

fn check_config() -> anyhow::Result<()> {
    let mut problems = Vec::new();
    match Config::load() {
        Ok(c) => {
            for (key, binding) in &c.keybindings {
                if let Some(b) = binding {
                    if key.parse::<muda::accelerator::Accelerator>().is_err() {
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
            if !["off", "system", "always"].contains(&c.pages.dark_mode.as_str()) {
                problems.push(format!("pages: dark_mode is {:?}; use off, system or always", c.pages.dark_mode));
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
        }
        Err(e) => problems.push(format!("config.yaml: {e:#}")),
    }
    problems.extend(config::unknown_keys(&std::fs::read_to_string(config::config_path()).unwrap_or_default()));
    let (plugins, errors) = plugins::load_all();
    problems.extend(errors.into_iter().map(|e| format!("plugin {e}")));
    if problems.is_empty() {
        println!("OK: config and {} plugin(s) are valid", plugins.len());
        Ok(())
    } else {
        for p in &problems {
            eprintln!("error: {p}");
        }
        std::process::exit(1)
    }
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
    let config = Config::load().unwrap_or_else(|e| {
        eprintln!("valerie: config error, using defaults: {e:#}");
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
        Some("mcp-config") => {
            let token = mcp::load_or_create_token();
            println!(
                "claude mcp add --transport http valerie http://127.0.0.1:{}/mcp --header \"Authorization: Bearer {token}\"",
                config.mcp.port
            );
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
        Some("call") => {
            let Some(tool) = args.get(1) else { anyhow::bail!("usage: valerie call <tool> [json]") };
            return call(config.mcp.port, tool, args.get(2).map(String::as_str).unwrap_or("{}"));
        }
        // Which AI apps answer right now (installed, signed in, plan active).
        // The plugin marketplace from the command line (for people and their AI assistants).
        Some("plugins") => {
            use anyhow::Context;
            let index = || market::fetch(&config.plugin_marketplace.index);
            match args.get(1).map(String::as_str) {
                Some("search") => {
                    let q = args[2..].join(" ");
                    let all = index()?;
                    let hits = market::search(&all, &q);
                    if hits.is_empty() {
                        println!("No plugin like that in the marketplace yet.");
                    }
                    for e in hits.iter().take(10) {
                        println!("{:<24} {}{}", e.name, e.description, if e.programs.is_empty() { "" } else { "  [runs programs]" });
                    }
                }
                Some("install") => {
                    let name = args.get(2).context("usage: valerie plugins install <name>")?;
                    let all = index()?;
                    let e = all.iter().find(|e| &e.name == name).with_context(|| format!("no plugin called {name} in the marketplace"))?;
                    if !e.programs.is_empty() {
                        println!("{name} runs these on your computer:\n{}", e.programs.join("\n---\n"));
                    }
                    let dir = market::install(e, &plugins::plugins_dir())?;
                    println!("Installed {name} in {} (Valerie picks it up by itself).", dir.display());
                }
                Some("share") => {
                    let name = args.get(2).context("usage: valerie plugins share <name>")?;
                    let dir = plugins::plugins_dir().join(name);
                    let token = github::token().context("sign in to GitHub first: run Set Up Sync in Valerie, or `gh auth login`")?;
                    let all = index().unwrap_or_default();
                    println!("Shared: {}", market::share(&dir, &config.plugin_marketplace.repo, &all, &token)?);
                }
                _ => {
                    let all = index()?;
                    let mine: Vec<String> = plugins::load_all().0.into_iter().map(|p| p.name).collect();
                    for e in &all {
                        println!("{:<24} {}{}", e.name, e.description, if mine.contains(&e.name) { "  (installed)" } else { "" });
                    }
                    println!("\nvalerie plugins search <words> | install <name> | share <name>");
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
        Some("paths") => {
            println!("config: {}\ndata:   {}", config::config_path().display(), config::data_dir().display());
            return Ok(());
        }
        _ => {}
    }
    let urls: Vec<String> = args.into_iter().filter(|a| !a.starts_with('-')).collect();
    if !urls.is_empty() && config.mcp.enabled && mcp::forward_open(config.mcp.port, &urls) {
        return Ok(());
    }
    app::run(config, urls)
}
