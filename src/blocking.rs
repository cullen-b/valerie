//! Content blocking: hosts-style blocklists compiled into WebKit content rules (macOS).
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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

/// Whether a list is in Adblock filter syntax (EasyList and the like) rather than a hosts file.
pub fn is_adblock(text: &str) -> bool {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    first.starts_with("[Adblock") || text.lines().take(400).any(|l| l.starts_with("||") || l.contains("##"))
}

/// WebKit rules from Adblock filters: (blocking and hiding rules, exceptions). Filters WebKit can't express
/// (regular expressions, scriptlets, rewrites) are skipped.
pub fn parse_adblock(text: &str) -> (Vec<Value>, Vec<Value>) {
    let (mut rules, mut exceptions, mut generic) = (Vec::new(), Vec::new(), Vec::new());
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('!') || line.starts_with('[') || !line.is_ascii() {
            continue;
        }
        if let Some((domains, selector)) = line.split_once("##") {
            // Procedural filters (:has-text, :-abp-…, :xpath) aren't CSS; #@#, #?#, #$# aren't plain hiding.
            if selector.is_empty() || domains.ends_with(['@', '?', '$', '%']) || selector.contains(":-abp-") || selector.contains(":has-text")
                || selector.contains(":xpath") || selector.contains(":matches-") || selector.contains(":upward") || selector.contains(":remove")
                || selector.contains(":style") || selector.contains(":min-text") || selector.contains(":watch-attr") || selector.starts_with("+js")
            {
                continue;
            }
            if domains.is_empty() {
                generic.push(selector.to_string());
            } else if let Some(trigger) = domain_trigger(domains, ',') {
                rules.push(json!({ "trigger": trigger, "action": { "type": "css-display-none", "selector": selector } }));
            }
            continue;
        }
        if line.contains("#@#") || line.contains("#?#") || line.contains("#$#") || line.contains("#%#") {
            continue;
        }
        let (exception, body) = match line.strip_prefix("@@") {
            Some(b) => (true, b),
            None => (false, line),
        };
        let Some(rule) = network_rule(body, exception) else { continue };
        if exception { exceptions.push(rule) } else { rules.push(rule) }
    }
    // Generic hiding selectors go in batches, so one WebKit can't use doesn't take all the others with it.
    for chunk in generic.chunks(250) {
        rules.push(json!({ "trigger": { "url-filter": ".*" }, "action": { "type": "css-display-none", "selector": chunk.join(", ") } }));
    }
    (rules, exceptions)
}

/// "a.com,~b.com" (or with `|` for $domain=) as a WebKit trigger on the page's domain.
fn domain_trigger(list: &str, sep: char) -> Option<Value> {
    let (mut yes, mut no) = (Vec::new(), Vec::new());
    for d in list.split(sep).map(str::trim).filter(|d| !d.is_empty()) {
        let (neg, d) = match d.strip_prefix('~') {
            Some(x) => (true, x),
            None => (false, d),
        };
        if d.contains(['*', '/']) || !d.contains('.') {
            continue;
        }
        let d = format!("*{}", d.to_ascii_lowercase());
        if neg { no.push(d) } else { yes.push(d) }
    }
    match (yes.is_empty(), no.is_empty()) {
        (false, _) => Some(json!({ "url-filter": ".*", "if-domain": yes })),
        (true, false) => Some(json!({ "url-filter": ".*", "unless-domain": no })),
        (true, true) => None,
    }
}

