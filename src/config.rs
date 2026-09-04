use std::path::{Path, PathBuf};

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

    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));

    match config_home {
        Some(dir) => dir.join("salt-radar").join("salt-radar.toml"),
        None => cwd_path,
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
            service_mem_abs_threshold_master_mb: 0.0,
            service_mem_pct_threshold_master: 0.0,
            service_mem_abs_threshold_minion_mb: 0.0,
            service_mem_pct_threshold_minion: 0.0,
            service_mem_abs_threshold_api_mb: 0.0,
            service_mem_pct_threshold_api: 0.0,
            service_mem_abs_threshold_syndic_mb: 0.0,
            service_mem_pct_threshold_syndic: 0.0,
            zmq_latency_source: "agent_ipc".into(),
            show_zmq_graph: true,
            show_events_graph: true,
            show_activity_graph: true,
            show_cpu_graph: true,
            show_mem_graph: true,
            graph_order: "cpu,mem,zmq,events,activity".into(),
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
}

#[derive(serde::Deserialize)]
struct DashboardConfig {
    zmq_latency_source: Option<String>,
    show_zmq_graph: Option<bool>,
    show_events_graph: Option<bool>,
    show_activity_graph: Option<bool>,
    show_cpu_graph: Option<bool>,
    show_mem_graph: Option<bool>,
    graph_order: Option<String>,
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
         \n\
         [dashboard]\n\
         zmq_latency_source = {:?}\n\
         show_zmq_graph = {}\n\
         show_events_graph = {}\n\
         show_activity_graph = {}\n\
         show_cpu_graph = {}\n\
         show_mem_graph = {}\n\
         graph_order = {:?}\n",
        config.agent_path,
        config.python_bin,
        config.master_config,
        config.dashboard_secs,
        config.minions_secs,
        config.config_secs,
        config.event_buffer_size,
        config.sniffer_buffer_size,
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
        config.zmq_latency_source,
        config.show_zmq_graph,
        config.show_events_graph,
        config.show_activity_graph,
        config.show_cpu_graph,
        config.show_mem_graph,
        config.graph_order,
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
            }
            if let Some(d) = toml_cfg.dashboard {
                if let Some(v) = d.zmq_latency_source { cfg.zmq_latency_source = v; }
                if let Some(v) = d.show_zmq_graph { cfg.show_zmq_graph = v; }
                if let Some(v) = d.show_events_graph { cfg.show_events_graph = v; }
                if let Some(v) = d.show_activity_graph { cfg.show_activity_graph = v; }
                if let Some(v) = d.show_cpu_graph { cfg.show_cpu_graph = v; }
                if let Some(v) = d.show_mem_graph { cfg.show_mem_graph = v; }
                if let Some(v) = d.graph_order { cfg.graph_order = v; }
            }
        }
    }
    cfg
}
