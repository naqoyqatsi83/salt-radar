use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, MinionActivityItem};
use crate::types::AlertSeverity;

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
        return ("return", color::GREEN, Some(format!("{}/{}", fun, jid.unwrap_or_default())));
    }
    if tag.contains("/start") || tag.contains("minion_start") {
        return ("start", color::CYAN, None);
    }
    if tag.starts_with("salt/beacon/") {
        let beacon = tag.split('/').nth(2).unwrap_or("beacon");
        return ("beacon", color::YELLOW, Some(beacon.to_string()));
    }
    if tag.contains("salt/auth") {
        return ("auth", color::PURPLE, None);
    }
    if let Some(fun) = data["fun"].as_str() {
        return ("fun", color::TEXT, Some(fun.to_string()));
    }
    ("other", color::TEXT_DIM, None)
}

fn alert_color(severity: AlertSeverity) -> Color {
    match severity {
        AlertSeverity::Crit => color::RED,
        AlertSeverity::Warn => color::ORANGE,
        AlertSeverity::Info => color::CYAN,
    }
}

pub fn draw_minion_events(f: &mut Frame, area: Rect, app: &App, minion_id: &str, selected: usize) {
    let area = detail_popup::centered_rect(85, 80, area);
    f.render_widget(Clear, area);

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
        ]
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER_HI))
        .style(Style::default().bg(color::BG_POPUP))
        .title_bottom(super::border_hints(&hints).left_aligned())
        .title_bottom(super::border_info("dim = older than 5m").right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items = app.minion_activity(minion_id);
    let total = items.len();
    let sel = selected.min(total.saturating_sub(1));

    let mut lines: Vec<Line> = Vec::new();
    // Full-width rule matching the popup's actual rendered width, instead of
    // a fixed-length string that only ever matched one particular size.
    let rule = format!("  {}", "\u{2500}".repeat((inner.width as usize).saturating_sub(2)));

    // Title bar with minion summary — os/version only if grains have
    // actually been retrieved for this minion, instead of showing blank
    // runs like "[connected,    v]" for whatever fields aren't known yet.
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
    lines.push(Line::from(Span::styled(
        title,
        Style::default().fg(color::TEXT_TITLE).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        rule.clone(),
        Style::default().fg(Color::Rgb(35, 50, 80)),
    )));

    if total == 0 {
        lines.push(Line::from(Span::styled(
            format!("  No activity for {}. Salt scheduler or beacons will populate this view.", minion_id),
            Style::default().fg(color::TEXT_DIM),
        )));
    } else {
        // Column widths: Time | Type | JID/Fun | Tag
        let avail = (inner.width as usize).saturating_sub(6);
        let time_w: usize = 14;
        let type_w: usize = 10;
        let jid_w: usize = 24;
        let tag_w = avail.saturating_sub(time_w + type_w + jid_w + 3).max(8);

        lines.push(Line::from(vec![
            Span::styled(format!("  {:<tw$}  {:<tyw$}  {:<jw$}  {:<tag_w$}",
                "Time", "Type", "JID/Fun", "Source",
                tw = time_w, tyw = type_w, jw = jid_w, tag_w = tag_w),
                Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(Span::styled(
            rule.clone(),
            Style::default().fg(Color::Rgb(35, 50, 80)),
        )));

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let stale_threshold = 300.0;

        for idx in 0..total {
            let item = &items[idx];
            let is_sel = idx == sel;
            let sel_bg = if is_sel { color::select_bg(app.high_contrast).bg } else { None };
            let span_style = |s: Style| -> Style {
                match sel_bg { Some(bg) => s.bg(bg), None => s }
            };

            match item {
                MinionActivityItem::Event(_, evt) => {
                    let is_stale = now - evt.ts > stale_threshold;
                    let local_t = format_time_local(evt.ts);
                    let (etype, ecolor, extra) = classify_tag(&evt.tag, &evt.data);

                    let extra_str = extra.as_deref().unwrap_or("");
                    let extra_disp = if extra_str.len() > jid_w {
                        format!("{}..", &extra_str[..jid_w.saturating_sub(2)])
                    } else {
                        extra_str.to_string()
                    };

                    let tag_disp = if evt.tag.len() > tag_w {
                        format!("{}..", &evt.tag[..tag_w.saturating_sub(2)])
                    } else {
                        evt.tag.clone()
                    };

                    lines.push(Line::from(vec![
                        Span::styled(format!("  {:<tw$}", local_t, tw = time_w),
                            span_style(Style::default().fg(if is_stale { color::TEXT_DIM } else { color::TEXT }))),
                        Span::styled(format!("  {:<tyw$}", etype, tyw = type_w),
                            span_style(Style::default().fg(if is_stale { color::TEXT_DIM } else { ecolor }).add_modifier(Modifier::BOLD))),
                        Span::styled(format!("  {:<jw$}", extra_disp, jw = jid_w),
                            span_style(Style::default().fg(if is_stale { color::TEXT_DIM } else if etype == "return" { color::YELLOW } else { color::TEXT }))),
                        Span::styled(format!("  {:<tag_w$}", tag_disp, tag_w = tag_w),
                            span_style(Style::default().fg(if is_stale { color::TEXT_DIM } else { color::CYAN }))),
                    ]));
                }
                MinionActivityItem::CacheJob(job) => {
                    let is_stale = now - job.start_epoch > stale_threshold;
                    let base_fg = if is_stale { color::TEXT_DIM } else { color::TEXT };
                    let local_t = format_time_local(job.start_epoch);
                    let jid_disp = if job.jid.len() > jid_w {
                        format!("{}..", &job.jid[..jid_w.saturating_sub(2)])
                    } else {
                        job.jid.clone()
                    };

                    lines.push(Line::from(vec![
                        Span::styled(format!("  {:<tw$}", local_t, tw = time_w),
                            span_style(Style::default().fg(base_fg))),
                        Span::styled(format!("  {:<tyw$}", "job_cache", tyw = type_w),
                            span_style(Style::default().fg(if is_stale { color::TEXT_DIM } else { color::CYAN }).add_modifier(Modifier::BOLD))),
                        Span::styled(format!("  {:<jw$}", job.fun, jw = jid_w),
                            span_style(Style::default().fg(if is_stale { color::TEXT_DIM } else { color::YELLOW }))),
                        Span::styled(format!("  {:<tag_w$}", jid_disp, tag_w = tag_w),
                            span_style(Style::default().fg(if is_stale { color::TEXT_DIM } else { color::CYAN }))),
                    ]));
                }
                MinionActivityItem::Alert(alert) => {
                    let acolor = alert_color(alert.severity);
                    let desc_disp = if alert.description.len() > tag_w {
                        format!("{}..", &alert.description[..tag_w.saturating_sub(2)])
                    } else {
                        alert.description.clone()
                    };

                    lines.push(Line::from(vec![
                        Span::styled(format!("  {:<tw$}", "  --", tw = time_w),
                            span_style(Style::default().fg(color::TEXT_DIM))),
                        Span::styled(format!("  {:<tyw$}", "alert", tyw = type_w),
                            span_style(Style::default().fg(acolor).add_modifier(Modifier::BOLD))),
                        Span::styled(format!("  {:<jw$}", alert.severity.label(), jw = jid_w),
                            span_style(Style::default().fg(acolor))),
                        Span::styled(format!("  {:<tag_w$}", desc_disp, tag_w = tag_w),
                            span_style(Style::default().fg(color::TEXT))),
                    ]));
                }
            }
        }

        // Footer with total count
        lines.push(Line::from(Span::styled(
            format!("  \u{2500} {} items \u{2500}", total),
            Style::default().fg(Color::Rgb(35, 50, 80)),
        )));
    }

    // Scroll
    let content_h = (inner.height as usize).saturating_sub(4);
    let scroll = if sel > content_h / 2 {
        sel.saturating_sub(content_h / 2).min(lines.len().saturating_sub(content_h))
    } else {
        0
    };

    // Status: only the live filter buffer needs a row — shortcuts are in the border.
    let status = if app.minion_events_is_filtering {
        Some(Line::from(Span::styled(
            format!("  /{}_", app.minion_events_filter),
            Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
        )))
    } else {
        None
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(if status.is_some() { 1 } else { 0 })])
        .split(inner);

    f.render_widget(
        Paragraph::new(lines).scroll((scroll as u16, 0)).style(Style::default().bg(color::BG_POPUP)),
        chunks[0],
    );
    if let Some(status_line) = status {
        f.render_widget(
            Paragraph::new(status_line).style(Style::default().bg(color::BG_POPUP)),
            chunks[1],
        );
    }
}
