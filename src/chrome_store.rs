//! Adding an extension from the Chrome Web Store: download its package, unpack it into the extensions folder, and let
//! Valerie's usual "Load this extension?" question take it from there.
use anyhow::{bail, Context, Result};
use std::path::Path;

/// Marks a folder that came from the store, so adding the same extension again updates it and nothing else.
const MARKER: &str = ".chrome-web-store-id";
/// The Chrome version Valerie asks the store as, and that store extensions are told they run in.
const CHROME: &str = "142.0.7444.60";
/// Put first in a store extension's background and pages (`prepare`).
const SHIM: &str = include_str!("../ui/extension-shim.js");
const SHIM_FILE: &str = "valerie-shim.js";
/// Which shim a prepared folder carries, so it's only redone when the shim changes.
const STAMP: &str = ".valerie-shim";
const START: &str = "/* Valerie: Chrome behaviour WebKit lacks (ui/extension-shim.js) */";
const END: &str = "/* Valerie: end of shim */";
const TAG: &str = "<script src=\"/valerie-shim.js\"></script>";

/// The extension's ID from a store link or a bare ID.
pub fn extension_id(link_or_id: &str) -> Option<String> {
    let s = link_or_id.trim().trim_end_matches('/');
    let s = s.split(['?', '#']).next().unwrap_or(s);
    let last = s.rsplit('/').next().unwrap_or(s).to_ascii_lowercase();
    (last.len() == 32 && last.bytes().all(|b| (b'a'..=b'p').contains(&b))).then_some(last)
}

/// The zip inside a store package (CRX2 or CRX3).
pub fn zip_of(crx: &[u8]) -> Result<&[u8]> {
    let u32_at = |i: usize| crx.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    if crx.get(..4) != Some(b"Cr24") {
        bail!("the download isn't an extension package");
    }
    let start = match u32_at(4) {
        Some(3) => u32_at(8).map(|n| 12 + n),
        Some(2) => u32_at(8).zip(u32_at(12)).map(|(k, s)| 16 + k + s),
        _ => None,
    };
    let zip = start.and_then(|s| crx.get(s..)).context("the extension package is cut short")?;
    if zip.get(..2) != Some(b"PK") {
        bail!("the extension package has no files in it");
    }
    Ok(zip)
}

/// A folder name from the manifest's name (looked up in its translations when it's a `__MSG_x__` key).
fn folder_name(dir: &Path, id: &str) -> String {
    let manifest = std::fs::read_to_string(dir.join("manifest.json")).ok().and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok()).unwrap_or_default();
    let name = crate::extensions::manifest_text(&manifest, "name", dir);
    let slug = name.to_ascii_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-");
    if slug.is_empty() { id.to_string() } else { slug.chars().take(40).collect::<String>().trim_end_matches('-').to_string() }
}

fn unzip(zip: &Path, to: &Path) -> Result<()> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = crate::platform::command("ditto");
        c.args(["-x", "-k"]).arg(zip).arg(to);
        c
    } else if cfg!(windows) {
        let mut c = crate::platform::command("tar");
        c.arg("-xf").arg(zip).arg("-C").arg(to);
        c
    } else {
        let mut c = crate::platform::command("unzip");
        c.args(["-q", "-o"]).arg(zip).arg("-d").arg(to);
        c
    };
    if !cmd.status().context("couldn't run the unzip tool")?.success() {
        bail!("couldn't unpack the extension");
    }
    Ok(())
}

/// Download the extension behind `link_or_id` into `dir/<name>`; returns the folder name.
pub fn add(link_or_id: &str, dir: &Path) -> Result<String> {
    let id = extension_id(link_or_id).context("that isn't a Chrome Web Store link or extension ID (32 letters a to p)")?;
    std::fs::create_dir_all(dir).with_context(|| format!("couldn't make {}", dir.display()))?;
    let work = dir.join(format!(".adding-{id}"));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    let result = (|| -> Result<String> {
        let crx = work.join("package.crx");
        let url = format!("https://clients2.google.com/service/update2/crx?response=redirect&prodversion={CHROME}&acceptformat=crx2,crx3&x=id%3D{id}%26uc");
        let ok = crate::platform::command("curl").args(["-fsSL", "--max-time", "120", "--proto", "=https", "-o"]).arg(&crx).arg(&url).status()?.success();
        let bytes = std::fs::read(&crx).unwrap_or_default();
        if !ok || bytes.is_empty() {
            bail!("the Chrome Web Store didn't send extension {id} (check the link, or it may not be available)");
        }
        let zip = work.join("package.zip");
        std::fs::write(&zip, zip_of(&bytes)?)?;
        let files = work.join("files");
        std::fs::create_dir_all(&files)?;
        unzip(&zip, &files)?;
        if !files.join("manifest.json").is_file() {
            bail!("the extension has no manifest.json");
        }
        let name = folder_name(&files, &id);
        let dest = dir.join(&name);
        if dest.exists() {
            let theirs = std::fs::read_to_string(dest.join(MARKER)).unwrap_or_default();
            if theirs.trim() != id {
                bail!("there's already an extension folder called {name} that didn't come from this store page");
            }
            std::fs::remove_dir_all(&dest).with_context(|| format!("couldn't replace {}", dest.display()))?;
        }
        std::fs::write(files.join(MARKER), &id)?;
        prepare(&files)?;
        std::fs::rename(&files, &dest).with_context(|| format!("couldn't move it into {}", dest.display()))?;
        Ok(name)
    })();
    let _ = std::fs::remove_dir_all(&work);
    if let Err(e) = &result {
        crate::log::line("extensions", format!("adding {id} from the Chrome Web Store: {e:#}"));
    }
    result
}

