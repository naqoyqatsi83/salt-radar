use std::collections::{HashSet, VecDeque};
use std::io::Read;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::process::Stdio;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
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
    /// Tab where the current popup was opened (popup only shows on that tab)
    pub popup_tab: Option<Tab>,
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
    /// Pending jid_lookup (request_id, jid) for auto-updating the popup
    pending_jid_lookup: Option<(u64, String)>,
    /// When pending_jid_lookup was started, for timeout detection
    pending_jid_lookup_at: Option<Instant>,
    /// When Y is pressed, stores (request_id, jid) so the response auto-copies JSON
    pending_y_copy: Option<(u64, String)>,
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
    /// Transient message shown in status bar (e.g. clipboard hints)
    pub clipboard_msg: Option<(String, Instant)>,
    pub sparkline_style: SparklineStyle,
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

    // Job cache
    pub jobs_cache: Vec<CacheJobEntry>,
    pub cache_selected: usize,
    pub cache_filter_text: String,
    pub cache_server_filter: String,
    pub cache_is_filtering: bool,
    pub cache_focus: bool,
    pub cache_loading: bool,
    pub cache_loaded: bool,
    pub cache_load_error: Option<String>,
    pub cache_total_count: usize,
    pub cache_max_results: usize,
    pub cache_offset: usize,
    pub cache_loading_at: Option<Instant>,
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
    SaltEvent(SaltEvent, usize, bool),
    JobDetail { jid: String, lookup: Option<JidLookup>, selected_return: Option<usize>, job_fun: String, job_tgt: String, job_start_epoch: f64, minion_scroll: usize, return_filter: String, return_is_filtering: bool, panel_focus: bool, content_scroll: usize, content_scroll_max: usize, content_scroll_mode: bool },
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
            popup_tab: None,
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

            jobs_cache: Vec::new(),
            cache_selected: 0,
            cache_filter_text: String::new(),
            cache_server_filter: String::new(),
            cache_is_filtering: false,
            cache_focus: false,
            cache_loading: false,
            cache_loaded: false,
            cache_load_error: None,
            cache_total_count: 0,
            cache_max_results: 500,
            cache_offset: 0,
            cache_loading_at: None,

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
            pending_jid_lookup_at: None,
            pending_y_copy: None,
            events_total: 0,
            events_last_sec_count: 0,
            events_last_sec_time: Instant::now(),
            secs_since_last_event: 0,
            last_event_time: None,
            event_rate_history: [0.0; 2000],
            event_rate_idx: 0,
            clipboard_at: None,
            clipboard_msg: None,
            sparkline_style: SparklineStyle::default(),
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

    /// Poll pending agent messages from background thread.
    /// Processes at most MAX_PER_DRAIN messages to prevent runaway when
    /// the event arrival rate exceeds the processing rate.
    pub fn drain_agent_messages(&mut self) {
        let mut q = self.agent_rx.lock().unwrap();
        let mut y_copy_json: Option<String> = None;
        let mut processed = 0;
        const MAX_PER_DRAIN: usize = 200;
        while let Some(msg) = q.pop_front() {
            processed += 1;
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
                        "jobs_list" => {
                            self.cache_loading_at = None;
                            if ok {
                                self.jobs_cache = parse_cache_jobs(&data);
                                self.cache_total_count = data["total_count"].as_u64().unwrap_or(0) as usize;
                                self.cache_loaded = true;
                                self.cache_load_error = None;
                            } else {
                                self.cache_load_error = data["error"].as_str().map(|s| s.to_string());
                            }
                            self.cache_loading = false;
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
                                    self.pending_jid_lookup_at = None;
                                    if let Some(DetailPopup::JobDetail { ref mut lookup, .. }) = self.detail_popup {
                                        *lookup = Some(jid_data.clone());
                                    }
                                    // Snap to first filter match if filter was committed before data arrived
                                    if let Some(DetailPopup::JobDetail { ref return_filter, ref mut selected_return, ref mut minion_scroll, ref lookup, .. }) = self.detail_popup {
                                        if !return_filter.is_empty() {
                                            if let Some(ref l) = lookup {
                                                let first = l.returns.iter().position(|ret| ret.minion_id.contains(return_filter));
                                                if let Some(idx) = first {
                                                    *selected_return = Some(idx);
                                                    *minion_scroll = idx.saturating_sub(4);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            // Y key pending: copy pretty JSON of selected return
                            if let Some((y_id, _)) = self.pending_y_copy {
                                if id == y_id {
                                    self.pending_y_copy = None;
                                    let idx = match self.detail_popup {
                                        Some(DetailPopup::JobDetail { selected_return, .. }) => selected_return.unwrap_or(0),
                                        _ => 0,
                                    };
                                    if idx < jid_data.returns.len() {
                                        let json = serde_json::to_string_pretty(&jid_data.returns[idx].full_return)
                                            .unwrap_or_else(|_| jid_data.returns[idx].summary.clone());
                                        y_copy_json = Some(json);
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
            if processed >= MAX_PER_DRAIN { break; }
        }
        if let Some(json) = y_copy_json {
            // Store in a temp file for clipboard
            let _ = std::fs::write("/tmp/salt-radar-copy.txt", &json);
            self.clipboard_msg = Some((
                format!(" Copied {} bytes of JSON job return -- use y for display format ", json.len()),
                std::time::Instant::now(),
            ));
            self.clipboard_at = Some(std::time::Instant::now());
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

        // Timeout for stuck jid_lookup (agent subprocess may hang)
        if self.pending_jid_lookup.is_some() {
            if let Some(started) = self.pending_jid_lookup_at {
                if started.elapsed().as_secs() > 65 {
                    self.pending_jid_lookup = None;
                    self.pending_jid_lookup_at = None;
                    self.detail_popup = None;
                }
            }
        }

        self.clamp_selection();
        if self.sniffer_dirty {
            self.sniffer_dirty = false;
            true
        } else {
            false
        }
    }

    /// True when the main loop should poll at a shorter interval to stay
    /// responsive for an active feature (sniffer, fleet scan, pending lookups).
    pub fn needs_fast_poll(&self) -> bool {
        self.pending_jid_lookup.is_some()
            || self.pending_y_copy.is_some()
            || self.fleet_scanning
            || self.cache_loading
            || matches!(self.detail_popup, Some(DetailPopup::EventSniffer(ref s)) if !s.paused)
    }

    /// Clamp all cursor selections to valid bounds
    fn clamp_selection(&mut self) {
        self.event_selected = self.event_selected
            .min(self.filtered_events_len().saturating_sub(1));
        self.job_selected = self.job_selected
            .min(self.filtered_jobs_len().saturating_sub(1));
        self.cache_selected = self.cache_selected
            .min(self.filtered_cache_len().saturating_sub(1));
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

    // ── Cache job filter helpers ─────────────────────────────────────────

    pub fn filtered_cache_len(&self) -> usize {
        // Server already filtered — no client-side re-filter
        self.jobs_cache.len()
    }

    pub fn get_filtered_cache_job(&self, index: usize) -> Option<&CacheJobEntry> {
        // Server already filtered — no client-side re-filter
        self.jobs_cache.get(index)
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
                    self.detail_popup = Some(DetailPopup::SaltEvent(evt.clone(), 0, false));
                    self.popup_tab = Some(self.current_tab);
                }
            }
            Tab::Jobs => {
                if self.cache_focus {
                    let cache_idx = self.cache_selected;
                    let (jid, fun, tgt, start_epoch, exists) = match self.get_filtered_cache_job(cache_idx) {
                        Some(job) => (job.jid.clone(), job.fun.clone(), job.tgt.clone(), job.start_epoch, true),
                        None => (String::new(), String::new(), String::new(), 0.0, false),
                    };
                    if exists {
                        let req_id = self.send_agent_cmd("jid_lookup", Some(serde_json::json!({"jid": &jid})));
                        self.pending_jid_lookup = Some((req_id, jid.clone()));
                        self.pending_jid_lookup_at = Some(Instant::now());
                        self.detail_popup = Some(DetailPopup::JobDetail {
                            jid,
                            lookup: None,
                            selected_return: None,
                            job_fun: fun,
                            job_tgt: tgt,
                            job_start_epoch: start_epoch,
                            minion_scroll: 0,
                            return_filter: String::new(),
                            return_is_filtering: false,
                            panel_focus: false,
                            content_scroll: 0,
                            content_scroll_max: 0,
                            content_scroll_mode: false,
                        });
                        self.popup_tab = Some(self.current_tab);
                    }
                } else {
                    let (jid, fun, tgt, start_epoch, exists) = {
                        let job = self.get_filtered_job(self.job_selected);
                        match job {
                            Some(job) => {
                                (job.jid.clone(), job.fun.clone(), job.tgt.clone(), job.created_epoch, true)
                            }
                            None => (String::new(), String::new(), String::new(), 0.0, false),
                        }
                    };
                    if exists {
                        let req_id = self.send_agent_cmd("jid_lookup", Some(serde_json::json!({"jid": &jid})));
                        self.pending_jid_lookup = Some((req_id, jid.clone()));
                        self.pending_jid_lookup_at = Some(Instant::now());
                        self.detail_popup = Some(DetailPopup::JobDetail {
                            jid,
                            lookup: None,
                            selected_return: None,
                            job_fun: fun,
                            job_tgt: tgt,
                            job_start_epoch: start_epoch,
                            minion_scroll: 0,
                            return_filter: String::new(),
                            return_is_filtering: false,
                            panel_focus: false,
                            content_scroll: 0,
                            content_scroll_max: 0,
                            content_scroll_mode: false,
                        });
                        self.popup_tab = Some(self.current_tab);
                    }
                }
            }
            Tab::Alerts => {
                if let Some(alert) = self.get_filtered_alert(self.alert_selected) {
                    self.detail_popup = Some(DetailPopup::Alert(alert.clone()));
                    self.popup_tab = Some(self.current_tab);
                }
            }
            Tab::Minions => {
                if let Some(minion) = self.get_filtered_minion(self.minion_selected) {
                    self.detail_popup = Some(DetailPopup::Minion(minion.clone()));
                    self.popup_tab = Some(self.current_tab);
                }
            }
            _ => {}
        }
    }

    /// Strip ANSI escape sequences from a string.
    pub fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                // Consume until we hit 'm' (end of CSI sequence)
                while let Some(n) = chars.next() {
                    if n == 'm' { break; }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    /// Parse ANSI-escaped text into styled Spans for ratatui rendering.
    /// SGR resets fall back to `default_style`.
    pub fn ansi_to_styled_spans(text: &str, default_style: Style) -> Vec<Span<'static>> {
        if !text.contains('\x1b') {
            return vec![Span::styled(text.to_string(), default_style)];
        }

        fn apply_sgr(style: &mut Style, params: &[u16], default_style: &Style) {
            if params.is_empty() {
                *style = *default_style;
                return;
            }
            let mut i = 0;
            while i < params.len() {
                if params[i] == 0 {
                    // Reset to default, then continue for any remaining params
                    *style = *default_style;
                    i += 1;
                    continue;
                }
                match params[i] {
                    1 => *style = style.add_modifier(Modifier::BOLD),
                    3 => *style = style.add_modifier(Modifier::ITALIC),
                    4 => *style = style.add_modifier(Modifier::UNDERLINED),
                    22 => *style = style.remove_modifier(Modifier::BOLD),
                    23 => *style = style.remove_modifier(Modifier::ITALIC),
                    24 => *style = style.remove_modifier(Modifier::UNDERLINED),
                    // Standard foreground (30-37)
                    30 => *style = style.fg(Color::Black),
                    31 => *style = style.fg(Color::Red),
                    32 => *style = style.fg(Color::Green),
                    33 => *style = style.fg(Color::Yellow),
                    34 => *style = style.fg(Color::Blue),
                    35 => *style = style.fg(Color::Magenta),
                    36 => *style = style.fg(Color::Cyan),
                    37 => *style = style.fg(Color::White),
                    38 => {
                        // Extended foreground: 38;5;N or 38;2;R;G;B
                        if i + 1 < params.len() {
                            match params[i + 1] {
                                5 if i + 2 < params.len() => {
                                    let idx = params[i + 2] as u8;
                                    *style = style.fg(Color::Indexed(idx));
                                    i += 2;
                                }
                                2 if i + 4 < params.len() => {
                                    let r = params[i + 2] as u8;
                                    let g = params[i + 3] as u8;
                                    let b = params[i + 4] as u8;
                                    *style = style.fg(Color::Rgb(r, g, b));
                                    i += 4;
                                }
                                _ => {}
                            }
                        }
                    }
                    39 => *style = style.fg(Color::Reset),
                    // Standard background (40-47)
                    40 => *style = style.bg(Color::Black),
                    41 => *style = style.bg(Color::Red),
                    42 => *style = style.bg(Color::Green),
                    43 => *style = style.bg(Color::Yellow),
                    44 => *style = style.bg(Color::Blue),
                    45 => *style = style.bg(Color::Magenta),
                    46 => *style = style.bg(Color::Cyan),
                    47 => *style = style.bg(Color::White),
                    48 => {
                        // Extended background: 48;5;N or 48;2;R;G;B
                        if i + 1 < params.len() {
                            match params[i + 1] {
                                5 if i + 2 < params.len() => {
                                    let idx = params[i + 2] as u8;
                                    *style = style.bg(Color::Indexed(idx));
                                    i += 2;
                                }
                                2 if i + 4 < params.len() => {
                                    let r = params[i + 2] as u8;
                                    let g = params[i + 3] as u8;
                                    let b = params[i + 4] as u8;
                                    *style = style.bg(Color::Rgb(r, g, b));
                                    i += 4;
                                }
                                _ => {}
                            }
                        }
                    }
                    49 => *style = style.bg(Color::Reset),
                    // Bright foreground (90-97)
                    90 => *style = style.fg(Color::Rgb(128, 128, 128)),
                    91 => *style = style.fg(Color::Rgb(255, 85, 85)),
                    92 => *style = style.fg(Color::Rgb(85, 255, 85)),
                    93 => *style = style.fg(Color::Rgb(255, 255, 85)),
                    94 => *style = style.fg(Color::Rgb(85, 85, 255)),
                    95 => *style = style.fg(Color::Rgb(255, 85, 255)),
                    96 => *style = style.fg(Color::Rgb(85, 255, 255)),
                    97 => *style = style.fg(Color::Rgb(255, 255, 255)),
                    // Bright background (100-107)
                    100 => *style = style.bg(Color::Rgb(128, 128, 128)),
                    101 => *style = style.bg(Color::Rgb(255, 85, 85)),
                    102 => *style = style.bg(Color::Rgb(85, 255, 85)),
                    103 => *style = style.bg(Color::Rgb(255, 255, 85)),
                    104 => *style = style.bg(Color::Rgb(85, 85, 255)),
                    105 => *style = style.bg(Color::Rgb(255, 85, 255)),
                    106 => *style = style.bg(Color::Rgb(85, 255, 255)),
                    107 => *style = style.bg(Color::Rgb(255, 255, 255)),
                    _ => {}
                }
                i += 1;
            }
        }

        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut buf = String::new();
        let mut style = default_style;
        let mut chars = text.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '\x1b' {
                // Flush accumulated text
                if !buf.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut buf), style));
                }
                // Read escape sequence: expect '[' then params then 'm'
                if chars.next_if(|&n| n == '[').is_some() {
                    let mut params_str = String::new();
                    while let Some(&n) = chars.peek() {
                        if n == 'm' { chars.next(); break; }
                        params_str.push(n);
                        chars.next();
                    }
                    let params: Vec<u16> = params_str.split(';')
                        .filter_map(|s| s.parse::<u16>().ok())
                        .collect();
                    apply_sgr(&mut style, &params, &default_style);
                }
            } else {
                buf.push(c);
            }
        }
        if !buf.is_empty() {
            spans.push(Span::styled(buf, style));
        }
        spans
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

    /// Run a command and capture its stdout (with timeout).
    fn run_cmd_with_output(cmd: &mut std::process::Command, timeout_secs: u64) -> Option<String> {
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
    fn tmux_global_env(name: &str) -> Option<String> {
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
    fn resolve_x11_env() -> (Option<String>, Option<String>) {
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
    fn find_xauthority(username: &str) -> Option<String> {
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
    fn xauthority_from_process(username: &str) -> Option<String> {
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

    pub fn copy_to_clipboard(&mut self, text: &str) {
        // 1. Write to local file (always works, easy debug)
        let _ = std::fs::write("/tmp/salt-radar-copy.txt", text);

        let mut warnings: Vec<&str> = Vec::new();

        // 2. tmux load-buffer (for Ctrl+b ] paste inside tmux).
        let ok_tmux = if let Some(socket) = Self::tmux_socket_path() {
            let (ok, _) = Self::run_cmd_with_stdin(
                std::process::Command::new("tmux").args(["-S", &socket, "load-buffer", "-"]),
                text, 3,
            );
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
            ok
        } else { false };

        // 3. xclip — resolves DISPLAY and XAUTHORITY from process env or
        //    tmux global environment, then forwards them via su.
        let xclip_avail = std::process::Command::new("sh")
            .args(["-c", "command -v xclip"])
            .stdout(Stdio::null()).stderr(Stdio::null())
            .status().map(|s| s.success()).unwrap_or(false);
        if !xclip_avail {
            warnings.push("xclip not installed");
        }

        let (display, xauth) = Self::resolve_x11_env();

        let mut ok_xclip = false;
        let mut xclip_log: Vec<String> = Vec::new();

        if let Some(ref disp) = display {
            // Collect candidate users (X11 socket owner first, then current)
            let x11_user = Self::detect_x11_user(disp);
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
                    xclip_log.push(format!("user {} not found, skip", user));
                    continue;
                }

                // Strategy 1: su -l (login shell — triggers pam_xauth which
                // generates the X11 auth cookie for this session automatically).
                if xclip_avail {
                    let (ok, err) = Self::run_cmd_with_stdin(
                        std::process::Command::new("su")
                            .args(["-l", user, "-c", &format!("DISPLAY={} xclip -selection clipboard", disp)]),
                        text, 3,
                    );
                    if ok { ok_xclip = true; xclip_log.push(format!("su -l {}: OK", user)); break; }
                    xclip_log.push(format!("su -l {}: FAIL err=[{}]", user, err.trim()));
                }

                // Strategy 2: su with explicit XAUTHORITY
                if xclip_avail {
                    if let Some(ref xa) = xauth {
                        let (ok, err) = Self::run_cmd_with_stdin(
                            std::process::Command::new("su")
                                .args(["-c", &format!("XAUTHORITY={} DISPLAY={} xclip -selection clipboard", xa, disp), user]),
                            text, 3,
                        );
                        if ok { ok_xclip = true; xclip_log.push(format!("su {} with-XA: OK", user)); break; }
                        xclip_log.push(format!("su {} with-XA: FAIL err=[{}]", user, err.trim()));
                    }
                }

                // Strategy 3: su without explicit XAUTHORITY (xclip auto-detects
                // from ~/.Xauthority, which su maps via HOME).
                if xclip_avail {
                    let (ok, err) = Self::run_cmd_with_stdin(
                        std::process::Command::new("su")
                            .args(["-c", &format!("DISPLAY={} xclip -selection clipboard", disp), user]),
                        text, 3,
                    );
                    if ok { ok_xclip = true; xclip_log.push(format!("su {} no-XA: OK", user)); break; }
                    xclip_log.push(format!("su {} no-XA: FAIL err=[{}]", user, err.trim()));
                }
            }

            if !xclip_avail {
                xclip_log.push("xclip not installed".to_string());
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
            "/tmp/salt-radar-clip.log",
            format!(
                "file={}b tmux={} xclip={} display={} xauthority={} xclip_installed={}\n{}",
                text.len(),
                if ok_tmux { "ok" } else { "skip" },
                if ok_xclip { "ok" } else { "fail" },
                disp_info, xa_info, xclip_avail,
                xclip_log.join("\n"),
            ),
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

    fn current_user() -> String {
        std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_else(|_| "root".to_string())
    }

    /// Tries to find the unprivileged user that owns the X11 session by
    /// checking ownership of /tmp/.X11-unix/X<display_number>.
    fn detect_x11_user(display: &str) -> String {
        let dnum = display
            .trim_start_matches("localhost:")
            .trim_start_matches(':')
            .split('.')
            .next()
            .unwrap_or("")
            .trim();
        if dnum.is_empty() {
            return Self::current_user();
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
        Self::current_user()
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

        // Auto-load job cache when first viewing Jobs tab
        if self.current_tab == Tab::Jobs && !self.cache_loaded && !self.cache_loading && self.cache_load_error.is_none() {
            self.send_cache_jobs_list(None, true);
        }

        // Cache loading timeout — reset after 30s in case agent hangs
        if self.cache_loading {
            if let Some(since) = self.cache_loading_at {
                if since.elapsed().as_secs() >= 120 {
                    self.cache_loading = false;
                    self.cache_loading_at = None;
                    self.cache_load_error = Some("Request timed out after 120s. Press r to retry.".to_string());
                }
            }
        }

        // Clamp cursor selections to valid range
        self.clamp_selection();

        // Clear transient clipboard message after 8 seconds
        if let Some((_, at)) = self.clipboard_msg {
            if at.elapsed().as_secs() > 8 {
                self.clipboard_msg = None;
            }
        }

        self.tick_count += 1;
    }

    fn send_cache_jobs_list(&mut self, filter: Option<&str>, reset_offset: bool) {
        if reset_offset {
            self.cache_offset = 0;
        }
        let mut extra = serde_json::json!({"max_results": self.cache_max_results, "offset": self.cache_offset});
        if let Some(f) = filter {
            extra["filter_text"] = serde_json::Value::String(f.to_string());
        }
        self.cache_loading = true;
        self.cache_loading_at = Some(Instant::now());
        self.cache_loaded = false;
        self.cache_load_error = None;
        self.send_agent_cmd("jobs_list", Some(extra));
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

        // Sniffer input mode (text entry before launching, only on Events tab)
        if self.sniffer_input_active && self.current_tab == Tab::Events {
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
                                minion_ids: ids.clone(),
                                buffer: std::collections::VecDeque::with_capacity(1000),
                                paused: false,
                                selected: 0,
                                highlight_keywords: ids,
                                highlight_input: None,
                            }
                        ));
                        self.popup_tab = Some(self.current_tab);
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
        let had_popup = self.detail_popup.is_some() && self.popup_tab == Some(self.current_tab);

        // Contrast toggle and tab switching work universally (even with popups open)
        // but are muted when a text input mode is active inside the popup
        if had_popup {
            let popup_text_input = match &self.detail_popup {
                Some(DetailPopup::EventSniffer(ref s)) => s.highlight_input.is_some(),
                Some(DetailPopup::JobDetail { return_is_filtering: true, .. }) => true,
                _ => false,
            };
            if !popup_text_input {
                match code {
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        self.high_contrast = !self.high_contrast;
                        return false;
                    }
                    KeyCode::Tab => { self.current_tab = self.current_tab.next(); return false; }
                    KeyCode::BackTab => { self.current_tab = self.current_tab.prev(); return false; }
                    KeyCode::Char('1') => { self.current_tab = Tab::Dashboard; return false; }
                    KeyCode::Char('2') => { self.current_tab = Tab::Minions; return false; }
                    KeyCode::Char('3') => { self.current_tab = Tab::Events; return false; }
                    KeyCode::Char('4') => { self.current_tab = Tab::Jobs; return false; }
                    KeyCode::Char('5') => { self.current_tab = Tab::Alerts; return false; }
                    _ => {}
                }
            }
        }

        // Local variable to defer jid_lookup agent command (avoids borrow conflict)
        let mut trigger_y_lookup: Option<String> = None;
        if self.popup_tab == Some(self.current_tab) {
            if let Some(ref mut detail) = self.detail_popup {
            match *detail {
                DetailPopup::JobDetail {
                    ref mut selected_return,
                    ref mut minion_scroll,
                    ref mut return_filter,
                    ref mut return_is_filtering,
                    ref mut panel_focus,
                    ref mut content_scroll,
                    ref mut content_scroll_max,
                    ref mut content_scroll_mode,
                    ref lookup,
                    ref jid,
                    ..
                } => {
                    // Return filter input mode
                    if *return_is_filtering {
                        match code {
                            KeyCode::Esc => {
                                *return_is_filtering = false;
                                return_filter.clear();
                            }
                            KeyCode::Enter => {
                                *return_is_filtering = false;
                                // Snap selection to nearest matching entry
                                if !return_filter.is_empty() {
                                    if let Some(ref l) = lookup {
                                        let cur = selected_return.unwrap_or(0);
                                        // Try forward from current position
                                        let next = (cur..l.returns.len())
                                            .find(|&i| l.returns[i].minion_id.contains(&*return_filter));
                                        if let Some(idx) = next {
                                            *selected_return = Some(idx);
                                            *minion_scroll = idx.saturating_sub(4);
                                        } else {
                                            // Try backward from current position
                                            let prev = (0..cur).rev()
                                                .find(|&i| l.returns[i].minion_id.contains(&*return_filter));
                                            if let Some(idx) = prev {
                                                *selected_return = Some(idx);
                                                *minion_scroll = idx.saturating_sub(4);
                                            }
                                        }
                                    }
                                }
                            }
                            KeyCode::Backspace => { return_filter.pop(); }
                            KeyCode::Char(c) => { return_filter.push(c); }
                            _ => {}
                        }
                        return false;
                    }
                    // Right panel (content preview) focused
                    if *panel_focus {
                        if *content_scroll_mode {
                            match code {
                                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                                    *content_scroll_mode = false;
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    *content_scroll = content_scroll.saturating_sub(1);
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    *content_scroll += 1;
                                }
                                KeyCode::PageUp => {
                                    *content_scroll = content_scroll.saturating_sub(10);
                                }
                                KeyCode::PageDown => {
                                    *content_scroll += 10;
                                }
                                KeyCode::Home => { *content_scroll = 0; }
                                KeyCode::End => { *content_scroll = usize::MAX / 2; }
                                _ => {}
                            }
                            return false;
                        }
                        match code {
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                                *panel_focus = false;
                            }
                            KeyCode::Enter => {
                                *content_scroll_mode = true;
                                *content_scroll = 0;
                                if let Some(ref l) = lookup {
                                    let sel = selected_return.unwrap_or(0).min(l.returns.len().saturating_sub(1));
                                    if sel < l.returns.len() {
                                        let display_str = &l.returns[sel].formatted_text;
                                        *content_scroll_max = if display_str.is_empty() { 0 } else { display_str.lines().count() };
                                    }
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                *content_scroll += 1;
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                *content_scroll = content_scroll.saturating_sub(1);
                            }
                            KeyCode::PageUp => {
                                *content_scroll = content_scroll.saturating_sub(10);
                            }
                            KeyCode::PageDown => {
                                *content_scroll += 10;
                            }
                            KeyCode::Home => { *content_scroll = 0; }
                            KeyCode::End => { *content_scroll = usize::MAX / 2; }
                            KeyCode::Char('y') => {
                                if let Some(ref l) = lookup {
                                    let idx = selected_return.unwrap_or(0);
                                    if idx < l.returns.len() {
                                        let ret = &l.returns[idx];
                                        clipboard_text = Some(
                                            if !ret.formatted_text.is_empty() {
                                                Self::strip_ansi(&ret.formatted_text)
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
                                        clipboard_text = Some(
                                            serde_json::to_string_pretty(&l.returns[idx].full_return)
                                                .unwrap_or_else(|_| l.returns[idx].summary.clone())
                                        );
                                    }
                                }
                                trigger_y_lookup = Some(jid.clone());
                            }
                            _ => {}
                        }
                        // Don't return false — let clipboard_text flow through
                    } else {
                    // Left panel (return list) focused
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            self.detail_popup = None;
                        }
                        KeyCode::Enter => {
                            if lookup.is_some() {
                                *panel_focus = true;
                                *content_scroll = 0;
                                *content_scroll_mode = false;
                                if let Some(ref l) = lookup {
                                    let sel = selected_return.unwrap_or(0).min(l.returns.len().saturating_sub(1));
                                    if sel < l.returns.len() {
                                        let display_str = &l.returns[sel].formatted_text;
                                        *content_scroll_max = if display_str.is_empty() { 0 } else { display_str.lines().count() };
                                    }
                                }
                            }
                        }
                        KeyCode::Char('/') => {
                            *return_is_filtering = true;
                            return_filter.clear();
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if let Some(ref l) = lookup {
                                if !l.returns.is_empty() {
                                    let pat = &*return_filter;
                                    let cur = selected_return.unwrap_or(0);
                                    if cur > 0 {
                                        let mut new_sel = cur.saturating_sub(1);
                                        loop {
                                            if pat.is_empty() || l.returns[new_sel].minion_id.contains(pat) { break; }
                                            if new_sel == 0 { break; }
                                            new_sel -= 1;
                                        }
                                        *selected_return = Some(new_sel);
                                        if *minion_scroll > 0 && *minion_scroll >= new_sel {
                                            *minion_scroll = new_sel;
                                        }
                                    }
                                }
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if let Some(ref l) = lookup {
                                if !l.returns.is_empty() {
                                    let pat = &*return_filter;
                                    let cur = selected_return.unwrap_or(0) + 1;
                                    if cur < l.returns.len() {
                                        let mut new_sel = cur;
                                        while new_sel < l.returns.len() {
                                            if pat.is_empty() || l.returns[new_sel].minion_id.contains(pat) { break; }
                                            new_sel += 1;
                                        }
                                        if new_sel < l.returns.len() {
                                            *selected_return = Some(new_sel);
                                            if *minion_scroll + 8 < new_sel {
                                                *minion_scroll = new_sel.saturating_sub(4);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        KeyCode::PageUp => {
                            if let Some(ref l) = lookup {
                                if !l.returns.is_empty() {
                                    let cur = selected_return.unwrap_or(0);
                                    *selected_return = Some(cur.saturating_sub(10));
                                    if *minion_scroll >= selected_return.unwrap_or(0) {
                                        *minion_scroll = selected_return.unwrap_or(0).saturating_sub(4);
                                    }
                                }
                            }
                        }
                        KeyCode::PageDown => {
                            if let Some(ref l) = lookup {
                                if !l.returns.is_empty() {
                                    let cur = selected_return.unwrap_or(0) + 10;
                                    *selected_return = Some(cur.min(l.returns.len().saturating_sub(1)));
                                    if *minion_scroll + 8 < selected_return.unwrap_or(0) {
                                        *minion_scroll = selected_return.unwrap_or(0).saturating_sub(4);
                                    }
                                }
                            }
                        }
                        KeyCode::Home => {
                            if let Some(ref l) = lookup {
                                if !l.returns.is_empty() {
                                    *selected_return = Some(0);
                                    *minion_scroll = 0;
                                }
                            }
                        }
                        KeyCode::End => {
                            if let Some(ref l) = lookup {
                                if !l.returns.is_empty() {
                                    *selected_return = Some(l.returns.len().saturating_sub(1));
                                    *minion_scroll = l.returns.len().saturating_sub(1);
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
                                            Self::strip_ansi(&ret.formatted_text)
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
                                    clipboard_text = Some(
                                        serde_json::to_string_pretty(&l.returns[idx].full_return)
                                            .unwrap_or_else(|_| l.returns[idx].summary.clone())
                                    );
                                }
                            }
                            trigger_y_lookup = Some(jid.clone());
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
                    // Highlight keyword input mode
                    if let Some(ref mut buf) = state.highlight_input {
                        match code {
                            KeyCode::Enter => {
                                let keywords: Vec<String> = buf.split(',')
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                                    .collect();
                                if !keywords.is_empty() {
                                    state.highlight_keywords = keywords;
                                }
                                state.highlight_input = None;
                            }
                            KeyCode::Esc => {
                                state.highlight_input = None;
                            }
                            KeyCode::Backspace => { buf.pop(); }
                            KeyCode::Char(c) => { buf.push(c); }
                            _ => {}
                        }
                        return false;
                    }
                    match code {
                        KeyCode::Enter => {
                            let rev_idx = state.buffer.len().saturating_sub(1).saturating_sub(state.selected);
                            if let Some(evt) = state.buffer.get(rev_idx) {
                                self.return_to_sniffer = Some(state.clone());
                                self.detail_popup = Some(DetailPopup::SaltEvent(evt.clone(), 0, false));
                                self.popup_tab = Some(self.current_tab);
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
                        KeyCode::Home => {
                            state.selected = 0;
                        }
                        KeyCode::End => {
                            state.selected = state.buffer.len().saturating_sub(1);
                        }
                        KeyCode::Char('p') | KeyCode::Char('P') => {
                            state.paused = !state.paused;
                        }
                        KeyCode::Char('h') | KeyCode::Char('H') => {
                            state.highlight_input = Some(state.highlight_keywords.join(", "));
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
                DetailPopup::SaltEvent(ref evt, ref mut scroll_offset, ref mut scroll_mode) => {
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
                            KeyCode::Home => { *scroll_offset = 0; }
                            KeyCode::End => { *scroll_offset = usize::MAX / 2; }
                            _ => {}
                        }
                        return false;
                    }
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            if let Some(sniffer) = self.return_to_sniffer.take() {
                                self.detail_popup = Some(DetailPopup::EventSniffer(sniffer));
                                self.popup_tab = Some(self.current_tab);
                            } else {
                                self.detail_popup = None;
                            }
                        }
                        KeyCode::Enter => {
                            *scroll_mode = true;
                            *scroll_offset = 0;
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
        }
        if let Some(jid) = trigger_y_lookup {
            let req_id = self.send_agent_cmd("jid_lookup", Some(serde_json::json!({"jid": &jid})));
            self.pending_y_copy = Some((req_id, jid));
        }
        if had_popup {
            return false;
        }

        // Filter input mode (live jobs / other tabs)
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

        // Cache filter input mode (Jobs tab, right panel)
        if self.cache_is_filtering {
            match code {
                KeyCode::Esc => {
                    self.cache_is_filtering = false;
                    self.cache_filter_text.clear();
                    self.cache_server_filter.clear();
                    self.cache_selected = 0;
                    self.clamp_selection();
                    self.send_cache_jobs_list(None, true);
                }
                KeyCode::Enter => {
                    self.cache_is_filtering = false;
                    self.cache_server_filter = self.cache_filter_text.clone();
                    self.cache_filter_text.clear();
                    self.cache_selected = 0;
                    self.clamp_selection();
                    let ft = self.cache_server_filter.clone();
                    self.send_cache_jobs_list(if ft.is_empty() { None } else { Some(&ft) }, true);
                }
                KeyCode::Backspace => {
                    self.cache_filter_text.pop();
                    self.clamp_selection();
                }
                KeyCode::Char(c) => {
                    self.cache_filter_text.push(c);
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
                    self.popup_tab = Some(self.current_tab);
                }
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                if self.admin_mode && self.current_tab == Tab::Minions {
                    // Reset any stuck scan state and start fresh
                    self.fleet_scanning = true;
                    self.fleet_scan_jid = None;
                    self.fleet_scan_started_at = None;
                    self.fleet_data = None;
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
            KeyCode::Char('r') | KeyCode::Char('R') => {
                if self.current_tab == Tab::Jobs && !self.cache_loading {
                    let ft = self.cache_server_filter.clone();
                    self.send_cache_jobs_list(if ft.is_empty() { None } else { Some(&ft) }, false);
                }
            }
            KeyCode::Char(']') | KeyCode::Char('}') => {
                if self.current_tab == Tab::Jobs && !self.cache_loading {
                    self.cache_max_results = (self.cache_max_results + 500).min(10000);
                    let ft = self.cache_server_filter.clone();
                    self.send_cache_jobs_list(if ft.is_empty() { None } else { Some(&ft) }, false);
                }
            }
            KeyCode::Char('[') | KeyCode::Char('{') => {
                if self.current_tab == Tab::Jobs && !self.cache_loading {
                    self.cache_max_results = self.cache_max_results.saturating_sub(500).max(200);
                    let ft = self.cache_server_filter.clone();
                    self.send_cache_jobs_list(if ft.is_empty() { None } else { Some(&ft) }, false);
                }
            }
            KeyCode::Char('>') | KeyCode::Char('.') => {
                if self.current_tab == Tab::Jobs && !self.cache_loading && self.cache_loaded {
                    self.cache_offset += self.cache_max_results;
                    if self.cache_offset >= self.cache_total_count {
                        self.cache_offset = self.cache_total_count.saturating_sub(self.cache_max_results);
                    }
                    let ft = self.cache_server_filter.clone();
                    self.send_cache_jobs_list(if ft.is_empty() { None } else { Some(&ft) }, false);
                }
            }
            KeyCode::Char('<') | KeyCode::Char(',') => {
                if self.current_tab == Tab::Jobs && !self.cache_loading && self.cache_loaded {
                    self.cache_offset = self.cache_offset.saturating_sub(self.cache_max_results);
                    let ft = self.cache_server_filter.clone();
                    self.send_cache_jobs_list(if ft.is_empty() { None } else { Some(&ft) }, false);
                }
            }
            KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::Char('H') => {
                self.show_help = true;
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.high_contrast = !self.high_contrast;
            }
            KeyCode::Char('m') | KeyCode::Char('M') => {
                self.sparkline_style = match self.sparkline_style {
                    SparklineStyle::Bars => SparklineStyle::Dots,
                    SparklineStyle::Dots => SparklineStyle::Bars,
                };
            }
            KeyCode::Char('/') => {
                if self.current_tab == Tab::Jobs {
                    if self.cache_focus {
                        self.cache_is_filtering = true;
                        self.cache_filter_text.clear();
                        self.cache_server_filter.clear();
                    } else {
                        self.is_filtering = true;
                        self.filter_text.clear();
                    }
                } else {
                    self.is_filtering = true;
                    self.filter_text.clear();
                }
            }
            KeyCode::Tab => {
                self.current_tab = self.current_tab.next();
            }
            KeyCode::BackTab => self.current_tab = self.current_tab.prev(),
            KeyCode::Left => {
                if self.current_tab == Tab::Jobs && !had_popup
                    && !self.is_filtering && !self.cache_is_filtering
                {
                    self.cache_focus = false;
                }
            }
            KeyCode::Right => {
                if self.current_tab == Tab::Jobs && !had_popup
                    && !self.is_filtering && !self.cache_is_filtering
                {
                    self.cache_focus = true;
                }
            }
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
            Tab::Jobs => {
                if self.cache_focus {
                    self.cache_selected = self.cache_selected.saturating_sub(1);
                } else {
                    self.job_selected = self.job_selected.saturating_sub(1);
                }
            }
            Tab::Alerts => self.alert_selected = self.alert_selected.saturating_sub(1),
            _ => {}
        }
    }

    fn move_down(&mut self) {
        match self.current_tab {
            Tab::Minions => self.minion_selected += 1,
            Tab::Events => self.event_selected += 1,
            Tab::Jobs => {
                if self.cache_focus {
                    self.cache_selected += 1;
                } else {
                    self.job_selected += 1;
                }
            }
            Tab::Alerts => self.alert_selected += 1,
            _ => {}
        }
    }

    fn move_to_first(&mut self) {
        match self.current_tab {
            Tab::Minions => self.minion_selected = 0,
            Tab::Events => self.event_selected = 0,
            Tab::Jobs => {
                if self.cache_focus {
                    self.cache_selected = 0;
                } else {
                    self.job_selected = 0;
                }
            }
            Tab::Alerts => self.alert_selected = 0,
            _ => {}
        }
    }

    fn move_to_last(&mut self) {
        match self.current_tab {
            Tab::Minions => self.minion_selected = self.filtered_minions_len().saturating_sub(1),
            Tab::Events => self.event_selected = self.filtered_events_len().saturating_sub(1),
            Tab::Jobs => {
                if self.cache_focus {
                    self.cache_selected = self.filtered_cache_len().saturating_sub(1);
                } else {
                    self.job_selected = self.filtered_jobs_len().saturating_sub(1);
                }
            }
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

/// Build a JidLookup directly from events in the buffer (no agent call).
fn build_jid_lookup_from_events(jid: &str, events: &VecDeque<SaltEvent>) -> Option<JidLookup> {
    let prefix = format!("salt/job/{}/ret/", jid);
    let mut returns: Vec<JidReturn> = Vec::new();
    let mut fun = String::new();
    let mut tgt = String::new();

    for evt in events {
        if evt.tag == format!("salt/job/{}/new", jid) {
            fun = evt.data["fun"].as_str().unwrap_or("").to_string();
            tgt = evt.data["tgt"].as_str().unwrap_or("").to_string();
        }
        if !evt.tag.starts_with(&prefix) { continue; }
        let minion_id = match evt.data["id"].as_str() {
            Some(m) if !m.is_empty() => m.to_string(),
            _ => continue,
        };
        let full_return = evt.data["return"].clone();
        let summary = match &full_return {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                // Use a short summary: first 80 chars of formatted representation
                let formatted = App::format_return_as_console(&minion_id, &full_return);
                let first_line = formatted.lines().next().unwrap_or(&formatted);
                let truncated = if first_line.len() > 80 {
                    format!("{}...", &first_line[..80])
                } else {
                    first_line.to_string()
                };
                truncated
            }
            other => other.to_string(),
        };
        returns.push(JidReturn {
            minion_id,
            success: evt.data["success"].as_bool().unwrap_or(false),
            retcode: evt.data["retcode"].as_i64().unwrap_or(0),
            summary,
            full_return,
            formatted_text: String::new(), // computed on-demand in y handler
        });
    }
    if returns.is_empty() { return None; }
    Some(JidLookup {
        found: true,
        fun,
        tgt,
        start_time: String::new(),
        returns,
    })
}

pub fn derive_success_retcode(full_return: &serde_json::Value) -> (i64, bool) {
    match full_return {
        serde_json::Value::Object(obj) => {
            // state dict: check 'result' key
            if let Some(result_val) = obj.get("result") {
                if !result_val.as_bool().unwrap_or(false) {
                    return (1, false);
                }
                if result_val.is_boolean() {
                    return (0, true);
                }
            }
            // check explicit retcode
            let rc = obj.get("retcode").and_then(|v| v.as_i64()).unwrap_or(0);
            (rc, rc == 0)
        }
        serde_json::Value::Bool(b) => (0, *b),
        serde_json::Value::Null => (1, false),
        serde_json::Value::String(s) => {
            // Salt returns error strings starting with "ERROR:" for cmd.run etc.
            if s.starts_with("ERROR:") || s.starts_with("error:") {
                (1, false)
            } else if s.trim().is_empty() {
                (1, false)
            } else {
                (0, true)
            }
        }
        _ => (0, true),
    }
}

fn write_jid_debug(full_return: &serde_json::Value, success: bool, retcode: i64, minion_id: &str) {
    let _ = (full_return, success, retcode, minion_id);
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
            let full_return = entry["full_return"].clone();
            let (derived_retcode, derived_success) = derive_success_retcode(&full_return);
            write_jid_debug(&full_return, derived_success, derived_retcode,
                entry["minion_id"].as_str().unwrap_or("?"));
            result.returns.push(JidReturn {
                minion_id: entry["minion_id"].as_str().unwrap_or("?").to_string(),
                success: derived_success,
                retcode: derived_retcode,
                summary: entry["summary"].as_str().unwrap_or("").to_string(),
                full_return,
                formatted_text: entry["formatted_text"].as_str().unwrap_or("").to_string(),
            });
        }
    }
    result
}

fn parse_cache_jobs(data: &serde_json::Value) -> Vec<CacheJobEntry> {
    let mut result = Vec::new();
    if let Some(arr) = data["jobs"].as_array() {
        for entry in arr {
            result.push(CacheJobEntry {
                jid: entry["jid"].as_str().unwrap_or("?").to_string(),
                fun: entry["fun"].as_str().unwrap_or("").to_string(),
                tgt: entry["tgt"].as_str().unwrap_or("").to_string(),
                tgt_type: entry["tgt_type"].as_str().unwrap_or("glob").to_string(),
                user: entry["user"].as_str().unwrap_or("").to_string(),
                start_time: entry["start_time"].as_str().unwrap_or("").to_string(),
                start_epoch: entry["start_epoch"].as_f64().unwrap_or(0.0),
                arguments: entry["arguments"].as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default(),
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
