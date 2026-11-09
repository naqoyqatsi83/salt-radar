//! Copy to clipboard (tmux / OSC 52 and X11 via xclip, including finding the
//! X11 session) and the save-to-file prompt.

use super::*;

/// The tmux pane salt-radar runs in.
pub(crate) struct TmuxPane {
    socket: String,
    session: String,
    clients: Vec<String>,
}

/// The X11 display to copy to, the cookie for it and the user it belongs to.
pub(crate) struct X11Env {
    display: Option<String>,
    xauth: Option<String>,
    user: String,
    source: String,
}

impl App {
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

    /// This process and its ancestors, nearest first.
    fn ancestor_pids() -> Vec<u32> {
        let mut pids = Vec::new();
        let mut pid = std::process::id();
        while pid > 1 && pids.len() < 32 {
            pids.push(pid);
            let ppid = std::fs::read_to_string(format!("/proc/{}/stat", pid)).ok()
                .and_then(|s| s.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse::<u32>().ok());
            pid = ppid.unwrap_or(0);
        }
        pids
    }

    fn proc_env(pid: u32, name: &str) -> Option<String> {
        let env = std::fs::read(format!("/proc/{}/environ", pid)).ok()?;
        let prefix = format!("{}=", name);
        env.split(|&b| b == 0)
            .find_map(|e| e.strip_prefix(prefix.as_bytes()).map(|v| String::from_utf8_lossy(v).to_string()))
            .filter(|v| !v.is_empty())
    }

    fn owner_uid(path: &str) -> Option<u32> {
        std::fs::metadata(path).ok().map(|m| m.uid())
    }

    /// The terminals of this process and its ancestors, nearest first:
    /// `sudo` with `use_pty` (the default on Ubuntu and RHEL) runs us on a
    /// pty of its own, so the tmux pane's tty is only on sudo's parent.
    fn own_and_ancestor_ttys() -> Vec<String> {
        let mut ttys: Vec<String> = Vec::new();
        for pid in Self::ancestor_pids() {
            if let Ok(tty) = std::fs::read_link(format!("/proc/{}/fd/0", pid)) {
                let tty = tty.to_string_lossy().to_string();
                if tty.starts_with("/dev/") && !ttys.contains(&tty) {
                    ttys.push(tty);
                }
            }
        }
        ttys
    }

    /// `$TMUX`'s socket first, then every server under /tmp/tmux-*, since
    /// `sudo` drops `$TMUX` and root may see several.
    fn tmux_sockets() -> Vec<String> {
        let mut sockets: Vec<String> = std::env::var("TMUX").ok()
            .and_then(|t| t.split(',').next().map(str::to_string))
            .filter(|s| !s.is_empty())
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
        sockets
    }

    /// The tmux pane salt-radar runs in, matched by terminal rather than
    /// `$TMUX`, with the clients showing it. None outside tmux.
    pub(crate) fn tmux_own_pane() -> Option<TmuxPane> {
        let ttys = Self::own_and_ancestor_ttys();
        for socket in Self::tmux_sockets() {
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
            return Some(TmuxPane {
                session: session.to_string(),
                clients: clients.lines().filter(|l| !l.is_empty()).map(str::to_string).collect(),
                socket,
            });
        }
        None
    }

    /// A variable from a tmux session's environment, or the server's global
    /// one when `session` is None.
    fn tmux_env(socket: &str, session: Option<&str>, name: &str) -> Option<String> {
        let mut cmd = std::process::Command::new("tmux");
        cmd.args(["-S", socket, "show-environment"]);
        match session {
            Some(s) => cmd.args(["-t", s]),
            None => cmd.arg("-g"),
        };
        let out = Self::run_cmd_with_output(cmd.arg(name), 3)?;
        out.trim().strip_prefix(&format!("{}=", name)).filter(|v| !v.is_empty()).map(str::to_string)
    }

