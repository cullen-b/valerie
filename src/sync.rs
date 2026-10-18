//! Sync through a private GitHub repo: the config folder is a checkout of it. Settings and plugins are plain files
//! (merged line by line); each machine's tabs, chats and passwords are encrypted with the sync passphrase. Each
//! machine only writes files named after its own id, so pushes never conflict.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
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
    /// The machine's name as people see it.
    pub device: String,
    /// The machine's id (its files in the repo are named after it).
    #[serde(default)]
    pub id: String,
    pub updated: u64,
    pub spaces: Vec<SyncedSpace>,
    #[serde(default)]
    pub favorites: Vec<SyncNode>,
    #[serde(default)]
    pub favorites_modified: u64,
}

/// How to reach the repo and what to sync.
#[derive(Default)]
pub struct Options {
    /// The address git uses (the SSH one with a repo key).
    pub remote: String,
    /// A GitHub sign-in for an https address.
    pub token: Option<String>,
    /// GIT_SSH_COMMAND for this machine's repo key.
    pub ssh: Option<String>,
    /// Encrypt tabs, chats and passwords (sync.encrypt).
    pub encrypt: bool,
    /// The sync passphrase's key, when this machine has it.
    pub key: Option<crate::crypt::Key>,
    /// This machine's chats folder, when chats sync.
    pub chats: Option<PathBuf>,
    /// Password vaults to sync (empty = passwords don't).
    pub vaults: Vec<String>,
    /// Hold plugins and agent programs other machines add until they're allowed here.
    pub hold: bool,
    /// Replace the repo's history with one commit (after encryption is turned on, so nothing readable stays).
    pub fresh_history: bool,
    /// Names this machine's tabs file had before ids (removed once it writes under its id).
    pub old_names: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub devices: Vec<DeviceTabs>,
    /// Plugins and agent programs from other machines now waiting to be allowed.
    pub held_plugins: Vec<String>,
    pub held_programs: Vec<String>,
    /// The repo is encrypted and this machine doesn't have the passphrase yet.
    pub locked: bool,
    /// The repo has a sync passphrase.
    pub has_passphrase: bool,
    pub passwords_changed: usize,
    pub chats_changed: bool,
}

struct Git<'a> {
    dir: &'a Path,
    token: Option<&'a str>,
    ssh: Option<&'a str>,
}

