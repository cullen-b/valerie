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

## Keep the settings file readable for people and LLMs

`config/default.yaml` is the reference for every option; users mostly edit their own small
`~/.config/valerie/config.yaml` (often by asking an LLM to). So:

- Group options under the existing section banners; don't scatter related keys.
- Give every key a one-line comment: what it does, the units (pixels, minutes), and the allowed values.
- Use plain words and positive names (`url_bar: true` shows it), not double negatives.
- Never break existing keys silently: users' files only hold overrides and are merged onto the defaults.

## Building and checking

- `cargo build`, `cargo test` (one Keychain test is opt-in: `cargo test -- --ignored keychain`).
- UI is plain HTML/JS in `ui/`; `VALERIE_UI_DIR=ui cargo run` reloads it from disk.
- Run test instances with `VALERIE_CONFIG_DIR`/`VALERIE_DATA_DIR` pointing at a temp dir and a non-default
  `mcp.port` (e.g. 7790) so you never touch the owner's real browser. Drive them through the MCP server
  (token in `$VALERIE_DATA_DIR/mcp-token`); the `screenshot` tool shows a page.
- The agent shell can't capture the screen. Check UI changes by loading `ui/*.html` in Playwright with a stubbed
  `window.ipc` and mock state.
- `cargo check --target x86_64-pc-windows-msvc` type-checks the Windows build; CI builds macOS, Linux, Windows.
- Plan and roadmap: `PLAN.md`.
