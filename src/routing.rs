use crate::config::Route;

/// `*` matches any run of characters; matching is case-insensitive.
pub fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

/// Air Traffic Control: the space a URL should open in, if any rule matches.
pub fn route<'a>(routes: &'a [Route], url: &str) -> Option<&'a str> {
    let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    routes
        .iter()
        .find(|r| {
            if r.pattern.contains("://") {
                glob(&r.pattern, url)
            } else {
                glob(&r.pattern, &host) || glob(&format!("*.{}", r.pattern), &host)
            }
        })
        .map(|r| r.space.as_str())
}

/// Turn command-bar input into a URL: real URLs pass through, bare domains get https, everything else is a search.
pub fn resolve_input(input: &str, search_engine: &str) -> String {
    let s = input.trim();
    let has_scheme = ["http://", "https://", "file://", "about:", "data:", "view-source:"]
        .iter()
        .any(|p| s.to_lowercase().starts_with(p));
    if has_scheme {
        return s.to_string();
    }
    let looks_like_host = !s.contains(' ')
        && (s.starts_with("localhost")
            || s.split('/').next().is_some_and(|h| {
                let h = h.split(':').next().unwrap_or(h);
                h.contains('.') && !h.starts_with('.') && !h.ends_with('.')
                    && h.rsplit('.').next().is_some_and(|tld| tld.chars().all(|c| c.is_ascii_alphanumeric()) && tld.len() >= 2)
            }));
    if looks_like_host {
        let scheme = if s.starts_with("localhost") || s.starts_with("127.") { "http" } else { "https" };
        return format!("{scheme}://{s}");
    }
    let q: String = url::form_urlencoded::byte_serialize(s.as_bytes()).collect();
    search_engine.replace("%s", &q)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globbing() {
        assert!(glob("*.edu", "canvas.mit.edu"));
        assert!(!glob("*.edu", "education.com"));
        assert!(glob("https://docs.google.com/*", "https://docs.google.com/doc/1"));
    }

    #[test]
    fn routes_match_host_and_subdomains() {
        let r = vec![
            Route { pattern: "*.edu".into(), space: "School".into() },
            Route { pattern: "github.com".into(), space: "Work".into() },
        ];
        assert_eq!(route(&r, "https://www.stanford.edu/x"), Some("School"));
        assert_eq!(route(&r, "https://gist.github.com/"), Some("Work"));
        assert_eq!(route(&r, "https://example.com"), None);
    }

    #[test]
    fn input_resolution() {
        let se = "https://s/?q=%s";
        assert_eq!(resolve_input("example.com", se), "https://example.com");
        assert_eq!(resolve_input("localhost:3000", se), "http://localhost:3000");
        assert_eq!(resolve_input("rust borrow checker", se), "https://s/?q=rust+borrow+checker");
        assert_eq!(resolve_input("v1.2", se), "https://s/?q=v1.2");
    }
}

/// Remove tracking parameters (`utm_*`, `fbclid`, …) from a URL's query. Returns the URL unchanged when none match.
pub fn strip_tracking(url: &str, params: &[String]) -> String {
    let Ok(mut u) = url::Url::parse(url) else { return url.to_string() };
    if u.query().is_none() || params.is_empty() {
        return url.to_string();
    }
    let hit = |k: &str| params.iter().any(|p| p.strip_suffix('*').map_or(p.eq_ignore_ascii_case(k), |pre| k.to_ascii_lowercase().starts_with(&pre.to_ascii_lowercase())));
    let pairs: Vec<(String, String)> = u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    if !pairs.iter().any(|(k, _)| hit(k)) {
        return url.to_string();
    }
    let keep: Vec<_> = pairs.into_iter().filter(|(k, _)| !hit(k)).collect();
    if keep.is_empty() {
        u.set_query(None);
    } else {
        u.query_pairs_mut().clear().extend_pairs(keep);
    }
    u.to_string()
}

#[cfg(test)]
mod tracking_tests {
    #[test]
    fn strips_only_tracking_params() {
        let p = vec!["utm_*".to_string(), "fbclid".to_string()];
        assert_eq!(super::strip_tracking("https://a.com/x?utm_source=t&id=3&fbclid=9", &p), "https://a.com/x?id=3");
        assert_eq!(super::strip_tracking("https://a.com/x?utm_medium=e", &p), "https://a.com/x");
        assert_eq!(super::strip_tracking("https://a.com/x?q=a+b&page=2", &p), "https://a.com/x?q=a+b&page=2");
    }
}