    /// Finds the X11 display to copy to, and whose it is. In order:
    /// 1. the environment of our tmux session, which tmux updates on every
    ///    attach, so it follows a re-attach from a new SSH login
    /// 2. this process and its ancestors: `sudo -i` and `su -` clear the
    ///    environment, but the login shell above them still has DISPLAY
    /// 3. the tmux server's global environment, from when it was started
    ///
    /// XAUTHORITY comes from the same place, or else the cookie files of the
    /// user it belongs to (root's own cookie doesn't open a user's display).
    pub(crate) fn resolve_x11_env(pane: Option<&TmuxPane>) -> X11Env {
        let user_of = |uid: Option<u32>| {
            let name = uid.map(Self::uid_to_username).unwrap_or_default();
            match std::env::var("SUDO_USER") {
                Ok(sudo_user) if (name.is_empty() || name == "root") && !sudo_user.is_empty() => sudo_user,
                _ => name,
            }
        };
        let mut found: Option<(String, Option<String>, String, String)> = None;
        if let Some(p) = pane {
            if let Some(d) = Self::tmux_env(&p.socket, Some(&p.session), "DISPLAY") {
                let xa = Self::tmux_env(&p.socket, Some(&p.session), "XAUTHORITY");
                found = Some((d, xa, user_of(Self::owner_uid(&p.socket)), format!("tmux session {}", p.session)));
            }
        }
        if found.is_none() {
            let with_display: Vec<(u32, String)> = Self::ancestor_pids().into_iter()
                .filter_map(|pid| Some((pid, Self::proc_env(pid, "DISPLAY")?)))
                .collect();
            if let Some((pid, d)) = with_display.first() {
                // The nearest one may be root's (sudo keeps the caller's
                // environment); the display belongs to the first non-root
                // process above it with the same DISPLAY.
                let uid = with_display.iter()
                    .filter(|(_, other)| other == d)
                    .filter_map(|(p, _)| Self::owner_uid(&format!("/proc/{}", p)))
                    .find(|&u| u != 0)
                    .or(Some(0));
                let source = if *pid == std::process::id() { "own environment".to_string() } else { format!("process {}", pid) };
                found = Some((d.clone(), Self::proc_env(*pid, "XAUTHORITY"), user_of(uid), source));
            }
        }
        if found.is_none() {
            let socket = pane.map(|p| p.socket.clone()).or_else(|| Self::tmux_sockets().into_iter().next());
            if let Some(s) = socket {
                if let Some(d) = Self::tmux_env(&s, None, "DISPLAY") {
                    let xa = Self::tmux_env(&s, None, "XAUTHORITY");
                    found = Some((d, xa, user_of(Self::owner_uid(&s)), "tmux global environment".to_string()));
                }
            }
        }
        match found {
            Some((display, xauth, user, source)) => X11Env {
                xauth: xauth.or_else(|| Self::find_xauthority(&user)),
                display: Some(display),
                user,
                source,
            },
            None => X11Env { display: None, xauth: None, user: String::new(), source: "not found".to_string() },
        }
    }

