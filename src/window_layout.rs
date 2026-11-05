//! Layouts: the whole window described in a file. `layouts/<name>.yaml` (in the config folder, or in a plugin's
//! `layouts/` folder) is a tree of rows and columns whose blocks are Valerie's own parts or plugins' panels, with the
//! settings and CSS that go with it. A broken edit keeps the last good version and says why in `<name>.error`.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

/// The window Valerie has always had, arranged by the `layout` and `sidebar` settings. Needs no file.
pub const CLASSIC: &str = "classic";

/// Layouts that ship with Valerie. A file with the same name in `layouts/` replaces one.
const BUILT_IN: &[(&str, &str)] = &[
    ("workbench", include_str!("../ui/layouts/workbench.yaml")),
    ("tab-strip", include_str!("../ui/layouts/tab-strip.yaml")),
    ("valerie-98", include_str!("../ui/layouts/valerie-98.yaml")),
];

/// The settings sections a layout may bring: how the window looks, never what Valerie is allowed to do. Layouts arrive
/// through sync and from other people, so anything that runs programs or reaches data stays out of reach.
const LOOK_SECTIONS: &[&str] = &["layout", "sidebar", "window", "command_bar", "start_page", "toast_seconds"];

/// The CSS a layout may bring, one entry per window (see `ui/*.css` in the guide).
const CSS_WINDOWS: &[&str] = &["chrome", "palette", "chat", "terminal", "little", "answer", "welcome", "plugins", "page"];

/// Valerie's own windows a layout can replace: what people call each one, and the file that draws it.
pub const WINDOWS: &[(&str, &str)] = &[
    ("sidebar", "chrome.html"),
    ("command_bar", "palette.html"),
    ("chats", "chat.html"),
    ("terminal", "terminal.html"),
    ("little_window", "little.html"),
    ("answer", "answer.html"),
    ("welcome", "welcome.html"),
    ("plugins", "plugins.html"),
];

/// What each window can reach, for the question asked before it's replaced.
pub fn window_powers(window: &str) -> &'static str {
    match window {
        "terminal" => "It can run any program on your computer, in your shells.",
        "chats" => "It can talk to your AI agents and read your chats.",
        "sidebar" | "command_bar" => "It can run every Valerie command and see your tabs and history.",
        "plugins" => "It can install and remove plugins.",
        _ => "It can run Valerie commands.",
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Node {
    /// Commands find a block by this. Panels and parts default to their name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<Vec<Node>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<Vec<Node>>,
    /// One of Valerie's own parts: pages | sidebar | top_bar | terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// `<plugin>` (its panel.html) or `<plugin>/<file>` (its <file>.html).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<String>,
    /// Given to the panel as `valerie.props`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub props: Option<Json>,
    /// Pixels along the row or column it's in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// Its share of what the sized blocks leave (default 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default)]
    pub hidden: bool,
    /// Pixels between the children of a row or column. Dragging that gap resizes them.
    #[serde(default)]
    pub gap: f64,
    /// Pixels of empty space inside this block's edge.
    #[serde(default)]
    pub padding: f64,
    /// A panel's rounded corners, pixels (default `layout.corner_radius`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
}

impl Node {
    fn children(&self) -> Option<&Vec<Node>> {
        self.row.as_ref().or(self.column.as_ref())
    }

    /// What commands call it.
    pub fn name(&self) -> Option<String> {
        self.id.clone().or_else(|| self.part.clone()).or_else(|| self.panel.as_ref().map(|p| p.replace('/', "-")))
    }

    fn walk<'a>(&'a self, out: &mut Vec<&'a Node>) {
        out.push(self);
        for c in self.children().into_iter().flatten() {
            c.walk(out);
        }
    }
}

/// A panel floating over the window, against one edge.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Floating {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub panel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub props: Option<Json>,
    /// top | right | bottom | left
    pub edge: String,
    /// start | center | end, along that edge.
    #[serde(default = "center")]
    pub align: String,
    pub width: f64,
    pub height: f64,
    /// Pixels from the edge.
    #[serde(default = "twelve")]
    pub margin: f64,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
}

impl Floating {
    pub fn name(&self) -> String {
        self.id.clone().unwrap_or_else(|| self.panel.replace('/', "-"))
    }
}

fn center() -> String {
    "center".into()
}

fn twelve() -> f64 {
    12.0
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    /// The name people see in the command bar.
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Look settings used while this layout shows, on top of the user's.
    #[serde(default)]
    pub settings: Option<serde_yaml::Value>,
    /// CSS per window while this layout shows (`chrome`, `palette`, …), after the user's `ui/*.css`.
    #[serde(default)]
    pub css: BTreeMap<String, String>,
    /// The window's tree. Without one, Valerie's usual window with this layout's settings and CSS.
    #[serde(default)]
    pub window: Option<Node>,
    #[serde(default)]
    pub floating: Vec<Floating>,
    /// Valerie's own windows drawn by a plugin's HTML instead: window name → `<plugin>/<file>`.
    #[serde(default)]
    pub replace: BTreeMap<String, String>,
}

impl Layout {
    /// Every panel it uses, as written.
    pub fn panels(&self) -> Vec<String> {
        let mut all = Vec::new();
        if let Some(w) = &self.window {
            w.walk(&mut all);
        }
        all.iter().filter_map(|n| n.panel.clone()).chain(self.floating.iter().map(|f| f.panel.clone())).collect()
    }

