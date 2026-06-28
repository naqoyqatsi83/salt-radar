use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;

use super::color;

fn format_time_local(ts: f64) -> String {
    if ts < 1.0 {
        return "\u{2014}".to_string();
    }
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "\u{2014}".to_string())
}

fn format_time_utc(ts: f64) -> String {
    if ts < 1.0 {
        return "\u{2014}".to_string();
    }
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "\u{2014}".to_string())
}

pub fn draw_events(f: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    draw_event_list(f, cols[0], app);
    draw_event_preview(f, cols[1], app);
}

fn draw_event_list(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER))
        .title(format!(" Events ({}) ", app.events_total))
        .style(Style::default().bg(color::BG_DARK));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    let total = app.filtered_events_len();
    let selected = app.event_selected.min(total.saturating_sub(1));
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
            Span::styled("  Press / to filter", Style::default().fg(color::TEXT_DIM)),
        ]));
    }

    if app.events.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No events received.",
            Style::default().fg(color::TEXT_DIM),
        )));
    } else {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<19}  {:<19}", "Local", "UTC"), Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {:<45}", "Tag"), Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        ]));

        if total == 0 {
            lines.push(Line::from(Span::styled(
                "  No events match the filter.",
                Style::default().fg(color::TEXT_DIM),
            )));
        } else {
            let scroll = if selected > visible / 2 { selected - visible / 2 } else { 0 };
            let scroll = scroll.min(total.saturating_sub(visible));

            for idx in scroll..(scroll + visible).min(total) {
                let evt = match app.get_filtered_event(idx) {
                    Some(e) => e,
                    None => break,
                };

                let is_selected = idx == selected;
                let sel_style = if is_selected {
                    color::select_bg(app.high_contrast)
                } else {
                    Style::default().bg(color::BG_DARK)
                };

                let local_t = format_time_local(evt.ts);
                let utc_t = format_time_utc(evt.ts);

                let tag = if evt.tag.len() > 45 {
                    format!("{}..", &evt.tag[..43])
                } else {
                    evt.tag.clone()
                };

                lines.push(Line::from(vec![
                    Span::styled(format!("  {}  {}", local_t, utc_t), Style::default().fg(color::TEXT_DIM)),
                    Span::styled(format!("  {:<45}", tag), Style::default().fg(color::CYAN)),
                ]).style(sel_style));
            }
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}

/// Right panel: preview of the selected event
fn draw_event_preview(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER))
        .title(" Preview ")
        .style(Style::default().bg(color::BG_DARK));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let total = app.filtered_events_len();
    if total == 0 {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled("  No event selected.", Style::default().fg(color::TEXT_DIM))))
                .style(Style::default().bg(color::BG_DARK)),
            inner,
        );
        return;
    }

    let selected = app.event_selected.min(total.saturating_sub(1));
    let evt = match app.get_filtered_event(selected) {
        Some(e) => e,
        None => return,
    };

    let mut lines = Vec::new();

    // Tag
    lines.push(Line::from(vec![
        Span::styled("  Tag", Style::default().fg(color::TEXT_LABEL)),
    ]));
    lines.push(Line::from(vec![
        Span::styled(format!("  {}", evt.tag), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)),
    ]));
    lines.push(Line::from(""));

    // Timestamps
    lines.push(Line::from(vec![
        Span::styled("  Timestamp", Style::default().fg(color::TEXT_LABEL)),
    ]));
    lines.push(Line::from(vec![
        Span::styled(format!("  Local  {}", format_time_local(evt.ts)), Style::default().fg(color::TEXT_DIM)),
    ]));
    lines.push(Line::from(vec![
        Span::styled(format!("  UTC   {}", format_time_utc(evt.ts)), Style::default().fg(color::TEXT_DIM)),
    ]));
    lines.push(Line::from(""));

    // Summary
    let fun = evt.data["fun"].as_str().unwrap_or("");
    if !fun.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("  Function", Style::default().fg(color::TEXT_LABEL)),
        ]));
        lines.push(Line::from(vec![
            Span::styled(format!("  {}", fun), Style::default().fg(color::YELLOW)),
        ]));
        lines.push(Line::from(""));
    }

    // JSON body — render as pretty-printed, truncate to available height
    lines.push(Line::from(vec![
        Span::styled("  Body", Style::default().fg(color::TEXT_LABEL)),
    ]));

    let json_str = serde_json::to_string_pretty(&evt.data).unwrap_or_default();
    let data_lines: Vec<&str> = json_str.lines().collect();
    let max_body = (inner.height as usize).saturating_sub(lines.len() + 2);

    for data_line in data_lines.iter().take(max_body) {
        lines.push(Line::from(Span::styled(
            format!("  {}", data_line),
            Style::default().fg(color::TEXT),
        )));
    }
    if data_lines.len() > max_body {
        lines.push(Line::from(Span::styled(
            "  ... (press Enter to view full)",
            Style::default().fg(color::YELLOW),
        )));
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}
