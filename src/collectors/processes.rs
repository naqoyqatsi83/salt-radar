use crate::types::{SaltProcess, SaltProcessRole};

/// Find salt-master, salt-api, salt-syndic processes by scanning /proc directly.
pub fn collect_processes() -> Vec<SaltProcess> {
    let mut procs = Vec::new();

    let proc_dir = match std::fs::read_dir("/proc") {
        Ok(d) => d,
        Err(_) => return procs,
    };

    for entry in proc_dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let pid: u32 = match name_str.parse() {
            Ok(n) => n,
            Err(_) => continue,
        };

        let cmdline = match std::fs::read_to_string(format!("/proc/{}/cmdline", pid)) {
            Ok(c) => c.replace('\0', " "),
            Err(_) => continue,
        };

        let role = match () {
            _ if cmdline.contains("salt-master") && cmdline.contains("Master") => {
                SaltProcessRole::Master
            }
            _ if cmdline.contains("salt-master") && cmdline.contains("MWorker") => {
                SaltProcessRole::Worker
            }
            _ if cmdline.contains("salt-master") && cmdline.contains("EventPublisher") => {
                SaltProcessRole::EventPublisher
            }
            _ if cmdline.contains("salt-master") && cmdline.contains("ReqServer") => {
                SaltProcessRole::ReqServer
            }
            _ if cmdline.contains("salt-master") => SaltProcessRole::Worker,
            _ if cmdline.contains("salt-api") => SaltProcessRole::API,
            _ if cmdline.contains("salt-syndic") => SaltProcessRole::Syndic,
            _ => continue,
        };

        // Read RSS from /proc/<pid>/status
        let rss_kb = read_rss_from_status(pid);

        procs.push(SaltProcess {
            pid,
            role,
            rss_kb,
            cpu_percent: 0.0,
        });
    }

    procs
}

fn read_rss_from_status(pid: u32) -> u64 {
    let status = match std::fs::read_to_string(format!("/proc/{}/status", pid)) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    for line in status.lines() {
        if let Some(val) = line.strip_prefix("VmRSS:") {
            // Format: "VmRSS:   12345 kB"
            let parts: Vec<&str> = val.trim().split_whitespace().collect();
            if let Some(kb) = parts.first().and_then(|s| s.parse::<u64>().ok()) {
                return kb;
            }
        }
    }
    0
}
