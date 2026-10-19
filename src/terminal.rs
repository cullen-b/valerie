//! The terminal panel's shells. Each runs in a real pseudo-terminal (colours, vim, Ctrl-C work); what it prints goes to
//! ui/terminal.html, where xterm.js draws it.

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub struct Term {
    pub id: u32,
    pub title: String,
    pub cwd: PathBuf,
    /// The coding chat this shell belongs to (it opens in that chat's project).
    pub chat: Option<String>,
    /// Printed text not yet handed to the panel; the reader thread fills it, the event loop empties it.
    pub pending: Arc<Mutex<String>>,
    /// A command to type once the shell has printed its prompt.
    pub queued: Option<String>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

/// The shell to run: terminal_panel.shell, else the user's login shell.
pub fn shell(configured: &str) -> String {
    if !configured.trim().is_empty() {
        return configured.trim().to_string();
    }
    #[cfg(windows)]
    {
        "powershell.exe".to_string()
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| if cfg!(target_os = "macos") { "/bin/zsh".into() } else { "/bin/bash".into() })
    }
}

/// Where a cut between reads would split a UTF-8 character: hand over only whole characters.
fn whole_chars(carry: &mut Vec<u8>) -> String {
    let cut = match std::str::from_utf8(carry) {
        Ok(_) => carry.len(),
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => carry.len(),
    };
    let rest = carry.split_off(cut);
    let text = String::from_utf8_lossy(carry).into_owned();
    *carry = rest;
    text
}

impl Term {
    /// Start `shell` in `cwd`. `on_output` runs (on the reader thread) when output arrives and nothing was pending;
    /// `on_exit` when the shell ends.
    pub fn spawn(
        id: u32,
        shell: &str,
        cwd: &Path,
        size: (u16, u16),
        on_output: impl Fn() + Send + 'static,
        on_exit: impl FnOnce() + Send + 'static,
    ) -> Result<Term> {
        let pair = native_pty_system()
            .openpty(PtySize { rows: size.1.max(2), cols: size.0.max(10), pixel_width: 0, pixel_height: 0 })
            .context("opening a terminal")?;
        let mut cmd = CommandBuilder::new(shell);
        let name = Path::new(shell).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // A login shell reads the user's profile, so PATH is the one they know (apps opened from the Dock get a bare one).
        if matches!(name.as_str(), "zsh" | "bash" | "fish" | "sh" | "ksh" | "tcsh" | "nu") {
            cmd.arg("-l");
        }
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "Valerie");
        if std::env::var_os("LANG").is_none() {
            cmd.env("LANG", "en_US.UTF-8");
        }
        let mut child = pair.slave.spawn_command(cmd).with_context(|| format!("starting {shell}"))?;
        drop(pair.slave);
        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let pending = Arc::new(Mutex::new(String::new()));
        let fill = pending.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            let mut carry = Vec::new();
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                carry.extend_from_slice(&buf[..n]);
                let text = whole_chars(&mut carry);
                if text.is_empty() {
                    continue;
                }
                let first = {
                    let mut p = fill.lock().unwrap();
                    let was = p.is_empty();
                    p.push_str(&text);
                    was
                };
                if first {
                    on_output();
                }
                // Don't run far ahead of the panel when something prints without end.
                while fill.lock().map(|p| p.len() > 4 << 20).unwrap_or(false) {
                    std::thread::sleep(std::time::Duration::from_millis(15));
                }
            }
        });
        std::thread::spawn(move || {
            let _ = child.wait();
            on_exit();
        });
        let title = cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| cwd.display().to_string());
        Ok(Term { id, title, cwd: cwd.to_path_buf(), chat: None, pending, queued: None, writer, master: pair.master, killer })
    }

    /// What the shell printed since the last call.
    pub fn take_output(&self) -> String {
        std::mem::take(&mut *self.pending.lock().unwrap())
    }

    pub fn write(&mut self, data: &[u8]) {
        let _ = self.writer.write_all(data);
        let _ = self.writer.flush();
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.master.resize(PtySize { rows: rows.max(2), cols: cols.max(10), pixel_width: 0, pixel_height: 0 });
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        let _ = self.killer.kill();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn whole_chars_keeps_a_split_character_for_later() {
        let mut carry = "é".as_bytes()[..1].to_vec();
        assert_eq!(super::whole_chars(&mut carry), "");
        carry.extend_from_slice(&"é".as_bytes()[1..]);
        carry.extend_from_slice(b"x");
        assert_eq!(super::whole_chars(&mut carry), "éx");
        assert!(carry.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_shell_runs_in_a_real_terminal() {
        let (tx, rx) = std::sync::mpsc::channel();
        let dir = std::env::temp_dir();
        let mut t = super::Term::spawn(1, "/bin/sh", &dir, (80, 24), move || { let _ = tx.send(()); }, || {}).unwrap();
        t.write(b"test -t 0 && echo TTY-$((6*7))\n");
        let mut out = String::new();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !out.contains("TTY-42") && std::time::Instant::now() < until {
            let _ = rx.recv_timeout(std::time::Duration::from_millis(200));
            out.push_str(&t.take_output());
        }
        assert!(out.contains("TTY-42"), "{out:?}");
    }
}
