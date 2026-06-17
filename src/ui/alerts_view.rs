use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;

use super::color;

pub fn draw_alerts(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER))
        .title(format!(" Alerts ({}) ", app.alert_engine.active_count()))
        .style(Style::default().bg(color::BG_DARK));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    let total = app.filtered_alerts_len();
    let selected = app.alert_selected.min(total.saturating_sub(1));
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

    if app.alert_engine.alerts.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No active alerts. Everything looks healthy.",
            Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD),
        )));
    } else {
        // Header (always shown when alerts exist)
        lines.push(Line::from(vec![
            Span::styled("  Severity", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled("   ", Style::default().fg(color::TEXT_LABEL)),
            Span::styled("Rule", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled("               ", Style::default().fg(color::TEXT_LABEL)),
            Span::styled("Description", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        ]));

        if total == 0 {
            lines.push(Line::from(Span::styled(
                "  No alerts match the filter.",
                Style::default().fg(color::TEXT_DIM),
            )));
        } else {
            let scroll = if selected > visible / 2 { selected - visible / 2 } else { 0 };
            let scroll = scroll.min(total.saturating_sub(visible));

            for idx in scroll..(scroll + visible).min(total) {
                let alert = match app.get_filtered_alert(idx) {
                    Some(a) => a,
                    None => break,
                };

                let is_selected = idx == selected;
                let sel_style = if is_selected {
                    color::select_bg(app.high_contrast)
                } else {
                    Style::default().bg(color::BG_DARK)
                };

                let sev_color = match alert.severity {
                    crate::types::AlertSeverity::Info => color::CYAN,
                    crate::types::AlertSeverity::Warn => color::YELLOW,
                    crate::types::AlertSeverity::Crit => color::RED,
                };

                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<8}", alert.severity.label()), Style::default().fg(sev_color).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("{:<16}", alert.rule), Style::default().fg(color::CYAN)),
                    Span::styled(alert.description.clone(), Style::default().fg(color::TEXT)),
                ]).style(sel_style));
            }
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}
