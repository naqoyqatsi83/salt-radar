use std::path::{Path, PathBuf};

use ratatui::style::Color;

use crate::types::TableId;
use crate::ui::color::Theme;

/// Where `salt-radar.toml` lives: a `salt-radar.toml` in the current
/// directory wins if present (lets you keep a per-directory config, and
/// preserves behavior for anyone already relying on the old cwd-only
/// lookup); otherwise it resolves to the XDG config dir
/// (`$XDG_CONFIG_HOME/salt-radar/salt-radar.toml`, falling back to
/// `~/.config/salt-radar/salt-radar.toml`) instead of silently writing
/// into whatever directory the binary happened to be launched from.
pub fn resolve_config_path() -> PathBuf {
    let cwd_path = PathBuf::from("salt-radar.toml");
    if cwd_path.is_file() {
        return cwd_path;
    }

    match user_config_dir() {
        Some(dir) => dir.join("salt-radar.toml"),
        None => cwd_path,
    }
}

/// `$XDG_CONFIG_HOME/salt-radar`, falling back to `~/.config/salt-radar`.
fn user_config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|dir| dir.join("salt-radar"))
}

pub const DEFAULT_THEME: &str = "Default";

/// Where `*.theme` files are looked up, lowest priority first: the
/// packaged themes, then the user's own (which win on a name clash).
fn theme_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/usr/share/salt-radar/themes")];
    dirs.extend(user_config_dir().map(|d| d.join("themes")));
    dirs
}

/// Names selectable as `color_theme`: the built-in `Default`, then every
/// `*.theme` file found in `theme_dirs`, by file stem.
pub fn theme_names() -> Vec<String> {
    let mut names: Vec<String> = theme_dirs().iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()? != "theme" { return None; }
            Some(path.file_stem()?.to_string_lossy().into_owned())
        })
        .filter(|name| name != DEFAULT_THEME)
        .collect();
    names.sort();
    names.dedup();
    names.insert(0, DEFAULT_THEME.to_string());
    names
}

/// Resolves `color_theme` — a theme name from `theme_names`, or a path to a
/// `.theme` file — onto the built-in default; keys the file doesn't set (or
/// sets to something unparseable) keep their default color, and an unknown
/// theme is just the default.
pub fn load_theme(name: &str) -> Theme {
    let mut theme = Theme::DEFAULT;
    if name == DEFAULT_THEME { return theme; }
    let path = if name.contains('/') || name.ends_with(".theme") {
        Some(PathBuf::from(name))
    } else {
        theme_dirs().into_iter().rev()
            .map(|dir| dir.join(format!("{}.theme", name)))
            .find(|p| p.is_file())
    };
    let Some(data) = path.and_then(|p| std::fs::read_to_string(p).ok()) else { return theme };
    for line in data.lines() {
        let Some((key, value)) = parse_theme_line(line) else { continue };
        if let (Some(slot), Some(color)) = (theme.slot(key), parse_theme_color(value)) {
            *slot = color;
        }
    }
    theme
}

/// `theme[key]="value"` (btop's format); anything else is skipped.
fn parse_theme_line(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim().strip_prefix("theme[")?;
    let (key, rest) = rest.split_once(']')?;
    let value = rest.trim_start().strip_prefix('=')?.trim();
    Some((key.trim(), value.trim_matches('"')))
}

