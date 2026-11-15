use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use crate::app::AgentMessage;

/// Hard cap on the inbound queue so a sustained event-bus burst (e.g. a
/// broadcast job against a large fleet) can't grow it — and the process's
/// memory — without bound. Once full, oldest messages are dropped in favor
/// of the freshest ones; `dropped` tracks how many so the UI can surface it.
const MAX_QUEUE_LEN: usize = 20_000;

/// Runs the Python agent for the lifetime of the app, respawning it whenever
/// it exits (crash, OOM-kill, an unhandled exception in a busy long-running
/// session, ...) instead of leaving the connection dead forever — this used
/// to `return` on exit, silently freezing every view that depends on agent
/// data (Minions, Pulse, ...) for the rest of the session with no recovery.
/// Returns once the app drops its end of `cmds`.
pub fn agent_main(
    tx: Arc<Mutex<VecDeque<AgentMessage>>>,
    dropped: Arc<AtomicU64>,
    agent_path: &str,
    python_bin: &str,
    master_config: &str,
    cmds: Receiver<String>,
) {
    loop {
        let Some(last_words) = run_agent_once(&tx, &dropped, agent_path, python_bin, master_config, &cmds) else { return };
        {
            let mut q = tx.lock().unwrap();
            q.push_back(AgentMessage::Error { message: exit_message(&last_words) });
        }
        thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// Lines of the agent's stderr kept for `exit_message`.
const STDERR_TAIL: usize = 20;

/// Why the agent is being restarted: the last thing it said on stderr,
/// which for a Python process that died is the exception.
fn exit_message(stderr_tail: &VecDeque<String>) -> String {
    match stderr_tail.iter().rev().find(|l| !l.trim().is_empty()) {
        Some(line) => format!("agent exited, restarting: {}", line.trim()),
        None => "agent exited, restarting".to_string(),
    }
}

fn run_agent_once(
    tx: &Arc<Mutex<VecDeque<AgentMessage>>>,
    dropped: &Arc<AtomicU64>,
    agent_path: &str,
    python_bin: &str,
    master_config: &str,
    cmds: &Receiver<String>,
) -> Option<VecDeque<String>> {
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
                message: format!("Failed to spawn agent: {}", e),
            });
            // Nothing will read what was sent meanwhile: drop it, or every
            // poll piles up here for as long as the agent can't start.
            while cmds.try_recv().is_ok() {}
            return Some(VecDeque::new());
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
            let received = Instant::now();
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
                    received,
                },
                Some("event") => AgentMessage::Event {
                    tag: parsed["tag"].as_str().unwrap_or("").to_string(),
                    data: parsed["data"].clone(),
                    ts: parsed["ts"].as_f64().unwrap_or(0.0),
                    received,
                },
                Some("error") => AgentMessage::Error {
                    message: parsed["message"].as_str().unwrap_or("unknown error").to_string(),
                },
                Some("event_bus") => AgentMessage::EventBus {
                    error: (!parsed["ok"].as_bool().unwrap_or(false))
                        .then(|| parsed["error"].as_str().unwrap_or("unknown error").to_string()),
                },
                Some("progress") => AgentMessage::Progress {
                    cmd: parsed["cmd"].as_str().unwrap_or("").to_string(),
                    loaded: parsed["data"]["loaded"].as_u64().unwrap_or(0) as usize,
                },
                _ => continue,
            };

            let mut q = tx_clone.lock().unwrap();
            if q.len() >= MAX_QUEUE_LEN {
                // Never let a job-start ("/new") event be the one silently
                // dropped here -- losing it permanently mistracks that job
                // for the rest of its life (Job tracking has no other way
                // to learn the real target/count, so it falls back to
                // treating the job's first-seen `/ret/` as a one-off local
                // execution and never recovers). Every other message is
                // either transient or has other copies close behind it (the
                // job's own further `/ret/`s), so evict the oldest of those
                // instead, scanning forward only in the rare case the queue
                // is nothing but `/new` events.
                let victim = q.iter()
                    .position(|m| !matches!(m, AgentMessage::Event { tag, .. } if tag.ends_with("/new")))
                    .unwrap_or(0);
                q.remove(victim);
                dropped_clone.fetch_add(1, Ordering::Relaxed);
            }
            q.push_back(msg);
        }
    });

    // Stderr reader thread: the agent's stderr is this terminal, which the
    // TUI draws on, so its last lines are kept for the status bar instead.
    let stderr = child.stderr.take().unwrap();
    let stderr_tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL)));
    let tail = stderr_tail.clone();
    let stderr_reader = thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for l in reader.lines().map_while(Result::ok) {
            let mut tail = tail.lock().unwrap();
            if tail.len() == STDERR_TAIL {
                tail.pop_front();
            }
            tail.push_back(l);
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

    // Forward each command the moment it's sent — round-trip timings (the
    // ZMQ panel's agent_ipc latency) depend on it — waking at least every
    // 100ms to notice the agent exiting. Returns the end of its stderr when
    // the agent exited (agent_main respawns it), None when the app has gone
    // away.
    loop {
        match cmds.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(cmd) => { let _ = stdin.write_all(cmd.as_bytes()); }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                let _ = child.kill();
                return None;
            }
        }
        match child.try_wait() {
            Ok(None) => {}
            _ => {
                // Its stderr ends with it; a child of it holding the pipe
                // open mustn't hold up the restart.
                let deadline = Instant::now() + std::time::Duration::from_millis(500);
                while !stderr_reader.is_finished() && Instant::now() < deadline {
                    thread::sleep(std::time::Duration::from_millis(10));
                }
                return Some(std::mem::take(&mut *stderr_tail.lock().unwrap()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_message_quotes_the_agents_last_words() {
        let tail: VecDeque<String> = ["Traceback (most recent call last):", "ModuleNotFoundError: No module named 'salt'  ", ""]
            .into_iter().map(String::from).collect();
        assert_eq!(exit_message(&tail), "agent exited, restarting: ModuleNotFoundError: No module named 'salt'");
        assert_eq!(exit_message(&VecDeque::new()), "agent exited, restarting");
    }
}
