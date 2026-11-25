//! The Stats tab's numbers: event rates per tag family since the window
//! started, and the master's load over the same window. Everything is a rate
//! or an average, so two masters can be compared however long salt-radar has
//! been running on each.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use crate::types::{is_jid, CpuThrottling, DiskCounters, MinionInfo, SaltProcess, SaltProcessRole, SystemInfo};

/// Families every master shows, first and in this order, even with no
/// events yet, so two salt-radar screens side by side line up row by row.
pub const CORE_FAMILIES: &[&str] = &["salt/auth", "minion_start", "salt/minion/*/start", "salt/job/*/new", "salt/job/*/ret/*"];

/// Where events go once this many families exist: custom tags can carry
/// anything (paths, timestamps), and each family keeps per-minion counts.
const MAX_FAMILIES: usize = 300;
pub const OTHER_FAMILY: &str = "(other tags)";
pub const JID_FAMILY: &str = "<jid>";
pub const RETURN_FAMILY: &str = "salt/job/*/ret/*";
pub const JOB_NEW_FAMILY: &str = "salt/job/*/new";
pub const RUN_NEW_FAMILY: &str = "salt/run/*/new";
pub const AUTH_FAMILY: &str = "salt/auth";
/// Distinct values a field is counted for per family; the rest share one.
const MAX_VALUES: usize = 200;
pub const OTHER_VALUE: &str = "(other)";
/// One per pillar a minion asked the master for, fired once the master
/// stored it in its minion data cache (see #216).
pub const PILLAR_FAMILY: &str = "minion/refresh/*";
const MAX_MINIONS_PER_FAMILY: usize = 50_000;
const MAX_INTERVALS: usize = 1000;
/// The "last 15 minutes" columns.
pub const RECENT_MINUTES: u64 = 15;
const RSS_SAMPLE_EVERY: Duration = Duration::from_secs(60);
const RSS_SAMPLES: usize = 24 * 60;

/// One minion's share of a family (or the events no minion could be named
/// for): its events, their size, how many of them were failed job returns,
/// and its busiest minute.
#[derive(Debug, Default)]
pub struct PerMinion {
    pub count: u64,
    pub bytes: u64,
    pub failed: u64,
    pub per_minute: Throughput,
    last_ts: Option<f64>,
}

impl PerMinion {
    fn merge(&mut self, other: PerMinion) {
        self.count += other.count;
        self.bytes += other.bytes;
        self.failed += other.failed;
        self.per_minute.merge(&other.per_minute);
        self.last_ts = match (self.last_ts, other.last_ts) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
    }

    /// Counts an event; returns when the previous one came.
    fn record(&mut self, ts: f64, minute: u64, bytes: u64, failed: bool) -> Option<f64> {
        self.count += 1;
        self.bytes += bytes;
        self.failed += u64::from(failed);
        self.per_minute.add(1, minute);
        self.last_ts.replace(ts)
    }
}

#[derive(Debug, Default)]
pub struct Family {
    pub count: u64,
    pub bytes: u64,
    /// Job returns judged failed (only `salt/job/*/ret/*` has any).
    pub failed: u64,
    /// Events per minute: the busiest minute and when it was.
    pub per_minute: Throughput,
    /// (minute of the window, events, bytes) for the last `RECENT_MINUTES`.
    minutes: VecDeque<(u64, u64, u64)>,
    pub minions: HashMap<String, PerMinion>,
    /// Events no minion could be named for (`salt/job/*/new`, runners).
    pub unattributed: PerMinion,
    /// Seconds between two events of the family from the same minion (or
    /// between two unattributed ones), the latest `MAX_INTERVALS`.
    intervals: VecDeque<f64>,
    /// Its events by the values of the fields some groupings read.
    pub values: HashMap<Grouping, HashMap<String, ValueCount>>,
}

impl Family {
    fn record(&mut self, minion: Option<String>, ts: f64, minute: u64, bytes: u64, failed: bool, values: Vec<(Grouping, String)>) {
        self.count += 1;
        self.bytes += bytes;
        self.failed += u64::from(failed);
        self.per_minute.add(1, minute);
        match self.minutes.back_mut() {
            Some((m, n, b)) if *m == minute => {
                *n += 1;
                *b += bytes;
            }
            _ => self.minutes.push_back((minute, 1, bytes)),
        }
        while self.minutes.front().is_some_and(|(m, _, _)| m + RECENT_MINUTES <= minute) {
            self.minutes.pop_front();
        }
        let full = self.minions.len() >= MAX_MINIONS_PER_FAMILY;
        let minion = minion.filter(|id| self.minions.contains_key(id) || !full);
        for (grouping, value) in values {
            let per_value = self.values.entry(grouping).or_default();
            let value = if per_value.len() >= MAX_VALUES && !per_value.contains_key(&value) { OTHER_VALUE.to_string() } else { value };
            let count = per_value.entry(value).or_default();
            count.total.record(ts, minute, bytes, failed);
            match &minion {
                Some(id) => count.minions.entry(id.clone()).or_default().record(ts, minute, bytes, failed),
                None => count.unattributed.record(ts, minute, bytes, failed),
            };
        }
        let who = match minion {
            Some(id) => self.minions.entry(id).or_default(),
            None => &mut self.unattributed,
        };
        if let Some(prev) = who.record(ts, minute, bytes, failed).filter(|&prev| ts > prev) {
            self.intervals.push_back(ts - prev);
            if self.intervals.len() > MAX_INTERVALS {
                self.intervals.pop_front();
            }
        }
    }

    /// Events and bytes in the last `RECENT_MINUTES` minutes, the current
    /// one included.
    fn recent(&self, minute: u64) -> (u64, u64) {
        self.minutes.iter()
            .filter(|(m, _, _)| m + RECENT_MINUTES > minute)
            .fold((0, 0), |(n, b), (_, en, eb)| (n + en, b + eb))
    }

    /// Average size of its events, in bytes.
    pub fn avg_size(&self) -> Option<f64> {
        (self.count > 0).then(|| self.bytes as f64 / self.count as f64)
    }

    /// Takes in a family recorded under another name: its events go to
    /// `minion` when the old name was that minion's tag.
    fn merge(&mut self, other: Family, minion: Option<String>) {
        self.count += other.count;
        self.bytes += other.bytes;
        self.failed += other.failed;
        self.per_minute.merge(&other.per_minute);
        for (minute, n, b) in other.minutes {
            match self.minutes.iter_mut().find(|(m, _, _)| *m == minute) {
                Some((_, sn, sb)) => {
                    *sn += n;
                    *sb += b;
                }
                None => self.minutes.push_back((minute, n, b)),
            }
        }
        self.minutes.make_contiguous().sort_by_key(|(m, _, _)| *m);
        for (id, pm) in other.minions {
            self.minions.entry(id).or_default().merge(pm);
        }
        match &minion {
            Some(id) => self.minions.entry(id.clone()).or_default().merge(other.unattributed),
            None => self.unattributed.merge(other.unattributed),
        }
        self.intervals.extend(other.intervals);
        while self.intervals.len() > MAX_INTERVALS {
            self.intervals.pop_front();
        }
        for (grouping, values) in other.values {
            let mine = self.values.entry(grouping).or_default();
            for (value, count) in values {
                mine.entry(value).or_default().merge(count, minion.as_deref());
            }
        }
    }

    fn sorted_intervals(&self) -> Vec<f64> {
        let mut v: Vec<f64> = self.intervals.iter().copied().collect();
        v.sort_by(f64::total_cmp);
        v
    }

    pub fn median_interval(&self) -> Option<f64> {
        let v = self.sorted_intervals();
        (v.len() >= 3).then(|| v[v.len() / 2])
    }

    /// Its events come at regular intervals: most of them within 10% of
    /// the median, like a minion restarting every 5 minutes or a schedule.
    pub fn periodic(&self) -> bool {
        let v = self.sorted_intervals();
        if v.len() < 10 {
            return false;
        }
        let median = v[v.len() / 2];
        let near = v.iter().filter(|&&i| (i - median).abs() <= median * 0.1).count();
        median >= 1.0 && near * 10 >= v.len() * 6
    }
}

