use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::types::Tab;

use super::color;

pub fn draw_jobs(f: &mut Frame, area: Rect, app: &App, tab_focused: bool) {
    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(55),
            Constraint::Percentage(45),
        ])
        .split(area);

    draw_live_jobs_panel(f, layout[0], app, tab_focused);
    draw_cache_panel(f, layout[1], app, tab_focused);
}

fn draw_live_jobs_panel(f: &mut Frame, area: Rect, app: &App, tab_focused: bool) {
    let focused = tab_focused && !app.cache_focus;
    let border_style = if focused {
        Style::default().fg(color::BORDER_HI)
    } else {
        Style::default().fg(color::BORDER)
    };

    let total = app.filtered_jobs_len();
    let selected = app.job_selected.min(total.saturating_sub(1));
    let visible = (area.height as usize).saturating_sub(2).saturating_sub(2);
    let (scroll, has_more_above, has_more_below) = super::scroll_window(selected, visible, total);

    let mut hints: Vec<(&str, String)> = vec![
        ("/", "filter".to_string()),
        ("Enter", "detail".to_string()),
        ("PgUp/PgDn", "page".to_string()),
        ("Home/End", "top/bottom".to_string()),
    ];
    if focused {
        hints.push(("\u{2192}", "cache".to_string()));
    }
    let block = super::list_block_hinted(format!(" Live Jobs ({}) ", app.jobs.len()), border_style, color::BG_DARK, has_more_above, has_more_below, &hints);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // Filter bar
    if app.is_filtering {
        let cursor = if app.filter_text.len() % 2 == 0 { '|' } else { ' ' };
        lines.push(Line::from(vec![
            Span::styled(format!(" /{} {}", app.filter_text, cursor), Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {} items", total), Style::default().fg(color::TEXT_DIM)),
        ]));
    } else if !app.filter_text_for(Tab::Jobs).is_empty() {
        lines.push(Line::from(vec![
            Span::styled(format!(" /{}/ ", app.filter_text_for(Tab::Jobs)), Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{} items", total), Style::default().fg(color::TEXT_DIM)),
        ]));
    }

    if app.jobs.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No active jobs.",
            Style::default().fg(color::TEXT_DIM),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!("  {:<24}{:<26}{:<22}{:<8}{:>6}{:>6}{:>6}{:>6}", "JID", "Fun", "Target", "Source", "Tgt", "OK", "Fail", "Wait"),
            Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD),
        )));

        if total == 0 {
            lines.push(Line::from(Span::styled(
                "  No jobs match the filter.",
                Style::default().fg(color::TEXT_DIM),
            )));
        } else {
            for idx in scroll..(scroll + visible).min(total) {
                let job = match app.get_filtered_job(idx) {
                    Some(j) => j,
                    None => break,
                };

                let is_selected = idx == selected;
                let sel_style = if is_selected {
                    color::select_bg(app.high_contrast)
                } else {
                    Style::default().bg(color::BG_DARK)
                };

                let jid_str = if job.jid.len() > 24 { format!("{}..", &job.jid[..22]) } else { job.jid.clone() };
                let fun_str = if job.fun.len() > 26 { format!("{}..", &job.fun[..24]) } else { job.fun.clone() };
                let tgt_str = if job.tgt.len() > 22 { format!("{}..", &job.tgt[..20]) } else { job.tgt.clone() };
                let source_str = if job.local { "local" } else { "master" };

                let mut row = vec![
                    Span::styled(format!("  {:<24}", jid_str), Style::default().fg(color::TEXT_DIM)),
                    Span::styled(format!("{:<26}", fun_str), Style::default().fg(color::CYAN)),
                    Span::styled(format!("{:<22}", tgt_str), Style::default().fg(color::YELLOW)),
                    Span::styled(format!("{:<8}", source_str), Style::default().fg(if job.local { color::PURPLE } else { color::GREEN })),
                ];
                // Four spelled-out, fixed-width columns (matching the Tgt/OK/
                // Fail/Wait header above) instead of a bare returned/target
                // count or inline symbols -- a real sub-table needs no
                // legend, every column lines up top to bottom regardless of
                // how large the target count or how mixed the outcome is,
                // and "how many were targeted" is answered directly instead
                // of requiring the reader to add up succeeded+failed+wait.
                if job.target_count == 0 {
                    row.push(Span::styled(format!("{:>24}", "?"), Style::default().fg(color::ORANGE).add_modifier(Modifier::BOLD)));
                } else {
                    let pending = job.target_count.saturating_sub(job.returned);
                    let ok_style = if job.succeeded > 0 { Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD) } else { Style::default().fg(color::TEXT_DIM) };
                    let fail_style = if job.failed > 0 { Style::default().fg(color::RED).add_modifier(Modifier::BOLD) } else { Style::default().fg(color::TEXT_DIM) };
                    let pend_style = if pending > 0 { Style::default().fg(color::ORANGE) } else { Style::default().fg(color::TEXT_DIM) };
                    row.push(Span::styled(format!("{:>6}", job.target_count), Style::default().fg(color::TEXT)));
                    row.push(Span::styled(format!("{:>6}", job.succeeded), ok_style));
                    row.push(Span::styled(format!("{:>6}", job.failed), fail_style));
                    row.push(Span::styled(format!("{:>6}", pending), pend_style));
                }

                lines.push(Line::from(row).style(sel_style));
            }
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}

