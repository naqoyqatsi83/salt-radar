use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use std::collections::HashSet;

use crate::app::App;
use crate::types::{KeyStatus, MinionInfo};

use super::color;
use ratatui::style::Color;

pub fn draw_pulse(f: &mut Frame, area: Rect, app: &mut App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(area);

    draw_dot_panel(f, cols[0], app);
    draw_info_panel(f, cols[1], app);
}

fn draw_dot_panel(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER))
        .title(format!(" Minion Pulse ({}) ", app.minion_info.len()))
        .style(Style::default().bg(color::BG_DARK));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let minions = gather_minions(app);
    if minions.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  No minion data available.",
                Style::default().fg(color::TEXT_DIM),
            )))
            .style(Style::default().bg(color::BG_DARK)),
            inner,
        );
        return;
    }

    // Grid layout: one dot per column, packed left-to-right, top-to-bottom
    let cols = inner.width.max(1) as usize;
    app.pulse_grid_cols = cols;

    let mut buf: Vec<Vec<(char, Style)>> =
        vec![vec![(' ', Style::default()); inner.width as usize]; inner.height as usize];

    for (idx, minion) in minions.iter().enumerate() {
        let row = idx / cols;
        let col = idx % cols;

        if row >= inner.height as usize {
            break;
        }

        let style = dot_style(minion, idx == app.pulse_selected, app);
        let ch = if idx == app.pulse_selected { '\u{25C9}' } else { '\u{00B7}' };
        buf[row][col] = (ch, style);
    }

    let lines: Vec<Line> = buf
        .iter()
        .map(|row| {
            let spans: Vec<Span> = row
                .iter()
                .map(|(ch, style)| Span::styled(ch.to_string(), *style))
                .collect();
            Line::from(spans)
        })
        .collect();

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}

/// Determine dot color and style based on return recency.
///
/// Tiers (from newest to oldest):
///   - timed_out          → RED
///   - return < 5s ago    → BRIGHT GREEN + BOLD (flash)
///   - return < 5min ago  → GREEN
///   - return < 10min ago → ORANGE
///   - return >= 10min ago→ TEXT_DIM (grey)
///   - no return ever     → TEXT_DIM
fn dot_style(minion: &MinionInfo, selected: bool, app: &App) -> Style {
    let elapsed = app.last_zmq_return.get(&minion.id).map(|t| t.elapsed());

    let (fg, modif) = if minion.timed_out {
        (color::RED, Modifier::empty())
    } else if let Some(dur) = elapsed {
        let secs = dur.as_secs_f64();
        if secs < 5.0 {
            (Color::Rgb(120, 255, 120), Modifier::BOLD)
        } else if secs < 300.0 {
            (color::GREEN, Modifier::empty())
        } else if secs < 600.0 {
            (Color::Rgb(255, 165, 0), Modifier::empty())
        } else {
            (color::TEXT_DIM, Modifier::empty())
        }
    } else {
        (color::TEXT_DIM, Modifier::empty())
    };

    let bg = if selected {
        color::TAB_ACTIVE_BG
    } else {
        color::BG_DARK
    };
    let mut style = Style::default().fg(fg).bg(bg).add_modifier(modif);
    if selected {
        style = style.add_modifier(Modifier::REVERSED);
    }
    style
}

/// Info panel: selected minion details
fn draw_info_panel(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER))
        .title(" Details ")
        .style(Style::default().bg(color::BG_DARK));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let minions = gather_minions(app);
    if minions.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  No minion data.",
                Style::default().fg(color::TEXT_DIM),
            )))
            .style(Style::default().bg(color::BG_DARK)),
            inner,
        );
        return;
    }

    let idx = app.pulse_selected.min(minions.len().saturating_sub(1));
    let minion = &minions[idx];

    let flash = app
        .last_zmq_return
        .get(&minion.id)
        .map(|t| format!("{:.0}s ago", t.elapsed().as_secs_f64()))
        .unwrap_or_else(|| "\u{2014}".to_string());

    let mut lines = Vec::new();

    // ID
    lines.push(Line::from(vec![Span::styled(
        "  ID",
        Style::default().fg(color::TEXT_LABEL),
    )]));
    lines.push(Line::from(vec![Span::styled(
        format!("  {}", minion.id),
        Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    // Status
    lines.push(Line::from(vec![Span::styled(
        "  Status",
        Style::default().fg(color::TEXT_LABEL),
    )]));
    let status_color = if minion.timed_out {
        color::RED
    } else if minion.connected {
        color::GREEN
    } else {
        color::YELLOW
    };
    let status_text = if minion.timed_out {
        "Timed out"
    } else if minion.connected {
        "Connected"
    } else {
        "Unknown"
    };
    lines.push(Line::from(vec![Span::styled(
        format!(
            "  {} {}",
            if minion.timed_out {
                "\u{25CB}"
            } else {
                "\u{25CF}"
            },
            status_text
        ),
        Style::default()
            .fg(status_color)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    // Last ZMQ return
    lines.push(Line::from(vec![Span::styled(
        "  Last return",
        Style::default().fg(color::TEXT_LABEL),
    )]));
    let flash_color = if let Some(t) = app.last_zmq_return.get(&minion.id) {
        let secs = t.elapsed().as_secs_f64();
        if secs < 5.0 { Color::Rgb(120, 255, 120) }
        else if secs < 300.0 { color::GREEN }
        else if secs < 600.0 { Color::Rgb(255, 165, 0) }
        else { color::TEXT_DIM }
    } else {
        color::TEXT_DIM
    };
    lines.push(Line::from(vec![Span::styled(
        format!("  {}", flash),
        Style::default().fg(flash_color),
    )]));
    lines.push(Line::from(""));

    // OS
    let os_str = if !minion.osfinger.is_empty() {
        &minion.osfinger
    } else if !minion.os.is_empty() {
        &minion.os
    } else {
        "?"
    };
    lines.push(Line::from(vec![Span::styled(
        "  OS",
        Style::default().fg(color::TEXT_LABEL),
    )]));
    lines.push(Line::from(vec![Span::styled(
        format!("  {}", os_str),
        Style::default().fg(color::TEXT),
    )]));

    // Salt version
    if !minion.salt_version.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "  Salt version",
            Style::default().fg(color::TEXT_LABEL),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!("  {}", minion.salt_version),
            Style::default().fg(color::YELLOW),
        )]));
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}

fn gather_minions(app: &App) -> Vec<MinionInfo> {
    let accepted: HashSet<&str> = app.keys.iter()
        .filter(|k| k.status == KeyStatus::Accepted)
        .map(|k| k.id.as_str())
        .collect();
    let mut minions: Vec<MinionInfo> = app.minion_info.iter()
        .filter(|m| accepted.contains(m.id.as_str()))
        .cloned()
        .collect();
    minions.sort_by(|a, b| a.id.cmp(&b.id));
    minions
}
