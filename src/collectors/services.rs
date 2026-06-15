use crate::types::ServiceHealth;

/// Check systemd service health for salt-master, salt-api, salt-syndic.
/// Cross-references /proc processes with systemd status so processes
/// running without a systemd unit are still reported as active.
pub fn collect_services() -> Vec<ServiceHealth> {
    let units = ["salt-master", "salt-api", "salt-syndic"];
    let mut services = Vec::new();

    // Snapshot which salt processes are alive in /proc right now
    let running_procs = find_running_salt_processes();

    for &unit in &units {
        let health = check_unit(unit, running_procs.contains(&unit));
        services.push(health);
    }

    services
}

/// Return the set of unit names whose processes are found in /proc.
fn find_running_salt_processes() -> Vec<&'static str> {
    let mut found = Vec::new();
    let proc_dir = match std::fs::read_dir("/proc") {
        Ok(d) => d,
        Err(_) => return found,
    };

    for entry in proc_dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let _pid: u32 = match name_str.parse() {
            Ok(n) => n,
            Err(_) => continue,
        };

        let cmdline = match std::fs::read_to_string(format!("/proc/{}/cmdline", _pid)) {
            Ok(c) => c.replace('\0', " "),
            Err(_) => continue,
        };

        if cmdline.contains("salt-master") {
            if !found.contains(&"salt-master") {
                found.push("salt-master");
            }
        }
        if cmdline.contains("salt-api") && !found.contains(&"salt-api") {
            found.push("salt-api");
        }
        if cmdline.contains("salt-syndic") && !found.contains(&"salt-syndic") {
            found.push("salt-syndic");
        }
    }

    found
}

fn check_unit(unit: &str, proc_running: bool) -> ServiceHealth {
    // Try systemctl is-active
    let is_active = std::process::Command::new("systemctl")
        .arg("is-active")
        .arg(unit)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let systemd_active = is_active == "active";

    // Get detailed properties from systemd (only if unit exists)
    let (active_state, sub_state, timestamp) = if systemd_active || is_active == "inactive" || is_active == "failed" {
        get_systemd_properties(unit)
    } else {
        (String::new(), String::new(), String::new())
    };

    // Cross-reference: process in /proc beats systemctl
    let active = proc_running || systemd_active;

    let state = if proc_running && !systemd_active && is_active != "active" {
        if is_active.is_empty() || is_active == "unknown" {
            "running (no systemd unit)".to_string()
        } else {
            format!("running (systemd: {})", is_active)
        }
    } else if active_state.is_empty() && sub_state.is_empty() {
        if proc_running {
            "running".to_string()
        } else if is_active == "unknown" || is_active.is_empty() {
            "not found".to_string()
        } else {
            is_active.clone()
        }
    } else {
        format!("{}/{}", active_state, sub_state)
    };

    let uptime = parse_uptime(&timestamp);

    ServiceHealth {
        unit: unit.to_string(),
        active,
        state,
        uptime_seconds: uptime,
        active_enter_timestamp: timestamp,
    }
}

fn get_systemd_properties(unit: &str) -> (String, String, String) {
    let output = std::process::Command::new("systemctl")
        .arg("show")
        .arg(unit)
        .arg("--property=ActiveState,SubState,ActiveEnterTimestamp")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    let mut active_state = String::new();
    let mut sub_state = String::new();
    let mut timestamp = String::new();

    for line in output.lines() {
        if let Some(v) = line.strip_prefix("ActiveState=") {
            active_state = v.to_string();
        } else if let Some(v) = line.strip_prefix("SubState=") {
            sub_state = v.to_string();
        } else if let Some(v) = line.strip_prefix("ActiveEnterTimestamp=") {
            timestamp = v.to_string();
        }
    }

    (active_state, sub_state, timestamp)
}

fn parse_uptime(timestamp: &str) -> u64 {
    // systemd timestamps look like: "Mon 2024-01-15 10:30:00 UTC"
    let _ = timestamp;
    0
}
