use std::time::Instant;

use crate::types::{
    AlertEntry, AlertSeverity, KeyInfo, KeyStatus, MinionInfo, ServiceHealth, SystemInfo,
};

/// Lightweight representation of an active alert condition.
struct ActiveAlert {
    key: String,
    rule: &'static str,
    severity: AlertSeverity,
    description: String,
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
        stale_minion_secs: u64,
        pending_key_secs: u64,
        disk_usage_pct: f64,
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
            });
        }
        if sys.pki_disk_usage_pct > disk_usage_pct {
            active.push(ActiveAlert {
                key: "disk:pki".to_string(),
                rule: "Disk usage high",
                severity: AlertSeverity::Warn,
                description: format!("pki disk at {:.0}%", sys.pki_disk_usage_pct),
            });
        }

        // Sync active alerts into the engine — add new, update existing, remove resolved
        self.sync_alerts(&active);
    }

    fn sync_alerts(&mut self, active: &[ActiveAlert]) {
        let active_keys: std::collections::HashSet<&str> =
            active.iter().map(|a| a.key.as_str()).collect();

        // Remove resolved alerts (their key is no longer active)
        self.alerts.retain(|a| active_keys.contains(a.key.as_str()));

        // Add new alerts or update descriptions
        for a in active {
            match self.alerts.iter_mut().find(|e| e.key == a.key) {
                Some(existing) => {
                    existing.count += 1;
                    existing.description = a.description.clone();
                }
                None => {
                    self.alerts.push(AlertEntry {
                        key: a.key.clone(),
                        severity: a.severity,
                        rule: a.rule,
                        description: a.description.clone(),
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
