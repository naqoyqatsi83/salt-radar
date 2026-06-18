use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, DetailPopup};
use crate::types::SnifferState;

use super::color;

pub fn draw_detail_popup(f: &mut Frame, app: &App) {
    let Some(ref detail) = app.detail_popup else { return };

    match detail {
        DetailPopup::Minion(minion) => {
            let area = centered_rect(65, 50, f.area());
            f.render_widget(Clear, area);
            draw_minion_detail(f, area, minion);
        }
        DetailPopup::SaltEvent(evt) => {
            let area = centered_rect(75, 70, f.area());
            f.render_widget(Clear, area);
            draw_event_detail(f, area, evt);
        }
        DetailPopup::JobDetail { jid, lookup, selected_return, scroll_offset, scroll_mode, job_fun, job_tgt, job_start_epoch } => {
            let area = centered_rect(75, 75, f.area());
            f.render_widget(Clear, area);
            draw_job_detail(f, area, jid, lookup, *selected_return, *scroll_offset, *scroll_mode, job_fun, job_tgt, *job_start_epoch, app.clipboard_at);
        }
        DetailPopup::Alert(alert) => {
            let area = centered_rect(70, if alert.details.is_empty() { 40 } else { 70 }, f.area());
            f.render_widget(Clear, area);
            draw_alert_detail(f, area, alert, app.clipboard_at);
        }
        DetailPopup::AdminConfirm => {
            let area = centered_rect(55, 35, f.area());
            f.render_widget(Clear, area);
            draw_admin_confirm(f, area);
        }
        DetailPopup::EventSniffer(state) => {
            let area = centered_rect(80, 85, f.area());
            f.render_widget(Clear, area);
            draw_event_sniffer(f, area, state, app.clipboard_at, app.high_contrast);
        }
    }
}

pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let pad_v = (100u16.saturating_sub(percent_y)) / 2;
    let pad_h = (100u16.saturating_sub(percent_x)) / 2;
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(pad_v),
            Constraint::Percentage(percent_y),
            Constraint::Percentage(pad_v),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(pad_h),
            Constraint::Percentage(percent_x),
            Constraint::Percentage(pad_h),
        ])
        .split(vertical[1])[1]
}

fn header_lines(title: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            format!("  {}", title),
            Style::default()
                .fg(color::TEXT_TITLE)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
            Style::default().fg(Color::Rgb(35, 50, 80)),
        )),
        Line::from(""),
    ]
}

fn row(label: &'static str, value: String, value_color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {:<16}", label),
            Style::default().fg(color::TEXT_LABEL),
        ),
        Span::styled(value, Style::default().fg(value_color).add_modifier(Modifier::BOLD)),
    ])
}

fn dismiss_line(clipboard_at: Option<std::time::Instant>, scroll_mode: bool) -> Line<'static> {
    let show_copied = clipboard_at
        .map(|t| t.elapsed().as_secs_f64() < 3.0)
        .unwrap_or(false);
    if show_copied {
        return Line::from(Span::styled(
            "  \u{2713} Copied                                ",
            Style::default().fg(super::color::GREEN).add_modifier(Modifier::BOLD),
        ));
    }
    if scroll_mode {
        Line::from(Span::styled(
            "  [ q exit scroll | \u{2191}\u{2193} \u{21C4} page scroll output ]",
            Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
        ))
    } else {
        Line::from(Span::styled(
            "  [ q/Esc close | \u{2191}\u{2193} navigate | Enter=scroll | y=copy Y=json ]",
            Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
        ))
    }
}

fn render_popup(f: &mut Frame, area: Rect, lines: &[Line<'static>], footer: Option<Line<'static>>, scroll: u16) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER_HI))
        .style(Style::default().bg(color::BG_POPUP));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if let Some(footer_line) = footer {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        f.render_widget(
            Paragraph::new(lines.to_vec()).scroll((scroll, 0)).style(Style::default().bg(color::BG_POPUP)),
            chunks[0],
        );
        f.render_widget(
            Paragraph::new(footer_line).style(Style::default().bg(color::BG_POPUP)),
            chunks[1],
        );
    } else {
        f.render_widget(
            Paragraph::new(lines.to_vec()).scroll((scroll, 0)).style(Style::default().bg(color::BG_POPUP)),
            inner,
        );
    }
}

