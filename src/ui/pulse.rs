use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::types::MinionInfo;

use super::color;

/// Precomputed ring structure: number of slots per ring level.
struct Rings {
    slots_per_ring: Vec<usize>,
    total: usize,
}

impl Rings {
    fn new(count: usize) -> Self {
        let mut slots_per_ring = Vec::new();
        let mut remaining = count;
        let mut r = 0;
        while remaining > 0 {
            let slots = if r == 0 { 1 } else { 8 * r };
            let take = slots.min(remaining);
            slots_per_ring.push(take);
            remaining -= take;
            r += 1;
        }
        let total = slots_per_ring.iter().sum();
        Self { slots_per_ring, total }
    }

    fn ring_count(&self) -> usize {
        self.slots_per_ring.len()
    }

    fn slots_in_ring(&self, ring: usize) -> usize {
        *self.slots_per_ring.get(ring).unwrap_or(&0)
    }

    /// Convert a flat index to (ring, slot).
    fn index_to_ring_slot(&self, idx: usize) -> (usize, usize) {
        let mut accum = 0;
        for (ring, &slots) in self.slots_per_ring.iter().enumerate() {
            if accum + slots > idx {
                return (ring, idx - accum);
            }
            accum += slots;
        }
        // Clamp to last slot
        let last_ring = self.slots_per_ring.len().saturating_sub(1);
        let last_slot = self.slots_per_ring.last().copied().unwrap_or(1).saturating_sub(1);
        (last_ring, last_slot)
    }

    /// Convert (ring, slot) to a flat index.
    fn ring_slot_to_index(&self, ring: usize, slot: usize) -> usize {
        let before: usize = self.slots_per_ring.iter().take(ring).sum();
        before + slot
    }
}

pub fn draw_pulse(f: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(area);

    draw_dot_panel(f, cols[0], app);
    draw_info_panel(f, cols[1], app);
}

fn draw_dot_panel(f: &mut Frame, area: Rect, app: &App) {
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
            Paragraph::new(Line::from(Span::styled("  No minion data available.", Style::default().fg(color::TEXT_DIM))))
                .style(Style::default().bg(color::BG_DARK)),
            inner,
        );
        return;
    }

    let rings = Rings::new(minions.len());
    let cx = inner.width as f64 / 2.0;
    let cy = inner.height as f64 / 2.0;
    let max_usable_r = (cx.min(cy) * 2.0) - 1.0; // in character units, double for aspect
    let ring_spacing = if rings.ring_count() > 1 {
        (max_usable_r / (rings.ring_count() as f64)).max(2.0)
    } else {
        2.0
    };

    let selected = app.pulse_selected;

    // Build line buffer: map terminal positions to (content, style)
    // We render dots as individual positioned characters using a Paragraph with spaces
    // and colored dots at calculated positions.
    let mut buf: Vec<Vec<(char, Style)>> = vec![vec![(' ', Style::default()); inner.width as usize]; inner.height as usize];

    for (idx, minion) in minions.iter().enumerate() {
        let (ring, slot) = rings.index_to_ring_slot(idx);
        let slots = rings.slots_in_ring(ring);
        let angle = 2.0 * std::f64::consts::PI * slot as f64 / slots as f64 - std::f64::consts::PI / 2.0;
        let r = ring as f64 * ring_spacing;

        let x = (cx + r * angle.cos() * 0.5).round() as i32;
        let y = (cy + r * angle.sin()).round() as i32;

        if x < 0 || x >= inner.width as i32 || y < 0 || y >= inner.height as i32 {
            continue;
        }

        let (ch, base_style) = dot_style(minion, idx == selected, app);

        let bx = x as usize;
        let by = y as usize;
        buf[by][bx] = (ch, base_style);
    }

    // Convert buffer to lines
    let lines: Vec<Line> = buf.iter().map(|row| {
        let spans: Vec<Span> = row.iter().map(|(ch, style)| {
            Span::styled(ch.to_string(), *style)
        }).collect();
        Line::from(spans)
    }).collect();

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}

