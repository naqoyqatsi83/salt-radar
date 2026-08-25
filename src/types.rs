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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SparklineStyle {
    Bars,
    Dots,
}

impl Default for SparklineStyle {
    fn default() -> Self { Self::Bars }
}

impl Tab {
    pub fn next(&self) -> Self {
        match self {
            Self::Dashboard => Self::Minions,
            Self::Minions => Self::Events,
            Self::Events => Self::Jobs,
            Self::Jobs => Self::Alerts,
            Self::Alerts => Self::Pulse,
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
            Self::Pulse => Self::Alerts,
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