/// One sampled figure of the master's load: its latest value, average,
/// highest and lowest over the window.
#[derive(Clone, Copy, Debug, Default)]
pub struct Metric {
    pub now: f64,
    pub peak: f64,
    pub low: f64,
    sum: f64,
    n: u64,
}

impl Metric {
    fn add(&mut self, v: f64) {
        self.now = v;
        self.peak = if self.n == 0 { v } else { self.peak.max(v) };
        self.low = if self.n == 0 { v } else { self.low.min(v) };
        self.sum += v;
        self.n += 1;
    }

    pub fn avg(&self) -> Option<f64> {
        (self.n > 0).then(|| self.sum / self.n as f64)
    }
}

/// Events (or bytes) counted per minute of the window, for the rate now (the
/// last whole minute), on average and in the busiest minute.
#[derive(Clone, Debug, Default)]
pub struct Throughput {
    pub total: u64,
    minute: u64,
    in_minute: u64,
    last_minute: u64,
    /// Events (or bytes) in the busiest minute.
    pub peak_minute: u64,
    /// Which minute of the window was the busiest.
    pub peak_at: u64,
}

impl Throughput {
    /// Adds another count kept over the same window. Of earlier minutes
    /// only the busiest of each is known, so the busier of the two stands
    /// for both; the minute under way adds up.
    fn merge(&mut self, other: &Throughput) {
        self.total += other.total;
        if other.peak_minute > self.peak_minute {
            self.peak_minute = other.peak_minute;
            self.peak_at = other.peak_at;
        }
        if other.minute == self.minute {
            self.in_minute += other.in_minute;
            self.last_minute += other.last_minute;
            if self.in_minute > self.peak_minute {
                self.peak_minute = self.in_minute;
                self.peak_at = self.minute;
            }
        }
    }

    fn add(&mut self, n: u64, minute: u64) {
        self.roll(minute);
        self.total += n;
        self.in_minute += n;
        if self.in_minute > self.peak_minute {
            self.peak_minute = self.in_minute;
            self.peak_at = minute;
        }
    }

    fn roll(&mut self, minute: u64) {
        if minute != self.minute {
            self.last_minute = if minute == self.minute + 1 { self.in_minute } else { 0 };
            self.minute = minute;
            self.in_minute = 0;
        }
    }

    /// Per hour over the last whole minute; None in the window's first one.
    pub fn now_per_hour(&self, minute: u64) -> Option<f64> {
        let last = if minute == self.minute {
            self.last_minute
        } else if minute == self.minute + 1 {
            self.in_minute
        } else {
            0
        };
        (minute > 0).then_some(last as f64 * 60.0)
    }

    pub fn peak_per_hour(&self) -> f64 {
        self.peak_minute as f64 * 60.0
    }
}

/// How the MWorkers spent their time, from the `salt/stats/<worker>` events
/// a master with `master_stats: True` fires: each says how long the worker
/// ran since its last one and the runs and mean time of each request type.
#[derive(Clone, Debug, Default)]
pub struct WorkerTime {
    /// Seconds the workers ran, summed.
    pub total_secs: f64,
    /// Request type (`_pillar`, `_return`, `_auth`, ...): runs and seconds.
    pub by_request: HashMap<String, (u64, f64)>,
    /// Each request type's share of the workers' time per whole minute.
    per_minute: HashMap<String, Metric>,
    minute: u64,
    /// The minute under way: the workers' seconds, and each type's.
    in_minute: (f64, HashMap<String, f64>),
}

impl WorkerTime {
    fn record(&mut self, data: &serde_json::Value, minute: u64) {
        let Some(stats) = data["stats"].as_object() else { return };
        if minute != self.minute {
            self.roll();
            self.minute = minute;
        }
        let time = data["time"].as_f64().unwrap_or(0.0).max(0.0);
        self.total_secs += time;
        self.in_minute.0 += time;
        for (request, v) in stats {
            let runs = v["runs"].as_u64().unwrap_or(0);
            if runs == 0 {
                continue;
            }
            let secs = runs as f64 * v["mean"].as_f64().unwrap_or(0.0).max(0.0);
            let e = self.by_request.entry(request.clone()).or_default();
            e.0 += runs;
            e.1 += secs;
            *self.in_minute.1.entry(request.clone()).or_default() += secs;
        }
    }

    /// Closes the minute under way: every request type seen so far gets its
    /// share of it, none at all included.
    fn roll(&mut self) {
        let (total, secs) = std::mem::take(&mut self.in_minute);
        if total <= 0.0 {
            return;
        }
        for request in self.by_request.keys() {
            let share = secs.get(request).copied().unwrap_or(0.0) / total;
            self.per_minute.entry(request.clone()).or_default().add(share);
        }
    }

    /// Request types by their share of the workers' time over the window,
    /// largest first.
    pub fn shares(&self) -> Vec<(&str, f64)> {
        if self.total_secs <= 0.0 {
            return Vec::new();
        }
        let mut v: Vec<(&str, f64)> = self.by_request.iter().map(|(k, (_, secs))| (k.as_str(), secs / self.total_secs)).collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        v
    }

    /// A request type's share in the last whole minute and its largest in
    /// one minute, once a minute has passed.
    pub fn minute_share(&self, request: &str) -> Option<&Metric> {
        self.per_minute.get(request)
    }

    /// Mean seconds one request of this type took.
    pub fn mean(&self, request: &str) -> Option<f64> {
        self.by_request.get(request).filter(|(runs, _)| *runs > 0).map(|(runs, secs)| secs / *runs as f64)
    }
}

/// The work the master does, per minute of the window.
#[derive(Clone, Debug, Default)]
pub struct Flow {
    pub events: Throughput,
    /// Size of the events as the agent passes them on (JSON, a little more
    /// than went over the bus as msgpack).
    pub bytes: Throughput,
    pub returns: Throughput,
    pub failed_returns: u64,
    pub jobs: Throughput,
    pub pillar_requests: Throughput,
    /// `salt/auth` by what the master did: accept, pend, reject, denied.
    pub auth: Throughput,
    pub auth_by_act: HashMap<String, u64>,
    pub worker_time: WorkerTime,
}

impl Flow {
    /// Counts an event; returns whether it was a failed job return.
    fn record(&mut self, tag: &str, data: &serde_json::Value, bytes: u64, minute: u64) -> bool {
        self.events.add(1, minute);
        self.bytes.add(bytes, minute);
        if tag.starts_with("salt/job/") && tag.contains("/ret/") {
            self.returns.add(1, minute);
            let failed = !crate::app::return_outcome(data, &data["return"]).0;
            self.failed_returns += u64::from(failed);
            return failed;
        } else if tag.starts_with("salt/job/") && tag.ends_with("/new") {
            self.jobs.add(1, minute);
        } else if tag.starts_with("minion/refresh/") {
            self.pillar_requests.add(1, minute);
        } else if tag == "salt/auth" {
            self.auth.add(1, minute);
            *self.auth_by_act.entry(data["act"].as_str().unwrap_or("?").to_string()).or_default() += 1;
        } else if tag.starts_with("salt/stats/") {
            self.worker_time.record(data, minute);
        }
        false
    }

    pub fn failed_share(&self) -> Option<f64> {
        (self.returns.total > 0).then(|| self.failed_returns as f64 / self.returns.total as f64)
    }
}

/// Whether the master is starving for a resource rather than just busy,
/// from the host's counters between two polls of `get_system_info` and
/// `list_processes`. A reading the host doesn't give stays empty.
#[derive(Debug, Default)]
pub struct Saturation {
    pub cpu_busy_pct: Metric,
    pub steal_pct: Metric,
    pub iowait_pct: Metric,
    /// Processes ready to run per core.
    pub run_queue: Metric,
    pub blocked: Metric,
    pub psi_cpu_pct: Metric,
    pub psi_memory_pct: Metric,
    pub psi_io_pct: Metric,
    pub mem_avail_pct: Metric,
    /// Pages swapped in and out per second.
    pub swap_per_sec: Metric,
    /// The disk holding the job cache: how long a request took on average,
    /// how much of the time it was busy, and megabytes moved per second.
    /// A slow disk at low throughput is a throttled (cloud) volume.
    pub disk: Option<String>,
    pub disk_latency_ms: Metric,
    pub disk_busy_pct: Metric,
    pub disk_mb_per_sec: Metric,
    /// Share of TCP segments sent again: loss between minions and master.
    pub retransmit_pct: Metric,
    /// Since the window started.
    pub oom_kills: Option<u64>,
    pub listen_drops: Option<u64>,
    /// The salt-master process closest to its open files limit: its share
    /// of the limit, and its count and limit at the last poll.
    pub open_files_pct: Metric,
    pub open_files: Option<(u64, u64)>,
    /// Share of the cgroup's CPU periods its quota held it back.
    pub throttled_pct: Metric,
    pub quota: bool,
    prev_system: Option<(Instant, SystemInfo)>,
    /// Cumulative counters when the window started.
    base_oom: Option<u64>,
    base_drops: Option<u64>,
    prev_throttling: Option<CpuThrottling>,
}

