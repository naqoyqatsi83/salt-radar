//! Copy to clipboard (tmux / OSC 52 and X11 via xclip, including finding the
//! X11 session) and the save-to-file prompt.

use super::*;

impl App {
    pub(crate) fn tmux_socket_path() -> Option<String> {
        // Check TMUX env var first: "tmux/0" -> socket_dir = /tmp/tmux-<client_pid>
        // TMUX format: "/tmp/tmux-1000/default,0" -> parse out the socket path
        if let Ok(tmux_env) = std::env::var("TMUX") {
            let path = tmux_env.split(',').next().unwrap_or("").to_string();
            if !path.is_empty() && std::path::Path::new(&path).exists() {
                return Some(path);
            }
        }
        // Fallback: find any /tmp/tmux-*/default socket
        if let Ok(entries) = std::fs::read_dir("/tmp") {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("tmux-") {
                    let sock = format!("/tmp/{}/default", name);
                    if std::path::Path::new(&sock).exists() {
                        return Some(sock);
                    }
                }
            }
        }
        None
    }

    /// Run a command with piped stdin and kill it after `timeout_secs`.
    /// Returns (success, stderr_output).
    pub(crate) fn run_cmd_with_stdin(
        cmd: &mut std::process::Command,
        text: &str,
        timeout_secs: u64,
    ) -> (bool, String) {
        use std::sync::mpsc;
        use std::time::Duration;

        let mut child = match cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return (false, String::new()),
        };

        // Write stdin in background thread
        let text_owned = text.to_string();
        if let Some(mut s) = child.stdin.take() {
            std::thread::spawn(move || {
                let _ = s.write_all(text_owned.as_bytes());
                // drop closes stdin -> EOF to child
            });
        }

        // Wait for child in background thread with timeout
        let (tx, rx) = mpsc::channel();
        let pid = child.id();
        std::thread::spawn(move || {
            let status = child.wait();
            let mut err = String::new();
            let _ = child.stderr.take().map(|mut s| s.read_to_string(&mut err));
            let success = status.ok().map(|s| s.success()).unwrap_or(false);
            let _ = tx.send((success, err));
        });

        match rx.recv_timeout(Duration::from_secs(timeout_secs)) {
            Ok((ok, err)) => (ok, err),
            Err(_) => {
                // Timed out — kill child
                let _ = std::process::Command::new("kill")
                    .args(["-9", &pid.to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                (false, format!("timeout>{}s", timeout_secs))
            }
        }
    }

    /// Run a command and capture its stdout (with timeout).
    pub(crate) fn run_cmd_with_output(cmd: &mut std::process::Command, timeout_secs: u64) -> Option<String> {
        use std::sync::mpsc;
        use std::time::Duration;

        let mut child = match cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return None,
        };

        let (tx, rx) = mpsc::channel();
        let pid = child.id();
        std::thread::spawn(move || {
            let mut output = String::new();
            let _ = child.stdout.take().map(|mut s| s.read_to_string(&mut output));
            let _ = child.wait();
            let _ = tx.send(output);
        });

        match rx.recv_timeout(Duration::from_secs(timeout_secs)) {
            Ok(out) => Some(out),
            Err(_) => {
                let _ = std::process::Command::new("kill")
                    .args(["-9", &pid.to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                None
            }
        }
    }

    /// Read a tmux global environment variable (inherited from the process
    /// that started the tmux server, which is the original SSH session).
    pub(crate) fn tmux_global_env(name: &str) -> Option<String> {
        let socket = Self::tmux_socket_path();
        let mut cmd = std::process::Command::new("tmux");
        if let Some(s) = &socket {
            cmd.args(["-S", s]);
        }
        cmd.args(["show-environment", "-g", name]);

        let output = Self::run_cmd_with_output(&mut cmd, 3)?;
        let line = output.trim();
        if line.is_empty() || line.starts_with('-') {
            return None;
        }
        line.strip_prefix(&format!("{}=", name)).map(|v| v.to_string())
    }

    /// Resolve DISPLAY and XAUTHORITY. Checks:
    /// 1. Process environment vars
    /// 2. tmux global environment (original SSH session)
    /// 3. X11 socket owner's ~/.Xauthority
    /// 4. X11 socket owner's /run/user/<uid>/xauth (pam_xauth on Ubuntu)
    pub(crate) fn resolve_x11_env() -> (Option<String>, Option<String>) {
        let display = std::env::var("DISPLAY").ok()
            .or_else(|| Self::tmux_global_env("DISPLAY"))
            .filter(|d| !d.is_empty());
        let xauth = std::env::var("XAUTHORITY").ok()
            .or_else(|| Self::tmux_global_env("XAUTHORITY"))
            .filter(|d| !d.is_empty())
            .or_else(|| {
                if let Some(ref disp) = display {
                    Self::find_xauthority(&Self::detect_x11_user(disp))
                } else { None }
            });
        (display, xauth)
    }

    /// Find the Xauthority file for a user. Checks (in order):
    /// 1. XAUTHORITY env from running processes of that user (/proc/<pid>/environ)
    /// 2. ~/.Xauthority from the user's passwd home directory
    /// 3. /run/user/<uid>/xauth* (globbing for pam_xauth random filenames)
    pub(crate) fn find_xauthority(username: &str) -> Option<String> {
        // Most reliable: read XAUTHORITY directly from a running process of
        // the target user. This captures the actual pam_xauth-created path
        // (e.g. /run/user/<uid>/xauth_XXXXXXXXXX).
        if let Some(path) = Self::xauthority_from_process(username) {
            return Some(path);
        }

        let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
        for line in passwd.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.first() != Some(&username) { continue; }
            let home = parts.get(5)?;
            let uid: u32 = parts.get(2)?.parse().ok()?;

            // Check ~/.Xauthority
            let dot_path = format!("{}/.Xauthority", home);
            if std::path::Path::new(&dot_path).exists() {
                return Some(dot_path);
            }

            // Glob for /run/user/<uid>/xauth* (pam_xauth creates random names)
            let run_dir = format!("/run/user/{}", uid);
            if let Ok(entries) = std::fs::read_dir(&run_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name == "xauth" || name.starts_with("xauth") {
                        let path = entry.path().to_string_lossy().to_string();
                        if std::path::Path::new(&path).exists() {
                            return Some(path);
                        }
                    }
                }
            }
            break;
        }
        None
    }

    /// Try to read XAUTHORITY from /proc/<pid>/environ for any running
    /// process of the given user. Tries newest PIDs first.
    pub(crate) fn xauthority_from_process(username: &str) -> Option<String> {
        let out = std::process::Command::new("pgrep")
            .args(["-u", username]).output().ok()?;
        let mut pids: Vec<u32> = String::from_utf8(out.stdout).ok()
            .map(|s| s.lines().filter_map(|l| l.trim().parse().ok()).collect())
            .unwrap_or_default();
        // Newest PID first — more likely to be from the current X11 session
        pids.sort_by(|a, b| b.cmp(a));
        for pid in pids {
            if let Ok(env) = std::fs::read_to_string(format!("/proc/{}/environ", pid)) {
                for entry in env.split('\0') {
                    if let Some(val) = entry.strip_prefix("XAUTHORITY=") {
                        if !val.is_empty() && std::path::Path::new(val).exists() {
                            return Some(val.to_string());
                        }
                    }
                }
            }
        }
        None
    }

    /// Dump comprehensive X11 diagnostics to a file so the user can share
    /// what's happening on their system.
    pub(crate) fn debug_x11(display: &Option<String>, xauth: &Option<String>, x11_user: &str, debug_path: &str) {
        use std::fmt::Write;
        let mut d = String::new();
        writeln!(d, "=== salt-radar X11 debug ===").ok();
        writeln!(d, "process DISPLAY env: {:?}", std::env::var("DISPLAY").ok()).ok();
        writeln!(d, "process XAUTHORITY env: {:?}", std::env::var("XAUTHORITY").ok()).ok();
        writeln!(d, "process USER: {:?}", std::env::var("USER").ok()).ok();
        writeln!(d, "process LOGNAME: {:?}", std::env::var("LOGNAME").ok()).ok();
        writeln!(d, "current_user(): {}", Self::current_user()).ok();
        if let Some(tmux) = Self::tmux_socket_path() {
            writeln!(d, "tmux socket: {}", tmux).ok();
            writeln!(d, "tmux DISPLAY: {:?}", Self::tmux_global_env("DISPLAY")).ok();
            writeln!(d, "tmux XAUTHORITY: {:?}", Self::tmux_global_env("XAUTHORITY")).ok();
        } else {
            writeln!(d, "tmux: not detected").ok();
        }
        writeln!(d, "resolved DISPLAY: {:?}", display).ok();
        writeln!(d, "resolved XAUTHORITY: {:?}", xauth).ok();
        writeln!(d, "detected X11 user: {}", x11_user).ok();

        // Dump /tmp/.X11-unix/ content
        writeln!(d, "\n--- /tmp/.X11-unix/ ---").ok();
        if let Ok(entries) = std::fs::read_dir("/tmp/.X11-unix") {
            for e in entries.filter_map(|e| e.ok()) {
                let meta = e.metadata().ok();
                let uid = meta.as_ref().map(|m| m.uid()).unwrap_or(0);
                // Resolve uid to username
                let user = Self::uid_to_username(uid);
                writeln!(d, "  {} owner_uid={} ({})", e.file_name().to_string_lossy(), uid, user).ok();
            }
        } else {
            writeln!(d, "  (cannot read)").ok();
        }

        // Try to read the Xauthority file
        if let Some(ref xa) = xauth {
            writeln!(d, "\n--- XAUTHORITY file: {} ---", xa).ok();
            let meta = std::fs::metadata(xa).ok();
            if let Some(ref m) = meta {
                writeln!(d, "  size: {} bytes", m.len()).ok();
                writeln!(d, "  readable: {:?}", std::fs::read(xa).ok().map(|v| v.len())).ok();
                // Try xauth list on it
                let out = std::process::Command::new("xauth")
                    .args(["-f", xa, "list"]).output().ok();
                if let Some(ref o) = out {
                    let stdout = String::from_utf8_lossy(&o.stdout);
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    writeln!(d, "  xauth list: stdout=[{}] stderr=[{}]", stdout.trim(), stderr.trim()).ok();
                } else {
                    writeln!(d, "  xauth list: (failed to run)").ok();
                }
            } else {
                writeln!(d, "  (file not found or no metadata)").ok();
            }
        } else {
            writeln!(d, "\n--- XAUTHORITY: not resolved ---").ok();
        }

        // Check known Xauthority locations
        writeln!(d, "\n--- common Xauthority paths ---").ok();
        for path in &[
            "/root/.Xauthority",
            &format!("/home/{}/.Xauthority", x11_user),
            "/run/user/0/xauth",
        ] {
            let exists = std::path::Path::new(path).exists();
            writeln!(d, "  {} exists={}", path, exists).ok();
        }

        // Check /run/user/<uid>/ for xauth files
        if let Ok(entries) = std::fs::read_dir("/run/user") {
            for e in entries.filter_map(|e| e.ok()) {
                let uid_str = e.file_name();
                let uid: u32 = uid_str.to_string_lossy().parse().unwrap_or(u32::MAX);
                let user = Self::uid_to_username(uid);
                writeln!(d, "  /run/user/{} ({})", uid_str.to_string_lossy(), user).ok();
                if let Ok(files) = std::fs::read_dir(e.path()) {
                    for f in files.filter_map(|f| f.ok()) {
                        let name = f.file_name().to_string_lossy().to_string();
                        if name.contains("xauth") {
                            let meta = f.metadata().ok();
                            let size = meta.map(|m| m.len()).unwrap_or(0);
                            writeln!(d, "    {} ({} bytes)", name, size).ok();
                        }
                    }
                }
            }
        }

        // Show xclip version (xclip outputs to stderr)
        let xclip_v = std::process::Command::new("xclip").arg("-version").output().ok()
            .map(|o| {
                let mut s = String::from_utf8_lossy(&o.stdout).to_string();
                if s.trim().is_empty() {
                    s = String::from_utf8_lossy(&o.stderr).to_string();
                }
                s
            });
        writeln!(d, "\n--- xclip ---").ok();
        writeln!(d, "  version: {:?}", xclip_v.unwrap_or_default().trim()).ok();

        // Show tmux socket owner as detected user
        writeln!(d, "\n--- tmux-derived user ---").ok();
        let tmux_user = display.as_ref().map(|d| Self::detect_x11_user(d)).unwrap_or_default();
        writeln!(d, "  detect_x11_user: {}", tmux_user).ok();
        writeln!(d, "  find_xauthority for this user: {:?}", Self::find_xauthority(&tmux_user)).ok();

        let _ = std::fs::write(debug_path, &d);
    }

    pub(crate) fn uid_to_username(uid: u32) -> String {
        if let Ok(passwd) = std::fs::read_to_string("/etc/passwd") {
            for line in passwd.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if let Some(u) = parts.get(2).and_then(|s| s.parse::<u32>().ok()) {
                    if u == uid {
                        if let Some(name) = parts.first() {
                            return name.to_string();
                        }
                    }
                }
            }
        }
        format!("uid:{}", uid)
    }

    /// The terminals of this process and its ancestors, nearest first:
    /// `sudo` with `use_pty` (the default on Ubuntu and RHEL) runs us on a
    /// pty of its own, so the tmux pane's tty is only on sudo's parent.
    fn own_and_ancestor_ttys() -> Vec<String> {
        let mut ttys = Vec::new();
        let mut pid = std::process::id();
        for _ in 0..16 {
            if let Ok(tty) = std::fs::read_link(format!("/proc/{}/fd/0", pid)) {
                let tty = tty.to_string_lossy().to_string();
                if tty.starts_with("/dev/") && !ttys.contains(&tty) {
                    ttys.push(tty);
                }
            }
            let ppid = std::fs::read_to_string(format!("/proc/{}/stat", pid)).ok()
                .and_then(|s| s.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse::<u32>().ok());
            match ppid {
                Some(p) if p > 1 => pid = p,
                _ => break,
            }
        }
        ttys
    }

    /// The tmux socket and clients showing the pane salt-radar runs in,
    /// matched by terminal rather than `$TMUX`, which `sudo` drops. Every
    /// server under /tmp/tmux-* is asked, since root may see several.
    pub(crate) fn tmux_own_pane() -> Option<(String, Vec<String>)> {
        let ttys = Self::own_and_ancestor_ttys();
        let mut sockets: Vec<String> = std::env::var("TMUX").ok()
            .and_then(|t| t.split(',').next().map(str::to_string))
            .into_iter()
            .collect();
        if let Ok(entries) = std::fs::read_dir("/tmp") {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().to_string();
                let sock = format!("/tmp/{}/default", name);
                if name.starts_with("tmux-") && !sockets.contains(&sock) && std::path::Path::new(&sock).exists() {
                    sockets.push(sock);
                }
            }
        }
        for socket in sockets {
            let Some(panes) = Self::run_cmd_with_output(
                std::process::Command::new("tmux").args(["-S", &socket, "list-panes", "-a", "-F", "#{pane_tty} #{session_id}"]),
                3,
            ) else { continue };
            let session = ttys.iter().find_map(|tty| {
                panes.lines().find_map(|l| l.strip_prefix(tty.as_str())?.strip_prefix(' '))
            });
            let Some(session) = session else { continue };
            let clients = Self::run_cmd_with_output(
                std::process::Command::new("tmux").args(["-S", &socket, "list-clients", "-t", session, "-F", "#{client_tty}"]),
                3,
            )?;
            return Some((socket, clients.lines().filter(|l| !l.is_empty()).map(str::to_string).collect()));
        }
        None
    }

    /// The OSC 52 "set clipboard" sequence, optionally wrapped in tmux's
    /// passthrough for a tmux too old for `load-buffer -w`.
    pub(crate) fn osc52_sequence(text: &str, tmux_passthrough: bool) -> String {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
        if tmux_passthrough {
            format!("\x1bPtmux;\x1b\x1b]52;c;{}\x07\x1b\\", b64)
        } else {
            format!("\x1b]52;c;{}\x07", b64)
        }
    }

    /// Runs one xclip invocation (direct or through `su`) with the copy file
    /// as stdin. xclip forks into the background once it has read its input,
    /// so the process we started exits right away: 0 when it reached the X
    /// server, non-zero with the reason on stderr when it couldn't.
    fn run_xclip(&self, cmd: &mut std::process::Command) -> Result<(), String> {
        let input = std::fs::File::open(&self.copy_path).map_err(|e| e.to_string())?;
        let err_path = format!("{}.err", self.clip_log_path);
        let err_file = std::fs::File::create(&err_path).map_err(|e| e.to_string())?;
        let mut child = cmd.stdin(input).stdout(Stdio::null()).stderr(err_file).spawn().map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(3);
        let status = loop {
            match child.try_wait() {
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
                Ok(status) => break status,
                Err(_) => break None,
            }
        };
        let stderr = std::fs::read_to_string(&err_path).unwrap_or_default();
        let _ = std::fs::remove_file(&err_path);
        let reason = || stderr.lines().find(|l| !l.trim().is_empty()).map(|l| l.trim().to_string());
        match status {
            Some(s) if s.success() => Ok(()),
            Some(s) => Err(reason().unwrap_or_else(|| s.to_string())),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                Err(reason().unwrap_or_else(|| "no answer from the X server in 3s".to_string()))
            }
        }
    }

    /// Copies the copy file to the X11 clipboard: directly first, then (as
    /// root) through `su` as the user that owns the X11 session, since the
    /// X server may not let root's cookie in. Returns the first failure,
    /// which is the one about the direct attempt.
    fn copy_via_xclip(&self, display: &str, xauth: Option<&str>, x11_user: &str, log: &mut Vec<String>) -> Result<(), String> {
        let xclip = |xa: Option<&str>| {
            let mut c = std::process::Command::new("xclip");
            c.args(["-selection", "clipboard", "-display", display]);
            if let Some(xa) = xa {
                c.env("XAUTHORITY", xa);
            }
            c
        };
        let mut attempts = vec![(format!("xclip XAUTHORITY={:?}", xauth), xclip(xauth))];
        if xauth.is_some() {
            attempts.push(("xclip without XAUTHORITY".to_string(), xclip(None)));
        }
        // SAFETY: geteuid can't fail.
        if unsafe { libc::geteuid() } == 0 {
            let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
            let disp = format!("DISPLAY={}", quote(display));
            let mut users = vec![x11_user.to_string()];
            users.extend(std::env::var("SUDO_USER").ok());
            users.dedup();
            for user in users.iter().filter(|u| !u.is_empty() && *u != "root") {
                let su = |login: bool, env: &str| {
                    let mut c = std::process::Command::new("su");
                    if login {
                        c.arg("-l");
                    }
                    c.args(["-c", &format!("{} xclip -selection clipboard", env), user]);
                    c
                };
                attempts.push((format!("su -l {}", user), su(true, &disp)));
                if let Some(xa) = xauth {
                    attempts.push((format!("su {} XAUTHORITY={}", user, xa), su(false, &format!("XAUTHORITY={} {}", quote(xa), disp))));
                }
                attempts.push((format!("su {}", user), su(false, &disp)));
            }
        }
        let mut first_err = None;
        for (label, mut cmd) in attempts {
            match self.run_xclip(&mut cmd) {
                Ok(()) => {
                    log.push(format!("{}: ok", label));
                    return Ok(());
                }
                Err(e) => {
                    log.push(format!("{}: {}", label, e));
                    first_err.get_or_insert(e);
                }
            }
        }
        Err(first_err.unwrap_or_default())
    }

    pub fn copy_to_clipboard(&mut self, text: &str) {
        let _ = std::fs::write(&self.copy_path, text);

        let mut log: Vec<String> = Vec::new();
        let mut copied: Vec<&str> = Vec::new();
        let mut problems: Vec<String> = Vec::new();

        // OSC 52 asks the terminal itself to set the clipboard, so it works
        // over plain SSH. Inside tmux it has to go through tmux, which sends
        // it on to the terminal of each client showing our pane.
        let mut osc52 = None;
        match Self::tmux_own_pane() {
            Some((socket, clients)) => {
                let mut sent = false;
                for client in &clients {
                    let (ok, err) = Self::run_cmd_with_stdin(
                        std::process::Command::new("tmux").args(["-S", &socket, "load-buffer", "-w", "-t", client, "-"]),
                        text, 3,
                    );
                    log.push(format!("tmux load-buffer -w -t {}: {}", client, if ok { "ok" } else { err.trim() }));
                    sent |= ok;
                }
                if sent {
                    copied.push("OSC 52 via tmux");
                } else {
                    let (ok, err) = Self::run_cmd_with_stdin(
                        std::process::Command::new("tmux").args(["-S", &socket, "load-buffer", "-"]),
                        text, 3,
                    );
                    log.push(format!("tmux load-buffer: {}", if ok { "ok" } else { err.trim() }));
                    if ok {
                        copied.push("tmux buffer");
                    }
                    osc52 = Some(Self::osc52_sequence(text, true));
                    copied.push("OSC 52 via tmux passthrough");
                }
            }
            None => {
                osc52 = Some(Self::osc52_sequence(text, false));
                copied.push("OSC 52");
            }
        }

        let (display, xauth) = Self::resolve_x11_env();
        let x11_user = display.as_ref().map(|d| Self::detect_x11_user(d)).unwrap_or_default();
        Self::debug_x11(&display, &xauth, &x11_user, &self.clip_debug_path);
        let xclip_avail = std::process::Command::new("sh")
            .args(["-c", "command -v xclip"])
            .stdout(Stdio::null()).stderr(Stdio::null())
            .status().map(|s| s.success()).unwrap_or(false);
        match display.as_deref() {
            None => log.push("X11: no DISPLAY".to_string()),
            Some(_) if !xclip_avail => {
                log.push("X11: xclip not installed".to_string());
                problems.push("X11 skipped: xclip not installed".to_string());
            }
            Some(disp) => match self.copy_via_xclip(disp, xauth.as_deref(), &x11_user, &mut log) {
                Ok(()) => copied.push("X11"),
                Err(e) => problems.push(format!("X11 copy failed: {}", e)),
            },
        }

        self.clipboard_at = (!copied.is_empty()).then(Instant::now);
        self.clipboard_msg = match (copied.is_empty(), problems.is_empty()) {
            (_, true) => None,
            (true, false) => Some(format!(" Copy failed: {} ", problems.join("; "))),
            (false, false) => Some(format!(" {}; sent via {} ", problems.join("; "), copied.join(" + "))),
        }
        .map(|m| (m, Instant::now()));

        let _ = std::fs::write(
            &self.clip_log_path,
            format!(
                "file={}b sent={} display={} xauthority={} xclip_installed={}\n{}\n",
                text.len(),
                copied.join(","),
                display.as_deref().unwrap_or("not_set"),
                xauth.as_deref().unwrap_or("not_set"),
                xclip_avail,
                log.join("\n"),
            ),
        );

        // Leave the alt screen and show the text, so it can also be selected by hand
        use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
        use crossterm::execute;

        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        if let Some(seq) = osc52 {
            print!("{}", seq);
        }

        println!();
        for line in text.lines() {
            println!(" {}", line);
        }
        println!();
        let _ = std::io::stdout().flush();

        let _ = execute!(std::io::stdout(), EnterAlternateScreen);
        let _ = enable_raw_mode();

        self.needs_terminal_clear = true;
    }

    /// `config.save_dir` with `~`/`~/...` expanded against $HOME at call
    /// time (not cached), so editing it live in the Config Editor or
    /// changing $HOME between launches takes effect immediately.
    pub fn resolved_save_dir(&self) -> std::path::PathBuf {
        let raw = self.config.save_dir.trim();
        crate::config::expand_home(if raw.is_empty() { "~" } else { raw })
    }

    /// Opens the "save to file" prompt (the 's'/'S' counterpart of 'y'/'Y'
    /// clipboard copy) with `content` and a filename proposed from
    /// `name_hint`, which the user can edit before confirming.
    pub fn open_save_prompt(&mut self, content: String, name_hint: &str, ext: &str) {
        self.input_cursor = None;
        self.save_prompt = Some(SaveFilePrompt {
            filename: Self::propose_save_filename(name_hint, ext),
            content,
            confirm_overwrite: false,
        });
    }

    /// Turns a free-form hint (e.g. a job id, minion id, event tag) into a
    /// filesystem-safe proposed filename with a timestamp, so repeated
    /// saves of similar content don't silently collide.
    pub(crate) fn propose_save_filename(hint: &str, ext: &str) -> String {
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let clean: String = hint
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' })
            .collect();
        let clean = clean.trim_matches('-');
        let clean = if clean.is_empty() { "salt-radar" } else { clean };
        format!("{}-{}.{}", clean, ts, ext)
    }

    /// Key handling for the active `save_prompt` overlay — text entry for
    /// the filename, Esc to cancel, Enter to confirm (a second Enter if the
    /// target path already exists, since `confirm_overwrite` starts false
    /// and any further edit resets it).
    pub fn handle_save_prompt_key(&mut self, code: crossterm::event::KeyCode) -> bool {
        use crossterm::event::KeyCode;
        let dir = self.resolved_save_dir();
        match code {
            KeyCode::Esc => {
                self.save_prompt = None;
            }
            KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete | KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End => {
                if let Some(ref mut prompt) = self.save_prompt {
                    if super::line_edit::edit(&mut prompt.filename, &mut self.input_cursor, code) {
                        prompt.confirm_overwrite = false;
                    }
                }
            }
            KeyCode::Enter => {
                let filename_empty = self.save_prompt.as_ref().is_none_or(|p| p.filename.trim().is_empty());
                if filename_empty {
                    return false;
                }
                let path = dir.join(&self.save_prompt.as_ref().unwrap().filename);
                let already_confirmed = self.save_prompt.as_ref().unwrap().confirm_overwrite;
                if path.exists() && !already_confirmed {
                    self.save_prompt.as_mut().unwrap().confirm_overwrite = true;
                    return false;
                }
                let content = self.save_prompt.as_ref().unwrap().content.clone();
                let result = crate::config::create_dir_all_owned(&dir).and_then(|_| std::fs::write(&path, &content));
                if result.is_ok() {
                    crate::config::give_to_sudo_user(&path);
                }
                self.clipboard_msg = Some(match result {
                    Ok(()) => {
                        self.save_at = Some(Instant::now());
                        (format!(" Saved to {} ", path.display()), Instant::now())
                    }
                    Err(e) => (format!(" Save failed: {} ", e), Instant::now()),
                });
                self.save_prompt = None;
            }
            _ => {}
        }
        false
    }

    pub(crate) fn current_user() -> String {
        std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_else(|_| "root".to_string())
    }

    /// Tries to find the unprivileged user that owns the X11 session by:
    /// 1. Ownership of /tmp/.X11-unix/X<display_number>
    /// 2. Ownership of the tmux socket (/tmp/tmux-<uid>/default)
    /// 3. current_user() as last resort
    pub(crate) fn detect_x11_user(display: &str) -> String {
        // 1. Try X11 socket ownership
        let dnum = display
            .trim_start_matches("localhost:")
            .trim_start_matches(':')
            .split('.')
            .next()
            .unwrap_or("")
            .trim();
        if !dnum.is_empty() {
            if let Ok(entries) = std::fs::read_dir("/tmp/.X11-unix") {
                for entry in entries.filter_map(|e| e.ok()) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.strip_prefix('X') == Some(dnum) {
                        if let Ok(meta) = entry.metadata() {
                            #[cfg(target_os = "linux")]
                            {
                                let uid = meta.uid();
                                if let Ok(passwd) = std::fs::read_to_string("/etc/passwd") {
                                    for line in passwd.lines() {
                                        let parts: Vec<&str> = line.split(':').collect();
                                        let uid_str = parts.get(2).and_then(|s| s.parse::<u32>().ok());
                                        if uid_str == Some(uid) {
                                            if let Some(name) = parts.first() {
                                                return name.to_string();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }

        // 2. Try tmux socket ownership — SSH X11 forwarding uses TCP
        //    (localhost:N) so there's no Unix socket in /tmp/.X11-unix/.
        //    The tmux socket path contains the real user's UID.
        if let Some(sock) = Self::tmux_socket_path() {
            // Parse /tmp/tmux-<uid>/default to extract uid
            if let Some(tail) = sock.strip_prefix("/tmp/tmux-") {
                if let Some(uid_str) = tail.split('/').next() {
                    if let Ok(uid) = uid_str.parse::<u32>() {
                        return Self::uid_to_username(uid);
                    }
                }
            }
        }

        // 3. Fallback to current process user
        Self::current_user()
    }
}
