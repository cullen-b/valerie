#!/usr/bin/env python3
"""Runs what the owner really does in Valerie on a throwaway Linux or Windows machine, through Valerie's own tools.

  scripts/linux-test.sh up ubuntu && scripts/usage-test.py linux
  scripts/windows-vm.sh up && scripts/windows-vm.sh try && scripts/usage-test.py windows

Uses the owner's settings and plugins with sync removed; the machine is thrown away afterwards. Results go to
target/usage-test/<machine>.json. Exits non-zero if anything failed.
"""
import base64, json, os, pathlib, re, subprocess, sys, tempfile, time

MACHINE = sys.argv[1]
ONLY = set(sys.argv[2:])
HOME = pathlib.Path.home()
REPO = pathlib.Path(__file__).resolve().parent.parent
OUT = REPO / "target" / "usage-test"
CONFIG = HOME / ".config" / "valerie"
VM = HOME / "VMs" / "valerie-test" / "windows"
CONTAINER = "valerie-linux-test"
SSH = ["-q", "-i", str(VM / "key"), "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null", "-o", "ConnectTimeout=5"]
WIN_EXE = r"C:\Users\tester\Downloads\Valerie\valerie.exe"
results = []


def sh(args, timeout=120, stdin=None):
    try:
        p = subprocess.run(args, capture_output=True, text=True, timeout=timeout, input=stdin, errors="replace")
        return p.returncode, p.stdout + p.stderr
    except subprocess.TimeoutExpired:
        return 124, "timed out"


def ps(script, timeout=120):
    encoded = base64.b64encode(("$ProgressPreference='SilentlyContinue'; " + script).encode("utf-16-le")).decode()
    return sh(["ssh", *SSH, "-p", "2222", "tester@127.0.0.1", f"powershell -NoProfile -OutputFormat Text -EncodedCommand {encoded}"], timeout)


def machine_sh(script, timeout=120):
    """A command in the machine's own shell, as the test user."""
    if MACHINE == "linux":
        return sh(["docker", "exec", "-u", "tester", "-e", "HOME=/home/tester", "-e", "DISPLAY=:1", CONTAINER, "sh", "-c", script], timeout)
    return ps(script, timeout)


def call(tool, args=None, timeout=90):
    body = json.dumps(args or {})
    if MACHINE == "linux":
        return sh(["docker", "exec", "-u", "tester", "-e", "HOME=/home/tester", "-e", "DISPLAY=:1", CONTAINER, "valerie", "call", tool, body], timeout)
    # Piped, or Windows won't wait for or read a windowed app. Base64 survives PowerShell 5's quoting of JSON.
    encoded = base64.b64encode(body.encode()).decode()
    return ps(f"$j = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{encoded}')) -replace '\"','\\\"'; & '{WIN_EXE}' call {tool} $j 2>&1 | Out-String", timeout)


def press(keys):
    if MACHINE == "linux":
        return sh(["docker", "exec", "-u", "tester", "-e", "DISPLAY=:1", CONTAINER, "xdotool", "key", keys.lower().replace("return", "Return")])
    return sh([str(REPO / "scripts" / "windows-vm.sh"), "key", keys.lower().replace("+", "-").replace("return", "ret")])


def click_center():
    sh([str(REPO / "scripts" / "windows-vm.sh"), "click", "400", "300"])
    time.sleep(2)


def type_text(text):
    if MACHINE == "linux":
        return sh(["docker", "exec", "-u", "tester", "-e", "DISPLAY=:1", CONTAINER, "xdotool", "type", "--delay", "15", text])
    names = {" ": "spc", ".": "dot"}
    for c in text:
        key = names.get(c) or ("shift-" + c.lower() if c.isupper() else c)
        sh([str(REPO / "scripts" / "windows-vm.sh"), "key", key])


def command(cmd, **options):
    return call("run_command", {"command": {"cmd": cmd, **options}})


def check(name, ok, detail=""):
    detail = " ".join(str(detail).split())[:400]
    results.append({"name": name, "ok": bool(ok), "detail": detail})
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  ({detail[:160]})" if detail and not ok else ""), flush=True)


def tabs(space=None):
    code, out = call("list_tabs", {"space": space} if space else {})
    return out


def find_tab(url_part, space=None):
    try:
        data = json.loads(tabs(space))
    except ValueError:
        return None
    found = []

    def walk(v):
        if isinstance(v, dict):
            if "tab" in v and url_part in str(v.get("url", "")):
                found.append(v["tab"])
            for x in v.values():
                walk(x)
        elif isinstance(v, list):
            for x in v:
                walk(x)

    walk(data)
    return found[-1] if found else None


def logs(contains, minutes=30):
    return call("read_logs", {"contains": contains, "since_minutes": minutes})[1]


