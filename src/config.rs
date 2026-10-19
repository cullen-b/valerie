use crate::command::Command;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const DEFAULT_YAML: &str = include_str!("../config/default.yaml");
pub const STARTER_YAML: &str = include_str!("../config/starter.yaml");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub search_engine: String,
    pub editor: String,
    pub terminal: String,
    pub new_tab_url: String,
    pub user_agent: String,
    pub layout: Layout,
    pub sidebar: SidebarConfig,
    pub start_page: String,
    pub escape_hints: bool,
    pub welcome: bool,
    pub command_bar: CommandBarConfig,
    pub site_search: BTreeMap<String, Option<SiteSearch>>,
    pub find: FindConfig,
    pub links: LinksConfig,
    pub tabs: TabsConfig,
    pub window: WindowConfig,
    pub peek: PeekConfig,
    pub history: HistoryConfig,
    pub downloads: DownloadsConfig,
    pub toast_seconds: f64,
    pub reader_view: ReaderConfig,
    pub capture: CaptureConfig,
    pub privacy: PrivacyConfig,
    pub chats: ChatsConfig,
    pub terminal_panel: TerminalPanelConfig,
    pub ai_actions: BTreeMap<String, Option<AiAction>>,
    pub watch: WatchConfig,
    pub notifications: NotificationsConfig,
    pub tasks: Vec<TaskConfig>,
    /// Older name for tasks that write a page.
    pub briefs: Vec<TaskConfig>,
    pub permissions: PermissionsConfig,
    pub pages: PagesConfig,
    pub translate: TranslateConfig,
    pub little_window: LittleWindowConfig,
    pub profiles: Vec<ProfileConfig>,
    pub spaces: Vec<SpaceConfig>,
    pub routes: Vec<Route>,
    /// Host pattern → profile that site always uses.
    pub site_profiles: BTreeMap<String, Option<String>>,
    pub keybindings: BTreeMap<String, Option<Binding>>,
    pub vim: VimConfig,
    pub mcp: McpConfig,
    pub sync: SyncConfig,
    pub updates: UpdatesConfig,
    pub plugin_marketplace: MarketplaceConfig,
    pub agents: AgentsConfig,
    pub content_blocking: BlockingConfig,
    pub passwords: PasswordsConfig,
    pub themes: BTreeMap<String, Theme>,
    pub ui: UiConfig,
    pub menu: MenuConfig,
    pub context_menu: ContextMenuConfig,
    pub mouse: MouseConfig,
    pub gestures: GesturesConfig,
    pub app: AppConfig,
    pub global_keybindings: BTreeMap<String, Option<Binding>>,
    pub rules: Vec<NetRule>,
    pub sites: BTreeMap<String, Option<SiteSettings>>,
    pub language: String,
    pub extensions: ExtensionsConfig,
    pub logs: LogsConfig,
}

/// Valerie's log files (what went wrong, for you and your AI to read).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogsConfig {
    pub keep_days: u64,
    /// errors | events | verbose.
    #[serde(default = "default_log_level")]
    pub level: String,
}

fn default_log_level() -> String {
    "events".into()
}

/// Your own CSS for Valerie's windows (ui/*.css in the config folder).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    pub custom_css: bool,
}

/// The menu bar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuConfig {
    pub hide: Vec<String>,
    pub rename: BTreeMap<String, Option<String>>,
    pub commands: BTreeMap<String, Option<String>>,
    pub quit_shortcut: String,
    pub confirm_quit: bool,
}

/// Valerie's right-click menus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMenuConfig {
    pub hide: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseConfig {
    pub back_button: Option<Binding>,
    pub forward_button: Option<Binding>,
    pub middle_click_tab: Option<Binding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GesturesConfig {
    pub swipe_to_go_back: bool,
    pub pinch_to_zoom: bool,
}

/// Valerie outside its window: the Dock and the menu bar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub icon: String,
    pub dock_menu: Vec<Binding>,
    pub menu_bar_icon: String,
    pub menu_bar_menu: Vec<Binding>,
}

/// A network rule: block, redirect, hide or add a header. Exactly one of block / redirect / hide / header.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetRule {
    /// Requests to stop: Adblock syntax ("||ads.example.com^") or an address with * wildcards.
    pub block: String,
    /// Pages to send elsewhere (an address with * wildcards) …
    pub redirect: String,
    /// … and where: the address, where each * is what the matching * in `redirect` matched.
    pub to: String,
    /// Elements to hide (a CSS selector).
    pub hide: String,
    /// A request header to add ("Name: value") to pages you open.
    pub header: String,
    /// Which sites it applies to (hosts like "example.com", "*.edu"); empty = every site.
    pub site: String,
}

/// Settings for one site (`sites:`); anything left out follows the global setting.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SiteSettings {
    pub camera: Option<String>,
    pub microphone: Option<String>,
    pub notifications: Option<String>,
    pub user_agent: Option<String>,
    pub zoom: Option<f64>,
    pub autoplay: Option<bool>,
    pub dark_mode: Option<String>,
    pub vim: Option<bool>,
    pub links: Option<BTreeMap<String, String>>,
    pub unload: Option<bool>,
    pub ad_blocking: Option<bool>,
    pub popups: Option<String>,
    pub cookie_banners: Option<String>,
}

impl SiteSettings {
    /// Fill in what `more` sets (it wins).
    fn overlay(&mut self, more: &SiteSettings) {
        macro_rules! take {
            ($($f:ident),*) => { $( if more.$f.is_some() { self.$f = more.$f.clone(); } )* };
        }
        take!(camera, microphone, notifications, user_agent, zoom, autoplay, dark_mode, vim, links, unload, ad_blocking, popups, cookie_banners);
    }
}

