use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, DetailPopup};

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
            let area = centered_rect(60, 40, f.area());
            f.render_widget(Clear, area);
            draw_alert_detail(f, area, alert);
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

fn render_popup(f: &mut Frame, area: Rect, lines: &[Line<'static>]) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER_HI))
        .style(Style::default().bg(color::BG_POPUP));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(
        Paragraph::new(lines.to_vec()).style(Style::default().bg(color::BG_POPUP)),
        inner,
    );
}

fn draw_minion_detail(f: &mut Frame, area: Rect, minion: &crate::types::MinionInfo) {
    let mut lines = header_lines(" Minion Detail ");
    lines.push(row("ID", minion.id.clone(), color::CYAN));
    lines.push(row("OS", format!("{} {}", minion.os, minion.osrelease), color::GREEN));
    lines.push(row("Salt Version", minion.salt_version.clone(), color::YELLOW));
    lines.push(row("Arch", minion.cpuarch.clone(), color::TEXT));
    lines.push(row("Last Seen", format_epoch(minion.last_seen_epoch), color::TEXT_DIM));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [ Esc / q to close ]",
        Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
    )));
    render_popup(f, area, &lines);
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

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [ Esc / q to close ]",
        Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
    )));
    render_popup(f, area, &lines);
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

            // Show full cli_output from salt-run
            if !l.cli_output.is_empty() {
                let all_lines: Vec<&str> = l.cli_output.lines().collect();
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
    lines.push(dismiss_line(clipboard_at, scroll_mode));
    render_popup(f, area, &lines);
}

fn draw_alert_detail(f: &mut Frame, area: Rect, alert: &crate::types::AlertEntry) {
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
    lines.push(Line::from(Span::styled(
        "  [ Esc / q to close ]",
        Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
    )));
    render_popup(f, area, &lines);
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
