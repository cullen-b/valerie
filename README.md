# Valerie

A homemade browser: **Rust-native, configured with a text file, synced (encrypted) through GitHub, and drivable by AI
agents**, with a coding workspace for the agent you already use.
Arc's best ideas (spaces, pinned tabs and folders, the command bar, little windows, Air Traffic Control, split
view) on top of the system web engine, so on macOS you get Safari's engine and its speed and battery life.

See [PLAN.md](PLAN.md) for the architecture, the engine decision and the roadmap.

## Install

- **Terminal** (smoothest): `curl -fsSL https://cullen-b.github.io/valerie/install.sh | bash`
- **Homebrew**: `brew install --cask cullen-b/valerie/valerie`
- **Download**: [Valerie.dmg](https://github.com/cullen-b/valerie/releases/latest/download/Valerie.dmg), or Linux and
  Windows builds from [Releases](https://github.com/cullen-b/valerie/releases/latest). Website:
  https://cullen-b.github.io/valerie/

Valerie updates itself (`updates:` in the settings; "Check for Updates" in the command bar) from **stable**
releases: builds that have been used and approved before they go out. Every push to `main` becomes an **edge**
build first. To get those as they're made, run **Toggle Test Builds** from the command bar (it sets
`updates.channel: edge` for that computer); run it again to go back to stable. Until releases are notarized, the first open of a downloaded or Homebrew copy needs System
Settings › Privacy & Security › Open Anyway, once.

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
After a keyboard click (`f`, or `Enter` in a text box), the cursor jumps into the sign-in, reply or compose box that
shows up, so you can type straight away (header search bars and feed composers are left alone; `vim.jump_to_text_box`).

Each space has **pinned** tabs (icon tiles), **Kept** tabs (a list under them that stays until you remove it), and
**today** tabs (archived after 12 hours unused). Pinned, Kept and favorite tabs show **−** instead of ×: it closes
the page but keeps the tab. Ones you haven't opened in 3 months get a gentle "keep these?" (Keep All, Remove All, or
One by One; removed ones stay searchable in the archive; `tabs.ask_to_keep_unused_days`, or "Review Saved Tabs" any
time). Folders work in pinned and Kept. Drag tabs anywhere in the sidebar:
onto the pinned tiles, into Kept or a folder, onto the favorites row, or onto a space icon. Drag the sidebar's edge to
resize it: narrow it to show icon + domain, narrower still for icons only (`layout.sidebar_size`, double-click the
edge to cycle). Hide the sidebar with `⌘S`, then touch the window edge to peek at it.

**Markdown files** (`.md`, on your computer or the web) open as formatted documents: GitHub-style tables, task lists,
notes and warnings, footnotes, highlighted code with a Copy button, and an outline of the headings beside the text.
Source shows the plain text. For a file on your computer, Edit opens Valerie's own editor: the text beside a live
preview, saved as you type (`⌘S` saves now, `⌘B`/`⌘I` bold and italic, lists carry on when you press Enter, Esc is
Done). If the file changes somewhere else while you have unsaved edits, Valerie waits for you to choose. Prefer your own
app? `pages.markdown_edit: editor` opens your `editor` instead. Either way the page updates the moment the file is
saved, by you or your AI. "Open Markdown Files in Valerie" in the command bar makes Valerie the app `.md` files open in;
one opened from Finder or another app comes up in a little window (`pages.markdown`, `markdown_theme`,
`markdown_outline`, `markdown_live`, `markdown_edit`).

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

