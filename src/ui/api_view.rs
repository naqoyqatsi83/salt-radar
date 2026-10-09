use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::types::{ApiLogEntry, Column, HeaderHit, Tab, TableId};

use super::color;
use super::table::{cell, column_widths, header_line};

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
pub fn draw_api(f: &mut Frame, area: Rect, app: &App, tab_focused: bool, hits: &mut Vec<HeaderHit>) {
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

    draw_request_log(f, layout[0], app, tab_focused, hits);
    draw_endpoint_breakdown(f, layout[1], app);
}

fn draw_unavailable(f: &mut Frame, area: Rect, app: &App) {
    let authed = app.config.api_stats_auth_mode != "none";
    let hint = if !app.api_stats.configured {
        "rest_cherrypy is not configured on this master.".to_string()
    } else {
        format!(
            "{}:{} is not answering /stats.",
            app.api_stats.scheme, app.api_stats.port,
        )
    };
    let mut lines = vec![
        Line::from(Span::styled(
            "  salt-api request stats aren't available.",
            Style::default().fg(color::inactive_fg()).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(format!("  {}", hint), Style::default().fg(color::inactive_fg()))),
    ];
    if !app.api_stats.error.is_empty() {
        lines.push(Line::from(Span::styled(format!("  error: {}", app.api_stats.error), Style::default().fg(color::red()))));
    }
    lines.push(Line::from(""));
    if authed {
        lines.push(Line::from(Span::styled(
            "  Set the following in the master config and restart salt-api:",
            Style::default().fg(color::inactive_fg()),
        )));
        lines.push(Line::from(Span::styled(
            "    rest_cherrypy: {collect_stats: True}",
            Style::default().fg(color::cyan()),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  Authenticating via stats_auth_mode = {:?} (Config Editor, 'F3') —", app.config.api_stats_auth_mode),
            Style::default().fg(color::inactive_fg()),
        )));
        lines.push(Line::from(Span::styled(
            "  check the error above and the eauth/credentials/pillar settings there.",
            Style::default().fg(color::inactive_fg()),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  Set the following in the master config and restart salt-api:",
            Style::default().fg(color::inactive_fg()),
        )));
        lines.push(Line::from(Span::styled(
            "    rest_cherrypy: {collect_stats: True, stats_disable_auth: True}",
            Style::default().fg(color::cyan()),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  See README for why stats_disable_auth is required (or set",
            Style::default().fg(color::inactive_fg()),
        )));
        lines.push(Line::from(Span::styled(
            "  stats_auth_mode instead of stats_disable_auth — Config Editor, 'F3').",
            Style::default().fg(color::inactive_fg()),
        )));
    }

    let block = super::list_block_hinted(" API ", Style::default().fg(color::border()), color::main_bg(), false, false, &[]);
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::main_bg())).wrap(Wrap { trim: false }),
        inner,
    );
}

fn request_cells(entry: &ApiLogEntry, col: Column, w: usize) -> Span<'static> {
    let right = col.right_aligned();
    match col {
        Column::Age => cell(
            &format!("{} ago", super::dashboard::format_duration(entry.received_at.elapsed().as_secs())),
            w, right, Style::default().fg(color::inactive_fg()),
        ),
        Column::Method => cell(&entry.method, w, right, Style::default().fg(color::purple())),
        Column::Client => cell(&entry.client, w, right, Style::default().fg(color::inactive_fg())),
        Column::Path => cell(&entry.path, w, right, Style::default().fg(color::cyan())),
        Column::Status => {
            let status_code = entry.response_status.split_whitespace().next().unwrap_or("?");
            let status_color = match status_code.chars().next() {
                Some('2') | Some('3') => color::green(),
                Some('4') | Some('5') => color::red(),
                _ => color::inactive_fg(),
            };
            cell(&entry.response_status, w, right, Style::default().fg(status_color))
        }
        Column::Time => cell(&format!("{:.2}s", entry.processing_time_secs), w, right, Style::default().fg(color::inactive_fg())),
        _ => cell("", w, right, Style::default()),
    }
}

