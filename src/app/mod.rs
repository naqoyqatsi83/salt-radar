mod clipboard;
mod config_editor;
mod keys;
pub(crate) mod line_edit;
mod messages;

pub use config_editor::{BrowserEntry, ConfigEditorState, ConfigFieldKind, PathBrowser, PathKind};

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
use crate::config::{load_config_or_fallback, AppConfig};
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
    /// `config` as last loaded from or saved to disk — what the live
    /// hotkey toggles (F4-F8, `f`, `x`) are diffed against on exit.
    persisted_config: AppConfig,
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
    /// Minion ID -> position in `minion_info`; rebuilt via `reindex_minions`
    /// whenever `minion_info` is replaced, reordered or shrunk, instead of on
    /// every drain (#104).
    minion_index: HashMap<String, usize>,

    // Events buffer
    pub events: VecDeque<SaltEvent>,
    pub events_since_last_tick: u64,
    pub event_breakdown: EventBreakdown,
    pub events_per_sec: f64,
    /// Incrementally maintained subset of `events` matching `applied_filter`.
    /// Rebuilt in full only when the filter itself changes (Enter/Esc/`/`);
    /// kept in sync on every ingested/evicted event in O(1) instead of
    /// re-scanning the whole buffer -- see `sync_filtered_events_cache` and
    /// `rebuild_filtered_events_cache`.
    filtered_events_cache: VecDeque<SaltEvent>,
    /// `applied_filter` resolved against the accepted keys, for matching
    /// each incoming event without re-resolving it.
    events_matcher: EventMatcher,

    // Live jobs
    pub jobs: Vec<JobEntry>,
    /// JID -> position in `jobs`. A busy master can hold tens of thousands
    /// of live jobs (60 min retention) and send hundreds of `/ret/` events a
    /// second, so a linear JID scan per event starved the UI thread (#104).
    jobs_index: HashMap<String, usize>,

    // Alerts
    pub alert_engine: AlertEngine,

    // UI state
    pub last_frame_size: Rect,
    pub detail_popup: Option<DetailPopup>,
    /// The Config Editor, a global overlay drawn over `detail_popup` so the
    /// popup underneath comes back unchanged when the editor closes.
    pub config_editor: Option<ConfigEditorState>,
    /// Tab where the current popup was opened (popup only shows on that tab)
    pub popup_tab: Option<Tab>,
    pub show_help: bool,
    /// Help overlay scroll offset, clamped while drawing.
    pub help_scroll: u16,
    /// Which page the help overlay shows.
    pub help_page: HelpPage,
    /// Help search, like Job Detail's output search: the query, whether it's
    /// being typed, the line of each match on the page shown (written back
    /// every render) and the occurrence `n`/`N` landed on.
    pub help_search: String,
    pub help_searching: bool,
    pub help_search_matches: Vec<usize>,
    pub help_search_focus: Option<usize>,
    /// Enter was pressed on a help search: the next draw, which has the
    /// matches for the final query, jumps to the first one.
    pub help_search_jump: bool,
    /// Where the help overlay's page labels were drawn: (row, first column,
    /// end column, page), for clicks.
    pub help_page_hits: Vec<(u16, u16, u16, HelpPage)>,
    /// The version that ran before this one, when salt-radar was upgraded
    /// since it last ran: the title bar points at "What's new" until it's
    /// been opened.
    pub upgraded_from: Option<String>,
    /// The `(color_theme, theme_background)` currently installed via
    /// `color::set_theme`, so `sync_theme` only reloads on a change.
    applied_theme: Option<(String, bool)>,

    /// The "Quit salt-radar?" prompt `q` opens (`confirm_quit`). A global
    /// overlay rather than a `detail_popup`, which may hold a popup parked on
    /// another tab.
    pub quit_confirm: bool,

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
    pub master_filter: String,
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
    /// Same tail-follow as `events_following`, for Live Jobs: while the
    /// cursor is on the newest job it moves to each new one as it arrives.
    /// Only in the default (unsorted, oldest-to-newest) order.
    pub jobs_following: bool,
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
    /// A redraw is waiting for `max_output_kbs` to allow it; the header says so.
    pub output_held: bool,
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

    /// The Master Config tab's configuration, fetched on first visit and on `r`.
    pub master_config: Option<MasterConfig>,
    pub master_config_loading: bool,
    pub master_config_error: Option<String>,
    pub master_selected: usize,
    /// Last scroll position of the setting popup, set when it's drawn.
    pub master_popup_scroll_max: usize,

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
    /// NDJSON command lines for the agent thread to forward to the agent.
    agent_cmd_tx: std::sync::mpsc::Sender<String>,
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

    // Network rate tracking
    prev_network_rx_bytes: u64,
    prev_network_tx_bytes: u64,
    prev_network_time: Option<Instant>,

    // System-wide CPU% tracking (from /proc/stat cumulative jiffies)
    prev_cpu_total_jiffies: u64,
    prev_cpu_idle_jiffies: u64,
    prev_cpu_time: Option<Instant>,

    // Mouse support
    pub needs_mouse_toggle: bool,
    last_click_at: Option<Instant>,
    last_click_col: u16,
    last_click_row: u16,

    // Sniffer
    pub sniffer_input_active: bool,
    pub sniffer_input_text: String,
    /// Cursor of whichever text input is active (see `line_edit`).
    pub input_cursor: Option<usize>,
    /// Why the sniffer input wasn't accepted (an invalid regex term).
    pub sniffer_input_error: Option<String>,
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
    pub job_detail_hits: JobDetailHits,
    /// Job Detail output search: the query, whether it's being typed, the
    /// line of each match (one entry per occurrence, written back every
    /// render, like the scroll max) and the occurrence `n`/`N` landed on.
    pub job_search: String,
    pub job_searching: bool,
    pub job_search_matches: Vec<usize>,
    pub job_search_focus: Option<usize>,
    /// Lines of the Job Detail output holding a failed state (`f`/`F`).
    pub job_fail_lines: Vec<usize>,
    /// The failed state's line the last `f`/`F` jump landed on, marked in the pane.
    pub job_focus_line: Option<usize>,
    /// Minion whose return Job Detail selects once its lookup arrives.
    pub job_detail_select_minion: Option<String>,
    /// When to look the open Job Detail's job up again: set when a return for
    /// it arrives, a moment later so a burst of returns costs one lookup.
    job_detail_refresh_at: Option<Instant>,
    /// Each minion's latest return outcomes, for its health summary and the
    /// Minions Fail% column. Kept apart from the event buffer so busy fleets
    /// don't evict a quiet minion's history.
    pub minion_returns: HashMap<String, MinionHealth>,
    pub jobs_failed_only: bool,
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
    /// Session-only column sort per table; a table with no entry shows its
    /// natural order. The Job Cache is sorted by the agent across the whole
    /// job cache, not just the page on screen.
    pub table_sorts: HashMap<TableId, SortSpec>,
    pub header_hits: Vec<HeaderHit>,
    pub cache_loading: bool,
    pub cache_loaded: bool,
    pub cache_load_error: Option<String>,
    pub cache_total_count: usize,
    pub cache_max_results: usize,
    pub cache_offset: usize,
    pub cache_loading_at: Option<Instant>,
    /// Request id of the latest `jobs_list` sent. Replies to anything older
    /// are dropped, so a slow unfiltered load finishing after a search was
    /// typed can't overwrite that search's results.
    cache_request_id: u64,
    /// Live count of jobs read so far during a `jobs_list` fetch, from the
    /// agent's incremental progress messages — `None` until the first one
    /// arrives (or if the agent fell back to the non-incremental path).
    pub cache_load_progress: Option<usize>,
}

#[derive(Clone, Debug)]
pub enum AgentMessage {
    /// `received` is when the reader thread got the line, so round-trip
    /// timings don't include however long it then waited in the queue for
    /// the main loop to drain it.
    Result { id: u64, cmd: String, ok: bool, data: serde_json::Value, received: Instant },
    Event { tag: String, data: serde_json::Value, ts: f64, received: Instant },
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

/// The Columns editor's full item list: shown items in their configured
/// order, then the hidden ones in their canonical order.
pub fn column_editor_items(value: &str, all: &[&'static str]) -> (Vec<&'static str>, usize) {
    let mut items: Vec<&'static str> = Vec::new();
    for key in value.split(',') {
        if let Some(k) = all.iter().find(|a| **a == key.trim()) {
            if !items.contains(k) { items.push(k); }
        }
    }
    let shown = items.len();
    let hidden: Vec<&'static str> = all.iter().filter(|a| !items.contains(a)).copied().collect();
    items.extend(hidden);
    (items, shown)
}

pub const ZMQ_LATENCY_SOURCES: &[&str] = &["agent_ipc", "job_ping", "master_stats"];
pub const ON_OFF: &[&str] = &["on", "off"];
pub const GRAPH_STYLES: &[&str] = &["dots", "bars"];
pub const API_STATS_AUTH_MODES: &[&str] = &["none", "plaintext", "pillar"];

fn on_off(b: bool) -> String {
    if b { "on" } else { "off" }.to_string()
}

/// Orders dotted version strings numerically part by part, so "3006.10"
/// sorts after "3006.9" instead of before it.
fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |s: &str| s.split('.').map(|p| (p.parse::<u64>().unwrap_or(u64::MAX), p.to_string())).collect::<Vec<_>>();
    key(a).cmp(&key(b))
}

fn column_keys(cols: &[Column]) -> String {
    cols.iter().map(|c| c.key()).collect::<Vec<_>>().join(",")
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

/// How many of a minion's latest returns the health summary covers.
pub const MINION_HEALTH_WINDOW: usize = 50;

/// Rows above the list in the minion events popup: title, health summary,
/// rule, column header, rule.
pub const MINION_EVENTS_HEADER_ROWS: u16 = 5;

/// Return times under this count as this for lag ratios: a `test.ping`
/// answered in 30 ms by most minions would otherwise rate one answering in
/// 120 ms 4x slow over jitter nobody would notice. A job where both the
/// minion and the median are under it says nothing about lag and isn't
/// rated at all, or a fleet running mostly fast jobs would read 1.0x for
/// every minion, however slow it is on the jobs that take time.
const LAG_MIN_SECS: f64 = 0.5;

#[derive(Debug, Default, PartialEq)]
pub struct MinionHealth {
    /// Outcomes of the latest returns, oldest first; `true` = success.
    pub recent: VecDeque<bool>,
    /// When the newest failure happened, and the job's function and args.
    pub last_failure: Option<(f64, String)>,
    /// Lag ratios of the latest jobs, oldest first: the minion's return
    /// time over the median of the job's minions (see `JobLag`).
    pub lag: VecDeque<f64>,
    /// Seconds from publish to return of the minion's latest `test.ping`.
    pub last_ping_secs: Option<f64>,
}

impl MinionHealth {
    fn record(&mut self, ok: bool, ts: f64, failure: Option<String>) {
        self.recent.push_back(ok);
        if self.recent.len() > MINION_HEALTH_WINDOW { self.recent.pop_front(); }
        if let Some(what) = failure { self.last_failure = Some((ts, what)); }
    }

    fn record_lag(&mut self, secs: f64, job_median: f64) {
        if secs < LAG_MIN_SECS && job_median < LAG_MIN_SECS { return; }
        self.lag.push_back(secs.max(LAG_MIN_SECS) / job_median.max(LAG_MIN_SECS));
        if self.lag.len() > MINION_HEALTH_WINDOW { self.lag.pop_front(); }
    }

    /// The lag ratio at percentile `p` (0.5 = median) of the latest jobs.
    pub fn lag_at(&self, p: f64) -> Option<f64> {
        let mut v: Vec<f64> = self.lag.iter().copied().collect();
        v.sort_by(f64::total_cmp);
        percentile(&v, p)
    }

    pub fn failed(&self) -> usize {
        self.recent.iter().filter(|ok| !**ok).count()
    }

    pub fn fail_ratio(&self) -> Option<f64> {
        (!self.recent.is_empty()).then(|| self.failed() as f64 / self.recent.len() as f64)
    }
}

#[derive(Debug, PartialEq)]
pub struct ReturnResult {
    pub success: bool,
    pub retcode: i64,
    pub counts: Option<StateCounts>,
}

/// A row of the Minions list.
#[derive(Clone, Copy)]
pub enum MinionRow<'a> {
    /// An accepted minion, with what the master knows about it.
    Minion(&'a MinionInfo),
    /// A key with no minion data: pending, rejected or denied (a denied key
    /// often duplicates an accepted ID), or accepted but never seen.
    Key(&'a KeyInfo),
    /// The collapsible "Timed out" section header.
    TimedOutHeader,
}

impl<'a> MinionRow<'a> {
    pub fn minion(&self) -> Option<&'a MinionInfo> {
        match *self {
            Self::Minion(m) => Some(m),
            _ => None,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Minion(m) => &m.id,
            Self::Key(k) => &k.id,
            Self::TimedOutHeader => "",
        }
    }

    pub fn key_status(&self) -> Option<KeyStatus> {
        match self {
            Self::Minion(_) => Some(KeyStatus::Accepted),
            Self::Key(k) => Some(k.status),
            Self::TimedOutHeader => None,
        }
    }

    fn timed_out(&self) -> bool {
        matches!(self, Self::Minion(m) if m.timed_out)
    }
}

/// The Minions list's rows, one per key (see `App::minion_entries`).
fn minion_list_rows<'a>(keys: &'a [KeyInfo], minions: &'a [MinionInfo], index: &HashMap<String, usize>) -> Vec<MinionRow<'a>> {
    if keys.is_empty() {
        return minions.iter().filter(|m| m.key_status == Some(KeyStatus::Accepted)).map(MinionRow::Minion).collect();
    }
    keys.iter().map(|k| match (k.status, index.get(&k.id)) {
        (KeyStatus::Accepted, Some(&i)) => MinionRow::Minion(&minions[i]),
        _ => MinionRow::Key(k),
    }).collect()
}

/// The JID of a `salt/job/<jid>/ret/<minion>` event.
pub fn return_event_jid(evt: &SaltEvent) -> Option<&str> {
    let mut parts = evt.tag.split('/');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("salt"), Some("job"), Some(jid), Some("ret")) => Some(jid),
        _ => None,
    }
}

