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

### Phase 1: working prototype (tonight)
- [x] Repo, plan, engine choice
- [ ] Window + sidebar + content webviews, Arc-style transparent titlebar, edge-to-edge content (configurable bezel)
- [ ] Spaces with per-space themes (gradient + accent), switching with Ctrl+1..9 / swipe of sidebar
- [ ] Profiles: isolated data stores, spaces map to profiles
- [ ] Tabs: pinned (with folders), today tabs, favorites row, auto-archive after N hours, archive search
- [ ] Command bar centred on screen: Cmd+T / Cmd+D / Cmd+L, fuzzy search over tabs, history, bookmarks, commands,
      synced tabs; URL vs search detection
- [ ] Cmd+Shift+C copies the URL (with toast)
- [ ] Split view (up to 4 panes), focus panes
- [ ] Little windows: external links open in a small "baby" window; one click moves them into the main window
- [ ] Air Traffic Control rules
- [ ] Vim motions + link hints in pages
- [ ] Built-in MCP server: tabs, navigation, page snapshot, click/type, JS eval, plus a generic `run_command`
- [ ] YAML config with hot reload; keybindings remappable
- [ ] GitHub sync (per-device files in a private repo)
- [ ] Import from Arc / Chrome-family / Safari / Firefox
- [ ] Sidebar or top tab bar (`layout.tab_bar: side|top`)
- [ ] `.app` bundle script registering as an http/https handler (needed to be the default browser)

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
