use ratatui::layout::Rect;
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
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER))
        .title(format!(" Events ({})  {:.1}/s ", app.events_total, app.events_per_sec))
        .style(Style::default().bg(color::BG_DARK));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    let total = app.filtered_events_len();
    let selected = app.event_selected.min(total.saturating_sub(1));
    let visible = (inner.height as usize).saturating_sub(2); // header + filter bar

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

    if app.events.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No events received.",
            Style::default().fg(color::TEXT_DIM),
        )));
    } else {
        // Header (always shown when events exist)
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<19}  {:<19}", "Local", "UTC"), Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {:<55}", "Tag"), Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled("  Summary", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
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

                let tag = if evt.tag.len() > 55 {
                    format!("{}..", &evt.tag[..53])
                } else {
                    evt.tag.clone()
                };

                let summary = evt.data["fun"].as_str()
                    .or_else(|| {
                        if evt.tag.contains("/ret/") { Some("return") } else { None }
                    })
                    .unwrap_or("");

                lines.push(Line::from(vec![
                    Span::styled(format!("  {}  {}", local_t, utc_t), Style::default().fg(color::TEXT_DIM)),
                    Span::styled(format!("  {:<55}", tag), Style::default().fg(color::CYAN)),
                    Span::styled(summary, Style::default().fg(color::TEXT)),
                ]).style(sel_style));
            }
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}
