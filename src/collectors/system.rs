use std::path::Path;

use crate::types::SystemInfo;

pub fn collect_system(cachedir: &str, pki_dir: &str) -> SystemInfo {
    SystemInfo {
        load_1: read_loadavg(0),
        load_5: read_loadavg(1),
        load_15: read_loadavg(2),
        mem_total_kb: read_meminfo("MemTotal"),
        mem_avail_kb: read_meminfo("MemAvailable"),
        cache_disk_usage_pct: disk_usage_pct(cachedir),
        pki_disk_usage_pct: disk_usage_pct(pki_dir),
    }
}

fn read_loadavg(index: usize) -> f64 {
    let data = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    data.split_whitespace()
        .nth(index)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
}

fn read_meminfo(field: &str) -> u64 {
    let data = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    for line in data.lines() {
        if line.starts_with(field) {
            if let Some(kb_str) = line.split_whitespace().nth(1) {
                return kb_str.parse::<u64>().unwrap_or(0);
            }
        }
    }
    0
}

fn disk_usage_pct(path: &str) -> f64 {
    if !Path::new(path).exists() {
        return 0.0;
    }
    let output = std::process::Command::new("df")
        .arg("--output=pcent")
        .arg(path)
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).to_string();
            s.lines().nth(1).map(|l| l.trim().trim_end_matches('%').to_string())
        })
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    output
}
