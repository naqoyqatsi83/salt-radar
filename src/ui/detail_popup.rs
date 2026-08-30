use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, DetailPopup, PingState};
use crate::types::{KeyStatus, SnifferState};

use super::color;

pub fn draw_detail_popup(f: &mut Frame, app: &mut App) {
    let Some(ref detail) = app.detail_popup else { return };

    match detail {
        DetailPopup::Minion(minion_id, ping_state, grains, grains_scroll) => {
            let area = centered_rect(80, 70, f.area());
            f.render_widget(Clear, area);
            if let Some(minion) = app.minion_info.iter().find(|m| m.id == *minion_id) {
                draw_minion_detail(f, area, minion, ping_state, grains, *grains_scroll, app);
            }
        }
        DetailPopup::SaltEvent(evt, scroll_offset) => {
            let area = centered_rect(75, 70, f.area());
            f.render_widget(Clear, area);
            let highlight = app.return_to_minion_events.as_ref().map(|(id, _)| id.as_str());
            let clipboard_at = app.clipboard_at;
            let clipboard_msg = app.clipboard_msg.as_ref().map(|(m, _)| m.as_str());
            draw_event_detail(f, area, evt, *scroll_offset, &mut app.event_detail_scroll_max, clipboard_at, clipboard_msg, highlight);
        }
        DetailPopup::JobDetail { jid, lookup, selected_return, job_fun, job_tgt, job_start_epoch, minion_scroll, return_filter, return_is_filtering, panel_focus, content_scroll, content_scroll_mode, .. } => {
            let area = centered_rect(85, 85, f.area());
            f.render_widget(Clear, area);
            let clipboard_at = app.clipboard_at;
            let clipboard_msg = app.clipboard_msg.as_ref().map(|(m, _)| m.as_str());
            draw_job_detail(f, area, jid, lookup, *selected_return, job_fun, job_tgt, *job_start_epoch, *minion_scroll, return_filter, *return_is_filtering, *panel_focus, *content_scroll, *content_scroll_mode, &mut app.job_detail_scroll_max, clipboard_at, clipboard_msg);
        }
        DetailPopup::Alert(alert) => {
            let area = centered_rect(70, if alert.details.is_empty() { 40 } else { 70 }, f.area());
            f.render_widget(Clear, area);
            draw_alert_detail(f, area, alert, app.clipboard_at, app.clipboard_msg.as_ref().map(|(m, _)| m.as_str()));
        }
        DetailPopup::AdminConfirm => {
            let area = centered_rect(55, 35, f.area());
            f.render_widget(Clear, area);
            draw_admin_confirm(f, area);
        }
        DetailPopup::EventSniffer(state) => {
            let area = centered_rect(80, 85, f.area());
            f.render_widget(Clear, area);
            draw_event_sniffer(f, area, state, app.clipboard_at, app.high_contrast, app.clipboard_msg.as_ref().map(|(m, _)| m.as_str()));
        }
        DetailPopup::MinionEvents { minion_id, selected } => {
            super::minion_events::draw_minion_events(f, f.area(), app, minion_id, *selected);
        }
        DetailPopup::ConfigEditor(_) => {
            super::config_editor::draw_config_editor(f, app);
        }
    }
}

pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let pad_v = (100u16.saturating_sub(percent_y)) / 2;
    let pad_h = (100u16.saturating_sub(percent_x)) / 2;
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(pad_v),
            Constraint::Percentage(percent_y),
            Constraint::Percentage(pad_v),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(pad_h),
            Constraint::Percentage(percent_x),
            Constraint::Percentage(pad_h),
        ])
        .split(vertical[1])[1]
}

fn header_lines(title: &str, area_width: u16) -> Vec<Line<'static>> {
    let rule = "\u{2500}".repeat((area_width as usize).saturating_sub(2));
    vec![
        Line::from(Span::styled(
            format!("  {}", title),
            Style::default()
                .fg(color::TEXT_TITLE)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("  {}", rule),
            Style::default().fg(Color::Rgb(35, 50, 80)),
        )),
        Line::from(""),
    ]
}

fn row(label: &'static str, value: String, value_color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {:<16}", label),
            Style::default().fg(color::TEXT_LABEL),
        ),
        Span::styled(value, Style::default().fg(value_color).add_modifier(Modifier::BOLD)),
    ])
}

fn event_detail_hints() -> Vec<(&'static str, String)> {
    vec![
        ("q/Esc", "close".to_string()),
        ("\u{2191}\u{2193}", "scroll".to_string()),
        ("PgUp/PgDn", "page".to_string()),
        ("Home/End", "top/bottom".to_string()),
        ("y", "copy".to_string()),
        ("Y", "json".to_string()),
    ]
}

