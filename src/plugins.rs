//! Local plugins: folders in `<config dir>/plugins/<name>/` with a readable `plugin.yaml`, optional `style.css` and
//! `script.js` for matching sites, command-bar commands (macros, page JS or local scripts) and page-load hooks.
use crate::config::{config_dir, Binding};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default)]
    pub description: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Sites `style.css` / `script.js` apply to: hosts ("github.com", "*.edu") or URL globs. Empty = every site.
    #[serde(default)]
    pub matches: Vec<String>,
    #[serde(default)]
    pub commands: Vec<PluginCommand>,
    #[serde(default)]
    pub on_page_load: Vec<Hook>,
    /// Options for this plugin, available to `script.js` as the `settings` object.
    #[serde(default)]
    pub settings: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCommand {
    /// Shown in the command bar and the Plugins menu.
    pub title: String,
    /// Optional shortcut, e.g. "CmdOrCtrl+Shift+G".
    #[serde(default)]
    pub key: Option<String>,
    /// Built-in commands to run in order (a macro), e.g. ["next_space", { cmd: new_tab, url: "https://..." }].
    #[serde(default)]
    pub actions: Vec<Binding>,
    /// JavaScript run in the current page (as a function body).
    #[serde(default)]
    pub js: Option<String>,
    /// Shell command run in the plugin folder, with VALERIE_URL / VALERIE_TITLE / VALERIE_TAB set.
    #[serde(default)]
    pub run: Option<String>,
    /// What to do with `run`'s output. one of: toast | tab | none
    #[serde(default = "toast")]
    pub output: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hook {
    pub matches: Vec<String>,
    pub run: String,
}

#[derive(Debug, Clone)]
pub struct Plugin {
    pub name: String,
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub css: String,
    pub js: String,
}

fn yes() -> bool {
    true
}

fn toast() -> String {
    "toast".into()
}

pub fn plugins_dir() -> PathBuf {
    config_dir().join("plugins")
}

/// Load every plugin folder. Broken plugins are reported, not fatal.
pub fn load_all() -> (Vec<Plugin>, Vec<String>) {
    let (mut plugins, mut errors) = (Vec::new(), Vec::new());
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(plugins_dir()).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    for dir in dirs {
        match load(&dir) {
            Ok(Some(p)) => plugins.push(p),
            Ok(None) => {}
            Err(e) => errors.push(format!("{}: {e:#}", dir.file_name().unwrap_or_default().to_string_lossy())),
        }
    }
    (plugins, errors)
}

fn load(dir: &std::path::Path) -> Result<Option<Plugin>> {
    let manifest_path = dir.join("plugin.yaml");
    if !manifest_path.exists() {
        return Ok(None);
    }
    let manifest: Manifest = serde_yaml::from_str(&std::fs::read_to_string(&manifest_path)?).context("plugin.yaml")?;
    for c in &manifest.commands {
        for a in &c.actions {
            if a.command().is_none() {
                anyhow::bail!("command '{}': unknown action {:?}", c.title, a);
            }
        }
        if let Some(k) = &c.key {
            k.parse::<muda::accelerator::Accelerator>().map_err(|e| anyhow::anyhow!("command '{}': bad key {k:?}: {e}", c.title))?;
        }
        if !matches!(c.output.as_str(), "toast" | "tab" | "none") {
            anyhow::bail!("command '{}': output must be one of toast | tab | none", c.title);
        }
    }
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap_or_default();
    Ok(Some(Plugin {
        name: dir.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        dir: dir.to_path_buf(),
        css: read("style.css"),
        js: read("script.js"),
        manifest,
    }))
}

/// Same matching rules as routes: a host pattern also covers subdomains; patterns with "://" match the full URL.
pub fn matches(patterns: &[String], url: &str) -> bool {
    if patterns.is_empty() {
        return url.starts_with("http");
    }
    let routes: Vec<crate::config::Route> = patterns.iter().map(|p| crate::config::Route { pattern: p.clone(), space: String::new() }).collect();
    crate::routing::route(&routes, url).is_some()
}

/// Run a plugin script; returns its combined output.
pub fn run_script(dir: &std::path::Path, script: &str, env: &[(&str, String)]) -> Result<String> {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", script]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", script]);
        c
    };
    // So scripts can call `valerie call …` even when Valerie isn't on the user's PATH.
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        let mut paths: Vec<PathBuf> = vec![d];
        paths.extend(std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());
        if let Ok(joined) = std::env::join_paths(paths) {
            cmd.env("PATH", joined);
        }
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.current_dir(dir).stdin(std::process::Stdio::null()).output().context("could not start script")?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        anyhow::bail!("exited with {}: {}", out.status, text.trim());
    }
    Ok(text)
}

/// Plugins that ship with Valerie. Each is installed once; after that the folder is the user's to edit or delete.
const BUNDLED: &[(&str, &[(&str, &str)])] = &[(
    "youtube-speed",
    &[
        ("plugin.yaml", include_str!("../plugins/youtube-speed/plugin.yaml")),
        ("script.js", include_str!("../plugins/youtube-speed/script.js")),
        ("style.css", include_str!("../plugins/youtube-speed/style.css")),
    ],
)];

