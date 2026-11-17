#!/usr/bin/env python3
"""Valerie's speed and footprint, measured the way ardia.dev/benchmarks measures browsers.

  scripts/bundle-macos.sh && scripts/perf-bench.py [--app target/Valerie.app] [--rounds 5] [--only launch,tabs,idle]

Runs a test copy (its own folders, profile and MCP port 7795; never the owner's Valerie), started through Launch
Services without taking focus. Every figure is a median. Same test page as Ardia's: a local page with 60 boxes, 120
paragraphs and a 1 s timer, window 1280x800. CPU, wakeups and memory are summed over Valerie and every WebKit process
it is responsible for (proc_pid_rusage). VALERIE_BENCH=1 makes the test window's pages draw as if in view, so the app
must be a build that has it. Run it at normal priority: under `taskpolicy -b` the pages never get to draw.
"""
import argparse, ctypes, http.server, json, os, shutil, statistics, subprocess, sys, tempfile, threading, time, urllib.request

PORT, PAGE_PORT = 7795, 9462
PAGE = """<!doctype html><meta charset=utf-8><title>bench</title><style>.b{display:inline-block;width:80px;height:40px;margin:4px;
background:#8ad}</style><body><div id=t>0</div>%s%s<script>let n=0;setInterval(()=>{document.getElementById('t').textContent=++n},1000);
document.addEventListener('visibilitychange',()=>{if(!document.hidden)requestAnimationFrame(()=>{window.__shown=performance.timeOrigin+performance.now()})});
</script>""" % ("<div class=b></div>" * 60, "".join(f"<p>Paragraph {i} with some text to lay out across the line.</p>" for i in range(120)))

# ---------------------------------------------------------------- process accounting (proc_pid_rusage, V4)
lib = ctypes.CDLL("/usr/lib/libSystem.B.dylib")
lib.responsibility_get_pid_responsible_for_pid.argtypes = [ctypes.c_int]

class RU(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_uint8 * 16)] + [(n, ctypes.c_uint64) for n in (
        "user_time", "system_time", "pkg_idle_wkups", "interrupt_wkups", "pageins", "wired_size", "resident_size",
        "phys_footprint", "proc_start_abstime", "proc_exit_abstime", "child_user_time", "child_system_time",
        "child_pkg_idle_wkups", "child_interrupt_wkups", "child_pageins", "child_elapsed_abstime", "diskio_bytesread",
        "diskio_byteswritten", "cpu_time_qos_default", "cpu_time_qos_maintenance", "cpu_time_qos_background",
        "cpu_time_qos_utility", "cpu_time_qos_legacy", "cpu_time_qos_user_initiated", "cpu_time_qos_user_interactive",
        "billed_system_time", "serviced_system_time", "logical_writes", "lifetime_max_phys_footprint", "instructions",
        "cycles", "billed_energy", "serviced_energy", "interval_max_phys_footprint", "runnable_time")]

class TB(ctypes.Structure):
    _fields_ = [("numer", ctypes.c_uint32), ("denom", ctypes.c_uint32)]
tb = TB(); lib.mach_timebase_info(ctypes.byref(tb))
TICK_NS = tb.numer / tb.denom

def family(root):
    pids = [int(p) for p in subprocess.run(["ps", "-axo", "pid="], capture_output=True, text=True).stdout.split()]
    return [p for p in pids if p == root or lib.responsibility_get_pid_responsible_for_pid(p) == root]

def usage(root):
    cpu = wk = mem = 0
    for p in family(root):
        ru = RU()
        if lib.proc_pid_rusage(p, 4, ctypes.byref(ru)) == 0:
            cpu += (ru.user_time + ru.system_time) * TICK_NS / 1e9
            wk += ru.pkg_idle_wkups + ru.interrupt_wkups
            mem += ru.phys_footprint
    return cpu, wk, mem / 1e6

