# Valerie — the plan

Valerie is a homemade browser: Rust-native, configured with plain text files, synced through GitHub, and
fully controllable by AI agents over MCP. It keeps Arc's best ideas and drops the weight.

## Tenets

1. **Rust.** The whole browser shell is Rust. Web content comes from an engine behind a trait.
2. **Configurable, with strong defaults.** One `config.yaml` drives keybindings, spaces, profiles, themes, routing
   rules and layout. It hot-reloads and can live in a git repo.
3. **Sync through GitHub, no server.** Tabs and spaces sync through a private git repo you own.
4. **Agent-native.** Every user action is a `Command`. Keybindings, the sidebar, the command bar, vim motions and
   the built-in MCP server all dispatch the same commands, so an agent can do anything you can.
5. **Vim motions everywhere.** `j/k`, `gg/G`, `f` link hints, `H/L` history, `J/K` tabs, `x` close, `o` open.

## Engine decision

| Option | Perf | Extensions | Rust fit | Cross-platform | Verdict |
|---|---|---|---|---|---|
| **System webview (wry/tao)**: WKWebView / WebView2 / WebKitGTK | Best on macOS (it *is* Safari's engine), low memory | No Chrome extensions | Excellent (Tauri's crates) | Yes | **Phase 1 engine** |
| Chromium via CEF (`cef` crate) | Good, heavier (≈200 MB runtime) | Partial (CEF has limited MV3 support) | OK (C API bindings) | Yes | Phase 3 optional backend |
| Servo (`servo` crate, v0.1 in Apr 2026) | Promising, not daily-driver ready | None | Native Rust | Partial | Watch it; add as a backend when mature |

We start on **wry**, which gives "Safari-like performance" on macOS on day one. The engine sits behind a small
`Engine` trait (create view, navigate, evaluate JS, set bounds, events), so a CEF backend can come later for people
who need Chrome extensions. The things people use extensions for most (ad blocking, vim motions, password
autofill, dark mode) become **built-in features** instead: content blockers, injected scripts and the MCP layer.

## Architecture

```
┌───────────────────────── tao window (transparent titlebar, full-size content) ─────────────────────────┐
│ chrome webview (whole window, behind): space gradient, sidebar/top bar UI, new-tab page                 │
│   ┌──────────────── content webviews (one per live tab, lazily created; several when split) ──────────┐ │
│   │                                                                                                     │ │
│   └─────────────────────────────────────────────────────────────────────────────────────────────────────┘ │
│ overlay webview (transparent, top-most): command bar, centred like Arc                                  │
└─────────────────────────────────────────────────────────────────────────────────────────────────────────┘
          ▲ ipc (JSON commands)                 ▲ menu accelerators (muda)            ▲ MCP over HTTP
          └──────────────── App::dispatch(Command) ◄── one action vocabulary ──────────┘
```

* `src/command.rs` holds the action vocabulary (serde-tagged enum). Everything goes through `App::dispatch`.
* `src/state.rs` holds spaces, pinned tree (folders), today tabs, favorites, archive, history. Saved to JSON.
* `src/config.rs` holds YAML config, defaults, theme presets, hot reload.
* `src/app.rs` owns the windows, webviews, layout, little windows and the event loop glue.
* `src/mcp.rs` is an MCP Streamable-HTTP server on `127.0.0.1` with a bearer token. It also has a stdio bridge.
* `src/sync.rs` handles GitHub sync. Each device writes `devices/<name>.json` into a private repo, so merge conflicts
  can't happen. Other devices' tabs show up in the command bar.
* `src/routing.rs` handles Air Traffic Control: glob rules send URLs to a space.
* `src/import.rs` imports from Arc (`StorableSidebar.json`), Chrome/Brave/Edge/Vivaldi (`Bookmarks` JSON), Safari
  (`Bookmarks.plist`) and Firefox (`places.sqlite`).
* `ui/*.html|js|css` holds the chrome and command bar UI (vanilla JS, no build step) and `content.js` (vim motions,
  link hints, metadata reporting) injected into pages.