impl Saturation {
    pub fn record_system(&mut self, info: &SystemInfo, cpu_count: u32, now: Instant) {
        let drops = info.listen_overflows.zip(info.listen_drops).map(|(o, d)| o.max(d));
        self.base_oom = self.base_oom.or(info.oom_kills);
        self.base_drops = self.base_drops.or(drops);
        self.oom_kills = info.oom_kills.zip(self.base_oom).map(|(n, b)| n.saturating_sub(b));
        self.listen_drops = drops.zip(self.base_drops).map(|(n, b)| n.saturating_sub(b));
        if info.mem_total_kb > 0 {
            self.mem_avail_pct.add(info.mem_avail_kb as f64 * 100.0 / info.mem_total_kb as f64);
        }
        if let Some(running) = info.procs_running {
            self.run_queue.add(running as f64 / cpu_count.max(1) as f64);
        }
        if let Some(blocked) = info.procs_blocked {
            self.blocked.add(blocked as f64);
        }
        let Some((prev_at, prev)) = self.prev_system.replace((now, info.clone())) else { return };
        let secs = now.saturating_duration_since(prev_at).as_secs_f64();
        let total = info.cpu_total_jiffies.saturating_sub(prev.cpu_total_jiffies);
        if secs <= 0.0 || total == 0 {
            return;
        }
        let share = |a: Option<u64>, b: Option<u64>| a.zip(b).map(|(a, b)| a.saturating_sub(b) as f64 * 100.0 / total as f64);
        let idle = info.cpu_idle_jiffies.saturating_sub(prev.cpu_idle_jiffies);
        self.cpu_busy_pct.add(total.saturating_sub(idle) as f64 * 100.0 / total as f64);
        if let Some(v) = share(info.cpu_steal_jiffies, prev.cpu_steal_jiffies) {
            self.steal_pct.add(v);
        }
        if let Some(v) = share(info.cpu_iowait_jiffies, prev.cpu_iowait_jiffies) {
            self.iowait_pct.add(v);
        }
        let stall = |a: Option<u64>, b: Option<u64>| a.zip(b).map(|(a, b)| (a.saturating_sub(b) as f64 / (secs * 1e6) * 100.0).min(100.0));
        let (psi, prev_psi) = (info.psi_us.unwrap_or_default(), prev.psi_us.unwrap_or_default());
        for (metric, a, b) in [
            (&mut self.psi_cpu_pct, psi.cpu, prev_psi.cpu),
            (&mut self.psi_memory_pct, psi.memory, prev_psi.memory),
            (&mut self.psi_io_pct, psi.io, prev_psi.io),
        ] {
            if let Some(v) = stall(a, b) {
                metric.add(v);
            }
        }
        let swapped = |i: &SystemInfo| i.swap_in_pages.zip(i.swap_out_pages).map(|(a, b)| a + b);
        if let (Some(a), Some(b)) = (swapped(info), swapped(&prev)) {
            self.swap_per_sec.add(a.saturating_sub(b) as f64 / secs);
        }
        self.disk = info.disk.as_ref().map(|d| d.device.clone());
        if let (Some(d), Some(p)) = (&info.disk, &prev.disk) {
            self.record_disk(d, p, secs);
        }
        if let (Some(out), Some(re), Some(prev_out), Some(prev_re)) = (info.tcp_out_segs, info.tcp_retrans_segs, prev.tcp_out_segs, prev.tcp_retrans_segs) {
            let sent = out.saturating_sub(prev_out);
            if sent > 0 {
                self.retransmit_pct.add(re.saturating_sub(prev_re) as f64 * 100.0 / sent as f64);
            }
        }
    }

    fn record_disk(&mut self, d: &DiskCounters, prev: &DiskCounters, secs: f64) {
        if d.device != prev.device {
            return;
        }
        let ios = d.ios.saturating_sub(prev.ios);
        if ios > 0 {
            self.disk_latency_ms.add(d.io_ms.saturating_sub(prev.io_ms) as f64 / ios as f64);
        }
        self.disk_busy_pct.add((d.busy_ms.saturating_sub(prev.busy_ms) as f64 / (secs * 10.0)).min(100.0));
        self.disk_mb_per_sec.add(d.sectors.saturating_sub(prev.sectors) as f64 * 512.0 / 1e6 / secs);
    }

    pub fn record_processes(&mut self, procs: &[SaltProcess], throttling: Option<CpuThrottling>) {
        let worst = procs.iter()
            .filter_map(|p| p.open_files.zip(p.open_files_limit).filter(|(_, limit)| *limit > 0))
            .max_by(|a, b| (a.0 as f64 / a.1 as f64).total_cmp(&(b.0 as f64 / b.1 as f64)));
        if let Some((n, limit)) = worst {
            self.open_files = Some((n, limit));
            self.open_files_pct.add(n as f64 * 100.0 / limit as f64);
        }
        self.quota = throttling.is_some();
        if let (Some(t), Some(prev)) = (throttling, self.prev_throttling) {
            let periods = t.periods.saturating_sub(prev.periods);
            if periods > 0 {
                self.throttled_pct.add(t.throttled.saturating_sub(prev.throttled) as f64 * 100.0 / periods as f64);
            }
        }
        self.prev_throttling = throttling;
    }

    /// Whether the kernel tracks pressure stall: the PSI rows only show then.
    pub fn has_psi(&self) -> bool {
        self.psi_cpu_pct.avg().is_some() || self.psi_io_pct.avg().is_some()
    }

    /// For a new window: the poll baselines stay, so the next poll counts.
    fn carry_over(&mut self) -> Self {
        Self {
            prev_system: self.prev_system.take(),
            prev_throttling: self.prev_throttling,
            ..Self::default()
        }
    }
}

#[derive(Debug, Default)]
pub struct Load {
    /// MWorkers tied up at the poll: running, waiting on I/O, or waiting for
    /// a process they started (gpg decrypting pillar), which uses no CPU of
    /// their own, so `worker_cores` alone can read low while all are stuck.
    pub workers_busy: Metric,
    /// The MWorkers' CPU, in cores.
    pub worker_cores: Metric,
    pub worker_rss_kb: Metric,
    /// CPU of the processes the MWorkers ran and waited for (gpg rendering
    /// encrypted pillar, `cmd` renderers), in cores.
    pub subprocess_cores: Metric,
    pub gpg: Metric,
    pub zmq_ms: Metric,
    /// The 95th percentile of how long recent jobs took to complete.
    pub job_p95_secs: Metric,
    rss_samples: VecDeque<(Instant, f64)>,
    /// Each MWorker's reaped-children CPU ticks at the previous poll.
    prev_children: HashMap<u32, u64>,
    prev_at: Option<Instant>,
}

impl Load {
    /// A `list_processes` poll. The first one only sets the baseline: CPU
    /// figures are deltas between two polls.
    pub fn record_processes(&mut self, procs: &[SaltProcess], gpg: u64, clk_tck: f64, now: Instant) {
        let workers = mworkers(procs);
        let children: HashMap<u32, u64> = workers.iter().map(|p| (p.pid, p.child_cpu_ticks)).collect();
        let prev_at = self.prev_at.replace(now);
        let prev_children = std::mem::replace(&mut self.prev_children, children);
        let Some(prev_at) = prev_at else { return };
        let secs = now.saturating_duration_since(prev_at).as_secs_f64();
        if workers.is_empty() || secs <= 0.0 {
            return;
        }
        self.workers_busy.add(workers.iter().filter(|p| worker_busy(p)).count() as f64);
        self.worker_cores.add(workers.iter().map(|p| p.cpu_percent as f64 / 100.0).sum());
        let rss = workers.iter().map(|p| p.rss_kb).sum::<u64>() as f64;
        self.worker_rss_kb.add(rss);
        let ticks: u64 = workers.iter()
            .filter_map(|p| prev_children.get(&p.pid).map(|prev| p.child_cpu_ticks.saturating_sub(*prev)))
            .sum();
        self.subprocess_cores.add(ticks as f64 / clk_tck.max(1.0) / secs);
        self.gpg.add(gpg as f64);
        if self.rss_samples.back().is_none_or(|(t, _)| now.saturating_duration_since(*t) >= RSS_SAMPLE_EVERY) {
            self.rss_samples.push_back((now, rss));
            if self.rss_samples.len() > RSS_SAMPLES {
                self.rss_samples.pop_front();
            }
        }
    }