/// Shared popup chrome: shortcut hints go in the block's own border
/// (btop-style, same `border_hints` every other panel uses) instead of a
/// content-row footer; `status` is reserved for genuinely transient content
/// a border hint can't show (a live-typed buffer, a "Copied" confirmation,
/// a clipboard warning) and only consumes a row when actually present.
fn render_popup(f: &mut Frame, area: Rect, lines: &[Line<'static>], hints: &[(&str, String)], status: Option<Line<'static>>, more: (bool, bool), scroll: u16) {
    // Clear the area first — prevents underlying content from bleeding through
    f.render_widget(Clear, area);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER_HI))
        .style(Style::default().bg(color::BG_POPUP));
    if more.0 {
        block = block.title_top(Line::from(Span::styled(" \u{25B2} more above ", Style::default().fg(color::TEXT_DIM))).right_aligned());
    }
    if !hints.is_empty() {
        block = block.title_bottom(super::border_hints(hints).left_aligned());
    }
    if more.1 {
        block = block.title_bottom(Line::from(Span::styled(" \u{25BC} more below ", Style::default().fg(color::TEXT_DIM))).right_aligned());
    }
    let inner = block.inner(area);
    f.render_widget(block, area);

    if let Some(status_line) = status {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        f.render_widget(
            Paragraph::new(lines.to_vec()).scroll((scroll, 0)).style(Style::default().bg(color::BG_POPUP)),
            chunks[0],
        );
        f.render_widget(
            Paragraph::new(status_line).style(Style::default().bg(color::BG_POPUP)),
            chunks[1],
        );
    } else {
        f.render_widget(
            Paragraph::new(lines.to_vec()).scroll((scroll, 0)).style(Style::default().bg(color::BG_POPUP)),
            inner,
        );
    }
}

/// "✓ Copied" / clipboard-warning transient status shared by every popup
/// footer — `None` when neither applies, so callers can tell there's
/// nothing to show in the status row.
fn clipboard_status(clipboard_at: Option<std::time::Instant>, clipboard_warning: Option<&str>) -> Option<Line<'static>> {
    let show_copied = clipboard_at
        .map(|t| t.elapsed().as_secs_f64() < 3.0)
        .unwrap_or(false);
    if show_copied {
        return Some(Line::from(Span::styled(
            "  \u{2713} Copied                                ",
            Style::default().fg(super::color::GREEN).add_modifier(Modifier::BOLD),
        )));
    }
    if let Some(warn) = clipboard_warning {
        return Some(Line::from(Span::styled(
            format!("  \u{26a0} {}", warn),
            Style::default().fg(super::color::YELLOW).add_modifier(Modifier::BOLD),
        )));
    }
    None
}

/// Lightweight, allocation-cheap coloring for a single line of a grain
/// value — the agent pretty-prints nested lists/dicts as JSON, so a plain
/// per-token parse isn't needed, just enough of a heuristic (quoted
/// strings, booleans/null, numbers, bare structural punctuation) to make
/// a wall of grains scannable at a glance.
fn grain_value_style(line: &str) -> Style {
    let trimmed = line.trim().trim_end_matches(',');
    if trimmed == "true" {
        Style::default().fg(color::GREEN)
    } else if trimmed == "false" {
        Style::default().fg(color::RED)
    } else if trimmed == "null" {
        Style::default().fg(color::TEXT_DIM)
    } else if matches!(trimmed, "[" | "]" | "{" | "}" | "[]" | "{}") {
        Style::default().fg(color::TEXT_DIM)
    } else if trimmed.starts_with('"') {
        Style::default().fg(color::TEXT)
    } else if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-') {
        Style::default().fg(color::YELLOW)
    } else {
        Style::default().fg(color::TEXT)
    }
}

/// Rendered line count for one grain: 1 for a scalar (`key: value` on one
/// line), or 1 + the value's own line count for a pretty-printed nested
/// list/dict (a `key:` line followed by each indented value line).
fn grain_height(val: &str) -> usize {
    if val.contains('\n') { 1 + val.lines().count() } else { 1 }
}