/// The `sites:` settings for an address: every matching entry, the most specific (longest pattern) winning.
pub fn site_settings_for(sites: &BTreeMap<String, Option<SiteSettings>>, url: &str) -> SiteSettings {
    let mut hits: Vec<(&String, &SiteSettings)> = sites
        .iter()
        .filter_map(|(k, v)| v.as_ref().map(|v| (k, v)))
        .filter(|(k, _)| url.starts_with("http") && crate::plugins::matches(&[(*k).clone()], url))
        .collect();
    hits.sort_by_key(|(k, _)| k.len());
    let mut out = SiteSettings::default();
    for (_, s) in hits {
        out.overlay(s);
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionsConfig {
    pub enabled: bool,
    pub folder: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layout {
    pub tab_bar: TabBar,
    pub sidebar_visible: bool,
    pub sidebar_width: f64,
    pub sidebar_size: String,
    pub sidebar_position: SidebarPosition,
    pub reveal_sidebar_on_edge: bool,
    pub bezel: f64,
    pub corner_radius: f64,
    pub split_gap: f64,
    pub split_focus: String,
    pub pinned_style: String,
    pub animation_ms: f64,
    pub top_bar_height: f64,
    pub font: String,
    pub split_max_panes: usize,
    pub split_unfocused_opacity: f64,
    pub split_border_width: f64,
}

/// Which bits of sidebar text and chrome are shown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidebarConfig {
    pub nav_buttons: bool,
    pub url_bar: bool,
    pub url_path: bool,
    pub url_lock_icon: bool,
    pub space_name: bool,
    pub favorites: bool,
    pub pinned_hint: bool,
    pub compact_pinned: String,
    pub pinned_folder_icon: String,
    pub kept_section: bool,
    pub minus_for_saved_tabs: bool,
    pub new_tab_button: bool,
    pub today_label: bool,
    pub clear_button: String,
    pub space_switcher: bool,
    pub order: Vec<String>,
    pub tab_height: f64,
    pub now_playing: bool,
    pub compact_width: f64,
    pub medium_width: f64,
    pub max_width: f64,
    pub pinned_columns: u32,
    pub pinned_tile_height: f64,
    pub swipe_between_spaces: bool,
    pub swipe_distance: f64,
    pub preload_on_hover: bool,
    pub preload_delay_ms: f64,
    pub preload_at_most: f64,
    pub double_click_edge: bool,
}

/// What modified clicks on links do; values are link actions (see config/default.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinksConfig {
    pub middle_click: String,
    pub cmd_click: String,
    pub shift_click: String,
    pub option_click: String,
    pub cmd_shift_click: String,
    pub option_shift_click: String,
    pub context_menu: bool,
    pub show_url_on_hover: bool,
    pub popups: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindConfig {
    pub minimap: bool,
    pub highlight_all: bool,
    pub highlight_color: String,
    pub current_color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteSearch {
    pub keywords: Vec<String>,
    /// Search URL with `%s` where the query goes.
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandBarConfig {
    pub site_search: bool,
    pub footer_hints: bool,
    pub empty_hint: bool,
    pub calculator: bool,
    pub top_result: String,
    pub ask_agent_words: usize,
    pub ask_agent_starts: Vec<String>,
    pub width: f64,
    pub top: f64,
    pub font_size: f64,
    pub max_results: usize,
    pub dim_background: f64,
    pub result_weights: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabBar {
    Side,
    Top,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarPosition {
    Left,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabsConfig {
    pub archive_after_hours: f64,
    pub ask_to_keep_unused_days: f64,
    pub unload_after_minutes: f64,
    pub new_tab_position: String,
    pub autoplay_in_background: bool,
    pub picture_in_picture_on_leave: bool,
    pub zoom: String,
    pub default_zoom: f64,
    pub zoom_step: f64,
    pub zoom_min: f64,
    pub zoom_max: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReaderConfig {
    pub theme: String,
    pub font: String,
    pub font_size: f64,
    pub width: f64,
}

/// One agent: a CLI Valerie knows (claude, codex, opencode, pi, cursor, hermes) and the model to ask for, or any
/// command that takes a prompt and prints an answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentDef {
    pub cli: String,
    pub model: String,
    pub command: Vec<String>,
}

/// A one-click job for an agent (`ai_actions`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AiAction {
    /// "page": answered under the find bar from the page alone (no tools). "chat": opens in Chats and can use the browser.
    pub mode: String,
    pub prompt: String,
    /// Which agent (a name under agents.available); empty = the one for that mode's job.
    pub agent: String,
}

/// Something an agent does for the user (`tasks`): a morning brief, joining meetings, opening pages, watching a feed…
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskConfig {
    pub name: String,
    /// What to do, in the user's own words.
    pub want: String,
    /// When, in plain words: "every day at 8am", "weekdays at 9", "mondays at 5pm", "every 5 minutes",
    /// "when Valerie starts", "only when I ask".
    pub when: String,
    /// Where the result goes: "page" (a tab with the latest result) | "chat" (a chat with each run) | "notify".
    pub result: String,
    pub space: String,
    /// Which agent; empty = agents.use.tasks.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub agent: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// Only this machine runs it (so synced machines don't each run it); empty = any.
    pub machine: String,
    #[serde(skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// Older brief fields: a time and days instead of `when`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub days: String,
}

impl Default for TaskConfig {
    fn default() -> Self {
        TaskConfig {
            name: String::new(),
            want: String::new(),
            when: String::new(),
            result: String::new(),
            space: String::new(),
            agent: String::new(),
            model: String::new(),
            machine: String::new(),
            enabled: true,
            at: String::new(),
            days: String::new(),
        }
    }
}

fn is_true(b: &bool) -> bool {
    *b
}

/// When a task runs.
#[derive(Debug, Clone, PartialEq)]
pub enum When {
    /// Once a day at "HH:MM" on the given days ("every day" | "weekdays" | "weekends" | "mon, wed, fri").
    Daily { at: String, days: String },
    Every(u64),
    OnStart,
    Manual,
}

impl When {
    pub fn runs_on(days: &str, weekday: &str) -> bool {
        let d = weekday.to_lowercase();
        let d = &d[..d.len().min(3)];
        match days.trim().to_lowercase().as_str() {
            "" | "every day" | "daily" => true,
            "weekdays" => !matches!(d, "sat" | "sun"),
            "weekends" => matches!(d, "sat" | "sun"),
            list => list.split([',', ' ']).any(|x| x.trim().starts_with(d)),
        }
    }

    /// Plain words for showing to people.
    pub fn describe(&self) -> String {
        match self {
            When::Daily { at, days } => format!("{} at {at}", if days.is_empty() { "every day" } else { days }),
            When::Every(s) if s % 3600 == 0 => format!("every {} hour{}", s / 3600, if *s == 3600 { "" } else { "s" }),
            When::Every(s) => format!("every {} minute{}", s / 60, if *s == 60 { "" } else { "s" }),
            When::OnStart => "when Valerie starts".into(),
            When::Manual => "only when asked".into(),
        }
    }
}

/// Read a schedule written in plain words. None when it can't be understood.
pub fn parse_when(s: &str) -> Option<When> {
    let t = s.trim().to_lowercase().replace(" am", "am").replace(" pm", "pm").replace(" o'clock", "");
    if t.is_empty() || ["manual", "never", "on demand"].contains(&t.as_str()) || t.contains("when i ask") || t.contains("only when") {
        return Some(When::Manual);
    }
    if t.contains("start") || t.contains("launch") || t.contains("when valerie opens") || t.contains("on open") {
        return Some(When::OnStart);
    }
    let words: Vec<&str> = t.split([' ', ',']).filter(|w| !w.is_empty()).collect();
    if t == "hourly" {
        return Some(When::Every(3600));
    }
    if let Some(i) = words.iter().position(|w| *w == "every") {
        let unit = |u: &str| -> Option<u64> {
            match u.trim_end_matches('s') {
                "m" | "min" | "minute" => Some(60),
                "h" | "hr" | "hour" => Some(3600),
                _ => None,
            }
        };
        let (n, u) = match (words.get(i + 1), words.get(i + 2)) {
            (Some(a), _) if a.ends_with(['m', 'h']) && a[..a.len() - 1].parse::<u64>().is_ok() => (a[..a.len() - 1].parse().ok(), unit(&a[a.len() - 1..])),
            (Some(a), Some(b)) if a.parse::<u64>().is_ok() => (a.parse().ok(), unit(b)),
            (Some(a), _) => (Some(1), unit(a)),
            _ => (None, None),
        };
        if let (Some(n), Some(u)) = (n, u) {
            return (n > 0).then_some(When::Every(n * u));
        }
    }
    let at = words.iter().filter(|w| **w != "at").find_map(|w| {
        let looks = w.contains(':') || w.ends_with("am") || w.ends_with("pm") || w.parse::<u32>().is_ok_and(|n| n < 24);
        looks.then(|| parse_time(w)).flatten()
    })?;
    let names = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    let days = if t.contains("weekday") {
        "weekdays".to_string()
    } else if t.contains("weekend") {
        "weekends".to_string()
    } else {
        let picked: Vec<&str> = names.iter().copied().filter(|d| words.iter().any(|w| w.starts_with(d))).collect();
        if picked.is_empty() { "every day".to_string() } else { picked.join(", ") }
    };
    Some(When::Daily { at, days })
}

impl TaskConfig {
    pub fn slug(&self) -> String {
        let s: String = self.name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
    }

    pub fn schedule(&self) -> Option<When> {
        if self.when.trim().is_empty() && !self.at.is_empty() {
            return parse_time(&self.at).map(|at| When::Daily { at, days: if self.days.is_empty() { "every day".into() } else { self.days.clone() } });
        }
        parse_when(&self.when)
    }

    /// "page" | "chat" | "notify" (a missing one is a page, like the briefs tasks grew out of).
    pub fn result(&self) -> &str {
        match self.result.as_str() {
            "chat" | "notify" => &self.result,
            _ => "page",
        }
    }
}

/// "8", "8:30", "8am", "5:15pm", "17:00" → "08:00"-style 24-hour time.
pub fn parse_time(s: &str) -> Option<String> {
    let t = s.trim().to_lowercase().replace(' ', "");
    let (t, pm, am) = match (t.strip_suffix("pm"), t.strip_suffix("am")) {
        (Some(x), _) => (x.to_string(), true, false),
        (_, Some(x)) => (x.to_string(), false, true),
        _ => (t.clone(), false, false),
    };
    let (h, m) = t.split_once(':').map_or((t.as_str(), "0"), |(h, m)| (h, m));
    let (mut h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    if pm && h < 12 {
        h += 12;
    }
    if am && h == 12 {
        h = 0;
    }
    (h < 24 && m < 60).then(|| format!("{h:02}:{m:02}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationsConfig {
    pub from_valerie: bool,
    pub websites: String,
    pub sites: BTreeMap<String, String>,
    pub sound: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchConfig {
    pub every_minutes: f64,
    pub notify: bool,
}

/// Which agent (by name) does each job; empty = agents.default.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentUse {
    pub chat: String,
    pub find: String,
    pub tasks: String,
    pub customize: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatsConfig {
    pub enabled: bool,
    pub theme: String,
    pub access: String,
    pub effort: String,
    pub own_tools: bool,
    pub auto_settle_days: f64,
    pub plan_button: bool,
    pub composer_hint: bool,
    pub sync: bool,
    pub ask_in_find: bool,
    pub find_question_words: usize,
    pub page_text_limit: usize,
    pub browser_context: bool,
    pub suggestions: Vec<String>,
    pub projects: Vec<String>,
    pub group_by_project: bool,
    pub steps: String,
    pub usage: bool,
    pub changes: bool,
    pub change_actions: BTreeMap<String, Option<String>>,
    pub worktrees: String,
    pub worktree_folder: String,
    pub worktree_copy: Vec<String>,
    pub closed_list: bool,
    pub from_space: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalPanelConfig {
    pub enabled: bool,
    pub shell: String,
    pub folder: String,
    pub height: f64,
    pub font: String,
    pub font_size: f64,
    pub line_height: f64,
    pub cursor: String,
    pub cursor_blink: bool,
    pub scrollback: u32,
    pub copy_on_select: bool,
    pub follow_chat: bool,
    pub ask_agent_button: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    pub remember: bool,
    pub width: f64,
    pub height: f64,
    pub screen_fraction: f64,
    pub min_width: f64,
    pub min_height: f64,
    pub always_on_top: bool,
    pub opacity: f64,
    pub vibrancy: String,
    pub traffic_lights: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeekConfig {
    pub margin: f64,
    pub dim: f64,
    pub corner_radius: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryConfig {
    pub max_pages: usize,
    pub max_archived_tabs: usize,
    pub max_closed_tabs: usize,
    #[serde(default)]
    pub page: HistoryPage,
}

/// The History page (valerie://page/history).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryPage {
    /// day | site | none
    pub group_by: String,
    pub site_icons: bool,
    pub show_visits: bool,
    pub show_address: bool,
}

impl Default for HistoryPage {
    fn default() -> Self {
        HistoryPage { group_by: "day".into(), site_icons: true, show_visits: true, show_address: true }
    }
}

/// The Downloads page (valerie://page/downloads).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadsPage {
    /// day | site | type | none
    pub group_by: String,
    pub show_site: bool,
    /// file | folder
    pub click_opens: String,
}

impl Default for DownloadsPage {
    fn default() -> Self {
        DownloadsPage { group_by: "day".into(), show_site: true, click_opens: "file".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadsConfig {
    pub folder: String,
    pub keep_in_list: usize,
    pub ask: bool,
    pub open_when_done: bool,
    pub rules: Vec<DownloadRule>,
    #[serde(default)]
    pub page: DownloadsPage,
}

/// Where files of a kind (or from a site) go.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DownloadRule {
    /// File names to match, e.g. "*.pdf" or "*.{jpg,png}"; empty = any.
    #[serde(rename = "match")]
    pub pattern: String,
    /// Sites it came from (hosts); empty = any.
    pub site: String,
    /// Folder to save into (~ allowed); empty = the usual folder.
    pub folder: String,
    pub open: Option<bool>,
    pub ask: Option<bool>,
}

impl Config {
    /// Every task: `tasks`, plus the older `briefs` (pages) and `agents.jobs` (results as notifications).
    pub fn all_tasks(&self) -> Vec<TaskConfig> {
        let mut all: Vec<TaskConfig> = self.tasks.iter().filter(|t| t.enabled).cloned().collect();
        all.extend(self.briefs.iter().filter(|t| t.enabled).cloned());
        for j in self.agents.jobs.iter().filter(|j| j.enabled) {
            let when = match (&j.at, &j.every) {
                (Some(at), _) => format!("every day at {at}"),
                (None, Some(every)) if every.ends_with('d') => "every day at 09:00".into(),
                (None, Some(every)) => format!("every {every}"),
                (None, None) => "only when I ask".into(),
            };
            all.push(TaskConfig {
                name: j.name.clone(),
                want: j.prompt.clone(),
                when,
                result: "notify".into(),
                agent: j.agent.clone().unwrap_or_default(),
                model: j.model.clone().unwrap_or_default(),
                ..Default::default()
            });
        }
        all
    }

    /// The agent for a job ("chat", "find", "tasks", "customize"): its name and definition. A name that isn't under
    /// agents.available but is a known CLI (e.g. "codex") means that CLI with the default model.
    pub fn agent_for(&self, job: &str) -> (String, AgentDef) {
        let u = &self.agents.use_for;
        let pick = match job {
            "chat" => &u.chat,
            "find" => &u.find,
            "tasks" => &u.tasks,
            "customize" => &u.customize,
            _ => &self.agents.default,
        };
        let name = if pick.is_empty() { self.agents.default.clone() } else { pick.clone() };
        self.agent_named(&name)
    }

    /// The user's own instructions for agents doing `job` (chat, find, tasks, customize), ready to go after Valerie's
    /// own rules; empty when they have none.
    pub fn extra_instructions(&self, job: &str) -> String {
        let mine: Vec<&str> = [self.agents.instructions.as_str(), self.agents.instructions_for.get(job).map(String::as_str).unwrap_or("")]
            .into_iter()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if mine.is_empty() {
            return String::new();
        }
        format!("\n\nThe user's own instructions for you (follow them unless they conflict with the rules above):\n{}", mine.join("\n"))
    }

    pub fn agent_named(&self, name: &str) -> (String, AgentDef) {
        let def = self.agent_def(name);
        if self.agents.fallback && def.command.is_empty() && crate::ai::kind(&def.cli).is_some() && !installed(&def.cli) {
            if let Some(k) = crate::ai::KINDS.iter().find(|k| installed(k.id)) {
                // Another machine's choice: use what this one has (its own model, not the other CLI's).
                let mut sub = self.agent_def(k.id);
                if self.agents.available.get(k.id).cloned().flatten().is_none_or(|d| d.model.is_empty()) {
                    sub.model.clear();
                }
                note_fallback(&def.cli, k.id);
                return (k.id.to_string(), sub);
            }
        }
        (name.to_string(), def)
    }

    fn agent_def(&self, name: &str) -> AgentDef {
        let mut def = self.agents.available.get(name).cloned().flatten().unwrap_or_else(|| AgentDef { cli: name.to_string(), ..Default::default() });
        if def.cli.is_empty() && def.command.is_empty() {
            def.cli = name.to_string();
        }
        if def.model.is_empty() {
            def.model = self.agents.model.clone();
        }
        def
    }
}

impl DownloadsConfig {
    /// Where downloads and captures go: `folder` (~ allowed), else the system Downloads folder.
    pub fn dir(&self) -> PathBuf {
        let f = self.folder.trim();
        if f.is_empty() {
            return dirs::download_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default());
        }
        match f.strip_prefix("~/") {
            Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
            None => PathBuf::from(f),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyConfig {
    pub strip_tracking: bool,
    pub tracking_params: Vec<String>,
    pub cookie_banners: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionsConfig {
    pub camera: String,
    pub microphone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PagesConfig {
    pub dark_mode: String,
    pub loading_bar: bool,
    pub markdown: bool,
    pub markdown_theme: String,
    pub markdown_outline: bool,
    pub markdown_live: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslateConfig {
    pub language: String,
    pub service: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    pub copy_to_clipboard: bool,
    pub save_to_downloads: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LittleWindowConfig {
    pub enabled: bool,
    pub space_picker: bool,
    pub copy_button: bool,
    pub keep_open_when_main_closes: bool,
    pub open_links_alone: bool,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileConfig {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceConfig {
    pub name: String,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub theme: Option<ThemeRef>,
    #[serde(default)]
    pub icon: Option<String>,
    /// Settings that apply while this space is the one you're in (same shape as config.yaml).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ThemeRef {
    Named(String),
    Inline(Theme),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    #[serde(default = "dark")]
    pub mode: String,
    pub background: Vec<String>,
    pub accent: String,
    #[serde(default)]
    pub foreground: Option<String>,
}

fn dark() -> String {
    "dark".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    #[serde(rename = "match")]
    pub pattern: String,
    pub space: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Binding {
    Name(String),
    Full(Command),
}

impl Binding {
    pub fn command(&self) -> Option<Command> {
        match self {
            Binding::Name(n) => Command::from_name(n),
            Binding::Full(c) => Some(c.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VimConfig {
    pub enabled: bool,
    pub key_timeout_ms: f64,
    pub scroll_step: f64,
    pub smooth_scroll: bool,
    pub scroll_duration_ms: f64,
    pub hold_to_accelerate: bool,
    pub block_autofocus: bool,
    pub jump_to_text_box: bool,
    pub hold_delay_ms: f64,
    pub hold_scroll_speed: f64,
    pub hold_scroll_acceleration: f64,
    pub hold_scroll_max_speed: f64,
    pub hint_characters: String,
    pub excluded_hosts: Vec<String>,
    pub bindings: BTreeMap<String, Option<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    pub enabled: bool,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceConfig {
    pub index: String,
    pub repo: String,
    pub offer_to_share: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatesConfig {
    pub check: bool,
    pub every_hours: f64,
    pub install: String,
    pub feed: String,
    pub channel: String,
}

impl UpdatesConfig {
    /// Where to look for updates: the edge feed on the edge channel (unless `feed` points somewhere of its own).
    pub fn feed_url(&self) -> String {
        if self.channel == "edge" && (self.feed.is_empty() || self.feed == crate::update::STABLE_FEED) {
            crate::update::EDGE_FEED.to_string()
        } else {
            self.feed.clone()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub repo: String,
    pub device: String,
    pub interval_minutes: f64,
    pub on_change: bool,
    pub delay_seconds: f64,
    pub repo_name: String,
    pub github_client_id: String,
    pub encrypt: bool,
    pub passwords: bool,
    pub ask_before_plugins: bool,
    pub repo_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordsConfig {
    pub enabled: bool,
    pub save_prompt_seconds: f64,
    pub offer_passkeys: String,
    pub passkeys_through_safari: bool,
    pub shared: Vec<SharedVault>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedVault {
    pub name: String,
    pub profiles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockingConfig {
    pub enabled: bool,
    pub lists: Vec<String>,
    pub allow: Vec<String>,
    pub refresh_days: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentsConfig {
    pub timeout_minutes: f64,
    pub available: BTreeMap<String, Option<AgentDef>>,
    pub models: BTreeMap<String, Vec<String>>,
    #[serde(rename = "use")]
    pub use_for: AgentUse,
    pub default: String,
    /// Model passed to the agent CLI; empty = the CLI's own default.
    pub model: String,
    pub space: String,
    pub jobs: Vec<AgentJob>,
    /// Added to every agent's instructions (after Valerie's own safety rules).
    pub instructions: String,
    /// Added for one job: chat, find, tasks, customize.
    pub instructions_for: BTreeMap<String, String>,
    /// Use an installed agent when the chosen one isn't on this computer.
    pub fallback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentJob {
    pub name: String,
    pub prompt: String,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// Interval like "30m", "6h", "1d".
    #[serde(default)]
    pub every: Option<String>,
    /// Daily local time "HH:MM".
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// Parse "90s", "30m", "6h", "1d" into seconds.

/// Tests that point VALERIE_CONFIG_DIR somewhere take turns (the environment is shared by every test thread).
#[cfg(test)]
pub static TEST_DIRS: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn config_dir() -> PathBuf {
    if let Ok(d) = std::env::var("VALERIE_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    dirs::home_dir().unwrap_or_default().join(".config").join("valerie")
}

pub fn data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("VALERIE_DATA_DIR") {
        return PathBuf::from(d);
    }
    dirs::data_dir().unwrap_or_default().join("Valerie")
}

/// Set one setting in the user's config.yaml (`path` like ["agents", "default"], `value` written as YAML), keeping
/// the rest of the file and its comments. Only falls back to rewriting the file if the key sits inside a one-line
/// `{ … }` block.
pub fn set_setting(path: &[&str], value: &str) -> Result<()> {
    let file = config_path();
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let new = match set_in_text(&text, path, value) {
        Some(t) => t,
        None => {
            let mut v: serde_yaml::Value = serde_yaml::from_str(&text).unwrap_or(serde_yaml::Value::Mapping(Default::default()));
            let mut at = &mut v;
            for k in path {
                if !at.get(*k).is_some_and(|x| x.is_mapping()) {
                    at[*k] = serde_yaml::Value::Mapping(Default::default());
                }
                at = &mut at[*k];
            }
            *at = serde_yaml::from_str(value)?;
            serde_yaml::to_string(&v)?
        }
    };
    std::fs::write(&file, new)?;
    Ok(())
}

/// `set_setting` on text; None when a parent key holds a one-line value it can't edit in place.
fn set_in_text(text: &str, path: &[&str], value: &str) -> Option<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let indent_of = |l: &str| l.len() - l.trim_start().len();
    let blank = |l: &str| l.trim().is_empty() || l.trim_start().starts_with('#');
    let (mut start, mut end, mut indent) = (0, lines.len(), 0);
    for (depth, key) in path.iter().enumerate() {
        let last = depth + 1 == path.len();
        let found = (start..end).find(|&i| {
            let l = &lines[i];
            !blank(l) && indent_of(l) == indent && l.trim_start().strip_prefix(*key).is_some_and(|r| r.starts_with(':'))
        });
        match found {
            Some(i) => {
                let rest = lines[i].trim_start()[key.len() + 1..].to_string();
                let rest = rest.split(" #").next().unwrap_or("").trim().to_string();
                if last {
                    lines[i] = format!("{}{key}: {value}", " ".repeat(indent));
                    // A block value (a list or map on the lines below) is replaced too, not left behind.
                    let below = (i + 1..end).find(|&j| !blank(&lines[j]) && (indent_of(&lines[j]) <= indent && !lines[j].trim_start().starts_with("- ") || indent_of(&lines[j]) < indent)).unwrap_or(end);
                    let last_child = (i + 1..below).rev().find(|&j| !blank(&lines[j])).map_or(i, |j| j);
                    lines.drain(i + 1..=last_child);
                    return Some(lines.join("\n") + "\n");
                }
                if !rest.is_empty() {
                    return None;
                }
                let block_end = (i + 1..end).find(|&j| !blank(&lines[j]) && indent_of(&lines[j]) <= indent).unwrap_or(end);
                let child = (i + 1..block_end).find(|&j| !blank(&lines[j])).map(|j| indent_of(&lines[j])).unwrap_or(indent + 2);
                (start, end, indent) = (i + 1, block_end, child);
            }
            None => {
                // Add the missing keys at the end of the parent block (or the file).
                let mut add = Vec::new();
                let mut ind = indent;
                for (k, key) in path[depth..].iter().enumerate() {
                    if depth + k + 1 == path.len() {
                        add.push(format!("{}{key}: {value}", " ".repeat(ind)));
                    } else {
                        add.push(format!("{}{key}:", " ".repeat(ind)));
                        ind += 2;
                    }
                }
                let mut at = end;
                while at > start && lines[at - 1].trim().is_empty() {
                    at -= 1;
                }
                if depth == 0 && at > 0 {
                    add.insert(0, String::new());
                }
                for (k, l) in add.into_iter().enumerate() {
                    lines.insert(at + k, l);
                }
                return Some(lines.join("\n") + "\n");
            }
        }
    }
    None
}

/// What config/default.yaml says about a setting (`["sidebar", "tab_height"]`): its line and comments, or the whole
/// section with its banner for a section. None when it isn't a setting.
pub fn reference(path: &[&str]) -> Option<String> {
    let lines: Vec<&str> = DEFAULT_YAML.lines().collect();
    let indent_of = |l: &str| l.len() - l.trim_start().len();
    let blank = |l: &str| l.trim().is_empty() || l.trim_start().starts_with('#');
    let (mut start, mut end, mut indent) = (0, lines.len(), 0);
    for (depth, key) in path.iter().enumerate() {
        let i = (start..end).find(|&i| !blank(lines[i]) && indent_of(lines[i]) == indent && lines[i].trim_start().strip_prefix(*key).is_some_and(|r| r.starts_with(':')))?;
        let block_end = (i + 1..end).find(|&j| !blank(lines[j]) && indent_of(lines[j]) <= indent).unwrap_or(end);
        if depth + 1 == path.len() {
            let mut from = i;
            while from > start && lines[from - 1].trim_start().starts_with('#') && indent_of(lines[from - 1]) == indent {
                from -= 1;
            }
            let mut to = block_end;
            while to > i + 1 && lines[to - 1].trim().is_empty() {
                to -= 1;
            }
            return Some(lines[from..to].join("\n"));
        }
        let child = (i + 1..block_end).find(|&j| !blank(lines[j])).map(|j| indent_of(lines[j]))?;
        (start, end, indent) = (i + 1, block_end, child);
    }
    None
}

/// Change one setting the way an agent asks to (`value` is YAML or JSON): only real settings, and the file goes back
/// to how it was if Valerie can't load the result. In this-mac.yaml when `this_computer`.
pub fn change_setting(path: &[&str], value: &str, this_computer: bool) -> Result<()> {
    let def: Value = serde_yaml::from_str(DEFAULT_YAML)?;
    let mut at = &def;
    for (i, k) in path.iter().enumerate() {
        if FREE_FORM.contains(&path[..i].join(".").as_str()) {
            break;
        }
        at = at.get(*k).with_context(|| format!("`{}` isn't a setting; look in get_settings for the right name", path[..=i].join(".")))?;
    }
    let parsed: Value = serde_yaml::from_str(value).with_context(|| format!("{value:?} isn't a value Valerie can read"))?;
    let text = serde_json::to_string(&parsed)?;
    let file = if this_computer { this_mac_path() } else { config_path() };
    let before = std::fs::read_to_string(&file).ok();
    let (known, _) = crate::config_problems();
    if this_computer { set_this_mac_setting(path, &text)? } else { set_setting(path, &text)? }
    let new: Vec<String> = crate::config_problems().0.into_iter().filter(|p| !known.contains(p)).collect();
    if !new.is_empty() {
        match before {
            Some(b) => std::fs::write(&file, b)?,
            None => std::fs::remove_file(&file)?,
        }
        anyhow::bail!("nothing changed, because {}", new.join("; "));
    }
    Ok(())
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.yaml")
}

/// Settings for this computer only: merged over config.yaml, never synced.
pub fn this_mac_path() -> PathBuf {
    config_dir().join("this-mac.yaml")
}

/// `set_setting` for this computer only (this-mac.yaml).
pub fn set_this_mac_setting(path: &[&str], value: &str) -> Result<()> {
    let file = this_mac_path();
    let text = std::fs::read_to_string(&file).unwrap_or_else(|_| "# Settings for this computer only (they win over config.yaml and never sync).\n".into());
    let new = set_in_text(&text, path, value).context("this-mac.yaml has a one-line block there")?;
    std::fs::create_dir_all(config_dir())?;
    std::fs::write(&file, new)?;
    Ok(())
}

/// Whether an agent CLI is on this computer (checked at most every 30 seconds, so installing one is noticed).
pub fn installed(cli: &str) -> bool {
    use std::sync::Mutex;
    static SEEN: Mutex<Option<(std::time::Instant, BTreeMap<String, bool>)>> = Mutex::new(None);
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen.as_ref().is_none_or(|(t, _)| t.elapsed() > std::time::Duration::from_secs(30)) {
        *seen = Some((std::time::Instant::now(), BTreeMap::new()));
    }
    let map = &mut seen.as_mut().expect("just set").1;
    *map.entry(cli.to_string()).or_insert_with(|| crate::ai::kind(cli).is_some_and(|k| crate::ai::find_bin(k, &crate::ai::search_path()).is_some()))
}

static FALLBACK_NOTE: std::sync::Mutex<(Vec<String>, Option<String>)> = std::sync::Mutex::new((Vec::new(), None));

fn note_fallback(missing: &str, using: &str) {
    let mut n = FALLBACK_NOTE.lock().unwrap_or_else(|e| e.into_inner());
    let pair = format!("{missing}>{using}");
    if !n.0.contains(&pair) {
        n.0.push(pair);
        let title = |id: &str| crate::ai::kind(id).map_or(id.to_string(), |k| k.title.to_string());
        n.1 = Some(format!("{} isn't on this computer, so Valerie is using {} here", title(missing), title(using)));
    }
}

/// A message to show once when an agent stood in for one that isn't installed here.
pub fn take_fallback_note() -> Option<String> {
    FALLBACK_NOTE.lock().unwrap_or_else(|e| e.into_inner()).1.take()
}

/// A random name for this installation that never changes (unlike the hostname, which macOS renumbers when another
/// device on the network has the same name). Names this machine's files in the sync repo.
pub fn device_id() -> String {
    let path = data_dir().join("device-id");
    if let Some(id) = std::fs::read_to_string(&path).ok().map(|s| s.trim().to_string()).filter(|s| s.len() >= 8) {
        return id;
    }
    let id = crate::crypt::hex(&crate::crypt::random::<6>());
    let _ = std::fs::create_dir_all(data_dir());
    let _ = std::fs::write(&path, &id);
    id
}

/// User values override defaults key by key; lists replace wholesale; `null` removes a map entry.
/// Settings whose keys are names you choose (shortcuts, sites, themes…) rather than fixed options.
const FREE_FORM: &[&str] = &[
    "keybindings", "site_search", "site_profiles", "themes", "vim.bindings", "command_bar.result_weights", "agents.available", "agents.models",
    "ai_actions", "notifications.sites", "global_keybindings", "sites", "menu.rename", "menu.commands", "agents.instructions_for",
];

/// Keys in the user's config that aren't Valerie settings (usually typos, which would otherwise be silently
/// ignored), each with the closest real key when there is one.
pub fn unknown_keys(user_text: &str) -> Vec<String> {
    fn distance(a: &str, b: &str) -> usize {
        let b: Vec<char> = b.chars().collect();
        let mut row: Vec<usize> = (0..=b.len()).collect();
        for (i, ca) in a.chars().enumerate() {
            let mut prev = row[0];
            row[0] = i + 1;
            for j in 0..b.len() {
                let cur = row[j + 1];
                row[j + 1] = (prev + usize::from(ca != b[j])).min(row[j] + 1).min(cur + 1);
                prev = cur;
            }
        }
        row[b.len()]
    }
    fn walk(path: &str, user: &Value, def: &Value, out: &mut Vec<String>) {
        let (Value::Mapping(u), Value::Mapping(d)) = (user, def) else { return };
        if FREE_FORM.contains(&path) {
            return;
        }
        for (k, v) in u {
            let Some(key) = k.as_str() else { continue };
            let full = if path.is_empty() { key.to_string() } else { format!("{path}.{key}") };
            match d.get(k) {
                Some(dv) => walk(&full, v, dv, out),
                None => {
                    let close = d.keys().filter_map(Value::as_str).map(|c| (distance(key, c), c)).filter(|(n, _)| *n <= 3).min();
                    out.push(match close {
                        Some((_, c)) => format!("`{full}` isn't a setting (did you mean `{c}`?)"),
                        None => format!("`{full}` isn't a setting"),
                    });
                }
            }
        }
    }
    let (Ok(user), Ok(def)) = (serde_yaml::from_str::<Value>(user_text), serde_yaml::from_str::<Value>(DEFAULT_YAML)) else { return vec![] };
    let mut out = Vec::new();
    walk("", &user, &def, &mut out);
    out
}

pub fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Mapping(b), Value::Mapping(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(bv @ Value::Mapping(_)) if v.is_mapping() => merge(bv, v),
                    _ => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}

impl Config {
    pub fn defaults() -> Config {
        serde_yaml::from_str(DEFAULT_YAML).expect("default config is valid")
    }

    /// Load the user config, writing a small starter file on first run (defaults fill in the rest).
    pub fn load() -> Result<Config> {
        let path = config_path();
        if !path.exists() {
            std::fs::create_dir_all(config_dir())?;
            std::fs::write(&path, STARTER_YAML)?;
        }
        let text = std::fs::read_to_string(&path)?;
        Self::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Config> {
        Ok(serde_yaml::from_value(Self::merged(text)?)?)
    }

    /// The defaults with the user's file on top, then this computer's own settings, as YAML (what spaces'
    /// `settings` go on top of).
    pub fn merged(text: &str) -> Result<Value> {
        #[allow(unused_mut)]
        let mut base = Self::merged_shared(text)?;
        #[cfg(not(test))]
        if let Some(here) = std::fs::read_to_string(this_mac_path()).ok().and_then(|t| serde_yaml::from_str::<Value>(&t).ok()).filter(|v| v.is_mapping()) {
            merge(&mut base, here);
        }
        Ok(base)
    }

    /// The defaults with the user's (shared, synced) file on top, without this computer's own settings.
    pub fn merged_shared(text: &str) -> Result<Value> {
        let mut base: Value = serde_yaml::from_str(DEFAULT_YAML)?;
        let user: Value = serde_yaml::from_str(text)?;
        if !user.is_null() {
            merge(&mut base, user);
        }
        Ok(base)
    }

    /// These settings with a space's own `settings` on top (None when it has none).
    pub fn for_space(&self, base: &Value, space: &str) -> Option<Result<Config>> {
        let over = self.spaces.iter().find(|s| s.name == space)?.settings.clone()?;
        let mut v = base.clone();
        merge(&mut v, serde_yaml::to_value(over).ok()?);
        Some(serde_yaml::from_value(v).map_err(|e| anyhow::anyhow!("space {space:?} settings: {e}")))
    }

    pub fn theme(&self, r: Option<&ThemeRef>) -> Theme {
        match r {
            Some(ThemeRef::Inline(t)) => t.clone(),
            Some(ThemeRef::Named(n)) => self
                .themes
                .get(n)
                .cloned()
                .or_else(|| preset(n))
                .unwrap_or_else(|| preset("midnight").unwrap()),
            None => preset("midnight").unwrap(),
        }
    }

    pub fn theme_names(&self) -> Vec<String> {
        let mut v: Vec<String> = PRESETS.iter().map(|p| p.0.to_string()).collect();
        v.extend(self.themes.keys().cloned());
        v
    }

    /// Vaults a profile can use: its own first, then shared vaults that include it.
    pub fn vaults_for(&self, profile: &str) -> Vec<String> {
        let mut v = vec![profile.to_string()];
        v.extend(self.passwords.shared.iter().filter(|s| s.profiles.iter().any(|p| p == profile)).map(|s| s.name.clone()));
        v
    }

    pub fn device_name(&self) -> String {
        if !self.sync.device.is_empty() {
            return self.sync.device.clone();
        }
        #[cfg(target_os = "macos")]
        if let Some(n) = std::process::Command::new("scutil").args(["--get", "ComputerName"]).output().ok().and_then(|o| String::from_utf8(o.stdout).ok()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            return n;
        }
        std::process::Command::new("hostname")
            .arg("-s")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "device".into())
    }
}

const PRESETS: &[(&str, &str, &[&str], &str)] = &[
    ("midnight", "dark", &["#171a2b", "#2a1f3d"], "#9d8cff"),
    ("rose", "dark", &["#2b1520", "#4a1f33"], "#ff7ab6"),
    ("forest", "dark", &["#10211b", "#1d3a2c"], "#5fd39a"),
    ("ocean", "dark", &["#0e1c2e", "#123a52"], "#4cc2ff"),
    ("grape", "dark", &["#1d1430", "#3b1f5e"], "#c58bff"),
    ("ember", "dark", &["#24130c", "#4a2412"], "#ff9a4c"),
    ("sand", "light", &["#f3e9dc", "#e8d5bf"], "#c2703d"),
    ("paper", "light", &["#f6f6f4", "#e9e9e6"], "#3d6bff"),
    ("black", "dark", &["#000000", "#000000"], "#9a9aff"),
];

pub fn preset(name: &str) -> Option<Theme> {
    PRESETS.iter().find(|p| p.0 == name).map(|p| Theme {
        mode: p.1.into(),
        background: p.2.iter().map(|s| s.to_string()).collect(),
        accent: p.3.into(),
        foreground: None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn sets_settings_in_place() {
        let text = "# mine\nprofiles:\n  - name: Me\nagents:\n  default: claude   # who answers\n  available:\n    claude: { cli: claude }\n\nsync: { repo: x }\n";
        let t = super::set_in_text(text, &["agents", "default"], "opencode").unwrap();
        assert!(t.contains("  default: opencode\n") && t.contains("# mine") && t.contains("    claude: { cli: claude }"));
        let t = super::set_in_text(&t, &["agents", "available", "opencode"], "{ cli: opencode, model: \"opencode/x-free\" }").unwrap();
        assert!(t.contains("    claude: { cli: claude }\n    opencode: { cli: opencode, model: \"opencode/x-free\" }\n"), "{t}");
        let t = super::set_in_text(&t, &["welcome"], "false").unwrap();
        assert!(t.ends_with("sync: { repo: x }\n\nwelcome: false\n"), "{t}");
        assert!(super::set_in_text(&t, &["sync", "repo"], "y").is_none());
        let blocks = "spaces:\n  - name: A\n    icon: x\n  - name: B\nwelcome: true\nlist:\n- a\n- b\n# end\n";
        let t = super::set_in_text(blocks, &["spaces"], "[{\"name\":\"B\"}]").unwrap();
        let t = super::set_in_text(&t, &["list"], "[c]").unwrap();
        assert_eq!(t, "spaces: [{\"name\":\"B\"}]\nwelcome: true\nlist: [c]\n# end\n");
        let t = super::set_in_text("", &["agents", "default"], "pi").unwrap();
        assert_eq!(t, "agents:\n  default: pi\n");
        let parsed: serde_yaml::Value = serde_yaml::from_str(&super::set_in_text(text, &["agents", "available", "pi"], "{ cli: pi }").unwrap()).unwrap();
        assert_eq!(parsed["agents"]["available"]["pi"]["cli"], "pi");
    }

    use super::*;

    #[test]
    fn defaults_parse_and_bindings_resolve() {
        let c = Config::defaults();
        assert!(c.keybindings.len() > 10);
        for (k, b) in &c.keybindings {
            assert!(b.as_ref().unwrap().command().is_some(), "binding {k}");
        }
    }

    #[test]
    fn starter_parses_and_task_schedules() {
        let c = Config::parse(STARTER_YAML).unwrap();
        assert_eq!(c.spaces.len(), 3);
        let daily = |at: &str, days: &str| Some(When::Daily { at: at.into(), days: days.into() });
        assert_eq!(parse_when("every day at 8am"), daily("08:00", "every day"));
        assert_eq!(parse_when("weekdays at 9"), daily("09:00", "weekdays"));
        assert_eq!(parse_when("Mondays and Fridays at 5:30 pm"), daily("17:30", "mon, fri"));
        assert_eq!(parse_when("17:00"), daily("17:00", "every day"));
        assert_eq!(parse_when("every 5 minutes"), Some(When::Every(300)));
        assert_eq!(parse_when("every 30m"), Some(When::Every(1800)));
        assert_eq!(parse_when("every 2 hours"), Some(When::Every(7200)));
        assert_eq!(parse_when("hourly"), Some(When::Every(3600)));
        assert_eq!(parse_when("when Valerie starts"), Some(When::OnStart));
        assert_eq!(parse_when("only when I ask"), Some(When::Manual));
        assert_eq!(parse_when("every morning"), None);
        assert_eq!(parse_when(&daily("08:00", "weekdays").unwrap().describe()), daily("08:00", "weekdays"));
    }

    #[test]
    fn user_overrides_merge() {
        let c = Config::parse("layout: { bezel: 8 }\nkeybindings: { \"CmdOrCtrl+D\": null }\n").unwrap();
        assert_eq!(c.layout.bezel, 8.0);
        assert_eq!(c.layout.sidebar_width, 248.0);
        assert!(c.keybindings["CmdOrCtrl+D"].is_none());
        assert!(c.keybindings.contains_key("CmdOrCtrl+L"));
    }

    /// An agent that isn't on this computer is replaced by one that is (with that one's own model), once noted.
    #[test]
    fn missing_agent_falls_back() {
        let Some(missing) = crate::ai::KINDS.iter().rev().find(|k| !super::installed(k.id)) else { return };
        let Some(present) = crate::ai::KINDS.iter().find(|k| super::installed(k.id)) else { return };
        let c = Config::parse(&format!("agents: {{ default: {}, available: {{ {}: {{ cli: {}, model: big-model }} }} }}\n", missing.id, missing.id, missing.id)).unwrap();
        let (name, def) = c.agent_named(missing.id);
        assert_eq!(name, present.id);
        assert_eq!(def.cli, present.id);
        assert!(def.model.is_empty(), "the missing agent's model went along: {}", def.model);
        assert!(super::take_fallback_note().is_some_and(|n| n.contains(missing.title)));
        let off = Config::parse(&format!("agents: {{ default: {}, fallback: false }}\n", missing.id)).unwrap();
        assert_eq!(off.agent_named(missing.id).0, missing.id);
    }

    /// Stable by default; edge reads the edge feed, unless the feed was pointed somewhere of its own (a fork).
    #[test]
    fn update_channels_pick_their_feed() {
        let stable = Config::parse("").unwrap();
        assert_eq!(stable.updates.channel, "stable");
        assert_eq!(stable.updates.feed_url(), crate::update::STABLE_FEED);
        let edge = Config::parse("updates: { channel: edge }\n").unwrap();
        assert_eq!(edge.updates.feed_url(), crate::update::EDGE_FEED);
        let fork = Config::parse("updates: { channel: edge, feed: \"https://example.com/latest.json\" }\n").unwrap();
        assert_eq!(fork.updates.feed_url(), "https://example.com/latest.json");
    }

    /// Toggle Test Builds writes this-mac.yaml: turning them off also undoes an explicit edge feed (the owner's Mac
    /// was put on edge that way before channels existed), so it really goes back to stable.
    #[test]
    fn test_builds_toggle_writes_this_mac() {
        let _g = super::TEST_DIRS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("valerie-channel-test-{}", crate::crypt::hex(&crate::crypt::random::<4>())));
        std::env::set_var("VALERIE_CONFIG_DIR", &dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(super::this_mac_path(), format!("# mine\nupdates:\n  channel: edge\n  feed: \"{}\"\n", crate::update::EDGE_FEED)).unwrap();
        let read = || Config::parse(&std::fs::read_to_string(super::this_mac_path()).unwrap()).unwrap();
        assert_eq!(read().updates.feed_url(), crate::update::EDGE_FEED);
        super::set_this_mac_setting(&["updates", "channel"], "stable").unwrap();
        super::set_this_mac_setting(&["updates", "feed"], &serde_json::json!(crate::update::STABLE_FEED).to_string()).unwrap();
        assert_eq!(read().updates.channel, "stable");
        assert_eq!(read().updates.feed_url(), crate::update::STABLE_FEED);
        assert!(std::fs::read_to_string(super::this_mac_path()).unwrap().starts_with("# mine"));
        super::set_this_mac_setting(&["updates", "channel"], "edge").unwrap();
        assert_eq!(read().updates.feed_url(), crate::update::EDGE_FEED);
        std::env::remove_var("VALERIE_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reference_quotes_default_yaml() {
        let tab = super::reference(&["sidebar", "tab_height"]).unwrap();
        assert!(tab.trim_start().starts_with("tab_height:") && tab.contains('#'), "{tab}");
        let logs = super::reference(&["logs"]).unwrap();
        assert!(logs.starts_with("# Valerie's log") && logs.contains("level: events"), "{logs}");
        assert!(super::reference(&["sidebar", "nope"]).is_none());
    }

    /// Agents' set_setting: real settings only, checked values, and the file is left alone when it's refused.
    #[test]
    fn change_setting_checks_before_keeping() {
        let _g = super::TEST_DIRS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("valerie-set-test-{}", crate::crypt::hex(&crate::crypt::random::<4>())));
        std::env::set_var("VALERIE_CONFIG_DIR", &dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(super::config_path(), "# mine\nsidebar:\n  tab_height: 30\n").unwrap();
        super::change_setting(&["sidebar", "tab_height"], "40", false).unwrap();
        super::change_setting(&["sites", "example.com", "ad_blocking"], "false", false).unwrap();
        let typo = super::change_setting(&["sidebar", "tab_hieght"], "40", false).unwrap_err().to_string();
        let bad = super::change_setting(&["logs", "level"], "\"loud\"", false).unwrap_err().to_string();
        let text = std::fs::read_to_string(super::config_path()).unwrap();
        std::env::remove_var("VALERIE_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(typo.contains("isn't a setting"), "{typo}");
        assert!(bad.contains("level"), "{bad}");
        assert!(text.starts_with("# mine") && text.contains("tab_height: 40") && !text.contains("loud"), "{text}");
        assert!(text.contains("example.com"), "{text}");
    }
}
