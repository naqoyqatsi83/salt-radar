//! The Stats tab's numbers: event rates per tag family since the window
//! started, and the master's load over the same window. Everything is a rate
//! or an average, so two masters can be compared however long salt-radar has
//! been running on each.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use crate::types::{is_jid, CpuThrottling, MinionInfo, SaltProcess, SaltProcessRole, SystemInfo};

/// Families every master shows, first and in this order, even with no
/// events yet, so two salt-radar screens side by side line up row by row.
pub const CORE_FAMILIES: &[&str] = &["salt/auth", "minion_start", "salt/minion/*/start", "salt/job/*/new", "salt/job/*/ret/*"];

/// Where events go once this many families exist: custom tags can carry
/// anything (paths, timestamps), and each family keeps per-minion counts.
const MAX_FAMILIES: usize = 300;
pub const OTHER_FAMILY: &str = "(other tags)";
pub const JID_FAMILY: &str = "<jid>";
const MAX_MINIONS_PER_FAMILY: usize = 50_000;
const MAX_INTERVALS: usize = 1000;
/// The "last 15 minutes" columns.
pub const RECENT_MINUTES: u64 = 15;
const RSS_SAMPLE_EVERY: Duration = Duration::from_secs(60);
const RSS_SAMPLES: usize = 24 * 60;

#[derive(Debug, Default)]
pub struct PerMinion {
    pub count: u64,
    last_ts: f64,
}

#[derive(Debug, Default)]
pub struct Family {
    pub count: u64,
    /// (minute of the window, events in it) for the last `RECENT_MINUTES`.
    minutes: VecDeque<(u64, u64)>,
    pub minions: HashMap<String, PerMinion>,
    /// Events no minion could be named for (`salt/job/*/new`, runners).
    pub unattributed: u64,
    last_unattributed_ts: Option<f64>,
    /// Seconds between two events of the family from the same minion (or
    /// between two unattributed ones), the latest `MAX_INTERVALS`.
    intervals: VecDeque<f64>,
}

impl Family {
    fn record(&mut self, minion: Option<String>, ts: f64, minute: u64) {
        self.count += 1;
        match self.minutes.back_mut() {
            Some((m, n)) if *m == minute => *n += 1,
            _ => self.minutes.push_back((minute, 1)),
        }
        while self.minutes.front().is_some_and(|(m, _)| m + RECENT_MINUTES <= minute) {
            self.minutes.pop_front();
        }
        let previous = match minion {
            Some(id) => {
                let full = self.minions.len() >= MAX_MINIONS_PER_FAMILY;
                match self.minions.get_mut(&id) {
                    Some(m) => {
                        m.count += 1;
                        Some(std::mem::replace(&mut m.last_ts, ts))
                    }
                    None if full => {
                        self.unattributed += 1;
                        None
                    }
                    None => {
                        self.minions.insert(id, PerMinion { count: 1, last_ts: ts });
                        None
                    }
                }
            }
            None => {
                self.unattributed += 1;
                self.last_unattributed_ts.replace(ts)
            }
        };
        if let Some(prev) = previous.filter(|&prev| ts > prev) {
            self.intervals.push_back(ts - prev);
            if self.intervals.len() > MAX_INTERVALS {
                self.intervals.pop_front();
            }
        }
    }

    /// Events in the last `RECENT_MINUTES` minutes, the current one included.
    fn recent(&self, minute: u64) -> u64 {
        self.minutes.iter().filter(|(m, _)| m + RECENT_MINUTES > minute).map(|(_, n)| n).sum()
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
    peak_minute: u64,
}

impl Throughput {
    fn add(&mut self, n: u64, minute: u64) {
        self.roll(minute);
        self.total += n;
        self.in_minute += n;
        self.peak_minute = self.peak_minute.max(self.in_minute);
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
}

impl WorkerTime {
    fn record(&mut self, data: &serde_json::Value) {
        let Some(stats) = data["stats"].as_object() else { return };
        self.total_secs += data["time"].as_f64().unwrap_or(0.0).max(0.0);
        for (request, v) in stats {
            let runs = v["runs"].as_u64().unwrap_or(0);
            if runs == 0 {
                continue;
            }
            let e = self.by_request.entry(request.clone()).or_default();
            e.0 += runs;
            e.1 += runs as f64 * v["mean"].as_f64().unwrap_or(0.0).max(0.0);
        }
    }