impl Git<'_> {
    /// Run git; with a token it goes through a one-off credential helper (never written to disk).
    fn run(&self, args: &[&str]) -> Result<String> {
        let mut c = Command::new("git");
        c.arg("-C").arg(self.dir).env("GIT_TERMINAL_PROMPT", "0");
        if let Some(t) = self.token {
            c.args(["-c", "credential.helper=", "-c", "credential.helper=!f() { echo username=x-access-token; echo \"password=$VALERIE_GIT_TOKEN\"; }; f"]);
            c.env("VALERIE_GIT_TOKEN", t);
        }
        if let Some(s) = self.ssh {
            c.env("GIT_SSH_COMMAND", s);
        }
        let out = c.args(["-c", "user.name=Valerie", "-c", "user.email=valerie@localhost"]).args(args).output().context("git isn't installed")?;
        if !out.status.success() {
            bail!("git {}: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn ok(&self, args: &[&str]) -> bool {
        self.run(args).is_ok()
    }

    fn show(&self, rev: &str, path: &str) -> Option<Vec<u8>> {
        let out = Command::new("git").arg("-C").arg(self.dir).args(["show", &format!("{rev}:{path}")]).output().ok()?;
        out.status.success().then_some(out.stdout)
    }
}

fn safe_name(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect()
}

/// Files in the config folder that are this machine's own (regenerated each launch) or secret: never synced.
const GITIGNORE: &[&str] = &[
    "# Valerie writes these on every launch for this machine; they aren't synced.",
    "AGENTS.md",
    "reference/",
    "plugins/.installed",
    "*.before-sync.yaml",
    "# This computer's own settings.",
    "this-mac.yaml",
    "# Chats travel encrypted (chats/*.enc); the readable copies stay on each machine.",
    "chats/*.json",
    "chats/closed/",
    "# Never sync secrets or stray backups.",
    "*.csv",
    ".env",
    "*.bak",
    "*.bak-*",
    ".DS_Store",
];

/// Add any lines Valerie needs to .gitignore (keeping the user's own), and stop tracking files it now ignores.
fn ignore_files(g: &Git) -> Result<()> {
    let path = g.dir.join(".gitignore");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let missing: Vec<&str> = GITIGNORE.iter().copied().filter(|l| !l.starts_with('#') && !text.lines().any(|x| x.trim() == *l)).collect();
    if !missing.is_empty() {
        let mut t = text.trim_end().to_string();
        if t.is_empty() {
            t = GITIGNORE.join("\n");
        } else {
            t.push_str("\n");
            t.push_str(&missing.join("\n"));
        }
        std::fs::write(&path, t + "\n")?;
    }
    let tracked = g.run(&["ls-files", "-ci", "--exclude-standard"]).unwrap_or_default();
    for f in tracked.lines().filter(|l| !l.is_empty()) {
        let _ = g.run(&["rm", "-q", "--cached", "--", f]);
    }
    Ok(())
}

/// Plugin folders (name → their files) and agent programs as they are in `rev`, for those that `rev` changed since
/// `since` (everything when `since` is None).
fn arrivals(g: &Git, since: Option<&str>, rev: &str) -> (Vec<(String, Vec<(String, Vec<u8>)>)>, Vec<String>) {
    let changed: Vec<String> = match since {
        Some(base) => g.run(&["diff", "--name-only", &format!("{base}...{rev}")]).unwrap_or_default().lines().map(str::to_string).collect(),
        None => g.run(&["ls-tree", "-r", "--name-only", rev]).unwrap_or_default().lines().map(str::to_string).collect(),
    };
    let mut names: Vec<String> = changed.iter().filter_map(|p| p.strip_prefix("plugins/")?.split_once('/').map(|(n, _)| n.to_string())).collect();
    names.sort();
    names.dedup();
    let plugins = names
        .into_iter()
        .map(|n| {
            let files = g
                .run(&["ls-tree", "-r", "--name-only", rev, "--", &format!("plugins/{n}/")])
                .unwrap_or_default()
                .lines()
                .filter_map(|p| Some((p.strip_prefix(&format!("plugins/{n}/"))?.to_string(), g.show(rev, p)?)))
                .collect();
            (n, files)
        })
        .collect();
    let programs = if changed.iter().any(|p| p == "config.yaml") {
        let cmds = |text: Option<Vec<u8>>| -> Vec<String> {
            let v: serde_yaml::Value = text.and_then(|t| serde_yaml::from_slice(&t).ok()).unwrap_or_default();
            v.get("agents")
                .and_then(|a| a.get("available"))
                .and_then(|a| a.as_mapping())
                .into_iter()
                .flatten()
                .filter_map(|(_, d)| d.get("command")?.as_sequence().map(|s| s.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" ")))
                .filter(|c| !c.is_empty())
                .collect()
        };
        let theirs = cmds(g.show(rev, "config.yaml"));
        let mine = since.map(|b| cmds(g.show(b, "config.yaml"))).unwrap_or_default();
        theirs.into_iter().filter(|c| !mine.contains(c)).collect()
    } else {
        Vec::new()
    };
    (plugins, programs)
}

/// What this machine last wrote, so unchanged things aren't re-encrypted (fresh ciphertext every time would make a
/// commit on every sync).
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    written: BTreeMap<String, String>,
    passwords: String,
    /// The passphrase (by salt) whose chats have all been read here.
    chats_read: String,
}

fn state_path() -> PathBuf {
    crate::config::data_dir().join("sync-state.json")
}

fn digest(b: &[u8]) -> String {
    use sha2::Digest;
    crate::crypt::hex(&sha2::Sha256::digest(b)[..16])
}