    fn has_part(&self, part: &str) -> bool {
        let mut all = Vec::new();
        if let Some(w) = &self.window {
            w.walk(&mut all);
        }
        all.iter().any(|n| n.part.as_deref() == Some(part))
    }

    /// The settings to lay over the user's while this layout shows: its own, plus what its tree implies.
    pub fn overlay(&self) -> Option<serde_yaml::Value> {
        let mut over = self.settings.clone().unwrap_or(serde_yaml::Value::Mapping(Default::default()));
        let Some(root) = &self.window else { return self.settings.clone() };
        let map = over.as_mapping_mut()?;
        let section = map.entry("layout".into()).or_insert(serde_yaml::Value::Mapping(Default::default()));
        let section = section.as_mapping_mut()?;
        section.insert("tab_bar".into(), if self.has_part("top_bar") { "top".into() } else { "side".into() });
        if !self.has_part("sidebar") {
            section.insert("sidebar_visible".into(), false.into());
            section.insert("reveal_sidebar_on_edge".into(), false.into());
        } else {
            // The sidebar's own edge handle and padding go by which side of the pages it's on.
            let mut order = Vec::new();
            root.walk(&mut order);
            let at = |p: &str| order.iter().position(|n| n.part.as_deref() == Some(p));
            let side = if at("sidebar") < at("pages") { "left" } else { "right" };
            section.insert("sidebar_position".into(), side.into());
        }
        Some(over)
    }

    /// What's wrong with its structure (nothing here looks at plugins, which come and go).
    pub fn validate(&self) -> Result<()> {
        for key in self.settings.as_ref().and_then(|s| s.as_mapping()).into_iter().flat_map(|m| m.keys()) {
            let k = key.as_str().unwrap_or("");
            if !LOOK_SECTIONS.contains(&k) {
                bail!("settings: a layout can only change how the window looks ({}), not {k:?}", LOOK_SECTIONS.join(", "));
            }
        }
        if self.settings.as_ref().is_some_and(|s| s.get("layout").is_some_and(|l| l.get("arrangement").is_some())) {
            bail!("settings: a layout can't choose the layout");
        }
        for (window, panel) in &self.replace {
            if !WINDOWS.iter().any(|(w, _)| w == window) {
                bail!("replace: {window:?} isn't one of Valerie's windows; use {}", WINDOWS.iter().map(|(w, _)| *w).collect::<Vec<_>>().join(", "));
            }
            panel_parts(panel)?;
        }
        for k in self.css.keys() {
            if !CSS_WINDOWS.contains(&k.as_str()) {
                bail!("css: {k:?} isn't one of Valerie's windows; use {}", CSS_WINDOWS.join(", "));
            }
        }
        let mut seen = HashSet::new();
        let mut all = Vec::new();
        if let Some(w) = &self.window {
            w.walk(&mut all);
            let mut pages = 0;
            for n in &all {
                let what = [n.row.is_some(), n.column.is_some(), n.part.is_some(), n.panel.is_some()].iter().filter(|b| **b).count();
                if what != 1 {
                    bail!("window: each block needs exactly one of row, column, part or panel ({})", describe(n));
                }
                if let Some(p) = &n.part {
                    match p.as_str() {
                        "pages" => {
                            pages += 1;
                            if n.hidden {
                                bail!("window: the pages block can't be hidden");
                            }
                        }
                        "sidebar" | "top_bar" | "terminal" => {}
                        other => bail!("window: part {other:?} isn't one of pages, sidebar, top_bar, terminal"),
                    }
                }
                if let Some(p) = &n.panel {
                    panel_parts(p)?;
                }
                if n.size.is_some_and(|s| !(s > 0.0)) || n.weight.is_some_and(|s| !(s > 0.0)) {
                    bail!("window: size and weight must be above 0 ({})", describe(n));
                }
                if n.size.is_some() && n.weight.is_some() {
                    bail!("window: use size or weight, not both ({})", describe(n));
                }
                if n.gap < 0.0 || n.padding < 0.0 {
                    bail!("window: gap and padding can't be negative ({})", describe(n));
                }
                if let Some(name) = n.name() {
                    if !seen.insert(name.clone()) {
                        bail!("window: two blocks are called {name:?}; give one an `id`");
                    }
                }
            }
            if pages != 1 {
                bail!("window: it needs exactly one `part: pages` block, or there's nowhere to show web pages (found {pages})");
            }
            if self.has_part("sidebar") && self.has_part("top_bar") {
                bail!("window: use part sidebar or part top_bar, not both (they show the same tabs)");
            }
        }
        for f in &self.floating {
            panel_parts(&f.panel)?;
            if !["top", "right", "bottom", "left"].contains(&f.edge.as_str()) {
                bail!("floating: edge is {:?}; use top, right, bottom or left", f.edge);
            }
            if !["start", "center", "end"].contains(&f.align.as_str()) {
                bail!("floating: align is {:?}; use start, center or end", f.align);
            }
            if !(f.width > 0.0 && f.height > 0.0) {
                bail!("floating: {} needs a width and height above 0", f.panel);
            }
            if !seen.insert(f.name()) {
                bail!("floating: two blocks are called {:?}; give one an `id`", f.name());
            }
        }
        Ok(())
    }
}

fn describe(n: &Node) -> String {
    n.name().unwrap_or_else(|| if n.row.is_some() { "a row".into() } else if n.column.is_some() { "a column".into() } else { "a block with nothing in it".into() })
}

