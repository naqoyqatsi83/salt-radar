use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::process::Stdio;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::alerts::{AlertEngine, ServiceMemThresholds};
use crate::config::{load_config, AppConfig};
use crate::types::*;

const JOB_STATS_WINDOW: usize = 500;
const SERVICE_TREND_WINDOW: Duration = Duration::from_secs(24 * 3600);
const SERVICE_MEM_SAMPLE_MIN_INTERVAL: Duration = Duration::from_secs(60);

/// Per-unit restart/flap timestamps and memory sample history, used to derive
/// rolling-24h flap counts and memory-growth trends (see update_service_trends).
#[derive(Debug, Default)]
pub struct ServiceTrend {
    last_uptime_seconds: u64,
    restart_events: VecDeque<Instant>,
    mem_samples: VecDeque<(Instant, u64)>,
}

pub struct App {
    // Config
    pub config: AppConfig,
    /// Where `config` was loaded from and gets saved back to — see
    /// `config::resolve_config_path`.
    pub config_path: std::path::PathBuf,

    // Tabs
    pub current_tab: Tab,
    pub session_start: Instant,
    pub tick_count: u64,

    // Data from agent
    pub agent: AgentState,
    pub config_snapshot: Option<ConfigSnapshot>,

    // Collector data
    pub keys: Vec<KeyInfo>,
    /// Accepted key IDs, kept in sync whenever `keys` is (re)populated —
    /// event/job filtering matches against this on every keystroke while
    /// filtering, and on a master with thousands of keys, rebuilding this
    /// set from `keys` on every single one of those calls (previously what
    /// `accepted_ids()` did) made typing into the filter box visibly lag,
    /// worse the busier the master, since a redraw fires on every character.
    accepted_id_set: HashSet<String>,
    pub processes: Vec<SaltProcess>,
    pub services: Vec<ServiceHealth>,
    /// Per-unit restart/flap history and memory sample history, keyed by unit name.
    pub service_trends: HashMap<String, ServiceTrend>,
    pub zmq_connections: Vec<ZmqConn>,
    pub system_info: SystemInfo,
    pub api_stats: ApiStats,

    // Minion cache (from agent)
    pub minion_info: Vec<MinionInfo>,
    // Set whenever minion_info is mutated; the per-tick sort in update()
    // only runs when this is set, then clears it.
    minion_info_dirty: bool,

    // Events buffer
    pub events: VecDeque<SaltEvent>,
    pub events_since_last_tick: Vec<SaltEvent>,
    pub event_breakdown: EventBreakdown,
    pub events_per_sec: f64,
    /// Incrementally maintained subset of `events` matching `applied_filter`.
    /// Rebuilt in full only when the filter itself changes (Enter/Esc/`/`);
    /// kept in sync on every ingested/evicted event in O(1) instead of
    /// re-scanning the whole buffer -- see `sync_filtered_events_cache` and
    /// `rebuild_filtered_events_cache`.
    filtered_events_cache: VecDeque<SaltEvent>,

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
    pub fleet_scan_completed: bool,
    pub fleet_data: Option<FleetScan>,

    // Filter state
    /// Live edit buffer while `is_filtering` is true, for whichever tab is
    /// currently focused — NOT itself "the filter" for any tab once editing
    /// ends. Each of Minions/Jobs/Alerts/Api keeps its own confirmed filter
    /// in its dedicated field below (Events keeps its own in
    /// `applied_filter`) precisely so that typing a filter on one tab
    /// doesn't silently also filter an unrelated tab (a host filter typed
    /// into Events has no sensible meaning applied to the Api tab's
    /// method/path/status search, for instance). Tab-switching is blocked
    /// while `is_filtering` is true (see the key handler), so there's never
    /// more than one tab's filter being edited at once.
    pub filter_text: String,
    pub is_filtering: bool,
    pub minions_filter: String,
    pub jobs_filter: String,
    pub alerts_filter: String,
    pub api_filter: String,
    /// Events-tab filter actually applied to `self.events` — committed from
    /// `filter_text` on Enter rather than re-filtered on every keystroke.
    /// See `filtered_events`/`get_filtered_event`.
    pub applied_filter: String,

    // Cursor positions
    pub event_selected: usize,
    /// Events are displayed oldest-at-top/newest-at-bottom, tail-style. While
    /// following, the selection stays pinned to the newest event as new ones
    /// arrive (like `tail -f`); the moment the user moves selection off the
    /// last row, following turns off so a busy master doesn't yank the
    /// selection out from under them, and turns back on once they navigate
    /// back down to the tail (or press End).
    pub events_following: bool,
    pub job_selected: usize,
    pub alert_selected: usize,
    pub minion_selected: usize,
    pub pulse_selected: usize,
    pub pulse_grid_cols: usize,
    /// The dot grid's actual rendered inner area, set by draw_dot_panel
    /// each frame — mouse-click hit testing reads this directly instead of
    /// re-deriving the panel's screen geometry by hand, so the two can't
    /// silently drift apart the way they did when the Legend panel was
    /// added below the grid without the click handler being updated.
    pub pulse_grid_area: Rect,

    // Agent NDJSON reader thread
    pub agent_rx: Arc<Mutex<VecDeque<AgentMessage>>>,
    /// Count of inbound messages the reader thread dropped because
    /// `agent_rx` hit its cap (see agent::MAX_QUEUE_LEN) — surfaced in the
    /// title frame so a sustained event-bus burst outrunning the drain rate
    /// is visible instead of silently growing memory.
    pub agent_backlog_dropped: Arc<std::sync::atomic::AtomicU64>,
    agent_req_id: u64,
    config_tick: u64,
    /// Pending jid_lookup (request_id, jid) waiting for async result
    /// Pending jid_lookup (request_id, jid) for auto-updating the popup
    pending_jid_lookup: Option<(u64, String)>,
    /// When pending_jid_lookup was started, for timeout detection
    pending_jid_lookup_at: Option<Instant>,
    /// When Y is pressed, stores (request_id, jid) so the response auto-copies JSON
    pending_y_copy: Option<(u64, String)>,
    /// Event Detail's own jid_lookup, requested when pretty view is first
    /// toggled on for a job-return event (request_id, jid, minion_id) — kept
    /// separate from `pending_jid_lookup` (Job Detail's own) since the two
    /// popups can be independently open/closed across a popup return-stack.
    pending_event_pretty: Option<(u64, String, String)>,
    /// When `pending_event_pretty` was started, for timeout detection
    pending_event_pretty_at: Option<Instant>,
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
    /// Per-minion last ZMQ return timestamp for Pulse tab flash effect
    pub last_zmq_return: HashMap<String, Instant>,
    /// Timestamp of last successful clipboard copy (for UI feedback)
    pub clipboard_at: Option<Instant>,
    /// Timestamp of last successful save-to-file (separate from
    /// `clipboard_at` so the popup's transient "✓ Copied"/"✓ Saved"
    /// indicator shows the label matching what actually happened)
    pub save_at: Option<Instant>,
    /// Transient message shown in status bar (e.g. clipboard hints)
    pub clipboard_msg: Option<(String, Instant)>,
    /// Active "save to file" prompt (the 's'/'S' counterpart of 'y'/'Y'
    /// clipboard copy), if one is open. Global overlay, like ConfigEditor.
    pub save_prompt: Option<SaveFilePrompt>,
    pub sparkline_style: SparklineStyle,
    /// Flag for main loop to call terminal.clear() after alt-screen disruption
    pub needs_terminal_clear: bool,
    /// Set to true when config editor saves — main loop will restart
    pub needs_restart: bool,
    /// Dashboard ZMQ panel's latency figure, in milliseconds. Populated by
    /// whichever mechanism `config.zmq_latency_source` selects — see the
    /// three `*_ping`/`*_pending` fields below for the source-specific state.
    pub zmq_latency_ms: f64,
    /// Whether a real sample has ever been recorded under the *current*
    /// source — distinguishes "no data yet" from a genuine ~0ms reading.
    /// Reset on every config save so a stale figure from a previously
    /// selected source can't linger mislabeled under a new one.
    pub zmq_latency_seen: bool,
    /// Ring buffer of recent samples for the sparkline (cadence depends on
    /// the active source: ~every tick for agent_ipc, irregular/periodic for
    /// job_ping and master_stats)
    pub zmq_latency_history: [f64; 2000],
    pub zmq_latency_idx: usize,
    /// agent_ipc source: when the last zmq_ping was sent, for Rust-side
    /// round-trip measurement
    zmq_ping_sent_at: Option<Instant>,
    /// job_ping source: (jid, sent_at) of an in-flight periodic test.ping,
    /// timed from dispatch to its first real /ret event on the event bus
    job_ping_pending: Option<(String, Instant)>,

    /// Rolling window of individual job-return outcomes (success/fail)
    pub job_returns_window: VecDeque<bool>,
    /// Rolling window of full-completion latencies in seconds
    pub job_completion_latencies: VecDeque<f64>,
    /// Rolling window of job close-out outcomes (true = partial return)
    pub job_outcome_window: VecDeque<bool>,

    // salt-api stats tracking (see App::update's "get_api_stats" handler).
    // cpstats' Requests/Second is a lifetime average since salt-api started,
    // which barely moves on a long-running server — prev_api_total_requests
    // lets us compute a real recent rate from the delta instead, the same
    // way network/CPU rates are derived elsewhere in this file.
    prev_api_total_requests: u64,
    prev_api_poll_time: Option<Instant>,
    pub api_requests_per_sec_recent: f64,
    /// Last-seen End Time per worker thread ID, so a newly-observed request
    /// (End Time advanced since last poll) can be told apart from the same
    /// still-latest one cpstats keeps showing between polls — without this,
    /// a quiet API would double-count its last request on every poll.
    api_seen_end_times: HashMap<String, f64>,
    /// Rolling window of individual (non-cpstats-provided) request outcomes:
    /// true = 2xx, false = 4xx/5xx — cpstats has no cumulative error counter.
    pub api_status_window: VecDeque<bool>,
    /// Rolling window of POST /login processing times, in seconds.
    pub api_login_latencies: VecDeque<f64>,
    /// Every newly-observed salt-api request (see api_seen_end_times above),
    /// oldest first, capped at config.api_log_cap — feeds the API tab's
    /// request log and per-endpoint breakdown.
    pub api_requests_log: VecDeque<ApiLogEntry>,
    pub api_log_selected: usize,

    /// Per-pid (cpu_ticks, sampled_at) from the previous list_processes poll,
    /// used to compute cpu_percent from the delta between polls.
    prev_proc_cpu: HashMap<u32, (u64, Instant)>,
    /// Clock ticks per second on the master host (from the agent's os.sysconf(SC_CLK_TCK))
    clk_tck: f64,

    /// salt/auth events seen in the current 1s window
    auth_events_this_sec: u64,
    /// Ring buffer of recent auth rates (events/sec) for sparkline
    pub auth_rate_history: [f64; 2000],
    pub auth_rate_idx: usize,
    /// Most recent auth events/sec sample
    pub current_auth_rate: f64,

    /// PID-based temp paths to avoid conflicts between users/instances
    pid: u32,
    cmd_path: String,
    copy_path: String,
    clip_log_path: String,
    clip_debug_path: String,

    // ── Activity matrix (job return events per sec) ──────────────────────────
    /// Count of salt/job/*/ret/* events per second for last 60s (ring buffer)
    pub activity_matrix: [u64; 2000],
    pub activity_matrix_idx: usize,

    /// Ring buffers of CPU/Memory utilization percent, sampled once per
    /// second alongside the other *_history buffers above — feeds the
    /// Dashboard's optional CPU/Memory sparkline panels (config.show_cpu_graph
    /// / show_mem_graph).
    pub cpu_history: [f64; 2000],
    pub cpu_history_idx: usize,
    pub mem_history: [f64; 2000],
    pub mem_history_idx: usize,
    /// How many real cpu_history/mem_history samples have been recorded so
    /// far (saturating past 2000) — lets the CPU/Memory panels' "avg" stat
    /// average only over genuinely-recorded samples instead of being diluted
    /// by the ring buffer's zero-initialized, not-yet-written slots during
    /// the first few minutes after launch.
    pub cpu_mem_samples: u64,
    pub current_sec_returns: u64,
    pub last_activity_minion: String,
    pub last_activity_fun: String,
    pub last_activity_at: Instant,
    /// Animation frame index for radar spinner in header
    pub header_radar_frame: u8,

    /// User-adjustable tick interval in milliseconds (default 2000 = 2s)
    pub tick_interval_ms: u64,

    // Network rate tracking
    prev_network_rx_bytes: u64,
    prev_network_tx_bytes: u64,
    prev_network_time: Option<Instant>,

    // System-wide CPU% tracking (from /proc/stat cumulative jiffies)
    prev_cpu_total_jiffies: u64,
    prev_cpu_idle_jiffies: u64,
    prev_cpu_time: Option<Instant>,

    // Mouse support
    pub mouse_enabled: bool,
    pub needs_mouse_toggle: bool,
    last_click_at: Option<Instant>,
    last_click_col: u16,
    last_click_row: u16,

    // Sniffer
    pub sniffer_input_active: bool,
    pub sniffer_input_text: String,
    pub sniffer_dirty: bool,
    /// Lives outside `detail_popup` (unlike every other popup) because it's
    /// meant to keep capturing in the background while you use other tabs
    /// or open unrelated popups there -- it only ever displays itself while
    /// on the Events tab, and only when no other Events-tab popup (e.g. a
    /// SaltEvent detail opened from within it) is on top of it.
    pub sniffer: Option<SnifferState>,
    /// When viewing event detail from MinionEvents, pressing q returns to the list
    pub return_to_minion_events: Option<(String, usize)>,
    /// When admin mode is enabled/cancelled from within the Minion Detail
    /// popup (its own hint advertises `A` for this), the popup should come
    /// back afterward instead of dropping to the Minions list.
    pub return_to_minion_detail: Option<String>,
    /// Viewport-aware last valid scroll position for the Event Detail
    /// popup, recomputed and written back every render (see
    /// `ui::detail_popup::draw_event_detail`) — the key handler clamps
    /// Down/PageDown/End against this real bound instead of guessing,
    /// which is what keeps PageUp responsive right after End.
    pub event_detail_scroll_max: usize,
    /// Same as `event_detail_scroll_max`, for the Job Detail popup's return
    /// content preview (right panel).
    pub job_detail_scroll_max: usize,
    /// Filter state for MinionEvents popup
    pub minion_events_filter: String,
    pub minion_events_is_filtering: bool,

    /// Hide timed-out (non-responsive) minions from list
    pub hide_timed_out: bool,
    /// Collapse the timed-out minion section header
    pub timed_out_collapsed: bool,

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
    /// Live count of jobs read so far during a `jobs_list` fetch, from the
    /// agent's incremental progress messages — `None` until the first one
    /// arrives (or if the agent fell back to the non-incremental path).
    pub cache_load_progress: Option<usize>,
}

#[derive(Clone, Debug)]
pub enum AgentMessage {
    Result { id: u64, cmd: String, ok: bool, data: serde_json::Value },
    Event { tag: String, data: serde_json::Value, ts: f64 },
    Error { id: Option<u64>, message: String },
    /// Incremental progress for a long-running command (currently just
    /// `jobs_list`'s live job-cache walk) — arrives zero or more times
    /// before the final `Result`.
    Progress { cmd: String, loaded: usize },
}

#[derive(Clone, Debug)]
pub enum PingState {
    Idle,
    Pinging(Instant),
    Result(bool),
}

// ─── Config editor ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub enum ConfigFieldKind {
    Section,
    Str,
    U64,
    F64,
    /// A field with a fixed, small set of valid values — edited via an
    /// inline picker (arrow keys + Enter) instead of free-text entry.
    Choice(&'static [&'static str]),
    /// A comma-separated ordered list (currently just `graph_order`) —
    /// edited by moving a cursor between items (arrow keys) and moving the
    /// selected item up/down in the order (`+`/`-`), instead of hand-typing
    /// the raw comma-separated string where a typo or dropped token is easy
    /// to make and hard to notice.
    Order,
    SaveButton,
}

pub const ZMQ_LATENCY_SOURCES: &[&str] = &["agent_ipc", "job_ping", "master_stats"];
pub const ON_OFF: &[&str] = &["on", "off"];
pub const API_STATS_AUTH_MODES: &[&str] = &["none", "plaintext", "pillar"];

fn on_off(b: bool) -> String {
    if b { "on" } else { "off" }.to_string()
}

#[derive(Clone, Debug)]
pub struct ConfigField {
    pub label: &'static str,
    pub label_value: String,
    pub kind: ConfigFieldKind,
}

/// State for the 's'/'S' "save to file" prompt — proposes a filename (based
/// on whatever content is about to be saved), lets the user edit it, and
/// requires a second Enter to overwrite if that name already exists in the
/// target directory (see `confirm_overwrite`).
#[derive(Clone, Debug)]
pub struct SaveFilePrompt {
    pub filename: String,
    pub content: String,
    /// Set once the user has already been warned the target file exists
    /// and pressed Enter again to proceed. Reset by any further edit to
    /// `filename`, so a changed name is re-checked from scratch.
    pub confirm_overwrite: bool,
}

#[derive(Clone, Debug)]
pub struct ConfigEditorState {
    pub fields: Vec<ConfigField>,
    pub selected: usize,
    pub editing: bool,
    pub input: String,
    /// Cursor position within the option list while editing a `Choice` field
    pub choice_idx: usize,
    pub save_error: Option<String>,
}

/// A single item in the unified per-minion activity view.
#[derive(Clone, Debug)]
pub enum MinionActivityItem {
    Event(usize, SaltEvent),
    CacheJob(CacheJobEntry),
    Alert(AlertEntry),
}

impl MinionActivityItem {
    pub fn ts(&self) -> f64 {
        match self {
            MinionActivityItem::Event(_, evt) => evt.ts,
            MinionActivityItem::CacheJob(job) => job.start_epoch,
            MinionActivityItem::Alert(_) => 0.0,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            MinionActivityItem::Event(_, evt) => &evt.tag,
            MinionActivityItem::CacheJob(job) => &job.fun,
            MinionActivityItem::Alert(alert) => &alert.description,
        }
    }
}

// ─── Detail popup variants ──────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum DetailPopup {
    Minion(String, PingState, Option<Vec<(String, String)>>, usize),
    SaltEvent { evt: SaltEvent, scroll: usize, pretty: bool, pretty_fetch: crate::types::PrettyFetch },
    JobDetail { jid: String, lookup: Option<JidLookup>, selected_return: Option<usize>, job_fun: String, job_tgt: String, job_start_epoch: f64, minion_scroll: usize, return_filter: String, return_is_filtering: bool, panel_focus: bool, content_scroll: usize, content_scroll_mode: bool },
    Alert(AlertEntry),
    AdminConfirm,
    ConfigEditor(ConfigEditorState),
    MinionEvents { minion_id: String, selected: usize },
    ApiRequest(ApiLogEntry),
}

