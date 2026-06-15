use std::path::Path;
use std::time::SystemTime;

use crate::types::{KeyInfo, KeyStatus};

/// List key directories under pki_dir/master/ and return all keys.
pub fn collect_keys(pki_dir: &str) -> Vec<KeyInfo> {
    let mut keys = Vec::new();

    let dirs = [
        ("minions", KeyStatus::Accepted),
        ("minions_pre", KeyStatus::Pending),
        ("minions_rejected", KeyStatus::Rejected),
        ("minions_denied", KeyStatus::Denied),
    ];

    for (subdir, status) in &dirs {
        let path = Path::new(pki_dir).join(subdir);
        if !path.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&path) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    let id = entry.file_name().to_string_lossy().to_string();
                    let mtime = entry
                        .metadata()
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    keys.push(KeyInfo {
                        id,
                        status: *status,
                        mtime_epoch: mtime,
                    });
                }
            }
        }
    }

    keys
}