/// A path from the manifest, if it's a plain file inside `dir` (not a link, not outside it).
fn inside(dir: &Path, rel: &str) -> Option<std::path::PathBuf> {
    let rel = rel.trim_start_matches('/');
    if rel.split(['/', '\\']).any(|p| p == "..") || Path::new(rel).is_absolute() {
        return None;
    }
    let p = dir.join(rel);
    std::fs::symlink_metadata(&p).ok().filter(|m| m.is_file()).map(|_| p)
}

/// The script without a shim this put in front of it before.
fn without_shim(source: &str) -> &str {
    let mut s = source;
    if s.starts_with(START) {
        if let Some(i) = s.find(END) {
            s = s[i + END.len()..].trim_start_matches('\n');
        }
    }
    let import = format!("import \"/{SHIM_FILE}\";\n");
    s.strip_prefix(import.as_str()).unwrap_or(s)
}

/// The page with the shim's script tag as the first thing in its head.
fn page_with_shim(html: &str) -> String {
    let html = html.replace(TAG, "");
    let lower = html.to_ascii_lowercase();
    let at = lower.find("<head").and_then(|h| lower[h..].find('>').map(|e| h + e + 1)).or_else(|| {
        lower.trim_start().starts_with("<!doctype").then(|| lower.find('>').map(|e| e + 1)).flatten()
    }).unwrap_or(0);
    format!("{}{TAG}{}", &html[..at], &html[at..])
}

fn html_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(t) = e.file_type() else { continue };
        let p = e.path();
        if t.is_dir() {
            html_files(&p, out);
        } else if t.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("html") || x.eq_ignore_ascii_case("htm")) {
            out.push(p);
        }
    }
}

