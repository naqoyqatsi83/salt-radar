use std::collections::{HashSet, VecDeque};
use std::io::Read;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::process::Stdio;
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
    pub high_contrast: bool,

    // Admin mode
    pub admin_mode: bool,
    pub admin_mode_pending: bool,
    pub fleet_scanning: bool,
    pub fleet_data: Option<FleetScan>,

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
    pub event_rate_history: [f64; 2000],
    pub event_rate_idx: usize,
    /// Timestamp of last successful clipboard copy (for UI feedback)
    pub clipboard_at: Option<Instant>,
    /// Flag for main loop to call terminal.clear() after alt-screen disruption
    pub needs_terminal_clear: bool,
    /// Measured ZMQ/master response latency in milliseconds (from zmq_ping agent command)
    pub zmq_latency_ms: f64,
    /// When the last zmq_ping was sent (for Rust-side round-trip measurement)
    zmq_ping_sent_at: Option<Instant>,
    /// Ring buffer of recent latency samples for sparkline (60 samples = 1 per tick)
    pub zmq_latency_history: [f64; 2000],
    pub zmq_latency_idx: usize,

    // ── Activity matrix (job return events per sec) ──────────────────────────
    /// Count of salt/job/*/ret/* events per second for last 60s (ring buffer)
    pub activity_matrix: [u64; 2000],
    pub activity_matrix_idx: usize,
    pub current_sec_returns: u64,
    pub last_activity_minion: String,
    pub last_activity_fun: String,
    pub last_activity_at: Instant,
    /// Animation frame index for radar spinner in header
    pub header_radar_frame: u8,

    /// User-adjustable tick interval in milliseconds (default 2000 = 2s)
    pub tick_interval_ms: u64,

    // Sniffer
    pub sniffer_input_active: bool,
    pub sniffer_input_text: String,
    pub sniffer_dirty: bool,
    /// When viewing event detail from sniffer, pressing q returns to sniffer popup
    pub return_to_sniffer: Option<SnifferState>,

    /// Hide timed-out (non-responsive) minions from list
    pub hide_timed_out: bool,

    /// Fleet scan async tracking (JID from cmd_async + event-driven returns)
    pub fleet_scan_jid: Option<String>,
    pub fleet_scan_expected: Vec<String>,
    pub fleet_scan_responded: HashSet<String>,
    pub fleet_scan_started_at: Option<Instant>,
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
    JobDetail { jid: String, lookup: Option<JidLookup>, selected_return: Option<usize>, scroll_offset: usize, scroll_mode: bool, job_fun: String, job_tgt: String, job_start_epoch: f64 },
    Alert(AlertEntry),
    AdminConfirm,
    EventSniffer(crate::types::SnifferState),
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
            high_contrast: false,

            sniffer_input_active: false,
            sniffer_input_text: String::new(),
            sniffer_dirty: false,
            return_to_sniffer: None,
            hide_timed_out: false,
            fleet_scan_jid: None,
            fleet_scan_expected: Vec::new(),
            fleet_scan_responded: HashSet::new(),
            fleet_scan_started_at: None,

            admin_mode: false,
            admin_mode_pending: false,
            fleet_scanning: false,
            fleet_data: None,

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
            event_rate_history: [0.0; 2000],
            event_rate_idx: 0,
            clipboard_at: None,
            needs_terminal_clear: false,
            zmq_latency_ms: 0.0,
            zmq_ping_sent_at: None,
            zmq_latency_history: [0.0; 2000],
            zmq_latency_idx: 0,

            activity_matrix: [0; 2000],
            activity_matrix_idx: 0,
            current_sec_returns: 0,
            last_activity_minion: String::new(),
            last_activity_fun: String::new(),
            last_activity_at: Instant::now(),
            header_radar_frame: 0,

            tick_interval_ms: 2000,
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
                            if let Some(sent) = self.zmq_ping_sent_at {
                                let elapsed_ms = sent.elapsed().as_secs_f64() * 1000.0;
                                self.zmq_latency_ms = elapsed_ms;
                                self.zmq_latency_history[self.zmq_latency_idx] = elapsed_ms;
                                self.zmq_latency_idx = (self.zmq_latency_idx + 1) % 2000;
                            }
                            self.zmq_ping_sent_at = None;
                        }
                        "fleet_scan" => {
                            if ok {
                                let jid = data["jid"].as_str().unwrap_or("").to_string();
                                let expected: Vec<String> = data["expected_minions"].as_array()
                                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                                    .unwrap_or_default();
                                self.fleet_scan_jid = Some(jid);
                                self.fleet_scan_expected = expected;
                                self.fleet_scan_responded.clear();
                                self.fleet_scan_started_at = Some(Instant::now());
                                self.fleet_scanning = true;
                                self.fleet_data = Some(FleetScan {
                                    minions: Vec::new(),
                                    scanned_at: Instant::now(),
                                });
                            } else {
                                self.fleet_scanning = false;
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
                    // ── Normal event processing ─────────────────────────
                    // Track return events for activity matrix
                    if tag.contains("/ret/") {
                        self.current_sec_returns += 1;
                        if let Some(mid) = tag.rsplit('/').next() {
                            if !mid.is_empty() {
                                self.last_activity_minion = mid.to_string();
                                self.last_activity_fun = data["fun"].as_str().unwrap_or("?").to_string();
                                self.last_activity_at = Instant::now();
                                // Update last_seen for this minion from return events
                                let ts_epoch = ts as u64;
                                if let Some(info) = self.minion_info.iter_mut().find(|m| m.id == mid) {
                                    if ts_epoch > info.last_seen_epoch {
                                        info.last_seen_epoch = ts_epoch;
                                    }
                                }
                            }
                        }
                    }
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

                    // Sniffer: inject matching events into sniffer buffer
                    if let Some(DetailPopup::EventSniffer(ref mut state)) = self.detail_popup {
                        if !state.paused {
                            if let Some(evt) = self.events.back() {
                                if Self::matches_sniffer_event(evt, &state.minion_ids) {
                                    if state.buffer.len() >= 1000 {
                                        state.buffer.pop_front();
                                    }
                                    state.buffer.push_back(evt.clone());
                                    state.selected = 0;
                                    self.sniffer_dirty = true;
                                }
                            }
                        }
                    }
                }
                AgentMessage::Error { message, .. } => {
                    self.agent.last_error = Some(message);
                    self.agent.connected = false;
                }
            }
        }
    }

    /// Send a command to the agent, returns the request id.
    /// Appends to the command file (avoids overwriting a concurrently-written command).
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
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .write(true)
            .open("/tmp/salt-radar-cmd")
            .and_then(|f| {
                let mut f = std::io::LineWriter::new(f);
                f.write_all(payload.as_bytes())
            });
        id
    }

    /// Fast poll between ticks
    pub fn fast_poll(&mut self) -> bool {
        self.drain_agent_messages();
        if self.sniffer_dirty {
            self.sniffer_dirty = false;
            true
        } else {
            false
        }
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

    fn matches_event(evt: &SaltEvent, pat: &str, known_minions: &HashSet<&str>) -> bool {
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

    fn matches_sniffer_event(evt: &SaltEvent, minion_ids: &[String]) -> bool {
        for mid in minion_ids {
            if evt.tag.contains(mid.as_str()) { return true; }
            if let Some(id) = evt.data["id"].as_str() {
                if id.contains(mid.as_str()) || mid.contains(id) { return true; }
            }
            if let Some(tgt) = evt.data["tgt"].as_str() {
                if tgt == "*" || tgt.contains(mid.as_str()) || mid.contains(tgt) { return true; }
            }
            if Self::sniffer_search_value(&evt.data, mid) { return true; }
        }
        false
    }

    fn sniffer_search_value(value: &serde_json::Value, pattern: &str) -> bool {
        match value {
            serde_json::Value::String(s) => s.contains(pattern),
            serde_json::Value::Object(map) => map.values().any(|v| Self::sniffer_search_value(v, pattern)),
            serde_json::Value::Array(arr) => arr.iter().any(|v| Self::sniffer_search_value(v, pattern)),
            _ => false,
        }
    }

    fn matches_job(job: &JobEntry, pat: &str, known_minions: &HashSet<&str>) -> bool {
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

    fn accepted_ids(&self) -> HashSet<&str> {
        self.keys.iter()
            .filter(|k| k.status == KeyStatus::Accepted)
            .map(|k| k.id.as_str())
            .collect()
    }

    pub fn filtered_events_len(&self) -> usize {
        if self.filter_text.is_empty() {
            return self.events.len();
        }
        let pat = &self.filter_text;
        let known = self.accepted_ids();
        self.events.iter().rev().filter(|e| Self::matches_event(e, pat, &known)).count()
    }

    pub fn get_filtered_event(&self, index: usize) -> Option<&SaltEvent> {
        if self.filter_text.is_empty() {
            let rev_idx = self.events.len().saturating_sub(1).saturating_sub(index);
            return self.events.get(rev_idx);
        }
        let pat = &self.filter_text;
        let known = self.accepted_ids();
        self.events.iter().rev().filter(|e| Self::matches_event(e, pat, &known)).nth(index)
    }

    pub fn filtered_jobs_len(&self) -> usize {
        if self.filter_text.is_empty() {
            return self.jobs.len();
        }
        let pat = &self.filter_text;
        let known = self.accepted_ids();
        self.jobs.iter().filter(|j| Self::matches_job(j, pat, &known)).count()
    }

    pub fn get_filtered_job(&self, index: usize) -> Option<&JobEntry> {
        if self.filter_text.is_empty() {
            return self.jobs.get(index);
        }
        let pat = &self.filter_text;
        let known = self.accepted_ids();
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
        let accepted: HashSet<&str> = self.keys.iter()
            .filter(|k| k.status == KeyStatus::Accepted)
            .map(|k| k.id.as_str())
            .collect();
        let hide = self.hide_timed_out;
        let pat = &self.filter_text;
        self.minion_info.iter()
            .filter(|m| accepted.contains(m.id.as_str()))
            .filter(|m| !hide || !m.timed_out)
            .filter(|m| pat.is_empty() || Self::matches_minion(m, pat))
            .count()
    }

    pub fn get_filtered_minion(&self, index: usize) -> Option<&MinionInfo> {
        let accepted: HashSet<&str> = self.keys.iter()
            .filter(|k| k.status == KeyStatus::Accepted)
            .map(|k| k.id.as_str())
            .collect();
        let hide = self.hide_timed_out;
        let pat = &self.filter_text;
        self.minion_info.iter()
            .filter(|m| accepted.contains(m.id.as_str()))
            .filter(|m| !hide || !m.timed_out)
            .filter(|m| pat.is_empty() || Self::matches_minion(m, pat))
            .nth(index)
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
                    let job_fun = job.fun.clone();
                    let job_tgt = job.tgt.clone();
                    let job_start_epoch = job.created_epoch;
                    let req_id = self.send_agent_cmd("jid_lookup", Some(serde_json::json!({"jid": jid.clone()})));
                    self.pending_jid_lookup = Some((req_id, jid.clone()));
                    self.detail_popup = Some(DetailPopup::JobDetail {
                        jid,
                        lookup: None,
                        selected_return: None,
                        scroll_offset: 0,
                        scroll_mode: false,
                        job_fun,
                        job_tgt,
                        job_start_epoch,
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

    /// Format a job return value in salt's nested outputter format.
    /// Example:
    ///   syn99a08ld015:
    ///       ----------
    ///       osfinger:
    ///           Ubuntu-22.04
    pub fn format_return_as_console(minion_id: &str, value: &serde_json::Value) -> String {
        fn fmt_val(v: &serde_json::Value, indent: usize) -> String {
            let pad = " ".repeat(indent);
            match v {
                serde_json::Value::Null => format!("{}null", pad),
                serde_json::Value::Bool(b) => format!("{}{}", pad, b),
                serde_json::Value::Number(n) => format!("{}{}", pad, n),
                serde_json::Value::String(s) => format!("{}{}", pad, s),
                serde_json::Value::Array(arr) => {
                    if arr.is_empty() { return format!("{}[]", pad); }
                    arr.iter().map(|e| fmt_val(e, indent)).collect::<Vec<_>>().join("\n")
                }
                serde_json::Value::Object(obj) => {
                    if obj.is_empty() { return format!("{}{{}}", pad); }
                    let mut out = String::new();
                    let mut first = true;
                    for (k, v) in obj.iter() {
                        // Skip Salt internal keys like __id__, __run_num__, __sls__
                        if k.starts_with("__") { continue; }
                        if !first { out.push('\n'); }
                        first = false;
                        out.push_str(&format!("{}----------\n{}{}:\n", pad, pad, k));
                        out.push_str(&fmt_val(v, indent + 4));
                    }
                    if first { return format!("{}{{}}", pad); }
                    out
                }
            }
        }
        // Top level: just the minion header then the nested body
        format!("{}:\n{}", minion_id, fmt_val(value, 4))
    }

    fn tmux_socket_path() -> Option<String> {
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
    fn run_cmd_with_stdin(
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

    pub fn copy_to_clipboard(&mut self, text: &str) {
        // 1. Write to local file (always works, easy debug)
        let _ = std::fs::write("/tmp/salt-radar-copy.txt", text);

        // 2. tmux load-buffer (for Ctrl+b ] paste inside tmux).
        let ok_tmux = if let Some(socket) = Self::tmux_socket_path() {
            let (ok, _) = Self::run_cmd_with_stdin(
                std::process::Command::new("tmux").args(["-S", &socket, "load-buffer", "-"]),
                text, 3,
            );
            ok
        } else { false };

        // 3. xclip (pushes to X11 clipboard → MobaXterm → Windows).
        let display = std::env::var("DISPLAY").unwrap_or_else(|_| "localhost:10.0".to_string());
        let x11_user = Self::detect_x11_user(&display);

        let mut ok_xclip = false;
        // Direct xclip as root — only when DISPLAY was actually set by the
        // environment (not the localhost:10.0 fallback). Root has the SSH
        // X11 auth cookie when X11 forwarding is active.
        if let Ok(d) = std::env::var("DISPLAY") {
            if !d.is_empty() {
                let (ok, _) = Self::run_cmd_with_stdin(
                    std::process::Command::new("xclip")
                        .args(["-selection", "clipboard", "-display", &display]),
                    text, 3,
                );
                if ok { ok_xclip = true; }
            }
        }
        if !ok_xclip {
            let users_to_try = [&x11_user[..], "vagrant", "peto"];
            for user in users_to_try {
                if !std::process::Command::new("id")
                    .arg(user).stdout(Stdio::null()).stderr(Stdio::null())
                    .status().map(|s| s.success()).unwrap_or(false)
                {
                    continue;
                }
                let (ok, _) = Self::run_cmd_with_stdin(
                    std::process::Command::new("su")
                        .args(["-c", &format!("DISPLAY={} xclip -selection clipboard", display), user]),
                    text, 3,
                );
                if ok { ok_xclip = true; break; }
            }
        }

        // 4. Debug log
        let _ = std::fs::write(
            "/tmp/salt-radar-clip.log",
            format!("file={}b tmux={} xclip={} display={}", text.len(), if ok_tmux { "ok" } else { "skip" }, if ok_xclip { "ok" } else { "fail" }, display),
        );

        // 5. Leave alt screen and show text visibly
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

    /// Tries to find the unprivileged user that owns the X11 session by
    /// checking ownership of /tmp/.X11-unix/X<display_number>.
    fn detect_x11_user(display: &str) -> String {
        // Extract display number from "localhost:10.0" or ":1" or "10" etc.
        let dnum = display
            .trim_start_matches("localhost:")
            .trim_start_matches(':')
            .split('.')   // strip screen number: "10.0" → "10"
            .next()
            .unwrap_or("")
            .trim();
        if dnum.is_empty() {
            return "peto".to_string();
        }
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
        "peto".to_string()
    }

    /// Scan events buffer for fleet scan returns matching the known JID
    fn process_fleet_returns(&mut self) {
        let jid = match &self.fleet_scan_jid {
            Some(j) => j.clone(),
            None => return,
        };
        let prefix = format!("salt/job/{}/ret/", jid);
        for evt in &self.events {
            if !evt.tag.starts_with(&prefix) {
                continue;
            }
            let mid = match evt.data["id"].as_str() {
                Some(m) if !m.is_empty() => m.to_string(),
                _ => continue,
            };
            if evt.data["fun"].as_str() != Some("grains.items") {
                continue;
            }
            let grains = &evt.data["return"];
            if !grains.is_object() {
                continue;
            }
            let osfinger = grains.get("osfinger").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let osrelease = grains.get("osrelease").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let saltversion = grains.get("saltversion").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let cpuarch = grains.get("cpuarch").and_then(|v| v.as_str()).unwrap_or("").to_string();

            // Merge into minion_info
            if let Some(existing) = self.minion_info.iter_mut().find(|m| m.id == mid) {
                existing.osfinger = osfinger.clone();
                existing.osrelease = osrelease.clone();
                existing.salt_version = saltversion.clone();
                existing.cpuarch = cpuarch.clone();
                existing.timed_out = false;
            } else {
                self.minion_info.push(MinionInfo {
                    id: mid.clone(),
                    osfinger: osfinger.clone(),
                    osrelease: osrelease.clone(),
                    salt_version: saltversion.clone(),
                    cpuarch: cpuarch.clone(),
                    ..Default::default()
                });
            }

            // Merge into fleet_data for overview
            if let Some(ref mut fd) = self.fleet_data {
                let fm = FleetMinion {
                    id: mid.clone(),
                    osfinger,
                    osrelease,
                    saltversion,
                    cpuarch,
                    timed_out: false,
                };
                if let Some(existing) = fd.minions.iter_mut().find(|m| m.id == mid) {
                    *existing = fm;
                } else {
                    fd.minions.push(fm);
                }
            }

            // Track as responded and remove from expected list
            self.fleet_scan_responded.insert(mid.clone());
            if !self.fleet_scan_expected.is_empty() {
                self.fleet_scan_expected.retain(|e| *e != mid);
            }
        }
    }

    /// Called every tick (1s)
    pub fn update(&mut self) {
        // Drain agent messages (must happen every tick)
        self.drain_agent_messages();

        // Process fleet scan returns from events buffer
        self.process_fleet_returns();

        // Fleet scan timeout: mark minions that haven't responded as timed_out after 60s
        if self.fleet_scanning {
            if let Some(started) = self.fleet_scan_started_at {
                if started.elapsed().as_secs() > 60 {
                    // Find all minions that should have responded but didn't.
                    // Source 1: accepted keys from Rust key cache.
                    // Source 2: agent-provided expected list (fallback if keys cache is empty).
                    let accepted: HashSet<&str> = self.keys.iter()
                        .filter(|k| k.status == KeyStatus::Accepted)
                        .map(|k| k.id.as_str())
                        .collect();
                    let to_check: Vec<String> = if !accepted.is_empty() {
                        accepted.into_iter().map(String::from).collect()
                    } else {
                        std::mem::take(&mut self.fleet_scan_expected)
                    };
                    for mid in &to_check {
                        if self.fleet_scan_responded.contains(mid.as_str()) {
                            continue;
                        }
                        // Mark in minion_info
                        if let Some(existing) = self.minion_info.iter_mut().find(|m| m.id.as_str() == mid) {
                            existing.timed_out = true;
                        } else {
                            self.minion_info.push(MinionInfo {
                                id: mid.clone(),
                                timed_out: true,
                                ..Default::default()
                            });
                        }
                        // Mark in fleet_data
                        if let Some(ref mut fd) = self.fleet_data {
                            let fm = FleetMinion {
                                id: mid.clone(),
                                timed_out: true,
                                ..Default::default()
                            };
                            if let Some(existing) = fd.minions.iter_mut().find(|m| m.id == *mid) {
                                *existing = fm;
                            } else {
                                fd.minions.push(fm);
                            }
                        }
                    }
                    self.fleet_scan_jid = None;
                    self.fleet_scan_responded.clear();
                    self.fleet_scan_expected.clear();
                    self.fleet_scan_started_at = None;
                    self.fleet_scanning = false;
                }
            }
        }

        // Sort minions: alive first, timed-out last, each group alphabetically by ID
        self.minion_info.sort_by(|a, b| {
            a.timed_out
                .cmp(&b.timed_out)
                .then(a.id.cmp(&b.id))
        });

        // Refresh collectors at interval, not every tick
        if let Some(ref cfg) = self.config_snapshot {
            let interval = self.config.dashboard_secs.max(1);
            if self.tick_count % interval == 0 {
                self.keys = collectors::keys::collect_keys(&cfg.pki_dir);
                self.processes = collectors::processes::collect_processes();
                self.services = collectors::services::collect_services();
                self.system_info = collectors::system::collect_system(&cfg.cachedir, &cfg.pki_dir);
            }
            // Connections are cheap — /proc/net/tcp reads — refresh every tick
            self.zmq_connections = collectors::connections::collect_connections_with_api(cfg.publish_port, cfg.ret_port, cfg.api_port);
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
            self.event_rate_idx = (self.event_rate_idx + 1) % 2000;

            // Activity matrix: store return count for this second
            self.activity_matrix[self.activity_matrix_idx] = self.current_sec_returns;
            self.activity_matrix_idx = (self.activity_matrix_idx + 1) % 2000;
            self.current_sec_returns = 0;

            // Header radar animation cycle
            self.header_radar_frame = (self.header_radar_frame + 1) % 4;
        }
        self.events_last_sec_count += self.events_since_last_tick.len() as u64;
        self.events_since_last_tick.clear();


        // Update host uptime from collector
        if let Some(ref cfg) = self.config_snapshot {
            self.system_info = collectors::system::collect_system(&cfg.cachedir, &cfg.pki_dir);
        }

        // Seconds since last event
        self.secs_since_last_event = self.last_event_time
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(u64::MAX);

        // Config refresh (keeps at config_secs interval)
        self.config_tick += 1;
        if self.config_tick >= self.config.config_secs {
            self.config_tick = 0;
            self.send_agent_cmd("config_snapshot", None);
        }

        // ZMQ latency ping every tick (same cadence as event rate / activity)
        self.zmq_ping_sent_at = Some(Instant::now());
        self.send_agent_cmd("zmq_ping", None);

        // Alert engine checks
        let recent_events: Vec<SaltEvent> = self.events.iter().rev().take(500).cloned().collect();
        self.alert_engine.tick(
            &self.minion_info,
            &self.keys,
            &self.services,
            &self.system_info,
            &recent_events,
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

        // Sniffer input mode (text entry before launching)
        if self.sniffer_input_active {
            match code {
                KeyCode::Esc => {
                    self.sniffer_input_active = false;
                    self.sniffer_input_text.clear();
                }
                KeyCode::Enter => {
                    let ids: Vec<String> = self.sniffer_input_text
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !ids.is_empty() {
                        self.detail_popup = Some(DetailPopup::EventSniffer(
                            crate::types::SnifferState {
                                minion_ids: ids,
                                buffer: std::collections::VecDeque::with_capacity(1000),
                                paused: false,
                                selected: 0,
                            }
                        ));
                    }
                    self.sniffer_input_active = false;
                    self.sniffer_input_text.clear();
                }
                KeyCode::Backspace => {
                    self.sniffer_input_text.pop();
                }
                KeyCode::Char(c) => {
                    self.sniffer_input_text.push(c);
                }
                _ => {}
            }
            return false;
        }

        // Detail popup gets first crack
        let mut clipboard_text: Option<String> = None;
        let had_popup = self.detail_popup.is_some();

        // Contrast toggle works universally (even with popups open)
        if had_popup {
            match code {
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    self.high_contrast = !self.high_contrast;
                    return false;
                }
                _ => {}
            }
        }

        if let Some(ref mut detail) = self.detail_popup {
            match *detail {
                DetailPopup::JobDetail {
                    ref mut selected_return,
                    ref mut scroll_offset,
                    ref mut scroll_mode,
                    ref lookup, ..
                } => {
                    if *scroll_mode {
                        match code {
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                                *scroll_mode = false;
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                *scroll_offset = scroll_offset.saturating_sub(1);
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                *scroll_offset += 1;
                            }
                            KeyCode::PageUp => {
                                *scroll_offset = scroll_offset.saturating_sub(10);
                            }
                            KeyCode::PageDown => {
                                *scroll_offset += 10;
                            }
                            _ => {}
                        }
                    } else {
                        match code {
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                                self.detail_popup = None;
                            }
                            KeyCode::Enter => {
                                if lookup.is_some() {
                                    *scroll_mode = true;
                                    *scroll_offset = 0;
                                }
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                if let Some(ref l) = lookup {
                                    if !l.returns.is_empty() {
                                        let cur = selected_return.unwrap_or(0);
                                        *selected_return = Some(cur.saturating_sub(1));
                                        *scroll_mode = false;
                                        *scroll_offset = 0;
                                    }
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                if let Some(ref l) = lookup {
                                    if !l.returns.is_empty() {
                                        let cur = selected_return.unwrap_or(0) + 1;
                                        *selected_return = Some(cur.min(l.returns.len().saturating_sub(1)));
                                        *scroll_mode = false;
                                        *scroll_offset = 0;
                                    }
                                }
                            }
                            KeyCode::Char('y') => {
                                if let Some(ref l) = lookup {
                                    let idx = selected_return.unwrap_or(0);
                                    if idx < l.returns.len() {
                                        let ret = &l.returns[idx];
                                        clipboard_text = Some(
                                            if !ret.formatted_text.is_empty() {
                                                ret.formatted_text.clone()
                                            } else {
                                                Self::format_return_as_console(&ret.minion_id, &ret.full_return)
                                            }
                                        );
                                    }
                                }
                            }
                            KeyCode::Char('Y') => {
                                if let Some(ref l) = lookup {
                                    let idx = selected_return.unwrap_or(0);
                                    if idx < l.returns.len() {
                                        let ret = &l.returns[idx];
                                        clipboard_text = Some(
                                            ret.full_return.as_str().unwrap_or(&ret.summary).to_string()
                                        );
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                DetailPopup::AdminConfirm => {
                    match code {
                        KeyCode::Enter => {
                            self.admin_mode = true;
                            self.admin_mode_pending = false;
                            self.detail_popup = None;
                        }
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Char('a') | KeyCode::Char('A') => {
                            self.admin_mode_pending = false;
                            self.detail_popup = None;
                        }
                        _ => {}
                    }
                }
                DetailPopup::EventSniffer(ref mut state) => {
                    match code {
                        KeyCode::Enter => {
                            let rev_idx = state.buffer.len().saturating_sub(1).saturating_sub(state.selected);
                            if let Some(evt) = state.buffer.get(rev_idx) {
                                self.return_to_sniffer = Some(state.clone());
                                self.detail_popup = Some(DetailPopup::SaltEvent(evt.clone()));
                            }
                        }
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            self.detail_popup = None;
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            state.selected = state.selected.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let max = state.buffer.len().saturating_sub(1);
                            state.selected = state.selected.saturating_add(1).min(max);
                        }
                        KeyCode::PageUp => {
                            state.selected = state.selected.saturating_sub(10);
                        }
                        KeyCode::PageDown => {
                            let max = state.buffer.len().saturating_sub(1);
                            state.selected = state.selected.saturating_add(10).min(max);
                        }
                        KeyCode::Char('p') | KeyCode::Char('P') => {
                            state.paused = !state.paused;
                        }
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            let mut text = String::new();
                            for (i, evt) in state.buffer.iter().enumerate() {
                                let ts = chrono::DateTime::from_timestamp(evt.ts as i64, 0)
                                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                                    .unwrap_or_else(|| evt.ts.to_string());
                                text.push_str(&format!(
                                    "Event {}\nTag: {}\nTimestamp: {}\nData:\n{}\n\n",
                                    i + 1,
                                    evt.tag,
                                    ts,
                                    serde_json::to_string_pretty(&evt.data).unwrap_or_default()
                                ));
                            }
                            if !text.is_empty() {
                                clipboard_text = Some(text);
                            }
                        }
                        _ => {}
                    }
                }
                DetailPopup::SaltEvent(ref evt) => {
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            if let Some(sniffer) = self.return_to_sniffer.take() {
                                self.detail_popup = Some(DetailPopup::EventSniffer(sniffer));
                            } else {
                                self.detail_popup = None;
                            }
                        }
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            let ts = chrono::DateTime::from_timestamp(evt.ts as i64, 0)
                                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                                .unwrap_or_else(|| evt.ts.to_string());
                            clipboard_text = Some(format!(
                                "Tag: {}\nTimestamp: {}\n\nData:\n{}",
                                evt.tag,
                                ts,
                                serde_json::to_string_pretty(&evt.data).unwrap_or_default()
                            ));
                        }
                        _ => {}
                    }
                }
                DetailPopup::Alert(ref alert) => {
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            self.detail_popup = None;
                        }
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            let mut text = format!(
                                "Alert: {}\nSeverity: {}\nRule: {}\n{}\n",
                                alert.key,
                                alert.severity.label(),
                                alert.rule,
                                alert.description,
                            );
                            if !alert.details.is_empty() {
                                text.push_str("\nDetails:\n");
                                for d in &alert.details {
                                    text.push_str(&format!("  {}\n", d));
                                }
                            }
                            clipboard_text = Some(text);
                        }
                        _ => {}
                    }
                }
                _ => {
                    match code {
                        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            self.detail_popup = None;
                        }
                        _ => {}
                    }
                }
            }
        }
        if let Some(text) = clipboard_text {
            self.copy_to_clipboard(&text);
            self.clipboard_at = Some(Instant::now());
            return false;
        }
        if had_popup {
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
            KeyCode::Char('=') | KeyCode::Char('+') => {
                if self.current_tab == Tab::Dashboard {
                    self.tick_interval_ms = (self.tick_interval_ms + 500).min(30_000);
                }
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                if self.current_tab == Tab::Dashboard {
                    self.tick_interval_ms = self.tick_interval_ms.saturating_sub(500).max(500);
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                if self.admin_mode {
                    self.admin_mode = false;
                } else if self.detail_popup.is_none() && !self.is_filtering {
                    self.admin_mode_pending = true;
                    self.detail_popup = Some(DetailPopup::AdminConfirm);
                }
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                if self.admin_mode && self.current_tab == Tab::Minions && !self.fleet_scanning {
                    self.fleet_scanning = true;
                    self.send_agent_cmd("fleet_scan", None);
                }
            }
            KeyCode::Char('x') | KeyCode::Char('X') => {
                if self.current_tab == Tab::Minions && self.fleet_data.is_some() {
                    self.hide_timed_out = !self.hide_timed_out;
                    self.clamp_selection();
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                if self.current_tab == Tab::Events && self.detail_popup.is_none() && !self.is_filtering {
                    self.sniffer_input_active = true;
                    self.sniffer_input_text.clear();
                }
            }
            KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::Char('H') => {
                self.show_help = true;
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.high_contrast = !self.high_contrast;
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
            KeyCode::Home => self.move_to_first(),
            KeyCode::End => self.move_to_last(),
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

    fn move_to_first(&mut self) {
        match self.current_tab {
            Tab::Minions => self.minion_selected = 0,
            Tab::Events => self.event_selected = 0,
            Tab::Jobs => self.job_selected = 0,
            Tab::Alerts => self.alert_selected = 0,
            _ => {}
        }
    }

    fn move_to_last(&mut self) {
        match self.current_tab {
            Tab::Minions => self.minion_selected = self.filtered_minions_len().saturating_sub(1),
            Tab::Events => self.event_selected = self.filtered_events_len().saturating_sub(1),
            Tab::Jobs => self.job_selected = self.filtered_jobs_len().saturating_sub(1),
            Tab::Alerts => self.alert_selected = self.filtered_alerts_len().saturating_sub(1),
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
        hostname: data["hostname"].as_str().unwrap_or("?").to_string(),
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
                osfinger: entry["osfinger"].as_str().unwrap_or("").to_string(),
                salt_version: entry["saltversion"].as_str().unwrap_or("").to_string(),
                cpuarch: entry["cpuarch"].as_str().unwrap_or("").to_string(),
                last_seen_epoch: entry["last_seen"].as_u64().unwrap_or(0),
                cache_updated_epoch: entry["cache_updated"].as_u64().unwrap_or(0),
                ..Default::default()
            });
        }
    }
    result.sort_by(|a, b| a.id.cmp(&b.id));
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
                formatted_text: entry["formatted_text"].as_str().unwrap_or("").to_string(),
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
            local: false,
        });
    } else if evt.tag.contains("/ret/") {
        let jid = evt.tag
            .strip_prefix("salt/job/")
            .and_then(|s| s.split("/ret/").next())
            .unwrap_or("");
        if jid.is_empty() { return; }
        if let Some(job) = jobs.iter_mut().find(|j| j.jid == jid) {
            job.returned += 1;
        } else {
            // Unknown JID → salt-call / local execution (no /new event was emitted)
            let fun = evt.data["fun"].as_str().unwrap_or("?").to_string();
            let tgt = evt.data["id"].as_str().unwrap_or("?").to_string();
            jobs.push(JobEntry {
                jid: jid.to_string(),
                fun,
                tgt,
                target_count: 1,
                returned: 1,
                age_seconds: 0,
                created_epoch: evt.ts,
                local: true,
            });
        }
    }
}

fn parse_fleet_scan(data: &serde_json::Value) -> Vec<FleetMinion> {
    let mut result = Vec::new();
    if let Some(arr) = data["minions"].as_array() {
        for entry in arr {
            result.push(FleetMinion {
                id: entry["id"].as_str().unwrap_or("?").to_string(),
                osfinger: entry["osfinger"].as_str().unwrap_or("").to_string(),
                osrelease: entry["osrelease"].as_str().unwrap_or("").to_string(),
                saltversion: entry["saltversion"].as_str().unwrap_or("").to_string(),
                cpuarch: entry["cpuarch"].as_str().unwrap_or("").to_string(),
                timed_out: entry["timed_out"].as_bool().unwrap_or(false),
            });
        }
    }
    result.sort_by(|a, b| a.id.cmp(&b.id));
    result
}
