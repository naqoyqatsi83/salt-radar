use std::path::Path;

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
        }
    }
}

#[derive(serde::Deserialize)]
struct TomlConfig {
    salt: Option<SaltConfig>,
    refresh: Option<RefreshConfig>,
    events: Option<EventsConfig>,
    alerts: Option<AlertsConfig>,
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
}

pub fn write_config(path: &Path, config: &AppConfig) -> std::io::Result<()> {
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
         service_mem_pct_threshold = {}\n",
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
            }
        }
    }
    cfg
}
