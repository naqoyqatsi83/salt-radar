use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::app::AgentMessage;

/// Hard cap on the inbound queue so a sustained event-bus burst (e.g. a
/// broadcast job against a large fleet) can't grow it — and the process's
/// memory — without bound. Once full, oldest messages are dropped in favor
/// of the freshest ones; `dropped` tracks how many so the UI can surface it.
const MAX_QUEUE_LEN: usize = 20_000;

pub fn agent_main(
    tx: Arc<Mutex<VecDeque<AgentMessage>>>,
    dropped: Arc<AtomicU64>,
    agent_path: &str,
    python_bin: &str,
    master_config: &str,
    cmd_path: &str,
) {
    // Start the Python agent process
    let mut child = match Command::new(python_bin)
        .arg(agent_path)
        .arg("--master-config")
        .arg(master_config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let mut q = tx.lock().unwrap();
            q.push_back(AgentMessage::Error {
                id: None,
                message: format!("Failed to spawn agent: {}", e),
            });
            return;
        }
    };

    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    let tx_clone = tx.clone();
    let dropped_clone = dropped.clone();

    // Reader thread: reads NDJSON lines from agent stdout
    thread::spawn(move || {
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            if line.trim().is_empty() {
                continue;
            }
            let parsed: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let msg = match parsed["type"].as_str() {
                Some("result") => AgentMessage::Result {
                    id: parsed["id"].as_u64().unwrap_or(0),
                    cmd: parsed["cmd"].as_str().unwrap_or("").to_string(),
                    ok: parsed["ok"].as_bool().unwrap_or(false),
                    data: parsed["data"].clone(),
                },
                Some("event") => AgentMessage::Event {
                    tag: parsed["tag"].as_str().unwrap_or("").to_string(),
                    data: parsed["data"].clone(),
                    ts: parsed["ts"].as_f64().unwrap_or(0.0),
                },
                Some("error") => AgentMessage::Error {
                    id: parsed.get("id").and_then(|v| v.as_u64()),
                    message: parsed["message"].as_str().unwrap_or("unknown error").to_string(),
                },
                _ => continue,
            };

            let mut q = tx_clone.lock().unwrap();
            if q.len() >= MAX_QUEUE_LEN {
                q.pop_front();
                dropped_clone.fetch_add(1, Ordering::Relaxed);
            }
            q.push_back(msg);
        }
    });

    // Stderr reader thread (logging only)
    let stderr = child.stderr.take().unwrap();
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(l) = line {
                eprintln!("[salt-radar-agent] {}", l);
            }
        }
    });

    // Keep stdin handle alive and write commands from a file-based queue
    let mut stdin = child.stdin.take().unwrap();

    // Send initial commands
    let init_cmds = vec![
        r#"{"id": 1, "cmd": "version"}"#,
        r#"{"id": 2, "cmd": "config_snapshot"}"#,
        r#"{"id": 3, "cmd": "minion_cache_list"}"#,
    ];
    for cmd in init_cmds {
        let _ = writeln!(stdin, "{}", cmd);
        thread::sleep(std::time::Duration::from_millis(50));
    }

    // Poll the command file for new commands
    loop {
        if let Ok(data) = std::fs::read_to_string(cmd_path) {
            if !data.trim().is_empty() {
                let _ = writeln!(stdin, "{}", data.trim());
                let _ = std::fs::write(cmd_path, "");
            }
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                // Process exited
                let mut q = tx.lock().unwrap();
                q.push_back(AgentMessage::Error {
                    id: None,
                    message: "Agent process exited".to_string(),
                });
                return;
            }
            Ok(None) => {}
            Err(_) => return,
        }
        thread::sleep(std::time::Duration::from_millis(100));
    }
}