- **Chats** (`⌘J`, or swipe left past your first space; black by default, `chats.theme`): conversations with your
  agent. The agent can see and use your whole browser, so "merge my approved PRs once checks pass" or "compare the
  laptops in my tabs" just works, and it keeps your own MCP servers, plugins and skills (`chats.own_tools`).
  - The composer picks the agent, the model (each CLI's own list), the reasoning effort, and access: **Read only**
    (it can look but not act), **Ask first** (you approve each action in a sheet), or **Full access**.
  - Send while it's working and your message waits in a queue. A timer shows how long it's been at it.
  - Pin, **snooze** (an hour, this evening, tomorrow, next week…) or **settle** chats; untouched chats settle after
    3 days (`chats.auto_settle_days`). Working and snoozed chats sit at the bottom of the sidebar; unread answers
    get a dot.
  - A chat started from a space (the command bar, an AI action, `⌘N`) **floats** over the middle of the window: `Esc`
    sends it on to Chats so you keep working, `Shift+Esc` goes with it (`chats.from_space`). Closed chats sit in a
    folded Closed list; click one to bring it back.
  - **Copy Chat ID** and paste `chat:<id>` into another chat to bring that conversation along.
  - Chats sync between your machines, encrypted with your sync passphrase (`chats.sync`). A chat started on
    another machine carries on here with whichever AI this machine has.
- **Coding:** give a chat a project folder (the folder button in the composer) and its agent works there, with your
  browser to check what it builds. Each step shows its diff or output, approvals happen in the chat, and a card lists
  the uncommitted changes with Commit, Open a pull request and Merge back buttons (`chats.change_actions`). Several
  chats on one project each get their own git worktree (`chats.worktrees`). The terminal panel (`Ctrl+'`) runs real
  shells along the bottom, in the chat's project; the agent can start a dev server there and read its output.
- **Ask from the command bar:** type a question or a request; it opens in Chats (`> do this` / `? ask that` force it).
- **Ask from find:** press `/`, type a question ("what's the return policy here?"), Enter: the answer appears right
  under the find bar, in Valerie's own panel (the page can't read it). Any agent works; it only sees that page and runs
  with as little power as its CLI allows (no tools for Claude, Pi and OpenCode; read-only and offline for Codex,
  Cursor and Hermes), since the page's text could be trying to trick it.
- **Pick an agent per job:** `agents.available` names as many as you like (same CLI with another model, or any
  command), `agents.use` picks one for chat, find, scheduled tasks and customizing: e.g. a strong model to chat and a
  fast one for find.

- **AI actions** (`ai_actions`): one-click jobs in the command bar, bindable to keys: Summarize this page, Explain
  the selection, Key facts and dates, Tidy my tabs, Triage my tabs (a plan for every open tab, done only when you say
  go), Short tab titles, Find related reading, and anything you add in YAML (`{title}`, `{url}`, `{selection}`,
  `{input}` fill-ins; `mode: page` answers on the page, `mode: chat` can use the browser).
- **Report a Problem…:** your AI reads Valerie's log, works out what went wrong, and fixes the setting or writes the
  bug report for you.