/// Write `plain` to `path` in the repo (encrypted with `key`) unless it's what was written last time.
fn put(st: &mut State, dir: &Path, path: &str, plain: &[u8], key: &crate::crypt::Key) -> Result<()> {
    let d = digest(plain);
    let file = dir.join(path);
    if st.written.get(path) == Some(&d) && file.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(file.parent().context("repo path")?)?;
    std::fs::write(&file, crate::crypt::seal(key, plain))?;
    st.written.insert(path.to_string(), d);
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct ChatFile {
    closed: bool,
    chat: serde_json::Value,
}

/// This machine's own files: its tabs, and (encrypted) its passwords and chats.
fn write_mine(dir: &Path, me: &DeviceTabs, opt: &Options, st: &mut State) -> Result<()> {
    let devices = dir.join("devices");
    std::fs::create_dir_all(&devices)?;
    for old in &opt.old_names {
        let f = devices.join(format!("{}.json", safe_name(old)));
        let _ = std::fs::remove_file(f);
    }
    let mut tabs = serde_json::to_value(me)?;
    tabs["updated"] = serde_json::json!(0);
    let plain = |v: &serde_json::Value| serde_json::to_vec_pretty(v).unwrap_or_default();
    if !opt.encrypt {
        let file = devices.join(format!("{}.json", me.id));
        let mut old: serde_json::Value = std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        if old.is_object() {
            old["updated"] = serde_json::json!(0);
        }
        if old != tabs {
            std::fs::write(&file, serde_json::to_string_pretty(me)?)?;
        }
        return Ok(());
    }
    let _ = std::fs::remove_file(devices.join(format!("{}.json", me.id)));
    let Some(key) = &opt.key else { return Ok(()) };
    // The tabs file only changes when the tabs do (the time inside it is set fresh when it's written).
    let d = digest(&plain(&tabs));
    let path = format!("devices/{}.json.enc", me.id);
    if st.written.get(&path) != Some(&d) || !dir.join(&path).exists() {
        std::fs::write(dir.join(&path), crate::crypt::seal(key, &serde_json::to_vec(me)?))?;
        st.written.insert(path, d);
    }
    if !opt.vaults.is_empty() {
        let sig = crate::passwords::signature(&opt.vaults);
        let path = format!("devices/{}.passwords.enc", me.id);
        if sig != st.passwords || !dir.join(&path).exists() {
            let all = crate::passwords::export(&opt.vaults);
            std::fs::write(dir.join(&path), crate::crypt::seal(key, &serde_json::to_vec(&all)?))?;
            st.passwords = sig;
        }
    }
    if let Some(chats) = &opt.chats {
        for (sub, closed) in [("", false), ("closed", true)] {
            for e in std::fs::read_dir(chats.join(sub)).into_iter().flatten().flatten() {
                let p = e.path();
                let Some(id) = p.file_stem().and_then(|s| s.to_str()).filter(|_| p.extension().is_some_and(|x| x == "json")) else { continue };
                let Some(chat) = std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()) else { continue };
                let body = serde_json::to_vec(&ChatFile { closed, chat })?;
                put(st, dir, &format!("chats/{}.enc", safe_name(id)), &body, key)?;
            }
        }
    }
    Ok(())
}

