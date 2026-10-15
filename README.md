# Valerie

A homemade browser: **Rust-native, configured with a text file, synced through GitHub, and drivable by AI agents.**
Arc's best ideas (spaces, pinned tabs and folders, the command bar, little windows, Air Traffic Control, split
view) on top of the system web engine, so on macOS you get Safari's engine and its speed and battery life.

See [PLAN.md](PLAN.md) for the architecture, the engine decision and the roadmap.

## Run it

```sh
cargo run --release              # or: cargo build --release && ./target/release/valerie
./scripts/bundle-macos.sh        # builds Valerie.app (needed to set it as your default browser)
```

On first launch Valerie writes a small `~/.config/valerie/config.yaml` (hot-reloaded) holding only your overrides;
`valerie default-config` prints every option with comments. Every piece of sidebar text and chrome has an on/off
switch there (`sidebar:`, `start_page`, `command_bar:`). `valerie paths` shows where config and data live.

## Keys (all remappable in `config.yaml`)

| Keys | Action |
|---|---|
| `⌘D` | Command bar: search tabs, history, bookmarks, commands, synced tabs; enter a URL or search |
| `⌘L` | Edit the current URL |
| `⌘⇧C` | Copy the current URL |
| `⌘S` | Toggle the sidebar |
| `⌘W` / `⌘⇧T` | Close tab (unloads pinned tabs) / reopen closed tab |
| `⌘⌥↓` `⌘⌥↑`, `⌃Tab` | Next / previous tab |
| `⌘⌥→` `⌘⌥←`, `⌃1…5`, two-finger swipe on sidebar | Switch space |
| `⌘1…8` | Jump to tab |
| `⌘⇧D` | Pin / unpin |
| `⌃⇧=` / `⌃⇧-` / `⌃⇧→` | Split view / close split / focus next pane |
| `⌘O` | Move a little window into the main window |
| `⌘F`, `⌘=` `⌘-` `⌘0`, `⌘⌥I` | Find, zoom, dev tools |
| `⌘,` | Open config |

**Tab to search a site:** type `youtube`, `claude`, `chatgpt`, `gh`, `wiki`… in the command bar, press Tab, and
type your search (Backspace on an empty box goes back). Add any site with one line under `site_search:` in your
config.

**Vim motions inside pages** (off while typing; `Esc` leaves a field): `j/k/h/l` scroll, `d/U` half page,
`gg/G` top/bottom (hold `j`/`k` to scroll faster and faster), `f`/`F` link hints (same/new tab), `u` back, `H`/`L` back/forward (in split view: focus the pane to the left/right), `J/K` or `gt/gT` next/prev tab,
`x`/`X` close/reopen, `o`/`O` command bar/edit URL, `t` new tab, `yy` copy URL, `/` find, `i` insert mode,
`s` toggle sidebar, `]s`/`[s` next/prev space.

Each space has **pinned** tabs (icon tiles), **Kept** tabs (a list under them that stays until you remove it), and
**today** tabs (archived after 12 hours unused). Folders work in pinned and Kept. Drag tabs anywhere in the sidebar:
onto the pinned tiles, into Kept or a folder, onto the favorites row, or onto a space icon. Drag the sidebar's edge to
resize it: narrow it to show icon + domain, narrower still for icons only (`layout.sidebar_size`, double-click the
edge to cycle). Hide the sidebar with `⌘S`, then touch the window edge to peek at it.

## Make Valerie yours with AI

Valerie is built to be changed by your AI assistant. Its config folder (`~/.config/valerie/`) holds a generated
`AGENTS.md` / `CLAUDE.md` that explains every setting, command, plugin format and browser API, so you can open
Claude Code, Codex or OpenCode there and say "make my browser do X". Or do it from inside the browser: type
`! hide the URL bar and make ⌃⇧G open GitHub` in the command bar (or pick "Customize Valerie" under any search).
Your assistant edits the config or writes a plugin, checks it with `valerie check-config`, and Valerie reloads it
instantly.

Valerie ships with a **YouTube speed** plugin: `+` / `-` change the speed (0.25 steps, up to 16×), `*` resets, Shift+scroll
over the video works too, a badge on the player shows the speed, and your speed carries over to the next video. Tune it
in `~/.config/valerie/plugins/youtube-speed/plugin.yaml`.

**Plugins** are folders in `~/.config/valerie/plugins/` with a readable `plugin.yaml` (see `plugins/_example/`):
command-bar commands with optional shortcuts that run built-in action macros, JavaScript in the page, or local
scripts in any language; `style.css` / `script.js` for matching sites; and hooks when pages load. Local code drives
the browser with `valerie call <tool> '<json>'` (for example `valerie call read_page '{}'`).

