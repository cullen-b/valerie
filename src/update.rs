//! Updates. Every push to main builds a release (.github/workflows/release.yml) with a `latest.json` describing it.
//! Valerie reads that now and then, downloads a newer build, checks it's signed by Valerie's release key, and puts it
//! in place of the running app; the new version starts the next time Valerie opens.
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Releases are signed with this key (assets/release-signing.pem; the private half lives in CI's secrets and the
/// owner's keychain). An update is installed only if its signature meets this, or the running app's own requirement.
/// Moving to a new key (a Developer ID, say) means shipping one release that lists both, signed with the old key.
pub const RELEASE_REQUIREMENTS: &[&str] = &[r#"identifier "dev.valerie.browser" and certificate leaf = H"C69D28F44851570281196E2FC1C24BDCB9B9B6D1""#];

/// Stable releases: GitHub's "latest release", which only ever points at a build the maintainer promoted.
pub const STABLE_FEED: &str = "https://github.com/cullen-b/valerie/releases/latest/download/latest.json";
/// Every build of main, as soon as it's built (the rolling `edge` release; .github/workflows/release.yml).
pub const EDGE_FEED: &str = "https://github.com/cullen-b/valerie/releases/download/edge/latest.json";

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub version: String,
    /// The release's web page, for platforms that update by hand.
    #[serde(default)]
    pub page: String,
    #[serde(default)]
    pub assets: HashMap<String, Asset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub url: String,
    pub sha256: String,
}

/// This build's version, e.g. "0.1.142" (the last number counts commits on main, so later builds are bigger).
pub fn current() -> &'static str {
    option_env!("VALERIE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// Next to the app: the update being unpacked, and the copy still running after an update.
const KEPT_FOLDER: &str = ".valerie-update";

/// Whether version `a` is later than `b` ("0.1.10" > "0.1.9").
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.trim_start_matches('v').split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parts(a) > parts(b)
}

pub fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// The Valerie.app this process runs from, if it runs from one (the bare `target/*/valerie` binary doesn't).
pub fn app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    exe.ancestors().nth(3).filter(|p| p.extension().is_some_and(|x| x == "app")).map(Path::to_path_buf)
}

