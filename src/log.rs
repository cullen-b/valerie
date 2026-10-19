//! Valerie's log: what went wrong and what happened (commands, agent tool calls, page loads, plugin messages), one
//! file a day in the data folder's `logs`, so people and agents can see why something failed after the fact. Files
//! older than `logs.keep_days` are removed; `logs.level` sets how much goes in.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Days to keep (0 = no log files) and the day last written, to prune again when the date changes.
static STATE: Mutex<Option<(u64, String)>> = Mutex::new(None);
/// 0 = errors and messages only, 1 = also events, 2 = also every page load.
static LEVEL: AtomicU8 = AtomicU8::new(1);

pub fn dir() -> PathBuf {
    crate::config::data_dir().join("logs")
}

/// Start logging to files (until this runs, lines only go to stderr, which keeps tests out of the real log).
pub fn init(keep_days: u64) {
    *STATE.lock().unwrap_or_else(|e| e.into_inner()) = Some((keep_days, String::new()));
}

/// `logs.level`: errors | events | verbose.
pub fn set_level(level: &str) {
    LEVEL.store(match level { "errors" => 0, "verbose" => 2, _ => 1 }, Ordering::Relaxed);
}

/// Log something that happened (not a failure): kept at `logs.level` events or verbose.
pub fn event(area: &str, msg: impl std::fmt::Display) {
    if LEVEL.load(Ordering::Relaxed) >= 1 {
        line(area, msg);
    }
}

/// Log something frequent (every page load): kept only at `logs.level: verbose`.
pub fn verbose(area: &str, msg: impl std::fmt::Display) {
    if LEVEL.load(Ordering::Relaxed) >= 2 {
        line(area, msg);
    }
}

/// Log one line: `line("sync", format!("failed: {e}"))`. Also printed to stderr.
pub fn line(area: &str, msg: impl std::fmt::Display) {
    let (day, time) = now();
    let text = format!("{day}T{time}Z [{area}] {}", msg.to_string().replace('\n', "\n    "));
    eprintln!("valerie: {text}");
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let Some((keep, last)) = st.as_mut() else { return };
    if *keep == 0 {
        return;
    }
    if *last != day {
        prune(*keep);
        *last = day.clone();
    }
    let _ = std::fs::create_dir_all(dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir().join(format!("valerie-{day}.log"))) {
        let _ = writeln!(f, "{text}");
    }
}

fn prune(keep_days: u64) {
    let cutoff = SystemTime::now() - Duration::from_secs(keep_days * 86_400);
    for e in std::fs::read_dir(dir()).into_iter().flatten().flatten() {
        let old = e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < cutoff);
        if old && e.file_name().to_string_lossy().ends_with(".log") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// The newest log entries, oldest first: only `area` (a prefix, so "plugin" takes in "plugin:dark-reader"), only
/// entries containing `contains` (any case), only the last `since_minutes`; at most `limit` of them.
pub fn read(area: &str, contains: &str, since_minutes: Option<u64>, limit: usize) -> Vec<String> {
    let cutoff = since_minutes.map(|m| {
        let (d, t) = stamp(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs().saturating_sub(m * 60));
        format!("{d}T{t}Z")
    });
    let needle = contains.to_lowercase();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir()).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("valerie-") && n.to_string_lossy().ends_with(".log")))
        .collect();
    files.sort();
    let mut out: Vec<String> = Vec::new();
    for f in files.iter().rev() {
        let text = std::fs::read_to_string(f).unwrap_or_default();
        // An entry is its first line plus the indented lines after it.
        let mut entries: Vec<String> = Vec::new();
        for l in text.lines() {
            match entries.last_mut() {
                Some(e) if l.starts_with("    ") => {
                    e.push('\n');
                    e.push_str(l);
                }
                _ => entries.push(l.to_string()),
            }
        }
        let mut done = false;
        for e in entries.into_iter().rev() {
            if cutoff.as_ref().is_some_and(|c| e.as_str() < c.as_str()) {
                done = true;
                break;
            }
            let tag = e.split_once(" [").and_then(|(_, r)| r.split_once(']')).map_or("", |(a, _)| a);
            if (area.is_empty() || tag.starts_with(area)) && (needle.is_empty() || e.to_lowercase().contains(&needle)) {
                out.push(e);
                if out.len() >= limit {
                    done = true;
                    break;
                }
            }
        }
        if done {
            break;
        }
    }
    out.reverse();
    out
}

