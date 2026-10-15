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

### Next up (Phase 1.5)
- Agent `screenshot` tool (WKWebView `takeSnapshot`), agent action log + approval prompt for sensitive actions
- Peek previews, hover-to-reveal sidebar when hidden, Boosts (per-site CSS/JS in the config dir)
- Per-profile WebContext data dirs on Windows/Linux; CI builds for all three OSes
- App icon, signing/notarisation, auto-update

### Phase 2: daily driver
- Password manager: per-profile vaults in the OS keychain, shared vault groups across profiles, autofill script,
  import from Chrome/Arc CSV
- Content blocking (WKContentRuleList, EasyList-compiled rules) and per-site settings
- Find in page, downloads panel, history page, permissions UI, print
- Peek (preview a link in an overlay), Boosts (per-site CSS/JS from the config dir)
- Drag-and-drop everywhere, tab previews on hover, media controls in sidebar
- Session restore across crashes, closed-tab stack per space
- Smarter sync: pinned tabs/spaces merge (CRDT-ish, last-writer-wins per item), config sync

### Phase 3: everywhere
- Windows/Linux polish (WebView2 profiles, WebKitGTK data dirs), CI release builds for all three
- Optional CEF backend for Chrome extensions
- Mobile: PWA that reads the sync repo through the GitHub API (tabs, pinned, send-to-device)
- Agent features: per-agent sandboxed spaces, action approval prompts, recorded action log

## Defaults (my preferred set)
Sidebar on the left, edge-to-edge content, dark "midnight" theme, Cmd+T/Cmd+D command bar, Cmd+S toggles
sidebar, Cmd+Shift+C copies the URL, today tabs archive after 12 hours, vim motions on, MCP on at port 7777.
