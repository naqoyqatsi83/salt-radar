use std::path::Path;

use crate::types::SystemInfo;

pub fn collect_system(cachedir: &str, pki_dir: &str) -> SystemInfo {
    let (rx, tx) = read_net_dev();
    SystemInfo {
        load_1: read_loadavg(0),
        load_5: read_loadavg(1),
        load_15: read_loadavg(2),
        mem_total_kb: read_meminfo("MemTotal"),
        mem_avail_kb: read_meminfo("MemAvailable"),
        cache_disk_usage_pct: disk_usage_pct(cachedir),
        pki_disk_usage_pct: disk_usage_pct(pki_dir),
        uptime_secs: read_uptime_secs(),
        network_rx_bytes: rx,
        network_tx_bytes: tx,
        ..Default::default()
    }
}

fn read_loadavg(index: usize) -> f64 {
    let data = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    data.split_whitespace()
        .nth(index)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
}

fn read_uptime_secs() -> u64 {
    let data = std::fs::read_to_string("/proc/uptime").unwrap_or_default();
    data.split_whitespace()
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|f| f as u64)
        .unwrap_or(0)
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

fn read_net_dev() -> (u64, u64) {
    let data = std::fs::read_to_string("/proc/net/dev").unwrap_or_default();
    let mut rx_total: u64 = 0;
    let mut tx_total: u64 = 0;
    for line in data.lines().skip(2) {
        let line = line.trim();
        if line.is_empty() || line.starts_with("lo:") {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 10 {
            rx_total += fields[1].parse::<u64>().unwrap_or(0);
            tx_total += fields[9].parse::<u64>().unwrap_or(0);
        }
    }
    (rx_total, tx_total)
}
