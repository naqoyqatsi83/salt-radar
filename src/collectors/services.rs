use crate::types::ServiceHealth;

/// Discover and check systemd service health for all salt-* services.
/// Dynamic discovery from installed .service unit files, not hardcoded.
pub fn collect_services() -> Vec<ServiceHealth> {
    let units = discover_salt_units();
    let running_procs = find_running_salt_processes(&units);

    let mut services = Vec::new();
    for unit in &units {
        let health = check_unit(unit, running_procs.contains(unit));
        services.push(health);
    }
    services
}

/// Discover all installed salt-*.service unit files via systemctl list-unit-files.
fn discover_salt_units() -> Vec<String> {
    let mut units = Vec::new();

    // Use systemctl list-unit-files --all --no-legend 'salt-*.service' to list all salt services
    let output = match std::process::Command::new("systemctl")
        .arg("list-unit-files")
        .arg("--all")
        .arg("--no-legend")
        .arg("salt-*.service")
        .output()
    {
        Ok(o) => o,
        Err(_) => return units,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.trim().is_empty() { continue; }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 1 {
            let unit_name = parts[0];
            if let Some(stripped) = unit_name.strip_suffix(".service") {
                units.push(stripped.to_string());
            }
        }
    }

    units
}

/// Return the set of unit names whose processes are found in /proc.
fn find_running_salt_processes(units: &[String]) -> Vec<String> {
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

        // Direct process name to unit name mapping
        if cmdline.contains("salt-master") {
            if !found.contains(&"salt-master".to_string()) {
                found.push("salt-master".to_string());
            }
        } else if cmdline.contains("salt-api") && !found.contains(&"salt-api".to_string()) {
            found.push("salt-api".to_string());
        } else if cmdline.contains("salt-syndic") && !found.contains(&"salt-syndic".to_string()) {
            found.push("salt-syndic".to_string());
        } else if cmdline.contains("salt-minion") && !found.contains(&"salt-minion".to_string()) {
            found.push("salt-minion".to_string());
        } else if cmdline.contains("salt-proxy") && !found.contains(&"salt-proxy@".to_string()) {
            found.push("salt-proxy@".to_string());
        }

        // For salt-job-archive and other services, check if it's running
        for unit in units {
            if !found.contains(&unit) {
                let service_exe = unit.trim_start_matches("salt-");
                let full_exe = format!("salt-{}", service_exe);
                if cmdline.contains(&full_exe) {
                    found.push(unit.clone());
                }
            }
        }
    }

    found
}

fn check_unit(unit: &String, proc_running: bool) -> ServiceHealth {
    let unit_str = unit.as_str();
    // Try systemctl is-active
    let is_active = std::process::Command::new("systemctl")
        .arg("is-active")
        .arg(unit_str)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let systemd_active = is_active == "active";

    // Get detailed properties from systemd (only if unit exists)
    let (active_state, sub_state, timestamp, memory_kb) = if systemd_active || is_active == "inactive" || is_active == "failed" {
        get_systemd_properties(unit_str)
    } else {
        (String::new(), String::new(), String::new(), 0)
    };

    // Cross-reference: process in /proc beats systemctl
    let active = proc_running || systemd_active;

    // Simple state based on systemd status
    let state = if systemd_active {
        if !active_state.is_empty() && !sub_state.is_empty() {
            format!("{} / {}", active_state, sub_state)
        } else {
            "active".to_string()
        }
    } else {
        is_active.clone()
    };

    let uptime = if active { parse_uptime(&timestamp) } else { 0 };

    ServiceHealth {
        unit: unit_str.to_string(),
        active,
        state,
        uptime_seconds: uptime,
        active_enter_timestamp: timestamp,
        memory_kb,
    }
}

fn get_systemd_properties(unit: &str) -> (String, String, String, u64) {
    let output = std::process::Command::new("systemctl")
        .arg("show")
        .arg(unit)
        .arg("--property=ActiveState,SubState,ActiveEnterTimestamp,MemoryCurrent")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    let mut active_state = String::new();
    let mut sub_state = String::new();
    let mut timestamp = String::new();
    let mut memory_kb = 0u64;

    for line in output.lines() {
        if let Some(v) = line.strip_prefix("ActiveState=") {
            active_state = v.to_string();
        } else if let Some(v) = line.strip_prefix("SubState=") {
            sub_state = v.to_string();
        } else if let Some(v) = line.strip_prefix("ActiveEnterTimestamp=") {
            timestamp = v.to_string();
        } else if let Some(v) = line.strip_prefix("MemoryCurrent=") {
            if let Ok(mem) = v.parse::<u64>() {
                memory_kb = mem / 1024; // Convert bytes to KB
            }
        }
    }

    // Ensure memory_kb is set even when MemoryCurrent property doesn't exist
    if memory_kb == 0 {
        // Try Memory unit or fallback to 0
        memory_kb = 0;
    }

    (active_state, sub_state, timestamp, memory_kb)
}

fn parse_uptime(timestamp: &str) -> u64 {
    if timestamp.is_empty() { return 0; }
    let now = std::time::SystemTime::now();

    // Try multiple systemd timestamp formats
    let formats = [
        "%a %Y-%m-%d %H:%M:%S %Z",
        "%a %Y-%m-%d %H:%M:%S %z",
        "%Y-%m-%d %H:%M:%S",
        "%a %Y-%m-%d %H:%M:%S",
    ];

    let mut start = 0u64;

    // Try each format
    for fmt in formats {
        if let Ok(dt) = chrono::DateTime::parse_from_str(timestamp, fmt) {
            start = dt.timestamp() as u64;
            break;
        }
    }

    // Fallback to RFC2822
    if start == 0 {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(timestamp) {
            start = dt.timestamp() as u64;
        }
    }

    // Last resort: try parsing just the date/time part, ignoring timezone
    // systemd format: "Mon 2026-06-15 09:30:04 CEST" -> parts[1..4] = "2026-06-15", "09:30:04", "CEST"
    if start == 0 {
        let parts: Vec<&str> = timestamp.split_whitespace().collect();
        if parts.len() >= 4 {
            let dt_str = format!("{} {}", parts[1], parts[2]);
            if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&dt_str, "%Y-%m-%d %H:%M:%S") {
                start = dt.and_utc().timestamp() as u64;
            }
        }
    }

    if start == 0 { return 0; }
    let elapsed = now.duration_since(std::time::UNIX_EPOCH).unwrap_or(std::time::Duration::from_secs(0));
    elapsed.as_secs().saturating_sub(start)
}