fn draw_cache_panel(f: &mut Frame, area: Rect, app: &App, tab_focused: bool) {
    let focused = tab_focused && app.cache_focus;
    let border_style = if focused {
        Style::default().fg(color::BORDER_HI)
    } else {
        Style::default().fg(color::BORDER)
    };

    let total_hint = if app.cache_loaded {
        let page = if app.cache_total_count > 0 {
            (app.cache_offset / app.cache_max_results) + 1
        } else {
            0
        };
        format!("{} found (pg {}, limit {})", app.cache_total_count, page, app.cache_max_results)
    } else if app.cache_loading {
        match app.cache_load_progress {
            Some(loaded) => format!("loading... {}", loaded),
            None => "loading...".to_string(),
        }
    } else {
        format!("{}", app.jobs_cache.len())
    };
    let total = app.filtered_cache_len();
    let selected = app.cache_selected.min(total.saturating_sub(1));
    let visible = (area.height as usize).saturating_sub(2).saturating_sub(2);
    let (scroll, has_more_above, has_more_below) = super::scroll_window(selected, visible, total);

    let mut hints: Vec<(&str, String)> = vec![
        ("/", "filter".to_string()),
        ("Enter", "detail".to_string()),
        ("PgUp/PgDn", "scroll".to_string()),
        ("Home/End", "top/bottom".to_string()),
        ("[ ]", "limit".to_string()),
        (", .", "page".to_string()),
        ("r", "refresh".to_string()),
    ];
    if focused {
        hints.push(("\u{2190}", "live".to_string()));
    }
    let block = super::list_block_hinted(format!(" Job Cache ({}) ", total_hint), border_style, color::BG_DARK, has_more_above, has_more_below, &hints);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // Loading state — a live "N jobs read" count from the agent's
    // incremental progress messages, where available (the fast local_cache
    // walk reports it; the salt-run fallback for other job-cache backends
    // doesn't, so this just falls back to a plain "Loading..." line).
    if app.cache_loading && !app.cache_loaded {
        let elapsed = app.cache_loading_at.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0);
        let mut spans = vec![Span::styled("  Loading job cache... ", Style::default().fg(color::TEXT_DIM))];
        if let Some(loaded) = app.cache_load_progress {
            spans.push(Span::styled(format!("{}", loaded), Style::default().fg(color::TEXT_TITLE).add_modifier(Modifier::BOLD)));
            spans.push(Span::styled(" jobs read", Style::default().fg(color::TEXT_DIM)));
        }
        spans.push(Span::styled(format!(" ({:.0}s)", elapsed), Style::default().fg(color::TEXT_DIM)));
        lines.push(Line::from(spans));
        f.render_widget(
            Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
            inner,
        );
        return;
    }

    // Error state
    if let Some(ref err) = app.cache_load_error {
        lines.push(Line::from(Span::styled(
            format!("  Error: {}", err),
            Style::default().fg(color::RED),
        )));
        f.render_widget(
            Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
            inner,
        );
        return;
    }

    // Filter bar with total_count context
    if app.cache_is_filtering {
        let cursor = if app.cache_filter_text.len() % 2 == 0 { '|' } else { ' ' };
        lines.push(Line::from(vec![
            Span::styled(format!(" /{} {}", app.cache_filter_text, cursor), Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {} items", total), Style::default().fg(color::TEXT_DIM)),
        ]));
    } else if !app.cache_server_filter.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(format!(" /{}/ ", app.cache_server_filter), Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{} items (of {} total)", total, app.cache_total_count), Style::default().fg(color::TEXT_DIM)),
        ]));
    }

    if app.jobs_cache.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No cached jobs.",
            Style::default().fg(color::TEXT_DIM),
        )));
    } else {
        // Header — JID/User/Started (fixed-width, no truncation fallback) come
        // before Target/Fun (already truncate past their width) so a narrow
        // panel clips the columns that already handle overflow instead of
        // cutting off Started, which has no fallback of its own (#39).
        lines.push(Line::from(Span::styled(
            format!("  {:<23} {:<12} {:<19} {:<20} {}", "JID", "User", "Started", "Target", "Fun"),
            Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD),
        )));

        if total == 0 {
            lines.push(Line::from(Span::styled(
                "  No cached jobs match the filter.",
                Style::default().fg(color::TEXT_DIM),
            )));
        } else {
            for idx in scroll..(scroll + visible).min(total) {
                let job = match app.get_filtered_cache_job(idx) {
                    Some(j) => j,
                    None => break,
                };

                let is_selected = idx == selected;
                let sel_style = if is_selected {
                    color::select_bg(app.high_contrast)
                } else {
                    Style::default().bg(color::BG_DARK)
                };

                let jid_str = if job.jid.len() > 23 { format!("{}..", &job.jid[..21]) } else { job.jid.clone() };
                let fun_str = if job.fun.len() > 26 { format!("{}..", &job.fun[..24]) } else { job.fun.clone() };
                let tgt_str = if job.tgt.len() > 20 { format!("{}..", &job.tgt[..18]) } else { job.tgt.clone() };
                let user_str = if job.user.len() > 12 { format!("{}..", &job.user[..10]) } else { job.user.clone() };
                // Format from start_epoch as local time
                let started = if job.start_epoch > 0.0 {
                    chrono::DateTime::from_timestamp(job.start_epoch as i64, 0)
                        .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| "?".to_string())
                } else {
                    "?".to_string()
                };

                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<23}", jid_str), Style::default().fg(color::TEXT_DIM)),
                    Span::styled(" ", Style::default()),
                    Span::styled(format!("{:<12}", user_str), Style::default().fg(color::PURPLE)),
                    Span::styled(" ", Style::default()),
                    Span::styled(format!("{:<19}", started), Style::default().fg(color::TEXT_DIM)),
                    Span::styled(" ", Style::default()),
                    Span::styled(format!("{:<20}", tgt_str), Style::default().fg(color::YELLOW)),
                    Span::styled(" ", Style::default()),
                    Span::styled(fun_str, Style::default().fg(color::CYAN)),
                ]).style(sel_style));
            }
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}
