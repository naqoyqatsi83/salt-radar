use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;

use super::color;

pub fn draw_jobs(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER))
        .title(format!(" Jobs ({}) ", app.jobs.len()))
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
        let filter_text = format!(" /{} {}", app.filter_text, cursor);
        lines.push(Line::from(vec![
            Span::styled(filter_text, Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {} items", total), Style::default().fg(color::TEXT_DIM)),
        ]));
    } else if !app.filter_text.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(format!(" /{}/ ", app.filter_text), Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{} items", total), Style::default().fg(color::TEXT_DIM)),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("   Press / to filter", Style::default().fg(color::TEXT_DIM)),
        ]));
    }

    if app.jobs.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No active jobs.",
            Style::default().fg(color::TEXT_DIM),
        )));
    } else {
        // Header (always shown when jobs exist)
        lines.push(Line::from(Span::styled(
            format!("  {:<24}{:<26}{:<30}{:<8}Progress", "JID", "Fun", "Target", "Source"),
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

                // Truncate fields to prevent column overlap
                let jid_str = if job.jid.len() > 24 { format!("{}..", &job.jid[..22]) } else { job.jid.clone() };
                let fun_str = if job.fun.len() > 26 { format!("{}..", &job.fun[..24]) } else { job.fun.clone() };
                let tgt_str = if job.tgt.len() > 30 { format!("{}..", &job.tgt[..28]) } else { job.tgt.clone() };
                let source_str = if job.local { "local" } else { "master" };

                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<24}", jid_str), Style::default().fg(color::TEXT_DIM)),
                    Span::styled(format!("{:<26}", fun_str), Style::default().fg(color::CYAN)),
                    Span::styled(format!("{:<30}", tgt_str), Style::default().fg(color::YELLOW)),
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
