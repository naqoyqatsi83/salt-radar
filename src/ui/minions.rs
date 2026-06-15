use std::collections::HashMap;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;

use super::color;

pub fn draw_minions(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER))
        .title(format!(" Minions ({}) ", app.minion_info.len()))
        .style(Style::default().bg(color::BG_DARK));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    let total = app.filtered_minions_len();
    let selected = app.minion_selected.min(total.saturating_sub(1));
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

    if total == 0 && app.minion_info.is_empty() && app.keys.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No minion data available.",
            Style::default().fg(color::TEXT_DIM),
        )));
    } else if total > 0 {
        // Header
        lines.push(Line::from(vec![
            Span::styled("  ID", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled("                    ", Style::default().fg(color::TEXT_LABEL)),
            Span::styled("Key", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled("     ", Style::default().fg(color::TEXT_LABEL)),
            Span::styled("OS", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled("                    ", Style::default().fg(color::TEXT_LABEL)),
            Span::styled("Salt", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        ]));

        // Build a map of key status
        let mut key_map: HashMap<String, crate::types::KeyStatus> = HashMap::new();
        for k in &app.keys {
            key_map.insert(k.id.clone(), k.status);
        }

        let scroll = if selected > visible / 2 { selected - visible / 2 } else { 0 };
        let scroll = scroll.min(total.saturating_sub(visible));

        for idx in scroll..(scroll + visible).min(total) {
            let minion = match app.get_filtered_minion(idx) {
                Some(m) => m,
                None => break,
            };

            let is_selected = idx == selected;
            let sel_style = if is_selected {
                Style::default().bg(color::BG_LIGHT)
            } else {
                Style::default().bg(color::BG_DARK)
            };

            let ks = key_map.get(&minion.id).copied();
            let (ks_label, ks_color) = match ks {
                Some(crate::types::KeyStatus::Accepted) => ("accepted", color::GREEN),
                Some(crate::types::KeyStatus::Pending) => ("pending", color::YELLOW),
                Some(crate::types::KeyStatus::Rejected) => ("rejected", color::RED),
                Some(crate::types::KeyStatus::Denied) => ("denied", color::ORANGE),
                None => ("\u{2014}", color::TEXT_DIM),
            };

            lines.push(Line::from(vec![
                Span::styled(format!("  {:<20}", minion.id), Style::default().fg(color::TEXT).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{:<8}", ks_label), Style::default().fg(ks_color)),
                Span::styled(format!("{:<8} {}", minion.os, minion.osrelease), Style::default().fg(color::CYAN)),
                Span::raw("  "),
                Span::styled(minion.salt_version.clone(), Style::default().fg(color::TEXT_DIM)),
            ]).style(sel_style));
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}