/// Put the shim first in a store extension's background and in every page it ships, so it runs before the extension's
/// own code. manifest.json is never changed (Valerie's "Load this extension?" answer is tied to it). Folders that didn't
/// come from the store, and ones already carrying this shim, are left alone.
pub fn prepare(dir: &Path) -> Result<()> {
    if !dir.join(MARKER).is_file() {
        return Ok(());
    }
    let script = SHIM.replace("__VALERIE_CHROME__", CHROME);
    let version = crate::blocking::hash(&script);
    if std::fs::read_to_string(dir.join(STAMP)).ok().as_deref() == Some(version.as_str()) {
        return Ok(());
    }
    let manifest: serde_json::Value = serde_json::from_str(std::fs::read_to_string(dir.join("manifest.json"))?.trim_start_matches('\u{feff}'))?;
    std::fs::write(dir.join(SHIM_FILE), &script)?;
    let bg = &manifest["background"];
    let module = bg["type"].as_str() == Some("module");
    let first = bg["service_worker"].as_str().or_else(|| bg["scripts"].get(0).and_then(|v| v.as_str()));
    if let Some(path) = first.and_then(|f| inside(dir, f)) {
        let source = std::fs::read_to_string(&path)?;
        let rest = without_shim(&source);
        let out = if module && bg["service_worker"].is_string() { format!("import \"/{SHIM_FILE}\";\n{rest}") } else { format!("{START}\n{script}\n{END}\n{rest}") };
        std::fs::write(&path, out)?;
    }
    let mut pages = vec![];
    html_files(dir, &mut pages);
    for p in pages {
        if let Ok(html) = std::fs::read_to_string(&p) {
            std::fs::write(&p, page_with_shim(&html))?;
        }
    }
    std::fs::write(dir.join(STAMP), version)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "nngceckbapebfimnlniiiahkandclblb";

    #[test]
    fn ids_come_from_links_or_stand_alone() {
        assert_eq!(extension_id(&format!("https://chromewebstore.google.com/detail/bitwarden-password-manager/{ID}")).as_deref(), Some(ID));
        assert_eq!(extension_id(&format!("https://chrome.google.com/webstore/detail/bitwarden/{ID}?hl=en")).as_deref(), Some(ID));
        assert_eq!(extension_id(&format!("  {}/ ", ID.to_uppercase())).as_deref(), Some(ID));
        assert_eq!(extension_id("nngceckbapebfimnlniiiahkandclblz"), None);
        assert_eq!(extension_id("nngceckbapebfimnlniiiahkandclbl"), None);
        assert_eq!(extension_id("../../etc/passwd"), None);
    }

    #[test]
    fn package_headers_are_stripped() {
        let mut v3 = b"Cr24".to_vec();
        v3.extend(3u32.to_le_bytes());
        v3.extend(2u32.to_le_bytes());
        v3.extend(b"xxPK\x03\x04");
        assert_eq!(zip_of(&v3).unwrap(), b"PK\x03\x04");
        let mut v2 = b"Cr24".to_vec();
        v2.extend(2u32.to_le_bytes());
        v2.extend(1u32.to_le_bytes());
        v2.extend(2u32.to_le_bytes());
        v2.extend(b"kssPK");
        assert_eq!(zip_of(&v2).unwrap(), b"PK");
        assert!(zip_of(b"PK\x03\x04").is_err());
        let mut short = b"Cr24".to_vec();
        short.extend(3u32.to_le_bytes());
        short.extend(99u32.to_le_bytes());
        assert!(zip_of(&short).is_err());
    }

    #[test]
    fn folder_names_come_from_the_manifest_or_its_translations() {
        let dir = std::env::temp_dir().join(format!("valerie-cws-name-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("_locales/en")).unwrap();
        std::fs::write(dir.join("manifest.json"), r#"{"name":"__MSG_extName__","default_locale":"en"}"#).unwrap();
        std::fs::write(dir.join("_locales/en/messages.json"), r#"{"extName":{"message":"Bitwarden Password Manager"}}"#).unwrap();
        assert_eq!(folder_name(&dir, ID), "bitwarden-password-manager");
        std::fs::write(dir.join("manifest.json"), r#"{"name":"uBlock Origin"}"#).unwrap();
        assert_eq!(folder_name(&dir, ID), "ublock-origin");
        std::fs::write(dir.join("manifest.json"), r#"{"name":"…"}"#).unwrap();
        assert_eq!(folder_name(&dir, ID), ID);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_shim_goes_first_once() {
        let dir = std::env::temp_dir().join(format!("valerie-cws-shim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("popup")).unwrap();
        let manifest = r#"{"manifest_version":3,"background":{"service_worker":"bg.js"}}"#;
        std::fs::write(dir.join("manifest.json"), manifest).unwrap();
        std::fs::write(dir.join("bg.js"), "start();\n").unwrap();
        std::fs::write(dir.join("popup/index.html"), "<!doctype html><html><HEAD><title>x</title></head></html>").unwrap();
        prepare(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("bg.js")).unwrap(), "start();\n", "not from the store: untouched");
        std::fs::write(dir.join(MARKER), ID).unwrap();
        prepare(&dir).unwrap();
        std::fs::remove_file(dir.join(STAMP)).unwrap();
        prepare(&dir).unwrap();
        let bg = std::fs::read_to_string(dir.join("bg.js")).unwrap();
        assert!(bg.starts_with(START) && bg.ends_with(&format!("{END}\nstart();\n")) && bg.matches(START).count() == 1, "{bg}");
        assert!(bg.contains(CHROME) && !bg.contains("__VALERIE_CHROME__"));
        let page = std::fs::read_to_string(dir.join("popup/index.html")).unwrap();
        assert_eq!(page, format!("<!doctype html><html><HEAD>{TAG}<title>x</title></head></html>"));
        assert_eq!(std::fs::read_to_string(dir.join("manifest.json")).unwrap(), manifest);
        std::fs::write(dir.join("manifest.json"), r#"{"background":{"service_worker":"bg.js","type":"module"}}"#).unwrap();
        std::fs::remove_file(dir.join(STAMP)).unwrap();
        prepare(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("bg.js")).unwrap(), format!("import \"/{SHIM_FILE}\";\nstart();\n"));
        std::fs::write(dir.join("manifest.json"), r#"{"background":{"service_worker":"../outside.js"}}"#).unwrap();
        std::fs::remove_file(dir.join(STAMP)).unwrap();
        prepare(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
