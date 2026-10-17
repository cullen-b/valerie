use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SOURCES: [&str; 8] = [
    "arc", "chrome", "brave", "edge", "vivaldi", "chromium", "safari", "firefox",
];
const MAX_DEPTH: usize = 64;
const ARC_SIDEBAR: &str = "Library/Application Support/Arc/StorableSidebar.json";
const SAFARI_BOOKMARKS: &str = "Library/Safari/Bookmarks.plist";
const FIREFOX_PROFILES: &str = "Library/Application Support/Firefox/Profiles";
const FIREFOX_QUERY: &str = "SELECT b.id AS id, b.parent AS parent, b.type AS kind, \
    b.title AS title, p.url AS url FROM moz_bookmarks b \
    LEFT JOIN moz_places p ON p.id = b.fk;";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportedTab {
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImportedNode {
    Tab(ImportedTab),
    Folder {
        title: String,
        children: Vec<ImportedNode>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportedSpace {
    pub title: String,
    pub pinned: Vec<ImportedNode>,
    pub tabs: Vec<ImportedTab>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Bookmark {
    pub title: String,
    pub url: String,
    pub folder: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Imported {
    pub spaces: Vec<ImportedSpace>,
    pub favorites: Vec<ImportedNode>,
    pub bookmarks: Vec<Bookmark>,
}

/// Sources whose data exists on this machine.
pub fn available() -> Vec<&'static str> {
    SOURCES.into_iter().filter(|s| locate(s).is_some()).collect()
}

pub fn import(source: &str) -> Result<Imported> {
    if !SOURCES.contains(&source) {
        bail!("unknown import source: {source}");
    }
    let path = locate(source).ok_or_else(|| anyhow!("no {source} data found"))?;
    match source {
        "arc" => arc(&path),
        "safari" => safari(&path),
        "firefox" => firefox(&path),
        _ => chromium(&path),
    }
}

/// The browsers on this computer Valerie can import from.
pub fn found() -> Vec<&'static str> {
    SOURCES.iter().copied().filter(|s| locate(s).is_some()).collect()
}

fn locate(source: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let path = match source {
        "arc" => home.join(ARC_SIDEBAR),
        "safari" => home.join(SAFARI_BOOKMARKS),
        "firefox" => return firefox_places(&home.join(FIREFOX_PROFILES)),
        "chrome" => home.join("Library/Application Support/Google/Chrome/Default/Bookmarks"),
        "brave" => {
            home.join("Library/Application Support/BraveSoftware/Brave-Browser/Default/Bookmarks")
        }
        "edge" => home.join("Library/Application Support/Microsoft Edge/Default/Bookmarks"),
        "vivaldi" => home.join("Library/Application Support/Vivaldi/Default/Bookmarks"),
        "chromium" => home.join("Library/Application Support/Chromium/Default/Bookmarks"),
        _ => return None,
    };
    path.is_file().then_some(path)
}

fn firefox_places(profiles: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(profiles)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path().join("places.sqlite"))
        .filter(|p| p.is_file())
        .collect();
    found.sort();
    let chosen = found
        .iter()
        .find(|p| p.to_string_lossy().contains("default-release"))
        .or_else(|| found.first())
        .cloned();
    chosen
}

fn read_json(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

/// Splits a Swift-encoded flat array of alternating keys and values.
fn flat_pairs(v: Option<&Value>) -> Vec<(&str, &Value)> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.chunks(2)
                .filter_map(|c| match c {
                    [k, o] => Some((k.as_str()?, o)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn arc(path: &Path) -> Result<Imported> {
    let root = read_json(path).context("reading Arc sidebar")?;
    let containers = root
        .pointer("/sidebar/containers")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Arc sidebar has no containers"))?;
    let c = containers
        .iter()
        .find(|c| c.get("spaces").is_some())
        .ok_or_else(|| anyhow!("Arc sidebar has no space container"))?;

    let mut w = Walker {
        items: flat_pairs(c.get("items")).into_iter().collect(),
        seen: HashSet::new(),
    };
    let mut out = Imported::default();

    for id in c
        .get("topAppsContainerIDs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        out.favorites.extend(w.expand(id, 0));
    }

    for (_, space) in flat_pairs(c.get("spaces")) {
        let mut pinned_roots = Vec::new();
        let mut unpinned_roots = Vec::new();
        let mut section: Option<bool> = None;
        for tok in space
            .get("containerIDs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            match tok {
                "pinned" => section = Some(true),
                "unpinned" => section = Some(false),
                id => match section {
                    Some(true) => pinned_roots.push(id),
                    Some(false) => unpinned_roots.push(id),
                    None => {}
                },
            }
        }

        let mut pinned = Vec::new();
        for id in pinned_roots {
            pinned.extend(w.expand(id, 0));
        }
        let mut unpinned = Vec::new();
        for id in unpinned_roots {
            unpinned.extend(w.expand(id, 0));
        }
        let mut tabs = Vec::new();
        flatten_tabs(&unpinned, &mut tabs);

        out.spaces.push(ImportedSpace {
            title: str_field(space, "title"),
            pinned,
            tabs,
        });
    }
    Ok(out)
}

struct Walker<'a> {
    items: HashMap<&'a str, &'a Value>,
    seen: HashSet<&'a str>,
}

impl<'a> Walker<'a> {
    fn expand(&mut self, id: &'a str, depth: usize) -> Vec<ImportedNode> {
        if depth > MAX_DEPTH || !self.seen.insert(id) {
            return Vec::new();
        }
        let Some(&item) = self.items.get(id) else {
            return Vec::new();
        };
        let Some(data) = item.get("data") else {
            return Vec::new();
        };
        if let Some(tab) = data.get("tab") {
            let url = str_field(tab, "savedURL");
            if url.is_empty() {
                return Vec::new();
            }
            let title = non_empty(str_field(item, "title"))
                .unwrap_or_else(|| str_field(tab, "savedTitle"));
            return vec![ImportedNode::Tab(ImportedTab { title, url })];
        }
        if data.get("list").is_some() {
            let children = self.kids(item, depth);
            return vec![ImportedNode::Folder {
                title: str_field(item, "title"),
                children,
            }];
        }
        if data.get("splitView").is_some() || data.get("itemContainer").is_some() {
            return self.kids(item, depth);
        }
        Vec::new()
    }

    fn kids(&mut self, item: &'a Value, depth: usize) -> Vec<ImportedNode> {
        let mut out = Vec::new();
        for child in item
            .get("childrenIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            out.extend(self.expand(child, depth + 1));
        }
        out
    }
}

fn flatten_tabs(nodes: &[ImportedNode], out: &mut Vec<ImportedTab>) {
    for node in nodes {
        match node {
            ImportedNode::Tab(t) => out.push(t.clone()),
            ImportedNode::Folder { children, .. } => flatten_tabs(children, out),
        }
    }
}

fn chromium(path: &Path) -> Result<Imported> {
    let root = read_json(path).context("reading bookmarks file")?;
    let roots = root
        .get("roots")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("bookmarks file has no roots"))?;
    let mut out = Imported::default();
    for key in ["bookmark_bar", "other", "synced"] {
        if let Some(node) = roots.get(key) {
            chromium_walk(node, &mut Vec::new(), &mut out.bookmarks, 0);
        }
    }
    Ok(out)
}

fn chromium_walk(node: &Value, folder: &mut Vec<String>, out: &mut Vec<Bookmark>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match node.get("type").and_then(Value::as_str) {
        Some("url") => {
            let url = str_field(node, "url");
            if !url.is_empty() {
                out.push(Bookmark {
                    title: str_field(node, "name"),
                    url,
                    folder: folder.clone(),
                });
            }
        }
        Some("folder") => {
            let name = str_field(node, "name");
            let pushed = !name.is_empty();
            if pushed {
                folder.push(name);
            }
            for child in node.get("children").and_then(Value::as_array).into_iter().flatten() {
                chromium_walk(child, folder, out, depth + 1);
            }
            if pushed {
                folder.pop();
            }
        }
        _ => {}
    }
}

fn safari(path: &Path) -> Result<Imported> {
    let root: plist::Value = plist::from_file(path).context("reading Safari bookmarks")?;
    let mut out = Imported::default();
    safari_walk(&root, &mut Vec::new(), &mut out.bookmarks, 0);
    Ok(out)
}

fn safari_walk(node: &plist::Value, folder: &mut Vec<String>, out: &mut Vec<Bookmark>, depth: usize) {
    let Some(dict) = node.as_dictionary() else {
        return;
    };
    if depth > MAX_DEPTH {
        return;
    }
    let text = |key: &str| {
        dict.get(key)
            .and_then(plist::Value::as_string)
            .unwrap_or_default()
            .to_owned()
    };
    match dict.get("WebBookmarkType").and_then(plist::Value::as_string) {
        Some("WebBookmarkTypeLeaf") => {
            let url = text("URLString");
            let title = dict
                .get("URIDictionary")
                .and_then(plist::Value::as_dictionary)
                .and_then(|d| d.get("title"))
                .and_then(plist::Value::as_string)
                .unwrap_or_default()
                .to_owned();
            if !url.is_empty() {
                out.push(Bookmark {
                    title,
                    url,
                    folder: folder.clone(),
                });
            }
        }
        Some("WebBookmarkTypeList") => {
            let title = text("Title");
            let name = if title == "com.apple.ReadingList" {
                "Reading List".to_owned()
            } else {
                title
            };
            let pushed = !name.is_empty();
            if pushed {
                folder.push(name);
            }
            if let Some(children) = dict.get("Children").and_then(plist::Value::as_array) {
                for child in children {
                    safari_walk(child, folder, out, depth + 1);
                }
            }
            if pushed {
                folder.pop();
            }
        }
        _ => {}
    }
}

#[derive(Deserialize)]
struct PlacesRow {
    id: i64,
    parent: Option<i64>,
    kind: i64,
    title: Option<String>,
    url: Option<String>,
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn firefox(db: &Path) -> Result<Imported> {
    let rows = query_places_copy(db)?;
    let by_id: HashMap<i64, &PlacesRow> = rows.iter().map(|r| (r.id, r)).collect();
    let mut out = Imported::default();
    for row in rows.iter().filter(|r| r.kind == 1) {
        let Some(url) = row
            .url
            .as_deref()
            .filter(|u| !u.is_empty() && !u.starts_with("place:"))
        else {
            continue;
        };
        out.bookmarks.push(Bookmark {
            title: row.title.clone().unwrap_or_default(),
            url: url.to_owned(),
            folder: firefox_folder(row.parent, &by_id),
        });
    }
    Ok(out)
}

fn firefox_folder(mut id: Option<i64>, by_id: &HashMap<i64, &PlacesRow>) -> Vec<String> {
    let mut parts = Vec::new();
    for _ in 0..MAX_DEPTH {
        let Some(row) = id.and_then(|i| by_id.get(&i)) else {
            break;
        };
        if row.kind == 2 {
            if let Some(t) = row.title.as_deref().filter(|t| !t.is_empty()) {
                parts.push(t.to_owned());
            }
        }
        id = row.parent;
    }
    parts.reverse();
    parts
}

/// Copies places.sqlite (and its WAL) to a temp dir so Firefox's lock does not matter.
fn query_places_copy(db: &Path) -> Result<Vec<PlacesRow>> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("valerie-import-{}-{nanos}", std::process::id()));
    let _guard = TempDir(dir.clone());
    fs::create_dir_all(&dir).context("creating temp dir")?;
    let copy = dir.join("places.sqlite");
    fs::copy(db, &copy).context("copying Firefox places database")?;
    let mut wal = db.as_os_str().to_owned();
    wal.push("-wal");
    let wal = PathBuf::from(wal);
    if wal.is_file() {
        fs::copy(&wal, dir.join("places.sqlite-wal")).context("copying Firefox WAL")?;
    }
    let output = Command::new("/usr/bin/sqlite3")
        .arg("-json")
        .arg(&copy)
        .arg(FIREFOX_QUERY)
        .output()
        .context("running sqlite3")?;
    if !output.status.success() {
        bail!("sqlite3 failed reading Firefox database");
    }
    if output.stdout.iter().all(u8::is_ascii_whitespace) {
        return Ok(Vec::new());
    }
    serde_json::from_slice(&output.stdout).context("parsing sqlite3 output")
}
