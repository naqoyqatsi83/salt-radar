use std::collections::HashMap;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
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
    } else if app.hide_timed_out {
        lines.push(Line::from(Span::styled("  [Hiding non-responsive minions]", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD))));
    }

    // ── Admin status line ─────────────────────────────────────────────────
    if app.admin_mode {
        if app.fleet_scanning {
            lines.push(Line::from(Span::styled("  [Admin] Scanning fleet...", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD))));
        } else if app.fleet_data.is_some() {
            lines.push(Line::from(Span::styled("  [Admin] Scan complete", Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD))));
        } else {
            lines.push(Line::from(Span::styled("  [Admin] Ready to scan fleet", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD))));
        }
    }

    let overhead = lines.len();
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner_h = area.height.saturating_sub(2) as usize;
    let mut has_more_above = false;
    let mut has_more_below = false;

    if app.minion_info.is_empty() && app.keys.is_empty() {
        if overhead == 0 {
            lines.push(Line::from(Span::styled("  No minion data available.", Style::default().fg(color::TEXT_DIM))));
        }
    } else {
        // ── Column widths — total = 8 + sum = inner width ──────────────
        let avail = inner_w.saturating_sub(8);
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
            let available = inner_h.saturating_sub(overhead).saturating_sub(2); // -header -one row
            let visible = available.max(1);
            let (scroll, more_above, more_below) = super::scroll_window(selected, visible, total);
            has_more_above = more_above;
            has_more_below = more_below;

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

                    // "Freshly active" marker in the existing 2-space ID gutter
                    // (1 char marker + 1 space, same total width as before).
                    let fresh = !minion.timed_out && app.is_minion_fresh(minion.last_seen_epoch);
                    let fresh_marker = if fresh { "\u{25cf}" } else { " " };

                    let line = Line::from(vec![
                        Span::styled(fresh_marker, Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD)),
                        Span::styled(format!(" {:<id_w$}", id_str, id_w = id_w), Style::default().fg(fg).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("  {:<key_w$}", ks_label, key_w = key_w), Style::default().fg(label_color)),
                        Span::styled(format!("  {:<os_w$}", os_display, os_w = os_w), Style::default().fg(os_color)),
                        Span::styled(format!("{}", ver_display), Style::default().fg(color::TEXT_DIM)),
                    ]);
                    lines.push(line.style(sel_style));
                } else if to_count > 0 && !app.hide_timed_out {
                    // Collapsible section header row
                    let prefix = if app.timed_out_collapsed { "\u{25B6}" } else { "\u{25BC}" };
                    let hint = if app.timed_out_collapsed { "  'z'/'Enter' to expand" } else { "" };
                    let line = Line::from(vec![
                        Span::styled(format!("  {} Timed out ({}){}", prefix, to_count, hint),
                            Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
                    ]);
                    lines.push(line.style(sel_style));
                }
            }
        }
    }

    let mut hints: Vec<(&str, String)> = vec![
        ("/", "filter".to_string()),
        ("Enter", "detail".to_string()),
        ("PgUp/PgDn", "page".to_string()),
        ("Home/End", "top/bottom".to_string()),
        ("r", "events".to_string()),
    ];
    if app.fleet_data.is_some() {
        hints.push(("x", if app.hide_timed_out { "show timed-out".to_string() } else { "hide timed-out".to_string() }));
        if !app.hide_timed_out {
            hints.push(("z", "collapse timed-out".to_string()));
        }
    }
    if app.admin_mode {
        hints.push(("s", "scan fleet".to_string()));
    }
    let block = super::list_block_hinted(
        format!(" Minions ({}) ", total),
        Style::default().fg(color::BORDER),
        color::BG_DARK,
        has_more_above,
        has_more_below,
        &hints,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);
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
    let block = super::titled_block(" Fleet Overview ", Style::default().fg(color::BORDER), color::BG_DARK);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled("  Check-in Age:", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD))));
    match app.checkin_age_percentiles() {
        Some((p50, p90, p99)) => {
            lines.push(Line::from(vec![
                Span::styled("  p50:", Style::default().fg(color::TEXT_DIM)),
                Span::styled(format!("{}s", p50), Style::default().fg(color::TEXT)),
                Span::styled("  p90:", Style::default().fg(color::TEXT_DIM)),
                Span::styled(format!("{}s", p90), Style::default().fg(color::YELLOW)),
                Span::styled("  p99:", Style::default().fg(color::TEXT_DIM)),
                Span::styled(format!("{}s", p99), Style::default().fg(color::ORANGE)),
            ]));
        }
        None => {
            lines.push(Line::from(Span::styled("  no data yet", Style::default().fg(color::TEXT_DIM))));
        }
    }
    lines.push(Line::from(""));

    if let Some(ref fs) = app.fleet_data {
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

            let mut ver_counts: HashMap<&str, usize> = HashMap::new();
            for m in &fs.minions {
                let key = if m.saltversion.is_empty() { "?" } else { m.saltversion.as_str() };
                *ver_counts.entry(key).or_insert(0) += 1;
            }
            let mut ver_sorted: Vec<(&str, usize)> = ver_counts.into_iter().collect();
            ver_sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));

            let mut arch_counts: HashMap<&str, usize> = HashMap::new();
            for m in &fs.minions {
                let key = if m.cpuarch.is_empty() { "?" } else { m.cpuarch.as_str() };
                *arch_counts.entry(key).or_insert(0) += 1;
            }
            let mut arch_sorted: Vec<(&str, usize)> = arch_counts.into_iter().collect();
            arch_sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));

            // Name column width is derived from the longest name actually present
            // across all three tables (shared so their bars/counts/% still line up
            // with each other), instead of a hardcoded width that long osfinger
            // strings run past — clamped so one pathological name can't crush the
            // bar column on a narrow panel.
            const MIN_NAME_W: usize = 22;
            const MAX_NAME_W: usize = 40;
            let longest_name = os_sorted.iter().map(|(s, _)| s.chars().count())
                .chain(ver_sorted.iter().map(|(s, _)| s.chars().count()))
                .chain(arch_sorted.iter().map(|(s, _)| s.chars().count()))
                .max().unwrap_or(0);
            let name_w = longest_name.clamp(MIN_NAME_W, MAX_NAME_W);
            // Reserves: 2 prefix + name_w + 5 count (sp+4) + 5 pct (sp+3+%) + 1 margin
            let bar_width = (inner.width as usize).saturating_sub(name_w + 13);

            let fit_name = |s: &str| -> String {
                if s.chars().count() > name_w {
                    let truncated: String = s.chars().take(name_w.saturating_sub(2)).collect();
                    format!("{}..", truncated)
                } else {
                    s.to_string()
                }
            };

            // ── OS Distribution ───────────────────────────────────────────
            lines.push(Line::from(Span::styled("  OS Distribution:", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD))));
            let os_max = os_sorted.first().map(|(_, c)| *c).unwrap_or(1).max(1);
            for (os, count) in &os_sorted {
                let bar_len = if bar_width > 0 { (*count * bar_width) / os_max } else { 0 };
                let bar: String = (0..bar_len).map(|_| '\u{2588}').collect();
                let pct = *count as f64 / total_minions as f64 * 100.0;
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<name_w$}", fit_name(os), name_w = name_w), Style::default().fg(color::TEXT)),
                    Span::styled(format!(" {:>4}", count), Style::default().fg(color::TEXT_DIM)),
                    Span::styled(format!(" {}", bar), Style::default().fg(color::CYAN)),
                    Span::styled(format!(" {:.0}%", pct), Style::default().fg(color::TEXT_DIM)),
                ]));
            }

            // ── Salt Version ──────────────────────────────────────────────
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  Salt Version:", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD))));
            let ver_max = ver_sorted.first().map(|(_, c)| *c).unwrap_or(1).max(1);
            for (ver, count) in &ver_sorted {
                let bar_len = if bar_width > 0 { (*count * bar_width) / ver_max } else { 0 };
                let bar: String = (0..bar_len).map(|_| '\u{2588}').collect();
                let pct = *count as f64 / total_minions as f64 * 100.0;
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<name_w$}", fit_name(ver), name_w = name_w), Style::default().fg(color::TEXT)),
                    Span::styled(format!(" {:>4}", count), Style::default().fg(color::TEXT_DIM)),
                    Span::styled(format!(" {}", bar), Style::default().fg(color::YELLOW)),
                    Span::styled(format!(" {:.0}%", pct), Style::default().fg(color::TEXT_DIM)),
                ]));
            }

            // ── Architecture ──────────────────────────────────────────────
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  Architecture:", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD))));
            let arch_max = arch_sorted.first().map(|(_, c)| *c).unwrap_or(1).max(1);
            for (arch, count) in &arch_sorted {
                let bar_len = if bar_width > 0 { (*count * bar_width) / arch_max } else { 0 };
                let bar: String = (0..bar_len).map(|_| '\u{2588}').collect();
                let pct = *count as f64 / total_minions as f64 * 100.0;
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<name_w$}", fit_name(arch), name_w = name_w), Style::default().fg(color::TEXT)),
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
                "    Enable admin mode and press 's' for full fleet scan",
                Style::default().fg(color::TEXT_DIM),
            )));
        }
    } else if app.fleet_scanning {
        lines.push(Line::from(Span::styled("  Scanning fleet...", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD))));
    } else {
        lines.push(Line::from(Span::styled("  No fleet data.", Style::default().fg(color::TEXT_DIM))));
        lines.push(Line::from(Span::styled("  Activate admin mode and", Style::default().fg(color::TEXT_DIM))));
        lines.push(Line::from(Span::styled("  press 's' to scan the fleet.", Style::default().fg(color::TEXT_DIM))));
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}