fn draw_minion_detail(f: &mut Frame, area: Rect, minion: &crate::types::MinionInfo, ping_state: &PingState, grains: &Option<Vec<(String, String)>>, grains_scroll: usize, app: &App) {
    let mut lines = header_lines(" Minion Detail ", area.width);
    lines.push(row("ID", minion.id.clone(), color::CYAN));

    let key_status_text = match minion.key_status {
        Some(KeyStatus::Accepted) => "accepted".to_string(),
        Some(KeyStatus::Pending) => "pending".to_string(),
        Some(KeyStatus::Rejected) => "rejected".to_string(),
        Some(KeyStatus::Denied) => "denied".to_string(),
        None => "unknown".to_string(),
    };
    let key_status_color = match minion.key_status {
        Some(KeyStatus::Accepted) => color::GREEN,
        Some(KeyStatus::Pending) => color::YELLOW,
        Some(KeyStatus::Rejected) | Some(KeyStatus::Denied) => color::RED,
        None => color::TEXT_DIM,
    };
    lines.push(row("Key Status", key_status_text, key_status_color));

    // Connection status — depends on PingState
    let (connected_text, connected_color) = match *ping_state {
        PingState::Idle => (
            "Admin mode required (press 'a')".to_string(),
            color::TEXT_DIM,
        ),
        PingState::Pinging(started) => {
            if started.elapsed().as_secs() > 15 {
                ("test.ping timed out".to_string(), color::RED)
            } else {
                ("test.ping in progress...".to_string(), color::YELLOW)
            }
        },
        PingState::Result(true) => ("connected".to_string(), color::GREEN),
        PingState::Result(false) => ("disconnected".to_string(), color::RED),
    };
    lines.push(row("Connection", connected_text, connected_color));

    // OS information
    let os_display = if !minion.osfinger.is_empty() {
        minion.osfinger.clone()
    } else if !minion.osrelease.is_empty() {
        format!("{} {}", minion.os, minion.osrelease)
    } else {
        minion.os.clone()
    };
    lines.push(row("OS", os_display, color::GREEN));
    if !minion.osfinger.is_empty() && !minion.osrelease.is_empty() {
        lines.push(row("OS Release", minion.osrelease.clone(), color::TEXT));
    }

    // Grains information
    lines.push(row("Salt Version", minion.salt_version.clone(), color::YELLOW));
    lines.push(row("CPU Arch", minion.cpuarch.clone(), color::TEXT));

    // Last seen information
    lines.push(row("Last Seen", format_epoch(minion.last_seen_epoch), color::TEXT_DIM));

    // ── Grains section ────────────────────────────────────────────────────
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  \u{2500}\u{2500}\u{2500}\u{2500} Full Grains {}", "\u{2500}".repeat((area.width as usize).saturating_sub(21))),
        Style::default().fg(Color::Rgb(35, 50, 80)),
    )));

    let mut more_above = false;
    let mut more_below = false;
    match grains {
        None => {
            if app.admin_mode {
                lines.push(Line::from(Span::styled(
                    "  Press 'g' to retrieve full grains from this minion",
                    Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    "  Admin mode required to fetch grains (press 'a')",
                    Style::default().fg(color::TEXT_DIM),
                )));
            }
        }
        Some(pairs) => {
            if pairs.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  Loading grains...",
                    Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
                )));
            } else {
                // Compute available height for grains. Scroll is by grain
                // (key), not by rendered line, since a nested value (a list
                // or dict, pretty-printed by the agent) can span many lines.
                // max_scroll is found by walking backward from the last
                // grain accumulating line-heights, so scrolling to the end
                // lands the last rendered line at the bottom of the frame
                // instead of pinning the last grain to the top with blank
                // space below it (and further scrolling has nowhere to go).
                let used = lines.len();
                let avail = (area.height as usize).saturating_sub(used).saturating_sub(2); // borders(2)
                let mut max_scroll = 0usize;
                let mut budget = avail;
                for (i, (_, val)) in pairs.iter().enumerate().rev() {
                    let h = grain_height(val);
                    if h > budget {
                        max_scroll = i + 1;
                        break;
                    }
                    budget -= h;
                }
                let scroll = grains_scroll.min(max_scroll);

                let mut rendered = 0usize;
                let mut shown = 0usize;
                for (key, val) in pairs.iter().skip(scroll) {
                    if rendered >= avail { break; }
                    let mut val_lines = val.lines();
                    let first = val_lines.next().unwrap_or("");
                    if val.contains('\n') {
                        lines.push(Line::from(Span::styled(
                            format!("  {}:", key),
                            Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD),
                        )));
                        rendered += 1;
                        for l in std::iter::once(first).chain(val_lines) {
                            if rendered >= avail { break; }
                            lines.push(Line::from(Span::styled(format!("    {}", l), grain_value_style(l))));
                            rendered += 1;
                        }
                    } else {
                        lines.push(Line::from(vec![
                            Span::styled(format!("  {}: ", key), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)),
                            Span::styled(first.to_string(), grain_value_style(first)),
                        ]));
                        rendered += 1;
                    }
                    shown += 1;
                }
                more_above = scroll > 0;
                more_below = scroll + shown < pairs.len();
            }
        }
    }

    let hints: Vec<(&str, String)> = if app.admin_mode {
        vec![
            ("Esc/q", "close".to_string()),
            ("y", "copy".to_string()),
            ("g", "fetch grains".to_string()),
            ("\u{2191}\u{2193}", "scroll grains".to_string()),
            ("PgUp/PgDn", "page".to_string()),
            ("Home/End", "top/bottom".to_string()),
            ("a", "admin off".to_string()),
        ]
    } else {
        vec![("Esc/q", "close".to_string()), ("y", "copy".to_string()), ("a", "admin mode for grains".to_string())]
    };
    render_popup(f, area, &lines, &hints, None, (more_above, more_below), 0);
}