    pub fn record_latency(&mut self, ms: f64) {
        self.zmq_ms.add(ms);
    }

    pub fn record_job_p95(&mut self, secs: f64) {
        self.job_p95_secs.add(secs);
    }

    /// How fast the MWorkers' memory grows, once there are six minutes of
    /// samples.
    pub fn rss_growth_mb_per_hour(&self) -> Option<f64> {
        if self.rss_samples.len() < 6 {
            return None;
        }
        let t0 = self.rss_samples.front()?.0;
        let pts: Vec<(f64, f64)> = self.rss_samples.iter()
            .map(|(t, kb)| (t.saturating_duration_since(t0).as_secs_f64(), kb / 1024.0))
            .collect();
        Some(crate::app::linreg_slope_per_sec(&pts) * 3600.0)
    }
}

fn worker_busy(p: &SaltProcess) -> bool {
    matches!(p.state.as_str(), "R" | "D") || p.children > 0
}

/// The master's MWorkers, the `worker_threads` that answer minions. A runner
/// a worker forks keeps the worker's title but isn't one of them.
fn mworkers(procs: &[SaltProcess]) -> Vec<&SaltProcess> {
    let workers: Vec<&SaltProcess> = procs.iter().filter(|p| p.role == SaltProcessRole::Worker).collect();
    let pids: HashSet<u32> = workers.iter().map(|p| p.pid).collect();
    workers.into_iter().filter(|p| !pids.contains(&p.ppid)).collect()
}

/// How many MWorkers are tied up: running, waiting on I/O, or waiting for a
/// process they started (gpg decrypting pillar), which takes no CPU of
/// their own.
pub fn busy_workers(procs: &[SaltProcess]) -> usize {
    mworkers(procs).into_iter().filter(|p| worker_busy(p)).count()
}

/// The tag's family, with job IDs, minion IDs and numbers replaced by `*`
/// (`salt/job/*/ret/*`), and the minion the event is about: the event's
/// `id`, or an accepted minion named in the tag. The first segment, and a
/// `salt/` tag's second, stay as they are, so a minion called `job` doesn't
/// turn every job tag into `salt/*`. Beacon tags are cut after the beacon's
/// name (`inotify` adds the file's path), others after six segments. The
/// event Salt fires with a new job's bare JID as its tag is `<jid>`.
pub fn family_of(tag: &str, id: Option<&str>, accepted: &HashSet<String>) -> (String, Option<String>) {
    let id = id.filter(|i| !i.is_empty());
    if is_jid(tag) {
        return (JID_FAMILY.to_string(), id.map(String::from));
    }
    let mut minion = id.map(String::from);
    let keep = if tag.starts_with("salt/") { 2 } else { 1 };
    let limit = if tag.starts_with("salt/beacon/") { 4 } else { 6 };
    let mut out: Vec<&str> = Vec::new();
    for (i, seg) in tag.split('/').enumerate() {
        if i == limit {
            out.push("\u{2026}");
            break;
        }
        let is_minion = i >= keep && !seg.is_empty() && (Some(seg) == id || accepted.contains(seg));
        if is_minion && minion.is_none() {
            minion = Some(seg.to_string());
        }
        let wild = i >= keep && !seg.is_empty() && (is_minion || is_jid(seg) || seg.bytes().all(|b| b.is_ascii_digit()));
        out.push(if wild { "*" } else { seg });
    }
    (out.join("/"), minion)
}

#[derive(Debug)]
pub struct Stats {
    started: Instant,
    /// When the window was last reset (`X`), for the header.
    pub reset_at: Option<chrono::DateTime<chrono::Local>>,
    families: HashMap<String, Family>,
    pub load: Load,
    pub flow: Flow,
    pub saturation: Saturation,
}

impl Default for Stats {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl Stats {
    pub fn new(now: Instant) -> Self {
        Self {
            started: now,
            reset_at: None,
            families: CORE_FAMILIES.iter().map(|f| (f.to_string(), Family::default())).collect(),
            load: Load::default(),
            flow: Flow::default(),
            saturation: Saturation::default(),
        }
    }

    /// Starts a new window. The polls' baselines are kept, so the next poll
    /// already counts.
    pub fn reset(&mut self, now: Instant) {
        let load = Load {
            prev_children: std::mem::take(&mut self.load.prev_children),
            prev_at: self.load.prev_at,
            ..Load::default()
        };
        let saturation = self.saturation.carry_over();
        *self = Self { reset_at: Some(chrono::Local::now()), load, saturation, ..Self::new(now) };
    }

    pub fn observed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.started)
    }

    /// Families named before the minions were known: until the agent has
    /// listed the accepted keys, a minion's ID in a tag can't be told from
    /// a fixed part of it (`minion/refresh/web1`). Once it has, they join
    /// the family they belong to, credited to that minion.
    pub fn rekey(&mut self, accepted: &HashSet<String>) {
        let moved: Vec<(String, String, Option<String>)> = self.families.keys()
            .filter(|name| name.as_str() != OTHER_FAMILY && name.as_str() != JID_FAMILY)
            .filter_map(|name| {
                let (family, minion) = family_of(name, None, accepted);
                (family != *name).then(|| (name.clone(), family, minion))
            })
            .collect();
        for (old, new, minion) in moved {
            if let Some(family) = self.families.remove(&old) {
                self.families.entry(new).or_default().merge(family, minion);
            }
        }
    }

    /// How many minions sent any event in the window.
    pub fn minions_heard(&self) -> usize {
        self.families.values().flat_map(|f| f.minions.keys()).collect::<HashSet<_>>().len()
    }

    pub fn total_events(&self) -> u64 {
        self.families.values().map(|f| f.count).sum()
    }

    pub fn minute(&self, now: Instant) -> u64 {
        self.observed(now).as_secs() / 60
    }

    /// Counts an event: in its family (for its minion), and in what it says
    /// about the master's work. `bytes` is its size as the agent sent it.
    pub fn record_event(&mut self, tag: &str, data: &serde_json::Value, bytes: u64, ts: f64, accepted: &HashSet<String>, now: Instant) {
        let minute = self.minute(now);
        let failed = self.flow.record(tag, data, bytes, minute);
        let (family, minion) = family_of(tag, data["id"].as_str(), accepted);
        let family = if self.families.len() >= MAX_FAMILIES && !self.families.contains_key(&family) {
            OTHER_FAMILY.to_string()
        } else {
            family
        };
        let values = event_values(&family, data);
        self.families.entry(family).or_default().record(minion, ts, minute, bytes, failed, values);
    }

    pub fn family(&self, name: &str) -> Option<&Family> {
        self.families.get(name)
    }

    /// The families in their fixed order: the core ones, then the rest by
    /// name (`OTHER_FAMILY` last).
    pub fn rows(&self) -> Vec<(&str, &Family)> {
        let rank = |name: &str| CORE_FAMILIES.iter().position(|c| *c == name).unwrap_or(CORE_FAMILIES.len() + usize::from(name == OTHER_FAMILY));
        let mut rows: Vec<(&str, &Family)> = self.families.iter().map(|(k, v)| (k.as_str(), v)).collect();
        rows.sort_by(|a, b| rank(a.0).cmp(&rank(b.0)).then_with(|| a.0.cmp(b.0)));
        rows
    }

    /// Events per hour since the window started.
    pub fn per_hour(&self, count: u64, now: Instant) -> f64 {
        count as f64 * 3600.0 / self.observed(now).as_secs_f64().max(1.0)
    }

    /// Events and bytes per hour over the last `RECENT_MINUTES` minutes
    /// (the whole window while it's shorter).
    pub fn recent_rates(&self, family: &Family, now: Instant) -> (f64, f64) {
        let secs = self.observed(now).as_secs();
        let minute = secs / 60;
        let span = (if minute + 1 >= RECENT_MINUTES { (RECENT_MINUTES - 1) * 60 + secs % 60 } else { secs } as f64).max(1.0);
        let (n, b) = family.recent(minute);
        (n as f64 * 3600.0 / span, b as f64 * 3600.0 / span)
    }

    /// How many times its average rate the family's busiest minute was;
    /// only after ten minutes, before which the average is too short to
    /// compare with.
    pub fn burst_ratio(&self, family: &Family, now: Instant) -> Option<f64> {
        let avg = self.per_hour(family.count, now);
        (self.observed(now) >= Duration::from_secs(600) && avg > 0.0).then(|| family.per_minute.peak_per_hour() / avg)
    }

    /// The wall-clock time a minute of the window started.
    pub fn minute_time(&self, minute: u64, now: Instant) -> chrono::DateTime<chrono::Local> {
        let ago = self.observed(now).saturating_sub(Duration::from_secs(minute * 60));
        chrono::Local::now() - chrono::Duration::from_std(ago).unwrap_or_default()
    }

    /// The window is shorter than twice the family's interval: a few more
    /// or fewer events still move its rate a lot.
    pub fn low_confidence(&self, family: &Family, now: Instant) -> bool {
        family.median_interval().is_some_and(|i| self.observed(now).as_secs_f64() < 2.0 * i)
    }
}

