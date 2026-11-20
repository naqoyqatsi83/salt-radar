use std::time::{Duration, Instant};

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::stats::{Grouping, Metric, RECENT_MINUTES};

use super::color;
use super::table::pad;

/// Below this many columns inside the families panel, the last-15-minutes
/// columns are left out.
const RECENT_MIN_WIDTH: usize = 84;

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

fn header() -> Style {
    Style::default().fg(color::header_fg()).add_modifier(Modifier::BOLD)
}

fn border(focused: bool) -> Style {
    Style::default().fg(if focused { color::border_focus() } else { color::border() })
}

/// The Stats tab: event families, the selected one's breakdown, the
/// master's load. Returns where each family row was drawn, for clicks.
pub fn draw_stats(f: &mut Frame, area: Rect, app: &App, focused: bool) -> Vec<(u16, String)> {
    let groups = app.stats_groups();
    // The families keep at least 8 rows; the breakdown gets what they don't
    // need, at least 8 rows of its own when there's room.
    let spare = area.height.saturating_sub(8);
    let load_h = if spare >= 9 { 9 } else { 0 };
    let avail = spare.saturating_sub(load_h);
    let families_need = (app.stats_rows().len() as u16 + 4).saturating_sub(8);
    let group_h = (groups.len().max(1) as u16 + 3).min(avail.saturating_sub(families_need).max(8)).min(avail);
    let group_h = if group_h < 5 { 0 } else { group_h };
    let [families, breakdown, load] = Layout::vertical([Constraint::Min(0), Constraint::Length(group_h), Constraint::Length(load_h)]).areas(area);
    let hits = draw_families(f, families, app, focused);
    if group_h > 0 {
        draw_breakdown(f, breakdown, app, &groups);
    }
    if load_h > 0 {
        draw_load(f, load, app);
    }
    hits
}

fn draw_families(f: &mut Frame, area: Rect, app: &App, focused: bool) -> Vec<(u16, String)> {
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
        ("s", if app.stats_by_count { "fixed order" } else { "sort by count" }.to_string()),
        ("g", "group".to_string()),
        ("X", "reset".to_string()),
    ];
    let block = super::list_block_hinted(title, border(focused), color::main_bg(), more_above, more_below, &hints, area.width);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let width = inner.width as usize;
    let recent = width >= RECENT_MIN_WIDTH;
    let fixed = 2 + 8 + 9 + 10 + if recent { 9 + 10 } else { 0 } + 9 + 7;
    let longest = rows.iter().map(|(n, _)| n.chars().count()).max().unwrap_or(0);
    let name_w = width.saturating_sub(fixed).min(longest.max(20) + 2).max(8);
    let per_minion = |per_hour: f64| if minions > 0 { fmt_rate(per_hour / minions as f64) } else { "\u{2014}".to_string() };

    let mut group_line = vec![Span::raw(" ".repeat(2 + name_w)), Span::styled(pad("since start", 27, false), dim())];
    if recent {
        group_line.push(Span::styled(pad(&format!("last {}m", RECENT_MINUTES), 19, false), dim()));
    }
    group_line.push(Span::styled("interval", dim()));
    let mut head = vec![
        Span::raw("  "),
        Span::styled(pad("family", name_w, false), header()),
        Span::styled(pad("count", 8, true), header()),
        Span::styled(pad("/h", 9, true), header()),
        Span::styled(pad("/h/minion", 10, true), header()),
    ];
    if recent {
        head.push(Span::styled(pad("/h", 9, true), header()));
        head.push(Span::styled(pad("/h/minion", 10, true), header()));
    }
    head.push(Span::styled(pad("median", 9, true), header()));
    let mut lines = vec![Line::from(group_line), Line::from(head)];

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
            Span::styled(pad(&per_minion(per_hour), 10, true), num),
        ];
        if recent {
            let recent_hour = stats.recent_per_hour(fam, now);
            row.push(Span::styled(pad(&fmt_rate(recent_hour), 9, true), num));
            row.push(Span::styled(pad(&per_minion(recent_hour), 10, true), num));
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

fn draw_breakdown(f: &mut Frame, area: Rect, app: &App, groups: &[crate::stats::GroupRow]) {
    let now = Instant::now();
    let stats = &app.stats;
    let total = stats.family(&app.stats_family).map_or(0, |fam| fam.count);
    let visible = (area.height as usize).saturating_sub(3);
    let title = format!(" By {} \u{00B7} {} ", app.stats_grouping.label(), app.stats_family);
    let hints = [("g", "group".to_string())];
    let block = super::list_block_hinted(title, border(false), color::main_bg(), false, groups.len() > visible, &hints, area.width);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let by_minion = app.stats_grouping == Grouping::Minion;
    let width = inner.width as usize;
    let bar_w = width.saturating_sub(70).min(20);
    let fixed = 2 + if by_minion { 0 } else { 9 } + 9 + if by_minion { 0 } else { 10 } + 8 + bar_w;
    let label_w = groups.iter().map(|g| g.label.chars().count()).max().unwrap_or(0);
    let longest = groups.iter().map(|g| label_w + 1 + g.detail.chars().count()).max().unwrap_or(0);
    let name_w = width.saturating_sub(fixed).min(longest.max(20) + 2).max(8);
    let label_w = label_w.min(name_w / 2);

    let mut head = vec![Span::raw("  "), Span::styled(pad(app.stats_grouping.label(), name_w, false), header())];
    if !by_minion {
        head.push(Span::styled(pad("minions", 9, true), header()));
    }
    head.push(Span::styled(pad("/h", 9, true), header()));
    if !by_minion {
        head.push(Span::styled(pad("/h/minion", 10, true), header()));
    }
    head.push(Span::styled(pad("share", 8, true), header()));
    let mut lines = vec![Line::from(head)];
    if total == 0 {
        lines.push(Line::from(Span::styled("  No events of this family yet.", dim())));
    }
    for g in groups.iter().take(visible) {
        let per_hour = stats.per_hour(g.events, now);
        let share = if total > 0 { g.events as f64 / total as f64 } else { 0.0 };
        let num = if g.events == 0 { dim() } else { text() };
        let name = if g.detail.is_empty() { g.label.clone() } else { format!("{:<label_w$} {}", g.label, g.detail) };
        let mut row = vec![Span::raw("  "), Span::styled(pad(&name, name_w, false), Style::default().fg(color::cyan()))];
        if !by_minion {
            row.push(Span::styled(pad(&g.minions.to_string(), 9, true), num));
        }
        row.push(Span::styled(pad(&fmt_rate(per_hour), 9, true), num));
        if !by_minion {
            let per_minion = if g.minions > 0 { fmt_rate(per_hour / g.minions as f64) } else { "\u{2014}".to_string() };
            row.push(Span::styled(pad(&per_minion, 10, true), num));
        }
        row.push(Span::styled(pad(&format!("{:.1}%", share * 100.0), 8, true), num));
        if bar_w > 0 && g.events > 0 {
            let cells = ((share * bar_w as f64).round() as usize).max(1);
            row.push(Span::styled(format!(" {}", "\u{2588}".repeat(cells)), Style::default().fg(color::gradient(share))));
        }
        lines.push(Line::from(row));
    }
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(color::main_bg())), inner);
}