/// Read the release feed (a small JSON file next to the release's downloads).
pub fn fetch(feed: &str) -> Result<Release> {
    let out = crate::platform::command("curl").args(["-fsSL", "--max-time", "20", "--proto", "=https,file", feed]).output().context("couldn't run curl")?;
    if !out.status.success() {
        bail!("couldn't reach {feed}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    serde_json::from_slice(&out.stdout).context("the update feed isn't valid")
}

/// At launch: throw away an update that was only half unpacked. The version that ran before the last update stays
/// (`previous.app`) until this one has proved itself, so a release that crashes at launch can be rolled back.
pub fn clean_up_previous() {
    if let Some(parent) = app_bundle().filter(|_| !is_test_instance()).as_deref().and_then(Path::parent) {
        let _ = std::fs::remove_dir_all(parent.join(KEPT_FOLDER).join("incoming"));
    }
}

/// This version has run for `log::HEALTHY_SECS` without crashing: the kept older version isn't needed any more.
pub fn forget_previous() {
    let _ = std::fs::remove_file(crate::config::data_dir().join(CRASH_COUNT));
    if let Some(parent) = app_bundle().filter(|_| !is_test_instance()).as_deref().and_then(Path::parent) {
        let kept = parent.join(KEPT_FOLDER);
        let _ = std::fs::remove_dir_all(kept.join("previous.app"));
        let _ = std::fs::remove_dir_all(kept.join("crashing.app"));
    }
}

/// In the data folder: how many launches in a row of this version crashed early ("version count").
const CRASH_COUNT: &str = "launch-crashes";
/// In the data folder: a version that was rolled back, which updates skip until a newer one is out.
const SKIPPED: &str = "skipped-version";
/// In the data folder: "bad version<TAB>version gone back to", for the notice the older version shows.
const ROLLED_BACK: &str = "rolled-back";
/// Rolled back after this many launches in a row crashed early.
const CRASHES_BEFORE_ROLLBACK: u32 = 2;

/// When this version keeps crashing at launch (`CRASHES_BEFORE_ROLLBACK` early crashes in a row) and the version from
/// before the last update is still kept, put that one back, open it, and return true (this process should exit).
/// Tabs and settings live outside the app, so going back doesn't touch them.
pub fn roll_back_if_crashing(crash: Option<&crate::log::Crash>, enabled: bool) -> bool {
    let file = crate::config::data_dir().join(CRASH_COUNT);
    let Some(_) = crash.filter(|c| c.early && c.version == current()) else {
        let _ = std::fs::remove_file(&file);
        return false;
    };
    let before = std::fs::read_to_string(&file).ok().and_then(|t| {
        let (v, n) = t.trim().split_once(' ')?;
        (v == current()).then(|| n.parse::<u32>().ok()).flatten()
    });
    let count = before.unwrap_or(0) + 1;
    let _ = std::fs::write(&file, format!("{} {count}", current()));
    if !enabled || count < CRASHES_BEFORE_ROLLBACK || !cfg!(target_os = "macos") || is_test_instance() {
        return false;
    }
    match roll_back() {
        Ok(old) => {
            crate::log::line("update", format!("Valerie {} crashed at launch {count} times in a row: went back to {old}", current()));
            let _ = std::fs::remove_file(&file);
            true
        }
        Err(e) => {
            crate::log::line("update", format!("Valerie {} keeps crashing at launch and couldn't go back: {e:#}", current()));
            false
        }
    }
}

fn roll_back() -> Result<String> {
    let app = app_bundle().context("not running from Valerie.app")?;
    let kept = app.parent().context("no folder around the app")?.join(KEPT_FOLDER);
    let previous = kept.join("previous.app");
    let old = bundle_version(&previous).context("no earlier version is kept (it's dropped once a version runs well)")?;
    if !newer(current(), &old) {
        bail!("the kept version ({old}) isn't older than this one");
    }
    verify(&previous, &app)?;
    let bad = kept.join("crashing.app");
    let _ = std::fs::remove_dir_all(&bad);
    std::fs::rename(&app, &bad).context("couldn't move this version aside")?;
    if let Err(e) = std::fs::rename(&previous, &app) {
        let _ = std::fs::rename(&bad, &app);
        return Err(e).context("couldn't put the earlier version back");
    }
    let lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    let _ = crate::platform::command(lsregister).arg("-f").arg(&app).status();
    let data = crate::config::data_dir();
    let _ = std::fs::write(data.join(SKIPPED), current());
    let _ = std::fs::write(data.join(ROLLED_BACK), format!("{}\t{old}", current()));
    crate::log::note_quit();
    // `open` doesn't pass this process's environment on; a test copy's VALERIE_* folders have to go with it.
    let mut open = crate::platform::command("open");
    open.arg("-n");
    for (k, v) in std::env::vars().filter(|(k, _)| k.starts_with("VALERIE_")) {
        open.arg("--env").arg(format!("{k}={v}"));
    }
    open.arg(&app).spawn().context("couldn't open the earlier version")?;
    Ok(old)
}

/// The version an app bundle says it is.
fn bundle_version(app: &Path) -> Option<String> {
    let out = crate::platform::command("plutil").args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"]).arg(app.join("Contents/Info.plist")).output().ok()?;
    let v = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !v.is_empty()).then_some(v)
}

/// A version that was rolled back because it crashed at launch: updates leave it alone.
pub fn skipped() -> Option<String> {
    std::fs::read_to_string(crate::config::data_dir().join(SKIPPED)).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// After a roll back, once: (the version that crashed, the one now running).
pub fn take_rollback_note() -> Option<(String, String)> {
    let path = crate::config::data_dir().join(ROLLED_BACK);
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    text.trim().split_once('\t').map(|(a, b)| (a.to_string(), b.to_string()))
}

/// A test instance leaves updates alone, unless VALERIE_UPDATE_TEST lets it update its own copy of the app.
pub fn is_test_instance() -> bool {
    (std::env::var_os("VALERIE_CONFIG_DIR").is_some() || std::env::var_os("VALERIE_DATA_DIR").is_some()) && std::env::var_os("VALERIE_UPDATE_TEST").is_none()
}

/// Download `release` and put it in place of `app` (macOS). Only a build signed with Valerie's release key (or the
/// same way as the running app) is accepted; anything else is thrown away.
pub fn install(release: &Release, app: &Path) -> Result<()> {
    let asset = release.assets.get(platform()).context("this release has no download for this computer")?;
    let parent = app.parent().context("no folder around the app")?;
    // Unpack next to the app, so the swap is a rename on the same disk.
    let kept = parent.join(KEPT_FOLDER);
    let work = kept.join("incoming");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).with_context(|| format!("Valerie can't write to {} (it updates itself there)", parent.display()))?;
    let result = (|| {
        let zip = work.join("Valerie.zip");
        run(crate::platform::command("curl").args(["-fsSL", "--max-time", "600", "--proto", "=https,file", "-o"]).arg(&zip).arg(&asset.url), "download the update")?;
        let sum = String::from_utf8(crate::platform::command("shasum").args(["-a", "256"]).arg(&zip).output()?.stdout)?;
        if !sum.split_whitespace().next().is_some_and(|s| s.eq_ignore_ascii_case(&asset.sha256)) {
            bail!("the download was damaged (its checksum doesn't match)");
        }
        let unpacked = work.join("unpacked");
        run(crate::platform::command("ditto").args(["-x", "-k"]).arg(&zip).arg(&unpacked), "unpack the update")?;
        let new = unpacked.join("Valerie.app");
        verify(&new, app)?;
        // Swap: the new one is what opens next time. The running copy stays on disk until then (macOS refuses it the
        // Downloads folder once its files are gone, so downloads fail); if it's already kept, `app` never ran.
        let previous = kept.join("previous.app");
        let old = if previous.exists() { work.join("replaced.app") } else { previous };
        std::fs::rename(app, &old).context("couldn't move the current Valerie aside")?;
        if let Err(e) = std::fs::rename(&new, app) {
            let _ = std::fs::rename(&old, app);
            return Err(e).context("couldn't put the new Valerie in place");
        }
        let lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
        let _ = crate::platform::command(lsregister).arg("-f").arg(app).status();
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// The new app must have a valid signature that meets the release key's requirement or the running app's own.
fn verify(new: &Path, running: &Path) -> Result<()> {
    run(crate::platform::command("codesign").args(["--verify", "--deep", "--strict"]).arg(new), "check the update's signature")?;
    let own = crate::platform::command("codesign").args(["-d", "-r-"]).arg(running).output().ok().and_then(|o| {
        let text = String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr);
        text.lines().find_map(|l| l.strip_prefix("designated => ").map(str::to_string))
    });
    // An ad-hoc signature's requirement is its exact build, which no other build meets; skip it.
    let own = own.filter(|r| !r.starts_with("cdhash"));
    for req in RELEASE_REQUIREMENTS.iter().map(|r| r.to_string()).chain(own) {
        let ok = crate::platform::command("codesign").arg("--verify").arg(format!("--test-requirement=={req}")).arg(new).status().is_ok_and(|s| s.success());
        if ok {
            return Ok(());
        }
    }
    bail!("the update isn't signed by Valerie's release key, so it wasn't installed")
}

fn run(cmd: &mut Command, what: &str) -> Result<()> {
    let out = cmd.output().with_context(|| format!("couldn't {what}"))?;
    if !out.status.success() {
        bail!("couldn't {what}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn compares_versions() {
        assert!(super::newer("0.1.10", "0.1.9"));
        assert!(super::newer("v0.2.0", "0.1.99"));
        assert!(!super::newer("0.1.9", "0.1.9"));
        assert!(!super::newer("0.1.8", "0.1.9"));
    }
}