/// `#RRGGBB`, `#RR` (a gray level), or decimal `R G B`; empty means the
/// terminal's own default color.
fn parse_theme_color(value: &str) -> Option<Color> {
    let value = value.trim();
    if value.is_empty() { return Some(Color::Reset); }
    if let Some(hex) = value.strip_prefix('#') {
        if !hex.is_ascii() { return None; }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        return match hex.len() {
            2 => byte(0).map(|v| Color::Rgb(v, v, v)),
            6 => Some(Color::Rgb(byte(0)?, byte(2)?, byte(4)?)),
            _ => None,
        };
    }
    let rgb: Vec<u8> = value.split_whitespace().map(|v| v.parse().ok()).collect::<Option<_>>()?;
    match rgb[..] {
        [r, g, b] => Some(Color::Rgb(r, g, b)),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub master_config: String,
    pub python_bin: String,
    pub agent_path: String,
    pub dashboard_secs: u64,
    pub minions_secs: u64,
    pub config_secs: u64,
    pub event_buffer_size: usize,
    pub sniffer_buffer_size: usize,
    pub stale_minion_secs: u64,
    pub pending_key_secs: u64,
    pub job_straggler_secs: u64,
    pub disk_usage_pct: f64,
    pub auth_storm_rate_per_sec: f64,
    pub worker_saturation_pct: f64,
    pub pending_key_churn_per_min: u64,
    pub service_flap_threshold_24h: u64,
    pub service_mem_growth_mb_per_hour: f64,
    pub service_mem_abs_threshold_mb: f64,
    pub service_mem_pct_threshold: f64,
    /// p95 POST /login processing time (seconds) that triggers the "API auth
    /// latency" alert. Only meaningful once the salt-api panel has real
    /// data — see App::api_login_latency_p50_p95.
    pub api_auth_latency_warn_secs: f64,
    /// Rolling window (in observed requests) backing the salt-api panel's
    /// error-rate and POST /login p50/p95 latency figures — see
    /// App::api_login_latency_p50_p95.
    pub api_stats_window: usize,
    /// Max entries kept in the API tab's live request log before the oldest
    /// are evicted, independent of the age-based prune below.
    pub api_log_cap: usize,
    /// Entries older than this (seconds) are dropped from the API tab's
    /// request log every poll regardless of how far under api_log_cap it is
    /// — a quiet API shouldn't let the log hold onto hours-old requests.
    pub api_log_max_age_secs: u64,
    /// Whether salt-radar's own `GET /stats` polling (the request that
    /// populates this very log) is filtered out of the API tab's request
    /// log. Defaults on since polling at the Dashboard refresh interval
    /// would otherwise drown out real traffic with one entry per tick; set
    /// to false to see it anyway.
    pub api_hide_stats_polls: bool,
    /// How the agent authenticates to salt-api's `/stats` endpoint:
    /// "none" (default) relies on `stats_disable_auth: True` in the master
    /// config, so `/stats` needs no credentials at all — see the README's
    /// security note on that tradeoff. "plaintext" logs in with
    /// `api_stats_auth_username`/`api_stats_auth_password` read straight
    /// from this config file (marked insecure — stored and transmitted to
    /// the agent as plain text). "pillar" resolves those same two values,
    /// once and then cached, from `api_stats_auth_username_pillar_path`/
    /// `api_stats_auth_password_pillar_path` — like two independent
    /// `pillar.get` calls, entirely master-side for this master's own local
    /// minion (no live round-trip, and the credential never passes through
    /// this config file or the Rust process at all) — see the doc comment
    /// on the path fields below.
    pub api_stats_auth_mode: String,
    /// eauth backend the login uses (pam, ldap, file, ...) — see Salt's
    /// `external_auth` docs. Only meaningful when api_stats_auth_mode is
    /// "plaintext" or "pillar".
    pub api_stats_auth_eauth: String,
    /// Plaintext credentials for api_stats_auth_mode = "plaintext". See the
    /// mode's own doc comment above for the security tradeoff.
    pub api_stats_auth_username: String,
    pub api_stats_auth_password: String,
    /// Colon-delimited pillar paths resolving directly to the username/
    /// password strings, for api_stats_auth_mode = "pillar" — e.g.
    /// "salt_radar:api_user" for a pillar shaped `salt_radar: {api_user:
    /// ...}`. Compiled for this master's own local minion ID (the same one
    /// `salt-call pillar.get` would use on this box) — not something you
    /// configure here, and requires a minion co-located with the master. A
    /// `top.sls` entry keyed to that minion's own ID (not `'*'`, which
    /// would hand the credential to every minion) is all that's needed.
    pub api_stats_auth_username_pillar_path: String,
    pub api_stats_auth_password_pillar_path: String,
    /// Per-unit overrides for the standard bundled Salt services — each 0.0
    /// means "no override, fall back to the global service_mem_*_threshold*
    /// above" (the global fields already use 0.0 to mean "check disabled",
    /// so these reuse that same sentinel to mean "not set").
    pub service_mem_abs_threshold_master_mb: f64,
    pub service_mem_pct_threshold_master: f64,
    pub service_mem_abs_threshold_minion_mb: f64,
    pub service_mem_pct_threshold_minion: f64,
    pub service_mem_abs_threshold_api_mb: f64,
    pub service_mem_pct_threshold_api: f64,
    pub service_mem_abs_threshold_syndic_mb: f64,
    pub service_mem_pct_threshold_syndic: f64,
    /// Which mechanism computes the Dashboard ZMQ panel's latency figure:
    /// "agent_ipc" (local Rust<->Python subprocess round-trip — a liveness
    /// check, not real Salt transport), "job_ping" (periodic async test.ping,
    /// timed from dispatch to first real return over the actual event bus),
    /// or "master_stats" (Salt's own master_stats instrumentation, requires
    /// master_stats: True in the master's own config). Unrecognized values
    /// fall back to "agent_ipc" behavior.
    pub zmq_latency_source: String,
    /// Latency (ms) at the top of the ZMQ graph's log scale — drawn red, and
    /// anything slower is clipped there. 0 means the default for the
    /// configured zmq_latency_source (see `zmq_latency_ceiling`).
    pub zmq_latency_ceiling_ms: f64,
    /// Which of the Dashboard's bottom sparkline panels are shown. ZMQ /
    /// Events / Activity default on (today's fixed set); CPU / Memory
    /// default off since enabling them changes the existing layout's panel
    /// count — opt-in rather than a default appearance change.
    pub show_zmq_graph: bool,
    pub show_events_graph: bool,
    pub show_activity_graph: bool,
    pub show_cpu_graph: bool,
    pub show_mem_graph: bool,
    /// Stacking order of the Dashboard's sparkline panels, as a comma-
    /// separated list of panel keys ("cpu", "mem", "zmq", "events",
    /// "activity"). Unrecognized/duplicate tokens are ignored; any panel
    /// missing from the list is appended after the ones named, so a partial
    /// or malformed value degrades gracefully instead of losing a panel.
    pub graph_order: String,
    /// Which of the Dashboard's fixed (non-sparkline) panels are shown, all
    /// defaulting on so existing configs/behavior are unaffected. salt-api
    /// still only ever renders when the process is actually detected —
    /// this just adds a user opt-out on top of that.
    pub show_host_resources: bool,
    pub show_services: bool,
    pub show_key_status: bool,
    pub show_event_types: bool,
    pub show_job_pulse: bool,
    pub show_salt_api: bool,
    /// Whether the Minions tab's Fleet Overview breakdown (OS/version/arch)
    /// scopes itself to the active list filter, or always aggregates the
    /// whole fleet regardless of what's filtered on the left. Toggled live
    /// with `f` on the Minions tab; this is just the startup default.
    pub minions_follow_filter: bool,
    /// Default directory `s`/`S` ("save to file") writes into, from any
    /// popup that supports it. `~` (or `~/...`) is expanded against $HOME
    /// at save time, not at load time, so changing $HOME between launches
    /// (or editing this value live in the Config Editor) takes effect
    /// immediately.
    pub save_dir: String,
    /// How long (seconds) a Job Detail lookup (`salt-run jobs.lookup_jid`)
    /// may run before it's reported as timed out. Huge jobs (e.g.
    /// `grains.items` across thousands of minions) legitimately take tens of
    /// seconds or more to read back from the job cache.
    pub lookup_timeout_secs: u64,
    /// Which columns each list table shows, and in what order, as comma-
    /// separated column keys (see types::TableId::all_columns). Any valid
    /// key left out is hidden.
    pub jobs_live_columns: String,
    pub jobs_cache_columns: String,
    pub minion_columns: String,
    pub event_columns: String,
    pub alert_columns: String,
    pub api_request_columns: String,
    /// Theme name (see `theme_names`) or path to a `.theme` file.
    pub color_theme: String,
    /// When false, the theme's screen backgrounds are left to the
    /// terminal (transparent); popups and the cursor keep theirs.
    pub theme_background: bool,
}

impl AppConfig {
    /// The ZMQ graph's top-of-scale latency: `zmq_latency_ceiling_ms`, or
    /// when unset, a default sized to what the source measures — a local
    /// agent round trip is a few ms when healthy, a real test.ping over the
    /// bus or Salt's own command timings are hundreds.
    pub fn zmq_latency_ceiling(&self) -> f64 {
        if self.zmq_latency_ceiling_ms > 0.0 {
            return self.zmq_latency_ceiling_ms;
        }
        match self.zmq_latency_source.as_str() {
            "job_ping" => 1000.0,
            "master_stats" => 500.0,
            _ => 100.0,
        }
    }

    pub fn columns_for(&self, table: TableId) -> &str {
        match table {
            TableId::LiveJobs => &self.jobs_live_columns,
            TableId::JobCache => &self.jobs_cache_columns,
            TableId::Minions => &self.minion_columns,
            TableId::Events => &self.event_columns,
            TableId::Alerts => &self.alert_columns,
            TableId::ApiLog => &self.api_request_columns,
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            master_config: "/etc/salt/master".into(),
            python_bin: "/opt/saltstack/salt/bin/python3".into(),
            agent_path: "/usr/share/salt-radar/salt_radar_agent.py".into(),
            dashboard_secs: 2,
            minions_secs: 5,
            config_secs: 60,
            event_buffer_size: 10000,
            sniffer_buffer_size: 1000,
            stale_minion_secs: 3600,
            pending_key_secs: 3600,
            job_straggler_secs: 120,
            disk_usage_pct: 85.0,
            auth_storm_rate_per_sec: 5.0,
            worker_saturation_pct: 85.0,
            pending_key_churn_per_min: 10,
            service_flap_threshold_24h: 3,
            service_mem_growth_mb_per_hour: 50.0,
            service_mem_abs_threshold_mb: 2048.0,
            service_mem_pct_threshold: 8.0,
            api_auth_latency_warn_secs: 3.0,
            api_stats_window: 200,
            api_log_cap: 1000,
            api_log_max_age_secs: 3600,
            api_hide_stats_polls: true,
            api_stats_auth_mode: "none".into(),
            api_stats_auth_eauth: "pam".into(),
            api_stats_auth_username: String::new(),
            api_stats_auth_password: String::new(),
            api_stats_auth_username_pillar_path: String::new(),
            api_stats_auth_password_pillar_path: String::new(),
            service_mem_abs_threshold_master_mb: 0.0,
            service_mem_pct_threshold_master: 0.0,
            service_mem_abs_threshold_minion_mb: 0.0,
            service_mem_pct_threshold_minion: 0.0,
            service_mem_abs_threshold_api_mb: 0.0,
            service_mem_pct_threshold_api: 0.0,
            service_mem_abs_threshold_syndic_mb: 0.0,
            service_mem_pct_threshold_syndic: 0.0,
            zmq_latency_source: "agent_ipc".into(),
            zmq_latency_ceiling_ms: 0.0,
            show_zmq_graph: true,
            show_events_graph: true,
            show_activity_graph: true,
            show_cpu_graph: true,
            show_mem_graph: true,
            graph_order: "cpu,mem,zmq,events,activity".into(),
            show_host_resources: true,
            show_services: true,
            show_key_status: true,
            show_event_types: true,
            show_job_pulse: true,
            show_salt_api: true,
            minions_follow_filter: true,
            save_dir: "~".into(),
            lookup_timeout_secs: 300,
            jobs_live_columns: TableId::LiveJobs.default_columns().into(),
            jobs_cache_columns: TableId::JobCache.default_columns().into(),
            minion_columns: TableId::Minions.default_columns().into(),
            event_columns: TableId::Events.default_columns().into(),
            alert_columns: TableId::Alerts.default_columns().into(),
            api_request_columns: TableId::ApiLog.default_columns().into(),
            color_theme: DEFAULT_THEME.into(),
            theme_background: true,
        }
    }
}

#[derive(serde::Deserialize)]
struct TomlConfig {
    salt: Option<SaltConfig>,
    refresh: Option<RefreshConfig>,
    events: Option<EventsConfig>,
    alerts: Option<AlertsConfig>,
    dashboard: Option<DashboardConfig>,
    api: Option<ApiConfig>,
    minions: Option<MinionsConfig>,
    export: Option<ExportConfig>,
    jobs: Option<JobsConfig>,
    ui: Option<UiConfig>,
}

#[derive(serde::Deserialize)]
struct SaltConfig {
    master_config: Option<String>,
    python_bin: Option<String>,
    agent_path: Option<String>,
}

#[derive(serde::Deserialize)]
struct RefreshConfig {
    dashboard_secs: Option<u64>,
    minions_secs: Option<u64>,
    config_secs: Option<u64>,
}

#[derive(serde::Deserialize)]
struct EventsConfig {
    buffer_size: Option<usize>,
    sniffer_buffer_size: Option<usize>,
    event_columns: Option<String>,
}

#[derive(serde::Deserialize)]
struct AlertsConfig {
    stale_minion_secs: Option<u64>,
    pending_key_secs: Option<u64>,
    job_straggler_secs: Option<u64>,
    disk_usage_pct: Option<f64>,
    auth_storm_rate_per_sec: Option<f64>,
    worker_saturation_pct: Option<f64>,
    pending_key_churn_per_min: Option<u64>,
    service_flap_threshold_24h: Option<u64>,
    service_mem_growth_mb_per_hour: Option<f64>,
    service_mem_abs_threshold_mb: Option<f64>,
    service_mem_pct_threshold: Option<f64>,
    api_auth_latency_warn_secs: Option<f64>,
    service_mem_abs_threshold_master_mb: Option<f64>,
    service_mem_pct_threshold_master: Option<f64>,
    service_mem_abs_threshold_minion_mb: Option<f64>,
    service_mem_pct_threshold_minion: Option<f64>,
    service_mem_abs_threshold_api_mb: Option<f64>,
    service_mem_pct_threshold_api: Option<f64>,
    service_mem_abs_threshold_syndic_mb: Option<f64>,
    service_mem_pct_threshold_syndic: Option<f64>,
    alert_columns: Option<String>,
}

#[derive(serde::Deserialize)]
struct ApiConfig {
    stats_window: Option<usize>,
    log_cap: Option<usize>,
    log_max_age_secs: Option<u64>,
    hide_stats_polls: Option<bool>,
    stats_auth_mode: Option<String>,
    stats_auth_eauth: Option<String>,
    stats_auth_username: Option<String>,
    stats_auth_password: Option<String>,
    stats_auth_username_pillar_path: Option<String>,
    stats_auth_password_pillar_path: Option<String>,
    request_columns: Option<String>,
}

#[derive(serde::Deserialize)]
struct MinionsConfig {
    follow_filter: Option<bool>,
    minion_columns: Option<String>,
}

#[derive(serde::Deserialize)]
struct ExportConfig {
    save_dir: Option<String>,
}

#[derive(serde::Deserialize)]
struct JobsConfig {
    lookup_timeout_secs: Option<u64>,
    live_columns: Option<String>,
    cache_columns: Option<String>,
}

#[derive(serde::Deserialize)]
struct UiConfig {
    color_theme: Option<String>,
    theme_background: Option<bool>,
}

#[derive(serde::Deserialize)]
struct DashboardConfig {
    zmq_latency_source: Option<String>,
    zmq_latency_ceiling_ms: Option<f64>,
    show_zmq_graph: Option<bool>,
    show_events_graph: Option<bool>,
    show_activity_graph: Option<bool>,
    show_cpu_graph: Option<bool>,
    show_mem_graph: Option<bool>,
    graph_order: Option<String>,
    show_host_resources: Option<bool>,
    show_services: Option<bool>,
    show_key_status: Option<bool>,
    show_event_types: Option<bool>,
    show_job_pulse: Option<bool>,
    show_salt_api: Option<bool>,
}

pub fn write_config(path: &Path, config: &AppConfig) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let toml = format!(
        "[salt]\n\
         agent_path = {:?}\n\
         python_bin = {:?}\n\
         master_config = {:?}\n\
         \n\
         [refresh]\n\
         dashboard_secs = {}\n\
         minions_secs = {}\n\
         config_secs = {}\n\
         \n\
         [events]\n\
         buffer_size = {}\n\
         sniffer_buffer_size = {}\n\
         event_columns = {:?}\n\
         \n\
         [alerts]\n\
         stale_minion_secs = {}\n\
         pending_key_secs = {}\n\
         job_straggler_secs = {}\n\
         disk_usage_pct = {}\n\
         auth_storm_rate_per_sec = {}\n\
         worker_saturation_pct = {}\n\
         pending_key_churn_per_min = {}\n\
         service_flap_threshold_24h = {}\n\
         service_mem_growth_mb_per_hour = {}\n\
         service_mem_abs_threshold_mb = {}\n\
         service_mem_pct_threshold = {}\n\
         api_auth_latency_warn_secs = {}\n\
         service_mem_abs_threshold_master_mb = {}\n\
         service_mem_pct_threshold_master = {}\n\
         service_mem_abs_threshold_minion_mb = {}\n\
         service_mem_pct_threshold_minion = {}\n\
         service_mem_abs_threshold_api_mb = {}\n\
         service_mem_pct_threshold_api = {}\n\
         service_mem_abs_threshold_syndic_mb = {}\n\
         service_mem_pct_threshold_syndic = {}\n\
         alert_columns = {:?}\n\
         \n\
         [api]\n\
         stats_window = {}\n\
         log_cap = {}\n\
         log_max_age_secs = {}\n\
         hide_stats_polls = {}\n\
         stats_auth_mode = {:?}\n\
         stats_auth_eauth = {:?}\n\
         stats_auth_username = {:?}\n\
         stats_auth_password = {:?}\n\
         stats_auth_username_pillar_path = {:?}\n\
         stats_auth_password_pillar_path = {:?}\n\
         request_columns = {:?}\n\
         \n\
         [dashboard]\n\
         zmq_latency_source = {:?}\n\
         zmq_latency_ceiling_ms = {}\n\
         show_zmq_graph = {}\n\
         show_events_graph = {}\n\
         show_activity_graph = {}\n\
         show_cpu_graph = {}\n\
         show_mem_graph = {}\n\
         graph_order = {:?}\n\
         show_host_resources = {}\n\
         show_services = {}\n\
         show_key_status = {}\n\
         show_event_types = {}\n\
         show_job_pulse = {}\n\
         show_salt_api = {}\n\
         \n\
         [minions]\n\
         follow_filter = {}\n\
         minion_columns = {:?}\n\
         \n\
         [export]\n\
         save_dir = {:?}\n\
         \n\
         [jobs]\n\
         lookup_timeout_secs = {}\n\
         live_columns = {:?}\n\
         cache_columns = {:?}\n\
         \n\
         [ui]\n\
         color_theme = {:?}\n\
         theme_background = {}\n",
        config.agent_path,
        config.python_bin,
        config.master_config,
        config.dashboard_secs,
        config.minions_secs,
        config.config_secs,
        config.event_buffer_size,
        config.sniffer_buffer_size,
        config.event_columns,
        config.stale_minion_secs,
        config.pending_key_secs,
        config.job_straggler_secs,
        config.disk_usage_pct,
        config.auth_storm_rate_per_sec,
        config.worker_saturation_pct,
        config.pending_key_churn_per_min,
        config.service_flap_threshold_24h,
        config.service_mem_growth_mb_per_hour,
        config.service_mem_abs_threshold_mb,
        config.service_mem_pct_threshold,
        config.api_auth_latency_warn_secs,
        config.service_mem_abs_threshold_master_mb,
        config.service_mem_pct_threshold_master,
        config.service_mem_abs_threshold_minion_mb,
        config.service_mem_pct_threshold_minion,
        config.service_mem_abs_threshold_api_mb,
        config.service_mem_pct_threshold_api,
        config.service_mem_abs_threshold_syndic_mb,
        config.service_mem_pct_threshold_syndic,
        config.alert_columns,
        config.api_stats_window,
        config.api_log_cap,
        config.api_log_max_age_secs,
        config.api_hide_stats_polls,
        config.api_stats_auth_mode,
        config.api_stats_auth_eauth,
        config.api_stats_auth_username,
        config.api_stats_auth_password,
        config.api_stats_auth_username_pillar_path,
        config.api_stats_auth_password_pillar_path,
        config.api_request_columns,
        config.zmq_latency_source,
        config.zmq_latency_ceiling_ms,
        config.show_zmq_graph,
        config.show_events_graph,
        config.show_activity_graph,
        config.show_cpu_graph,
        config.show_mem_graph,
        config.graph_order,
        config.show_host_resources,
        config.show_services,
        config.show_key_status,
        config.show_event_types,
        config.show_job_pulse,
        config.show_salt_api,
        config.minions_follow_filter,
        config.minion_columns,
        config.save_dir,
        config.lookup_timeout_secs,
        config.jobs_live_columns,
        config.jobs_cache_columns,
        config.color_theme,
        config.theme_background,
    );
    std::fs::write(path, toml)
}