# ---------------------------------------------------------------- test copy
class Copy:
    def __init__(self, app, root):
        app = os.path.realpath(app)
        self.app, self.bin = app, os.path.join(app, "Contents/MacOS/valerie")
        self.cfg, self.data = os.path.join(root, "config"), os.path.join(root, "data")
        self.pid = None

    def fresh(self):
        if subprocess.run(["lsof", "-ti", f"tcp:{PORT}", "-sTCP:LISTEN"], capture_output=True).stdout:
            sys.exit(f"something already listens on port {PORT} (a test copy left running?)")
        shutil.rmtree(os.path.dirname(self.cfg), ignore_errors=True)
        os.makedirs(self.cfg); os.makedirs(self.data)
        with open(os.path.join(self.cfg, "config.yaml"), "w") as f:
            f.write(f"mcp: {{ port: {PORT} }}\nprofiles: [{{ name: Test }}]\nspaces: [{{ name: Test, profile: Test }}]\n"
                    "updates: { check: false }\nwelcome: false\nwindow: { width: 1280, height: 800 }\n")

    def start(self, *args, wait=("active page drawn",)):
        # Through Launch Services, like a click in the Dock: then WebKit's processes count as Valerie's (from a shell
        # they'd count as the terminal's), and the time includes what macOS does to start an app.
        env = dict(VALERIE_CONFIG_DIR=self.cfg, VALERIE_DATA_DIR=self.data, VALERIE_TRACE="1", VALERIE_TEST_WINDOWS="1", VALERIE_BENCH="1")
        log = os.path.join(self.data, "trace.log")
        open(log, "w").close()
        before = set(self.pids())
        t0 = time.perf_counter()
        subprocess.run(["open", "-n", "-g", "-a", self.app, "--stderr", log, *sum((["--env", f"{k}={v}"] for k, v in env.items()), []),
                        *(["--args", *args] if args else [])], check=True)
        seen, want, self.pid = {}, set(wait), None
        with open(log) as f:
            while not want <= seen.keys():
                if time.perf_counter() - t0 > 20:
                    raise RuntimeError(f"timed out; saw {list(seen)}")
                line = f.readline()
                if not line.endswith("\n"):
                    f.seek(f.tell() - len(line)); time.sleep(0.002); continue
                if "valerie:" in line and " ms  " in line:
                    step, at = line.split(" ms  ", 1)[1].strip().split(" (")[0], (time.perf_counter() - t0) * 1000
                    seen.setdefault(step, at)
                    if step.endswith("page drawn"):  # a link opens in a little window, or a tab
                        seen.setdefault("page drawn", at)
        self.pid = next(iter(set(self.pids()) - before))
        self.trace = seen
        return {k: seen[k] for k in wait}

    def pids(self):
        out = subprocess.run(["pgrep", "-lf", self.bin], capture_output=True, text=True).stdout.splitlines()
        return [int(l.split()[0]) for l in out if "mcp-stdio" not in l]

    def call(self, tool, args):
        token = open(os.path.join(self.data, "mcp-token")).read().strip()
        body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}}).encode()
        req = urllib.request.Request(f"http://127.0.0.1:{PORT}/mcp", body, {"Authorization": f"Bearer {token}", "Content-Type": "application/json",
                                     "Accept": "application/json, text/event-stream"})
        raw = urllib.request.urlopen(req, timeout=30).read().decode()
        raw = next((l[5:] for l in raw.splitlines() if l.startswith("data:")), raw)
        r = json.loads(raw)["result"]
        text = "".join(c.get("text", "") for c in r.get("content", []))
        if r.get("isError"):
            raise RuntimeError(f"{tool}: {text}")
        try:
            return json.loads(text)
        except ValueError:
            return text

    def wait_mcp(self):
        for _ in range(500):
            try:
                return self.call("list_tabs", {})
            except Exception:
                time.sleep(0.02)
        raise RuntimeError("MCP never answered")

    def stop(self):
        if self.pid:
            kids = family(self.pid)
            try:  # quit like a person would, so it saves its tabs
                self.call("run_command", {"command": {"cmd": "quit"}})
            except Exception:
                pass
            for p in [self.pid] + kids:  # WebKit's processes linger a little after the app goes; a cold start shouldn't find them.
                for i in range(200):
                    try:
                        os.kill(p, 0); time.sleep(0.05)
                    except OSError:
                        break
                    if i == 150:
                        os.kill(p, 9)
            self.pid = None
        time.sleep(1)

def tab_ids(c):
    out = c.call("list_tabs", {})
    items = out if isinstance(out, list) else out.get("tabs", [])
    return [t["tab"] for t in items if t.get("section") != "hidden"]

def eval_js(c, tab, script):
    r = c.call("eval_js", {"tab": tab, "script": script})
    return r.get("result", r) if isinstance(r, dict) and "result" in r else r

# ---------------------------------------------------------------- benchmarks
def bench_launch(c, rounds, url):
    out = {"Cold launch → window on screen": [], "Cold launch → page painted (link from another app)": [],
           "Cold launch → restored tab painted": []}
    for _ in range(rounds):
        c.fresh(); c.stop()
        t = c.start(url, wait=("page drawn",))  # link: the page it was opened with
        out["Cold launch → page painted (link from another app)"].append(t["page drawn"])
        c.wait_mcp(); c.call("open_tab", {"url": url, "activate": True}); time.sleep(3); c.stop()
        t0 = time.time() * 1000
        t = c.start(wait=("window shown", "active page drawn"))  # restored: the tab it had last time
        out["Cold launch → window on screen"].append(t["window shown"])
        # The page's own first paint (Paint Timing), not just Valerie hearing that it loaded.
        c.wait_mcp()
        tab = next(x["tab"] for x in c.call("list_tabs", {}) if x.get("active"))
        fcp = None
        for _ in range(300):
            fcp = eval_js(c, tab, "const e=performance.getEntriesByName('first-contentful-paint')[0];return e?performance.timeOrigin+e.startTime:null")
            if fcp:
                break
            time.sleep(0.01)
        out["Cold launch → restored tab painted"].append(fcp - t0)
        c.stop()
    return out