- **Tasks:** "Create Task…" in the command bar asks two things in plain words: what ("go through my Jira and tell
  me what's on my plate today", "join my Google Meet calls when they start", "open my dashboards side by side") and
  when ("weekdays at 8am", "every 2 minutes", "when Valerie starts", "only when I ask"). An agent with your signed-in
  browser does it. Things to read become a tab (in Kept, always the latest, with earlier ones and "Run it again now");
  things to do log to a chat or just notify you. Tasks live under `tasks:` in your config, so you (or your LLM) can
  reword them any time. A morning brief is just one task.
- **Notifications:** Valerie's own (a task's page is ready, a watched page changed, a chat answered while you were
  away) and websites' (Gmail, Slack, Calendar…: Valerie asks once per site) show as normal macOS notifications;
  clicking one takes you to the brief, chat or tab. Settings under `notifications:`; "Notifications from This Site"
  flips the site you're on.
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
scripts in any language; `style.css` / `script.js` for matching sites (before the site's own code, in frames, again
when a web app changes its address); hooks for browser events (a tab opens, a download finishes, you go idle…);
right-click entries, sidebar buttons with live text, live command-bar results and a start page; and output as a
toast, a page, a list to pick from, a notification or the clipboard. Page scripts reach the browser through
`valerie.call(tool, args)` with the tools their plugin lists (you're asked once). Local code drives the browser with
`valerie call <tool> '<json>'` (for example `valerie call read_page '{}'`).

**Almost everything else is a setting**, so an assistant can change it from a sentence: Valerie's own look
(`ui/chrome.css` and friends), its language (`language` + `strings/<code>.yaml`), the sidebar's order, the menu bar,
right-click menus, mouse buttons, gestures and key sequences, per-site settings (`sites:` camera, zoom, user agent,
autoplay…), per-space settings, network rules (block with Adblock syntax or EasyList, redirect, hide, add headers),
download rules, the window (always on top, see-through, frosted), the Dock and menu bar icons, shortcuts that work in
every app, extra instructions for your agents, and Safari-style browser extensions (macOS 15.4+). Every AI change can
be undone with **Undo Last Customization**.

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
from then on syncs a few seconds after every change. Your **config folder is the repo**: settings, boosts and plugins
as readable files, and each machine's tabs, chats and saved passwords **encrypted with your sync passphrase**
(XChaCha20-Poly1305, with a key made from the passphrase by Argon2id). You type the passphrase once on each machine;
GitHub, or anyone who gets the repo, sees only scrambled files. Files Valerie regenerates per machine (`AGENTS.md`,
`reference/`), `this-mac.yaml` and anything secret-looking (`*.csv`, `.env`, `*.bak`) stay out.

**Safe by default:**
- Each machine syncs with **its own key that can reach only the sync repo** (a GitHub deploy key, listed under the
  repo's Settings › Deploy keys, so a lost machine can be cut off there). Valerie keeps no GitHub sign-in of its own;
  the sign-in during setup is only used to make the repo and add the key (`sync.repo_key`).
- **Plugins and AI programs that arrive from another machine wait** until you allow them on this one ("Allow Plugins
  from Other Machines"), showing what they'd run. What you make on a machine is never held (`sync.ask_before_plugins`).
- Turning encryption on starts the repo's history over, so nothing readable from before stays in it.
- **Change the passphrase** any time from "Sync Passphrase" on a machine that has it: everything is encrypted again,
  the history starts over, and each other machine asks for the new one once. Forgot it everywhere? "Start over"
  there: every machine still has its own tabs, chats and passwords and sends them again with the new passphrase.
- **Different AI apps on different machines** just work: settings for one machine go in `this-mac.yaml`, the AI you
  pick while setting up a new machine stays that machine's, and if a machine doesn't have the AI the others use,
  Valerie uses one it has (`agents.fallback`).

**New machine:** install Valerie, run Set Up Sync and type your sync passphrase: it finds your repo and pulls
everything down (your old `config.yaml` is kept as `config.before-sync.yaml`). Or have your assistant clone the repo into the config folder
(`valerie paths` prints where that is) before first launch.

Pinned tabs, folders, favorites and spaces sync for real (the newest edit to a space wins). Today tabs stay on their
device and show up in the command bar as "Synced". If two machines change the same line of a setting, the one that
syncs last wins. Saved passwords: the newest change wins, deletions included. `⌘⇧Y` syncs now. Nothing but GitHub is
involved.

**Links:** middle-, ⌘- or ⇧-click opens a background tab, ⌥-click opens it in split view, ⌥⇧-click Peeks at it
(`esc` closes, `⌘O` keeps it as a tab), and right-click gives Valerie's link menu (⇧-right-click for the system
menu). Each gesture is configurable under `links:`.

## Profiles, spaces and Air Traffic Control

Profiles isolate cookies and site data (separate WebKit data stores). Spaces pick a profile and a theme.
`site_profiles` pins a site to a profile wherever you open it, e.g. `youtube.com: YouTube` keeps YouTube signed in to
one Google account while the rest of Google uses another. `routes`
send matching links to a space (e.g. `*.edu` → School); other external links open in a little window, whose
"Open in …" button has a ▾ to move the page into any space.

**A little window from anywhere:** `Ctrl+;` (from any app; change it under `global_keybindings`) opens an empty little
window asking where to go: type an address or a search and press Enter, or Esc to close it. Another app can do the same
with `open -b dev.valerie.browser 'valerie://little'` (`?url=https://…` opens that page instead). With Karabiner-Elements,
for example, right ⌥A for Valerie and right ⌥V for a little window:

```json
{ "from": { "key_code": "a", "modifiers": { "mandatory": ["right_option"] } },
  "to": [{ "shell_command": "open -b dev.valerie.browser" }], "type": "basic" },
{ "from": { "key_code": "v", "modifiers": { "mandatory": ["right_option"] } },
  "to": [{ "shell_command": "open -b dev.valerie.browser 'valerie://little'" }], "type": "basic" }
```

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
