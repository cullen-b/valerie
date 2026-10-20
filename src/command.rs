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
    /// Move tabs and folders together (ids of either), in order: like move_tab, for a group dragged in the sidebar.
    MoveItems {
        ids: Vec<TabId>,
        #[serde(default)]
        section: Option<String>,
        #[serde(default)]
        favorite: bool,
        #[serde(default)]
        space: Option<String>,
        #[serde(default)]
        folder: Option<TabId>,
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
    /// Remove these tabs from the sidebar for good (pinned ones too) and these folders with everything in them.
    DeleteItems {
        #[serde(default)]
        tabs: Vec<TabId>,
        #[serde(default)]
        folders: Vec<TabId>,
        /// Leave the folders' tabs in the sidebar instead.
        #[serde(default)]
        keep_tabs: bool,
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
        /// The space to rename (default: the one you're in).
        #[serde(default)]
        space: Option<String>,
    },
    /// Remove a space; its tabs are archived (still searchable). Spaces from config.yaml are taken out of it too.
    DeleteSpace {
        #[serde(default)]
        name: Option<String>,
        /// Ask first ("Remove the space …?").
        #[serde(default)]
        confirm: bool,
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
    /// An empty little window asking where to go (bind it under global_keybindings to open one from any app).
    NewLittleWindow,
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
    /// Switch back to the tab you were on before this one (in this space).
    LastTab,
    /// Mute or unmute everything a tab plays.
    ToggleMute {
        #[serde(default)]
        tab: Option<TabId>,
    },
    /// Play or pause a tab's video/audio (the one it played last).
    ToggleMedia {
        #[serde(default)]
        tab: Option<TabId>,
    },
    /// Show the page's article as clean, readable text (again to go back).
    ReaderView,
    /// Pick an element on the page (hover, ↑ for its parent, click) to hide it on this site for good.
    HideElement,
    /// Bring back everything hidden with hide_element on this site.
    ShowHiddenElements,
    /// Open the page in the translation service from `translate`.
    TranslatePage,
    /// Copy "[title](url)" for the current page.
    CopyMarkdownLink,
    /// Copy a link that scrolls to and highlights the selected text.
    CopyLinkToHighlight,
    /// Search the web for the selected text in a new tab.
    SearchSelection,
    /// Open the URL (or search for the text) on the clipboard in a new tab.
    PasteAndGo,
    /// Open the print dialog for the current page.
    Print,
    /// The Chats space: conversations with your agents (left of the first space).
    OpenChats,
    /// Show or hide the terminal panel (in a coding chat it opens on that chat's project).
    ToggleTerminal,
    /// A new shell in the terminal panel: in `folder`, else the open coding chat's project, else terminal_panel.folder.
    NewTerminal {
        #[serde(default)]
        folder: Option<String>,
    },
    /// The terminal panel on a coding chat's project (its shell is started if it has none).
    OpenTerminal {
        #[serde(default)]
        chat: Option<String>,
    },
    /// Start a chat, optionally asking something right away.
    NewChat {
        #[serde(default)]
        prompt: Option<String>,
        #[serde(default)]
        agent: Option<String>,
        /// A project folder for the agent to work in.
        #[serde(default)]
        folder: Option<String>,
        /// In Chats, start it in the open chat's project folder (when it has one and `folder` isn't given).
        #[serde(default)]
        in_project: bool,
    },
    /// The project folder a chat's agent works in ("" = none), before its first message.
    SetChatFolder {
        #[serde(default)]
        chat: Option<String>,
        folder: String,
    },
    /// Pick a chat's project folder in a folder window.
    PickChatFolder {
        #[serde(default)]
        chat: Option<String>,
    },
    /// Answer an Ask-first chat's question about an action.
    AnswerChatAsk {
        id: String,
        allow: bool,
        #[serde(default)]
        always: bool,
    },
    /// Look again at what's changed in a chat's project folder.
    ChatChanges {
        #[serde(default)]
        chat: Option<String>,
    },
    /// Show the changes in a chat's project folder (one file, or all).
    ChatDiff {
        #[serde(default)]
        chat: Option<String>,
        #[serde(default)]
        path: Option<String>,
    },
    /// Open a file from a coding chat's project folder in your editor (the `editor` setting).
    OpenChatFile {
        #[serde(default)]
        chat: Option<String>,
        path: String,
    },
    /// Bring a closed chat back from the Closed list.
    ReopenChat {
        chat: String,
    },
    /// Ask the last question in a chat again.
    RetryChat {
        #[serde(default)]
        chat: Option<String>,
    },
    /// Send a message in a chat (the open one if none is given; a new one if none is open).
    SendChat {
        #[serde(default)]
        chat: Option<String>,
        text: String,
        #[serde(default)]
        agent: Option<String>,
        /// Options picked in the composer before the chat existed.
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        effort: Option<String>,
        #[serde(default)]
        access: Option<String>,
        #[serde(default)]
        plan: Option<bool>,
        /// Files pasted into the composer (saved by Valerie first).
        #[serde(default)]
        files: Vec<String>,
    },
    StopChat {
        #[serde(default)]
        chat: Option<String>,
    },
    CloseChat {
        #[serde(default)]
        chat: Option<String>,
    },
    TogglePinChat {
        #[serde(default)]
        chat: Option<String>,
    },
    ActivateChat {
        chat: String,
    },
    RenameChat {
        chat: String,
        title: String,
    },
    /// Which agent a chat talks to from now on.
    /// Change a chat's agent, model, reasoning effort, access ("read" | "ask" | "full") or plan mode.
    #[serde(alias = "set_chat_agent")]
    SetChatOptions {
        chat: String,
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        effort: Option<String>,
        #[serde(default)]
        access: Option<String>,
        #[serde(default)]
        plan: Option<bool>,
    },
    /// Settle a chat (set it aside) or bring it back.
    ToggleSettleChat {
        #[serde(default)]
        chat: Option<String>,
    },
    /// Snooze a chat: "hour" | "three_hours" | "evening" | "tomorrow" | "next_week" | "wake".
    SnoozeChat {
        #[serde(default)]
        chat: Option<String>,
        until: String,
    },
    MarkChatUnread {
        chat: String,
    },
    /// Copy chat:<id>; pasted into another chat it brings this chat's conversation along.
    CopyChatId {
        #[serde(default)]
        chat: Option<String>,
    },
    /// Create a task: what an agent should do and when, in plain words (asked for when missing).
    #[serde(alias = "new_brief")]
    NewTask {
        #[serde(default)]
        want: Option<String>,
        #[serde(default, alias = "at")]
        when: Option<String>,
    },
    /// Run a task now instead of waiting for its time.
    #[serde(alias = "run_brief")]
    RunTask {
        name: String,
    },
    /// Show a task's result (its page or chat).
    #[serde(alias = "open_brief")]
    OpenTask {
        name: String,
    },
    /// Run one of the `ai_actions` from the config.
    AiAction {
        name: String,
        #[serde(default)]
        input: Option<String>,
    },
    /// Have an agent check this page in the background until something happens.
    WatchPage {
        #[serde(default)]
        condition: Option<String>,
    },
    StopWatching {
        id: String,
    },
    /// Quit and open again (to use a newly installed version).
    Restart,
    /// Look for a newer Valerie now, and install it (or open its download page where Valerie can't update itself).
    CheckForUpdates,
    /// The welcome guide: AI, sync, describing your browser in your own words, and a tour of what Valerie does.
    Welcome,
    /// The plugin marketplace: browse, install and remove plugins people shared, and share yours.
    PluginMarketplace,
    /// Share one of your plugins in the marketplace (opens a pull request from your GitHub account).
    SharePlugin {
        #[serde(default)]
        name: Option<String>,
    },
    /// scripts/install-macos.sh says a new version is in /Applications.
    UpdateInstalled,
    /// Allow or stop notifications from the site you're on.
    ToggleSiteNotifications,
    /// Install (if needed) and sign in to an agent CLI in one step.
    SetUpAgent {
        agent: String,
    },
    /// Close Today tabs showing the same page as another tab in this space.
    CloseDuplicateTabs,
    /// Switch this computer between test builds (every change as soon as it's built) and stable releases.
    ToggleTestBuilds,
    /// Ask about pinned, kept and favorite tabs you haven't opened in `tabs.ask_to_keep_unused_days`.
    ReviewSavedTabs,
    /// Turn Valerie's dark mode for light websites on or off for this site.
    ToggleDarkPage,
    /// Sign in to GitHub and sync settings and tabs through a private repo (created if needed).
    SetUpSync,
    /// Choose the sync passphrase (first machine) or type it (every other one); tabs, chats and passwords sync
    /// encrypted with it.
    SyncPassphrase,
    /// Ask about plugins and AI programs that arrived from another machine and are waiting to be allowed here.
    AllowFromOtherMachines,
    /// Copy text to the clipboard (the command bar's calculator uses this).
    CopyText {
        text: String,
    },
    /// Screenshot the visible page: copied to the clipboard and/or saved to Downloads (see `capture` settings).
    CapturePage,
    /// The whole page, top to bottom, as a picture (clipboard and Downloads, like capture_page).
    CaptureFullPage,
    /// Put the `valerie` command on PATH so a terminal can use it.
    InstallCommandLineTool,
    /// Open a file from the downloads list (the command bar shows them). Never available to pages or agents.
    OpenDownload {
        index: usize,
    },
    ShowDownloads,
    /// The Downloads page: every file you've downloaded (valerie://page/downloads).
    Downloads,
    /// The History page: every page you've visited (valerie://page/history).
    History,
    /// A page from the config folder's pages/ (or a built-in one) at valerie://page/<name>.
    OpenPage {
        name: String,
    },
    /// Clear cookies, caches and storage for the current site (signs you out of it).
    ClearSiteData,
    /// Open this page in Safari, where passkeys work, then offer to bring the login back.
    SignInWithSafari,
    /// Copy this site's login (its cookies) from Safari into this tab's profile, then reload.
    BringLoginFromSafari,
    SetDefaultBrowser,
    /// Make Valerie the app Markdown files (.md) open in (macOS).
    SetDefaultMarkdownApp,
    /// Run a command defined by a plugin in `<config dir>/plugins/<plugin>/plugin.yaml`.
    Plugin {
        plugin: String,
        command: String,
        /// What was right-clicked (link, selection, image, page): passed to the command as VALERIE_LINK and so on.
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        context: std::collections::BTreeMap<String, String>,
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
    /// Open a link the way Valerie's link menu does: new_tab | background_tab | split | peek | little_window.
    OpenLink {
        url: String,
        #[serde(default)]
        mode: String,
    },
    /// A key of a key sequence ("CmdOrCtrl+K CmdOrCtrl+S") was pressed; Valerie waits for the rest.
    KeySequence {
        first: String,
        #[serde(default)]
        next: Option<String>,
    },
    /// A window of its own for a page (or the command bar's new tab page when there's none).
    NewWindow {
        #[serde(default)]
        url: Option<String>,
    },
    /// Put Valerie's settings, plugins and styles back the way they were before the last change an AI made.
    UndoCustomization,
    /// A chat where the user's AI helps find out what went wrong (the triage playbook).
    ReportProblem,
    /// The browser extensions Valerie loaded (and their buttons).
    Extensions,
    /// Open an extension's popup or options page.
    ExtensionAction {
        name: String,
        #[serde(default)]
        options: bool,
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

    /// Commands that expose or destroy secrets or open local files; agents (MCP) may not run them.
    pub fn agent_forbidden(&self) -> bool {
        matches!(
            self,
            Command::CopyPassword { .. }
                | Command::DeletePassword { .. }
                | Command::OpenDownload { .. }
                | Command::BringLoginFromSafari
                // An agent starting agents could loop forever; setting one up installs software.
                | Command::NewChat { .. }
                | Command::SendChat { .. }
                | Command::RetryChat { .. }
                // Only the person answers a chat's questions and picks the folder its agent works in.
                | Command::AnswerChatAsk { .. }
                | Command::SetChatFolder { .. }
                | Command::PickChatFolder { .. }
                | Command::OpenChatFile { .. }
                | Command::AiAction { .. }
                | Command::WatchPage { .. }
                | Command::NewTask { .. }
                | Command::RunTask { .. }
                | Command::SetUpAgent { .. }
                | Command::CheckForUpdates
                | Command::Welcome
                | Command::PluginMarketplace
                | Command::SharePlugin { .. }
                // Only the person at the computer lets in what another machine sent, or handles the passphrase.
                | Command::AllowFromOtherMachines
                // It asks the person at the computer which tabs to remove.
                | Command::ReviewSavedTabs
                // Which builds this computer runs is the person's choice.
                | Command::ToggleTestBuilds
                | Command::SyncPassphrase
        )
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
                | LastTab
                | ToggleMute { tab: None }
                | ToggleMedia { tab: None }
                | ReaderView
                | HideElement
                | CopyMarkdownLink
                | CopyLinkToHighlight
                | SearchSelection
                | ToggleDarkPage
                | TranslatePage
                | Print
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
    ("delete_space", "Remove Space (archives its tabs)"),
    ("split_view", "Split View"),
    ("unsplit", "Close Split View"),
    ("focus_next_pane", "Focus Next Split Pane"),
    ("zoom_in", "Zoom In"),
    ("zoom_out", "Zoom Out"),
    ("zoom_reset", "Actual Size"),
    ("dev_tools", "Developer Tools"),
    ("new_little_window", "New Little Window"),
    ("move_to_main_window", "Move Little Window to Main Window"),
    ("find_in_page", "Find in Page"),
    ("last_tab", "Switch to Last Tab"),
    ("toggle_mute", "Mute / Unmute Tab"),
    ("toggle_media", "Play / Pause Media"),
    ("reader_view", "Reader View"),
    ("hide_element", "Hide an Element on This Site…"),
    ("show_hidden_elements", "Show Hidden Elements on This Site"),
    ("translate_page", "Translate Page"),
    ("copy_markdown_link", "Copy Link as Markdown"),
    ("copy_link_to_highlight", "Copy Link to Highlighted Text"),
    ("search_selection", "Search Selected Text"),
    ("paste_and_go", "Paste and Go"),
    ("close_duplicate_tabs", "Close Duplicate Tabs"),
    ("toggle_test_builds", "Toggle Test Builds (get every change as soon as it's made, or go back to stable releases)"),
    ("review_saved_tabs", "Review Saved Tabs (pinned and kept ones you haven't opened in a while)"),
    ("print", "Print Page…"),
    ("open_chats", "Chats"),
    ("toggle_terminal", "Terminal (show or hide the terminal panel)"),
    ("new_terminal", "New Terminal"),
    ("new_chat", "New Chat"),
    ("stop_chat", "Stop the Agent in This Chat"),
    ("close_chat", "Close Chat"),
    ("toggle_pin_chat", "Pin / Unpin Chat"),
    ("toggle_settle_chat", "Settle / Un-settle Chat"),
    ("copy_chat_id", "Copy Chat ID (share its context with another chat)"),
    ("watch_page", "Watch This Page… (an agent tells you when something changes)"),
    ("restart", "Restart Valerie (use a newly installed version)"),
    ("check_for_updates", "Check for Updates"),
    ("welcome", "Welcome Tour (set up AI and sync, make Valerie yours, see what it does)"),
    ("plugin_marketplace", "Plugins (marketplace: browse, install and share plugins)"),
    ("share_plugin", "Share Plugin (put one of yours in the marketplace)"),
    ("toggle_site_notifications", "Notifications from This Site: Allow / Block"),
    ("new_task", "Create Task… (an agent does something for you: a morning brief, joining meetings, opening pages…)"),
    ("toggle_dark_page", "Dark Mode for This Site"),
    ("capture_page", "Capture Page (screenshot)"),
    ("capture_full_page", "Capture Full Page (screenshot of the whole page)"),
    ("install_command_line_tool", "Install Command Line Tool (valerie in your terminal)"),
    ("downloads", "Downloads (every file you've downloaded: open, show in folder, clear)"),
    ("history", "History (every page you've visited, by day; search and forget)"),
    ("show_downloads", "Show Downloads Folder"),
    ("toggle_site_blocking", "Toggle Ad Blocking for This Site"),
    ("clear_site_data", "Clear Site Data for This Site (signs you out of it)"),
    ("sign_in_with_safari", "Sign In with Safari (for passkeys)"),
    ("bring_login_from_safari", "Bring Login from Safari"),
    ("set_default_browser", "Make Valerie the Default Browser"),
    ("set_default_markdown_app", "Open Markdown Files in Valerie"),
    ("open_config", "Open Config File"),
    ("reload_config", "Reload Config"),
    ("sync", "Sync Now"),
    ("set_up_sync", "Set Up Sync (sign in to GitHub)"),
    ("sync_passphrase", "Sync Passphrase (set, enter or change the one that encrypts your synced tabs, chats and passwords)"),
    ("allow_from_other_machines", "Allow Plugins from Other Machines"),
    ("new_window", "New Window"),
    ("report_problem", "Report a Problem… (your AI helps find what went wrong, fixes it or writes the bug report)"),
    ("undo_customization", "Undo Last Customization (put settings back as they were before the AI changed them)"),
    ("extensions", "Extensions"),
    ("quit", "Quit Valerie"),
];

/// Commands that take options, with their shapes (shared by the MCP tool description and the LLM guide).
pub const WITH_ARGS: &[(&str, &str)] = &[
    ("share_plugin", "{ name? }  (a folder in plugins/; without one, the marketplace opens on your plugins)"),
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
    ("move_items", "{ ids: [tab or folder], section?: pinned | kept | today, space?, folder?, index?, favorite? }  (moves them together, in order; folders bring their tabs)"),
    ("toggle_keep", "{ tab? }"),
    ("copy_url", "{ tab? }"),
    ("set_pinned_url", "{ tab? }  (a pinned/kept/favorite tab's home becomes its current page)"),
    ("reset_pinned_url", "{ tab? }  (go back to the tab's pinned page)"),
    ("rename_tab", "{ tab, title }"),
    ("new_folder", "{ name?, section?: kept | pinned }"),
    ("delete_items", "{ tabs?: [tab], folders?: [folder], keep_tabs? }  (closes the tabs for good, pinned ones too; folders go with their tabs unless keep_tabs)"),
    ("switch_space", "{ index? | name? }"),
    ("new_space", "{ name? }"),
    ("rename_space", "{ name, space? }"),
    ("delete_space", "{ name?, confirm? }  (its tabs are archived)"),
    ("set_space_theme", "{ theme }"),
    ("split_view", "{ tab? }"),
    ("focus_pane", "{ direction: left | right }  (outside split view: back / forward)"),
    ("open_little_window", "{ url }"),
    ("move_to_main_window", "{ space? }"),
    ("peek", "{ url }"),
    ("import", "{ source: arc | chrome | brave | edge | vivaldi | chromium | safari | firefox }"),
    ("find_in_page", "{ query? }"),
    ("toggle_mute", "{ tab? }"),
    ("copy_text", "{ text }"),
    ("new_terminal", "{ folder? }  (a shell in the terminal panel; default: the open coding chat's project)"),
    ("open_terminal", "{ chat? }  (the terminal panel on a coding chat's project)"),
    ("new_chat", "{ prompt?, agent?, folder?, in_project? }  (ask an agent something in a new chat; it can see and use the browser)"),
    ("send_chat", "{ text, chat?, agent? }"),
    ("activate_chat", "{ chat }"),
    ("close_chat", "{ chat? }  (it moves to the Closed list)"),
    ("reopen_chat", "{ chat }  (from the Closed list)"),
    ("rename_chat", "{ chat, title }"),
    ("set_chat_options", "{ chat, agent?, model?, effort?: low|medium|high|max, access?: read|ask|full, plan? }"),
    ("toggle_settle_chat", "{ chat? }"),
    ("snooze_chat", "{ chat?, until: hour | three_hours | evening | tomorrow | next_week | wake }"),
    ("copy_chat_id", "{ chat? }"),
    ("ai_action", "{ name, input? }  (run one of the ai_actions from the config)"),
    ("watch_page", "{ condition? }"),
    ("new_task", "{ want?, when? }  (what to do, and when in plain words: \"every day at 8am\", \"every 5 minutes\", \"when Valerie starts\")"),
    ("run_task", "{ name }"),
    ("open_task", "{ name }"),
    ("stop_watching", "{ id }"),
    ("set_up_agent", "{ agent: claude | codex | opencode | pi | cursor | hermes }  (install if needed, then sign in)"),
    ("toggle_media", "{ tab? }  (play / pause)"),
    ("run_agent", "{ name? | prompt?, agent? }"),
    ("customize", "{ request }  (have the user's AI assistant change Valerie's settings or plugins)"),
    ("new_window", "{ url? }  (a separate window for one page)"),
    ("extension_action", "{ name, options? }  (open a browser extension's popup, or its options page)"),
    ("open_page", "{ name }  (a page from the config folder's pages/, e.g. one an agent made with show_page; or downloads, history)"),
    ("plugin", "{ plugin, command, context? }  (run a plugin command by its title; context: { link?, selection?, image?, page? })"),
];

pub fn title_for(name: &str) -> String {
    CATALOG
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, t)| t.to_string())
        .unwrap_or_else(|| name.replace('_', " "))
}
