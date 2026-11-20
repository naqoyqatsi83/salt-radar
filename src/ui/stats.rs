use std::time::{Duration, Instant};

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::stats::{Grouping, Metric, RECENT_MINUTES};
use crate::types::{HeaderHit, ListCol, ListId};

use super::color;
use super::table::{header_style, pad, HeaderRow};

/// Below this many columns inside the families panel, the last-15-minutes
/// columns are left out.
const RECENT_MIN_WIDTH: usize = 86;

pub fn fmt_count(n: u64) -> String {
    match n {
        0..=9_999 => n.to_string(),
        10_000..=999_999 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// A rate: two decimals below 10, so a per-minion rate like 0.02 still shows.
pub fn fmt_rate(v: f64) -> String {
    if v <= 0.0 {
        "0".to_string()
    } else if v < 0.01 {
        "<0.01".to_string()
    } else if v < 10.0 {
        format!("{:.2}", v)
    } else if v < 100.0 {
        format!("{:.1}", v)
    } else if v < 10_000.0 {
        format!("{:.0}", v)
    } else if v < 1e6 {
        format!("{:.1}k", v / 1e3)
    } else {
        format!("{:.1}M", v / 1e6)
    }
}

pub fn fmt_interval(secs: f64) -> String {
    if secs < 10.0 {
        format!("{:.1}s", secs)
    } else if secs < 60.0 {
        format!("{:.0}s", secs)
    } else if secs < 3600.0 {
        let s = secs.round() as u64;
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        let m = (secs / 60.0).round() as u64;
        format!("{}h{:02}m", m / 60, m % 60)
    }
}

pub fn fmt_window(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=59 => format!("{}s", s),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h{:02}m", s / 3600, s % 3600 / 60),
        _ => format!("{}d{:02}h", s / 86_400, s % 86_400 / 3600),
    }
}

fn fmt_kb(kb: f64) -> String {
    let mb = kb / 1024.0;
    if mb < 1024.0 { format!("{:.0}M", mb) } else { format!("{:.1}G", mb / 1024.0) }
}

fn fmt_ms(ms: f64) -> String {
    if ms < 1000.0 { format!("{:.0}ms", ms) } else { format!("{:.1}s", ms / 1000.0) }
}

fn dim() -> Style {
    Style::default().fg(color::inactive_fg())
}

fn text() -> Style {
    Style::default().fg(color::main_fg())
}

fn border(focused: bool) -> Style {
    Style::default().fg(if focused { color::border_focus() } else { color::border() })
}

/// Where the Stats tab drew its rows, for clicks: the families, or the
/// breakdown's groups while it's open full-screen.
pub struct StatsHits {
    pub families: Vec<(u16, String)>,
    pub groups: Vec<(u16, usize)>,
}

/// The Stats tab: event families, the selected one's breakdown and the
/// master's health (beside them on a wide terminal, below otherwise); or
/// the breakdown alone once `Enter` opened it, still beside the health.
pub fn draw_stats(f: &mut Frame, area: Rect, app: &App, focused: bool, header_hits: &mut Vec<HeaderHit>) -> StatsHits {
    let groups = app.stats_groups();
    let beside = area.width >= HEALTH_BESIDE_MIN_WIDTH;
    let area = if beside {
        let [main, health] = Layout::horizontal([Constraint::Min(0), Constraint::Length(HEALTH_WIDTH)]).areas(area);
        draw_health(f, health, app);
        main
    } else {
        area
    };
    if app.stats_drill {
        return StatsHits { families: Vec::new(), groups: draw_groups(f, area, app, &groups, true, focused, header_hits) };
    }
    // The families keep at least 8 rows; below them the health keeps its
    // verdict and most telling rows, and the breakdown gets what's left, at
    // least 8 rows of its own when there's room.
    let spare = area.height.saturating_sub(8);
    let load_h = if !beside && spare >= 9 { spare.min(12) } else { 0 };
    let avail = spare.saturating_sub(load_h);
    let families_need = (app.stats_rows().len() as u16 + 4).saturating_sub(8);
    let group_h = (groups.len().max(1) as u16 + 3).min(avail.saturating_sub(families_need).max(8)).min(avail);
    let group_h = if group_h < 5 { 0 } else { group_h };
    let [families, breakdown, load] = Layout::vertical([Constraint::Min(0), Constraint::Length(group_h), Constraint::Length(load_h)]).areas(area);
    let hits = draw_families(f, families, app, focused, header_hits);
    if group_h > 0 {
        draw_groups(f, breakdown, app, &groups, false, false, header_hits);
    }
    if load_h > 0 {
        draw_health(f, load, app);
    }
    StatsHits { families: hits, groups: Vec::new() }
}

fn draw_families(f: &mut Frame, area: Rect, app: &App, focused: bool, header_hits: &mut Vec<HeaderHit>) -> Vec<(u16, String)> {
    let now = Instant::now();
    let stats = &app.stats;
    let rows = app.stats_rows();
    let minions = app.minion_count();
    let up = app.minion_info.iter().filter(|m| m.connected).count();
    let host = app.config_snapshot.as_ref().map_or("?", |c| c.hostname.as_str());
    let version = app.config_snapshot.as_ref().map_or("?", |c| c.salt_version.as_str());
    let reset = stats.reset_at.map(|t| format!(" (reset {})", t.format("%H:%M"))).unwrap_or_default();
    let title = format!(
        " {} \u{00B7} Salt {} \u{00B7} {} minions ({} up) \u{00B7} observed {}{} ",
        host, version, minions, up, fmt_window(stats.observed(now)), reset,
    );

    let selected = rows.iter().position(|(n, _)| *n == app.stats_family).unwrap_or(0);
    let visible = (area.height as usize).saturating_sub(4);
    let (scroll, more_above, more_below) = super::scroll_window(selected, visible, rows.len());
    let hints = [
        ("\u{2191}\u{2193}", "family".to_string()),
        ("Enter", "breakdown".to_string()),
        ("o/O", "sort".to_string()),
        ("g", "group".to_string()),
        ("X", "reset".to_string()),
    ];
    let block = super::list_block_hinted(title, border(focused), color::main_bg(), more_above, more_below, &hints, area.width);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let width = inner.width as usize;
    let recent = width >= RECENT_MIN_WIDTH;
    let fixed = 2 + 8 + 9 + 11 + if recent { 9 + 11 } else { 0 } + 9 + 7;
    let longest = rows.iter().map(|(n, _)| n.chars().count()).max().unwrap_or(0);
    let name_w = width.saturating_sub(fixed).min(longest.max(20) + 2).max(8);
    let per_minion = |per_hour: f64| if minions > 0 { fmt_rate(per_hour / minions as f64) } else { "\u{2014}".to_string() };

    let mut group_line = vec![Span::raw(" ".repeat(2 + name_w)), Span::styled(pad("Since start", 28, false), dim())];
    if recent {
        group_line.push(Span::styled(pad(&format!("Last {}m", RECENT_MINUTES), 20, false), dim()));
    }
    group_line.push(Span::styled("Interval", dim()));
    let mut head = HeaderRow::new(ListId::StatsFamilies, app.list_sort(ListId::StatsFamilies), inner.x, inner.y + 1);
    head.raw(Span::raw("  "));
    head.col("Family", name_w, false, ListCol::Family, header_hits);
    head.col("Count", 8, true, ListCol::Count, header_hits);
    head.col("/h", 9, true, ListCol::Rate, header_hits);
    head.col("/h/minion", 11, true, ListCol::RatePerMinion, header_hits);
    if recent {
        head.col("/h", 9, true, ListCol::RecentRate, header_hits);
        head.col("/h/minion", 11, true, ListCol::RecentRatePerMinion, header_hits);
    }
    head.col("Median", 9, true, ListCol::Median, header_hits);
    let mut lines = vec![Line::from(group_line), head.line()];

    let mut hits = Vec::new();
    for (i, (name, fam)) in rows.iter().enumerate().skip(scroll).take(visible) {
        let per_hour = stats.per_hour(fam.count, now);
        let quiet = fam.count == 0;
        let num = if quiet { dim() } else { text() };
        let mut row = vec![
            Span::raw("  "),
            Span::styled(pad(name, name_w, false), Style::default().fg(if quiet { color::inactive_fg() } else { color::cyan() })),
            Span::styled(pad(&fmt_count(fam.count), 8, true), num),
            Span::styled(pad(&fmt_rate(per_hour), 9, true), num),
            Span::styled(pad(&per_minion(per_hour), 11, true), num),
        ];
        if recent {
            let recent_hour = stats.recent_per_hour(fam, now);
            row.push(Span::styled(pad(&fmt_rate(recent_hour), 9, true), num));
            row.push(Span::styled(pad(&per_minion(recent_hour), 11, true), num));
        }
        let median = fam.median_interval();
        row.push(Span::styled(pad(&median.map_or("\u{2014}".to_string(), fmt_interval), 9, true), num));
        if fam.periodic() {
            row.push(Span::styled(" \u{21BB}", Style::default().fg(color::cyan()).add_modifier(Modifier::BOLD)));
        }
        if stats.low_confidence(fam, now) {
            row.push(Span::styled(" \u{25CC} low", Style::default().fg(color::yellow())));
        }
        let line = Line::from(row);
        hits.push((inner.y + lines.len() as u16, name.to_string()));
        lines.push(if i == selected { color::selected_row(line) } else { line });
    }
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(color::main_bg())), inner);
    hits
}

