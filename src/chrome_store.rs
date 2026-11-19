//! Adding an extension from the Chrome Web Store: download its package, unpack it into the extensions folder, and let
//! Valerie's usual "Load this extension?" question take it from there.
use anyhow::{bail, Context, Result};
use std::path::Path;

/// Marks a folder that came from the store, so adding the same extension again updates it and nothing else.
const MARKER: &str = ".chrome-web-store-id";

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
        let url = format!("https://clients2.google.com/service/update2/crx?response=redirect&prodversion=142.0.7444.60&acceptformat=crx2,crx3&x=id%3D{id}%26uc");
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
        std::fs::rename(&files, &dest).with_context(|| format!("couldn't move it into {}", dest.display()))?;
        Ok(name)
    })();
    let _ = std::fs::remove_dir_all(&work);
    if let Err(e) = &result {
        crate::log::line("extensions", format!("adding {id} from the Chrome Web Store: {e:#}"));
    }
    result
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
}