impl App {
    pub fn new() -> Self {
        let config_path = crate::config::resolve_config_path();
        let config = load_config(&config_path);
        let config_secs = config.config_secs;
        let event_buffer_size = config.event_buffer_size;
        let agent_rx: Arc<Mutex<VecDeque<AgentMessage>>> = Arc::new(Mutex::new(VecDeque::new()));
        let agent_backlog_dropped = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let pid = std::process::id();

        // PID-based temp paths to avoid conflicts between multiple users or instances
        let cmd_path = format!("/tmp/.salt-radar-cmd-{}", pid);
        let copy_path = format!("/tmp/.salt-radar-copy-{}.txt", pid);
        let clip_log_path = format!("/tmp/.salt-radar-clip-{}.log", pid);
        let clip_debug_path = format!("/tmp/.salt-radar-clip-debug-{}.log", pid);

        // Spawn agent
        let rx = agent_rx.clone();
        let dropped = agent_backlog_dropped.clone();
        let agent_path = config.agent_path.clone();
        let python_bin = config.python_bin.clone();
        let master_config = config.master_config.clone();
        let cmd_path_agent = cmd_path.clone();
        std::thread::spawn(move || {
            crate::agent::agent_main(rx, dropped, &agent_path, &python_bin, &master_config, &cmd_path_agent);
        });

        Self {
            config,
            config_path,
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
            accepted_id_set: HashSet::new(),
            processes: Vec::new(),
            services: Vec::new(),
            service_trends: HashMap::new(),
            zmq_connections: Vec::new(),
            system_info: SystemInfo::default(),
            api_stats: ApiStats::default(),

            minion_info: Vec::new(),
            minion_info_dirty: false,

            events: VecDeque::with_capacity(event_buffer_size),
            events_since_last_tick: Vec::new(),
            event_breakdown: EventBreakdown::default(),
            events_per_sec: 0.0,
            filtered_events_cache: VecDeque::new(),

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
            sniffer: None,
            return_to_minion_events: None,
            return_to_minion_detail: None,
            event_detail_scroll_max: 0,
            job_detail_scroll_max: 0,
            minion_events_filter: String::new(),
            minion_events_is_filtering: false,
            hide_timed_out: false,
            timed_out_collapsed: false,
            mouse_enabled: true,
            needs_mouse_toggle: false,
            last_click_at: None,
            last_click_col: 0,
            last_click_row: 0,
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
            cache_load_progress: None,

            admin_mode: false,
            admin_mode_pending: false,
            fleet_scanning: false,
            fleet_scan_completed: false,
            fleet_data: None,

            filter_text: String::new(),
            is_filtering: false,
            minions_filter: String::new(),
            jobs_filter: String::new(),
            alerts_filter: String::new(),
            api_filter: String::new(),
            applied_filter: String::new(),

            event_selected: 0,
            events_following: true,
            job_selected: 0,
            alert_selected: 0,
            minion_selected: 0,
            pulse_selected: 0,
            pulse_grid_cols: 0,
            pulse_grid_area: Rect::default(),

            agent_rx,
            agent_backlog_dropped,
            agent_req_id: 0,
            config_tick: config_secs,
            pending_jid_lookup: None,
            pending_jid_lookup_at: None,
            pending_y_copy: None,
            pending_event_pretty: None,
            pending_event_pretty_at: None,
            events_total: 0,
            events_last_sec_count: 0,
            events_last_sec_time: Instant::now(),
            secs_since_last_event: 0,
            last_event_time: None,
            event_rate_history: [0.0; 2000],
            event_rate_idx: 0,
            last_zmq_return: HashMap::new(),
            clipboard_at: None,
            save_at: None,
            clipboard_msg: None,
            save_prompt: None,
            sparkline_style: SparklineStyle::default(),
            needs_terminal_clear: false,
            needs_restart: false,
            zmq_latency_ms: 0.0,
            zmq_latency_seen: false,
            zmq_latency_history: [0.0; 2000],
            zmq_latency_idx: 0,
            zmq_ping_sent_at: None,
            job_ping_pending: None,
            job_returns_window: VecDeque::new(),
            job_completion_latencies: VecDeque::new(),
            prev_api_total_requests: 0,
            prev_api_poll_time: None,
            api_requests_per_sec_recent: 0.0,
            api_seen_end_times: HashMap::new(),
            api_status_window: VecDeque::new(),
            api_login_latencies: VecDeque::new(),
            api_requests_log: VecDeque::new(),
            api_log_selected: 0,
            job_outcome_window: VecDeque::new(),
            prev_proc_cpu: HashMap::new(),
            clk_tck: 100.0,
            auth_events_this_sec: 0,
            auth_rate_history: [0.0; 2000],
            auth_rate_idx: 0,
            current_auth_rate: 0.0,

            activity_matrix: [0; 2000],
            activity_matrix_idx: 0,
            cpu_history: [0.0; 2000],
            cpu_history_idx: 0,
            mem_history: [0.0; 2000],
            mem_history_idx: 0,
            cpu_mem_samples: 0,
            current_sec_returns: 0,
            last_activity_minion: String::new(),
            last_activity_fun: String::new(),
            last_activity_at: Instant::now(),
            header_radar_frame: 0,

            tick_interval_ms: 2000,

            prev_network_rx_bytes: 0,
            prev_network_tx_bytes: 0,
            prev_network_time: None,

            prev_cpu_total_jiffies: 0,
            prev_cpu_idle_jiffies: 0,
            prev_cpu_time: None,

            pid,
            cmd_path,
            copy_path,
            clip_log_path,
            clip_debug_path,
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
        // Index by minion ID to avoid an O(n) Vec scan per minion/event in
        // this batch. Rebuilt whenever minion_info is structurally replaced
        // (see the "minion_cache_list" arm below).
        let mut minion_index: HashMap<String, usize> = self.minion_info
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id.clone(), i))
            .collect();
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
                        "minion_cache_list" if ok => {
                            let mut minions = parse_minion_list(&data);
                            for minion in &mut minions {
                                if minion.key_status.is_none() {
                                    minion.key_status = Some(KeyStatus::Accepted);
                                }
                                // Preserve enriched fields from event processing
                                if let Some(&i) = minion_index.get(&minion.id) {
                                    let existing = &self.minion_info[i];
                                    if !existing.osfinger.is_empty() { minion.osfinger.clone_from(&existing.osfinger); }
                                    if !existing.osrelease.is_empty() { minion.osrelease.clone_from(&existing.osrelease); }
                                    if !existing.salt_version.is_empty() { minion.salt_version.clone_from(&existing.salt_version); }
                                    if !existing.cpuarch.is_empty() { minion.cpuarch.clone_from(&existing.cpuarch); }
                                    if !existing.os.is_empty() { minion.os.clone_from(&existing.os); }
                                    if existing.last_seen_epoch > minion.last_seen_epoch { minion.last_seen_epoch = existing.last_seen_epoch; }
                                    if existing.cache_updated_epoch > minion.cache_updated_epoch { minion.cache_updated_epoch = existing.cache_updated_epoch; }
                                    // Fleet scan timed_out status is not known to the cache — preserve it
                                    if existing.timed_out { minion.timed_out = true; }
                                    // Being present in the master's minion cache means it was
                                    // synced at some point — it says nothing about whether the
                                    // minion is reachable *now*. Only alive_check (the real
                                    // liveness probe) should set this; otherwise it fights with
                                    // alive_check's result every cache refresh, flapping the
                                    // Pulse Details panel between Connected/Unknown regardless
                                    // of actual minion state.
                                    minion.connected = existing.connected;
                                }
                            }
                            // Sort: timed-out last, then by ID
                            minions.sort_by(|a, b| {
                                a.timed_out
                                    .cmp(&b.timed_out)
                                    .then(a.id.cmp(&b.id))
                            });
                            self.minion_info = minions;
                            // Roster changed shape — reindex, and drop pulse-tracking
                            // entries for minions no longer in the roster so
                            // last_zmq_return doesn't grow unbounded on churn.
                            minion_index = self.minion_info
                                .iter()
                                .enumerate()
                                .map(|(i, m)| (m.id.clone(), i))
                                .collect();
                            self.last_zmq_return.retain(|id, _| minion_index.contains_key(id));
                        }
                        "list_keys" => {
                            if ok {
                                self.keys = serde_json::from_value(data["keys"].clone()).unwrap_or_default();
                                self.accepted_id_set = self.keys.iter()
                                    .filter(|k| k.status == KeyStatus::Accepted)
                                    .map(|k| k.id.clone())
                                    .collect();
                            }
                        }
                        "list_processes" => {
                            if ok {
                                if let Some(tck) = data["clk_tck"].as_f64() {
                                    self.clk_tck = tck;
                                }
                                self.processes = serde_json::from_value(data["processes"].clone()).unwrap_or_default();
                                let now = Instant::now();
                                let mut seen_pids: HashSet<u32> = HashSet::new();
                                for p in self.processes.iter_mut() {
                                    seen_pids.insert(p.pid);
                                    if let Some((prev_ticks, prev_at)) = self.prev_proc_cpu.get(&p.pid) {
                                        let elapsed = now.duration_since(*prev_at).as_secs_f64();
                                        if elapsed > 0.0 {
                                            let tick_delta = p.cpu_ticks.saturating_sub(*prev_ticks);
                                            p.cpu_percent = ((tick_delta as f64 / self.clk_tck) / elapsed * 100.0) as f32;
                                        }
                                    }
                                    self.prev_proc_cpu.insert(p.pid, (p.cpu_ticks, now));
                                }
                                self.prev_proc_cpu.retain(|pid, _| seen_pids.contains(pid));
                            }
                        }
                        "list_services" => {
                            if ok {
                                self.services = serde_json::from_value(data["services"].clone()).unwrap_or_default();
                                update_service_trends(&mut self.service_trends, &self.services);
                            }
                        }
                        "get_system_info" => {
                            if ok {
                                self.system_info = serde_json::from_value(data.clone()).unwrap_or_default();
                                let now = Instant::now();
                                if let Some(prev_at) = self.prev_cpu_time {
                                    let elapsed = now.duration_since(prev_at).as_secs_f64();
                                    let total_delta = self.system_info.cpu_total_jiffies.saturating_sub(self.prev_cpu_total_jiffies);
                                    let idle_delta = self.system_info.cpu_idle_jiffies.saturating_sub(self.prev_cpu_idle_jiffies);
                                    if elapsed > 0.0 && total_delta > 0 {
                                        let busy_delta = total_delta.saturating_sub(idle_delta);
                                        self.system_info.cpu_percent = (busy_delta as f64 / total_delta as f64 * 100.0).clamp(0.0, 100.0);
                                    }
                                }
                                self.prev_cpu_total_jiffies = self.system_info.cpu_total_jiffies;
                                self.prev_cpu_idle_jiffies = self.system_info.cpu_idle_jiffies;
                                self.prev_cpu_time = Some(now);
                            }
                        }
                        "get_zmq_connections" => {
                            if ok {
                                self.zmq_connections = serde_json::from_value(data["connections"].clone()).unwrap_or_default();
                            }
                        }
                        "get_api_stats" => {
                            if ok {
                                self.api_stats = ApiStats {
                                    configured: data["configured"].as_bool().unwrap_or(false),
                                    reachable: data["reachable"].as_bool().unwrap_or(false),
                                    port: data["port"].as_u64().unwrap_or(0) as u16,
                                    scheme: data["scheme"].as_str().unwrap_or("").to_string(),
                                    error: data["error"].as_str().unwrap_or("").to_string(),
                                    current_requests: data["current_requests"].as_u64().unwrap_or(0),
                                    total_requests: data["total_requests"].as_u64().unwrap_or(0),
                                    requests_per_sec: data["requests_per_sec"].as_f64().unwrap_or(0.0),
                                    bytes_read_per_sec: data["bytes_read_per_sec"].as_f64().unwrap_or(0.0),
                                    bytes_written_per_sec: data["bytes_written_per_sec"].as_f64().unwrap_or(0.0),
                                    uptime_secs: data["uptime_secs"].as_f64().unwrap_or(0.0),
                                    requests: serde_json::from_value(data["requests"].clone()).unwrap_or_default(),
                                };

                                if self.api_stats.reachable {
                                    let now = Instant::now();
                                    if let Some(prev_at) = self.prev_api_poll_time {
                                        let elapsed = now.duration_since(prev_at).as_secs_f64();
                                        let delta = self.api_stats.total_requests.saturating_sub(self.prev_api_total_requests);
                                        if elapsed > 0.0 {
                                            self.api_requests_per_sec_recent = delta as f64 / elapsed;
                                        }
                                    }
                                    self.prev_api_total_requests = self.api_stats.total_requests;
                                    self.prev_api_poll_time = Some(now);

                                    // Counts entries evicted from the front (oldest) this
                                    // poll, whether by the cap below or the age-based
                                    // prune after the loop, so api_log_selected can be
                                    // shifted to keep pointing at the same logical
                                    // entry -- the same correction Events already makes
                                    // via advance_tail_selection when its buffer evicts.
                                    let mut api_log_evicted: usize = 0;
                                    for rec in &self.api_stats.requests {
                                        if rec.end_time <= 0.0 {
                                            continue;
                                        }
                                        let is_new = self.api_seen_end_times.get(&rec.thread_id)
                                            .map(|prev_end| rec.end_time > *prev_end)
                                            .unwrap_or(true);
                                        self.api_seen_end_times.insert(rec.thread_id.clone(), rec.end_time);
                                        if !is_new {
                                            continue;
                                        }

                                        let is_ok = rec.response_status.split_whitespace().next()
                                            .map(|code| code.starts_with('2') || code.starts_with('3'))
                                            .unwrap_or(true);
                                        self.api_status_window.push_back(is_ok);
                                        if self.api_status_window.len() > self.config.api_stats_window { self.api_status_window.pop_front(); }

                                        if rec.request_line.contains("/login") {
                                            self.api_login_latencies.push_back(rec.processing_time_secs);
                                            if self.api_login_latencies.len() > self.config.api_stats_window { self.api_login_latencies.pop_front(); }
                                        }

                                        let (method, path) = Self::parse_request_line(&rec.request_line);
                                        // salt-radar's own GET /stats poll (the request that
                                        // populates this very panel) is always stored -- it's
                                        // filtered out at display time instead (see
                                        // api_log_entry_visible) so toggling `x` live shows/
                                        // hides it retroactively rather than only affecting
                                        // requests captured from that point on.
                                        self.api_requests_log.push_back(ApiLogEntry {
                                            received_at: now,
                                            client: rec.client.clone(),
                                            method,
                                            path,
                                            raw_request_line: rec.request_line.clone(),
                                            response_status: rec.response_status.clone(),
                                            processing_time_secs: rec.processing_time_secs,
                                        });
                                        if self.api_requests_log.len() > self.config.api_log_cap {
                                            self.api_requests_log.pop_front();
                                            api_log_evicted += 1;
                                        }
                                    }
                                    // Threads that vanish from the snapshot (worker recycled,
                                    // thread pool shrank) would otherwise linger here forever.
                                    let live_threads: HashSet<&str> = self.api_stats.requests.iter().map(|r| r.thread_id.as_str()).collect();
                                    self.api_seen_end_times.retain(|tid, _| live_threads.contains(tid.as_str()));

                                    let len_before_age_prune = self.api_requests_log.len();
                                    let api_log_max_age = Duration::from_secs(self.config.api_log_max_age_secs);
                                    self.api_requests_log.retain(|e| e.received_at.elapsed() < api_log_max_age);
                                    api_log_evicted += len_before_age_prune - self.api_requests_log.len();

                                    self.api_log_selected = self.api_log_selected.saturating_sub(api_log_evicted);
                                }
                            }
                        }
                        "zmq_ping" => {
                            if let Some(sent) = self.zmq_ping_sent_at {
                                let elapsed_ms = sent.elapsed().as_secs_f64() * 1000.0;
                                self.zmq_latency_ms = elapsed_ms;
                                self.zmq_latency_seen = true;
                                self.zmq_latency_history[self.zmq_latency_idx] = elapsed_ms;
                                self.zmq_latency_idx = (self.zmq_latency_idx + 1) % 2000;
                            }
                            self.zmq_ping_sent_at = None;
                        }
                        "job_ping" => {
                            // Fill in the real jid on the pending entry created
                            // at dispatch time — keep that earlier Instant so
                            // the round-trip clock still starts from before
                            // the agent actually published the job, not from
                            // when this response happens to arrive.
                            let jid = if ok { data["jid"].as_str().map(String::from) } else { None };
                            match (jid, self.job_ping_pending.take()) {
                                (Some(jid), Some((_, sent_at))) => {
                                    self.job_ping_pending = Some((jid, sent_at));
                                }
                                _ => {
                                    self.job_ping_pending = None;
                                }
                            }
                        }
                        "alive_check" => {
                            if ok {
                                let up: Vec<String> = data["up"].as_array()
                                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                                    .unwrap_or_default();
                                // Mark up minions connected, clear others
                                for minion in &mut self.minion_info {
                                    minion.connected = up.contains(&minion.id);
                                }
                            }
                        }
                        "minion_ping" => {
                            if let Some(mid) = data["minion_id"].as_str() {
                                let ping_ok = data["connected"].as_bool().unwrap_or(false);
                                if let Some(minion) = self.minion_info.iter_mut().find(|m| m.id == mid) {
                                    minion.connected = ping_ok;
                                }
                                // Update popup state
                                if let Some(DetailPopup::Minion(ref popup_id, ref mut state, _, _)) = self.detail_popup {
                                    if popup_id == mid {
                                        *state = PingState::Result(ping_ok);
                                    }
                                }
                            }
                        }
                        "minion_grains" => {
                            if let Some(mid) = data["minion_id"].as_str() {
                                let pairs: Vec<(String, String)> = data["grains"].as_array()
                                    .map(|arr| {
                                        arr.iter().filter_map(|v| {
                                            let obj = v.as_object()?;
                                            let key = obj.get("key")?.as_str()?;
                                            let val = obj.get("value")?.as_str()?;
                                            Some((key.to_string(), val.to_string()))
                                        }).collect()
                                    })
                                    .unwrap_or_default();
                                if let Some(DetailPopup::Minion(ref popup_id, _, ref mut grains, _)) = self.detail_popup {
                                    if popup_id == mid {
                                        *grains = Some(pairs);
                                    }
                                }
                            }
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
                                self.fleet_scan_completed = false;
                                self.fleet_data = Some(FleetScan {
                                    minions: Vec::new(),
                                    scanned_at: Instant::now(),
                                });
                            } else {
                                self.fleet_scanning = false;
                                self.fleet_scan_completed = true;
                            }
                        }
                        "jobs_list" => {
                            self.cache_loading_at = None;
                            self.cache_load_progress = None;
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
                            // Event Detail's own pretty-view fetch: find this
                            // specific minion's real formatted_text (or note
                            // its absence) and hand it to the open popup.
                            if let Some((ev_id, _, ref mid)) = self.pending_event_pretty {
                                if id == ev_id {
                                    let text = jid_data.returns.iter()
                                        .find(|r| &r.minion_id == mid)
                                        .map(|r| r.formatted_text.clone())
                                        .unwrap_or_default();
                                    self.pending_event_pretty = None;
                                    self.pending_event_pretty_at = None;
                                    if let Some(DetailPopup::SaltEvent { ref mut pretty_fetch, .. }) = self.detail_popup {
                                        *pretty_fetch = if text.trim().is_empty() {
                                            crate::types::PrettyFetch::Failed
                                        } else {
                                            crate::types::PrettyFetch::Ready(text)
                                        };
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
                    // Track event breakdown
                    self.event_breakdown.record(&tag);
                    if tag.starts_with("salt/auth") {
                        self.auth_events_this_sec += 1;
                    }

                    // ZMQ panel latency, source "master_stats": Salt's own
                    // instrumentation (requires master_stats: True in the master
                    // config, which salt-radar doesn't set itself) — one event
                    // per worker process, each carrying that worker's own mean
                    // command-execution times since its last reset. Collapse to
                    // a single weighted-mean sample per event.
                    if self.config.zmq_latency_source == "master_stats" && tag.starts_with("salt/stats/") {
                        if let Some(stats) = data["stats"].as_object() {
                            let mut weighted = 0.0;
                            let mut total_runs: u64 = 0;
                            for v in stats.values() {
                                let mean_secs = v["mean"].as_f64().unwrap_or(0.0);
                                let runs = v["runs"].as_u64().unwrap_or(0);
                                weighted += mean_secs * runs as f64;
                                total_runs += runs;
                            }
                            if total_runs > 0 {
                                let avg_ms = weighted / total_runs as f64 * 1000.0;
                                self.zmq_latency_ms = avg_ms;
                                self.zmq_latency_seen = true;
                                self.zmq_latency_history[self.zmq_latency_idx] = avg_ms;
                                self.zmq_latency_idx = (self.zmq_latency_idx + 1) % 2000;
                            }
                        }
                    }

                    // ZMQ panel latency, source "job_ping": time our own
                    // periodic test.ping from dispatch to its first real
                    // return over the actual event bus/ZMQ transport.
                    if let Some((ref pending_jid, sent_at)) = self.job_ping_pending {
                        if !pending_jid.is_empty() && tag.starts_with("salt/job/") && tag.contains(pending_jid.as_str()) && tag.contains("/ret/") {
                            let elapsed_ms = sent_at.elapsed().as_secs_f64() * 1000.0;
                            self.zmq_latency_ms = elapsed_ms;
                            self.zmq_latency_seen = true;
                            self.zmq_latency_history[self.zmq_latency_idx] = elapsed_ms;
                            self.zmq_latency_idx = (self.zmq_latency_idx + 1) % 2000;
                            self.job_ping_pending = None;
                        }
                    }

                    // ── Normal event processing ─────────────────────────
                    // Track return events for activity matrix
                    if tag.contains("/ret/") {
                        self.current_sec_returns += 1;
                        let success = data["success"].as_bool()
                            .unwrap_or_else(|| derive_success_retcode(&data["return"]).1);
                        self.job_returns_window.push_back(success);
                        if self.job_returns_window.len() > JOB_STATS_WINDOW { self.job_returns_window.pop_front(); }
                        if let Some(mid) = tag.rsplit('/').next() {
                            if !mid.is_empty() {
                                self.last_activity_minion = mid.to_string();
                                self.last_activity_fun = data["fun"].as_str().unwrap_or("?").to_string();
                                self.last_activity_at = Instant::now();
                                // Update last_seen for this minion from return events
                                let ts_epoch = ts as u64;
                                let minion_pos = minion_index.get(mid).copied();
                                if let Some(i) = minion_pos {
                                    let info = &mut self.minion_info[i];
                                    if ts_epoch > info.last_seen_epoch {
                                        info.last_seen_epoch = ts_epoch;
                                    }
                                    if info.key_status.is_none() {
                                        info.key_status = Some(KeyStatus::Accepted);
                                    }
                                }
                                // Passive harvest: populate minion data from grains.item/items returns
                                let fun = data["fun"].as_str().unwrap_or("");
                                if fun == "grains.items" || fun == "grains.item" {
                                    if let Some(return_data) = data["return"].as_object() {
                                        let osfinger = return_data.get("osfinger").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let osrelease = return_data.get("osrelease").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let os = return_data.get("os").or_else(|| return_data.get("osfullname")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let salt_version = return_data.get("saltversion").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let cpuarch = return_data.get("cpuarch").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let known = !osfinger.is_empty() || !salt_version.is_empty() || !os.is_empty();
                                        if known {
                                            let fm = if let Some(i) = minion_pos {
                                                let existing = &mut self.minion_info[i];
                                                if !osfinger.is_empty() { existing.osfinger = osfinger; }
                                                if !osrelease.is_empty() { existing.osrelease = osrelease; }
                                                if !os.is_empty() { existing.os = os; }
                                                if !salt_version.is_empty() { existing.salt_version = salt_version; }
                                                if !cpuarch.is_empty() { existing.cpuarch = cpuarch; }
                                                if existing.timed_out {
                                                    existing.timed_out = false;
                                                    self.minion_info_dirty = true;
                                                }
                                                if existing.key_status.is_none() {
                                                    existing.key_status = Some(KeyStatus::Accepted);
                                                }
                                                // Also populate fleet_data from the merged minion_info
                                                Some(FleetMinion {
                                                    id: mid.to_string(),
                                                    osfinger: existing.osfinger.clone(),
                                                    osrelease: existing.osrelease.clone(),
                                                    saltversion: existing.salt_version.clone(),
                                                    cpuarch: existing.cpuarch.clone(),
                                                    timed_out: false,
                                                })
                                            } else {
                                                None
                                            };
                                            if let Some(fm) = fm {
                                                if let Some(ref mut fd) = self.fleet_data {
                                                    if let Some(existing) = fd.minions.iter_mut().find(|m| m.id == mid) {
                                                        *existing = fm;
                                                    } else {
                                                        fd.minions.push(fm);
                                                    }
                                                } else {
                                                    self.fleet_data = Some(FleetScan {
                                                        minions: vec![fm],
                                                        scanned_at: Instant::now(),
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                                // Pulse tracking: every return event is minion activity
                                self.last_zmq_return.insert(mid.to_string(), Instant::now());
                            }
                        }
                    } else {
                        // Non-return events: try extracting minion ID from data for Pulse
                        if let Some(mid) = data["id"].as_str() {
                            if !mid.is_empty() {
                                self.last_zmq_return.insert(mid.to_string(), Instant::now());
                            }
                        }
                    }
                    let evt = SaltEvent { tag, data, ts };
                    self.events_since_last_tick.push(evt.clone());
                    let events_evicted = self.events.len() >= self.config.event_buffer_size;
                    let evicted_evt = if events_evicted { self.events.pop_front() } else { None };
                    // O(1) amortized: checks only the one event coming in and
                    // (at most) the one going out, instead of re-scanning the
                    // whole buffer -- see `sync_filtered_events_cache`. A
                    // busy master can push hundreds of events through a
                    // single drain_agent_messages() call, and a full re-scan
                    // per event (or even once per tick) here is what made the
                    // TUI stall/black out while an Events filter was active
                    // (#98).
                    Self::sync_filtered_events_cache(
                        &mut self.filtered_events_cache,
                        &self.applied_filter,
                        &self.accepted_id_set,
                        &evt,
                        evicted_evt.as_ref(),
                    );
                    self.events.push_back(evt);
                    self.events_total += 1;
                    self.last_event_time = Some(Instant::now());
                    if self.applied_filter.is_empty() {
                        Self::advance_tail_selection(&mut self.event_selected, self.events_following, events_evicted, self.events.len());
                    } else if self.events_following {
                        self.event_selected = self.filtered_events_cache.len().saturating_sub(1);
                    }

                    // Track live jobs from events
                    if let Some(evt) = self.events.back() {
                        track_job_from_event(&mut self.jobs, evt);
                    }

                    // Sniffer: inject matching events into sniffer buffer
                    if let Some(ref mut state) = self.sniffer {
                        if !state.paused {
                            if let Some(evt) = self.events.back() {
                                if Self::matches_sniffer_event(evt, &state.minion_ids) {
                                    let will_evict = state.buffer.len() >= state.max_size;
                                    // `selected`/`following` track the filtered (displayed)
                                    // view, not the raw buffer — only touch them when this
                                    // push/evict actually changes what's shown, i.e. when the
                                    // evicted item (if any) or the new item itself passes the
                                    // active highlight-keyword filter.
                                    let evicted_visible = will_evict && state.buffer.front()
                                        .map(|e| state.highlight_keywords.is_empty() || crate::types::SnifferState::event_matches_keywords(e, &state.highlight_keywords))
                                        .unwrap_or(false);
                                    if will_evict {
                                        state.buffer.pop_front();
                                    }
                                    state.buffer.push_back(evt.clone());
                                    let new_visible = state.highlight_keywords.is_empty()
                                        || crate::types::SnifferState::event_matches_keywords(evt, &state.highlight_keywords);
                                    if new_visible || evicted_visible {
                                        let visible_len = state.filtered_indices().len();
                                        Self::advance_tail_selection(&mut state.selected, state.following, evicted_visible, visible_len);
                                    }
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
                AgentMessage::Progress { cmd, loaded } => {
                    if cmd == "jobs_list" {
                        self.cache_load_progress = Some(loaded);
                    }
                }
            }
            if processed >= MAX_PER_DRAIN { break; }
        }
        if let Some(json) = y_copy_json {
            // Store in a temp file for clipboard
            let _ = std::fs::write(&self.copy_path, &json);
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
            .open(&self.cmd_path)
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
                    self.return_to_minion_events = None;
                    self.detail_popup = None;
                }
            }
        }

        // Timeout for Event Detail's own pretty-view fetch -- falls back to
        // the client-side approximation rather than closing the popup, since
        // (unlike Job Detail) there's already a usable render without it.
        if self.pending_event_pretty.is_some() {
            if let Some(started) = self.pending_event_pretty_at {
                if started.elapsed().as_secs() > 65 {
                    self.pending_event_pretty = None;
                    self.pending_event_pretty_at = None;
                    if let Some(DetailPopup::SaltEvent { ref mut pretty_fetch, .. }) = self.detail_popup {
                        if matches!(pretty_fetch, crate::types::PrettyFetch::Loading) {
                            *pretty_fetch = crate::types::PrettyFetch::Failed;
                        }
                    }
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
            || self.sniffer.as_ref().is_some_and(|s| !s.paused)
            // A zmq_ping/job_ping round-trip is outstanding — without this,
            // the reply sits unread in agent_rx until the main loop's
            // event::poll(timeout) happens to expire at the next full tick,
            // and since latency is measured from send to drain, that idle
            // wait gets counted as "latency" — the whole figure would then
            // just track tick_interval_ms instead of real round-trip time.
            || self.zmq_ping_sent_at.is_some()
            || self.job_ping_pending.is_some()
            // A backlog in agent_rx means the event-bus arrival rate is
            // outrunning MAX_PER_DRAIN at the normal tick cadence — poll
            // faster until drain_agent_messages() catches back up, instead
            // of leaving the excess queued for a full tick_interval_ms.
            || !self.agent_rx.lock().unwrap().is_empty()
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
        self.pulse_selected = self.pulse_selected
            .min(self.pulse_minions_count().saturating_sub(1));
        self.api_log_selected = self.api_log_selected
            .min(self.filtered_api_log_len().saturating_sub(1));
    }

    /// Clamp only the selection for the tab actively being typed/pasted into.
    ///
    /// `filter_text`/`is_filtering` (and `cache_filter_text`/`cache_is_filtering`)
    /// are shared across Minions/Events/Alerts/Api/Jobs, but only `current_tab`
    /// is ever the one being filtered (Tab/BackTab are unreachable while
    /// filtering) -- recomputing every other tab's filtered length on every
    /// keystroke, most notably Events' full JSON-stringify fallback in
    /// `matches_event`, is pure waste and the cause of the paste-a-JID-into-Jobs
    /// slowdown on masters with a large event buffer. Called from the hot
    /// per-character paths; `clamp_selection()` still runs on Esc/Enter (once
    /// per filter session) so every tab's selection stays valid once filtering
    /// stops.
    fn clamp_active_filter_selection(&mut self) {
        match self.current_tab {
            Tab::Minions => self.minion_selected = self.minion_selected
                .min(self.filtered_minions_len().saturating_sub(1)),
            Tab::Events => self.event_selected = self.event_selected
                .min(self.filtered_events_len().saturating_sub(1)),
            Tab::Alerts => self.alert_selected = self.alert_selected
                .min(self.filtered_alerts_len().saturating_sub(1)),
            Tab::Api => self.api_log_selected = self.api_log_selected
                .min(self.filtered_api_log_len().saturating_sub(1)),
            Tab::Jobs => {
                if self.cache_focus {
                    self.cache_selected = self.cache_selected
                        .min(self.filtered_cache_len().saturating_sub(1));
                } else {
                    self.job_selected = self.job_selected
                        .min(self.filtered_jobs_len().saturating_sub(1));
                }
            }
            _ => {}
        }
    }

    /// While live-typing an Events filter, keep the selection pinned to the
    /// newest match (as long as it was already following) so the narrowing
    /// result set stays tail-anchored instead of settling on whatever
    /// index the previous filter text happened to leave selected.
    fn follow_events_tail_if_filtering(&mut self) {
        if self.current_tab == Tab::Events && self.events_following {
            self.event_selected = self.filtered_events_len().saturating_sub(1);
        }
    }

    fn matches_filter(text: &str, pat: &str) -> bool {
        if pat.is_empty() { return true; }
        text.to_lowercase().contains(&pat.to_lowercase())
    }

    /// Splits a filter pattern into its comma-separated terms (trimmed, empty
    /// tokens dropped) — mirrors Salt's own list-targeting syntax
    /// (`salt -L 'web1,web2'`) instead of introducing regex: a filter of
    /// `web1, web2` means "matches web1 OR web2", not a literal substring
    /// containing a comma.
    fn filter_terms(pat: &str) -> Vec<&str> {
        pat.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()).collect()
    }

    /// Case-insensitive wildcard match (`*` = any run of characters, `?` =
    /// any single character), anchored across the whole of `text` — the same
    /// semantics as Salt's own glob minion targeting (`salt 'web*' ...`).
    /// Classic O(n*m) DP wildcard-matching table; hostnames are short enough
    /// that this is effectively instant.
    fn glob_match(text: &str, pattern: &str) -> bool {
        let t: Vec<char> = text.to_lowercase().chars().collect();
        let p: Vec<char> = pattern.to_lowercase().chars().collect();
        let (tn, pn) = (t.len(), p.len());
        let mut dp = vec![vec![false; pn + 1]; tn + 1];
        dp[0][0] = true;
        for j in 1..=pn {
            if p[j - 1] == '*' { dp[0][j] = dp[0][j - 1]; }
        }
        for i in 1..=tn {
            for j in 1..=pn {
                dp[i][j] = match p[j - 1] {
                    '*' => dp[i - 1][j] || dp[i][j - 1],
                    '?' => dp[i - 1][j - 1],
                    c => dp[i - 1][j - 1] && c == t[i - 1],
                };
            }
        }
        dp[tn][pn]
    }

    /// A term matches a host/minion-id field via glob semantics if it looks
    /// like a glob (contains `*`/`?`), otherwise via the same plain substring
    /// semantics as every other field — so `web` still matches `web1` without
    /// requiring `web*`, but `web*` anchors to the whole id like Salt's CLI
    /// targeting does.
    fn host_matches_term(host: &str, term: &str) -> bool {
        if term.contains('*') || term.contains('?') {
            Self::glob_match(host, term)
        } else {
            Self::matches_filter(host, term)
        }
    }

    fn matches_event(evt: &SaltEvent, pat: &str, known_minions: &HashSet<String>) -> bool {
        let terms = Self::filter_terms(pat);
        if terms.is_empty() { return true; }
        terms.iter().any(|term| Self::matches_event_term(evt, term, known_minions))
    }

    fn matches_event_term(evt: &SaltEvent, term: &str, known_minions: &HashSet<String>) -> bool {
        if Self::matches_filter(&evt.tag, term)
            || evt.data["fun"].as_str().map(|f| Self::matches_filter(f, term)).unwrap_or(false)
            || evt.data["id"].as_str().map(|i| Self::host_matches_term(i, term)).unwrap_or(false)
            || Self::matches_filter(&evt.data.to_string(), term)
        {
            return true;
        }
        // If the term matches a known minion ID, also show events targeted at
        // all minions ("*") or whose tgt pattern could match that minion.
        if known_minions.iter().any(|m| Self::host_matches_term(m, term)) {
            if let Some(tgt) = evt.data["tgt"].as_str() {
                if tgt == "*" || known_minions.iter().any(|m| Self::matches_filter(tgt, m)) {
                    return true;
                }
            }
        }
        false
    }

    /// Keeps a tail-following selection index correct as one item is
    /// appended to a capped, oldest-at-front buffer (Events, Sniffer) that
    /// may have just evicted its oldest entry to stay under its cap. While
    /// `following`, the selection stays pinned to the newest item (tail
    /// mode, like `tail -f`); otherwise, an eviction shifts every
    /// surviving index down by one, so `selected` is adjusted to keep
    /// pointing at the same logical item (clamped to 0 if that item was
    /// itself the one just evicted).
    fn advance_tail_selection(selected: &mut usize, following: bool, evicted: bool, new_len: usize) {
        if following {
            *selected = new_len.saturating_sub(1);
        } else if evicted {
            *selected = selected.saturating_sub(1);
        }
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

    fn matches_job(job: &JobEntry, pat: &str, known_minions: &HashSet<String>) -> bool {
        let terms = Self::filter_terms(pat);
        if terms.is_empty() { return true; }
        terms.iter().any(|term| Self::matches_job_term(job, term, known_minions))
    }

    fn matches_job_term(job: &JobEntry, term: &str, known_minions: &HashSet<String>) -> bool {
        if Self::matches_filter(&job.jid, term)
            || Self::matches_filter(&job.fun, term)
            || Self::host_matches_term(&job.tgt, term)
        {
            return true;
        }
        // Show jobs targeting "*" when searching for a known minion
        if known_minions.iter().any(|m| Self::host_matches_term(m, term)) {
            if job.tgt == "*" {
                return true;
            }
        }
        false
    }

    fn matches_alert(alert: &AlertEntry, pat: &str) -> bool {
        let terms = Self::filter_terms(pat);
        if terms.is_empty() { return true; }
        terms.iter().any(|term| {
            Self::matches_filter(alert.rule, term) || Self::matches_filter(&alert.description, term)
        })
    }

    pub fn matches_minion(minion: &MinionInfo, pat: &str) -> bool {
        let terms = Self::filter_terms(pat);
        if terms.is_empty() { return true; }
        terms.iter().any(|term| {
            Self::host_matches_term(&minion.id, term)
                || Self::matches_filter(&minion.os, term)
                || Self::matches_filter(&minion.osrelease, term)
                || Self::matches_filter(&minion.salt_version, term)
        })
    }

    /// Same matching rules as `matches_minion`, for the separate `FleetMinion`
    /// type used by the Fleet Overview scan — lets the Minions-tab filter also
    /// scope the OS/version/arch breakdown, not just the list.
    pub fn matches_fleet_minion(minion: &crate::types::FleetMinion, pat: &str) -> bool {
        let terms = Self::filter_terms(pat);
        if terms.is_empty() { return true; }
        terms.iter().any(|term| {
            Self::host_matches_term(&minion.id, term)
                || Self::matches_filter(&minion.osfinger, term)
                || Self::matches_filter(&minion.osrelease, term)
                || Self::matches_filter(&minion.saltversion, term)
        })
    }

    fn accepted_ids(&self) -> &HashSet<String> {
        &self.accepted_id_set
    }

    /// The filter terms currently driving the Events list — `filter_text`
    /// while actively typing (so highlighting tracks live as you type),
    /// `applied_filter` once confirmed (what `filtered_events()` itself
    /// matches against; see the `applied_filter` field doc for why Events
    /// debounces to Enter rather than filtering per-keystroke like every
    /// other tab). Used to color-highlight matched terms in the event list.
    /// The confirmed filter text for `tab` — the live edit buffer
    /// (`filter_text`) if `tab` is the one currently being typed into,
    /// otherwise that tab's own dedicated field, never another tab's.
    pub fn filter_text_for(&self, tab: Tab) -> &str {
        if self.is_filtering && self.current_tab == tab {
            return &self.filter_text;
        }
        match tab {
            Tab::Minions => &self.minions_filter,
            Tab::Jobs => &self.jobs_filter,
            Tab::Alerts => &self.alerts_filter,
            Tab::Api => &self.api_filter,
            _ => &self.filter_text,
        }
    }

    /// Commits `text` as `self.current_tab`'s own filter — called on
    /// Enter/Esc from the filter-input handler once editing ends. A no-op
    /// for tabs that don't keep a dedicated field (Events uses
    /// `applied_filter` instead, set directly by its own Enter/Esc arms).
    fn commit_tab_filter(&mut self, text: &str) {
        match self.current_tab {
            Tab::Minions => self.minions_filter = text.to_string(),
            Tab::Jobs => self.jobs_filter = text.to_string(),
            Tab::Alerts => self.alerts_filter = text.to_string(),
            Tab::Api => self.api_filter = text.to_string(),
            _ => {}
        }
    }

    /// Expands any glob term (contains `*`/`?`) to the concrete minion IDs
    /// it matches — a glob never literally appears in rendered text (there's
    /// no "debian12-*" substring anywhere to highlight), so used as-is it
    /// would never match anything. Falls back to the raw term if it matches
    /// no known minion, so an in-progress/nonsense glob doesn't just vanish.
    /// Plain (non-glob) terms pass through unchanged. Shared by the Events
    /// tab's own filter terms and the Event Sniffer's `color_terms` (see
    /// `SnifferState`) so both resolve host globs identically.
    fn resolve_host_terms<'a>(&self, terms: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        let known = self.accepted_ids();
        terms.into_iter().flat_map(|term| {
            if term.contains('*') || term.contains('?') {
                let matched: Vec<String> = known.iter().filter(|m| Self::glob_match(m, term)).cloned().collect();
                if matched.is_empty() { vec![term.to_string()] } else { matched }
            } else {
                vec![term.to_string()]
            }
        }).collect()
    }

    pub fn events_filter_terms(&self) -> Vec<String> {
        let pat: &str = if self.is_filtering { &self.filter_text } else { &self.applied_filter };
        let terms = Self::filter_terms(pat);
        self.resolve_host_terms(terms)
    }

    pub fn filtered_events_len(&self) -> usize {
        if self.applied_filter.is_empty() {
            self.events.len()
        } else {
            self.filtered_events_cache.len()
        }
    }

    /// Index 0 is the oldest buffered event — events display oldest-at-top,
    /// newest-at-bottom (tail-style, see `events_following`), matching
    /// `self.events`'s own front-to-back order (oldest pushed first).
    pub fn get_filtered_event(&self, index: usize) -> Option<&SaltEvent> {
        if self.applied_filter.is_empty() {
            self.events.get(index)
        } else {
            self.filtered_events_cache.get(index)
        }
    }

    /// Full filtered event list. O(1) — `filtered_events_cache` is kept in
    /// sync incrementally as events arrive/are evicted (see
    /// `sync_filtered_events_cache`) rather than re-scanned here, since a
    /// busy master can push hundreds of events through the buffer between
    /// redraws and a per-call re-filter of the whole buffer was the actual
    /// cause of the TUI stalling/blacking out while an Events filter was
    /// active (#98).
    pub fn filtered_events(&self) -> Vec<&SaltEvent> {
        if self.applied_filter.is_empty() {
            self.events.iter().collect()
        } else {
            self.filtered_events_cache.iter().collect()
        }
    }

    /// Full O(events) rebuild of `filtered_events_cache` against the current
    /// `applied_filter`. Only called when the filter itself changes
    /// (committed on Enter, or cleared on Esc/`/`) — never per-event or
    /// per-tick.
    fn rebuild_filtered_events_cache(&mut self) {
        if self.applied_filter.is_empty() {
            self.filtered_events_cache.clear();
            return;
        }
        let pat = self.applied_filter.clone();
        let known = self.accepted_ids().clone();
        self.filtered_events_cache = self.events.iter()
            .filter(|e| Self::matches_event(e, &pat, &known))
            .cloned()
            .collect();
    }

    /// Keeps `filtered_events_cache` consistent with a single event
    /// ingested/evicted from `self.events`, in O(1) amortized instead of
    /// re-scanning the buffer. Call this once per event, immediately after
    /// pushing `new_evt` and (if `evicted` is set) popping it off the front.
    ///
    /// Takes its fields individually (rather than `&mut self`) so callers
    /// already holding an unrelated borrow through `self` -- e.g.
    /// `drain_agent_messages`' `self.agent_rx.lock()` guard -- can still call
    /// it; a `&mut self` method here would conflict with that guard.
    fn sync_filtered_events_cache(
        cache: &mut VecDeque<SaltEvent>,
        pat: &str,
        known: &HashSet<String>,
        new_evt: &SaltEvent,
        evicted: Option<&SaltEvent>,
    ) {
        if pat.is_empty() {
            return;
        }
        if let Some(evicted_evt) = evicted {
            if Self::matches_event(evicted_evt, pat, known) {
                cache.pop_front();
            }
        }
        if Self::matches_event(new_evt, pat, known) {
            cache.push_back(new_evt.clone());
        }
    }

    /// Returns Vec of (index, SaltEvent) pairs for events related to the given minion_id,
    /// newest first. Matches on data["id"], tag patterns /ret/<id>, salt/minion/<id>/start,
    /// and salt/beacon/<id>. Returns owned SaltEvents to avoid borrow conflicts.
    pub fn events_for_minion(&self, minion_id: &str) -> Vec<(usize, SaltEvent)> {
        self.events
            .iter()
            .rev()
            .enumerate()
            .filter(|(_idx, evt)| {
                evt.data["id"].as_str() == Some(minion_id)
                    || evt.tag.ends_with(&format!("/ret/{}", minion_id))
                    || evt.tag == format!("salt/minion/{}/start", minion_id)
                    || evt.tag.starts_with(&format!("salt/beacon/{}", minion_id))
            })
            .map(|(idx, evt)| (idx, evt.clone()))
            .collect()
    }

    /// Unified activity list for a minion: events + cache jobs + alerts.
    pub fn minion_activity(&self, minion_id: &str) -> Vec<MinionActivityItem> {
        let mut items: Vec<MinionActivityItem> = Vec::new();

        // Events from event buffer
        for (idx, evt) in self.events_for_minion(minion_id) {
            items.push(MinionActivityItem::Event(idx, evt));
        }

        // Cache jobs targeting this minion
        for job in &self.jobs_cache {
            if job.tgt == "*" || job.tgt == minion_id {
                items.push(MinionActivityItem::CacheJob(job.clone()));
            }
        }

        // Alerts mentioning this minion
        for alert in &self.alert_engine.alerts {
            if alert.description.contains(minion_id) || alert.details.iter().any(|d| d.contains(minion_id)) {
                items.push(MinionActivityItem::Alert(alert.clone()));
            }
        }

        // Sort by timestamp descending (alerts with ts=0.0 fall to end)
        items.sort_by(|a, b| b.ts().partial_cmp(&a.ts()).unwrap_or(std::cmp::Ordering::Equal));
        items
    }

    /// Rolling percentage of job returns with success == true, over the last
    /// JOB_STATS_WINDOW returns. None until at least one return has been seen.
    pub fn job_success_pct(&self) -> Option<f64> {
        if self.job_returns_window.is_empty() { return None; }
        let ok = self.job_returns_window.iter().filter(|s| **s).count();
        Some(100.0 * ok as f64 / self.job_returns_window.len() as f64)
    }

    /// Count of active jobs dispatched but not fully returned past job_straggler_secs.
    pub fn job_stuck_count(&self) -> usize {
        self.jobs.iter().filter(|j| {
            j.target_count > 0 && j.returned < j.target_count
                && j.age_seconds > self.config.job_straggler_secs
        }).count()
    }

    /// p50/p95 job completion latency in seconds, over the last JOB_STATS_WINDOW
    /// fully-completed jobs. None until at least one job has fully completed.
    pub fn job_completion_latency_p50_p95(&self) -> Option<(f64, f64)> {
        if self.job_completion_latencies.is_empty() { return None; }
        let mut v: Vec<f64> = self.job_completion_latencies.iter().copied().collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pick = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
        Some((pick(0.50), pick(0.95)))
    }

    /// Splits a cpstats "Request-Line" ("METHOD /path?query HTTP/1.1") into
    /// (method, path) for grouping — drops the query string and HTTP version.
    fn parse_request_line(line: &str) -> (String, String) {
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or("?").to_string();
        let path = parts.next().unwrap_or("?").split('?').next().unwrap_or("?").to_string();
        (method, path)
    }

    /// Rolling percentage of salt-api requests with a non-2xx/3xx status,
    /// over the last config.api_stats_window observed requests. None until
    /// at least one has been observed.
    pub fn api_error_pct(&self) -> Option<f64> {
        if self.api_status_window.is_empty() { return None; }
        let errors = self.api_status_window.iter().filter(|ok| !**ok).count();
        Some(100.0 * errors as f64 / self.api_status_window.len() as f64)
    }

    /// p50/p95 POST /login processing time in seconds, over the last
    /// config.api_stats_window logins observed. None until at least one has been seen.
    pub fn api_login_latency_p50_p95(&self) -> Option<(f64, f64)> {
        if self.api_login_latencies.is_empty() { return None; }
        let mut v: Vec<f64> = self.api_login_latencies.iter().copied().collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pick = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
        Some((pick(0.50), pick(0.95)))
    }

    /// Rolling percentage of closed-out jobs where returned < target_count,
    /// over the last JOB_STATS_WINDOW closed jobs. None until one has closed.
    pub fn job_partial_pct(&self) -> Option<f64> {
        if self.job_outcome_window.is_empty() { return None; }
        let partial = self.job_outcome_window.iter().filter(|p| **p).count();
        Some(100.0 * partial as f64 / self.job_outcome_window.len() as f64)
    }

    /// Worker pool saturation: sum of Worker-role process CPU%, normalized
    /// against the configured worker_threads count. None until both the
    /// worker_threads config and at least one CPU sample are available.
    pub fn worker_busy_pct(&self) -> Option<f64> {
        let worker_threads = self.config_snapshot.as_ref()?.worker_threads?;
        if worker_threads <= 0 { return None; }
        let total_cpu: f64 = self.processes.iter()
            .filter(|p| p.role == SaltProcessRole::Worker)
            .map(|p| p.cpu_percent as f64)
            .sum();
        Some((total_cpu / (worker_threads as f64 * 100.0) * 100.0).clamp(0.0, 100.0))
    }

    /// Count of pending keys whose PKI file mtime is within the last minute.
    pub fn pending_key_churn_per_min(&self) -> usize {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.keys.iter().filter(|k| {
            k.status == KeyStatus::Pending && k.mtime_epoch > 0
                && now.saturating_sub(k.mtime_epoch) < 60
        }).count()
    }

    /// p50/p90/p99 check-in age (now - last_seen_epoch) across the fleet, in seconds.
    pub fn checkin_age_percentiles(&self) -> Option<(u64, u64, u64)> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut ages: Vec<u64> = self.minion_info.iter()
            .filter(|m| m.last_seen_epoch > 0)
            .map(|m| now.saturating_sub(m.last_seen_epoch))
            .collect();
        if ages.is_empty() { return None; }
        ages.sort_unstable();
        let pick = |p: f64| ages[((ages.len() - 1) as f64 * p).round() as usize];
        Some((pick(0.50), pick(0.90), pick(0.99)))
    }

    /// Whether a minion checked in recently enough to be considered
    /// "freshly active" right now. The threshold is the fleet's own median
    /// check-in age (checkin_age_percentiles' p50) rather than a fixed
    /// number, so it stays meaningful whether minions ping every few
    /// seconds or every few minutes — floored at 5s so a tightly-clustered
    /// fleet (everyone returning within the same tick) doesn't collapse the
    /// threshold to 0, and capped at 60s so a fleet that goes fully idle
    /// doesn't drag p50 up with it and leave the marker showing "fresh"
    /// forever (observed live: a one-off fleet-wide ping with no follow-up
    /// traffic made p50 climb in lockstep with every minion's own age,
    /// since there was nothing fresher left to pull the median down).
    pub fn is_minion_fresh(&self, last_seen_epoch: u64) -> bool {
        if last_seen_epoch == 0 {
            return false;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let threshold = self.checkin_age_percentiles().map(|(p50, _, _)| p50.clamp(5, 60)).unwrap_or(30);
        now.saturating_sub(last_seen_epoch) <= threshold
    }

    /// Restart/flap count for a unit over the last 24h.
    pub fn service_restart_count_24h(&self, unit: &str) -> usize {
        self.service_trends.get(unit).map_or(0, |t| t.restart_events.len())
    }

    /// Memory growth rate in MB/hour (linear-regression slope over the sample
    /// window) and whether the most recent third of the window is still
    /// trending upward (vs. having grown then plateaued). None until enough
    /// samples have accumulated for the unit.
    pub fn service_memory_growth(&self, unit: &str) -> Option<(f64, bool)> {
        let trend = self.service_trends.get(unit)?;
        if trend.mem_samples.len() < 6 { return None; }
        let t0 = trend.mem_samples.front()?.0;
        let pts: Vec<(f64, f64)> = trend.mem_samples.iter()
            .map(|(t, kb)| (t.duration_since(t0).as_secs_f64(), *kb as f64 / 1024.0))
            .collect();
        let full_slope_mb_per_sec = linreg_slope_per_sec(&pts);
        let recent_start = pts.len() - pts.len() / 3;
        let recent_slope_mb_per_sec = linreg_slope_per_sec(&pts[recent_start..]);
        Some((full_slope_mb_per_sec * 3600.0, recent_slope_mb_per_sec > 0.0))
    }

    /// Per-unit restart/flap + memory-growth snapshot, for the alert engine.
    pub fn service_trend_snapshots(&self) -> Vec<ServiceTrendSnapshot> {
        self.services.iter().map(|svc| {
            let (mem_growth_mb_per_hour, mem_still_growing) = match self.service_memory_growth(&svc.unit) {
                Some((rate, growing)) => (Some(rate), growing),
                None => (None, false),
            };
            ServiceTrendSnapshot {
                unit: svc.unit.clone(),
                restart_count_24h: self.service_restart_count_24h(&svc.unit),
                mem_growth_mb_per_hour,
                mem_still_growing,
            }
        }).collect()
    }

    pub fn filtered_jobs_len(&self) -> usize {
        let pat = self.filter_text_for(Tab::Jobs);
        if pat.is_empty() {
            return self.jobs.len();
        }
        let known = self.accepted_ids();
        self.jobs.iter().filter(|j| Self::matches_job(j, pat, &known)).count()
    }

    pub fn get_filtered_job(&self, index: usize) -> Option<&JobEntry> {
        let pat = self.filter_text_for(Tab::Jobs);
        if pat.is_empty() {
            return self.jobs.get(index);
        }
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
        let pat = self.filter_text_for(Tab::Alerts);
        if pat.is_empty() {
            return self.alert_engine.alerts.len();
        }
        self.alert_engine.alerts.iter().filter(|a| Self::matches_alert(a, pat)).count()
    }

    pub fn get_filtered_alert(&self, index: usize) -> Option<&AlertEntry> {
        let pat = self.filter_text_for(Tab::Alerts);
        if pat.is_empty() {
            return self.alert_engine.alerts.get(index);
        }
        self.alert_engine.alerts.iter().filter(|a| Self::matches_alert(a, pat)).nth(index)
    }

    fn matches_api_log_entry(entry: &ApiLogEntry, pat: &str) -> bool {
        let terms = Self::filter_terms(pat);
        if terms.is_empty() { return true; }
        terms.iter().any(|term| {
            Self::matches_filter(&entry.method, term)
                || Self::matches_filter(&entry.path, term)
                || Self::matches_filter(&entry.response_status, term)
                || Self::host_matches_term(&entry.client, term)
        })
    }

    /// Whether an entry should be visible given the current
    /// hide_stats_polls setting -- checked at display time (not ingestion)
    /// so toggling `x` live shows/hides already-buffered entries instead of
    /// only affecting requests captured from that point on.
    fn api_log_entry_visible(&self, entry: &ApiLogEntry) -> bool {
        !(self.config.api_hide_stats_polls && entry.method == "GET" && entry.path == "/stats")
    }

    /// Index 0 is the oldest buffered request — same oldest-at-top,
    /// newest-at-bottom convention as Events (self.events).
    pub fn filtered_api_log_len(&self) -> usize {
        let pat = self.filter_text_for(Tab::Api);
        self.api_requests_log.iter()
            .filter(|e| self.api_log_entry_visible(e) && Self::matches_api_log_entry(e, pat))
            .count()
    }

    pub fn get_filtered_api_log(&self, index: usize) -> Option<&ApiLogEntry> {
        let pat = self.filter_text_for(Tab::Api);
        self.api_requests_log.iter()
            .filter(|e| self.api_log_entry_visible(e) && Self::matches_api_log_entry(e, pat))
            .nth(index)
    }

    /// Per-endpoint (method+path) breakdown of the captured request log:
    /// count, avg/p95 latency, error rate — sorted by count descending.
    pub fn api_endpoint_breakdown(&self) -> Vec<ApiEndpointStats> {
        let mut groups: HashMap<(String, String), Vec<(f64, bool)>> = HashMap::new();
        for entry in &self.api_requests_log {
            let is_ok = entry.response_status.split_whitespace().next()
                .map(|c| c.starts_with('2') || c.starts_with('3'))
                .unwrap_or(true);
            groups.entry((entry.method.clone(), entry.path.clone()))
                .or_default()
                .push((entry.processing_time_secs, is_ok));
        }
        let mut stats: Vec<ApiEndpointStats> = groups.into_iter().map(|((method, path), mut samples)| {
            let count = samples.len();
            let errors = samples.iter().filter(|(_, ok)| !ok).count();
            samples.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let avg = samples.iter().map(|(t, _)| t).sum::<f64>() / count as f64;
            let p95_idx = ((count - 1) as f64 * 0.95).round() as usize;
            let p95 = samples[p95_idx].0;
            ApiEndpointStats {
                method, path, count,
                avg_secs: avg,
                p95_secs: p95,
                error_pct: 100.0 * errors as f64 / count as f64,
            }
        }).collect();
        stats.sort_by(|a, b| b.count.cmp(&a.count));
        stats
    }

    pub fn pulse_minions_count(&self) -> usize {
        let accepted: HashSet<&str> = self.keys.iter()
            .filter(|k| k.status == KeyStatus::Accepted)
            .map(|k| k.id.as_str())
            .collect();
        self.minion_info.iter()
            .filter(|m| accepted.contains(m.id.as_str()) || m.key_status == Some(KeyStatus::Accepted))
            .count()
    }

    pub fn filtered_minions_len(&self) -> usize {
        let accepted: HashSet<&str> = self.keys.iter()
            .filter(|k| k.status == KeyStatus::Accepted)
            .map(|k| k.id.as_str())
            .collect();
        let hide = self.hide_timed_out;
        let pat = self.filter_text_for(Tab::Minions);
        let resp_count = self.minion_info.iter()
            .filter(|m| accepted.contains(m.id.as_str()) || m.key_status == Some(KeyStatus::Accepted))
            .filter(|m| pat.is_empty() || Self::matches_minion(m, pat))
            .filter(|m| !m.timed_out)
            .count();
        let to_count = self.minion_info.iter()
            .filter(|m| accepted.contains(m.id.as_str()) || m.key_status == Some(KeyStatus::Accepted))
            .filter(|m| pat.is_empty() || Self::matches_minion(m, pat))
            .filter(|m| m.timed_out)
            .count();
        if hide || to_count == 0 {
            resp_count
        } else {
            resp_count + 1 + if self.timed_out_collapsed { 0 } else { to_count }
        }
    }

    pub fn get_filtered_minion(&self, index: usize) -> Option<&MinionInfo> {
        let accepted: HashSet<&str> = self.keys.iter()
            .filter(|k| k.status == KeyStatus::Accepted)
            .map(|k| k.id.as_str())
            .collect();
        let hide = self.hide_timed_out;
        let pat = self.filter_text_for(Tab::Minions);

        let resp_count = self.minion_info.iter()
            .filter(|m| accepted.contains(m.id.as_str()) || m.key_status == Some(KeyStatus::Accepted))
            .filter(|m| pat.is_empty() || Self::matches_minion(m, pat))
            .filter(|m| !m.timed_out)
            .count();

        if index < resp_count {
            return self.minion_info.iter()
                .filter(|m| accepted.contains(m.id.as_str()) || m.key_status == Some(KeyStatus::Accepted))
                .filter(|m| pat.is_empty() || Self::matches_minion(m, pat))
                .filter(|m| !m.timed_out)
                .nth(index);
        }

        if hide { return None; }

        // index == resp_count is the collapsible section header (not a minion)
        let to_idx = index.wrapping_sub(resp_count).wrapping_sub(1);
        self.minion_info.iter()
            .filter(|m| accepted.contains(m.id.as_str()) || m.key_status == Some(KeyStatus::Accepted))
            .filter(|m| pat.is_empty() || Self::matches_minion(m, pat))
            .filter(|m| m.timed_out)
            .nth(to_idx)
    }

    fn open_config_editor(&mut self) {
        // Clone the current config into an editable state
        let cfg = &self.config;
        self.detail_popup = Some(DetailPopup::ConfigEditor(ConfigEditorState {
            fields: vec![
                ConfigField { label: "[salt]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "agent_path", label_value: cfg.agent_path.clone(), kind: ConfigFieldKind::Str },
                ConfigField { label: "python_bin", label_value: cfg.python_bin.clone(), kind: ConfigFieldKind::Str },
                ConfigField { label: "master_config", label_value: cfg.master_config.clone(), kind: ConfigFieldKind::Str },
                ConfigField { label: "[refresh]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "dashboard_secs", label_value: cfg.dashboard_secs.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "minions_secs", label_value: cfg.minions_secs.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "config_secs", label_value: cfg.config_secs.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "[events]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "buffer_size", label_value: cfg.event_buffer_size.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "sniffer_buffer_size", label_value: cfg.sniffer_buffer_size.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "[alerts]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "stale_minion_secs", label_value: cfg.stale_minion_secs.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "pending_key_secs", label_value: cfg.pending_key_secs.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "job_straggler_secs", label_value: cfg.job_straggler_secs.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "disk_usage_pct", label_value: cfg.disk_usage_pct.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "auth_storm_rate_per_sec", label_value: cfg.auth_storm_rate_per_sec.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "worker_saturation_pct", label_value: cfg.worker_saturation_pct.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "pending_key_churn_per_min", label_value: cfg.pending_key_churn_per_min.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "service_flap_threshold_24h", label_value: cfg.service_flap_threshold_24h.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "service_mem_growth_mb_per_hour", label_value: cfg.service_mem_growth_mb_per_hour.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_abs_threshold_mb", label_value: cfg.service_mem_abs_threshold_mb.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_pct_threshold", label_value: cfg.service_mem_pct_threshold.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "api_auth_latency_warn_secs", label_value: cfg.api_auth_latency_warn_secs.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_abs_threshold_master_mb", label_value: cfg.service_mem_abs_threshold_master_mb.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_pct_threshold_master", label_value: cfg.service_mem_pct_threshold_master.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_abs_threshold_minion_mb", label_value: cfg.service_mem_abs_threshold_minion_mb.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_pct_threshold_minion", label_value: cfg.service_mem_pct_threshold_minion.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_abs_threshold_api_mb", label_value: cfg.service_mem_abs_threshold_api_mb.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_pct_threshold_api", label_value: cfg.service_mem_pct_threshold_api.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_abs_threshold_syndic_mb", label_value: cfg.service_mem_abs_threshold_syndic_mb.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_pct_threshold_syndic", label_value: cfg.service_mem_pct_threshold_syndic.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "[api]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "stats_window", label_value: cfg.api_stats_window.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "log_cap", label_value: cfg.api_log_cap.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "log_max_age_secs", label_value: cfg.api_log_max_age_secs.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "hide_stats_polls", label_value: on_off(cfg.api_hide_stats_polls), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "stats_auth_mode", label_value: cfg.api_stats_auth_mode.clone(), kind: ConfigFieldKind::Choice(API_STATS_AUTH_MODES) },
                ConfigField { label: "stats_auth_eauth", label_value: cfg.api_stats_auth_eauth.clone(), kind: ConfigFieldKind::Str },
                ConfigField { label: "stats_auth_username", label_value: cfg.api_stats_auth_username.clone(), kind: ConfigFieldKind::Str },
                ConfigField { label: "stats_auth_password", label_value: cfg.api_stats_auth_password.clone(), kind: ConfigFieldKind::Str },
                ConfigField { label: "stats_auth_username_pillar_path", label_value: cfg.api_stats_auth_username_pillar_path.clone(), kind: ConfigFieldKind::Str },
                ConfigField { label: "stats_auth_password_pillar_path", label_value: cfg.api_stats_auth_password_pillar_path.clone(), kind: ConfigFieldKind::Str },
                ConfigField { label: "[dashboard]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "zmq_latency_source", label_value: cfg.zmq_latency_source.clone(), kind: ConfigFieldKind::Choice(ZMQ_LATENCY_SOURCES) },
                ConfigField { label: "show_zmq_graph", label_value: on_off(cfg.show_zmq_graph), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_events_graph", label_value: on_off(cfg.show_events_graph), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_activity_graph", label_value: on_off(cfg.show_activity_graph), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_cpu_graph", label_value: on_off(cfg.show_cpu_graph), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_mem_graph", label_value: on_off(cfg.show_mem_graph), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_host_resources", label_value: on_off(cfg.show_host_resources), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_services", label_value: on_off(cfg.show_services), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_key_status", label_value: on_off(cfg.show_key_status), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_event_types", label_value: on_off(cfg.show_event_types), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_job_pulse", label_value: on_off(cfg.show_job_pulse), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_salt_api", label_value: on_off(cfg.show_salt_api), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "graph_order", label_value: cfg.graph_order.clone(), kind: ConfigFieldKind::Order },
                ConfigField { label: "[minions]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "follow_filter", label_value: on_off(cfg.minions_follow_filter), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "[export]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "save_dir", label_value: cfg.save_dir.clone(), kind: ConfigFieldKind::Str },
                ConfigField { label: "[Save]", label_value: String::new(), kind: ConfigFieldKind::SaveButton },
            ],
            selected: 1, // first editable field (skip section header)
            editing: false,
            input: String::new(),
            choice_idx: 0,
            save_error: None,
        }));
        self.popup_tab = None; // global — works from any tab
    }

    fn save_config_from_editor(&mut self) -> Result<(), String> {
        let fields = match self.detail_popup {
            Some(DetailPopup::ConfigEditor(ref state)) => &state.fields,
            _ => return Err("No editor state".into()),
        };
        // Extract values by label
        let mut vals: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for f in fields {
            match f.kind {
                ConfigFieldKind::Str | ConfigFieldKind::U64 | ConfigFieldKind::F64 | ConfigFieldKind::Choice(_) | ConfigFieldKind::Order => {
                    vals.insert(f.label, f.label_value.as_str());
                }
                _ => {}
            }
        }
        // Parse and build new config
        let parse_u64 = |key| vals.get(key).and_then(|v| v.parse::<u64>().ok()).ok_or_else(|| format!("Invalid {}: expected integer", key));
        let parse_usize = |key| vals.get(key).and_then(|v| v.parse::<usize>().ok()).ok_or_else(|| format!("Invalid {}: expected integer", key));
        let parse_f64 = |key| vals.get(key).and_then(|v| v.parse::<f64>().ok()).ok_or_else(|| format!("Invalid {}: expected number", key));
        let parse_bool = |key| vals.get(key) == Some(&"on");

        let new_config = AppConfig {
            agent_path: vals.get("agent_path").unwrap_or(&"").to_string(),
            python_bin: vals.get("python_bin").unwrap_or(&"").to_string(),
            master_config: vals.get("master_config").unwrap_or(&"").to_string(),
            dashboard_secs: parse_u64("dashboard_secs")?,
            minions_secs: parse_u64("minions_secs")?,
            config_secs: parse_u64("config_secs")?,
            event_buffer_size: parse_usize("buffer_size")?,
            sniffer_buffer_size: parse_usize("sniffer_buffer_size")?,
            stale_minion_secs: parse_u64("stale_minion_secs")?,
            pending_key_secs: parse_u64("pending_key_secs")?,
            job_straggler_secs: parse_u64("job_straggler_secs")?,
            disk_usage_pct: parse_f64("disk_usage_pct")?,
            auth_storm_rate_per_sec: parse_f64("auth_storm_rate_per_sec")?,
            worker_saturation_pct: parse_f64("worker_saturation_pct")?,
            pending_key_churn_per_min: parse_u64("pending_key_churn_per_min")?,
            service_flap_threshold_24h: parse_u64("service_flap_threshold_24h")?,
            service_mem_growth_mb_per_hour: parse_f64("service_mem_growth_mb_per_hour")?,
            service_mem_abs_threshold_mb: parse_f64("service_mem_abs_threshold_mb")?,
            service_mem_pct_threshold: parse_f64("service_mem_pct_threshold")?,
            api_auth_latency_warn_secs: parse_f64("api_auth_latency_warn_secs")?,
            service_mem_abs_threshold_master_mb: parse_f64("service_mem_abs_threshold_master_mb")?,
            service_mem_pct_threshold_master: parse_f64("service_mem_pct_threshold_master")?,
            service_mem_abs_threshold_minion_mb: parse_f64("service_mem_abs_threshold_minion_mb")?,
            service_mem_pct_threshold_minion: parse_f64("service_mem_pct_threshold_minion")?,
            service_mem_abs_threshold_api_mb: parse_f64("service_mem_abs_threshold_api_mb")?,
            service_mem_pct_threshold_api: parse_f64("service_mem_pct_threshold_api")?,
            service_mem_abs_threshold_syndic_mb: parse_f64("service_mem_abs_threshold_syndic_mb")?,
            service_mem_pct_threshold_syndic: parse_f64("service_mem_pct_threshold_syndic")?,
            api_stats_window: parse_usize("stats_window")?,
            api_log_cap: parse_usize("log_cap")?,
            api_log_max_age_secs: parse_u64("log_max_age_secs")?,
            api_hide_stats_polls: parse_bool("hide_stats_polls"),
            api_stats_auth_mode: vals.get("stats_auth_mode").unwrap_or(&"none").to_string(),
            api_stats_auth_eauth: vals.get("stats_auth_eauth").unwrap_or(&"pam").to_string(),
            api_stats_auth_username: vals.get("stats_auth_username").unwrap_or(&"").to_string(),
            api_stats_auth_password: vals.get("stats_auth_password").unwrap_or(&"").to_string(),
            api_stats_auth_username_pillar_path: vals.get("stats_auth_username_pillar_path").unwrap_or(&"").to_string(),
            api_stats_auth_password_pillar_path: vals.get("stats_auth_password_pillar_path").unwrap_or(&"").to_string(),
            zmq_latency_source: vals.get("zmq_latency_source").unwrap_or(&"agent_ipc").to_string(),
            show_zmq_graph: parse_bool("show_zmq_graph"),
            show_events_graph: parse_bool("show_events_graph"),
            show_activity_graph: parse_bool("show_activity_graph"),
            show_cpu_graph: parse_bool("show_cpu_graph"),
            show_mem_graph: parse_bool("show_mem_graph"),
            graph_order: vals.get("graph_order").unwrap_or(&"cpu,mem,zmq,events,activity").to_string(),
            show_host_resources: parse_bool("show_host_resources"),
            show_services: parse_bool("show_services"),
            show_key_status: parse_bool("show_key_status"),
            show_event_types: parse_bool("show_event_types"),
            show_job_pulse: parse_bool("show_job_pulse"),
            show_salt_api: parse_bool("show_salt_api"),
            minions_follow_filter: parse_bool("follow_filter"),
            save_dir: {
                let v = vals.get("save_dir").unwrap_or(&"~").trim();
                if v.is_empty() { "~".to_string() } else { v.to_string() }
            },
        };

        crate::config::write_config(&self.config_path, &new_config)
            .map_err(|e| format!("Write failed: {}", e))?;
        self.config = new_config;
        // A changed zmq_latency_source (or any save) shouldn't leave a stale
        // figure — or sparkline history in different units — from a
        // previously active source mislabeled under a new one.
        self.zmq_latency_ms = 0.0;
        self.zmq_latency_seen = false;
        self.zmq_latency_history = [0.0; 2000];
        self.zmq_latency_idx = 0;
        self.zmq_ping_sent_at = None;
        self.job_ping_pending = None;
        Ok(())
    }

    fn handle_config_editor_key(&mut self, code: KeyCode) -> bool {
        // --- Editing mode (borrow state) ---
        if let Some(DetailPopup::ConfigEditor(ref mut s)) = self.detail_popup {
            if s.editing {
                // Choice fields are edited via an inline picker (arrow keys +
                // Enter over the option list) instead of free-text entry.
                if let Some(ConfigFieldKind::Choice(options)) = s.fields.get(s.selected).map(|f| f.kind) {
                    match code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            s.choice_idx = s.choice_idx.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if s.choice_idx + 1 < options.len() { s.choice_idx += 1; }
                        }
                        KeyCode::Enter => {
                            if let Some(opt) = options.get(s.choice_idx) {
                                if let Some(field) = s.fields.get_mut(s.selected) {
                                    field.label_value = opt.to_string();
                                }
                            }
                            s.editing = false;
                        }
                        KeyCode::Esc => {
                            s.editing = false;
                        }
                        _ => {}
                    }
                    return false;
                }
                // Order fields (currently just graph_order): arrows move a
                // cursor between items, +/- move the item under the cursor
                // up/down in the order — no free-text entry, so a typo or
                // dropped token isn't possible.
                if matches!(s.fields.get(s.selected).map(|f| f.kind), Some(ConfigFieldKind::Order)) {
                    match code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            s.choice_idx = s.choice_idx.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let len = s.fields.get(s.selected).map_or(0, |f| f.label_value.split(',').count());
                            if s.choice_idx + 1 < len { s.choice_idx += 1; }
                        }
                        KeyCode::Char('+') | KeyCode::Char('=') => {
                            if let Some(field) = s.fields.get_mut(s.selected) {
                                let mut items: Vec<&str> = field.label_value.split(',').collect();
                                if s.choice_idx > 0 && s.choice_idx < items.len() {
                                    items.swap(s.choice_idx, s.choice_idx - 1);
                                    field.label_value = items.join(",");
                                    s.choice_idx -= 1;
                                }
                            }
                        }
                        KeyCode::Char('-') | KeyCode::Char('_') => {
                            if let Some(field) = s.fields.get_mut(s.selected) {
                                let mut items: Vec<&str> = field.label_value.split(',').collect();
                                if s.choice_idx + 1 < items.len() {
                                    items.swap(s.choice_idx, s.choice_idx + 1);
                                    field.label_value = items.join(",");
                                    s.choice_idx += 1;
                                }
                            }
                        }
                        KeyCode::Enter => {
                            s.editing = false;
                            s.input.clear();
                        }
                        KeyCode::Esc => {
                            if let Some(field) = s.fields.get_mut(s.selected) {
                                field.label_value = s.input.clone();
                            }
                            s.editing = false;
                            s.input.clear();
                        }
                        _ => {}
                    }
                    return false;
                }
                match code {
                    KeyCode::Enter => {
                        if let Some(field) = s.fields.get_mut(s.selected) {
                            field.label_value = s.input.clone();
                        }
                        s.editing = false;
                        s.input.clear();
                    }
                    KeyCode::Esc => {
                        s.editing = false;
                        s.input.clear();
                    }
                    KeyCode::Backspace => { s.input.pop(); }
                    KeyCode::Char(c) => { s.input.push(c); }
                    _ => {}
                }
                return false;
            }
        } else {
            return false;
        }

        // --- Navigation / action mode (borrow state per arm, not function-level) ---
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(DetailPopup::ConfigEditor(ref mut s)) = self.detail_popup {
                    let mut new_sel = s.selected;
                    loop {
                        if new_sel == 0 { break; }
                        new_sel = new_sel.saturating_sub(1);
                        if let Some(f) = s.fields.get(new_sel) {
                            if !matches!(f.kind, ConfigFieldKind::Section) {
                                s.selected = new_sel;
                                break;
                            }
                        }
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(DetailPopup::ConfigEditor(ref mut s)) = self.detail_popup {
                    let last = s.fields.len().saturating_sub(1);
                    let mut new_sel = s.selected;
                    loop {
                        if new_sel >= last { break; }
                        new_sel = new_sel.saturating_add(1);
                        if let Some(f) = s.fields.get(new_sel) {
                            if !matches!(f.kind, ConfigFieldKind::Section) {
                                s.selected = new_sel;
                                break;
                            }
                        }
                    }
                }
            }
            KeyCode::PageUp => {
                for _ in 0..5 {
                    if let Some(DetailPopup::ConfigEditor(ref mut s)) = self.detail_popup {
                        let mut new_sel = s.selected;
                        loop {
                            if new_sel == 0 { break; }
                            new_sel = new_sel.saturating_sub(1);
                            if let Some(f) = s.fields.get(new_sel) {
                                if !matches!(f.kind, ConfigFieldKind::Section) {
                                    s.selected = new_sel;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            KeyCode::PageDown => {
                for _ in 0..5 {
                    if let Some(DetailPopup::ConfigEditor(ref mut s)) = self.detail_popup {
                        let last = s.fields.len().saturating_sub(1);
                        let mut new_sel = s.selected;
                        loop {
                            if new_sel >= last { break; }
                            new_sel = new_sel.saturating_add(1);
                            if let Some(f) = s.fields.get(new_sel) {
                                if !matches!(f.kind, ConfigFieldKind::Section) {
                                    s.selected = new_sel;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            KeyCode::Home => {
                if let Some(DetailPopup::ConfigEditor(ref mut s)) = self.detail_popup {
                    s.selected = 1;
                }
            }
            KeyCode::End => {
                if let Some(DetailPopup::ConfigEditor(ref mut s)) = self.detail_popup {
                    s.selected = s.fields.len().saturating_sub(1);
                }
            }
            KeyCode::Enter => {
                // Determine action from field at current selection (immutable borrow)
                let action = match self.detail_popup {
                    Some(DetailPopup::ConfigEditor(ref s)) => {
                        match s.fields.get(s.selected).map(|f| &f.kind) {
                            Some(ConfigFieldKind::SaveButton) => Some("save"),
                            Some(ConfigFieldKind::Str) | Some(ConfigFieldKind::U64) | Some(ConfigFieldKind::F64) | Some(ConfigFieldKind::Choice(_)) | Some(ConfigFieldKind::Order) => {
                                Some("edit")
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                match action {
                    Some("save") => {
                        match self.save_config_from_editor() {
                            Ok(()) => {
                                self.detail_popup = None;
                                self.clipboard_msg = Some((" Config saved ".into(), std::time::Instant::now()));
                            }
                            Err(e) => {
                                if let Some(DetailPopup::ConfigEditor(ref mut s)) = self.detail_popup {
                                    s.save_error = Some(e);
                                }
                            }
                        }
                    }
                    Some("edit") => {
                        let field_info = match self.detail_popup {
                            Some(DetailPopup::ConfigEditor(ref s)) => {
                                s.fields.get(s.selected).map(|f| (f.label_value.clone(), f.kind))
                            }
                            _ => None,
                        };
                        if let Some(DetailPopup::ConfigEditor(ref mut s)) = self.detail_popup {
                            if let Some((val, kind)) = field_info {
                                if let ConfigFieldKind::Choice(options) = kind {
                                    s.choice_idx = options.iter().position(|o| *o == val).unwrap_or(0);
                                } else if let ConfigFieldKind::Order = kind {
                                    // s.input snapshots the pre-edit order so Esc can restore it —
                                    // +/- mutate field.label_value live so the list re-renders in
                                    // its new order immediately, not just on confirm.
                                    s.input = val;
                                    s.choice_idx = 0;
                                } else {
                                    s.input = val;
                                }
                                s.editing = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.detail_popup = None;
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.high_contrast = !self.high_contrast;
            }
            _ => {}
        }
        false
    }

    fn open_detail(&mut self) {
        match self.current_tab {
            Tab::Events => {
                self.return_to_minion_events = None;
                if let Some(evt) = self.get_filtered_event(self.event_selected) {
                    self.detail_popup = Some(DetailPopup::SaltEvent { evt: evt.clone(), scroll: 0, pretty: false, pretty_fetch: crate::types::PrettyFetch::Idle });
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
            Tab::Api => {
                if let Some(entry) = self.get_filtered_api_log(self.api_log_selected) {
                    self.detail_popup = Some(DetailPopup::ApiRequest(entry.clone()));
                    self.popup_tab = Some(self.current_tab);
                }
            }
            Tab::Minions => {
                if let Some(minion) = self.get_filtered_minion(self.minion_selected) {
                    let mid = minion.id.clone();
                    let state = if self.admin_mode {
                        self.send_agent_cmd("minion_ping", Some(serde_json::json!({"minion_id": &mid})));
                        PingState::Pinging(Instant::now())
                    } else {
                        PingState::Idle
                    };
                    self.detail_popup = Some(DetailPopup::Minion(mid, state, None, 0));
                    self.popup_tab = Some(self.current_tab);
                } else if !self.hide_timed_out {
                    // Header row selected — toggle collapse
                    self.timed_out_collapsed = !self.timed_out_collapsed;
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

    /// Like `ansi_to_styled_spans`, but for multi-line text — splits the
    /// result into one `Line` per `\n`, carrying the active style *across*
    /// line boundaries instead of resetting to `default_style` at each one.
    /// Real colored CLI output routinely sets a color once for a whole
    /// multi-line block (e.g. a state's `Changes`/stdout text) rather than
    /// repeating the escape code on every line, relying on the terminal to
    /// keep that color active until the next code — calling
    /// `ansi_to_styled_spans` independently per line (as this used to do,
    /// via `text.lines()` at the call site) lost that carried-over color on
    /// every line after the first one that actually contained an escape
    /// code, falling back to `default_style` instead.
    pub fn ansi_text_to_lines(text: &str, default_style: Style) -> Vec<Line<'static>> {
        let spans = Self::ansi_to_styled_spans(text, default_style);
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut current: Vec<Span<'static>> = Vec::new();
        for span in spans {
            let style = span.style;
            let content = span.content.into_owned();
            let mut parts = content.split('\n');
            if let Some(first) = parts.next() {
                if !first.is_empty() {
                    current.push(Span::styled(first.to_string(), style));
                }
            }
            for part in parts {
                lines.push(Line::from(std::mem::take(&mut current)));
                if !part.is_empty() {
                    current.push(Span::styled(part.to_string(), style));
                }
            }
        }
        lines.push(Line::from(current));
        lines
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

    /// Dump comprehensive X11 diagnostics to a file so the user can share
    /// what's happening on their system.
    fn debug_x11(display: &Option<String>, xauth: &Option<String>, x11_user: &str, debug_path: &str) {
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

    fn uid_to_username(uid: u32) -> String {
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
    fn run_xclip_detached(
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
                let with_xa = xauth.as_ref().map(|s| s.as_str());
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
        let raw = if raw.is_empty() { "~" } else { raw };
        let home = || std::env::var_os("HOME").map(std::path::PathBuf::from);
        if raw == "~" {
            return home().unwrap_or_else(|| std::path::PathBuf::from("/root"));
        }
        if let Some(rest) = raw.strip_prefix("~/") {
            if let Some(h) = home() {
                return h.join(rest);
            }
        }
        std::path::PathBuf::from(raw)
    }

    /// Opens the "save to file" prompt (the 's'/'S' counterpart of 'y'/'Y'
    /// clipboard copy) with `content` and a filename proposed from
    /// `name_hint`, which the user can edit before confirming.
    pub fn open_save_prompt(&mut self, content: String, name_hint: &str) {
        self.save_prompt = Some(SaveFilePrompt {
            filename: Self::propose_save_filename(name_hint),
            content,
            confirm_overwrite: false,
        });
    }

    /// Turns a free-form hint (e.g. a job id, minion id, event tag) into a
    /// filesystem-safe proposed filename with a timestamp, so repeated
    /// saves of similar content don't silently collide.
    fn propose_save_filename(hint: &str) -> String {
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let clean: String = hint
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' })
            .collect();
        let clean = clean.trim_matches('-');
        let clean = if clean.is_empty() { "salt-radar" } else { clean };
        format!("{}-{}.txt", clean, ts)
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
            KeyCode::Backspace => {
                if let Some(ref mut prompt) = self.save_prompt {
                    prompt.filename.pop();
                    prompt.confirm_overwrite = false;
                }
            }
            KeyCode::Char(c) if !c.is_control() => {
                if let Some(ref mut prompt) = self.save_prompt {
                    prompt.filename.push(c);
                    prompt.confirm_overwrite = false;
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

    fn current_user() -> String {
        std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_else(|_| "root".to_string())
    }

    /// Tries to find the unprivileged user that owns the X11 session by:
    /// 1. Ownership of /tmp/.X11-unix/X<display_number>
    /// 2. Ownership of the tmux socket (/tmp/tmux-<uid>/default)
    /// 3. current_user() as last resort
    fn detect_x11_user(display: &str) -> String {
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

    /// Clean up per-instance temp files. Called on exit.
    pub fn cleanup(&self) {
        let _ = std::fs::remove_file(&self.cmd_path);
        let _ = std::fs::remove_file(&self.copy_path);
        let _ = std::fs::remove_file(&self.clip_log_path);
        let _ = std::fs::remove_file(&self.clip_debug_path);

        // Persist the current dashboard graph layout (F4-F8 toggles) so it
        // survives a restart instead of reverting to whatever was last
        // saved through the Config Editor.
        let _ = crate::config::write_config(&self.config_path, &self.config);
    }

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
                if existing.timed_out {
                    existing.timed_out = false;
                    self.minion_info_dirty = true;
                }
                if existing.key_status.is_none() {
                    existing.key_status = Some(KeyStatus::Accepted);
                }
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
                            if !existing.timed_out {
                                existing.timed_out = true;
                                self.minion_info_dirty = true;
                            }
                            if existing.key_status.is_none() {
                                existing.key_status = Some(KeyStatus::Accepted);
                            }
                        } else {
                            self.minion_info.push(MinionInfo {
                                id: mid.clone(),
                                timed_out: true,
                                key_status: Some(KeyStatus::Accepted),
                                ..Default::default()
                            });
                            self.minion_info_dirty = true;
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
                    self.fleet_scan_completed = true;
                }
            }
        }

        // Sort minions: alive first, timed-out last, each group alphabetically by ID.
        // Only when something actually changed the roster this tick.
        if self.minion_info_dirty {
            self.minion_info.sort_by(|a, b| {
                a.timed_out
                    .cmp(&b.timed_out)
                    .then(a.id.cmp(&b.id))
            });
            self.minion_info_dirty = false;
        }

        // Refresh collectors at interval, not every tick
        if self.config_snapshot.is_some() {
            let interval = self.config.dashboard_secs.max(1);
            if self.tick_count % interval == 0 {
                self.send_agent_cmd("minion_cache_list", None);
                self.send_agent_cmd("list_keys", None);
                self.send_agent_cmd("list_processes", None);
                self.send_agent_cmd("list_services", None);
                self.send_agent_cmd("get_system_info", None);
                if self.processes.iter().any(|p| p.role == SaltProcessRole::API) {
                    let auth = if self.config.api_stats_auth_mode == "none" {
                        None
                    } else {
                        Some(serde_json::json!({
                            "auth_mode": self.config.api_stats_auth_mode,
                            "auth_eauth": self.config.api_stats_auth_eauth,
                            "auth_username": self.config.api_stats_auth_username,
                            "auth_password": self.config.api_stats_auth_password,
                            "auth_username_pillar_path": self.config.api_stats_auth_username_pillar_path,
                            "auth_password_pillar_path": self.config.api_stats_auth_password_pillar_path,
                        }))
                    };
                    self.send_agent_cmd("get_api_stats", auth);
                }
            }
            // Connections are cheap — /proc/net/tcp reads — refresh every tick
            self.send_agent_cmd("get_zmq_connections", None);
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

            // Auth rate: same per-second treatment
            self.current_auth_rate = self.auth_events_this_sec as f64 / elapsed;
            self.auth_events_this_sec = 0;
            self.auth_rate_history[self.auth_rate_idx] = self.current_auth_rate;
            self.auth_rate_idx = (self.auth_rate_idx + 1) % 2000;

            // Activity matrix: store return count for this second
            self.activity_matrix[self.activity_matrix_idx] = self.current_sec_returns;
            self.activity_matrix_idx = (self.activity_matrix_idx + 1) % 2000;
            self.current_sec_returns = 0;

            // CPU/Memory utilization: same per-second sampling cadence as the
            // rate-based histories above, so all Dashboard sparklines share a
            // consistent time axis. Gated on real system_info having arrived
            // at least once — before that, mem_total_kb is 0 and treating it
            // as "no memory at all" would compute a false 100%-full reading
            // that then lingers in the ring buffer (unlike the live gauge,
            // which just self-corrects on the next real sample).
            if self.system_info.mem_total_kb > 0 {
                self.cpu_history[self.cpu_history_idx] = self.system_info.cpu_percent;
                self.cpu_history_idx = (self.cpu_history_idx + 1) % 2000;

                let mem_total_kb = self.system_info.mem_total_kb;
                let mem_pct = ((mem_total_kb.saturating_sub(self.system_info.mem_avail_kb)) as f64 / mem_total_kb as f64 * 100.0).clamp(0.0, 100.0);
                self.mem_history[self.mem_history_idx] = mem_pct;
                self.mem_history_idx = (self.mem_history_idx + 1) % 2000;

                self.cpu_mem_samples = self.cpu_mem_samples.saturating_add(1);
            }

            // Header radar animation cycle
            self.header_radar_frame = (self.header_radar_frame + 1) % 4;
        }
        self.events_last_sec_count += self.events_since_last_tick.len() as u64;
        self.events_since_last_tick.clear();


        // Compute network transfer rates from cumulative byte counters
        if let Some(prev_time) = self.prev_network_time {
            let elapsed = prev_time.elapsed().as_secs_f64();
            if elapsed > 0.0 {
                let rx_delta = self.system_info.network_rx_bytes.saturating_sub(self.prev_network_rx_bytes);
                let tx_delta = self.system_info.network_tx_bytes.saturating_sub(self.prev_network_tx_bytes);
                self.system_info.network_rx_rate = rx_delta as f64 / elapsed;
                self.system_info.network_tx_rate = tx_delta as f64 / elapsed;
            }
        }
        self.prev_network_rx_bytes = self.system_info.network_rx_bytes;
        self.prev_network_tx_bytes = self.system_info.network_tx_bytes;
        self.prev_network_time = Some(Instant::now());

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

        // Alive detection every 5 ticks
        if self.tick_count % 5 == 0 {
            self.send_agent_cmd("alive_check", None);
        }

        // ZMQ panel latency: dispatch/measure per the configured source.
        match self.config.zmq_latency_source.as_str() {
            "job_ping" => {
                // Drop a stale pending ping (no return within a reasonable
                // window) so a lost/dropped job doesn't permanently block
                // future pings from ever being sent again.
                if let Some((_, sent_at)) = self.job_ping_pending {
                    if sent_at.elapsed().as_secs() > 30 {
                        self.job_ping_pending = None;
                    }
                }
                // Throttled well below tick cadence — this puts real traffic
                // on the event bus, unlike the other two sources. The clock
                // starts *before* dispatch (empty jid = "in flight, unknown
                // yet"), not when the agent's response with the real jid
                // arrives — dispatch already happens inside that same agent
                // round-trip, so starting the clock afterward could catch the
                // job already returned in a fast/local fleet, always reading
                // a flat ~0ms instead of the real round-trip time.
                if self.job_ping_pending.is_none() && self.tick_count % 15 == 0 {
                    self.job_ping_pending = Some((String::new(), Instant::now()));
                    self.send_agent_cmd("job_ping", None);
                }
            }
            "master_stats" => {
                // Passive: just listening for the master's own salt/stats/*
                // events (handled in the event branch above), nothing to send.
            }
            _ => {
                // agent_ipc (default)
                self.zmq_ping_sent_at = Some(Instant::now());
                self.send_agent_cmd("zmq_ping", None);
            }
        }

        // Update job ages from dispatch time (previously always 0 — also fixes
        // self.jobs growing unbounded since the age-out retain below never removed anything)
        let now_epoch_f64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let now_epoch = now_epoch_f64 as u64;
        for job in self.jobs.iter_mut() {
            job.age_seconds = now_epoch.saturating_sub(job.created_epoch as u64);
        }

        // Alert engine checks
        let recent_events: Vec<SaltEvent> = self.events.iter().rev().take(500).cloned().collect();
        let worker_busy_pct = self.worker_busy_pct();
        self.alert_engine.tick(
            &self.minion_info,
            &self.keys,
            &self.services,
            &self.system_info,
            &recent_events,
            self.config.stale_minion_secs,
            self.config.pending_key_secs,
            self.config.disk_usage_pct,
            &self.jobs,
            self.config.job_straggler_secs,
            self.current_auth_rate,
            worker_busy_pct,
            self.config.auth_storm_rate_per_sec,
            self.config.worker_saturation_pct,
            self.config.pending_key_churn_per_min,
            &self.service_trend_snapshots(),
            self.config.service_flap_threshold_24h,
            self.config.service_mem_growth_mb_per_hour,
            self.config.service_mem_abs_threshold_mb,
            self.config.service_mem_pct_threshold,
            &ServiceMemThresholds {
                master_abs_mb: self.config.service_mem_abs_threshold_master_mb,
                master_pct: self.config.service_mem_pct_threshold_master,
                minion_abs_mb: self.config.service_mem_abs_threshold_minion_mb,
                minion_pct: self.config.service_mem_pct_threshold_minion,
                api_abs_mb: self.config.service_mem_abs_threshold_api_mb,
                api_pct: self.config.service_mem_pct_threshold_api,
                syndic_abs_mb: self.config.service_mem_abs_threshold_syndic_mb,
                syndic_pct: self.config.service_mem_pct_threshold_syndic,
            },
            self.api_login_latency_p50_p95().map(|(_, p95)| p95),
            self.config.api_auth_latency_warn_secs,
        );

        // Eagerly finalize jobs that reached full completion — record completion
        // latency and a non-partial outcome for the rolling job-health KPIs.
        for job in self.jobs.iter_mut() {
            // target_count == 0 means the `/new` event's minion list was empty or
            // absent (unresolved glob/compound match, async batch, etc.) — we can
            // never learn the true total, so finalize on first return instead of
            // never, or this job would silently never contribute a latency sample.
            let complete = if job.target_count > 0 {
                job.returned >= job.target_count
            } else {
                job.returned > 0
            };
            if !job.finalized && complete {
                job.finalized = true;
                self.job_completion_latencies.push_back((now_epoch_f64 - job.created_epoch).max(0.0));
                if self.job_completion_latencies.len() > JOB_STATS_WINDOW { self.job_completion_latencies.pop_front(); }
                self.job_outcome_window.push_back(false);
                if self.job_outcome_window.len() > JOB_STATS_WINDOW { self.job_outcome_window.pop_front(); }
            }
        }

        // Age out old jobs — anything removed while still unfinished is a partial close-out
        let max_age = self.config.config_secs * 60; // 60 min default
        let mut kept_jobs = Vec::with_capacity(self.jobs.len());
        for job in self.jobs.drain(..) {
            if job.age_seconds < max_age {
                kept_jobs.push(job);
            } else if !job.finalized {
                self.job_outcome_window.push_back(true);
                if self.job_outcome_window.len() > JOB_STATS_WINDOW { self.job_outcome_window.pop_front(); }
            }
        }
        self.jobs = kept_jobs;

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
                    self.cache_load_error = Some("Request timed out after 120s. Press 'r' to retry.".to_string());
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

        // Purge minion_info entries without accepted keys (ghost protection)
        if !self.keys.is_empty() {
            let accepted: HashSet<&str> = self.keys.iter()
                .filter(|k| k.status == KeyStatus::Accepted)
                .map(|k| k.id.as_str())
                .collect();
            self.minion_info.retain(|m| accepted.contains(m.id.as_str()));
        }
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
        self.cache_load_progress = None;
        self.send_agent_cmd("jobs_list", Some(extra));
    }

    pub fn handle_key(&mut self, code: KeyCode) -> bool {
        // Help overlay handles its own dismiss
        if self.show_help {
            match code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Char('?') | KeyCode::F(1) => {
                    self.show_help = false;
                }
                _ => {}
            }
            return false;
        }

        // Config editor is a global overlay — handle it early regardless of tab
        if matches!(self.detail_popup, Some(DetailPopup::ConfigEditor(_))) {
            return self.handle_config_editor_key(code);
        }

        // Save-to-file prompt is a global overlay too, same reasoning
        if self.save_prompt.is_some() {
            return self.handle_save_prompt_key(code);
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
                        let sniffer_max = self.config.sniffer_buffer_size;
                        // Color occurrences of the just-entered minion
                        // target(s) within captured content (glob-resolved,
                        // same as the Events tab's own filter terms) --
                        // purely cosmetic, not a second filter on top of
                        // `minion_ids`'s own broader relevance matching. See
                        // `SnifferState::color_terms` doc for why: a
                        // wildcard-targeted job relevant to a sniffed minion
                        // doesn't literally contain that minion's name
                        // anywhere in its JSON, so re-filtering display by
                        // literal text match on these same terms would hide
                        // legitimate, already-captured traffic.
                        let color_terms = self.resolve_host_terms(ids.iter().map(|s| s.as_str()));
                        self.sniffer = Some(crate::types::SnifferState {
                            minion_ids: ids.clone(),
                            buffer: std::collections::VecDeque::with_capacity(sniffer_max),
                            max_size: sniffer_max,
                            paused: false,
                            selected: 0,
                            following: true,
                            color_terms,
                            highlight_keywords: Vec::new(),
                            highlight_input: None,
                        });
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
        // Sibling of clipboard_text for 's'/'S' ("save to file"): (content, filename hint)
        let mut save_text: Option<(String, String)> = None;
        let had_popup = self.detail_popup.is_some() && self.popup_tab == Some(self.current_tab);
        // The sniffer lives outside `detail_popup` (see its field doc) so it
        // keeps running when you're elsewhere, but it still owns the Events
        // tab's key input whenever it's the thing on screen there.
        let sniffer_showing = self.current_tab == Tab::Events && self.detail_popup.is_none() && self.sniffer.is_some();

        // Contrast toggle and tab switching work universally (even with popups open)
        // but are muted when a text input mode is active inside the popup
        if had_popup || sniffer_showing {
            let popup_text_input = if sniffer_showing {
                self.sniffer.as_ref().is_some_and(|s| s.highlight_input.is_some())
            } else {
                match &self.detail_popup {
                    Some(DetailPopup::JobDetail { return_is_filtering: true, .. }) => true,
                    _ => false,
                }
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
                    KeyCode::Char('6') => { self.current_tab = Tab::Api; return false; }
                    KeyCode::Char('7') => { self.current_tab = Tab::Pulse; return false; }
                    KeyCode::F(3) => { self.open_config_editor(); return false; }
                    KeyCode::F(2) => {
                        self.mouse_enabled = !self.mouse_enabled;
                        self.needs_mouse_toggle = true;
                        return false;
                    }
                    KeyCode::Char('?') | KeyCode::F(1) => {
                        self.show_help = true;
                        return false;
                    }
                    _ => {}
                }
            }
        }

        // Local variable to defer jid_lookup agent command (avoids borrow conflict)
        let mut trigger_y_lookup: Option<String> = None;
        let mut trigger_grains_fetch: Option<String> = None;
        let mut trigger_event_pretty_lookup: Option<(String, String)> = None;
        // Pre-compute events for MinionEvents popup to avoid borrow conflict
        let mut minion_events_cache: Option<(String, Vec<MinionActivityItem>)> = None;
        if let Some(DetailPopup::MinionEvents { ref minion_id, .. }) = self.detail_popup {
            let mut items = self.minion_activity(minion_id);
            // Apply filter if set
            if !self.minion_events_filter.is_empty() {
                let pat = self.minion_events_filter.to_lowercase();
                items.retain(|item| matches_filter(item, &pat));
            }
            if self.popup_tab == Some(self.current_tab) {
                minion_events_cache = Some((minion_id.clone(), items));
            }
        }
        if had_popup {
            if let Some(ref mut detail) = self.detail_popup {
            match *detail {
                DetailPopup::JobDetail {
                    ref mut selected_return,
                    ref mut minion_scroll,
                    ref mut return_filter,
                    ref mut return_is_filtering,
                    ref mut panel_focus,
                    ref mut content_scroll,
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
                        // self.job_detail_scroll_max is the true viewport-aware
                        // last-scroll-position for the return preview, recomputed
                        // and written back every render (see draw_job_detail) —
                        // clamping against it (instead of growing unbounded) is
                        // what keeps End from scrolling past the last line and
                        // PageUp responsive right afterward.
                        if *content_scroll_mode {
                            match code {
                                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                                    *content_scroll_mode = false;
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    *content_scroll = content_scroll.saturating_sub(1);
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    *content_scroll = (*content_scroll + 1).min(self.job_detail_scroll_max);
                                }
                                KeyCode::PageUp => {
                                    *content_scroll = content_scroll.saturating_sub(10);
                                }
                                KeyCode::PageDown => {
                                    *content_scroll = (*content_scroll + 10).min(self.job_detail_scroll_max);
                                }
                                KeyCode::Home => { *content_scroll = 0; }
                                KeyCode::End => { *content_scroll = self.job_detail_scroll_max; }
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
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                *content_scroll = (*content_scroll + 1).min(self.job_detail_scroll_max);
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                *content_scroll = content_scroll.saturating_sub(1);
                            }
                            KeyCode::PageUp => {
                                *content_scroll = content_scroll.saturating_sub(10);
                            }
                            KeyCode::PageDown => {
                                *content_scroll = (*content_scroll + 10).min(self.job_detail_scroll_max);
                            }
                            KeyCode::Home => { *content_scroll = 0; }
                            KeyCode::End => { *content_scroll = self.job_detail_scroll_max; }
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
                            KeyCode::Char('s') => {
                                if let Some(ref l) = lookup {
                                    let idx = selected_return.unwrap_or(0);
                                    if idx < l.returns.len() {
                                        let ret = &l.returns[idx];
                                        let text = if !ret.formatted_text.is_empty() {
                                            Self::strip_ansi(&ret.formatted_text)
                                        } else {
                                            Self::format_return_as_console(&ret.minion_id, &ret.full_return)
                                        };
                                        save_text = Some((text, format!("job-{}-{}", jid, ret.minion_id)));
                                    }
                                }
                            }
                            KeyCode::Char('S') => {
                                if let Some(ref l) = lookup {
                                    let idx = selected_return.unwrap_or(0);
                                    if idx < l.returns.len() {
                                        let json = serde_json::to_string_pretty(&l.returns[idx].full_return)
                                            .unwrap_or_else(|_| l.returns[idx].summary.clone());
                                        save_text = Some((json, format!("job-{}-{}", jid, l.returns[idx].minion_id)));
                                    }
                                }
                            }
                            _ => {}
                        }
                        // Don't return false — let clipboard_text/save_text flow through
                    } else {
                    // Left panel (return list) focused
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            if let Some((mid, sel)) = self.return_to_minion_events.take() {
                                self.detail_popup = Some(DetailPopup::MinionEvents {
                                    minion_id: mid, selected: sel,
                                });
                            } else {
                                self.detail_popup = None;
                            }
                        }
                        KeyCode::Enter => {
                            if lookup.is_some() {
                                *panel_focus = true;
                                *content_scroll = 0;
                                *content_scroll_mode = false;
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
                        KeyCode::Char('s') => {
                            if let Some(ref l) = lookup {
                                let idx = selected_return.unwrap_or(0);
                                if idx < l.returns.len() {
                                    let ret = &l.returns[idx];
                                    let text = if !ret.formatted_text.is_empty() {
                                        Self::strip_ansi(&ret.formatted_text)
                                    } else {
                                        Self::format_return_as_console(&ret.minion_id, &ret.full_return)
                                    };
                                    save_text = Some((text, format!("job-{}-{}", jid, ret.minion_id)));
                                }
                            }
                        }
                        KeyCode::Char('S') => {
                            if let Some(ref l) = lookup {
                                let idx = selected_return.unwrap_or(0);
                                if idx < l.returns.len() {
                                    let json = serde_json::to_string_pretty(&l.returns[idx].full_return)
                                        .unwrap_or_else(|_| l.returns[idx].summary.clone());
                                    save_text = Some((json, format!("job-{}-{}", jid, l.returns[idx].minion_id)));
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
                            self.detail_popup = self.return_to_minion_detail.take()
                                .map(|mid| DetailPopup::Minion(mid, PingState::Idle, None, 0));
                        }
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Char('a') | KeyCode::Char('A') => {
                            self.admin_mode_pending = false;
                            self.detail_popup = self.return_to_minion_detail.take()
                                .map(|mid| DetailPopup::Minion(mid, PingState::Idle, None, 0));
                        }
                        _ => {}
                    }
                }
                DetailPopup::SaltEvent { ref evt, scroll: ref mut scroll_offset, ref mut pretty, ref mut pretty_fetch } => {
                    // self.event_detail_scroll_max is the true viewport-aware
                    // last-scroll-position, recomputed and written back every
                    // render (see draw_event_detail) — clamping against it here
                    // (rather than against the popup's raw line count, or not at
                    // all) is what keeps Down/PageDown/End from running
                    // scroll_offset past where content actually ends, which
                    // previously made PageUp appear to do nothing for several
                    // presses after End.
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            // Sniffer, if any, isn't stashed here — it lives in
                            // `self.sniffer` independent of `detail_popup` and
                            // reappears on its own once this popup closes.
                            if let Some((mid, sel)) = self.return_to_minion_events.take() {
                                self.detail_popup = Some(DetailPopup::MinionEvents {
                                    minion_id: mid, selected: sel,
                                });
                                self.popup_tab = Some(self.current_tab);
                            } else {
                                self.detail_popup = None;
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            *scroll_offset = scroll_offset.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            *scroll_offset = (*scroll_offset + 1).min(self.event_detail_scroll_max);
                        }
                        KeyCode::PageUp => {
                            *scroll_offset = scroll_offset.saturating_sub(10);
                        }
                        KeyCode::PageDown => {
                            *scroll_offset = (*scroll_offset + 10).min(self.event_detail_scroll_max);
                        }
                        KeyCode::Home => { *scroll_offset = 0; }
                        KeyCode::End => { *scroll_offset = self.event_detail_scroll_max; }
                        KeyCode::Char('y') => {
                            let ts = chrono::DateTime::from_timestamp(evt.ts as i64, 0)
                                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                                .unwrap_or_else(|| evt.ts.to_string());
                            clipboard_text = Some(format!(
                                "Tag: {}\nTimestamp: {}\n\nData:\n{}",
                                evt.tag,
                                ts,
                                event_body_as_copied_text(evt, *pretty, pretty_fetch),
                            ));
                        }
                        KeyCode::Char('Y') => {
                            clipboard_text = Some(event_body_as_copied_text(evt, *pretty, pretty_fetch));
                        }
                        KeyCode::Char('s') => {
                            let ts = chrono::DateTime::from_timestamp(evt.ts as i64, 0)
                                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                                .unwrap_or_else(|| evt.ts.to_string());
                            let text = format!(
                                "Tag: {}\nTimestamp: {}\n\nData:\n{}",
                                evt.tag,
                                ts,
                                event_body_as_copied_text(evt, *pretty, pretty_fetch),
                            );
                            save_text = Some((text, format!("event-{}", evt.tag)));
                        }
                        KeyCode::Char('S') => {
                            save_text = Some((
                                event_body_as_copied_text(evt, *pretty, pretty_fetch),
                                format!("event-{}", evt.tag),
                            ));
                        }
                        KeyCode::Char('t') | KeyCode::Char('T') => {
                            *pretty = !*pretty;
                            *scroll_offset = 0;
                            // Fetch the real `salt-run jobs.lookup_jid` text once
                            // per event (same subprocess + flags Job Detail uses
                            // -- see `cmd_jid_lookup` in the agent) instead of
                            // relying solely on the client-side approximation,
                            // so pretty-view colors/formatting match this
                            // master's actual Salt output. Only worth doing for
                            // a job-return event, and only once (Idle).
                            if *pretty && matches!(pretty_fetch, crate::types::PrettyFetch::Idle) {
                                let jid = evt.data.get("jid").and_then(|v| v.as_str());
                                let mid = evt.data.get("id").and_then(|v| v.as_str());
                                if let (Some(jid), Some(mid)) = (jid, mid) {
                                    trigger_event_pretty_lookup = Some((jid.to_string(), mid.to_string()));
                                    *pretty_fetch = crate::types::PrettyFetch::Loading;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                DetailPopup::Alert(ref alert) => {
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            if let Some((mid, sel)) = self.return_to_minion_events.take() {
                                self.detail_popup = Some(DetailPopup::MinionEvents {
                                    minion_id: mid, selected: sel,
                                });
                            } else {
                                self.detail_popup = None;
                            }
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
                        KeyCode::Char('s') | KeyCode::Char('S') => {
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
                            save_text = Some((text, format!("alert-{}", alert.key)));
                        }
                        _ => {}
                    }
                }
                DetailPopup::Minion(ref mid, _, ref mut grains, ref mut grains_scroll) => {
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            self.detail_popup = None;
                        }
                        KeyCode::Char('g') | KeyCode::Char('G') if self.admin_mode => {
                            trigger_grains_fetch = Some(mid.clone());
                            *grains = Some(Vec::new());
                            *grains_scroll = 0;
                        }
                        KeyCode::Char('a') | KeyCode::Char('A') if self.admin_mode => {
                            self.admin_mode = false;
                        }
                        KeyCode::Char('a') | KeyCode::Char('A') if !self.admin_mode && !self.is_filtering => {
                            self.return_to_minion_detail = Some(mid.clone());
                            self.admin_mode_pending = true;
                            self.detail_popup = Some(DetailPopup::AdminConfirm);
                            self.popup_tab = Some(self.current_tab);
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            *grains_scroll = grains_scroll.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let max = grains.as_ref().map_or(0, |p| p.len().saturating_sub(1));
                            *grains_scroll = (*grains_scroll + 1).min(max);
                        }
                        KeyCode::PageUp => {
                            *grains_scroll = grains_scroll.saturating_sub(10);
                        }
                        KeyCode::PageDown => {
                            let max = grains.as_ref().map_or(0, |p| p.len().saturating_sub(1));
                            *grains_scroll = (*grains_scroll + 10).min(max);
                        }
                        KeyCode::Home => { *grains_scroll = 0; }
                        KeyCode::End => {
                            *grains_scroll = grains.as_ref().map_or(0, |p| p.len().saturating_sub(1));
                        }
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            clipboard_text = Some(mid.clone());
                        }
                        KeyCode::Char('s') | KeyCode::Char('S') => {
                            save_text = Some((mid.clone(), format!("minion-{}", mid)));
                        }
                        _ => {}
                    }
                }
                DetailPopup::MinionEvents { ref minion_id, ref mut selected } => {
                    let items = minion_events_cache.as_ref()
                        .filter(|(id, _)| id == minion_id)
                        .map(|(_, e)| e.clone())
                        .unwrap_or_default();
                    let items_len = items.len();
                    let max_idx = items_len.saturating_sub(1);

                    if self.minion_events_is_filtering {
                        match code {
                            KeyCode::Esc => {
                                self.minion_events_is_filtering = false;
                                self.minion_events_filter.clear();
                            }
                            KeyCode::Enter => {
                                self.minion_events_is_filtering = false;
                                *selected = 0;
                            }
                            KeyCode::Backspace => {
                                self.minion_events_filter.pop();
                            }
                            KeyCode::Char(c) if !c.is_control() => {
                                self.minion_events_filter.push(c);
                            }
                            _ => {}
                        }
                    } else {
                        let evt_enter = match code {
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                                self.return_to_minion_events = None;
                                self.minion_events_is_filtering = false;
                                self.minion_events_filter.clear();
                                self.detail_popup = None;
                                None
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                *selected = selected.saturating_sub(1);
                                None
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                *selected = (*selected + 1).min(max_idx);
                                None
                            }
                            KeyCode::PageUp => {
                                *selected = selected.saturating_sub(10);
                                None
                            }
                            KeyCode::PageDown => {
                                *selected = (*selected + 10).min(max_idx);
                                None
                            }
                            KeyCode::Home => { *selected = 0; None }
                            KeyCode::End => { *selected = max_idx; None }
                            KeyCode::Char('/') => {
                                self.minion_events_is_filtering = true;
                                self.minion_events_filter.clear();
                                None
                            }
                            KeyCode::Enter => {
                                self.return_to_minion_events = Some((minion_id.clone(), *selected));
                                items.get(*selected).and_then(|item| match item {
                                    MinionActivityItem::Event(_, evt) => Some(evt.clone()),
                                    MinionActivityItem::CacheJob(job) => {
                                        let jid = job.jid.clone();
                                        let req_id = self.send_agent_cmd("jid_lookup", Some(serde_json::json!({"jid": &jid})));
                                        self.pending_jid_lookup = Some((req_id, jid.clone()));
                                        self.pending_jid_lookup_at = Some(Instant::now());
                                        self.detail_popup = Some(DetailPopup::JobDetail {
                                            jid,
                                            lookup: None,
                                            selected_return: None,
                                            job_fun: job.fun.clone(),
                                            job_tgt: job.tgt.clone(),
                                            job_start_epoch: job.start_epoch,
                                            minion_scroll: 0,
                                            return_filter: String::new(),
                                            return_is_filtering: false,
                                            panel_focus: false,
                                            content_scroll: 0,
                                            content_scroll_mode: false,
                                        });
                                        self.popup_tab = Some(self.current_tab);
                                        None
                                    }
                                    MinionActivityItem::Alert(alert) => {
                                        self.detail_popup = Some(DetailPopup::Alert(alert.clone()));
                                        None
                                    }
                                })
                            }
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                if let Some(item) = items.get(*selected) {
                                    match item {
                                        MinionActivityItem::Event(_, evt) => {
                                            clipboard_text = Some(serde_json::to_string_pretty(&evt.data)
                                                .unwrap_or_else(|_| evt.tag.clone()));
                                        }
                                        MinionActivityItem::CacheJob(job) => {
                                            clipboard_text = Some(format!("{} {} {} {}",
                                                job.jid, job.fun, job.tgt, job.start_time));
                                        }
                                        MinionActivityItem::Alert(alert) => {
                                            clipboard_text = Some(format!("[{}] {}: {}",
                                                alert.severity.label(), alert.rule, alert.description));
                                        }
                                    }
                                }
                                None
                            }
                            KeyCode::Char('s') | KeyCode::Char('S') => {
                                if let Some(item) = items.get(*selected) {
                                    match item {
                                        MinionActivityItem::Event(_, evt) => {
                                            let text = serde_json::to_string_pretty(&evt.data)
                                                .unwrap_or_else(|_| evt.tag.clone());
                                            save_text = Some((text, format!("event-{}", evt.tag)));
                                        }
                                        MinionActivityItem::CacheJob(job) => {
                                            let text = format!("{} {} {} {}",
                                                job.jid, job.fun, job.tgt, job.start_time);
                                            save_text = Some((text, format!("job-{}-{}", job.jid, minion_id)));
                                        }
                                        MinionActivityItem::Alert(alert) => {
                                            let text = format!("[{}] {}: {}",
                                                alert.severity.label(), alert.rule, alert.description);
                                            save_text = Some((text, format!("alert-{}", alert.key)));
                                        }
                                    }
                                }
                                None
                            }
                            _ => None,
                        };
                        if let Some(evt) = evt_enter {
                            self.detail_popup = Some(DetailPopup::SaltEvent { evt, scroll: 0, pretty: false, pretty_fetch: crate::types::PrettyFetch::Idle });
                        }
                    }
                }
                DetailPopup::ApiRequest(ref entry) => {
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            self.detail_popup = None;
                        }
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            clipboard_text = Some(format!(
                                "{}\nClient: {}\nStatus: {}\nProcessing time: {:.3}s",
                                entry.raw_request_line, entry.client,
                                entry.response_status, entry.processing_time_secs,
                            ));
                        }
                        KeyCode::Char('s') | KeyCode::Char('S') => {
                            let text = format!(
                                "{}\nClient: {}\nStatus: {}\nProcessing time: {:.3}s",
                                entry.raw_request_line, entry.client,
                                entry.response_status, entry.processing_time_secs,
                            );
                            save_text = Some((text, format!("api-request-{}", entry.client)));
                        }
                        _ => {}
                    }
                }
                DetailPopup::ConfigEditor(_) => {}
            }
        }
        if let Some(text) = clipboard_text {
            self.copy_to_clipboard(&text);
            self.clipboard_at = Some(Instant::now());
            return false;
        }
        if let Some((content, hint)) = save_text {
            self.open_save_prompt(content, &hint);
            return false;
        }
        } else if sniffer_showing {
            if let Some(ref mut state) = self.sniffer {
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
                // Nav/selection/Enter/y all operate on the filtered
                // (displayed) view, not raw buffer indices — see
                // `SnifferState::filtered_indices`.
                let visible = state.filtered_indices();
                match code {
                    KeyCode::Enter => {
                        if let Some(evt) = visible.get(state.selected).and_then(|&i| state.buffer.get(i)) {
                            // `self.sniffer` isn't touched -- it keeps
                            // capturing underneath and reappears on its own
                            // once this SaltEvent popup closes.
                            self.detail_popup = Some(DetailPopup::SaltEvent { evt: evt.clone(), scroll: 0, pretty: false, pretty_fetch: crate::types::PrettyFetch::Idle });
                            self.popup_tab = Some(self.current_tab);
                        }
                    }
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                        self.sniffer = None;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        state.selected = state.selected.saturating_sub(1);
                        state.following = state.selected + 1 >= visible.len();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let max = visible.len().saturating_sub(1);
                        state.selected = state.selected.saturating_add(1).min(max);
                        state.following = state.selected + 1 >= visible.len();
                    }
                    KeyCode::PageUp => {
                        state.selected = state.selected.saturating_sub(10);
                        state.following = state.selected + 1 >= visible.len();
                    }
                    KeyCode::PageDown => {
                        let max = visible.len().saturating_sub(1);
                        state.selected = state.selected.saturating_add(10).min(max);
                        state.following = state.selected + 1 >= visible.len();
                    }
                    KeyCode::Home => {
                        state.selected = 0;
                        state.following = visible.len() <= 1;
                    }
                    KeyCode::End => {
                        state.selected = visible.len().saturating_sub(1);
                        state.following = true;
                    }
                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        state.paused = !state.paused;
                    }
                    KeyCode::Char('h') | KeyCode::Char('H') => {
                        state.highlight_input = Some(state.highlight_keywords.join(", "));
                    }
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        let mut text = String::new();
                        for (i, &buf_idx) in visible.iter().enumerate() {
                            let Some(evt) = state.buffer.get(buf_idx) else { continue };
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
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        let mut text = String::new();
                        for (i, &buf_idx) in visible.iter().enumerate() {
                            let Some(evt) = state.buffer.get(buf_idx) else { continue };
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
                            save_text = Some((text, format!("sniffer-{}", state.minion_ids.join("-"))));
                        }
                    }
                    _ => {}
                }
            }
            if let Some(text) = clipboard_text {
                self.copy_to_clipboard(&text);
                self.clipboard_at = Some(Instant::now());
                return false;
            }
            if let Some((content, hint)) = save_text {
                self.open_save_prompt(content, &hint);
                return false;
            }
        }
        if let Some((jid, mid)) = trigger_event_pretty_lookup {
            let req_id = self.send_agent_cmd("jid_lookup", Some(serde_json::json!({"jid": &jid})));
            self.pending_event_pretty = Some((req_id, jid, mid));
            self.pending_event_pretty_at = Some(Instant::now());
        }
        if let Some(jid) = trigger_y_lookup {
            let req_id = self.send_agent_cmd("jid_lookup", Some(serde_json::json!({"jid": &jid})));
            self.pending_y_copy = Some((req_id, jid));
        }
        if let Some(mid) = trigger_grains_fetch {
            self.send_agent_cmd("minion_grains", Some(serde_json::json!({"minion_id": mid})));
        }
        if had_popup || sniffer_showing {
            return false;
        }

        // Filter input mode (live jobs / other tabs)
        if self.is_filtering {
            match code {
                KeyCode::Esc => {
                    self.is_filtering = false;
                    self.filter_text.clear();
                    if self.current_tab == Tab::Events {
                        self.applied_filter.clear();
                        self.filtered_events_cache.clear();
                    } else {
                        self.commit_tab_filter("");
                    }
                    self.clamp_selection();
                    self.follow_events_tail_if_filtering();
                }
                KeyCode::Enter => {
                    self.is_filtering = false;
                    self.job_selected = 0;
                    self.alert_selected = 0;
                    self.minion_selected = 0;
                    if self.current_tab == Tab::Events {
                        // Committed here rather than kept live while typing --
                        // see the Backspace/Char arms below.
                        self.applied_filter = self.filter_text.clone();
                        self.rebuild_filtered_events_cache();
                        self.event_selected = self.filtered_events_len().saturating_sub(1);
                        self.events_following = true;
                    } else {
                        self.event_selected = 0;
                        let text = self.filter_text.clone();
                        self.commit_tab_filter(&text);
                    }
                    self.clamp_selection();
                }
                KeyCode::Backspace => {
                    self.filter_text.pop();
                    // Events applies its filter on Enter (see above), not per
                    // keystroke -- `self.applied_filter` doesn't change here,
                    // so re-clamping/re-following against it on every
                    // character is wasted work (and was the direct cause of
                    // typing itself stalling the TUI on masters with large
                    // event buffers). Other tabs still filter live.
                    if self.current_tab != Tab::Events {
                        self.clamp_active_filter_selection();
                        self.follow_events_tail_if_filtering();
                    }
                }
                KeyCode::Char(c) => {
                    self.filter_text.push(c);
                    if self.current_tab != Tab::Events {
                        self.clamp_active_filter_selection();
                        self.follow_events_tail_if_filtering();
                    }
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
                    self.clamp_active_filter_selection();
                }
                KeyCode::Char(c) => {
                    self.cache_filter_text.push(c);
                    self.clamp_active_filter_selection();
                }
                _ => {}
            }
            return false;
        }

        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => return true,
            KeyCode::Char('=') | KeyCode::Char('+') => {
                self.tick_interval_ms = (self.tick_interval_ms + 500).min(30_000);
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                self.tick_interval_ms = self.tick_interval_ms.saturating_sub(500).max(500);
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
                    self.fleet_scan_completed = false;
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
                } else if self.current_tab == Tab::Api {
                    self.config.api_hide_stats_polls = !self.config.api_hide_stats_polls;
                }
            }
            KeyCode::Char('z') | KeyCode::Char('Z') => {
                if self.current_tab == Tab::Minions && !self.hide_timed_out {
                    self.timed_out_collapsed = !self.timed_out_collapsed;
                    self.clamp_selection();
                }
            }
            KeyCode::Char('f') | KeyCode::Char('F') => {
                if self.current_tab == Tab::Minions {
                    self.config.minions_follow_filter = !self.config.minions_follow_filter;
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                if self.current_tab == Tab::Events && self.detail_popup.is_none() && self.sniffer.is_none() && !self.is_filtering {
                    self.sniffer_input_active = true;
                    self.sniffer_input_text.clear();
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                if self.current_tab == Tab::Minions {
                    if let Some(minion) = self.get_filtered_minion(self.minion_selected) {
                        self.detail_popup = Some(DetailPopup::MinionEvents {
                            minion_id: minion.id.clone(),
                            selected: 0,
                        });
                        self.popup_tab = Some(self.current_tab);
                        if !self.cache_loaded {
                            self.send_cache_jobs_list(None, false);
                        }
                    }
                } else if self.current_tab == Tab::Jobs && !self.cache_loading {
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
            KeyCode::Char('?') | KeyCode::F(1) => {
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
                if self.current_tab == Tab::Pulse {
                    // Pulse tab has no filter
                } else if self.current_tab == Tab::Jobs {
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
                    if self.current_tab == Tab::Events {
                        self.applied_filter.clear();
                        self.filtered_events_cache.clear();
                    }
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
                } else if self.current_tab == Tab::Pulse && !had_popup {
                    self.pulse_left();
                }
            }
            KeyCode::Right => {
                if self.current_tab == Tab::Jobs && !had_popup
                    && !self.is_filtering && !self.cache_is_filtering
                {
                    self.cache_focus = true;
                } else if self.current_tab == Tab::Pulse && !had_popup {
                    self.pulse_right();
                }
            }
            KeyCode::Char('1') => self.current_tab = Tab::Dashboard,
            KeyCode::Char('2') => self.current_tab = Tab::Minions,
            KeyCode::Char('3') => self.current_tab = Tab::Events,
            KeyCode::Char('4') => self.current_tab = Tab::Jobs,
            KeyCode::Char('5') => self.current_tab = Tab::Alerts,
            KeyCode::Char('6') => self.current_tab = Tab::Api,
            KeyCode::Char('7') => self.current_tab = Tab::Pulse,
            KeyCode::Up => {
                if self.current_tab == Tab::Pulse && !had_popup {
                    self.pulse_up();
                } else {
                    self.move_up();
                }
            }
            KeyCode::Down => {
                if self.current_tab == Tab::Pulse && !had_popup {
                    self.pulse_down();
                } else {
                    self.move_down();
                }
            }
            KeyCode::PageUp => { for _ in 0..10 { self.move_up(); } }
            KeyCode::PageDown => { for _ in 0..10 { self.move_down(); } }
            KeyCode::Home => self.move_to_first(),
            KeyCode::End => self.move_to_last(),
            KeyCode::Enter => self.open_detail(),
            KeyCode::F(2) => {
                self.mouse_enabled = !self.mouse_enabled;
                self.needs_mouse_toggle = true;
            }
            KeyCode::F(3) => self.open_config_editor(),
            KeyCode::F(4) if self.current_tab == Tab::Dashboard => {
                self.config.show_cpu_graph = !self.config.show_cpu_graph;
            }
            KeyCode::F(5) if self.current_tab == Tab::Dashboard => {
                self.config.show_mem_graph = !self.config.show_mem_graph;
            }
            KeyCode::F(6) if self.current_tab == Tab::Dashboard => {
                self.config.show_zmq_graph = !self.config.show_zmq_graph;
            }
            KeyCode::F(7) if self.current_tab == Tab::Dashboard => {
                self.config.show_events_graph = !self.config.show_events_graph;
            }
            KeyCode::F(8) if self.current_tab == Tab::Dashboard => {
                self.config.show_activity_graph = !self.config.show_activity_graph;
            }
            _ => {}
        }
        false
    }

    /// Handle a mouse click at the given (col, row) position.
    /// Row 1 = tab bar: switch to clicked tab.
    /// Content area = list tabs: select item by proportional position.
    pub fn handle_mouse_click(&mut self, col: u16, row: u16, frame_size: Rect) -> bool {
        let h = frame_size.height as usize;

        // Popup showing — AdminConfirm and MinionEvents handle left-click
        if self.detail_popup.is_some() && self.popup_tab == Some(self.current_tab) {
            if matches!(self.detail_popup, Some(DetailPopup::AdminConfirm)) {
                return self.handle_admin_confirm_click(col, row, frame_size);
            }
            if matches!(self.detail_popup, Some(DetailPopup::MinionEvents { .. })) {
                return self.handle_minion_events_click(col, row, frame_size);
            }
            // Other popups: ignore left-click (right-click closes from main.rs)
            return false;
        }

        // Tab bar click (row 1) — only within an actual tab label's column
        // range; clicking the blank space between/after labels does nothing
        // instead of silently falling through to Pulse (tab_at_col's old
        // catch-all made any row-1 click switch tabs, which made a single
        // stray/garbled mouse event — e.g. from a KVM switch's input
        // handoff — very likely to swap the view even though it didn't
        // land anywhere near a real tab).
        if row == 1 {
            if let Some(tab) = Self::tab_at_col(col) {
                self.current_tab = tab;
            }
            return false;
        }

        // Content area click (rows 3 to h-2; h-1 = key hints).
        // Layout (src/ui/mod.rs): title/tab frame is Length(3) (rows 0-2),
        // content area starts at row 3, status bar is the last row (h-1).
        let r = row as usize;
        if r < 3 || r >= h.saturating_sub(1) {
            return false;
        }

        // Content area height = h-4 (title/tab frame + status bar).
        // Inner panel height = (h-4) - 2 (block top/bottom borders) = h-6.
        let inner_h = h.saturating_sub(6);

        match self.current_tab {
            Tab::Minions => {
                let total = self.filtered_minions_len();
                if total == 0 { return false; }

                // Must match draw_minion_list's own conditions exactly (src/ui/minions.rs)
                // -- unlike Events/Alerts/Jobs, Minions' own `visible` count is reduced
                // by this same overhead, not just its data_start.
                let filter_shown = self.is_filtering || !self.filter_text_for(Tab::Minions).is_empty() || self.hide_timed_out;
                let overhead = if filter_shown { 1 } else { 0 }
                    + if self.admin_mode { 1 } else { 0 };
                let visible = inner_h.saturating_sub(overhead).saturating_sub(2).max(1);

                let selected = self.minion_selected.min(total.saturating_sub(1));
                let scroll = scroll_offset(selected, visible, total);

                // Data starts at: inner.y(4) + overhead + header(1)
                let data_start = 4 + overhead + 1;
                if r >= data_start {
                    let visual_idx = r - data_start;
                    if visual_idx < visible {
                        self.minion_selected = (scroll + visual_idx).min(total - 1);
                        self.clamp_selection();
                    }
                }
            }
            Tab::Events => {
                // Events has 50/50 horizontal split: only left half is the event list
                let split_col = (frame_size.width / 2) as usize;
                if (col as usize) >= split_col {
                    return false;
                }
                let total = self.filtered_events_len();
                if total == 0 { return false; }

                // draw_event_list's `visible` always reserves 2 rows (filter+header)
                // regardless of whether the filter bar actually renders, but its
                // data_start only advances past the filter bar when it's actually
                // shown -- must match src/ui/events.rs exactly.
                let visible = inner_h.saturating_sub(2).max(1);
                let selected = self.event_selected.min(total.saturating_sub(1));
                let scroll = scroll_offset(selected, visible, total);

                let filter_shown = self.is_filtering || !self.applied_filter.is_empty();
                let data_start = 4 + if filter_shown { 1 } else { 0 } + 1;
                if r >= data_start {
                    let visual_idx = r - data_start;
                    if visual_idx < visible {
                        self.event_selected = (scroll + visual_idx).min(total - 1);
                        self.events_following = self.event_selected + 1 >= total;
                        self.clamp_selection();
                    }
                }
            }
            Tab::Alerts => {
                let total = self.filtered_alerts_len();
                if total == 0 { return false; }

                // Same shape as Events -- must match src/ui/alerts_view.rs exactly.
                let visible = inner_h.saturating_sub(2).max(1);
                let selected = self.alert_selected.min(total.saturating_sub(1));
                let scroll = scroll_offset(selected, visible, total);

                let filter_shown = self.is_filtering || !self.filter_text_for(Tab::Alerts).is_empty();
                let data_start = 4 + if filter_shown { 1 } else { 0 } + 1;
                if r >= data_start {
                    let visual_idx = r - data_start;
                    if visual_idx < visible {
                        self.alert_selected = (scroll + visual_idx).min(total - 1);
                        self.clamp_selection();
                    }
                }
            }
            Tab::Api => {
                let total = self.filtered_api_log_len();
                if total == 0 { return false; }

                // Same shape as Events -- must match draw_api's request-log panel exactly.
                let visible = inner_h.saturating_sub(2).max(1);
                let selected = self.api_log_selected.min(total.saturating_sub(1));
                let scroll = scroll_offset(selected, visible, total);

                let filter_shown = self.is_filtering || !self.filter_text_for(Tab::Api).is_empty();
                let data_start = 4 + if filter_shown { 1 } else { 0 } + 1;
                if r >= data_start {
                    let visual_idx = r - data_start;
                    if visual_idx < visible {
                        self.api_log_selected = (scroll + visual_idx).min(total - 1);
                        self.clamp_selection();
                    }
                }
            }
            Tab::Jobs => {
                // 55/45 split: left = live jobs, right = cache
                let col_split = (frame_size.width as u16 * 55 / 100) as usize;
                let live_click = (col as usize) < col_split;

                if self.cache_focus == live_click {
                    // Click is on the non-focused panel — just switch focus
                    self.cache_focus = !self.cache_focus;
                    return false;
                }

                if self.cache_focus {
                    let total = self.filtered_cache_len();
                    if total == 0 { return false; }

                    // Same shape as Events -- must match draw_cache_panel exactly.
                    let visible = inner_h.saturating_sub(2).max(1);
                    let selected = self.cache_selected.min(total.saturating_sub(1));
                    let scroll = scroll_offset(selected, visible, total);

                    let filter_shown = self.cache_is_filtering || !self.cache_server_filter.is_empty();
                    let data_start = 4 + if filter_shown { 1 } else { 0 } + 1;
                    if r >= data_start {
                        let visual_idx = r - data_start;
                        if visual_idx < visible {
                            self.cache_selected = (scroll + visual_idx).min(total - 1);
                            self.clamp_selection();
                        }
                    }
                } else {
                    let total = self.filtered_jobs_len();
                    if total == 0 { return false; }

                    // Same shape as Events -- must match draw_live_jobs_panel exactly.
                    let visible = inner_h.saturating_sub(2).max(1);
                    let selected = self.job_selected.min(total.saturating_sub(1));
                    let scroll = scroll_offset(selected, visible, total);

                    let filter_shown = self.is_filtering || !self.filter_text_for(Tab::Jobs).is_empty();
                    let data_start = 4 + if filter_shown { 1 } else { 0 } + 1;
                    if r >= data_start {
                        let visual_idx = r - data_start;
                        if visual_idx < visible {
                            self.job_selected = (scroll + visual_idx).min(total - 1);
                            self.clamp_selection();
                        }
                    }
                }
            }
            Tab::Pulse => {
                // Hit-test against the dot grid's actual rendered area
                // (recorded by draw_dot_panel each frame) rather than
                // re-deriving its screen geometry here, so this can't
                // silently drift out of sync with the real layout again.
                let area = self.pulse_grid_area;
                let cx = col as usize;
                let cy = row as usize;

                if cy >= area.y as usize && cy < (area.y + area.height) as usize
                    && cx >= area.x as usize && cx < (area.x + area.width) as usize
                {
                    let grid_col = cx - area.x as usize;
                    let grid_row = cy - area.y as usize;
                    let cols = self.pulse_grid_cols.max(1);
                    let idx = grid_row * cols + grid_col;
                    let count = self.pulse_minions_count();
                    if idx < count {
                        self.pulse_selected = idx;
                    }
                }
            }
            Tab::Dashboard => {}
        }

        // Double-click detection on list tabs
        let dbl_click_ms: u128 = 400;
        if self.current_tab != Tab::Dashboard && self.current_tab != Tab::Pulse {
            let now = Instant::now();
            let is_double = self.last_click_at
                .map(|t| now.duration_since(t).as_millis() < dbl_click_ms && col == self.last_click_col && row == self.last_click_row)
                .unwrap_or(false);
            self.last_click_at = Some(now);
            self.last_click_col = col;
            self.last_click_row = row;
            if is_double {
                self.open_detail();
            }
        } else {
            self.last_click_at = None;
        }

        false
    }

    fn handle_admin_confirm_click(&mut self, col: u16, row: u16, frame_size: Rect) -> bool {
        let area = Self::centered_rect(55, 35, frame_size);
        let inside = row >= area.top() && row < area.top() + area.height
            && col >= area.left() && col < area.left() + area.width;

        if inside {
            let inner_x = col.saturating_sub(area.left() + 1);
            // "     [ OK ] Enter          [ Cancel ] Esc/q/a"
            //  [ OK ] at inner_x 5-11
            if inner_x >= 5 && inner_x <= 11 {
                self.admin_mode = true;
            }
            self.admin_mode_pending = false;
        } else {
            self.admin_mode_pending = false;
        }
        self.detail_popup = None;
        false
    }

    fn handle_minion_events_click(&mut self, col: u16, row: u16, frame_size: Rect) -> bool {
        let mid = match self.detail_popup {
            Some(DetailPopup::MinionEvents { ref minion_id, .. }) => minion_id.clone(),
            _ => return false,
        };

        let area = Self::centered_rect(85, 80, frame_size);
        let ix = area.x + 1;
        let iy = area.y + 1;
        let iw = area.width.saturating_sub(2);
        let ih = area.height.saturating_sub(2);

        if col < ix || col >= ix + iw
            || row < iy || row >= iy + ih.saturating_sub(1)
        {
            return false;
        }

        let para_row = (row - iy) as usize;
        if para_row < 4 {
            return false;
        }

        let items = self.minion_activity(&mid);
        let total = items.len();
        if total == 0 { return false; }

        let current_sel = match self.detail_popup {
            Some(DetailPopup::MinionEvents { selected, .. }) => selected,
            _ => return false,
        };
        let sel = current_sel.min(total.saturating_sub(1));

        let content_h = (ih as usize).saturating_sub(4);
        let scroll = if sel > content_h / 2 {
            sel.saturating_sub(content_h / 2).min(total.saturating_sub(1).saturating_sub(content_h / 2).max(0))
        } else {
            0
        };

        let item_idx = scroll + para_row - 4;
        if item_idx >= total { return false; }

        if let Some(DetailPopup::MinionEvents { ref mut selected, .. }) = self.detail_popup {
            *selected = item_idx;

            let dbl_click_ms: u128 = 400;
            let now = Instant::now();
            let is_double = self.last_click_at
                .map(|t| now.duration_since(t).as_millis() < dbl_click_ms && col == self.last_click_col && row == self.last_click_row)
                .unwrap_or(false);
            self.last_click_at = Some(now);
            self.last_click_col = col;
            self.last_click_row = row;
            if is_double {
                if let Some(item) = items.get(item_idx) {
                    match item {
                        MinionActivityItem::Event(_, evt) => {
                            self.return_to_minion_events = Some((mid, item_idx));
                            self.detail_popup = Some(DetailPopup::SaltEvent { evt: evt.clone(), scroll: 0, pretty: false, pretty_fetch: crate::types::PrettyFetch::Idle });
                        }
                        MinionActivityItem::CacheJob(job) => {
                            self.return_to_minion_events = Some((mid, item_idx));
                            let jid = job.jid.clone();
                            let req_id = self.send_agent_cmd("jid_lookup", Some(serde_json::json!({"jid": &jid})));
                            self.pending_jid_lookup = Some((req_id, jid.clone()));
                            self.pending_jid_lookup_at = Some(Instant::now());
                            self.detail_popup = Some(DetailPopup::JobDetail {
                                jid,
                                lookup: None,
                                selected_return: None,
                                job_fun: job.fun.clone(),
                                job_tgt: job.tgt.clone(),
                                job_start_epoch: job.start_epoch,
                                minion_scroll: 0,
                                return_filter: String::new(),
                                return_is_filtering: false,
                                panel_focus: false,
                                content_scroll: 0,
                                content_scroll_mode: false,
                            });
                            self.popup_tab = Some(self.current_tab);
                        }
                        MinionActivityItem::Alert(alert) => {
                            self.return_to_minion_events = Some((mid, item_idx));
                            self.detail_popup = Some(DetailPopup::Alert(alert.clone()));
                        }
                    }
                }
            }
        }

        false
    }

    fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
        let pad_v = (100u16.saturating_sub(percent_y)) / 2;
        let pad_h = (100u16.saturating_sub(percent_x)) / 2;
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(pad_v),
                Constraint::Percentage(percent_y),
                Constraint::Percentage(pad_v),
            ])
            .split(r);
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(pad_h),
                Constraint::Percentage(percent_x),
                Constraint::Percentage(pad_h),
            ])
            .split(vertical[1])[1]
    }

    /// Map a column position on the tab bar (row 1) to a tab.
    /// `None` for a column that doesn't land on any tab's label (the gaps
    /// between them, or anything past the last one) — callers should treat
    /// that as "not a tab bar click" rather than defaulting to some tab.
    fn tab_at_col(col: u16) -> Option<Tab> {
        match col {
            0..=12 => Some(Tab::Dashboard),
            16..=26 => Some(Tab::Minions),
            30..=39 => Some(Tab::Events),
            43..=50 => Some(Tab::Jobs),
            54..=63 => Some(Tab::Alerts),
            67..=73 => Some(Tab::Api),
            77..=85 => Some(Tab::Pulse),
            _ => None,
        }
    }

    fn move_up(&mut self) {
        match self.current_tab {
            Tab::Minions => self.minion_selected = self.minion_selected.saturating_sub(1),
            Tab::Events => {
                self.event_selected = self.event_selected.saturating_sub(1);
                self.events_following = self.event_selected + 1 >= self.filtered_events_len();
            }
            Tab::Jobs => {
                if self.cache_focus {
                    self.cache_selected = self.cache_selected.saturating_sub(1);
                } else {
                    self.job_selected = self.job_selected.saturating_sub(1);
                }
            }
            Tab::Alerts => self.alert_selected = self.alert_selected.saturating_sub(1),
            Tab::Api => self.api_log_selected = self.api_log_selected.saturating_sub(1),
            Tab::Pulse => {
                let max = self.pulse_minions_count().saturating_sub(1);
                self.pulse_selected = self.pulse_selected.saturating_sub(1).min(max);
            }
            _ => {}
        }
    }

    fn move_down(&mut self) {
        match self.current_tab {
            Tab::Minions => self.minion_selected += 1,
            Tab::Events => {
                self.event_selected += 1;
                self.events_following = self.event_selected + 1 >= self.filtered_events_len();
            }
            Tab::Jobs => {
                if self.cache_focus {
                    self.cache_selected += 1;
                } else {
                    self.job_selected += 1;
                }
            }
            Tab::Alerts => self.alert_selected += 1,
            Tab::Api => self.api_log_selected += 1,
            Tab::Pulse => {
                let max = self.pulse_minions_count().saturating_sub(1);
                self.pulse_selected = self.pulse_selected.saturating_add(1).min(max);
            }
            _ => {}
        }
    }

    fn move_to_first(&mut self) {
        match self.current_tab {
            Tab::Minions => self.minion_selected = 0,
            Tab::Events => {
                self.event_selected = 0;
                self.events_following = self.filtered_events_len() <= 1;
            }
            Tab::Jobs => {
                if self.cache_focus {
                    self.cache_selected = 0;
                } else {
                    self.job_selected = 0;
                }
            }
            Tab::Alerts => self.alert_selected = 0,
            Tab::Api => self.api_log_selected = 0,
            Tab::Pulse => self.pulse_selected = 0,
            _ => {}
        }
    }

    fn move_to_last(&mut self) {
        match self.current_tab {
            Tab::Minions => self.minion_selected = self.filtered_minions_len().saturating_sub(1),
            Tab::Events => {
                self.event_selected = self.filtered_events_len().saturating_sub(1);
                self.events_following = true;
            }
            Tab::Jobs => {
                if self.cache_focus {
                    self.cache_selected = self.filtered_cache_len().saturating_sub(1);
                } else {
                    self.job_selected = self.filtered_jobs_len().saturating_sub(1);
                }
            }
            Tab::Alerts => self.alert_selected = self.filtered_alerts_len().saturating_sub(1),
            Tab::Api => self.api_log_selected = self.filtered_api_log_len().saturating_sub(1),
            Tab::Pulse => self.pulse_selected = self.pulse_minions_count().saturating_sub(1),
            _ => {}
        }
    }

    /// Navigate left in the grid (wraps).
    fn pulse_left(&mut self) {
        let count = self.pulse_minions_count();
        if count == 0 { return; }
        self.pulse_selected = if self.pulse_selected > 0 {
            self.pulse_selected - 1
        } else {
            count - 1
        };
    }

    /// Navigate right in the grid (wraps).
    fn pulse_right(&mut self) {
        let count = self.pulse_minions_count();
        if count == 0 { return; }
        self.pulse_selected = if self.pulse_selected + 1 < count {
            self.pulse_selected + 1
        } else {
            0
        };
    }

    /// Navigate up one row in the grid (wraps).
    fn pulse_up(&mut self) {
        let count = self.pulse_minions_count();
        if count == 0 { return; }
        let cols = self.pulse_grid_cols.max(1);
        if self.pulse_selected >= cols {
            self.pulse_selected -= cols;
        } else {
            // wrap to bottom row at same column
            let total_rows = (count + cols - 1) / cols;
            let last_row_start = (total_rows - 1) * cols;
            let col = self.pulse_selected;
            self.pulse_selected = (last_row_start + col).min(count - 1);
        }
    }

    /// Navigate down one row in the grid (wraps).
    fn pulse_down(&mut self) {
        let count = self.pulse_minions_count();
        if count == 0 { return; }
        let cols = self.pulse_grid_cols.max(1);
        if self.pulse_selected + cols < count {
            self.pulse_selected += cols;
        } else {
            // wrap to top row at same column
            let col = self.pulse_selected % cols;
            self.pulse_selected = col.min(count - 1);
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
/// Update per-unit restart/flap and memory-sample history from the latest
/// services snapshot. Called after each `list_services` refresh. Takes
/// disjoint field references rather than `&mut App` so it can be called
/// while other App fields (e.g. the locked agent_rx queue) are borrowed.
fn update_service_trends(trends: &mut HashMap<String, ServiceTrend>, services: &[ServiceHealth]) {
    let now = Instant::now();
    for svc in services {
        let trend = trends.entry(svc.unit.clone()).or_insert_with(|| ServiceTrend {
            last_uptime_seconds: svc.uptime_seconds,
            restart_events: VecDeque::new(),
            mem_samples: VecDeque::new(),
        });

        // Uptime dropping between polls (rather than growing monotonically)
        // means the unit restarted since the last sample.
        if svc.uptime_seconds < trend.last_uptime_seconds {
            trend.restart_events.push_back(now);
        }
        trend.last_uptime_seconds = svc.uptime_seconds;
        while trend.restart_events.front().is_some_and(|t| now.duration_since(*t) > SERVICE_TREND_WINDOW) {
            trend.restart_events.pop_front();
        }

        // Downsample memory history so a 24h window stays bounded regardless
        // of how often list_services is actually polled.
        let should_sample = trend.mem_samples.back()
            .is_none_or(|(t, _)| now.duration_since(*t) >= SERVICE_MEM_SAMPLE_MIN_INTERVAL);
        if should_sample && svc.memory_kb > 0 {
            trend.mem_samples.push_back((now, svc.memory_kb));
        }
        while trend.mem_samples.front().is_some_and(|(t, _)| now.duration_since(*t) > SERVICE_TREND_WINDOW) {
            trend.mem_samples.pop_front();
        }
    }
    // Drop history for units that no longer appear (service removed/renamed).
    let live: HashSet<&str> = services.iter().map(|s| s.unit.as_str()).collect();
    trends.retain(|k, _| live.contains(k.as_str()));
}

/// Least-squares slope (units of y per unit of x) over a set of points.
/// Returns 0.0 for fewer than 2 points or a degenerate (zero-variance) x range.
fn linreg_slope_per_sec(points: &[(f64, f64)]) -> f64 {
    let n = points.len() as f64;
    if n < 2.0 { return 0.0; }
    let sum_x: f64 = points.iter().map(|(x, _)| *x).sum();
    let sum_y: f64 = points.iter().map(|(_, y)| *y).sum();
    let sum_xy: f64 = points.iter().map(|(x, y)| x * y).sum();
    let sum_xx: f64 = points.iter().map(|(x, _)| x * x).sum();
    let denom = n * sum_xx - sum_x * sum_x;
    if denom.abs() < f64::EPSILON { return 0.0; }
    (n * sum_xy - sum_x * sum_y) / denom
}

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
        subprocess_secs: 0.0,
        postprocess_secs: 0.0,
    })
}

/// What `y`/`Y` copy for an event's body — matches whatever `draw_event_detail`
/// is currently rendering (raw JSON vs. the highstate-style pretty view)
/// rather than always copying JSON regardless of the on-screen mode.
fn event_body_as_copied_text(evt: &SaltEvent, pretty: bool, pretty_fetch: &crate::types::PrettyFetch) -> String {
    if pretty {
        if let crate::types::PrettyFetch::Ready(text) = pretty_fetch {
            return App::strip_ansi(text);
        }
        let lines = match crate::ui::detail_popup::job_return_wrapper(evt) {
            Some((id, ret, retcode)) => crate::ui::detail_popup::pretty_return_lines(&id, &ret, retcode),
            None => crate::ui::detail_popup::nested_return_lines(&evt.data, 0),
        };
        crate::ui::detail_popup::plain_text(&lines)
    } else {
        serde_json::to_string_pretty(&evt.data).unwrap_or_default()
    }
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
            } else if !obj.is_empty() && obj.values().all(|v| {
                v.as_object().is_some_and(|s| s.contains_key("result") || s.contains_key("__run_num__"))
            }) {
                // state.apply-style return: a dict keyed by state ID, each
                // value its own per-state result dict -- there's no
                // top-level `result`/`retcode` to find above, so without
                // this the whole job silently defaulted to "success" (rc=0)
                // even when every state inside had `result: false`.
                let any_failed = obj.values()
                    .any(|v| v.get("result").and_then(|r| r.as_bool()) == Some(false));
                return if any_failed { (1, false) } else { (0, true) };
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
        subprocess_secs: data["subprocess_secs"].as_f64().unwrap_or(0.0),
        postprocess_secs: data["postprocess_secs"].as_f64().unwrap_or(0.0),
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

/// Sorted oldest-first (ascending `start_epoch`) so the Job Cache panel
/// displays newest-at-bottom, matching Live Jobs' convention — regardless
/// of whatever order the agent's `jobs.list_jobs` call happened to return.
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
    result.sort_by(|a, b| a.start_epoch.total_cmp(&b.start_epoch));
    result
}

fn track_job_from_event(jobs: &mut Vec<JobEntry>, evt: &SaltEvent) {
    if evt.tag.ends_with("/new") {
        let jid = evt.data["jid"].as_str().unwrap_or("").to_string();
        if jid.is_empty() { return; }
        let fun = evt.data["fun"].as_str().unwrap_or("?").to_string();
        let tgt = evt.data["tgt"].as_str().unwrap_or("?").to_string();
        let minions = evt.data["minions"].as_array().map(|a| a.len()).unwrap_or(0);
        if let Some(job) = jobs.iter_mut().find(|j| j.jid == jid) {
            // A `/ret/` for this jid was processed before this `/new` --
            // most commonly because `/new` was the one evicted under
            // backlog pressure (see agent::MAX_QUEUE_LEN) -- and was
            // mistaken below for a one-off local/salt-call execution. Now
            // that the real dispatch info is in (better late than never),
            // correct it in place instead of leaving it stuck showing the
            // wrong target/count for the rest of the job's life -- whatever
            // it already recorded as returned/succeeded/failed is still
            // valid and is kept as-is.
            job.fun = fun;
            job.tgt = tgt;
            job.target_count = minions;
            job.local = false;
            return;
        }
        jobs.push(JobEntry {
            jid,
            fun,
            tgt,
            target_count: minions,
            returned: 0,
            succeeded: 0,
            failed: 0,
            age_seconds: 0,
            created_epoch: evt.ts,
            local: false,
            finalized: false,
        });
    } else if evt.tag.contains("/ret/") {
        let jid = evt.tag
            .strip_prefix("salt/job/")
            .and_then(|s| s.split("/ret/").next())
            .unwrap_or("");
        if jid.is_empty() { return; }
        // The event bus's own `success` (set by Salt itself from the real
        // return, not re-derived here) is authoritative -- unlike the
        // best-effort reconstruction `derive_success_retcode` has to do for
        // the cached `jobs.lookup_jid` return shown in Job Detail.
        let success = evt.data["success"].as_bool().unwrap_or(true);
        if let Some(job) = jobs.iter_mut().find(|j| j.jid == jid) {
            job.returned += 1;
            if success { job.succeeded += 1; } else { job.failed += 1; }
            // Guards against ever displaying more returns than the job was
            // supposedly targeted at (e.g. a dropped/never-received `/new`
            // leaving `target_count` stuck at the single-minion fallback
            // guess below) -- always widen to match reality rather than
            // show a target count smaller than what's actually come back.
            job.target_count = job.target_count.max(job.returned);
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
                succeeded: if success { 1 } else { 0 },
                failed: if success { 0 } else { 1 },
                age_seconds: 0,
                created_epoch: evt.ts,
                local: true,
                finalized: false,
            });
        }
    }
}

fn scroll_offset(selected: usize, visible: usize, total: usize) -> usize {
    if visible == 0 || total <= visible { return 0; }
    let scroll = if selected > visible / 2 { selected - visible / 2 } else { 0 };
    scroll.min(total.saturating_sub(visible))
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

/// Check if a MinionActivityItem matches a lowercase filter pattern — a
/// comma-separated list of terms is OR'd, same as every other filter in the
/// app (see `App::filter_terms`); `pat` is already lowercased once by the
/// caller since this runs per-item over a potentially large activity list.
fn matches_filter(item: &MinionActivityItem, pat: &str) -> bool {
    pat.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()).any(|term| match item {
        MinionActivityItem::Event(_, evt) => {
            evt.tag.to_lowercase().contains(term)
                || evt.data.to_string().to_lowercase().contains(term)
        }
        MinionActivityItem::CacheJob(job) => {
            job.jid.to_lowercase().contains(term)
                || job.fun.to_lowercase().contains(term)
        }
        MinionActivityItem::Alert(alert) => {
            alert.description.to_lowercase().contains(term)
                || alert.key.to_lowercase().contains(term)
        }
    })
}

