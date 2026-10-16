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
    pub new_tab_url: String,
    pub user_agent: String,
    pub layout: Layout,
    pub sidebar: SidebarConfig,
    pub start_page: String,
    pub escape_hints: bool,
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
    pub agents: AgentsConfig,
    pub content_blocking: BlockingConfig,
    pub passwords: PasswordsConfig,
    pub themes: BTreeMap<String, Theme>,
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
    pub new_tab_button: bool,
    pub today_label: bool,
    pub clear_button: String,
    pub space_switcher: bool,
    pub tab_height: f64,
    pub now_playing: bool,
    pub compact_width: f64,
    pub medium_width: f64,
    pub max_width: f64,
    pub pinned_columns: u32,
    pub pinned_tile_height: f64,
    pub swipe_between_spaces: bool,
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    pub width: f64,
    pub height: f64,
    pub min_width: f64,
    pub min_height: f64,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadsConfig {
    pub folder: String,
    pub keep_in_list: usize,
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

    pub fn agent_named(&self, name: &str) -> (String, AgentDef) {
        let mut def = self.agents.available.get(name).cloned().flatten().unwrap_or_else(|| AgentDef { cli: name.to_string(), ..Default::default() });
        if def.cli.is_empty() && def.command.is_empty() {
            def.cli = name.to_string();
        }
        if def.model.is_empty() {
            def.model = self.agents.model.clone();
        }
        (name.to_string(), def)
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
pub struct SyncConfig {
    pub repo: String,
    pub device: String,
    pub interval_minutes: f64,
    pub on_change: bool,
    pub delay_seconds: f64,
    pub repo_name: String,
    pub github_client_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordsConfig {
    pub enabled: bool,
    pub save_prompt_seconds: f64,
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

pub fn config_path() -> PathBuf {
    config_dir().join("config.yaml")
}

/// User values override defaults key by key; lists replace wholesale; `null` removes a map entry.
/// Settings whose keys are names you choose (shortcuts, sites, themes…) rather than fixed options.
const FREE_FORM: &[&str] = &["keybindings", "site_search", "site_profiles", "themes", "vim.bindings", "command_bar.result_weights", "agents.available", "agents.models", "ai_actions", "notifications.sites"];

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

fn merge(base: &mut Value, over: Value) {
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
        let mut base: Value = serde_yaml::from_str(DEFAULT_YAML)?;
        let user: Value = serde_yaml::from_str(text)?;
        if !user.is_null() {
            merge(&mut base, user);
        }
        Ok(serde_yaml::from_value(base)?)
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
}