/// Other machines' tabs, passwords and chats.
fn read_theirs(dir: &Path, me: &DeviceTabs, opt: &Options, st: &mut State, changed: Option<&[String]>, out: &mut Outcome) {
    let mut lists = Vec::new();
    for e in std::fs::read_dir(dir.join("devices")).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let own = name.starts_with(&format!("{}.", me.id)) || opt.old_names.iter().any(|o| name == format!("{}.json", safe_name(o)));
        if own {
            continue;
        }
        let bytes = std::fs::read(e.path()).unwrap_or_default();
        if name.ends_with(".passwords.enc") {
            if let (Some(k), false) = (&opt.key, opt.vaults.is_empty()) {
                if let Some(l) = crate::crypt::open(k, &bytes).ok().and_then(|b| serde_json::from_slice::<Vec<crate::passwords::Synced>>(&b).ok()) {
                    lists.push(l);
                }
            }
        } else if name.ends_with(".json.enc") {
            if let Some(d) = opt.key.as_ref().and_then(|k| crate::crypt::open(k, &bytes).ok()).and_then(|b| serde_json::from_slice::<DeviceTabs>(&b).ok()) {
                out.devices.push(d);
            }
        } else if name.ends_with(".json") {
            // From a machine still on a Valerie that doesn't encrypt.
            if let Ok(d) = serde_json::from_slice::<DeviceTabs>(&bytes) {
                out.devices.push(d);
            }
        }
    }
    out.devices.sort_by(|a, b| b.updated.cmp(&a.updated));
    if !lists.is_empty() {
        out.passwords_changed = crate::passwords::import(&crate::passwords::newest(lists), &opt.vaults);
    }
    let (Some(key), Some(chats)) = (&opt.key, &opt.chats) else { return };
    let salt = crate::crypt::read_key_file(dir).map(|k| k.salt).unwrap_or_default();
    let all = st.chats_read != salt;
    for e in std::fs::read_dir(dir.join("chats")).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".enc") else { continue };
        if !all && !changed.is_some_and(|c| c.iter().any(|p| p == &format!("chats/{name}"))) {
            continue;
        }
        let Some(f) = std::fs::read(e.path()).ok().and_then(|b| crate::crypt::open(key, &b).ok()).and_then(|b| serde_json::from_slice::<ChatFile>(&b).ok()) else { continue };
        let updated = |v: &serde_json::Value| v["updated"].as_u64().unwrap_or(0);
        let (open_p, closed_p) = (chats.join(format!("{id}.json")), chats.join("closed").join(format!("{id}.json")));
        let local = [&open_p, &closed_p].into_iter().find_map(|p| std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()));
        if local.as_ref().is_some_and(|l| updated(l) >= updated(&f.chat)) {
            continue;
        }
        let (to, other) = if f.closed { (&closed_p, &open_p) } else { (&open_p, &closed_p) };
        if std::fs::create_dir_all(to.parent().unwrap_or(chats)).is_ok() && std::fs::write(to, serde_json::to_vec_pretty(&f.chat).unwrap_or_default()).is_ok() {
            let _ = std::fs::remove_file(other);
            // Already in the repo as it is: don't write it straight back.
            if let Ok(body) = serde_json::to_vec(&f) {
                st.written.insert(format!("chats/{name}"), digest(&body));
            }
            out.chats_changed = true;
        }
    }
    st.chats_read = salt;
}

/// Turn the config folder into a checkout of the repo. When the repo already has settings (set up on another
/// machine), those win; this machine's old config.yaml is kept as config.before-sync.yaml. Returns whether it just
/// checked the repo out.
fn ensure_repo(g: &Git, remote: &str, opt: &Options, out: &mut Outcome) -> Result<bool> {
    let dir = g.dir;
    std::fs::create_dir_all(dir)?;
    if dir.join(".git").exists() {
        let current = g.run(&["remote", "get-url", "origin"]).unwrap_or_default();
        if current.trim() != remote {
            if current.trim().is_empty() {
                g.run(&["remote", "add", "origin", remote])?;
            } else {
                g.run(&["remote", "set-url", "origin", remote])?;
            }
        }
        return Ok(false);
    }
    g.run(&["init", "-q", "-b", "main"])?;
    g.run(&["remote", "add", "origin", remote])?;
    let heads = g.run(&["ls-remote", "--heads", "origin"])?;
    let branches: Vec<&str> = heads.lines().filter_map(|l| l.split("refs/heads/").nth(1)).map(str::trim).collect();
    let Some(branch) = branches.iter().find(|b| **b == "main").or(branches.first()).map(|b| b.to_string()) else { return Ok(false) };
    g.run(&["fetch", "-q", "origin", &branch])?;
    let rev = format!("origin/{branch}");
    // Everything in the repo came from another machine: hold what can act before it's on disk.
    if opt.hold {
        let (p, c) = arrivals(g, None, &rev);
        (out.held_plugins, out.held_programs) = crate::trust::hold_arrivals(&p, &c);
    }
    let cfg = dir.join("config.yaml");
    if cfg.exists() && g.ok(&["cat-file", "-e", &format!("{rev}:config.yaml")]) {
        std::fs::copy(&cfg, dir.join("config.before-sync.yaml"))?;
    }
    g.run(&["checkout", "-q", "-f", "-B", "main", &rev])?;
    Ok(true)
}

