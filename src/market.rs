//! The plugin marketplace: a public repo of plugins people shared (github.com/cullen-b/valerie-plugins by default).
//! Its CI publishes `index.json` (every plugin, its files and their checksums, pinned to a commit). Valerie installs
//! from that list, and shares a plugin back by opening a pull request with the user's GitHub sign-in.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Written into a plugin folder installed from the marketplace (which files, so updates can be spotted).
pub const STAMP: &str = ".marketplace.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub name: String,
    pub description: String,
    pub author: String,
    /// Sites it works on; empty = every site.
    pub matches: Vec<String>,
    /// Titles of the commands it adds to the command bar.
    pub commands: Vec<String>,
    /// The shell commands it can run on the user's computer (command `run` and `on_page_load`), shown before installing.
    pub programs: Vec<String>,
    pub files: Vec<File>,
    pub updated: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct File {
    pub path: String,
    pub url: String,
    pub sha256: String,
}

impl Entry {
    /// A fingerprint of its files, to tell whether an installed copy is the current one.
    pub fn version(&self) -> String {
        self.files.iter().map(|f| f.sha256.get(..8).unwrap_or("")).collect::<Vec<_>>().join("")
    }
}

pub fn fetch(index: &str) -> Result<Vec<Entry>> {
    let out = Command::new("curl").args(["-fsSL", "--max-time", "20", "--proto", "=https,file", index]).output().context("couldn't run curl")?;
    if !out.status.success() {
        bail!("couldn't reach the plugin marketplace: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let v: Value = serde_json::from_slice(&out.stdout).context("the marketplace list isn't valid")?;
    Ok(serde_json::from_value(v.get("plugins").cloned().unwrap_or(v))?)
}

/// Plugins whose name or description share words with `query`, best first.
pub fn search<'a>(entries: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    let words: Vec<String> = query.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 2).map(str::to_string).collect();
    let mut scored: Vec<(usize, &Entry)> = entries
        .iter()
        .map(|e| {
            let hay = format!("{} {} {} {}", e.name.replace('-', " "), e.description, e.matches.join(" "), e.commands.join(" ")).to_lowercase();
            (words.iter().filter(|w| hay.contains(w.as_str())).count(), e)
        })
        .filter(|(n, _)| *n > 0 || words.is_empty())
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored.into_iter().map(|(_, e)| e).collect()
}

fn safe_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 60 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !name.starts_with('-')
}

fn safe_path(p: &str) -> bool {
    !p.is_empty() && !p.starts_with('/') && !p.split('/').any(|s| s.is_empty() || s == "." || s == ".." || s.starts_with('.'))
}

fn sha256(path: &Path) -> Result<String> {
    let out = Command::new("shasum").args(["-a", "256"]).arg(path).output().context("couldn't run shasum")?;
    Ok(String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap_or("").to_string())
}

