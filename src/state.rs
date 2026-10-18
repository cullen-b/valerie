use crate::command::TabId;
use crate::config::Config;
use crate::import::{Bookmark, ImportedNode, ImportedTab, Imported};
use crate::sync::{DeviceTabs, SyncNode};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tab {
    pub id: TabId,
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub custom_title: Option<String>,
    #[serde(default)]
    pub favicon: Option<String>,
    /// Where a pinned tab "lives" even after you browse away inside it; this is what syncs.
    #[serde(default)]
    pub pinned_url: Option<String>,
    #[serde(default)]
    pub last_active: u64,
    /// When the user chose to keep it after being asked about a saved tab they hadn't opened in a long while.
    #[serde(default)]
    pub kept_at: u64,
    #[serde(default, skip)]
    pub playing: bool,
    #[serde(default, skip)]
    pub loading: bool,
    /// Muted from Valerie (sidebar ♪, toggle_mute).
    #[serde(default, skip)]
    pub muted: bool,
    /// The tab's last-played video/audio: "playing" | "paused" (drives the sidebar mini player).
    #[serde(default, skip)]
    pub media: Option<String>,
}

impl Tab {
    pub fn display_title(&self) -> &str {
        self.custom_title
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(Some(self.title.as_str()).filter(|s| !s.is_empty()))
            .unwrap_or(&self.url)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Node {
    Tab {
        id: TabId,
    },
    Folder {
        id: TabId,
        name: String,
        #[serde(default)]
        open: bool,
        #[serde(default)]
        children: Vec<Node>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Space {
    pub name: String,
    #[serde(default)]
    pub pinned: Vec<Node>,
    /// "Kept" tabs: a list under the pinned icons that stays until you remove it (never auto-archived).
    #[serde(default)]
    pub kept: Vec<Node>,
    #[serde(default)]
    pub today: Vec<TabId>,
    #[serde(default)]
    pub active: Option<TabId>,
    #[serde(default)]
    pub split: Vec<TabId>,
    #[serde(default)]
    pub theme: Option<String>,
    /// Last local edit of the pinned tree (0 = never edited here, so it never overrides another device).
    #[serde(default)]
    pub pinned_modified: u64,
    #[serde(default)]
    pub pinned_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Archived {
    pub url: String,
    pub title: String,
    pub space: String,
    pub archived_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Closed {
    pub tab: Tab,
    pub space: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watch {
    pub id: String,
    pub url: String,
    pub title: String,
    pub condition: String,
    #[serde(default)]
    pub last: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Download {
    pub name: String,
    pub path: String,
    pub url: String,
    pub at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    pub visits: u32,
    pub last: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub next_id: TabId,
    #[serde(default)]
    pub tabs: BTreeMap<TabId, Tab>,
    #[serde(default)]
    pub spaces: Vec<Space>,
    #[serde(default)]
    pub favorites: Vec<TabId>,
    #[serde(default)]
    pub active_space: usize,
    #[serde(default)]
    pub archive: Vec<Archived>,
    #[serde(default)]
    pub closed: Vec<Closed>,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
    /// Last run time of each scheduled agent job.
    #[serde(default)]
    pub agent_last: BTreeMap<String, u64>,
    /// Sites where the user switched content blocking off.
    #[serde(default)]
    pub blocking_allow: Vec<String>,
    /// Origins where the user chose "Never" save passwords.
    #[serde(default)]
    pub password_never: Vec<String>,
    /// Sidebar width the user dragged to (overrides layout.sidebar_size until they pick a size again).
    #[serde(default)]
    pub sidebar_width: Option<f64>,
    /// Where the window was last (x, y, width, height in points), to reopen it there.
    #[serde(default)]
    pub window_frame: Option<(f64, f64, f64, f64)>,
    #[serde(default)]
    pub favorites_modified: u64,
    #[serde(default)]
    pub favorites_hash: Option<String>,
    /// Zoom level per site (host), when tabs.zoom is "site".
    #[serde(default)]
    pub site_zoom: BTreeMap<String, f64>,
    /// Sites where the user turned Valerie's dark mode on (true) or off (false), overriding pages.dark_mode.
    #[serde(default)]
    pub dark_sites: BTreeMap<String, bool>,
    /// How much history, archive and closed tabs to keep (from the `history` settings; not saved).
    #[serde(skip)]
    pub limits: Limits,
    /// The date each task last ran ("2026-10-16").
    #[serde(default, alias = "brief_last")]
    pub task_last: BTreeMap<String, String>,
    /// When Valerie last looked for an update (unix seconds).
    #[serde(default)]
    pub update_checked: u64,
    /// The welcome guide was finished or skipped.
    #[serde(default)]
    pub welcome_done: bool,
    /// Answers to "Allow this site to send notifications?" (site → allowed).
    #[serde(default)]
    pub notify_sites: BTreeMap<String, bool>,
    /// Pages an agent checks in the background (Watch This Page…).
    #[serde(default)]
    pub watches: Vec<Watch>,
    /// Finished downloads, newest first.
    #[serde(default)]
    pub downloads: Vec<Download>,
    /// Browser tools each plugin's page script was allowed to use (plugin → its `permissions` when approved).
    #[serde(default)]
    pub plugin_grants: BTreeMap<String, Vec<String>>,
    /// Browser extensions the user allowed (folder name → fingerprint of the manifest they allowed).
    #[serde(default)]
    pub extension_grants: BTreeMap<String, String>,
}


/// How many history pages, archived tabs and closed tabs to keep.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub history: usize,
    pub archive: usize,
    pub closed: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { history: 5000, archive: 1000, closed: 50 }
    }
}

/// Every tab id in a tree, folders included.
pub fn flat_ids(nodes: &[Node]) -> Vec<TabId> {
    let mut out = Vec::new();
    walk(nodes, &mut out, false);
    out
}

fn walk(nodes: &[Node], out: &mut Vec<TabId>, only_open: bool) {
    for n in nodes {
        match n {
            Node::Tab { id } => out.push(*id),
            Node::Folder { children, open, .. } => {
                if *open || !only_open {
                    walk(children, out, only_open)
                }
            }
        }
    }
}

fn remove_from(nodes: &mut Vec<Node>, tab: TabId) -> bool {
    if let Some(i) = nodes.iter().position(|n| matches!(n, Node::Tab { id } if *id == tab)) {
        nodes.remove(i);
        return true;
    }
    nodes.iter_mut().any(|n| match n {
        Node::Folder { children, .. } => remove_from(children, tab),
        _ => false,
    })
}

pub fn find_folder(nodes: &mut [Node], folder: TabId) -> Option<&mut Node> {
    for n in nodes.iter_mut() {
        if matches!(n, Node::Folder { id, .. } if *id == folder) {
            return Some(n);
        }
        if let Node::Folder { children, .. } = n {
            if let Some(f) = find_folder(children, folder) {
                return Some(f);
            }
        }
    }
    None
}

/// Remove a folder, returning it so its tabs can be relocated.
fn take_folder(nodes: &mut Vec<Node>, folder: TabId) -> Option<Node> {
    if let Some(i) = nodes.iter().position(|n| matches!(n, Node::Folder { id, .. } if *id == folder)) {
        return Some(nodes.remove(i));
    }
    nodes.iter_mut().find_map(|n| match n {
        Node::Folder { children, .. } => take_folder(children, folder),
        _ => None,
    })
}

impl State {
    pub fn load(path: &Path) -> State {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn new_id(&mut self) -> TabId {
        self.next_id += 1;
        self.next_id
    }

    /// Make sure every configured space exists, in config order, followed by spaces created at runtime.
    pub fn reconcile_spaces(&mut self, config: &Config) {
        let active = self.spaces.get(self.active_space).map(|s| s.name.clone());
        let mut ordered: Vec<Space> = Vec::new();
        for sc in &config.spaces {
            let s = match self.spaces.iter().position(|s| s.name == sc.name) {
                Some(i) => self.spaces.remove(i),
                None => Space { name: sc.name.clone(), ..Default::default() },
            };
            ordered.push(s);
        }
        ordered.append(&mut self.spaces);
        if ordered.is_empty() {
            ordered.push(Space { name: "Home".into(), ..Default::default() });
        }
        self.spaces = ordered;
        self.active_space = active
            .and_then(|n| self.spaces.iter().position(|s| s.name == n))
            .unwrap_or(0);
        // Drop tab records nothing refers to anymore.
        let mut live: Vec<TabId> = self.favorites.clone();
        for s in &self.spaces {
            walk(&s.pinned, &mut live, false);
            walk(&s.kept, &mut live, false);
            live.extend(&s.today);
        }
        self.tabs.retain(|id, _| live.contains(id));
    }

    pub fn space(&self) -> &Space {
        &self.spaces[self.active_space]
    }

    pub fn space_mut(&mut self) -> &mut Space {
        &mut self.spaces[self.active_space]
    }

    pub fn space_index(&self, name: &str) -> Option<usize> {
        self.spaces.iter().position(|s| s.name.eq_ignore_ascii_case(name))
    }

    pub fn active_tab(&self) -> Option<TabId> {
        self.space().active.filter(|t| self.tabs.contains_key(t))
    }

    /// Tabs in sidebar order for one space: pinned, kept (inside open folders), then today.
    pub fn order(&self, space: usize, only_open: bool) -> Vec<TabId> {
        let s = &self.spaces[space];
        let mut v = Vec::new();
        walk(&s.pinned, &mut v, only_open);
        walk(&s.kept, &mut v, only_open);
        v.extend(&s.today);
        v
    }

    /// Pinned or kept: tabs that stay in the sidebar until you remove them (closing only unloads them).
    pub fn is_pinned(&self, tab: TabId) -> bool {
        self.section_of(tab).is_some_and(|s| s == "pinned" || s == "kept")
    }

    /// Which part of the sidebar holds a tab: "favorites", "pinned", "kept" or "today".
    pub fn section_of(&self, tab: TabId) -> Option<&'static str> {
        if self.favorites.contains(&tab) {
            return Some("favorites");
        }
        self.spaces.iter().find_map(|s| {
            let (mut p, mut k) = (Vec::new(), Vec::new());
            walk(&s.pinned, &mut p, false);
            walk(&s.kept, &mut k, false);
            if p.contains(&tab) {
                Some("pinned")
            } else if k.contains(&tab) {
                Some("kept")
            } else if s.today.contains(&tab) {
                Some("today")
            } else {
                None
            }
        })
    }

    /// A folder in either the pinned or kept section of any space.
    pub fn folder_mut(&mut self, folder: TabId) -> Option<&mut Node> {
        for s in &mut self.spaces {
            if let Some(f) = find_folder(&mut s.pinned, folder) {
                return Some(f);
            }
            if let Some(f) = find_folder(&mut s.kept, folder) {
                return Some(f);
            }
        }
        None
    }

    /// The space that owns a tab. Favorites belong to no space.
    pub fn space_of(&self, tab: TabId) -> Option<usize> {
        (0..self.spaces.len()).find(|&i| self.order(i, false).contains(&tab))
    }

    pub fn add_tab(&mut self, space: usize, url: &str, at_top: bool) -> TabId {
        let id = self.new_id();
        self.tabs.insert(
            id,
            Tab { id, url: url.to_string(), last_active: now(), ..Default::default() },
        );
        let today = &mut self.spaces[space].today;
        if at_top {
            today.insert(0, id);
        } else {
            today.push(id);
        }
        id
    }

    /// Detach a tab from favorites, pinned trees and today lists (the Tab record stays).
    pub fn detach(&mut self, tab: TabId) {
        self.favorites.retain(|t| *t != tab);
        for s in &mut self.spaces {
            remove_from(&mut s.pinned, tab);
            remove_from(&mut s.kept, tab);
            s.today.retain(|t| *t != tab);
            s.split.retain(|t| *t != tab);
            if s.active == Some(tab) {
                s.active = None;
            }
        }
    }

    /// Close a tab, remembering it for reopen. Returns the tab that should become active in its space.
    pub fn close_tab(&mut self, tab: TabId) -> Option<TabId> {
        let space = self.space_of(tab).unwrap_or(self.active_space);
        let order = self.order(space, true);
        let pos = order.iter().position(|t| *t == tab);
        let was_active = self.spaces[space].active == Some(tab);
        self.detach(tab);
        if let Some(t) = self.tabs.remove(&tab) {
            self.closed.push(Closed { tab: t, space });
            if self.closed.len() > self.limits.closed {
                self.closed.remove(0);
            }
        }
        if !was_active {
            return self.spaces[space].active;
        }
        let order = self.order(space, true);
        let next = pos.and_then(|p| order.get(p).or_else(|| order.last()).copied());
        self.spaces[space].active = next;
        next
    }

    /// Tabs kept in the sidebar: favorites, pinned and saved ("kept"), in every space.
    pub fn saved_tabs(&self) -> Vec<TabId> {
        let mut v = self.favorites.clone();
        for s in &self.spaces {
            v.extend(flat_ids(&s.pinned));
            v.extend(flat_ids(&s.kept));
        }
        v
    }

    /// Saved tabs not opened (or kept when asked) for `days`. A tab from before Valerie tracked this starts counting
    /// now rather than looking ancient.
    pub fn unused_saved(&mut self, days: f64, now: u64) -> Vec<TabId> {
        let limit = (days * 86400.0) as u64;
        let mut out = Vec::new();
        for id in self.saved_tabs() {
            let Some(t) = self.tabs.get_mut(&id) else { continue };
            if t.last_active == 0 {
                t.last_active = now;
                continue;
            }
            if limit > 0 && now.saturating_sub(t.last_active.max(t.kept_at)) > limit {
                out.push(id);
            }
        }
        out
    }

    pub fn archive_tab(&mut self, tab: TabId) {
        let space = self.space_of(tab).map(|i| self.spaces[i].name.clone()).unwrap_or_default();
        self.detach(tab);
        if let Some(t) = self.tabs.remove(&tab) {
            self.archive.insert(
                0,
                Archived { url: t.url.clone(), title: t.display_title().to_string(), space, archived_at: now() },
            );
            self.archive.truncate(self.limits.archive);
        }
    }

    pub fn pin(&mut self, tab: TabId) {
        let space = self.space_of(tab).unwrap_or(self.active_space);
        let active = self.spaces[space].active;
        self.detach(tab);
        self.spaces[space].pinned.push(Node::Tab { id: tab });
        self.spaces[space].active = active;
    }

    pub fn unpin(&mut self, tab: TabId) {
        let space = self.space_of(tab).unwrap_or(self.active_space);
        let active = self.spaces[space].active;
        self.detach(tab);
        self.spaces[space].today.insert(0, tab);
        self.spaces[space].active = active;
    }

    pub fn delete_folder(&mut self, folder: TabId) {
        for s in &mut self.spaces {
            if let Some(Node::Folder { children, .. }) = take_folder(&mut s.pinned, folder) {
                s.pinned.extend(children);
                return;
            }
            if let Some(Node::Folder { children, .. }) = take_folder(&mut s.kept, folder) {
                s.kept.extend(children);
                return;
            }
        }
    }

    pub fn record_visit(&mut self, url: &str, title: &str) {
        if url.is_empty() || url.starts_with("about:") {
            return;
        }
        if let Some(i) = self.history.iter().position(|h| h.url == url) {
            let mut h = self.history.remove(i);
            h.visits += 1;
            h.last = now();
            if !title.is_empty() {
                h.title = title.to_string();
            }
            self.history.insert(0, h);
        } else {
            self.history.insert(
                0,
                HistoryEntry { url: url.into(), title: title.into(), visits: 1, last: now() },
            );
            self.history.truncate(self.limits.history);
        }
    }

    pub fn update_history_title(&mut self, url: &str, title: &str) {
        if let Some(h) = self.history.iter_mut().find(|h| h.url == url) {
            h.title = title.to_string();
        }
    }

    /// Merge imported data: spaces with matching names get their pinned/today tabs appended, new spaces are added.
    pub fn apply_import(&mut self, source: &str, imp: Imported) -> String {
        let mut n_tabs = 0;
        for (i, isp) in imp.spaces.into_iter().enumerate() {
            let name = if isp.title.trim().is_empty() { format!("{source} {}", i + 1) } else { isp.title.clone() };
            let idx = match self.space_index(&name) {
                Some(i) => i,
                None => {
                    self.spaces.push(Space { name, ..Default::default() });
                    self.spaces.len() - 1
                }
            };
            let pinned = self.import_nodes(&isp.pinned, &mut n_tabs);
            self.spaces[idx].pinned.extend(pinned);
            for t in &isp.tabs {
                let id = self.import_tab(t);
                n_tabs += 1;
                self.spaces[idx].today.push(id);
            }
        }
        let mut favs = Vec::new();
        flatten(&imp.favorites, &mut favs);
        for t in &favs {
            if self.favorites.iter().any(|f| self.tabs.get(f).is_some_and(|x| x.url == t.url)) {
                continue;
            }
            let id = self.import_tab(t);
            n_tabs += 1;
            self.favorites.push(id);
        }
        let n_bm = imp.bookmarks.len();
        for b in imp.bookmarks {
            if !self.bookmarks.iter().any(|x| x.url == b.url) {
                self.bookmarks.push(b);
            }
        }
        format!("Imported {n_tabs} tabs and {n_bm} bookmarks from {source}")
    }

    fn import_tab(&mut self, t: &ImportedTab) -> TabId {
        let id = self.new_id();
        self.tabs.insert(
            id,
            Tab { id, url: t.url.clone(), title: t.title.clone(), last_active: now(), ..Default::default() },
        );
        id
    }

    fn import_nodes(&mut self, nodes: &[ImportedNode], n: &mut usize) -> Vec<Node> {
        nodes
            .iter()
            .map(|node| match node {
                ImportedNode::Tab(t) => {
                    *n += 1;
                    Node::Tab { id: self.import_tab(t) }
                }
                ImportedNode::Folder { title, children } => {
                    let id = self.new_id();
                    let children = self.import_nodes(children, n);
                    Node::Folder { id, name: title.clone(), open: false, children }
                }
            })
            .collect()
    }
}

fn flatten(nodes: &[ImportedNode], out: &mut Vec<ImportedTab>) {
    for n in nodes {
        match n {
            ImportedNode::Tab(t) => out.push(t.clone()),
            ImportedNode::Folder { children, .. } => flatten(children, out),
        }
    }
}

fn tree_hash(nodes: &[SyncNode]) -> String {
    // Titles drift as pages load, so only structure, URLs and custom names count as edits.
    fn strip(n: &SyncNode) -> serde_json::Value {
        match n {
            SyncNode::Tab { url, custom_title, .. } => serde_json::json!([url, custom_title]),
            SyncNode::Folder { name, children } => serde_json::json!([name, children.iter().map(strip).collect::<Vec<_>>()]),
        }
    }
    crate::blocking::hash(&serde_json::Value::Array(nodes.iter().map(strip).collect()).to_string())
}

impl State {
    fn sync_tab(&self, id: TabId) -> Option<SyncNode> {
        let t = self.tabs.get(&id)?;
        Some(SyncNode::Tab {
            title: t.title.clone(),
            url: t.pinned_url.clone().unwrap_or_else(|| t.url.clone()),
            custom_title: t.custom_title.clone(),
        })
    }

    pub fn sync_tree(&self, nodes: &[Node]) -> Vec<SyncNode> {
        nodes
            .iter()
            .filter_map(|n| match n {
                Node::Tab { id } => self.sync_tab(*id),
                Node::Folder { name, children, .. } => Some(SyncNode::Folder { name: name.clone(), children: self.sync_tree(children) }),
            })
            .collect()
    }

    /// Pinned and kept together, for change detection (an edit to either counts as an edit to the space).
    fn space_hash(&self, i: usize) -> String {
        let s = &self.spaces[i];
        let mut t = self.sync_tree(&s.pinned);
        t.push(SyncNode::Folder { name: "\u{1}kept".into(), children: self.sync_tree(&s.kept) });
        tree_hash(&t)
    }

    pub fn favorites_tree(&self) -> Vec<SyncNode> {
        self.favorites.iter().filter_map(|id| self.sync_tab(*id)).collect()
    }

    /// Note local edits to pinned trees and favorites. The first observation only records a baseline.
    pub fn track_changes(&mut self) -> bool {
        let mut pinned_ids = self.favorites.clone();
        for s in &self.spaces {
            walk(&s.pinned, &mut pinned_ids, false);
            walk(&s.kept, &mut pinned_ids, false);
        }
        for id in pinned_ids {
            if let Some(t) = self.tabs.get_mut(&id) {
                if t.pinned_url.is_none() {
                    t.pinned_url = Some(t.url.clone());
                }
            }
        }
        let mut changed = false;
        for i in 0..self.spaces.len() {
            let h = self.space_hash(i);
            let s = &mut self.spaces[i];
            match &s.pinned_hash {
                Some(old) if *old == h => {}
                Some(_) => {
                    s.pinned_hash = Some(h);
                    s.pinned_modified = now();
                    changed = true;
                }
                None => s.pinned_hash = Some(h),
            }
        }
        let h = tree_hash(&self.favorites_tree());
        match &self.favorites_hash {
            Some(old) if *old == h => {}
            Some(_) => {
                self.favorites_hash = Some(h);
                self.favorites_modified = now();
                changed = true;
            }
            None => self.favorites_hash = Some(h),
        }
        changed
    }

    /// Rebuild nodes from a synced tree, reusing local tabs with the same URL. Returns the new nodes.
    fn adopt(&mut self, tree: &[SyncNode], pool: &mut Vec<(String, TabId)>, folders: &[(String, bool)]) -> Vec<Node> {
        tree.iter()
            .map(|n| match n {
                SyncNode::Tab { title, url, custom_title } => {
                    let id = match pool.iter().position(|(u, _)| u == url) {
                        Some(i) => pool.remove(i).1,
                        None => {
                            let id = self.new_id();
                            self.tabs.insert(id, Tab { id, url: url.clone(), title: title.clone(), last_active: now(), ..Default::default() });
                            id
                        }
                    };
                    if let Some(t) = self.tabs.get_mut(&id) {
                        t.pinned_url = Some(url.clone());
                        t.custom_title = custom_title.clone();
                    }
                    Node::Tab { id }
                }
                SyncNode::Folder { name, children } => {
                    let id = self.new_id();
                    let open = folders.iter().any(|(n, o)| n == name && *o);
                    Node::Folder { id, name: name.clone(), open, children: self.adopt(children, pool, folders) }
                }
            })
            .collect()
    }

    fn pool_of(&self, nodes: &[Node]) -> (Vec<(String, TabId)>, Vec<(String, bool)>) {
        fn folders(nodes: &[Node], out: &mut Vec<(String, bool)>) {
            for n in nodes {
                if let Node::Folder { name, open, children, .. } = n {
                    out.push((name.clone(), *open));
                    folders(children, out);
                }
            }
        }
        let mut ids = Vec::new();
        walk(nodes, &mut ids, false);
        let pool = ids.into_iter().filter_map(|id| self.tabs.get(&id).map(|t| (t.pinned_url.clone().unwrap_or_else(|| t.url.clone()), id))).collect();
        let mut f = Vec::new();
        folders(nodes, &mut f);
        (pool, f)
    }

    /// Adopt any pinned tree or favorites edited more recently on another device. Returns tabs that went away.
    pub fn apply_remote(&mut self, devices: &[DeviceTabs]) -> Vec<TabId> {
        let mut removed = Vec::new();
        let mut names: Vec<String> = Vec::new();
        for d in devices {
            for s in &d.spaces {
                if !names.contains(&s.name) {
                    names.push(s.name.clone());
                }
            }
        }
        for name in names {
            let Some(best) = devices.iter().flat_map(|d| d.spaces.iter()).filter(|s| s.name == name).max_by_key(|s| s.modified) else { continue };
            if best.modified == 0 {
                continue;
            }
            let idx = match self.space_index(&name) {
                Some(i) => i,
                None if !best.tree.is_empty() => {
                    self.spaces.push(Space { name: name.clone(), ..Default::default() });
                    self.spaces.len() - 1
                }
                None => continue,
            };
            if best.modified <= self.spaces[idx].pinned_modified {
                continue;
            }
            let mut old = std::mem::take(&mut self.spaces[idx].pinned);
            old.extend(std::mem::take(&mut self.spaces[idx].kept));
            let (mut pool, folders) = self.pool_of(&old);
            let pinned = self.adopt(&best.tree, &mut pool, &folders);
            let kept = self.adopt(&best.kept, &mut pool, &folders);
            self.spaces[idx].pinned = pinned;
            self.spaces[idx].kept = kept;
            let h = self.space_hash(idx);
            let s = &mut self.spaces[idx];
            s.pinned_hash = Some(h);
            s.pinned_modified = best.modified;
            for (_, id) in pool {
                if s.active == Some(id) {
                    s.active = None;
                }
                s.split.retain(|t| *t != id);
                self.tabs.remove(&id);
                removed.push(id);
            }
        }
        if let Some(best) = devices.iter().max_by_key(|d| d.favorites_modified) {
            if best.favorites_modified > self.favorites_modified {
                let old: Vec<Node> = self.favorites.iter().map(|id| Node::Tab { id: *id }).collect();
                let (mut pool, _) = self.pool_of(&old);
                let nodes = self.adopt(&best.favorites, &mut pool, &[]);
                self.favorites = nodes.iter().filter_map(|n| if let Node::Tab { id } = n { Some(*id) } else { None }).collect();
                self.favorites_hash = Some(tree_hash(&self.favorites_tree()));
                self.favorites_modified = best.favorites_modified;
                for (_, id) in pool {
                    for s in &mut self.spaces {
                        if s.active == Some(id) {
                            s.active = None;
                        }
                    }
                    self.tabs.remove(&id);
                    removed.push(id);
                }
            }
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        let mut s = State::default();
        s.spaces.push(Space { name: "A".into(), ..Default::default() });
        s
    }

    /// Only pinned, kept and favorite tabs unopened for the whole period count; keeping one restarts its clock, and a
    /// tab from before Valerie tracked this starts counting now.
    #[test]
    fn finds_saved_tabs_nobody_opens() {
        let mut s = state();
        let day = 86400;
        let now = 400 * day;
        let ids: Vec<TabId> = (0..5).map(|i| s.add_tab(0, &format!("https://site{i}.com"), false)).collect();
        for &id in &ids[..4] {
            s.pin(id);
        }
        s.tabs.get_mut(&ids[0]).unwrap().last_active = now - 100 * day; // pinned, old: asked about
        s.tabs.get_mut(&ids[1]).unwrap().last_active = now - 10 * day; // pinned, recent
        s.tabs.get_mut(&ids[2]).unwrap().last_active = now - 100 * day; // pinned, old, but kept when asked
        s.tabs.get_mut(&ids[2]).unwrap().kept_at = now - 5 * day;
        s.tabs.get_mut(&ids[3]).unwrap().last_active = 0; // from before this was tracked
        s.tabs.get_mut(&ids[4]).unwrap().last_active = now - 100 * day; // a Today tab: not saved
        assert_eq!(s.unused_saved(90.0, now), vec![ids[0]]);
        assert_eq!(s.tabs[&ids[3]].last_active, now);
        s.archive_tab(ids[0]);
        assert!(s.unused_saved(90.0, now).is_empty() && s.archive[0].url == "https://site0.com");
    }

    #[test]
    fn close_activates_neighbour_and_reopen_remembers() {
        let mut s = state();
        let a = s.add_tab(0, "https://a", false);
        let b = s.add_tab(0, "https://b", false);
        let c = s.add_tab(0, "https://c", false);
        s.spaces[0].active = Some(b);
        assert_eq!(s.close_tab(b), Some(c));
        assert_eq!(s.close_tab(c), Some(a));
        assert_eq!(s.closed.len(), 2);
    }

    fn device(spaces: Vec<crate::sync::SyncedSpace>) -> DeviceTabs {
        DeviceTabs { device: "other".into(), id: "other".into(), updated: 1, spaces, favorites: vec![], favorites_modified: 0 }
    }

    fn synced(name: &str, urls: &[&str], modified: u64) -> crate::sync::SyncedSpace {
        crate::sync::SyncedSpace {
            name: name.into(),
            pinned: vec![],
            tabs: vec![],
            tree: urls.iter().map(|u| SyncNode::Tab { title: String::new(), url: u.to_string(), custom_title: None }).collect(),
            kept: vec![],
            modified,
        }
    }

    #[test]
    fn sync_newest_pinned_tree_wins_and_reuses_tabs() {
        let mut s = state();
        let a = s.add_tab(0, "https://a", false);
        s.pin(a);
        s.track_changes(); // baseline: never edited locally
        assert_eq!(s.spaces[0].pinned_modified, 0);
        // An untouched remote (modified 0) never overrides.
        assert!(s.apply_remote(&[device(vec![synced("A", &[], 0)])]).is_empty());
        assert_eq!(s.order(0, false), vec![a]);
        // A newer remote edit replaces the tree, keeping the existing tab for a URL it still has.
        s.apply_remote(&[device(vec![synced("A", &["https://b", "https://a"], 50), synced("New", &["https://c"], 40)])]);
        let order = s.order(0, false);
        assert_eq!(order.len(), 2);
        assert_eq!(order[1], a);
        assert_eq!(s.spaces[0].pinned_modified, 50);
        assert_eq!(s.space_index("New").map(|i| s.order(i, false).len()), Some(1));
        // Browsing inside a pinned tab is not an edit; unpinning is.
        s.tabs.get_mut(&a).unwrap().url = "https://a/deep/page".into();
        assert!(!s.track_changes());
        s.unpin(a);
        assert!(s.track_changes());
        assert!(s.spaces[0].pinned_modified > 50);
    }

    #[test]
    fn pin_moves_between_sections_and_folders_flatten() {
        let mut s = state();
        let a = s.add_tab(0, "https://a", false);
        s.pin(a);
        assert!(s.is_pinned(a) && s.spaces[0].today.is_empty());
        let f = s.new_id();
        s.spaces[0].pinned.push(Node::Folder { id: f, name: "F".into(), open: false, children: vec![] });
        s.detach(a);
        if let Some(Node::Folder { children, .. }) = find_folder(&mut s.spaces[0].pinned, f) {
            children.push(Node::Tab { id: a });
        }
        assert_eq!(s.order(0, true), Vec::<TabId>::new());
        assert_eq!(s.order(0, false), vec![a]);
        s.delete_folder(f);
        assert_eq!(s.order(0, true), vec![a]);
    }
}
