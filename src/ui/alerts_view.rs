use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;

use super::color;

const SEV_W: usize = 9;
const RULE_W: usize = 22;

pub fn draw_alerts(f: &mut Frame, area: Rect, app: &App) {
    let total = app.filtered_alerts_len();
    let selected = app.alert_selected.min(total.saturating_sub(1));
    let visible = (area.height as usize).saturating_sub(2).saturating_sub(2);
    let (scroll, has_more_above, has_more_below) = super::scroll_window(selected, visible, total);

    let hints: &[(&str, String)] = &[("/", "filter".to_string()), ("Enter", "detail".to_string())];
    let block = super::list_block_hinted(
        format!(" Alerts ({}) ", app.alert_engine.active_count()),
        Style::default().fg(color::BORDER),
        color::BG_DARK,
        has_more_above,
        has_more_below,
        hints,
    );

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

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
    }

    if app.alert_engine.alerts.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No active alerts. Everything looks healthy.",
            Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD),
        )));
    } else {
        // Header spacers are derived from the same SEV_W/RULE_W field widths
        // the data row uses below, so they can't drift out of sync.
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<SEV_W$}", "Severity"), Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:<RULE_W$}", "Rule"), Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled("Description", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        ]));

        if total == 0 {
            lines.push(Line::from(Span::styled(
                "  No alerts match the filter.",
                Style::default().fg(color::TEXT_DIM),
            )));
        } else {
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

                let detail_count = if alert.details.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", alert.details.len())
                };
                let rule_str = if alert.rule.len() > RULE_W {
                    format!("{}..", &alert.rule[..RULE_W - 2])
                } else {
                    alert.rule.to_string()
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<SEV_W$}", alert.severity.label()), Style::default().fg(sev_color).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("{:<RULE_W$}", rule_str), Style::default().fg(color::CYAN)),
                    Span::styled(alert.description.clone(), Style::default().fg(color::TEXT)),
                    Span::styled(detail_count, Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
                ]).style(sel_style));
            }
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}