fn draw_minion_detail(f: &mut Frame, area: Rect, minion: &crate::types::MinionInfo) {
    let mut lines = header_lines(" Minion Detail ");
    lines.push(row("ID", minion.id.clone(), color::CYAN));
    let os_display = if !minion.osfinger.is_empty() {
        minion.osfinger.clone()
    } else if !minion.osrelease.is_empty() {
        format!("{} {}", minion.os, minion.osrelease)
    } else {
        minion.os.clone()
    };
    lines.push(row("OS", os_display, color::GREEN));
    if !minion.osfinger.is_empty() && !minion.osrelease.is_empty() {
        lines.push(row("OS Release", minion.osrelease.clone(), color::TEXT));
    }
    lines.push(row("Salt Version", minion.salt_version.clone(), color::YELLOW));
    lines.push(row("Arch", minion.cpuarch.clone(), color::TEXT));
    lines.push(row("Last Seen", format_epoch(minion.last_seen_epoch), color::TEXT_DIM));
    let footer = Line::from(Span::styled(
        "  [ Esc / q to close ]",
        Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
    ));
    render_popup(f, area, &lines, Some(footer), 0);
}

fn draw_event_detail(f: &mut Frame, area: Rect, evt: &crate::types::SaltEvent) {
    let mut lines = header_lines(" Event Detail ");
    lines.push(row("Tag", evt.tag.clone(), color::CYAN));
    lines.push(row("Timestamp", format_ts(evt.ts), color::TEXT_DIM));
    lines.push(Line::from(""));

    let json_str = serde_json::to_string_pretty(&evt.data).unwrap_or_default();
    for line in json_str.lines().take(20) {
        lines.push(Line::from(Span::styled(line.to_string(), Style::default().fg(color::TEXT))));
    }

    let footer = Line::from(Span::styled(
        "  [ Esc/q close | y=copy ]",
        Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
    ));
    render_popup(f, area, &lines, Some(footer), 0);
}

fn draw_job_detail(
    f: &mut Frame,
    area: Rect,
    jid: &str,
    lookup: &Option<crate::types::JidLookup>,
    selected_return: Option<usize>,
    scroll_offset: usize,
    scroll_mode: bool,
    job_fun: &str,
    job_tgt: &str,
    job_start_epoch: f64,
    clipboard_at: Option<std::time::Instant>,
) {
    let short_jid: String = jid.chars().take(28).collect();
    let mut lines = header_lines(&format!(" Job {}", short_jid));

    // Show job metadata from cached event data (not from lookup which may be empty)
    lines.push(Line::from(""));
    lines.push(row("Function", job_fun.to_string(), color::CYAN));
    lines.push(row("Target", job_tgt.to_string(), color::YELLOW));

    if job_start_epoch > 0.0 {
        use chrono::{DateTime, Local};
        if let Some(utc) = DateTime::from_timestamp(job_start_epoch as i64, 0) {
            let local: DateTime<Local> = utc.into();
            lines.push(row("Start (UTC)", utc.format("%Y-%m-%d %H:%M:%S").to_string(), color::TEXT_DIM));
            lines.push(row("Start (local)", local.format("%Y-%m-%d %H:%M:%S").to_string(), color::TEXT_DIM));
        }
    }

    if let Some(ref l) = lookup {
        lines.push(Line::from(Span::styled(
            format!("  Returns ({}):", l.returns.len()),
            Style::default().fg(color::TEXT_LABEL),
        )));

        let sel = selected_return.unwrap_or(0).min(l.returns.len().saturating_sub(1));

        if l.returns.is_empty() {
            lines.push(Line::from(Span::styled(
                "    No returns yet.",
                Style::default().fg(color::TEXT_DIM),
            )));
        } else {
            for (i, ret) in l.returns.iter().enumerate() {
                let cursor = if i == sel { "\u{25b6}" } else { " " };
                let status = if ret.success { "OK" } else { "FAIL" };
                let status_color = if ret.success { color::GREEN } else { color::RED };

                let summary_short = if ret.summary.len() > 50 {
                    format!("{}..", &ret.summary[..48])
                } else {
                    ret.summary.clone()
                };

                lines.push(Line::from(vec![
                    Span::styled(format!("  {} {}: ", cursor, ret.minion_id),
                        Style::default().fg(if i == sel { color::CYAN } else { color::TEXT }).add_modifier(if i == sel { Modifier::BOLD } else { Modifier::empty() })),
                    Span::styled(status, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
                    Span::styled(format!(" (retcode={}) ", ret.retcode), Style::default().fg(color::TEXT_DIM)),
                    Span::styled(summary_short, Style::default().fg(color::TEXT_DIM)),
                ]));
            }

            // Show full return data for selected minion
            if sel < l.returns.len() {
                let ret = &l.returns[sel];
                let display_str = if !ret.formatted_text.is_empty() {
                    // Salt's own outputter already includes the ----------
                    // separator at top — no header needed
                    ret.formatted_text.clone()
                } else {
                    // Fallback for non-state returns
                    App::format_return_as_console(&ret.minion_id, &ret.full_return)
                };
                let all_lines: Vec<&str> = display_str.lines().collect();
                if scroll_mode {
                    for line in all_lines.iter().skip(scroll_offset) {
                        lines.push(Line::from(Span::styled(line.to_string(), Style::default().fg(color::TEXT))));
                    }
                } else {
                    for line in all_lines.iter().take(30) {
                        lines.push(Line::from(Span::styled(line.to_string(), Style::default().fg(color::TEXT))));
                    }
                }
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  Querying Salt master for job data...",
            Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
        )));
    }

    lines.push(Line::from(""));
    let footer = dismiss_line(clipboard_at, scroll_mode);
    render_popup(f, area, &lines, Some(footer), 0);
}

