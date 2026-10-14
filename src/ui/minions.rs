use std::collections::HashMap;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, MinionHealth, MinionRow};
use crate::types::{Column, HeaderHit, KeyStatus, MinionInfo, Tab, TableId};

use super::color;
use super::table::{cell, column_widths, header_line};

pub fn draw_minions(f: &mut Frame, area: Rect, app: &App, focused: bool, hits: &mut Vec<HeaderHit>) {
    let cols = super::split_panes(area, app);

    draw_minion_list(f, cols[0], app, focused, hits);
    draw_fleet_overview(f, cols[1], app);
}

fn draw_minion_list(f: &mut Frame, area: Rect, app: &App, focused: bool, hits: &mut Vec<HeaderHit>) {
    let mut lines: Vec<Line> = Vec::new();
    let total = app.filtered_minions_len();
    let entries = app.minion_entry_count();
    let selected = app.minion_selected.min(total.saturating_sub(1));

    // ── Filter bar ────────────────────────────────────────────────────────
    let filter = if app.is_filtering { &app.filter_text } else { app.filter_text_for(Tab::Minions) };
    if let Some(bar) = super::filter_bar(filter, app.is_filtering, app.input_cursor, format!("{} items", entries), true) {
        lines.push(bar);
    } else if app.hide_timed_out {
        lines.push(Line::from(Span::styled("  [Hiding non-responsive minions]", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD))));
    }

    // ── Admin status line ─────────────────────────────────────────────────
    if app.admin_mode {
        if app.fleet_scanning {
            lines.push(Line::from(Span::styled("  [Admin] Scanning fleet...", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD))));
        } else if app.fleet_data.is_some() {
            lines.push(Line::from(Span::styled("  [Admin] Scan complete", Style::default().fg(color::green()).add_modifier(Modifier::BOLD))));
        } else {
            lines.push(Line::from(Span::styled("  [Admin] Ready to scan fleet", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD))));
        }
    }

    let overhead = lines.len();
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner_h = area.height.saturating_sub(2) as usize;
    let mut has_more_above = false;
    let mut has_more_below = false;

    if app.minion_info.is_empty() && app.keys.is_empty() {
        if overhead == 0 {
            lines.push(Line::from(Span::styled("  No minion data available.", Style::default().fg(color::inactive_fg()))));
        }
    } else {
        let cols = app.columns(TableId::Minions);
        let widths = column_widths(&cols, inner_w.saturating_sub(2));
        lines.push(header_line(TableId::Minions, &cols, &widths, app.sort_of(TableId::Minions), area.x + 1, area.y + 1 + overhead as u16, hits));

        // Filtered and sorted once per frame, not once per visible row.
        let rows = app.minion_rows();
        let total = rows.len();
        if total == 0 {
            lines.push(Line::from(Span::styled("  (no minions match this filter)", Style::default().fg(color::inactive_fg()))));
        } else {
            let available = inner_h.saturating_sub(overhead).saturating_sub(2); // -header -one row
            let visible = available.max(1);
            let (scroll, more_above, more_below) = super::scroll_window(selected, visible, total);
            has_more_above = more_above;
            has_more_below = more_below;

            let to_count = app.filtered_timed_out_count();
            let now = chrono::Utc::now().timestamp().max(0) as u64;

            for (idx, row) in rows.iter().enumerate().skip(scroll).take(visible) {
                let select = |line: Line<'static>| if idx == selected { color::selected_row(line) } else { line };

                match row {
                    MinionRow::Minion(minion) => {
                        // "Freshly active" marker in the 2-space row indent.
                        let fresh = !minion.timed_out && app.is_minion_fresh(minion.last_seen_epoch);
                        let mut spans = vec![
                            Span::styled(if fresh { "\u{25cf}" } else { " " }, Style::default().fg(color::green()).add_modifier(Modifier::BOLD)),
                            Span::raw(" "),
                        ];
                        for (&col, &w) in cols.iter().zip(&widths) {
                            spans.push(minion_cell(minion, Some(KeyStatus::Accepted), col, w, now, app.minion_health(&minion.id)));
                        }
                        lines.push(select(Line::from(spans)));
                    }
                    MinionRow::Key(key) => {
                        // No minion data: every column but the ID and key reads as a dash.
                        let stub = MinionInfo { id: key.id.clone(), ..Default::default() };
                        let mut spans = vec![Span::raw("  ")];
                        for (&col, &w) in cols.iter().zip(&widths) {
                            spans.push(minion_cell(&stub, Some(key.status), col, w, now, None));
                        }
                        lines.push(select(Line::from(spans)));
                    }
                    MinionRow::TimedOutHeader => {
                        // Collapsible section header row
                        let prefix = if app.timed_out_collapsed { "\u{25B6}" } else { "\u{25BC}" };
                        let hint = if app.timed_out_collapsed { "  'z'/'Enter' to expand" } else { "" };
                        let line = Line::from(vec![
                            Span::styled(format!("  {} Timed out ({}){}", prefix, to_count, hint),
                                Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD)),
                        ]);
                        lines.push(select(line));
                    }
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
    hints.extend(super::table::hints(TableId::Minions));
    let block = super::list_block_hinted(
        format!(" Minions ({}) ", entries),
        Style::default().fg(if focused { color::border_focus() } else { color::border() }),
        color::main_bg(),
        has_more_above,
        has_more_below,
        &hints,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::main_bg())),
        inner,
    );
}