/// The selected family's breakdown: a preview below the families, cut
/// short with a row summing up the rest, or (`drill`) the whole tab with a
/// cursor, a filter and the groups it was narrowed to in the title.
/// Returns where each group row was drawn.
fn draw_groups(f: &mut Frame, area: Rect, app: &App, groups: &[crate::stats::GroupRow], drill: bool, focused: bool, header_hits: &mut Vec<HeaderHit>) -> Vec<(u16, usize)> {
    let now = Instant::now();
    let stats = &app.stats;
    let family = stats.family(&app.stats_family);
    let total = family.map_or(0, |fam| fam.count);
    let by_minion = app.stats_grouping == Grouping::Minion;
    let filter = app.filter_text_for(crate::types::Tab::Stats);
    let filter_bar = if drill { super::filter_bar(filter, app.is_filtering, app.input_cursor, format!("{} rows", groups.len()), true) } else { None };
    let note = if total == 0 {
        Some("  No events of this family yet.")
    } else if groups.is_empty() && !filter.is_empty() {
        Some("  Nothing matches the filter.")
    } else if groups.is_empty() {
        Some("  No minions in this group.")
    } else if family.is_some_and(|fam| fam.minions.is_empty()) {
        Some("  The master fires these events itself: no minion sends them.")
    } else {
        None
    };
    let overhead = 3 + usize::from(filter_bar.is_some()) + usize::from(note.is_some());
    let visible = (area.height as usize).saturating_sub(overhead);
    let selected = app.stats_group_index(groups);
    let (scroll, more_above, more_below) = if drill { super::scroll_window(selected, visible, groups.len()) } else { (0, false, false) };

    let mut path = vec![app.stats_family.clone()];
    if drill {
        path.extend(app.stats_scopes.iter().map(|s| s.title()));
    }
    let mut title = format!(" {} \u{203A} by {} ", path.join(" \u{203A} "), app.stats_grouping.label());
    if drill && app.stats_scopes.is_empty() {
        let sent = family.map_or(0, |fam| fam.minions.len());
        title.push_str(&format!("\u{00B7} {} of {} minions sent it ", sent, app.minion_count()));
    }
    let hints: Vec<(&str, String)> = if drill {
        let mut h = vec![
            ("\u{2191}\u{2193}", "row".to_string()),
            ("Enter", if by_minion { "detail" } else { "its minions" }.to_string()),
            ("g", "group".to_string()),
            ("o/O", "sort".to_string()),
            ("/", "filter".to_string()),
            ("i/I", "minion list".to_string()),
        ];
        if filter.is_empty() {
            h.push(("Esc/q", "back".to_string()));
        } else {
            h.extend([("Esc", "clear filter".to_string()), ("q", "back".to_string())]);
        }
        h
    } else {
        vec![("g", "group".to_string())]
    };
    let block = super::list_block_hinted(title, border(drill && focused), color::main_bg(), more_above, more_below, &hints, area.width);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let width = inner.width as usize;
    let bar_w = width.saturating_sub(70).min(20);
    let fixed = 2 + if by_minion { 0 } else { 9 } + 9 + if by_minion { 0 } else { 11 } + 8 + bar_w;
    let label_w = groups.iter().map(|g| g.label.chars().count()).max().unwrap_or(0);
    let longest = groups.iter().map(|g| label_w + 1 + g.detail.chars().count()).max().unwrap_or(0);
    let name_w = width.saturating_sub(fixed).min(longest.max(20) + 2).max(8);
    let label_w = label_w.min(name_w / 2);
    let share_of = |events: u64| if total > 0 { events as f64 / total as f64 } else { 0.0 };

    let mut lines: Vec<Line> = filter_bar.into_iter().collect();
    let mut head = HeaderRow::new(ListId::StatsGroups, app.list_sort(ListId::StatsGroups), inner.x, inner.y + lines.len() as u16);
    head.raw(Span::raw("  "));
    head.col(app.stats_grouping.header(), name_w, false, ListCol::Group, header_hits);
    if !by_minion {
        head.col("Minions", 9, true, ListCol::Minions, header_hits);
    }
    head.col("/h", 9, true, ListCol::Rate, header_hits);
    if !by_minion {
        head.col("/h/minion", 11, true, ListCol::RatePerMinion, header_hits);
    }
    head.col("Share", 8, true, ListCol::Share, header_hits);
    lines.push(head.line());
    lines.extend(note.map(|n| Line::from(Span::styled(n, dim()))));

    // The preview gives its last row to the groups it has no room for.
    let cut = !drill && groups.len() > visible;
    let shown = if cut { visible.saturating_sub(1) } else { visible };
    let mut hits = Vec::new();
    for (i, g) in groups.iter().enumerate().skip(scroll).take(shown) {
        let per_hour = stats.per_hour(g.events, now);
        let share = share_of(g.events);
        let num = if g.events == 0 { dim() } else { text() };
        let name = if g.detail.is_empty() { g.label.clone() } else { format!("{:<label_w$} {}", g.label, g.detail) };
        let mut row = vec![Span::raw("  "), Span::styled(pad(&name, name_w, false), Style::default().fg(color::cyan()))];
        if !by_minion {
            row.push(Span::styled(pad(&g.minions.to_string(), 9, true), num));
        }
        row.push(Span::styled(pad(&fmt_rate(per_hour), 9, true), num));
        if !by_minion {
            let per_minion = if g.minions > 0 { fmt_rate(per_hour / g.minions as f64) } else { "\u{2014}".to_string() };
            row.push(Span::styled(pad(&per_minion, 11, true), num));
        }
        row.push(Span::styled(pad(&format!("{:.1}%", share * 100.0), 8, true), num));
        if bar_w > 0 && g.events > 0 {
            let cells = ((share * bar_w as f64).round() as usize).max(1);
            row.push(Span::styled(format!(" {}", "\u{2588}".repeat(cells)), Style::default().fg(color::gradient(share))));
        }
        let line = Line::from(row);
        hits.push((inner.y + lines.len() as u16, i));
        lines.push(if drill && i == selected { color::selected_row(line) } else { line });
    }
    if cut {
        let rest = &groups[shown..];
        let events: u64 = rest.iter().map(|g| g.events).sum();
        let what = if by_minion { "minions" } else { "groups" };
        let label = format!("\u{2026} {} more {}", rest.len(), what);
        let mut row = vec![Span::raw("  "), Span::styled(pad(&label, name_w, false), dim())];
        if !by_minion {
            row.push(Span::styled(pad(&rest.iter().map(|g| g.minions).sum::<usize>().to_string(), 9, true), dim()));
        }
        row.push(Span::styled(pad(&fmt_rate(stats.per_hour(events, now)), 9, true), dim()));
        if !by_minion {
            row.push(Span::raw(" ".repeat(11)));
        }
        row.push(Span::styled(pad(&format!("{:.1}%", share_of(events) * 100.0), 8, true), dim()));
        lines.push(Line::from(row));
    }
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(color::main_bg())), inner);
    hits
}