fn draw_alert_detail(f: &mut Frame, area: Rect, alert: &crate::types::AlertEntry, clipboard_at: Option<std::time::Instant>) {
    let sev_color = match alert.severity {
        crate::types::AlertSeverity::Info => color::CYAN,
        crate::types::AlertSeverity::Warn => color::YELLOW,
        crate::types::AlertSeverity::Crit => color::RED,
    };
    let mut lines = header_lines(" Alert Detail ");
    lines.push(row("Severity", alert.severity.label().to_string(), sev_color));
    lines.push(row("Rule", alert.rule.to_string(), color::CYAN));
    lines.push(row("Description", alert.description.clone(), color::TEXT_HI));
    lines.push(Line::from(""));

    if !alert.details.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  Items ({}):", alert.details.len()),
            Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD),
        )));
        for detail in &alert.details {
            lines.push(Line::from(Span::styled(
                format!("    {}", detail),
                Style::default().fg(color::TEXT),
            )));
        }
    }

    lines.push(Line::from(""));
    let show_copied = clipboard_at
        .map(|t| t.elapsed().as_secs_f64() < 3.0)
        .unwrap_or(false);
    let footer = if show_copied {
        Line::from(Span::styled(
            "  \u{2713} Copied                                ",
            Style::default().fg(super::color::GREEN).add_modifier(Modifier::BOLD),
        ))
    } else {
        Line::from(Span::styled(
            "  [ Esc/q close | y=copy ]",
            Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
        ))
    };
    render_popup(f, area, &lines, Some(footer), 0);
}