/// Outcome of a job return event, by the same rule as every other view (see
/// `return_outcome`).
pub fn return_event_result(data: &serde_json::Value) -> ReturnResult {
    let ret = &data["return"];
    let (success, retcode) = return_outcome(data, ret);
    ReturnResult { success, retcode, counts: state_counts(ret) }
}

/// Whether a Job Cache entry targeted this minion. Only targets that can be
/// decided from the ID alone (glob, list) are matched; grain, pillar and
/// compound targets need the minion's data, so those jobs show up through
/// their return events instead.
pub fn cache_job_targets(job: &CacheJobEntry, minion_id: &str) -> bool {
    match job.tgt_type.as_str() {
        "list" => job.tgt.split(',').any(|t| t.trim() == minion_id),
        "glob" | "" => App::glob_match(minion_id, &job.tgt),
        _ => job.tgt == "*" || job.tgt == minion_id,
    }
}

/// What Job Detail's header shows about the job, taken from wherever it was
/// opened (Live Jobs, Job Cache, a minion's activity).
#[derive(Clone, Debug, Default)]
pub struct JobHeader {
    pub fun: String,
    pub tgt: String,
    pub args: String,
    pub test_run: bool,
    pub start_epoch: f64,
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
    JobDetail { jid: String, lookup: Option<JidLookup>, selected_return: Option<usize>, job: JobHeader, return_filter: String, return_is_filtering: bool, panel_focus: bool, content_scroll: usize, content_scroll_mode: bool },
    Alert(AlertEntry),
    AdminConfirm,
    MinionEvents { minion_id: String, selected: usize },
    ApiRequest(ApiLogEntry),
    /// A Master Config tab setting's full value; the index is into
    /// `master_config.settings`.
    MasterSetting { index: usize, scroll: usize },
}

impl App {
    pub fn new() -> Self {
        let config_path = crate::config::resolve_config_path();
        let config = load_config_or_fallback(&config_path);
        let config_secs = config.config_secs;
        let event_buffer_size = config.event_buffer_size;
        let agent_rx: Arc<Mutex<VecDeque<AgentMessage>>> = Arc::new(Mutex::new(VecDeque::new()));
        let agent_backlog_dropped = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let pid = std::process::id();

        // PID-based temp paths to avoid conflicts between multiple users or instances
        let copy_path = format!("/tmp/.salt-radar-copy-{}.txt", pid);
        let clip_log_path = format!("/tmp/.salt-radar-clip-{}.log", pid);
        let clip_debug_path = format!("/tmp/.salt-radar-clip-debug-{}.log", pid);

        // Spawn agent
        let rx = agent_rx.clone();
        let dropped = agent_backlog_dropped.clone();
        let agent_path = config.agent_path.clone();
        let python_bin = config.python_bin.clone();
        let master_config = config.master_config.clone();
        let (agent_cmd_tx, agent_cmd_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            crate::agent::agent_main(rx, dropped, &agent_path, &python_bin, &master_config, agent_cmd_rx);
        });

