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

    /// Spawn xclip in the background (detached) and pipe text to its stdin.
    /// xclip MUST stay alive as the clipboard selection owner — we can't wait
    /// for it or kill it. Returns true if spawn succeeded.
    pub(crate) fn run_xclip_detached(
        xauth: Option<&str>,
        display: &str,
        text: &str,
    ) -> bool {
        let mut cmd = std::process::Command::new("xclip");
        cmd.args(["-selection", "clipboard", "-display", display]);
        cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
        if let Some(xa) = xauth {
            cmd.env("XAUTHORITY", xa);
        }

        match cmd.spawn() {
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    let owned = text.to_string();
                    std::thread::spawn(move || {
                        let _ = stdin.write_all(owned.as_bytes());
                        // stdin drops → EOF → xclip reads full input
                    });
                }
                // Forget child to prevent Drop from reaping (which blocks
                // forever since xclip waits for clipboard paste requests).
                std::mem::forget(child);
                true
            }
            Err(_) => false,
        }
    }

    pub fn copy_to_clipboard(&mut self, text: &str) {
        // 1. Write to local file (always works, easy debug)
        let _ = std::fs::write(&self.copy_path, text);

        let mut warnings: Vec<&str> = Vec::new();
        let mut ok_tmux = false;
        let mut ok_xclip = false;

        // 2. tmux load-buffer (for Ctrl+b ] paste inside tmux).
        if let Some(socket) = Self::tmux_socket_path() {
            let (ok, _) = Self::run_cmd_with_stdin(
                std::process::Command::new("tmux").args(["-S", &socket, "load-buffer", "-"]),
                text, 3,
            );
            ok_tmux = ok;
            // Check if tmux will push to system clipboard via OSC 52
            let set_clip_on = std::process::Command::new("tmux")
                .args(["-S", &socket, "show-options", "-g", "set-clipboard"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim() == "set-clipboard on")
                .unwrap_or(false);
            if ok && !set_clip_on {
                warnings.push("tmux set-clipboard is off — paste with Ctrl+b ]");
            }
        }

        // 3. xclip
        let (display, xauth) = Self::resolve_x11_env();
        let x11_user = display.as_ref().map(|d| Self::detect_x11_user(d)).unwrap_or_default();

        // Write full diagnostics before attempting anything
        Self::debug_x11(&display, &xauth, &x11_user, &self.clip_debug_path);

        let xclip_avail = std::process::Command::new("sh")
            .args(["-c", "command -v xclip"])
            .stdout(Stdio::null()).stderr(Stdio::null())
            .status().map(|s| s.success()).unwrap_or(false);

        let mut xclip_log: Vec<String> = Vec::new();

        if let Some(ref disp) = display {
            if !xclip_avail {
                warnings.push("xclip not installed");
                xclip_log.push("xclip not installed".to_string());
            } else {
                // --- Strategy 0: Direct xclip as root (detached) ---
                // xclip MUST stay alive as clipboard owner — spawn and
                // forget. We can't use run_cmd_with_stdin() which enforces
                // a timeout and kills the process.
                let with_xa = xauth.as_deref();
                if Self::run_xclip_detached(with_xa, disp, text) {
                    ok_xclip = true;
                    xclip_log.push(format!("detached xclip(root) XA={:?}: OK", with_xa));
                } else {
                    xclip_log.push(format!("detached xclip(root) XA={:?}: FAIL", with_xa));
                }

                // --- Strategy 1: Direct xclip as root without XAUTHORITY ---
                if !ok_xclip {
                    if Self::run_xclip_detached(None, disp, text) {
                        ok_xclip = true;
                        xclip_log.push("detached xclip(root) no-XA: OK".to_string());
                    } else {
                        xclip_log.push("detached xclip(root) no-XA: FAIL".to_string());
                    }
                }

                // --- su approaches as fallback (these use run_cmd_with_stdin
                //     because su should exit quickly) ---
                if !ok_xclip && xclip_avail {
                    let cur_user = Self::current_user();
                    let mut users_to_try = vec![x11_user.clone()];
                    if cur_user != x11_user {
                        users_to_try.push(cur_user);
                    }

                    for user in &users_to_try {
                        if !std::process::Command::new("id")
                            .arg(user).stdout(Stdio::null()).stderr(Stdio::null())
                            .status().map(|s| s.success()).unwrap_or(false)
                        {
                            xclip_log.push(format!("su: user {} not found, skip", user));
                            continue;
                        }

                        // su -l (login shell triggers pam_xauth) — background
                        // xclip so su exits immediately (xclip stays alive)
                        let copy_path = &self.copy_path;
                        let cmd1 = format!("nohup sh -c 'DISPLAY={} xclip -selection clipboard < {}' >/dev/null 2>&1 &", disp, copy_path);
                        let (ok, _) = Self::run_cmd_with_stdin(
                            std::process::Command::new("su").args(["-l", user, "-c", &cmd1]),
                            "", 5,
                        );
                        if ok {
                            ok_xclip = true;
                            xclip_log.push(format!("su -l {}: OK", user));
                            break;
                        }
                        xclip_log.push(format!("su -l {}: FAIL", user));

                        // su with explicit XAUTHORITY
                        if let Some(ref xa) = xauth {
                            let cmd2 = format!("nohup sh -c 'XAUTHORITY={} DISPLAY={} xclip -selection clipboard < {}' >/dev/null 2>&1 &", xa, disp, copy_path);
                            let (ok, _) = Self::run_cmd_with_stdin(
                                std::process::Command::new("su").args(["-c", &cmd2, user]),
                                "", 5,
                            );
                            if ok {
                                ok_xclip = true;
                                xclip_log.push(format!("su {} with-XA: OK", user));
                                break;
                            }
                            xclip_log.push(format!("su {} with-XA: FAIL", user));
                        }

                        // su without XAUTHORITY (xclip auto-detects)
                        let cmd3 = format!("nohup sh -c 'DISPLAY={} xclip -selection clipboard < {}' >/dev/null 2>&1 &", disp, copy_path);
                        let (ok, _) = Self::run_cmd_with_stdin(
                            std::process::Command::new("su").args(["-c", &cmd3, user]),
                            "", 5,
                        );
                        if ok {
                            ok_xclip = true;
                            xclip_log.push(format!("su {} no-XA: OK", user));
                            break;
                        }
                        xclip_log.push(format!("su {} no-XA: FAIL", user));
                    }
                }
            }
        } else {
            xclip_log.push("no DISPLAY".to_string());
        }

        // 4. Compose status message with all findings
        let details = if !ok_tmux && !ok_xclip {
            warnings.push("clipboard copy failed");
            warnings.join("; ")
        } else if ok_tmux && ok_xclip {
            String::new()
        } else if ok_tmux {
            format!("Copied to tmux buffer ({}b). {}", text.len(), warnings.join("; "))
        } else {
            format!("Copied via X11 ({}b). {}", text.len(), warnings.join("; "))
        };
        if details.is_empty() {
            self.clipboard_msg = None;
        } else {
            self.clipboard_msg = Some((details, Instant::now()));
        }

        // 5. Debug log
        let xa_info = xauth.as_deref().unwrap_or("not_set");
        let disp_info = display.as_deref().unwrap_or("not_set");
        let _ = std::fs::write(
            &self.clip_log_path,
            format!(
                "file={}b tmux={} xclip={} display={} xauthority={} xclip_installed={}\n{}",
                text.len(),
                if ok_tmux { "ok" } else { "skip" },
                if ok_xclip { "ok" } else { "fail" },
                disp_info, xa_info, xclip_avail,
                xclip_log.join("\n"),
            ),
        );

        // 6. Leave alt screen and show text visibly
        use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
        use crossterm::execute;

        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);

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
                let result = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &content));
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