fn draw_event_detail(f: &mut Frame, area: Rect, evt: &crate::types::SaltEvent, scroll_offset: usize, scroll_max: &mut usize, clipboard_at: Option<std::time::Instant>, clipboard_warning: Option<&str>, highlight: Option<&str>) {
    let mut lines = header_lines(" Event Detail ", area.width);
    lines.push(row("Tag", evt.tag.clone(), color::CYAN));
    lines.push(row("Timestamp", format_ts(evt.ts), color::TEXT_DIM));
    lines.push(Line::from(""));

    let json_str = serde_json::to_string_pretty(&evt.data).unwrap_or_default();
    let data_lines: Vec<&str> = json_str.lines().collect();
    let total_data = data_lines.len();

    // Clamp scroll_offset so the last line can appear at viewport bottom, and
    // write the true bound back so the key handler can clamp Down/PageDown/End
    // against it directly next time instead of guessing at a raw line count.
    let viewport_h = (area.height as usize).saturating_sub(5); // borders(2) + header(3)
    let max_scroll = total_data.saturating_sub(viewport_h);
    *scroll_max = max_scroll;
    let effective_scroll = scroll_offset.min(max_scroll);
    let more_above = effective_scroll > 0;
    let more_below = effective_scroll + viewport_h < total_data;

    for line in data_lines.iter().skip(effective_scroll) {
        if let Some(h) = highlight {
            if !h.is_empty() && line.contains(h) {
                let mut spans = Vec::new();
                let mut remaining = *line;
                while let Some(pos) = remaining.find(h) {
                    let before = &remaining[..pos];
                    if !before.is_empty() {
                        spans.push(Span::styled(before.to_string(), Style::default().fg(color::TEXT)));
                    }
                    spans.push(Span::styled(h.to_string(), Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)));
                    remaining = &remaining[pos + h.len()..];
                }
                if !remaining.is_empty() {
                    spans.push(Span::styled(remaining.to_string(), Style::default().fg(color::TEXT)));
                }
                lines.push(Line::from(spans));
                continue;
            }
        }
        lines.push(Line::from(Span::styled(line.to_string(), Style::default().fg(color::TEXT))));
    }

    let hints = event_detail_hints();
    let status = clipboard_status(clipboard_at, clipboard_warning);
    render_popup(f, area, &lines, &hints, status, (more_above, more_below), 0);
}

