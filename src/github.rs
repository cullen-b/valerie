//! GitHub sign-in and the private sync repo. Uses the GitHub CLI's sign-in when it's installed, or GitHub's device
//! flow when `sync.github_client_id` is set. Talks to the API with curl, so there's no HTTP library to ship.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

const KEYCHAIN: &str = "Valerie GitHub";

/// Progress of "Set Up Sync", shown to the user as it happens.
#[derive(Debug, Clone)]
pub enum Step {
    /// Enter this code on GitHub's device page to finish signing in.
    Code { code: String, url: String },
    Done { login: String, repo: String },
    Failed(String),
}

fn gh(args: &[&str]) -> Option<String> {
    let o = Command::new("gh").args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

fn token_file() -> std::path::PathBuf {
    crate::config::data_dir().join("github-token")
}

/// Keep Valerie's own GitHub token in the Keychain (a private file where there's no Keychain support yet).
fn store_token(t: &str) -> Result<()> {
    if crate::passwords::keychain::save(KEYCHAIN, "token", "Valerie GitHub sync", t).is_ok() {
        return Ok(());
    }
    let path = token_file();
    std::fs::create_dir_all(path.parent().context("data dir")?)?;
    std::fs::write(&path, t)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// A token for GitHub: one Valerie stored after its own sign-in, else the GitHub CLI's.
pub fn token() -> Option<String> {
    crate::passwords::keychain::get(KEYCHAIN, "token")
        .ok()
        .flatten()
        .or_else(|| std::fs::read_to_string(token_file()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
        .or_else(|| gh(&["auth", "token"]))
}

/// Call the GitHub API. The token goes to curl on stdin, so it never shows up in the process list.
pub fn api(token: &str, method: &str, path: &str, body: Option<&Value>) -> Result<(u16, Value)> {
    let mut c = Command::new("curl");
    c.args(["-sS", "-X", method, "-H", "@-", "-H", "Accept: application/vnd.github+json", "-H", "X-GitHub-Api-Version: 2022-11-28"]);
    c.args(["-w", "\n%{http_code}"]);
    if let Some(b) = body {
        c.args(["-H", "Content-Type: application/json", "-d", &b.to_string()]);
    }
    c.arg(format!("https://api.github.com{path}"));
    let mut child = c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().context("curl is needed for sync")?;
    child.stdin.take().context("curl stdin")?.write_all(format!("Authorization: Bearer {token}\n").as_bytes())?;
    let out = child.wait_with_output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    Ok((code.trim().parse().unwrap_or(0), serde_json::from_str(body).unwrap_or(Value::Null)))
}

fn post_form(url: &str, form: &[(&str, &str)]) -> Result<Value> {
    let mut c = Command::new("curl");
    c.args(["-sS", "-X", "POST", "-H", "Accept: application/json"]);
    for (k, v) in form {
        c.args(["--data-urlencode", &format!("{k}={v}")]);
    }
    let out = c.arg(url).output().context("curl is needed for sync")?;
    Ok(serde_json::from_slice(&out.stdout).unwrap_or(Value::Null))
}

/// GitHub's device flow: show a code, wait while the user approves Valerie in the browser.
fn device_flow(client_id: &str, notify: &dyn Fn(Step)) -> Result<String> {
    let start = post_form("https://github.com/login/device/code", &[("client_id", client_id), ("scope", "repo")])?;
    let (Some(device), Some(code)) = (start["device_code"].as_str(), start["user_code"].as_str()) else {
        bail!("GitHub didn't start the sign-in: {}", start["error_description"].as_str().unwrap_or("check sync.github_client_id"));
    };
    notify(Step::Code { code: code.to_string(), url: start["verification_uri"].as_str().unwrap_or("https://github.com/login/device").to_string() });
    let mut wait = start["interval"].as_u64().unwrap_or(5);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(start["expires_in"].as_u64().unwrap_or(900));
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_secs(wait));
        let r = post_form(
            "https://github.com/login/oauth/access_token",
            &[("client_id", client_id), ("device_code", device), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code")],
        )?;
        if let Some(t) = r["access_token"].as_str() {
            store_token(t)?;
            return Ok(t.to_string());
        }
        match r["error"].as_str() {
            Some("authorization_pending") => {}
            Some("slow_down") => wait += 5,
            Some(e) => bail!("GitHub sign-in failed: {e}"),
            None => bail!("GitHub sign-in failed"),
        }
    }
    bail!("the GitHub code expired; try again")
}

/// `gh auth login` in the background: pass its one-time code to the user, wait for them to approve it.
fn gh_login(notify: &dyn Fn(Step)) -> Result<String> {
    let mut child = Command::new("gh")
        .args(["auth", "login", "--web", "--hostname", "github.com", "--git-protocol", "https", "--scopes", "repo"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"\n");
    }
    if let Some(err) = child.stderr.take() {
        for line in BufReader::new(err).lines().map_while(Result::ok) {
            if let Some(code) = line.split("one-time code:").nth(1) {
                notify(Step::Code { code: code.trim().to_string(), url: "https://github.com/login/device".into() });
            }
        }
    }
    if !child.wait()?.success() {
        bail!("GitHub CLI sign-in didn't finish");
    }
    gh(&["auth", "token"]).context("the GitHub CLI signed in but gave no token")
}

/// Sign in to GitHub by whatever means is available, without asking for anything but approval in the browser.
pub fn sign_in(client_id: &str, notify: &dyn Fn(Step)) -> Result<String> {
    if let Some(t) = token().filter(|t| matches!(api(t, "GET", "/user", None), Ok((200, _)))) {
        return Ok(t);
    }
    if !client_id.is_empty() {
        return device_flow(client_id, notify);
    }
    if gh(&["--version"]).is_some() {
        return gh_login(notify);
    }
    bail!("install the GitHub CLI (brew install gh, or https://cli.github.com) or set sync.github_client_id, then try again")
}

/// Find or create the user's private sync repo. Returns (login, clone URL).
pub fn ensure_repo(token: &str, name: &str) -> Result<(String, String)> {
    let (code, user) = api(token, "GET", "/user", None)?;
    let Some(login) = user["login"].as_str().filter(|_| code == 200) else { bail!("GitHub didn't accept the sign-in (HTTP {code})") };
    let (code, _) = api(token, "GET", &format!("/repos/{login}/{name}"), None)?;
    if code == 404 {
        let body = json!({ "name": name, "private": true, "description": "Valerie browser: settings, plugins and tabs, synced between machines" });
        let (code, r) = api(token, "POST", "/user/repos", Some(&body))?;
        if code != 201 {
            bail!("couldn't create the {name} repo: {}", r["message"].as_str().unwrap_or("unknown error"));
        }
    } else if code != 200 {
        bail!("couldn't look up the {name} repo (HTTP {code})");
    }
    Ok((login.to_string(), format!("https://github.com/{login}/{name}.git")))
}

/// The whole "Set Up Sync": sign in, find or create the repo, and (with `repo_key`) give this machine its own key
/// for that repo alone, after which Valerie keeps no GitHub sign-in of its own.
pub fn set_up(client_id: &str, repo_name: &str, repo_key: bool, device: &str, notify: &dyn Fn(Step)) -> Step {
    let done = sign_in(client_id, notify).and_then(|t| {
        let (login, repo) = ensure_repo(&t, repo_name)?;
        // Without ssh-keygen (or if GitHub says no) sync carries on with the sign-in, as before.
        if repo_key && add_repo_key(&t, &repo, device).is_ok() {
            forget_token();
        }
        Ok((login, repo))
    });
    match done {
        Ok((login, repo)) => Step::Done { login, repo },
        Err(e) => Step::Failed(format!("{e:#}")),
    }
}

// ---------------------------------------------------------------- this machine's key for the sync repo
// A GitHub deploy key: an SSH key that can read and write one repo and nothing else, one per machine (each shows in
// the repo's Settings › Deploy keys, so a lost machine can be cut off there).

pub fn repo_key_path() -> std::path::PathBuf {
    crate::config::data_dir().join("sync-repo-key")
}

/// GitHub's published SSH host keys (api.github.com/meta), pinned so sync never talks to an impostor.
const GITHUB_HOST_KEYS: &[&str] = &[
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl",
    "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBEmKSENjQEezOmxkZMy7opKgwFB9nkt5YRrYMjNuG5N87uRgg6CLrbo5wAdT/y6v0mKV0U2w0WZ2YB/++Tpockg=",
    "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQCj7ndNxQowgcQnjshcLrqPEiiphnt+VTTvDP6mHBL9j1aNUkY4Ue1gvwnGLVlOhGeYrnZaMgRK6+PKCUXaDbC7qtbW8gIkhL7aGCsOr/C56SJMy/BCZfxd1nWzAOxSDPgVsmerOBYfNqltV9/hWCqBywINIR+5dIg6JTJ72pcEpEjcYgXkE2YEFXV1JHnsKgbLWNlhScqb2UmyRkQyytRLtL+38TGxkxCflmO+5Z8CSSNY7GidjMIZ7Q4zMjA2n1nGrlTDkzwDCsw+wqFPGQA179cnfGWOWRVruj16z6XyvxvjJwbz0wQZ75XK5tKSb7FNyeIEs4TT4jk+S4dhPeAUC5y+bDYirYgM4GC7uEnztnZyaVWQ7B381AK4Qdrwt51ZqExKbQpTUNn+EjqoTwvqNj4kqx5QUCI0ThS/YkOxJCXmPUWZbhjpCg56i+2aB6CmK2JGhn57K5mj0MNdBXA4/WnwH6XoPWJzK5Nyu2zB3nAZp+S5hpQs+p1vN1/wsjk=",
];

/// "https://github.com/me/valerie-sync.git" → ("me", "valerie-sync").
pub fn owner_repo(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("https://github.com/")?.trim_end_matches('/').trim_end_matches(".git");
    let (o, r) = rest.split_once('/')?;
    (!o.is_empty() && !r.is_empty() && !r.contains('/')).then(|| (o.to_string(), r.to_string()))
}

/// The address git uses with this machine's repo key: SSH over port 443, which works on networks that block 22.
pub fn ssh_url(url: &str) -> Option<String> {
    owner_repo(url).map(|(o, r)| format!("ssh://git@ssh.github.com:443/{o}/{r}.git"))
}

/// How git should run ssh for the sync repo: only this key, only GitHub's real host keys, never a prompt.
pub fn ssh_command() -> Option<String> {
    let key = repo_key_path();
    if !key.exists() {
        return None;
    }
    let hosts = crate::config::data_dir().join("github-host-keys");
    let lines: String = GITHUB_HOST_KEYS.iter().map(|k| format!("[ssh.github.com]:443 {k}\ngithub.com {k}\n")).collect();
    if std::fs::read_to_string(&hosts).ok().as_deref() != Some(lines.as_str()) {
        std::fs::write(&hosts, &lines).ok()?;
    }
    let q = |p: &std::path::Path| format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"));
    Some(format!(
        "ssh -i {} -o IdentitiesOnly=yes -o IdentityAgent=none -o UserKnownHostsFile={} -o StrictHostKeyChecking=yes -o BatchMode=yes",
        q(&key),
        q(&hosts)
    ))
}

/// Make this machine's key (once) and add it to the repo with write access.
pub fn add_repo_key(token: &str, repo: &str, device: &str) -> Result<()> {
    let (owner, name) = owner_repo(repo).context("the sync repo isn't on github.com")?;
    let key = repo_key_path();
    if !key.exists() {
        std::fs::create_dir_all(key.parent().context("data dir")?)?;
        let out = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", &format!("Valerie on {device}"), "-f"])
            .arg(&key)
            .output()
            .context("ssh-keygen isn't installed")?;
        if !out.status.success() {
            bail!("couldn't make a key: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
    }
    let public = std::fs::read_to_string(key.with_extension("pub")).context("the key's public half is missing")?;
    let public = public.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
    let (code, list) = api(token, "GET", &format!("/repos/{owner}/{name}/keys"), None)?;
    if code == 200 && list.as_array().into_iter().flatten().any(|k| k["key"].as_str() == Some(public.as_str()) && k["read_only"] == false) {
        return Ok(());
    }
    let body = json!({ "title": format!("Valerie on {device}"), "key": public, "read_only": false });
    let (code, r) = api(token, "POST", &format!("/repos/{owner}/{name}/keys"), Some(&body))?;
    if code != 201 {
        bail!("couldn't give this machine a key for {name}: {}", r["message"].as_str().unwrap_or("unknown error"));
    }
    Ok(())
}

/// Drop the GitHub sign-in Valerie stored itself (the GitHub CLI keeps its own, which is the CLI's business).
pub fn forget_token() {
    let _ = crate::passwords::keychain::delete(KEYCHAIN, "token");
    let _ = std::fs::remove_file(token_file());
}


#[cfg(test)]
mod tests {
    /// Real GitHub: a repo key reaches the sync repo (VALERIE_TEST_SYNC_REPO) and can't push anywhere else; the key
    /// is removed afterwards. `VALERIE_TEST_SYNC_REPO=https://github.com/you/valerie-sync.git cargo test -- --ignored repo_key`
    #[test]
    #[ignore]
    fn repo_key_reaches_only_the_sync_repo() {
        let _g = crate::config::TEST_DIRS.lock().unwrap_or_else(|e| e.into_inner());
        let data = std::env::temp_dir().join(format!("valerie-key-test-{}", crate::crypt::hex(&crate::crypt::random::<4>())));
        std::env::set_var("VALERIE_DATA_DIR", &data);
        let repo = std::env::var("VALERIE_TEST_SYNC_REPO").expect("VALERIE_TEST_SYNC_REPO");
        let token = super::gh(&["auth", "token"]).expect("gh auth token");
        super::add_repo_key(&token, &repo, "key test").unwrap();
        let ssh = super::ssh_command().unwrap();
        let git = |args: &[&str]| std::process::Command::new("git").env("GIT_SSH_COMMAND", &ssh).env("GIT_TERMINAL_PROMPT", "0").args(args).output().unwrap();
        let url = super::ssh_url(&repo).unwrap();
        let reach = git(&["ls-remote", "--heads", &url]);
        let work = data.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let w = work.to_string_lossy().to_string();
        let local = |args: &[&str]| git(&[&["-C", &w][..], args].concat());
        local(&["init", "-q"]);
        local(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "key test"]);
        let push = local(&["push", "-q", &url, "HEAD:refs/heads/valerie-key-test"]);
        let delete = local(&["push", "-q", &url, ":refs/heads/valerie-key-test"]);
        let elsewhere = local(&["push", "-q", "--dry-run", "ssh://git@ssh.github.com:443/cullen-b/valerie.git", "HEAD:refs/heads/valerie-key-test"]);
        // Remove the test key from the repo whatever happened.
        let public = std::fs::read_to_string(super::repo_key_path().with_extension("pub")).unwrap();
        let public = public.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
        let (o, r) = super::owner_repo(&repo).unwrap();
        let (_, keys) = super::api(&token, "GET", &format!("/repos/{o}/{r}/keys"), None).unwrap();
        for k in keys.as_array().into_iter().flatten().filter(|k| k["key"].as_str() == Some(public.as_str())) {
            let (code, _) = super::api(&token, "DELETE", &format!("/repos/{o}/{r}/keys/{}", k["id"]), None).unwrap();
            eprintln!("removed the test key: HTTP {code}");
        }
        std::env::remove_var("VALERIE_DATA_DIR");
        let _ = std::fs::remove_dir_all(&data);
        assert!(reach.status.success(), "couldn't reach the sync repo: {}", String::from_utf8_lossy(&reach.stderr));
        assert!(push.status.success() && delete.status.success(), "couldn't push: {}{}", String::from_utf8_lossy(&push.stderr), String::from_utf8_lossy(&delete.stderr));
        assert!(!elsewhere.status.success(), "the repo key could push to another repo");
        eprintln!("refused elsewhere: {}", String::from_utf8_lossy(&elsewhere.stderr).trim());
    }
}
