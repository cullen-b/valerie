//! Valerie's log: what went wrong (and a few notable events), one file a day in the data folder's `logs`, so people
//! and agents can see why something failed after the fact. Files older than `logs.keep_days` are removed.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Days to keep (0 = no log files) and the day last written, to prune again when the date changes.
static STATE: Mutex<Option<(u64, String)>> = Mutex::new(None);

pub fn dir() -> PathBuf {
    crate::config::data_dir().join("logs")
}

/// Start logging to files (until this runs, lines only go to stderr, which keeps tests out of the real log).
pub fn init(keep_days: u64) {
    *STATE.lock().unwrap_or_else(|e| e.into_inner()) = Some((keep_days, String::new()));
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

/// ("2026-10-18", "17:03:12") in UTC.
fn now() -> (String, String) {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
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
}