## Headless agents

Your own Claude Code, Codex or OpenCode subscription can drive the browser in the background:

- Type `> find me flights to Tokyo in March` in the command bar (or pick "Ask agent" under any search).
- Schedule jobs in `config.yaml` under `agents.jobs` (`every: 6h`, `at: "08:30"`).

Agents open their tabs in their own space and leave yours alone. A dot in the sidebar shows running agents, and
"Agent run: …" in the command bar opens a run's log. Agents can't read your passwords.

## Passwords

Each profile has its own vault in the macOS Keychain; `passwords.shared` lets several profiles share one. Valerie
offers to save after you sign in, and shows your saved logins when you focus a login field. Search "Password:" in
the command bar to copy one (the clipboard clears after 45s). Import an export with
`valerie import-passwords export.csv [vault]`. Passkeys need an Apple entitlement; see PLAN.md.

## Ad blocking and Boosts

Ads and trackers are blocked with hosts-style lists compiled into WebKit content rules (`content_blocking`). Use
"Toggle Ad Blocking for This Site" in the command bar when a site breaks. Drop `example.com.css` or `example.com.js`
into `~/.config/valerie/boosts/` to restyle or script a site (CSS updates live).

## Agents (MCP)

Valerie runs an MCP server inside the browser on `127.0.0.1:7777` (bearer-token protected). Connect Claude Code:

```sh
$(valerie mcp-config)            # prints and runs: claude mcp add --transport http valerie ... --header "Authorization: Bearer …"
```

For stdio-only clients, use `valerie mcp-stdio` as the command. Tools: `list_spaces`, `list_tabs`, `open_tab`,
`open_link`, `navigate`, `activate_tab`, `close_tab`, `history`, `read_page`, `snapshot`, `screenshot`, `click`,
`type`, `press_key`, `scroll`, `wait_for`, `eval_js`, and `run_command`, which runs any browser command the keyboard
can (except reading passwords).

## Sync

```sh
valerie sync-setup               # creates a private <you>/valerie-sync repo with gh and writes it into config.yaml
```

Each device pushes `devices/<hostname>.json` every few minutes (`⌘⇧Y` syncs now). Pinned tabs, folders, favorites
and spaces sync for real (the newest edit to a space wins). Today tabs stay on their device and show up in the
command bar as "Synced". Nothing but GitHub is involved.

**Links:** middle-, ⌘- or ⇧-click opens a background tab, ⌥-click opens it in split view, ⌥⇧-click Peeks at it
(`esc` closes, `⌘O` keeps it as a tab), and right-click gives Valerie's link menu (⇧-right-click for the system
menu). Each gesture is configurable under `links:`.

## Profiles, spaces and Air Traffic Control

Profiles isolate cookies and site data (separate WebKit data stores). Spaces pick a profile and a theme. `routes`
send matching links to a space (e.g. `*.edu` → School); other external links open in a little window, whose
"Open in …" button has a ▾ to move the page into any space.

## Import

Open the command bar and type "import": Arc (spaces, pinned tabs, folders, favorites), Chrome, Brave, Edge,
Vivaldi, Safari (needs Full Disk Access) and Firefox.

## Signing and passkeys (macOS)

`./scripts/bundle-macos.sh` signs `Valerie.app` with the best certificate in your keychain:

- **Free Apple account**: in Xcode, Settings → Accounts → your Apple ID → Manage Certificates → + → *Apple
  Development*. The app is then signed with hardened runtime and runs on your Mac. If the script still signs ad-hoc
  and `security find-identity -v -p codesigning` shows 0 valid identities, install Apple's
  [WWDR G3 intermediate](https://www.apple.com/certificateauthority/AppleWWDRCAG3.cer) (double-click it).
- **Paid account**: a *Developer ID Application* certificate is picked automatically. Set `VALERIE_NOTARY_PROFILE`
  (see `xcrun notarytool store-credentials`) to notarize so the app runs on other Macs.
- **Passkeys** need Apple's `web-browser.public-key-credential` entitlement (paid account, requested from Apple).
  Point `VALERIE_PROVISIONING_PROFILE` at the profile that carries it and the script adds the entitlement.

Run "Make Valerie the Default Browser" from the command bar once the app is in /Applications.

## Develop

`VALERIE_UI_DIR=ui cargo run` loads the HTML/JS UI from disk, so UI changes only need a relaunch. `cargo test`
runs the unit tests.
