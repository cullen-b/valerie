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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncNode {
    Tab {
        title: String,
        url: String,
        #[serde(default)]
        custom_title: Option<String>,
    },
    Folder {
        name: String,
        children: Vec<SyncNode>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncedSpace {
    pub name: String,
    pub pinned: Vec<SyncedTab>,
    pub tabs: Vec<SyncedTab>,
    /// The pinned tree and when it was last edited; the newest edit across devices wins.
    #[serde(default)]
    pub tree: Vec<SyncNode>,
    #[serde(default)]
    pub kept: Vec<SyncNode>,
    #[serde(default)]
    pub modified: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceTabs {
    pub device: String,
    pub updated: u64,
    pub spaces: Vec<SyncedSpace>,
    #[serde(default)]
    pub favorites: Vec<SyncNode>,
    #[serde(default)]
    pub favorites_modified: u64,
}

/// Run git in `dir`. With a token, git gets it through a one-off credential helper (it's never written to disk).
fn git_auth(dir: &Path, args: &[&str], token: Option<&str>) -> Result<String> {
    let mut c = Command::new("git");
    c.arg("-C").arg(dir).env("GIT_TERMINAL_PROMPT", "0");
    if let Some(t) = token {
        c.args(["-c", "credential.helper=", "-c", "credential.helper=!f() { echo username=x-access-token; echo \"password=$VALERIE_GIT_TOKEN\"; }; f"]);
        c.env("VALERIE_GIT_TOKEN", t);
    }
    let out = c.args(args).output().context("git isn't installed")?;
    if !out.status.success() {
        bail!("git {}: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    git_auth(dir, args, None)
}

fn safe_name(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect()
}

/// Files in the config folder that are this machine's own (regenerated each launch) or secret: never synced.
const GITIGNORE: &str = "# Valerie writes these on every launch for this machine; they aren't synced.
AGENTS.md
reference/
plugins/.installed
*.before-sync.yaml
# Never sync secrets.
*.csv
.env
.DS_Store
";

/// Turn the config folder into a checkout of `repo`. When the repo already has settings (set up on another
/// machine), those win; this machine's old config.yaml is kept as config.before-sync.yaml.
fn ensure_repo(repo: &str, dir: &Path, token: Option<&str>) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    if dir.join(".git").exists() {
        let current = git(dir, &["remote", "get-url", "origin"]).unwrap_or_default();
        if current.trim() != repo {
            if current.trim().is_empty() {
                git(dir, &["remote", "add", "origin", repo])?;
            } else {
                git(dir, &["remote", "set-url", "origin", repo])?;
            }
        }
        return Ok(());
    }
    git(dir, &["init", "-q", "-b", "main"])?;
    git(dir, &["remote", "add", "origin", repo])?;
    let heads = git_auth(dir, &["ls-remote", "--heads", "origin"], token)?;
    if let Some(branch) = heads.lines().filter_map(|l| l.split("refs/heads/").nth(1)).find(|b| *b == "main").or_else(|| heads.lines().filter_map(|l| l.split("refs/heads/").nth(1)).next()) {
        let branch = branch.trim().to_string();
        git_auth(dir, &["fetch", "-q", "origin", &branch], token)?;
        let cfg = dir.join("config.yaml");
        if cfg.exists() && git(dir, &["cat-file", "-e", &format!("origin/{branch}:config.yaml")]).is_ok() {
            std::fs::copy(&cfg, dir.join("config.before-sync.yaml"))?;
        }
        git(dir, &["checkout", "-q", "-f", "-B", "main", &format!("origin/{branch}")])?;
    }
    Ok(())
}

/// Sync the config folder (settings, boosts, plugins, every device's tabs) with `repo`: commit what changed here,
/// rebase onto the other machines' changes (this machine wins a clash on the same line), push. Returns the other
/// devices' tabs.
pub fn run(repo: &str, dir: &Path, me: &DeviceTabs, token: Option<&str>) -> Result<Vec<DeviceTabs>> {
    ensure_repo(repo, dir, token)?;
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, GITIGNORE)?;
    }
    let devices = dir.join("devices");
    std::fs::create_dir_all(&devices)?;
    // Only rewrite this device's file when its tabs changed, so an idle browser makes no commits.
    let file = devices.join(format!("{}.json", safe_name(&me.device)));
    let mut old: serde_json::Value = std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    if old.is_object() {
        old["updated"] = serde_json::json!(me.updated);
    }
    if old != serde_json::to_value(me)? {
        std::fs::write(&file, serde_json::to_string_pretty(me)?)?;
    }
    let who = ["-c", "user.name=Valerie", "-c", "user.email=valerie@localhost"];
    git(dir, &["add", "-A"])?;
    if git(dir, &["diff", "--cached", "--quiet"]).is_err() {
        git(dir, &[&who[..], &["commit", "-q", "-m", &format!("Sync from {}", me.device)]].concat())?;
    }
    if git_auth(dir, &["fetch", "-q", "origin"], token).is_ok() && git(dir, &["rev-parse", "--verify", "-q", "origin/main"]).is_ok() {
        // In a rebase "theirs" is the commit being replayed: this machine's change wins a same-line clash.
        if let Err(e) = git(dir, &[&who[..], &["rebase", "-q", "-X", "theirs", "--autostash", "origin/main"]].concat()) {
            let _ = git(dir, &["rebase", "--abort"]);
            bail!("couldn't merge the other machines' changes: {e}");
        }
    }
    if git(dir, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok() {
        let ahead = git(dir, &["rev-list", "--count", "origin/main..HEAD"]).map(|n| n.trim() != "0").unwrap_or(true);
        if ahead {
            git_auth(dir, &["push", "-q", "-u", "origin", "HEAD:main"], token)?;
        }
    }
    let mut others = Vec::new();
    for entry in std::fs::read_dir(&devices)?.flatten() {
        if entry.path() == file {
            continue;
        }
        if let Ok(d) = std::fs::read_to_string(entry.path()).map_err(anyhow::Error::from).and_then(|s| Ok(serde_json::from_str::<DeviceTabs>(&s)?)) {
            others.push(d);
        }
    }
    others.sort_by(|a, b| b.updated.cmp(&a.updated));
    Ok(others)
}

/// Point `sync.repo` in the user's config.yaml at `url`, keeping the rest of the file (and its comments) as it is.
pub fn write_repo_setting(url: &str) -> Result<()> {
    let path = crate::config::config_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let line = format!("  repo: \"{url}\"");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    match lines.iter().position(|l| l.trim_end() == "sync:") {
        Some(i) => {
            let end = lines[i + 1..].iter().position(|l| !l.starts_with(' ') && !l.trim().is_empty()).map_or(lines.len(), |j| i + 1 + j);
            match lines[i + 1..end].iter().position(|l| l.trim_start().starts_with("repo:")) {
                Some(j) => lines[i + 1 + j] = line,
                None => lines.insert(i + 1, line),
            }
        }
        None if lines.iter().any(|l| l.starts_with("sync:")) => {
            // `sync: { … }` on one line: rewrite through YAML (this file's comments survive elsewhere).
            let mut v: serde_yaml::Value = serde_yaml::from_str(&text)?;
            v["sync"]["repo"] = serde_yaml::Value::String(url.to_string());
            return Ok(std::fs::write(&path, serde_yaml::to_string(&v)?)?);
        }
        None => {
            lines.push(String::new());
            lines.push("# Settings, plugins and tabs sync through this private GitHub repo (set up by \"Set Up Sync\").".into());
            lines.push("sync:".into());
            lines.push(line);
        }
    }
    std::fs::write(&path, lines.join("\n") + "\n")?;
    Ok(())
}

/// `valerie sync-setup`: sign in to GitHub, create the private repo, point the config at it.
pub fn setup() -> Result<()> {
    let config = crate::config::Config::load()?;
    let step = crate::github::set_up(&config.sync.github_client_id, &config.sync.repo_name, &|s| {
        if let crate::github::Step::Code { code, url } = s {
            println!("Open {url} and enter the code {code}");
            crate::platform::open_external(&url);
        }
    });
    match step {
        crate::github::Step::Done { login, repo } => {
            write_repo_setting(&repo)?;
            println!("Signed in as {login}. Valerie syncs settings and tabs with {repo}.");
            Ok(())
        }
        crate::github::Step::Failed(e) => bail!(e),
        crate::github::Step::Code { .. } => Ok(()),
    }
}
