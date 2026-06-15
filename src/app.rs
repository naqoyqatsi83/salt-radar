use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use crossterm::event::KeyCode;
use ratatui::layout::Rect;

use crate::alerts::AlertEngine;
use crate::collectors;
use crate::config::{load_config, AppConfig};
use crate::types::*;

pub struct App {
    // Config
    pub config: AppConfig,

    // Tabs
    pub current_tab: Tab,
    pub session_start: Instant,
    pub tick_count: u64,

    // Data from agent
    pub agent: AgentState,
    pub config_snapshot: Option<ConfigSnapshot>,

    // Collector data
    pub keys: Vec<KeyInfo>,
    pub processes: Vec<SaltProcess>,
    pub services: Vec<ServiceHealth>,
    pub zmq_connections: Vec<ZmqConn>,
    pub system_info: SystemInfo,

    // Minion cache (from agent)
    pub minion_info: Vec<MinionInfo>,

    // Events buffer
    pub events: VecDeque<SaltEvent>,
    pub events_since_last_tick: Vec<SaltEvent>,
    pub events_per_sec: f64,

    // Live jobs
    pub jobs: Vec<JobEntry>,

    // Alerts
    pub alert_engine: AlertEngine,

    // UI state
    pub last_frame_size: Rect,
    pub detail_popup: Option<DetailPopup>,
    pub show_help: bool,

    // Filter state
    pub filter_text: String,
    pub is_filtering: bool,

    // Cursor positions
    pub event_selected: usize,
    pub job_selected: usize,
    pub alert_selected: usize,
    pub minion_selected: usize,

    // Agent NDJSON reader thread
    pub agent_rx: Arc<Mutex<VecDeque<AgentMessage>>>,
    agent_req_id: u64,
    config_tick: u64,
    /// Pending jid_lookup (request_id, jid) waiting for async result
    pending_jid_lookup: Option<(u64, String)>,
    /// Events statistics
    pub events_total: u64,
    events_last_sec_count: u64,
    events_last_sec_time: Instant,
    /// Seconds since last event
    pub secs_since_last_event: u64,
    last_event_time: Option<Instant>,
    /// Ring buffer of recent event rates for sparkline (60 samples = 1 min)
    pub event_rate_history: [f64; 60],
    event_rate_idx: usize,
    /// Timestamp of last successful clipboard copy (for UI feedback)
    pub clipboard_at: Option<Instant>,
    /// Flag for main loop to call terminal.clear() after alt-screen disruption
    pub needs_terminal_clear: bool,
    /// Measured ZMQ/master response latency in milliseconds (from zmq_ping agent command)
    pub zmq_latency_ms: f64,
    /// Ring buffer of recent latency samples for sparkline (60 samples = 1 per tick)
    pub zmq_latency_history: [f64; 60],
    zmq_latency_idx: usize,
}

#[derive(Clone, Debug)]
pub enum AgentMessage {
    Result { id: u64, cmd: String, ok: bool, data: serde_json::Value },
    Event { tag: String, data: serde_json::Value, ts: f64 },
    Error { id: Option<u64>, message: String },
}

#[derive(Clone, Debug)]
pub enum DetailPopup {
    Minion(MinionInfo),
    SaltEvent(SaltEvent),
    JobDetail { jid: String, lookup: Option<JidLookup>, selected_return: Option<usize> },
    Alert(AlertEntry),
}

