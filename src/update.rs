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

/// Delete the copy of Valerie the last update kept for the process that was running then. Call when the app starts.
pub fn clean_up_previous() {
    if let Some(parent) = app_bundle().filter(|_| !is_test_instance()).as_deref().and_then(Path::parent) {
        let _ = std::fs::remove_dir_all(parent.join(KEPT_FOLDER));
    }
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