def setup():
    # The owner's settings, minus sync: this machine must never reach the real sync repo.
    text = (CONFIG / "config.yaml").read_text()
    text = re.sub(r"^sync:\n(?:[ \t]+.*\n)*", "", text, flags=re.M)
    assert "valerie-sync" not in text
    text += "\nwelcome: false\nupdates: { check: false }\n"
    tmp = pathlib.Path(tempfile.mkdtemp())
    (tmp / "config.yaml").write_text(text)
    code, paths = machine_sh("valerie paths" if MACHINE == "linux" else f"& '{WIN_EXE}' paths | Out-String")
    conf = re.search(r"config:\s*(.+)", paths).group(1).strip()
    folder = conf if not conf.endswith((".yaml", ".yml")) else str(pathlib.PurePath(conf).parent) if MACHINE == "linux" else conf.rsplit("\\", 1)[0]
    plugins = [p for p in (CONFIG / "plugins").iterdir() if not p.name.startswith("_")]
    if MACHINE == "linux":
        machine_sh("pkill -x valerie; sleep 1; true")
        machine_sh(f"mkdir -p '{folder}/plugins' '{folder}/extensions'")
        sh(["docker", "cp", str(tmp / "config.yaml"), f"{CONTAINER}:{folder}/config.yaml"])
        for p in plugins:
            sh(["docker", "cp", str(p), f"{CONTAINER}:{folder}/plugins/"])
        sh(["docker", "cp", str(CONFIG / "extensions" / "youtube-theater"), f"{CONTAINER}:{folder}/extensions/"])
        sh(["docker", "exec", CONTAINER, "chown", "-R", "tester:tester", "/home/tester"])
        sh(["docker", "exec", "-d", "-u", "tester", "-e", "HOME=/home/tester", "-e", "DISPLAY=:1", CONTAINER,
            "dbus-launch", "--exit-with-session", "sh", "-c", 'valerie >"$HOME/valerie.out" 2>&1'])
    else:
        ps("Stop-Process -Name valerie,msedgewebview2 -Force -EA 0; Start-Sleep 2")
        ps(f"New-Item -ItemType Directory -Force '{folder}\\plugins','{folder}\\extensions' | Out-Null")
        win = folder.replace("\\", "/")
        scp = ["scp", *SSH, "-P", "2222", "-r"]
        sh([*scp, str(tmp / "config.yaml"), f"tester@127.0.0.1:{win}/config.yaml"])
        for p in plugins:
            sh([*scp, str(p), f"tester@127.0.0.1:{win}/plugins/"])
        sh([*scp, str(CONFIG / "extensions" / "youtube-theater"), f"tester@127.0.0.1:{win}/extensions/"])
        ps("schtasks /run /tn open-valerie | Out-Null")
    for _ in range(40):
        time.sleep(2)
        code, out = call("list_spaces")
        if "Personal" in out:
            return True
    print("Valerie never answered:", out)
    return False


def t_spaces():
    out = call("list_spaces")[1]
    check("my three spaces are there", all(n in out for n in ["Personal", "School", "Work"]), out)
    command("next_space"); time.sleep(1)
    moved = call("list_spaces")[1]
    command("prev_space"); time.sleep(1)
    back = call("list_spaces")[1]
    check("next_space and prev_space move between spaces", moved != out and back == out, moved)


def t_tabs():
    code, out = call("open_tab", {"url": "https://example.com", "activate": True})
    time.sleep(4)
    t = find_tab("example.com")
    check("open a tab", t is not None, out)
    page = call("read_page", {"tab": t})[1]
    check("read the page", "Example Domain" in page, page)
    check("run script in the page", "Example Domain" in call("eval_js", {"tab": t, "script": "return document.title"})[1])
    call("navigate", {"tab": t, "url": "https://www.iana.org/help/example-domains"}); time.sleep(5)
    command("back"); time.sleep(4)
    check("back", "example.com" in (call("eval_js", {"tab": t, "script": "return location.href"})[1]))
    command("forward"); time.sleep(4)
    check("forward", "iana.org" in (call("eval_js", {"tab": t, "script": "return location.href"})[1]))
    command("back"); time.sleep(3)
    call("eval_js", {"tab": t, "script": "window.__mark = 1; return 1"})
    command("reload"); time.sleep(4)
    check("reload", "true" in call("eval_js", {"tab": t, "script": "return window.__mark === undefined"})[1])
    call("open_tab", {"url": "https://www.wikipedia.org", "activate": False}); time.sleep(4)
    w = find_tab("wikipedia.org")
    check("second tab", w is not None)
    code, out = call("activate_tab", {"tab": w}); time.sleep(1)
    check("switch tabs", code == 0 and "error" not in out.lower(), out)
    code, out = command("copy_url")
    check("copy the address", "Copied" in logs("Copied", 2), out)
    command("close_tab"); time.sleep(2)
    check("close the active tab, others stay", find_tab("wikipedia.org") is None and find_tab("example.com") is not None, tabs())
    command("reopen_closed_tab"); time.sleep(4)
    check("reopen a closed tab", find_tab("wikipedia.org") is not None)


