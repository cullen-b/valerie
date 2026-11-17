//! Saved settings: whole copies of config.yaml kept under a name, to switch between (a daily set, a demo set, …).
//! Each lives in the config folder's `saved-settings/<name>.yaml`; `saved-settings/.current` names the one in use.

use crate::config::{self, Config};
use anyhow::{bail, Context, Result};
use std::path::PathBuf;

pub fn dir() -> PathBuf {
    config::config_dir().join("saved-settings")
}

fn file(name: &str) -> PathBuf {
    dir().join(format!("{name}.yaml"))
}

fn current_file() -> PathBuf {
    dir().join(".current")
}

/// A name that's safe as a file name on every system.
pub fn check_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 || name.starts_with('.') || name.ends_with('.') {
        bail!("give it a name of 1 to 60 characters that doesn't start or end with a dot");
    }
    if name.chars().any(|c| c.is_control() || "/\\:*?\"<>|".contains(c)) {
        bail!("a name can't contain / \\ : * ? \" < > |");
    }
    Ok(name.to_string())
}

/// The saved names, A to Z.
pub fn list() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".yaml")).map(str::to_string))
        .filter(|n| check_name(n).is_ok())
        .collect();
    out.sort_by_key(|n| n.to_lowercase());
    out
}

/// The saved settings in use, if it's still saved.
pub fn current() -> Option<String> {
    let name = std::fs::read_to_string(current_file()).ok()?.trim().to_string();
    file(&name).exists().then_some(name)
}

fn set_current(name: Option<&str>) -> Result<()> {
    match name {
        Some(n) => std::fs::write(current_file(), n)?,
        None => {
            let _ = std::fs::remove_file(current_file());
        }
    }
    Ok(())
}

fn find(name: &str) -> Result<String> {
    let all = list();
    all.iter()
        .find(|n| n.as_str() == name.trim())
        .or_else(|| all.iter().find(|n| n.eq_ignore_ascii_case(name.trim())))
        .cloned()
        .with_context(|| format!("there are no saved settings called {name:?} (saved: {})", if all.is_empty() { "none yet".to_string() } else { all.join(", ") }))
}

/// Keep the settings in use now (config.yaml) under `name`, replacing one by that name; it becomes the one in use.
pub fn save(name: &str) -> Result<String> {
    let name = check_name(name)?;
    std::fs::create_dir_all(dir())?;
    let text = std::fs::read_to_string(config::config_path()).unwrap_or_default();
    std::fs::write(file(&name), text)?;
    set_current(Some(&name))?;
    Ok(name)
}

/// Make `name` the settings in use. With `keep_changes`, the settings in use first go back into their own saved copy;
/// settings that aren't saved anywhere are kept as "Before <name>" either way, so switching never loses any.
pub fn switch(name: &str, keep_changes: bool) -> Result<String> {
    let name = find(name)?;
    let text = std::fs::read_to_string(file(&name))?;
    Config::merged(&text).and_then(|v| Ok(serde_yaml::from_value::<Config>(v)?)).with_context(|| format!("the saved settings {name:?} have a mistake"))?;
    let now = std::fs::read_to_string(config::config_path()).unwrap_or_default();
    match current().filter(|c| *c != name) {
        Some(c) if keep_changes => {
            std::fs::write(file(&c), &now)?;
        }
        _ if !now.trim().is_empty() && !list().iter().any(|n| std::fs::read_to_string(file(n)).is_ok_and(|t| t == now)) => {
            let keep = check_name(&format!("Before {name}")).unwrap_or_else(|_| "Before switching".into());
            std::fs::write(file(&keep), &now)?;
        }
        _ => {}
    }
    crate::history::before_change(&format!("Switched to the saved settings {name:?}"));
    std::fs::write(config::config_path(), text)?;
    set_current(Some(&name))?;
    Ok(name)
}

/// The saved settings after the one in use, A to Z, wrapping around.
pub fn next() -> Option<String> {
    let all = list();
    let at = current().and_then(|c| all.iter().position(|n| *n == c));
    all.get(at.map_or(0, |i| (i + 1) % all.len().max(1))).cloned()
}

/// Forget saved settings (the settings in use stay as they are).
pub fn delete(name: &str) -> Result<String> {
    let name = find(name)?;
    std::fs::remove_file(file(&name))?;
    if current().is_none() {
        set_current(None)?;
    }
    Ok(name)
}

/// For pages and agents: every saved name and which is in use.
pub fn summary() -> serde_json::Value {
    serde_json::json!({ "saved": list(), "current": current(), "folder": dir() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_must_be_file_safe() {
        assert!(check_name("  Demo: fun ").is_err());
        assert_eq!(check_name(" Party mode ").unwrap(), "Party mode");
        assert!(check_name("../x").is_err());
        assert!(check_name(".hidden").is_err());
        assert!(check_name("").is_err());
    }
}