pub fn load_config(path: &Path) -> AppConfig {
    let mut cfg = AppConfig::default();
    let data = std::fs::read_to_string(path).ok();
    if let Some(data) = data {
        if let Ok(toml_cfg) = toml::from_str::<TomlConfig>(&data) {
            if let Some(s) = toml_cfg.salt {
                if let Some(v) = s.master_config { cfg.master_config = v; }
                if let Some(v) = s.python_bin { cfg.python_bin = v; }
                if let Some(v) = s.agent_path { cfg.agent_path = v; }
            }
            if let Some(r) = toml_cfg.refresh {
                if let Some(v) = r.dashboard_secs { cfg.dashboard_secs = v; }
                if let Some(v) = r.minions_secs { cfg.minions_secs = v; }
                if let Some(v) = r.config_secs { cfg.config_secs = v; }
            }
            if let Some(e) = toml_cfg.events {
                if let Some(v) = e.buffer_size { cfg.event_buffer_size = v; }
                if let Some(v) = e.sniffer_buffer_size { cfg.sniffer_buffer_size = v; }
                if let Some(v) = e.event_columns { cfg.event_columns = v; }
            }
            if let Some(a) = toml_cfg.alerts {
                if let Some(v) = a.stale_minion_secs { cfg.stale_minion_secs = v; }
                if let Some(v) = a.pending_key_secs { cfg.pending_key_secs = v; }
                if let Some(v) = a.job_straggler_secs { cfg.job_straggler_secs = v; }
                if let Some(v) = a.disk_usage_pct { cfg.disk_usage_pct = v; }
                if let Some(v) = a.auth_storm_rate_per_sec { cfg.auth_storm_rate_per_sec = v; }
                if let Some(v) = a.worker_saturation_pct { cfg.worker_saturation_pct = v; }
                if let Some(v) = a.pending_key_churn_per_min { cfg.pending_key_churn_per_min = v; }
                if let Some(v) = a.service_flap_threshold_24h { cfg.service_flap_threshold_24h = v; }
                if let Some(v) = a.service_mem_growth_mb_per_hour { cfg.service_mem_growth_mb_per_hour = v; }
                if let Some(v) = a.service_mem_abs_threshold_mb { cfg.service_mem_abs_threshold_mb = v; }
                if let Some(v) = a.service_mem_pct_threshold { cfg.service_mem_pct_threshold = v; }
                if let Some(v) = a.api_auth_latency_warn_secs { cfg.api_auth_latency_warn_secs = v; }
                if let Some(v) = a.service_mem_abs_threshold_master_mb { cfg.service_mem_abs_threshold_master_mb = v; }
                if let Some(v) = a.service_mem_pct_threshold_master { cfg.service_mem_pct_threshold_master = v; }
                if let Some(v) = a.service_mem_abs_threshold_minion_mb { cfg.service_mem_abs_threshold_minion_mb = v; }
                if let Some(v) = a.service_mem_pct_threshold_minion { cfg.service_mem_pct_threshold_minion = v; }
                if let Some(v) = a.service_mem_abs_threshold_api_mb { cfg.service_mem_abs_threshold_api_mb = v; }
                if let Some(v) = a.service_mem_pct_threshold_api { cfg.service_mem_pct_threshold_api = v; }
                if let Some(v) = a.service_mem_abs_threshold_syndic_mb { cfg.service_mem_abs_threshold_syndic_mb = v; }
                if let Some(v) = a.service_mem_pct_threshold_syndic { cfg.service_mem_pct_threshold_syndic = v; }
                if let Some(v) = a.alert_columns { cfg.alert_columns = v; }
            }
            if let Some(a) = toml_cfg.api {
                if let Some(v) = a.stats_window { cfg.api_stats_window = v; }
                if let Some(v) = a.log_cap { cfg.api_log_cap = v; }
                if let Some(v) = a.log_max_age_secs { cfg.api_log_max_age_secs = v; }
                if let Some(v) = a.hide_stats_polls { cfg.api_hide_stats_polls = v; }
                if let Some(v) = a.stats_auth_mode { cfg.api_stats_auth_mode = v; }
                if let Some(v) = a.stats_auth_eauth { cfg.api_stats_auth_eauth = v; }
                if let Some(v) = a.stats_auth_username { cfg.api_stats_auth_username = v; }
                if let Some(v) = a.stats_auth_password { cfg.api_stats_auth_password = v; }
                if let Some(v) = a.stats_auth_username_pillar_path { cfg.api_stats_auth_username_pillar_path = v; }
                if let Some(v) = a.stats_auth_password_pillar_path { cfg.api_stats_auth_password_pillar_path = v; }
                if let Some(v) = a.request_columns { cfg.api_request_columns = v; }
            }
            if let Some(d) = toml_cfg.dashboard {
                if let Some(v) = d.zmq_latency_source { cfg.zmq_latency_source = v; }
                if let Some(v) = d.zmq_latency_ceiling_ms { cfg.zmq_latency_ceiling_ms = v.max(0.0); }
                if let Some(v) = d.show_zmq_graph { cfg.show_zmq_graph = v; }
                if let Some(v) = d.show_events_graph { cfg.show_events_graph = v; }
                if let Some(v) = d.show_activity_graph { cfg.show_activity_graph = v; }
                if let Some(v) = d.show_cpu_graph { cfg.show_cpu_graph = v; }
                if let Some(v) = d.show_mem_graph { cfg.show_mem_graph = v; }
                if let Some(v) = d.graph_order { cfg.graph_order = v; }
                if let Some(v) = d.show_host_resources { cfg.show_host_resources = v; }
                if let Some(v) = d.show_services { cfg.show_services = v; }
                if let Some(v) = d.show_key_status { cfg.show_key_status = v; }
                if let Some(v) = d.show_event_types { cfg.show_event_types = v; }
                if let Some(v) = d.show_job_pulse { cfg.show_job_pulse = v; }
                if let Some(v) = d.show_salt_api { cfg.show_salt_api = v; }
            }
            if let Some(m) = toml_cfg.minions {
                if let Some(v) = m.follow_filter { cfg.minions_follow_filter = v; }
                if let Some(v) = m.minion_columns { cfg.minion_columns = v; }
            }
            if let Some(e) = toml_cfg.export {
                if let Some(v) = e.save_dir { cfg.save_dir = v; }
            }
            if let Some(j) = toml_cfg.jobs {
                if let Some(v) = j.lookup_timeout_secs { cfg.lookup_timeout_secs = v.max(1); }
                if let Some(v) = j.live_columns { cfg.jobs_live_columns = v; }
                if let Some(v) = j.cache_columns { cfg.jobs_cache_columns = v; }
            }
            if let Some(u) = toml_cfg.ui {
                if let Some(v) = u.color_theme { cfg.color_theme = v; }
                if let Some(v) = u.theme_background { cfg.theme_background = v; }
            }
        }
    }
    cfg
}