/// How a health row reads against its thresholds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Rating {
    Ok,
    Busy,
    Starving,
}

impl Rating {
    /// `busy` and `starving` are where the value starts to count as such;
    /// below `busy` when `lower_worse` (memory available).
    fn of(v: f64, busy: f64, starving: f64, lower_worse: bool) -> Self {
        let past = |limit: f64| if lower_worse { v <= limit } else { v >= limit };
        if past(starving) {
            Self::Starving
        } else if past(busy) {
            Self::Busy
        } else {
            Self::Ok
        }
    }

    fn color(self) -> ratatui::style::Color {
        match self {
            Self::Ok => color::green(),
            Self::Busy => color::yellow(),
            Self::Starving => color::red(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Busy => "busy",
            Self::Starving => "starving",
        }
    }
}

/// What the verdict line sums the rows up into.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Resource {
    Cpu,
    Memory,
    Disk,
    Network,
    Workers,
}

const RESOURCES: [(Resource, &str); 5] = [
    (Resource::Cpu, "CPU"),
    (Resource::Memory, "Memory"),
    (Resource::Disk, "Disk"),
    (Resource::Network, "Network"),
    (Resource::Workers, "Workers"),
];

/// How much a row matters: when the panel is too short, the least
/// important rows go first, whole, rather than the panel being cut off.
type Priority = u8;