def t_screen():
    code, out = call("screenshot", {})
    check("screenshot a page", "image saved" in out, out)
    code, out = call("screenshot_ui", {})
    check("screenshot Valerie's own UI", "image saved" in out, out)
    command("toggle_sidebar"); time.sleep(1); command("toggle_sidebar"); time.sleep(1)
    check("hide and show the sidebar", "toggle_sidebar" in logs("toggle_sidebar", 2))
    command("open_command_bar"); time.sleep(1); command("close_command_bar")
    check("open and close the command bar", "close_command_bar" in logs("close_command_bar", 2))
    code, out = command("split_view"); time.sleep(2)
    check("split view runs", code == 0, out)
    command("split_view"); time.sleep(1)


def t_routes_and_profiles():
    # Routes are for links from other apps; a link opens in a little window, which belongs to the route's space.
    call("open_link", {"url": "https://www.mit.edu"}); time.sleep(5)
    command("move_to_main_window"); time.sleep(3)
    t = find_tab("mit.edu", "School")
    check("a *.edu link from another app goes to the School space", t is not None, tabs("School"))
    call("open_tab", {"url": "https://example.org", "space": "Personal", "activate": False}); time.sleep(4)
    a = find_tab("example.org", "Personal")
    call("open_tab", {"url": "https://example.org", "space": "Work", "activate": False}); time.sleep(4)
    b = find_tab("example.org", "Work")
    call("eval_js", {"tab": a, "script": "document.cookie='valerie_test=personal; max-age=600'; localStorage.setItem('valerie_test','personal'); return 1"})
    time.sleep(1)
    seen = call("eval_js", {"tab": b, "script": "return String(document.cookie) + '|' + localStorage.getItem('valerie_test')"})[1]
    check("a login in one space stays out of the other", a and b and "personal" not in seen, seen)


def t_little_window():
    code, out = call("open_link", {"url": "https://example.net"}); time.sleep(5)
    check("external link opens a little window", code == 0 and "failed" not in logs("little", 2).lower(), logs("little", 2) + out)
    # It acts on the little window that has the keyboard, and Windows won't hand that to a window opened from the background.
    if MACHINE == "windows":
        click_center()
    code, out = command("move_to_main_window"); time.sleep(3)
    check("little window moves to the main window", find_tab("example.net") is not None, tabs())


def t_downloads():
    # The owner's failed downloads were a large file from a redirecting Microsoft host.
    t = call("open_tab", {"url": "https://proof.ovh.net/files/1Mb.dat", "activate": True})
    for _ in range(20):
        time.sleep(2)
        out = call("list_downloads")[1]
        if "1Mb" in out:
            break
    check("download a file", "1Mb" in out, out + logs("download", 3))


def t_plugins():
    out = call("get_settings", {})[1]
    code, listing = machine_sh("valerie plugins list" if MACHINE == "linux" else f"& '{WIN_EXE}' plugins list | Out-String")
    check("my marketplace plugins are found", "youtube-speed" in listing, listing)
    call("open_tab", {"url": "https://www.youtube.com/watch?v=dQw4w9WgXcQ", "activate": True}); time.sleep(10)
    t = find_tab("youtube.com")
    state = call("site_state", {"site": "youtube.com"})[1]
    check("youtube-speed plugin applies on YouTube", "youtube-speed" in state, state)
    code, out = call("eval_js", {"tab": t, "script": "return !!document.querySelector('.valerie-speed') || !!document.getElementById('movie_player')"})
    check("youtube page has the plugin's player to work on", "true" in out, out)
    command("close_tab")
    code, out = command("extensions"); time.sleep(1)
    ext = logs("xtension", 2)
    check("youtube-theater extension loads", "youtube-theater" in ext and "work on macOS" not in ext, ext + out)
    machine_sh("valerie plugins install youtube-theater-mode" if MACHINE == "linux" else f"& '{WIN_EXE}' plugins install youtube-theater-mode | Out-String", 90)
    time.sleep(3)
    check("the marketplace's YouTube theater plugin installs, standing in for the extension", "youtube-theater-mode" in call("site_state", {"site": "youtube.com"})[1])


def t_web_tools():
    code, out = call("fetch_page", {"urls": ["https://example.com"]}, timeout=60)
    check("agents can read a page (fetch_page)", "Example Domain" in out, out)
    code, out = call("web_search", {"query": "valerie browser"}, timeout=60)
    check("agents can search (web_search)", '"results"' in out and '"url"' in out, out)