fn draw_load(f: &mut Frame, area: Rect, app: &App) {
    let load = &app.stats.load;
    let block = super::titled_block(" Master load ", border(false), color::main_bg());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let label_w = 26;
    let none = || "\u{2014}".to_string();
    let cols = |m: &Metric, fmt: &dyn Fn(f64) -> String| -> [String; 3] {
        match m.avg() {
            Some(avg) => [fmt(m.now), fmt(avg), fmt(m.peak)],
            None => [none(), none(), none()],
        }
    };
    let row = |label: &str, values: [String; 3], note: Option<Span<'static>>| {
        let mut spans = vec![Span::styled(pad(&format!("  {}", label), label_w, false), Style::default().fg(color::label_fg()))];
        for v in values {
            spans.push(Span::styled(pad(&v, 10, true), text()));
        }
        spans.extend(note);
        Line::from(spans)
    };

    let threads = app.config_snapshot.as_ref().and_then(|c| c.worker_threads).filter(|&t| t > 0);
    let mut busy = cols(&load.workers_busy, &|v| format!("{:.1}", v));
    if load.workers_busy.avg().is_some() {
        busy[0] = format!("{:.0}", load.workers_busy.now);
        busy[2] = format!("{:.0}", load.workers_busy.peak);
        if let Some(t) = threads {
            busy[0] = format!("{}/{}", busy[0], t);
        }
    }
    let all_busy = threads.is_some_and(|t| load.workers_busy.avg().is_some() && load.workers_busy.now >= t as f64);
    let busy_note = all_busy.then(|| Span::styled("  \u{26A0} all busy", Style::default().fg(color::red()).add_modifier(Modifier::BOLD)));
    let mut workers = cols(&load.worker_cores, &|v| format!("{:.1}", v));
    if let (Some(t), Some(_)) = (threads, load.worker_cores.avg()) {
        workers[0] = format!("{:.1}/{}", load.worker_cores.now, t);
    }
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

    let mut head = vec![Span::raw(" ".repeat(label_w))];
    for h in ["now", "avg", "peak"] {
        head.push(Span::styled(pad(h, 10, true), header()));
    }
    let lines = vec![
        Line::from(head),
        row("MWorkers busy", busy, busy_note),
        row("MWorker CPU (cores)", workers, None),
        row("MWorker memory", cols(&load.worker_rss_kb, &fmt_kb), growth),
        row("Subprocess CPU (cores)", cols(&load.subprocess_cores, &|v| format!("{:.2}", v)), None),
        row("gpg processes", cols(&load.gpg, &|v| format!("{:.0}", v)), None),
        row("ZMQ latency", cols(&load.zmq_ms, &fmt_ms), None),
    ];
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(color::main_bg())), inner);
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
}