struct HealthRows {
    rows: Vec<(Priority, Line<'static>)>,
    ratings: Vec<(Resource, Rating)>,
}

const HEALTH_LABEL_W: usize = 28;
const HEALTH_COL_W: usize = 10;
/// Wide enough for the label, Now/Avg/Peak and a short note.
pub const HEALTH_WIDTH: u16 = 78;
/// From this width on, the health column sits right of the families.
pub const HEALTH_BESIDE_MIN_WIDTH: u16 = 160;

impl HealthRows {
    fn push(&mut self, prio: Priority, line: Line<'static>) {
        self.rows.push((prio, line));
    }

    /// An empty line, the first thing to go when the panel is short.
    fn gap(&mut self) {
        self.push(7, Line::default());
    }

    fn section(&mut self, title: &str) {
        self.gap();
        self.push(6, Line::from(Span::styled(format!("  {}", title), header_style())));
    }

    /// A row of three values; `rated` puts a dot before it by `now` and
    /// counts it in the verdict.
    fn values(&mut self, prio: Priority, label: &str, values: [String; 3], rated: Option<(Resource, Rating)>, note: Option<Span<'static>>) {
        let dot = match rated {
            Some((_, r)) => Span::styled("\u{25CF} ", Style::default().fg(r.color())),
            None => Span::raw("  "),
        };
        let mut spans = vec![Span::raw("  "), dot, Span::styled(pad(label, HEALTH_LABEL_W - 4, false), Style::default().fg(color::label_fg()))];
        for v in values {
            spans.push(Span::styled(pad(&v, HEALTH_COL_W, true), text()));
        }
        spans.extend(note);
        self.ratings.extend(rated);
        self.push(prio, Line::from(spans));
    }