def t_settings():
    code, out = call("set_setting", {"key": "sidebar.tab_height", "value": "36"})
    time.sleep(2)
    check("an agent can change a setting", code == 0 and "36" in call("get_settings", {"key": "sidebar.tab_height"})[1], out)
    command("reload_config"); time.sleep(2)
    check("reload the settings", "Config reloaded" in logs("Config reloaded", 2))
    machine_sh("valerie check-config" if MACHINE == "linux" else f"& '{WIN_EXE}' check-config | Out-String")
    code, out = machine_sh("valerie check-config" if MACHINE == "linux" else f"& '{WIN_EXE}' check-config | Out-String")
    check("my synced settings are valid here (terminal: iTerm, editor: hx)", code == 0 and "iTerm" not in out, out)
    command("open_config"); time.sleep(4)
    machine_sh("pkill -x xdg-open; true" if MACHINE == "linux" else "Stop-Process -Name notepad,WindowsTerminal,OpenConsole -Force -EA 0")
    check("hx isn't here, so the settings file opens in the usual app", "isn't on this computer" in logs("editor", 2), logs("editor", 2))
    check("open the settings file from Valerie", "couldn't" not in logs("", 1).lower() and "failed" not in logs("", 1).lower(), logs("", 1))


def t_chat():
    # The owner's default is Claude; a machine without it should use what it has. Needs free AI set up first.
    install = "curl -fsSL https://opencode.ai/install | bash" if MACHINE == "linux" else "$d = \"$HOME\\.opencode\\bin\"; New-Item -ItemType Directory -Force $d | Out-Null; irm https://github.com/anomalyco/opencode/releases/latest/download/opencode-windows-x64.zip -OutFile $env:TEMP\\oc.zip; Expand-Archive -Force $env:TEMP\\oc.zip $d"
    machine_sh(install, 300)
    time.sleep(35)  # Valerie looks again at most every 30 seconds
    # Agents may not start chats themselves, so this goes through the keyboard as a person would.
    press("Ctrl+J"); time.sleep(4)
    type_text("Reply with exactly the word PINEAPPLE and nothing else."); time.sleep(1)
    press("Return"); time.sleep(3)
    chats = call("list_chats", {})[1]
    check("start a chat", "PINEAPPLE" in chats or '"id"' in chats, chats + logs("send_chat", 3))
    found = ""
    for _ in range(30):
        time.sleep(5)
        chats = call("list_chats", {})[1]
        ids = re.findall(r'"id":\s*"?([\w-]+)', chats)
        if ids:
            found = call("read_chat", {"id": ids[0]})[1]
            if "PINEAPPLE" in found.replace("exactly the word PINEAPPLE", ""):
                break
    check("the chat answers (Claude missing, so another AI stands in)", "PINEAPPLE" in found.replace("exactly the word PINEAPPLE", ""), found[-300:] + logs("chat", 6)[-300:])


def t_restart():
    before = find_tab("example.com")
    pid = lambda: machine_sh("pgrep -x valerie" if MACHINE == "linux" else "(Get-Process valerie -EA 0).Id")[1].split()
    old = pid()
    command("restart")
    came_back = False
    for _ in range(45):
        time.sleep(2)
        code, out = call("list_spaces")
        now = pid()
        if "Personal" in out and now and now != old:
            came_back = True
            time.sleep(5)
            break
    check("restart really starts a new Valerie", came_back, f"{old} -> {pid()}")
    check("my tabs come back after a restart", before is not None and find_tab("example.com") is not None, tabs())


def t_errors():
    code, out = call("read_logs", {"since_minutes": 60})
    bad = [l for l in out.splitlines() if re.search(r"\b(failed|couldn't|could not|panic|crash)", l, re.I)]
    check("nothing failed in Valerie's log", not bad, "; ".join(bad)[:400])


TESTS = [t_spaces, t_tabs, t_screen, t_routes_and_profiles, t_little_window, t_downloads, t_plugins, t_web_tools, t_settings, t_chat, t_restart, t_errors]

OUT.mkdir(parents=True, exist_ok=True)
if not setup():
    sys.exit(2)
for test in TESTS:
    if ONLY and test.__name__ not in ONLY:
        continue
    print(f"--- {test.__name__}", flush=True)
    try:
        test()
    except Exception as e:  # one broken area shouldn't hide the rest
        check(f"{test.__name__} ran to the end", False, repr(e))
(OUT / f"{MACHINE}.json").write_text(json.dumps(results, indent=1))
failed = [r for r in results if not r["ok"]]
print(f"\n{len(results) - len(failed)} passed, {len(failed)} failed")
sys.exit(1 if failed else 0)