### Security notes
* MCP binds to `127.0.0.1` only, requires a random bearer token (stored mode 0600), and rejects non-local `Host`
  headers (DNS-rebinding) and browser `Origin`s.
* Page-injected IPC is untrusted. Pages can only send a small whitelist (metadata, vim navigation). They can never
  read other tabs or reach MCP-level commands.
* Profiles are isolated cookie/storage containers (WKWebsiteDataStore identifiers on macOS 14+, data dirs elsewhere).

## Feature roadmap

### Phase 1: working prototype ✅ (built 2026-10-15)
- [x] Repo, plan, engine choice
- [x] Window + sidebar + content webviews, transparent titlebar, edge-to-edge content (configurable bezel/radius)
- [x] Spaces with per-space themes (8 presets + custom), Ctrl+1..5, ⌘⌥←/→, two-finger swipe on the sidebar
- [x] Profiles: isolated WKWebsiteDataStores per profile (macOS); spaces map to profiles
- [x] Tabs: pinned (with folders), today tabs, favorites grid, auto-archive, idle unload, drag-and-drop, context menus
- [x] Command bar (⌘T/⌘D/⌘L): fuzzy search over tabs, history, bookmarks, commands, spaces, themes, imports, synced
      and archived tabs; URL vs search detection
- [x] ⌘⇧C copies the URL (with toast)
- [x] Split view (up to 4 panes), focus next pane
- [x] Little windows for external links, ⌘O moves them to the main window
- [x] Air Traffic Control rules
- [x] Vim motions, link hints, find bar in pages
- [x] Built-in MCP server (HTTP + stdio bridge): 17 tools including `run_command` for every browser command
- [x] YAML config with hot reload; keybindings and vim bindings remappable
- [x] GitHub sync (per-device files in a private repo) + `valerie sync-setup`
- [x] Import from Arc / Chrome-family / Safari / Firefox
- [x] Sidebar or top tab bar (`layout.tab_bar: side|top`), sidebar on left or right
- [x] `scripts/bundle-macos.sh` builds Valerie.app registered for http/https

**Verified:** unit tests (config merge, routing, input resolution, state ops); MCP end to end against the running app
(open, read, snapshot, type + submit, awaited eval, spaces, Arc import, auth rejection); CLI hand-off with ATC routing
and a little window; quit saving state; vim motions and hints on a real page; sidebar, light/dark themes and command
bar rendered with mock state.
**Not yet verified by eye:** the native window composition (the screen could not be captured overnight). Check the
command bar overlay sits above pages, traffic lights line up with the sidebar, and the rounded-corner option.

### Feedback round 1 ✅ (2026-10-15)
- [x] Safari user agent so Google/YouTube serve full pages (`user_agent` overrides it)
- [x] `u` goes back (half-page-up moved to `U`)
- [x] Drag and drop rebuilt on pointer events: pin, unpin, reorder, favorite, into folders, onto a space
- [x] Pinned tabs as Arc-style icon tiles (`layout.pinned_style: icons|list`)
- [x] Sidebar toggle animates the page natively instead of jumping; `layout.animation_ms`
- [x] Space switches: sidebar slides and follows the two-finger swipe, theme crossfades, page fades in
- [x] Command bar: no hint text, recent tabs when empty
- [x] Starter config on first run (only your overrides), `valerie default-config` prints every option

### Headless agents ✅ MVP
Agents run on the user's **own** Claude Code / Codex / OpenCode subscription: Valerie launches the CLI headless
(`claude -p`, `codex exec`, `opencode run`) and wires it to the browser through the `valerie mcp-stdio` bridge.
- [x] Ad hoc from the command bar: type `> task`, or pick "Ask agent" under any search
- [x] Scheduled jobs in config (`agents.jobs`: `every: 6h` and/or `at: "08:30"`), "Run agent: name" in the bar
- [x] Agents work in their own space (`agents.space`) and leave your tabs alone; a pulsing sidebar dot shows runs
- [x] Logs per run (`<data dir>/agents/*.log`), "Agent run: …" in the bar opens one; toast when done
- [x] Claude Code restricted to Valerie's tools (`--strict-mcp-config --allowedTools mcp__valerie`), Codex read-only sandbox
- [ ] Truly headless window (off-screen webviews) so agents never share the visible window's process budget
- [ ] Approval prompts for sensitive actions (purchases, sending messages, form submits on new domains)
- [x] Stop a running agent from the command bar ("Stop agent: …")
- [x] `screenshot` MCP tool (native WKWebView snapshot) so agents can see pages
- [ ] Per-agent action log in the sidebar, result notifications via macOS Notification Center

