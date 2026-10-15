use crate::command::TabId;
use crate::config::Config;
use crate::import::{Bookmark, ImportedNode, ImportedTab, Imported};
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
    #[serde(default)]
    pub last_active: u64,
    #[serde(default, skip)]
    pub playing: bool,
    #[serde(default, skip)]
    pub loading: bool,
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
    #[serde(default)]
    pub today: Vec<TabId>,
    #[serde(default)]
    pub active: Option<TabId>,
    #[serde(default)]
    pub split: Vec<TabId>,
    #[serde(default)]
    pub theme: Option<String>,
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
}

const MAX_HISTORY: usize = 5000;
const MAX_ARCHIVE: usize = 1000;
const MAX_CLOSED: usize = 50;

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

    /// Tabs in sidebar order for one space: pinned (inside open folders) then today.
    pub fn order(&self, space: usize, only_open: bool) -> Vec<TabId> {
        let s = &self.spaces[space];
        let mut v = Vec::new();
        walk(&s.pinned, &mut v, only_open);
        v.extend(&s.today);
        v
    }

    pub fn is_pinned(&self, tab: TabId) -> bool {
        self.spaces.iter().any(|s| {
            let mut v = Vec::new();
            walk(&s.pinned, &mut v, false);
            v.contains(&tab)
        })
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
            if self.closed.len() > MAX_CLOSED {
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

    pub fn archive_tab(&mut self, tab: TabId) {
        let space = self.space_of(tab).map(|i| self.spaces[i].name.clone()).unwrap_or_default();
        self.detach(tab);
        if let Some(t) = self.tabs.remove(&tab) {
            self.archive.insert(
                0,
                Archived { url: t.url.clone(), title: t.display_title().to_string(), space, archived_at: now() },
            );
            self.archive.truncate(MAX_ARCHIVE);
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
            self.history.truncate(MAX_HISTORY);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        let mut s = State::default();
        s.spaces.push(Space { name: "A".into(), ..Default::default() });
        s
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
