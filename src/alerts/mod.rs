use std::time::Instant;

use crate::types::{
    AlertEntry, AlertSeverity, JobEntry, KeyInfo, KeyStatus, MinionInfo, SaltEvent, ServiceHealth,
    ServiceTrendSnapshot, SystemInfo,
};

/// Rule 13 per-unit overrides for the standard bundled Salt services — 0.0
/// in either half means "no override, fall back to the global
/// service_mem_abs_threshold_mb / service_mem_pct_threshold".
pub struct ServiceMemThresholds {
    pub master_abs_mb: f64,
    pub master_pct: f64,
    pub minion_abs_mb: f64,
    pub minion_pct: f64,
    pub api_abs_mb: f64,
    pub api_pct: f64,
    pub syndic_abs_mb: f64,
    pub syndic_pct: f64,
}

/// Lightweight representation of an active alert condition.
struct ActiveAlert {
    key: String,
    rule: &'static str,
    severity: AlertSeverity,
    description: String,
    details: Vec<String>,
}

pub struct AlertEngine {
    pub alerts: Vec<AlertEntry>,
}

impl AlertEngine {
    pub fn new() -> Self {
        Self {
            alerts: Vec::new(),
        }
    }

    pub fn tick(
        &mut self,
        minion_info: &[MinionInfo],
        keys: &[KeyInfo],
        services: &[ServiceHealth],
        sys: &SystemInfo,
        events: &[&SaltEvent],
        stale_minion_secs: u64,
        pending_key_secs: u64,
        disk_usage_pct: f64,
        jobs: &[JobEntry],
        job_straggler_secs: u64,
        auth_rate: f64,
        worker_busy_pct: Option<f64>,
        auth_storm_rate_per_sec: f64,
        worker_saturation_pct: f64,
        pending_key_churn_threshold: u64,
        service_trends: &[ServiceTrendSnapshot],
        service_flap_threshold_24h: u64,
        service_mem_growth_mb_per_hour: f64,
        service_mem_abs_threshold_mb: f64,
        service_mem_pct_threshold: f64,
        service_mem_overrides: &ServiceMemThresholds,
        api_login_p95_secs: Option<f64>,
        api_auth_latency_warn_secs: f64,
    ) {
        let mut active = Vec::new();

        // Rule 1: Stale minion
        for minion in minion_info {
            let last_seen = minion.last_seen_epoch.max(minion.cache_updated_epoch);
            if last_seen == 0 {
                continue;
            }
            let age = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs().saturating_sub(last_seen))
                .unwrap_or(0);
            if age > stale_minion_secs {
                active.push(ActiveAlert {
                    key: format!("stale:{}", minion.id),
                    rule: "Stale minion",
                    severity: AlertSeverity::Warn,
                    description: format!("{} not seen for {}s", minion.id, age),
                    details: Vec::new(),
                });
            }
        }