fn draw_request_log(f: &mut Frame, area: Rect, app: &App, tab_focused: bool, hits: &mut Vec<HeaderHit>) {
    let border_style = if tab_focused {
        Style::default().fg(color::border_focus())
    } else {
        Style::default().fg(color::border())
    };

    // Filtered and sorted once per frame, not once per visible row.
    let filtered = app.filtered_api_log();
    let total = filtered.len();
    let selected = app.api_log_selected.min(total.saturating_sub(1));
    let visible = (area.height as usize).saturating_sub(2).saturating_sub(2);
    let (scroll, has_more_above, has_more_below) = super::scroll_window(selected, visible, total);

    let stats_poll_hint = if app.config.api_hide_stats_polls { "show /stats" } else { "hide /stats" };
    let mut hints: Vec<(&str, String)> = vec![
        ("/", "filter".to_string()),
        ("Enter", "detail".to_string()),
        ("x", stats_poll_hint.to_string()),
        ("PgUp/PgDn", "page".to_string()),
        ("Home/End", "top/bottom".to_string()),
    ];
    hints.extend(super::table::hints(TableId::ApiLog));
    let title = if app.config.api_hide_stats_polls {
        format!(" API Requests ({} captured, /stats hidden) ", app.api_requests_log.len())
    } else {
        format!(" API Requests ({} captured) ", app.api_requests_log.len())
    };
    let block = super::list_block_hinted(
        title,
        border_style, color::main_bg(), has_more_above, has_more_below, &hints,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    if app.is_filtering {
        let cursor = if app.filter_text.len().is_multiple_of(2) { '|' } else { ' ' };
        lines.push(Line::from(vec![
            Span::styled(format!(" /{} {}", app.filter_text, cursor), Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {} items", total), Style::default().fg(color::inactive_fg())),
        ]));
    } else if !app.filter_text_for(Tab::Api).is_empty() {
        lines.push(Line::from(vec![
            Span::styled(format!(" /{}/ ", app.filter_text_for(Tab::Api)), Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{} items", total), Style::default().fg(color::inactive_fg())),
        ]));
    }

    if app.api_requests_log.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No requests captured yet.",
            Style::default().fg(color::inactive_fg()),
        )));
    } else {
        let cols = app.columns(TableId::ApiLog);
        let widths = column_widths(&cols, (inner.width as usize).saturating_sub(2));
        lines.push(header_line(TableId::ApiLog, &cols, &widths, app.sort_of(TableId::ApiLog), inner.x, inner.y + lines.len() as u16, hits));

        if total == 0 {
            lines.push(Line::from(Span::styled(
                "  No requests match the filter.",
                Style::default().fg(color::inactive_fg()),
            )));
        } else {
            for (idx, entry) in filtered.iter().enumerate().skip(scroll).take(visible) {
                let mut row = vec![Span::raw("  ")];
                for (&col, &w) in cols.iter().zip(&widths) {
                    row.push(request_cells(entry, col, w));
                }
                let line = Line::from(row);
                lines.push(if idx == selected { color::selected_row(line) } else { line });
            }
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::main_bg())),
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
    let block = super::list_block_hinted(title, Style::default().fg(color::border()), color::main_bg(), false, false, &[]);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(" current:", Style::default().fg(color::label_fg())),
        Span::styled(stats.current_requests.to_string(), Style::default().fg(color::cyan()).add_modifier(Modifier::BOLD)),
        Span::styled(" total:", Style::default().fg(color::label_fg())),
        Span::styled(stats.total_requests.to_string(), Style::default().fg(color::inactive_fg())),
    ]));
    lines.push(Line::from(""));

    if breakdown.is_empty() {
        lines.push(Line::from(Span::styled("  No requests captured yet.", Style::default().fg(color::inactive_fg()))));
    } else {
        lines.push(Line::from(Span::styled(
            format!(" {:<7}{:<20}{:<6}{:<7}{:<7}{}", "Method", "Path", "Cnt", "Avg", "p95", "Err%"),
            Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD),
        )));
        let max_rows = (inner.height as usize).saturating_sub(3);
        for ep in breakdown.iter().take(max_rows) {
            let path_str = truncate(&ep.path, 20);
            let err_color = if ep.error_pct >= 10.0 { color::red() } else if ep.error_pct > 0.0 { color::yellow() } else { color::green() };
            lines.push(Line::from(vec![
                Span::styled(format!(" {:<7}", ep.method), Style::default().fg(color::purple())),
                Span::styled(format!("{:<20}", path_str), Style::default().fg(color::cyan())),
                Span::styled(format!("{:<6}", ep.count), Style::default().fg(color::inactive_fg())),
                Span::styled(format!("{:<7}", format!("{:.2}s", ep.avg_secs)), Style::default().fg(color::inactive_fg())),
                Span::styled(format!("{:<7}", format!("{:.2}s", ep.p95_secs)), Style::default().fg(color::inactive_fg())),
                Span::styled(format!("{:.0}%", ep.error_pct), Style::default().fg(err_color)),
            ]));
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::main_bg())),
        inner,
    );
}
