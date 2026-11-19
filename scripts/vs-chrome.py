#!/usr/bin/env python3
"""Valerie vs Chrome on this Mac, for the figures on the website: launch to first paint, memory with 1, 10 and 20 tabs, idle CPU.

  scripts/vs-chrome.py /Applications/Valerie.app [rounds]

Same test page and process accounting as perf-bench.py, clean profiles, the two taking turns. Chrome opens in the
background with a throwaway profile. Run it at normal priority. Prints medians and writes vs.json.
"""
import importlib.util, http.server, json, os, shutil, statistics, subprocess, sys, tempfile, threading, time, urllib.request, urllib.parse
spec = importlib.util.spec_from_file_location("pb", os.path.join(os.path.dirname(os.path.abspath(__file__)), "perf-bench.py")); pb = importlib.util.module_from_spec(spec); spec.loader.exec_module(pb)
PAGE_PORT, CDP = 9463, 9334
painted = {}
PAGE = pb.PAGE + """<script>new PerformanceObserver(l=>{for(const e of l.getEntries())if(e.name==='first-contentful-paint')fetch('/painted?k='+encodeURIComponent(location.search)+'&t='+(performance.timeOrigin+e.startTime))}).observe({type:'paint',buffered:true})</script>"""
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(s):
        u = urllib.parse.urlparse(s.path)
        if u.path == "/painted":
            q = urllib.parse.parse_qs(u.query, keep_blank_values=True); painted[q["k"][0]] = float(q["t"][0])
            s.send_response(204); s.end_headers(); return
        s.send_response(200); s.send_header("Content-Type", "text/html"); s.end_headers(); s.wfile.write(PAGE.encode())
    def log_message(s, *a): pass
threading.Thread(target=http.server.ThreadingHTTPServer(("127.0.0.1", PAGE_PORT), H).serve_forever, daemon=True).start()
URL = f"http://127.0.0.1:{PAGE_PORT}/"
def wait_paint(key, secs=30):
    t = time.time()
    while key not in painted:
        if time.time() - t > secs: raise RuntimeError("never painted " + key)
        time.sleep(0.005)
    return painted[key]

class Chrome:
    name = "Chrome"
    def __init__(s): s.dir = os.path.join(tempfile.gettempdir(), "vs-chrome-profile"); s.pid = None
    def start(s, url):
        shutil.rmtree(s.dir, ignore_errors=True)
        subprocess.run(["open", "-n", "-g", "-a", "Google Chrome", "--args", f"--user-data-dir={s.dir}", f"--remote-debugging-port={CDP}", "--no-first-run",
                        "--no-default-browser-check", "--window-size=1280,800", "--disable-backgrounding-occluded-windows", "--disable-features=CalculateNativeWinOcclusion", url], check=True)
        for _ in range(400):
            out = subprocess.run(["pgrep", "-f", f"user-data-dir={s.dir}"], capture_output=True, text=True).stdout.split()
            mains = [int(p) for p in out if "--type=" not in subprocess.run(["ps", "-o", "command=", "-p", p], capture_output=True, text=True).stdout]
            if mains: s.pid = mains[0]; return
            time.sleep(0.01)
        raise RuntimeError("chrome didn't start")
    def tab(s, url):
        urllib.request.urlopen(urllib.request.Request(f"http://127.0.0.1:{CDP}/json/new?{url}", method="PUT"), timeout=10).read()
    def stop(s):
        if not s.pid: return
        kids = pb.family(s.pid)
        subprocess.run(["kill", str(s.pid)])
        for p in [s.pid] + kids:
            for i in range(200):
                try: os.kill(p, 0); time.sleep(0.05)
                except OSError: break
                if i == 150: os.kill(p, 9)
        s.pid = None; time.sleep(1)

class Valerie:
    name = "Valerie"
    def __init__(s, app): s.c = pb.Copy(app, os.path.join(tempfile.gettempdir(), f"valerie-bench-{pb.PORT}", "x")); s.pid = None
    def start(s, url):
        s.c.fresh(); s.c.start(url, wait=("page drawn",)); s.pid = s.c.pid; s.c.wait_mcp()
    def tab(s, url): s.c.call("open_tab", {"url": url, "activate": True})
    def stop(s): s.c.stop(); s.pid = None

browsers = [Valerie(sys.argv[1]), Chrome()]
rounds = int(sys.argv[2]) if len(sys.argv) > 2 else 3
res = {b.name: {} for b in browsers}
def add(b, k, v): res[b.name].setdefault(k, []).append(v); print(f"{b.name:8} {k:28} {v:9.1f}", flush=True)
try:
    for r in range(rounds + 2):  # launch: first round is a warm-up and isn't counted
        for b in (browsers if r % 2 else browsers[::-1]):
            key = f"?launch-{b.name}-{r}"; painted.pop(key, None)
            t0 = time.time() * 1000; b.start(URL + key); ms = wait_paint(key) - t0
            if r: add(b, "launch to page painted (ms)", ms)
            time.sleep(2); b.stop()
    for r in range(rounds):
        for b in (browsers if r % 2 else browsers[::-1]):
            b.start(URL + f"?m{r}"); time.sleep(30); add(b, "memory, 1 tab (MB)", pb.usage(b.pid)[2])
            for n in range(19):
                b.tab(URL + f"?m{r}-{n}"); time.sleep(0.4)
                if n == 8: time.sleep(10); add(b, "memory, 10 tabs (MB)", pb.usage(b.pid)[2])
            time.sleep(20); add(b, "memory, 20 tabs (MB)", pb.usage(b.pid)[2])
            c0 = pb.usage(b.pid)[0]; time.sleep(30); add(b, "idle CPU, 20 tabs (%)", (pb.usage(b.pid)[0] - c0) / 30 * 100)
            b.stop()
finally:
    for b in browsers:
        try: b.stop()
        except Exception as e: print("stop failed", e)
print("\nMEDIANS")
out = {n: {k: round(statistics.median(v), 1) for k, v in d.items()} for n, d in res.items()}
print(json.dumps(out, indent=1)); json.dump({"medians": out, "raw": res}, open("vs.json", "w"), indent=1)
