use std::collections::VecDeque;
use std::time::Instant;

// ─── Tabs ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Minions,
    Events,
    Jobs,
    Alerts,
    Pulse,
    Api,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SparklineStyle {
    Bars,
    Dots,
}

impl Default for SparklineStyle {
    fn default() -> Self { Self::Dots }
}

impl Tab {
    pub fn next(&self) -> Self {
        match self {
            Self::Dashboard => Self::Minions,
            Self::Minions => Self::Events,
            Self::Events => Self::Jobs,
            Self::Jobs => Self::Alerts,
            Self::Alerts => Self::Api,
            Self::Api => Self::Pulse,
            Self::Pulse => Self::Dashboard,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            Self::Dashboard => Self::Pulse,
            Self::Minions => Self::Dashboard,
            Self::Events => Self::Minions,
            Self::Jobs => Self::Events,
            Self::Alerts => Self::Jobs,
            Self::Api => Self::Alerts,
            Self::Pulse => Self::Api,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::Minions => "Minions",
            Self::Events => "Events",
            Self::Jobs => "Jobs",
            Self::Alerts => "Alerts",
            Self::Pulse => "Pulse",
            Self::Api => "API",
        }
    }
}

// ─── Key types ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Deserialize)]
pub struct KeyInfo {
    pub id: String,
    pub status: KeyStatus,
    pub mtime_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
pub enum KeyStatus {
    Accepted,
    Pending,
    Rejected,
    Denied,
}

impl KeyStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Accepted => "accepted",
            Self::Pending => "pending",
            Self::Rejected => "rejected",
            Self::Denied => "denied",
        }
    }
}

// ─── Minion info ───────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct MinionInfo {
    pub id: String,
    pub key_status: Option<KeyStatus>,
    pub os: String,
    pub osrelease: String,
    pub osfinger: String,
    pub salt_version: String,
    pub cpuarch: String,
    pub last_seen_epoch: u64,
    pub cache_updated_epoch: u64,
    pub connected: bool,
    pub timed_out: bool,
}

// ─── Process info ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Deserialize)]
pub struct SaltProcess {
    pub pid: u32,
    pub role: SaltProcessRole,
    pub rss_kb: u64,
    /// Raw utime+stime clock ticks from the agent; cpu_percent is computed
    /// Rust-side from the delta between consecutive polls (see App::prev_proc_cpu).
    #[serde(default)]
    pub cpu_ticks: u64,
    #[serde(default)]
    pub cpu_percent: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub enum SaltProcessRole {
    Master,
    Worker,
    API,
    Syndic,
    Minion,
    EventPublisher,
    ReqServer,
    Unknown,
}

impl SaltProcessRole {
    pub fn label(&self) -> &str {
        match self {
            Self::Master => "salt-master",
            Self::Worker => "worker",
            Self::API => "salt-api",
            Self::Syndic => "salt-syndic",
            Self::Minion => "salt-minion",
            Self::EventPublisher => "EventPublisher",
            Self::ReqServer => "ReqServer",
            Self::Unknown => "unknown",
        }
    }
}

// ─── Service health ────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Deserialize)]
pub struct ServiceHealth {
    pub unit: String,
    pub active: bool,
    pub state: String,
    pub uptime_seconds: u64,
    pub active_enter_timestamp: String,
    pub memory_kb: u64,
}

// ─── Service trend snapshot (restart/flap + memory growth) ────────────────

#[derive(Clone, Debug)]
pub struct ServiceTrendSnapshot {
    pub unit: String,
    pub restart_count_24h: usize,
    /// MB/hour linear-regression slope over the memory sample window; None
    /// until enough samples have accumulated.
    pub mem_growth_mb_per_hour: Option<f64>,
    /// Whether the most recent third of the sample window is still trending
    /// upward (distinguishes an active leak from growth that has plateaued).
    pub mem_still_growing: bool,
}

// ─── ZMQ connection ───────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Deserialize)]
pub struct ZmqConn {
    pub local_port: u16,
    pub remote_addr: String,
    pub remote_port: u16,
    pub state: String,
}

// ─── System info ───────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct SystemInfo {
    pub load_1: f64,
    pub load_5: f64,
    pub load_15: f64,
    pub mem_total_kb: u64,
    pub mem_avail_kb: u64,
    pub cache_disk_usage_pct: f64,
    pub pki_disk_usage_pct: f64,
    pub uptime_secs: u64,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
    pub network_rx_rate: f64,
    pub network_tx_rate: f64,
    /// Cumulative system-wide CPU jiffies from /proc/stat ("cpu " line);
    /// cpu_percent is computed from the delta between polls (see App::update),
    /// same as load_1 was previously misused for this — load average is a
    /// ~1-minute exponentially-decayed figure that lags well behind real
    /// instantaneous usage (e.g. tools like btop), unlike this.
    pub cpu_total_jiffies: u64,
    pub cpu_idle_jiffies: u64,
    #[serde(default)]
    pub cpu_percent: f64,
}

