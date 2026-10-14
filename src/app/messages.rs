//! Agent traffic: processing what the agent sends (results, events,
//! progress) and sending it commands, including fleet scan returns.

use super::*;

impl App {
    /// Poll pending agent messages from background thread.
    /// Processes at most MAX_PER_DRAIN messages to prevent runaway when
    /// the event arrival rate exceeds the processing rate.
    pub fn drain_agent_messages(&mut self) {
        const MAX_PER_DRAIN: usize = 200;
        // Take the batch and release the lock before processing it, so the
        // reader thread never stalls behind UI-side work.
        let batch: Vec<AgentMessage> = {
            let mut q = self.agent_rx.lock().unwrap();
            let n = q.len().min(MAX_PER_DRAIN);
            q.drain(..n).collect()
        };
        let mut y_copy_json: Option<String> = None;
        let mut sniffer_tail_moved = false;
        for msg in batch {
            match msg {
                AgentMessage::Result { id, cmd, ok, data, received } => {
                    self.agent.connected = true;
                    self.agent.last_error = None;
                    let is_jid_lookup = cmd == "jid_lookup";
                    let jid_data = if is_jid_lookup { Some(parse_jid_lookup(&data)) } else { None };
                    match cmd.as_str() {
                        "version" => {
                            self.agent.version = Some(parse_version(&data));
                        }
                        "config_snapshot" => {
                            self.config_snapshot = Some(parse_config_snapshot(&data));
                        }
                        "minion_cache_list" if ok => {
                            let mut minions = parse_minion_list(&data);
                            for minion in &mut minions {
                                if minion.key_status.is_none() {
                                    minion.key_status = Some(KeyStatus::Accepted);
                                }
                                // Preserve enriched fields from event processing
                                if let Some(&i) = self.minion_index.get(&minion.id) {
                                    let existing = &self.minion_info[i];
                                    if !existing.osfinger.is_empty() { minion.osfinger.clone_from(&existing.osfinger); }
                                    if !existing.osrelease.is_empty() { minion.osrelease.clone_from(&existing.osrelease); }
                                    if !existing.salt_version.is_empty() { minion.salt_version.clone_from(&existing.salt_version); }
                                    if !existing.cpuarch.is_empty() { minion.cpuarch.clone_from(&existing.cpuarch); }
                                    if !existing.os.is_empty() { minion.os.clone_from(&existing.os); }
                                    if existing.last_seen_epoch > minion.last_seen_epoch { minion.last_seen_epoch = existing.last_seen_epoch; }
                                    if existing.cache_updated_epoch > minion.cache_updated_epoch { minion.cache_updated_epoch = existing.cache_updated_epoch; }
                                    // Fleet scan timed_out status is not known to the cache — preserve it
                                    if existing.timed_out { minion.timed_out = true; }
                                    // Being present in the master's minion cache means it was
                                    // synced at some point — it says nothing about whether the
                                    // minion is reachable *now*. Only alive_check (the real
                                    // liveness probe) should set this; otherwise it fights with
                                    // alive_check's result every cache refresh, flapping the
                                    // Pulse Details panel between Connected/Unknown regardless
                                    // of actual minion state.
                                    minion.connected = existing.connected;
                                }
                            }
                            // Sort: timed-out last, then by ID
                            minions.sort_by(|a, b| {
                                a.timed_out
                                    .cmp(&b.timed_out)
                                    .then(a.id.cmp(&b.id))
                            });
                            self.minion_info = minions;
                            // Roster changed shape — reindex, and drop pulse-tracking
                            // entries for minions no longer in the roster so
                            // last_zmq_return doesn't grow unbounded on churn.
                            self.reindex_minions();
                            let index = &self.minion_index;
                            self.last_zmq_return.retain(|id, _| index.contains_key(id));
                        }
                        "list_keys" => {
                            if ok {
                                self.keys = serde_json::from_value(data["keys"].clone()).unwrap_or_default();
                                self.accepted_id_set = self.keys.iter()
                                    .filter(|k| k.status == KeyStatus::Accepted)
                                    .map(|k| k.id.clone())
                                    .collect();
                                if !self.applied_filter.is_empty() {
                                    self.refresh_events_matcher();
                                }
                            }
                        }
                        "list_processes" => {
                            if ok {
                                if let Some(tck) = data["clk_tck"].as_f64() {
                                    self.clk_tck = tck;
                                }
                                self.processes = serde_json::from_value(data["processes"].clone()).unwrap_or_default();
                                let now = Instant::now();
                                let mut seen_pids: HashSet<u32> = HashSet::new();
                                for p in self.processes.iter_mut() {
                                    seen_pids.insert(p.pid);
                                    if let Some((prev_ticks, prev_at)) = self.prev_proc_cpu.get(&p.pid) {
                                        let elapsed = now.duration_since(*prev_at).as_secs_f64();
                                        if elapsed > 0.0 {
                                            let tick_delta = p.cpu_ticks.saturating_sub(*prev_ticks);
                                            p.cpu_percent = ((tick_delta as f64 / self.clk_tck) / elapsed * 100.0) as f32;
                                        }
                                    }
                                    self.prev_proc_cpu.insert(p.pid, (p.cpu_ticks, now));
                                }
                                self.prev_proc_cpu.retain(|pid, _| seen_pids.contains(pid));
                            }
                        }
                        "list_services" => {
                            if ok {
                                self.services = serde_json::from_value(data["services"].clone()).unwrap_or_default();
                                update_service_trends(&mut self.service_trends, &self.services);
                            }
                        }
                        "get_system_info" => {
                            if ok {
                                self.system_info = serde_json::from_value(data.clone()).unwrap_or_default();
                                let now = Instant::now();
                                if let Some(prev_at) = self.prev_cpu_time {
                                    let elapsed = now.duration_since(prev_at).as_secs_f64();
                                    let total_delta = self.system_info.cpu_total_jiffies.saturating_sub(self.prev_cpu_total_jiffies);
                                    let idle_delta = self.system_info.cpu_idle_jiffies.saturating_sub(self.prev_cpu_idle_jiffies);
                                    if elapsed > 0.0 && total_delta > 0 {
                                        let busy_delta = total_delta.saturating_sub(idle_delta);
                                        self.system_info.cpu_percent = (busy_delta as f64 / total_delta as f64 * 100.0).clamp(0.0, 100.0);
                                    }
                                }
                                self.prev_cpu_total_jiffies = self.system_info.cpu_total_jiffies;
                                self.prev_cpu_idle_jiffies = self.system_info.cpu_idle_jiffies;
                                self.prev_cpu_time = Some(now);
                            }
                        }
                        "get_zmq_connections" => {
                            if ok {
                                self.zmq_connections = serde_json::from_value(data["connections"].clone()).unwrap_or_default();
                            }
                        }
                        "get_api_stats" => {
                            if ok {
                                self.api_stats = ApiStats {
                                    configured: data["configured"].as_bool().unwrap_or(false),
                                    reachable: data["reachable"].as_bool().unwrap_or(false),
                                    port: data["port"].as_u64().unwrap_or(0) as u16,
                                    scheme: data["scheme"].as_str().unwrap_or("").to_string(),
                                    error: data["error"].as_str().unwrap_or("").to_string(),
                                    current_requests: data["current_requests"].as_u64().unwrap_or(0),
                                    total_requests: data["total_requests"].as_u64().unwrap_or(0),
                                    requests_per_sec: data["requests_per_sec"].as_f64().unwrap_or(0.0),
                                    bytes_read_per_sec: data["bytes_read_per_sec"].as_f64().unwrap_or(0.0),
                                    bytes_written_per_sec: data["bytes_written_per_sec"].as_f64().unwrap_or(0.0),
                                    uptime_secs: data["uptime_secs"].as_f64().unwrap_or(0.0),
                                    requests: serde_json::from_value(data["requests"].clone()).unwrap_or_default(),
                                };

                                if self.api_stats.reachable {
                                    let now = Instant::now();
                                    if let Some(prev_at) = self.prev_api_poll_time {
                                        let elapsed = now.duration_since(prev_at).as_secs_f64();
                                        let delta = self.api_stats.total_requests.saturating_sub(self.prev_api_total_requests);
                                        if elapsed > 0.0 {
                                            self.api_requests_per_sec_recent = delta as f64 / elapsed;
                                        }
                                    }
                                    self.prev_api_total_requests = self.api_stats.total_requests;
                                    self.prev_api_poll_time = Some(now);

                                    // Counts entries evicted from the front (oldest) this
                                    // poll, whether by the cap below or the age-based
                                    // prune after the loop, so api_log_selected can be
                                    // shifted to keep pointing at the same logical
                                    // entry -- the same correction Events already makes
                                    // via advance_tail_selection when its buffer evicts.
                                    let mut api_log_evicted: usize = 0;
                                    for rec in &self.api_stats.requests {
                                        if rec.end_time <= 0.0 {
                                            continue;
                                        }
                                        let is_new = self.api_seen_end_times.get(&rec.thread_id)
                                            .map(|prev_end| rec.end_time > *prev_end)
                                            .unwrap_or(true);
                                        self.api_seen_end_times.insert(rec.thread_id.clone(), rec.end_time);
                                        if !is_new {
                                            continue;
                                        }

                                        let is_ok = rec.response_status.split_whitespace().next()
                                            .map(|code| code.starts_with('2') || code.starts_with('3'))
                                            .unwrap_or(true);
                                        self.api_status_window.push_back(is_ok);
                                        if self.api_status_window.len() > self.config.api_stats_window { self.api_status_window.pop_front(); }

                                        if rec.request_line.contains("/login") {
                                            self.api_login_latencies.push_back(rec.processing_time_secs);
                                            if self.api_login_latencies.len() > self.config.api_stats_window { self.api_login_latencies.pop_front(); }
                                        }

                                        let (method, path) = Self::parse_request_line(&rec.request_line);
                                        // salt-radar's own GET /stats poll (the request that
                                        // populates this very panel) is always stored -- it's
                                        // filtered out at display time instead (see
                                        // api_log_entry_visible) so toggling `x` live shows/
                                        // hides it retroactively rather than only affecting
                                        // requests captured from that point on.
                                        self.api_requests_log.push_back(ApiLogEntry {
                                            received_at: now,
                                            client: rec.client.clone(),
                                            method,
                                            path,
                                            raw_request_line: rec.request_line.clone(),
                                            response_status: rec.response_status.clone(),
                                            processing_time_secs: rec.processing_time_secs,
                                        });
                                        if self.api_requests_log.len() > self.config.api_log_cap {
                                            self.api_requests_log.pop_front();
                                            api_log_evicted += 1;
                                        }
                                    }
                                    // Threads that vanish from the snapshot (worker recycled,
                                    // thread pool shrank) would otherwise linger here forever.
                                    let live_threads: HashSet<&str> = self.api_stats.requests.iter().map(|r| r.thread_id.as_str()).collect();
                                    self.api_seen_end_times.retain(|tid, _| live_threads.contains(tid.as_str()));

                                    let len_before_age_prune = self.api_requests_log.len();
                                    let api_log_max_age = Duration::from_secs(self.config.api_log_max_age_secs);
                                    self.api_requests_log.retain(|e| e.received_at.elapsed() < api_log_max_age);
                                    api_log_evicted += len_before_age_prune - self.api_requests_log.len();

                                    self.api_log_selected = self.api_log_selected.saturating_sub(api_log_evicted);
                                }
                            }
                        }
                        "zmq_ping" => {
                            if let Some(sent) = self.zmq_ping_sent_at {
                                let elapsed_ms = received.saturating_duration_since(sent).as_secs_f64() * 1000.0;
                                self.zmq_latency_ms = elapsed_ms;
                                self.zmq_latency_seen = true;
                                self.zmq_latency_history[self.zmq_latency_idx] = elapsed_ms;
                                self.zmq_latency_idx = (self.zmq_latency_idx + 1) % 2000;
                            }
                            self.zmq_ping_sent_at = None;
                        }
                        "job_ping" => {
                            // Fill in the real jid on the pending entry created
                            // at dispatch time — keep that earlier Instant so
                            // the round-trip clock still starts from before
                            // the agent actually published the job, not from
                            // when this response happens to arrive.
                            let jid = if ok { data["jid"].as_str().map(String::from) } else { None };
                            match (jid, self.job_ping_pending.take()) {
                                (Some(jid), Some((_, sent_at))) => {
                                    self.job_ping_pending = Some((jid, sent_at));
                                }
                                _ => {
                                    self.job_ping_pending = None;
                                }
                            }
                        }
                        "alive_check" => {
                            if ok {
                                let up: Vec<String> = data["up"].as_array()
                                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                                    .unwrap_or_default();
                                // Mark up minions connected, clear others
                                for minion in &mut self.minion_info {
                                    minion.connected = up.contains(&minion.id);
                                }
                            }
                        }
                        "minion_ping" => {
                            if let Some(mid) = data["minion_id"].as_str() {
                                let ping_ok = data["connected"].as_bool().unwrap_or(false);
                                if let Some(minion) = self.minion_info.iter_mut().find(|m| m.id == mid) {
                                    minion.connected = ping_ok;
                                }
                                // Update popup state
                                if let Some(DetailPopup::Minion(ref popup_id, ref mut state, _, _)) = self.detail_popup {
                                    if popup_id == mid {
                                        *state = PingState::Result(ping_ok);
                                    }
                                }
                            }
                        }
                        "minion_grains" => {
                            if let Some(mid) = data["minion_id"].as_str() {
                                let pairs: Vec<(String, String)> = data["grains"].as_array()
                                    .map(|arr| {
                                        arr.iter().filter_map(|v| {
                                            let obj = v.as_object()?;
                                            let key = obj.get("key")?.as_str()?;
                                            let val = obj.get("value")?.as_str()?;
                                            Some((key.to_string(), val.to_string()))
                                        }).collect()
                                    })
                                    .unwrap_or_default();
                                if let Some(DetailPopup::Minion(ref popup_id, _, ref mut grains, _)) = self.detail_popup {
                                    if popup_id == mid {
                                        *grains = Some(pairs);
                                    }
                                }
                            }
                        }
                        "fleet_scan" => {
                            if ok {
                                let jid = data["jid"].as_str().unwrap_or("").to_string();
                                let expected: Vec<String> = data["expected_minions"].as_array()
                                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                                    .unwrap_or_default();
                                self.fleet_scan_jid = Some(jid);
                                self.fleet_scan_expected = expected;
                                self.fleet_scan_responded.clear();
                                self.fleet_scan_started_at = Some(Instant::now());
                                self.fleet_scanning = true;
                                self.fleet_scan_completed = false;
                                self.fleet_data = Some(FleetScan {
                                    minions: Vec::new(),
                                    scanned_at: Instant::now(),
                                });
                            } else {
                                self.fleet_scanning = false;
                                self.fleet_scan_completed = true;
                            }
                        }
                        "jobs_list" if id == self.cache_request_id => {
                            self.cache_loading_at = None;
                            self.cache_load_progress = None;
                            if ok {
                                self.jobs_cache = parse_cache_jobs(&data);
                                self.cache_total_count = data["total_count"].as_u64().unwrap_or(0) as usize;
                                self.cache_loaded = true;
                                self.cache_load_error = None;
                            } else {
                                self.cache_load_error = data["error"].as_str().map(|s| s.to_string());
                            }
                            self.cache_loading = false;
                        }
                        _ => {}
                    }
                    if !ok {
                        self.agent.last_error = data["error"].as_str().map(|s| s.to_string());
                    }
                    // Handle jid_lookup outside the mutable borrow of match
                    if is_jid_lookup {
                        if let Some(jid_data) = jid_data {
                            if let Some((pending_id, _)) = self.pending_jid_lookup {
                                if id == pending_id {
                                    self.pending_jid_lookup = None;
                                    self.pending_jid_lookup_at = None;
                                    // On a refresh, stay on the minion that was selected; a refresh
                                    // that fails keeps what's already on screen.
                                    let mut keep_minion: Option<String> = None;
                                    if let Some(DetailPopup::JobDetail { ref mut lookup, selected_return, .. }) = self.detail_popup {
                                        let refresh_failed = jid_data.entry_count() == 0 && !jid_data.error.is_empty();
                                        match lookup {
                                            Some(prev) if prev.entry_count() > 0 && refresh_failed => {}
                                            Some(prev) => {
                                                keep_minion = prev.entry_id(selected_return.unwrap_or(0)).map(String::from);
                                                *lookup = Some(jid_data.clone());
                                            }
                                            None => *lookup = Some(jid_data.clone()),
                                        }
                                    }
                                    let select_minion = self.job_detail_select_minion.take().or(keep_minion);
                                    // Opened from a minion's activity: select that minion's return.
                                    // Otherwise snap to the first filter match if one was committed before data arrived.
                                    if let Some(DetailPopup::JobDetail { ref return_filter, ref mut selected_return, ref lookup, .. }) = self.detail_popup {
                                        if let (Some(mid), Some(l)) = (select_minion, lookup) {
                                            if let Some(idx) = (0..l.entry_count()).find(|&i| l.entry_id(i) == Some(mid.as_str())) {
                                                *selected_return = Some(idx);
                                            }
                                        } else if !return_filter.is_empty() {
                                            if let Some(ref l) = lookup {
                                                let first = (0..l.entry_count()).find(|&i| l.entry_id(i).is_some_and(|id| crate::app::return_matches(id, return_filter)));
                                                if let Some(idx) = first {
                                                    *selected_return = Some(idx);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            // Event Detail's own pretty-view fetch: find this
                            // specific minion's real formatted_text (or note
                            // its absence) and hand it to the open popup.
                            if let Some((ev_id, _, ref mid)) = self.pending_event_pretty {
                                if id == ev_id {
                                    let text = jid_data.returns.iter()
                                        .find(|r| &r.minion_id == mid)
                                        .map(|r| r.formatted_text.clone())
                                        .unwrap_or_default();
                                    self.pending_event_pretty = None;
                                    self.pending_event_pretty_at = None;
                                    if let Some(DetailPopup::SaltEvent { ref mut pretty_fetch, .. }) = self.detail_popup {
                                        *pretty_fetch = if text.trim().is_empty() {
                                            crate::types::PrettyFetch::Failed
                                        } else {
                                            crate::types::PrettyFetch::Ready(text)
                                        };
                                    }
                                }
                            }
                            // Y key pending: copy pretty JSON of selected return
                            if let Some((y_id, _)) = self.pending_y_copy {
                                if id == y_id {
                                    self.pending_y_copy = None;
                                    let idx = match self.detail_popup {
                                        Some(DetailPopup::JobDetail { selected_return, .. }) => selected_return.unwrap_or(0),
                                        _ => 0,
                                    };
                                    if idx < jid_data.returns.len() {
                                        let json = serde_json::to_string_pretty(&jid_data.returns[idx].full_return)
                                            .unwrap_or_else(|_| jid_data.returns[idx].summary.clone());
                                        y_copy_json = Some(json);
                                    }
                                }
                            }
                        }
                    }
                }
                AgentMessage::Event { tag, data, ts, received } => {
                    // Track event breakdown
                    self.event_breakdown.record(&tag);
                    if tag.starts_with("salt/auth") {
                        self.auth_events_this_sec += 1;
                    }

                    // ZMQ panel latency, source "master_stats": Salt's own
                    // instrumentation (requires master_stats: True in the master
                    // config, which salt-radar doesn't set itself) — one event
                    // per worker process, each carrying that worker's own mean
                    // command-execution times since its last reset. Collapse to
                    // a single weighted-mean sample per event.
                    if self.config.zmq_latency_source == "master_stats" && tag.starts_with("salt/stats/") {
                        if let Some(stats) = data["stats"].as_object() {
                            let mut weighted = 0.0;
                            let mut total_runs: u64 = 0;
                            for v in stats.values() {
                                let mean_secs = v["mean"].as_f64().unwrap_or(0.0);
                                let runs = v["runs"].as_u64().unwrap_or(0);
                                weighted += mean_secs * runs as f64;
                                total_runs += runs;
                            }
                            if total_runs > 0 {
                                let avg_ms = weighted / total_runs as f64 * 1000.0;
                                self.zmq_latency_ms = avg_ms;
                                self.zmq_latency_seen = true;
                                self.zmq_latency_history[self.zmq_latency_idx] = avg_ms;
                                self.zmq_latency_idx = (self.zmq_latency_idx + 1) % 2000;
                            }
                        }
                    }

                    // ZMQ panel latency, source "job_ping": time our own
                    // periodic test.ping from dispatch to its first real
                    // return over the actual event bus/ZMQ transport.
                    if let Some((ref pending_jid, sent_at)) = self.job_ping_pending {
                        if !pending_jid.is_empty() && tag.starts_with("salt/job/") && tag.contains(pending_jid.as_str()) && tag.contains("/ret/") {
                            let elapsed_ms = received.saturating_duration_since(sent_at).as_secs_f64() * 1000.0;
                            self.zmq_latency_ms = elapsed_ms;
                            self.zmq_latency_seen = true;
                            self.zmq_latency_history[self.zmq_latency_idx] = elapsed_ms;
                            self.zmq_latency_idx = (self.zmq_latency_idx + 1) % 2000;
                            self.job_ping_pending = None;
                        }
                    }

                    // ── Normal event processing ─────────────────────────
                    // Track return events for activity matrix
                    if tag.contains("/ret/") {
                        self.note_return_for_open_job(&tag);
                        self.current_sec_returns += 1;
                        let (success, _) = return_outcome(&data, &data["return"]);
                        self.job_returns_window.push_back(success);
                        if self.job_returns_window.len() > JOB_STATS_WINDOW { self.job_returns_window.pop_front(); }
                        if let Some(mid) = tag.rsplit('/').next() {
                            if !mid.is_empty() {
                                self.record_minion_return(mid, ts, &data);
                                self.last_activity_minion = mid.to_string();
                                self.last_activity_fun = data["fun"].as_str().unwrap_or("?").to_string();
                                self.last_activity_at = Instant::now();
                                // Update last_seen for this minion from return events
                                let ts_epoch = ts as u64;
                                let minion_pos = self.minion_index.get(mid).copied();
                                if let Some(i) = minion_pos {
                                    let info = &mut self.minion_info[i];
                                    if ts_epoch > info.last_seen_epoch {
                                        info.last_seen_epoch = ts_epoch;
                                    }
                                    if info.key_status.is_none() {
                                        info.key_status = Some(KeyStatus::Accepted);
                                    }
                                }
                                // Passive harvest: populate minion data from grains.item/items returns
                                let fun = data["fun"].as_str().unwrap_or("");
                                if fun == "grains.items" || fun == "grains.item" {
                                    if let Some(return_data) = data["return"].as_object() {
                                        let osfinger = return_data.get("osfinger").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let osrelease = return_data.get("osrelease").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let os = return_data.get("os").or_else(|| return_data.get("osfullname")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let salt_version = return_data.get("saltversion").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let cpuarch = return_data.get("cpuarch").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let known = !osfinger.is_empty() || !salt_version.is_empty() || !os.is_empty();
                                        if known {
                                            let fm = if let Some(i) = minion_pos {
                                                let existing = &mut self.minion_info[i];
                                                if !osfinger.is_empty() { existing.osfinger = osfinger; }
                                                if !osrelease.is_empty() { existing.osrelease = osrelease; }
                                                if !os.is_empty() { existing.os = os; }
                                                if !salt_version.is_empty() { existing.salt_version = salt_version; }
                                                if !cpuarch.is_empty() { existing.cpuarch = cpuarch; }
                                                if existing.timed_out {
                                                    existing.timed_out = false;
                                                    self.minion_info_dirty = true;
                                                }
                                                if existing.key_status.is_none() {
                                                    existing.key_status = Some(KeyStatus::Accepted);
                                                }
                                                // Also populate fleet_data from the merged minion_info
                                                Some(FleetMinion {
                                                    id: mid.to_string(),
                                                    osfinger: existing.osfinger.clone(),
                                                    osrelease: existing.osrelease.clone(),
                                                    saltversion: existing.salt_version.clone(),
                                                    cpuarch: existing.cpuarch.clone(),
                                                    timed_out: false,
                                                })
                                            } else {
                                                None
                                            };
                                            if let Some(fm) = fm {
                                                if let Some(ref mut fd) = self.fleet_data {
                                                    if let Some(existing) = fd.minions.iter_mut().find(|m| m.id == mid) {
                                                        *existing = fm;
                                                    } else {
                                                        fd.minions.push(fm);
                                                    }
                                                } else {
                                                    self.fleet_data = Some(FleetScan {
                                                        minions: vec![fm],
                                                        scanned_at: Instant::now(),
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                                // Pulse tracking: every return event is minion activity
                                self.last_zmq_return.insert(mid.to_string(), Instant::now());
                            }
                        }
                    } else {
                        // Non-return events: try extracting minion ID from data for Pulse
                        if let Some(mid) = data["id"].as_str() {
                            if !mid.is_empty() {
                                self.last_zmq_return.insert(mid.to_string(), Instant::now());
                            }
                        }
                    }
                    let evt = SaltEvent { tag, data, ts };
                    self.events_since_last_tick += 1;
                    let events_evicted = self.events.len() >= self.config.event_buffer_size;
                    let evicted_evt = if events_evicted { self.events.pop_front() } else { None };
                    // O(1) amortized: checks only the one event coming in and
                    // (at most) the one going out, instead of re-scanning the
                    // whole buffer -- see `sync_filtered_events_cache`. A
                    // busy master can push hundreds of events through a
                    // single drain_agent_messages() call, and a full re-scan
                    // per event (or even once per tick) here is what made the
                    // TUI stall/black out while an Events filter was active
                    // (#98).
                    Self::sync_filtered_events_cache(
                        &mut self.filtered_events_cache,
                        &self.applied_filter,
                        &self.events_matcher,
                        &evt,
                        evicted_evt.as_ref(),
                    );
                    self.events.push_back(evt);
                    self.events_total += 1;
                    self.last_event_time = Some(Instant::now());
                    if self.applied_filter.is_empty() {
                        Self::advance_tail_selection(&mut self.event_selected, self.events_following, events_evicted, self.events.len());
                    } else if self.events_following {
                        self.event_selected = self.filtered_events_cache.len().saturating_sub(1);
                    }

                    // Track live jobs from events
                    if let Some(evt) = self.events.back() {
                        if let Some(timing) = track_job_from_event(&mut self.jobs, &mut self.jobs_index, evt) {
                            let fun = evt.data["fun"].as_str().unwrap_or("").to_string();
                            self.record_return_timing(timing, &fun);
                        }
                    }

                    // Sniffer: inject matching events into sniffer buffer
                    if let Some(ref mut state) = self.sniffer {
                        if !state.paused {
                            if let Some(evt) = self.events.back() {
                                if crate::sniff::matches(&state.terms, evt) {
                                    let will_evict = state.buffer.len() >= state.max_size;
                                    // `selected`/`following` track the filtered (displayed)
                                    // view, not the raw buffer — only touch them when this
                                    // push/evict actually changes what's shown, i.e. when the
                                    // evicted item (if any) or the new item itself passes the
                                    // active highlight-keyword filter.
                                    let evicted_visible = will_evict && state.buffer.front()
                                        .map(|e| state.highlight_terms.is_empty() || crate::sniff::matches(&state.highlight_terms, e))
                                        .unwrap_or(false);
                                    if will_evict {
                                        state.buffer.pop_front();
                                    }
                                    state.buffer.push_back(evt.clone());
                                    let new_visible = state.highlight_terms.is_empty()
                                        || crate::sniff::matches(&state.highlight_terms, evt);
                                    // The visible length is an O(buffer) keyword
                                    // re-scan, so a following selection is
                                    // resynced once after the batch instead of
                                    // per captured event.
                                    if state.following {
                                        sniffer_tail_moved |= new_visible || evicted_visible;
                                    } else if evicted_visible {
                                        state.selected = state.selected.saturating_sub(1);
                                    }
                                    self.sniffer_dirty = true;
                                }
                            }
                        }
                    }
                }
                AgentMessage::Error { message, .. } => {
                    self.agent.last_error = Some(message);
                    self.agent.connected = false;
                }
                AgentMessage::Progress { cmd, loaded } => {
                    if cmd == "jobs_list" {
                        self.cache_load_progress = Some(loaded);
                    }
                }
            }
        }
        if sniffer_tail_moved {
            if let Some(ref mut state) = self.sniffer {
                let visible_len = state.filtered_indices().len();
                Self::advance_tail_selection(&mut state.selected, state.following, false, visible_len);
            }
        }
        if let Some(json) = y_copy_json {
            // Store in a temp file for clipboard
            let _ = std::fs::write(&self.copy_path, &json);
            self.clipboard_msg = Some((
                format!(" Copied {} bytes of JSON job return -- use y for display format ", json.len()),
                std::time::Instant::now(),
            ));
            self.clipboard_at = Some(std::time::Instant::now());
        }
    }

    pub(crate) fn send_jid_lookup(&mut self, jid: &str) -> u64 {
        let timeout = self.config.lookup_timeout_secs;
        self.send_agent_cmd("jid_lookup", Some(serde_json::json!({"jid": jid, "timeout": timeout})))
    }

    /// Send a command to the agent, returns the request id.
    /// Appends to the command file (avoids overwriting a concurrently-written command).
    pub fn send_agent_cmd(&mut self, cmd: &str, extra: Option<serde_json::Value>) -> u64 {
        self.agent_req_id += 1;
        let id = self.agent_req_id;
        let mut req = serde_json::json!({
            "id": id,
            "cmd": cmd,
        });
        if let Some(e) = extra {
            if let Some(obj) = req.as_object_mut() {
                for (k, v) in e.as_object().unwrap_or(&serde_json::Map::new()) {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
        let _ = self.agent_cmd_tx.send(serde_json::to_string(&req).unwrap() + "\n");
        id
    }

    pub(crate) fn send_cache_jobs_list(&mut self, filter: Option<&str>, reset_offset: bool) {
        if reset_offset {
            self.cache_offset = 0;
        }
        let mut extra = serde_json::json!({"max_results": self.cache_max_results, "offset": self.cache_offset});
        if let Some(f) = filter {
            extra["filter_text"] = serde_json::Value::String(f.to_string());
        }
        if let Some((col, desc)) = self.sort_of(TableId::JobCache) {
            extra["sort_key"] = serde_json::Value::String(col.key().to_string());
            extra["sort_desc"] = serde_json::Value::Bool(desc);
        }
        self.cache_loading = true;
        self.cache_loading_at = Some(Instant::now());
        self.cache_loaded = false;
        self.cache_load_error = None;
        self.cache_load_progress = None;
        self.cache_request_id = self.send_agent_cmd("jobs_list", Some(extra));
    }

    pub(crate) fn process_fleet_returns(&mut self) {
        let jid = match &self.fleet_scan_jid {
            Some(j) => j.clone(),
            None => return,
        };
        let prefix = format!("salt/job/{}/ret/", jid);
        for evt in &self.events {
            if !evt.tag.starts_with(&prefix) {
                continue;
            }
            let mid = match evt.data["id"].as_str() {
                Some(m) if !m.is_empty() => m.to_string(),
                _ => continue,
            };
            if evt.data["fun"].as_str() != Some("grains.items") {
                continue;
            }
            let grains = &evt.data["return"];
            if !grains.is_object() {
                continue;
            }
            let osfinger = grains.get("osfinger").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let osrelease = grains.get("osrelease").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let saltversion = grains.get("saltversion").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let cpuarch = grains.get("cpuarch").and_then(|v| v.as_str()).unwrap_or("").to_string();

            // Merge into minion_info
            if let Some(existing) = self.minion_info.iter_mut().find(|m| m.id == mid) {
                existing.osfinger = osfinger.clone();
                existing.osrelease = osrelease.clone();
                existing.salt_version = saltversion.clone();
                existing.cpuarch = cpuarch.clone();
                if existing.timed_out {
                    existing.timed_out = false;
                    self.minion_info_dirty = true;
                }
                if existing.key_status.is_none() {
                    existing.key_status = Some(KeyStatus::Accepted);
                }
            }

            // Merge into fleet_data for overview
            if let Some(ref mut fd) = self.fleet_data {
                let fm = FleetMinion {
                    id: mid.clone(),
                    osfinger,
                    osrelease,
                    saltversion,
                    cpuarch,
                    timed_out: false,
                };
                if let Some(existing) = fd.minions.iter_mut().find(|m| m.id == mid) {
                    *existing = fm;
                } else {
                    fd.minions.push(fm);
                }
            }

            // Track as responded and remove from expected list
            self.fleet_scan_responded.insert(mid.clone());
            if !self.fleet_scan_expected.is_empty() {
                self.fleet_scan_expected.retain(|e| *e != mid);
            }
        }
        if self.fleet_scanning && self.fleet_scan_all_responded() {
            self.finish_fleet_scan();
        }
    }

    /// Every minion the scan is waiting on has answered: all accepted keys,
    /// or — when the key list isn't available — the agent's own expected
    /// list, which shrinks as answers arrive.
    pub(crate) fn fleet_scan_all_responded(&self) -> bool {
        let mut accepted = self.keys.iter().filter(|k| k.status == KeyStatus::Accepted).peekable();
        if accepted.peek().is_some() {
            accepted.all(|k| self.fleet_scan_responded.contains(&k.id))
        } else {
            !self.fleet_scan_responded.is_empty() && self.fleet_scan_expected.is_empty()
        }
    }

    pub(crate) fn finish_fleet_scan(&mut self) {
        self.fleet_scan_jid = None;
        self.fleet_scan_responded.clear();
        self.fleet_scan_expected.clear();
        self.fleet_scan_started_at = None;
        self.fleet_scanning = false;
        self.fleet_scan_completed = true;
    }
}
