// Real test of first launch on a throwaway Windows machine (.github/workflows/windows-e2e.yml): starts valerie.exe the
// way Explorer does, watches for console windows, and clicks "Install and sign in" in the real onboarding page.
// It deletes Codex and ~/.codex between steps, so never run it on a computer someone uses.
import { execFileSync, spawn } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const EXE = process.argv[2], OUT = process.argv[3], PORT = 9222;
const HOME = process.env.USERPROFILE, CODEX_HOME = join(HOME, ".codex");
const CODEX_DIR = join(process.env.LOCALAPPDATA, "Programs", "OpenAI", "Codex");
const CODEX = join(CODEX_DIR, "bin", "codex.exe");
const WINDOWS = join(OUT, "console-windows.jsonl");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const ps = (cmd) => execFileSync("powershell", ["-NoProfile", "-Command", cmd], { encoding: "utf8", windowsHide: true });
const say = (s) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${s}`);
const check = (ok, what) => {
  say(`${ok ? "PASS" : "FAIL"}: ${what}`);
  if (!ok) process.exitCode = 1;
};

// Writes every visible console window that appears, polling fast enough to catch a flash.
const WATCH = `
Add-Type @'
using System; using System.Text; using System.Collections.Generic; using System.Runtime.InteropServices;
public static class W {
  delegate bool F(IntPtr h, IntPtr l);
  [DllImport("user32")] static extern bool EnumWindows(F f, IntPtr l);
  [DllImport("user32")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  public static List<string> Consoles() {
    var o = new List<string>();
    EnumWindows((h, l) => {
      var c = new StringBuilder(256); GetClassName(h, c, 256);
      if (IsWindowVisible(h) && (c.ToString() == "ConsoleWindowClass" || c.ToString() == "CASCADIA_HOSTING_WINDOW_CLASS")) {
        var t = new StringBuilder(512); GetWindowText(h, t, 512);
        o.Add(h + "|" + c + "|" + t);
      }
      return true;
    }, IntPtr.Zero);
    return o;
  }
}
'@
$seen = @{}
foreach ($w in [W]::Consoles()) { $seen[$w.Split('|')[0]] = 1 }
Set-Content -LiteralPath '${WINDOWS}' -Value ''
while ($true) {
  foreach ($w in [W]::Consoles()) {
    $h = $w.Split('|')[0]
    if (-not $seen[$h]) { $seen[$h] = 1; Add-Content -LiteralPath '${WINDOWS}' -Value $w }
  }
  Start-Sleep -Milliseconds 40
}`;

const consoles = () => (existsSync(WINDOWS) ? readFileSync(WINDOWS, "utf8").split(/\r?\n/).filter(Boolean) : []);

function evaluate(ws, expression) {
  return new Promise((resolve, reject) => {
    const s = new WebSocket(ws);
    const timer = setTimeout(() => { s.close(); reject(new Error("no answer from the page")); }, 10000);
    s.onerror = () => { clearTimeout(timer); reject(new Error("couldn't reach the page")); };
    s.onopen = () => s.send(JSON.stringify({ id: 1, method: "Runtime.evaluate", params: { expression, returnByValue: true } }));
    s.onmessage = (m) => { clearTimeout(timer); s.close(); resolve(JSON.parse(m.data).result?.result?.value); };
  });
}

async function screenshot(ws, name) {
  await new Promise((resolve) => {
    const s = new WebSocket(ws);
    s.onerror = resolve;
    s.onopen = () => s.send(JSON.stringify({ id: 1, method: "Page.captureScreenshot" }));
    s.onmessage = (m) => {
      const data = JSON.parse(m.data).result?.data;
      if (data) writeFileSync(join(OUT, name), Buffer.from(data, "base64"));
      s.close();
      resolve();
    };
  });
}

async function findWelcome() {
  for (let i = 0; i < 180; i++) {
    try {
      const targets = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      for (const t of targets.filter((t) => t.type === "page")) {
        if (await evaluate(t.webSocketDebuggerUrl, "!!(window.welcome && typeof W !== 'undefined' && W && W.agents)").catch(() => false)) return t.webSocketDebuggerUrl;
      }
    } catch {}
    await sleep(500);
  }
  throw new Error("the onboarding page never appeared");
}

const CARD = `[...document.querySelectorAll('.card.agent')].find((c) => c.innerText.includes('Codex'))`;
const chip = (ws) => evaluate(ws, `(${CARD})?.querySelector('.chip')?.innerText`);

async function launchAndClick(label) {
  say(`--- ${label}`);
  writeFileSync(WINDOWS, "");
  // Start-Process goes through the shell like a double-click: no console to inherit.
  ps(`Start-Process -FilePath '${EXE}'`);
  const ws = await findWelcome();
  await sleep(5000); // let everything that runs at launch run
  check(consoles().length === 0, `${label}: no console window at launch (${consoles().length})`);
  await evaluate(ws, `go(STEPS.indexOf("ai"))`);
  await sleep(500);
  check((await chip(ws)) === "Not installed", `${label}: Codex starts out not installed`);
  const clicked = await evaluate(ws, `(() => { const b = [...(${CARD}).querySelectorAll('button')].find((b) => b.innerText === 'Install and sign in'); if (b) b.click(); return !!b; })()`);
  check(clicked, `${label}: clicked Install and sign in`);
  return ws;
}

async function watchChips(ws, stop, seconds) {
  const seen = [];
  for (let i = 0; i < seconds * 2; i++) {
    const c = await chip(ws).catch(() => null);
    if (c && seen.at(-1) !== c) { seen.push(c); say(`card: ${c}`); }
    if (stop(c)) return seen;
    await sleep(500);
  }
  return seen;
}

function closeValerie() {
  ps(`Stop-Process -Name valerie,msedgewebview2 -Force -ErrorAction SilentlyContinue; Start-Sleep 2`);
}

function removeCodex() {
  rmSync(CODEX_DIR, { recursive: true, force: true });
  rmSync(CODEX_HOME, { recursive: true, force: true });
}

mkdirSync(OUT, { recursive: true });
process.env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = `--remote-debugging-port=${PORT}`;
writeFileSync(join(OUT, "watch.ps1"), WATCH);
const watcher = spawn("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", join(OUT, "watch.ps1")], { stdio: "ignore", windowsHide: true });

try {
  await sleep(4000); // the watcher compiles its helper
  const path = execFileSync(EXE, ["paths"], { encoding: "utf8", windowsHide: true });
  check(path.includes("config:"), "command line: `valerie paths` still prints when its output is read");
  check(!ps("Get-Command codex -ErrorAction SilentlyContinue").trim(), "this machine has no Codex to begin with");

  // Nobody signed in: installs with no window, then one terminal for the sign-in. It also proves the watcher sees windows.
  let ws = await launchAndClick("not signed in");
  let seen = await watchChips(ws, (c) => c === "In terminal" || c === "Didn't finish", 300);
  await screenshot(ws, "not-signed-in.png");
  check(seen.includes("Installing"), "not signed in: the card said Installing");
  check(seen.at(-1) === "In terminal", `not signed in: the card asks to finish in the terminal (${seen.at(-1)})`);
  check(existsSync(CODEX), "not signed in: the real Codex CLI is installed");
  await sleep(4000);
  const shown = consoles();
  say(`console windows: ${JSON.stringify(shown)}`);
  check(shown.length === 1, `not signed in: exactly one terminal opened (${shown.length})`);
  const procs = ps(`Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -match 'setup.codex.ps1|codex(\\.exe)?"? login' } | ForEach-Object { $_.CommandLine }`);
  say(procs);
  check(/login/.test(procs), "not signed in: that terminal is running codex login");
  check(execFileSync(CODEX, ["--version"], { encoding: "utf8", windowsHide: true }).includes("codex"), "the installed Codex runs");

  // Make a real sign-in file with Codex itself (a made-up key: the status check only reads the file), then start over.
  closeValerie();
  ps(`Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -match 'setup.codex.ps1' -or $_.Name -eq 'codex.exe' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }; Start-Sleep 2`);
  execFileSync(CODEX, ["login", "--with-api-key"], { input: "sk-made-up-for-this-test\n", encoding: "utf8", windowsHide: true });
  const saved = join(OUT, "auth-copy.json");
  copyFileSync(join(CODEX_HOME, "auth.json"), saved);
  removeCodex();
  mkdirSync(CODEX_HOME, { recursive: true });
  copyFileSync(saved, join(CODEX_HOME, "auth.json"));
  rmSync(saved);

  // Already signed in, as with the Codex app: the whole thing happens with no window at all.
  ws = await launchAndClick("already signed in");
  seen = await watchChips(ws, (c) => ["Checking", "Ready", "Needs sign-in", "Didn't finish", "In terminal"].includes(c), 300);
  await sleep(4000);
  await screenshot(ws, "already-signed-in.png");
  check(seen.includes("Installing"), "already signed in: the card said Installing");
  check(!seen.includes("In terminal") && !seen.includes("Didn't finish"), `already signed in: no terminal step and no failure (${seen.join(" > ")})`);
  check(existsSync(CODEX), "already signed in: the real Codex CLI is installed");
  check(consoles().length === 0, `already signed in: no console window at any point (${JSON.stringify(consoles())})`);
} catch (e) {
  say(`FAIL: ${e.stack || e}`);
  process.exitCode = 1;
} finally {
  watcher.kill();
  closeValerie();
  rmSync(join(OUT, "watch.ps1"), { force: true });
}
