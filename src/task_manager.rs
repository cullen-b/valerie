//! The Task Manager page (valerie://page/task-manager): what each tab and Valerie's own processes cost, and ending a
//! tab's web process. A child of app.rs (declared there) so it can read the App's views. Nothing here runs unless the
//! page asks: it measures only while it's on screen.

use super::{site_of, App};
use crate::command::TabId;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Instant;
use wry::WebView;

/// One process's memory (bytes) and CPU time so far (nanoseconds), where the system says.
#[derive(Clone, Copy, Default)]
struct Usage {
    memory: Option<u64>,
    cpu_ns: Option<u64>,
}

/// The last CPU reading of each process, to turn CPU time into a share of one core between two looks.
static LAST: Mutex<Option<(Instant, HashMap<u32, u64>)>> = Mutex::new(None);

/// Percent of one core each process used since the last look (none on the first look, or one too soon after).
fn cpu_shares(now: &HashMap<u32, Usage>) -> HashMap<u32, f64> {
    let Ok(mut last) = LAST.lock() else { return HashMap::new() };
    let t = Instant::now();
    let mut out = HashMap::new();
    if let Some((then, before)) = last.as_ref() {
        let wall = t.duration_since(*then).as_nanos() as f64;
        if wall < 1e8 {
            return out;
        }
        for (pid, u) in now {
            if let (Some(c), Some(b)) = (u.cpu_ns, before.get(pid)) {
                out.insert(*pid, c.saturating_sub(*b) as f64 / wall * 100.0);
            }
        }
    }
    *last = Some((t, now.iter().filter_map(|(p, u)| u.cpu_ns.map(|c| (*p, c))).collect()));
    out
}

/// A process the page lists: who's in it and what it is.
struct Row {
    kind: &'static str,
    name: String,
    tabs: Vec<Value>,
}

