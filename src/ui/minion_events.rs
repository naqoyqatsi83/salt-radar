use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{return_event_jid, return_event_result, App, MinionActivityItem, ReturnResult, MINION_EVENTS_HEADER_ROWS};
use crate::types::{format_job_args, AlertSeverity};

use super::color;
use super::detail_popup;

fn format_time_local(ts: f64) -> String {
    if ts < 1.0 {
        return "\u{2014}".to_string();
    }
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "\u{2014}".to_string())
}

/// Classify an event tag into a short type label and return color + JID if applicable.
fn classify_tag(tag: &str, data: &serde_json::Value) -> (&'static str, Color, Option<String>) {
    if tag.contains("/ret/") {
        let jid = tag.split('/').nth(2).map(|s| {
            if s.len() > 18 { format!("{}..", &s[..16]) } else { s.to_string() }
        });
        let fun = data["fun"].as_str().unwrap_or("return");
        return ("return", color::green(), Some(format!("{}/{}", fun, jid.unwrap_or_default())));
    }
    if tag.contains("/start") || tag.contains("minion_start") {
        return ("start", color::cyan(), None);
    }
    if tag.starts_with("salt/beacon/") {
        let beacon = tag.split('/').nth(2).unwrap_or("beacon");
        return ("beacon", color::yellow(), Some(beacon.to_string()));
    }
    if tag.contains("salt/auth") {
        return ("auth", color::purple(), None);
    }
    if let Some(fun) = data["fun"].as_str() {
        return ("fun", color::main_fg(), Some(fun.to_string()));
    }
    ("other", color::inactive_fg(), None)
}

fn alert_color(severity: AlertSeverity) -> Color {
    match severity {
        AlertSeverity::Crit => color::red(),
        AlertSeverity::Warn => color::orange(),
        AlertSeverity::Info => color::cyan(),
    }
}

/// Truncates to `w` characters, marking a cut with `..`.
fn fit(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(w.saturating_sub(2)).collect();
        out.push_str("..");
        out
    }
}

fn ago(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    match s {
        0..=59 => format!("{}s", s),
        60..=3599 => format!("{}m", s / 60),
        3600..=86399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86400),
    }
}

/// The Result cell of a job return: state counts for state runs, else OK or
/// FAIL with the retcode.
fn result_cell(r: &ReturnResult) -> (String, Color) {
    let color = if !r.success { color::outcome_failed() } else { color::outcome_ok() };
    match r.counts {
        Some(c) => {
            let mut text = format!("{} ok", c.ok);
            if c.chg() > 0 { text.push_str(&format!(" {} chg", c.chg())); }
            if c.failed > 0 { text.push_str(&format!(" {} fail", c.failed)); }
            let color = if c.failed > 0 || !r.success {
                color::outcome_failed()
            } else if c.pending > 0 {
                color::outcome_pending()
            } else if c.changed > 0 {
                color::outcome_changed()
            } else {
                color::outcome_ok()
            };
            (text, color)
        }
        None if r.success => ("OK".to_string(), color),
        None => (format!("FAIL rc={}", r.retcode), color),
    }
}

fn health_line(app: &App, minion_id: &str, now: f64) -> Line<'static> {
    let Some(h) = app.minion_health(minion_id).filter(|h| !h.recent.is_empty()) else {
        return Line::from(Span::styled(
            "  No job returns from this minion since salt-radar started.",
            Style::default().fg(color::inactive_fg()),
        ));
    };
    let (returns, failed) = (h.recent.len(), h.failed());
    let mut spans = vec![
        Span::styled(format!("  Last {} returns: ", returns), Style::default().fg(color::label_fg())),
        Span::styled(format!("{} OK", returns - failed), Style::default().fg(color::outcome_ok()).add_modifier(Modifier::BOLD)),
    ];
    if failed == 0 {
        spans.push(Span::styled(" \u{b7} no failures", Style::default().fg(color::inactive_fg())));
    } else {
        spans.push(Span::styled(" \u{b7} ", Style::default().fg(color::inactive_fg())));
        spans.push(Span::styled(
            format!("{} FAIL ({:.0}%)", failed, (failed as f64 * 100.0 / returns as f64).round().max(1.0)),
            Style::default().fg(color::outcome_failed()).add_modifier(Modifier::BOLD),
        ));
        if let Some((ts, what)) = &h.last_failure {
            spans.push(Span::styled(format!(" \u{b7} last failure {} ago: ", ago(now - ts)), Style::default().fg(color::label_fg())));
            spans.push(Span::styled(what.clone(), Style::default().fg(color::yellow())));
        }
    }
    if let (Some(median), Some(p95)) = (h.lag_at(0.5), h.lag_at(0.95)) {
        spans.push(Span::styled(format!(" \u{b7} lag over {} jobs: ", h.lag.len()), Style::default().fg(color::label_fg())));
        spans.push(Span::styled(super::format_lag(median), Style::default().fg(super::lag_color(median)).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(", p95 ", Style::default().fg(color::label_fg())));
        spans.push(Span::styled(super::format_lag(p95), Style::default().fg(super::lag_color(p95))));
    }
    if let Some(ping) = h.last_ping_secs {
        spans.push(Span::styled(" \u{b7} last ping ", Style::default().fg(color::label_fg())));
        spans.push(Span::styled(super::format_secs_short(ping), Style::default().fg(color::main_fg())));
    }
    Line::from(spans)
}