// ─── salt-api stats (rest_cherrypy collect_stats, polled over /stats) ─────

#[derive(Clone, Debug, Default)]
pub struct ApiStats {
    /// rest_cherrypy is configured on this master at all (opts has the key).
    pub configured: bool,
    /// The /stats endpoint actually answered — false if collect_stats/
    /// stats_disable_auth aren't set, salt-api isn't running, or the poll
    /// otherwise failed (see `error`).
    pub reachable: bool,
    pub port: u16,
    pub scheme: String,
    pub error: String,
    pub current_requests: u64,
    pub total_requests: u64,
    pub requests_per_sec: f64,
    pub bytes_read_per_sec: f64,
    pub bytes_written_per_sec: f64,
    pub uptime_secs: f64,
    /// One entry per CherryPy worker thread: that thread's current (or most
    /// recently completed) request. Not a request log — a live snapshot of
    /// in-flight/recent activity, capped at however many worker threads
    /// salt-api has (thread_pool, default 100).
    pub requests: Vec<ApiRequestRecord>,
}

/// One completed salt-api request, kept in App::api_requests_log — built by
/// watching cpstats' per-thread snapshot for requests that are new since the
/// last poll (see App::update's "get_api_stats" handler). Not a complete
/// record: if a single worker thread completes more than one request between
/// two polls, only the last one is ever visible in the snapshot this is
/// built from.
#[derive(Clone, Debug)]
pub struct ApiLogEntry {
    pub received_at: std::time::Instant,
    pub client: String,
    pub method: String,
    pub path: String,
    /// Full cpstats Request-Line, query string included (the table/log row
    /// only shows the bare path; the detail popup shows this in full).
    pub raw_request_line: String,
    pub response_status: String,
    pub processing_time_secs: f64,
}

/// Per-endpoint (method+path) rollup of the captured request log —
/// see App::api_endpoint_breakdown.
#[derive(Clone, Debug)]
pub struct ApiEndpointStats {
    pub method: String,
    pub path: String,
    pub count: usize,
    pub avg_secs: f64,
    pub p95_secs: f64,
    pub error_pct: f64,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct ApiRequestRecord {
    pub thread_id: String,
    pub client: String,
    pub request_line: String,
    pub response_status: String,
    pub processing_time_secs: f64,
    pub end_time: f64,
}

// ─── Config snapshot (from agent) ──────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct ConfigSnapshot {
    pub salt_version: String,
    pub python_version: String,
    pub worker_threads: Option<i64>,
    pub transport: String,
    pub sock_dir: String,
    pub reactor_count: usize,
    pub fileserver_backends: Vec<String>,
    pub gitfs_remote_count: usize,
    pub ext_pillar_names: Vec<String>,
    pub master_job_cache: String,
    pub pillar_cache: bool,
    pub presence_events: bool,
    pub interface: String,
    pub publish_port: u16,
    pub ret_port: u16,
    pub api_port: u16,
    pub pki_dir: String,
    pub cachedir: String,
    pub cpu_count: u32,
    pub hostname: String,
}

// ─── Event type breakdown ─────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default)]
pub struct EventBreakdown {
    pub auth: u64,
    pub job_return: u64,
    pub start: u64,
    pub beacon: u64,
    pub refresh: u64,
    pub other: u64,
    pub total: u64,
}