impl App {
    pub fn new() -> Self {
        let config = load_config(&std::path::Path::new("salt-radar.toml"));
        let agent_rx: Arc<Mutex<VecDeque<AgentMessage>>> = Arc::new(Mutex::new(VecDeque::new()));

        // Spawn agent
        let rx = agent_rx.clone();
        let agent_path = config.agent_path.clone();
        let python_bin = config.python_bin.clone();
        let master_config = config.master_config.clone();
        std::thread::spawn(move || {
            crate::agent::agent_main(rx, &agent_path, &python_bin, &master_config);
        });

        Self {
            config,
            current_tab: Tab::Dashboard,
            session_start: Instant::now(),
            tick_count: 0,

            agent: AgentState {
                connected: false,
                version: None,
                last_error: None,
            },
            config_snapshot: None,

            keys: Vec::new(),
            processes: Vec::new(),
            services: Vec::new(),
            zmq_connections: Vec::new(),
            system_info: SystemInfo::default(),

            minion_info: Vec::new(),

            events: VecDeque::with_capacity(5000),
            events_since_last_tick: Vec::new(),
            events_per_sec: 0.0,

            jobs: Vec::new(),

            alert_engine: AlertEngine::new(),

            last_frame_size: Rect::default(),
            detail_popup: None,
            show_help: false,

            filter_text: String::new(),
            is_filtering: false,

            event_selected: 0,
            job_selected: 0,
            alert_selected: 0,
            minion_selected: 0,

            agent_rx,
            agent_req_id: 0,
            config_tick: 0,
            pending_jid_lookup: None,
            events_total: 0,
            events_last_sec_count: 0,
            events_last_sec_time: Instant::now(),
            secs_since_last_event: 0,
            last_event_time: None,
            event_rate_history: [0.0; 60],
            event_rate_idx: 0,
            clipboard_at: None,
            needs_terminal_clear: false,
            zmq_latency_ms: 0.0,
            zmq_latency_history: [0.0; 60],
            zmq_latency_idx: 0,
        }
    }

    /// Poll pending agent messages from background thread
    pub fn drain_agent_messages(&mut self) {
        let mut q = self.agent_rx.lock().unwrap();
        while let Some(msg) = q.pop_front() {
            match msg {
                AgentMessage::Result { id, cmd, ok, data } => {
                    self.agent.connected = true;
                    self.agent.last_error = None;
                    let is_jid_lookup = cmd == "jid_lookup";
                    let jid_data = if is_jid_lookup { Some(parse_jid_lookup(&data)) } else { None };
                    match cmd.as_str() {
                        "version" => {
                            self.agent.version = Some(parse_version(&data));
                        }
                        "config_snapshot" => {
                            self.config_snapshot = Some(parse_config_snapshot(&data));
                        }
                        "minion_cache_list" => {
                            self.minion_info = parse_minion_list(&data);
                        }
                        "zmq_ping" => {
                            if let Some(lat) = data["latency_ms"].as_f64() {
                                self.zmq_latency_ms = lat;
                                self.zmq_latency_history[self.zmq_latency_idx] = lat;
                                self.zmq_latency_idx = (self.zmq_latency_idx + 1) % 60;
                            }
                        }
                        _ => {}
                    }
                    if !ok {
                        self.agent.last_error = data["error"].as_str().map(|s| s.to_string());
                    }
                    // Handle jid_lookup outside the mutable borrow of match
                    if is_jid_lookup {
                        if let Some(jid_data) = jid_data {
                            if let Some((pending_id, _)) = self.pending_jid_lookup {
                                if id == pending_id {
                                    self.pending_jid_lookup = None;
                                    if let Some(DetailPopup::JobDetail { ref mut lookup, .. }) = self.detail_popup {
                                        *lookup = Some(jid_data);
                                    }
                                }
                            } else {
                                // No pending — update if popup has no data
                                if let Some(DetailPopup::JobDetail { ref mut lookup, .. }) = self.detail_popup {
                                    if lookup.is_none() {
                                        *lookup = Some(jid_data);
                                    }
                                }
                            }
                        }
                    }
                }
                AgentMessage::Event { tag, data, ts } => {
                    let evt = SaltEvent { tag, data, ts };
                    self.events_since_last_tick.push(evt.clone());
                    if self.events.len() >= self.config.event_buffer_size {
                        self.events.pop_front();
                    }
                    self.events.push_back(evt);
                    self.events_total += 1;
                    self.last_event_time = Some(Instant::now());

                    // Track live jobs from events
                    if let Some(evt) = self.events.back() {
                        track_job_from_event(&mut self.jobs, evt);
                    }
                }
                AgentMessage::Error { message, .. } => {
                    self.agent.last_error = Some(message);
                    self.agent.connected = false;
                }
            }
        }
    }

