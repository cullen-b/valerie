# Working on Valerie

Notes for every agent (Claude Code, Codex, OpenCode, …) working in this repo.

## The owner's standing rule: every add/remove becomes a setting

When the owner asks to **remove** something (a label, a button, a behaviour) or **add** something, do it, **and**
add a setting so anyone can switch it back on or off:

- Make the requested state the default, and keep the other state one setting away.
- Name the setting after what you see (`sidebar.new_tab_button`, `start_page`), not after the implementation.
- Put it in `config/default.yaml` in the matching section, with a short comment saying what it controls and the
  allowed values (`true | false`, or "one of: a | b | c").
- Add the field to the matching struct in `src/config.rs` and read it where the feature lives.

## Never nag

No "press Esc to exit full screen" banners, ever, and no "esc to …" reminders unless `escape_hints` is on. The owner
finds them maddening.

## Page scripts: no innerHTML

Scripts that run inside web pages (`ui/content.js`, plugin `script.js`) must build DOM with `createElement` /
`textContent`, never `innerHTML`: sites with Trusted Types (YouTube, Google) throw on it and the feature silently
breaks there.

## Page scripts: send through `post()`

`ui/content.js` talks to Valerie with `post()`, which writes JSON itself. Don't send messages with `JSON.stringify` of an
object: it hands the object to the page's `Object.prototype.toJSON` first, and the page could read nonces or plugin keys.

## Keep the settings file readable for people and LLMs

`config/default.yaml` is the reference for every option; users mostly edit their own small
`~/.config/valerie/config.yaml` (often by asking an LLM to). So:

- Group options under the existing section banners; don't scatter related keys.
- Give every key a one-line comment: what it does, the units (pixels, minutes), and the allowed values.
- Use plain words and positive names (`url_bar: true` shows it), not double negatives.
- Never break existing keys silently: users' files only hold overrides and are merged onto the defaults.

## Keep Valerie customisable by LLMs

Users change Valerie by asking an LLM, which reads the guide Valerie writes into their config folder
(`src/llm_guide.rs`, also `valerie llm-guide`). When you add a command, setting, plugin field or MCP tool, make sure
the guide covers it: commands with options go in `WITH_ARGS` (`src/command.rs`), settings get comments in
`config/default.yaml`, and `valerie check-config` should catch mistakes in anything new.

## Building and checking

- `cargo build`, `cargo test` (one Keychain test is opt-in: `cargo test -- --ignored keychain`).
- Every push to `main` is an **edge** (testing) build (`.github/workflows/release.yml`): tested, built for macOS,
  Linux and Windows, signed with Valerie's release key and published as a pre-release with `latest.json`. Copies on
  `updates.channel: edge` (the owner's Mac, via its `this-mac.yaml`) install it within hours, so merging still ships
  to the owner. Everyone else is on **stable**: a build reaches them only when the owner approves it and it's promoted
  with `gh workflow run promote.yml` (no rebuild; the notes list every change since the last stable release). Never
  promote without the owner's yes; every now and then, ask whether they want a stable release, listing the changes in
  plain words. `scripts/install-macos.sh` is only for trying a build before it's pushed.
- The release key (`assets/release-signing.pem`; private half in the repo's `MACOS_SIGNING_P12` secret and the owner's
  keychain) must not change: macOS ties camera, microphone and keychain permissions to it, and the updater only
  accepts builds signed with it (`RELEASE_REQUIREMENTS`, also `site/install.sh`). Changing keys takes one release that
  lists both.
- Test instances never update themselves (unless `VALERIE_UPDATE_TEST=1`, which updates that instance's own copy of
  the app); "Restart Valerie" keeps the instance's `VALERIE_*` folders.
- Run the app as the `.app`, not the bare `target/*/valerie` binary: WebKit keeps the bare binary's logins in a
  separate store (`~/Library/WebKit/valerie` vs `~/Library/WebKit/dev.valerie.browser`).
- UI is plain HTML/JS in `ui/`; `VALERIE_UI_DIR=ui cargo run` reloads it from disk.
- Run test instances with `VALERIE_CONFIG_DIR`/`VALERIE_DATA_DIR` pointing at a temp dir and a non-default
  `mcp.port` (e.g. 7790) so you never touch the owner's real browser. Also give the test config its own profiles
  and spaces (e.g. `profiles: [{ name: Test }]`, `spaces: [{ name: Test, profile: Test }]`): cookies and site
  storage are keyed by profile name, so a test using "Personal" reads and writes the owner's real logins. Drive them through the MCP server
  (token in `$VALERIE_DATA_DIR/mcp-token`); the `screenshot` tool shows a page.
- Agents a test instance starts (customize, chats, tasks) run with that instance's folders, but they can still read
  the owner's real `~/.config/valerie`. After testing one, check that the real config didn't change.
- The agent shell can't capture the screen. Check UI changes by loading `ui/*.html` in Playwright with a stubbed
  `window.ipc` and mock state.
- `cargo check --target x86_64-pc-windows-msvc` type-checks the Windows build; CI builds macOS, Linux, Windows.
- Plan and roadmap: `PLAN.md`.