fn draw_job_detail(
    f: &mut Frame,
    area: Rect,
    jid: &str,
    lookup: &Option<crate::types::JidLookup>,
    selected_return: Option<usize>,
    job_fun: &str,
    job_tgt: &str,
    job_start_epoch: f64,
    minion_scroll: usize,
    return_filter: &str,
    return_is_filtering: bool,
    panel_focus: bool,
    content_scroll: usize,
    content_scroll_mode: bool,
    content_scroll_max: &mut usize,
    clipboard_at: Option<std::time::Instant>,
    clipboard_warning: Option<&str>,
) {
    // ── Outer frame ──
    f.render_widget(Clear, area);
    let hints = job_detail_hints(return_is_filtering, panel_focus, content_scroll_mode);
    let status = clipboard_status(clipboard_at, clipboard_warning);
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER_HI))
        .style(Style::default().bg(color::BG_POPUP))
        .title_bottom(super::border_hints(&hints).left_aligned());
    let outer_inner = outer_block.inner(area);
    f.render_widget(outer_block, area);

    // ── Build header lines (JID, metadata) ──
    let short_jid: String = jid.chars().take(28).collect();
    let mut header_lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(format!("  Job {}", short_jid),
            Style::default().fg(color::TEXT_TITLE).add_modifier(Modifier::BOLD))),
    ];
    header_lines.push(Line::from(""));
    let epoch_str = if job_start_epoch > 0.0 {
        use chrono::{DateTime, Local};
        if let Some(utc) = DateTime::from_timestamp(job_start_epoch as i64, 0) {
            let local: DateTime<Local> = utc.into();
            format!(" UTC {}  local {}", utc.format("%H:%M:%S"), local.format("%H:%M:%S"))
        } else { String::new() }
    } else { String::new() };
    let returns_str = lookup.as_ref().map_or(String::new(), |l| format!("  Returns: {}", l.returns.len()));
    header_lines.push(Line::from(vec![
        Span::styled(format!("  Fun: "), Style::default().fg(color::TEXT_LABEL)),
        Span::styled(job_fun.to_string(), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)),
        Span::styled(format!("  Tgt: "), Style::default().fg(color::TEXT_LABEL)),
        Span::styled(job_tgt.to_string(), Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled(epoch_str, Style::default().fg(color::TEXT_DIM)),
        Span::styled(returns_str, Style::default().fg(color::TEXT_LABEL)),
    ]));
    let header_h = (header_lines.len() + 1) as u16; // +1 for spacing after header

    // ── Layout: header / body / status (only when there's something transient to show) ──
    let vert = Layout::vertical([
        Constraint::Length(header_h),
        Constraint::Min(1),
        Constraint::Length(if status.is_some() { 1 } else { 0 }),
    ])
    .split(outer_inner);

    // Render header
    f.render_widget(
        Paragraph::new(header_lines).style(Style::default().bg(color::BG_POPUP)),
        vert[0],
    );

    // ── Split body: left (returns list) / right (preview) ──
    let horiz = Layout::horizontal([
        Constraint::Ratio(25, 100),
        Constraint::Ratio(75, 100),
    ])
    .split(vert[1]);

    // ── Build LEFT panel: compact return list ──
    let left_border_style = if !panel_focus {
        Style::default().fg(color::BORDER_HI)
    } else {
        Style::default().fg(color::BORDER)
    };
    let left_block = Block::default()
        .borders(Borders::ALL)
        .border_style(left_border_style)
        .title(format!(" Returns ({}) ", lookup.as_ref().map_or(0, |l| l.returns.len())))
        .title_style(Style::default().fg(color::TEXT_TITLE).add_modifier(Modifier::BOLD))
        .style(Style::default().bg(color::BG_POPUP));
    let left_inner = left_block.inner(horiz[0]);
    f.render_widget(&left_block, horiz[0]);

    let mut left_lines: Vec<Line<'static>> = Vec::new();
    if let Some(ref l) = lookup {
        let sel = selected_return.unwrap_or(0).min(l.returns.len().saturating_sub(1));
        if l.returns.is_empty() {
            left_lines.push(Line::from(Span::styled(
                "  No returns yet.",
                Style::default().fg(color::TEXT_DIM),
            )));
        } else {
            for (i, ret) in l.returns.iter().enumerate() {
                if !return_filter.is_empty() && !return_is_filtering && !ret.minion_id.contains(return_filter) {
                    continue;
                }
                let cursor = if i == sel { "\u{25b6}" } else { " " };
                let (live_retcode, live_success) = crate::app::derive_success_retcode(&ret.full_return);
                let status = if live_success { "OK" } else { "FAIL" };
                let sc = if live_success { color::GREEN } else { color::RED };
                left_lines.push(Line::from(vec![
                    Span::styled(format!(" {} {}: ", cursor, ret.minion_id),
                        Style::default().fg(if i == sel { color::CYAN } else { color::TEXT })
                            .add_modifier(if i == sel { Modifier::BOLD } else { Modifier::empty() })),
                    Span::styled(status, Style::default().fg(sc).add_modifier(Modifier::BOLD)),
                    Span::styled(format!(" (retcode={})", live_retcode), Style::default().fg(color::TEXT_DIM)),
                ]));
            }
        }
        // Compute position of sel within the filtered view for scroll
        let left_list_h = left_inner.height as usize;
        let left_scroll = if minion_scroll > 0 { minion_scroll as u16 } else { 0 };
        let (sel_vis_idx, is_filtered) = if return_filter.is_empty() || return_is_filtering {
            (sel, false)
        } else {
            let vis = l.returns.iter().take(sel + 1).filter(|ret| ret.minion_id.contains(return_filter)).count().saturating_sub(1);
            (vis, true)
        };
        if left_lines.is_empty() {
            left_lines.push(Line::from(Span::styled(
                "  No matching hosts.",
                Style::default().fg(color::TEXT_DIM),
            )));
        }
        let left_scroll_adj = if sel_vis_idx > left_list_h.saturating_sub(2) {
            (sel_vis_idx.saturating_sub(left_list_h.saturating_sub(2))) as u16
        } else if is_filtered {
            // Filter confirmed: minion_scroll is an unfiltered index, not valid for
            // the filtered left_lines. The selected item is already visible, so start at 0.
            0
        } else {
            left_scroll.min(left_lines.len().saturating_sub(1) as u16)
        };
        f.render_widget(
            Paragraph::new(left_lines)
                .scroll((left_scroll_adj, 0))
                .style(Style::default().bg(color::BG_POPUP)),
            left_inner,
        );
    } else {
        left_lines.push(Line::from(Span::styled(
            "  Querying master...",
            Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
        )));
        f.render_widget(
            Paragraph::new(left_lines)
                .style(Style::default().bg(color::BG_POPUP)),
            left_inner,
        );
    }

    // Filter bar overlay if active
    if return_is_filtering {
        let filter_bar = Paragraph::new(Line::from(Span::styled(
            format!("  Filter: {}_", return_filter),
            Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(color::BG_POPUP));
        // Draw filter bar at bottom of left panel
        let filter_area = Layout::vertical([Constraint::Min(1), Constraint::Length(1)])
            .split(left_inner)[1];
        f.render_widget(filter_bar, filter_area);
    }

    // ── Build RIGHT panel: return content preview ──
    let right_border_style = if panel_focus {
        Style::default().fg(color::BORDER_HI)
    } else {
        Style::default().fg(color::BORDER)
    };
    let right_title = if let Some(ref l) = lookup {
        let sel = selected_return.unwrap_or(0).min(l.returns.len().saturating_sub(1));
        if sel < l.returns.len() {
            format!(" Return from {} ", l.returns[sel].minion_id)
        } else { " Preview ".to_string() }
    } else { " Preview ".to_string() };

    let mut right_lines: Vec<Line<'static>> = Vec::new();
    if let Some(ref l) = lookup {
        let sel = selected_return.unwrap_or(0).min(l.returns.len().saturating_sub(1));
        if sel < l.returns.len() {
            let ret = &l.returns[sel];
            let display_str = if !ret.formatted_text.is_empty() {
                ret.formatted_text.clone()
            } else {
                App::format_return_as_console(&ret.minion_id, &ret.full_return)
            };
            for line in display_str.lines() {
                let spans = crate::app::App::ansi_to_styled_spans(line, Style::default().fg(color::TEXT));
                right_lines.push(Line::from(spans));
            }
        } else {
            right_lines.push(Line::from(Span::styled(
                "  No return data.",
                Style::default().fg(color::TEXT_DIM),
            )));
        }
    } else {
        right_lines.push(Line::from(Span::styled(
            "  Querying Salt master...",
            Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
        )));
    }

    // Viewport-aware max scroll — a `Borders::ALL` block always insets 1 on
    // each side, so this matches what `right_block.inner()` will produce
    // without needing the block built first just to ask it. Written back so
    // the key handler clamps Down/PageDown/End against the real bound
    // instead of scrolling past the last line.
    let right_viewport_h = (horiz[1].height as usize).saturating_sub(2);
    let max_scroll = right_lines.len().saturating_sub(right_viewport_h);
    *content_scroll_max = max_scroll;
    let effective_scroll = content_scroll.min(max_scroll);
    let more_above = effective_scroll > 0;
    let more_below = effective_scroll + right_viewport_h < right_lines.len();

    let mut right_block = Block::default()
        .borders(Borders::ALL)
        .border_style(right_border_style)
        .title(right_title)
        .title_style(Style::default().fg(if panel_focus { color::CYAN } else { color::TEXT_TITLE }).add_modifier(Modifier::BOLD))
        .style(Style::default().bg(color::BG_POPUP));
    if more_above {
        right_block = right_block.title_top(Line::from(Span::styled(" \u{25B2} more above ", Style::default().fg(color::TEXT_DIM))).right_aligned());
    }
    if more_below {
        right_block = right_block.title_bottom(Line::from(Span::styled(" \u{25BC} more below ", Style::default().fg(color::TEXT_DIM))).right_aligned());
    }
    let right_inner = right_block.inner(horiz[1]);
    f.render_widget(&right_block, horiz[1]);

    f.render_widget(
        Paragraph::new(right_lines)
            .scroll((effective_scroll as u16, 0))
            .style(Style::default().bg(color::BG_POPUP)),
        right_inner,
    );

    // ── Status (transient only — shortcuts are in the outer border) ──
    if let Some(status_line) = status {
        f.render_widget(
            Paragraph::new(status_line).style(Style::default().bg(color::BG_POPUP)),
            vert[2],
        );
    }
}

fn job_detail_hints(filter_active: bool, right_focus: bool, scroll_mode: bool) -> Vec<(&'static str, String)> {
    if filter_active {
        return vec![("Esc", "cancel filter".to_string()), ("Enter", "confirm".to_string())];
    }
    if right_focus {
        if scroll_mode {
            vec![
                ("Esc/q", "exit scroll".to_string()),
                ("\u{2191}\u{2193}", "scroll".to_string()),
                ("PgUp/PgDn", "page".to_string()),
                ("Home/End", "top/bottom".to_string()),
                ("y", "copy".to_string()),
                ("Y", "json".to_string()),
            ]
        } else {
            vec![
                ("Esc/q", "back to list".to_string()),
                ("Enter", "scroll".to_string()),
                ("\u{2191}\u{2193}", "scroll".to_string()),
                ("PgUp/PgDn", "page".to_string()),
                ("Home/End", "top/bottom".to_string()),
                ("y", "copy".to_string()),
                ("Y", "json".to_string()),
            ]
        }
    } else {
        vec![
            ("q/Esc", "close".to_string()),
            ("\u{2191}\u{2193}", "navigate".to_string()),
            ("PgUp/PgDn", "page".to_string()),
            ("Home/End", "top/bottom".to_string()),
            ("Enter", "view".to_string()),
            ("/", "filter".to_string()),
            ("y", "copy".to_string()),
            ("Y", "json".to_string()),
        ]
    }
}

fn draw_alert_detail(f: &mut Frame, area: Rect, alert: &crate::types::AlertEntry, clipboard_at: Option<std::time::Instant>, clipboard_warning: Option<&str>) {
    let sev_color = match alert.severity {
        crate::types::AlertSeverity::Info => color::CYAN,
        crate::types::AlertSeverity::Warn => color::YELLOW,
        crate::types::AlertSeverity::Crit => color::RED,
    };
    let mut lines = header_lines(" Alert Detail ", area.width);
    lines.push(row("Severity", alert.severity.label().to_string(), sev_color));
    lines.push(row("Rule", alert.rule.to_string(), color::CYAN));
    lines.push(row("Description", alert.description.clone(), color::TEXT_HI));
    lines.push(Line::from(""));

    if !alert.details.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  Items ({}):", alert.details.len()),
            Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD),
        )));
        for detail in &alert.details {
            lines.push(Line::from(Span::styled(
                format!("    {}", detail),
                Style::default().fg(color::TEXT),
            )));
        }
    }

    lines.push(Line::from(""));
    let hints: [(&str, String); 2] = [("Esc/q", "close".to_string()), ("y", "copy".to_string())];
    let status = clipboard_status(clipboard_at, clipboard_warning);
    render_popup(f, area, &lines, &hints, status, (false, false), 0);
}