/// One Adblock network filter ("||ads.example.com^$third-party") as a WebKit rule.
fn network_rule(body: &str, exception: bool) -> Option<Value> {
    // Options come after the last $ (unless that $ is part of a regex-looking pattern).
    let (pattern, options) = match body.rfind('$') {
        Some(i) if !body.starts_with('/') => (&body[..i], Some(&body[i + 1..])),
        _ => (body, None),
    };
    if pattern.starts_with('/') && pattern.ends_with('/') && pattern.len() > 1 {
        return None; // regular expressions: WebKit's subset is too different
    }
    let mut trigger = serde_json::Map::new();
    let mut types: Vec<&str> = Vec::new();
    for opt in options.unwrap_or("").split(',').map(str::trim).filter(|o| !o.is_empty()) {
        let (neg, name) = match opt.strip_prefix('~') {
            Some(n) => (true, n),
            None => (false, opt),
        };
        match name {
            "third-party" | "3p" => drop(trigger.insert("load-type".into(), json!([if neg { "first-party" } else { "third-party" }]))),
            "first-party" | "1p" => drop(trigger.insert("load-type".into(), json!([if neg { "third-party" } else { "first-party" }]))),
            "match-case" => drop(trigger.insert("url-filter-is-case-sensitive".into(), json!(true))),
            "important" | "all" => {}
            _ if neg => {} // "not this type": WebKit can't say that; block every type instead
            "script" => types.push("script"),
            "image" => types.push("image"),
            "stylesheet" | "css" => types.push("style-sheet"),
            "font" => types.push("font"),
            "media" => types.push("media"),
            "xmlhttprequest" | "xhr" => types.push("raw"),
            "subdocument" | "frame" => types.push("document"),
            "document" | "doc" => types.push("document"),
            "websocket" => types.push("websocket"),
            "ping" => types.push("ping"),
            "popup" => types.push("popup"),
            "other" | "object" => types.push("other"),
            _ if name.starts_with("domain=") || name.starts_with("from=") => {
                let list = name.split_once('=').map(|x| x.1).unwrap_or("");
                if let Some(Value::Object(t)) = domain_trigger(list, '|') {
                    for (k, v) in t.into_iter().filter(|(k, _)| k != "url-filter") {
                        trigger.insert(k, v);
                    }
                }
            }
            // Anything else changes what the filter means (redirects, rewrites, CSP…): leave the filter out.
            _ => return None,
        }
    }
    if !types.is_empty() {
        trigger.insert("resource-type".into(), json!(types));
    }
    let regex = filter_regex(pattern)?;
    // A filter that matches everything is only all right when it's limited to some sites.
    if regex == ".*" && !trigger.contains_key("if-domain") && !exception {
        return None;
    }
    trigger.insert("url-filter".into(), json!(regex));
    Some(json!({ "trigger": trigger, "action": { "type": if exception { "ignore-previous-rules" } else { "block" } } }))
}

/// An Adblock address pattern as a WebKit url-filter: ||host anchors to a domain, | to the start or end, * is anything,
/// ^ is a separator.
fn filter_regex(pattern: &str) -> Option<String> {
    let mut p = pattern;
    let mut out = String::new();
    let host_anchor = p.starts_with("||");
    if host_anchor {
        p = &p[2..];
        out.push_str("^[^:]+://+([^:/]+\\.)?");
    } else if let Some(rest) = p.strip_prefix('|') {
        p = rest;
        out.push('^');
    }
    let end_anchor = p.ends_with('|') && !p.ends_with("\\|");
    if end_anchor {
        p = &p[..p.len() - 1];
    }
    let p = p.trim_start_matches('*');
    if p.is_empty() && !host_anchor {
        return Some(".*".into());
    }
    let mut in_host = host_anchor;
    let chars: Vec<char> = p.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        match c {
            '*' => out.push_str(".*"),
            '^' if i + 1 == chars.len() => {
                // A trailing separator after a host is the port or the path; elsewhere it may also be the end.
                if in_host {
                    out.push_str("[:/]");
                }
            }
            '^' => out.push_str("[^a-zA-Z0-9_.%-]"),
            '.' | '+' | '?' | '(' | ')' | '{' | '}' | '[' | ']' | '\\' | '$' | '|' => {
                out.push('\\');
                out.push(*c);
            }
            c => out.push(*c),
        }
        if matches!(c, '/' | '^' | ':' | '?' | '*') {
            in_host = false;
        }
    }
    if end_anchor {
        out.push('$');
    }
    Some(out)
}

/// Your own `rules:` entries that block requests, as WebKit rules (Adblock syntax, or an address with * wildcards).
pub fn user_block_rules(rules: &[crate::config::NetRule]) -> (Vec<Value>, Vec<Value>) {
    let mut text = String::new();
    for r in rules.iter().filter(|r| !r.block.trim().is_empty()) {
        let b = r.block.trim();
        let line = if b.contains("://") && !b.starts_with('|') && !b.starts_with("@@") { format!("|{b}") } else { b.to_string() };
        let site = r.site.trim();
        let opts = if site.is_empty() { String::new() } else { format!("{}domain={site}", if line.contains('$') { "," } else { "$" }) };
        text.push_str(&format!("{line}{opts}\n"));
    }
    parse_adblock(&text)
}