impl App {
    /// Valerie's own views that aren't tabs or little windows' pages, with what each is.
    fn tm_own_views(&self) -> Vec<(&'static str, &WebView)> {
        let mut out: Vec<(&'static str, &WebView)> = vec![("Sidebar and bars", &self.chrome), ("Sidebar and bars", &self.overlay)];
        out.extend(self.chat_view.iter().map(|v| ("Chat", v)));
        out.extend(self.terminal_view.iter().map(|v| ("Terminal", v)));
        out.extend(self.welcome_view.iter().map(|v| ("Welcome", v)));
        out.extend(self.market_view.iter().map(|v| ("Plugins window", v)));
        out.extend(self.answer_view.iter().map(|v| ("Answer", v)));
        out.extend(self.float_bar.iter().map(|(_, v)| ("Sidebar and bars", v)));
        out.extend(self.split_handles.iter().map(|v| ("Sidebar and bars", v)));
        out.extend(self.peek.iter().map(|p| ("Peek", &p.view)));
        out.extend(self.panels.values().map(|p| ("Panels", &p.view)));
        out.extend(self.littles.values().map(|l| ("Little window bars", &l.bar)));
        out
    }

    /// A tab as the page shows it.
    fn tm_tab(&self, id: TabId) -> Option<Value> {
        let t = self.state.tabs.get(&id)?;
        let host = if t.url.starts_with("valerie:") { Some("Valerie".to_string()) } else { site_of(&t.url) };
        let space = match self.state.space_of(id) {
            Some(i) => self.state.spaces.get(i).map(|s| s.name.clone()),
            None if self.state.favorites.contains(&id) => Some("Favorites".into()),
            None if self.agent_tabs.contains_key(&id) => Some("Opened by an agent".into()),
            None => None,
        };
        Some(json!({ "tab": id, "title": t.display_title(), "host": host, "space": space, "active": self.state.active_tab() == Some(id), "playing": t.playing }))
    }

    /// The Task Manager's list (page op `list_tasks`): processes with their figures and the tabs in each, tabs this
    /// system can't measure, and sleeping tabs.
    pub(super) fn task_list(&self) -> Value {
        let me = std::process::id();
        let mut rows: BTreeMap<u32, Row> = BTreeMap::new();
        let mut unmeasured: Vec<Value> = Vec::new();
        rows.insert(me, Row { kind: "valerie", name: "Valerie".into(), tabs: vec![] });
        let mut own: Vec<u32> = Vec::new();
        let mut ids: Vec<TabId> = self.views.keys().copied().collect();
        ids.sort_by_key(|id| std::cmp::Reverse(self.state.tabs.get(id).map_or(0, |t| t.last_active)));
        for id in ids {
            let Some(tab) = self.tm_tab(id) else { continue };
            match sys::web_pid(&self.views[&id]) {
                Some(pid) => rows.entry(pid).or_insert_with(|| Row { kind: "tabs", name: String::new(), tabs: vec![] }).tabs.push(tab),
                None => unmeasured.push(tab),
            }
        }
        for l in self.littles.values() {
            let pages = std::iter::once((&l.view, &l.url, &l.title)).chain(l.others.iter().map(|p| (&p.view, &p.url, &p.title)));
            for (v, url, title) in pages {
                let tab = json!({ "title": if title.is_empty() { url } else { title }, "host": site_of(url), "space": self.state.spaces.get(l.space).map(|s| &s.name), "little": true });
                match sys::web_pid(v) {
                    Some(pid) => rows.entry(pid).or_insert_with(|| Row { kind: "tabs", name: String::new(), tabs: vec![] }).tabs.push(tab),
                    None => unmeasured.push(tab),
                }
            }
        }
        for (what, v) in self.tm_own_views() {
            if let Some(pid) = sys::web_pid(v) {
                own.push(pid);
                let r = rows.entry(pid).or_insert_with(|| Row { kind: "own", name: String::new(), tabs: vec![] });
                if r.kind == "tabs" {
                    r.kind = "mixed";
                }
                if !r.name.split(", ").any(|n| n == what) {
                    r.name = if r.name.is_empty() { what.to_string() } else { format!("{}, {what}", r.name) };
                }
            }
        }
        let views: Vec<&WebView> = self.views.values().chain(self.tm_own_views().into_iter().map(|(_, v)| v)).collect();
        for (pid, name) in sys::helpers(&views) {
            rows.entry(pid).or_insert_with(|| Row { kind: "helper", name, tabs: vec![] });
        }
        let usage: HashMap<u32, Usage> = rows.keys().map(|&p| (p, sys::usage(p))).collect();
        let cpu = cpu_shares(&usage);
        let ending = cfg!(target_os = "macos") && self.config.task_manager.end_process;
        let (mut memory, mut total_cpu, mut measured) = (0u64, 0f64, false);
        let processes: Vec<Value> = rows
            .into_iter()
            .map(|(pid, r)| {
                let u = usage.get(&pid).copied().unwrap_or_default();
                memory += u.memory.unwrap_or(0);
                total_cpu += cpu.get(&pid).copied().unwrap_or(0.0);
                measured |= u.memory.is_some();
                let can_end = ending && r.kind == "tabs" && pid != me && !own.contains(&pid);
                json!({ "pid": pid, "kind": r.kind, "name": r.name, "memory": u.memory, "cpu": cpu.get(&pid), "tabs": r.tabs, "can_end": can_end })
            })
            .collect();
        let mut asleep: Vec<(u64, Value)> = self.state.tabs.values().filter(|t| !self.views.contains_key(&t.id)).filter_map(|t| Some((t.last_active, self.tm_tab(t.id)?))).collect();
        asleep.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        json!({
            "system": std::env::consts::OS,
            "per_tab": sys::PER_TAB,
            "processes": processes,
            "unmeasured": unmeasured,
            "asleep": asleep.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
            "total": { "memory": measured.then_some(memory), "cpu": (!cpu.is_empty()).then_some(total_cpu) },
        })
    }

    /// End the web process a tab is in (page op `end_tab_process` { tab, pid }): Mac only, only a process that holds
    /// nothing but pages (never the sidebar's or a helper), and only the one the page showed. Every page in it reloads
    /// when it's next shown, as after any crash.
    pub(super) fn end_tab_process(&mut self, args: &Value) -> Result<Value, String> {
        if !cfg!(target_os = "macos") {
            return Err("ending a page's process only works on a Mac".into());
        }
        if !self.config.task_manager.end_process {
            return Err("task_manager.end_process is off".into());
        }
        let tab: TabId = serde_json::from_value(args["tab"].clone()).map_err(|_| "which tab?".to_string())?;
        let shown = args["pid"].as_u64().unwrap_or(0) as u32;
        let view = self.views.get(&tab).ok_or("that tab is asleep, so it has no process to end")?;
        let pid = sys::web_pid(view).ok_or("that tab has no process right now")?;
        if pid != shown {
            return Err("that tab moved to another process; look again".into());
        }
        if pid == std::process::id() || self.tm_own_views().iter().any(|(_, v)| sys::web_pid(v) == Some(pid)) {
            return Err("that process also draws Valerie's own window, so it can't be ended here".into());
        }
        let ended: Vec<TabId> = self.views.iter().filter(|(_, v)| sys::web_pid(v) == Some(pid)).map(|(t, _)| *t).collect();
        let tabs = ended.len();
        // Their pages sleep instead of reloading at once (see UserEvent::PageCrashed in app.rs).
        let at = std::time::Instant::now();
        self.ended_by_user.extend(ended.iter().map(|t| (*t, at)));
        if let Err(e) = sys::end_web_process(pid) {
            for t in &ended {
                self.ended_by_user.remove(t);
            }
            return Err(e);
        }
        crate::log::line("tasks", format!("ended web process {pid} ({tabs} tabs) from the Task Manager"));
        Ok(json!({ "ended": pid, "tabs": tabs }))
    }
}

#[cfg(target_os = "macos")]
mod sys {
    use super::Usage;
    use objc2::runtime::AnyObject;
    use objc2::{msg_send, sel};
    use std::ffi::c_void;
    use wry::{WebView, WebViewExtMacOS};