    /// A `Metric` row: Now, Avg and Peak, rated by its latest value. Peak is
    /// the worst value, the lowest where lower is worse.
    fn metric(&mut self, prio: Priority, label: &str, m: &Metric, fmt: &dyn Fn(f64) -> String, rate: Option<(Resource, f64, f64, bool)>, note: Option<Span<'static>>) {
        let Some(avg) = m.avg() else { return };
        let worst = rate.filter(|r| r.3).map_or(m.peak, |_| m.low);
        let rated = rate.map(|(res, busy, starving, lower)| (res, Rating::of(m.now, busy, starving, lower)));
        self.values(prio, label, [fmt(m.now), fmt(avg), fmt(worst)], rated, note);
    }

    /// The panel's lines, dropping the least important rows until they fit
    /// in `height`.
    fn fit(mut self, height: usize) -> Vec<Line<'static>> {
        while self.rows.len() > height {
            let Some(at) = self.rows.iter().enumerate().max_by_key(|(i, (p, _))| (*p, *i)).map(|(i, _)| i) else { break };
            self.rows.remove(at);
        }
        self.rows.into_iter().map(|(_, l)| l).collect()
    }
}

fn pct(v: f64) -> String {
    if v < 10.0 { format!("{:.1}%", v) } else { format!("{:.0}%", v) }
}

fn fmt_bytes_per_hour(v: f64) -> String {
    let mb = v / 1e6;
    if mb < 1.0 { format!("{:.0}k", v / 1e3) } else if mb < 1000.0 { format!("{:.0}M", mb) } else { format!("{:.1}G", mb / 1e3) }
}

