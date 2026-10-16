use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::types::{AlertEntry, Column, HeaderHit, Tab, TableId};

use super::color;
use super::table::{cell, cell_with_suffix, column_widths, header_line};

fn alert_cells(alert: &AlertEntry, col: Column, w: usize) -> Vec<Span<'static>> {
    let right = col.right_aligned();
    let span = match col {
        Column::Severity => {
            let sev_color = match alert.severity {
                crate::types::AlertSeverity::Info => color::cyan(),
                crate::types::AlertSeverity::Warn => color::yellow(),
                crate::types::AlertSeverity::Crit => color::red(),
            };
            cell(alert.severity.label(), w, right, Style::default().fg(sev_color).add_modifier(Modifier::BOLD))
        }
        Column::Rule => cell(alert.rule, w, right, Style::default().fg(color::cyan())),
        Column::Description => {
            let detail_count = if alert.details.is_empty() {
                String::new()
            } else {
                format!(" [{}]", alert.details.len())
            };
            return cell_with_suffix(
                &alert.description, &detail_count, w,
                Style::default().fg(color::main_fg()),
                Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD),
            );
        }
        Column::Since => cell(&super::dashboard::format_duration(alert.active_since.elapsed().as_secs()), w, right, Style::default().fg(color::inactive_fg())),
        Column::Count => cell(&alert.count.to_string(), w, right, Style::default().fg(color::main_fg())),
        _ => cell("", w, right, Style::default()),
    };
    vec![span]
}

pub fn draw_alerts(f: &mut Frame, area: Rect, app: &App, focused: bool, hits: &mut Vec<HeaderHit>) {
    // Filtered and sorted once per frame, not once per visible row.
    let filtered = app.filtered_alerts();
    let total = filtered.len();
    let selected = app.alert_selected.min(total.saturating_sub(1));
    let visible = (area.height as usize).saturating_sub(2).saturating_sub(2);
    let (scroll, has_more_above, has_more_below) = super::scroll_window(selected, visible, total);

    let mut hints: Vec<(&str, String)> = vec![
        ("/", "filter".to_string()),
        ("Enter", "detail".to_string()),
        ("PgUp/PgDn", "page".to_string()),
        ("Home/End", "top/bottom".to_string()),
    ];
    hints.extend(super::table::hints(TableId::Alerts));
    let block = super::list_block_hinted(
        format!(" Alerts ({}) ", app.alert_engine.active_count()),
        Style::default().fg(if focused { color::border_focus() } else { color::border() }),
        color::main_bg(),
        has_more_above,
        has_more_below,
        &hints,
        area.width,
    );

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // Filter bar
    let (filter, count) = if app.is_filtering {
        (app.filter_text.as_str(), format!("{} items", total))
    } else {
        (app.filter_text_for(Tab::Alerts), format!("{} items", total))
    };
    if let Some(bar) = super::filter_bar(filter, app.is_filtering, app.input_cursor, count, true) {
        lines.push(bar);
    }

    if app.alert_engine.alerts.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No active alerts. Everything looks healthy.",
            Style::default().fg(color::green()).add_modifier(Modifier::BOLD),
        )));
    } else {
        let cols = app.columns(TableId::Alerts);
        let widths = column_widths(&cols, (inner.width as usize).saturating_sub(2));
        lines.push(header_line(TableId::Alerts, &cols, &widths, app.sort_of(TableId::Alerts), inner.x, inner.y + lines.len() as u16, hits));

        if total == 0 {
            lines.push(Line::from(Span::styled(
                "  No alerts match the filter.",
                Style::default().fg(color::inactive_fg()),
            )));
        } else {
            for (idx, alert) in filtered.iter().enumerate().skip(scroll).take(visible) {
                let mut row = vec![Span::raw("  ")];
                for (&col, &w) in cols.iter().zip(&widths) {
                    row.extend(alert_cells(alert, col, w));
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