fn draw_admin_confirm(f: &mut Frame, area: Rect) {
    let mut lines = vec![
        Line::from(Span::styled(
            "  \u{26a0} DANGER ZONE ",
            Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  ADMIN MODE",
            Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  You are about to enable admin mode.",
            Style::default().fg(color::TEXT_HI),
        )),
        Line::from(Span::styled(
            "  Actions performed in this mode may have",
            Style::default().fg(color::TEXT),
        )),
        Line::from(Span::styled(
            "  consequences on your Salt infrastructure.",
            Style::default().fg(color::TEXT),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Only proceed if you know what you are doing.",
            Style::default().fg(color::ORANGE).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    // Simulated buttons
    lines.push(Line::from(vec![
        Span::styled("     [ ", Style::default().fg(color::TEXT_DIM)),
        Span::styled("OK", Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(" ] ", Style::default().fg(color::TEXT_DIM)),
        Span::styled("Enter", Style::default().fg(color::TEXT_DIM).add_modifier(Modifier::ITALIC)),
        Span::styled("          ", Style::default().fg(color::TEXT_DIM)),
        Span::styled("[ ", Style::default().fg(color::TEXT_DIM)),
        Span::styled("Cancel", Style::default().fg(color::RED).add_modifier(Modifier::BOLD)),
        Span::styled(" ] ", Style::default().fg(color::TEXT_DIM)),
        Span::styled("Esc/q/a", Style::default().fg(color::TEXT_DIM).add_modifier(Modifier::ITALIC)),
    ]));
    let hints: [(&str, String); 2] = [("Enter", "OK".to_string()), ("Esc/q/a", "cancel".to_string())];
    render_popup(f, area, &lines, &hints, None, (false, false), 0);
}

fn draw_event_sniffer(
    f: &mut Frame,
    area: Rect,
    state: &SnifferState,
    clipboard_at: Option<std::time::Instant>,
    high_contrast: bool,
    clipboard_warning: Option<&str>,
) {
    let ids_str = state.minion_ids.join(", ");
    let title = if ids_str.len() > 50 {
        format!(" Event Sniffer: {}... ", &ids_str[..47])
    } else {
        format!(" Event Sniffer: {} ", ids_str)
    };

    let mut lines = header_lines(&title, area.width);

    // Status bar
    let (status_label, status_color) = if state.paused {
        ("PAUSED", color::YELLOW)
    } else {
        ("LIVE", color::GREEN)
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!("  Status: {}", status_label),
            Style::default().fg(status_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  Captured: {} / max 1000", state.buffer.len()),
            Style::default().fg(color::TEXT_DIM),
        ),
    ]));
    lines.push(Line::from(""));

    if state.buffer.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No matching events yet. Waiting for new events...",
            Style::default().fg(color::TEXT_DIM),
        )));
    } else {
        // Column widths: # | Time | JID | Type
        let num_w: usize = 8;
        let time_w: usize = 22;
        let type_w: usize = 18;
        let jid_w = (area.width as usize).saturating_sub(num_w + time_w + type_w + 4);

        // Column header
        lines.push(Line::from(vec![
            Span::styled(format!("  {:>nw$} ", "#", nw = num_w - 3), Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:<tw$}", "Time", tw = time_w), Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:<jw$}", "JID", jw = jid_w), Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:<tyw$}", "Type", tyw = type_w), Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(Span::styled(
            format!("  {}", "\u{2500}".repeat((area.width as usize).saturating_sub(2))),
            Style::default().fg(Color::Rgb(35, 50, 80)),
        )));

        let total = state.buffer.len();
        let sel = state.selected.min(total.saturating_sub(1));
        let mut event_line_starts: Vec<usize> = Vec::with_capacity(total);
        // Popup inner width = area width minus left+right borders
        let inner_w = (area.width as usize).saturating_sub(2);

        for display_idx in 0..total {
            let buf_idx = total - 1 - display_idx;
            let evt = match state.buffer.get(buf_idx) {
                Some(e) => e,
                None => break,
            };

            event_line_starts.push(lines.len());

            let is_sel = display_idx == sel;
            let sel_style = if is_sel {
                color::select_bg(high_contrast)
            } else {
                Style::default()
            };

            let event_num = display_idx + 1;
            let t = sniffer_format_ts(evt.ts);
            let etype = evt.data["fun"]
                .as_str()
                .or_else(|| {
                    if evt.tag.contains("/ret/") { Some("return") } else { None }
                })
                .unwrap_or("");

            let tag_disp = if evt.tag.len() > jid_w {
                format!("{}..", &evt.tag[..jid_w.saturating_sub(2)])
            } else {
                evt.tag.clone()
            };

            // Header row: # | Time | JID | Type
            let mut header_spans = vec![
                Span::styled(format!("  {:>nw$} ", event_num, nw = num_w - 3), Style::default().fg(color::TEXT_DIM)),
                Span::styled(format!("{:<tw$}", t, tw = time_w), Style::default().fg(color::TEXT_DIM)),
                Span::styled(format!("{:<jw$}", tag_disp, jw = jid_w), Style::default().fg(color::CYAN)),
                Span::styled(format!("{:<tyw$}", etype, tyw = type_w), Style::default().fg(color::GREEN)),
            ];
            if is_sel {
                let hdr_w: usize = header_spans.iter().map(|s| s.content.len()).sum();
                let pad = inner_w.saturating_sub(hdr_w);
                if pad > 0 {
                    header_spans.push(Span::styled(" ".repeat(pad), sel_style));
                }
            }
            lines.push(Line::from(header_spans).style(sel_style));

            // Full JSON body (no truncation) with keyword highlighting
            let json_str = serde_json::to_string_pretty(&evt.data).unwrap_or_default();
            let indent = format!("{:w$}", "", w = num_w + time_w + 3);
            if !state.highlight_keywords.is_empty() {
                for raw_line in json_str.lines() {
                    let text = format!("{}{}", indent, raw_line);
                    let mut spans = highlight_keywords(&text, &state.highlight_keywords);
                    if is_sel {
                        let pad = inner_w.saturating_sub(text.len());
                        if pad > 0 {
                            spans.push(Span::styled(" ".repeat(pad), sel_style));
                        }
                    }
                    lines.push(Line::from(spans).style(sel_style));
                }
            } else {
                for line in json_str.lines() {
                    let text = format!("{}{}", indent, line);
                    let style = Style::default().fg(color::TEXT);
                    if is_sel {
                        let pad = inner_w.saturating_sub(text.len());
                        lines.push(Line::from(vec![
                            Span::styled(text, style),
                            Span::styled(" ".repeat(pad), sel_style),
                        ]).style(sel_style));
                    } else {
                        lines.push(Line::from(Span::styled(text, style)));
                    }
                }
            }
        }

        // Auto-scroll: keep selected event header visible (at ~1/3 from top)
        let content_height = (area.height as usize).saturating_sub(3); // borders(2) + footer(1)
        let scroll = if sel < event_line_starts.len() {
            let sel_line = event_line_starts[sel];
            let max_scroll = lines.len().saturating_sub(content_height);
            sel_line.saturating_sub(content_height / 3).min(max_scroll) as u16
        } else {
            0
        };

        let hints = sniffer_hints(state);
        let status = sniffer_status(clipboard_at, state, clipboard_warning);
        render_popup(f, area, &lines, &hints, status, (false, false), scroll);
        return;
    }

    lines.push(Line::from(""));
    let hints = sniffer_hints(state);
    let status = sniffer_status(clipboard_at, state, clipboard_warning);
    render_popup(f, area, &lines, &hints, status, (false, false), 0);
}

