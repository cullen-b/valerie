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
| `⌘⌥↓` `⌘⌥↑` | Next / previous tab |
| `⌃Tab` | Back to the tab you were just on |
| `⌘⌥→` `⌘⌥←`, `⌃1…5`, two-finger swipe on sidebar | Switch space |
| `⌘1…8` | Jump to tab |
| `⌘⇧D` | Pin / unpin |
| `⌃⇧=` / `⌃⇧-` / `⌃⇧→` | Split view / close split / focus next pane |
| `⌘O` | Move a little window into the main window |
| `⌘F`, `⌘=` `⌘-` `⌘0`, `⌘⌥I` | Find, zoom (remembered per site), dev tools |
| `⌘⇧R` / `⌘⇧2` / `⌘⇧M` | Reader view / capture the page (clipboard + Downloads) / mute the tab |
| `⌘⇧V` / `⌘⌥⇧C` | Paste and go / copy the page as a Markdown link |
| `⌘P` | Print |
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

**Media:** tabs opened in the background wait to play until you look at them; a playing video pops out into
picture-in-picture when you switch away; a mini player above the space icons pauses, mutes or jumps back to whatever
is playing in another tab; click a tab's ♪ to mute it. **Downloads** go to your Downloads folder and show up in the
command bar (type the file name).

**Little things:** the command bar is a calculator and unit converter (`12*7`, `15% of 80`, `5 km in mi`, `70 f to c`;
Enter copies). Hovering a link shows where it goes. Tracking junk (`utm_*`, `fbclid`…) is stripped from links you
open or copy. Cookie pop-ups get "Reject all" clicked for you. "Hide an Element" removes anything annoying from a site
for good (it's saved as a line in `boosts/<site>.css`). Plus Translate Page, Copy Link to Highlighted Text, Search
Selected Text, Close Duplicate Tabs, and dark mode for light websites (`pages.dark_mode`, or per site).

**Pages behave:** `alert` / `confirm` / `prompt` show as proper sheets on the window, "Sign in with Google"-style
pop-ups open as small linked windows so sign-ins complete (`links.popups`), sites ask before using the camera or
microphone (`permissions`), and a thin line along the top shows a page loading (`pages.loading_bar`).

## AI: Chats, questions in find, and an agent that drives your browser

Valerie uses the agent apps you already have, signed in with your own subscriptions: **Claude Code, Codex, OpenCode,
Pi, Cursor and Hermes**. "Set Up <name>" in the command bar installs one if needed and opens its sign-in: one step.

- **Chats** (`⌘J`, or swipe left past your first space): conversations with your agent, pinned and closed like tabs.
  The agent can see and use your whole browser, so "merge my approved PRs once checks pass" or "compare the laptops
  in my tabs" just works. Chats are saved in the config folder, so they sync.
- **Ask from the command bar:** type a question or a request; it opens in Chats (`> do this` / `? ask that` force it).
- **Ask from find:** press `/`, type a question ("what's the return policy here?"), Enter: the answer appears right
  under the find bar, in Valerie's own panel (the page can't read it). Any agent works; it only sees that page and runs
  with as little power as its CLI allows (no tools for Claude, Pi and OpenCode; read-only and offline for Codex,
  Cursor and Hermes), since the page's text could be trying to trick it.
- **Pick an agent per job:** `agents.available` names as many as you like (same CLI with another model, or any
  command), `agents.use` picks one for chat, find, scheduled tasks and customizing: e.g. a strong model to chat and a
  fast one for find.

- **AI actions** (`ai_actions`): one-click jobs in the command bar, bindable to keys: Summarize this page, Explain
  the selection, Key facts and dates, Tidy my tabs, Short tab titles, Find related reading, and anything you add in
  YAML (`{title}`, `{url}`, `{selection}`, `{input}` fill-ins; `mode: page` answers on the page, `mode: chat` can use
  the browser).
- **Watch This Page…:** "tell me when the price drops below $500" / "when checks pass on this PR". An agent checks in
  the background every 30 minutes (`watch.every_minutes`), notifies you once it happens, and leaves the details in Chats.

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

Run **Set Up Sync** from the command bar (or `valerie sync-setup`). It signs you in to GitHub (through the GitHub
CLI, or Valerie's own sign-in if `sync.github_client_id` is set), creates a private `<you>/valerie-sync` repo, and
from then on syncs a few seconds after every change. Your **config folder is the repo**: settings, boosts, plugins and
every device's tabs (`devices/<hostname>.json`). Files Valerie regenerates per machine (`AGENTS.md`, `reference/`) and
anything secret-looking (`*.csv`, `.env`) stay out.

**New machine:** install Valerie, run Set Up Sync, done: it finds your repo and pulls everything down (your old
`config.yaml` is kept as `config.before-sync.yaml`). Or have your assistant clone the repo into the config folder
(`valerie paths` prints where that is) before first launch.

Pinned tabs, folders, favorites and spaces sync for real (the newest edit to a space wins). Today tabs stay on their
device and show up in the command bar as "Synced". If two machines change the same line of a setting, the one that
syncs last wins. `⌘⇧Y` syncs now. Nothing but GitHub is involved.

**Links:** middle-, ⌘- or ⇧-click opens a background tab, ⌥-click opens it in split view, ⌥⇧-click Peeks at it
(`esc` closes, `⌘O` keeps it as a tab), and right-click gives Valerie's link menu (⇧-right-click for the system
menu). Each gesture is configurable under `links:`.

## Profiles, spaces and Air Traffic Control

Profiles isolate cookies and site data (separate WebKit data stores). Spaces pick a profile and a theme.
`site_profiles` pins a site to a profile wherever you open it, e.g. `youtube.com: YouTube` keeps YouTube signed in to
one Google account while the rest of Google uses another. `routes`
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