fn minion_cell(minion: &MinionInfo, ks: Option<KeyStatus>, col: Column, w: usize, now: u64, health: Option<&MinionHealth>) -> Span<'static> {
    let right = col.right_aligned();
    let dim = Style::default().fg(color::inactive_fg());
    // A timed-out minion's grains are stale/unknown, so its detail columns
    // read as a dash and everything is dimmed.
    let unknown = |s: &str| if minion.timed_out || s.is_empty() { "\u{2014}".to_string() } else { s.to_string() };
    match col {
        Column::MinionId => {
            let fg = if minion.timed_out { color::inactive_fg() } else { color::main_fg() };
            cell(&minion.id, w, right, Style::default().fg(fg).add_modifier(Modifier::BOLD))
        }
        Column::KeyStatus => {
            let (label, ks_color) = match ks {
                Some(KeyStatus::Accepted) => ("accepted", color::green()),
                Some(KeyStatus::Pending) => ("pending", color::yellow()),
                Some(KeyStatus::Rejected) => ("rejected", color::red()),
                Some(KeyStatus::Denied) => ("denied", color::orange()),
                None => ("\u{2014}", color::inactive_fg()),
            };
            cell(label, w, right, Style::default().fg(if minion.timed_out { color::inactive_fg() } else { ks_color }))
        }
        Column::Os => cell(&unknown(&minion.os_label()), w, right, Style::default().fg(if minion.timed_out { color::inactive_fg() } else { color::cyan() })),
        Column::SaltVersion => cell(&unknown(&minion.salt_version), w, right, dim),
        Column::Proto => match crate::types::auth_protocol(&minion.salt_version) {
            None => cell("\u{2014}", w, right, dim),
            Some(p) => cell(&format!("v{}", p), w, right, Style::default().fg(if p < 3 { color::yellow() } else { color::inactive_fg() })),
        },
        Column::Arch => cell(&unknown(&minion.cpuarch), w, right, dim),
        Column::LastSeen => {
            let seen = if minion.last_seen_epoch == 0 {
                "\u{2014}".to_string()
            } else {
                super::dashboard::format_duration(now.saturating_sub(minion.last_seen_epoch))
            };
            cell(&seen, w, right, dim)
        }
        Column::FailPct => match health.and_then(MinionHealth::fail_ratio) {
            None => cell("\u{2014}", w, right, dim),
            Some(r) => {
                let fg = if r == 0.0 { color::inactive_fg() } else { color::outcome_failed() };
                // Any failure reads as at least 1%, so it never rounds away to 0%.
                let pct = if r == 0.0 { 0.0 } else { (r * 100.0).round().max(1.0) };
                cell(&format!("{:.0}%", pct), w, right, Style::default().fg(fg))
            }
        },
        Column::Lag => match health.and_then(|h| h.lag_at(0.5)) {
            None => cell("\u{2014}", w, right, dim),
            Some(r) => cell(&super::format_lag(r), w, right, Style::default().fg(super::lag_color(r))),
        },
        Column::Ping => match health.and_then(|h| h.last_ping_secs) {
            None => cell("\u{2014}", w, right, dim),
            Some(s) => cell(&super::format_secs_short(s), w, right, dim),
        },
        _ => cell("", w, right, dim),
    }
}