/// Download a plugin into `plugins_dir/<name>`, checking every file's checksum; replaces an older copy.
pub fn install(e: &Entry, plugins_dir: &Path) -> Result<PathBuf> {
    if !safe_name(&e.name) {
        bail!("{:?} isn't a valid plugin name", e.name);
    }
    if !e.files.iter().any(|f| f.path == "plugin.yaml") {
        bail!("{} has no plugin.yaml", e.name);
    }
    let dest = plugins_dir.join(&e.name);
    let tmp = plugins_dir.join(format!(".{}.installing", e.name));
    let _ = std::fs::remove_dir_all(&tmp);
    let result = (|| {
        for f in &e.files {
            if !safe_path(&f.path) || !f.url.starts_with("https://") {
                bail!("{} lists a file Valerie won't download ({})", e.name, f.path);
            }
            let to = tmp.join(&f.path);
            std::fs::create_dir_all(to.parent().context("no folder")?)?;
            let ok = Command::new("curl").args(["-fsSL", "--max-time", "60", "--proto", "=https", "-o"]).arg(&to).arg(&f.url).status()?.success();
            if !ok {
                bail!("couldn't download {}", f.path);
            }
            if !sha256(&to)?.eq_ignore_ascii_case(&f.sha256) {
                bail!("{} changed since the marketplace listed it; try again in a minute", f.path);
            }
        }
        std::fs::write(tmp.join(STAMP), serde_json::to_string_pretty(&json!({ "name": e.name, "version": e.version(), "author": e.author }))?)?;
        crate::plugins::check_dir(&tmp).with_context(|| format!("{} doesn't load", e.name))?;
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&tmp, &dest)?;
        Ok(dest.clone())
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// The marketplace version of an installed plugin, if it came from there.
pub fn installed_version(dir: &Path) -> Option<String> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(STAMP)).ok()?).ok()?;
    v["version"].as_str().map(str::to_string)
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// The files of a plugin folder that get shared: text files, nothing hidden, nothing huge.
fn shareable(dir: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d)?.flatten() {
            let p = entry.path();
            let rel = p.strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
            if !safe_path(&rel) {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let data = std::fs::read(&p)?;
            if data.len() > 200_000 || std::str::from_utf8(&data).is_err() {
                bail!("{rel} is too big or isn't text; the marketplace takes text files under 200 KB");
            }
            out.push((rel, data));
        }
    }
    if out.len() > 30 {
        bail!("that's a lot of files ({}); the marketplace takes up to 30 per plugin", out.len());
    }
    out.sort();
    Ok(out)
}