    pub const PER_TAB: bool = true;

    extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut c_void) -> i32;
        fn proc_name(pid: i32, buffer: *mut c_void, size: u32) -> i32;
        fn proc_listallpids(buffer: *mut c_void, size: i32) -> i32;
        fn mach_timebase_info(info: *mut [u32; 2]) -> i32;
        fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
        fn kill(pid: i32, signal: i32) -> i32;
    }

    fn ptr(wv: &WebView) -> *mut AnyObject {
        let v = wv.webview();
        objc2::rc::Retained::as_ptr(&v) as *mut AnyObject
    }

    /// WebKit's private process ids on a WKWebView (0 when it has none, or this WebKit doesn't say).
    fn ids(wv: &WebView) -> [u32; 3] {
        let v = ptr(wv);
        unsafe {
            let web: i32 = if msg_send![v, respondsToSelector: sel!(_webProcessIdentifier)] { msg_send![v, _webProcessIdentifier] } else { 0 };
            let net: i32 = if msg_send![v, respondsToSelector: sel!(_networkProcessIdentifier)] { msg_send![v, _networkProcessIdentifier] } else { 0 };
            let gpu: i32 = if msg_send![v, respondsToSelector: sel!(_gpuProcessIdentifier)] { msg_send![v, _gpuProcessIdentifier] } else { 0 };
            [web.max(0) as u32, net.max(0) as u32, gpu.max(0) as u32]
        }
    }

    pub fn web_pid(wv: &WebView) -> Option<u32> {
        Some(ids(wv)[0]).filter(|p| *p > 0)
    }

    fn name_of(pid: u32) -> String {
        let mut name = [0u8; 64];
        let n = unsafe { proc_name(pid as i32, name.as_mut_ptr().cast(), name.len() as u32) };
        String::from_utf8_lossy(&name[..n.max(0) as usize]).into_owned()
    }

    fn label(name: &str) -> String {
        match name.strip_prefix("com.apple.WebKit.").unwrap_or(name) {
            "Networking" => "Networking".into(),
            "GPU" => "Graphics (GPU)".into(),
            n if n.starts_with("WebContent") => "Spare web process".into(),
            n => n.to_string(),
        }
    }

    /// The networking and graphics processes the views name, and, when Valerie is its own app (not run from a terminal,
    /// whose other programs would count too), any other WebKit process it's responsible for (a spare one kept ready).
    pub fn helpers(views: &[&WebView]) -> Vec<(u32, String)> {
        let mut out: Vec<(u32, String)> = Vec::new();
        for v in views {
            let [_, net, gpu] = ids(v);
            for (pid, what) in [(net, "Networking"), (gpu, "Graphics (GPU)")] {
                if pid > 0 && !out.iter().any(|(p, _)| *p == pid) {
                    out.push((pid, what.to_string()));
                }
            }
        }
        let symbol = unsafe { dlsym(-2isize as *mut c_void, c"responsibility_get_pid_responsible_for_pid".as_ptr()) };
        if symbol.is_null() {
            return out;
        }
        let responsible: extern "C" fn(i32) -> i32 = unsafe { std::mem::transmute(symbol) };
        let me = std::process::id() as i32;
        if responsible(me) != me {
            return out;
        }
        let mut pids = vec![0i32; 8192];
        let found = unsafe { proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * 4) as i32) };
        for pid in pids.into_iter().take(found.max(0) as usize).filter(|p| *p > 0) {
            let name = name_of(pid as u32);
            if name.starts_with("com.apple.WebKit.") && responsible(pid) == me && !out.iter().any(|(p, _)| *p == pid as u32) {
                out.push((pid as u32, label(&name)));
            }
        }
        out
    }

    /// Memory as Activity Monitor counts it (the footprint) and CPU time, from proc_pid_rusage.
    pub fn usage(pid: u32) -> Usage {
        let mut ru = [0u64; 48];
        if unsafe { proc_pid_rusage(pid as i32, 4, ru.as_mut_ptr().cast()) } != 0 {
            return Usage::default();
        }
        let mut tb = [0u32; 2];
        unsafe { mach_timebase_info(&mut tb) };
        let tick = if tb[1] == 0 { 1.0 } else { tb[0] as f64 / tb[1] as f64 };
        // rusage_info_v4: a 16-byte uuid, then user_time, system_time, …, phys_footprint (the 8th number).
        Usage { memory: Some(ru[2 + 7]), cpu_ns: Some(((ru[2] + ru[3]) as f64 * tick) as u64) }
    }

    pub fn end_web_process(pid: u32) -> Result<(), String> {
        if !name_of(pid).starts_with("com.apple.WebKit.WebContent") {
            return Err("that isn't a web page process".into());
        }
        if unsafe { kill(pid as i32, 9) } != 0 {
            return Err(format!("couldn't end process {pid}: {}", std::io::Error::last_os_error()));
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod sys {
    use super::Usage;
    use std::ffi::c_void;
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Environment8, ICoreWebView2_2, COREWEBVIEW2_PROCESS_KIND};
    use windows::core::Interface;
    use wry::{WebView, WebViewExtWindows};

    /// WebView2 doesn't say cheaply which renderer draws which tab.
    pub const PER_TAB: bool = false;

    #[repr(C)]
    #[derive(Default)]
    struct MemoryCounters {
        cb: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        quota: [usize; 4],
        pagefile: usize,
        peak_pagefile: usize,
        private: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn K32GetProcessMemoryInfo(handle: *mut c_void, counters: *mut MemoryCounters, cb: u32) -> i32;
        fn GetProcessTimes(handle: *mut c_void, created: *mut [u32; 2], exited: *mut [u32; 2], kernel: *mut [u32; 2], user: *mut [u32; 2]) -> i32;
    }

    pub fn web_pid(_wv: &WebView) -> Option<u32> {
        None
    }

    /// Every process of each view's WebView2 environment, by kind.
    pub fn helpers(views: &[&WebView]) -> Vec<(u32, String)> {
        let mut out: Vec<(u32, String)> = Vec::new();
        let mut seen_env = 0;
        for v in views {
            let infos = unsafe { v.webview().cast::<ICoreWebView2_2>().and_then(|w| w.Environment()).and_then(|e| e.cast::<ICoreWebView2Environment8>()).and_then(|e| e.GetProcessInfos()) };
            let Ok(infos) = infos else { continue };
            let mut count = 0u32;
            if unsafe { infos.Count(&mut count) }.is_err() {
                continue;
            }
            for i in 0..count {
                let Ok(info) = (unsafe { infos.GetValueAtIndex(i) }) else { continue };
                let (mut pid, mut kind) = (0i32, COREWEBVIEW2_PROCESS_KIND::default());
                if unsafe { info.ProcessId(&mut pid) }.is_err() || unsafe { info.Kind(&mut kind) }.is_err() || pid <= 0 {
                    continue;
                }
                let name = match kind.0 {
                    0 => "WebView2 browser process",
                    1 => "Web content",
                    2 => "Utility (networking, audio, …)",
                    3 => "Sandbox helper",
                    4 => "Graphics (GPU)",
                    _ => "Plug-in",
                };
                if !out.iter().any(|(p, _)| *p == pid as u32) {
                    out.push((pid as u32, name.to_string()));
                }
            }
            seen_env += 1;
            // Views share an environment per profile; a few looks cover them all.
            if seen_env >= 4 {
                break;
            }
        }
        out
    }

    /// Private memory (what Windows' Task Manager calls the commit size) and CPU time.
    pub fn usage(pid: u32) -> Usage {
        const QUERY_LIMITED: u32 = 0x1000;
        let h = unsafe { OpenProcess(QUERY_LIMITED, 0, pid) };
        if h.is_null() {
            return Usage::default();
        }
        let mut m = MemoryCounters { cb: std::mem::size_of::<MemoryCounters>() as u32, ..Default::default() };
        let memory = (unsafe { K32GetProcessMemoryInfo(h, &mut m, m.cb) } != 0).then_some(m.private as u64);
        let (mut a, mut b, mut k, mut u) = ([0u32; 2], [0u32; 2], [0u32; 2], [0u32; 2]);
        let ft = |t: [u32; 2]| ((t[1] as u64) << 32 | t[0] as u64) * 100;
        let cpu_ns = (unsafe { GetProcessTimes(h, &mut a, &mut b, &mut k, &mut u) } != 0).then(|| ft(k) + ft(u));
        unsafe { CloseHandle(h) };
        Usage { memory, cpu_ns }
    }

    pub fn end_web_process(_pid: u32) -> Result<(), String> {
        Err("ending a page's process only works on a Mac".into())
    }
}

#[cfg(target_os = "linux")]
mod sys {
    use super::Usage;
    use wry::WebView;

    /// WebKitGTK doesn't say which web process draws which tab.
    pub const PER_TAB: bool = false;

    extern "C" {
        fn sysconf(name: i32) -> i64;
    }

    pub fn web_pid(_wv: &WebView) -> Option<u32> {
        None
    }

    /// WebKit's processes under Valerie (its children, or theirs when sandboxed).
    pub fn helpers(_views: &[&WebView]) -> Vec<(u32, String)> {
        let me = std::process::id();
        let procs: Vec<(u32, u32, String)> = std::fs::read_dir("/proc")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
            .filter_map(|pid| {
                let (ppid, comm) = super::parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)?.0;
                Some((pid, ppid, comm))
            })
            .collect();
        let ours = |mut pid: u32| {
            for _ in 0..4 {
                match procs.iter().find(|p| p.0 == pid) {
                    Some(p) if p.1 == me => return true,
                    Some(p) => pid = p.1,
                    None => return false,
                }
            }
            false
        };
        procs
            .iter()
            .filter(|(pid, _, comm)| comm.starts_with("WebKit") && ours(*pid))
            .map(|(pid, _, comm)| {
                let name = match comm.as_str() {
                    c if c.starts_with("WebKitWebProc") => "Web content",
                    c if c.starts_with("WebKitNetwork") => "Networking",
                    c if c.starts_with("WebKitGPU") => "Graphics (GPU)",
                    _ => "WebKit helper",
                };
                (*pid, name.to_string())
            })
            .collect()
    }

    /// Resident memory from /proc/<pid>/statm and CPU time from /proc/<pid>/stat.
    pub fn usage(pid: u32) -> Usage {
        const CLK_TCK: i32 = 2;
        const PAGESIZE: i32 = 30;
        let (page, tick) = unsafe { (sysconf(PAGESIZE).max(1) as u64, sysconf(CLK_TCK).max(1) as u64) };
        let memory = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok()).map(|p| p * page);
        let cpu_ns = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok().and_then(|s| super::parse_stat(&s)).map(|(_, ticks)| ticks * 1_000_000_000 / tick);
        Usage { memory, cpu_ns }
    }

    pub fn end_web_process(_pid: u32) -> Result<(), String> {
        Err("ending a page's process only works on a Mac".into())
    }
}