/// The master's health: whether it's starving for CPU, memory, disk or its
/// workers, what its workers do, how fast it answers and how much work goes
/// through it, all as rates or shares so two masters compare.
fn draw_health(f: &mut Frame, area: Rect, app: &App) {
    let now = Instant::now();
    let stats = &app.stats;
    let sat = &stats.saturation;
    let load = &stats.load;
    let flow = &stats.flow;
    let minute = stats.minute(now);
    let cores = app.config_snapshot.as_ref().map_or(0, |c| c.cpu_count);
    let mem_gb = app.system_info.mem_total_kb as f64 / 1024.0 / 1024.0;
    let title = format!(" Master health \u{00B7} {} cores \u{00B7} {:.1}G ", cores, mem_gb);
    let block = super::titled_block(title, border(false), color::main_bg());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut h = HealthRows { rows: Vec::new(), ratings: Vec::new() };
    let none = || "\u{2014}".to_string();
    let int = |v: f64| format!("{:.0}", v);
    let two = |v: f64| format!("{:.2}", v);

    // Saturation
    h.section("Saturation");
    h.metric(1, "CPU busy", &sat.cpu_busy_pct, &pct, Some((Resource::Cpu, 70.0, 90.0, false)), None);
    h.metric(2, "CPU steal (VM)", &sat.steal_pct, &pct, Some((Resource::Cpu, 2.0, 10.0, false)), None);
    h.metric(2, "Run queue per core", &sat.run_queue, &two, Some((Resource::Cpu, 1.0, 2.0, false)), None);
    if sat.quota {
        h.metric(2, "CPU quota throttled", &sat.throttled_pct, &pct, Some((Resource::Cpu, 5.0, 25.0, false)), None);
    }
    h.metric(1, "Memory available", &sat.mem_avail_pct, &pct, Some((Resource::Memory, 20.0, 10.0, true)), None);
    h.metric(3, "Swap pages /s", &sat.swap_per_sec, &int, Some((Resource::Memory, 10.0, 500.0, false)), None);
    if let Some(oom) = sat.oom_kills {
        let rated = Rating::of(oom as f64, 1.0, 1.0, false);
        h.values(3, "OOM kills", [oom.to_string(), none(), none()], Some((Resource::Memory, rated)), Some(Span::styled("  in window", dim())));
    }
    h.metric(2, "I/O wait", &sat.iowait_pct, &pct, Some((Resource::Disk, 10.0, 30.0, false)), None);
    h.metric(4, "Blocked on I/O", &sat.blocked, &int, None, None);
    if let Some(dev) = &sat.disk {
        let ms = |v: f64| if v < 10.0 { format!("{:.1}ms", v) } else { format!("{:.0}ms", v) };
        h.metric(2, &format!("Disk latency ({})", dev), &sat.disk_latency_ms, &ms, Some((Resource::Disk, 20.0, 100.0, false)), None);
        h.metric(3, "Disk busy", &sat.disk_busy_pct, &pct, Some((Resource::Disk, 70.0, 95.0, false)), None);
        h.metric(4, "Disk MB/s", &sat.disk_mb_per_sec, &|v| format!("{:.1}", v), None, None);
    }
    if sat.has_psi() {
        h.metric(2, "Pressure: CPU", &sat.psi_cpu_pct, &pct, Some((Resource::Cpu, 20.0, 50.0, false)), None);
        h.metric(2, "Pressure: memory", &sat.psi_memory_pct, &pct, Some((Resource::Memory, 5.0, 20.0, false)), None);
        h.metric(2, "Pressure: I/O", &sat.psi_io_pct, &pct, Some((Resource::Disk, 10.0, 30.0, false)), None);
    } else if sat.cpu_busy_pct.avg().is_some() {
        h.push(5, Line::from(Span::styled("    Pressure stall: not tracked by this kernel", dim())));
    }
    if let Some(drops) = sat.listen_drops {
        let rated = Rating::of(drops as f64, 1.0, 100.0, false);
        h.values(3, "Connections dropped", [drops.to_string(), none(), none()], Some((Resource::Network, rated)), Some(Span::styled("  in window", dim())));
    }
    h.metric(3, "TCP retransmits", &sat.retransmit_pct, &pct, Some((Resource::Network, 1.0, 5.0, false)), None);
    if let Some((n, limit)) = sat.open_files {
        let m = &sat.open_files_pct;
        let rated = Rating::of(m.now, 70.0, 90.0, false);
        let note = Span::styled(format!("  {} of {}", fmt_count(n), fmt_count(limit)), dim());
        h.values(3, "Open files (of limit)", [pct(m.now), pct(m.avg().unwrap_or(m.now)), pct(m.peak)], Some((Resource::Network, rated)), Some(note));
    }

    // Workers
    h.section("Workers");
    let threads = app.config_snapshot.as_ref().and_then(|c| c.worker_threads).filter(|&t| t > 0);
    if let Some(avg) = load.workers_busy.avg() {
        let share = threads.map(|t| load.workers_busy.now / t as f64);
        let rated = share.map(|s| (Resource::Workers, Rating::of(s, 0.75, 1.0, false)));
        let now_s = match threads {
            Some(t) => format!("{:.0}/{}", load.workers_busy.now, t),
            None => int(load.workers_busy.now),
        };
        let all = share.is_some_and(|s| s >= 1.0);
        let note = all.then(|| Span::styled("  \u{26A0} all busy", Style::default().fg(color::red()).add_modifier(Modifier::BOLD)));
        h.values(1, "MWorkers busy", [now_s, format!("{:.1}", avg), int(load.workers_busy.peak)], rated, note);
    }
    h.metric(3, "MWorker CPU (cores)", &load.worker_cores, &|v| format!("{:.1}", v), None, None);
    let growth = load.rss_growth_mb_per_hour().map(|mb| {
        let (arrow, style) = if mb > 1.0 {
            ("\u{2197}", Style::default().fg(color::yellow()))
        } else if mb < -1.0 {
            ("\u{2198}", dim())
        } else {
            ("\u{2192}", dim())
        };
        Span::styled(format!("  {} {:+.0}M/h", arrow, mb), style)
    });
    h.metric(3, "MWorker memory", &load.worker_rss_kb, &fmt_kb, None, growth);
    h.metric(4, "Subprocess CPU (cores)", &load.subprocess_cores, &two, None, None);
    h.metric(4, "gpg processes", &load.gpg, &int, None, None);
    let shares = flow.worker_time.shares();
    if shares.is_empty() {
        h.push(5, Line::from(Span::styled("    Worker time by request: set master_stats: True", dim())));
    } else {
        let parts: Vec<String> = shares.iter().take(4).map(|(k, s)| format!("{} {:.0}%", k, s * 100.0)).collect();
        h.push(4, Line::from(vec![Span::styled("    Worker time  ", Style::default().fg(color::label_fg())), Span::styled(parts.join(" \u{00B7} "), text())]));
    }

    // Responsiveness
    h.section("Responsiveness");
    h.metric(2, "ZMQ latency", &load.zmq_ms, &fmt_ms, Some((Resource::Workers, 100.0, 1000.0, false)), None);
    h.metric(3, "Job completion p95", &load.job_p95_secs, &|v| fmt_ms(v * 1000.0), None, None);

    // Throughput
    h.section("Throughput /h");
    let tp = |t: &crate::stats::Throughput, fmt: &dyn Fn(f64) -> String| -> [String; 3] {
        // Early on the average still counts the minute under way, which a
        // burst can lift above the busiest whole minute.
        let avg = stats.per_hour(t.total, now);
        [t.now_per_hour(minute).map_or_else(none, fmt), fmt(avg), fmt(t.peak_per_hour().max(avg))]
    };
    let failed = flow.failed_share().map(|s| Span::styled(format!("  {:.1}% failed", s * 100.0), if s >= 0.1 { Style::default().fg(color::yellow()) } else { dim() }));
    h.values(2, "Events", tp(&flow.events, &fmt_rate), None, None);
    h.values(2, "Job returns", tp(&flow.returns, &fmt_rate), None, failed);
    h.values(3, "Jobs published", tp(&flow.jobs, &fmt_rate), None, None);
    let render_mean = flow.worker_time.mean("_pillar").map(|s| Span::styled(format!("  mean {}", fmt_ms(s * 1000.0)), dim()));
    h.values(3, "Pillar renders", tp(&flow.renders, &fmt_rate), None, render_mean);
    let acts: Vec<String> = crate::stats::Flow::AUTH_ACTS.iter()
        .filter_map(|a| flow.auth_by_act.get(*a).map(|n| format!("{} {}", a, n)))
        .collect();
    let auth_note = (!acts.is_empty()).then(|| Span::styled(format!("  {}", acts.join(" \u{00B7} ")), dim()));
    h.values(3, "Auth requests", tp(&flow.auth, &fmt_rate), None, auth_note);
    h.values(3, "Event bus", tp(&flow.bytes, &fmt_bytes_per_hour), None, None);

    // Fleet
    h.gap();
    let accepted = app.minion_count();
    let up = app.minion_info.iter().filter(|m| m.connected).count();
    let pending = app.keys.iter().filter(|k| k.status == crate::types::KeyStatus::Pending).count();
    let heard = stats.minions_heard();
    let silent = accepted.saturating_sub(heard);
    h.push(2, Line::from(vec![
        Span::styled("  Fleet  ", header_style()),
        Span::styled(format!("{} accepted \u{00B7} {} up \u{00B7} {} silent in window \u{00B7} {} pending", accepted, up, silent, pending), text()),
    ]));

    // The verdict and the column heads go on top.
    let verdict = verdict_line(&h.ratings);
    let mut head = vec![Span::raw(" ".repeat(HEALTH_LABEL_W))];
    for col in ["Now", "Avg", "Peak"] {
        head.push(Span::styled(pad(col, HEALTH_COL_W, true), header_style()));
    }
    h.rows.insert(0, (0, verdict));
    h.rows.insert(1, (1, Line::from(head)));
    let lines = h.fit(inner.height as usize);
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(color::main_bg())), inner);
}