        // Rule 2: Pending key too long
        for key in keys {
            if key.status != KeyStatus::Pending || key.mtime_epoch == 0 {
                continue;
            }
            let age = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs().saturating_sub(key.mtime_epoch))
                .unwrap_or(0);
            if age > pending_key_secs {
                active.push(ActiveAlert {
                    key: format!("pending-key:{}", key.id),
                    rule: "Pending key",
                    severity: AlertSeverity::Warn,
                    description: format!("{} pending for {}s", key.id, age),
                    details: Vec::new(),
                });
            }
        }

        // Rule 3: Service down
        for svc in services {
            if !svc.active && svc.state != "not found" {
                active.push(ActiveAlert {
                    key: format!("svc-down:{}", svc.unit),
                    rule: "Service down",
                    severity: AlertSeverity::Crit,
                    description: format!("{} is {}", svc.unit, svc.state),
                    details: Vec::new(),
                });
            }
        }

        // Rule 4: Disk usage high
        if sys.cache_disk_usage_pct > disk_usage_pct {
            active.push(ActiveAlert {
                key: "disk:cache".to_string(),
                rule: "Disk usage high",
                severity: AlertSeverity::Warn,
                description: format!("cache disk at {:.0}%", sys.cache_disk_usage_pct),
                details: Vec::new(),
            });
        }
        if sys.pki_disk_usage_pct > disk_usage_pct {
            active.push(ActiveAlert {
                key: "disk:pki".to_string(),
                rule: "Disk usage high",
                severity: AlertSeverity::Warn,
                description: format!("pki disk at {:.0}%", sys.pki_disk_usage_pct),
                details: Vec::new(),
            });
        }

        // ── ZMQ / Security alert rules ──────────────────────────────────────

        // Rule 5: Legacy protocol minions
        // Scan events for tags containing "legacy" or "minion_legacy"
        let mut legacy_minions: Vec<String> = Vec::new();
        for evt in events {
            if evt.tag.contains("minion_legacy") || evt.tag.contains("/legacy/") {
                let mid = evt.tag.rsplit('/').next().unwrap_or("?");
                let msg = evt.data["message"].as_str().unwrap_or("legacy protocol");
                let detail = format!("{}: {}", mid, msg);
                if !legacy_minions.contains(&detail) {
                    legacy_minions.push(detail);
                }
            }
        }
        if !legacy_minions.is_empty() {
            active.push(ActiveAlert {
                key: "zmq:legacy".to_string(),
                rule: "Legacy protocol",
                severity: AlertSeverity::Warn,
                description: format!("{} minion(s) using legacy protocol", legacy_minions.len()),
                details: legacy_minions,
            });
        }

        // Rule 6: Salt warning events (ZMQ / protocol issues)
        let mut warnings: Vec<String> = Vec::new();
        for evt in events {
            if !evt.tag.starts_with("salt/warning/") {
                continue;
            }
            let msg = evt.data["message"].as_str().unwrap_or("");
            if msg.is_empty() {
                continue;
            }
            // Already handled by rule 5
            if msg.contains("legacy") {
                continue;
            }
            let mid = evt.data["id"].as_str().unwrap_or("?");
            let detail = format!("{}: {}", mid, msg);
            if !warnings.contains(&detail) {
                warnings.push(detail);
            }
        }
        if !warnings.is_empty() {
            active.push(ActiveAlert {
                key: "zmq:warnings".to_string(),
                rule: "Salt warning",
                severity: AlertSeverity::Warn,
                description: format!("{} warning(s) on event bus", warnings.len()),
                details: warnings,
            });
        }

        // Rule 7: Auth failures
        let mut auth_fails: Vec<String> = Vec::new();
        for evt in events {
            if !evt.tag.starts_with("salt/auth") {
                continue;
            }
            let result = evt.data["result"].as_bool().unwrap_or(true);
            if result {
                continue;
            }
            let mid = evt.data["id"].as_str().unwrap_or("?");
            let reason = evt.data["reason"].as_str().unwrap_or("unknown");
            let detail = format!("{}: {}", mid, reason);
            if !auth_fails.contains(&detail) {
                auth_fails.push(detail);
            }
        }
        if !auth_fails.is_empty() {
            active.push(ActiveAlert {
                key: "zmq:auth".to_string(),
                rule: "Auth failure",
                severity: AlertSeverity::Crit,
                description: format!("{} auth failure(s) on ZMQ bus", auth_fails.len()),
                details: auth_fails,
            });
        }

        // Rule 8: Stuck job
        for job in jobs {
            if job.target_count > 0 && job.returned < job.target_count && job.age_seconds > job_straggler_secs {
                active.push(ActiveAlert {
                    key: format!("stuck-job:{}", job.jid),
                    rule: "Stuck job",
                    severity: AlertSeverity::Warn,
                    description: format!(
                        "{} ({}) {}/{} returned after {}s",
                        job.jid, job.fun, job.returned, job.target_count, job.age_seconds
                    ),
                    details: Vec::new(),
                });
            }
        }

        // Rule 9: Reauth storm — auth rate and worker saturation elevated together
        if auth_rate > auth_storm_rate_per_sec && worker_busy_pct.unwrap_or(0.0) > worker_saturation_pct {
            active.push(ActiveAlert {
                key: "reauth-storm".to_string(),
                rule: "Reauth storm",
                severity: AlertSeverity::Crit,
                description: format!(
                    "auth rate {:.1}/s with workers at {:.0}% busy",
                    auth_rate, worker_busy_pct.unwrap_or(0.0)
                ),
                details: Vec::new(),
            });
        }

        // Rule 10: Pending-key churn — new pending keys arriving too fast
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let churn = keys.iter().filter(|k| {
            k.status == KeyStatus::Pending && k.mtime_epoch > 0
                && now.saturating_sub(k.mtime_epoch) < 60
        }).count() as u64;
        if churn > pending_key_churn_threshold {
            active.push(ActiveAlert {
                key: "pending-key-churn".to_string(),
                rule: "Pending key churn",
                severity: AlertSeverity::Warn,
                description: format!("{} new pending key(s) in the last minute", churn),
                details: Vec::new(),
            });
        }

        // Rule 11: Service flapping — unit restarted too many times in 24h
        for t in service_trends {
            if t.restart_count_24h as u64 >= service_flap_threshold_24h {
                active.push(ActiveAlert {
                    key: format!("svc-flap:{}", t.unit),
                    rule: "Service flapping",
                    severity: AlertSeverity::Crit,
                    description: format!("{} restarted {} time(s) in the last 24h", t.unit, t.restart_count_24h),
                    details: Vec::new(),
                });
            }
        }

        // Rule 12: Sustained service memory growth — independent of flapping,
        // catches a slow leak in a unit that never actually restarts.
        for t in service_trends {
            if let Some(rate) = t.mem_growth_mb_per_hour {
                if t.mem_still_growing && rate > service_mem_growth_mb_per_hour {
                    active.push(ActiveAlert {
                        key: format!("svc-mem-growth:{}", t.unit),
                        rule: "Service memory growth",
                        severity: AlertSeverity::Warn,
                        description: format!("{} memory growing at {:.1} MB/h", t.unit, rate),
                        details: Vec::new(),
                    });
                }
            }
        }

        // Rule 13: Service memory over threshold — flags a unit that's already
        // sitting on an unreasonable amount of memory, independent of current
        // growth slope (a slow, low-slope leak running for weeks would never
        // trip Rule 12's rate check). Either the absolute or percent-of-total
        // threshold crossing is enough; a threshold of 0 disables that check.
        if sys.mem_total_kb > 0 {
            let resolve = |over: f64, default: f64| if over > 0.0 { over } else { default };
            for svc in services {
                let mem_mb = svc.memory_kb as f64 / 1024.0;
                let mem_pct = svc.memory_kb as f64 / sys.mem_total_kb as f64 * 100.0;
                let (override_abs, override_pct) = match svc.unit.as_str() {
                    "salt-master.service" => (service_mem_overrides.master_abs_mb, service_mem_overrides.master_pct),
                    "salt-minion.service" => (service_mem_overrides.minion_abs_mb, service_mem_overrides.minion_pct),
                    "salt-api.service" => (service_mem_overrides.api_abs_mb, service_mem_overrides.api_pct),
                    "salt-syndic.service" => (service_mem_overrides.syndic_abs_mb, service_mem_overrides.syndic_pct),
                    _ => (0.0, 0.0),
                };
                let abs_threshold = resolve(override_abs, service_mem_abs_threshold_mb);
                let pct_threshold = resolve(override_pct, service_mem_pct_threshold);
                let over_abs = abs_threshold > 0.0 && mem_mb > abs_threshold;
                let over_pct = pct_threshold > 0.0 && mem_pct > pct_threshold;
                if over_abs || over_pct {
                    active.push(ActiveAlert {
                        key: format!("svc-mem-high:{}", svc.unit),
                        rule: "Service memory high",
                        severity: AlertSeverity::Crit,
                        description: format!("{} using {:.0} MB ({:.1}% of system memory)", svc.unit, mem_mb, mem_pct),
                        details: Vec::new(),
                    });
                }
            }
        }

        // Rule 14: API auth latency — salt-api's POST /login p95 processing
        // time has crept up, a classic early symptom of PAM/LDAP/eauth
        // backend trouble before it gets bad enough to actually time out.
        // Only fires once the salt-api panel has real login samples to judge.
        if let Some(p95) = api_login_p95_secs {
            if p95 > api_auth_latency_warn_secs {
                active.push(ActiveAlert {
                    key: "api-auth-latency".to_string(),
                    rule: "API auth latency",
                    severity: AlertSeverity::Warn,
                    description: format!("salt-api POST /login p95 at {:.2}s", p95),
                    details: Vec::new(),
                });
            }
        }

        // Sync active alerts into the engine
        self.sync_alerts(&active);
    }

    fn sync_alerts(&mut self, active: &[ActiveAlert]) {
        let active_keys: std::collections::HashSet<&str> =
            active.iter().map(|a| a.key.as_str()).collect();

        // Remove resolved alerts
        self.alerts.retain(|a| active_keys.contains(a.key.as_str()));

        // Add new or update existing
        for a in active {
            match self.alerts.iter_mut().find(|e| e.key == a.key) {
                Some(existing) => {
                    existing.count += 1;
                    existing.description = a.description.clone();
                    existing.details = a.details.clone();
                }
                None => {
                    self.alerts.push(AlertEntry {
                        key: a.key.clone(),
                        severity: a.severity,
                        rule: a.rule,
                        description: a.description.clone(),
                        details: a.details.clone(),
                        active_since: Instant::now(),
                        count: 1,
                    });
                }
            }
        }
    }

    pub fn active_count(&self) -> usize {
        self.alerts.len()
    }
}