    /// Send a command to the agent, returns the request id
    pub fn send_agent_cmd(&mut self, cmd: &str, extra: Option<serde_json::Value>) -> u64 {
        self.agent_req_id += 1;
        let id = self.agent_req_id;
        let mut req = serde_json::json!({
            "id": id,
            "cmd": cmd,
        });
        if let Some(e) = extra {
            if let Some(obj) = req.as_object_mut() {
                for (k, v) in e.as_object().unwrap_or(&serde_json::Map::new()) {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
        let payload = serde_json::to_string(&req).unwrap() + "\n";
        let _ = std::fs::write("/tmp/salt-radar-cmd", &payload);
        id
    }

    /// Fast poll between ticks
    pub fn fast_poll(&mut self) -> bool {
        self.drain_agent_messages();
        false
    }

    /// Clamp all cursor selections to valid bounds
    fn clamp_selection(&mut self) {
        self.event_selected = self.event_selected
            .min(self.filtered_events_len().saturating_sub(1));
        self.job_selected = self.job_selected
            .min(self.filtered_jobs_len().saturating_sub(1));
        self.alert_selected = self.alert_selected
            .min(self.filtered_alerts_len().saturating_sub(1));
        self.minion_selected = self.minion_selected
            .min(self.filtered_minions_len().saturating_sub(1));
    }

    fn matches_filter(text: &str, pat: &str) -> bool {
        if pat.is_empty() { return true; }
        text.to_lowercase().contains(&pat.to_lowercase())
    }

    fn matches_event(evt: &SaltEvent, pat: &str, known_minions: &[String]) -> bool {
        if pat.is_empty() { return true; }
        if Self::matches_filter(&evt.tag, pat)
            || evt.data["fun"].as_str().map(|f| Self::matches_filter(f, pat)).unwrap_or(false)
            || evt.data["id"].as_str().map(|i| Self::matches_filter(i, pat)).unwrap_or(false)
            || Self::matches_filter(&evt.data.to_string(), pat)
        {
            return true;
        }
        // If the filter matches a known minion ID, also show events targeted at
        // all minions ("*") or whose tgt pattern could match that minion.
        if known_minions.iter().any(|m| Self::matches_filter(m, pat)) {
            if let Some(tgt) = evt.data["tgt"].as_str() {
                if tgt == "*" || known_minions.iter().any(|m| Self::matches_filter(tgt, m)) {
                    return true;
                }
            }
        }
        false
    }

    fn matches_job(job: &JobEntry, pat: &str, known_minions: &[String]) -> bool {
        if pat.is_empty() { return true; }
        if Self::matches_filter(&job.jid, pat)
            || Self::matches_filter(&job.fun, pat)
            || Self::matches_filter(&job.tgt, pat)
        {
            return true;
        }
        // Show jobs targeting "*" when searching for a known minion
        if known_minions.iter().any(|m| Self::matches_filter(m, pat)) {
            if job.tgt == "*" {
                return true;
            }
        }
        false
    }

    fn matches_alert(alert: &AlertEntry, pat: &str) -> bool {
        if pat.is_empty() { return true; }
        Self::matches_filter(alert.rule, pat)
            || Self::matches_filter(&alert.description, pat)
    }

    fn matches_minion(minion: &MinionInfo, pat: &str) -> bool {
        if pat.is_empty() { return true; }
        Self::matches_filter(&minion.id, pat)
            || Self::matches_filter(&minion.os, pat)
            || Self::matches_filter(&minion.osrelease, pat)
            || Self::matches_filter(&minion.salt_version, pat)
    }

    pub fn filtered_events_len(&self) -> usize {
        if self.filter_text.is_empty() {
            return self.events.len();
        }
        let pat = &self.filter_text;
        let known: Vec<String> = self.minion_info.iter().map(|m| m.id.clone()).collect();
        self.events.iter().rev().filter(|e| Self::matches_event(e, pat, &known)).count()
    }

    pub fn get_filtered_event(&self, index: usize) -> Option<&SaltEvent> {
        if self.filter_text.is_empty() {
            let rev_idx = self.events.len().saturating_sub(1).saturating_sub(index);
            return self.events.get(rev_idx);
        }
        let pat = &self.filter_text;
        let known: Vec<String> = self.minion_info.iter().map(|m| m.id.clone()).collect();
        self.events.iter().rev().filter(|e| Self::matches_event(e, pat, &known)).nth(index)
    }

    pub fn filtered_jobs_len(&self) -> usize {
        if self.filter_text.is_empty() {
            return self.jobs.len();
        }
        let pat = &self.filter_text;
        let known: Vec<String> = self.minion_info.iter().map(|m| m.id.clone()).collect();
        self.jobs.iter().filter(|j| Self::matches_job(j, pat, &known)).count()
    }

    pub fn get_filtered_job(&self, index: usize) -> Option<&JobEntry> {
        if self.filter_text.is_empty() {
            return self.jobs.get(index);
        }
        let pat = &self.filter_text;
        let known: Vec<String> = self.minion_info.iter().map(|m| m.id.clone()).collect();
        self.jobs.iter().filter(|j| Self::matches_job(j, pat, &known)).nth(index)
    }

    pub fn filtered_alerts_len(&self) -> usize {
        if self.filter_text.is_empty() {
            return self.alert_engine.alerts.len();
        }
        let pat = &self.filter_text;
        self.alert_engine.alerts.iter().filter(|a| Self::matches_alert(a, pat)).count()
    }

    pub fn get_filtered_alert(&self, index: usize) -> Option<&AlertEntry> {
        if self.filter_text.is_empty() {
            return self.alert_engine.alerts.get(index);
        }
        let pat = &self.filter_text;
        self.alert_engine.alerts.iter().filter(|a| Self::matches_alert(a, pat)).nth(index)
    }

    pub fn filtered_minions_len(&self) -> usize {
        if self.filter_text.is_empty() {
            return self.minion_info.len();
        }
        let pat = &self.filter_text;
        self.minion_info.iter().filter(|m| Self::matches_minion(m, pat)).count()
    }

    pub fn get_filtered_minion(&self, index: usize) -> Option<&MinionInfo> {
        if self.filter_text.is_empty() {
            return self.minion_info.get(index);
        }
        let pat = &self.filter_text;
        self.minion_info.iter().filter(|m| Self::matches_minion(m, pat)).nth(index)
    }

    fn open_detail(&mut self) {
        match self.current_tab {
            Tab::Events => {
                if let Some(evt) = self.get_filtered_event(self.event_selected) {
                    self.detail_popup = Some(DetailPopup::SaltEvent(evt.clone()));
                }
            }
            Tab::Jobs => {
                if let Some(job) = self.get_filtered_job(self.job_selected) {
                    let jid = job.jid.clone();
                    let req_id = self.send_agent_cmd("jid_lookup", Some(serde_json::json!({"jid": jid.clone()})));
                    self.pending_jid_lookup = Some((req_id, jid.clone()));
                    self.detail_popup = Some(DetailPopup::JobDetail {
                        jid,
                        lookup: None,
                        selected_return: None,
                    });
                }
            }
            Tab::Alerts => {
                if let Some(alert) = self.get_filtered_alert(self.alert_selected) {
                    self.detail_popup = Some(DetailPopup::Alert(alert.clone()));
                }
            }
            Tab::Minions => {
                if let Some(minion) = self.get_filtered_minion(self.minion_selected) {
                    self.detail_popup = Some(DetailPopup::Minion(minion.clone()));
                }
            }
            _ => {}
        }
    }

    pub fn copy_to_clipboard(&mut self, text: &str) {
        // 1. Write to local file (always works)
        let _ = std::fs::write("/tmp/salt-radar-copy.txt", text);

        // 2. tmux load-buffer (for tmux Ctrl+b ] paste)
        if std::env::var("TMUX").is_ok() {
            let _ = std::process::Command::new("tmux")
                .arg("load-buffer").arg("/tmp/salt-radar-copy.txt")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }

        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());

        // 3. Try OSC 52 directly via /dev/tty (bypasses crossterm/stdout routing)
        //    Send CLIPBOARD (c) for Ctrl+V and PRIMARY (p) for Shift+Insert
        if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
            use std::io::Write;
            if std::env::var("TMUX").is_ok() {
                // DCS passthrough inside tmux: wrap OSC in \x1bPtmux;...\x1b\\
                let _ = tty.write_all(b"\x1bPtmux;\x1b]52;c;");
                let _ = tty.write_all(b64.as_bytes());
                let _ = tty.write_all(b"\x07\x1b\\");
                let _ = tty.write_all(b"\x1bPtmux;\x1b]52;p;");
                let _ = tty.write_all(b64.as_bytes());
                let _ = tty.write_all(b"\x07\x1b\\");
            } else {
                let _ = tty.write_all(b"\x1b]52;c;");
                let _ = tty.write_all(b64.as_bytes());
                let _ = tty.write_all(b"\x07");
                let _ = tty.write_all(b"\x1b]52;p;");
                let _ = tty.write_all(b64.as_bytes());
                let _ = tty.write_all(b"\x07");
            }
            let _ = tty.flush();
        }

        // 4. Also try OSC 52 via shell printf (bypasses any Rust I/O buffering)
        let _ = std::process::Command::new("bash")
            .arg("-c")
            .arg(format!(
                "printf '\\033]52;c;{}\\007\\033]52;p;{}\\007' >/dev/tty",
                b64, b64
            ))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        // 5. Leave alt screen and show text visibly (allows manual mouse selection)
        use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
        use crossterm::execute;
        use std::io::Write;

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

    /// Called every tick (1s)
    pub fn update(&mut self) {
        // Drain agent messages
        self.drain_agent_messages();

        // Refresh collectors
        if let Some(ref cfg) = self.config_snapshot {
            self.keys = collectors::keys::collect_keys(&cfg.pki_dir);
            self.processes = collectors::processes::collect_processes();
            self.services = collectors::services::collect_services();
            self.zmq_connections = collectors::connections::collect_connections_with_api(cfg.publish_port, cfg.ret_port, cfg.api_port);
            self.system_info = collectors::system::collect_system(&cfg.cachedir, &cfg.pki_dir);
        }

        // Calculate events/sec
        let now = Instant::now();
        let elapsed = now.duration_since(self.events_last_sec_time).as_secs_f64();
        if elapsed >= 1.0 {
            self.events_per_sec = self.events_last_sec_count as f64 / elapsed;
            self.events_last_sec_count = 0;
            self.events_last_sec_time = now;
            // Record into ring buffer for sparkline
            self.event_rate_history[self.event_rate_idx] = self.events_per_sec;
            self.event_rate_idx = (self.event_rate_idx + 1) % 60;
        }
        self.events_last_sec_count += self.events_since_last_tick.len() as u64;
        self.events_since_last_tick.clear();

        // Seconds since last event
        self.secs_since_last_event = self.last_event_time
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(u64::MAX);

        // Config refresh + ZMQ latency ping
        self.config_tick += 1;
        if self.config_tick >= self.config.config_secs {
            self.config_tick = 0;
            self.send_agent_cmd("config_snapshot", None);
            self.send_agent_cmd("zmq_ping", None);
        }
        // Also ping early on first few ticks to show data sooner
        if self.tick_count == 0 || self.tick_count == 5 || self.tick_count == 15 {
            self.send_agent_cmd("zmq_ping", None);
        }

        // Alert engine checks
        self.alert_engine.tick(
            &self.minion_info,
            &self.keys,
            &self.services,
            &self.system_info,
            self.config.stale_minion_secs,
            self.config.pending_key_secs,
            self.config.disk_usage_pct,
        );

        // Age out old jobs
        let max_age = self.config.config_secs * 60; // 60 min default
        self.jobs.retain(|j| j.age_seconds < max_age);

        // Clamp cursor selections to valid range
        self.clamp_selection();

        self.tick_count += 1;
    }

    pub fn handle_key(&mut self, code: KeyCode) -> bool {
        // Help overlay handles its own dismiss
        if self.show_help {
            match code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Char('?') => {
                    self.show_help = false;
                }
                _ => {}
            }
            return false;
        }

        // Detail popup gets first crack
        if let Some(ref mut detail) = self.detail_popup {
            match *detail {
                DetailPopup::JobDetail { ref mut selected_return, ref lookup, .. } => {
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') => {
                            self.detail_popup = None;
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if let Some(ref l) = lookup {
                                if !l.returns.is_empty() {
                                    let cur = selected_return.unwrap_or(0);
                                    *selected_return = Some(cur.saturating_sub(1));
                                }
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if let Some(ref l) = lookup {
                                if !l.returns.is_empty() {
                                    let cur = selected_return.unwrap_or(0) + 1;
                                    *selected_return = Some(cur.min(l.returns.len().saturating_sub(1)));
                                }
                            }
                        }
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            if let Some(ref l) = lookup {
                                let idx = selected_return.unwrap_or(0);
                                if idx < l.returns.len() {
                                    let ret = &l.returns[idx];
                                    let payload = serde_json::to_string_pretty(&ret.full_return)
                                        .unwrap_or_else(|_| ret.summary.clone());
                                    self.copy_to_clipboard(&payload);
                                    self.clipboard_at = Some(Instant::now());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {
                    match code {
                        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                            self.detail_popup = None;
                        }
                        _ => {}
                    }
                }
            }
            return false;
        }

        // Filter input mode
        if self.is_filtering {
            match code {
                KeyCode::Esc => {
                    self.is_filtering = false;
                    self.filter_text.clear();
                    self.clamp_selection();
                }
                KeyCode::Enter => {
                    self.is_filtering = false;
                    self.event_selected = 0;
                    self.job_selected = 0;
                    self.alert_selected = 0;
                    self.minion_selected = 0;
                    self.clamp_selection();
                }
                KeyCode::Backspace => {
                    self.filter_text.pop();
                    self.clamp_selection();
                }
                KeyCode::Char(c) => {
                    self.filter_text.push(c);
                    self.clamp_selection();
                }
                _ => {}
            }
            return false;
        }

        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => return true,
            KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::Char('H') => {
                self.show_help = true;
            }
            KeyCode::Char('/') => {
                self.is_filtering = true;
                self.filter_text.clear();
            }
            KeyCode::Tab => self.current_tab = self.current_tab.next(),
            KeyCode::BackTab => self.current_tab = self.current_tab.prev(),
            KeyCode::Char('1') => self.current_tab = Tab::Dashboard,
            KeyCode::Char('2') => self.current_tab = Tab::Minions,
            KeyCode::Char('3') => self.current_tab = Tab::Events,
            KeyCode::Char('4') => self.current_tab = Tab::Jobs,
            KeyCode::Char('5') => self.current_tab = Tab::Alerts,
            KeyCode::Up => self.move_up(),
            KeyCode::Down => self.move_down(),
            KeyCode::PageUp => { for _ in 0..10 { self.move_up(); } }
            KeyCode::PageDown => { for _ in 0..10 { self.move_down(); } }
            KeyCode::Enter => self.open_detail(),
            _ => {}
        }
        false
    }

    fn move_up(&mut self) {
        match self.current_tab {
            Tab::Minions => self.minion_selected = self.minion_selected.saturating_sub(1),
            Tab::Events => self.event_selected = self.event_selected.saturating_sub(1),
            Tab::Jobs => self.job_selected = self.job_selected.saturating_sub(1),
            Tab::Alerts => self.alert_selected = self.alert_selected.saturating_sub(1),
            _ => {}
        }
    }

    fn move_down(&mut self) {
        match self.current_tab {
            Tab::Minions => self.minion_selected += 1,
            Tab::Events => self.event_selected += 1,
            Tab::Jobs => self.job_selected += 1,
            Tab::Alerts => self.alert_selected += 1,
            _ => {}
        }
    }
}

fn parse_version(data: &serde_json::Value) -> ConfigSnapshot {
    ConfigSnapshot {
        salt_version: data["salt_version"].as_str().unwrap_or("?").to_string(),
        python_version: data["python_version"].as_str().unwrap_or("?").to_string(),
        ..Default::default()
    }
}

fn parse_config_snapshot(data: &serde_json::Value) -> ConfigSnapshot {
    ConfigSnapshot {
        salt_version: data["salt_version"].as_str().unwrap_or("?").to_string(),
        python_version: data["python_version"].as_str().unwrap_or("?").to_string(),
        worker_threads: data["worker_threads"].as_i64(),
        transport: data["transport"].as_str().unwrap_or("zeromq").to_string(),
        sock_dir: data["sock_dir"].as_str().unwrap_or("").to_string(),
        reactor_count: data["reactor_count"].as_u64().unwrap_or(0) as usize,
        fileserver_backends: data["fileserver_backends"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        gitfs_remote_count: data["gitfs_remote_count"].as_u64().unwrap_or(0) as usize,
        ext_pillar_names: data["ext_pillar_names"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        master_job_cache: data["master_job_cache"].as_str().unwrap_or("local").to_string(),
        pillar_cache: data["pillar_cache"].as_bool().unwrap_or(false),
        presence_events: data["presence_events"].as_bool().unwrap_or(false),
        interface: data["interface"].as_str().unwrap_or("0.0.0.0").to_string(),
        publish_port: data["publish_port"].as_u64().unwrap_or(4505) as u16,
        ret_port: data["ret_port"].as_u64().unwrap_or(4506) as u16,
        api_port: data["api_port"].as_u64().unwrap_or(0) as u16,
        pki_dir: data["pki_dir"].as_str().unwrap_or("/etc/salt/pki/master").to_string(),
        cachedir: data["cachedir"].as_str().unwrap_or("/var/cache/salt/master").to_string(),
        cpu_count: data["cpu_count"].as_u64().unwrap_or(0) as u32,
    }
}

fn parse_minion_list(data: &serde_json::Value) -> Vec<MinionInfo> {
    let mut result = Vec::new();
    if let Some(arr) = data["minions"].as_array() {
        for entry in arr {
            result.push(MinionInfo {
                id: entry["id"].as_str().unwrap_or("?").to_string(),
                os: entry["os"].as_str().unwrap_or("").to_string(),
                osrelease: entry["osrelease"].as_str().unwrap_or("").to_string(),
                salt_version: entry["saltversion"].as_str().unwrap_or("").to_string(),
                cpuarch: entry["cpuarch"].as_str().unwrap_or("").to_string(),
                last_seen_epoch: entry["last_seen"].as_u64().unwrap_or(0),
                cache_updated_epoch: entry["cache_updated"].as_u64().unwrap_or(0),
                ..Default::default()
            });
        }
    }
    result
}

fn parse_jid_lookup(data: &serde_json::Value) -> JidLookup {
    let mut result = JidLookup {
        found: data["found"].as_bool().unwrap_or(false),
        fun: data["fun"].as_str().unwrap_or("").to_string(),
        tgt: data["tgt"].as_str().unwrap_or("").to_string(),
        start_time: data["start_time"].as_str().unwrap_or("").to_string(),
        returns: Vec::new(),
    };
    if let Some(arr) = data["returns"].as_array() {
        for entry in arr {
            result.returns.push(JidReturn {
                minion_id: entry["minion_id"].as_str().unwrap_or("?").to_string(),
                success: entry["success"].as_bool().unwrap_or(false),
                retcode: entry["retcode"].as_i64().unwrap_or(0),
                summary: entry["summary"].as_str().unwrap_or("").to_string(),
                full_return: entry["full_return"].clone(),
            });
        }
    }
    result
}

fn track_job_from_event(jobs: &mut Vec<JobEntry>, evt: &SaltEvent) {
    if evt.tag.ends_with("/new") {
        let jid = evt.data["jid"].as_str().unwrap_or("").to_string();
        if jid.is_empty() { return; }
        if jobs.iter().any(|j| j.jid == jid) { return; }
        let fun = evt.data["fun"].as_str().unwrap_or("?").to_string();
        let tgt = evt.data["tgt"].as_str().unwrap_or("?").to_string();
        let minions = evt.data["minions"].as_array().map(|a| a.len()).unwrap_or(0);
        jobs.push(JobEntry {
            jid,
            fun,
            tgt,
            target_count: minions,
            returned: 0,
            age_seconds: 0,
            created_epoch: evt.ts,
        });
    } else if evt.tag.contains("/ret/") {
        let jid = evt.tag
            .strip_prefix("salt/job/")
            .and_then(|s| s.split("/ret/").next())
            .unwrap_or("");
        if let Some(job) = jobs.iter_mut().find(|j| j.jid == jid) {
            job.returned += 1;
        }
    }
}
