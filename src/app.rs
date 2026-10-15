use crate::command::{title_for, BarMode, Command, TabId, CATALOG};
use crate::config::{self, Config, SidebarPosition, TabBar, Theme, ThemeRef};
use crate::mcp::McpRequest;
use crate::state::{find_folder, now, Node, State, Tab};
use crate::{agents, blocking, import, passwords, platform, routing, sync};
use anyhow::Result;
use muda::{Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use tao::dpi::LogicalSize;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::window::{Window, WindowBuilder, WindowId};
use wry::{NewWindowResponse, PageLoadEvent, Rect, WebView, WebViewBuilder};

const TOP_BAR: f64 = 40.0;
const LITTLE_BAR: f64 = 36.0;
const MAX_UNTRUSTED_IPC: usize = 64 * 1024;
/// URL schemes handed to other apps when a page links to them.
const EXTERNAL_SCHEMES: &[&str] = &[
    "mailto", "tel", "sms", "facetime", "zoommtg", "zoomus", "msteams", "slack", "spotify", "vscode", "cursor",
    "figma", "notion", "linear", "discord", "obsidian", "raycast", "things", "itms-apps", "maps", "music",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    Tab(TabId),
    Little(WindowId),
    Peek,
}

#[derive(Debug, Clone, Copy)]
pub enum Source {
    Chrome,
    Overlay,
    LittleBar(WindowId),
    Page(Target),
}

pub enum UserEvent {
    Ipc { from: Source, msg: String },
    PageLoad { target: Target, url: String, finished: bool },
    Title { target: Target, title: String },
    NewWindow { target: Target, url: String },
    ExternalScheme(String),
    Menu(MenuId),
    Mcp(McpRequest),
    Tick,
    ConfigChanged,
    SyncDone(Result<Vec<sync::DeviceTabs>, String>),
    Downloaded { ok: bool },
    AgentDone(agents::Finished),
    /// Blocklist JSON ready to compile (None = unchanged since last compile, so reuse WebKit's cached copy).
    BlocklistBuilt { json: Option<String>, hash: String },
    #[cfg(target_os = "macos")]
    RulesReady(Result<blocking::mac::RuleList, String>, Option<String>),
}

struct Little {
    window: Window,
    bar: WebView,
    view: WebView,
    url: String,
    title: String,
    space: usize,
}

pub struct App {
    window: Window,
    chrome: WebView,
    overlay: WebView,
    views: HashMap<TabId, WebView>,
    zoom: HashMap<TabId, f64>,
    littles: HashMap<WindowId, Little>,
    focused_little: Option<WindowId>,
    config: Config,
    state: State,
    proxy: EventLoopProxy<UserEvent>,
    _menu: Option<Menu>,
    menu_cmds: HashMap<MenuId, Command>,
    bar_open: bool,
    remote: Vec<sync::DeviceTabs>,
    dirty: bool,
    last_sync: u64,
    syncing: bool,
    last_page_open: std::time::Instant,
    user_agent: String,
    agents_running: Vec<String>,
    agent_runs: Vec<agents::Finished>,
    boosts: Vec<Boost>,
    sidebar_peek: bool,
    /// Saved logins offered to a page, keyed by the nonce the page must echo back to fill one.
    fill_offers: HashMap<Target, (String, Vec<passwords::Entry>)>,
    pending_save: Option<PendingSave>,
    peek: Option<PeekView>,
    #[cfg(not(target_os = "macos"))]
    contexts: HashMap<String, wry::WebContext>,
    #[cfg(target_os = "macos")]
    rules: Option<blocking::mac::RuleList>,
    pub quit: bool,
}

struct PeekView {
    view: WebView,
    url: String,
}

struct PendingSave {
    target: Target,
    origin: String,
    username: String,
    password: String,
    nonce: String,
    at: u64,
    reoffered: bool,
}

/// Per-site CSS/JS from `<config dir>/boosts/<host>.css|.js`; a host also matches its subdomains.
#[derive(Default)]
struct Boost {
    host: String,
    css: String,
    js: String,
}

fn boosts_dir() -> PathBuf {
    config::config_dir().join("boosts")
}

fn load_boosts() -> Vec<Boost> {
    let mut map: std::collections::BTreeMap<String, Boost> = Default::default();
    for e in std::fs::read_dir(boosts_dir()).into_iter().flatten().flatten() {
        let path = e.path();
        let (Some(stem), Some(ext)) = (path.file_stem().and_then(|s| s.to_str()), path.extension().and_then(|s| s.to_str())) else { continue };
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let b = map.entry(stem.to_lowercase()).or_insert_with(|| Boost { host: stem.to_lowercase(), ..Default::default() });
        match ext {
            "css" => b.css = text,
            "js" => b.js = text,
            _ => {}
        }
    }
    map.into_values().collect()
}

fn boost_matches(pattern: &str, host: &str) -> bool {
    let host = host.to_lowercase();
    host == pattern || host.ends_with(&format!(".{pattern}"))
}

fn state_path() -> PathBuf {
    config::data_dir().join("state.json")
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect {
        position: wry::dpi::LogicalPosition::new(x, y).into(),
        size: wry::dpi::LogicalSize::new(w.max(1.0), h.max(1.0)).into(),
    }
}

/// UI files are compiled in; set VALERIE_UI_DIR to load them from disk while iterating on the design.
fn ui(name: &str) -> String {
    if let Ok(dir) = std::env::var("VALERIE_UI_DIR") {
        if let Ok(s) = std::fs::read_to_string(PathBuf::from(dir).join(name)) {
            return s;
        }
    }
    match name {
        "chrome.html" => include_str!("../ui/chrome.html"),
        "palette.html" => include_str!("../ui/palette.html"),
        "little.html" => include_str!("../ui/little.html"),
        "content.js" => include_str!("../ui/content.js"),
        _ => "",
    }
    .to_string()
}

/// wry hands back the JS result JSON-encoded; our scripts return JSON strings, so unwrap up to twice.
fn decode_js(s: &str) -> Value {
    match serde_json::from_str::<Value>(s) {
        Ok(Value::String(inner)) => serde_json::from_str(&inner).unwrap_or(Value::String(inner)),
        Ok(v) => v,
        Err(_) => Value::String(s.to_string()),
    }
}

fn scheme_of(url: &str) -> String {
    url.split(':').next().unwrap_or("").to_ascii_lowercase()
}

fn send(proxy: &EventLoopProxy<UserEvent>, ev: UserEvent) {
    let _ = proxy.send_event(ev);
}

fn mac_window(wb: WindowBuilder) -> WindowBuilder {
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::WindowBuilderExtMacOS;
        wb.with_titlebar_transparent(true).with_fullsize_content_view(true).with_title_hidden(true)
    }
    #[cfg(not(target_os = "macos"))]
    wb
}

impl App {
    fn new(elwt: &EventLoopWindowTarget<UserEvent>, config: Config, proxy: EventLoopProxy<UserEvent>) -> Result<App> {
        let window = mac_window(WindowBuilder::new())
            .with_title("Valerie")
            .with_inner_size(LogicalSize::new(1440.0, 920.0))
            .with_min_inner_size(LogicalSize::new(480.0, 360.0))
            .build(elwt)?;
        let size = window.inner_size().to_logical::<f64>(window.scale_factor());
        let full = rect(0.0, 0.0, size.width, size.height);
        let p = proxy.clone();
        let chrome = WebViewBuilder::new()
            .with_html(ui("chrome.html"))
            .with_bounds(full)
            .with_accept_first_mouse(true)
            .with_devtools(true)
            .with_ipc_handler(move |r| send(&p, UserEvent::Ipc { from: Source::Chrome, msg: r.into_body() }))
            .build_as_child(&window)?;
        let p = proxy.clone();
        let overlay = WebViewBuilder::new()
            .with_html(ui("palette.html"))
            .with_bounds(full)
            .with_transparent(true)
            .with_visible(false)
            .with_ipc_handler(move |r| send(&p, UserEvent::Ipc { from: Source::Overlay, msg: r.into_body() }))
            .build_as_child(&window)?;
        let mut state = State::load(&state_path());
        state.reconcile_spaces(&config);
        let user_agent = if config.user_agent.is_empty() { platform::safari_user_agent() } else { config.user_agent.clone() };
        Ok(App {
            window,
            chrome,
            overlay,
            views: HashMap::new(),
            zoom: HashMap::new(),
            littles: HashMap::new(),
            focused_little: None,
            config,
            state,
            proxy,
            _menu: None,
            menu_cmds: HashMap::new(),
            bar_open: false,
            remote: Vec::new(),
            dirty: false,
            last_sync: 0,
            syncing: false,
            last_page_open: std::time::Instant::now(),
            user_agent,
            agents_running: Vec::new(),
            agent_runs: Vec::new(),
            boosts: load_boosts(),
            sidebar_peek: false,
            fill_offers: HashMap::new(),
            pending_save: None,
            peek: None,
            #[cfg(not(target_os = "macos"))]
            contexts: HashMap::new(),
            #[cfg(target_os = "macos")]
            rules: None,
            quit: false,
        })
    }

    // ---------------------------------------------------------------- spaces, themes, profiles

    fn space_cfg(&self, i: usize) -> Option<&config::SpaceConfig> {
        let name = &self.state.spaces.get(i)?.name;
        self.config.spaces.iter().find(|s| &s.name == name)
    }

    fn theme_for(&self, i: usize) -> Theme {
        if let Some(n) = self.state.spaces.get(i).and_then(|s| s.theme.clone()) {
            return self.config.theme(Some(&ThemeRef::Named(n)));
        }
        match self.space_cfg(i).and_then(|s| s.theme.as_ref()) {
            Some(t) => self.config.theme(Some(t)),
            None => {
                let names = self.config.theme_names();
                self.config.theme(Some(&ThemeRef::Named(names[i % names.len()].clone())))
            }
        }
    }

    fn profile_for(&self, i: usize) -> String {
        self.space_cfg(i)
            .and_then(|s| s.profile.clone())
            .or_else(|| self.config.profiles.first().map(|p| p.name.clone()))
            .unwrap_or_else(|| "Default".into())
    }

    // ---------------------------------------------------------------- webviews

    /// Settings the in-page script needs: vim bindings, accent, boost CSS, and whether to report the screen edge.
    fn page_config(&self) -> Value {
        let l = &self.config.layout;
        let edge = (l.tab_bar == TabBar::Side && !l.sidebar_visible).then_some(match l.sidebar_position {
            SidebarPosition::Left => "left",
            SidebarPosition::Right => "right",
        });
        let css: serde_json::Map<String, Value> =
            self.boosts.iter().filter(|b| !b.css.is_empty()).map(|b| (b.host.clone(), json!(b.css))).collect();
        json!({ "vim": self.config.vim, "accent": self.theme_for(self.state.active_space).accent, "boost_css": css, "edge": edge, "peek": self.sidebar_peek })
    }

    fn content_script(&self) -> String {
        format!("(function (CFG) {{\n{}\n}})({});", ui("content.js"), self.page_config())
    }

