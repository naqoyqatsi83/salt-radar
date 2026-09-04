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
        DetailPopup::ApiRequest(entry) => {
            let area = centered_rect(65, 40, f.area());
            f.render_widget(Clear, area);
            draw_api_request_detail(f, area, entry, app.clipboard_at, app.clipboard_msg.as_ref().map(|(m, _)| m.as_str()));
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
/// `pinned_header` is how many lines at the start of `lines` stay fixed at
/// the top of the body — e.g. Event Sniffer's title/status/column-header
/// rows — instead of scrolling away with the rest; `0` means the whole
/// body scrolls together as before. `scroll` applies only to the
/// non-pinned remainder.
fn render_popup(f: &mut Frame, area: Rect, lines: &[Line<'static>], hints: &[(&str, String)], status: Option<Line<'static>>, more: (bool, bool), scroll: u16, pinned_header: usize) {
    // Clear the area first — prevents underlying content from bleeding through
    f.render_widget(Clear, area);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER_HI))
        .style(Style::default().bg(color::BG_POPUP));
    if more.0 {
        block = block.title_top(Line::from(Span::styled(" \u{25B2} more above ", Style::default().fg(color::BORDER_HI))).right_aligned());
    }
    if !hints.is_empty() {
        block = block.title_bottom(super::border_hints(hints, color::BORDER_HI).left_aligned());
    }
    if more.1 {
        block = block.title_bottom(Line::from(Span::styled(" \u{25BC} more below ", Style::default().fg(color::BORDER_HI))).right_aligned());
    }
    let inner = block.inner(area);
    f.render_widget(block, area);

    let body_area = if let Some(status_line) = status {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        f.render_widget(
            Paragraph::new(status_line).style(Style::default().bg(color::BG_POPUP)),
            chunks[1],
        );
        chunks[0]
    } else {
        inner
    };

    let pinned_header = pinned_header.min(lines.len());
    if pinned_header == 0 {
        f.render_widget(
            Paragraph::new(lines.to_vec()).scroll((scroll, 0)).style(Style::default().bg(color::BG_POPUP)),
            body_area,
        );
    } else {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(pinned_header as u16), Constraint::Min(0)])
            .split(body_area);
        f.render_widget(
            Paragraph::new(lines[..pinned_header].to_vec()).style(Style::default().bg(color::BG_POPUP)),
            split[0],
        );
        f.render_widget(
            Paragraph::new(lines[pinned_header..].to_vec()).scroll((scroll, 0)).style(Style::default().bg(color::BG_POPUP)),
            split[1],
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

/// Coloring for one line of `App::format_return_as_console`'s output —
/// mirrors Salt's own highstate-style CLI coloring (green/red result,
/// cyan keys) well enough to keep job output scannable without needing a
/// second `salt-run --force-color` subprocess call just to get ANSI codes
/// to translate back into spans.
/// Faithful port of Salt's own `nested` outputter (`salt/output/nested.py`
/// — the `NestDisplay.display` method), which is what colors
/// `salt-run jobs.lookup_jid`'s CLI output. Reimplemented here instead of
/// shelling out a second `salt-run --force-color` subprocess call per
/// lookup purely to get colored text, since the ANSI codes it produces
/// would just get parsed back into spans anyway — this builds the same
/// spans directly from the JSON return already fetched. Colors are mapped
/// onto this app's own palette (not raw ANSI) to match the surrounding UI;
/// the recursive structure (the `----------` dividers, `key:` lines,
/// `- `/`|_` list bullets, per-branch coloring, retcode-driven red) is a
/// line-for-line match to the original.
fn nested_return_lines(value: &serde_json::Value, retcode: i64) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    nested_display(value, 0, "", retcode, &mut out);
    out
}

fn nested_push(out: &mut Vec<Line<'static>>, indent: usize, fg: Color, prefix: &str, msg: &str, suffix: &str) {
    let pad = " ".repeat(indent);
    let mut spans = vec![Span::styled(format!("{}{}{}", pad, prefix, msg), Style::default().fg(fg))];
    if !suffix.is_empty() {
        spans.push(Span::styled(suffix.to_string(), Style::default().fg(color::TEXT)));
    }
    out.push(Line::from(spans));
}

