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
`valerie default-config` prints every option with comments. `valerie paths` shows where config and data live.

## Keys (all remappable in `config.yaml`)

| Keys | Action |
|---|---|
| `⌘T` / `⌘D` | Command bar: search tabs, history, bookmarks, commands, synced tabs; enter a URL or search |
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

**Vim motions inside pages** (off while typing; `Esc` leaves a field): `j/k/h/l` scroll, `d/U` half page,
`gg/G` top/bottom, `f`/`F` link hints (same/new tab), `u`/`H` back, `L` forward, `K/J` or `gt/gT` next/prev tab,
`x`/`X` close/reopen, `o`/`O` command bar/edit URL, `t` new tab, `yy` copy URL, `/` find, `i` insert mode,
`s` toggle sidebar, `]s`/`[s` next/prev space.

Drag tabs anywhere in the sidebar: onto the pinned tiles to pin, into folders, onto the favorites row, or onto a
space icon. Hide the sidebar with `⌘S`, then touch the window edge to peek at it.

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

Shift-click any link to **Peek** at it (`esc` closes, `⌘O` keeps it as a tab).

## Profiles, spaces and Air Traffic Control

Profiles isolate cookies and site data (separate WebKit data stores). Spaces pick a profile and a theme. `routes`
send matching links to a space (e.g. `*.edu` → School); other external links open in a little window.

## Import

Open the command bar and type "import": Arc (spaces, pinned tabs, folders, favorites), Chrome, Brave, Edge,
Vivaldi, Safari (needs Full Disk Access) and Firefox.

## Develop

`VALERIE_UI_DIR=ui cargo run` loads the HTML/JS UI from disk, so UI changes only need a relaunch. `cargo test`
runs the unit tests.
