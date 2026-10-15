//! Content blocking: hosts-style blocklists compiled into WebKit content rules (macOS).
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const IDENTIFIER: &str = "valerie-blocklist";
/// WebKit rejects rule lists above 150k rules.
const MAX_RULES: usize = 145_000;

/// Hide the most common ad containers that domain blocking leaves as empty boxes.
const COSMETIC: &str = ".adsbygoogle, ins.adsbygoogle, [id^='google_ads_iframe'], [id^='div-gpt-ad'], .ad-slot, .ad-banner, \
    [data-ad-slot], [data-google-query-id], amp-ad, .taboola, [id^='taboola-'], .OUTBRAIN, [data-widget-id^='outbrain']";

pub fn dir() -> PathBuf {
    crate::config::data_dir().join("blocklists")
}

/// Domains from a hosts file ("0.0.0.0 ads.example.com") or a plain one-domain-per-line list.
pub fn parse_hosts(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let l = l.split('#').next()?.trim();
            let mut parts = l.split_whitespace();
            let first = parts.next()?;
            let domain = match parts.next() {
                Some(d) if matches!(first, "0.0.0.0" | "127.0.0.1" | "::" | "::1") => d,
                Some(_) => return None,
                None => first,
            };
            let d = domain.trim_end_matches('.').to_ascii_lowercase();
            let valid = d.contains('.')
                && !matches!(d.as_str(), "localhost" | "localhost.localdomain" | "broadcasthost" | "local" | "0.0.0.0")
                && d.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
            valid.then_some(d)
        })
        .collect()
}

fn regex_escape(d: &str) -> String {
    d.replace('.', "\\.")
}

/// WebKit content-rule JSON: block requests to listed domains (and subdomains), hide ad slots, skip allowed sites.
pub fn rules_json(domains: &[String], allow: &[String]) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut rules: Vec<Value> = domains
        .iter()
        .filter(|d| seen.insert(d.as_str()))
        .take(MAX_RULES)
        .map(|d| json!({ "trigger": { "url-filter": format!("^[^:]+://+([^:/]+\\.)?{}[:/]", regex_escape(d)) }, "action": { "type": "block" } }))
        .collect();
    rules.push(json!({ "trigger": { "url-filter": ".*" }, "action": { "type": "css-display-none", "selector": COSMETIC } }));
    if !allow.is_empty() {
        let domains: Vec<String> = allow.iter().map(|a| format!("*{}", a.trim_start_matches("*."))).collect();
        rules.push(json!({ "trigger": { "url-filter": ".*", "if-domain": domains }, "action": { "type": "ignore-previous-rules" } }));
    }
    Value::Array(rules).to_string()
}

fn cache_path(url: &str) -> PathBuf {
    let h = url.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    dir().join(format!("{h:016x}.txt"))
}

fn fresh(path: &Path, max_age_days: f64) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs_f64() < max_age_days * 86400.0)
}

/// Download (or reuse cached copies of) every list and return the merged domains.
pub fn fetch(urls: &[String], refresh_days: f64) -> Result<Vec<String>> {
    std::fs::create_dir_all(dir())?;
    let mut domains = Vec::new();
    for url in urls {
        let path = cache_path(url);
        if !fresh(&path, refresh_days) {
            let tmp = path.with_extension("download");
            let ok = std::process::Command::new("curl")
                .args(["-fsSL", "--max-time", "60", "-o"])
                .arg(&tmp)
                .arg(url)
                .status()
                .is_ok_and(|s| s.success());
            if ok {
                std::fs::rename(&tmp, &path)?;
            } else if !path.exists() {
                bail!("could not download blocklist {url}");
            }
        }
        domains.extend(parse_hosts(&std::fs::read_to_string(&path)?));
    }
    Ok(domains)
}

pub fn hash(s: &str) -> String {
    format!("{:016x}", s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3)))
}

pub fn stored_hash() -> Option<String> {
    std::fs::read_to_string(dir().join("compiled.hash")).ok()
}

pub fn store_hash(h: &str) {
    let _ = std::fs::write(dir().join("compiled.hash"), h);
}

#[cfg(target_os = "macos")]
pub mod mac {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use wry::{WebView, WebViewExtMacOS};

    /// A compiled WKContentRuleList. Only ever touched on the main thread.
    pub struct RuleList(pub Retained<AnyObject>);
    // SAFETY: created in a WebKit completion handler on the main thread and only used there.
    unsafe impl Send for RuleList {}

    fn store() -> *mut AnyObject {
        unsafe { msg_send![class!(WKContentRuleListStore), defaultStore] }
    }

    fn handler(done: impl FnOnce(Result<RuleList, String>) + 'static) -> block2::RcBlock<dyn Fn(*mut AnyObject, *mut AnyObject)> {
        let done = std::cell::RefCell::new(Some(done));
        block2::RcBlock::new(move |list: *mut AnyObject, error: *mut AnyObject| {
            let Some(done) = done.borrow_mut().take() else { return };
            match unsafe { Retained::retain(list) } {
                Some(l) => done(Ok(RuleList(l))),
                None => {
                    let msg = unsafe {
                        if error.is_null() {
                            "no rule list".to_string()
                        } else {
                            let d: *mut objc2_foundation::NSString = msg_send![error, localizedDescription];
                            (*d).to_string()
                        }
                    };
                    done(Err(msg))
                }
            }
        })
    }

    pub fn compile(json: &str, done: impl FnOnce(Result<RuleList, String>) + 'static) {
        let id = objc2_foundation::NSString::from_str(super::IDENTIFIER);
        let rules = objc2_foundation::NSString::from_str(json);
        let block = handler(done);
        unsafe {
            let _: () = msg_send![store(), compileContentRuleListForIdentifier: &*id, encodedContentRuleList: &*rules, completionHandler: &*block];
        }
    }

    pub fn lookup(done: impl FnOnce(Result<RuleList, String>) + 'static) {
        let id = objc2_foundation::NSString::from_str(super::IDENTIFIER);
        let block = handler(done);
        unsafe {
            let _: () = msg_send![store(), lookUpContentRuleListForIdentifier: &*id, completionHandler: &*block];
        }
    }

    pub fn apply(wv: &WebView, list: Option<&RuleList>) {
        let ucc = wv.manager();
        unsafe {
            let _: () = msg_send![&*ucc, removeAllContentRuleLists];
            if let Some(l) = list {
                let _: () = msg_send![&*ucc, addContentRuleList: &*l.0];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hosts_and_plain_lists() {
        let t = "# c\n0.0.0.0 ads.example.com\n127.0.0.1 localhost\n0.0.0.0 0.0.0.0\ntracker.net # x\n10.0.0.1 lan.host\n";
        assert_eq!(parse_hosts(t), vec!["ads.example.com", "tracker.net"]);
    }

    #[test]
    fn rules_are_valid_json_with_allow_list() {
        let j: Value = serde_json::from_str(&rules_json(&["a.com".into(), "a.com".into()], &["ok.org".into()])).unwrap();
        let rules = j.as_array().unwrap();
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0]["trigger"]["url-filter"], "^[^:]+://+([^:/]+\\.)?a\\.com[:/]");
        assert_eq!(rules[2]["action"]["type"], "ignore-previous-rules");
    }
}
