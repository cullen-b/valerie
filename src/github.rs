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

/// The whole "Set Up Sync": sign in, then find or create the repo.
pub fn set_up(client_id: &str, repo_name: &str, notify: &dyn Fn(Step)) -> Step {
    match sign_in(client_id, notify).and_then(|t| ensure_repo(&t, repo_name)) {
        Ok((login, repo)) => Step::Done { login, repo },
        Err(e) => Step::Failed(format!("{e:#}")),
    }
}
