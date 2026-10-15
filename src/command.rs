use serde::{Deserialize, Serialize};

pub type TabId = u64;

/// The single action vocabulary: keybindings, sidebar, command bar, vim motions and MCP all dispatch these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    OpenCommandBar {
        #[serde(default)]
        mode: BarMode,
    },
    CloseCommandBar,
    NewTab {
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        space: Option<String>,
        #[serde(default)]
        background: bool,
    },
    /// Open user input (URL or search terms) from the command bar.
    Go {
        input: String,
        #[serde(default)]
        new_tab: bool,
    },
    Navigate {
        url: String,
        #[serde(default)]
        tab: Option<TabId>,
    },
    CloseTab {
        #[serde(default)]
        tab: Option<TabId>,
    },
    ReopenClosedTab,
    ActivateTab {
        tab: TabId,
    },
    NextTab,
    PrevTab,
    GoToTab {
        index: usize,
    },
    TogglePin {
        #[serde(default)]
        tab: Option<TabId>,
    },
    ToggleFavorite {
        #[serde(default)]
        tab: Option<TabId>,
    },
    DuplicateTab,
    MoveTab {
        tab: TabId,
        /// "pinned" | "kept" | "today" (overrides `pinned`).
        #[serde(default)]
        section: Option<String>,
        #[serde(default)]
        favorite: bool,
        #[serde(default)]
        space: Option<String>,
        #[serde(default)]
        folder: Option<TabId>,
        #[serde(default)]
        pinned: Option<bool>,
        #[serde(default)]
        index: Option<usize>,
    },
    RenameTab {
        tab: TabId,
        title: String,
    },
    NewFolder {
        #[serde(default)]
        name: Option<String>,
        /// "kept" (default) or "pinned".
        #[serde(default)]
        section: Option<String>,
    },
    /// Move a tab into (or out of) the Kept list under the pinned tabs.
    ToggleKeep {
        #[serde(default)]
        tab: Option<TabId>,
    },
    /// Step the sidebar through full → medium → compact.
    CycleSidebarSize,
    ToggleFolder {
        folder: TabId,
    },
    RenameFolder {
        folder: TabId,
        name: String,
    },
    DeleteFolder {
        folder: TabId,
    },
    ClearTodayTabs,
    RestoreArchived {
        index: usize,
    },
    Back {
        #[serde(default)]
        tab: Option<TabId>,
    },
    Forward {
        #[serde(default)]
        tab: Option<TabId>,
    },
    Reload {
        #[serde(default)]
        tab: Option<TabId>,
    },
    Stop,
    CopyUrl {
        #[serde(default)]
        tab: Option<TabId>,
    },
    ToggleSidebar,
    ToggleTabBarPosition,
    NextSpace,
    PrevSpace,
    SwitchSpace {
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        name: Option<String>,
    },
    NewSpace {
        #[serde(default)]
        name: Option<String>,
    },
    RenameSpace {
        name: String,
    },
    DeleteSpace {
        #[serde(default)]
        name: Option<String>,
    },
    SetSpaceTheme {
        theme: String,
    },
    SplitView {
        #[serde(default)]
        tab: Option<TabId>,
    },
    Unsplit,
    FocusNextPane,
    /// Move focus to the split-view pane on the left/right; outside split view, go back/forward.
    FocusPane {
        direction: String,
    },
    ZoomIn,
    ZoomOut,
    ZoomReset,
    DevTools,
    OpenLittleWindow {
        url: String,
    },
    /// Move a little window (or a Peek) into the main window, optionally into a named space.
    MoveToMainWindow {
        #[serde(default)]
        space: Option<String>,
    },
    /// Preview a link in a floating panel over the page (like Arc's Peek).
    Peek {
        url: String,
    },
    ClosePeek,
    OpenConfig,
    ReloadConfig,
    Sync,
    Import {
        source: String,
    },
    /// Run a headless agent: a configured job by name, or an ad-hoc prompt.
    RunAgent {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        prompt: Option<String>,
        #[serde(default)]
        agent: Option<String>,
    },
    ShowAgentRun {
        index: usize,
    },
    StopAgent {
        name: String,
    },
    ToggleSiteBlocking,
    /// Make a pinned / kept / favorite tab's home the page it's on now.
    SetPinnedUrl {
        #[serde(default)]
        tab: Option<TabId>,
    },
    /// Go back to a pinned / kept / favorite tab's home page.
    ResetPinnedUrl {
        #[serde(default)]
        tab: Option<TabId>,
    },
    /// Ask for a line of text in the command bar, then run `then` with the text in its `field`.
    Prompt {
        label: String,
        #[serde(default)]
        value: String,
        then: serde_json::Value,
        field: String,
    },
    /// Clear cookies, caches and storage for the current site (signs you out of it).
    ClearSiteData,
    SetDefaultBrowser,
    /// Run a command defined by a plugin in `<config dir>/plugins/<plugin>/plugin.yaml`.
    Plugin {
        plugin: String,
        command: String,
    },
    /// Ask the user's AI assistant to change Valerie itself (edits config and plugins).
    Customize {
        request: String,
    },
    /// Copy a saved password to the clipboard (cleared after 45s). Never available to pages or agents.
    CopyPassword {
        vault: String,
        origin: String,
        username: String,
    },
    DeletePassword {
        vault: String,
        origin: String,
        username: String,
    },
    FindInPage {
        #[serde(default)]
        query: Option<String>,
    },
    Quit,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BarMode {
    #[default]
    NewTab,
    /// Edit the current tab's URL in place.
    Current,
}