fn dot_style(minion: &MinionInfo, selected: bool, app: &App) -> (char, Style) {
    let flash = app.last_zmq_return.get(&minion.id)
        .map(|t| t.elapsed().as_secs_f64() < 2.0)
        .unwrap_or(false);

    let has_data = !minion.osfinger.is_empty()
        || !minion.os.is_empty()
        || !minion.salt_version.is_empty()
        || minion.last_seen_epoch > 0;

    let (fg, modif) = if flash {
        (color::GREEN, Modifier::BOLD)
    } else if minion.timed_out {
        (color::RED, Modifier::empty())
    } else if has_data {
        (color::GREEN, Modifier::empty())
    } else {
        (color::TEXT_DIM, Modifier::empty())
    };

    let ch = if selected { '\u{25C9}' } else { '\u{25CF}' };
    let bg = if selected { color::TAB_ACTIVE_BG } else { color::BG_DARK };
    let mut style = Style::default().fg(fg).bg(bg).add_modifier(modif);
    if selected {
        style = style.add_modifier(Modifier::REVERSED);
    }
    (ch, style)
}

/// Info panel: selected minion details
fn draw_info_panel(f: &mut Frame, area: Rect, app: &App) {
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
            Paragraph::new(Line::from(Span::styled("  No minion data.", Style::default().fg(color::TEXT_DIM))))
                .style(Style::default().bg(color::BG_DARK)),
            inner,
        );
        return;
    }

    let idx = app.pulse_selected.min(minions.len().saturating_sub(1));
    let minion = &minions[idx];

    let flash = app.last_zmq_return.get(&minion.id)
        .map(|t| format!("{:.0}s ago", t.elapsed().as_secs_f64()))
        .unwrap_or_else(|| "\u{2014}".to_string());

    let mut lines = Vec::new();

    // ID
    lines.push(Line::from(vec![
        Span::styled("  ID", Style::default().fg(color::TEXT_LABEL)),
    ]));
    lines.push(Line::from(vec![
        Span::styled(format!("  {}", minion.id), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)),
    ]));
    lines.push(Line::from(""));

    // Status
    lines.push(Line::from(vec![
        Span::styled("  Status", Style::default().fg(color::TEXT_LABEL)),
    ]));
    let status_color = if minion.timed_out { color::RED } else if minion.connected { color::GREEN } else { color::YELLOW };
    let status_text = if minion.timed_out { "Timed out" } else if minion.connected { "Connected" } else { "Unknown" };
    lines.push(Line::from(vec![
        Span::styled(format!("  {} {}", if minion.timed_out { "\u{25CB}" } else { "\u{25CF}" }, status_text),
            Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
    ]));
    lines.push(Line::from(""));

    // Last ZMQ return
    lines.push(Line::from(vec![
        Span::styled("  Last return", Style::default().fg(color::TEXT_LABEL)),
    ]));
    let flash_color = if flash.starts_with("0.") { color::GREEN } else { color::TEXT_DIM };
    lines.push(Line::from(vec![
        Span::styled(format!("  {}", flash), Style::default().fg(flash_color)),
    ]));
    lines.push(Line::from(""));

    // OS
    let os_str = if !minion.osfinger.is_empty() {
        &minion.osfinger
    } else if !minion.os.is_empty() {
        &minion.os
    } else {
        "?"
    };
    lines.push(Line::from(vec![
        Span::styled("  OS", Style::default().fg(color::TEXT_LABEL)),
    ]));
    lines.push(Line::from(vec![
        Span::styled(format!("  {}", os_str), Style::default().fg(color::TEXT)),
    ]));

    // Salt version
    if !minion.salt_version.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  Salt version", Style::default().fg(color::TEXT_LABEL)),
        ]));
        lines.push(Line::from(vec![
            Span::styled(format!("  {}", minion.salt_version), Style::default().fg(color::YELLOW)),
        ]));
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}

fn gather_minions(app: &App) -> Vec<MinionInfo> {
    let mut minions: Vec<MinionInfo> = app.minion_info.clone();
    // Sort by ID for stable ordering
    minions.sort_by(|a, b| a.id.cmp(&b.id));
    minions
}
