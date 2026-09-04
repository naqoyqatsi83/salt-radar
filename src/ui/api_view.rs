use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;

use super::color;

/// Truncates to at most `max` chars, appending ".." when it does (so the
/// returned string is always <= max chars) -- every fixed-width column
/// below relies on this to guarantee it never overflows into the next one.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let keep = max.saturating_sub(2);
        format!("{}..", s.chars().take(keep).collect::<String>())
    }
}

/// salt-api request log + per-endpoint breakdown. Only meaningful once
/// rest_cherrypy's `collect_stats`/`stats_disable_auth` are set on the
/// master (see the Dashboard's salt-api panel and README) — this tab shows
/// the same "not reachable" guidance full-width when that isn't the case,
/// since there's nothing else to show here without it.
pub fn draw_api(f: &mut Frame, area: Rect, app: &App, tab_focused: bool) {
    if !app.api_stats.reachable {
        draw_unavailable(f, area, app);
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(65),
            Constraint::Percentage(35),
        ])
        .split(area);

    draw_request_log(f, layout[0], app, tab_focused);
    draw_endpoint_breakdown(f, layout[1], app);
}

fn draw_unavailable(f: &mut Frame, area: Rect, app: &App) {
    let hint = if !app.api_stats.configured {
        "rest_cherrypy is not configured on this master.".to_string()
    } else {
        format!(
            "{}:{} is not answering /stats.",
            app.api_stats.scheme, app.api_stats.port,
        )
    };
    let lines = vec![
        Line::from(Span::styled(
            "  salt-api request stats aren't available.",
            Style::default().fg(color::TEXT_DIM).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(format!("  {}", hint), Style::default().fg(color::TEXT_DIM))),
        Line::from(""),
        Line::from(Span::styled(
            "  Set the following in the master config and restart salt-api:",
            Style::default().fg(color::TEXT_DIM),
        )),
        Line::from(Span::styled(
            "    rest_cherrypy: {collect_stats: True, stats_disable_auth: True}",
            Style::default().fg(color::CYAN),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  See README for why stats_disable_auth is required (this agent",
            Style::default().fg(color::TEXT_DIM),
        )),
        Line::from(Span::styled(
            "  never handles or stores salt-api auth tokens).",
            Style::default().fg(color::TEXT_DIM),
        )),
    ];

    let block = super::list_block_hinted(" API ", Style::default().fg(color::BORDER), color::BG_DARK, false, false, &[]);
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)).wrap(Wrap { trim: false }),
        inner,
    );
}