    /// Request types by their share of the workers' time, largest first.
    pub fn shares(&self) -> Vec<(&str, f64)> {
        if self.total_secs <= 0.0 {
            return Vec::new();
        }
        let mut v: Vec<(&str, f64)> = self.by_request.iter().map(|(k, (_, secs))| (k.as_str(), secs / self.total_secs)).collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        v
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
    pub renders: Throughput,
    /// `salt/auth` by what the master did: accept, pend, reject, denied.
    pub auth: Throughput,
    pub auth_by_act: HashMap<String, u64>,
    pub worker_time: WorkerTime,
}

impl Flow {
    pub const AUTH_ACTS: [&'static str; 4] = ["accept", "pend", "reject", "denied"];

    fn record(&mut self, tag: &str, data: &serde_json::Value, bytes: u64, minute: u64) {
        self.events.add(1, minute);
        self.bytes.add(bytes, minute);
        if tag.starts_with("salt/job/") && tag.contains("/ret/") {
            self.returns.add(1, minute);
            if !crate::app::return_outcome(data, &data["return"]).0 {
                self.failed_returns += 1;
            }
        } else if tag.starts_with("salt/job/") && tag.ends_with("/new") {
            self.jobs.add(1, minute);
        } else if tag.starts_with("minion/refresh/") {
            self.renders.add(1, minute);
        } else if tag == "salt/auth" {
            self.auth.add(1, minute);
            *self.auth_by_act.entry(data["act"].as_str().unwrap_or("?").to_string()).or_default() += 1;
        } else if tag.starts_with("salt/stats/") {
            self.worker_time.record(data);
        }
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

    /// How many minions sent any event in the window.
    pub fn minions_heard(&self) -> usize {
        self.families.values().flat_map(|f| f.minions.keys()).collect::<HashSet<_>>().len()
    }

    pub fn total_events(&self) -> u64 {
        self.families.values().map(|f| f.count).sum()
    }

    /// What an event says about the master's work: its size, and for job
    /// returns, publishes, pillar renders, auth and worker stats, more.
    pub fn record_flow(&mut self, tag: &str, data: &serde_json::Value, bytes: u64, now: Instant) {
        let minute = self.minute(now);
        self.flow.record(tag, data, bytes, minute);
    }

    pub fn minute(&self, now: Instant) -> u64 {
        self.observed(now).as_secs() / 60
    }

    pub fn record_event(&mut self, tag: &str, id: Option<&str>, ts: f64, accepted: &HashSet<String>, now: Instant) {
        let (family, minion) = family_of(tag, id, accepted);
        let family = if self.families.len() >= MAX_FAMILIES && !self.families.contains_key(&family) {
            OTHER_FAMILY.to_string()
        } else {
            family
        };
        let minute = self.minute(now);
        self.families.entry(family).or_default().record(minion, ts, minute);
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

    /// Events per hour over the last `RECENT_MINUTES` minutes (the whole
    /// window while it's shorter).
    pub fn recent_per_hour(&self, family: &Family, now: Instant) -> f64 {
        let secs = self.observed(now).as_secs();
        let minute = secs / 60;
        let span = if minute + 1 >= RECENT_MINUTES { (RECENT_MINUTES - 1) * 60 + secs % 60 } else { secs };
        family.recent(minute) as f64 * 3600.0 / (span as f64).max(1.0)
    }

    /// The window is shorter than twice the family's interval: a few more
    /// or fewer events still move its rate a lot.
    pub fn low_confidence(&self, family: &Family, now: Instant) -> bool {
        family.median_interval().is_some_and(|i| self.observed(now).as_secs_f64() < 2.0 * i)
    }
}

/// How the Stats tab breaks the selected family down (`g`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Grouping {
    #[default]
    VersionOs,
    Os,
    Version,
    Minion,
}

impl Grouping {
    pub fn next(self) -> Self {
        match self {
            Self::VersionOs => Self::Os,
            Self::Os => Self::Version,
            Self::Version => Self::Minion,
            Self::Minion => Self::VersionOs,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::VersionOs => "version \u{00D7} OS",
            Self::Os => "OS",
            Self::Version => "version",
            Self::Minion => "minion",
        }
    }

    pub fn header(self) -> &'static str {
        match self {
            Self::VersionOs => "Version \u{00D7} OS",
            Self::Os => "OS",
            Self::Version => "Version",
            Self::Minion => "Minion",
        }
    }
}

/// A row of the breakdown: a group of minions (or one minion), how many
/// minions the master has in it and how many of the family's events they
/// sent. `detail` is the OS next to a version, or a minion's version and OS.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupRow {
    pub label: String,
    pub detail: String,
    pub minions: usize,
    pub events: u64,
}

pub const NO_MINION: &str = "(no minion)";
pub const UNKNOWN_MINION: &str = "(not in cache)";

fn or_unknown(s: &str) -> &str {
    if s.is_empty() { "?" } else { s }
}

/// The minions one breakdown group stands for: the breakdown narrowed to
/// them after `Enter` on their row.
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
        if self.detail.is_empty() { self.label.clone() } else { format!("{} {}", self.label, self.detail) }
    }
}

fn group_key(m: &MinionInfo, grouping: Grouping) -> (String, String) {
    match grouping {
        Grouping::VersionOs => (or_unknown(&m.salt_version).to_string(), or_unknown(&m.os_label()).to_string()),
        Grouping::Os => (or_unknown(&m.os_label()).to_string(), String::new()),
        Grouping::Version => (or_unknown(&m.salt_version).to_string(), String::new()),
        Grouping::Minion => (m.id.clone(), format!("{} {}", or_unknown(&m.salt_version), m.os_label()).trim_end().to_string()),
    }
}

/// The selected family broken down by `grouping`, busiest first, over the
/// minions in every one of `scopes`. Groups whose minions sent
/// none of its events are listed too: a version that stays quiet is half of
/// the comparison.
pub fn group_rows(family: &Family, grouping: Grouping, scopes: &[Scope], minions: &[MinionInfo], index: &HashMap<String, usize>) -> Vec<GroupRow> {
    let unknown = || (UNKNOWN_MINION.to_string(), String::new());
    let in_scope = |m: Option<&MinionInfo>| scopes.iter().all(|s| {
        let key = m.map_or_else(unknown, |m| group_key(m, s.grouping));
        key.0 == s.label && key.1 == s.detail
    });
    let cached = |id: &str| index.get(id).and_then(|&i| minions.get(i));
    let mut rows: Vec<GroupRow> = Vec::new();
    if grouping == Grouping::Minion {
        rows = minions.iter().filter(|m| in_scope(Some(m))).map(|m| GroupRow {
            label: m.id.clone(),
            detail: group_key(m, grouping).1,
            minions: 1,
            events: family.minions.get(&m.id).map_or(0, |pm| pm.count),
        }).collect();
        rows.extend(family.minions.iter().filter(|(id, _)| cached(id).is_none() && in_scope(None)).map(|(id, pm)| {
            GroupRow { label: id.clone(), detail: UNKNOWN_MINION.to_string(), minions: 1, events: pm.count }
        }));
    } else {
        let mut at: HashMap<(String, String), usize> = HashMap::new();
        let mut slot = |rows: &mut Vec<GroupRow>, k: (String, String)| -> usize {
            *at.entry(k.clone()).or_insert_with(|| {
                rows.push(GroupRow { label: k.0, detail: k.1, minions: 0, events: 0 });
                rows.len() - 1
            })
        };
        for m in minions.iter().filter(|m| in_scope(Some(m))) {
            let i = slot(&mut rows, group_key(m, grouping));
            rows[i].minions += 1;
        }
        for (id, pm) in family.minions.iter().filter(|(id, _)| in_scope(cached(id))) {
            let i = slot(&mut rows, cached(id).map_or_else(unknown, |m| group_key(m, grouping)));
            rows[i].events += pm.count;
        }
    }
    if family.unattributed > 0 && scopes.is_empty() {
        rows.push(GroupRow { label: NO_MINION.to_string(), detail: String::new(), minions: 0, events: family.unattributed });
    }
    rows.sort_by(|a, b| b.events.cmp(&a.events).then(b.minions.cmp(&a.minions)).then_with(|| a.label.cmp(&b.label)));
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
            s.record_event("minion_start", Some("w"), (i * 60) as f64, &acc, at);
        }
        let now = t0 + Duration::from_secs(30 * 60);
        let fam = s.family("minion_start").unwrap();
        assert_eq!(fam.count, 30);
        assert!((s.per_hour(fam.count, now) - 60.0).abs() < 0.01);
        assert!((s.recent_per_hour(fam, now) - 60.0).abs() < 5.0);
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
            f.record(Some("w".into()), ts, 0);
        }
        assert!(!f.periodic());
    }

    #[test]
    fn core_families_come_first_in_a_fixed_order() {
        let t0 = Instant::now();
        let mut s = Stats::new(t0);
        let acc = accepted(&[]);
        for _ in 0..5 {
            s.record_event("aaa/custom", None, 1.0, &acc, t0);
        }
        s.record_event("zzz/custom", None, 1.0, &acc, t0);
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
                f.record(Some(id.into()), k as f64, 0);
            }
        }
        f.record(None, 1.0, 0);
        let rows = group_rows(&f, Grouping::VersionOs, &[], &minions, &index);
        let row = |label: &str, detail: &str, minions, events| GroupRow { label: label.into(), detail: detail.into(), minions, events };
        assert_eq!(rows, vec![
            row("3006.9", "Windows-2022", 2, 5),
            row(NO_MINION, "", 0, 1),
            row(UNKNOWN_MINION, "", 0, 1),
            row("3007.1", "Rocky-9", 1, 0),
        ]);
        let by_minion = group_rows(&f, Grouping::Minion, &[], &minions, &index);
        assert_eq!(by_minion[0], row("a", "3006.9 Windows-2022", 1, 3));
        let windows = Scope::of(Grouping::VersionOs, &rows[0]);
        let scoped: Vec<String> = group_rows(&f, Grouping::Minion, std::slice::from_ref(&windows), &minions, &index).into_iter().map(|r| r.label).collect();
        assert_eq!(scoped.len(), 2);
        assert!(scoped.contains(&"a".to_string()) && scoped.contains(&"b".to_string()));
        assert_eq!(group_rows(&f, Grouping::Os, std::slice::from_ref(&windows), &minions, &index), vec![row("Windows-2022", "", 2, 5)]);
        let ghosts = Scope::of(Grouping::VersionOs, &rows[2]);
        assert_eq!(group_rows(&f, Grouping::Minion, &[ghosts], &minions, &index), vec![row("ghost", UNKNOWN_MINION, 1, 1)]);
        assert_eq!(by_minion.len(), 5);
        let quiet = Scope::of(Grouping::VersionOs, &rows[3]);
        assert_eq!(group_rows(&f, Grouping::Minion, &[quiet], &minions, &index), vec![row("c", "3007.1 Rocky-9", 1, 0)]);
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
        assert_eq!((f.jobs.total, f.renders.total, f.auth.total), (1, 1, 2));
        assert_eq!((f.auth_by_act.get("accept"), f.auth_by_act.get("reject")), (Some(&1), Some(&1)));
        let shares = f.worker_time.shares();
        assert_eq!(shares[0].0, "_pillar");
        assert!((shares[0].1 - 0.5).abs() < 1e-9 && (shares[1].1 - 0.1).abs() < 1e-9);
        assert_eq!(f.worker_time.mean("_pillar"), Some(10.0));
        assert_eq!(f.worker_time.mean("_auth"), None);
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