/// What the Stats tab's number columns measure (`m`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Measure {
    #[default]
    Events,
    Bytes,
    Bursts,
    Failures,
}

impl Measure {
    pub const ALL: [Measure; 4] = [Self::Events, Self::Bytes, Self::Bursts, Self::Failures];

    /// The columns the families list and the breakdown can show for it.
    pub fn columns(self) -> &'static [crate::types::ListCol] {
        use crate::types::ListCol::*;
        match self {
            Self::Events | Self::Bytes => &[Family, Group, Minions, Sent, Count, Rate, RatePerMinion, RecentRate, RecentRatePerMinion, Median, Share],
            Self::Bursts => &[Family, Group, Minions, Sent, Count, Rate, Peak, PeakRatio, PeakAt, Share],
            Self::Failures => &[Family, Group, Minions, Sent, Count, Rate, Failed, FailPct],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Events => "events",
            Self::Bytes => "bytes",
            Self::Bursts => "bursts",
            Self::Failures => "failures",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Events => "how many events, per hour and per minion",
            Self::Bytes => "their size, to see what fills the event bus",
            Self::Bursts => "the busiest minute against the average",
            Self::Failures => "the job returns that failed",
        }
    }
}

/// How the Stats tab breaks the selected family down (`g`): by what the
/// minions are (version, OS, the minion itself), or for some families by
/// what their events say (the job's function, who sent it, what the master
/// did with an auth request).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Grouping {
    #[default]
    VersionOs,
    Os,
    Version,
    Minion,
    Function,
    User,
    Outcome,
}

const BY_MINIONS: &[Grouping] = &[Grouping::VersionOs, Grouping::Os, Grouping::Version, Grouping::Minion];

impl Grouping {
    /// The groupings `g` steps through for a family. A breakdown narrowed
    /// to some minions or to one value only takes those by minions. Job and
    /// runner publishes come from the master, so they only have theirs.
    pub fn for_family(family: &str, narrowed: bool) -> &'static [Grouping] {
        use Grouping::*;
        match family {
            _ if narrowed => BY_MINIONS,
            JOB_NEW_FAMILY | RUN_NEW_FAMILY => &[Function, User],
            RETURN_FAMILY => &[VersionOs, Os, Version, Minion, Function],
            AUTH_FAMILY => &[VersionOs, Os, Version, Minion, Outcome],
            _ => BY_MINIONS,
        }
    }

    /// This grouping if the family has it, else the family's first.
    pub fn within(self, family: &str, narrowed: bool) -> Self {
        let all = Self::for_family(family, narrowed);
        if all.contains(&self) { self } else { all[0] }
    }

    /// The event field a grouping by what the events say reads.
    pub fn field(self) -> Option<&'static str> {
        match self {
            Self::Function => Some("fun"),
            Self::User => Some("user"),
            Self::Outcome => Some("act"),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::VersionOs => "version \u{00D7} OS",
            Self::Os => "OS",
            Self::Version => "version",
            Self::Minion => "minion",
            Self::Function => "function",
            Self::User => "user",
            Self::Outcome => "outcome",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::VersionOs => "Salt version and OS together",
            Self::Os => "the minions' OS",
            Self::Version => "the minions' Salt version",
            Self::Minion => "each minion on its own",
            Self::Function => "the job's function",
            Self::User => "who sent it",
            Self::Outcome => "what the master did: accept, pend, reject, denied",
        }
    }

    pub fn header(self) -> &'static str {
        match self {
            Self::VersionOs => "Version \u{00D7} OS",
            Self::Os => "OS",
            Self::Version => "Version",
            Self::Minion => "Minion",
            Self::Function => "Function",
            Self::User => "User",
            Self::Outcome => "Outcome",
        }
    }
}

/// The values a family's events carry for the groupings by what they say,
/// e.g. `[(Function, "state.apply"), (User, "root")]` for a job publish.
fn event_values(family: &str, data: &serde_json::Value) -> Vec<(Grouping, String)> {
    Grouping::for_family(family, false).iter()
        .filter_map(|g| g.field().map(|field| (*g, data[field].as_str().filter(|v| !v.is_empty()).unwrap_or("?").to_string())))
        .collect()
}

/// A family's events with one value of a field (one function, one user,
/// one auth outcome): all of them, and per minion.
#[derive(Debug, Default)]
pub struct ValueCount {
    pub total: PerMinion,
    pub minions: HashMap<String, PerMinion>,
    pub unattributed: PerMinion,
}

impl ValueCount {
    fn merge(&mut self, other: ValueCount, minion: Option<&str>) {
        self.total.merge(other.total);
        for (id, pm) in other.minions {
            self.minions.entry(id).or_default().merge(pm);
        }
        match minion {
            Some(id) => self.minions.entry(id.to_string()).or_default().merge(other.unattributed),
            None => self.unattributed.merge(other.unattributed),
        }
    }
}

/// A row of the breakdown: a group of minions (or one minion, or one value
/// of a field), how many minions the master has in it, how many of them
/// sent the family, and what they sent. `detail` is the OS next to a
/// version, or a minion's version and OS.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupRow {
    pub label: String,
    pub detail: String,
    pub minions: usize,
    pub sent: usize,
    pub events: u64,
    pub bytes: u64,
    pub failed: u64,
    /// The busiest minute of its busiest minion, in events.
    pub peak_minute: u64,
}

impl GroupRow {
    /// The groups a preview has no room for, as one row.
    pub fn sum(rows: &[GroupRow]) -> Self {
        let mut sum = Self::new(String::new(), String::new(), 0);
        for r in rows {
            sum.minions += r.minions;
            sum.sent += r.sent;
            sum.events += r.events;
            sum.bytes += r.bytes;
            sum.failed += r.failed;
            sum.peak_minute = sum.peak_minute.max(r.peak_minute);
        }
        sum
    }

    fn new(label: String, detail: String, minions: usize) -> Self {
        Self { label, detail, minions, sent: 0, events: 0, bytes: 0, failed: 0, peak_minute: 0 }
    }

    fn add(&mut self, m: &PerMinion) {
        self.events += m.count;
        self.bytes += m.bytes;
        self.failed += m.failed;
        self.peak_minute = self.peak_minute.max(m.per_minute.peak_minute);
    }
}

pub const NO_MINION: &str = "(no minion)";
pub const UNKNOWN_MINION: &str = "(not in cache)";

fn or_unknown(s: &str) -> &str {
    if s.is_empty() { "?" } else { s }
}

/// What one breakdown row stands for: the breakdown narrowed to it after
/// `Enter` on the row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub grouping: Grouping,
    pub label: String,
    pub detail: String,
}

