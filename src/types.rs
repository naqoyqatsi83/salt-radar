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
    Master,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(Default)]
pub enum SparklineStyle {
    Bars,
    #[default]
    Dots,
}


impl Tab {
    /// The tab bar, left to right; `1`–`8` follow it.
    pub const ORDER: [Tab; 8] = [
        Tab::Dashboard, Tab::Minions, Tab::Events, Tab::Jobs, Tab::Alerts, Tab::Api, Tab::Master, Tab::Pulse,
    ];

    /// Its label in the tab bar, number included (` 7 Master Config `).
    pub fn bar_label(self) -> String {
        let n = Self::ORDER.iter().position(|&t| t == self).unwrap_or(0) + 1;
        format!(" {} {} ", n, self.label())
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Dashboard => Self::Minions,
            Self::Minions => Self::Events,
            Self::Events => Self::Jobs,
            Self::Jobs => Self::Alerts,
            Self::Alerts => Self::Api,
            Self::Api => Self::Master,
            Self::Master => Self::Pulse,
            Self::Pulse => Self::Dashboard,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            Self::Dashboard => Self::Pulse,
            Self::Pulse => Self::Master,
            Self::Minions => Self::Dashboard,
            Self::Events => Self::Minions,
            Self::Jobs => Self::Events,
            Self::Alerts => Self::Jobs,
            Self::Api => Self::Alerts,
            Self::Master => Self::Api,
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
            Self::Master => "Master Config",
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
    pub connected: bool,
    pub timed_out: bool,
}

impl MinionInfo {
    pub fn os_label(&self) -> String {
        if !self.osfinger.is_empty() {
            self.osfinger.clone()
        } else if !self.osrelease.is_empty() {
            format!("{} {}", self.os, self.osrelease)
        } else {
            self.os.clone()
        }
    }
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
    #[serde(rename = "API")]
    Api,
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
            Self::Api => "salt-api",
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
    /// The lowest auth protocol the master accepts; 0 when it predates the
    /// setting (before 3006.17 / 3007.9) and accepts any.
    pub minimum_auth_version: u64,
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

/// State of fetching the real `salt-run jobs.lookup_jid --force-color
/// --state-output=full --state-verbose=True` text for one event's pretty
/// view — the same subprocess call and flags Job Detail already uses (see
/// `cmd_jid_lookup` in the agent), so pretty-view colors/formatting are
/// authoritative Salt output instead of a hand-rolled approximation that
/// can drift from a given master's actual color scheme and doesn't respect
/// per-master output settings.
#[derive(Clone, Debug, Default)]
pub enum PrettyFetch {
    #[default]
    Idle,
    Loading,
    Ready(String),
    Failed,
}

// ─── Event sniffer state ─────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct SnifferState {
    /// The input the sniffer was started with, as typed (title, file names).
    pub query: String,
    /// What gets captured: events matching any of these (see `crate::sniff`
    /// for minion, keyword and regex terms). They also color their matches
    /// in what's shown.
    pub terms: Vec<crate::sniff::SniffTerm>,
    pub buffer: VecDeque<SaltEvent>,
    pub max_size: usize,
    pub paused: bool,
    pub selected: usize,
    /// Events display oldest-at-top, newest-at-bottom. While following,
    /// `selected` stays pinned to the newest captured event as new ones
    /// arrive (tail -f style); moving selection away from the last row
    /// turns this off so live traffic doesn't yank the selection around.
    pub following: bool,
    /// An optional, user-invoked (`h`) narrowing of what's shown: only
    /// captured events matching one of these keyword / regex terms, typed as
    /// `highlight_query`. Unlike `terms`, these filter the displayed view
    /// (see `filtered_indices`).
    pub highlight_query: String,
    pub highlight_terms: Vec<crate::sniff::SniffTerm>,
    /// When Some, the user is editing the `h` terms within the popup.
    pub highlight_input: Option<String>,
    /// Why the last `h` input wasn't applied (an invalid regex term).
    pub highlight_error: Option<String>,
}

impl SnifferState {
    /// Indices into `buffer` (raw capture, always every event matching the
    /// sniffer's minion-id filter) that also match at least one active
    /// highlight keyword — the *displayed* view. With no keywords set, every
    /// captured index is shown, same as before this existed. `selected` and
    /// scroll math key off this filtered view's length, not `buffer.len()`,
    /// so setting a highlight term narrows what's actually shown to what it
    /// matched instead of merely coloring it within an otherwise-unfiltered
    /// list — a keyword field a user reaches for expecting it to behave like
    /// a filter (find the needle) should actually filter, not just tint it.
    pub fn filtered_indices(&self) -> Vec<usize> {
        if self.highlight_terms.is_empty() {
            return (0..self.buffer.len()).collect();
        }
        self.buffer.iter().enumerate()
            .filter(|(_, evt)| crate::sniff::matches(&self.highlight_terms, evt))
            .map(|(i, _)| i)
            .collect()
    }

}

// ─── Master configuration ─────────────────────────────────────────────────

/// The master's configuration as the agent read it from disk (see
/// `cmd_master_config`), credentials already redacted.
#[derive(Clone, Debug, Default)]
pub struct MasterConfig {
    pub conf_file: String,
    /// In Salt's load order: later files override earlier ones.
    pub files: Vec<ConfigFile>,
    /// When the oldest salt-master process started (epoch secs, 0 unknown).
    pub master_started: u64,
    pub settings: Vec<ConfigSetting>,
}

#[derive(Clone, Debug, Default)]
pub struct ConfigFile {
    pub path: String,
    pub text: String,
    pub mtime: u64,
    /// Why Salt rejected the file (e.g. a key twice): none of its settings
    /// are in use, they fall back to the defaults.
    pub error: Option<String>,
    /// The line the error points at, 0 when unknown.
    pub error_line: usize,
}

impl ConfigFile {
    /// Written after salt-master started, so the running master isn't using
    /// what it says yet.
    pub fn changed_since(&self, master_started: u64) -> bool {
        master_started > 0 && self.mtime > master_started
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SettingKind {
    /// Set in a file, and one of Salt's core settings.
    Explicit,
    /// Set in a file, but not a core setting: a module's own option
    /// (`mysql.host`, `rest_cherrypy`) or a typo.
    Unknown,
    /// Only in files Salt rejected, so not in use.
    Ignored,
    /// In no file, but not Salt's default either: computed at startup.
    Derived,
    Default,
}

impl SettingKind {
    pub fn from_agent(kind: &str) -> Self {
        match kind {
            "explicit" => Self::Explicit,
            "unknown" => Self::Unknown,
            "ignored" => Self::Ignored,
            "derived" => Self::Derived,
            _ => Self::Default,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Explicit => "set",
            Self::Unknown => "not core",
            Self::Ignored => "ignored",
            Self::Derived => "derived",
            Self::Default => "default",
        }
    }
}

/// What a file's value does to the ones loaded before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceRelation {
    First,
    /// A later file's scalar (or a value of another type) wins.
    Overrides,
    /// Dicts and lists from files of one include group combine.
    Merged,
    /// A later include group (master.d over the main file) replaces the
    /// whole value, nested keys it doesn't set included.
    Replaces,
    /// In a file Salt rejected.
    Ignored,
}

impl SourceRelation {
    pub fn from_agent(relation: &str) -> Self {
        match relation {
            "overrides" => Self::Overrides,
            "merged" => Self::Merged,
            "replaces" => Self::Replaces,
            "ignored" => Self::Ignored,
            _ => Self::First,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::First => "sets it",
            Self::Overrides => "overrides the file above",
            Self::Merged => "merged with the file above",
            Self::Replaces => "replaces the whole value above",
            Self::Ignored => "ignored: Salt rejected this file",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConfigSource {
    pub file: usize,
    /// First line and end line (exclusive) of its block, 1-based; 0 when
    /// the agent couldn't place it.
    pub start: usize,
    pub end: usize,
    pub relation: SourceRelation,
    /// This file's own value (YAML).
    pub value: String,
    /// For `Replaces`: the nested keys of the value above that are lost.
    pub dropped: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ConfigSetting {
    pub key: String,
    pub kind: SettingKind,
    /// Every file that sets it, in load order.
    pub sources: Vec<ConfigSource>,
    /// The effective value on one line, and in full (YAML).
    pub line: String,
    pub full: String,
    /// Salt's default, for a core setting.
    pub default: Option<String>,
    pub redacted: bool,
    /// For an unknown key, the core setting it's closest to.
    pub suggest: Option<String>,
}

impl ConfigSetting {
    /// Set in a file written after salt-master started, so its value may
    /// change when the master restarts. (What the running master holds in
    /// memory can't be read, only when the files changed.)
    pub fn restart_pending(&self, mc: &MasterConfig) -> bool {
        self.sources.iter().any(|src| mc.files.get(src.file).is_some_and(|f| f.changed_since(mc.master_started)))
    }

    /// The file whose value is in use: the last one Salt didn't reject, or
    /// for a setting only in rejected files, the last of those.
    pub fn winner(&self) -> Option<&ConfigSource> {
        self.sources.iter().rev().find(|s| s.relation != SourceRelation::Ignored).or(self.sources.last())
    }

    /// The setting as it would be written in a config file.
    pub fn as_yaml(&self) -> String {
        if self.full.contains('\n') {
            let body: Vec<String> = self.full.lines().map(|l| format!("  {}", l)).collect();
            format!("{}:\n{}", self.key, body.join("\n"))
        } else {
            format!("{}: {}", self.key, self.full)
        }
    }
}

// ─── Salt versions ────────────────────────────────────────────────────────

/// Major and minor of a Salt version (`3006.16`, `3008.0rc1`, `2019.2.8`).
pub fn salt_major_minor(version: &str) -> Option<(u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor_text = parts.next().unwrap_or("0");
    let digits = minor_text.find(|c: char| !c.is_ascii_digit()).unwrap_or(minor_text.len());
    Some((major, minor_text[..digits].parse().unwrap_or(0)))
}

/// The request protocol a minion of this Salt version speaks, the `version`
/// in its payloads: 0 (none sent) before the March 2022 security releases,
/// 2 with their nonce and signed replies (3002.8, 3003.4, 3004.1, 3005), 3
/// with per-minion session encryption (3006.12, 3007.4, 3008). Masters from
/// 3006.17, 3007.9 and 3008 reject anything below 3 by default
/// (`minimum_auth_version: 3`).
pub fn auth_protocol(version: &str) -> Option<u64> {
    let (major, minor) = salt_major_minor(version)?;
    let first = |v2: u64, v3: u64| if minor >= v3 { 3 } else if minor >= v2 { 2 } else { 0 };
    Some(match major {
        ..3002 => 0,
        3002 => first(8, u64::MAX),
        3003 => first(4, u64::MAX),
        3004 => first(1, u64::MAX),
        3005 => 2,
        3006 => first(0, 12),
        3007 => first(0, 4),
        _ => 3,
    })
}

#[cfg(test)]
#[test]
fn auth_protocol_follows_salt_releases() {
    let p = |v| auth_protocol(v);
    assert_eq!([p("3006.11"), p("3006.12"), p("3007.3"), p("3007.4"), p("3008.0rc1")], [Some(2), Some(3), Some(2), Some(3), Some(3)]);
    assert_eq!([p("3002.7"), p("3002.8"), p("3003.3"), p("3003.4"), p("3004"), p("3004.1"), p("3005.1")], [Some(0), Some(2), Some(0), Some(2), Some(0), Some(2), Some(2)]);
    assert_eq!([p("2014.7.1"), p("2019.2.8"), p("3000.9"), p("3001.8"), p(""), p("?")], [Some(0), Some(0), Some(0), Some(0), None, None]);
    assert_eq!(salt_major_minor("3008.0rc1"), Some((3008, 0)));
    assert_eq!(["3006.16", "3006.17", "3007.8", "3007.9", "3008.0", "?"].map(enforces_minimum_auth_version), [false, true, false, true, true, false]);
}

/// Whether a master of this Salt version enforces `minimum_auth_version`
/// (from 3006.17, 3007.9 and 3008; older ones ignore the setting).
pub fn enforces_minimum_auth_version(master_version: &str) -> bool {
    salt_major_minor(master_version).is_some_and(|(major, minor)| match major {
        3006 => minor >= 17,
        3007 => minor >= 9,
        m => m >= 3008,
    })
}

// ─── Job entry (live) ─────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct JobEntry {
    pub jid: String,
    pub fun: String,
    pub tgt: String,
    pub target_count: usize,
    pub returned: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub age_seconds: u64,
    pub created_epoch: f64,
    pub local: bool,
    pub finalized: bool,
    pub args: String,
    pub test_run: bool,
    pub lag: JobLag,
}

/// A job's return times, for each minion's lag ratio (its time over the
/// job's median). Collected until the job is settled -- every target
/// returned, or `job_straggler_secs` passed -- then the median is fixed,
/// and minions returning later are rated against it.
#[derive(Clone, Debug, PartialEq)]
pub enum JobLag {
    /// (minion, seconds from the job's publish to its return)
    Collecting(Vec<(String, f64)>),
    /// The median, or None when fewer than two minions returned in time,
    /// or the job's publish time isn't known.
    Settled(Option<f64>),
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
    pub args: String,
    pub test_run: bool,
}

const STATE_APPLY_FUNS: &[&str] = &["state.apply", "state.sls", "state.highstate", "state.top"];

/// Renders a job's raw Salt `arg` list (positional values plus an optional
/// `{"__kwarg__": true, ...}` dict) for the Args column. For state runs this
/// is the requested SLS list, or "highstate" when none was named; `test=True`
/// is returned as a flag instead of text so it can be styled as a marker.
/// Must stay in sync with `_format_job_args` in the agent, which renders the
/// Job Cache side so it can be searched and sorted on the master.
pub fn format_job_args(fun: &str, arg: &serde_json::Value) -> (String, bool) {
    fn text(v: &serde_json::Value) -> String {
        match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(items) => items.iter().map(text).collect::<Vec<_>>().join(","),
            other => other.to_string(),
        }
    }
    let mut positional: Vec<String> = Vec::new();
    let mut kwargs: Vec<(&str, &serde_json::Value)> = Vec::new();
    let mut test_run = false;
    for v in arg.as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        match v.as_object() {
            Some(obj) if obj.get("__kwarg__").and_then(|k| k.as_bool()) == Some(true) => {
                for (k, val) in obj {
                    if k == "test" {
                        test_run = val.as_bool().unwrap_or_else(|| val.as_str().is_some_and(|s| s.eq_ignore_ascii_case("true")));
                    } else if !k.starts_with("__") {
                        kwargs.push((k, val));
                    }
                }
            }
            _ => positional.push(text(v)),
        }
    }
    if STATE_APPLY_FUNS.contains(&fun) {
        if let Some(pos) = kwargs.iter().position(|(k, _)| *k == "mods") {
            positional.insert(0, text(kwargs.remove(pos).1));
        }
        if positional.is_empty() && fun != "state.sls" {
            positional.push("highstate".to_string());
        }
    }
    let mut parts = positional;
    parts.extend(kwargs.into_iter().map(|(k, v)| format!("{}={}", k, text(v))));
    (parts.join(" "), test_run)
}

// ─── Table columns ────────────────────────────────────────────────────────

/// The list tables whose columns can be chosen, reordered and (except
/// Events) sorted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TableId {
    LiveJobs,
    JobCache,
    Minions,
    Events,
    Alerts,
    ApiLog,
}

impl TableId {
    /// Every column key valid for this table, in canonical order.
    pub fn all_columns(self) -> &'static [&'static str] {
        match self {
            Self::LiveJobs => &["jid", "fun", "args", "target", "source", "tgt", "ok", "fail", "wait", "started"],
            Self::JobCache => &["jid", "user", "started", "target", "fun", "args"],
            Self::Minions => &["id", "key", "os", "salt", "proto", "arch", "seen", "fail_pct", "lag", "ping"],
            Self::Events => &["local", "utc", "tag", "minion"],
            Self::Alerts => &["severity", "rule", "description", "since", "count"],
            Self::ApiLog => &["age", "method", "client", "path", "status", "time"],
        }
    }