/// Share the plugin in `dir` as a pull request to `repo` ("owner/name"). Returns the pull request's address.
/// A plugin of the same name by someone else is refused (rename yours); your own is updated.
pub fn share(dir: &Path, repo: &str, index: &[Entry], token: &str) -> Result<String> {
    use crate::github::api;
    let name = dir.file_name().and_then(|n| n.to_str()).context("no plugin name")?.to_string();
    if !safe_name(&name) {
        bail!("rename the plugin's folder to lowercase words with hyphens (like {}) before sharing", name.to_lowercase().replace(|c: char| !c.is_ascii_alphanumeric(), "-"));
    }
    crate::plugins::check_dir(dir).context("fix the plugin first")?;
    if dir.join(STAMP).exists() || crate::plugins::is_bundled(&name) {
        bail!("{name} came with Valerie or from the marketplace; share your own plugins");
    }
    let files = shareable(dir)?;
    let (code, me) = api(token, "GET", "/user", None)?;
    let login = me["login"].as_str().filter(|_| code == 200).context("GitHub sign-in didn't work; sign in again")?.to_string();
    if let Some(e) = index.iter().find(|e| e.name == name).filter(|e| !e.author.eq_ignore_ascii_case(&login)) {
        bail!("a plugin called {name} by {} is already in the marketplace: rename yours, or install theirs", e.author);
    }
    let (owner, _) = repo.split_once('/').context("plugin_marketplace.repo should look like owner/name")?;
    // Your own repo: a branch there. Anyone else's: a branch in your fork of it.
    let target = if owner.eq_ignore_ascii_case(&login) {
        repo.to_string()
    } else {
        let (code, fork) = api(token, "POST", &format!("/repos/{repo}/forks"), Some(&json!({ "default_branch_only": true })))?;
        if !(200..300).contains(&code) {
            bail!("couldn't fork {repo}: {}", fork["message"].as_str().unwrap_or("GitHub said no"));
        }
        let full = fork["full_name"].as_str().context("no fork name")?.to_string();
        // A new fork takes a moment to be ready.
        for _ in 0..20 {
            if matches!(api(token, "GET", &format!("/repos/{full}/git/ref/heads/main"), None), Ok((200, _))) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        full
    };
    let (code, main) = api(token, "GET", &format!("/repos/{repo}/git/ref/heads/main"), None)?;
    let base = main["object"]["sha"].as_str().filter(|_| code == 200).context("couldn't read the marketplace's main branch")?.to_string();
    let branch = format!("share-{name}-{}", crate::state::now());
    let (code, r) = api(token, "POST", &format!("/repos/{target}/git/refs"), Some(&json!({ "ref": format!("refs/heads/{branch}"), "sha": base })))?;
    if !(200..300).contains(&code) {
        bail!("couldn't make a branch for it: {}", r["message"].as_str().unwrap_or("GitHub said no"));
    }
    for (rel, data) in &files {
        let path = format!("plugins/{name}/{rel}");
        // The marketplace credits (and lets update) whoever plugin.yaml names; fill that in if it's missing.
        let data = if rel == "plugin.yaml" && !String::from_utf8_lossy(data).lines().any(|l| l.starts_with("author:")) {
            format!("author: {login}\n{}", String::from_utf8_lossy(data)).into_bytes()
        } else {
            data.clone()
        };
        // Updating your own plugin: GitHub wants the current file's id.
        let existing = api(token, "GET", &format!("/repos/{target}/contents/{path}?ref={branch}"), None).ok().filter(|(c, _)| *c == 200).and_then(|(_, v)| v["sha"].as_str().map(str::to_string));
        let mut body = json!({ "message": format!("{name}: {rel}"), "content": b64(&data), "branch": branch });
        if let Some(sha) = existing {
            body["sha"] = json!(sha);
        }
        let (code, r) = api(token, "PUT", &format!("/repos/{target}/contents/{path}"), Some(&body))?;
        if !(200..300).contains(&code) {
            bail!("couldn't upload {rel}: {}", r["message"].as_str().unwrap_or("GitHub said no"));
        }
    }
    let manifest: Value = serde_yaml::from_str(&String::from_utf8_lossy(&files.iter().find(|(p, _)| p == "plugin.yaml").map(|(_, d)| d.clone()).unwrap_or_default())).unwrap_or(Value::Null);
    let updating = index.iter().any(|e| e.name == name);
    let head = if target == repo { branch.clone() } else { format!("{login}:{branch}") };
    let body = format!(
        "{}\n\n{}\n\nShared from Valerie by @{login}.",
        manifest["description"].as_str().unwrap_or("(no description)"),
        files.iter().map(|(p, _)| format!("- `{p}`")).collect::<Vec<_>>().join("\n")
    );
    let title = format!("{} plugin: {name}", if updating { "Update" } else { "Add" });
    let (code, pr) = api(token, "POST", &format!("/repos/{repo}/pulls"), Some(&json!({ "title": title, "head": head, "base": "main", "body": body })))?;
    pr["html_url"].as_str().filter(|_| (200..300).contains(&code)).map(str::to_string).with_context(|| format!("couldn't open the pull request: {}", pr["message"].as_str().unwrap_or("GitHub said no")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, description: &str) -> Entry {
        Entry { name: name.into(), description: description.into(), ..Default::default() }
    }

    #[test]
    fn searches_by_words() {
        let all = vec![entry("youtube-speed", "Set YouTube playback speed"), entry("rakuten-cashback", "Cash back on store pages"), entry("dark-docs", "Dark mode for Google Docs")];
        let hits: Vec<&str> = search(&all, "change youtube speed").iter().map(|e| e.name.as_str()).collect();
        assert_eq!(hits, ["youtube-speed"]);
        assert_eq!(search(&all, "").len(), 3);
        assert!(search(&all, "zzz nothing").is_empty());
    }

    #[test]
    fn encodes_base64() {
        assert_eq!(b64(b""), "");
        assert_eq!(b64(b"f"), "Zg==");
        assert_eq!(b64(b"fo"), "Zm8=");
        assert_eq!(b64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn refuses_unsafe_names_and_paths() {
        assert!(safe_name("youtube-speed") && !safe_name("../x") && !safe_name("Bad Name") && !safe_name(""));
        assert!(safe_path("script.js") && safe_path("lib/a.js"));
        assert!(!safe_path("../x") && !safe_path("/etc/passwd") && !safe_path(".hidden") && !safe_path("a//b"));
    }
}