/// Split a line of text into styled spans, highlighting all occurrences of any keyword.
fn highlight_keywords(text: &str, keywords: &[String]) -> Vec<Span<'static>> {
    if keywords.is_empty() {
        return vec![Span::styled(text.to_string(), Style::default().fg(color::TEXT))];
    }
    let lower_text = text.to_lowercase();
    // Collect match positions (start, end) for each keyword occurrence
    let mut matches: Vec<(usize, usize)> = Vec::new();
    for kw in keywords {
        if kw.is_empty() { continue; }
        let lower_kw = kw.to_lowercase();
        let mut start = 0;
        while let Some(pos) = lower_text[start..].find(&lower_kw) {
            let abs_start = start + pos;
            let abs_end = abs_start + kw.len();
            matches.push((abs_start, abs_end));
            start = abs_end;
        }
    }
    if matches.is_empty() {
        return vec![Span::styled(text.to_string(), Style::default().fg(color::TEXT))];
    }
    // Sort by start position and merge overlapping ranges
    matches.sort_by_key(|m| m.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for m in &matches {
        if let Some(last) = merged.last_mut() {
            if m.0 <= last.1 {
                last.1 = last.1.max(m.1);
                continue;
            }
        }
        merged.push(*m);
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut pos = 0;
    for (start, end) in &merged {
        if *start > pos {
            spans.push(Span::styled(text[pos..*start].to_string(), Style::default().fg(color::TEXT)));
        }
        spans.push(Span::styled(
            text[*start..*end].to_string(),
            Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
        ));
        pos = *end;
    }
    if pos < text.len() {
        spans.push(Span::styled(text[pos..].to_string(), Style::default().fg(color::TEXT)));
    }
    spans
}

fn sniffer_format_ts(ts: f64) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "--:--:--".to_string())
}