    /// After each load: refresh page settings and run matching boost scripts (native eval, so page CSP can't block it).
    fn after_load(&self, view: &WebView, url: &str) {
        let _ = view.evaluate_script(&format!("window.__valerie && __valerie.configure({})", self.page_config()));
        let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        for b in self.boosts.iter().filter(|b| !b.js.is_empty() && boost_matches(&b.host, &host)) {
            let _ = view.evaluate_script(&format!("try {{\n{}\n}} catch (e) {{ console.error('valerie boost {}', e) }}", b.js, b.host));
        }
    }

    /// Run `f` with a builder bound to the profile's isolated storage: a WKWebsiteDataStore on macOS,
    /// a WebContext data directory elsewhere.
    fn with_profile<R>(&mut self, profile: &str, f: impl FnOnce(&Self, WebViewBuilder<'_>) -> R) -> R {
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewBuilderExtDarwin;
            f(self, WebViewBuilder::new().with_data_store_identifier(platform::profile_id(profile)))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let dir = config::data_dir().join("profiles").join(profile);
            let mut ctx = self.contexts.remove(profile).unwrap_or_else(|| wry::WebContext::new(Some(dir)));
            let r = f(self, WebViewBuilder::new_with_web_context(&mut ctx));
            self.contexts.insert(profile.to_string(), ctx);
            r
        }
    }