impl Command {
    /// Build a command from a name with no args, e.g. "new_tab".
    pub fn from_name(name: &str) -> Option<Command> {
        serde_json::from_value(serde_json::json!({ "cmd": name })).ok()
    }

    /// Commands that expose or destroy secrets; agents (MCP) may not run them.
    pub fn agent_forbidden(&self) -> bool {
        matches!(self, Command::CopyPassword { .. } | Command::DeletePassword { .. })
    }

    /// Commands a web page may trigger through injected vim motions. None of these can read or redirect other tabs.
    pub fn page_safe(&self) -> bool {
        use Command::*;
        matches!(
            self,
            OpenCommandBar { .. }
                | NewTab { url: None, space: None, .. }
                | CloseTab { tab: None }
                | ReopenClosedTab
                | NextTab
                | PrevTab
                | GoToTab { .. }
                | Back { tab: None }
                | Forward { tab: None }
                | Reload { tab: None }
                | CopyUrl { tab: None }
                | ToggleSidebar
                | NextSpace
                | PrevSpace
                | FocusNextPane
                | FocusPane { .. }
                | ClosePeek
                | ZoomIn
                | ZoomOut
                | ZoomReset
        )
    }
}

/// Human titles for no-argument commands, used by the command bar and the native menu.
pub const CATALOG: &[(&str, &str)] = &[
    ("new_tab", "New Tab"),
    ("close_tab", "Close Tab"),
    ("reopen_closed_tab", "Reopen Closed Tab"),
    ("duplicate_tab", "Duplicate Tab"),
    ("toggle_pin", "Pin / Unpin Tab"),
    ("toggle_keep", "Keep / Unkeep Tab"),
    ("cycle_sidebar_size", "Sidebar Size: Full / Medium / Compact"),
    ("toggle_favorite", "Add / Remove Favorite"),
    ("new_folder", "New Folder"),
    ("clear_today_tabs", "Clear Today Tabs"),
    ("next_tab", "Next Tab"),
    ("prev_tab", "Previous Tab"),
    ("copy_url", "Copy URL"),
    ("reload", "Reload Page"),
    ("back", "Back"),
    ("forward", "Forward"),
    ("toggle_sidebar", "Toggle Sidebar"),
    ("toggle_tab_bar_position", "Toggle Sidebar / Top Bar"),
    ("next_space", "Next Space"),
    ("prev_space", "Previous Space"),
    ("new_space", "New Space"),
    ("delete_space", "Delete Space"),
    ("split_view", "Split View"),
    ("unsplit", "Close Split View"),
    ("focus_next_pane", "Focus Next Split Pane"),
    ("zoom_in", "Zoom In"),
    ("zoom_out", "Zoom Out"),
    ("zoom_reset", "Actual Size"),
    ("dev_tools", "Developer Tools"),
    ("move_to_main_window", "Move Little Window to Main Window"),
    ("find_in_page", "Find in Page"),
    ("toggle_site_blocking", "Toggle Ad Blocking for This Site"),
    ("clear_site_data", "Clear Site Data for This Site (signs you out of it)"),
    ("set_default_browser", "Make Valerie the Default Browser"),
    ("open_config", "Open Config File"),
    ("reload_config", "Reload Config"),
    ("sync", "Sync Now"),
    ("quit", "Quit Valerie"),
];

/// Commands that take options, with their shapes (shared by the MCP tool description and the LLM guide).
pub const WITH_ARGS: &[(&str, &str)] = &[
    ("open_command_bar", "{ mode: new_tab | current }"),
    ("new_tab", "{ url?, space?, background? }"),
    ("go", "{ input, new_tab? }  (URL or search terms, like typing in the command bar)"),
    ("navigate", "{ url, tab? }"),
    ("close_tab", "{ tab? }"),
    ("activate_tab", "{ tab }"),
    ("go_to_tab", "{ index }  (0-based, favorites first)"),
    ("toggle_pin", "{ tab? }"),
    ("toggle_favorite", "{ tab? }"),
    ("move_tab", "{ tab, section?: pinned | kept | today, space?, folder?, index?, favorite? }"),
    ("toggle_keep", "{ tab? }"),
    ("copy_url", "{ tab? }"),
    ("set_pinned_url", "{ tab? }  (a pinned/kept/favorite tab's home becomes its current page)"),
    ("reset_pinned_url", "{ tab? }  (go back to the tab's pinned page)"),
    ("rename_tab", "{ tab, title }"),
    ("new_folder", "{ name?, section?: kept | pinned }"),
    ("switch_space", "{ index? | name? }"),
    ("new_space", "{ name? }"),
    ("rename_space", "{ name }"),
    ("delete_space", "{ name? }"),
    ("set_space_theme", "{ theme }"),
    ("split_view", "{ tab? }"),
    ("focus_pane", "{ direction: left | right }  (outside split view: back / forward)"),
    ("open_little_window", "{ url }"),
    ("move_to_main_window", "{ space? }"),
    ("peek", "{ url }"),
    ("import", "{ source: arc | chrome | brave | edge | vivaldi | chromium | safari | firefox }"),
    ("find_in_page", "{ query? }"),
    ("run_agent", "{ name? | prompt?, agent? }"),
    ("customize", "{ request }  (have the user's AI assistant change Valerie's settings or plugins)"),
    ("plugin", "{ plugin, command }  (run a plugin command by its title)"),
];

pub fn title_for(name: &str) -> String {
    CATALOG
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, t)| t.to_string())
        .unwrap_or_else(|| name.replace('_', " "))
}