/// Live-typed highlight-keyword buffer, or the shared copy/warning status —
/// the only things a static border hint can't show.
fn sniffer_status(
    clipboard_at: Option<std::time::Instant>,
    state: &SnifferState,
    clipboard_warning: Option<&str>,
) -> Option<Line<'static>> {
    if let Some(ref buf) = state.highlight_input {
        let cursor = if buf.len() % 2 == 0 { '|' } else { ' ' };
        let display = if buf.is_empty() {
            String::from("  Highlight keywords (comma-separated, Enter=confirm, Esc=cancel): ")
        } else {
            format!("  Highlight keywords: {} {}", buf, cursor)
        };
        return Some(Line::from(Span::styled(
            display,
            Style::default().fg(super::color::YELLOW).add_modifier(Modifier::BOLD),
        )));
    }
    clipboard_status(clipboard_at, clipboard_warning)
}

fn sniffer_hints(state: &SnifferState) -> Vec<(&'static str, String)> {
    vec![
        ("q/Esc", "close".to_string()),
        ("\u{2191}\u{2193}", "navigate".to_string()),
        ("PgUp/PgDn", "page".to_string()),
        ("Home/End", "top/bottom".to_string()),
        ("Enter", "detail".to_string()),
        ("h", "highlight".to_string()),
        ("p", if state.paused { "resume".to_string() } else { "pause".to_string() }),
        ("y", "copy".to_string()),
    ]
}

fn format_epoch(secs: u64) -> String {
    if secs == 0 {
        "unknown".to_string()
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let ago = now.saturating_sub(secs);
        if ago < 60 {
            format!("{}s ago", ago)
        } else if ago < 3600 {
            format!("{}m ago", ago / 60)
        } else if ago < 86400 {
            format!("{}h ago", ago / 3600)
        } else {
            format!("{}d ago", ago / 86400)
        }
    }
}

fn format_ts(ts: f64) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|dt| dt.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "--:--:--".to_string())
}