    fn page_builder<'a>(&self, base: WebViewBuilder<'a>, target: Target, url: &str) -> WebViewBuilder<'a> {
        let p = self.proxy.clone();
        let (p1, p2, p3, p4, p5) = (p.clone(), p.clone(), p.clone(), p.clone(), p.clone());
        let downloads = dirs::download_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default());
        base
            .with_url(if url.is_empty() { "about:blank" } else { url })
            .with_visible(false)
            .with_devtools(true)
            .with_user_agent(&self.user_agent)
            .with_back_forward_navigation_gestures(true)
            .with_accept_first_mouse(true)
            .with_initialization_script(self.content_script())
            .with_ipc_handler(move |r| send(&p1, UserEvent::Ipc { from: Source::Page(target), msg: r.into_body() }))
            .with_on_page_load_handler(move |ev, url| {
                send(&p2, UserEvent::PageLoad { target, url, finished: matches!(ev, PageLoadEvent::Finished) })
            })
            .with_document_title_changed_handler(move |title| send(&p3, UserEvent::Title { target, title }))
            .with_new_window_req_handler(move |url, _| {
                send(&p4, UserEvent::NewWindow { target, url });
                NewWindowResponse::Deny
            })
            .with_navigation_handler(move |url| {
                let scheme = scheme_of(&url);
                if matches!(scheme.as_str(), "http" | "https" | "about" | "data" | "blob" | "file") {
                    return true;
                }
                send(&p5, UserEvent::ExternalScheme(url));
                false
            })
            .with_download_started_handler(move |url, path| {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .filter(|n| !n.is_empty())
                    .or_else(|| url.split('?').next().and_then(|u| u.rsplit('/').next()).map(str::to_string))
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| "download".into());
                let mut dest = downloads.join(&name);
                let mut i = 1;
                while dest.exists() {
                    let (stem, ext) = name.rsplit_once('.').map(|(s, e)| (s.to_string(), format!(".{e}"))).unwrap_or((name.clone(), String::new()));
                    dest = downloads.join(format!("{stem} ({i}){ext}"));
                    i += 1;
                }
                *path = dest;
                true
            })
            .with_download_completed_handler(move |_, _, ok| send(&p, UserEvent::Downloaded { ok }))
    }

    fn ensure_view(&mut self, tab: TabId) {
        if self.views.contains_key(&tab) {
            return;
        }
        let Some(t) = self.state.tabs.get(&tab) else { return };
        let space = self.state.space_of(tab).unwrap_or(self.state.active_space);
        let profile = self.profile_for(space);
        let url = t.url.clone();
        let built = self.with_profile(&profile, |app, base| app.page_builder(base, Target::Tab(tab), &url).build_as_child(&app.window));
        match built {
            Ok(v) => {
                if self.config.layout.corner_radius > 0.0 {
                    platform::round_corners(&v, self.config.layout.corner_radius);
                }
                if let Some(z) = self.zoom.get(&tab) {
                    let _ = v.zoom(*z);
                }
                #[cfg(target_os = "macos")]
                if self.rules.is_some() {
                    blocking::mac::apply(&v, self.rules.as_ref());
                }
                self.views.insert(tab, v);
            }
            Err(e) => eprintln!("valerie: could not create webview: {e}"),
        }
    }

    fn unload(&mut self, tab: TabId) {
        self.views.remove(&tab);
        if let Some(t) = self.state.tabs.get_mut(&tab) {
            t.loading = false;
            t.playing = false;
        }
    }

    // ---------------------------------------------------------------- layout

    fn panes(&self) -> Vec<TabId> {
        let s = self.state.space();
        match self.state.active_tab() {
            None => vec![],
            Some(a) if s.split.len() > 1 && s.split.contains(&a) => s.split.clone(),
            Some(a) => vec![a],
        }
    }

    fn content_area(&self, w: f64, h: f64) -> (f64, f64, f64, f64) {
        let l = &self.config.layout;
        let b = l.bezel;
        match l.tab_bar {
            TabBar::Top => (b, TOP_BAR, w - 2.0 * b, h - TOP_BAR - b),
            TabBar::Side => {
                let sw = if l.sidebar_visible || self.sidebar_peek { l.sidebar_width } else { 0.0 };
                let (left, right) = match l.sidebar_position {
                    SidebarPosition::Left => (if sw > 0.0 { sw } else { b }, b),
                    SidebarPosition::Right => (b, if sw > 0.0 { sw } else { b }),
                };
                (left, b, w - left - right, h - 2.0 * b)
            }
        }
    }

    fn layout(&mut self) {
        self.layout_anim(0.0)
    }

    /// Lay out every view; with `secs > 0` visible panes glide to their new bounds instead of jumping.
    fn layout_anim(&mut self, secs: f64) {
        let size = self.window.inner_size().to_logical::<f64>(self.window.scale_factor());
        let full = rect(0.0, 0.0, size.width, size.height);
        let _ = self.chrome.set_bounds(full);
        let _ = self.overlay.set_bounds(full);
        let (x, y, w, h) = self.content_area(size.width, size.height);
        let panes = self.panes();
        let n = panes.len().max(1) as f64;
        let gap = self.config.layout.split_gap.max(0.0);
        let pw = (w - gap * (n - 1.0)) / n;
        for (id, v) in &self.views {
            if !panes.contains(id) {
                let _ = v.set_visible(false);
            }
        }
        for (i, id) in panes.iter().enumerate() {
            if let Some(v) = self.views.get(id) {
                let px = x + i as f64 * (pw + gap);
                if secs <= 0.0 || !platform::animate_bounds(v, px, y, pw, h, secs) {
                    let _ = v.set_bounds(rect(px, y, pw, h));
                }
                let _ = v.set_visible(true);
            }
        }
        let hide_lights = self.config.layout.tab_bar == TabBar::Side
            && (!self.config.layout.sidebar_visible || self.config.layout.sidebar_position == SidebarPosition::Right);
        platform::set_traffic_lights_hidden(&self.window, hide_lights);
        if let Some(p) = &self.peek {
            let (px, py, pw, ph) = self.peek_rect();
            let _ = p.view.set_bounds(rect(px, py, pw, ph));
        }
        if self.bar_open {
            platform::bring_to_front(&self.overlay);
        }
    }

    fn layout_little(&self, id: WindowId) {
        if let Some(l) = self.littles.get(&id) {
            let s = l.window.inner_size().to_logical::<f64>(l.window.scale_factor());
            let _ = l.bar.set_bounds(rect(0.0, 0.0, s.width, LITTLE_BAR));
            let _ = l.view.set_bounds(rect(0.0, LITTLE_BAR, s.width, s.height - LITTLE_BAR));
        }
    }

    fn focus_active(&self) {
        if self.bar_open {
            return;
        }
        match self.state.active_tab().and_then(|t| self.views.get(&t)) {
            Some(v) => {
                let _ = v.focus();
            }
            None => {
                let _ = self.chrome.focus();
            }
        }
    }

    // ---------------------------------------------------------------- rendering

    fn tab_vm(&self, t: &Tab) -> Value {
        json!({
            "id": t.id, "url": t.url, "favicon": t.favicon,
            "title": t.custom_title.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| t.title.clone()),
            "loaded": self.views.contains_key(&t.id), "loading": t.loading, "playing": t.playing,
            "pinned": self.state.is_pinned(t.id),
        })
    }

    fn node_vm(&self, n: &Node) -> Option<Value> {
        match n {
            Node::Tab { id } => {
                let mut v = self.tab_vm(self.state.tabs.get(id)?);
                v["type"] = json!("tab");
                Some(v)
            }
            Node::Folder { id, name, open, children } => Some(json!({
                "type": "folder", "id": id, "name": name, "open": open,
                "children": children.iter().filter_map(|c| self.node_vm(c)).collect::<Vec<_>>(),
            })),
        }
    }

    fn render(&self) {
        let s = self.state.space();
        let active = self.state.active_tab();
        let vm = json!({
            "spaces": self.state.spaces.iter().enumerate().map(|(i, sp)| json!({
                "name": sp.name, "icon": self.space_cfg(i).and_then(|c| c.icon.clone()),
            })).collect::<Vec<_>>(),
            "active_space": self.state.active_space,
            "theme": self.theme_for(self.state.active_space),
            "layout": self.config.layout,
            "favorites": self.state.favorites.iter().filter_map(|id| self.state.tabs.get(id)).map(|t| self.tab_vm(t)).collect::<Vec<_>>(),
            "pinned": s.pinned.iter().filter_map(|n| self.node_vm(n)).collect::<Vec<_>>(),
            "today": s.today.iter().filter_map(|id| self.state.tabs.get(id)).map(|t| self.tab_vm(t)).collect::<Vec<_>>(),
            "active": active,
            "split": if s.split.len() > 1 { s.split.clone() } else { vec![] },
            "url": active.and_then(|a| self.state.tabs.get(&a)).map(|t| t.url.clone()),
            "agents": self.agents_running,
            "peek": self.sidebar_peek,
        });
        let _ = self.chrome.evaluate_script(&format!("window.valerie && valerie.render({vm})"));
    }

    fn render_little(&self, id: WindowId) {
        if let Some(l) = self.littles.get(&id) {
            let vm = json!({
                "url": l.url, "title": l.title, "theme": self.theme_for(l.space),
                "space": self.state.spaces.get(l.space).map(|s| s.name.clone()),
            });
            let _ = l.bar.evaluate_script(&format!("window.little && little.render({vm})"));
        }
    }

    fn toast(&self, msg: &str) {
        let m = serde_json::to_string(msg).unwrap_or_default();
        let in_page = self.state.active_tab().and_then(|t| self.views.get(&t));
        match in_page {
            Some(v) if self.focused_little.is_none() => {
                let _ = v.evaluate_script(&format!("window.__valerie ? __valerie.toast({m}) : 0"));
            }
            _ => {
                let _ = self.chrome.evaluate_script(&format!("window.valerie && valerie.toast({m})"));
            }
        }
        if let Some(l) = self.focused_little.and_then(|id| self.littles.get(&id)) {
            let _ = l.view.evaluate_script(&format!("window.__valerie ? __valerie.toast({m}) : 0"));
        }
    }

    // ---------------------------------------------------------------- tab operations

    fn activate(&mut self, tab: TabId) {
        if !self.state.tabs.contains_key(&tab) {
            return;
        }
        if let Some(prev) = self.state.active_tab() {
            if let Some(t) = self.state.tabs.get_mut(&prev) {
                t.last_active = now();
            }
        }
        if let Some(sp) = self.state.space_of(tab) {
            self.state.active_space = sp;
        }
        self.state.space_mut().active = Some(tab);
        if let Some(t) = self.state.tabs.get_mut(&tab) {
            t.last_active = now();
        }
        for p in self.panes() {
            self.ensure_view(p);
        }
        self.layout();
        self.focus_active();
        self.render();
        self.dirty = true;
    }

    fn open_tab(&mut self, url: &str, space: Option<usize>, activate: bool) -> TabId {
        let sp = space.unwrap_or(self.state.active_space);
        let at_top = self.config.tabs.new_tab_position != "bottom";
        let id = self.state.add_tab(sp, url, at_top);
        if activate {
            self.activate(id);
        } else {
            self.ensure_view(id);
            self.layout();
            self.render();
        }
        self.dirty = true;
        id
    }

    fn navigate(&mut self, tab: TabId, url: &str) {
        if let Some(t) = self.state.tabs.get_mut(&tab) {
            t.url = url.to_string();
        }
        match self.views.get(&tab) {
            Some(v) => {
                let _ = v.load_url(url);
            }
            None => self.ensure_view(tab),
        }
        self.layout();
        self.render();
    }

    fn close(&mut self, tab: TabId) {
        let pinned_or_fav = self.state.is_pinned(tab) || self.state.favorites.contains(&tab);
        if pinned_or_fav {
            // Like Arc: closing a pinned tab unloads it but keeps it in the sidebar.
            let was_active = self.state.active_tab() == Some(tab);
            self.unload(tab);
            if was_active {
                let order: Vec<TabId> = self.state.order(self.state.active_space, true);
                let next = order.iter().copied().find(|t| *t != tab && self.views.contains_key(t));
                self.state.space_mut().active = next;
                if let Some(n) = next {
                    return self.activate(n);
                }
            }
        } else {
            let space = self.state.space_of(tab);
            let next = self.state.close_tab(tab);
            self.views.remove(&tab);
            if space == Some(self.state.active_space) || space.is_none() {
                if let Some(n) = next {
                    return self.activate(n);
                }
            }
        }
        let split = &mut self.state.space_mut().split;
        if split.len() < 2 {
            split.clear();
        }
        self.layout();
        self.focus_active();
        self.render();
        self.dirty = true;
    }

    fn anim_secs(&self) -> f64 {
        self.config.layout.animation_ms.max(0.0) / 1000.0
    }

    fn switch_space(&mut self, i: usize) {
        if i >= self.state.spaces.len() {
            return;
        }
        let changed = i != self.state.active_space;
        self.state.active_space = i;
        for p in self.panes() {
            self.ensure_view(p);
        }
        self.layout();
        if changed && self.anim_secs() > 0.0 {
            for p in self.panes() {
                if let Some(v) = self.views.get(&p) {
                    platform::fade_in(v, self.anim_secs());
                }
            }
        }
        self.focus_active();
        self.render();
        self.refresh_content_config();
        self.dirty = true;
    }

    fn cycle_list(&self) -> Vec<TabId> {
        let mut v = self.state.favorites.clone();
        v.extend(self.state.order(self.state.active_space, true));
        v
    }

    fn target_tab(&self, tab: Option<TabId>) -> Option<TabId> {
        tab.or_else(|| self.state.active_tab())
    }

    fn move_tab(&mut self, tab: TabId, space: Option<String>, folder: Option<TabId>, pinned: Option<bool>, index: Option<usize>) {
        if !self.state.tabs.contains_key(&tab) {
            return;
        }
        let from = self.state.space_of(tab);
        let was_active = from.map(|s| self.state.spaces[s].active == Some(tab)).unwrap_or(false);
        let dest = space.and_then(|n| self.state.space_index(&n)).or(from).unwrap_or(self.state.active_space);
        let pinned = pinned.unwrap_or(folder.is_some() || self.state.is_pinned(tab));
        self.state.detach(tab);
        let sp = &mut self.state.spaces[dest];
        if let Some(f) = folder.and_then(|f| find_folder(&mut sp.pinned, f)) {
            if let Node::Folder { children, open, .. } = f {
                let i = index.unwrap_or(children.len()).min(children.len());
                children.insert(i, Node::Tab { id: tab });
                *open = true;
            }
        } else if pinned {
            let i = index.unwrap_or(sp.pinned.len()).min(sp.pinned.len());
            sp.pinned.insert(i, Node::Tab { id: tab });
        } else {
            let i = index.unwrap_or(0).min(sp.today.len());
            sp.today.insert(i, tab);
        }
        if was_active && Some(dest) == from {
            self.state.spaces[dest].active = Some(tab);
        } else if was_active {
            // The tab left this space: its view must be moved to the new space's profile, so drop it.
            self.unload(tab);
            let next = self.state.order(from.unwrap(), true).first().copied();
            self.state.spaces[from.unwrap()].active = next;
            self.toast(&format!("Moved to {}", self.state.spaces[dest].name));
        }
        if from != Some(dest) {
            self.unload(tab);
        }
        for p in self.panes() {
            self.ensure_view(p);
        }
        self.layout();
        self.render();
        self.dirty = true;
    }

    // ---------------------------------------------------------------- command bar

    fn open_bar(&mut self, mode: BarMode) {
        let active = self.state.active_tab();
        let mode = if mode == BarMode::Current && active.is_none() { BarMode::NewTab } else { mode };
        let mut items: Vec<Value> = Vec::new();
        let mut rank = 0;
        let mut recent: Vec<&Tab> = self.state.tabs.values().collect();
        recent.sort_by(|a, b| b.last_active.cmp(&a.last_active));
        for t in recent {
            let space = self.state.space_of(t.id);
            let kind = if self.state.favorites.contains(&t.id) {
                "favorite"
            } else if self.state.is_pinned(t.id) {
                "pinned"
            } else {
                "tab"
            };
            if Some(t.id) == active {
                continue;
            }
            items.push(json!({ "kind": kind, "title": t.display_title(), "url": t.url, "tab": t.id, "favicon": t.favicon, "rank": rank,
                               "sub": space.map(|s| self.state.spaces[s].name.clone()) }));
            rank += 1;
        }
        for (name, title) in CATALOG {
            items.push(json!({ "kind": "command", "title": title, "cmd": { "cmd": name } }));
        }
        for (i, s) in self.state.spaces.iter().enumerate() {
            items.push(json!({ "kind": "space", "title": format!("Switch to {}", s.name), "cmd": { "cmd": "switch_space", "index": i } }));
        }
        for name in self.config.theme_names() {
            items.push(json!({ "kind": "theme", "title": format!("Space theme: {name}"), "cmd": { "cmd": "set_space_theme", "theme": name } }));
        }
        for src in import::available() {
            let title = format!("Import from {}{}", &src[..1].to_uppercase(), &src[1..]);
            items.push(json!({ "kind": "import", "title": title, "cmd": { "cmd": "import", "source": src } }));
        }
        for j in &self.config.agents.jobs {
            items.push(json!({ "kind": "agent", "title": format!("Run agent: {}", j.name), "sub": j.prompt, "cmd": { "cmd": "run_agent", "name": j.name } }));
        }
        for (i, r) in self.agent_runs.iter().enumerate() {
            items.push(json!({ "kind": "agent", "title": format!("Agent run: {} ({})", r.name, if r.ok { "ok" } else { "failed" }),
                               "sub": r.summary.replace('\n', " "), "cmd": { "cmd": "show_agent_run", "index": i } }));
        }
        if self.config.passwords.enabled {
            for e in passwords::list(&self.config.vaults_for(&self.profile_for(self.state.active_space))) {
                let host = url::Url::parse(&e.origin).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
                let cmd = json!({ "cmd": "copy_password", "vault": e.vault, "origin": e.origin, "username": e.username });
                items.push(json!({ "kind": "password", "title": format!("Password: {host} — {}", e.username), "sub": e.vault, "cmd": cmd }));
            }
        }
        for b in &self.state.bookmarks {
            items.push(json!({ "kind": "bookmark", "title": b.title, "url": b.url, "sub": b.folder.join(" / ") }));
        }
        for h in self.state.history.iter().take(3000) {
            items.push(json!({ "kind": "history", "title": h.title, "url": h.url, "visits": h.visits }));
        }
        for d in &self.remote {
            for s in &d.spaces {
                for t in s.pinned.iter().chain(&s.tabs) {
                    items.push(json!({ "kind": "synced", "title": t.title, "url": t.url, "sub": format!("{} · {}", d.device, s.name) }));
                }
            }
        }
        for a in self.state.archive.iter().take(500) {
            items.push(json!({ "kind": "archived", "title": a.title, "url": a.url, "sub": format!("Archived from {}", a.space) }));
        }
        let engine = url::Url::parse(&self.config.search_engine.replace("%s", "x"))
            .ok()
            .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").split('.').next().unwrap_or(h).to_string()))
            .map(|h| format!("{}{}", &h[..1].to_uppercase(), &h[1..]))
            .unwrap_or_else(|| "the web".into());
        let query = if mode == BarMode::Current { active.and_then(|t| self.state.tabs.get(&t)).map(|t| t.url.clone()) } else { None };
        let data = json!({ "mode": mode, "query": query, "engine": engine, "agent": self.config.agents.default, "theme": self.theme_for(self.state.active_space), "items": items });
        let _ = self.overlay.evaluate_script(&format!("palette.open({data})"));
        let _ = self.overlay.set_visible(true);
        platform::bring_to_front(&self.overlay);
        let _ = self.overlay.focus();
        self.bar_open = true;
    }

    fn close_bar(&mut self) {
        if !self.bar_open {
            return;
        }
        self.bar_open = false;
        let _ = self.overlay.evaluate_script("palette.hide()");
        let _ = self.overlay.set_visible(false);
        self.focus_active();
    }

    // ---------------------------------------------------------------- little windows

    fn open_little(&mut self, url: &str, space: usize, elwt: &EventLoopWindowTarget<UserEvent>) {
        let lw = &self.config.little_window;
        let window = match mac_window(WindowBuilder::new())
            .with_title("Valerie")
            .with_inner_size(LogicalSize::new(lw.width, lw.height))
            .with_min_inner_size(LogicalSize::new(360.0, 240.0))
            .build(elwt)
        {
            Ok(w) => w,
            Err(e) => return eprintln!("valerie: little window: {e}"),
        };
        let id = window.id();
        let p = self.proxy.clone();
        let bar = WebViewBuilder::new()
            .with_html(ui("little.html"))
            .with_bounds(rect(0.0, 0.0, lw.width, LITTLE_BAR))
            .with_accept_first_mouse(true)
            .with_ipc_handler(move |r| send(&p, UserEvent::Ipc { from: Source::LittleBar(id), msg: r.into_body() }))
            .build_as_child(&window);
        let (lw_w, lw_h) = (lw.width, lw.height);
        let profile = self.profile_for(space);
        let view = self.with_profile(&profile, |app, base| {
            app.page_builder(base, Target::Little(id), url)
                .with_visible(true)
                .with_bounds(rect(0.0, LITTLE_BAR, lw_w, lw_h - LITTLE_BAR))
                .build_as_child(&window)
        });
        let (Ok(bar), Ok(view)) = (bar, view) else { return eprintln!("valerie: little window webviews failed") };
        let _ = view.focus();
        window.set_focus();
        #[cfg(target_os = "macos")]
        if self.rules.is_some() {
            blocking::mac::apply(&view, self.rules.as_ref());
        }
        self.littles.insert(id, Little { window, bar, view, url: url.to_string(), title: String::new(), space });
        self.focused_little = Some(id);
    }

    fn little_to_main(&mut self, id: WindowId) {
        if let Some(l) = self.littles.remove(&id) {
            let url = l.url.clone();
            drop(l);
            self.focused_little = None;
            self.open_tab(&url, Some(self.state.active_space), true);
            self.window.set_focus();
            self.focus_active();
        }
    }

    /// Links from other apps: Air Traffic Control first, then a little window.
    fn open_external(&mut self, urls: Vec<String>, elwt: &EventLoopWindowTarget<UserEvent>) {
        for url in urls {
            let routed = routing::route(&self.config.routes, &url).and_then(|s| self.state.space_index(s));
            match routed {
                Some(space) => {
                    self.open_tab(&url, Some(space), true);
                    self.window.set_focus();
                }
                None if self.config.little_window.enabled => self.open_little(&url, self.state.active_space, elwt),
                None => {
                    self.open_tab(&url, None, true);
                    self.window.set_focus();
                }
            }
        }
    }

    // ---------------------------------------------------------------- config, menu, sync

    fn refresh_content_config(&self) {
        let cfg = self.page_config();
        for v in self.views.values() {
            let _ = v.evaluate_script(&format!("window.__valerie && __valerie.configure({cfg})"));
        }
    }

    fn reload_config(&mut self) {
        self.boosts = load_boosts();
        match Config::load() {
            Ok(c) => {
                let radius_changed = c.layout.corner_radius != self.config.layout.corner_radius;
                let blocking_changed = serde_json::to_value(&c.content_blocking).ok() != serde_json::to_value(&self.config.content_blocking).ok();
                self.config = c;
                self.state.reconcile_spaces(&self.config);
                if radius_changed {
                    for v in self.views.values() {
                        platform::round_corners(v, self.config.layout.corner_radius);
                    }
                }
                self.build_menu();
                if blocking_changed {
                    self.start_blocklist();
                }
                self.refresh_content_config();
                self.layout();
                self.render();
                self.toast("Config reloaded");
            }
            Err(e) => self.toast(&format!("Config error: {e:#}")),
        }
    }

    fn command_label(&self, cmd: &Command) -> String {
        match cmd {
            Command::SwitchSpace { index: Some(i), .. } => {
                format!("Switch to {}", self.state.spaces.get(*i).map(|s| s.name.clone()).unwrap_or(format!("Space {}", i + 1)))
            }
            Command::GoToTab { index } => format!("Go to Tab {}", index + 1),
            Command::OpenCommandBar { mode: BarMode::Current } => "Edit URL".into(),
            Command::OpenCommandBar { .. } => "Command Bar".into(),
            c => {
                let name = serde_json::to_value(c).ok().and_then(|v| v["cmd"].as_str().map(str::to_string)).unwrap_or_default();
                title_for(&name)
            }
        }
    }

    fn build_menu(&mut self) {
        let menu = Menu::new();
        let mut cmds = HashMap::new();
        let app_menu = Submenu::new("Valerie", true);
        let settings = MenuItem::with_id("settings", "Settings…", true, None);
        cmds.insert(settings.id().clone(), Command::OpenConfig);
        let quit = MenuItem::with_id("quit", "Quit Valerie", true, "CmdOrCtrl+Q".parse().ok());
        cmds.insert(quit.id().clone(), Command::Quit);
        let _ = app_menu.append_items(&[
            &PredefinedMenuItem::about(None, None),
            &PredefinedMenuItem::separator(),
            &settings,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &quit,
        ]);
        let edit = Submenu::new("Edit", true);
        let _ = edit.append_items(&[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
        ]);
        let groups = [Submenu::new("Tabs", true), Submenu::new("Spaces", true), Submenu::new("View", true), Submenu::new("Tools", true)];
        let mut bound: Vec<Command> = Vec::new();
        for (i, (accel, binding)) in self.config.keybindings.iter().enumerate() {
            let Some(cmd) = binding.as_ref().and_then(|b| b.command()) else { continue };
            let Ok(acc) = accel.parse::<muda::accelerator::Accelerator>() else {
                eprintln!("valerie: bad keybinding {accel:?}");
                continue;
            };
            let item = MenuItem::with_id(format!("k{i}"), self.command_label(&cmd), true, Some(acc));
            cmds.insert(item.id().clone(), cmd.clone());
            let g = match &cmd {
                Command::NextSpace | Command::PrevSpace | Command::SwitchSpace { .. } | Command::NewSpace { .. } => 1,
                Command::ToggleSidebar | Command::SplitView { .. } | Command::Unsplit | Command::FocusNextPane | Command::ZoomIn
                | Command::ZoomOut | Command::ZoomReset | Command::Reload { .. } | Command::FindInPage { .. } => 2,
                Command::DevTools | Command::OpenConfig | Command::Sync | Command::CopyUrl => 3,
                _ => 0,
            };
            let _ = groups[g].append(&item);
            bound.push(cmd);
        }
        // Unbound catalog commands still appear in the menu so everything is discoverable.
        for (name, title) in CATALOG {
            let Some(cmd) = Command::from_name(name) else { continue };
            if bound.contains(&cmd) || matches!(cmd, Command::Quit | Command::OpenConfig) {
                continue;
            }
            let item = MenuItem::with_id(format!("c-{name}"), *title, true, None);
            cmds.insert(item.id().clone(), cmd);
            let _ = groups[3].append(&item);
        }
        let window_menu = Submenu::new("Window", true);
        let _ = window_menu.append_items(&[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::fullscreen(None)]);
        let _ = menu.append(&app_menu);
        let _ = menu.append(&edit);
        for g in &groups {
            let _ = menu.append(g);
        }
        let _ = menu.append(&window_menu);
        #[cfg(target_os = "macos")]
        {
            menu.init_for_nsapp();
            window_menu.set_as_windows_menu_for_nsapp();
        }
        self.menu_cmds = cmds;
        self._menu = Some(menu);
    }

    fn view_for(&self, target: Target) -> Option<&WebView> {
        match target {
            Target::Tab(id) => self.views.get(&id),
            Target::Little(id) => self.littles.get(&id).map(|l| &l.view),
            Target::Peek => self.peek.as_ref().map(|p| &p.view),
        }
    }

    fn vaults_for_target(&self, target: Target) -> Vec<String> {
        let space = match target {
            Target::Tab(id) => self.state.space_of(id).unwrap_or(self.state.active_space),
            Target::Little(id) => self.littles.get(&id).map(|l| l.space).unwrap_or(self.state.active_space),
            Target::Peek => self.state.active_space,
        };
        self.config.vaults_for(&self.profile_for(space))
    }

    /// The page's real origin, read from the webview rather than trusting the page.
    fn origin_of_target(&self, target: Target) -> Option<String> {
        self.view_for(target).and_then(|v| v.url().ok()).and_then(|u| passwords::origin_of(&u))
    }

    fn offer_pending_save(&self) {
        let Some(p) = &self.pending_save else { return };
        if now().saturating_sub(p.at) > 120 || self.origin_of_target(p.target).is_none() {
            return;
        }
        let host = url::Url::parse(&p.origin).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        let args = json!([host, p.username, p.nonce, self.vaults_for_target(p.target)]);
        if let Some(v) = self.view_for(p.target) {
            let _ = v.evaluate_script(&format!("window.__valerie && __valerie.offerSave(...{args})"));
        }
    }

    fn handle_password_ipc(&mut self, target: Target, kind: &str, m: &Value) {
        if !self.config.passwords.enabled {
            return;
        }
        let Some(origin) = self.origin_of_target(target) else { return };
        match kind {
            "login_form" => {
                let entries: Vec<passwords::Entry> =
                    passwords::list(&self.vaults_for_target(target)).into_iter().filter(|e| e.origin == origin).collect();
                if entries.is_empty() {
                    return;
                }
                let nonce = crate::mcp::random_hex();
                let list = json!(entries.iter().map(|e| json!({ "username": e.username, "vault": e.vault })).collect::<Vec<_>>());
                if let Some(v) = self.view_for(target) {
                    let _ = v.evaluate_script(&format!("window.__valerie && __valerie.accounts({list}, {})", json!(nonce)));
                }
                self.fill_offers.insert(target, (nonce, entries));
            }
            "fill" => {
                let Some((nonce, entries)) = self.fill_offers.get(&target) else { return };
                if m["nonce"].as_str() != Some(nonce.as_str()) {
                    return;
                }
                let Some(e) = m["index"].as_u64().and_then(|i| entries.get(i as usize)).filter(|e| e.origin == origin) else { return };
                match passwords::password(e) {
                    Ok(Some(pw)) => {
                        if let Some(v) = self.view_for(target) {
                            let _ = v.evaluate_script(&format!("window.__valerie && __valerie.fill({}, {})", json!(e.username), json!(pw)));
                        }
                    }
                    Ok(None) => self.toast("That password is no longer in the keychain"),
                    Err(err) => self.toast(&format!("Keychain: {err}")),
                }
            }
            "credential" => {
                let (Some(username), Some(password)) = (m["username"].as_str(), m["password"].as_str()) else { return };
                if password.is_empty() || password.len() > 1024 || self.state.password_never.contains(&origin) {
                    return;
                }
                let vaults = self.vaults_for_target(target);
                let known = passwords::list(&vaults)
                    .into_iter()
                    .filter(|e| e.origin == origin && e.username == username)
                    .any(|e| passwords::password(&e).ok().flatten().as_deref() == Some(password));
                if known {
                    return;
                }
                self.pending_save = Some(PendingSave {
                    target,
                    origin,
                    username: username.to_string(),
                    password: password.to_string(),
                    nonce: crate::mcp::random_hex(),
                    at: now(),
                    reoffered: false,
                });
                self.offer_pending_save();
            }
            "save_password" | "never_password" | "dismiss_password" => {
                let Some(p) = self.pending_save.take() else { return };
                if m["nonce"].as_str() != Some(p.nonce.as_str()) || p.target != target {
                    self.pending_save = Some(p);
                    return;
                }
                if kind == "dismiss_password" {
                    return;
                }
                if kind == "never_password" {
                    self.state.password_never.push(p.origin);
                    self.dirty = true;
                    return;
                }
                let vault = m["vault"].as_str().unwrap_or_default().to_string();
                if !self.vaults_for_target(target).contains(&vault) {
                    return;
                }
                match passwords::save(&vault, &p.origin, &p.username, &p.password) {
                    Ok(()) => self.toast(&format!("Password saved to {vault}")),
                    Err(e) => self.toast(&format!("Could not save password: {e}")),
                }
            }
            _ => {}
        }
    }

    fn peek_rect(&self) -> (f64, f64, f64, f64) {
        let size = self.window.inner_size().to_logical::<f64>(self.window.scale_factor());
        let (x, y, w, h) = self.content_area(size.width, size.height);
        let (mx, my) = ((w * 0.06).max(16.0), (h * 0.05).max(16.0));
        (x + mx, y + my, w - 2.0 * mx, h - 2.0 * my)
    }

    fn open_peek(&mut self, url: &str) {
        self.close_peek();
        let (x, y, w, h) = self.peek_rect();
        let profile = self.profile_for(self.state.active_space);
        let built = self.with_profile(&profile, |app, base| {
            app.page_builder(base, Target::Peek, url).with_visible(true).with_bounds(rect(x, y, w, h)).build_as_child(&app.window)
        });
        let view = match built {
            Ok(v) => v,
            Err(e) => return eprintln!("valerie: peek: {e}"),
        };
        platform::round_corners(&view, 12.0);
        #[cfg(target_os = "macos")]
        if self.rules.is_some() {
            blocking::mac::apply(&view, self.rules.as_ref());
        }
        for p in self.panes() {
            if let Some(v) = self.views.get(&p) {
                platform::set_alpha(v, 0.45, self.anim_secs());
            }
        }
        platform::fade_in(&view, self.anim_secs());
        let _ = view.focus();
        self.peek = Some(PeekView { view, url: url.to_string() });
        let _ = self.peek.as_ref().map(|p| p.view.evaluate_script("setTimeout(() => window.__valerie && __valerie.toast('Peek · esc to close · ⌘O to open as tab'), 400)"));
    }

    fn close_peek(&mut self) {
        if self.peek.take().is_none() {
            return;
        }
        for p in self.panes() {
            if let Some(v) = self.views.get(&p) {
                platform::set_alpha(v, 1.0, self.anim_secs());
            }
        }
        self.focus_active();
    }

    fn start_agent(&mut self, name: String, prompt: String, agent: Option<String>, model: Option<String>) {
        if self.agents_running.contains(&name) {
            return self.toast(&format!("{name} is already running"));
        }
        let agent = agent.unwrap_or_else(|| self.config.agents.default.clone());
        let space = self.config.agents.space.clone();
        if self.state.space_index(&space).is_none() {
            self.state.spaces.push(crate::state::Space { name: space.clone(), ..Default::default() });
            self.dirty = true;
        }
        self.state.agent_last.insert(name.clone(), now());
        self.agents_running.push(name.clone());
        self.toast(&format!("Agent started: {name}"));
        self.render();
        let p = self.proxy.clone();
        let model = model.unwrap_or_else(|| self.config.agents.model.clone());
        let run = agents::Run { name, agent, model, prompt, space };
        std::thread::spawn(move || send(&p, UserEvent::AgentDone(agents::run(run))));
    }

    fn schedule_agents(&mut self) {
        let t = now();
        let mut hhmm: Option<String> = None;
        let mut due = Vec::new();
        for job in self.config.agents.jobs.iter().filter(|j| j.enabled) {
            let last = self.state.agent_last.get(&job.name).copied();
            let by_interval = job.every.as_deref().and_then(config::parse_interval).is_some_and(|secs| match last {
                Some(l) => t.saturating_sub(l) >= secs,
                None => false,
            });
            let by_time = job.at.as_deref().is_some_and(|at| {
                let now_hhmm = hhmm.get_or_insert_with(agents::local_hhmm);
                now_hhmm == at && last.is_none_or(|l| t.saturating_sub(l) > 120)
            });
            if last.is_none() && job.every.is_some() {
                // First sighting starts the clock rather than firing immediately.
                self.state.agent_last.insert(job.name.clone(), t);
                self.dirty = true;
            }
            if by_interval || by_time {
                due.push(job.clone());
            }
        }
        for job in due {
            self.start_agent(job.name, job.prompt, job.agent, job.model);
        }
    }

    /// Fetch lists and build rules off the main thread; WebKit compiles them asynchronously afterwards.
    fn start_blocklist(&mut self) {
        let cb = self.config.content_blocking.clone();
        if !cb.enabled {
            #[cfg(target_os = "macos")]
            {
                self.rules = None;
                self.apply_rules();
            }
            return;
        }
        let mut allow = cb.allow.clone();
        allow.extend(self.state.blocking_allow.iter().cloned());
        let p = self.proxy.clone();
        std::thread::spawn(move || match blocking::fetch(&cb.lists, cb.refresh_days) {
            Ok(domains) => {
                let json = blocking::rules_json(&domains, &allow);
                let hash = blocking::hash(&json);
                let unchanged = blocking::stored_hash().as_deref() == Some(hash.as_str());
                send(&p, UserEvent::BlocklistBuilt { json: (!unchanged).then_some(json), hash });
            }
            Err(e) => eprintln!("valerie: content blocking disabled: {e:#}"),
        });
    }

    #[cfg(target_os = "macos")]
    fn apply_rules(&self) {
        for v in self.views.values() {
            blocking::mac::apply(v, self.rules.as_ref());
        }
        for l in self.littles.values() {
            blocking::mac::apply(&l.view, self.rules.as_ref());
        }
    }

    fn start_sync(&mut self) {
        let repo = self.config.sync.repo.clone();
        if repo.is_empty() {
            return self.toast("Set sync.repo in config.yaml (or run `valerie sync-setup`)");
        }
        if self.syncing {
            return;
        }
        self.syncing = true;
        self.last_sync = now();
        let me = sync::DeviceTabs {
            device: self.config.device_name(),
            updated: now(),
            spaces: self
                .state
                .spaces
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let tab = |id: &TabId| self.state.tabs.get(id).map(|t| sync::SyncedTab { title: t.display_title().into(), url: t.url.clone() });
                    let mut pinned_ids = Vec::new();
                    let all = self.state.order(i, false);
                    for id in &all {
                        if !s.today.contains(id) {
                            pinned_ids.push(*id);
                        }
                    }
                    sync::SyncedSpace {
                        name: s.name.clone(),
                        pinned: pinned_ids.iter().filter_map(tab).collect(),
                        tabs: s.today.iter().filter_map(tab).collect(),
                    }
                })
                .collect(),
        };
        let dir = config::data_dir().join("sync");
        let p = self.proxy.clone();
        std::thread::spawn(move || send(&p, UserEvent::SyncDone(sync::run(&repo, &dir, &me).map_err(|e| format!("{e:#}")))));
    }

    fn save(&mut self) {
        if let Err(e) = self.state.save(&state_path()) {
            eprintln!("valerie: could not save state: {e}");
        }
        self.dirty = false;
    }

    fn tick(&mut self) {
        let t = now();
        let panes = self.panes();
        let archive_secs = (self.config.tabs.archive_after_hours * 3600.0) as u64;
        if archive_secs > 0 {
            let stale: Vec<TabId> = self
                .state
                .spaces
                .iter()
                .flat_map(|s| s.today.iter().copied().filter(|id| s.active != Some(*id) && !s.split.contains(id)))
                .filter(|id| self.state.tabs.get(id).is_some_and(|tab| !tab.playing && t.saturating_sub(tab.last_active) > archive_secs))
                .collect();
            for id in stale {
                self.state.archive_tab(id);
                self.views.remove(&id);
                self.dirty = true;
            }
        }
        let unload_secs = (self.config.tabs.unload_after_minutes * 60.0) as u64;
        if unload_secs > 0 {
            let idle: Vec<TabId> = self
                .views
                .keys()
                .copied()
                .filter(|id| !panes.contains(id))
                .filter(|id| self.state.tabs.get(id).is_some_and(|tab| !tab.playing && t.saturating_sub(tab.last_active) > unload_secs))
                .collect();
            for id in idle {
                self.unload(id);
            }
        }
        let interval = (self.config.sync.interval_minutes * 60.0) as u64;
        if !self.config.sync.repo.is_empty() && interval > 0 && t.saturating_sub(self.last_sync) > interval {
            self.start_sync();
        }
        self.schedule_agents();
        if self.dirty {
            self.save();
        }
        self.render();
    }

    // ---------------------------------------------------------------- dispatch

    pub fn dispatch(&mut self, cmd: Command, elwt: &EventLoopWindowTarget<UserEvent>) {
        if self.peek.is_some() {
            match &cmd {
                Command::MoveToMainWindow => {
                    let url = self.peek.as_ref().map(|p| p.url.clone()).unwrap_or_default();
                    self.close_peek();
                    self.open_tab(&url, None, true);
                    return;
                }
                Command::CloseTab { tab: None } | Command::ClosePeek => return self.close_peek(),
                Command::Back { tab: None } | Command::Forward { tab: None } | Command::Reload { tab: None } => {
                    if let Some(p) = &self.peek {
                        let _ = match cmd {
                            Command::Back { .. } => p.view.go_back(),
                            Command::Forward { .. } => p.view.go_forward(),
                            _ => p.view.reload(),
                        };
                    }
                    return;
                }
                Command::CopyUrl => {
                    let url = self.peek.as_ref().map(|p| p.url.clone()).unwrap_or_default();
                    let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(url));
                    return self.toast("Copied URL");
                }
                // Anything that changes what's on screen dismisses the preview first.
                _ => self.close_peek(),
            }
        }
        let little = self.focused_little.filter(|id| self.littles.contains_key(id));
        // Commands aimed at the focused little window.
        if let Some(id) = little {
            let l = &self.littles[&id];
            match &cmd {
                Command::CloseTab { tab: None } => {
                    self.littles.remove(&id);
                    self.focused_little = None;
                    return;
                }
                Command::MoveToMainWindow => return self.little_to_main(id),
                Command::CopyUrl => {
                    let url = l.url.clone();
                    let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(url));
                    return self.toast("Copied URL");
                }
                Command::Back { tab: None } => return drop(l.view.go_back()),
                Command::Forward { tab: None } => return drop(l.view.go_forward()),
                Command::Reload { tab: None } => return drop(l.view.reload()),
                Command::DevTools => return l.view.open_devtools(),
                _ => {}
            }
        }
        match cmd {
            Command::OpenCommandBar { mode } => {
                if little.is_some() {
                    self.window.set_focus();
                    self.focused_little = None;
                }
                self.open_bar(mode)
            }
            Command::CloseCommandBar => self.close_bar(),
            Command::NewTab { url, space, background } => {
                let space = space.and_then(|s| self.state.space_index(&s));
                let url = url.or_else(|| Some(self.config.new_tab_url.clone()).filter(|u| !u.is_empty()));
                match url {
                    Some(u) => {
                        let u = routing::resolve_input(&u, &self.config.search_engine);
                        self.open_tab(&u, space, !background);
                    }
                    None => {
                        if let Some(s) = space {
                            self.switch_space(s);
                        }
                        self.open_bar(BarMode::NewTab)
                    }
                }
            }
            Command::Go { input, new_tab } => {
                self.close_bar();
                let url = routing::resolve_input(&input, &self.config.search_engine);
                match self.state.active_tab().filter(|_| !new_tab) {
                    Some(t) => self.navigate(t, &url),
                    None => {
                        self.open_tab(&url, None, true);
                    }
                }
            }
            Command::Navigate { url, tab } => {
                if let Some(t) = self.target_tab(tab) {
                    let url = routing::resolve_input(&url, &self.config.search_engine);
                    self.navigate(t, &url);
                }
            }
            Command::CloseTab { tab } => {
                if let Some(t) = self.target_tab(tab) {
                    self.close(t);
                }
            }
            Command::ReopenClosedTab => {
                if let Some(c) = self.state.closed.pop() {
                    let space = c.space.min(self.state.spaces.len() - 1);
                    let id = c.tab.id;
                    self.state.tabs.insert(id, c.tab);
                    self.state.spaces[space].today.insert(0, id);
                    self.activate(id);
                }
            }
            Command::ActivateTab { tab } => {
                self.close_bar();
                self.activate(tab)
            }
            Command::NextTab | Command::PrevTab => {
                let list = self.cycle_list();
                if list.is_empty() {
                    return;
                }
                let cur = self.state.active_tab().and_then(|a| list.iter().position(|t| *t == a));
                let n = list.len();
                let next = match (cur, matches!(cmd, Command::NextTab)) {
                    (Some(i), true) => (i + 1) % n,
                    (Some(i), false) => (i + n - 1) % n,
                    (None, _) => 0,
                };
                self.activate(list[next]);
            }
            Command::GoToTab { index } => {
                if let Some(t) = self.cycle_list().get(index).copied() {
                    self.activate(t);
                }
            }
            Command::TogglePin { tab } => {
                if let Some(t) = self.target_tab(tab) {
                    if self.state.is_pinned(t) {
                        self.state.unpin(t);
                    } else {
                        if self.state.favorites.contains(&t) {
                            self.state.favorites.retain(|f| *f != t);
                        }
                        self.state.pin(t);
                    }
                    self.dirty = true;
                    self.render();
                }
            }
            Command::ToggleFavorite { tab } => {
                if let Some(t) = self.target_tab(tab) {
                    let space = self.state.space_of(t).unwrap_or(self.state.active_space);
                    let was_active = self.state.active_tab() == Some(t);
                    if self.state.favorites.contains(&t) {
                        self.state.detach(t);
                        self.state.spaces[space].today.insert(0, t);
                    } else {
                        self.state.detach(t);
                        self.state.favorites.push(t);
                    }
                    if was_active {
                        self.state.space_mut().active = Some(t);
                    }
                    self.dirty = true;
                    self.render();
                }
            }
            Command::DuplicateTab => {
                if let Some(url) = self.state.active_tab().and_then(|t| self.state.tabs.get(&t)).map(|t| t.url.clone()) {
                    self.open_tab(&url, None, true);
                }
            }
            Command::MoveTab { tab, favorite: true, index, .. } => {
                let was_active = self.state.active_tab() == Some(tab);
                self.state.detach(tab);
                let i = index.unwrap_or(self.state.favorites.len()).min(self.state.favorites.len());
                self.state.favorites.insert(i, tab);
                if was_active {
                    self.state.space_mut().active = Some(tab);
                }
                self.dirty = true;
                self.render();
            }
            Command::MoveTab { tab, space, folder, pinned, index, .. } => self.move_tab(tab, space, folder, pinned, index),
            Command::RenameTab { tab, title } => {
                if let Some(t) = self.state.tabs.get_mut(&tab) {
                    t.custom_title = Some(title).filter(|s| !s.trim().is_empty());
                    self.dirty = true;
                    self.render();
                }
            }
            Command::NewFolder { name } => {
                let id = self.state.new_id();
                let name = name.unwrap_or_else(|| "New Folder".into());
                self.state.space_mut().pinned.push(Node::Folder { id, name, open: true, children: vec![] });
                self.dirty = true;
                self.render();
            }
            Command::ToggleFolder { folder } => {
                if let Some(Node::Folder { open, .. }) = find_folder(&mut self.state.space_mut().pinned, folder) {
                    *open = !*open;
                }
                self.dirty = true;
                self.render();
            }
            Command::RenameFolder { folder, name } => {
                for s in &mut self.state.spaces {
                    if let Some(Node::Folder { name: n, .. }) = find_folder(&mut s.pinned, folder) {
                        *n = name.clone();
                    }
                }
                self.dirty = true;
                self.render();
            }
            Command::DeleteFolder { folder } => {
                self.state.delete_folder(folder);
                self.dirty = true;
                self.render();
            }
            Command::ClearTodayTabs => {
                let today = self.state.space().today.clone();
                for id in today {
                    self.state.archive_tab(id);
                    self.views.remove(&id);
                }
                self.state.space_mut().split.clear();
                self.dirty = true;
                self.layout();
                self.focus_active();
                self.render();
            }
            Command::RestoreArchived { index } => {
                if index < self.state.archive.len() {
                    let a = self.state.archive.remove(index);
                    self.open_tab(&a.url, self.state.space_index(&a.space), true);
                }
            }
            Command::Back { tab } | Command::Forward { tab } | Command::Reload { tab } => {
                if let Some(v) = self.target_tab(tab).and_then(|t| self.views.get(&t)) {
                    let _ = match cmd {
                        Command::Back { .. } => v.go_back(),
                        Command::Forward { .. } => v.go_forward(),
                        _ => v.reload(),
                    };
                } else if let Some(t) = self.target_tab(tab) {
                    self.ensure_view(t);
                    self.layout();
                }
            }
            Command::Stop => {
                if let Some(v) = self.state.active_tab().and_then(|t| self.views.get(&t)) {
                    let _ = v.evaluate_script("window.stop()");
                }
            }
            Command::CopyUrl => {
                if let Some(url) = self.state.active_tab().and_then(|t| self.state.tabs.get(&t)).map(|t| t.url.clone()) {
                    match arboard::Clipboard::new().and_then(|mut c| c.set_text(url)) {
                        Ok(_) => self.toast("Copied URL"),
                        Err(e) => self.toast(&format!("Clipboard error: {e}")),
                    }
                }
            }
            Command::ToggleSidebar => {
                self.config.layout.sidebar_visible = !self.config.layout.sidebar_visible;
                self.sidebar_peek = false;
                self.refresh_content_config();
                // Pages sit above the sidebar, so growing/shrinking them over it is the whole animation.
                let secs = self.anim_secs();
                self.layout_anim(secs);
                self.render();
            }
            Command::ToggleTabBarPosition => {
                self.config.layout.tab_bar = if self.config.layout.tab_bar == TabBar::Side { TabBar::Top } else { TabBar::Side };
                self.layout();
                self.render();
            }
            Command::NextSpace | Command::PrevSpace => {
                let n = self.state.spaces.len();
                let i = self.state.active_space;
                self.switch_space(if matches!(cmd, Command::NextSpace) { (i + 1) % n } else { (i + n - 1) % n });
            }
            Command::SwitchSpace { index, name } => {
                self.close_bar();
                if let Some(i) = index.or_else(|| name.and_then(|n| self.state.space_index(&n))) {
                    self.switch_space(i);
                }
            }
            Command::NewSpace { name } => {
                let name = name.unwrap_or_else(|| format!("Space {}", self.state.spaces.len() + 1));
                if self.state.space_index(&name).is_none() {
                    self.state.spaces.push(crate::state::Space { name, ..Default::default() });
                }
                let i = self.state.spaces.len() - 1;
                self.switch_space(i);
            }
            Command::RenameSpace { name } => {
                let i = self.state.active_space;
                if self.space_cfg(i).is_some() {
                    self.toast("This space is defined in config.yaml: rename it there");
                } else if self.state.space_index(&name).is_none() {
                    self.state.spaces[i].name = name;
                    self.dirty = true;
                    self.render();
                }
            }
            Command::DeleteSpace { name } => {
                let i = name.and_then(|n| self.state.space_index(&n)).unwrap_or(self.state.active_space);
                if self.space_cfg(i).is_some() {
                    return self.toast("This space is defined in config.yaml: remove it there");
                }
                if self.state.spaces.len() < 2 {
                    return;
                }
                for id in self.state.order(i, false) {
                    self.state.archive_tab(id);
                    self.views.remove(&id);
                }
                self.state.spaces.remove(i);
                self.state.active_space = self.state.active_space.min(self.state.spaces.len() - 1);
                let a = self.state.active_space;
                self.switch_space(a);
            }
            Command::SetSpaceTheme { theme } => {
                self.close_bar();
                self.state.space_mut().theme = Some(theme);
                self.dirty = true;
                self.render();
                self.refresh_content_config();
            }
            Command::SplitView { tab } => {
                let Some(a) = self.state.active_tab() else { return };
                let other = tab.filter(|t| *t != a).or_else(|| {
                    let list = self.state.order(self.state.active_space, true);
                    let split = &self.state.space().split;
                    list.iter().copied().find(|t| *t != a && !split.contains(t))
                });
                let Some(other) = other else { return self.toast("Open another tab to split with") };
                let s = self.state.space_mut();
                if !(s.split.len() > 1 && s.split.contains(&a)) {
                    s.split = vec![a];
                }
                if !s.split.contains(&other) && s.split.len() < 4 {
                    s.split.push(other);
                }
                self.activate(a);
            }
            Command::Unsplit => {
                self.state.space_mut().split.clear();
                self.layout();
                self.render();
            }
            Command::FocusNextPane => {
                let panes = self.panes();
                if panes.len() > 1 {
                    let cur = self.state.active_tab().and_then(|a| panes.iter().position(|p| *p == a)).unwrap_or(0);
                    self.state.space_mut().active = Some(panes[(cur + 1) % panes.len()]);
                    self.focus_active();
                    self.render();
                }
            }
            Command::ZoomIn | Command::ZoomOut | Command::ZoomReset => {
                if let Some(t) = self.state.active_tab() {
                    let z = self.zoom.get(&t).copied().unwrap_or(1.0);
                    let z = match cmd {
                        Command::ZoomIn => (z + 0.1).min(3.0),
                        Command::ZoomOut => (z - 0.1).max(0.3),
                        _ => 1.0,
                    };
                    self.zoom.insert(t, z);
                    if let Some(v) = self.views.get(&t) {
                        let _ = v.zoom(z);
                    }
                    self.toast(&format!("Zoom {:.0}%", z * 100.0));
                }
            }
            Command::DevTools => {
                if let Some(v) = self.state.active_tab().and_then(|t| self.views.get(&t)) {
                    v.open_devtools();
                }
            }
            Command::OpenLittleWindow { url } => {
                let url = routing::resolve_input(&url, &self.config.search_engine);
                self.open_little(&url, self.state.active_space, elwt)
            }
            Command::MoveToMainWindow | Command::ClosePeek => {}
            Command::Peek { url } => {
                let url = routing::resolve_input(&url, &self.config.search_engine);
                self.open_peek(&url)
            }
            Command::OpenConfig => {
                self.close_bar();
                platform::open_text_file(&config::config_path())
            }
            Command::ReloadConfig => self.reload_config(),
            Command::Sync => {
                self.close_bar();
                self.start_sync()
            }
            Command::Import { source } => {
                self.close_bar();
                match import::import(&source) {
                    Ok(imp) => {
                        let msg = self.state.apply_import(&source, imp);
                        self.dirty = true;
                        self.render();
                        self.toast(&msg);
                    }
                    Err(e) => self.toast(&format!("Import failed: {e:#}")),
                }
            }
            Command::RunAgent { name, prompt, agent } => {
                self.close_bar();
                let job = name.as_ref().and_then(|n| self.config.agents.jobs.iter().find(|j| &j.name == n).cloned());
                match (job, prompt) {
                    (Some(j), _) => self.start_agent(j.name, j.prompt, agent.or(j.agent), j.model),
                    (None, Some(p)) if !p.trim().is_empty() => {
                        let short: String = p.chars().take(40).collect();
                        self.start_agent(name.unwrap_or(short), p, agent, None)
                    }
                    _ => self.toast("No such agent job"),
                }
            }
            Command::ShowAgentRun { index } => {
                self.close_bar();
                if let Some(r) = self.agent_runs.get(index) {
                    let text = std::fs::read_to_string(&r.log).unwrap_or_else(|e| e.to_string());
                    let text: String = text.chars().take(200_000).collect();
                    let enc: String = url::form_urlencoded::byte_serialize(text.as_bytes()).collect::<String>().replace('+', "%20");
                    self.open_tab(&format!("data:text/plain;charset=utf-8,{enc}"), None, true);
                }
            }
            Command::CopyPassword { vault, origin, username } => {
                self.close_bar();
                let e = passwords::Entry { vault, origin, username };
                match passwords::password(&e) {
                    Ok(Some(pw)) => {
                        if arboard::Clipboard::new().and_then(|mut c| c.set_text(pw.clone())).is_ok() {
                            self.toast("Password copied: clears in 45 seconds");
                            std::thread::spawn(move || {
                                std::thread::sleep(std::time::Duration::from_secs(45));
                                if let Ok(mut c) = arboard::Clipboard::new() {
                                    if c.get_text().ok().as_deref() == Some(pw.as_str()) {
                                        let _ = c.set_text(String::new());
                                    }
                                }
                            });
                        }
                    }
                    Ok(None) => self.toast("Password not found"),
                    Err(err) => self.toast(&format!("Keychain: {err}")),
                }
            }
            Command::DeletePassword { vault, origin, username } => {
                self.close_bar();
                match passwords::delete(&passwords::Entry { vault, origin, username }) {
                    Ok(()) => self.toast("Password deleted"),
                    Err(e) => self.toast(&format!("Keychain: {e}")),
                }
            }
            Command::ToggleSiteBlocking => {
                let host = self
                    .state
                    .active_tab()
                    .and_then(|t| self.state.tabs.get(&t))
                    .and_then(|t| url::Url::parse(&t.url).ok())
                    .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string()));
                let Some(host) = host else { return };
                let allowed = if let Some(i) = self.state.blocking_allow.iter().position(|h| h == &host) {
                    self.state.blocking_allow.remove(i);
                    false
                } else {
                    self.state.blocking_allow.push(host.clone());
                    true
                };
                self.dirty = true;
                self.toast(&format!("Blocking {} for {host}: reload to apply", if allowed { "off" } else { "on" }));
                self.start_blocklist();
            }
            Command::FindInPage { query } => {
                if let Some(v) = self.state.active_tab().and_then(|t| self.views.get(&t)) {
                    let _ = v.focus();
                    let q = serde_json::to_string(&query).unwrap_or("null".into());
                    let _ = v.evaluate_script(&format!("window.__valerie && __valerie.find({q})"));
                }
            }
            Command::Quit => self.quit = true,
        }
    }

    // ---------------------------------------------------------------- events

    fn handle_ipc(&mut self, from: Source, msg: String, elwt: &EventLoopWindowTarget<UserEvent>) {
        let trusted = !matches!(from, Source::Page(_));
        if !trusted && msg.len() > MAX_UNTRUSTED_IPC {
            return;
        }
        let Ok(m) = serde_json::from_str::<Value>(&msg) else { return };
        let kind = m["t"].as_str().unwrap_or("");
        match (from, kind) {
            (Source::Chrome, "ready") => self.render(),
            (Source::Chrome, "unpeek") | (Source::Page(Target::Tab(_)), "unpeek") => {
                if self.sidebar_peek {
                    self.sidebar_peek = false;
                    let secs = self.anim_secs();
                    self.layout_anim(secs);
                    self.render();
                    self.refresh_content_config();
                }
            }
            (Source::Page(Target::Tab(_)), "edge") => {
                // Mouse hit the screen edge with the sidebar hidden: slide it in until the mouse leaves it.
                if !self.config.layout.sidebar_visible && !self.sidebar_peek && self.config.layout.tab_bar == TabBar::Side {
                    self.sidebar_peek = true;
                    let secs = self.anim_secs();
                    self.layout_anim(secs);
                    self.render();
                    self.refresh_content_config();
                }
            }
            (Source::Chrome, "drag") => {
                let _ = self.window.drag_window();
            }
            (Source::LittleBar(id), "ready") => self.render_little(id),
            (Source::LittleBar(id), "drag") => {
                if let Some(l) = self.littles.get(&id) {
                    let _ = l.window.drag_window();
                }
            }
            (_, "cmd") => {
                let Ok(cmd) = serde_json::from_value::<Command>(m["cmd"].clone()) else { return };
                match from {
                    Source::Page(_) if !cmd.page_safe() => return,
                    Source::Page(Target::Little(id)) | Source::LittleBar(id) => self.focused_little = Some(id),
                    Source::Page(Target::Peek) => self.focused_little = None,
                    Source::Page(Target::Tab(id)) => {
                        self.focused_little = None;
                        // A key pressed in a split pane acts on that pane.
                        if self.panes().contains(&id) {
                            self.state.space_mut().active = Some(id);
                        }
                    }
                    Source::Chrome => self.focused_little = None,
                    Source::Overlay => {
                        if !matches!(cmd, Command::CloseCommandBar) {
                            self.close_bar();
                        }
                    }
                }
                self.dispatch(cmd, elwt);
            }
            (Source::Page(target), "meta") => {
                let Target::Tab(id) = target else {
                    return;
                };
                let real_url = self.views.get(&id).and_then(|v| v.url().ok());
                if let Some(t) = self.state.tabs.get_mut(&id) {
                    if let Some(f) = m["favicon"].as_str() {
                        if matches!(scheme_of(f).as_str(), "http" | "https" | "data") && f.len() < 8192 {
                            t.favicon = Some(f.to_string());
                        }
                    }
                    t.playing = m["playing"].as_bool().unwrap_or(false);
                    if let Some(u) = real_url.filter(|u| !u.is_empty() && u != "about:blank") {
                        t.url = u;
                    }
                }
                self.render();
            }
            (Source::Page(target), k @ ("login_form" | "fill" | "credential" | "save_password" | "never_password" | "dismiss_password")) => {
                let k = k.to_string();
                self.handle_password_ipc(target, &k, &m);
            }
            (Source::Page(_), "peek") => {
                let Some(url) = m["url"].as_str().filter(|u| matches!(scheme_of(u).as_str(), "http" | "https")) else { return };
                if self.last_page_open.elapsed() < std::time::Duration::from_millis(400) {
                    return;
                }
                self.last_page_open = std::time::Instant::now();
                let url = url.to_string();
                self.open_peek(&url);
            }
            (Source::Page(target), "open") => {
                let Some(url) = m["url"].as_str().filter(|u| matches!(scheme_of(u).as_str(), "http" | "https")) else { return };
                // Pages can forge this message, so cap it to stop tab spam.
                if self.last_page_open.elapsed() < std::time::Duration::from_millis(400) {
                    return;
                }
                self.last_page_open = std::time::Instant::now();
                let space = match target {
                    Target::Tab(id) => self.state.space_of(id),
                    Target::Little(id) => self.littles.get(&id).map(|l| l.space),
                    Target::Peek => None,
                };
                let id = self.state.add_tab(space.unwrap_or(self.state.active_space), url, true);
                self.ensure_view(id);
                self.layout();
                self.render();
                self.toast("Opened in background");
            }
            _ => {}
        }
    }

    pub fn handle(&mut self, ev: UserEvent, elwt: &EventLoopWindowTarget<UserEvent>) {
        match ev {
            UserEvent::Ipc { from, msg } => self.handle_ipc(from, msg, elwt),
            UserEvent::PageLoad { target, url, finished } => match target {
                Target::Tab(id) => {
                    let mut visit = None;
                    if let Some(t) = self.state.tabs.get_mut(&id) {
                        if !url.is_empty() && url != "about:blank" {
                            t.url = url.clone();
                        }
                        t.loading = !finished;
                        if finished {
                            visit = Some((t.url.clone(), t.title.clone()));
                        }
                    }
                    if let Some((u, t)) = visit {
                        self.state.record_visit(&u, &t);
                        if let Some(v) = self.views.get(&id) {
                            self.after_load(v, &u);
                        }
                        self.fill_offers.remove(&Target::Tab(id));
                        // A login form that navigates on submit: show the save prompt once more on the page it lands on.
                        let reoffer = self.pending_save.as_mut().filter(|p| p.target == Target::Tab(id) && !p.reoffered && now().saturating_sub(p.at) <= 10);
                        if let Some(p) = reoffer {
                            p.reoffered = true;
                            self.offer_pending_save();
                        }
                    }
                    self.dirty = true;
                    self.render();
                }
                Target::Little(id) => {
                    if let Some(l) = self.littles.get_mut(&id) {
                        l.url = url.clone();
                    }
                    if let Some(l) = self.littles.get(&id).filter(|_| finished) {
                        self.after_load(&l.view, &url);
                    }
                    self.render_little(id);
                }
                Target::Peek => {
                    if let Some(p) = self.peek.as_mut() {
                        p.url = url.clone();
                    }
                    if let Some(p) = self.peek.as_ref().filter(|_| finished) {
                        self.after_load(&p.view, &url);
                    }
                }
            },
            UserEvent::Title { target, title } => match target {
                Target::Tab(id) => {
                    if let Some(t) = self.state.tabs.get_mut(&id) {
                        t.title = title.clone();
                        let url = t.url.clone();
                        self.state.update_history_title(&url, &title);
                    }
                    self.render();
                }
                Target::Little(id) => {
                    if let Some(l) = self.littles.get_mut(&id) {
                        l.title = title;
                    }
                    self.render_little(id);
                }
                Target::Peek => {}
            },
            UserEvent::NewWindow { target, url } => {
                let space = match target {
                    Target::Tab(id) => self.state.space_of(id),
                    Target::Little(id) => self.littles.get(&id).map(|l| l.space),
                    Target::Peek => None,
                };
                self.close_peek();
                self.open_tab(&url, space, true);
                self.window.set_focus();
            }
            UserEvent::ExternalScheme(url) => {
                let scheme = scheme_of(&url);
                if EXTERNAL_SCHEMES.contains(&scheme.as_str()) {
                    platform::open_external(&url);
                    self.toast(&format!("Opened {scheme}: link in another app"));
                } else {
                    self.toast(&format!("Blocked {scheme}: link"));
                }
            }
            UserEvent::Menu(id) => {
                if let Some(cmd) = self.menu_cmds.get(&id).cloned() {
                    self.dispatch(cmd, elwt);
                }
            }
            UserEvent::Mcp(req) => self.handle_mcp(req, elwt),
            UserEvent::Tick => self.tick(),
            UserEvent::ConfigChanged => self.reload_config(),
            UserEvent::SyncDone(r) => {
                self.syncing = false;
                match r {
                    Ok(devices) => self.remote = devices,
                    Err(e) => self.toast(&format!("Sync failed: {e}")),
                }
            }
            UserEvent::AgentDone(f) => {
                self.agents_running.retain(|n| n != &f.name);
                self.toast(&format!("Agent {}: {}", if f.ok { "finished" } else { "failed" }, f.name));
                self.agent_runs.insert(0, f);
                self.agent_runs.truncate(30);
                self.render();
            }
            UserEvent::BlocklistBuilt { json, hash } => {
                #[cfg(target_os = "macos")]
                {
                    let p = self.proxy.clone();
                    match json {
                        Some(j) => blocking::mac::compile(&j, move |r| send(&p, UserEvent::RulesReady(r, Some(hash)))),
                        None => blocking::mac::lookup(move |r| send(&p, UserEvent::RulesReady(r, None))),
                    }
                }
                #[cfg(not(target_os = "macos"))]
                let _ = (json, hash);
            }
            #[cfg(target_os = "macos")]
            UserEvent::RulesReady(r, hash) => match r {
                Ok(list) => {
                    if let Some(h) = hash {
                        blocking::store_hash(&h);
                    }
                    self.rules = Some(list);
                    self.apply_rules();
                }
                Err(_) if hash.is_none() => {
                    // WebKit's cached copy is gone: forget the hash and compile from scratch.
                    let _ = std::fs::remove_file(blocking::dir().join("compiled.hash"));
                    self.start_blocklist();
                }
                Err(e) => self.toast(&format!("Content blocking failed: {e}")),
            },
            UserEvent::Downloaded { ok } => self.toast(if ok { "Download complete" } else { "Download failed" }),
        }
    }

    fn handle_mcp(&mut self, req: McpRequest, elwt: &EventLoopWindowTarget<UserEvent>) {
        let McpRequest { op, args, reply } = req;
        let tab_arg = args.get("tab").and_then(|t| t.as_u64());
        let result: Result<Value, String> = match op.as_str() {
            "list_spaces" => Ok(json!(self.state.spaces.iter().enumerate().map(|(i, s)| json!({
                "index": i, "name": s.name, "profile": self.profile_for(i), "active": i == self.state.active_space,
                "tabs": self.state.order(i, false).len(),
            })).collect::<Vec<_>>())),
            "list_tabs" => {
                let filter = args.get("space").and_then(|s| s.as_str()).and_then(|s| self.state.space_index(s));
                let active = self.state.active_tab();
                let row = |id: &TabId, space: Option<usize>, section: &str| {
                    self.state.tabs.get(id).map(|t| json!({
                        "tab": t.id, "title": t.display_title(), "url": t.url, "section": section,
                        "space": space.map(|s| self.state.spaces[s].name.clone()),
                        "active": Some(t.id) == active, "loaded": self.views.contains_key(&t.id),
                    }))
                };
                let mut out: Vec<Value> = Vec::new();
                if filter.is_none() {
                    out.extend(self.state.favorites.iter().filter_map(|id| row(id, None, "favorite")));
                }
                for i in 0..self.state.spaces.len() {
                    if filter.is_some_and(|f| f != i) {
                        continue;
                    }
                    let today = &self.state.spaces[i].today;
                    for id in self.state.order(i, false) {
                        out.extend(row(&id, Some(i), if today.contains(&id) { "today" } else { "pinned" }));
                    }
                }
                Ok(json!(out))
            }
            "open_tab" => {
                let url = routing::resolve_input(args["url"].as_str().unwrap_or(""), &self.config.search_engine);
                let space = args.get("space").and_then(|s| s.as_str()).and_then(|s| self.state.space_index(s));
                let id = self.open_tab(&url, space, args["activate"].as_bool().unwrap_or(false));
                Ok(json!({ "tab": id, "url": url }))
            }
            "open_link" => {
                let url = args["url"].as_str().unwrap_or("").to_string();
                if matches!(scheme_of(&url).as_str(), "http" | "https" | "file") {
                    self.open_external(vec![url], elwt);
                    Ok(json!("opened"))
                } else {
                    Err("only http(s) and file URLs".into())
                }
            }
            "navigate" => match self.target_tab(tab_arg) {
                Some(t) => {
                    let url = routing::resolve_input(args["url"].as_str().unwrap_or(""), &self.config.search_engine);
                    self.navigate(t, &url);
                    Ok(json!({ "tab": t, "url": url }))
                }
                None => Err("no tab".into()),
            },
            "activate_tab" | "close_tab" => match tab_arg.filter(|t| self.state.tabs.contains_key(t)) {
                Some(t) => {
                    if op == "activate_tab" {
                        self.activate(t)
                    } else {
                        self.close(t)
                    }
                    Ok(json!("ok"))
                }
                None => Err("unknown tab".into()),
            },
            "run" => match serde_json::from_value::<Command>(args) {
                Ok(cmd) if cmd.agent_forbidden() => Err("that command is not available to agents".into()),
                Ok(cmd) => {
                    self.dispatch(cmd, elwt);
                    Ok(json!("ok"))
                }
                Err(e) => Err(format!("bad command: {e}")),
            },
            "ensure_loaded" => match self.target_tab(tab_arg).filter(|t| self.state.tabs.contains_key(t)) {
                Some(t) => {
                    self.ensure_view(t);
                    self.layout();
                    Ok(json!({ "tab": t }))
                }
                None => Err("no tab (open one first)".into()),
            },
            "screenshot" => {
                let Some(t) = self.target_tab(tab_arg).filter(|t| self.state.tabs.contains_key(t)) else {
                    let _ = reply.send(Err("no tab".into()));
                    return;
                };
                self.ensure_view(t);
                let Some(v) = self.views.get(&t) else {
                    let _ = reply.send(Err("tab has no page".into()));
                    return;
                };
                let width = args["width"].as_f64().unwrap_or(1024.0).clamp(200.0, 2400.0);
                platform::snapshot(v, width, move |r| {
                    let _ = reply.send(r.map(|png| json!({ "__image_png": crate::mcp::base64(&png) })));
                });
                return;
            }
            "eval" => {
                let Some(t) = self.target_tab(tab_arg) else {
                    let _ = reply.send(Err("no tab".into()));
                    return;
                };
                self.ensure_view(t);
                let Some(v) = self.views.get(&t) else {
                    let _ = reply.send(Err("tab has no page".into()));
                    return;
                };
                let script = args["script"].as_str().unwrap_or("");
                let js = format!(
                    "(function(){{try{{var __v=(function(){{\n{script}\n}})();\
                     if(__v&&typeof __v.then==='function'){{var __id='r'+Math.random().toString(36).slice(2);window.__vres=window.__vres||{{}};\
                     __v.then(function(x){{window.__vres[__id]={{ok:x===undefined?null:x}}}},function(e){{window.__vres[__id]={{error:String(e&&e.message||e)}}}});\
                     return JSON.stringify({{pending:__id}})}}\
                     return JSON.stringify({{ok:__v===undefined?null:__v}})}}catch(e){{return JSON.stringify({{error:String(e&&e.message||e)}})}}}})()"
                );
                let r = reply.clone();
                if let Err(e) = v.evaluate_script_with_callback(&js, move |s| {
                    let _ = r.send(Ok(decode_js(&s)));
                }) {
                    let _ = reply.send(Err(e.to_string()));
                }
                return;
            }
            _ => Err(format!("unknown op {op}")),
        };
        let _ = reply.send(result);
    }

    fn window_event(&mut self, id: WindowId, ev: WindowEvent, elwt: &EventLoopWindowTarget<UserEvent>) {
        let _ = elwt;
        let main = id == self.window.id();
        match ev {
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                if main {
                    self.layout()
                } else {
                    self.layout_little(id)
                }
            }
            WindowEvent::CloseRequested => {
                if main {
                    self.quit = true;
                } else {
                    self.littles.remove(&id);
                    if self.focused_little == Some(id) {
                        self.focused_little = None;
                    }
                }
            }
            WindowEvent::Focused(true) => {
                self.focused_little = if main { None } else { Some(id) };
            }
            _ => {}
        }
    }

    fn start(&mut self, urls: Vec<String>, elwt: &EventLoopWindowTarget<UserEvent>) {
        self.build_menu();
        self.start_blocklist();
        let a = self.state.active_space;
        self.switch_space(a);
        if !urls.is_empty() {
            self.open_external(urls, elwt);
        }
    }
}