    pub fn default_columns(self) -> &'static str {
        match self {
            Self::LiveJobs => "jid,fun,args,target,source,tgt,ok,fail,wait",
            Self::JobCache => "jid,user,started,target,fun,args",
            Self::Minions => "id,key,os,salt",
            Self::Events => "local,utc,tag",
            Self::Alerts => "severity,rule,description",
            Self::ApiLog => "age,method,client,path,status,time",
        }
    }

    /// The config key (and Config Editor label) holding this table's columns.
    pub fn config_key(self) -> &'static str {
        match self {
            Self::LiveJobs => "live_columns",
            Self::JobCache => "cache_columns",
            Self::Minions => "minion_columns",
            Self::Events => "event_columns",
            Self::Alerts => "alert_columns",
            Self::ApiLog => "request_columns",
        }
    }

    /// Events is a live tail that follows the newest event; reordering it
    /// would fight that, so it only gets column choice.
    pub fn sortable(self) -> bool {
        self != Self::Events
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Jid,
    Fun,
    Args,
    Target,
    Source,
    Targeted,
    Ok,
    Fail,
    Wait,
    User,
    Started,
    MinionId,
    KeyStatus,
    Os,
    SaltVersion,
    Proto,
    Arch,
    LastSeen,
    FailPct,
    Lag,
    Ping,
    Local,
    Utc,
    Tag,
    EventMinion,
    Severity,
    Rule,
    Description,
    Since,
    Count,
    Age,
    Method,
    Client,
    Path,
    Status,
    Time,
}

