#!/usr/bin/env python3
"""Presses every shortcut in config/default.yaml on a throwaway machine with a page focused, and checks the right command ran.

  scripts/linux-test.sh up ubuntu && scripts/shortcut-test.py linux
  scripts/windows-vm.sh up && scripts/windows-vm.sh try && scripts/shortcut-test.py windows

Results go to target/usage-test/shortcuts-<machine>.json. Exits non-zero if a shortcut didn't work.
"""
import base64, json, pathlib, re, subprocess, sys, time

MACHINE = sys.argv[1]
REPO = pathlib.Path(__file__).resolve().parent.parent
VM = pathlib.Path.home() / "VMs" / "valerie-test" / "windows"
CONTAINER = "valerie-linux-test"
SSH = ["-q", "-i", str(VM / "key"), "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null", "-o", "ConnectTimeout=5"]
WIN_EXE = r"C:\Users\tester\Downloads\Valerie\valerie.exe"
# These end the session or open a system dialog nothing here can close.
SKIP = {"quit", "close_window", "restart", "print", "new_window", "dev_tools", "open_config", "open_downloads_folder"}
XDOTOOL = {"Equal": "equal", "Minus": "minus", "Quote": "apostrophe", "Comma": "comma", "[": "bracketleft", "]": "bracketright",
           "PageUp": "Prior", "PageDown": "Next", "Semicolon": "semicolon", "Period": "period", "Slash": "slash", "Backslash": "backslash", "Space": "space"}
QEMU = {"Equal": "equal", "Minus": "minus", "Quote": "apostrophe", "Comma": "comma", "[": "bracket_left", "]": "bracket_right",
        "PageUp": "pgup", "PageDown": "pgdn", "Semicolon": "semicolon", "Period": "dot", "Slash": "slash", "Backslash": "backslash", "Space": "spc"}


def sh(args, timeout=60):
    try:
        p = subprocess.run(args, capture_output=True, text=True, timeout=timeout, errors="replace")
        return p.returncode, p.stdout + p.stderr
    except subprocess.TimeoutExpired:
        return 124, "timed out"


def call(tool, args=None):
    body = json.dumps(args or {})
    if MACHINE == "linux":
        return sh(["docker", "exec", "-u", "tester", "-e", "HOME=/home/tester", "-e", "DISPLAY=:1", CONTAINER, "valerie", "call", tool, body])
    quoted = '"' + body.replace("\\", "\\\\").replace('"', '\\"') + '"'
    return sh(["ssh", *SSH, "-p", "2222", "tester@127.0.0.1", f"{WIN_EXE} call {tool} {quoted}"])


def press(accel):
    parts = accel.split("+")
    mods, key = parts[:-1], parts[-1]
    if MACHINE == "linux":
        names = {"CmdOrCtrl": "ctrl", "Ctrl": "ctrl", "Alt": "alt", "Shift": "shift"}
        combo = "+".join([names[m] for m in mods] + [XDOTOOL.get(key, key.lower() if len(key) == 1 else key)])
        return sh(["docker", "exec", "-u", "tester", "-e", "DISPLAY=:1", CONTAINER, "xdotool", "key", combo])
    names = {"CmdOrCtrl": "ctrl", "Ctrl": "ctrl", "Alt": "alt", "Shift": "shift"}
    combo = "-".join([names[m] for m in mods] + [QEMU.get(key, key.lower())])
    return sh([str(REPO / "scripts" / "windows-vm.sh"), "key", combo])


def click(x, y):
    if MACHINE == "linux":
        return sh(["docker", "exec", "-u", "tester", "-e", "DISPLAY=:1", CONTAINER, "xdotool", "mousemove", str(x), str(y), "click", "1"])
    return sh([str(REPO / "scripts" / "windows-vm.sh"), "click", str(x), str(y)])


def bindings():
    text = (REPO / "config" / "default.yaml").read_text().split("\nkeybindings:\n", 1)[1]
    found = []
    for line in text.splitlines():
        if line and not line.startswith((" ", "#")):
            break
        m = re.match(r'\s+"([^"]+)":\s*(.+?)(?:\s+#.*)?$', line)
        if not m or " " in m.group(1) or m.group(2).startswith("null"):
            continue
        accel, value = m.group(1), m.group(2)
        c = re.search(r"cmd:\s*(\w+)", value)
        name = c.group(1) if c else value.strip()
        # Off a Mac, spaces take Alt+digit: Ctrl+digit is the tab's there.
        if name == "switch_space":
            accel = accel.replace("Ctrl+", "Alt+")
        found.append((accel, name))
    return found


def command_log():
    # read_logs caps how many it returns, so only look at the last minute.
    return [l for l in call("read_logs", {"area": "command", "since_minutes": 1})[1].splitlines() if "[command]" in l]


def reset():
    call("run_command", {"command": {"cmd": "close_command_bar"}})
    if "example.com" not in call("list_tabs")[1]:
        call("open_tab", {"url": "https://example.com", "activate": True})
        time.sleep(4)
    # Inside the page, so these are the shortcuts someone reading a page presses.
    click(*(500, 400) if MACHINE == "windows" else (800, 500))
    time.sleep(0.5)


results = []
all_bindings = [b for b in bindings() if len(sys.argv) < 3 or re.search(sys.argv[2], b[0])]
print(f"{len(all_bindings)} shortcuts", flush=True)
for accel, name in all_bindings:
    if name in SKIP:
        results.append({"accel": accel, "command": name, "ok": None, "got": "skipped"})
        continue
    reset()
    before = set(command_log())
    press(accel)
    time.sleep(1.5)
    new = [l for l in command_log() if l not in before]
    got = [l.split("[command]", 1)[1].split()[0].strip('",') for l in new if l.split("[command]", 1)[1].split()]
    ok = name in got
    results.append({"accel": accel, "command": name, "ok": ok, "got": got})
    print(f"{'PASS' if ok else 'FAIL'}  {accel:28} {name}" + ("" if ok else f"  got {got}"), flush=True)
    # A little window takes the keyboard, and the find bar keeps it: put things back for the next key.
    if name == "move_to_little_window":
        call("run_command", {"command": {"cmd": "move_to_main_window"}})
        time.sleep(2)
    if name in ("find_in_page", "reader_view"):
        press("Escape")
    if name in ("open_chats", "new_chat", "toggle_terminal"):
        call("run_command", {"command": {"cmd": "close_chat" if name != "toggle_terminal" else "toggle_terminal"}})
    alive = call("list_spaces")[0] == 0
    if not alive:
        print("Valerie stopped; ending here")
        results[-1]["ok"] = False
        break

out = REPO / "target" / "usage-test"
out.mkdir(parents=True, exist_ok=True)
(out / f"shortcuts-{MACHINE}.json").write_text(json.dumps(results, indent=1))
failed = [r for r in results if r["ok"] is False]
print(f"\n{sum(1 for r in results if r['ok'])} worked, {len(failed)} failed, {sum(1 for r in results if r['ok'] is None)} skipped")
sys.exit(1 if failed else 0)