        Self {
            persisted_config: config.clone(),
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
            minion_index: HashMap::new(),

            events: VecDeque::with_capacity(event_buffer_size),
            events_since_last_tick: 0,
            event_breakdown: EventBreakdown::default(),
            events_per_sec: 0.0,
            filtered_events_cache: VecDeque::new(),
            events_matcher: EventMatcher::default(),

            jobs: Vec::new(),
            jobs_index: HashMap::new(),

            alert_engine: AlertEngine::new(),

            last_frame_size: Rect::default(),
            detail_popup: None,
            config_editor: None,
            popup_tab: None,
            show_help: false,
            help_scroll: 0,
            help_page: HelpPage::Keys,
            help_search: String::new(),
            help_searching: false,
            help_search_matches: Vec::new(),
            help_search_focus: None,
            help_search_jump: false,
            help_page_hits: Vec::new(),
            upgraded_from: crate::config::note_version_seen(),
            applied_theme: None,

            sniffer_input_active: false,
            sniffer_input_text: String::new(),
            input_cursor: None,
            sniffer_input_error: None,
            sniffer_dirty: false,
            sniffer: None,
            return_to_minion_events: None,
            return_to_minion_detail: None,
            event_detail_scroll_max: 0,
            job_detail_scroll_max: 0,
            job_detail_hits: JobDetailHits::default(),
            job_search: String::new(),
            job_searching: false,
            job_search_matches: Vec::new(),
            job_search_focus: None,
            job_fail_lines: Vec::new(),
            job_focus_line: None,
            job_detail_select_minion: None,
            job_detail_refresh_at: None,
            minion_returns: HashMap::new(),
            jobs_failed_only: false,
            minion_events_filter: String::new(),
            minion_events_is_filtering: false,
            hide_timed_out: false,
            timed_out_collapsed: false,
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
            table_sorts: HashMap::new(),
            header_hits: Vec::new(),
            cache_loading: false,
            cache_loaded: false,
            cache_load_error: None,
            cache_total_count: 0,
            cache_max_results: 500,
            cache_request_id: 0,
            cache_offset: 0,
            cache_loading_at: None,
            cache_load_progress: None,

            admin_mode: false,
            quit_confirm: false,
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
            master_filter: String::new(),
            applied_filter: String::new(),

            event_selected: 0,
            events_following: true,
            jobs_following: true,
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
            output_held: false,
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
            master_config: None,
            master_config_loading: false,
            master_config_error: None,
            master_selected: 0,
            master_popup_scroll_max: 0,
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


            prev_network_rx_bytes: 0,
            prev_network_tx_bytes: 0,
            prev_network_time: None,

            prev_cpu_total_jiffies: 0,
            prev_cpu_idle_jiffies: 0,
            prev_cpu_time: None,

            pid,
            agent_cmd_tx,
            copy_path,
            clip_log_path,
            clip_debug_path,
        }
    }

    fn reindex_minions(&mut self) {
        self.minion_index = self.minion_info
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id.clone(), i))
            .collect();
    }

    /// Delay between a new return for the open job and looking it up again.
    const JOB_DETAIL_REFRESH_DELAY: Duration = Duration::from_millis(1000);

    /// A return arrived for the job open in Job Detail: schedule a refresh.
    pub(crate) fn note_return_for_open_job(&mut self, tag: &str) {
        let Some(DetailPopup::JobDetail { ref jid, lookup: Some(_), .. }) = self.detail_popup else { return };
        if self.job_detail_refresh_at.is_none() && tag.split('/').nth(2) == Some(jid.as_str()) {
            self.job_detail_refresh_at = Some(Instant::now() + Self::JOB_DETAIL_REFRESH_DELAY);
        }
    }

    fn refresh_job_detail_if_due(&mut self) {
        let Some(due) = self.job_detail_refresh_at else { return };
        if Instant::now() < due || self.pending_jid_lookup.is_some() {
            return;
        }
        self.job_detail_refresh_at = None;
        let Some(DetailPopup::JobDetail { ref jid, lookup: Some(_), .. }) = self.detail_popup else { return };
        let jid = jid.clone();
        let req_id = self.send_jid_lookup(&jid);
        self.pending_jid_lookup = Some((req_id, jid));
        self.pending_jid_lookup_at = Some(Instant::now());
    }

    /// Fast poll between ticks
    pub fn fast_poll(&mut self) -> bool {
        let live_job = self.selected_live_job();
        self.drain_agent_messages();

        // Timeout for stuck jid_lookup (agent subprocess may hang). The
        // agent enforces lookup_timeout_secs on salt-run itself; the slack
        // covers shipping and parsing a result that can be tens of MB.
        let lookup_deadline = self.config.lookup_timeout_secs + 30;
        if self.pending_jid_lookup.is_some() {
            if let Some(started) = self.pending_jid_lookup_at {
                if started.elapsed().as_secs() > lookup_deadline {
                    self.pending_jid_lookup = None;
                    self.pending_jid_lookup_at = None;
                    if let Some(DetailPopup::JobDetail { ref mut lookup, .. }) = self.detail_popup {
                        *lookup = Some(JidLookup::failed(format!(
                            "No answer from salt-run jobs.lookup_jid within {}s", lookup_deadline)));
                    } else {
                        self.return_to_minion_events = None;
                        self.detail_popup = None;
                    }
                }
            }
        }

        self.refresh_job_detail_if_due();

        // Timeout for Event Detail's own pretty-view fetch -- falls back to
        // the client-side approximation rather than closing the popup, since
        // (unlike Job Detail) there's already a usable render without it.
        if self.pending_event_pretty.is_some() {
            if let Some(started) = self.pending_event_pretty_at {
                if started.elapsed().as_secs() > lookup_deadline {
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

        self.reselect_live_job(live_job);
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

    /// JID of the selected Live Jobs row while the cursor isn't following the
    /// newest job — what `reselect_live_job` puts the cursor back on after
    /// new jobs arrive or old ones age out, which would otherwise leave the
    /// row index pointing at a different job.
    fn selected_live_job(&self) -> Option<String> {
        if self.jobs_following && self.sort_of(TableId::LiveJobs).is_none() {
            return None;
        }
        self.get_filtered_job(self.job_selected).map(|j| j.jid.clone())
    }

    fn reselect_live_job(&mut self, jid: Option<String>) {
        let Some(jid) = jid else { return };
        if let Some(pos) = self.filtered_jobs().iter().position(|j| j.jid == jid) {
            self.job_selected = pos;
        }
    }

    /// Clamp all cursor selections to valid bounds
    fn clamp_selection(&mut self) {
        self.event_selected = self.event_selected
            .min(self.filtered_events_len().saturating_sub(1));
        self.job_selected = if self.jobs_following && self.sort_of(TableId::LiveJobs).is_none() {
            self.filtered_jobs_len().saturating_sub(1)
        } else {
            self.job_selected.min(self.filtered_jobs_len().saturating_sub(1))
        };
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
        self.master_selected = self.master_selected
            .min(self.filtered_master_settings().len().saturating_sub(1));
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
            Tab::Master => self.master_selected = self.master_selected
                .min(self.filtered_master_settings().len().saturating_sub(1)),
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

    /// One filter term against one field: a `/regex/` term's match, or a
    /// case-insensitive substring.
    fn matches_filter(text: &str, pat: &str) -> bool {
        if pat.is_empty() { return true; }
        match crate::sniff::regex_term(pat) {
            Some(re) => re.is_match(text),
            None => text.to_lowercase().contains(&pat.to_lowercase()),
        }
    }

    /// Splits a filter pattern into its comma-separated terms (trimmed, empty
    /// tokens dropped) — mirrors Salt's own list-targeting syntax
    /// (`salt -L 'web1,web2'`): a filter of `web1, web2` means "matches web1
    /// OR web2". A term wrapped in slashes is a regex, commas inside it
    /// included (see `crate::sniff::filter_terms`).
    fn filter_terms(pat: &str) -> Vec<&str> {
        crate::sniff::filter_terms(pat)
    }

    /// Whether a filter term is a glob: it isn't a `/regex/` and has `*`,
    /// `?` or a `[…]` class.
    pub(crate) fn is_glob(term: &str) -> bool {
        crate::sniff::regex_term(term).is_none() && term.contains(['*', '?', '['])
    }

    /// Case-insensitive wildcard match with Salt's glob semantics (Python's
    /// fnmatch): `*` = any run of characters, `?` = any single character,
    /// `[…]` = one of a set (`[12]`, `[1-3]`, `[!0-9]` / `[^0-9]` negated; an
    /// unclosed `[` is literal), anchored across the whole of `text`.
    /// Classic O(n*m) DP wildcard-matching table; hostnames are short enough
    /// that this is effectively instant.
    pub(crate) fn glob_match(text: &str, pattern: &str) -> bool {
        enum Tok { Star, Any, Class(Vec<(char, char)>, bool), Lit(char) }
        let p: Vec<char> = pattern.to_lowercase().chars().collect();
        let mut toks = Vec::new();
        let mut i = 0;
        while i < p.len() {
            toks.push(match p[i] {
                '*' => Tok::Star,
                '?' => Tok::Any,
                '[' => match glob_class(&p, i) {
                    Some(class) => {
                        toks.push(Tok::Class(class.ranges, class.negated));
                        i = class.next;
                        continue;
                    }
                    None => Tok::Lit('['),
                },
                c => Tok::Lit(c),
            });
            i += 1;
        }
        let t: Vec<char> = text.to_lowercase().chars().collect();
        let (tn, pn) = (t.len(), toks.len());
        let mut dp = vec![vec![false; pn + 1]; tn + 1];
        dp[0][0] = true;
        for j in 1..=pn {
            if matches!(toks[j - 1], Tok::Star) { dp[0][j] = dp[0][j - 1]; }
        }
        for i in 1..=tn {
            for j in 1..=pn {
                let c = t[i - 1];
                dp[i][j] = match &toks[j - 1] {
                    Tok::Star => dp[i - 1][j] || dp[i][j - 1],
                    Tok::Any => dp[i - 1][j - 1],
                    Tok::Class(ranges, negated) => dp[i - 1][j - 1] && ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&c)) != *negated,
                    Tok::Lit(l) => dp[i - 1][j - 1] && *l == c,
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
        if let Some(re) = crate::sniff::regex_term(term) {
            re.is_match(host)
        } else if Self::is_glob(term) {
            Self::glob_match(host, term)
        } else {
            Self::matches_filter(host, term)
        }
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

    /// The Live Jobs filter as a predicate. Terms are resolved, and whether
    /// each names a known minion settled, once per pass rather than per job:
    /// per job, the latter cost a scan of every accepted minion (#165).
    fn job_matcher<'a>(pat: &'a str, known_minions: &HashSet<String>) -> impl Fn(&JobEntry) -> bool + 'a {
        let terms: Vec<(FilterTerm, bool)> = Self::filter_terms(pat).into_iter()
            .map(|term| FilterTerm::with_minion_check(term, known_minions))
            .collect();
        move |job| terms.is_empty() || terms.iter().any(|(term, names_minion)| Self::matches_job_term(job, term, *names_minion))
    }

    fn matches_job_term(job: &JobEntry, term: &FilterTerm, names_minion: bool) -> bool {
        term.text(&job.jid)
            || term.text(&job.fun)
            || term.text(&job.args)
            || term.host(&job.tgt)
            // Show jobs targeting "*" when searching for a known minion
            || (names_minion && job.tgt == "*")
    }

    fn matches_alert(alert: &AlertEntry, pat: &str) -> bool {
        let terms = Self::filter_terms(pat);
        if terms.is_empty() { return true; }
        terms.iter().any(|term| {
            Self::matches_filter(alert.rule, term) || Self::matches_filter(&alert.description, term)
        })
    }

    /// The Minions filter's rules, for the separate `FleetMinion`
    /// type used by the Fleet Overview scan — lets the Minions-tab filter also
    /// scope the OS/version/arch breakdown, not just the list.
    pub fn fleet_minion_matcher(pat: &str) -> impl Fn(&crate::types::FleetMinion) -> bool {
        let terms: Vec<FilterTerm> = Self::filter_terms(pat).into_iter().map(FilterTerm::new).collect();
        move |m| terms.is_empty() || terms.iter().any(|t| {
            t.host(&m.id) || t.text(&m.osfinger) || t.text(&m.osrelease) || t.text(&m.saltversion)
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
            Tab::Master => &self.master_filter,
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
            Tab::Master => self.master_filter = text.to_string(),
            _ => {}
        }
    }

    /// The known minions a sniffer term names: an exact minion ID, or every
    /// minion a glob matches.
    pub(crate) fn minions_for_term(&self, term: &str) -> Vec<String> {
        let known = self.accepted_ids();
        if Self::is_glob(term) {
            let mut ids: Vec<String> = known.iter().filter(|m| Self::glob_match(m, term)).cloned().collect();
            ids.sort();
            ids
        } else if known.contains(term) {
            vec![term.to_string()]
        } else {
            Vec::new()
        }
    }

    /// The Events filter's terms for highlighting, `filter_text` while it's
    /// being typed and `applied_filter` once confirmed. A glob never
    /// literally appears in rendered text (there's no "debian12-*" substring
    /// to highlight), so each glob carries the known minions it matches,
    /// sorted so their order — and the colors — stay put as the minion list
    /// refreshes.
    pub fn events_filter_terms(&self) -> Vec<HighlightTerm> {
        let pat: &str = if self.is_filtering { &self.filter_text } else { &self.applied_filter };
        Self::filter_terms(pat).into_iter().map(|term| {
            let mut ids: Vec<String> = if Self::is_glob(term) {
                self.accepted_ids().iter().filter(|m| Self::glob_match(m, term)).cloned().collect()
            } else {
                Vec::new()
            };
            ids.sort();
            HighlightTerm { term: term.to_string(), ids }
        }).collect()
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
        self.refresh_events_matcher();
        if self.applied_filter.is_empty() {
            self.filtered_events_cache.clear();
            return;
        }
        let matcher = &self.events_matcher;
        self.filtered_events_cache = self.events.iter()
            .filter(|e| matcher.matches(e))
            .cloned()
            .collect();
    }

    /// Re-resolves the Events filter against the current filter and keys.
    pub(crate) fn refresh_events_matcher(&mut self) {
        self.events_matcher = EventMatcher::new(&self.applied_filter, &self.accepted_id_set);
    }

    /// Keeps `filtered_events_cache` consistent with a single event
    /// ingested/evicted from `self.events`, in O(1) amortized instead of
    /// re-scanning the buffer. Call this once per event, immediately after
    /// pushing `new_evt` and (if `evicted` is set) popping it off the front.
    ///
    /// Takes its fields individually (rather than `&mut self`) so callers
    /// already holding an unrelated borrow through `self` can still call it.
    fn sync_filtered_events_cache(
        cache: &mut VecDeque<SaltEvent>,
        pat: &str,
        matcher: &EventMatcher,
        new_evt: &SaltEvent,
        evicted: Option<&SaltEvent>,
    ) {
        if pat.is_empty() {
            return;
        }
        if let Some(evicted_evt) = evicted {
            if matcher.matches(evicted_evt) {
                cache.pop_front();
            }
        }
        if matcher.matches(new_evt) {
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
    /// Filtered by the popup's `/` filter, so drawing, keys and clicks all see
    /// the same rows.
    pub fn minion_activity(&self, minion_id: &str) -> Vec<MinionActivityItem> {
        let mut items: Vec<MinionActivityItem> = Vec::new();
        let mut returned_jids: HashSet<String> = HashSet::new();

        for (idx, evt) in self.events_for_minion(minion_id) {
            if let Some(jid) = return_event_jid(&evt) {
                returned_jids.insert(jid.to_string());
            }
            items.push(MinionActivityItem::Event(idx, evt));
        }

        // A job whose return is already listed would only repeat it, minus the result.
        for job in &self.jobs_cache {
            if !returned_jids.contains(&job.jid) && cache_job_targets(job, minion_id) {
                items.push(MinionActivityItem::CacheJob(job.clone()));
            }
        }

        // Alerts mentioning this minion
        for alert in &self.alert_engine.alerts {
            if alert.description.contains(minion_id) || alert.details.iter().any(|d| d.contains(minion_id)) {
                items.push(MinionActivityItem::Alert(alert.clone()));
            }
        }

        let terms = Self::filter_terms(&self.minion_events_filter);
        if !terms.is_empty() {
            items.retain(|item| matches_activity_item(item, &terms));
        }

        // Sort by timestamp descending (alerts with ts=0.0 fall to end)
        items.sort_by(|a, b| b.ts().partial_cmp(&a.ts()).unwrap_or(std::cmp::Ordering::Equal));
        items
    }

    pub fn minion_health(&self, minion_id: &str) -> Option<&MinionHealth> {
        self.minion_returns.get(minion_id)
    }

    /// Share of the minion's latest returns that failed, 0.0–1.0.
    pub fn minion_fail_ratio(&self, minion_id: &str) -> Option<f64> {
        self.minion_health(minion_id).and_then(MinionHealth::fail_ratio)
    }

    fn record_minion_return(&mut self, minion_id: &str, ts: f64, data: &serde_json::Value) {
        let ok = return_event_result(data).success;
        let failure = (!ok).then(|| {
            let fun = data["fun"].as_str().unwrap_or("?");
            let (args, _) = format_job_args(fun, &data["fun_args"]);
            format!("{} {}", fun, args).trim_end().to_string()
        });
        self.minion_returns.entry(minion_id.to_string()).or_default().record(ok, ts, failure);
    }

    pub(crate) fn record_return_timing(&mut self, t: ReturnTiming, fun: &str) {
        let h = self.minion_returns.entry(t.minion).or_default();
        if fun == "test.ping" { h.last_ping_secs = Some(t.secs); }
        if let Some(median) = t.late_median { h.record_lag(t.secs, median); }
    }

    /// Settles the lag of jobs that are complete or past
    /// `job_straggler_secs`: fixes their median and rates the minions that
    /// returned so far against it.
    fn settle_job_lags(&mut self) {
        for job in self.jobs.iter_mut() {
            if let Some((times, median)) = settle_job_lag(job, self.config.job_straggler_secs) {
                for (minion, secs) in times {
                    self.minion_returns.entry(minion).or_default().record_lag(secs, median);
                }
            }
        }
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
        let path = parts.next().and_then(|p| p.split('?').next()).unwrap_or("?").to_string();
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
        if pat.is_empty() && !self.jobs_failed_only {
            return self.jobs.len();
        }
        self.jobs.iter().filter(self.job_shown()).count()
    }

    fn job_shown(&self) -> impl Fn(&&JobEntry) -> bool + '_ {
        let matches = Self::job_matcher(self.filter_text_for(Tab::Jobs), self.accepted_ids());
        move |job| (!self.jobs_failed_only || job.failed > 0) && matches(job)
    }

    /// Live Jobs in display order: filtered, then sorted. Unsorted, they
    /// run oldest to newest by JID (tail-style, like Events) rather than in
    /// event-arrival order, which a late or reordered `/new` can scramble.
    pub fn filtered_jobs(&self) -> Vec<&JobEntry> {
        let mut jobs: Vec<&JobEntry> = self.jobs.iter().filter(self.job_shown()).collect();
        jobs.sort_by(|a, b| a.jid.cmp(&b.jid));
        if let Some((col, desc)) = self.sort_of(TableId::LiveJobs) {
            jobs.sort_by(|a, b| {
                let ord = match col {
                    Column::Jid => a.jid.cmp(&b.jid),
                    Column::Fun => a.fun.cmp(&b.fun),
                    Column::Args => a.args.cmp(&b.args).then(a.test_run.cmp(&b.test_run)),
                    Column::Target => a.tgt.cmp(&b.tgt),
                    Column::Source => a.local.cmp(&b.local),
                    Column::Targeted => a.target_count.cmp(&b.target_count),
                    Column::Ok => a.succeeded.cmp(&b.succeeded),
                    Column::Fail => a.failed.cmp(&b.failed),
                    Column::Wait => a.target_count.saturating_sub(a.returned).cmp(&b.target_count.saturating_sub(b.returned)),
                    _ => a.created_epoch.total_cmp(&b.created_epoch),
                };
                if desc { ord.reverse() } else { ord }
            });
        }
        jobs
    }

    pub fn get_filtered_job(&self, index: usize) -> Option<&JobEntry> {
        self.filtered_jobs().get(index).copied()
    }

    pub fn columns(&self, table: TableId) -> Vec<Column> {
        Column::parse_list(self.config.columns_for(table), table)
    }

    pub fn sort_of(&self, table: TableId) -> Option<SortSpec> {
        self.table_sorts.get(&table).copied()
    }

    /// The configurable table the keyboard is on, if any.
    fn current_table(&self) -> Option<TableId> {
        match self.current_tab {
            Tab::Jobs if self.cache_focus => Some(TableId::JobCache),
            Tab::Jobs => Some(TableId::LiveJobs),
            Tab::Minions => Some(TableId::Minions),
            Tab::Events => Some(TableId::Events),
            Tab::Alerts => Some(TableId::Alerts),
            Tab::Api => Some(TableId::ApiLog),
            Tab::Dashboard | Tab::Pulse | Tab::Master => None,
        }
    }

    /// Header click: ascending, then descending, then back to natural order.
    fn click_sort(sort: Option<SortSpec>, col: Column) -> Option<SortSpec> {
        match sort {
            Some((c, false)) if c == col => Some((col, true)),
            Some((c, true)) if c == col => None,
            _ => Some((col, false)),
        }
    }

    /// `o`: step the sort to the next shown column (ascending), and past the
    /// last one back to natural order.
    fn next_sort(sort: Option<SortSpec>, cols: &[Column]) -> Option<SortSpec> {
        let next = match sort.and_then(|(c, _)| cols.iter().position(|x| *x == c)) {
            Some(i) => cols.get(i + 1),
            None => cols.first(),
        };
        next.map(|c| (*c, false))
    }

    fn set_sort(&mut self, table: TableId, sort: Option<SortSpec>) {
        if !table.sortable() || self.sort_of(table) == sort { return; }
        match sort {
            Some(s) => { self.table_sorts.insert(table, s); }
            None => { self.table_sorts.remove(&table); }
        }
        match table {
            TableId::JobCache => {
                self.cache_selected = 0;
                let ft = self.cache_server_filter.clone();
                self.send_cache_jobs_list(if ft.is_empty() { None } else { Some(&ft) }, true);
            }
            TableId::LiveJobs => self.job_selected = 0,
            TableId::Minions => self.minion_selected = 0,
            TableId::Alerts => self.alert_selected = 0,
            TableId::ApiLog => self.api_log_selected = 0,
            TableId::Events => {}
        }
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

    /// The Master Config tab's settings in display order, as indexes into
    /// `master_config.settings`: those set in files first, in load order
    /// (file, then line), then derived and default ones by name.
    pub fn filtered_master_settings(&self) -> Vec<usize> {
        let Some(mc) = &self.master_config else { return Vec::new() };
        let terms = Self::filter_terms(self.filter_text_for(Tab::Master));
        let mut rows: Vec<usize> = (0..mc.settings.len())
            .filter(|&i| {
                let s = &mc.settings[i];
                terms.is_empty() || terms.iter().any(|t| {
                    Self::matches_filter(&s.key, t)
                        || Self::matches_filter(&s.line, t)
                        || Self::matches_filter(s.kind.label(), t)
                        || s.about.as_deref().is_some_and(|a| Self::matches_filter(a, t))
                        || (!s.unknown_nested.is_empty() && Self::matches_filter(SettingKind::Unknown.label(), t))
                        || (s.restart_pending(mc) && Self::matches_filter("restart", t))
                        || s.sources.iter().any(|src| mc.files.get(src.file).is_some_and(|file| Self::matches_filter(&file.path, t)))
                })
            })
            .collect();
        let set_in = |i: usize| mc.settings[i].sources.first().map_or((usize::MAX, 0), |src| (src.file, src.start));
        rows.sort_by_key(|&i| {
            let s = &mc.settings[i];
            let group = if s.sources.is_empty() { s.kind } else { SettingKind::Explicit };
            (group, set_in(i), s.key.clone())
        });
        rows
    }

    /// Alerts in display order: filtered, then sorted.
    pub fn filtered_alerts(&self) -> Vec<&AlertEntry> {
        let pat = self.filter_text_for(Tab::Alerts);
        let mut alerts: Vec<&AlertEntry> = self.alert_engine.alerts.iter()
            .filter(|a| pat.is_empty() || Self::matches_alert(a, pat))
            .collect();
        if let Some((col, desc)) = self.sort_of(TableId::Alerts) {
            let rank = |s: AlertSeverity| match s {
                AlertSeverity::Info => 0,
                AlertSeverity::Warn => 1,
                AlertSeverity::Crit => 2,
            };
            alerts.sort_by(|a, b| {
                let ord = match col {
                    Column::Severity => rank(a.severity).cmp(&rank(b.severity)),
                    Column::Rule => a.rule.cmp(b.rule),
                    Column::Description => a.description.cmp(&b.description),
                    // Shortest-active first, matching the Since column's text.
                    Column::Since => b.active_since.cmp(&a.active_since),
                    _ => a.count.cmp(&b.count),
                };
                if desc { ord.reverse() } else { ord }
            });
        }
        alerts
    }

    pub fn get_filtered_alert(&self, index: usize) -> Option<&AlertEntry> {
        self.filtered_alerts().get(index).copied()
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

    /// API request log in display order: filtered, then sorted.
    pub fn filtered_api_log(&self) -> Vec<&ApiLogEntry> {
        let pat = self.filter_text_for(Tab::Api);
        let mut entries: Vec<&ApiLogEntry> = self.api_requests_log.iter()
            .filter(|e| self.api_log_entry_visible(e) && Self::matches_api_log_entry(e, pat))
            .collect();
        if let Some((col, desc)) = self.sort_of(TableId::ApiLog) {
            entries.sort_by(|a, b| {
                let ord = match col {
                    // Youngest first, matching the Age column's text.
                    Column::Age => b.received_at.cmp(&a.received_at),
                    Column::Method => a.method.cmp(&b.method),
                    Column::Client => a.client.cmp(&b.client),
                    Column::Path => a.path.cmp(&b.path),
                    Column::Status => a.response_status.cmp(&b.response_status),
                    _ => a.processing_time_secs.total_cmp(&b.processing_time_secs),
                };
                if desc { ord.reverse() } else { ord }
            });
        }
        entries
    }

    pub fn get_filtered_api_log(&self, index: usize) -> Option<&ApiLogEntry> {
        self.filtered_api_log().get(index).copied()
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
        stats.sort_by_key(|s| std::cmp::Reverse(s.count));
        stats
    }

    pub fn pulse_minions_count(&self) -> usize {
        let accepted = &self.accepted_id_set;
        self.minion_info.iter()
            .filter(|m| accepted.contains(m.id.as_str()) || m.key_status == Some(KeyStatus::Accepted))
            .count()
    }

    /// Minions list entries matching its filter, split into responsive rows
    /// and timed-out minions. One row per key, so the list adds up to the
    /// Dashboard's key counts: an accepted key shows its minion (or just the
    /// key while the master knows nothing more), any other key its status.
    /// Without readable keys, the accepted minions the master knows.
    fn minion_entries(&self) -> (Vec<MinionRow<'_>>, Vec<MinionRow<'_>>) {
        let matches = Self::minion_row_matcher(self.filter_text_for(Tab::Minions));
        minion_list_rows(&self.keys, &self.minion_info, &self.minion_index).into_iter()
            .filter(|r| matches(r))
            .partition(|r| !r.timed_out())
    }

    /// The Minions filter: ID, key status and, for minions, OS and version.
    fn minion_row_matcher(pat: &str) -> impl Fn(&MinionRow<'_>) -> bool {
        let terms: Vec<FilterTerm> = Self::filter_terms(pat).into_iter().map(FilterTerm::new).collect();
        move |r| terms.is_empty() || terms.iter().any(|t| {
            t.host(r.id())
                || r.key_status().is_some_and(|ks| t.text(ks.label()))
                || matches!(r, MinionRow::Minion(m) if t.text(&m.os) || t.text(&m.osrelease) || t.text(&m.salt_version))
        })
    }

    /// Rows the Minions list shows, the "Timed out" header included.
    pub fn filtered_minions_len(&self) -> usize {
        let (live, timed_out) = self.minion_entries();
        if self.hide_timed_out || timed_out.is_empty() {
            live.len()
        } else {
            live.len() + 1 + if self.timed_out_collapsed { 0 } else { timed_out.len() }
        }
    }

    /// Keys and minions matching the Minions filter, collapsed or hidden
    /// timed-out ones included: the count the list's title shows.
    pub fn minion_entry_count(&self) -> usize {
        let (live, timed_out) = self.minion_entries();
        live.len() + timed_out.len()
    }

    /// Minions list rows in display order: responsive rows, then (unless
    /// hidden) the collapsible "Timed out" header and, unless collapsed, the
    /// timed-out minions -- each group sorted on its own so a sort never
    /// mixes timed-out minions in with responsive ones. An ID with two keys
    /// (accepted and denied) gets two rows, next to each other by default.
    pub fn minion_rows(&self) -> Vec<MinionRow<'_>> {
        let (mut live, mut timed_out) = self.minion_entries();
        let rank = |r: &MinionRow| match r.key_status() {
            Some(KeyStatus::Accepted) => 0,
            Some(KeyStatus::Pending) => 1,
            Some(KeyStatus::Rejected) => 2,
            Some(KeyStatus::Denied) => 3,
            None => 4,
        };
        let sort = self.sort_of(TableId::Minions);
        let cmp = |a: &MinionRow, b: &MinionRow| {
            let by_id = a.id().cmp(b.id()).then(rank(a).cmp(&rank(b)));
            let Some((col, desc)) = sort else { return by_id };
            let ord = match (col, a.minion(), b.minion()) {
                (Column::KeyStatus, _, _) => rank(a).cmp(&rank(b)),
                // Key-only rows have no minion data; they sort as empty.
                (Column::Os, a, b) => a.map(|m| m.os_label()).cmp(&b.map(|m| m.os_label())),
                (Column::SaltVersion, a, b) => version_cmp(a.map_or("", |m| &m.salt_version), b.map_or("", |m| &m.salt_version)),
                (Column::Proto, a, b) => a.and_then(|m| auth_protocol(&m.salt_version)).cmp(&b.and_then(|m| auth_protocol(&m.salt_version))),
                (Column::Arch, a, b) => a.map(|m| &m.cpuarch).cmp(&b.map(|m| &m.cpuarch)),
                // Most recently seen first, matching the Seen column's text.
                (Column::LastSeen, a, b) => b.map_or(0, |m| m.last_seen_epoch).cmp(&a.map_or(0, |m| m.last_seen_epoch)),
                // Worst first; rows with no returns yet go last.
                (Column::FailPct, a, b) => {
                    let ratio = |m: Option<&MinionInfo>| m.and_then(|m| self.minion_fail_ratio(&m.id)).unwrap_or(-1.0);
                    ratio(b).total_cmp(&ratio(a))
                }
                (Column::Lag, a, b) => {
                    let lag = |m: Option<&MinionInfo>| m.and_then(|m| self.minion_health(&m.id)?.lag_at(0.5)).unwrap_or(-1.0);
                    lag(b).total_cmp(&lag(a))
                }
                (Column::Ping, a, b) => {
                    let ping = |m: Option<&MinionInfo>| m.and_then(|m| self.minion_health(&m.id)?.last_ping_secs).unwrap_or(-1.0);
                    ping(b).total_cmp(&ping(a))
                }
                _ => std::cmp::Ordering::Equal,
            };
            let ord = if desc { ord.reverse() } else { ord };
            ord.then(by_id)
        };
        live.sort_by(cmp);
        timed_out.sort_by(cmp);
        if !self.hide_timed_out && !timed_out.is_empty() {
            live.push(MinionRow::TimedOutHeader);
            if !self.timed_out_collapsed {
                live.extend(timed_out);
            }
        }
        live
    }

    /// The minion on a Minions list row; `None` for the header and for keys
    /// the master has no minion data for.
    pub fn get_filtered_minion(&self, index: usize) -> Option<&MinionInfo> {
        self.minion_rows().get(index).and_then(MinionRow::minion)
    }

    /// Timed-out minions matching the list filter, whether or not their
    /// section is collapsed -- the count its header shows.
    pub fn filtered_timed_out_count(&self) -> usize {
        self.minion_entries().1.len()
    }

    /// Strip ANSI escape sequences from a string.
    pub fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                // Consume until we hit 'm' (end of CSI sequence)
                for n in chars.by_ref() {
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
                    30..=37 => *style = style.fg(ansi_theme_color(params[i] - 30, false)),
                    38 => {
                        // Extended foreground: 38;5;N or 38;2;R;G;B
                        if i + 1 < params.len() {
                            match params[i + 1] {
                                5 if i + 2 < params.len() => {
                                    let idx = params[i + 2];
                                    *style = style.fg(if idx < 16 { ansi_theme_color(idx % 8, false) } else { Color::Indexed(idx as u8) });
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
                    39 => style.fg = default_style.fg,
                    40..=47 => *style = style.bg(ansi_theme_color(params[i] - 40, true)),
                    48 => {
                        // Extended background: 48;5;N or 48;2;R;G;B
                        if i + 1 < params.len() {
                            match params[i + 1] {
                                5 if i + 2 < params.len() => {
                                    let idx = params[i + 2];
                                    *style = style.bg(if idx < 16 { ansi_theme_color(idx % 8, true) } else { Color::Indexed(idx as u8) });
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
                    49 => style.bg = default_style.bg,
                    90..=97 => *style = style.fg(ansi_theme_color(params[i] - 90, false)),
                    100..=107 => *style = style.bg(ansi_theme_color(params[i] - 100, true)),
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

    /// Every return in a job lookup, concatenated (Salt outputter format,
    /// same per-return text `y`/`s` use) — for jobs with too many targeted
    /// minions to review one at a time via the left panel.
    fn format_all_returns_console(jid: &str, l: &crate::types::JidLookup) -> String {
        let mut out = format!(
            "JID: {}\nFunction: {}\nTarget: {}\n{} returns\n\n",
            jid, l.fun, l.tgt, l.returns.len(),
        );
        for ret in &l.returns {
            out.push_str(&"=".repeat(60));
            out.push('\n');
            if !ret.formatted_text.is_empty() {
                out.push_str(&Self::strip_ansi(&ret.formatted_text));
            } else {
                out.push_str(&Self::format_return_as_console(&ret.minion_id, &ret.full_return));
            }
            out.push_str("\n\n");
        }
        out
    }

    /// Every return in a job lookup as a single JSON object keyed by minion
    /// id — the `A` (all, JSON) counterpart of `format_all_returns_console`.
    fn format_all_returns_json(l: &crate::types::JidLookup) -> String {
        let map: serde_json::Map<String, serde_json::Value> = l.returns.iter()
            .map(|r| (r.minion_id.clone(), r.full_return.clone()))
            .collect();
        serde_json::to_string_pretty(&serde_json::Value::Object(map)).unwrap_or_default()
    }

    /// Clean up per-instance temp files. Called on exit.
    pub fn cleanup(&self) {
        let _ = std::fs::remove_file(&self.copy_path);
        let _ = std::fs::remove_file(&self.clip_log_path);
        let _ = std::fs::remove_file(&self.clip_debug_path);
    }

    /// Saves the hotkey toggles (F2 mouse, F4-F8 graphs, `m` graph style,
    /// `-`/`+` refresh speed, Minions `f`, API `x`, `Ctrl+←`/`→` splits) the moment they change, so the next start looks
    /// the same even when the session doesn't end with a clean quit (the
    /// terminal window closed, the SSH session dropped). Merged into the
    /// file as it is now instead of writing back the whole in-memory config,
    /// which would clobber anything saved to it meanwhile, by hand or from
    /// another salt-radar instance.
    pub(crate) fn save_toggles(&mut self) {
        let mut on_disk = load_config_or_fallback(&self.config_path);
        let (cur, base) = (&self.config, &self.persisted_config);
        let mut changed = false;
        macro_rules! keep_toggled {
            ($($field:ident),*) => {$(
                if cur.$field != base.$field {
                    on_disk.$field = cur.$field.clone();
                    changed = true;
                }
            )*};
        }
        keep_toggled!(show_cpu_graph, show_mem_graph, show_zmq_graph, show_events_graph, show_activity_graph,
            graph_style, mouse, tick_ms, minions_follow_filter, api_hide_stats_polls,
            minions_split_pct, events_split_pct, jobs_split_pct, api_split_pct, pulse_split_pct, master_split_pct);
        if !changed {
            return;
        }
        match crate::config::write_config(&self.config_path, &on_disk) {
            Ok(()) => self.persisted_config = self.config.clone(),
            Err(e) => self.clipboard_msg = Some((format!("Settings not saved: {}", e), Instant::now())),
        }
    }

    /// Called every tick (1s)
    pub fn update(&mut self) {
        // Drain agent messages (must happen every tick)
        let live_job = self.selected_live_job();
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
                    self.finish_fleet_scan();
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
            self.reindex_minions();
        }

        // Refresh collectors at interval, not every tick
        if self.config_snapshot.is_some() {
            let interval = self.config.dashboard_secs.max(1);
            if self.tick_count.is_multiple_of(interval) {
                self.send_agent_cmd("minion_cache_list", None);
                self.send_agent_cmd("list_keys", None);
                self.send_agent_cmd("list_processes", None);
                self.send_agent_cmd("list_services", None);
                self.send_agent_cmd("get_system_info", None);
                if self.processes.iter().any(|p| p.role == SaltProcessRole::Api) {
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
        self.events_last_sec_count += self.events_since_last_tick;
        self.events_since_last_tick = 0;

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
            if !self.master_config_loading {
                self.request_master_config();
            }
        }

        // Alive detection every 5 ticks
        if self.tick_count.is_multiple_of(5) {
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
                if self.job_ping_pending.is_none() && self.tick_count.is_multiple_of(15) {
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
        let recent_events: Vec<&SaltEvent> = self.events.iter().rev().take(500).collect();
        let worker_busy_pct = self.worker_busy_pct();
        let master = self.config_snapshot.as_ref();
        let rejected_config_files: Vec<String> = self.master_config.iter()
            .flat_map(|mc| &mc.files)
            .filter_map(|f| {
                let error = f.error.as_ref()?;
                Some(if f.error_line > 0 { format!("{}: {} (line {})", f.path, error, f.error_line) } else { format!("{}: {}", f.path, error) })
            })
            .collect();
        self.alert_engine.tick(
            &self.minion_info,
            master.map_or("", |s| s.salt_version.as_str()),
            master.map_or(0, |s| s.minimum_auth_version),
            &rejected_config_files,
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

        self.settle_job_lags();

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
        if kept_jobs.len() != self.jobs_index.len() {
            self.jobs_index = kept_jobs.iter().enumerate().map(|(i, j)| (j.jid.clone(), i)).collect();
        }
        self.jobs = kept_jobs;

        if self.current_tab == Tab::Master && self.master_config.is_none() && !self.master_config_loading && self.master_config_error.is_none() {
            self.request_master_config();
        }

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
        self.reselect_live_job(live_job);
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
            let before = self.minion_info.len();
            self.minion_info.retain(|m| accepted.contains(m.id.as_str()));
            if self.minion_info.len() != before {
                self.reindex_minions();
            }
        }
    }

    /// Map a column position on the tab bar (row 1) to a tab.
    /// `None` for a column that doesn't land on any tab's label (the gaps
    /// between them, or anything past the last one) — callers should treat
    /// that as "not a tab bar click" rather than defaulting to some tab.
    fn tab_at_col(col: u16) -> Option<Tab> {
        // Laid out like draw_title_frame: after the frame's left border,
        // each label, then a separator.
        let col = col as usize;
        let mut start = 1;
        for tab in Tab::ORDER {
            let end = start + tab.bar_label().chars().count();
            if (start..end).contains(&col) {
                return Some(tab);
            }
            start = end + crate::ui::TAB_SEPARATOR.chars().count();
        }
        None
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
            let total_rows = count.div_ceil(cols);
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

pub(crate) fn parse_master_config(data: &serde_json::Value) -> MasterConfig {
    let text = |v: &serde_json::Value| v.as_str().unwrap_or("").to_string();
    let files = data["files"].as_array().map(|a| a.iter().map(|f| ConfigFile {
        path: text(&f["path"]),
        text: text(&f["text"]),
        mtime: f["mtime"].as_u64().unwrap_or(0),
        error: f["error"].as_str().map(String::from),
        error_line: f["error_line"].as_u64().unwrap_or(0) as usize,
    }).collect()).unwrap_or_default();
    let settings = data["settings"].as_array().map(|a| a.iter().map(|s| ConfigSetting {
        key: text(&s["key"]),
        kind: SettingKind::from_agent(s["kind"].as_str().unwrap_or("")),
        sources: s["sources"].as_array().map(|src| src.iter().map(|r| ConfigSource {
            file: r["file"].as_u64().unwrap_or(0) as usize,
            start: r["start"].as_u64().unwrap_or(0) as usize,
            end: r["end"].as_u64().unwrap_or(0) as usize,
            relation: SourceRelation::from_agent(r["relation"].as_str().unwrap_or("")),
            value: text(&r["value"]),
            dropped: r["dropped"].as_array().map(|d| d.iter().filter_map(|k| k.as_str().map(String::from)).collect()).unwrap_or_default(),
        }).collect()).unwrap_or_default(),
        line: text(&s["line"]),
        full: text(&s["full"]),
        default: s["default"].as_str().map(String::from),
        redacted: s["redacted"].as_bool().unwrap_or(false),
        suggest: s["suggest"].as_str().map(String::from),
        about: s["about"].as_str().map(String::from),
        unknown_nested: s["unknown_nested"].as_array().map(|a| a.iter().map(|k| {
            (text(&k["key"]), k["suggest"].as_str().map(String::from))
        }).collect()).unwrap_or_default(),
        notes: s["notes"].as_array().map(|a| a.iter().filter_map(|n| n.as_str().map(String::from)).collect()).unwrap_or_default(),
    }).collect()).unwrap_or_default();
    MasterConfig {
        conf_file: text(&data["conf_file"]),
        files,
        master_started: data["master_started"].as_u64().unwrap_or(0),
        settings,
    }
}

fn parse_config_snapshot(data: &serde_json::Value) -> ConfigSnapshot {
    ConfigSnapshot {
        salt_version: data["salt_version"].as_str().unwrap_or("?").to_string(),
        minimum_auth_version: data["minimum_auth_version"].as_u64().unwrap_or(0),
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
        let (success, retcode) = return_outcome(&evt.data, &full_return);
        returns.push(JidReturn {
            minion_id,
            success,
            retcode,
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
        missing: Vec::new(),
        subprocess_secs: 0.0,
        postprocess_secs: 0.0,
        error: String::new(),
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

/// One filter term resolved once for a whole filter pass, matching like
/// `App::matches_filter` (`text`) and `App::host_matches_term` (`host`).
/// `regex_term` hands out a fresh clone of its cached `Regex`, and a fresh
/// clone starts with an empty match cache — rebuilt per field, that made
/// regexes with classes or anchors (`/^db1\d\d$/`) the slow filters (#165).
enum FilterTerm {
    Regex(regex::Regex),
    Glob { pattern: String, lower: String },
    Plain(String),
}

impl FilterTerm {
    fn new(term: &str) -> Self {
        match crate::sniff::regex_term(term) {
            Some(re) => Self::Regex(re),
            None if App::is_glob(term) => Self::Glob { pattern: term.to_string(), lower: term.to_lowercase() },
            None => Self::Plain(term.to_lowercase()),
        }
    }

    /// The term and whether it names a known minion — what makes the Live
    /// Jobs and Events filters also show jobs targeting that minion.
    fn with_minion_check(term: &str, known_minions: &HashSet<String>) -> (Self, bool) {
        let term = Self::new(term);
        let names_minion = known_minions.iter().any(|m| term.host(m));
        (term, names_minion)
    }

    fn text(&self, text: &str) -> bool {
        match self {
            Self::Regex(re) => re.is_match(text),
            Self::Glob { lower, .. } | Self::Plain(lower) => text.to_lowercase().contains(lower.as_str()),
        }
    }

    fn host(&self, host: &str) -> bool {
        match self {
            Self::Glob { pattern, .. } => App::glob_match(host, pattern),
            _ => self.text(host),
        }
    }
}

/// The Events filter, resolved when the filter or the accepted keys change
/// rather than per event: checked on every incoming event, so which known
/// minions each term names is settled once instead of scanning every
/// accepted key per event.
#[derive(Default)]
pub(crate) struct EventMatcher {
    /// Each term and the known minions it names, lowercased.
    terms: Vec<(FilterTerm, HashSet<String>)>,
    /// Whether a job's target covers one of a term's minions, per term and
    /// target seen: a busy master sends the same few targets over and over.
    tgt_covers: std::cell::RefCell<HashMap<(usize, String), bool>>,
}

impl EventMatcher {
    fn new(pat: &str, known_minions: &HashSet<String>) -> Self {
        let terms = App::filter_terms(pat).into_iter().map(|term| {
            let term = FilterTerm::new(term);
            let named = known_minions.iter().filter(|m| term.host(m)).map(|m| m.to_lowercase()).collect();
            (term, named)
        }).collect();
        Self { terms, tgt_covers: Default::default() }
    }

    fn matches(&self, evt: &SaltEvent) -> bool {
        self.terms.is_empty() || self.terms.iter().enumerate().any(|(i, (term, named))| self.matches_term(evt, i, term, named))
    }

    fn matches_term(&self, evt: &SaltEvent, i: usize, term: &FilterTerm, named: &HashSet<String>) -> bool {
        if term.text(&evt.tag)
            || evt.data["fun"].as_str().is_some_and(|f| term.text(f))
            || evt.data["id"].as_str().is_some_and(|i| term.host(i))
            || match term {
                // As the event is shown, like the sniffer.
                FilterTerm::Regex(re) => crate::sniff::pretty_json_lines(&evt.data).iter().any(|l| re.is_match(l)),
                _ => term.text(&evt.data.to_string()),
            }
        {
            return true;
        }
        let tokens = target_tokens(&evt.data["tgt"]);
        if tokens.iter().any(|t| term.host(t)) {
            return true;
        }
        // A term naming known minions also shows the jobs sent to them.
        if named.is_empty() || tokens.is_empty() {
            return false;
        }
        if tokens.iter().any(|t| t == "*") {
            return true;
        }
        let key = (i, tokens.join(","));
        if let Some(&hit) = self.tgt_covers.borrow().get(&key) {
            return hit;
        }
        let hit = tokens.iter().any(|t| {
            let t = t.to_lowercase();
            if App::is_glob(&t) { named.iter().any(|m| App::glob_match(m, &t)) } else { named.contains(&t) }
        });
        self.tgt_covers.borrow_mut().insert(key, hit);
        hit
    }
}

/// The minion names and globs in a job's target: a list (`a,b` or a JSON
/// array), a glob, or the `L@`/`G@`… words of a compound target.
fn target_tokens(tgt: &serde_json::Value) -> Vec<String> {
    let words: Vec<&str> = match tgt {
        serde_json::Value::String(s) => s.split(|c: char| c == ',' || c.is_whitespace()).collect(),
        serde_json::Value::Array(a) => a.iter().filter_map(|v| v.as_str()).collect(),
        _ => Vec::new(),
    };
    words.into_iter()
        .map(|w| match w.split_once('@') {
            Some((kind, rest)) if kind.len() == 1 && kind.chars().all(|c| c.is_ascii_uppercase()) => rest,
            _ => w,
        })
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// A parsed glob `[…]` class: its character ranges, whether it's negated
/// (`[!…]` / `[^…]`), and the pattern index after its `]`.
struct GlobClass {
    ranges: Vec<(char, char)>,
    negated: bool,
    next: usize,
}

/// The glob `[…]` class starting at `p[at]`, or `None` when it never closes.
/// A `]` first in the class is a literal, as in fnmatch.
fn glob_class(p: &[char], at: usize) -> Option<GlobClass> {
    let mut j = at + 1;
    let negated = matches!(p.get(j), Some('!' | '^'));
    if negated {
        j += 1;
    }
    let mut ranges = Vec::new();
    let mut first = true;
    while j < p.len() {
        if p[j] == ']' && !first {
            return Some(GlobClass { ranges, negated, next: j + 1 });
        }
        first = false;
        if p.get(j + 1) == Some(&'-') && p.get(j + 2).is_some_and(|&c| c != ']') {
            ranges.push((p[j], p[j + 2]));
            j += 3;
        } else {
            ranges.push((p[j], p[j]));
            j += 1;
        }
    }
    None
}

/// The help overlay's pages, in the order `Tab` cycles through them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpPage {
    /// Keyboard shortcuts.
    Keys,
    /// Filter, search and sniffer syntax, and notes on what panels show.
    Guide,
    /// The changelog.
    WhatsNew,
}

impl HelpPage {
    pub const ALL: [HelpPage; 3] = [HelpPage::Keys, HelpPage::Guide, HelpPage::WhatsNew];

    pub fn title(self) -> &'static str {
        match self {
            HelpPage::Keys => "Keys",
            HelpPage::Guide => "Guide",
            HelpPage::WhatsNew => "What's new",
        }
    }

    /// The next page (`forward`) or the previous one, wrapping around.
    pub fn step(self, forward: bool) -> HelpPage {
        let i = Self::ALL.iter().position(|&p| p == self).unwrap_or(0);
        let n = Self::ALL.len();
        Self::ALL[if forward { (i + 1) % n } else { (i + n - 1) % n }]
    }
}

/// A filter term to highlight, in one color: its matches, or for a glob
/// (`ids` non-empty) the known minions it matches.
pub struct HighlightTerm {
    pub term: String,
    pub ids: Vec<String>,
}

impl HighlightTerm {
    pub fn ranges(&self, text: &str) -> Vec<(usize, usize)> {
        if self.ids.is_empty() {
            crate::sniff::term_ranges(text, &self.term)
        } else {
            self.ids.iter().flat_map(|id| crate::sniff::term_ranges(text, id)).collect()
        }
    }
}

/// Job Detail's returns-list filter: comma-separated terms matched against
/// the minion ID like every host field (substring, glob or `/regex/`).
pub fn return_matches(minion_id: &str, filter: &str) -> bool {
    let terms = App::filter_terms(filter);
    terms.is_empty() || terms.iter().any(|t| App::host_matches_term(minion_id, t))
}

/// Where a filter's terms match in `text`, for highlighting: a regex term's
/// matches, a glob matching the whole text, or a plain term's occurrences.
pub fn filter_match_ranges(text: &str, filter: &str) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = App::filter_terms(filter).into_iter().flat_map(|t| {
        if App::is_glob(t) {
            if App::glob_match(text, t) { vec![(0, text.len())] } else { Vec::new() }
        } else {
            crate::sniff::term_ranges(text, t)
        }
    }).collect();
    ranges.sort_unstable();
    ranges
}

/// The search match `n` (`forward`) or `N` moves to, given the line of each
/// occurrence: the one after/before the current one, or with none current
/// the first at or below (last above) the scroll position; wraps around.
/// The help scroll offset that shows `line` a couple of lines from the top
/// (the draw clamps it).
pub(crate) fn help_scroll_to(line: usize) -> u16 {
    line.saturating_sub(2).min(u16::MAX as usize) as u16
}

pub(crate) fn step_match(matches: &[usize], current: Option<usize>, scroll: usize, forward: bool) -> Option<usize> {
    let last = matches.len().checked_sub(1)?;
    Some(match (current.filter(|&c| c <= last), forward) {
        (Some(c), true) => if c == last { 0 } else { c + 1 },
        (Some(c), false) => if c == 0 { last } else { c - 1 },
        (None, true) => matches.iter().position(|&l| l >= scroll).unwrap_or(0),
        (None, false) => matches.iter().rposition(|&l| l < scroll).unwrap_or(last),
    })
}

/// The theme's color for ANSI palette slot 0-7 (black, red, green, yellow,
/// blue, magenta, cyan, white; the bright variants share them), so Salt's
/// colored output follows the theme instead of the terminal's own palette.
fn ansi_theme_color(slot: u16, background: bool) -> Color {
    use crate::ui::color;
    match slot {
        0 if background => color::popup_bg(),
        0 => color::inactive_fg(),
        1 => color::red(),
        2 => color::green(),
        3 => color::yellow(),
        4 => color::highlight_1(),
        5 => color::purple(),
        6 => color::cyan(),
        _ if background => color::main_fg(),
        _ => color::hi_fg(),
    }
}

/// Moves the selection to the nearest returns-list entry matching `filter`,
/// looking forward from the current one first.
pub fn snap_to_matching_return(lookup: &JidLookup, selected: &mut Option<usize>, filter: &str) {
    if filter.is_empty() { return; }
    let n = lookup.entry_count();
    let matches = |i: usize| lookup.entry_id(i).is_some_and(|id| return_matches(id, filter));
    let cur = selected.unwrap_or(0).min(n.saturating_sub(1));
    let hit = (cur..n).find(|&i| matches(i)).or_else(|| (0..cur).rev().find(|&i| matches(i)));
    if hit.is_some() { *selected = hit; }
}

/// Per-state outcomes of one state run, split the way Salt's highstate
/// output colors them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StateCounts {
    /// Succeeded without changes.
    pub ok: usize,
    /// Succeeded and changed something.
    pub changed: usize,
    /// `result: None`: a `test=True` run's "would change".
    pub pending: usize,
    pub failed: usize,
}

impl StateCounts {
    /// What the `Chg` column shows: real changes, or a test run's would-be ones.
    pub fn chg(&self) -> usize {
        self.changed + self.pending
    }
}

/// State counts for a state.apply-style return (a dict keyed by state ID);
/// `None` for any other kind of return.
pub fn state_counts(full_return: &serde_json::Value) -> Option<StateCounts> {
    let obj = full_return.as_object()?;
    if obj.is_empty() || obj.contains_key("result") || !obj.values().all(|v| {
        v.as_object().is_some_and(|s| s.contains_key("result") || s.contains_key("__run_num__"))
    }) {
        return None;
    }
    let mut counts = StateCounts::default();
    for state in obj.values() {
        match state.get("result").and_then(|r| r.as_bool()) {
            Some(false) => counts.failed += 1,
            None => counts.pending += 1,
            Some(true) if state.get("changes").and_then(|c| c.as_object()).is_some_and(|c| !c.is_empty()) => counts.changed += 1,
            Some(true) => counts.ok += 1,
        }
    }
    Some(counts)
}

fn derive_success_retcode(full_return: &serde_json::Value) -> (i64, bool) {
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
        // Salt returns error strings starting with "ERROR:" for cmd.run etc.
        // An empty string is a valid result (`config.get` of a missing key).
        serde_json::Value::String(s) if s.starts_with("ERROR:") || s.starts_with("error:") => (1, false),
        _ => (0, true),
    }
}

/// A return's `(success, retcode)`, shared by Live Jobs, Job Detail and
/// minion health so they never disagree. `salt` is a return event or a
/// looked-up return: its `success`/`retcode` are Salt's verdict (the agent
/// reads the retcode stored in the job cache). The return data can only add
/// failures Salt misses -- a failed state, an `ERROR:` string -- never clear
/// one.
pub(crate) fn return_outcome(salt: &serde_json::Value, full_return: &serde_json::Value) -> (bool, i64) {
    let (derived_rc, derived_ok) = derive_success_retcode(full_return);
    let salt_ok = salt["success"].as_bool().unwrap_or(true);
    let salt_rc = salt["retcode"].as_i64().unwrap_or(0);
    if salt_ok && derived_ok && salt_rc == 0 {
        (true, 0)
    } else {
        (false, [salt_rc, derived_rc, 1].into_iter().find(|&rc| rc != 0).unwrap_or(1))
    }
}

fn parse_jid_lookup(data: &serde_json::Value) -> JidLookup {
    let mut result = JidLookup {
        found: data["found"].as_bool().unwrap_or(false),
        fun: data["fun"].as_str().unwrap_or("").to_string(),
        tgt: data["tgt"].as_str().unwrap_or("").to_string(),
        start_time: data["start_time"].as_str().unwrap_or("").to_string(),
        returns: Vec::new(),
        missing: data["missing"].as_array()
            .map(|a| a.iter().filter_map(|m| m.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        subprocess_secs: data["subprocess_secs"].as_f64().unwrap_or(0.0),
        postprocess_secs: data["postprocess_secs"].as_f64().unwrap_or(0.0),
        error: data["error"].as_str().unwrap_or("").to_string(),
    };
    if let Some(arr) = data["returns"].as_array() {
        for entry in arr {
            let full_return = entry["full_return"].clone();
            let (success, retcode) = return_outcome(entry, &full_return);
            result.returns.push(JidReturn {
                minion_id: entry["minion_id"].as_str().unwrap_or("?").to_string(),
                success,
                retcode,
                summary: entry["summary"].as_str().unwrap_or("").to_string(),
                full_return,
                formatted_text: entry["formatted_text"].as_str().unwrap_or("").to_string(),
            });
        }
    }
    result
}

/// With `natural_order`, sorted oldest-first (ascending `start_epoch`) so
/// the Job Cache panel displays newest-at-bottom, matching Live Jobs'
/// convention — regardless of whatever order the agent's `jobs.list_jobs`
/// call happened to return. Otherwise the agent already applied the user's
/// column sort across the whole cache, and its order is kept as-is.
fn event_time_utc(evt: &SaltEvent) -> String {
    chrono::DateTime::from_timestamp(evt.ts as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| evt.ts.to_string())
}

/// Events as readable text — what the Events tab's `s` export and the Event
/// Sniffer's copy/save produce.
pub(crate) fn events_as_text<'a>(events: impl IntoIterator<Item = &'a SaltEvent>) -> String {
    let mut text = String::new();
    for (i, evt) in events.into_iter().enumerate() {
        text.push_str(&format!(
            "Event {}\nTag: {}\nTimestamp: {}\nData:\n{}\n\n",
            i + 1,
            evt.tag,
            event_time_utc(evt),
            serde_json::to_string_pretty(&evt.data).unwrap_or_default(),
        ));
    }
    text
}

/// Events as NDJSON — one `{"tag", "ts", "time", "data"}` object per line,
/// for `jq` and other tools (the `S` exports).
pub(crate) fn events_as_ndjson<'a>(events: impl IntoIterator<Item = &'a SaltEvent>) -> String {
    events.into_iter()
        .map(|evt| serde_json::json!({
            "tag": evt.tag,
            "ts": evt.ts,
            "time": event_time_utc(evt),
            "data": evt.data,
        }).to_string() + "\n")
        .collect()
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
                args: entry["args"].as_str().unwrap_or("").to_string(),
                test_run: entry["test_run"].as_bool().unwrap_or(false),
            });
        }
    }
    result
}

/// A return's timing, for the minion's `Lag` and `Ping`.
#[derive(Debug, PartialEq)]
pub(crate) struct ReturnTiming {
    pub minion: String,
    /// Seconds from the job's publish to this return.
    pub secs: f64,
    /// The job's median, when it was settled before this return came in.
    pub late_median: Option<f64>,
}

/// Tracks the Live Jobs list from a job event; for the return of a job
/// whose publish was seen, also how long the minion took.
fn track_job_from_event(jobs: &mut Vec<JobEntry>, index: &mut HashMap<String, usize>, evt: &SaltEvent) -> Option<ReturnTiming> {
    if evt.tag.ends_with("/new") {
        let jid = evt.data["jid"].as_str().unwrap_or("").to_string();
        if jid.is_empty() { return None; }
        let fun = evt.data["fun"].as_str().unwrap_or("?").to_string();
        let tgt = match &evt.data["tgt"] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(ids) => ids.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(","),
            _ => "?".to_string(),
        };
        // Distinct IDs: a list target naming a minion twice (`-L a,b,a`) is
        // passed through as given, but that minion returns once.
        let minions = evt.data["minions"].as_array()
            .map(|a| a.iter().filter_map(|m| m.as_str()).collect::<HashSet<_>>().len())
            .unwrap_or(0);
        let (args, test_run) = format_job_args(&fun, &evt.data["arg"]);
        if let Some(job) = index.get(&jid).map(|&i| &mut jobs[i]) {
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
            job.args = args;
            job.test_run = test_run;
            // Its return times were measured from that return, not the publish.
            job.lag = JobLag::Settled(None);
            return None;
        }
        index.insert(jid.clone(), jobs.len());
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
            args,
            test_run,
            lag: JobLag::Collecting(Vec::new()),
        });
    } else if evt.tag.contains("/ret/") {
        let jid = evt.tag
            .strip_prefix("salt/job/")
            .and_then(|s| s.split("/ret/").next())
            .unwrap_or("");
        if jid.is_empty() { return None; }
        let (success, _) = return_outcome(&evt.data, &evt.data["return"]);
        if let Some(job) = index.get(jid).map(|&i| &mut jobs[i]) {
            job.returned += 1;
            if success { job.succeeded += 1; } else { job.failed += 1; }
            // Guards against ever displaying more returns than the job was
            // supposedly targeted at (e.g. a dropped/never-received `/new`
            // leaving `target_count` stuck at the single-minion fallback
            // guess below) -- always widen to match reality rather than
            // show a target count smaller than what's actually come back.
            job.target_count = job.target_count.max(job.returned);
            let minion = evt.tag.rsplit('/').next().unwrap_or("");
            if job.local || minion.is_empty() { return None; }
            let secs = (evt.ts - job.created_epoch).max(0.0);
            let late_median = match &mut job.lag {
                JobLag::Collecting(times) => {
                    times.push((minion.to_string(), secs));
                    None
                }
                JobLag::Settled(median) => *median,
            };
            return Some(ReturnTiming { minion: minion.to_string(), secs, late_median });
        } else {
            // Unknown JID → salt-call / local execution (no /new event was emitted)
            let fun = evt.data["fun"].as_str().unwrap_or("?").to_string();
            let tgt = evt.data["id"].as_str().unwrap_or("?").to_string();
            let (args, test_run) = format_job_args(&fun, &evt.data["fun_args"]);
            index.insert(jid.to_string(), jobs.len());
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
                args,
                test_run,
                lag: JobLag::Settled(None),
            });
        }
    }
    None
}

/// Settles a job's lag once it's complete or older than `straggler_secs`,
/// returning the return times collected so far and the job's median, when
/// at least two minions returned.
fn settle_job_lag(job: &mut JobEntry, straggler_secs: u64) -> Option<(Vec<(String, f64)>, f64)> {
    let JobLag::Collecting(times) = &mut job.lag else { return None };
    if !job.finalized && job.age_seconds <= straggler_secs { return None; }
    let times = std::mem::take(times);
    let mut secs: Vec<f64> = times.iter().map(|(_, s)| *s).collect();
    secs.sort_by(f64::total_cmp);
    let median = if secs.len() >= 2 { percentile(&secs, 0.5) } else { None };
    job.lag = JobLag::Settled(median);
    Some((times, median?))
}

/// The value at percentile `p` (0.0-1.0) of sorted values, nearest-rank.
fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    let last = sorted.len().checked_sub(1)?;
    Some(sorted[((last as f64 * p).round() as usize).min(last)])
}

fn scroll_offset(selected: usize, visible: usize, total: usize) -> usize {
    if visible == 0 || total <= visible { return 0; }
    let scroll = selected.saturating_sub(visible / 2);
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

/// Whether a minion activity item matches any of the filter's terms (OR'd,
/// same as every other filter in the app; see `App::filter_terms`).
fn matches_activity_item(item: &MinionActivityItem, terms: &[&str]) -> bool {
    let m = App::matches_filter;
    terms.iter().any(|&term| match item {
        MinionActivityItem::Event(_, evt) => m(&evt.tag, term) || m(&evt.data.to_string(), term),
        MinionActivityItem::CacheJob(job) => m(&job.jid, term) || m(&job.fun, term) || m(&job.args, term),
        MinionActivityItem::Alert(alert) => m(&alert.description, term) || m(&alert.key, term),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_bar_clicks_land_on_the_drawn_labels() {
        // As drawn at 160 columns: " 1 Dashboard " at 1..=13, " 6 API " at
        // 68..=74, " 7 Master Config " at 78..=94, " 8 Pulse " at 98..=106.
        let at = App::tab_at_col;
        assert_eq!([at(0), at(1), at(13), at(14)], [None, Some(Tab::Dashboard), Some(Tab::Dashboard), None]);
        assert_eq!([at(68), at(74), at(78), at(94), at(98), at(106), at(107)],
            [Some(Tab::Api), Some(Tab::Api), Some(Tab::Master), Some(Tab::Master), Some(Tab::Pulse), Some(Tab::Pulse), None]);
    }

    #[test]
    fn module_settings_count_their_ignored_options() {
        let mc = parse_master_config(&serde_json::json!({"settings": [
            {"key": "rest_cherrypy", "kind": "module", "about": "salt-api config (rest_cherrypy)",
             "unknown_nested": [{"key": "colect_stats", "suggest": "collect_stats"}, {"key": "x", "suggest": null}],
             "notes": ["socket_queue_size: documented, but salt-api reads queue_size"]},
            {"key": "auth.ldap.uri", "kind": "module", "about": "option of the ldap eauth backend"},
            {"key": "worker_thread", "kind": "unknown", "suggest": "worker_threads"},
        ]}));
        let [api, ldap, typo] = &mc.settings[..] else { panic!() };
        assert_eq!(api.kind, SettingKind::Module);
        assert_eq!(api.unknown_nested, vec![("colect_stats".to_string(), Some("collect_stats".to_string())), ("x".to_string(), None)]);
        assert_eq!(api.notes.len(), 1);
        assert_eq!([api.ignored_keys(), ldap.ignored_keys(), typo.ignored_keys()], [2, 0, 1]);
    }

    #[test]
    fn ansi_colors_follow_the_theme() {
        let base = Style::default().fg(crate::ui::color::main_fg());
        let spans = App::ansi_to_styled_spans("[31mfail[39m ok [1;92mchanged[38;5;3mwarn", base);
        let fg: Vec<_> = spans.iter().map(|s| s.style.fg).collect();
        assert_eq!(fg, vec![
            Some(crate::ui::color::red()),
            Some(crate::ui::color::main_fg()),
            Some(crate::ui::color::green()),
            Some(crate::ui::color::yellow()),
        ]);
    }

    #[test]
    fn filters_take_regex_terms() {
        assert!(App::matches_filter("state.highstate", r"/^state\.(apply|highstate)$/"));
        assert!(!App::matches_filter("state.single", r"/^state\.(apply|highstate)$/"));
        assert!(App::host_matches_term("web-03", "/web-0[1-5]$/"));
        assert!(!App::host_matches_term("web-07", "/web-0[1-5]$/"));
        assert!(App::host_matches_term("web-07", "web-*"), "globs still work");
        assert!(App::matches_filter("x /web-0 y", "/web-0"), "an unclosed slash is plain text");
        assert!(return_matches("db-02", "web*, /^db-0[0-9]$/"));
        assert!(!return_matches("db-12", "web*, /^db-0[0-9]$/"));
        assert_eq!(filter_match_ranges("web-01", "/b-0/, web*"), [(0, 6), (2, 5)]);
    }

    #[test]
    fn a_glob_highlights_its_minions_as_one_term() {
        let t = HighlightTerm { term: "web*".into(), ids: vec!["web1".into(), "web2".into()] };
        assert_eq!(t.ranges("web2 then web1"), [(10, 14), (0, 4)]);
        let plain = HighlightTerm { term: "/b[12]/".into(), ids: Vec::new() };
        assert_eq!(plain.ranges("web2"), [(2, 4)]);
    }

    #[test]
    fn globs_take_character_classes_like_salt() {
        assert!(App::host_matches_term("debian12-3006-9-0", "debian[1-2]*"));
        assert!(App::host_matches_term("debian11-3006-9-0", "DEBIAN1[12]-*"));
        assert!(!App::host_matches_term("debian10-3006-9-0", "debian1[12]-*"));
        assert!(App::glob_match("web-a", "web-[!0-9]"));
        assert!(!App::glob_match("web-1", "web-[^0-9]"));
        assert!(App::glob_match("a]b", "a[]]b"));
        assert!(App::glob_match("web[1", "web[1"), "an unclosed [ is literal");
    }

    #[test]
    fn step_match_moves_by_occurrence_and_wraps() {
        // Two occurrences on line 3, one on 9.
        let m = [3, 3, 9];
        assert_eq!(step_match(&m, None, 0, true), Some(0));
        assert_eq!(step_match(&m, Some(0), 0, true), Some(1));
        assert_eq!(step_match(&m, Some(1), 0, true), Some(2));
        assert_eq!(step_match(&m, Some(2), 0, true), Some(0));
        assert_eq!(step_match(&m, Some(0), 0, false), Some(2));
        assert_eq!(step_match(&m, None, 5, true), Some(2));
        assert_eq!(step_match(&m, None, 5, false), Some(1));
        assert_eq!(step_match(&m, None, 0, false), Some(2));
        assert_eq!(step_match(&[], None, 0, true), None);
    }
    use serde_json::json;

    #[test]
    fn filter_terms_split_on_commas_and_drop_blanks() {
        assert_eq!(App::filter_terms("web1, web2,,  db* "), vec!["web1", "web2", "db*"]);
        assert!(App::filter_terms(" , ").is_empty());
    }

    #[test]
    fn job_filter_shows_star_jobs_only_for_known_minion_terms() {
        let job = |tgt: &str| JobEntry {
            jid: "20261012133558123456".into(), fun: "state.apply".into(), tgt: tgt.into(),
            target_count: 1, returned: 0, succeeded: 0, failed: 0, age_seconds: 0, created_epoch: 0.0,
            local: false, finalized: false, args: String::new(), test_run: false, lag: JobLag::Settled(None),
        };
        let known: HashSet<String> = ["web01".to_string(), "db01".to_string()].into();
        let (star, db) = (job("*"), job("db01"));
        let web = App::job_matcher("web*", &known);
        assert!(web(&star) && !web(&db), "a known-minion term also shows jobs targeting *");
        let other = App::job_matcher("nginx, db01.prod", &known);
        assert!(!other(&star) && !other(&db), "terms naming no minion leave * jobs out");
        assert!(App::job_matcher("5581", &known)(&star), "the JID still matches");
        assert!(App::job_matcher(" , ", &known)(&db), "no terms shows everything");
        let re = App::job_matcher(r"/^db\d+$/", &known);
        assert!(re(&star) && re(&db), "a regex naming a known minion also shows jobs targeting *");
        assert!(App::job_matcher("/STATE/i", &known)(&db));
        assert!(!App::job_matcher("db*", &known)(&job("x-db01")), "globs anchor to the whole target");
    }

    #[test]
    fn event_filter_shows_targeted_jobs_for_known_minion_terms() {
        let evt = |tag: &str, data: serde_json::Value| SaltEvent { tag: tag.into(), data, ts: 0.0 };
        let star = evt("salt/job/1/new", json!({"fun": "test.ping", "tgt": "*"}));
        let listed = evt("salt/job/2/new", json!({"fun": "test.ping", "tgt": "DB01,web02"}));
        let other = evt("salt/job/3/new", json!({"fun": "test.ping", "tgt": "mail01"}));
        let ret = evt("salt/job/4/ret/db01", json!({"fun": "test.ping", "id": "db01"}));
        let known: HashSet<String> = ["web01".to_string(), "db01".to_string()].into();
        let web = EventMatcher::new("web0*", &known);
        assert!(web.matches(&star) && web.matches(&listed) && !web.matches(&other));
        assert!(web.matches(&listed), "a cached target answers the same");
        let re = EventMatcher::new(r"/^db\d+$/", &known);
        assert!(re.matches(&ret) && re.matches(&star) && !re.matches(&other));
        let plain = EventMatcher::new("mail", &known);
        assert!(plain.matches(&other) && !plain.matches(&star), "a term naming no minion leaves * jobs out");
        assert!(EventMatcher::new("", &known).matches(&other));

        let only_db = evt("salt/job/5/new", json!({"fun": "test.ping", "tgt": "db01"}));
        let web01 = EventMatcher::new("web01", &known);
        assert!(!web01.matches(&only_db), "a job sent to another known minion isn't one of web01's");
        for tgt in [json!("web*"), json!(["db01", "web01"]), json!("L@web01,db02 or G@os:Debian")] {
            assert!(web01.matches(&evt("salt/job/6/new", json!({"fun": "test.ping", "tgt": tgt.clone()}))), "{tgt}");
        }
        assert!(!web01.matches(&evt("salt/job/7/new", json!({"fun": "test.ping", "tgt": "web1*"}))));
    }

    #[test]
    fn host_terms_use_globs_only_when_they_contain_wildcards() {
        assert!(App::host_matches_term("web01.prod", "web"), "plain terms are substrings");
        assert!(App::host_matches_term("web01.prod", "WEB*"), "globs are case-insensitive");
        assert!(!App::host_matches_term("db-web01", "web*"), "globs anchor to the whole id");
        assert!(App::host_matches_term("web7", "web?"));
        assert!(!App::host_matches_term("web17", "web?"));
    }

    #[test]
    fn job_return_success_and_retcode() {
        assert_eq!(derive_success_retcode(&json!(true)), (0, true));
        assert_eq!(derive_success_retcode(&json!(false)), (0, false));
        assert_eq!(derive_success_retcode(&json!(null)), (1, false));
        assert_eq!(derive_success_retcode(&json!("ERROR: no such function")), (1, false));
        assert_eq!(derive_success_retcode(&json!("")), (0, true), "config.get of a missing key");
        assert_eq!(derive_success_retcode(&json!("15:01 up 3 days")), (0, true));
        assert_eq!(derive_success_retcode(&json!({"retcode": 2, "stdout": ""})), (2, false));
        let states_ok = json!({"file_|-a_|-/a_|-managed": {"result": true, "__run_num__": 0}});
        let states_bad = json!({
            "file_|-a_|-/a_|-managed": {"result": true, "__run_num__": 0},
            "cmd_|-b_|-false_|-run": {"result": false, "__run_num__": 1},
        });
        assert_eq!(derive_success_retcode(&states_ok), (0, true));
        assert_eq!(derive_success_retcode(&states_bad), (1, false), "one failed state fails the whole run");
    }

    #[test]
    fn job_cache_keeps_the_agents_newest_first_order() {
        let data = json!({"jobs": [
            {"jid": "20261009120000000003", "fun": "test.ping", "start_epoch": 3.0},
            {"jid": "20261009120000000002", "fun": "test.ping", "start_epoch": 3.0},
            {"jid": "20261009120000000001", "fun": "test.ping", "start_epoch": 1.0},
        ]});
        let jids: Vec<String> = parse_cache_jobs(&data).into_iter().map(|j| j.jid).collect();
        assert_eq!(jids, ["20261009120000000003", "20261009120000000002", "20261009120000000001"]);
    }

    #[test]
    fn jid_lookup_lists_missing_minions_after_the_returns() {
        let l = parse_jid_lookup(&json!({
            "found": true,
            "returns": [{"minion_id": "web01", "success": true, "retcode": 0, "full_return": true}],
            "missing": ["db01", "web02"],
        }));
        assert_eq!(l.entry_count(), 3);
        assert_eq!((l.entry_id(0), l.entry_id(1), l.entry_id(2), l.entry_id(3)), (Some("web01"), Some("db01"), Some("web02"), None));
        let mut sel = Some(0);
        snap_to_matching_return(&l, &mut sel, "web02");
        assert_eq!(sel, Some(2), "the filter can land on a missing minion");
    }

    #[test]
    fn lookup_outcome_trusts_the_stored_retcode_and_the_return_data() {
        let out = |entry: serde_json::Value| return_outcome(&entry, &entry["full_return"]);
        assert_eq!(out(json!({"success": true, "retcode": 0, "full_return": "up 3 days"})), (true, 0));
        assert_eq!(out(json!({"success": false, "retcode": 127, "full_return": "/bin/sh: uptime: not found"})), (false, 127),
            "an exit code the output text doesn't show");
        assert_eq!(out(json!({"success": true, "retcode": 0, "full_return": "ERROR: boom"})), (false, 1),
            "an error the agent missed");
        assert_eq!(out(json!({"full_return": "up 3 days"})), (true, 0), "missing fields default to success");
    }

    #[test]
    fn minions_list_has_a_row_per_key() {
        let key = |id: &str, status| KeyInfo { id: id.into(), status, mtime_epoch: 0 };
        let keys = [
            key("web01", KeyStatus::Accepted),
            key("db01", KeyStatus::Accepted),
            key("new01", KeyStatus::Pending),
            key("web01", KeyStatus::Denied),
        ];
        let minions = [MinionInfo { id: "web01".into(), key_status: Some(KeyStatus::Accepted), ..Default::default() }];
        let index: HashMap<String, usize> = [("web01".to_string(), 0)].into();
        let rows = minion_list_rows(&keys, &minions, &index);
        let shown: Vec<(&str, Option<KeyStatus>, bool)> = rows.iter().map(|r| (r.id(), r.key_status(), r.minion().is_some())).collect();
        assert_eq!(shown, [
            ("web01", Some(KeyStatus::Accepted), true),
            ("db01", Some(KeyStatus::Accepted), false),
            ("new01", Some(KeyStatus::Pending), false),
            ("web01", Some(KeyStatus::Denied), false),
        ], "the denied duplicate is its own row; db01 is listed before the master knows it");
        assert_eq!(minion_list_rows(&[], &minions, &HashMap::new()).len(), 1, "without keys, the known accepted minions");
    }

    #[test]
    fn live_jobs_counts_a_minion_targeted_twice_once() {
        let evt = |tag: &str, data: serde_json::Value| SaltEvent { tag: tag.into(), data, ts: 0.0 };
        let (mut jobs, mut index) = (Vec::new(), HashMap::new());
        track_job_from_event(&mut jobs, &mut index, &evt("salt/job/1/new", json!({
            "jid": "1", "fun": "test.ping", "tgt": "web01,db01,web01", "tgt_type": "list",
            "minions": ["web01", "db01", "web01"],
        })));
        for mid in ["web01", "db01"] {
            track_job_from_event(&mut jobs, &mut index, &evt(&format!("salt/job/1/ret/{mid}"),
                json!({"id": mid, "fun": "test.ping", "return": true, "retcode": 0, "success": true})));
        }
        assert_eq!((jobs[0].target_count, jobs[0].returned), (2, 2), "nothing left waiting");
    }

    #[test]
    fn live_jobs_and_job_detail_agree_on_a_return() {
        let cases = [
            (json!(""), true, "config.get of a missing key"),
            (json!("ERROR: boom"), false, "an error Salt reported as a success"),
            (json!({"pkg_|-vim_|-vim_|-installed": {"result": false}}), false, "a failed state"),
        ];
        for (ret, ok, what) in cases {
            let evt = SaltEvent {
                tag: "salt/job/1/ret/web01".into(),
                data: json!({"id": "web01", "fun": "config.get", "return": ret, "retcode": 0, "success": true}),
                ts: 0.0,
            };
            let (mut jobs, mut index) = (Vec::new(), HashMap::new());
            track_job_from_event(&mut jobs, &mut index, &evt);
            let detail = build_jid_lookup_from_events("1", &VecDeque::from([evt])).unwrap();
            assert_eq!((jobs[0].failed == 0, detail.returns[0].success), (ok, ok), "{what}");
        }
    }

    #[test]
    fn minion_health_keeps_a_window_and_the_latest_failure() {
        let mut h = MinionHealth::default();
        assert_eq!(h.fail_ratio(), None);
        h.record(false, 1.0, Some("cmd.run exit 3".into()));
        for i in 0..MINION_HEALTH_WINDOW {
            h.record(i % 10 != 0, 2.0 + i as f64, (i % 10 == 0).then(|| format!("job {}", i)));
        }
        assert_eq!(h.recent.len(), MINION_HEALTH_WINDOW, "the oldest outcome drops out");
        assert_eq!(h.failed(), MINION_HEALTH_WINDOW / 10);
        assert_eq!(h.fail_ratio(), Some(0.1));
        assert_eq!(h.last_failure, Some((42.0, "job 40".to_string())));
    }

    #[test]
    fn a_minion_is_rated_against_its_peers_in_the_same_job() {
        let evt = |tag: &str, ts: f64| SaltEvent { tag: tag.into(), ts, data: json!({
            "jid": "1", "fun": "state.apply", "tgt": "*", "minions": ["a", "b", "c", "d"],
        }) };
        let (mut jobs, mut index) = (Vec::new(), HashMap::new());
        track_job_from_event(&mut jobs, &mut index, &evt("salt/job/1/new", 100.0));
        for (mid, ts) in [("a", 110.0), ("b", 112.0), ("c", 140.0)] {
            let t = track_job_from_event(&mut jobs, &mut index, &evt(&format!("salt/job/1/ret/{mid}"), ts)).unwrap();
            assert_eq!((t.secs, t.late_median), (ts - 100.0, None), "collected until settled");
        }
        assert_eq!(settle_job_lag(&mut jobs[0], 300), None, "not complete and not a straggler yet");
        jobs[0].age_seconds = 301;
        let (times, median) = settle_job_lag(&mut jobs[0], 300).unwrap();
        assert_eq!((times.len(), median), (3, 12.0));
        let mut c = MinionHealth::default();
        c.record_lag(40.0, median);
        assert_eq!(c.lag_at(0.5), Some(40.0 / 12.0));
        let late = track_job_from_event(&mut jobs, &mut index, &evt("salt/job/1/ret/d", 190.0)).unwrap();
        assert_eq!(late.late_median, Some(12.0), "a late return is rated against the settled median");
        assert_eq!(settle_job_lag(&mut jobs[0], 300), None, "settled once");
    }

    #[test]
    fn fast_jobs_and_lone_returns_give_no_misleading_lag() {
        let mut h = MinionHealth::default();
        h.record_lag(0.12, 0.03);
        assert!(h.lag.is_empty(), "a job too fast to tell lag from jitter isn't rated");
        h.record_lag(2.0, 0.03);
        h.record_lag(0.3, 0.9);
        assert_eq!(Vec::from(h.lag.clone()), [4.0, 0.5 / 0.9], "a 2 s answer to a fast job is slow");
        let new = SaltEvent { tag: "salt/job/2/new".into(), ts: 0.0, data: json!({"jid": "2", "fun": "test.ping", "minions": ["a", "b"]}) };
        let ret = SaltEvent { tag: "salt/job/2/ret/a".into(), ts: 1.0, data: json!({"id": "a", "fun": "test.ping", "return": true}) };
        let (mut jobs, mut index) = (Vec::new(), HashMap::new());
        track_job_from_event(&mut jobs, &mut index, &new);
        track_job_from_event(&mut jobs, &mut index, &ret);
        jobs[0].age_seconds = 999;
        assert_eq!(settle_job_lag(&mut jobs[0], 300), None, "one return has no peers");
        assert_eq!(jobs[0].lag, JobLag::Settled(None));
        let local = SaltEvent { tag: "salt/job/3/ret/a".into(), ts: 5.0, data: json!({"id": "a", "fun": "test.ping", "return": true}) };
        assert_eq!(track_job_from_event(&mut jobs, &mut index, &local), None, "no publish seen, no timing");
    }

    #[test]
    fn percentiles_use_the_nearest_rank() {
        assert_eq!(percentile(&[], 0.5), None);
        assert_eq!(percentile(&[1.0, 2.0, 9.0], 0.5), Some(2.0));
        assert_eq!(percentile(&(1..=20).map(f64::from).collect::<Vec<_>>(), 0.95), Some(19.0));
    }

    #[test]
    fn cache_jobs_match_glob_and_list_targets() {
        let job = |tgt: &str, tgt_type: &str| CacheJobEntry {
            jid: "1".into(), fun: "test.ping".into(), tgt: tgt.into(), tgt_type: tgt_type.into(),
            user: "root".into(), start_time: String::new(), start_epoch: 0.0, args: String::new(), test_run: false,
        };
        assert!(cache_job_targets(&job("*", "glob"), "web01"));
        assert!(cache_job_targets(&job("web*", "glob"), "web01"));
        assert!(!cache_job_targets(&job("db*", "glob"), "web01"));
        assert!(cache_job_targets(&job("db01,web01", "list"), "web01"));
        assert!(!cache_job_targets(&job("db01,web011", "list"), "web01"));
        assert!(cache_job_targets(&job("web01", "grain"), "web01"));
        assert!(!cache_job_targets(&job("os:Debian", "grain"), "web01"));
    }

    #[test]
    fn return_events_report_jid_and_outcome() {
        let evt = SaltEvent { tag: "salt/job/2026/ret/web01".into(), data: json!({}), ts: 0.0 };
        assert_eq!(return_event_jid(&evt), Some("2026"));
        let new = SaltEvent { tag: "salt/job/2026/new".into(), data: json!({}), ts: 0.0 };
        assert_eq!(return_event_jid(&new), None);

        let ok = return_event_result(&json!({"return": true, "retcode": 0, "success": true}));
        assert_eq!(ok, ReturnResult { success: true, retcode: 0, counts: None });
        let failed_rc = return_event_result(&json!({"return": "boom", "retcode": 2, "success": true}));
        assert_eq!((failed_rc.success, failed_rc.retcode), (false, 2));
        let state = return_event_result(&json!({"retcode": 0, "return": {
            "a": {"result": true, "changes": {}, "__run_num__": 0},
            "b": {"result": false, "changes": {}, "__run_num__": 1},
        }}));
        assert_eq!(state, ReturnResult { success: false, retcode: 1, counts: Some(StateCounts { ok: 1, failed: 1, ..Default::default() }) });
    }

    #[test]
    fn state_counts_split_ok_changed_failed() {
        let ret = json!({
            "pkg_|-nginx_|-nginx_|-installed": {"result": true, "changes": {}, "__run_num__": 0},
            "file_|-conf_|-/etc/nginx.conf_|-managed": {"result": true, "changes": {"diff": "x"}, "__run_num__": 1},
            "service_|-nginx_|-nginx_|-running": {"result": false, "changes": {}, "__run_num__": 2},
            "cmd_|-check_|-true_|-run": {"result": null, "changes": {}, "__run_num__": 3},
        });
        assert_eq!(state_counts(&ret), Some(StateCounts { ok: 1, changed: 1, pending: 1, failed: 1 }),
            "result: null is a test run's would-change, not a success");
        assert_eq!(state_counts(&json!({"result": true})), None);
        assert_eq!(state_counts(&json!("output")), None);
        assert_eq!(state_counts(&json!({})), None);
    }

    #[test]
    fn events_export_as_text_and_ndjson() {
        let events = [
            SaltEvent { tag: "salt/job/1/new".into(), data: json!({"fun": "test.ping"}), ts: 0.0 },
            SaltEvent { tag: "salt/job/1/ret/web01".into(), data: json!({"return": true}), ts: 60.0 },
        ];
        let text = events_as_text(&events);
        assert!(text.starts_with("Event 1\nTag: salt/job/1/new\nTimestamp: 1970-01-01 00:00:00 UTC\nData:\n"), "{}", text);
        assert!(text.contains("Event 2\nTag: salt/job/1/ret/web01\nTimestamp: 1970-01-01 00:01:00 UTC"));

        let ndjson = events_as_ndjson(&events);
        let lines: Vec<serde_json::Value> = ndjson.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1]["tag"], "salt/job/1/ret/web01");
        assert_eq!(lines[1]["ts"], 60.0);
        assert_eq!(lines[1]["time"], "1970-01-01 00:01:00 UTC");
        assert_eq!(lines[1]["data"]["return"], true);
        assert!(events_as_ndjson(&[]).is_empty());
    }

    #[test]
    fn save_file_names_are_sanitized_and_keep_their_extension() {
        let name = App::propose_save_filename("job 123/web:1", "json");
        assert!(name.starts_with("job-123-web-1-"), "{}", name);
        assert!(name.ends_with(".json"), "{}", name);
        assert!(App::propose_save_filename("///", "txt").starts_with("salt-radar-"));
    }

    #[test]
    fn request_line_splits_method_and_path_without_query() {
        assert_eq!(App::parse_request_line("GET /jobs?limit=5 HTTP/1.1"), ("GET".into(), "/jobs".into()));
        assert_eq!(App::parse_request_line(""), ("?".into(), "?".into()));
    }

    #[test]
    fn path_browser_lists_dirs_first_and_opens_at_the_current_value() {
        let dir = std::env::temp_dir().join(format!("salt-radar-test-browser-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("b.py"), "").unwrap();
        std::fs::write(dir.join("a.conf"), "").unwrap();
        std::fs::write(dir.join(".hidden"), "").unwrap();

        let files = PathBrowser::open(PathKind::File, dir.join("b.py").to_str().unwrap());
        assert_eq!(files.dir, dir);
        assert_eq!(files.entries, vec![
            BrowserEntry::Parent,
            BrowserEntry::Dir("sub".into()),
            BrowserEntry::File("a.conf".into()),
            BrowserEntry::File("b.py".into()),
        ]);
        assert_eq!(files.entries[files.selected], BrowserEntry::File("b.py".into()), "opens with the current file selected");

        let dirs = PathBrowser::open(PathKind::Dir, dir.join("typo/deeper").to_str().unwrap());
        assert_eq!(dirs.dir, dir, "a missing path opens at its nearest existing ancestor");
        assert_eq!(dirs.entries, vec![BrowserEntry::SelectHere, BrowserEntry::Parent, BrowserEntry::Dir("sub".into())]);

        let mut jump = PathBrowser::open(PathKind::File, dir.join("a.conf").to_str().unwrap());
        let t0 = Instant::now();
        jump.type_ahead('B', t0);
        assert_eq!(jump.entries[jump.selected], BrowserEntry::File("b.py".into()), "case-insensitive prefix");
        jump.type_ahead('s', t0);
        assert_eq!(jump.entries[jump.selected], BrowserEntry::Dir("sub".into()), "no 'bs...': start over from 's'");
        jump.type_ahead('a', t0 + std::time::Duration::from_secs(2));
        assert_eq!(jump.typed, "a", "a pause starts a new name");
        assert_eq!(jump.entries[jump.selected], BrowserEntry::File("a.conf".into()));

        assert!(PathKind::Dir.exists(dir.to_str().unwrap()));
        assert!(PathKind::File.exists(dir.join("a.conf").to_str().unwrap()));
        assert!(!PathKind::File.exists(dir.to_str().unwrap()));
        assert!(!PathKind::Dir.exists(dir.join("nope").to_str().unwrap()));
        std::fs::remove_dir_all(dir).unwrap();
    }
}