const COLUMN_KEYS: &[(&str, Column)] = &[
    ("jid", Column::Jid),
    ("fun", Column::Fun),
    ("args", Column::Args),
    ("target", Column::Target),
    ("source", Column::Source),
    ("tgt", Column::Targeted),
    ("ok", Column::Ok),
    ("fail", Column::Fail),
    ("wait", Column::Wait),
    ("user", Column::User),
    ("started", Column::Started),
    ("id", Column::MinionId),
    ("key", Column::KeyStatus),
    ("os", Column::Os),
    ("salt", Column::SaltVersion),
    ("proto", Column::Proto),
    ("arch", Column::Arch),
    ("seen", Column::LastSeen),
    ("fail_pct", Column::FailPct),
    ("lag", Column::Lag),
    ("ping", Column::Ping),
    ("local", Column::Local),
    ("utc", Column::Utc),
    ("tag", Column::Tag),
    ("minion", Column::EventMinion),
    ("severity", Column::Severity),
    ("rule", Column::Rule),
    ("description", Column::Description),
    ("since", Column::Since),
    ("count", Column::Count),
    ("age", Column::Age),
    ("method", Column::Method),
    ("client", Column::Client),
    ("path", Column::Path),
    ("status", Column::Status),
    ("time", Column::Time),
];

