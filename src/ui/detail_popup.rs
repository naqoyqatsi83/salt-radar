use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, DetailPopup, PingState};
use crate::types::{KeyStatus, SnifferState};

use super::color;

pub fn draw_detail_popup(f: &mut Frame, app: &App) {
    let Some(ref detail) = app.detail_popup else { return };

    match detail {
        DetailPopup::Minion(minion_id, ping_state, grains, grains_scroll) => {
            let area = centered_rect(80, 70, f.area());
            f.render_widget(Clear, area);
            if let Some(minion) = app.minion_info.iter().find(|m| m.id == *minion_id) {
                draw_minion_detail(f, area, minion, ping_state, grains, *grains_scroll, app);
            }
        }
        DetailPopup::SaltEvent(evt, scroll_offset, scroll_mode) => {
            let area = centered_rect(75, 70, f.area());
            f.render_widget(Clear, area);
            draw_event_detail(f, area, evt, *scroll_offset, *scroll_mode, app.clipboard_at, app.clipboard_msg.as_ref().map(|(m, _)| m.as_str()));
        }
        DetailPopup::JobDetail { jid, lookup, selected_return, job_fun, job_tgt, job_start_epoch, minion_scroll, return_filter, return_is_filtering, panel_focus, content_scroll, content_scroll_mode, .. } => {
            let area = centered_rect(85, 85, f.area());
            f.render_widget(Clear, area);
            draw_job_detail(f, area, jid, lookup, *selected_return, job_fun, job_tgt, *job_start_epoch, *minion_scroll, return_filter, *return_is_filtering, *panel_focus, *content_scroll, *content_scroll_mode, app.clipboard_at, app.clipboard_msg.as_ref().map(|(m, _)| m.as_str()));
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

fn header_lines(title: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            format!("  {}", title),
            Style::default()
                .fg(color::TEXT_TITLE)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
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

fn dismiss_line(clipboard_at: Option<std::time::Instant>, scroll_mode: bool, clipboard_warning: Option<&str>) -> Line<'static> {
    let show_copied = clipboard_at
        .map(|t| t.elapsed().as_secs_f64() < 3.0)
        .unwrap_or(false);
    if show_copied {
        return Line::from(Span::styled(
            "  \u{2713} Copied                                ",
            Style::default().fg(super::color::GREEN).add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(warn) = clipboard_warning {
        return Line::from(Span::styled(
            format!("  \u{26a0} {}", warn),
            Style::default().fg(super::color::YELLOW).add_modifier(Modifier::BOLD),
        ));
    }
    if scroll_mode {
        Line::from(Span::styled(
            "  [ q exit scroll | \u{2191}\u{2193} \u{21C4} page scroll output ]",
            Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
        ))
    } else {
        Line::from(Span::styled(
            "  [ q/Esc close | \u{2191}\u{2193} navigate | Enter=scroll | y=copy Y=json ]",
            Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
        ))
    }
}

fn render_popup(f: &mut Frame, area: Rect, lines: &[Line<'static>], footer: Option<Line<'static>>, scroll: u16) {
    // Clear the area first — prevents underlying content from bleeding through
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER_HI))
        .style(Style::default().bg(color::BG_POPUP));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if let Some(footer_line) = footer {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        f.render_widget(
            Paragraph::new(lines.to_vec()).scroll((scroll, 0)).style(Style::default().bg(color::BG_POPUP)),
            chunks[0],
        );
        f.render_widget(
            Paragraph::new(footer_line).style(Style::default().bg(color::BG_POPUP)),
            chunks[1],
        );
    } else {
        f.render_widget(
            Paragraph::new(lines.to_vec()).scroll((scroll, 0)).style(Style::default().bg(color::BG_POPUP)),
            inner,
        );
    }
}

fn draw_minion_detail(f: &mut Frame, area: Rect, minion: &crate::types::MinionInfo, ping_state: &PingState, grains: &Option<Vec<(String, String)>>, grains_scroll: usize, app: &App) {
    let mut lines = header_lines(" Minion Detail ");
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
            "Admin mode required (press A)".to_string(),
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
        "  \u{2500}\u{2500}\u{2500}\u{2500} Full Grains \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        Style::default().fg(Color::Rgb(35, 50, 80)),
    )));

    match grains {
        None => {
            if app.admin_mode {
                lines.push(Line::from(Span::styled(
                    "  Press g to retrieve full grains from this minion",
                    Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    "  Admin mode required to fetch grains (press A)",
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
                // Compute available height for grains
                let used = lines.len();
                let avail = (area.height as usize).saturating_sub(used).saturating_sub(4); // borders(2) + footer(1) + spacing
                let scroll = grains_scroll.min(pairs.len().saturating_sub(avail));
                let end = (scroll + avail).min(pairs.len());
                for (key, val) in pairs.iter().skip(scroll).take(end.saturating_sub(scroll)) {
                    let display = format!("  {}: {}", key, val);
                    lines.push(Line::from(Span::styled(display, Style::default().fg(color::TEXT))));
                }
                if pairs.len() > avail {
                    let pct = if pairs.len() > 0 { (scroll * 100) / pairs.len() } else { 0 };
                    lines.push(Line::from(Span::styled(
                        format!("  \u{25B2}\u{25BC} scroll: {} items ({}%)", pairs.len(), pct),
                        Style::default().fg(color::TEXT_DIM),
                    )));
                }
            }
        }
    }

    let help_parts = if app.admin_mode {
        " Esc/q close | y copy | g fetch grains | \u{2191}\u{2193} scroll grains "
    } else {
        " Esc/q close | y copy | A admin mode for grains "
    };
    let footer = Line::from(Span::styled(
        format!("  [{}]", help_parts),
        Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
    ));
    render_popup(f, area, &lines, Some(footer), 0);
}

fn draw_event_detail(f: &mut Frame, area: Rect, evt: &crate::types::SaltEvent, scroll_offset: usize, scroll_mode: bool, clipboard_at: Option<std::time::Instant>, clipboard_warning: Option<&str>) {
    let mut lines = header_lines(" Event Detail ");
    lines.push(row("Tag", evt.tag.clone(), color::CYAN));
    lines.push(row("Timestamp", format_ts(evt.ts), color::TEXT_DIM));
    lines.push(Line::from(""));

    let json_str = serde_json::to_string_pretty(&evt.data).unwrap_or_default();
    let data_lines: Vec<&str> = json_str.lines().collect();
    let total_data = data_lines.len();

    // Clamp scroll_offset so the last line can appear at viewport bottom
    let viewport_h = (area.height as usize).saturating_sub(6); // borders(2) + footer(1) + header(3)
    let max_scroll = total_data.saturating_sub(viewport_h);
    let effective_scroll = if scroll_mode {
        scroll_offset.min(max_scroll)
    } else {
        0
    };

    for line in data_lines.iter().skip(effective_scroll) {
        lines.push(Line::from(Span::styled(line.to_string(), Style::default().fg(color::TEXT))));
    }

    let footer = dismiss_line(clipboard_at, scroll_mode, clipboard_warning);
    render_popup(f, area, &lines, Some(footer), 0);
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
    clipboard_at: Option<std::time::Instant>,
    clipboard_warning: Option<&str>,
) {
    // ── Outer frame ──
    f.render_widget(Clear, area);
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER_HI))
        .style(Style::default().bg(color::BG_POPUP));
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

    // ── Layout: header / body / footer ──
    let vert = Layout::vertical([
        Constraint::Length(header_h),
        Constraint::Min(1),
        Constraint::Length(1),
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
        .title_style(Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD))
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
    let right_block = Block::default()
        .borders(Borders::ALL)
        .border_style(right_border_style)
        .title(right_title)
        .title_style(Style::default().fg(if panel_focus { color::CYAN } else { color::TEXT_LABEL }).add_modifier(Modifier::BOLD))
        .style(Style::default().bg(color::BG_POPUP));
    let right_inner = right_block.inner(horiz[1]);
    f.render_widget(&right_block, horiz[1]);

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
    // Right panel scroll
    let right_scroll = if content_scroll_mode {
        content_scroll as u16
    } else if content_scroll > 0 {
        content_scroll as u16
    } else { 0 };
    f.render_widget(
        Paragraph::new(right_lines)
            .scroll((right_scroll, 0))
            .style(Style::default().bg(color::BG_POPUP)),
        right_inner,
    );

    // ── Footer ──
    let footer = dismiss_line_job_detail(clipboard_at, return_is_filtering, panel_focus, content_scroll_mode, clipboard_warning);
    f.render_widget(
        Paragraph::new(footer).style(Style::default().bg(color::BG_POPUP)),
        vert[2],
    );
}

fn dismiss_line_job_detail(clipboard_at: Option<std::time::Instant>, filter_active: bool, right_focus: bool, scroll_mode: bool, clipboard_warning: Option<&str>) -> Line<'static> {
    let show_copied = clipboard_at
        .map(|t| t.elapsed().as_secs_f64() < 3.0)
        .unwrap_or(false);
    if show_copied {
        return Line::from(Span::styled(
            "  \u{2713} Copied                                ",
            Style::default().fg(super::color::GREEN).add_modifier(Modifier::BOLD),
        ));
    }
    // Show clipboard warnings in the popup footer where the user is looking
    if let Some(warn) = clipboard_warning {
        return Line::from(Span::styled(
            format!("  \u{26a0} {}", warn),
            Style::default().fg(super::color::YELLOW).add_modifier(Modifier::BOLD),
        ));
    }
    if filter_active {
        return Line::from(Span::styled(
            "  [ Esc cancel filter | Enter confirm | type to filter ]",
            Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
        ));
    }
    if right_focus {
        if scroll_mode {
            Line::from(Span::styled(
                "  [ Esc back to list | q exit scroll | arrows/PgUp/PgDn/Home/End scroll | y=copy Y=json ]",
                Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
            ))
        } else {
            Line::from(Span::styled(
                "  [ Esc back to list | Enter=scroll | arrows scroll | y=copy Y=json ]",
                Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
            ))
        }
    } else {
        Line::from(Span::styled(
            "  [ q/Esc close | j/k navigate | Enter=view | /=filter | y=copy Y=json ]",
            Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
        ))
    }
}

fn draw_alert_detail(f: &mut Frame, area: Rect, alert: &crate::types::AlertEntry, clipboard_at: Option<std::time::Instant>, clipboard_warning: Option<&str>) {
    let sev_color = match alert.severity {
        crate::types::AlertSeverity::Info => color::CYAN,
        crate::types::AlertSeverity::Warn => color::YELLOW,
        crate::types::AlertSeverity::Crit => color::RED,
    };
    let mut lines = header_lines(" Alert Detail ");
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
    let show_copied = clipboard_at
        .map(|t| t.elapsed().as_secs_f64() < 3.0)
        .unwrap_or(false);
    let footer = if show_copied {
        Line::from(Span::styled(
            "  \u{2713} Copied                                ",
            Style::default().fg(super::color::GREEN).add_modifier(Modifier::BOLD),
        ))
    } else if let Some(warn) = clipboard_warning {
        Line::from(Span::styled(
            format!("  \u{26a0} {}", warn),
            Style::default().fg(super::color::YELLOW).add_modifier(Modifier::BOLD),
        ))
    } else {
        Line::from(Span::styled(
            "  [ Esc/q close | y=copy ]",
            Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
        ))
    };
    render_popup(f, area, &lines, Some(footer), 0);
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
    render_popup(f, area, &lines, None, 0);
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

    let mut lines = header_lines(&title);

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
            "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
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

        let footer = sniffer_footer(clipboard_at, state, clipboard_warning);
        render_popup(f, area, &lines, Some(footer), scroll);
        return;
    }

    lines.push(Line::from(""));
    let footer = sniffer_footer(clipboard_at, state, clipboard_warning);
    render_popup(f, area, &lines, Some(footer), 0);
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

fn sniffer_footer(
    clipboard_at: Option<std::time::Instant>,
    state: &SnifferState,
    clipboard_warning: Option<&str>,
) -> Line<'static> {
    if let Some(ref buf) = state.highlight_input {
        let cursor = if buf.len() % 2 == 0 { '|' } else { ' ' };
        let display = if buf.is_empty() {
            String::from("  Highlight keywords (comma-separated, Enter=confirm, Esc=cancel): ")
        } else {
            format!("  Highlight keywords: {} {}", buf, cursor)
        };
        return Line::from(Span::styled(
            display,
            Style::default().fg(super::color::YELLOW).add_modifier(Modifier::BOLD),
        ));
    }
    let show_copied = clipboard_at
        .map(|t| t.elapsed().as_secs_f64() < 3.0)
        .unwrap_or(false);
    if show_copied {
        return Line::from(Span::styled(
            "  \u{2713} Copied                                ",
            Style::default().fg(super::color::GREEN).add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(warn) = clipboard_warning {
        return Line::from(Span::styled(
            format!("  \u{26a0} {}", warn),
            Style::default().fg(super::color::YELLOW).add_modifier(Modifier::BOLD),
        ));
    }
    let hint = if state.paused {
        "  [ q/Esc close | \u{2191}\u{2193} | Enter=detail | h=highlight | p=resume | y=copy ]"
    } else {
        "  [ q/Esc close | \u{2191}\u{2193} | Enter=detail | h=highlight | p=pause | y=copy ]"
    };
    Line::from(Span::styled(
        hint,
        Style::default().fg(Color::Rgb(65, 80, 110)).add_modifier(Modifier::ITALIC),
    ))
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