### Passkeys (blocked on Apple)
WebAuthn/passkeys only work in a WKWebView browser that holds Apple's
`com.apple.developer.web-browser.public-key-credential` entitlement, which needs a paid Apple Developer account
(Account Holder role), a request to Apple, and a provisioning-profile-signed build. Plan:
1. Enrol in the Apple Developer Program and request the entitlement (form: "macOS Web Browser Public Key Credential")
2. Sign `Valerie.app` with a Developer ID + the entitlement in `scripts/bundle-macos.sh`
3. Until then: passkey-only sites fall back to password/2FA; the Chromium (CEF) backend would also solve it

### Round 2 ✅ (2026-10-15)
- [x] Password manager: per-profile Keychain vaults, shared vaults (`passwords.shared`), save prompt after login,
      account picker on login fields (fills only after a real click; pages can't forge it), copy from the command
      bar (clipboard clears in 45s), `valerie import-passwords export.csv`. Agents can never read passwords.
- [x] Ad/tracker blocking: hosts lists compiled into WebKit content rules (cached), cosmetic hiding, per-site toggle
- [x] Boosts: `~/.config/valerie/boosts/<host>.css|.js`, CSS live-reloads
- [x] Peek: shift-click a link to preview it over the page; esc closes, ⌘O opens as a tab
- [x] Hover-to-reveal sidebar when hidden
- [x] Real sync of pinned tabs, folders, favorites and spaces (newest edit per space wins; stable pinned URLs;
      untouched devices never overwrite). Today tabs stay per-device and show up as "Synced" in the command bar.
- [x] App icon, CI on macOS/Linux/Windows, release workflow (tag `v*` → Valerie.app zip on GitHub Releases)
- [x] Per-profile data directories on Windows/Linux

### LLM-customisable browser ✅ (2026-10-15)
- [x] Generated guide (`AGENTS.md` + `CLAUDE.md`) in the config folder: settings, commands, plugin format, browser API
- [x] Plugins: `plugins/<name>/plugin.yaml` with commands (action macros, page JS, local scripts), shortcuts,
      `style.css` / `script.js` for matching sites, page-load hooks; hot-reloaded; Plugins menu + command bar
- [x] `valerie call <tool> '<json>'` so local code in any language can drive the browser
- [x] `valerie check-config` validates config and plugins with exact errors (LLMs use it to verify their edits)
- [x] "Customize Valerie" (`! request` in the command bar) runs the user's own Claude/Codex/OpenCode in the config
      folder to make the change; tested end to end with OpenCode
- [ ] Plugin UI surfaces: sidebar panels/widgets, toolbar buttons, settings pages
- [ ] Share plugins: install from a git URL, a community index
- [ ] Undo for customisations (snapshot config + plugins before each AI change)

### Round 3 ✅
- [x] Tab-to-search in the command bar (`site_search`, including ChatGPT, Claude and Perplexity)
- [x] Clear Site Data for This Site; Google Workspace on the default ad-blocking allow list (Sheets "Loading issue")
- [x] YouTube speed plugin uses YouTube's player API and never mutes; bundled plugins update when untouched
- [x] Signing pipeline: Apple Development / Developer ID detection, hardened runtime, notarization, passkey
      entitlement when a provisioning profile is supplied; "Make Valerie the Default Browser"

### Everything is a page ✅ (2026-10-18)
- [x] `valerie://page/<name>`: one HTML file in the config folder's `pages/`, with Valerie's colours and a `valerie`
      object (`call` browser tools, `log` into the log, `on` changes); looking tools are free, others are declared in a
      `valerie-tools` meta tag and approved once; a per-response nonce CSP keeps shown text from running code
- [x] Agents make them with `show_page`; they're listed in the command bar; undo covers `pages/`
- [x] Downloads and History are built-in pages (commands `downloads`, `history`), configurable under `downloads.page`
      and `history.page`, replaceable by a page of the same name
- [x] Logs for agents: `logs.level`, `read_logs`, plugin and page `valerie.log`, every agent tool call timed
- [x] `get_settings` / `set_setting` for any agent; "Report a Problem…" + `valerie triage`; "Triage my tabs"
- [ ] Show an agent's page inside the chat, not only as a tab

### Coding harness ✅ (2026-10-18)
Chats good enough to replace a separate coding-agent app (T3 Code, Conductor…), with the browser as the edge.
- [x] A project folder per chat (composer menu, `chats.projects`, native folder window); fixed once started (the CLI's
      session belongs to it); sidebar groups chats by project with a + for a new one there
- [x] Steps where they happened in the answer: edits as diffs, commands with output and exit status, durations,
      thinking, the agent's plan (also pinned above the composer while it works); finished runs fold into a summary
      (`chats.steps`); time and context under each answer (`chats.usage`); Claude Code and Codex in full, others by name
- [x] Ask-first questions inside the chat (with the command or the edit), "Always Allow" per chat, a notification when
      the chat isn't in view
- [x] Uncommitted changes card (git, never taking the lock) with a full diff view and buttons from `chats.change_actions`
- [x] Pasted/dropped images go to the agent; file paths in answers open in the editor; Try Again on errors
- [x] `read_console` so an agent can check the app it's building
- [ ] Run several turns side by side in one project without them editing the same files (worktrees, or a lock)
- [ ] Slash commands and @file mentions in the composer; a terminal pane

### Next up
- Developer ID signing + notarisation (needs an Apple Developer account; also unlocks the passkey entitlement)
- Updates: done. Every push to main is a release (website, Homebrew tap `cullen-b/homebrew-valerie`, GitHub
  Releases, `site/install.sh`) and installed copies update themselves. Still to do: a Developer ID ($99/year,
  individual account is enough) to notarize releases, so downloads and Homebrew open without "Open Anyway"; switching
  keys means one release whose `RELEASE_REQUIREMENTS` lists both.
- Passkeys on Mac: WebKit only does them for a browser Apple grants `com.apple.developer.web-browser.public-key-credential`
  (requested by the Account Holder of an organization developer account, at developer.apple.com/contact/request/
  macos-browsers-passkeys, with a downloadable release). Then embed the provisioning profile and add the entitlement in
  `bundle-macos.sh` only when the profile grants it. Until then `passwords.offer_passkeys: auto` hides passkeys from sites and
  "Sign In with Safari" brings a Safari login's cookies over (needs Full Disk Access).
- Agent approvals for sensitive actions, action log, Notification Center results
- Site permissions UI

### Phase 2: daily driver
- Password generator, TOTP codes, Keychain-backed vaults on Windows/Linux (Credential Manager / Secret Service)
- EasyList-syntax filters (cosmetic + network) beyond hosts lists, per-site settings page
- Downloads panel, history page, permissions UI, print
- Tab previews on hover, media controls in sidebar
- Config sync through the same repo

### Phase 3: everywhere
- Windows/Linux polish (WebView2 profiles, WebKitGTK data dirs), CI release builds for all three
- Optional CEF backend for Chrome extensions
- Mobile: PWA that reads the sync repo through the GitHub API (tabs, pinned, send-to-device)
- Agent features: per-agent sandboxed spaces, action approval prompts, recorded action log

## Defaults (my preferred set)
Sidebar on the left, edge-to-edge content, dark "midnight" theme, Cmd+T/Cmd+D command bar, Cmd+S toggles
sidebar, Cmd+Shift+C copies the URL, today tabs archive after 12 hours, vim motions on, MCP on at port 7777.
