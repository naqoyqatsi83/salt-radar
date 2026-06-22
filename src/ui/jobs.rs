use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;

use super::color;

pub fn draw_jobs(f: &mut Frame, area: Rect, app: &App) {
    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(55),
            Constraint::Percentage(45),
        ])
        .split(area);

    draw_live_jobs_panel(f, layout[0], app);
    draw_cache_panel(f, layout[1], app);
}

fn draw_live_jobs_panel(f: &mut Frame, area: Rect, app: &App) {
    let focused = !app.cache_focus;
    let border_style = if focused {
        Style::default().fg(color::BORDER_HI)
    } else {
        Style::default().fg(color::BORDER)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(format!(" Live Jobs ({}) ", app.jobs.len()))
        .style(Style::default().bg(color::BG_DARK));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    let total = app.filtered_jobs_len();
    let selected = app.job_selected.min(total.saturating_sub(1));
    let visible = (inner.height as usize).saturating_sub(2);

    // Filter bar
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
    } else {
        lines.push(Line::from(vec![
            Span::styled("   / to filter", Style::default().fg(color::TEXT_DIM)),
        ]));
    }

    if app.jobs.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No active jobs.",
            Style::default().fg(color::TEXT_DIM),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!("  {:<24}{:<26}{:<22}{:<8}Progress", "JID", "Fun", "Target", "Source"),
            Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD),
        )));

        if total == 0 {
            lines.push(Line::from(Span::styled(
                "  No jobs match the filter.",
                Style::default().fg(color::TEXT_DIM),
            )));
        } else {
            let scroll = if selected > visible / 2 { selected - visible / 2 } else { 0 };
            let scroll = scroll.min(total.saturating_sub(visible));

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

                let progress = if job.target_count > 0 {
                    format!("{}/{}", job.returned, job.target_count)
                } else {
                    "?".to_string()
                };

                let jid_str = if job.jid.len() > 24 { format!("{}..", &job.jid[..22]) } else { job.jid.clone() };
                let fun_str = if job.fun.len() > 26 { format!("{}..", &job.fun[..24]) } else { job.fun.clone() };
                let tgt_str = if job.tgt.len() > 22 { format!("{}..", &job.tgt[..20]) } else { job.tgt.clone() };
                let source_str = if job.local { "local" } else { "master" };

                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<24}", jid_str), Style::default().fg(color::TEXT_DIM)),
                    Span::styled(format!("{:<26}", fun_str), Style::default().fg(color::CYAN)),
                    Span::styled(format!("{:<22}", tgt_str), Style::default().fg(color::YELLOW)),
                    Span::styled(format!("{:<8}", source_str), Style::default().fg(if job.local { color::PURPLE } else { color::GREEN })),
                    Span::styled(progress, Style::default().fg(if job.returned >= job.target_count && job.target_count > 0 { color::GREEN } else { color::ORANGE }).add_modifier(Modifier::BOLD)),
                ]).style(sel_style));
            }
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}

fn draw_cache_panel(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.cache_focus;
    let border_style = if focused {
        Style::default().fg(color::BORDER_HI)
    } else {
        Style::default().fg(color::BORDER)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(format!(" Job Cache ({}) ", app.jobs_cache.len()))
        .style(Style::default().bg(color::BG_DARK));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    let total = app.filtered_cache_len();
    let selected = app.cache_selected.min(total.saturating_sub(1));
    let visible = (inner.height as usize).saturating_sub(2);

    // Loading state
    if app.cache_loading && !app.cache_loaded {
        lines.push(Line::from(Span::styled(
            "  Loading job cache...",
            Style::default().fg(color::TEXT_DIM),
        )));
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

    // Filter bar
    if app.cache_is_filtering {
        let cursor = if app.cache_filter_text.len() % 2 == 0 { '|' } else { ' ' };
        lines.push(Line::from(vec![
            Span::styled(format!(" /{} {}", app.cache_filter_text, cursor), Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {} items", total), Style::default().fg(color::TEXT_DIM)),
        ]));
    } else if !app.cache_filter_text.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(format!(" /{}/ ", app.cache_filter_text), Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{} items", total), Style::default().fg(color::TEXT_DIM)),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("   / to filter", Style::default().fg(color::TEXT_DIM)),
        ]));
    }

    if app.jobs_cache.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No cached jobs.",
            Style::default().fg(color::TEXT_DIM),
        )));
    } else {
        // Header
        lines.push(Line::from(Span::styled(
            format!("  {:<20}{:<24}{:<18}{:<10}{}", "JID", "Fun", "Target", "User", "Started"),
            Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD),
        )));

        if total == 0 {
            lines.push(Line::from(Span::styled(
                "  No cached jobs match the filter.",
                Style::default().fg(color::TEXT_DIM),
            )));
        } else {
            let scroll = if selected > visible / 2 { selected - visible / 2 } else { 0 };
            let scroll = scroll.min(total.saturating_sub(visible));

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

                let jid_str = if job.jid.len() > 20 { format!("{}..", &job.jid[..18]) } else { job.jid.clone() };
                let fun_str = if job.fun.len() > 24 { format!("{}..", &job.fun[..22]) } else { job.fun.clone() };
                let tgt_str = if job.tgt.len() > 18 { format!("{}..", &job.tgt[..16]) } else { job.tgt.clone() };
                let user_str = if job.user.len() > 10 { format!("{}..", &job.user[..8]) } else { job.user.clone() };
                let started = if job.start_time.len() > 12 {
                    format!("{}..", &job.start_time[..10])
                } else {
                    job.start_time.clone()
                };

                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<20}", jid_str), Style::default().fg(color::TEXT_DIM)),
                    Span::styled(format!("{:<24}", fun_str), Style::default().fg(color::CYAN)),
                    Span::styled(format!("{:<18}", tgt_str), Style::default().fg(color::YELLOW)),
                    Span::styled(format!("{:<10}", user_str), Style::default().fg(color::PURPLE)),
                    Span::styled(started, Style::default().fg(color::TEXT_DIM)),
                ]).style(sel_style));
            }
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}
