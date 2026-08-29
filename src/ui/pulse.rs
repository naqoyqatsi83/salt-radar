use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
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

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(7)])
        .split(cols[0]);

    draw_dot_panel(f, left[0], app);
    draw_legend_panel(f, left[1]);
    draw_info_panel(f, cols[1], app);
}

fn draw_dot_panel(f: &mut Frame, area: Rect, app: &mut App) {
    let block = super::titled_block(
        format!(" Minion Pulse ({}) ", app.pulse_minions_count()),
        Style::default().fg(color::BORDER),
        color::BG_DARK,
    );

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

        let tier = pulse_tier(minion, app);
        let style = tier_style(tier, idx == app.pulse_selected);
        let ch = if idx == app.pulse_selected { '\u{25C9}' } else { tier_glyph(tier) };
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

/// Recency tier for a minion's last ZMQ return, from newest to oldest.
/// The single source of truth for both the dot's color (`tier_style`) and
/// its shape (`tier_glyph`) — color/boldness alone on a single terminal
/// cell reads as too subtle to reliably tell tiers apart at a glance.
#[derive(Clone, Copy)]
enum PulseTier {
    TimedOut,
    Flash,  // return < 5s ago
    Recent, // return < 5min ago
    Aging,  // return < 10min ago
    Stale,  // return >= 10min ago, or never
}

fn pulse_tier(minion: &MinionInfo, app: &App) -> PulseTier {
    if minion.timed_out {
        return PulseTier::TimedOut;
    }
    match app.last_zmq_return.get(&minion.id).map(|t| t.elapsed().as_secs_f64()) {
        Some(secs) if secs < 5.0 => PulseTier::Flash,
        Some(secs) if secs < 300.0 => PulseTier::Recent,
        Some(secs) if secs < 600.0 => PulseTier::Aging,
        _ => PulseTier::Stale,
    }
}

fn tier_color(tier: PulseTier) -> (Color, Modifier) {
    match tier {
        PulseTier::TimedOut => (color::RED, Modifier::empty()),
        PulseTier::Flash => (Color::Rgb(120, 255, 120), Modifier::BOLD),
        PulseTier::Recent => (color::GREEN, Modifier::empty()),
        PulseTier::Aging => (Color::Rgb(255, 165, 0), Modifier::empty()),
        PulseTier::Stale => (color::TEXT_DIM, Modifier::empty()),
    }
}

/// Shape shrinks with recency — • Flash → · Recent/Aging/Stale/TimedOut —
/// color is what actually carries the tier; shape only sets Flash apart.
fn tier_glyph(tier: PulseTier) -> char {
    match tier {
        PulseTier::Flash => '\u{2022}', // • bullet
        _ => '\u{00B7}',                // · middle dot
    }
}

fn tier_style(tier: PulseTier, selected: bool) -> Style {
    let (fg, modif) = tier_color(tier);

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

/// Legend panel: explains what each dot in the grid above means
fn draw_legend_panel(f: &mut Frame, area: Rect) {
    let block = super::titled_block(" Legend ", Style::default().fg(color::BORDER), color::BG_DARK);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let entries: [(PulseTier, &str); 5] = [
        (PulseTier::Flash, "return < 5s ago"),
        (PulseTier::Recent, "return < 5min ago"),
        (PulseTier::Aging, "return < 10min ago"),
        (PulseTier::Stale, "return >= 10min ago, or never"),
        (PulseTier::TimedOut, "minion timed out"),
    ];

    let lines: Vec<Line> = entries
        .iter()
        .map(|(tier, desc)| {
            let (fg, modif) = tier_color(*tier);
            Line::from(vec![
                Span::styled(
                    format!("  {} ", tier_glyph(*tier)),
                    Style::default().fg(fg).add_modifier(modif),
                ),
                Span::styled(*desc, Style::default().fg(color::TEXT_DIM)),
            ])
        })
        .collect();

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}

/// Info panel: selected minion details
fn draw_info_panel(f: &mut Frame, area: Rect, app: &mut App) {
    let block = super::titled_block(" Details ", Style::default().fg(color::BORDER), color::BG_DARK);

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
        .filter(|m| accepted.contains(m.id.as_str()) || m.key_status == Some(KeyStatus::Accepted))
        .cloned()
        .collect();
    minions.sort_by(|a, b| a.id.cmp(&b.id));
    minions
}