/// WebKit content-rule JSON: block requests to listed domains (and subdomains), then Adblock filters and your own
/// rules, hide ad slots, then exceptions, and skip allowed sites.
pub fn rules_json(domains: &[String], filters: &[Value], exceptions: &[Value], allow: &[String]) -> String {
    let mut seen = std::collections::HashSet::new();
    let room = MAX_RULES.saturating_sub(filters.len() + exceptions.len() + 2);
    let mut rules: Vec<Value> = domains
        .iter()
        .filter(|d| seen.insert(d.as_str()))
        .take(room)
        .map(|d| json!({ "trigger": { "url-filter": format!("^[^:]+://+([^:/]+\\.)?{}[:/]", regex_escape(d)) }, "action": { "type": "block" } }))
        .collect();
    rules.extend(filters.iter().take(MAX_RULES).cloned());
    rules.push(json!({ "trigger": { "url-filter": ".*" }, "action": { "type": "css-display-none", "selector": COSMETIC } }));
    rules.extend(exceptions.iter().cloned());
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

/// What the lists hold: domains from hosts files, and rules from Adblock lists.
#[derive(Default)]
pub struct Lists {
    pub domains: Vec<String>,
    pub filters: Vec<Value>,
    pub exceptions: Vec<Value>,
}

/// Download (or reuse cached copies of) every list and merge them.
pub fn fetch(urls: &[String], refresh_days: f64) -> Result<Lists> {
    std::fs::create_dir_all(dir())?;
    let mut lists = Lists::default();
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
        let text = std::fs::read_to_string(&path)?;
        if is_adblock(&text) {
            let (f, e) = parse_adblock(&text);
            lists.filters.extend(f);
            lists.exceptions.extend(e);
        } else {
            lists.domains.extend(parse_hosts(&text));
        }
    }
    Ok(lists)
}

pub fn hash(s: &str) -> String {
    format!("{:016x}", s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3)))
}

pub fn stored_hash() -> Option<String> {
    std::fs::read_to_string(dir().join("compiled.hash")).ok()
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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
        let j: Value = serde_json::from_str(&rules_json(&["a.com".into(), "a.com".into()], &[], &[], &["ok.org".into()])).unwrap();
        let rules = j.as_array().unwrap();
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0]["trigger"]["url-filter"], "^[^:]+://+([^:/]+\\.)?a\\.com[:/]");
        assert_eq!(rules[2]["action"]["type"], "ignore-previous-rules");
    }

    #[test]
    fn adblock_filters_become_webkit_rules() {
        let list = "[Adblock Plus 2.0]\n! comment\n||ads.example.com^\n||track.net^$third-party,script\n/banner/*\n@@||ads.example.com/ok^\n\
                    example.org##.promo\n##.ad-box\n~news.com##.side\nexample.org#@#.x\n/regex[0-9]/\n||cdn.net^$redirect=noop.js\n*$domain=a.com\n";
        assert!(is_adblock(list));
        let (rules, exceptions) = parse_adblock(list);
        let filter = |i: usize| rules[i]["trigger"]["url-filter"].as_str().unwrap().to_string();
        assert_eq!(filter(0), "^[^:]+://+([^:/]+\\.)?ads\\.example\\.com[:/]");
        assert_eq!(rules[1]["trigger"]["load-type"], json!(["third-party"]));
        assert_eq!(rules[1]["trigger"]["resource-type"], json!(["script"]));
        assert_eq!(filter(2), "/banner/.*");
        assert_eq!(rules[3]["action"]["selector"], ".promo");
        assert_eq!(rules[3]["trigger"]["if-domain"], json!(["*example.org"]));
        assert_eq!(rules[4]["trigger"]["unless-domain"], json!(["*news.com"]));
        assert_eq!(rules[5]["trigger"]["if-domain"], json!(["*a.com"]));
        assert_eq!(rules.last().unwrap()["action"]["selector"], ".ad-box");
        assert_eq!(rules.len(), 7, "regex, #@# and $redirect filters are left out: {rules:#?}");
        assert_eq!(exceptions.len(), 1);
        assert_eq!(exceptions[0]["action"]["type"], "ignore-previous-rules");
        assert!(!is_adblock("0.0.0.0 ads.example.com\n"));
        let mine = [crate::config::NetRule { block: "https://*.tracker.io/pixel*".into(), site: "news.com".into(), ..Default::default() }];
        let (r, _) = user_block_rules(&mine);
        assert_eq!(r[0]["trigger"]["url-filter"], "^https://.*\\.tracker\\.io/pixel.*");
        assert_eq!(r[0]["trigger"]["if-domain"], json!(["*news.com"]));
        // Everything is valid JSON WebKit can read.
        let all: Value = serde_json::from_str(&rules_json(&[], &rules, &exceptions, &[])).unwrap();
        assert!(all.as_array().unwrap().len() > 8);
    }
}