fn name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 60 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `<plugin>` or `<plugin>/<file>`: the plugin's folder name and the HTML file's name without `.html`.
pub fn panel_parts(panel: &str) -> Result<(&str, &str)> {
    let (plugin, file) = panel.split_once('/').unwrap_or((panel, "panel"));
    if !name_ok(plugin) || !name_ok(file) {
        bail!("panel {panel:?}: write <plugin> or <plugin>/<file>, letters, digits, - and _ only");
    }
    Ok((plugin, file))
}

/// A layout's text, parsed and checked.
pub fn parse(text: &str) -> Result<Layout> {
    let layout: Layout = serde_yaml::from_str(text)?;
    layout.validate()?;
    Ok(layout)
}

fn dir() -> PathBuf {
    crate::config::config_dir().join("layouts")
}

/// `layouts/` files first, then plugins' (not ones waiting to be allowed on this machine), then Valerie's own.
fn source(name: &str) -> Option<String> {
    if !name_ok(name) {
        return None;
    }
    let file = format!("{name}.yaml");
    if let Ok(t) = std::fs::read_to_string(dir().join(&file)) {
        return Some(t);
    }
    let trust = crate::trust::load();
    for p in plugin_folders() {
        if crate::trust::plugin_allowed(&trust, &p.file_name().unwrap_or_default().to_string_lossy()) {
            if let Ok(t) = std::fs::read_to_string(p.join("layouts").join(&file)) {
                return Some(t);
            }
        }
    }
    BUILT_IN.iter().find(|(n, _)| *n == name).map(|(_, t)| t.to_string())
}

fn plugin_folders() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(crate::plugins::plugins_dir()).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    dirs
}

/// The text behind a layout, to copy or read.
pub fn text_of(name: &str) -> Option<String> {
    source(name)
}

/// The layouts Valerie ships, for the guide's reference folder.
pub fn built_ins() -> &'static [(&'static str, &'static str)] {
    BUILT_IN
}

/// Every layout's name: classic first, then the rest in order.
pub fn names() -> Vec<String> {
    let mut out: Vec<String> = BUILT_IN.iter().map(|(n, _)| n.to_string()).collect();
    let yaml_names = |d: PathBuf| -> Vec<String> {
        std::fs::read_dir(d).into_iter().flatten().flatten().filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".yaml")).map(str::to_string)).filter(|n| name_ok(n)).collect()
    };
    out.extend(yaml_names(dir()));
    let trust = crate::trust::load();
    for p in plugin_folders() {
        if crate::trust::plugin_allowed(&trust, &p.file_name().unwrap_or_default().to_string_lossy()) {
            out.extend(yaml_names(p.join("layouts")));
        }
    }
    out.retain(|n| n != CLASSIC);
    out.sort();
    out.dedup();
    out.insert(0, CLASSIC.to_string());
    out
}

/// The last version of each layout that loaded, so a broken edit changes nothing.
static LAST_GOOD: Mutex<Option<HashMap<String, Layout>>> = Mutex::new(None);

fn error_file(name: &str) -> PathBuf {
    dir().join(format!("{name}.error"))
}

/// Why this layout's file isn't being used, if it isn't.
pub fn error_of(name: &str) -> Option<String> {
    std::fs::read_to_string(error_file(name)).ok().filter(|t| !t.trim().is_empty())
}

/// The layout to show. If its file is broken: the last good version and `<name>.error` saying why. None for classic,
/// or a layout that never loaded (Valerie's usual window).
pub fn active(name: &str) -> Option<Layout> {
    if name == CLASSIC || name.is_empty() {
        return None;
    }
    let result = source(name).with_context(|| format!("there's no layout called {name:?}")).and_then(|t| parse(&t));
    let mut good = LAST_GOOD.lock().ok()?;
    let good = good.get_or_insert_with(HashMap::new);
    match result {
        Ok(l) => {
            let _ = std::fs::remove_file(error_file(name));
            good.insert(name.to_string(), l.clone());
            Some(l)
        }
        Err(e) => {
            let why = format!("{e:#}");
            if error_of(name).as_deref() != Some(why.as_str()) {
                crate::log::line("layout", format!("{name}: {why}"));
                let _ = std::fs::create_dir_all(dir());
                let _ = std::fs::write(error_file(name), &why);
            }
            good.get(name).cloned()
        }
    }
}

