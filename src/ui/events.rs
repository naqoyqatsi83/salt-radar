use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::types::{Column, HeaderHit, TableId};

use super::color;
use super::table::{column_widths, header_line, pad};

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

pub fn draw_events(f: &mut Frame, area: Rect, app: &App, focused: bool, hits: &mut Vec<HeaderHit>) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Filtered once per frame and shared by both panels -- avoids each of
    // them separately re-scanning the whole (possibly `event_buffer_size`,
    // default 10000) event buffer.
    let filtered = app.filtered_events();

    draw_event_list(f, cols[0], app, focused, &filtered, hits);
    draw_event_preview(f, cols[1], app, &filtered);
}

fn draw_event_list(f: &mut Frame, area: Rect, app: &App, focused: bool, filtered: &[&crate::types::SaltEvent], hits: &mut Vec<HeaderHit>) {
    let total = filtered.len();
    let selected = app.event_selected.min(total.saturating_sub(1));
    let visible = (area.height as usize).saturating_sub(2).saturating_sub(2);
    let (scroll, has_more_above, has_more_below) = super::scroll_window(selected, visible, total);

    let mut hints: Vec<(&str, String)> = vec![
        ("/", "filter".to_string()),
        ("Enter", "detail".to_string()),
        ("PgUp/PgDn", "page".to_string()),
        ("Home/End", "top/bottom".to_string()),
        ("n", "sniffer".to_string()),
    ];
    hints.extend(super::table::hints(TableId::Events));
    let block = super::list_block_hinted(
        format!(" Events ({}/{}) ", app.events.len(), app.config.event_buffer_size),
        Style::default().fg(if focused { color::BORDER_HI } else { color::BORDER }),
        color::BG_DARK,
        has_more_above,
        has_more_below,
        &hints,
    );

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    let filter_terms = app.events_filter_terms();

    // Filter bar
    if app.is_filtering {
        let cursor = if app.filter_text.len() % 2 == 0 { '|' } else { ' ' };
        let filter_text = format!(" /{} {}", app.filter_text, cursor);
        lines.push(Line::from(vec![
            Span::styled(filter_text, Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {} items", total), Style::default().fg(color::TEXT_DIM)),
        ]));
    } else if !app.applied_filter.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(format!(" /{}/ ", app.applied_filter), Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{} items", total), Style::default().fg(color::TEXT_DIM)),
        ]));
    }

    if app.events.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No events received.",
            Style::default().fg(color::TEXT_DIM),
        )));
    } else {
        let cols = app.columns(TableId::Events);
        let widths = column_widths(&cols, (inner.width as usize).saturating_sub(2));
        lines.push(header_line(TableId::Events, &cols, &widths, None, inner.x, inner.y + lines.len() as u16, hits));

        if total == 0 {
            lines.push(Line::from(Span::styled(
                "  No events match the filter.",
                Style::default().fg(color::TEXT_DIM),
            )));
        } else {
            for idx in scroll..(scroll + visible).min(total) {
                let evt = filtered[idx];

                let mut spans = vec![Span::raw("  ")];
                for (&col, &w) in cols.iter().zip(&widths) {
                    let (text, fg) = match col {
                        Column::Local => (format_time_local(evt.ts), color::TEXT_TITLE),
                        Column::Utc => (format_time_utc(evt.ts), color::TEXT_TITLE),
                        Column::Tag => (evt.tag.clone(), color::CYAN),
                        Column::EventMinion => (evt.data["id"].as_str().unwrap_or("").to_string(), color::YELLOW),
                        _ => (String::new(), color::TEXT),
                    };
                    let padded = pad(&text, w, false);
                    // A row can be in the filtered set because a term matched
                    // some other field (fun, tgt, the whole JSON body) without
                    // this cell's text containing it — keep its plain color in
                    // that case instead of running it through the highlighter
                    // and getting an all-TEXT-colored cell that looks unstyled.
                    let lower = padded.to_lowercase();
                    let has_hit = matches!(col, Column::Tag | Column::EventMinion)
                        && filter_terms.iter().any(|t| lower.contains(&t.to_lowercase()));
                    if has_hit {
                        spans.extend(super::highlight_keywords(&padded, &filter_terms));
                    } else {
                        spans.push(Span::styled(padded, Style::default().fg(fg)));
                    }
                }

                let line = Line::from(spans);
                lines.push(if idx == selected { color::selected_row(line, app.cursor_bg()) } else { line });
            }
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}

/// Right panel: preview of the selected event
fn draw_event_preview(f: &mut Frame, area: Rect, app: &App, filtered: &[&crate::types::SaltEvent]) {
    let block = super::titled_block(" Preview ", Style::default().fg(color::BORDER), color::BG_DARK);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let total = filtered.len();
    if total == 0 {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled("  No event selected.", Style::default().fg(color::TEXT_DIM))))
                .style(Style::default().bg(color::BG_DARK)),
            inner,
        );
        return;
    }

    let selected = app.event_selected.min(total.saturating_sub(1));
    let evt = filtered[selected];

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
            "  ... (press 'Enter' to view full)",
            Style::default().fg(color::YELLOW),
        )));
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}