def bench_tabs(c, rounds, url):
    c.fresh(); c.start(wait=("window shown",)); c.wait_mcp(); c.call("open_tab", {"url": url, "activate": True}); time.sleep(3)
    new, switch = [], []
    for _ in range(rounds):
        t0 = time.time() * 1000
        tid = c.call("open_tab", {"url": url, "activate": True})["tab"]
        for _ in range(500):
            fcp = eval_js(c, tid, "const e=performance.getEntriesByName('first-contentful-paint')[0];return e?performance.timeOrigin+e.startTime:null")
            if fcp:
                break
            time.sleep(0.01)
        new.append(fcp - t0)
        time.sleep(1)
    ids = tab_ids(c)
    for i in range(rounds):
        target = ids[i % (len(ids) - 1)]
        eval_js(c, target, "window.__shown=null;return 1")
        t0 = time.time() * 1000
        c.call("activate_tab", {"tab": target})
        for _ in range(500):
            shown = eval_js(c, target, "return window.__shown||null")
            if shown:
                break
            time.sleep(0.005)
        switch.append(shown - t0)
        c.call("activate_tab", {"tab": ids[-1]}); time.sleep(1)
    c.stop()
    return {"New tab → first paint": new, "Tab switch → shown": switch}

def idle(pid, secs):
    c0, w0, _ = usage(pid); time.sleep(secs); c1, w1, m = usage(pid)
    return (c1 - c0) / secs * 100, (w1 - w0) / secs, m

def bench_idle_memory(c, rounds, url, settle=60):
    out = {}
    for _ in range(rounds):
        c.fresh(); c.start(wait=("window shown",)); c.wait_mcp(); pid = c.pid
        c.call("open_tab", {"url": url, "activate": True})
        time.sleep(30); out.setdefault("Memory, 1 tab (MB)", []).append(usage(pid)[2])
        cpu, wk, _ = idle(pid, settle); out.setdefault("Idle CPU, 1 tab (%)", []).append(cpu); out.setdefault("Idle wakeups, 1 tab (/s)", []).append(wk)
        for n in range(19):
            c.call("open_tab", {"url": f"{url}?{n}", "activate": True})
            if n == 8:
                time.sleep(10); out.setdefault("Memory, 10 tabs (MB)", []).append(usage(pid)[2])
        time.sleep(15); out.setdefault("Memory, 20 tabs (MB)", []).append(usage(pid)[2])
        cpu, wk, _ = idle(pid, settle); out.setdefault("Idle CPU, 20 tabs (%)", []).append(cpu); out.setdefault("Idle wakeups, 20 tabs (/s)", []).append(wk)
        for t in tab_ids(c)[:-1]:
            c.call("close_tab", {"tab": t})
        time.sleep(30); out.setdefault("Memory, back to 1 tab after 20 (MB)", []).append(usage(pid)[2])
        c.stop()
    return out

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--app", default="target/Valerie.app")
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--only", default="launch,tabs,idle")
    ap.add_argument("--json")
    a = ap.parse_args()
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", PAGE_PORT), type("H", (http.server.BaseHTTPRequestHandler,), {
        "do_GET": lambda s: (s.send_response(200), s.send_header("Content-Type", "text/html"), s.end_headers(), s.wfile.write(PAGE.encode())),
        "log_message": lambda *a: None}))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{PAGE_PORT}/"
    c = Copy(os.path.abspath(a.app), os.path.join(tempfile.gettempdir(), f"valerie-bench-{PORT}", "x"))
    results = {}
    try:
        for part in a.only.split(","):
            r = {"launch": bench_launch, "tabs": bench_tabs, "idle": bench_idle_memory}[part](c, a.rounds, url)
            results.update(r)
            for k, v in r.items():
                print(f"{k:55} {statistics.median(v):8.2f}   (min {min(v):.2f}, max {max(v):.2f}, n={len(v)})", flush=True)
    finally:
        c.stop()
    if a.json:
        json.dump(results, open(a.json, "w"), indent=1)

if __name__ == "__main__":
    main()