fn draw_request_log(f: &mut Frame, area: Rect, app: &App, tab_focused: bool) {
    let border_style = if tab_focused {
        Style::default().fg(color::BORDER_HI)
    } else {
        Style::default().fg(color::BORDER)
    };

    let total = app.filtered_api_log_len();
    let selected = app.api_log_selected.min(total.saturating_sub(1));
    let visible = (area.height as usize).saturating_sub(2).saturating_sub(2);
    let (scroll, has_more_above, has_more_below) = super::scroll_window(selected, visible, total);

    let hints: &[(&str, String)] = &[
        ("/", "filter".to_string()),
        ("Enter", "detail".to_string()),
        ("PgUp/PgDn", "page".to_string()),
        ("Home/End", "top/bottom".to_string()),
    ];
    let block = super::list_block_hinted(
        format!(" API Requests ({} captured) ", app.api_requests_log.len()),
        border_style, color::BG_DARK, has_more_above, has_more_below, hints,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    if app.is_filtering {
        let cursor = if app.filter_text.len() % 2 == 0 { '|' } else { ' ' };
        lines.push(Line::from(vec![
            Span::styled(format!(" /{} {}", app.filter_text, cursor), Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {} items", total), Style::default().fg(color::TEXT_DIM)),
        ]));
    } else if !app.filter_text.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(format!(" /{}/ ", app.filter_text), Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{} items", total), Style::default().fg(color::TEXT_DIM)),
        ]));
    }

    if app.api_requests_log.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No requests captured yet.",
            Style::default().fg(color::TEXT_DIM),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!("  {:<16}{:<7}{:<21}{:<20}{:<18}{}", "Age", "Method", "Client", "Path", "Status", "Time"),
            Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD),
        )));

        if total == 0 {
            lines.push(Line::from(Span::styled(
                "  No requests match the filter.",
                Style::default().fg(color::TEXT_DIM),
            )));
        } else {
            for idx in scroll..(scroll + visible).min(total) {
                let entry = match app.get_filtered_api_log(idx) {
                    Some(e) => e,
                    None => break,
                };

                let is_selected = idx == selected;
                let sel_style = if is_selected {
                    color::select_bg(app.high_contrast)
                } else {
                    Style::default().bg(color::BG_DARK)
                };

                // API_LOG_MAX_AGE prunes entries past 1h once per poll, so
                // this is normally well under that -- but truncate anyway
                // rather than assume the prune always wins the race, since
                // an un-truncated field here would silently misalign every
                // column after it (the exact bug class already fixed
                // elsewhere in this app for Event Types/Fleet Overview).
                let age_raw = format!("{} ago", super::dashboard::format_duration(entry.received_at.elapsed().as_secs()));
                let age = truncate(&age_raw, 16);
                let client_str = truncate(&entry.client, 21);
                let path_str = truncate(&entry.path, 20);
                let status_str = truncate(&entry.response_status, 18);
                let status_code = entry.response_status.split_whitespace().next().unwrap_or("?");
                let status_color = match status_code.chars().next() {
                    Some('2') | Some('3') => color::GREEN,
                    Some('4') | Some('5') => color::RED,
                    _ => color::TEXT_DIM,
                };

                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<16}", age), Style::default().fg(color::TEXT_DIM)),
                    Span::styled(format!("{:<7}", entry.method), Style::default().fg(color::PURPLE)),
                    Span::styled(format!("{:<21}", client_str), Style::default().fg(color::TEXT_DIM)),
                    Span::styled(format!("{:<20}", path_str), Style::default().fg(color::CYAN)),
                    Span::styled(format!("{:<18}", status_str), Style::default().fg(status_color)),
                    Span::styled(format!("{:.2}s", entry.processing_time_secs), Style::default().fg(color::TEXT_DIM)),
                ]).style(sel_style));
            }
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}

fn draw_endpoint_breakdown(f: &mut Frame, area: Rect, app: &App) {
    let stats = app.api_stats.clone();
    let breakdown = app.api_endpoint_breakdown();

    let title = format!(
        " Endpoints ({:.1} req/s, {} errors) ",
        app.api_requests_per_sec_recent,
        app.api_error_pct().map(|p| format!("{:.0}%", p)).unwrap_or_else(|| "--".into()),
    );
    let block = super::list_block_hinted(title, Style::default().fg(color::BORDER), color::BG_DARK, false, false, &[]);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(" current:", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(stats.current_requests.to_string(), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)),
        Span::styled(" total:", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(stats.total_requests.to_string(), Style::default().fg(color::TEXT_DIM)),
    ]));
    lines.push(Line::from(""));

    if breakdown.is_empty() {
        lines.push(Line::from(Span::styled("  No requests captured yet.", Style::default().fg(color::TEXT_DIM))));
    } else {
        lines.push(Line::from(Span::styled(
            format!(" {:<7}{:<20}{:<6}{:<7}{:<7}{}", "Method", "Path", "Cnt", "Avg", "p95", "Err%"),
            Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD),
        )));
        let max_rows = (inner.height as usize).saturating_sub(3);
        for ep in breakdown.iter().take(max_rows) {
            let path_str = truncate(&ep.path, 20);
            let err_color = if ep.error_pct >= 10.0 { color::RED } else if ep.error_pct > 0.0 { color::YELLOW } else { color::GREEN };
            lines.push(Line::from(vec![
                Span::styled(format!(" {:<7}", ep.method), Style::default().fg(color::PURPLE)),
                Span::styled(format!("{:<20}", path_str), Style::default().fg(color::CYAN)),
                Span::styled(format!("{:<6}", ep.count), Style::default().fg(color::TEXT_DIM)),
                Span::styled(format!("{:<7}", format!("{:.2}s", ep.avg_secs)), Style::default().fg(color::TEXT_DIM)),
                Span::styled(format!("{:<7}", format!("{:.2}s", ep.p95_secs)), Style::default().fg(color::TEXT_DIM)),
                Span::styled(format!("{:.0}%", ep.error_pct), Style::default().fg(err_color)),
            ]));
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}
