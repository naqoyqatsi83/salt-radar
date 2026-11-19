//! The Stats tab's numbers: event rates per tag family since the window
//! started, and the master's load over the same window. Everything is a rate
//! or an average, so two masters can be compared however long salt-radar has
//! been running on each.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use crate::types::{is_jid, MinionInfo, SaltProcess, SaltProcessRole};

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

/// One sampled figure of the master's load: its latest value, average and
/// peak over the window.
#[derive(Clone, Copy, Debug, Default)]
pub struct Metric {
    pub now: f64,
    pub peak: f64,
    sum: f64,
    n: u64,
}

impl Metric {
    fn add(&mut self, v: f64) {
        self.now = v;
        self.peak = if self.n == 0 { v } else { self.peak.max(v) };
        self.sum += v;
        self.n += 1;
    }

    pub fn avg(&self) -> Option<f64> {
        (self.n > 0).then(|| self.sum / self.n as f64)
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
        }
    }

    /// Starts a new window. The load's poll baseline is kept, so the next
    /// poll already counts.
    pub fn reset(&mut self, now: Instant) {
        let load = Load {
            prev_children: std::mem::take(&mut self.load.prev_children),
            prev_at: self.load.prev_at,
            ..Load::default()
        };
        *self = Self { reset_at: Some(chrono::Local::now()), load, ..Self::new(now) };
    }

    pub fn observed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.started)
    }

    pub fn total_events(&self) -> u64 {
        self.families.values().map(|f| f.count).sum()
    }

    fn minute(&self, now: Instant) -> u64 {
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

    /// The families in display order: the core ones, then the rest by name
    /// (`OTHER_FAMILY` last), or with `by_count`, the busiest first.
    pub fn rows(&self, by_count: bool) -> Vec<(&str, &Family)> {
        let rank = |name: &str| CORE_FAMILIES.iter().position(|c| *c == name).unwrap_or(CORE_FAMILIES.len() + usize::from(name == OTHER_FAMILY));
        let mut rows: Vec<(&str, &Family)> = self.families.iter().map(|(k, v)| (k.as_str(), v)).collect();
        if by_count {
            rows.sort_by(|a, b| b.1.count.cmp(&a.1.count).then_with(|| rank(a.0).cmp(&rank(b.0))).then_with(|| a.0.cmp(b.0)));
        } else {
            rows.sort_by(|a, b| rank(a.0).cmp(&rank(b.0)).then_with(|| a.0.cmp(b.0)));
        }
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

/// The selected family broken down by `grouping`, busiest first. Groups
/// whose minions sent none of its events are listed too: a version that
/// stays quiet is half of the comparison.
pub fn group_rows(family: &Family, grouping: Grouping, minions: &[MinionInfo], index: &HashMap<String, usize>) -> Vec<GroupRow> {
    let key = |m: &MinionInfo| -> (String, String) {
        match grouping {
            Grouping::VersionOs => (or_unknown(&m.salt_version).to_string(), or_unknown(&m.os_label()).to_string()),
            Grouping::Os => (or_unknown(&m.os_label()).to_string(), String::new()),
            Grouping::Version => (or_unknown(&m.salt_version).to_string(), String::new()),
            Grouping::Minion => (m.id.clone(), format!("{} {}", or_unknown(&m.salt_version), m.os_label()).trim_end().to_string()),
        }
    };
    let mut rows: Vec<GroupRow> = Vec::new();
    if grouping == Grouping::Minion {
        rows = family.minions.iter().map(|(id, pm)| {
            let detail = index.get(id).and_then(|&i| minions.get(i)).map(|m| key(m).1).unwrap_or_else(|| UNKNOWN_MINION.to_string());
            GroupRow { label: id.clone(), detail, minions: 1, events: pm.count }
        }).collect();
    } else {
        let mut at: HashMap<(String, String), usize> = HashMap::new();
        let mut slot = |rows: &mut Vec<GroupRow>, k: (String, String)| -> usize {
            *at.entry(k.clone()).or_insert_with(|| {
                rows.push(GroupRow { label: k.0, detail: k.1, minions: 0, events: 0 });
                rows.len() - 1
            })
        };
        for m in minions {
            let i = slot(&mut rows, key(m));
            rows[i].minions += 1;
        }
        for (id, pm) in &family.minions {
            let k = index.get(id).and_then(|&i| minions.get(i)).map(key).unwrap_or_else(|| (UNKNOWN_MINION.to_string(), String::new()));
            let i = slot(&mut rows, k);
            rows[i].events += pm.count;
        }
    }
    if family.unattributed > 0 {
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
        let names: Vec<&str> = s.rows(false).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, [CORE_FAMILIES, &["aaa/custom", "zzz/custom"]].concat());
        assert_eq!(s.rows(true)[0].0, "aaa/custom");
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
        let rows = group_rows(&f, Grouping::VersionOs, &minions, &index);
        let row = |label: &str, detail: &str, minions, events| GroupRow { label: label.into(), detail: detail.into(), minions, events };
        assert_eq!(rows, vec![
            row("3006.9", "Windows-2022", 2, 5),
            row(NO_MINION, "", 0, 1),
            row(UNKNOWN_MINION, "", 0, 1),
            row("3007.1", "Rocky-9", 1, 0),
        ]);
        let by_minion = group_rows(&f, Grouping::Minion, &minions, &index);
        assert_eq!(by_minion[0], row("a", "3006.9 Windows-2022", 1, 3));
    }

    #[test]
    fn load_needs_two_polls_and_counts_reaped_children() {
        let t0 = Instant::now();
        let worker = |pid, ticks, cpu, state: &str, children| SaltProcess {
            pid, role: SaltProcessRole::Worker, rss_kb: 1024, cpu_ticks: 0, child_cpu_ticks: ticks,
            state: state.into(), children, ppid: 1, cpu_percent: cpu,
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
}