fn nested_display(value: &serde_json::Value, indent: usize, prefix: &str, retcode: i64, out: &mut Vec<Line<'static>>) {
    match value {
        serde_json::Value::Null => nested_push(out, indent, color::YELLOW, prefix, "None", ""),
        serde_json::Value::Bool(b) => nested_push(out, indent, color::YELLOW, prefix, if *b { "True" } else { "False" }, ""),
        serde_json::Value::Number(n) => nested_push(out, indent, color::YELLOW, prefix, &n.to_string(), ""),
        serde_json::Value::String(s) => {
            if !s.is_empty() {
                let cont_prefix = " ".repeat(prefix.chars().count());
                for (i, line) in s.lines().enumerate() {
                    let p: &str = if i == 0 { prefix } else { &cont_prefix };
                    nested_push(out, indent, color::GREEN, p, line, "");
                }
            }
        }
        serde_json::Value::Array(arr) => {
            let list_color = if retcode != 0 { color::RED } else { color::GREEN };
            for item in arr {
                match item {
                    serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                        nested_push(out, indent, list_color, "", "|_", "");
                        let child_prefix = if matches!(item, serde_json::Value::Object(_)) { "" } else { "- " };
                        nested_display(item, indent + 2, child_prefix, retcode, out);
                    }
                    _ => nested_display(item, indent, "- ", retcode, out),
                }
            }
        }
        serde_json::Value::Object(map) => {
            let dict_color = if retcode != 0 { color::RED } else { color::CYAN };
            if indent > 0 {
                nested_push(out, indent, dict_color, "", "----------", "");
            }
            for (k, v) in map.iter() {
                nested_push(out, indent, dict_color, prefix, k, ":");
                nested_display(v, indent + 4, "", retcode, out);
            }
        }
    }
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
    lines.push(Line::from(vec![
        Span::styled("  \u{2500}\u{2500}\u{2500}\u{2500} ", Style::default().fg(Color::Rgb(35, 50, 80))),
        Span::styled("Full Grains", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {}", "\u{2500}".repeat((area.width as usize).saturating_sub(21))), Style::default().fg(Color::Rgb(35, 50, 80))),
    ]));

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
    render_popup(f, area, &lines, &hints, None, (more_above, more_below), 0, 0);
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
    render_popup(f, area, &lines, &hints, status, (more_above, more_below), 0, 0);
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
        .title_bottom(super::border_hints(&hints, color::BORDER_HI).left_aligned());
    let outer_inner = outer_block.inner(area);
    f.render_widget(outer_block, area);

    // ── Build header lines (JID, metadata) ──
    let short_jid: String = jid.chars().take(28).collect();
    let mut jid_line = vec![Span::styled(format!("  Job {}", short_jid),
        Style::default().fg(color::TEXT_TITLE).add_modifier(Modifier::BOLD))];
    // Per-lookup timing from the agent (salt-run subprocess + our own
    // post-processing) — right here in the header so a slow lookup is
    // visible without having to go dig through the agent's stderr log.
    if let Some(l) = lookup.as_ref() {
        if l.subprocess_secs > 0.0 || l.postprocess_secs > 0.0 {
            let total = l.subprocess_secs + l.postprocess_secs;
            let timing_color = if total >= 10.0 { color::ORANGE } else { color::TEXT_DIM };
            jid_line.push(Span::styled(
                format!("  (salt-run {:.1}s, parse {:.1}s)", l.subprocess_secs, l.postprocess_secs),
                Style::default().fg(timing_color),
            ));
        }
    }
    let mut header_lines: Vec<Line<'static>> = vec![Line::from(jid_line)];
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
    let left_border_color = if !panel_focus { color::BORDER_HI } else { color::BORDER };
    let left_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(left_border_color))
        .title(Line::from(super::bracket_title(vec![Span::styled(
            format!("Returns ({})", lookup.as_ref().map_or(0, |l| l.returns.len())),
            Style::default().fg(color::TEXT_TITLE).add_modifier(Modifier::BOLD),
        )], left_border_color)))
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
    let right_border_color = if panel_focus { color::BORDER_HI } else { color::BORDER };
    let right_border_style = Style::default().fg(right_border_color);
    let right_title_text = if let Some(ref l) = lookup {
        let sel = selected_return.unwrap_or(0).min(l.returns.len().saturating_sub(1));
        if sel < l.returns.len() {
            format!("Return from {}", l.returns[sel].minion_id)
        } else { "Preview".to_string() }
    } else { "Preview".to_string() };

    let mut right_lines: Vec<Line<'static>> = Vec::new();
    if let Some(ref l) = lookup {
        let sel = selected_return.unwrap_or(0).min(l.returns.len().saturating_sub(1));
        if sel < l.returns.len() {
            let ret = &l.returns[sel];
            if !ret.formatted_text.is_empty() {
                right_lines.extend(crate::app::App::ansi_text_to_lines(&ret.formatted_text, Style::default().fg(color::TEXT)));
            } else {
                // Wrap as {minion_id: full_return} so this renders exactly
                // as it would in real `salt-run jobs.lookup_jid` output,
                // where the minion ID is itself just the top-level dict key.
                let mut wrapper = serde_json::Map::new();
                wrapper.insert(ret.minion_id.clone(), ret.full_return.clone());
                // `salt-run jobs.lookup_jid` never passes an explicit `_retcode`
                // to the nested outputter (that kwarg is a `salt`/exec-module
                // concept, not a runner one) — it defaults to 0 in real Salt,
                // so the real CLI's dict/list coloring is always the success
                // colors regardless of any individual minion's own retcode.
                // Passing `ret.retcode` here colored failed minions red, which
                // doesn't happen in the actual command's output.
                right_lines.extend(nested_return_lines(&serde_json::Value::Object(wrapper), 0));
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
        .title(Line::from(super::bracket_title(vec![Span::styled(
            right_title_text,
            Style::default().fg(if panel_focus { color::CYAN } else { color::TEXT_TITLE }).add_modifier(Modifier::BOLD),
        )], right_border_color)))
        .style(Style::default().bg(color::BG_POPUP));
    if more_above {
        right_block = right_block.title_top(Line::from(Span::styled(" \u{25B2} more above ", Style::default().fg(right_border_color))).right_aligned());
    }
    if more_below {
        right_block = right_block.title_bottom(Line::from(Span::styled(" \u{25BC} more below ", Style::default().fg(right_border_color))).right_aligned());
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
    render_popup(f, area, &lines, &hints, status, (false, false), 0, 0);
}

fn draw_api_request_detail(f: &mut Frame, area: Rect, entry: &crate::types::ApiLogEntry, clipboard_at: Option<std::time::Instant>, clipboard_warning: Option<&str>) {
    let status_code = entry.response_status.split_whitespace().next().unwrap_or("?");
    let status_color = match status_code.chars().next() {
        Some('2') | Some('3') => color::GREEN,
        Some('4') | Some('5') => color::RED,
        _ => color::TEXT_DIM,
    };

    let mut lines = header_lines(" API Request ", area.width);
    lines.push(row("Method", entry.method.clone(), color::PURPLE));
    lines.push(row("Path", entry.path.clone(), color::CYAN));
    lines.push(row("Full request line", entry.raw_request_line.clone(), color::TEXT));
    lines.push(row("Client", entry.client.clone(), color::TEXT_HI));
    lines.push(row("Status", entry.response_status.clone(), status_color));
    lines.push(row("Processing time", format!("{:.3}s", entry.processing_time_secs), color::TEXT_HI));
    lines.push(row("Captured", format!("{}s ago", entry.received_at.elapsed().as_secs()), color::TEXT_DIM));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Only what cpstats records is available: no request/response body,",
        Style::default().fg(color::TEXT_DIM),
    )));
    lines.push(Line::from(Span::styled(
        "  no headers \u{2014} so no auth token, credentials, or payload content.",
        Style::default().fg(color::TEXT_DIM),
    )));

    let hints: [(&str, String); 2] = [("Esc/q", "close".to_string()), ("y", "copy".to_string())];
    let status = clipboard_status(clipboard_at, clipboard_warning);
    render_popup(f, area, &lines, &hints, status, (false, false), 0, 0);
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
    render_popup(f, area, &lines, &hints, None, (false, false), 0, 0);
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
        // Title/status/column-header rows built so far stay pinned at the
        // top of the body — see render_popup's `pinned_header` — so only
        // the event rows below scroll.
        let pinned_header = lines.len();

        for display_idx in 0..total {
            let buf_idx = display_idx;
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

        // Auto-scroll: keep selected event header visible (at ~1/3 from top
        // of the scrollable region), within just the event rows — the
        // pinned header above doesn't count against the viewport budget.
        let event_only_len = lines.len().saturating_sub(pinned_header);
        let scrollable_height = (area.height as usize).saturating_sub(3).saturating_sub(pinned_header); // borders(2) + footer(1) + pinned header
        let scroll = if sel < event_line_starts.len() {
            let sel_line = event_line_starts[sel].saturating_sub(pinned_header);
            let max_scroll = event_only_len.saturating_sub(scrollable_height);
            sel_line.saturating_sub(scrollable_height / 3).min(max_scroll) as u16
        } else {
            0
        };
        let more_above = scroll > 0;
        let more_below = (scroll as usize) + scrollable_height < event_only_len;

        let hints = sniffer_hints(state);
        let status = sniffer_status(clipboard_at, state, clipboard_warning);
        render_popup(f, area, &lines, &hints, status, (more_above, more_below), scroll, pinned_header);
        return;
    }

    lines.push(Line::from(""));
    let hints = sniffer_hints(state);
    let status = sniffer_status(clipboard_at, state, clipboard_warning);
    render_popup(f, area, &lines, &hints, status, (false, false), 0, 0);
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