fn watch_config(proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        let path = config::config_path();
        // Newest mtime across the config file, the boosts dir and the files in it.
        let mtime = || {
            let m = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
            let boosts = std::fs::read_dir(boosts_dir()).into_iter().flatten().flatten().filter_map(|e| m(&e.path()));
            [m(&path), m(&boosts_dir())].into_iter().flatten().chain(boosts).max()
        };
        let mut last = mtime();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(800));
            let m = mtime();
            if m != last {
                last = m;
                send(&proxy, UserEvent::ConfigChanged);
            }
        }
    });
}

pub fn run(config: Config, urls: Vec<String>) -> Result<()> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let p = proxy.clone();
    muda::MenuEvent::set_event_handler(Some(move |e: muda::MenuEvent| send(&p, UserEvent::Menu(e.id))));
    if config.mcp.enabled {
        crate::mcp::spawn(config.mcp.port, crate::mcp::load_or_create_token(), proxy.clone());
    }
    let p = proxy.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(30));
        send(&p, UserEvent::Tick);
    });
    watch_config(proxy.clone());
    let mut app = App::new(&event_loop, config, proxy)?;
    let mut initial = Some(urls);
    event_loop.run(move |event, elwt, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => app.start(initial.take().unwrap_or_default(), elwt),
            Event::UserEvent(ev) => app.handle(ev, elwt),
            Event::WindowEvent { window_id, event, .. } => app.window_event(window_id, event, elwt),
            Event::Opened { urls } => app.open_external(urls.into_iter().map(|u| u.to_string()).collect(), elwt),
            Event::LoopDestroyed => app.save(),
            _ => {}
        }
        if app.quit {
            app.save();
            *control_flow = ControlFlow::Exit;
        }
    });
}