/// ("2026-10-18", "17:03:12") in UTC.
fn now() -> (String, String) {
    stamp(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs())
}

/// A UTC time in seconds since 1970 as ("2026-10-18", "17:03:12").
fn stamp(secs: u64) -> (String, String) {
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // Days since 1970-01-01 to a calendar date (Howard Hinnant's civil_from_days).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (format!("{y:04}-{m:02}-{d:02}"), format!("{:02}:{:02}:{:02}", rem / 3600, rem / 60 % 60, rem % 60))
}

#[cfg(test)]
mod tests {
    #[test]
    fn writes_and_prunes() {
        let _g = crate::config::TEST_DIRS.lock().unwrap_or_else(|e| e.into_inner());
        let data = std::env::temp_dir().join(format!("valerie-log-test-{}", crate::crypt::hex(&crate::crypt::random::<4>())));
        std::env::set_var("VALERIE_DATA_DIR", &data);
        std::fs::create_dir_all(super::dir()).unwrap();
        let stale = super::dir().join("valerie-2000-01-01.log");
        let f = std::fs::File::create(&stale).unwrap();
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 86_400)).unwrap();
        super::init(7);
        super::line("test", "first\nsecond");
        let (day, _) = super::now();
        let text = std::fs::read_to_string(super::dir().join(format!("valerie-{day}.log"))).unwrap_or_default();
        let pruned = !stale.exists();
        *super::STATE.lock().unwrap() = None;
        std::env::remove_var("VALERIE_DATA_DIR");
        let _ = std::fs::remove_dir_all(&data);
        assert!(text.contains("[test] first\n    second\n"), "{text:?}");
        assert!(pruned, "an 8-day-old log wasn't removed");
        assert_eq!(day.len(), 10);
    }

    #[test]
    fn reads_entries_by_area_and_text() {
        let _g = crate::config::TEST_DIRS.lock().unwrap_or_else(|e| e.into_inner());
        let data = std::env::temp_dir().join(format!("valerie-log-read-{}", crate::crypt::hex(&crate::crypt::random::<4>())));
        std::env::set_var("VALERIE_DATA_DIR", &data);
        std::fs::create_dir_all(super::dir()).unwrap();
        std::fs::write(super::dir().join("valerie-2026-10-17.log"), "2026-10-17T10:00:00Z [sync] old failure\n").unwrap();
        std::fs::write(super::dir().join("valerie-2026-10-18.log"),
            "2026-10-18T09:00:00Z [plugin:dark] TypeError: x\n    at line 3\n2026-10-18T09:01:00Z [mcp] click 12ms ok\n2026-10-18T09:02:00Z [plugin:dark] hello\n").unwrap();
        let plugin = super::read("plugin", "", None, 50);
        let errors = super::read("", "typeerror", None, 50);
        let last = super::read("", "", None, 2);
        let all = super::read("", "", None, 50);
        std::env::remove_var("VALERIE_DATA_DIR");
        let _ = std::fs::remove_dir_all(&data);
        assert_eq!(plugin.len(), 2, "{plugin:?}");
        assert_eq!(errors, vec!["2026-10-18T09:00:00Z [plugin:dark] TypeError: x\n    at line 3".to_string()]);
        assert_eq!(last.len(), 2);
        assert!(last[1].ends_with("hello"), "{last:?}");
        assert!(all[0].contains("old failure"), "oldest first: {all:?}");
    }
}