impl EventBreakdown {
    pub fn record(&mut self, tag: &str) {
        self.total += 1;
        if tag.starts_with("salt/auth") {
            self.auth += 1;
        } else if tag.contains("/ret/") {
            self.job_return += 1;
        } else if (tag.starts_with("salt/minion/") && tag.contains("/start")) || tag.contains("minion_start") {
            self.start += 1;
        } else if tag.starts_with("salt/beacon") {
            self.beacon += 1;
        } else if tag.contains("minion/refresh") {
            // Minion-management traffic (grains/pillar/modules/beacon-list
            // refresh) — distinct from a minion actually starting/connecting.
            // Scoped to minion/refresh/* specifically for now, not every
            // tag containing "/refresh" anywhere.
            self.refresh += 1;
        } else {
            self.other += 1;
        }
    }
}

// ─── Event bus event ──────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct SaltEvent {
    pub tag: String,
    pub data: serde_json::Value,
    pub ts: f64,
}

// ─── Event sniffer state ─────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct SnifferState {
    pub minion_ids: Vec<String>,
    pub buffer: VecDeque<SaltEvent>,
    pub max_size: usize,
    pub paused: bool,
    pub selected: usize,
    /// Events display oldest-at-top, newest-at-bottom. While following,
    /// `selected` stays pinned to the newest captured event as new ones
    /// arrive (tail -f style); moving selection away from the last row
    /// turns this off so live traffic doesn't yank the selection around.
    pub following: bool,
    pub highlight_keywords: Vec<String>,
    /// When Some, the user is editing highlight keywords within the popup
    pub highlight_input: Option<String>,
}

// ─── Job entry (live) ─────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct JobEntry {
    pub jid: String,
    pub fun: String,
    pub tgt: String,
    pub target_count: usize,
    pub returned: usize,
    pub age_seconds: u64,
    pub created_epoch: f64,
    pub local: bool,
    pub finalized: bool,
}

// ─── Cache job entry (from agent jobs.list_jobs) ──────────────────────────

#[derive(Clone, Debug)]
pub struct CacheJobEntry {
    pub jid: String,
    pub fun: String,
    pub tgt: String,
    pub tgt_type: String,
    pub user: String,
    pub start_time: String,
    pub start_epoch: f64,
    pub arguments: Vec<String>,
}

// ─── JID lookup result ────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct JidLookup {
    pub found: bool,
    pub fun: String,
    pub tgt: String,
    pub start_time: String,
    pub returns: Vec<JidReturn>,
    /// How long the agent's `salt-run jobs.lookup_jid` subprocess call and
    /// its own post-processing each took — shown in the popup header so a
    /// slow lookup is visible right there instead of needing to go dig
    /// through the agent's stderr log.
    pub subprocess_secs: f64,
    pub postprocess_secs: f64,
}

#[derive(Clone, Debug)]
pub struct JidReturn {
    pub minion_id: String,
    pub success: bool,
    pub retcode: i64,
    pub summary: String,
    pub full_return: serde_json::Value,
    pub formatted_text: String,
}

// ─── Alert ─────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct AlertEntry {
    pub key: String,
    pub severity: AlertSeverity,
    pub rule: &'static str,
    pub description: String,
    pub details: Vec<String>,
    pub active_since: Instant,
    pub count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertSeverity {
    Info,
    Warn,
    Crit,
}

impl AlertSeverity {
    pub fn label(&self) -> &str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Crit => "crit",
        }
    }
}

// ─── Fleet scan ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct FleetMinion {
    pub id: String,
    pub osfinger: String,
    pub osrelease: String,
    pub saltversion: String,
    pub cpuarch: String,
    pub timed_out: bool,
}

#[derive(Clone, Debug)]
pub struct FleetScan {
    pub minions: Vec<FleetMinion>,
    pub scanned_at: std::time::Instant,
}

// ─── Agent state ───────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct AgentState {
    pub connected: bool,
    pub version: Option<ConfigSnapshot>,
    pub last_error: Option<String>,
}