/// Each resource's worst rated row as a colored dot (a dash before any
/// reading), then the ones not ok named in words, worst first.
fn verdict_line(ratings: &[(Resource, Rating)]) -> Line<'static> {
    let worst = |res: Resource| ratings.iter().filter(|(r, _)| *r == res).map(|(_, x)| *x).max();
    let mut spans = vec![Span::raw(" ")];
    for (res, name) in RESOURCES {
        let (mark, style) = match worst(res) {
            Some(r) => ("\u{25CF}", Style::default().fg(r.color())),
            None => ("\u{2014}", dim()),
        };
        spans.push(Span::styled(format!(" {} ", mark), style));
        spans.push(Span::styled(name, Style::default().fg(color::label_fg())));
    }
    let mut problems: Vec<Span<'static>> = Vec::new();
    for rating in [Rating::Starving, Rating::Busy] {
        let names: Vec<&str> = RESOURCES.iter().filter(|(res, _)| worst(*res) == Some(rating)).map(|(_, n)| *n).collect();
        if !names.is_empty() {
            let sep = if problems.is_empty() { "   " } else { " \u{00B7} " };
            problems.push(Span::raw(sep));
            problems.push(Span::styled(format!("{}: {}", rating.label(), names.join(", ")), Style::default().fg(rating.color()).add_modifier(Modifier::BOLD)));
        }
    }
    if problems.is_empty() && ratings.iter().any(|(_, r)| *r == Rating::Ok) {
        problems.push(Span::styled("   all ok", Style::default().fg(Rating::Ok.color())));
    }
    spans.extend(problems);
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_stay_short() {
        assert_eq!([fmt_count(42), fmt_count(108_000), fmt_count(2_500_000)], ["42", "108.0k", "2.5M"]);
        assert_eq!([fmt_rate(0.0), fmt_rate(0.004), fmt_rate(0.02), fmt_rate(12.04), fmt_rate(286.0), fmt_rate(25_700.0)], ["0", "<0.01", "0.02", "12.0", "286", "25.7k"]);
        assert_eq!([fmt_interval(2.34), fmt_interval(300.0), fmt_interval(3900.0)], ["2.3s", "5m00s", "1h05m"]);
        assert_eq!([fmt_window(Duration::from_secs(45)), fmt_window(Duration::from_secs(31 * 60)), fmt_window(Duration::from_secs(4 * 3600 + 12 * 60))], ["45s", "31m", "4h12m"]);
    }

    #[test]
    fn ratings_follow_thresholds_either_way() {
        assert_eq!([Rating::of(50.0, 70.0, 90.0, false), Rating::of(70.0, 70.0, 90.0, false), Rating::of(95.0, 70.0, 90.0, false)], [Rating::Ok, Rating::Busy, Rating::Starving]);
        assert_eq!([Rating::of(60.0, 20.0, 10.0, true), Rating::of(15.0, 20.0, 10.0, true), Rating::of(5.0, 20.0, 10.0, true)], [Rating::Ok, Rating::Busy, Rating::Starving]);
        let line = |s: &str| Line::from(s.to_string());
        let rows = HealthRows {
            rows: vec![(0, line("verdict")), (7, line("")), (6, line("section")), (1, line("cpu")), (4, line("gpg")), (7, line("")), (6, line("section 2")), (2, line("zmq"))],
            ratings: Vec::new(),
        };
        let kept: Vec<String> = rows.fit(4).iter().map(|l| l.to_string()).collect();
        assert_eq!(kept, ["verdict", "cpu", "gpg", "zmq"]);
        let text = verdict_line(&[(Resource::Cpu, Rating::Ok), (Resource::Cpu, Rating::Busy), (Resource::Workers, Rating::Starving), (Resource::Disk, Rating::Ok)]).to_string();
        assert_eq!(text, "  \u{25CF} CPU \u{2014} Memory \u{25CF} Disk \u{2014} Network \u{25CF} Workers   starving: Workers \u{00B7} busy: CPU");
        assert!(verdict_line(&[(Resource::Disk, Rating::Ok)]).to_string().ends_with("all ok"));
    }
}