/// Sync the config folder with the repo: write this machine's files, commit, take in the other machines' changes
/// (this machine wins a clash on the same line), push, then read what the others wrote.
pub fn run(dir: &Path, me: &DeviceTabs, opt: &Options) -> Result<Outcome> {
    let g = Git { dir, token: opt.token.as_deref(), ssh: opt.ssh.as_deref() };
    let mut out = Outcome::default();
    let mut st: State = std::fs::read_to_string(state_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let fresh = ensure_repo(&g, &opt.remote, opt, &mut out)?;
    if fresh {
        st = State::default();
    }
    ignore_files(&g)?;
    let commit = |g: &Git| -> Result<()> {
        g.run(&["add", "-A"])?;
        if !g.ok(&["diff", "--cached", "--quiet"]) {
            g.run(&["commit", "-q", "-m", &format!("Sync from {}", me.device)])?;
        }
        Ok(())
    };
    write_mine(dir, me, opt, &mut st)?;
    commit(&g)?;
    let mut changed: Option<Vec<String>> = if fresh { None } else { Some(Vec::new()) };
    if g.ok(&["fetch", "-q", "origin"]) && g.ok(&["rev-parse", "--verify", "-q", "origin/main"]) {
        let has_head = g.ok(&["rev-parse", "--verify", "-q", "HEAD"]);
        if has_head && !g.ok(&["merge-base", "HEAD", "origin/main"]) {
            // Another machine started the history over (after turning on encryption): take its copy, keep this
            // machine's settings file aside, and write this machine's files again on top.
            let cfg = dir.join("config.yaml");
            if cfg.exists() {
                let _ = std::fs::copy(&cfg, dir.join("config.before-sync.yaml"));
            }
            if opt.hold {
                let (p, c) = arrivals(&g, None, "origin/main");
                let (a, b) = crate::trust::hold_arrivals(&p, &c);
                out.held_plugins.extend(a);
                out.held_programs.extend(b);
            }
            g.run(&["reset", "-q", "--hard", "origin/main"])?;
            st.written.clear();
            st.passwords.clear();
            changed = None;
            write_mine(dir, me, opt, &mut st)?;
            commit(&g)?;
        } else if has_head {
            let theirs: Vec<String> = g.run(&["diff", "--name-only", "HEAD...origin/main"]).unwrap_or_default().lines().map(str::to_string).collect();
            if opt.hold {
                let (p, c) = arrivals(&g, Some("HEAD"), "origin/main");
                let (a, b) = crate::trust::hold_arrivals(&p, &c);
                out.held_plugins.extend(a);
                out.held_programs.extend(b);
            }
            if let Some(c) = changed.as_mut() {
                *c = theirs;
            }
            // In a rebase "theirs" is the commit being replayed: this machine's change wins a same-line clash.
            if let Err(e) = g.run(&["rebase", "-q", "-X", "theirs", "--autostash", "origin/main"]) {
                let _ = g.run(&["rebase", "--abort"]);
                bail!("couldn't merge the other machines' changes: {e}");
            }
        }
    }
    if g.ok(&["rev-parse", "--verify", "-q", "HEAD"]) {
        if opt.fresh_history {
            // One commit with everything as it is now; the old (readable) history is gone from the repo.
            g.run(&["checkout", "-q", "--orphan", "valerie-fresh"])?;
            g.run(&["commit", "-q", "--allow-empty", "-m", "Valerie sync (encrypted from here on)"])?;
            g.run(&["branch", "-q", "-M", "main"])?;
            g.run(&["push", "-q", "-f", "-u", "origin", "HEAD:main"])?;
        } else {
            let ahead = g.run(&["rev-list", "--count", "origin/main..HEAD"]).map(|n| n.trim() != "0").unwrap_or(true);
            if ahead {
                g.run(&["push", "-q", "-u", "origin", "HEAD:main"])?;
            }
        }
    }
    out.has_passphrase = crate::crypt::read_key_file(dir).is_some();
    out.locked = opt.encrypt && out.has_passphrase && opt.key.is_none();
    read_theirs(dir, me, opt, &mut st, changed.as_deref(), &mut out);
    if let Ok(s) = serde_json::to_string(&st) {
        let _ = std::fs::write(state_path(), s);
    }
    Ok(out)
}

/// Set the sync passphrase (the first machine) or enter it (every other one). Returns the key and whether this
/// machine just created it (then the next sync starts the history over, so nothing readable stays in the repo).
pub fn set_passphrase(dir: &Path, passphrase: &str) -> Result<(crate::crypt::Key, bool)> {
    if let Some(kf) = crate::crypt::read_key_file(dir) {
        let key = crate::crypt::unlock(&kf, passphrase)?;
        crate::crypt::store_key(&kf, &key)?;
        return Ok((key, false));
    }
    let (kf, key) = crate::crypt::new_key_file(passphrase)?;
    std::fs::write(dir.join(crate::crypt::KEY_FILE), serde_json::to_string_pretty(&kf)?)?;
    crate::crypt::store_key(&kf, &key)?;
    Ok((key, true))
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
    let step = crate::github::set_up(&config.sync.github_client_id, &config.sync.repo_name, config.sync.repo_key, &config.device_name(), &|s| {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) -> String {
        let o = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    fn tabs(id: &str, device: &str, url: &str) -> DeviceTabs {
        let tab = SyncedTab { title: "A page".into(), url: url.into() };
        DeviceTabs {
            device: device.into(),
            id: id.into(),
            updated: crate::state::now(),
            spaces: vec![SyncedSpace { name: "Personal".into(), pinned: vec![], tabs: vec![tab], tree: vec![], kept: vec![], modified: 0 }],
            favorites: vec![],
            favorites_modified: 0,
        }
    }

    /// Two machines through a real git remote: the first sets a passphrase (and the history starts over), the second
    /// can't read anything until it has it, a plugin that runs programs waits on the second until allowed, and chats
    /// and tabs come across encrypted.
    #[test]
    fn two_machines_sync_encrypted() {
        let _g = crate::config::TEST_DIRS.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("valerie-sync-test-{}", crate::crypt::hex(&crate::crypt::random::<4>())));
        let bare = root.join("remote.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare", "-b", "main"]);
        let remote = format!("file://{}", bare.display());
        let (a, b) = (root.join("a"), root.join("b"));
        let on = |m: &Path| {
            std::env::set_var("VALERIE_CONFIG_DIR", m.join("config"));
            std::env::set_var("VALERIE_DATA_DIR", m.join("data"));
        };
        let opt = |m: &Path, key: Option<crate::crypt::Key>, fresh: bool| Options {
            remote: remote.clone(),
            encrypt: true,
            key,
            chats: Some(m.join("data").join("chats")),
            hold: true,
            fresh_history: fresh,
            ..Default::default()
        };
        let (me_a, me_b) = (tabs("aaaa", "Mac A", "https://secret-a.example/reset?token=1"), tabs("bbbb", "Mac B", "https://b.example"));

        // Machine A: settings sync first, then it chooses a passphrase.
        on(&a);
        std::fs::create_dir_all(a.join("config")).unwrap();
        std::fs::write(a.join("config/config.yaml"), "search_engine: x\n").unwrap();
        std::fs::create_dir_all(a.join("data/chats")).unwrap();
        std::fs::write(a.join("data/chats/c1.json"), r#"{"id":"c1","title":"Packing list","updated":5,"messages":[{"role":"user","text":"my bank password hint"}]}"#).unwrap();
        run(&a.join("config"), &me_a, &opt(&a, None, false)).unwrap();
        let (key, created) = set_passphrase(&a.join("config"), "correct horse battery staple").unwrap();
        assert!(created);
        run(&a.join("config"), &me_a, &opt(&a, Some(key), true)).unwrap();
        assert_eq!(git(&bare, &["rev-list", "--count", "main"]).trim(), "1", "the history started over");
        let files = git(&bare, &["ls-tree", "-r", "--name-only", "main"]);
        for f in ["devices/aaaa.json.enc", "chats/c1.enc", "sync-key.json", "config.yaml"] {
            assert!(files.lines().any(|l| l == f), "{f} missing from {files}");
        }
        assert!(!files.contains("devices/aaaa.json\n") && !files.contains("chats/c1.json"));
        let everything = git(&bare, &["log", "-p", "--all", "--text"]);
        assert!(!everything.contains("secret-a.example") && !everything.contains("bank password"), "something readable reached the repo");

        // A adds a plugin that runs a program, and one that only restyles pages.
        std::fs::create_dir_all(a.join("config/plugins/evil")).unwrap();
        std::fs::write(a.join("config/plugins/evil/plugin.yaml"), "commands:\n  - title: Go\n    run: echo pwned\n").unwrap();
        std::fs::create_dir_all(a.join("config/plugins/nice")).unwrap();
        std::fs::write(a.join("config/plugins/nice/plugin.yaml"), "description: nicer pages\n").unwrap();
        std::fs::write(a.join("config/plugins/nice/style.css"), "body { color: red }").unwrap();
        run(&a.join("config"), &me_a, &opt(&a, Some(key), false)).unwrap();

        // Machine B joins: it has the settings, but can't read tabs or chats without the passphrase.
        on(&b);
        let out = run(&b.join("config"), &me_b, &opt(&b, None, false)).unwrap();
        assert!(out.locked && out.has_passphrase);
        assert!(out.devices.is_empty());
        assert_eq!(out.held_plugins, vec!["evil".to_string()]);
        assert!(std::fs::read_to_string(b.join("config/config.yaml")).unwrap().contains("search_engine: x"));
        assert!(set_passphrase(&b.join("config"), "not the passphrase at all").is_err());
        let (kb, created) = set_passphrase(&b.join("config"), "correct horse battery staple").unwrap();
        assert!(!created && kb == key);
        let out = run(&b.join("config"), &me_b, &opt(&b, Some(kb), false)).unwrap();
        assert!(out.devices.iter().any(|d| d.device == "Mac A" && d.spaces[0].tabs[0].url.contains("secret-a")));
        assert!(out.chats_changed);
        assert!(std::fs::read_to_string(b.join("data/chats/c1.json")).unwrap().contains("Packing list"));
        let names: Vec<String> = crate::plugins::load_all().0.into_iter().map(|p| p.name).collect();
        assert!(names.contains(&"nice".to_string()) && !names.contains(&"evil".to_string()), "{names:?}");
        assert!(crate::trust::allow_plugin("evil"));
        assert!(crate::plugins::load_all().0.iter().any(|p| p.name == "evil"));

        // A sees B's tabs; an idle sync makes no new commit.
        on(&a);
        let out = run(&a.join("config"), &me_a, &opt(&a, Some(key), false)).unwrap();
        assert!(out.devices.iter().any(|d| d.device == "Mac B"));
        let before = git(&bare, &["rev-list", "--count", "main"]);
        run(&a.join("config"), &me_a, &opt(&a, Some(key), false)).unwrap();
        assert_eq!(git(&bare, &["rev-list", "--count", "main"]), before, "an idle sync committed");

        std::env::remove_var("VALERIE_CONFIG_DIR");
        std::env::remove_var("VALERIE_DATA_DIR");
        let _ = std::fs::remove_dir_all(&root);
    }
}