impl Column {
    pub fn from_key(key: &str) -> Option<Self> {
        COLUMN_KEYS.iter().find(|(k, _)| *k == key).map(|(_, c)| *c)
    }

    pub fn key(self) -> &'static str {
        COLUMN_KEYS.iter().find(|(_, c)| *c == self).map(|(k, _)| *k).unwrap_or("")
    }

    pub fn header(self) -> &'static str {
        match self {
            Self::Jid => "JID",
            Self::Fun => "Fun",
            Self::Args => "Args",
            Self::Target => "Target",
            Self::Source => "Source",
            Self::Targeted => "Tgt",
            Self::Ok => "OK",
            Self::Fail => "Fail",
            Self::Wait => "Wait",
            Self::User => "User",
            Self::Started => "Started",
            Self::MinionId => "ID",
            Self::KeyStatus => "Key",
            Self::Os => "OS",
            Self::SaltVersion => "Salt",
            Self::Arch => "Arch",
            Self::LastSeen => "Seen",
            Self::FailPct => "Fail%",
            Self::Lag => "Lag",
            Self::Proto => "Proto",
            Self::Ping => "Ping",
            Self::Local => "Local",
            Self::Utc => "UTC",
            Self::Tag => "Tag",
            Self::EventMinion => "Minion",
            Self::Severity => "Severity",
            Self::Rule => "Rule",
            Self::Description => "Description",
            Self::Since => "Since",
            Self::Count => "Count",
            Self::Age => "Age",
            Self::Method => "Method",
            Self::Client => "Client",
            Self::Path => "Path",
            Self::Status => "Status",
            Self::Time => "Time",
        }
    }

    /// Cell width including the one-space gap that follows it.
    pub fn width(self) -> usize {
        match self {
            Self::Fun | Self::Args | Self::Os => 24,
            Self::Jid | Self::Target | Self::Client | Self::Path | Self::Local | Self::Utc
                | Self::MinionId | Self::EventMinion | Self::Started => 21,
            Self::Source | Self::Method | Self::Time => 8,
            Self::Targeted | Self::Ok | Self::Fail | Self::Wait | Self::Count => 6,
            Self::FailPct | Self::Lag | Self::Proto => 7,
            Self::Ping => 9,
            Self::User => 13,
            Self::KeyStatus | Self::Severity | Self::Arch => 10,
            Self::LastSeen | Self::Since => 13,
            Self::SaltVersion => 12,
            Self::Tag | Self::Description => 46,
            Self::Rule => 23,
            Self::Status => 19,
            Self::Age => 17,
        }
    }

    /// Narrowest a free-text column may be squeezed to when the panel can't
    /// fit every shown column at full width; `None` for fixed-format columns
    /// (JIDs, timestamps, counts) that would be unreadable truncated.
    pub fn min_width(self) -> Option<usize> {
        match self {
            Self::Fun | Self::Args | Self::Tag | Self::Description => Some(12),
            Self::Target | Self::Rule | Self::Os | Self::MinionId | Self::EventMinion | Self::Client | Self::Path => Some(10),
            Self::User | Self::Status | Self::SaltVersion => Some(6),
            _ => None,
        }
    }

    pub fn right_aligned(self) -> bool {
        matches!(self, Self::Targeted | Self::Ok | Self::Fail | Self::Wait | Self::Count | Self::Time | Self::FailPct | Self::Lag | Self::Ping | Self::Proto)
    }

    /// Parses a comma-separated column list from the config, keeping only
    /// keys valid for `table` and dropping duplicates. Falls back to the
    /// table's defaults if nothing usable is left, so a table can never end
    /// up with no columns at all.
    pub fn parse_list(value: &str, table: TableId) -> Vec<Self> {
        let parse = |s: &str| {
            let mut cols: Vec<Self> = Vec::new();
            for key in s.split(',').map(str::trim) {
                if let Some(col) = Self::from_key(key).filter(|c| table.all_columns().contains(&c.key())) {
                    if !cols.contains(&col) { cols.push(col); }
                }
            }
            cols
        };
        let cols = parse(value);
        if cols.is_empty() { parse(table.default_columns()) } else { cols }
    }
}

