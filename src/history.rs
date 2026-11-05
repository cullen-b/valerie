//! The change history: every state of the config folder's settings, plugins and looks, whoever changed them, so any of
//! them can be put back exactly. Git's idea without needing git (not every computer has it): files are kept once by
//! their SHA-256, each entry names a list of files and the entry before it, and going back adds an entry, never removes one.
//! It lives in the data folder, on this computer only, because settings and plugins can hold private values.

use crate::config;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// What's kept, in the config folder. Not chats, keys or the files Valerie writes for itself (AGENTS.md, reference/).
const ITEMS: &[&str] = &["config.yaml", "this-mac.yaml", "NOTES.md", "plugins", "boosts", "ui", "strings", "pages", "layouts", "extensions"];
/// How long the reason given for an AI's task names the changes after it (one setting's reason names only the next one).
const REASON_MINUTES: u64 = 10;
const ONE_CHANGE_SECONDS: u64 = 5;

static ENABLED: AtomicBool = AtomicBool::new(true);
static KEEP: AtomicUsize = AtomicUsize::new(0);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Change {
    pub path: String,
    /// added | changed | removed
    pub how: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub parent: String,
    pub tree: String,
    pub at: u64,
    pub why: String,
    pub changed: Vec<Change>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct FileRef {
    hash: String,
    #[serde(default)]
    exec: bool,
}

type Tree = BTreeMap<String, FileRef>;

/// The `change_history` settings: whether changes are recorded, and how many to keep (0 = all of them).
pub fn configure(enabled: bool, keep: usize) {
    ENABLED.store(enabled, Ordering::Relaxed);
    KEEP.store(keep, Ordering::Relaxed);
}

fn dir() -> PathBuf {
    config::data_dir().join("changes")
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn object_path(hash: &str) -> PathBuf {
    dir().join("objects").join(&hash[..2]).join(&hash[2..])
}

/// Only one Valerie (the app or the `valerie changes` command) touches the history or puts files back at a time.
struct Lock(PathBuf);

impl Lock {
    fn take() -> Result<Lock, String> {
        let root = dir();
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700));
        }
        let path = root.join(".lock");
        for _ in 0..300 {
            if std::fs::OpenOptions::new().write(true).create_new(true).open(&path).is_ok() {
                return Ok(Lock(path));
            }
            // Left behind by a Valerie that stopped halfway.
            let stale = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|m| m.elapsed().ok()).is_some_and(|age| age.as_secs() > 60);
            if stale {
                let _ = std::fs::remove_file(&path);
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        Err("the change history is busy; try again in a moment".into())
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Every kept file now, by path from the config folder with `/` between parts. Links are left out and left alone.
fn live_files() -> Result<BTreeMap<String, PathBuf>, String> {
    fn walk(path: &Path, rel: String, out: &mut BTreeMap<String, PathBuf>) -> Result<(), String> {
        let Ok(meta) = std::fs::symlink_metadata(path) else { return Ok(()) };
        if meta.is_dir() {
            for e in std::fs::read_dir(path).map_err(|e| format!("{rel}: {e}"))? {
                let e = e.map_err(|e| format!("{rel}: {e}"))?;
                let name = e.file_name();
                let Some(name) = name.to_str() else {
                    crate::log::line("changes", format!("left out a file in {rel}: its name isn't valid text"));
                    continue;
                };
                // Version control, the Mac's folder notes, a layout's last error, and our own half-written files.
                if name == ".git" || name == ".DS_Store" || name.ends_with(".error") || name.starts_with(".valerie-restore-") {
                    continue;
                }
                walk(&e.path(), format!("{rel}/{name}"), out)?;
            }
        } else if meta.is_file() {
            out.insert(rel, path.to_path_buf());
        }
        Ok(())
    }
    let cfg = config::config_dir();
    let mut out = BTreeMap::new();
    for item in ITEMS {
        walk(&cfg.join(item), (*item).to_string(), &mut out)?;
    }
    Ok(out)
}

#[cfg(unix)]
fn is_exec(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

#[cfg(not(unix))]
fn is_exec(_: &Path) -> bool {
    false
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("a file has no folder")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = parent.join(format!(".valerie-restore-{}", crate::mcp::random_hex()));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

fn store(bytes: &[u8]) -> Result<String, String> {
    let hash = sha(bytes);
    let path = object_path(&hash);
    if !path.exists() {
        write_atomic(&path, bytes)?;
    }
    Ok(hash)
}

/// A kept file's bytes, refused if they aren't exactly what was saved.
fn load(hash: &str) -> Result<Vec<u8>, String> {
    if hash.len() < 3 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("the history has a bad entry".into());
    }
    let bytes = std::fs::read(object_path(hash)).map_err(|_| format!("a saved file is missing ({})", &hash[..12.min(hash.len())]))?;
    if sha(&bytes) != hash {
        return Err(format!("a saved file is damaged ({})", &hash[..12.min(hash.len())]));
    }
    Ok(bytes)
}

/// Save everything as it is now; the tree's id and contents. A file that can't be read fails the whole thing, so a
/// half-seen state is never recorded.
fn capture() -> Result<(String, Tree), String> {
    let mut tree = Tree::new();
    for (rel, path) in live_files()? {
        let bytes = std::fs::read(&path).map_err(|e| format!("{rel}: {e}"))?;
        tree.insert(rel, FileRef { hash: store(&bytes)?, exec: is_exec(&path) });
    }
    let json = serde_json::to_vec(&tree).map_err(|e| e.to_string())?;
    Ok((store(&json)?, tree))
}

fn read_tree(id: &str) -> Result<Tree, String> {
    serde_json::from_slice(&load(id)?).map_err(|_| "the history has a bad entry".to_string())
}

/// Every entry, oldest first. A last line cut short by a crash is skipped.
pub fn entries() -> Vec<Entry> {
    let text = std::fs::read_to_string(dir().join("log.jsonl")).unwrap_or_default();
    text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn append(entry: &Entry) -> Result<(), String> {
    use std::io::Write;
    let line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    let mut log = std::fs::OpenOptions::new().create(true).append(true).open(dir().join("log.jsonl")).map_err(|e| e.to_string())?;
    writeln!(log, "{line}").map_err(|e| e.to_string())
}

fn differences(from: &Tree, to: &Tree) -> Vec<Change> {
    let mut out = Vec::new();
    for (path, f) in to {
        match from.get(path) {
            None => out.push(Change { path: path.clone(), how: "added".into() }),
            Some(old) if old != f => out.push(Change { path: path.clone(), how: "changed".into() }),
            _ => {}
        }
    }
    out.extend(from.keys().filter(|p| !to.contains_key(*p)).map(|p| Change { path: p.clone(), how: "removed".into() }));
    out
}

fn make_entry(parent: &str, tree: &str, why: String, changed: Vec<Change>) -> Entry {
    let at = crate::state::now();
    let id = sha(format!("{parent}\n{tree}\n{at}\n{why}").as_bytes())[..12].to_string();
    Entry { id, parent: parent.to_string(), tree: tree.to_string(), at, why, changed }
}

/// The reason an AI gave just before changing things, if it's recent.
fn recent_reason() -> Option<String> {
    let path = dir().join("reason");
    let text = std::fs::read_to_string(&path).ok()?;
    let (at, why) = text.split_once('\n')?;
    let one_change = one_change_reason(why);
    let limit = if one_change { ONE_CHANGE_SECONDS } else { REASON_MINUTES * 60 };
    let fresh = crate::state::now().saturating_sub(at.parse().ok()?) <= limit;
    if one_change || !fresh {
        let _ = std::fs::remove_file(path);
    }
    fresh.then(|| match why.strip_prefix("set ") {
        Some(key) => format!("Changed the setting {key}"),
        None => why.to_string(),
    })
}

/// One setting or page, changed by a tool at once. Anything after it wasn't that tool's doing.
fn one_change_reason(why: &str) -> bool {
    why.starts_with("set ") || why.starts_with("page ")
}

fn plain_label(changed: &[Change]) -> String {
    match changed {
        [one] => format!("{} {}", capitalised(&one.how), one.path),
        many => format!("Changed {} files", many.len()),
    }
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// With the lock held: add an entry if things differ from the newest one.
fn record_locked(why: Option<&str>) -> Result<Option<Entry>, String> {
    let (tree_id, tree) = capture()?;
    let all = entries();
    let last = all.last();
    if last.is_some_and(|l| l.tree == tree_id) {
        return Ok(None);
    }
    let changed = match last {
        Some(l) => differences(&read_tree(&l.tree)?, &tree),
        None => Vec::new(),
    };
    let label = match (last, why) {
        (None, _) => "How things were when this history began".to_string(),
        (_, Some(w)) => w.to_string(),
        _ => recent_reason().unwrap_or_else(|| plain_label(&changed)),
    };
    let entry = make_entry(last.map_or("", |l| l.id.as_str()), &tree_id, label, changed);
    append(&entry)?;
    prune();
    Ok(Some(entry))
}

/// Remember how things are now, if that's new. Does nothing with `change_history.enabled` off.
pub fn record() -> Result<Option<Entry>, String> {
    if !ENABLED.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let _lock = Lock::take()?;
    record_locked(None)
}

/// An AI is about to change things: keep how they are, and name what comes next after `why`.
pub fn before_change(why: &str) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let result = Lock::take().and_then(|_lock| {
        record_locked(None)?;
        let one_line = why.split_whitespace().collect::<Vec<_>>().join(" ");
        std::fs::write(dir().join("reason"), format!("{}\n{one_line}", crate::state::now())).map_err(|e| e.to_string())
    });
    if let Err(e) = result {
        crate::log::line("changes", format!("couldn't record the change history: {e}"));
    }
}

fn drop_empty_parents(path: &Path) {
    let cfg = config::config_dir();
    let mut at = path.parent();
    while let Some(p) = at {
        if p == cfg || !p.starts_with(&cfg) || std::fs::remove_dir(p).is_err() {
            break;
        }
        at = p.parent();
    }
}

/// Make the kept files exactly `tree`. Every saved file is checked before anything is touched.
fn apply(tree: &Tree) -> Result<(), String> {
    let cfg = config::config_dir();
    let mut contents = BTreeMap::new();
    for (rel, f) in tree {
        let inside = !rel.is_empty() && !rel.starts_with('/') && rel.split('/').all(|p| !p.is_empty() && p != "." && p != "..") && !rel.contains('\\');
        if !inside {
            return Err(format!("the history names a file outside your settings: {rel}"));
        }
        contents.insert(rel, load(&f.hash)?);
    }
    for (rel, path) in live_files()? {
        if !tree.contains_key(&rel) {
            std::fs::remove_file(&path).map_err(|e| format!("{rel}: {e}"))?;
            drop_empty_parents(&path);
        }
    }
    for (rel, f) in tree {
        let path = cfg.join(rel);
        let bytes = &contents[rel];
        if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
            write_atomic(&path, bytes).map_err(|e| format!("{rel}: {e}"))?;
        }
        #[cfg(unix)]
        if is_exec(&path) != f.exec {
            use std::os::unix::fs::PermissionsExt;
            let mode = if f.exec { 0o755 } else { 0o644 };
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).map_err(|e| format!("{rel}: {e}"))?;
        }
        #[cfg(not(unix))]
        let _ = f;
    }
    Ok(())
}

fn find(all: &[Entry], id: &str) -> Result<Entry, String> {
    let id = id.trim();
    if id.len() < 4 {
        return Err("give a change's id (`valerie changes` lists them)".into());
    }
    let found: Vec<&Entry> = all.iter().filter(|e| e.id.starts_with(id)).collect();
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(format!("there's no change {id} (`valerie changes` lists them)")),
        _ => Err(format!("{id} could be several changes: give more of its id")),
    }
}

/// Put everything back exactly as it was at change `id`. How things are now is recorded first and nothing is removed
/// from the history, so this can be undone the same way. If it can't finish, what was there is put back.
pub fn go_back(id: &str) -> Result<Entry, String> {
    let _lock = Lock::take()?;
    let target = find(&entries(), id)?;
    record_locked(None)?;
    let all = entries();
    let now = all.last().ok_or("the history is empty")?.clone();
    if now.tree == target.tree {
        return Err("everything is already as it was at that change".into());
    }
    let (before, wanted) = (read_tree(&now.tree)?, read_tree(&target.tree)?);
    let done = apply(&wanted).and_then(|_| {
        let (tree_id, _) = capture()?;
        if tree_id == target.tree { Ok(()) } else { Err("the files didn't come out as they were saved".to_string()) }
    });
    if let Err(e) = done {
        let put_back = apply(&before).map_err(|again| format!("{e}; putting things back failed too: {again}"));
        return Err(put_back.err().unwrap_or(e));
    }
    let entry = make_entry(&now.id, &target.tree, format!("Went back to: {}", target.why), differences(&before, &wanted));
    append(&entry)?;
    crate::log::line("changes", format!("went back to {} ({})", target.id, target.why));
    prune();
    Ok(entry)
}

/// Past `change_history.keep`: forget the oldest entries and the files only they used.
fn prune() {
    let keep = KEEP.load(Ordering::Relaxed);
    let all = entries();
    if keep == 0 || all.len() <= keep {
        return;
    }
    let kept = &all[all.len() - keep..];
    let mut used = HashSet::new();
    for e in kept {
        let Ok(tree) = read_tree(&e.tree) else {
            crate::log::line("changes", format!("didn't tidy the history: {} can't be read", e.id));
            return;
        };
        used.insert(e.tree.clone());
        used.extend(tree.into_values().map(|f| f.hash));
    }
    let text: String = kept.iter().filter_map(|e| serde_json::to_string(e).ok()).map(|l| l + "\n").collect();
    if write_atomic(&dir().join("log.jsonl"), text.as_bytes()).is_err() {
        return;
    }
    for shelf in std::fs::read_dir(dir().join("objects")).into_iter().flatten().flatten() {
        let prefix = shelf.file_name().to_string_lossy().to_string();
        for file in std::fs::read_dir(shelf.path()).into_iter().flatten().flatten() {
            if !used.contains(&format!("{prefix}{}", file.file_name().to_string_lossy())) {
                let _ = std::fs::remove_file(file.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_dirs(f: impl FnOnce(&Path)) {
        let _turn = config::TEST_DIRS.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("valerie-changes-{}", crate::mcp::random_hex()));
        std::env::set_var("VALERIE_CONFIG_DIR", tmp.join("config"));
        std::env::set_var("VALERIE_DATA_DIR", tmp.join("data"));
        let cfg = config::config_dir();
        std::fs::create_dir_all(&cfg).unwrap();
        configure(true, 0);
        f(&cfg);
        configure(true, 0);
        std::env::remove_var("VALERIE_CONFIG_DIR");
        std::env::remove_var("VALERIE_DATA_DIR");
        let _ = std::fs::remove_dir_all(tmp);
    }

    /// Every kept file's path and bytes, to compare two states exactly.
    fn state() -> BTreeMap<String, Vec<u8>> {
        live_files().unwrap().into_iter().map(|(rel, p)| (rel, std::fs::read(p).unwrap())).collect()
    }

    #[test]
    fn going_back_restores_every_byte_and_can_itself_be_undone() {
        with_dirs(|cfg| {
            std::fs::write(cfg.join("config.yaml"), "a: 1\n").unwrap();
            std::fs::create_dir_all(cfg.join("plugins/mine")).unwrap();
            std::fs::write(cfg.join("plugins/mine/plugin.yaml"), [0u8, 159, 146, 150]).unwrap();
            let first = record().unwrap().unwrap();
            let original = state();
            assert!(record().unwrap().is_none(), "nothing changed, so nothing is added");

            std::fs::write(cfg.join("config.yaml"), "a: 2\n").unwrap();
            std::fs::remove_dir_all(cfg.join("plugins/mine")).unwrap();
            std::fs::create_dir_all(cfg.join("layouts")).unwrap();
            std::fs::write(cfg.join("layouts/wild.yaml"), "x").unwrap();
            let second = record().unwrap().unwrap();
            let changed = state();
            assert_eq!(second.parent, first.id);
            assert!(second.changed.contains(&Change { path: "layouts/wild.yaml".into(), how: "added".into() }));
            assert!(second.changed.contains(&Change { path: "plugins/mine/plugin.yaml".into(), how: "removed".into() }));

            let back = go_back(&first.id).unwrap();
            assert_eq!(state(), original);
            assert!(!cfg.join("layouts").exists(), "a folder that wasn't there isn't left behind empty");
            assert_eq!(back.tree, first.tree);
            assert_eq!(entries().len(), 3, "going back adds an entry and removes none");

            go_back(&second.id).unwrap();
            assert_eq!(state(), changed);
            assert_eq!(go_back(&second.id).unwrap_err(), "everything is already as it was at that change");
        });
    }

    #[test]
    fn unrecorded_edits_survive_going_back() {
        with_dirs(|cfg| {
            std::fs::write(cfg.join("config.yaml"), "a: 1\n").unwrap();
            let first = record().unwrap().unwrap();
            std::fs::write(cfg.join("config.yaml"), "edited by hand\n").unwrap();
            go_back(&first.id).unwrap();
            let all = entries();
            assert_eq!(all.len(), 3);
            go_back(&all[1].id).unwrap();
            assert_eq!(std::fs::read_to_string(cfg.join("config.yaml")).unwrap(), "edited by hand\n");
        });
    }

    #[test]
    fn damaged_history_changes_nothing() {
        with_dirs(|cfg| {
            std::fs::write(cfg.join("config.yaml"), "a: 1\n").unwrap();
            std::fs::write(cfg.join("NOTES.md"), "keep me\n").unwrap();
            let first = record().unwrap().unwrap();
            std::fs::write(cfg.join("config.yaml"), "a: 2\n").unwrap();
            record().unwrap().unwrap();
            let before = state();
            std::fs::write(object_path(&sha(b"a: 1\n")), "tampered").unwrap();
            assert!(go_back(&first.id).unwrap_err().contains("damaged"));
            assert_eq!(state(), before);
        });
    }

    #[test]
    fn an_ai_reason_names_what_it_changed() {
        with_dirs(|cfg| {
            std::fs::write(cfg.join("config.yaml"), "a: 1\n").unwrap();
            before_change("Customize: make it blue");
            std::fs::write(cfg.join("config.yaml"), "a: 2\n").unwrap();
            assert_eq!(record().unwrap().unwrap().why, "Customize: make it blue");
        });
    }

    #[test]
    fn one_settings_reason_names_only_that_change() {
        with_dirs(|cfg| {
            std::fs::write(cfg.join("config.yaml"), "a: 1\n").unwrap();
            before_change("set sidebar.tab_height");
            std::fs::write(cfg.join("config.yaml"), "a: 2\n").unwrap();
            assert_eq!(record().unwrap().unwrap().why, "Changed the setting sidebar.tab_height");
            std::fs::write(cfg.join("NOTES.md"), "someone else\n").unwrap();
            assert_eq!(record().unwrap().unwrap().why, "Added NOTES.md");
        });
    }

    #[test]
    fn keeping_fewer_forgets_the_oldest_and_their_files() {
        with_dirs(|cfg| {
            configure(true, 2);
            for i in 0..4 {
                std::fs::write(cfg.join("config.yaml"), format!("a: {i}\n")).unwrap();
                record().unwrap().unwrap();
            }
            let all = entries();
            assert_eq!(all.len(), 2);
            assert!(!object_path(&sha(b"a: 0\n")).exists());
            assert!(object_path(&sha(b"a: 2\n")).exists());
            go_back(&all[0].id).unwrap();
            assert_eq!(std::fs::read_to_string(cfg.join("config.yaml")).unwrap(), "a: 2\n");
        });
    }

    #[test]
    fn switched_off_records_nothing() {
        with_dirs(|cfg| {
            configure(false, 0);
            std::fs::write(cfg.join("config.yaml"), "a: 1\n").unwrap();
            assert!(record().unwrap().is_none());
            assert!(entries().is_empty());
        });
    }

    #[test]
    fn its_own_and_generated_files_are_left_out() {
        with_dirs(|cfg| {
            std::fs::create_dir_all(cfg.join("layouts")).unwrap();
            std::fs::write(cfg.join("layouts/a.yaml"), "x").unwrap();
            std::fs::write(cfg.join("layouts/a.error"), "bad").unwrap();
            std::fs::write(cfg.join("AGENTS.md"), "guide").unwrap();
            assert_eq!(state().keys().cloned().collect::<Vec<_>>(), vec!["layouts/a.yaml".to_string()]);
        });
    }
}
