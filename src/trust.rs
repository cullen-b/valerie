//! What another machine sends through sync isn't trusted until you say so on this one. A plugin that can act (run
//! programs, run scripts in pages, use browser tools) or an agent `command` that arrives from the sync repo is held:
//! it doesn't load or run here until you allow it. Things made on this machine are never held.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Trust {
    /// Plugins held until allowed: name → fingerprint of its files when it arrived.
    pub held_plugins: BTreeMap<String, String>,
    /// Plugins allowed here: name → fingerprint allowed (a later change from another machine is held again).
    pub allowed_plugins: BTreeMap<String, String>,
    /// Agent `command`s held until allowed (each joined with spaces).
    pub held_programs: Vec<String>,
    pub allowed_programs: Vec<String>,
}

fn path() -> PathBuf {
    crate::config::data_dir().join("trust.json")
}

pub fn load() -> Trust {
    std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(t: &Trust) {
    let p = path();
    let _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
    if let Ok(s) = serde_json::to_string_pretty(t) {
        let _ = std::fs::write(p, s);
    }
}

/// A fingerprint of a plugin's files (path and contents), wherever they're read from.
pub fn fingerprint(files: &[(String, Vec<u8>)]) -> String {
    let mut sorted: Vec<&(String, Vec<u8>)> = files.iter().filter(|(p, _)| !p.ends_with(".DS_Store")).collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Sha256::new();
    for (p, c) in sorted {
        h.update(p.as_bytes());
        h.update([0]);
        h.update((c.len() as u64).to_le_bytes());
        h.update(c);
    }
    crate::crypt::hex(&h.finalize()[..16])
}

/// The files of a plugin folder on disk, with paths relative to it.
pub fn files_in(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    fn walk(base: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else if let (Ok(rel), Ok(c)) = (p.strip_prefix(base), std::fs::read(&p)) {
                out.push((rel.to_string_lossy().replace('\\', "/"), c));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

/// What a plugin can do beyond restyling pages, in plain words; empty when it only has style.css and built-in
/// commands.
pub fn powers(files: &[(String, Vec<u8>)]) -> Vec<String> {
    let get = |n: &str| files.iter().find(|(p, _)| p == n).map(|(_, c)| String::from_utf8_lossy(c).into_owned()).filter(|s| !s.trim().is_empty());
    let mut out = Vec::new();
    if get("script.js").is_some() {
        out.push("runs its script in web pages".to_string());
    }
    if get("start.html").is_some() {
        out.push("shows its own start page".to_string());
    }
    let Some(yaml) = get("plugin.yaml") else { return out };
    let v: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap_or_default();
    // A disabled plugin does nothing (turning it on from another machine is a change, held like any other).
    if v.get("enabled").and_then(|e| e.as_bool()) == Some(false) {
        return Vec::new();
    }
    let list = |k: &str| v.get(k).and_then(|x| x.as_sequence()).cloned().unwrap_or_default();
    let mut programs: Vec<String> = Vec::new();
    for c in list("commands") {
        if let Some(r) = c.get("run").and_then(|r| r.as_str()) {
            programs.push(r.to_string());
        }
        if c.get("js").and_then(|r| r.as_str()).is_some() && !out.iter().any(|o| o.contains("web pages")) {
            out.push("runs scripts in web pages".to_string());
        }
    }
    for k in ["on_page_load", "events", "search"] {
        programs.extend(list(k).iter().filter_map(|e| e.get("run").and_then(|r| r.as_str()).map(str::to_string)));
    }
    programs.extend(list("sidebar").iter().filter_map(|w| w.get("text_from").and_then(|r| r.as_str()).map(str::to_string)));
    for p in programs {
        out.push(format!("runs a program: {}", p.trim()));
    }
    let perms: Vec<String> = list("permissions").iter().filter_map(|p| p.as_str().map(str::to_string)).collect();
    if !perms.is_empty() {
        out.push(format!("uses browser tools: {}", perms.join(", ")));
    }
    out
}

/// Whether the plugin called `name` may load here.
pub fn plugin_allowed(t: &Trust, name: &str) -> bool {
    !t.held_plugins.contains_key(name)
}

pub fn allow_plugin(name: &str) -> bool {
    let mut t = load();
    let Some(fp) = t.held_plugins.remove(name) else { return false };
    let dir = crate::plugins::plugins_dir().join(name);
    let now = if dir.exists() { fingerprint(&files_in(&dir)) } else { fp };
    t.allowed_plugins.insert(name.to_string(), now);
    save(&t);
    true
}

pub fn program_allowed(t: &Trust, command: &[String]) -> bool {
    let joined = command.join(" ");
    !t.held_programs.contains(&joined)
}

pub fn allow_program(joined: &str) -> bool {
    let mut t = load();
    let before = t.held_programs.len();
    t.held_programs.retain(|p| p != joined);
    if t.held_programs.len() == before {
        return false;
    }
    if !t.allowed_programs.iter().any(|p| p == joined) {
        t.allowed_programs.push(joined.to_string());
    }
    save(&t);
    true
}

/// Record what arrived from other machines: plugins (name → their files as they arrived) and agent commands.
/// Returns the names of plugins and programs newly held.
pub fn hold_arrivals(plugins: &[(String, Vec<(String, Vec<u8>)>)], programs: &[String]) -> (Vec<String>, Vec<String>) {
    let mut t = load();
    let (mut new_plugins, mut new_programs) = (Vec::new(), Vec::new());
    for (name, files) in plugins {
        if files.is_empty() || powers(files).is_empty() || crate::plugins::shipped(name, files) {
            continue;
        }
        let fp = fingerprint(files);
        if t.allowed_plugins.get(name) == Some(&fp) {
            continue;
        }
        if t.held_plugins.insert(name.clone(), fp).is_none() {
            new_plugins.push(name.clone());
        }
    }
    for p in programs {
        if t.allowed_programs.contains(p) || t.held_programs.contains(p) {
            continue;
        }
        t.held_programs.push(p.clone());
        new_programs.push(p.clone());
    }
    if !new_plugins.is_empty() || !new_programs.is_empty() {
        save(&t);
    }
    (new_plugins, new_programs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_what_a_plugin_can_do() {
        let css_only = vec![("plugin.yaml".to_string(), b"description: x\n".to_vec()), ("style.css".into(), b"a{}".to_vec())];
        assert!(powers(&css_only).is_empty());
        let runs = vec![("plugin.yaml".to_string(), b"commands:\n  - title: Go\n    run: rm -rf ~\nsidebar:\n  - title: T\n    text_from: date\n".to_vec())];
        let p = powers(&runs);
        assert!(p.iter().any(|x| x.contains("rm -rf ~")) && p.iter().any(|x| x.contains("date")), "{p:?}");
        let off = vec![("plugin.yaml".to_string(), b"enabled: false\ncommands:\n  - title: Go\n    run: rm -rf ~\n".to_vec())];
        assert!(powers(&off).is_empty());
        let script = vec![("plugin.yaml".to_string(), b"{}".to_vec()), ("script.js".into(), b"alert(1)".to_vec())];
        assert_eq!(powers(&script), vec!["runs its script in web pages".to_string()]);
        assert_ne!(fingerprint(&runs), fingerprint(&script));
        assert_eq!(fingerprint(&css_only), fingerprint(&css_only.iter().rev().cloned().collect::<Vec<_>>()));
    }
}
