use std::collections::HashMap;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;

use super::color;

pub fn draw_minions(f: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    draw_minion_list(f, cols[0], app);
    draw_fleet_overview(f, cols[1], app);
}

fn draw_minion_list(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER))
        .title(format!(" Minions ({}) ", app.filtered_minions_len()))
        .style(Style::default().bg(color::BG_DARK));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    let total = app.filtered_minions_len();
    let selected = app.minion_selected.min(total.saturating_sub(1));

    // ── Filter bar ────────────────────────────────────────────────────────
    if app.is_filtering {
        let cursor = if app.filter_text.len() % 2 == 0 { '|' } else { ' ' };
        lines.push(Line::from(vec![
            Span::styled(format!(" /{} {}", app.filter_text, cursor), Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {} items", total), Style::default().fg(color::TEXT_DIM)),
        ]));
    } else if !app.filter_text.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(format!(" /{}/ ", app.filter_text), Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{} items", total), Style::default().fg(color::TEXT_DIM)),
        ]));
    } else {
        lines.push(Line::from(Span::styled("   Press / to filter", Style::default().fg(color::TEXT_DIM))));
    }

    // ── Hide/collapse hints ──────────────────────────────────────────────
    if app.hide_timed_out {
        lines.push(Line::from(Span::styled("  [Hiding non-responsive minions]  x to toggle", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD))));
    } else if app.fleet_data.is_some() {
        lines.push(Line::from(Span::styled("  x Hide  z Toggle section", Style::default().fg(color::TEXT_DIM))));
    }

    // ── Admin status line ─────────────────────────────────────────────────
    if app.admin_mode {
        if app.fleet_scanning {
            lines.push(Line::from(Span::styled("  [Admin] Scanning fleet...", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD))));
        } else if app.fleet_data.is_some() {
            lines.push(Line::from(Span::styled("  [Admin] Scan complete", Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD))));
        } else {
            lines.push(Line::from(Span::styled("  [Admin] Press S to scan fleet", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD))));
        }
    }

    let overhead = lines.len();

    if app.minion_info.is_empty() && app.keys.is_empty() {
        if overhead == 0 {
            lines.push(Line::from(Span::styled("  No minion data available.", Style::default().fg(color::TEXT_DIM))));
        }
    } else {
        // ── Column widths — total = 8 + sum = inner.width ──────────────
        let avail = (inner.width as usize).saturating_sub(8);
        let id_w = if avail > 24 { 18 } else { (avail / 3).max(4) };
        let key_w = if avail > 24 { 8 } else { (avail / 6).max(3) };
        let remaining = avail.saturating_sub(id_w + key_w + 4);
        let os_w = (remaining * 3 / 5).max(6).min(40);
        let sal_w = remaining.saturating_sub(os_w).max(4);

        // ── Table header (always shown) ───────────────────────────────────
        let h_id = format!("  {:<id_w$}", "ID", id_w = id_w);
        let h_key = format!("  {:<key_w$}", "Key", key_w = key_w);
        let h_os = format!("  {:<os_w$}", "OS", os_w = os_w);
        let h_sal = format!("  {:<sal_w$}", "Salt", sal_w = sal_w);
        lines.push(Line::from(vec![
            Span::styled(h_id, Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled(h_key, Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled(h_os, Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled(h_sal, Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        ]));

        // ── No-match message or data rows ────────────────────────────────
        if total == 0 {
            lines.push(Line::from(Span::styled("  (no minions match this filter)", Style::default().fg(color::TEXT_DIM))));
        } else {
            let available = (inner.height as usize).saturating_sub(overhead).saturating_sub(2); // -header -one row
            let visible = available.max(1);

            // Build a map of key status
            let mut key_map: HashMap<String, crate::types::KeyStatus> = HashMap::new();
            for k in &app.keys {
                key_map.insert(k.id.clone(), k.status);
            }

            // Pre-compute filtered timed-out count for the section header
            let accepted_keys: std::collections::HashSet<&str> = app.keys.iter()
                .filter(|k| k.status == crate::types::KeyStatus::Accepted)
                .map(|k| k.id.as_str())
                .collect();
            let to_count = app.minion_info.iter()
                .filter(|m| accepted_keys.contains(m.id.as_str()) || m.key_status == Some(crate::types::KeyStatus::Accepted))
                .filter(|m| app.filter_text.is_empty() || crate::app::App::matches_minion(m, &app.filter_text))
                .filter(|m| m.timed_out)
                .count();

            let scroll = if selected > visible / 2 { selected - visible / 2 } else { 0 };
            let scroll = scroll.min(total.saturating_sub(visible));

            for idx in scroll..(scroll + visible).min(total) {
                let minion = app.get_filtered_minion(idx);

                let is_selected = idx == selected;
                let sel_style = if is_selected {
                    color::select_bg(app.high_contrast)
                } else {
                    Style::default().bg(color::BG_DARK)
                };

                if let Some(minion) = minion {
                    let ks = key_map.get(&minion.id).copied().or(minion.key_status);
                    let (ks_label, ks_color) = match ks {
                        Some(crate::types::KeyStatus::Accepted) => ("accepted", color::GREEN),
                        Some(crate::types::KeyStatus::Pending) => ("pending", color::YELLOW),
                        Some(crate::types::KeyStatus::Rejected) => ("rejected", color::RED),
                        Some(crate::types::KeyStatus::Denied) => ("denied", color::ORANGE),
                        None => ("\u{2014}", color::TEXT_DIM),
                    };
                    let os_str = if !minion.osfinger.is_empty() {
                        minion.osfinger.clone()
                    } else if !minion.osrelease.is_empty() {
                        format!("{} {}", minion.os, minion.osrelease)
                    } else {
                        minion.os.clone()
                    };

                    let id_str = truncate(&minion.id, id_w);
                    let os_str = truncate(&os_str, os_w);
                    let ver_str = truncate(&minion.salt_version, sal_w);

                    let fg = if minion.timed_out { color::TEXT_DIM } else { color::TEXT };
                    let label_color = if minion.timed_out { color::TEXT_DIM } else { ks_color };
                    let os_color = if minion.timed_out { color::TEXT_DIM } else { color::CYAN };
                    let os_display = if minion.timed_out { "\u{2014}" } else { &os_str };
                    let ver_display = if minion.timed_out { "\u{2014}" } else { &ver_str };

                    let line = Line::from(vec![
                        Span::styled(format!("  {:<id_w$}", id_str, id_w = id_w), Style::default().fg(fg).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("  {:<key_w$}", ks_label, key_w = key_w), Style::default().fg(label_color)),
                        Span::styled(format!("  {:<os_w$}", os_display, os_w = os_w), Style::default().fg(os_color)),
                        Span::styled(format!("{}", ver_display), Style::default().fg(color::TEXT_DIM)),
                    ]);
                    lines.push(line.style(sel_style));
                } else if to_count > 0 && !app.hide_timed_out {
                    // Collapsible section header row
                    let prefix = if app.timed_out_collapsed { "\u{25B6}" } else { "\u{25BC}" };
                    let hint = if app.timed_out_collapsed { "  z/Enter to expand" } else { "" };
                    let line = Line::from(vec![
                        Span::styled(format!("  {} Timed out ({}){}", prefix, to_count, hint),
                            Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
                    ]);
                    lines.push(line.style(sel_style));
                }
            }
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max && max > 3 {
        format!("{}..", &s[..max.saturating_sub(2)])
    } else if s.len() > max {
        s[..max].to_string()
    } else {
        s.to_string()
    }
}

fn draw_fleet_overview(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER))
        .title(" Fleet Overview ")
        .style(Style::default().bg(color::BG_DARK));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    if let Some(ref fs) = app.fleet_data {
        // bar_width reserves space for: "  name<22> <count> <bar> <pct>%"
        // = 2 prefix + 22 name + 5 count (sp+4) + 5 pct (sp+3+%) = 34
        let fixed = 35;
        let bar_width = (inner.width as usize).saturating_sub(fixed);

        let timed_out_count = fs.minions.iter().filter(|m| m.timed_out).count();

        if app.fleet_scanning {
            let accepted = app.keys.iter().filter(|k| k.status == crate::types::KeyStatus::Accepted).count();
            let expected_total = accepted.max(1);
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  Scanning... {}/{} minions", fs.minions.len(), expected_total),
                    Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if timed_out_count > 0 { format!("  ({} timed out)", timed_out_count) } else { String::new() },
                    Style::default().fg(color::RED).add_modifier(Modifier::BOLD),
                ),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  Total: {} minions", fs.minions.len()),
                    Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if timed_out_count > 0 { format!("  ({} timed out)", timed_out_count) } else { String::new() },
                    Style::default().fg(color::RED).add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        lines.push(Line::from(""));

        if !fs.minions.is_empty() {
            let total_minions = fs.minions.len();

            // ── OS Distribution ───────────────────────────────────────────
            lines.push(Line::from(Span::styled("  OS Distribution:", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD))));

            let mut os_counts: HashMap<&str, usize> = HashMap::new();
            for m in &fs.minions {
                let key = if m.osfinger.is_empty() {
                    if m.osrelease.is_empty() { "?" } else { m.osrelease.as_str() }
                } else {
                    m.osfinger.as_str()
                };
                *os_counts.entry(key).or_insert(0) += 1;
            }
            let mut os_sorted: Vec<(&str, usize)> = os_counts.into_iter().collect();
            os_sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
            let os_max = os_sorted.first().map(|(_, c)| *c).unwrap_or(1).max(1);

            for (os, count) in &os_sorted {
                let bar_len = if bar_width > 0 { (*count * bar_width) / os_max } else { 0 };
                let bar: String = (0..bar_len).map(|_| '\u{2588}').collect();
                let pct = *count as f64 / total_minions as f64 * 100.0;
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<22}", os), Style::default().fg(color::TEXT)),
                    Span::styled(format!(" {:>4}", count), Style::default().fg(color::TEXT_DIM)),
                    Span::styled(format!(" {}", bar), Style::default().fg(color::CYAN)),
                    Span::styled(format!(" {:.0}%", pct), Style::default().fg(color::TEXT_DIM)),
                ]));
            }

            // ── Salt Version ──────────────────────────────────────────────
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  Salt Version:", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD))));

            let mut ver_counts: HashMap<&str, usize> = HashMap::new();
            for m in &fs.minions {
                let key = if m.saltversion.is_empty() { "?" } else { m.saltversion.as_str() };
                *ver_counts.entry(key).or_insert(0) += 1;
            }
            let mut ver_sorted: Vec<(&str, usize)> = ver_counts.into_iter().collect();
            ver_sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
            let ver_max = ver_sorted.first().map(|(_, c)| *c).unwrap_or(1).max(1);

            for (ver, count) in &ver_sorted {
                let bar_len = if bar_width > 0 { (*count * bar_width) / ver_max } else { 0 };
                let bar: String = (0..bar_len).map(|_| '\u{2588}').collect();
                let pct = *count as f64 / total_minions as f64 * 100.0;
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<22}", ver), Style::default().fg(color::TEXT)),
                    Span::styled(format!(" {:>4}", count), Style::default().fg(color::TEXT_DIM)),
                    Span::styled(format!(" {}", bar), Style::default().fg(color::YELLOW)),
                    Span::styled(format!(" {:.0}%", pct), Style::default().fg(color::TEXT_DIM)),
                ]));
            }

            // ── Architecture ──────────────────────────────────────────────
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  Architecture:", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD))));

            let mut arch_counts: HashMap<&str, usize> = HashMap::new();
            for m in &fs.minions {
                let key = if m.cpuarch.is_empty() { "?" } else { m.cpuarch.as_str() };
                *arch_counts.entry(key).or_insert(0) += 1;
            }
            let mut arch_sorted: Vec<(&str, usize)> = arch_counts.into_iter().collect();
            arch_sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
            let arch_max = arch_sorted.first().map(|(_, c)| *c).unwrap_or(1).max(1);

            for (arch, count) in &arch_sorted {
                let bar_len = if bar_width > 0 { (*count * bar_width) / arch_max } else { 0 };
                let bar: String = (0..bar_len).map(|_| '\u{2588}').collect();
                let pct = *count as f64 / total_minions as f64 * 100.0;
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<22}", arch), Style::default().fg(color::TEXT)),
                    Span::styled(format!(" {:>4}", count), Style::default().fg(color::TEXT_DIM)),
                    Span::styled(format!(" {}", bar), Style::default().fg(color::PURPLE)),
                    Span::styled(format!(" {:.0}%", pct), Style::default().fg(color::TEXT_DIM)),
                ]));
            }
        }
        if !app.fleet_scan_completed {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  \u{25B8} Passive collection \u{2014} data may be partial",
                Style::default().fg(color::YELLOW),
            )));
            lines.push(Line::from(Span::styled(
                "    Enable admin mode and press S for full fleet scan",
                Style::default().fg(color::TEXT_DIM),
            )));
        }
    } else if app.fleet_scanning {
        lines.push(Line::from(Span::styled("  Scanning fleet...", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD))));
    } else {
        lines.push(Line::from(Span::styled("  No fleet data.", Style::default().fg(color::TEXT_DIM))));
        lines.push(Line::from(Span::styled("  Activate admin mode and", Style::default().fg(color::TEXT_DIM))));
        lines.push(Line::from(Span::styled("  press S to scan the fleet.", Style::default().fg(color::TEXT_DIM))));
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}