/// Active sort of a table: column and whether it's descending. No entry on
/// the App side means the table's natural order.
pub type SortSpec = (Column, bool);

/// Screen span of one rendered column header, recorded each frame so a
/// header click hit-tests against what was actually drawn.
#[derive(Clone, Copy, Debug)]
pub struct HeaderHit {
    pub y: u16,
    pub x0: u16,
    pub x1: u16,
    pub col: Column,
    pub table: TableId,
}

/// Where the Job Detail popup's last frame put its returns list (each
/// visible row's screen y and return index) and its preview panel, so a
/// click hit-tests against what was actually drawn.
#[derive(Clone, Debug, Default)]
pub struct JobDetailHits {
    pub list: ratatui::layout::Rect,
    pub rows: Vec<(u16, usize)>,
    pub preview: ratatui::layout::Rect,
}

// ─── JID lookup result ────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct JidLookup {
    pub found: bool,
    pub fun: String,
    pub tgt: String,
    pub start_time: String,
    pub returns: Vec<JidReturn>,
    /// Targeted minions that haven't returned. Job Detail lists them after
    /// the returns, so entry `i` past `returns.len()` is one of these.
    pub missing: Vec<String>,
    /// How long the agent's `salt-run jobs.lookup_jid` subprocess call and
    /// its own post-processing each took — shown in the popup header so a
    /// slow lookup is visible right there instead of needing to go dig
    /// through the agent's stderr log.
    pub subprocess_secs: f64,
    pub postprocess_secs: f64,
    /// Why the lookup failed (salt-run error, timeout), shown in place of
    /// the return list instead of a misleading "No returns yet."
    pub error: String,
}

impl JidLookup {
    pub fn failed(error: String) -> Self {
        Self {
            found: false,
            fun: String::new(),
            tgt: String::new(),
            start_time: String::new(),
            returns: Vec::new(),
            missing: Vec::new(),
            subprocess_secs: 0.0,
            postprocess_secs: 0.0,
            error,
        }
    }

    /// Rows in Job Detail's returns list: the returns, then the missing minions.
    pub fn entry_count(&self) -> usize {
        self.returns.len() + self.missing.len()
    }

    pub fn entry_id(&self, i: usize) -> Option<&str> {
        match self.returns.get(i) {
            Some(r) => Some(&r.minion_id),
            None => self.missing.get(i - self.returns.len()).map(String::as_str),
        }
    }
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