impl Scope {
    pub fn of(grouping: Grouping, row: &GroupRow) -> Self {
        Self { grouping, label: row.label.clone(), detail: row.detail.clone() }
    }

    pub fn title(&self) -> String {
        if self.grouping.field().is_some() {
            format!("{} {}", self.grouping.label(), self.label)
        } else if self.detail.is_empty() {
            self.label.clone()
        } else {
            format!("{} {}", self.label, self.detail)
        }
    }
}

fn group_key(m: &MinionInfo, grouping: Grouping) -> (String, String) {
    match grouping {
        Grouping::VersionOs => (or_unknown(&m.salt_version).to_string(), or_unknown(&m.os_label()).to_string()),
        Grouping::Os => (or_unknown(&m.os_label()).to_string(), String::new()),
        Grouping::Version => (or_unknown(&m.salt_version).to_string(), String::new()),
        Grouping::Minion | Grouping::Function | Grouping::User | Grouping::Outcome => {
            (m.id.clone(), format!("{} {}", or_unknown(&m.salt_version), m.os_label()).trim_end().to_string())
        }
    }
}

/// The selected family broken down by `grouping`, busiest first, over the
/// minions in every one of `scopes`. A first scope on a field's value
/// (narrowed to `outcome reject`, say) counts only the events with it.
/// Groups whose minions sent none of its events are listed too: a version
/// that stays quiet is half of the comparison.
pub fn group_rows(family: &Family, grouping: Grouping, scopes: &[Scope], minions: &[MinionInfo], index: &HashMap<String, usize>) -> Vec<GroupRow> {
    let by_busiest = |rows: &mut Vec<GroupRow>| {
        rows.sort_by(|a, b| b.events.cmp(&a.events).then(b.minions.cmp(&a.minions)).then_with(|| a.label.cmp(&b.label)));
    };
    if grouping.field().is_some() {
        let mut rows: Vec<GroupRow> = family.values.get(&grouping).into_iter().flatten().map(|(value, count)| {
            let mut row = GroupRow::new(value.clone(), String::new(), 0);
            row.add(&count.total);
            row.sent = count.minions.len();
            row
        }).collect();
        by_busiest(&mut rows);
        return rows;
    }
    let (sent_by, unattributed, scopes) = match scopes.split_first() {
        Some((first, rest)) if first.grouping.field().is_some() => {
            match family.values.get(&first.grouping).and_then(|values| values.get(&first.label)) {
                Some(count) => (&count.minions, &count.unattributed, rest),
                None => return Vec::new(),
            }
        }
        _ => (&family.minions, &family.unattributed, scopes),
    };
    let unknown = || (UNKNOWN_MINION.to_string(), String::new());
    let in_scope = |m: Option<&MinionInfo>| scopes.iter().all(|s| {
        let key = m.map_or_else(unknown, |m| group_key(m, s.grouping));
        key.0 == s.label && key.1 == s.detail
    });
    let cached = |id: &str| index.get(id).and_then(|&i| minions.get(i));
    let mut rows: Vec<GroupRow> = Vec::new();
    if grouping == Grouping::Minion {
        rows = minions.iter().filter(|m| in_scope(Some(m))).map(|m| {
            let mut row = GroupRow::new(m.id.clone(), group_key(m, grouping).1, 1);
            if let Some(pm) = sent_by.get(&m.id) {
                row.add(pm);
                row.sent = 1;
            }
            row
        }).collect();
        rows.extend(sent_by.iter().filter(|(id, _)| cached(id).is_none() && in_scope(None)).map(|(id, pm)| {
            let mut row = GroupRow::new(id.clone(), UNKNOWN_MINION.to_string(), 1);
            row.add(pm);
            row.sent = 1;
            row
        }));
    } else {
        let mut at: HashMap<(String, String), usize> = HashMap::new();
        let mut slot = |rows: &mut Vec<GroupRow>, k: (String, String)| -> usize {
            *at.entry(k.clone()).or_insert_with(|| {
                rows.push(GroupRow::new(k.0, k.1, 0));
                rows.len() - 1
            })
        };
        for m in minions.iter().filter(|m| in_scope(Some(m))) {
            let i = slot(&mut rows, group_key(m, grouping));
            rows[i].minions += 1;
        }
        for (id, pm) in sent_by.iter().filter(|(id, _)| in_scope(cached(id))) {
            let i = slot(&mut rows, cached(id).map_or_else(unknown, |m| group_key(m, grouping)));
            rows[i].add(pm);
            rows[i].sent += 1;
        }
    }
    if unattributed.count > 0 && scopes.is_empty() {
        let mut row = GroupRow::new(NO_MINION.to_string(), String::new(), 0);
        row.add(unattributed);
        rows.push(row);
    }
    by_busiest(&mut rows);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accepted(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn tags_collapse_into_families() {
        let acc = accepted(&["web1", "job"]);
        let fam = |tag, id| family_of(tag, id, &acc);
        assert_eq!(fam("salt/job/20261119221500123456/ret/web1", Some("web1")), ("salt/job/*/ret/*".into(), Some("web1".into())));
        assert_eq!(fam("salt/job/20261119221500123456/new", None), ("salt/job/*/new".into(), None));
        assert_eq!(fam("salt/minion/web1/start", Some("web1")), ("salt/minion/*/start".into(), Some("web1".into())));
        assert_eq!(fam("minion_start", Some("web1")), ("minion_start".into(), Some("web1".into())));
        assert_eq!(fam("minion/refresh/web1", None), ("minion/refresh/*".into(), Some("web1".into())));
        assert_eq!(fam("salt/job/20261119221500123456/prog/web1/3", Some("web1")), ("salt/job/*/prog/*/*".into(), Some("web1".into())));
        assert_eq!(fam("salt/beacon/web1/inotify//etc/hosts", Some("web1")), ("salt/beacon/*/inotify/\u{2026}".into(), Some("web1".into())));
        assert_eq!(fam("salt/job/20261119221500123456/ret/other", Some("other")).0, "salt/job/*/ret/*");
        assert_eq!(fam("job/done", None).0, "job/done");
        assert_eq!(fam("20261119221500123456", None), (JID_FAMILY.into(), None));
    }

    #[test]
    fn rates_follow_the_window_and_the_last_minutes() {
        let t0 = Instant::now();
        let mut s = Stats::new(t0);
        let acc = accepted(&[]);
        for i in 0..30u64 {
            let at = t0 + Duration::from_secs(i * 60);
            s.record_event("minion_start", &serde_json::json!({"id": "w"}), 100, (i * 60) as f64, &acc, at);
        }
        let now = t0 + Duration::from_secs(30 * 60);
        let fam = s.family("minion_start").unwrap();
        assert_eq!(fam.count, 30);
        assert!((s.per_hour(fam.count, now) - 60.0).abs() < 0.01);
        let (events, bytes) = s.recent_rates(fam, now);
        assert!((events - 60.0).abs() < 5.0 && (bytes - 6000.0).abs() < 500.0);
        assert_eq!(fam.median_interval(), Some(60.0));
        assert!(fam.periodic());
        assert!(!s.low_confidence(fam, now));
        assert!(s.low_confidence(fam, t0 + Duration::from_secs(90)));
    }

    #[test]
    fn irregular_events_are_not_periodic() {
        let mut f = Family::default();
        let mut ts = 0.0;
        for i in 0..40 {
            ts += [5.0, 50.0, 300.0, 17.0][i % 4];
            f.record(Some("w".into()), ts, 0, 0, false, Vec::new());
        }
        assert!(!f.periodic());
    }

    #[test]
    fn some_families_group_by_what_their_events_say() {
        let t0 = Instant::now();
        let mut s = Stats::new(t0);
        let keys = accepted(&["web1", "web2", "db1"]);
        let auth = |s: &mut Stats, id: &str, act: &str| s.record_event(AUTH_FAMILY, &serde_json::json!({"id": id, "act": act, "result": act == "accept"}), 80, 1.0, &keys, t0);
        auth(&mut s, "web1", "accept");
        auth(&mut s, "web1", "reject");
        auth(&mut s, "web1", "reject");
        auth(&mut s, "db1", "reject");
        auth(&mut s, "web2", "accept");
        let new_job = |s: &mut Stats, fun: &str, user: &str| s.record_event("salt/job/20261120000000000001/new", &serde_json::json!({"fun": fun, "user": user}), 300, 1.0, &keys, t0);
        new_job(&mut s, "state.apply", "root");
        new_job(&mut s, "test.ping", "saltapi");
        new_job(&mut s, "test.ping", "root");

        let m = |id: &str, v: &str| MinionInfo { id: id.into(), salt_version: v.into(), osfinger: "Rocky-9".into(), ..Default::default() };
        let minions = vec![m("web1", "3006.9"), m("web2", "3006.9"), m("db1", "3007.1"), m("quiet", "3007.1")];
        let index: HashMap<String, usize> = minions.iter().enumerate().map(|(i, m)| (m.id.clone(), i)).collect();
        let brief = |rows: Vec<GroupRow>| -> Vec<(String, usize, usize, u64)> { rows.into_iter().map(|r| (r.label, r.minions, r.sent, r.events)).collect() };

        let family = s.family(AUTH_FAMILY).unwrap();
        let outcomes = group_rows(family, Grouping::Outcome, &[], &minions, &index);
        assert_eq!(brief(outcomes.clone()), vec![("reject".into(), 0, 2, 3), ("accept".into(), 0, 2, 2)]);
        let rejected = Scope::of(Grouping::Outcome, &outcomes[0]);
        assert_eq!(rejected.title(), "outcome reject");
        let who: Vec<(String, u64)> = group_rows(family, Grouping::Minion, std::slice::from_ref(&rejected), &minions, &index)
            .into_iter().filter(|r| r.events > 0).map(|r| (r.label, r.events)).collect();
        assert_eq!(who, vec![("web1".to_string(), 2), ("db1".to_string(), 1)]);
        assert_eq!(brief(group_rows(family, Grouping::Version, &[rejected], &minions, &index)), vec![
            ("3006.9".into(), 2, 1, 2), ("3007.1".into(), 2, 1, 1),
        ]);
        assert_eq!(brief(group_rows(family, Grouping::Version, &[], &minions, &index))[0], ("3006.9".into(), 2, 2, 4));

        let jobs = s.family(JOB_NEW_FAMILY).unwrap();
        assert_eq!(brief(group_rows(jobs, Grouping::Function, &[], &minions, &index)), vec![("test.ping".into(), 0, 0, 2), ("state.apply".into(), 0, 0, 1)]);
        assert_eq!(brief(group_rows(jobs, Grouping::User, &[], &minions, &index))[0], ("root".into(), 0, 0, 2));

        assert_eq!(Grouping::VersionOs.within(JOB_NEW_FAMILY, false), Grouping::Function);
        assert_eq!(Grouping::Outcome.within("minion_start", false), Grouping::VersionOs);
        assert_eq!(Grouping::Outcome.within(AUTH_FAMILY, true), Grouping::VersionOs);
        assert_eq!(Grouping::for_family(AUTH_FAMILY, true), BY_MINIONS);
    }

    #[test]
    fn families_named_before_the_keys_were_known_join_their_own() {
        let t0 = Instant::now();
        let mut s = Stats::new(t0);
        let none = accepted(&[]);
        s.record_event("minion/refresh/web1", &serde_json::Value::Null, 50, 1.0, &none, t0);
        s.record_event("minion/refresh/web1", &serde_json::Value::Null, 50, 2.0, &none, t0);
        s.record_event("minion/refresh/db1", &serde_json::Value::Null, 70, 1.0, &none, t0);
        s.record_event("salt/job/20261120000000000000/ret/web1", &serde_json::json!({"id": "web1", "success": true}), 90, 1.0, &none, t0);
        assert!(s.family("minion/refresh/web1").is_some());
        let keys = accepted(&["web1", "db1"]);
        s.record_event("minion/refresh/web1", &serde_json::Value::Null, 50, 3.0, &keys, t0);
        s.rekey(&keys);
        assert!(s.family("minion/refresh/web1").is_none() && s.family("minion/refresh/db1").is_none());
        let pillar = s.family(PILLAR_FAMILY).unwrap();
        assert_eq!((pillar.count, pillar.bytes, pillar.unattributed.count), (4, 220, 0));
        assert_eq!((pillar.minions["web1"].count, pillar.minions["db1"].count), (3, 1));
        assert_eq!(pillar.per_minute.peak_minute, 4);
        assert_eq!(s.family(RETURN_FAMILY).unwrap().count, 1);
    }

    #[test]
    fn core_families_come_first_in_a_fixed_order() {
        let t0 = Instant::now();
        let mut s = Stats::new(t0);
        let acc = accepted(&[]);
        for _ in 0..5 {
            s.record_event("aaa/custom", &serde_json::Value::Null, 10, 1.0, &acc, t0);
        }
        s.record_event("zzz/custom", &serde_json::Value::Null, 10, 1.0, &acc, t0);
        let names: Vec<&str> = s.rows().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, [CORE_FAMILIES, &["aaa/custom", "zzz/custom"]].concat());
    }

    #[test]
    fn groups_count_minions_and_their_events() {
        let m = |id: &str, v: &str, os: &str| MinionInfo { id: id.into(), salt_version: v.into(), osfinger: os.into(), ..Default::default() };
        let minions = vec![m("a", "3006.9", "Windows-2022"), m("b", "3006.9", "Windows-2022"), m("c", "3007.1", "Rocky-9")];
        let index: HashMap<String, usize> = minions.iter().enumerate().map(|(i, m)| (m.id.clone(), i)).collect();
        let mut f = Family::default();
        for (id, n) in [("a", 3), ("b", 2), ("ghost", 1)] {
            for k in 0..n {
                f.record(Some(id.into()), k as f64, k, 100, id == "b", Vec::new());
            }
        }
        f.record(None, 1.0, 0, 10, false, Vec::new());
        let rows = group_rows(&f, Grouping::VersionOs, &[], &minions, &index);
        let row = |label: &str, detail: &str, minions, events| (label.to_string(), detail.to_string(), minions, events);
        let brief = |rows: Vec<GroupRow>| -> Vec<(String, String, usize, u64)> { rows.into_iter().map(|r| (r.label, r.detail, r.minions, r.events)).collect() };
        assert_eq!((rows[0].bytes, rows[0].failed, rows[0].peak_minute), (500, 2, 1));
        assert_eq!(brief(rows.clone()), vec![
            row("3006.9", "Windows-2022", 2, 5),
            row(NO_MINION, "", 0, 1),
            row(UNKNOWN_MINION, "", 0, 1),
            row("3007.1", "Rocky-9", 1, 0),
        ]);
        let by_minion = group_rows(&f, Grouping::Minion, &[], &minions, &index);
        assert_eq!(brief(by_minion.clone())[0], row("a", "3006.9 Windows-2022", 1, 3));
        let windows = Scope::of(Grouping::VersionOs, &rows[0]);
        let scoped: Vec<String> = group_rows(&f, Grouping::Minion, std::slice::from_ref(&windows), &minions, &index).into_iter().map(|r| r.label).collect();
        assert_eq!(scoped.len(), 2);
        assert!(scoped.contains(&"a".to_string()) && scoped.contains(&"b".to_string()));
        assert_eq!(brief(group_rows(&f, Grouping::Os, std::slice::from_ref(&windows), &minions, &index)), vec![row("Windows-2022", "", 2, 5)]);
        let ghosts = Scope::of(Grouping::VersionOs, &rows[2]);
        assert_eq!(brief(group_rows(&f, Grouping::Minion, &[ghosts], &minions, &index)), vec![row("ghost", UNKNOWN_MINION, 1, 1)]);
        assert_eq!(by_minion.len(), 5);
        let quiet = Scope::of(Grouping::VersionOs, &rows[3]);
        assert_eq!(brief(group_rows(&f, Grouping::Minion, &[quiet], &minions, &index)), vec![row("c", "3007.1 Rocky-9", 1, 0)]);
    }

    #[test]
    fn load_needs_two_polls_and_counts_reaped_children() {
        let t0 = Instant::now();
        let worker = |pid, ticks, cpu, state: &str, children| SaltProcess {
            pid, role: SaltProcessRole::Worker, rss_kb: 1024, cpu_ticks: 0, child_cpu_ticks: ticks,
            state: state.into(), children, ppid: 1, cpu_percent: cpu, open_files: None, open_files_limit: None,
        };
        let mut load = Load::default();
        load.record_processes(&[worker(7, 100, 0.0, "S", 0)], 2, 100.0, t0);
        assert!(load.worker_cores.avg().is_none());
        let fork = SaltProcess { ppid: 9, ..worker(10, 0, 50.0, "R", 0) };
        let polled = [worker(7, 300, 150.0, "R", 0), worker(8, 0, 0.0, "S", 1), worker(9, 0, 0.0, "S", 0), fork];
        load.record_processes(&polled, 4, 100.0, t0 + Duration::from_secs(2));
        assert_eq!(load.workers_busy.now, 2.0);
        assert_eq!(busy_workers(&polled), 2);
        assert_eq!(load.worker_cores.now, 1.5);
        assert_eq!(load.subprocess_cores.now, 1.0);
        assert_eq!(load.gpg.avg(), Some(4.0));
    }

    #[test]
    fn throughput_now_is_the_last_whole_minute_and_peak_the_busiest() {
        let mut t = Throughput::default();
        t.add(5, 0);
        assert_eq!(t.now_per_hour(0), None);
        t.add(20, 1);
        t.add(2, 2);
        assert_eq!(t.now_per_hour(2), Some(20.0 * 60.0));
        assert_eq!(t.now_per_hour(3), Some(2.0 * 60.0));
        assert_eq!(t.now_per_hour(5), Some(0.0));
        assert_eq!(t.peak_per_hour(), 20.0 * 60.0);
        assert_eq!(t.total, 27);
        t.add(1, 9);
        assert_eq!(t.now_per_hour(9), Some(0.0));
    }

    #[test]
    fn flow_sorts_events_into_the_masters_work() {
        let mut f = Flow::default();
        let ok = serde_json::json!({"success": true, "retcode": 0, "return": true});
        let failed = serde_json::json!({"success": false, "retcode": 1, "return": "boom"});
        f.record("salt/job/20261120000000000000/ret/web1", &ok, 100, 0);
        f.record("salt/job/20261120000000000000/ret/web2", &failed, 120, 0);
        f.record("salt/job/20261120000000000000/new", &serde_json::json!({}), 80, 0);
        f.record("minion/refresh/web1", &serde_json::json!({}), 50, 0);
        f.record("salt/auth", &serde_json::json!({"act": "accept", "result": true}), 60, 0);
        f.record("salt/auth", &serde_json::json!({"act": "reject", "result": false}), 60, 0);
        f.record("salt/stats/MWorker-0", &serde_json::json!({
            "time": 60.0, "worker": "MWorker-0",
            "stats": {"_pillar": {"runs": 3, "mean": 10.0}, "_return": {"runs": 10, "mean": 0.6}, "_auth": {"runs": 0, "mean": 0}},
        }), 200, 0);
        assert_eq!((f.events.total, f.bytes.total), (7, 670));
        assert_eq!((f.returns.total, f.failed_share()), (2, Some(0.5)));
        assert_eq!((f.jobs.total, f.pillar_requests.total, f.auth.total), (1, 1, 2));
        assert_eq!((f.auth_by_act.get("accept"), f.auth_by_act.get("reject")), (Some(&1), Some(&1)));
        let shares = f.worker_time.shares();
        assert_eq!(shares[0].0, "_pillar");
        assert!((shares[0].1 - 0.5).abs() < 1e-9 && (shares[1].1 - 0.1).abs() < 1e-9);
        assert_eq!(f.worker_time.mean("_pillar"), Some(10.0));
        assert_eq!(f.worker_time.mean("_auth"), None);
        assert!(f.worker_time.minute_share("_pillar").is_none());
        let stats = |pillar: u64| serde_json::json!({"time": 60.0, "stats": {"_pillar": {"runs": pillar, "mean": 6.0}, "_return": {"runs": 10, "mean": 0.6}}});
        f.record("salt/stats/MWorker-0", &stats(0), 200, 1);
        f.record("salt/stats/MWorker-0", &stats(1), 200, 2);
        let pillar = f.worker_time.minute_share("_pillar").unwrap();
        assert!((pillar.peak - 0.5).abs() < 1e-9 && pillar.now.abs() < 1e-9);
        assert!((f.worker_time.minute_share("_return").unwrap().now - 0.1).abs() < 1e-9);
    }

    #[test]
    fn disk_latency_busy_and_retransmits_come_from_deltas() {
        let t0 = Instant::now();
        let info = |ios, io_ms, busy_ms, sectors, out, retrans, device: &str| SystemInfo {
            cpu_total_jiffies: 1000 + ios,
            disk: Some(DiskCounters { device: device.into(), ios, io_ms, busy_ms, sectors }),
            tcp_out_segs: Some(out), tcp_retrans_segs: Some(retrans),
            ..SystemInfo::default()
        };
        let mut s = Saturation::default();
        s.record_system(&info(100, 500, 1000, 0, 10_000, 10, "sda"), 1, t0);
        s.record_system(&info(300, 4500, 2500, 2_000_000, 12_000, 70, "sda"), 1, t0 + Duration::from_secs(2));
        assert_eq!(s.disk.as_deref(), Some("sda"));
        assert_eq!((s.disk_latency_ms.now, s.disk_busy_pct.now, s.disk_mb_per_sec.now), (20.0, 75.0, 512.0));
        assert_eq!(s.retransmit_pct.now, 3.0);
        s.record_system(&info(400, 4600, 2600, 2_000_000, 12_000, 70, "sdb"), 1, t0 + Duration::from_secs(4));
        assert_eq!((s.disk.as_deref(), s.disk_latency_ms.now), (Some("sdb"), 20.0));
    }

    #[test]
    fn saturation_reads_deltas_between_polls() {
        let t0 = Instant::now();
        let info = |total: u64, idle: u64, steal: u64, iowait: u64, psi_io: u64, swap: u64, drops: u64| SystemInfo {
            cpu_total_jiffies: total, cpu_idle_jiffies: idle, cpu_steal_jiffies: Some(steal), cpu_iowait_jiffies: Some(iowait),
            mem_total_kb: 1000, mem_avail_kb: 250, procs_running: Some(6), procs_blocked: Some(1),
            psi_us: Some(crate::types::PressureTotals { cpu: Some(0), memory: None, io: Some(psi_io) }),
            swap_in_pages: Some(swap), swap_out_pages: Some(0), oom_kills: Some(3), listen_overflows: Some(drops), listen_drops: Some(drops),
            ..SystemInfo::default()
        };
        let mut s = Saturation::default();
        s.record_system(&info(1000, 500, 0, 0, 0, 0, 10), 4, t0);
        assert_eq!((s.run_queue.now, s.mem_avail_pct.now), (1.5, 25.0));
        assert!(s.cpu_busy_pct.avg().is_none());
        s.record_system(&info(1200, 550, 20, 40, 500_000, 100, 13), 4, t0 + Duration::from_secs(2));
        assert_eq!((s.cpu_busy_pct.now, s.steal_pct.now, s.iowait_pct.now), (75.0, 10.0, 20.0));
        assert_eq!((s.psi_io_pct.now, s.swap_per_sec.now), (25.0, 50.0));
        assert!(s.psi_memory_pct.avg().is_none() && s.has_psi());
        assert_eq!((s.oom_kills, s.listen_drops), (Some(0), Some(3)));

        let proc = |files, limit| SaltProcess {
            pid: 1, role: SaltProcessRole::Master, rss_kb: 0, cpu_ticks: 0, child_cpu_ticks: 0, state: "S".into(),
            children: 0, ppid: 0, cpu_percent: 0.0, open_files: Some(files), open_files_limit: Some(limit),
        };
        let quota = |periods, throttled| Some(CpuThrottling { periods, throttled, throttled_usec: 0 });
        s.record_processes(&[proc(100, 1024), proc(900, 1024), proc(50, 100)], quota(100, 10));
        assert_eq!((s.open_files, s.open_files_pct.now), (Some((900, 1024)), 900.0 * 100.0 / 1024.0));
        assert!(s.quota && s.throttled_pct.avg().is_none());
        s.record_processes(&[], quota(200, 60));
        assert_eq!(s.throttled_pct.now, 50.0);
        s.record_processes(&[], None);
        assert!(!s.quota);
    }
}
