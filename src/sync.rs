//! Tab sync through a private GitHub repo. Each device only writes `devices/<device>.json`, so pushes never conflict.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncedTab {
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncedSpace {
    pub name: String,
    pub pinned: Vec<SyncedTab>,
    pub tabs: Vec<SyncedTab>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceTabs {
    pub device: String,
    pub updated: u64,
    pub spaces: Vec<SyncedSpace>,
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().context("git not found")?;
    if !out.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn safe_name(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect()
}

/// Push this device's tabs and return every other device's tabs.
pub fn run(repo: &str, dir: &Path, me: &DeviceTabs) -> Result<Vec<DeviceTabs>> {
    if !dir.join(".git").exists() {
        std::fs::create_dir_all(dir.parent().unwrap_or(dir))?;
        let out = Command::new("git").arg("clone").arg(repo).arg(dir).output().context("git not found")?;
        if !out.status.success() {
            bail!("git clone {repo}: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
    }
    // An empty remote has nothing to pull yet.
    if git(dir, &["rev-parse", "--verify", "HEAD"]).is_ok() {
        let _ = git(dir, &["pull", "--rebase", "--autostash", "-q"]);
    }
    let devices = dir.join("devices");
    std::fs::create_dir_all(&devices)?;
    let file = format!("devices/{}.json", safe_name(&me.device));
    std::fs::write(dir.join(&file), serde_json::to_string_pretty(me)?)?;
    git(dir, &["add", &file])?;
    if git(dir, &["diff", "--cached", "--quiet"]).is_err() {
        git(dir, &["commit", "-q", "-m", &format!("sync {}", me.device)])?;
        if git(dir, &["push", "-q", "-u", "origin", "HEAD"]).is_err() {
            git(dir, &["pull", "--rebase", "-q"])?;
            git(dir, &["push", "-q", "-u", "origin", "HEAD"])?;
        }
    }
    let mut others = Vec::new();
    for entry in std::fs::read_dir(&devices)?.flatten() {
        if entry.file_name().to_string_lossy() == format!("{}.json", safe_name(&me.device)) {
            continue;
        }
        if let Ok(d) = std::fs::read_to_string(entry.path()).map_err(anyhow::Error::from).and_then(|s| Ok(serde_json::from_str::<DeviceTabs>(&s)?)) {
            others.push(d);
        }
    }
    others.sort_by(|a, b| b.updated.cmp(&a.updated));
    Ok(others)
}

/// `valerie sync-setup`: create a private repo with the GitHub CLI and point the config at it.
pub fn setup() -> Result<()> {
    let name = "valerie-sync";
    let view = Command::new("gh").args(["repo", "view", name, "--json", "sshUrl", "-q", ".sshUrl"]).output();
    let url = match view {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => {
            let o = Command::new("gh")
                .args(["repo", "create", name, "--private", "--description", "Valerie browser tab sync"])
                .output()
                .context("the GitHub CLI (gh) is required: https://cli.github.com")?;
            if !o.status.success() {
                bail!("gh repo create failed: {}", String::from_utf8_lossy(&o.stderr).trim());
            }
            let o = Command::new("gh").args(["repo", "view", name, "--json", "sshUrl", "-q", ".sshUrl"]).output()?;
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }
    };
    if url.is_empty() {
        bail!("could not determine the repo URL");
    }
    let path = crate::config::config_path();
    crate::config::Config::load()?;
    let text = std::fs::read_to_string(&path)?;
    if text.contains("repo: \"\"") {
        std::fs::write(&path, text.replacen("repo: \"\"", &format!("repo: \"{url}\""), 1))?;
        println!("Sync repo {url} written to {}", path.display());
    } else {
        println!("Sync repo: {url}\nSet `sync.repo` in {} to enable sync.", path.display());
    }
    Ok(())
}