    /// Find the Xauthority file for a user. Checks (in order):
    /// 1. XAUTHORITY env from running processes of that user (/proc/<pid>/environ)
    /// 2. ~/.Xauthority from the user's passwd home directory
    /// 3. /run/user/<uid>/xauth* (globbing for pam_xauth random filenames)
    pub(crate) fn find_xauthority(username: &str) -> Option<String> {
        if username.is_empty() {
            return None;
        }
        if let Some(path) = Self::xauthority_from_process(username) {
            return Some(path);
        }

        let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
        for line in passwd.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.first() != Some(&username) { continue; }
            let home = parts.get(5)?;
            let uid: u32 = parts.get(2)?.parse().ok()?;

            let dot_path = format!("{}/.Xauthority", home);
            if std::path::Path::new(&dot_path).exists() {
                return Some(dot_path);
            }

            let run_dir = format!("/run/user/{}", uid);
            if let Ok(entries) = std::fs::read_dir(&run_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    if entry.file_name().to_string_lossy().starts_with("xauth") {
                        return Some(entry.path().to_string_lossy().to_string());
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
        pids.sort_by(|a, b| b.cmp(a));
        pids.into_iter().find_map(|pid| {
            Self::proc_env(pid, "XAUTHORITY").filter(|v| std::path::Path::new(v).exists())
        })
    }

    /// Writes where salt-radar looked for an X11 display and what it found,
    /// for a user to share when copying doesn't work.
    pub(crate) fn debug_x11(env: &X11Env, pane: Option<&TmuxPane>, debug_path: &str) {
        use std::fmt::Write;
        let mut d = String::new();
        writeln!(d, "=== salt-radar X11 debug ===").ok();
        for name in ["DISPLAY", "XAUTHORITY", "USER", "SUDO_USER", "TMUX"] {
            writeln!(d, "own {}: {:?}", name, std::env::var(name).ok()).ok();
        }
        match pane {
            Some(p) => {
                writeln!(d, "tmux pane: socket {} session {} clients {:?}", p.socket, p.session, p.clients).ok();
                writeln!(d, "tmux session DISPLAY: {:?}", Self::tmux_env(&p.socket, Some(&p.session), "DISPLAY")).ok();
            }
            None => {
                writeln!(d, "tmux pane: not found (ttys {:?}, sockets {:?})", Self::own_and_ancestor_ttys(), Self::tmux_sockets()).ok();
            }
        }
        for s in Self::tmux_sockets() {
            writeln!(d, "tmux global DISPLAY on {}: {:?}", s, Self::tmux_env(&s, None, "DISPLAY")).ok();
        }
        writeln!(d, "\n--- ancestors ---").ok();
        for pid in Self::ancestor_pids() {
            let cmd = std::fs::read(format!("/proc/{}/cmdline", pid)).ok()
                .map(|c| String::from_utf8_lossy(&c).replace('\0', " ").trim().to_string())
                .unwrap_or_default();
            let user = Self::owner_uid(&format!("/proc/{}", pid)).map(Self::uid_to_username).unwrap_or_default();
            writeln!(d, "  {} {} DISPLAY={:?} [{}]", pid, user, Self::proc_env(pid, "DISPLAY"), cmd).ok();
        }
        writeln!(d, "\n--- resolved ---").ok();
        writeln!(d, "DISPLAY: {:?} (from {})", env.display, env.source).ok();
        writeln!(d, "user: {}", env.user).ok();
        writeln!(d, "XAUTHORITY: {:?}", env.xauth).ok();
        if let Some(ref xa) = env.xauth {
            let out = std::process::Command::new("xauth").args(["-f", xa, "list"]).output().ok();
            match out {
                Some(o) => writeln!(d, "xauth list: [{}] {}", String::from_utf8_lossy(&o.stdout).trim(), String::from_utf8_lossy(&o.stderr).trim()).ok(),
                None => writeln!(d, "xauth list: (failed to run)").ok(),
            };
        }
        let xclip_v = std::process::Command::new("xclip").arg("-version").output().ok()
            .map(|o| String::from_utf8_lossy(if o.stdout.is_empty() { &o.stderr } else { &o.stdout }).lines().next().unwrap_or("").to_string());
        writeln!(d, "xclip: {:?}", xclip_v).ok();
        let _ = std::fs::write(debug_path, &d);
    }

    pub(crate) fn uid_to_username(uid: u32) -> String {
        if let Ok(passwd) = std::fs::read_to_string("/etc/passwd") {
            for line in passwd.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.get(2).and_then(|s| s.parse::<u32>().ok()) == Some(uid) {
                    if let Some(name) = parts.first() {
                        return name.to_string();
                    }
                }
            }
        }
        format!("uid:{}", uid)
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
        let pane = Self::tmux_own_pane();
        match &pane {
            Some(TmuxPane { socket, clients, .. }) => {
                let mut sent = false;
                for client in clients {
                    let (ok, err) = Self::run_cmd_with_stdin(
                        std::process::Command::new("tmux").args(["-S", socket, "load-buffer", "-w", "-t", client, "-"]),
                        text, 3,
                    );
                    log.push(format!("tmux load-buffer -w -t {}: {}", client, if ok { "ok" } else { err.trim() }));
                    sent |= ok;
                }
                if sent {
                    copied.push("OSC 52 via tmux");
                } else {
                    let (ok, err) = Self::run_cmd_with_stdin(
                        std::process::Command::new("tmux").args(["-S", socket, "load-buffer", "-"]),
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

        let x11 = Self::resolve_x11_env(pane.as_ref());
        Self::debug_x11(&x11, pane.as_ref(), &self.clip_debug_path);
        let X11Env { display, xauth, user: x11_user, source: x11_source } = x11;
        let xclip_avail = std::process::Command::new("sh")
            .args(["-c", "command -v xclip"])
            .stdout(Stdio::null()).stderr(Stdio::null())
            .status().map(|s| s.success()).unwrap_or(false);
        match display.as_deref() {
            None => {
                log.push("X11: no DISPLAY found".to_string());
                problems.push("no X11 display found".to_string());
            }
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
                "file={}b sent={} display={} (from {}, user {}) xauthority={} xclip_installed={}\n{}\n",
                text.len(),
                copied.join(","),
                display.as_deref().unwrap_or("not_set"),
                x11_source,
                x11_user,
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


}