fn draw_admin_confirm(f: &mut Frame, area: Rect) {
    let mut lines = vec![
        Line::from(Span::styled(
            "  \u{26a0} DANGER ZONE ",
            Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  ADMIN MODE",
            Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  You are about to enable admin mode.",
            Style::default().fg(color::TEXT_HI),
        )),
        Line::from(Span::styled(
            "  Actions performed in this mode may have",
            Style::default().fg(color::TEXT),
        )),
        Line::from(Span::styled(
            "  consequences on your Salt infrastructure.",
            Style::default().fg(color::TEXT),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Only proceed if you know what you are doing.",
            Style::default().fg(color::ORANGE).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    // Simulated buttons
    lines.push(Line::from(vec![
        Span::styled("     [ ", Style::default().fg(color::TEXT_DIM)),
        Span::styled("OK", Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(" ] ", Style::default().fg(color::TEXT_DIM)),
        Span::styled("Enter", Style::default().fg(color::TEXT_DIM).add_modifier(Modifier::ITALIC)),
        Span::styled("          ", Style::default().fg(color::TEXT_DIM)),
        Span::styled("[ ", Style::default().fg(color::TEXT_DIM)),
        Span::styled("Cancel", Style::default().fg(color::RED).add_modifier(Modifier::BOLD)),
        Span::styled(" ] ", Style::default().fg(color::TEXT_DIM)),
        Span::styled("Esc/q/a", Style::default().fg(color::TEXT_DIM).add_modifier(Modifier::ITALIC)),
    ]));
    render_popup(f, area, &lines, None, 0);
}

fn draw_event_sniffer(
    f: &mut Frame,
    area: Rect,
    state: &SnifferState,
    clipboard_at: Option<std::time::Instant>,
    high_contrast: bool,
) {
    let ids_str = state.minion_ids.join(", ");
    let title = if ids_str.len() > 50 {
        format!(" Event Sniffer: {}... ", &ids_str[..47])
    } else {
        format!(" Event Sniffer: {} ", ids_str)
    };

    let mut lines = header_lines(&title);

    // Status bar
    let (status_label, status_color) = if state.paused {
        ("PAUSED", color::YELLOW)
    } else {
        ("LIVE", color::GREEN)
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!("  Status: {}", status_label),
            Style::default().fg(status_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  Captured: {} / max 1000", state.buffer.len()),
            Style::default().fg(color::TEXT_DIM),
        ),
    ]));
    lines.push(Line::from(""));

    if state.buffer.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No matching events yet. Waiting for new events...",
            Style::default().fg(color::TEXT_DIM),
        )));
    } else {
        // Column widths: # | Time | JID | Type
        let num_w: usize = 8;
        let time_w: usize = 22;
        let type_w: usize = 18;
        let jid_w = (area.width as usize).saturating_sub(num_w + time_w + type_w + 4);

        // Column header
        lines.push(Line::from(vec![
            Span::styled(format!("  {:>nw$} ", "#", nw = num_w - 3), Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:<tw$}", "Time", tw = time_w), Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:<jw$}", "JID", jw = jid_w), Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:<tyw$}", "Type", tyw = type_w), Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(Span::styled(
            "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
            Style::default().fg(Color::Rgb(35, 50, 80)),
        )));

        let total = state.buffer.len();
        let sel = state.selected.min(total.saturating_sub(1));
        let mut event_line_starts: Vec<usize> = Vec::with_capacity(total);

        for display_idx in 0..total {
            let buf_idx = total - 1 - display_idx;
            let evt = match state.buffer.get(buf_idx) {
                Some(e) => e,
                None => break,
            };

            event_line_starts.push(lines.len());

            let is_sel = display_idx == sel;
            let sel_style = if is_sel {
                color::select_bg(high_contrast)
            } else {
                Style::default()
            };

            let event_num = display_idx + 1;
            let t = sniffer_format_ts(evt.ts);
            let etype = evt.data["fun"]
                .as_str()
                .or_else(|| {
                    if evt.tag.contains("/ret/") { Some("return") } else { None }
                })
                .unwrap_or("");

            let tag_disp = if evt.tag.len() > jid_w {
                format!("{}..", &evt.tag[..jid_w.saturating_sub(2)])
            } else {
                evt.tag.clone()
            };

            // Header row: # | Time | JID | Type
            lines.push(Line::from(vec![
                Span::styled(format!("  {:>nw$} ", event_num, nw = num_w - 3), Style::default().fg(color::TEXT_DIM)),
                Span::styled(format!("{:<tw$}", t, tw = time_w), Style::default().fg(color::TEXT_DIM)),
                Span::styled(format!("{:<jw$}", tag_disp, jw = jid_w), Style::default().fg(color::CYAN)),
                Span::styled(format!("{:<tyw$}", etype, tyw = type_w), Style::default().fg(color::GREEN)),
            ]).style(sel_style));

            // Full JSON body (no truncation)
            let json_str = serde_json::to_string_pretty(&evt.data).unwrap_or_default();
            let indent = format!("{:w$}", "", w = num_w + time_w + 3);
            for line in json_str.lines() {
                lines.push(Line::from(
                    Span::styled(
                        format!("{}{}", indent, line),
                        Style::default().fg(color::TEXT),
                    ),
                ).style(sel_style));
            }
        }

        // Auto-scroll: keep selected event header visible (at ~1/3 from top)
        let content_height = (area.height as usize).saturating_sub(3); // borders(2) + footer(1)
        let scroll = if sel < event_line_starts.len() {
            let sel_line = event_line_starts[sel];
            let max_scroll = lines.len().saturating_sub(content_height);
            sel_line.saturating_sub(content_height / 3).min(max_scroll) as u16
        } else {
            0
        };

        let footer = sniffer_dismiss_line(clipboard_at, state.paused);
        render_popup(f, area, &lines, Some(footer), scroll);
        return;
    }

    lines.push(Line::from(""));
    let footer = sniffer_dismiss_line(clipboard_at, state.paused);
    render_popup(f, area, &lines, Some(footer), 0);
}

fn sniffer_format_ts(ts: f64) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "--:--:--".to_string())
}

fn sniffer_dismiss_line(
    clipboard_at: Option<std::time::Instant>,
    paused: bool,
) -> Line<'static> {
    let show_copied = clipboard_at
        .map(|t| t.elapsed().as_secs_f64() < 3.0)
        .unwrap_or(false);
    if show_copied {
        return Line::from(Span::styled(
            "  \u{2713} Copied                                ",
            Style::default().fg(super::color::GREEN).add_modifier(Modifier::BOLD),
        ));
    }
    let hint = if paused {
        "  [ q/Esc close | \u{2191}\u{2193} | Enter=detail | p=resume | y=copy ]"
    } else {
        "  [ q/Esc close | \u{2191}\u{2193} | Enter=detail | p=pause | y=copy ]"
    };
    Line::from(Span::styled(
        hint,
        Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
    ))
}

fn format_epoch(secs: u64) -> String {
    if secs == 0 {
        "unknown".to_string()
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let ago = now.saturating_sub(secs);
        if ago < 60 {
            format!("{}s ago", ago)
        } else if ago < 3600 {
            format!("{}m ago", ago / 60)
        } else if ago < 86400 {
            format!("{}h ago", ago / 3600)
        } else {
            format!("{}d ago", ago / 86400)
        }
    }
}

fn format_ts(ts: f64) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|dt| dt.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "--:--:--".to_string())
}