/// What `check-config` reports about every layout: its file and the plugins it needs.
pub fn problems(plugin_panels: impl Fn(&str, &str) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    for name in names().iter().filter(|n| *n != CLASSIC) {
        match source(name).map(|t| parse(&t)) {
            Some(Ok(l)) => {
                for p in l.panels() {
                    match panel_parts(&p) {
                        Ok((plugin, file)) if !plugin_panels(plugin, file) => {
                            out.push(format!("layout {name}: panel {p:?} needs the file plugins/{plugin}/{file}.html"));
                        }
                        Err(e) => out.push(format!("layout {name}: {e:#}")),
                        _ => {}
                    }
                }
            }
            Some(Err(e)) => out.push(format!("layout {name}: {e:#}")),
            None => out.push(format!("layout {name}: can't read it")),
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A panel with its place in the window.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub id: String,
    pub panel: String,
    pub props: Json,
    pub rect: Rect,
    pub radius: Option<f64>,
    pub floating: bool,
}

/// A gap people can drag to resize a block.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Divider {
    pub id: String,
    /// A row's gap is dragged sideways; a column's up and down.
    pub across: bool,
    pub rect: Rect,
    pub px: f64,
    /// +1 when the block grows as the pointer moves right or down, -1 when it shrinks.
    pub sign: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Arranged {
    pub pages: Rect,
    pub sidebar: Option<Rect>,
    pub top_bar: Option<Rect>,
    pub terminal: Option<Rect>,
    pub panels: Vec<Placed>,
    pub dividers: Vec<Divider>,
}

/// What people changed by hand and what the app knows about its own parts.
pub struct Inputs<'a> {
    pub sidebar_width: f64,
    pub sidebar_visible: bool,
    pub top_bar_height: f64,
    /// The terminal's block only takes room while the terminal is open.
    pub terminal_open: bool,
    /// Sizes dragged to and blocks shown or hidden, by block name.
    pub sizes: &'a HashMap<String, f64>,
    pub shown: &'a HashMap<String, bool>,
    /// Panels that can't show (their plugin is off, missing or waiting to be allowed): their block folds away.
    pub unavailable: &'a HashSet<String>,
}

fn visible(n: &Node, i: &Inputs) -> bool {
    if n.part.as_deref() == Some("pages") {
        return true;
    }
    if n.part.as_deref() == Some("sidebar") && !i.sidebar_visible {
        return false;
    }
    if n.part.as_deref() == Some("terminal") && !i.terminal_open {
        return false;
    }
    if n.panel.as_ref().is_some_and(|p| i.unavailable.contains(p)) {
        return false;
    }
    if !n.name().and_then(|name| i.shown.get(&name).copied()).unwrap_or(!n.hidden) {
        return false;
    }
    match n.children() {
        Some(kids) => kids.iter().any(|k| visible(k, i)),
        None => true,
    }
}

fn fixed_size(n: &Node, i: &Inputs) -> Option<f64> {
    let own = n.name().and_then(|name| i.sizes.get(&name).copied()).or(n.size);
    let px = match n.part.as_deref() {
        Some("sidebar") => Some(i.sidebar_width),
        Some("top_bar") => own.or(Some(i.top_bar_height)),
        _ => own,
    }?;
    Some(px.max(n.min.unwrap_or(1.0)).min(n.max.unwrap_or(f64::MAX)).max(1.0))
}

fn place(n: &Node, r: Rect, i: &Inputs, out: &mut Arranged) {
    let r = Rect { x: r.x + n.padding, y: r.y + n.padding, w: (r.w - 2.0 * n.padding).max(1.0), h: (r.h - 2.0 * n.padding).max(1.0) };
    if let Some(p) = n.part.as_deref() {
        match p {
            "pages" => out.pages = r,
            "sidebar" => out.sidebar = Some(r),
            "terminal" => out.terminal = Some(r),
            _ => out.top_bar = Some(r),
        }
        return;
    }
    if let Some(p) = &n.panel {
        out.panels.push(Placed { id: n.name().unwrap_or_default(), panel: p.clone(), props: n.props.clone().unwrap_or(Json::Null), rect: r, radius: n.radius, floating: false });
        return;
    }
    let across = n.row.is_some();
    let kids: Vec<&Node> = n.children().into_iter().flatten().filter(|k| visible(k, i)).collect();
    if kids.is_empty() {
        return;
    }
    let length = if across { r.w } else { r.h };
    let free = (length - n.gap * (kids.len() - 1) as f64).max(1.0);
    let mut fixed: Vec<Option<f64>> = kids.iter().map(|k| fixed_size(k, i)).collect();
    // Blocks that share what's left. With none, the last block takes it, so the row is always full.
    let holds_pages = |k: &Node| { let mut all = Vec::new(); k.walk(&mut all); all.iter().any(|x| x.part.as_deref() == Some("pages")) };
    if fixed.iter().all(|f| f.is_some()) {
        let last = kids.iter().position(|k| holds_pages(k)).unwrap_or(kids.len() - 1);
        fixed[last] = None;
    }
    let sum: f64 = fixed.iter().flatten().sum();
    // Pages keep at least this much whatever sizes a layout asks for, so a window is never all panels.
    let room = (free - (free * 0.5).min(160.0)).max(1.0);
    let scale = if sum > room { room / sum } else { 1.0 };
    let weights: f64 = kids.iter().zip(&fixed).filter(|(_, f)| f.is_none()).map(|(k, _)| k.weight.unwrap_or(1.0)).sum();
    let left = (free - sum * scale).max(1.0);
    let mut at = if across { r.x } else { r.y };
    for (idx, k) in kids.iter().enumerate() {
        let px = match fixed[idx] {
            Some(f) => f * scale,
            None => left * k.weight.unwrap_or(1.0) / weights.max(0.0001),
        };
        let cell = if across { Rect { x: at, y: r.y, w: px, h: r.h } } else { Rect { x: r.x, y: at, w: r.w, h: px } };
        place(k, cell, i, out);
        if idx + 1 < kids.len() && n.gap > 0.0 {
            // The sized block on either side moves with the pointer; the sidebar has its own edge handle.
            let next = kids[idx + 1];
            let resizable = |b: &Node, f: Option<f64>| f.is_some() && b.part.as_deref() != Some("sidebar") && b.name().is_some();
            let chosen = if resizable(k, fixed[idx]) { Some((*k, fixed[idx], 1.0)) } else if resizable(next, fixed[idx + 1]) { Some((next, fixed[idx + 1], -1.0)) } else { None };
            if let Some((b, Some(size), sign)) = chosen {
                let rect = if across { Rect { x: at + px, y: r.y, w: n.gap, h: r.h } } else { Rect { x: r.x, y: at + px, w: r.w, h: n.gap } };
                out.dividers.push(Divider { id: b.name().unwrap_or_default(), across, rect, px: size * scale, sign, min: b.min.unwrap_or(40.0), max: b.max.unwrap_or(length - 120.0) });
            }
        }
        at += px + n.gap;
    }
}

/// Where everything goes in a window this big.
pub fn arrange(layout: &Layout, width: f64, height: f64, i: &Inputs) -> Option<Arranged> {
    let root = layout.window.as_ref()?;
    let mut out = Arranged::default();
    place(root, Rect { x: 0.0, y: 0.0, w: width, h: height }, i, &mut out);
    for f in &layout.floating {
        let name = f.name();
        if !i.shown.get(&name).copied().unwrap_or(!f.hidden) || i.unavailable.contains(&f.panel) {
            continue;
        }
        let (w, h) = (f.width.min(width - 2.0 * f.margin).max(1.0), f.height.min(height - 2.0 * f.margin).max(1.0));
        let along = |space: f64, own: f64| match f.align.as_str() {
            "start" => f.margin,
            "end" => space - own - f.margin,
            _ => (space - own) / 2.0,
        };
        let (x, y) = match f.edge.as_str() {
            "top" => (along(width, w), f.margin),
            "bottom" => (along(width, w), height - h - f.margin),
            "left" => (f.margin, along(height, h)),
            _ => (width - w - f.margin, along(height, h)),
        };
        out.panels.push(Placed { id: name, panel: f.panel.clone(), props: f.props.clone().unwrap_or(Json::Null), rect: Rect { x, y, w, h }, radius: f.radius, floating: true });
    }
    Some(out)
}

/// Every layout as the gallery shows it: its name, words, tree and where it comes from.
pub fn gallery() -> Vec<Json> {
    names().into_iter().map(|name| {
        if name == CLASSIC {
            let tree = serde_json::json!({ "row": [{ "part": "sidebar", "size": 248 }, { "part": "pages" }] });
            return serde_json::json!({ "name": name, "title": "Classic", "description": "The usual window, arranged by your settings.", "tree": tree, "from": "valerie" });
        }
        let from = if dir().join(format!("{name}.yaml")).exists() {
            "yours"
        } else if BUILT_IN.iter().any(|(n, _)| *n == name) {
            "valerie"
        } else {
            "plugin"
        };
        match source(&name).map(|t| parse(&t)) {
            Some(Ok(l)) => serde_json::json!({
                "name": name, "title": if l.title.is_empty() { name.replace(['-', '_'], " ") } else { l.title.clone() }, "description": l.description,
                "tree": l.window, "floating": l.floating, "replaces": l.replace.keys().collect::<Vec<_>>(), "from": from,
            }),
            Some(Err(e)) => serde_json::json!({ "name": name, "title": name, "error": format!("{e:#}"), "from": from }),
            None => serde_json::json!({ "name": name, "title": name, "error": "can't read it", "from": from }),
        }
    }).collect()
}

/// Turn the user's `layouts/<name>.yaml` into a plugin that can be shared on its own: `plugins/<name>/` with the layout
/// and a copy of every panel it uses, except Valerie's own `window-panels`, which everyone has.
pub fn package(name: &str, layouts: &std::path::Path, plugins: &std::path::Path) -> Result<PathBuf> {
    let dest = plugins.join(name);
    let file = format!("{name}.yaml");
    if dest.join("layouts").join(&file).exists() {
        return Ok(dest);
    }
    let path = layouts.join(&file);
    let text = std::fs::read_to_string(&path).with_context(|| format!("there's no layout called {name} of your own to share"))?;
    let parsed = parse(&text)?;
    if dest.exists() {
        bail!("a plugin called {name} already exists; rename the layout to share it");
    }
    let mut value: serde_yaml::Value = serde_yaml::from_str(&text)?;
    let mut permissions: Vec<String> = Vec::new();
    let mut copies: Vec<(String, String)> = Vec::new();
    let used: Vec<String> = parsed.panels().into_iter().chain(parsed.replace.values().cloned()).collect();
    let mut renames: HashMap<String, String> = HashMap::new();
    for panel in used {
        let (plugin, html) = panel_parts(&panel)?;
        if plugin == "window-panels" || renames.contains_key(&panel) {
            continue;
        }
        let from = plugins.join(plugin);
        let body = std::fs::read_to_string(from.join(format!("{html}.html"))).with_context(|| format!("panel {panel} isn't there"))?;
        let own = format!("{plugin}-{html}");
        let manifest: serde_yaml::Value = std::fs::read_to_string(from.join("plugin.yaml")).ok().and_then(|t| serde_yaml::from_str(&t).ok()).unwrap_or_default();
        permissions.extend(manifest.get("permissions").and_then(|p| p.as_sequence()).into_iter().flatten().filter_map(|t| t.as_str().map(str::to_string)));
        copies.push((format!("{own}.html"), body));
        renames.insert(panel.clone(), format!("{name}/{own}"));
    }
    rename_panels(&mut value, &renames, None);
    permissions.sort();
    permissions.dedup();
    let description = if parsed.description.is_empty() { format!("The {} layout.", name.replace('-', " ")) } else { parsed.description.clone() };
    let manifest = serde_yaml::to_string(&serde_json::json!({ "description": description, "permissions": permissions }))?;
    std::fs::create_dir_all(dest.join("layouts"))?;
    std::fs::write(dest.join("plugin.yaml"), manifest)?;
    for (file, body) in copies {
        std::fs::write(dest.join(file), body)?;
    }
    std::fs::write(dest.join("layouts").join(&file), serde_yaml::to_string(&value)?)?;
    std::fs::remove_file(&path)?;
    Ok(dest)
}

/// `panel:` values anywhere in a layout's tree, and the values of its `replace:` map.
fn rename_panels(v: &mut serde_yaml::Value, renames: &HashMap<String, String>, key: Option<&str>) {
    match v {
        serde_yaml::Value::String(s) if matches!(key, Some("panel") | Some("replaced")) => {
            if let Some(new) = renames.get(s.as_str()) {
                *s = new.clone();
            }
        }
        serde_yaml::Value::Sequence(items) => items.iter_mut().for_each(|i| rename_panels(i, renames, None)),
        serde_yaml::Value::Mapping(map) => {
            let inside_replace = key == Some("replace");
            for (k, val) in map.iter_mut() {
                let k = k.as_str().map(str::to_string);
                let key = if inside_replace { Some("replaced") } else { k.as_deref() };
                rename_panels(val, renames, key);
            }
        }
        _ => {}
    }
}

/// A window drawn by a plugin instead of Valerie, with what it takes to allow that.
#[derive(Debug, Clone, PartialEq)]
pub struct Replacement {
    pub window: String,
    pub page: &'static str,
    pub plugin: String,
    pub html: String,
    /// Where the user's yes is kept. One from the marketplace is asked about again when its file changes; one of the
    /// user's own stays allowed while they (or their AI) work on it.
    pub grant: String,
    pub print: String,
}

/// The windows this layout replaces with files that exist in plugins that are on.
pub fn replacements(layout: &Layout, plugins: &[crate::plugins::Plugin]) -> Vec<Replacement> {
    let mut out = Vec::new();
    for (window, panel) in &layout.replace {
        let (Some((_, page)), Ok((plugin, file))) = (WINDOWS.iter().find(|(w, _)| w == window), panel_parts(panel)) else { continue };
        let Some(p) = plugins.iter().find(|p| p.name == plugin && p.manifest.enabled) else { continue };
        let Ok(html) = std::fs::read_to_string(p.dir.join(format!("{file}.html"))) else { continue };
        let print = if p.dir.join(crate::market::STAMP).exists() { crate::blocking::hash(&html) } else { "own".to_string() };
        out.push(Replacement { window: window.clone(), page, plugin: plugin.to_string(), html, grant: format!("replace:{plugin}:{window}"), print });
    }
    out
}

/// The windows being drawn by plugins right now (file name → HTML), which `ui_page` serves instead of Valerie's own.
static REPLACED: Mutex<Option<HashMap<&'static str, String>>> = Mutex::new(None);
/// Replaced windows built but not yet heard from: they get a few seconds to say they're ready.
static WAITING: Mutex<Option<HashMap<String, std::time::Instant>>> = Mutex::new(None);

pub fn set_replaced(now: HashMap<&'static str, String>) -> Vec<&'static str> {
    let Ok(mut r) = REPLACED.lock() else { return Vec::new() };
    let was = r.take().unwrap_or_default();
    let changed = WINDOWS.iter().map(|(_, page)| *page).filter(|page| was.get(page) != now.get(page)).collect();
    *r = Some(now);
    changed
}

/// A replaced window's HTML, starting its clock.
pub fn replaced(page: &str) -> Option<String> {
    let html = REPLACED.lock().ok()?.as_ref()?.get(page).cloned()?;
    if let Ok(mut w) = WAITING.lock() {
        w.get_or_insert_with(HashMap::new).insert(page.to_string(), std::time::Instant::now());
    }
    Some(html)
}

pub fn is_replaced(page: &str) -> bool {
    REPLACED.lock().ok().is_some_and(|r| r.as_ref().is_some_and(|m| m.contains_key(page)))
}

/// The window said it's ready, so it works.
pub fn heard_from(page: &str) {
    if let Ok(mut w) = WAITING.lock() {
        if let Some(w) = w.as_mut() {
            w.remove(page);
        }
    }
}

/// Is this window still within the time a replacement has to prove it starts?
pub fn starting(page: &str) -> bool {
    WAITING.lock().ok().is_some_and(|w| w.as_ref().is_some_and(|w| w.contains_key(page)))
}

/// Replaced windows that never said they were ready.
pub fn overdue() -> Vec<String> {
    let Ok(w) = WAITING.lock() else { return Vec::new() };
    w.as_ref().map(|w| w.iter().filter(|(_, t)| t.elapsed() > std::time::Duration::from_secs(8)).map(|(p, _)| p.clone()).collect()).unwrap_or_default()
}

/// A block's name in the saved sizes: per layout, so each keeps its own.
pub fn block_key(layout: &str, block: &str) -> String {
    format!("{layout}/{block}")
}

/// One layout's saved values, keyed by block.
pub fn for_layout<T: Clone>(saved: &HashMap<String, T>, layout: &str) -> HashMap<String, T> {
    let prefix = format!("{layout}/");
    saved.iter().filter_map(|(k, v)| k.strip_prefix(&prefix).map(|b| (b.to_string(), v.clone()))).collect()
}

/// Every block a layout has that can be shown or hidden.
pub fn blocks(layout: &Layout) -> Vec<String> {
    let mut all = Vec::new();
    if let Some(w) = &layout.window {
        w.walk(&mut all);
    }
    all.iter().filter(|n| n.part.as_deref() != Some("pages")).filter_map(|n| n.name()).chain(layout.floating.iter().map(|f| f.name())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs<'a>(sizes: &'a HashMap<String, f64>, shown: &'a HashMap<String, bool>, none: &'a HashSet<String>) -> Inputs<'a> {
        Inputs { sidebar_width: 200.0, sidebar_visible: true, top_bar_height: 40.0, terminal_open: false, sizes, shown, unavailable: none }
    }

    const BASIC: &str = "
window:
  row:
    - part: sidebar
    - part: pages
    - panel: a/b
      size: 300
";

    #[test]
    fn built_in_layouts_are_valid() {
        for (name, text) in BUILT_IN {
            parse(text).unwrap_or_else(|e| panic!("{name}: {e:#}"));
        }
    }

    #[test]
    fn a_row_shares_the_window() {
        let l = parse(BASIC).unwrap();
        let (a, b, c) = (HashMap::new(), HashMap::new(), HashSet::new());
        let o = arrange(&l, 1000.0, 600.0, &inputs(&a, &b, &c)).unwrap();
        assert_eq!(o.sidebar, Some(Rect { x: 0.0, y: 0.0, w: 200.0, h: 600.0 }));
        assert_eq!(o.pages, Rect { x: 200.0, y: 0.0, w: 500.0, h: 600.0 });
        assert_eq!(o.panels[0].rect, Rect { x: 700.0, y: 0.0, w: 300.0, h: 600.0 });
    }

    #[test]
    fn a_missing_panel_gives_its_room_to_the_pages() {
        let l = parse(BASIC).unwrap();
        let (a, b) = (HashMap::new(), HashMap::new());
        let gone: HashSet<String> = ["a/b".to_string()].into();
        let o = arrange(&l, 1000.0, 600.0, &inputs(&a, &b, &gone)).unwrap();
        assert!(o.panels.is_empty());
        assert_eq!(o.pages.w, 800.0);
    }

    #[test]
    fn hiding_and_dragging_override_the_file() {
        let l = parse(BASIC).unwrap();
        let sizes: HashMap<String, f64> = [("a-b".to_string(), 120.0)].into();
        let (b, c) = (HashMap::new(), HashSet::new());
        assert_eq!(arrange(&l, 1000.0, 600.0, &inputs(&sizes, &b, &c)).unwrap().panels[0].rect.w, 120.0);
        let shown: HashMap<String, bool> = [("a-b".to_string(), false)].into();
        let a = HashMap::new();
        assert!(arrange(&l, 1000.0, 600.0, &inputs(&a, &shown, &c)).unwrap().panels.is_empty());
    }

    #[test]
    fn pages_keep_room_whatever_a_layout_asks_for() {
        let l = parse("window: { row: [ { panel: a, size: 5000 }, { part: pages } ] }").unwrap();
        let (a, b, c) = (HashMap::new(), HashMap::new(), HashSet::new());
        let o = arrange(&l, 1000.0, 600.0, &inputs(&a, &b, &c)).unwrap();
        assert!(o.pages.w >= 160.0, "pages got {}", o.pages.w);
    }

    #[test]
    fn a_row_of_only_sized_blocks_is_still_full() {
        let l = parse("window: { row: [ { part: pages, size: 100 }, { panel: a, size: 100 } ] }").unwrap();
        let (a, b, c) = (HashMap::new(), HashMap::new(), HashSet::new());
        let o = arrange(&l, 1000.0, 600.0, &inputs(&a, &b, &c)).unwrap();
        assert_eq!(o.pages.w, 900.0);
    }

    #[test]
    fn gaps_can_be_dragged_and_move_the_sized_block() {
        let l = parse("window: { row: [ { part: pages }, { panel: a, size: 300, min: 100 } ], gap: 6 }").unwrap();
        let (a, b, c) = (HashMap::new(), HashMap::new(), HashSet::new());
        let o = arrange(&l, 1000.0, 600.0, &inputs(&a, &b, &c)).unwrap();
        assert_eq!(o.dividers.len(), 1);
        let d = &o.dividers[0];
        assert_eq!((d.id.as_str(), d.sign, d.px, d.min), ("a", -1.0, 300.0, 100.0));
        assert_eq!(d.rect, Rect { x: 694.0, y: 0.0, w: 6.0, h: 600.0 });
    }

    #[test]
    fn floating_panels_sit_against_their_edge() {
        let l = parse("window: { part: pages }\nfloating: [ { panel: a, edge: right, align: end, width: 100, height: 50 } ]").unwrap();
        let (a, b, c) = (HashMap::new(), HashMap::new(), HashSet::new());
        let o = arrange(&l, 1000.0, 600.0, &inputs(&a, &b, &c)).unwrap();
        assert_eq!(o.panels[0].rect, Rect { x: 888.0, y: 538.0, w: 100.0, h: 50.0 });
        assert!(o.panels[0].floating);
    }

    #[test]
    fn mistakes_are_refused_with_a_reason() {
        for (text, says) in [
            ("window: { row: [ { panel: a } ] }", "exactly one `part: pages`"),
            ("window: { row: [ { part: pages }, { part: pages, id: two } ] }", "exactly one `part: pages`"),
            ("window: { part: pages, panel: a }", "exactly one of row"),
            ("window: { part: nope }", "isn't one of"),
            ("window: { part: pages, hidden: true }", "can't be hidden"),
            ("window: { row: [ { part: pages }, { panel: a }, { panel: a } ] }", "two blocks"),
            ("window: { row: [ { part: pages }, { panel: '../x' } ] }", "letters, digits"),
            ("window: { part: pages, sizee: 3 }", "sizee"),
            ("settings: { agents: { default: evil } }", "can only change how the window looks"),
            ("settings: { mcp: { port: 1 } }", "can only change how the window looks"),
            ("settings: { layout: { arrangement: x } }", "can't choose"),
            ("css: { nope: 'a{}' }", "isn't one of Valerie's windows"),
            ("window: { row: [ { part: pages }, { part: sidebar }, { part: top_bar } ] }", "not both"),
            ("floating: [ { panel: a, edge: middle, width: 1, height: 1 } ]", "edge"),
        ] {
            let e = format!("{:#}", parse(text).unwrap_err());
            assert!(e.contains(says), "{text}: {e}");
        }
    }

    #[test]
    fn sharing_gathers_a_layouts_panels_into_one_plugin() {
        let root = std::env::temp_dir().join(format!("valerie-package-{}", std::process::id()));
        let (layouts, plugins) = (root.join("layouts"), root.join("plugins"));
        std::fs::create_dir_all(&layouts).unwrap();
        std::fs::create_dir_all(plugins.join("mine")).unwrap();
        std::fs::write(plugins.join("mine/plugin.yaml"), "description: x\npermissions: [run_command]\n").unwrap();
        std::fs::write(plugins.join("mine/board.html"), "<b>board</b>").unwrap();
        std::fs::write(plugins.join("mine/bar.html"), "<b>bar</b>").unwrap();
        std::fs::write(layouts.join("cool.yaml"), "description: Cool.\nreplace: { command_bar: mine/bar }\nwindow: { row: [ { part: pages }, { panel: mine/board }, { panel: window-panels/outline } ] }\n").unwrap();
        let dest = package("cool", &layouts, &plugins).unwrap();
        let l = parse(&std::fs::read_to_string(dest.join("layouts/cool.yaml")).unwrap()).unwrap();
        assert_eq!(l.panels(), vec!["cool/mine-board".to_string(), "window-panels/outline".to_string()]);
        assert_eq!(l.replace["command_bar"], "cool/mine-bar");
        assert!(dest.join("mine-board.html").exists() && dest.join("mine-bar.html").exists());
        assert!(std::fs::read_to_string(dest.join("plugin.yaml")).unwrap().contains("run_command"));
        assert!(!layouts.join("cool.yaml").exists());
        assert_eq!(package("cool", &layouts, &plugins).unwrap(), dest);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn replace_names_real_windows_only() {
        assert!(parse("replace: { command_bar: a/b, terminal: a/c }").is_ok());
        assert!(format!("{:#}", parse("replace: { nope: a/b }").unwrap_err()).contains("isn't one of Valerie's windows"));
        assert!(parse("replace: { sidebar: '../../etc/x' }").is_err());
    }

    #[test]
    fn the_terminal_block_only_takes_room_while_open() {
        let l = parse("window: { column: [ { part: pages }, { part: terminal, size: 200 } ] }").unwrap();
        let (a, b, c) = (HashMap::new(), HashMap::new(), HashSet::new());
        let closed = arrange(&l, 1000.0, 600.0, &inputs(&a, &b, &c)).unwrap();
        assert_eq!((closed.terminal, closed.pages.h), (None, 600.0));
        let mut open = inputs(&a, &b, &c);
        open.terminal_open = true;
        let o = arrange(&l, 1000.0, 600.0, &open).unwrap();
        assert_eq!((o.terminal.unwrap().h, o.pages.h), (200.0, 400.0));
    }

    #[test]
    fn the_tree_decides_which_tab_bar_shows() {
        let l = parse("window: { column: [ { part: top_bar }, { part: pages } ] }").unwrap();
        let o = l.overlay().unwrap();
        assert_eq!(o["layout"]["tab_bar"].as_str(), Some("top"));
        assert_eq!(o["layout"]["sidebar_visible"].as_bool(), Some(false));
        let l = parse("window: { row: [ { part: pages }, { part: sidebar } ] }").unwrap();
        let o = l.overlay().unwrap();
        assert_eq!(o["layout"]["tab_bar"].as_str(), Some("side"));
        assert_eq!(o["layout"]["sidebar_position"].as_str(), Some("right"));
    }

    #[test]
    fn a_layout_without_a_tree_only_brings_its_look() {
        let l = parse("settings: { sidebar: { tab_height: 30 } }").unwrap();
        assert!(l.overlay().unwrap().get("layout").is_none());
        let (a, b, c) = (HashMap::new(), HashMap::new(), HashSet::new());
        assert!(arrange(&l, 1000.0, 600.0, &inputs(&a, &b, &c)).is_none());
    }
}
