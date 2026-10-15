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
    },
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
    CopyUrl,
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
    ZoomIn,
    ZoomOut,
    ZoomReset,
    DevTools,
    OpenLittleWindow {
        url: String,
    },
    MoveToMainWindow,
    OpenConfig,
    ReloadConfig,
    Sync,
    Import {
        source: String,
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
                | CopyUrl
                | ToggleSidebar
                | NextSpace
                | PrevSpace
                | FocusNextPane
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
    ("open_config", "Open Config File"),
    ("reload_config", "Reload Config"),
    ("sync", "Sync Now"),
    ("quit", "Quit Valerie"),
];

pub fn title_for(name: &str) -> String {
    CATALOG
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, t)| t.to_string())
        .unwrap_or_else(|| name.replace('_', " "))
}