/// /proc/<pid>/stat: ((parent pid, command name), user + system CPU ticks). The name is in brackets and may hold spaces.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_stat(s: &str) -> Option<((u32, String), u64)> {
    let (open, close) = (s.find('(')?, s.rfind(')')?);
    let comm = s.get(open + 1..close)?.to_string();
    let rest: Vec<&str> = s.get(close + 1..)?.split_whitespace().collect();
    // After the name: state, ppid, … utime is the 14th field of the line, so the 12th after the name.
    let ppid = rest.get(1)?.parse().ok()?;
    let ticks = rest.get(11)?.parse::<u64>().ok()? + rest.get(12)?.parse::<u64>().ok()?;
    Some(((ppid, comm), ticks))
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_proc_stat() {
        let line = "4242 (WebKitWebProces) S 4200 4242 4200 0 -1 4194560 1 0 0 0 150 30 0 0 20 0 9 0 1 2 3";
        assert_eq!(super::parse_stat(line), Some(((4200, "WebKitWebProces".to_string()), 180)));
        assert_eq!(super::parse_stat("7 (a b) c) R 1 7 7 0 -1 0 0 0 0 0 5 6 0"), Some(((1, "a b) c".to_string()), 11)));
        assert_eq!(super::parse_stat("garbage"), None);
    }

    #[test]
    fn cpu_needs_two_looks() {
        use std::collections::HashMap;
        let mut now: HashMap<u32, super::Usage> = HashMap::new();
        now.insert(1, super::Usage { memory: Some(1), cpu_ns: Some(0) });
        let _ = super::cpu_shares(&now);
        std::thread::sleep(std::time::Duration::from_millis(150));
        now.insert(1, super::Usage { memory: Some(1), cpu_ns: Some(75_000_000) });
        let share = super::cpu_shares(&now)[&1];
        assert!(share > 10.0 && share < 60.0, "{share}");
    }
}