/// Install bundled plugins once, and keep them updated: `.installed` records a fingerprint of every file we wrote,
/// and a file is replaced by a newer bundled version only while it still matches that fingerprint (i.e. the user
/// hasn't edited it). Deleting a bundled plugin's folder keeps it deleted.
pub fn install_bundled() {
    let dir = plugins_dir();
    let record = dir.join(".installed");
    // Lines: "<plugin>" (installed) or "<plugin>/<file> <fingerprint>".
    let text = std::fs::read_to_string(&record).unwrap_or_default();
    let mut names: Vec<String> = Vec::new();
    let mut prints: std::collections::BTreeMap<String, String> = Default::default();
    for line in text.lines() {
        match line.split_once(' ') {
            Some((file, fp)) => {
                prints.insert(file.to_string(), fp.to_string());
            }
            None if !line.is_empty() => names.push(line.to_string()),
            None => {}
        }
    }
    for (name, files) in BUNDLED {
        let target = dir.join(name);
        let known = names.iter().any(|n| n == name);
        if known && !target.exists() {
            continue; // the user deleted it
        }
        if !known && target.exists() {
            continue; // a folder of the user's own with the same name
        }
        if std::fs::create_dir_all(&target).is_err() {
            continue;
        }
        for (file, body) in *files {
            let key = format!("{name}/{file}");
            let path = target.join(file);
            let current = std::fs::read_to_string(&path).ok();
            let untouched = match (&current, prints.get(&key)) {
                (None, _) => true,
                (Some(c), Some(fp)) => &crate::blocking::hash(c) == fp,
                (Some(_), None) => false,
            };
            if untouched && current.as_deref() != Some(*body) {
                let _ = std::fs::write(&path, body);
            }
            if untouched {
                prints.insert(key, crate::blocking::hash(body));
            }
        }
        if !known {
            names.push(name.to_string());
        }
    }
    let mut out: Vec<String> = names;
    out.extend(prints.into_iter().map(|(k, v)| format!("{k} {v}")));
    let _ = std::fs::write(&record, out.join("\n") + "\n");
}

/// A disabled example so people and LLMs can copy the format.
pub const EXAMPLE: &str = r#"# Example plugin (disabled). Copy this folder, rename it, and set enabled: true.
description: Shows every kind of plugin feature.
enabled: false

# Sites that style.css and script.js (next to this file) apply to. Empty = every site.
matches: ["news.ycombinator.com"]

commands:
  # A macro: built-in commands in order (see `valerie llm-guide` for the full list).
  - title: "Morning: open my dashboards"
    key: "CmdOrCtrl+Shift+M"
    actions:
      - { cmd: switch_space, name: Work }
      - { cmd: new_tab, url: "https://linear.app" }
      - { cmd: new_tab, url: "https://mail.google.com" }

  # JavaScript in the current page.
  - title: "Word count"
    js: "return document.body.innerText.split(/\\s+/).length + ' words'"

  # A local script in any language. It can call back into the browser with `valerie call <tool> '<json>'`.
  - title: "Save page to reading list"
    run: 'echo "$VALERIE_TITLE — $VALERIE_URL" >> ~/reading-list.txt && echo "Saved"'
    output: toast            # one of: toast | tab | none

# Options for this plugin; script.js reads them from the `settings` object (e.g. settings.greeting).
settings:
  greeting: "Hello from a plugin"

# Scripts that run when a matching page finishes loading.
on_page_load:
  - matches: ["arxiv.org"]
    run: 'echo "$VALERIE_URL" >> ~/papers-seen.txt'
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_plugins_update_only_untouched_files() {
        let tmp = std::env::temp_dir().join(format!("valerie-plugins-{}", crate::mcp::random_hex()));
        std::env::set_var("VALERIE_CONFIG_DIR", &tmp);
        let yt = plugins_dir().join("youtube-speed");
        install_bundled();
        assert!(yt.join("script.js").exists());
        // Simulate an older bundled version the user never touched, plus a plugin.yaml the user edited.
        std::fs::write(yt.join("script.js"), "old").unwrap();
        std::fs::write(yt.join("plugin.yaml"), "enabled: false").unwrap();
        let rec = plugins_dir().join(".installed");
        let text = std::fs::read_to_string(&rec).unwrap().replace(
            &format!("youtube-speed/script.js {}", crate::blocking::hash(BUNDLED[0].1[1].1)),
            &format!("youtube-speed/script.js {}", crate::blocking::hash("old")),
        );
        std::fs::write(&rec, text).unwrap();
        install_bundled();
        assert_eq!(std::fs::read_to_string(yt.join("script.js")).unwrap(), BUNDLED[0].1[1].1);
        assert_eq!(std::fs::read_to_string(yt.join("plugin.yaml")).unwrap(), "enabled: false");
        // Deleting the plugin keeps it deleted.
        std::fs::remove_dir_all(&yt).unwrap();
        install_bundled();
        assert!(!yt.exists());
        std::env::remove_var("VALERIE_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[test]
    fn example_manifest_parses_and_matches() {
        let m: Manifest = serde_yaml::from_str(EXAMPLE).unwrap();
        assert_eq!(m.commands.len(), 3);
        assert_eq!(m.settings["greeting"], "Hello from a plugin");
        let yt: Manifest = serde_yaml::from_str(BUNDLED[0].1[0].1).unwrap();
        assert!(yt.commands.iter().all(|c| c.actions.iter().all(|a| a.command().is_some())));
        assert!(m.commands[0].actions.iter().all(|a| a.command().is_some()));
        assert!(matches(&m.matches, "https://news.ycombinator.com/item?id=1"));
        assert!(!matches(&m.matches, "https://example.com"));
        assert!(matches(&[], "https://anything.dev"));
    }
}