pub fn draw_minion_events(f: &mut Frame, area: Rect, app: &App, minion_id: &str, selected: usize) {
    let area = detail_popup::centered_rect(85, 80, area);
    f.render_widget(Clear, area);

    let items = app.minion_activity(minion_id);
    let total = items.len();
    let sel = selected.min(total.saturating_sub(1));

    let hints: Vec<(&str, String)> = if app.minion_events_is_filtering {
        vec![("Esc", "cancel".to_string()), ("Enter", "apply".to_string())]
    } else {
        vec![
            ("Esc/q", "close".to_string()),
            ("\u{2191}\u{2193}", "navigate".to_string()),
            ("PgUp/PgDn", "page".to_string()),
            ("Home/End", "top/bottom".to_string()),
            ("Enter", "detail".to_string()),
            ("/", "filter".to_string()),
            ("y", "copy".to_string()),
            ("s", "save".to_string()),
        ]
    };
    let count = if app.minion_events_filter.is_empty() {
        format!("{} items", total)
    } else {
        format!("filter {}: {} items", app.minion_events_filter, total)
    };
    let hint_line = super::border_hints(&hints, color::border_focus());
    let info = super::border_info(format!("{} \u{b7} dim = older than 5m", count), color::border_focus());
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::border_focus()))
        .style(Style::default().bg(color::popup_bg()));
    if super::fits_beside(hint_line.width(), &info, area.width) {
        block = block.title_bottom(info.right_aligned());
    }
    let block = block.title_bottom(hint_line.left_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);

    let rule = Line::from(Span::styled(
        format!("  {}", "\u{2500}".repeat((inner.width as usize).saturating_sub(2))),
        Style::default().fg(color::div_line()),
    ));

    // Title: os/version only once grains have been retrieved for this minion.
    let mut title = format!("  \u{25C8} {}", minion_id);
    if let Some(minion) = app.minion_info.iter().find(|m| m.id == *minion_id) {
        let mut parts = vec![if minion.connected { "connected" } else { "disconnected" }.to_string()];
        if !minion.os.is_empty() {
            parts.push(if !minion.osrelease.is_empty() {
                format!("{} {}", minion.os, minion.osrelease)
            } else {
                minion.os.clone()
            });
        }
        if !minion.salt_version.is_empty() {
            parts.push(format!("v{}", minion.salt_version));
        }
        title.push_str(&format!("  [{}]", parts.join(", ")));
    }

    let (time_w, type_w, fun_w, result_w, jid_w) = (14usize, 7usize, 18usize, 17usize, 20usize);
    let args_w = (inner.width as usize).saturating_sub(2 + time_w + type_w + fun_w + result_w + jid_w + 10).max(8);
    // Rows that aren't jobs (beacons, auth, alerts, ...) use Args..JID as one wide cell.
    let wide_w = args_w + result_w + jid_w + 4;

    let header = vec![
        Line::from(Span::styled(title, Style::default().fg(color::title()).add_modifier(Modifier::BOLD))),
        health_line(app, minion_id, now),
        rule.clone(),
        Line::from(Span::styled(
            format!("  {:<time_w$}  {:<type_w$}  {:<fun_w$}  {:<args_w$}  {:<result_w$}  {}",
                "Time", "Type", "Function", "Args", "Result", "JID / Source"),
            Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD),
        )),
        rule,
    ];

    let status = app.minion_events_is_filtering.then(|| Line::from(super::input_spans(
        "  filter: ",
        &app.minion_events_filter,
        app.input_cursor,
        Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD),
    )));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(MINION_EVENTS_HEADER_ROWS),
            Constraint::Min(1),
            Constraint::Length(status.is_some() as u16),
        ])
        .split(inner);
    f.render_widget(Paragraph::new(header).style(Style::default().bg(color::popup_bg())), chunks[0]);

    let mut lines: Vec<Line> = Vec::new();
    if total == 0 {
        let msg = if app.minion_events_filter.is_empty() {
            format!("  No activity for {}. Salt scheduler or beacons will populate this view.", minion_id)
        } else {
            "  Nothing matches the filter.".to_string()
        };
        lines.push(Line::from(Span::styled(msg, Style::default().fg(color::inactive_fg()))));
    }

    let stale_threshold = 300.0;
    for (idx, item) in items.iter().enumerate() {
        let stale = match item {
            MinionActivityItem::Alert(_) => false,
            _ => now - item.ts() > stale_threshold,
        };
        let dim = |c: Color| if stale { color::inactive_fg() } else { c };
        let cell = |text: &str, w: usize, c: Color| Span::styled(format!("  {:<w$}", fit(text, w)), Style::default().fg(dim(c)));
        let time = match item {
            MinionActivityItem::Alert(_) => "  --".to_string(),
            _ => format_time_local(item.ts()),
        };
        let mut spans = vec![cell(&time, time_w, color::main_fg())];

        let job = match item {
            MinionActivityItem::Event(_, evt) => return_event_jid(evt).map(|jid| {
                let fun = evt.data["fun"].as_str().unwrap_or("?").to_string();
                let (args, test_run) = format_job_args(&fun, &evt.data["fun_args"]);
                ("return", color::green(), fun, args, test_run, Some(return_event_result(&evt.data)), jid.to_string())
            }),
            MinionActivityItem::CacheJob(job) => Some((
                "cache", color::cyan(), job.fun.clone(), job.args.clone(), job.test_run, None, job.jid.clone(),
            )),
            MinionActivityItem::Alert(_) => None,
        };
        match (job, item) {
            (Some((kind, kind_color, fun, args, test_run, result, jid)), _) => {
                let args = if test_run { format!("{} test=True", args).trim_start().to_string() } else { args };
                let (result_text, result_color) = match &result {
                    Some(r) => result_cell(r),
                    None => ("\u{2014}".to_string(), color::inactive_fg()),
                };
                spans.push(Span::styled(format!("  {:<type_w$}", kind), Style::default().fg(dim(kind_color)).add_modifier(Modifier::BOLD)));
                spans.push(cell(&fun, fun_w, color::yellow()));
                spans.push(cell(&args, args_w, if test_run { color::orange() } else { color::main_fg() }));
                spans.push(cell(&result_text, result_w, result_color));
                spans.push(cell(&jid, jid_w, color::cyan()));
            }
            (None, MinionActivityItem::Alert(alert)) => {
                let acolor = alert_color(alert.severity);
                spans.push(Span::styled(format!("  {:<type_w$}", "alert"), Style::default().fg(acolor).add_modifier(Modifier::BOLD)));
                spans.push(Span::styled(format!("  {:<fun_w$}", alert.severity.label()), Style::default().fg(acolor)));
                spans.push(cell(&alert.description, wide_w, color::main_fg()));
            }
            (None, MinionActivityItem::Event(_, evt)) => {
                let (etype, ecolor, extra) = classify_tag(&evt.tag, &evt.data);
                spans.push(Span::styled(format!("  {:<type_w$}", etype), Style::default().fg(dim(ecolor)).add_modifier(Modifier::BOLD)));
                spans.push(cell(extra.as_deref().unwrap_or(""), fun_w, color::main_fg()));
                spans.push(cell(&evt.tag, wide_w, color::cyan()));
            }
            (None, MinionActivityItem::CacheJob(_)) => {}
        }
        let line = Line::from(spans);
        lines.push(if idx == sel { color::selected_row(line) } else { line });
    }

    let list_h = chunks[1].height as usize;
    let (scroll, more_above, more_below) = super::scroll_window(sel, list_h, lines.len());
    f.render_widget(
        Paragraph::new(lines).scroll((scroll as u16, 0)).style(Style::default().bg(color::popup_bg())),
        chunks[1],
    );
    if total > 0 && sel >= scroll && sel < scroll + list_h {
        let row = Rect { y: chunks[1].y + (sel - scroll) as u16, height: 1, ..chunks[1] };
        f.buffer_mut().set_style(row, color::select_bg());
    }
    if more_above || more_below {
        let marks = format!("{}{}", if more_above { "\u{25B2}" } else { "" }, if more_below { "\u{25BC}" } else { "" });
        let w = marks.chars().count() as u16 + 1;
        let at = Rect { x: inner.x + inner.width.saturating_sub(w), y: chunks[0].y + 3, width: w, height: 1 };
        f.render_widget(Paragraph::new(Span::styled(marks, Style::default().fg(color::label_fg()))), at);
    }
    if let Some(status_line) = status {
        f.render_widget(
            Paragraph::new(status_line).style(Style::default().bg(color::popup_bg())),
            chunks[2],
        );
    }
}