fn draw_fleet_overview(f: &mut Frame, area: Rect, app: &App) {
    let follow = app.config.minions_follow_filter;
    let hints: Vec<(&str, String)> = vec![
        ("f", format!("follow filter: {}", if follow { "on" } else { "off" })),
    ];
    let block = super::titled_block_hinted(" Fleet Overview ", Style::default().fg(color::border()), color::main_bg(), &hints);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled("  Check-in Age:", Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD))));
    match app.checkin_age_percentiles() {
        Some((p50, p90, p99)) => {
            lines.push(Line::from(vec![
                Span::styled("  p50:", Style::default().fg(color::inactive_fg())),
                Span::styled(format!("{}s", p50), Style::default().fg(color::main_fg())),
                Span::styled("  p90:", Style::default().fg(color::inactive_fg())),
                Span::styled(format!("{}s", p90), Style::default().fg(color::yellow())),
                Span::styled("  p99:", Style::default().fg(color::inactive_fg())),
                Span::styled(format!("{}s", p99), Style::default().fg(color::orange())),
            ]));
        }
        None => {
            lines.push(Line::from(Span::styled("  no data yet", Style::default().fg(color::inactive_fg()))));
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
                    Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if timed_out_count > 0 { format!("  ({} timed out)", timed_out_count) } else { String::new() },
                    Style::default().fg(color::red()).add_modifier(Modifier::BOLD),
                ),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  Total: {} minions", fs.minions.len()),
                    Style::default().fg(color::cyan()).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if timed_out_count > 0 { format!("  ({} timed out)", timed_out_count) } else { String::new() },
                    Style::default().fg(color::red()).add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        lines.push(Line::from(""));

        let pat = if follow { app.filter_text_for(Tab::Minions) } else { "" };
        let matches = crate::app::App::fleet_minion_matcher(pat);
        let filtered_minions: Vec<&crate::types::FleetMinion> = fs.minions.iter().filter(|m| matches(m)).collect();

        if !pat.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  Filtered: {} of {} minions match ", filtered_minions.len(), fs.minions.len()),
                    Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD),
                ),
                Span::styled(pat.to_string(), Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            ]));
            lines.push(Line::from(""));
        }

        if filtered_minions.is_empty() && !pat.is_empty() {
            lines.push(Line::from(Span::styled("  No minions match this filter.", Style::default().fg(color::inactive_fg()))));
        } else if !filtered_minions.is_empty() {
            let total_minions = filtered_minions.len();

            let mut os_counts: HashMap<&str, usize> = HashMap::new();
            for m in &filtered_minions {
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
            for m in &filtered_minions {
                let key = if m.saltversion.is_empty() { "?" } else { m.saltversion.as_str() };
                *ver_counts.entry(key).or_insert(0) += 1;
            }
            let mut ver_sorted: Vec<(&str, usize)> = ver_counts.into_iter().collect();
            ver_sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));

            let mut arch_counts: HashMap<&str, usize> = HashMap::new();
            for m in &filtered_minions {
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
            lines.push(Line::from(Span::styled("  OS Distribution:", Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD))));
            let os_max = os_sorted.first().map(|(_, c)| *c).unwrap_or(1).max(1);
            for (os, count) in &os_sorted {
                let bar_len = if bar_width > 0 { (*count * bar_width) / os_max } else { 0 };
                let bar: String = (0..bar_len).map(|_| '\u{2588}').collect();
                let pct = *count as f64 / total_minions as f64 * 100.0;
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<name_w$}", fit_name(os), name_w = name_w), Style::default().fg(color::main_fg())),
                    Span::styled(format!(" {:>4}", count), Style::default().fg(color::inactive_fg())),
                    Span::styled(format!(" {}", bar), Style::default().fg(color::cyan())),
                    Span::styled(format!(" {:.0}%", pct), Style::default().fg(color::inactive_fg())),
                ]));
            }

            // ── Salt Version ──────────────────────────────────────────────
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  Salt Version:", Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD))));
            let ver_max = ver_sorted.first().map(|(_, c)| *c).unwrap_or(1).max(1);
            for (ver, count) in &ver_sorted {
                let bar_len = if bar_width > 0 { (*count * bar_width) / ver_max } else { 0 };
                let bar: String = (0..bar_len).map(|_| '\u{2588}').collect();
                let pct = *count as f64 / total_minions as f64 * 100.0;
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<name_w$}", fit_name(ver), name_w = name_w), Style::default().fg(color::main_fg())),
                    Span::styled(format!(" {:>4}", count), Style::default().fg(color::inactive_fg())),
                    Span::styled(format!(" {}", bar), Style::default().fg(color::yellow())),
                    Span::styled(format!(" {:.0}%", pct), Style::default().fg(color::inactive_fg())),
                ]));
            }

            // ── Architecture ──────────────────────────────────────────────
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  Architecture:", Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD))));
            let arch_max = arch_sorted.first().map(|(_, c)| *c).unwrap_or(1).max(1);
            for (arch, count) in &arch_sorted {
                let bar_len = if bar_width > 0 { (*count * bar_width) / arch_max } else { 0 };
                let bar: String = (0..bar_len).map(|_| '\u{2588}').collect();
                let pct = *count as f64 / total_minions as f64 * 100.0;
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<name_w$}", fit_name(arch), name_w = name_w), Style::default().fg(color::main_fg())),
                    Span::styled(format!(" {:>4}", count), Style::default().fg(color::inactive_fg())),
                    Span::styled(format!(" {}", bar), Style::default().fg(color::purple())),
                    Span::styled(format!(" {:.0}%", pct), Style::default().fg(color::inactive_fg())),
                ]));
            }
        }
        if !app.fleet_scan_completed {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  \u{25B8} Passive collection \u{2014} data may be partial",
                Style::default().fg(color::yellow()),
            )));
            lines.push(Line::from(Span::styled(
                "    Enable admin mode and press 's' for full fleet scan",
                Style::default().fg(color::inactive_fg()),
            )));
        }
    } else if app.fleet_scanning {
        lines.push(Line::from(Span::styled("  Scanning fleet...", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD))));
    } else {
        lines.push(Line::from(Span::styled("  No fleet data.", Style::default().fg(color::inactive_fg()))));
        lines.push(Line::from(Span::styled("  Activate admin mode and", Style::default().fg(color::inactive_fg()))));
        lines.push(Line::from(Span::styled("  press 's' to scan the fleet.", Style::default().fg(color::inactive_fg()))));
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::main_bg())),
        inner,
    );
}
