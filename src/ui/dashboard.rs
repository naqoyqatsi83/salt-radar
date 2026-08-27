use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Sparkline, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::types::SparklineStyle;

use super::color;

const RADAR_FRAMES: &[char] = &['\u{25d4}', '\u{25d1}', '\u{25d5}', '\u{25d0}'];
// Frames: ◔ ◑ ◕ ◐

// Services / Minion Key Status / Event Types horizontal split, out of 5 parts.
// Key Status only ever shows 4 fixed-format count rows + a churn line, so it
// needs far less width than Services (gauge + per-role mem + service list) or
// Event Types; used both for the actual row layout and for estimating the
// Services column's wrapped height below, which must stay in sync with it.
const MID_ROW_RATIO: [u32; 3] = [2, 1, 2];

pub fn draw_dashboard(f: &mut Frame, area: Rect, app: &App) {
    // Small terminal fallback: vertical stacked layout
    if area.height < 26 {
        draw_dashboard_compact(f, area, app);
        return;
    }

    // Header line at the very top
    let header_body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);
    draw_header(f, header_body[0], app);

    let body = header_body[1];

    // Top section: fixed-height info panels
    // Bottom section: stacked full-width sparklines fill remaining space
    let key_status_h = 8u16; // 4 key rows + separator + total + 2 border
    // Services panel is the first of a 3-way horizontal split of `body`; mirror
    // that split just for its width so the proc-row estimate matches what
    // draw_proc_row actually renders with, rather than assuming a fixed 1 line.
    let services_col_width = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(MID_ROW_RATIO[0], 5),
            Constraint::Ratio(MID_ROW_RATIO[1], 5),
            Constraint::Ratio(MID_ROW_RATIO[2], 5),
        ])
        .split(Rect::new(0, 0, body.width, 1))[0]
        .width
        .saturating_sub(2); // block borders
    let proc_rows = proc_rows_for(&group_processes(&app.processes), services_col_width);
    let services_h = (4 + proc_rows + app.services.len() as u16).min(25); // gauge + proc lines + header + N services + 2 border
    let mid_h = key_status_h.max(services_h);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),  // Host Resources
            Constraint::Length(mid_h), // Services + Key Status (exact-fit)
            Constraint::Length(4),  // Job Pulse (KPI line + up to 3 jobs)
            Constraint::Min(6),     // Sparklines (at least 6 lines = 3×2 for 3 panels)
        ])
        .split(body);

    draw_host_row(f, sections[0], app);

    // Services + Minion key status + Event breakdown horizontal split
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(MID_ROW_RATIO[0], 5),
            Constraint::Ratio(MID_ROW_RATIO[1], 5),
            Constraint::Ratio(MID_ROW_RATIO[2], 5),
        ])
        .split(sections[1]);
    draw_proc_row(f, mid[0], app);
    draw_key_status(f, mid[1], app);
    draw_event_breakdown(f, mid[2], app);

    draw_job_pulse(f, sections[2], app);

    // Bottom: 3 stacked full-width sparklines for time-axis correlation
    let sparklines = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .split(sections[3]);

    draw_zmq_row(f, sparklines[0], app);
    draw_events_row(f, sparklines[1], app);
    draw_activity_matrix(f, sparklines[2], app);
}

/// Fallback for small terminals: vertical stacked layout (same as original)
fn draw_dashboard_compact(f: &mut Frame, area: Rect, app: &App) {
    let proc_rows = proc_rows_for(&group_processes(&app.processes), area.width.saturating_sub(2));
    let services_height = 4 + proc_rows + app.services.len() as u16;
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(5),
            Constraint::Length(services_height),
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(4),
        ])
        .split(area);

    draw_header(f, layout[0], app);
    draw_host_row(f, layout[1], app);
    draw_proc_row(f, layout[2], app);
    draw_zmq_row(f, layout[3], app);
    draw_events_row(f, layout[4], app);
    draw_activity_matrix(f, layout[5], app);
    draw_job_pulse(f, layout[6], app);
}

fn make_block_line(title: Line<'static>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER))
        .title(title)
        .style(Style::default().bg(color::BG_DARK))
}

fn count_keys_with_status(keys: &[crate::types::KeyInfo], status: crate::types::KeyStatus) -> usize {
    keys.iter().filter(|k| k.status == status).count()
}

fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{}h {:02}m {:02}s", h, m, s)
    } else if m > 0 {
        format!("{}m {:02}s", m, s)
    } else {
        format!("{}s", s)
    }
}

fn pct_gauge_style(pct: f64) -> Style {
    Style::default().fg(
        if pct > 80.0 { color::ORANGE }
        else if pct > 55.0 { color::YELLOW }
        else { color::GREEN }
    ).bg(color::BG_MID)
}

// ── Header ───────────────────────────────────────────────────────────────────

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let connected = app.agent.connected;
    let dot = if connected { "\u{25cf}" } else { "\u{25cb}" };
    let dot_color = if connected { color::GREEN } else { color::RED };
    let radar = RADAR_FRAMES[app.header_radar_frame as usize];

    let version = app.config_snapshot.as_ref()
        .map(|c| c.salt_version.as_str())
        .unwrap_or("?");

    let hostname = app.config_snapshot.as_ref()
        .map(|c| c.hostname.as_str())
        .unwrap_or("?");

    let app_uptime = app.session_start.elapsed().as_secs();
    let host_uptime = app.system_info.uptime_secs;

    let mut uptime_parts = vec![format_duration(app_uptime)];
    if host_uptime > app_uptime {
        uptime_parts.push(format_uptime(host_uptime - app_uptime));
    }
    let uptime_display = if uptime_parts.len() == 2 {
        format!("{} | host {} ", uptime_parts[0], uptime_parts[1])
    } else {
        format!("{} ", uptime_parts[0])
    };

    let line = Line::from(vec![
        Span::styled(" salt-radar", Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" v{} ", env!("CARGO_PKG_VERSION")), Style::default().fg(color::TEXT_DIM)),
        Span::styled(format!("{}", radar), Style::default().fg(color::CYAN)),
        Span::styled(format!(" {} ", dot), Style::default().fg(dot_color).add_modifier(Modifier::BOLD)),
        Span::styled(format!("Salt {}", version), Style::default().fg(color::TEXT).add_modifier(Modifier::BOLD)),
        Span::styled("  \u{2502}", Style::default().fg(color::BORDER)),
        Span::styled(" host: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{}  ", hostname), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)),
        Span::styled("\u{2502}", Style::default().fg(color::BORDER)),
        Span::styled(" uptime: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{}", uptime_display), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)),
    ]);

    f.render_widget(Paragraph::new(line).style(Style::default().bg(color::BG_DARK).fg(color::TEXT)), area);
}

// ── Minion Key Status ───────────────────────────────────────────────────────

fn draw_key_status(f: &mut Frame, area: Rect, app: &App) {
    let accepted = count_keys_with_status(&app.keys, crate::types::KeyStatus::Accepted);
    let pending = count_keys_with_status(&app.keys, crate::types::KeyStatus::Pending);
    let rejected = count_keys_with_status(&app.keys, crate::types::KeyStatus::Rejected);
    let denied = count_keys_with_status(&app.keys, crate::types::KeyStatus::Denied);

    let churn = app.pending_key_churn_per_min();
    let title = Line::from(vec![
        Span::styled(" Minion Key Status ", Style::default().fg(color::TEXT_TITLE)),
        Span::styled(
            format!("churn:{}/min", churn),
            Style::default().fg(if churn > 0 { color::YELLOW } else { color::TEXT_DIM }),
        ),
    ]);
    let block = make_block_line(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let cols: [(u16, &str, usize, Color); 4] = [
        (1, "Accepted", accepted, color::GREEN),
        (2, "Denied", denied, if denied > 0 { color::RED } else { color::TEXT_DIM }),
        (3, "Unaccepted", pending, if pending > 0 { color::YELLOW } else { color::TEXT_DIM }),
        (4, "Rejected", rejected, if rejected > 0 { color::RED } else { color::TEXT_DIM }),
    ];

    let mut lines = Vec::new();
    for (n, label, count, fg) in &cols {
        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", n), Style::default().fg(color::TEXT_LABEL)),
            Span::styled(format!("{:<15}", *label), Style::default().fg(color::TEXT)),
            Span::styled(format!("{}", count), Style::default().fg(*fg).add_modifier(Modifier::BOLD)),
        ]));
    }

    // total keys as last row
    let total_keys = accepted + pending + rejected + denied;
    if total_keys > 0 {
        lines.push(Line::from(Span::styled(
            "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}".to_string(),
            Style::default().fg(color::BORDER),
        )));
        lines.push(Line::from(vec![
            Span::styled("    Total keys  ", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{}", total_keys), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)),
        ]));
    }

    f.render_widget(Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)), inner);
}

// ── Processes & Services ─────────────────────────────────────────────────────

/// Group processes by role for the Services panel's per-role memory line.
/// Returns (role_label, total_rss_kb, process_count) in first-seen order.
fn group_processes(processes: &[crate::types::SaltProcess]) -> Vec<(String, u64, usize)> {
    let mut groups: Vec<(String, u64, usize)> = vec![];
    for p in processes {
        let label = match p.role {
            crate::types::SaltProcessRole::Master => "Master",
            crate::types::SaltProcessRole::API => "API",
            crate::types::SaltProcessRole::Worker => "Worker",
            crate::types::SaltProcessRole::Syndic => "Syndic",
            crate::types::SaltProcessRole::Minion => "Minion",
            crate::types::SaltProcessRole::EventPublisher => "EventPub",
            crate::types::SaltProcessRole::ReqServer => "ReqSrv",
            crate::types::SaltProcessRole::Unknown => "?",
        };
        if let Some((_, total, count)) = groups.iter_mut().find(|(l, _, _)| l == label) {
            *total += p.rss_kb;
            *count += 1;
        } else {
            groups.push((label.to_string(), p.rss_kb, 1));
        }
    }
    groups
}

fn proc_group_text(label: &str, rss_kb: u64, count: usize) -> String {
    let mb = rss_kb / 1024;
    if count > 1 {
        format!("{} x{}: {}MB  ", label, count, mb)
    } else {
        format!("{}: {}MB  ", label, mb)
    }
}

/// How many lines the wrapped per-role memory breakdown needs at a given
/// panel width, so callers can reserve the right amount of vertical space
/// instead of letting it silently get clipped (or clipping something below
/// it). Capped at 3: a pathological number of role groups still degrades
/// gracefully rather than eating the whole panel.
fn proc_rows_for(groups: &[(String, u64, usize)], width: u16) -> u16 {
    let text_len: usize = groups.iter().map(|(l, kb, c)| proc_group_text(l, *kb, *c).len()).sum();
    let row_width = width.max(1) as usize;
    (text_len.div_ceil(row_width)).clamp(1, 3) as u16
}

fn draw_proc_row(f: &mut Frame, area: Rect, app: &App) {
    let total_rss_used: u64 = app.processes.iter().map(|p| p.rss_kb).sum();
    let mem_total_kb = app.system_info.mem_total_kb.max(1);
    let mem_pct = (total_rss_used as f64 / mem_total_kb as f64) * 100.0;

    let proc_count = app.processes.len();

    let mem_gauge_fg = if mem_pct > 80.0 { color::ORANGE } else if mem_pct > 55.0 { color::YELLOW } else { color::GREEN };
    let title = Line::from(vec![
        Span::styled(" Services ", Style::default().fg(color::TEXT_TITLE)),
        Span::styled(format!("Mem: {:.0}% ", mem_pct), Style::default().fg(mem_gauge_fg)),
    ]);
    let block = make_block_line(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Per-process memory breakdown -- built before the layout so its wrapped
    // height (it can span multiple lines once there are enough role groups,
    // e.g. once a co-located Minion role is present alongside Master/Worker/
    // ReqSrv/EventPub) is known when carving up the block's rows.
    let proc_groups = group_processes(&app.processes);
    let proc_rows = proc_rows_for(&proc_groups, inner.width);
    let mut proc_spans: Vec<Span<'static>> = vec![];
    for (label, rss_kb, count) in &proc_groups {
        let color = match label.as_str() {
            "API" => color::PURPLE,
            "Master" => color::GREEN,
            "Worker" => color::CYAN,
            _ => color::TEXT,
        };
        proc_spans.push(Span::styled(proc_group_text(label, *rss_kb, *count), Style::default().fg(color)));
    }

    let inner_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(proc_rows), Constraint::Min(0)])
        .split(inner);

    let workers_label = match app.worker_busy_pct() {
        Some(pct) => format!("  Workers {:.0}% busy", pct),
        None => "  Workers --".to_string(),
    };
    let gauge = Gauge::default()
        .gauge_style(pct_gauge_style(mem_pct))
        .percent((mem_pct.round() as u16).min(100))
        .label(format!(
            "Salt RSS {} MB ({} proc)  {} minions{}",
            total_rss_used / 1024, proc_count, app.filtered_minions_len(), workers_label,
        ));

    let mut service_lines: Vec<Line<'static>> = vec![];

    service_lines.push(Line::from(vec![
        Span::styled("Unit                    ", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        Span::styled("Status", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        Span::styled(" State      ", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        Span::styled(" Flp", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        Span::styled(" Memory ", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        Span::styled(" T", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        Span::styled(" Uptime", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
    ]));

    for service in &app.services {
        let status_icon = if service.active { "\u{25cf}" } else { "\u{25cb}" };
        let status_color = if service.active { color::GREEN } else { color::RED };

        // Number and unit suffix are formatted into their own fixed-width
        // sub-fields (rather than right-aligning the combined string) so the
        // unit letters land in the same column across rows regardless of how
        // many digits the value has.
        let memory_mb = service.memory_kb / 1024;
        let memory_text = if memory_mb > 0 {
            format!("{:>3} MB", memory_mb)
        } else {
            format!("{:>6}", "--")
        };

        let uptime_text = if service.uptime_seconds > 0 {
            let secs = service.uptime_seconds;
            if secs >= 3600 {
                format!("{:>5}h", secs / 3600)
            } else if secs >= 60 {
                format!("{:>5}m", secs / 60)
            } else {
                format!("{:>5}s", secs)
            }
        } else {
            format!("{:>6}", "--")
        };

        let unit_display = if service.unit.len() > 24 {
            format!("{}..", &service.unit[..22])
        } else {
            service.unit.clone()
        };

        let state_display = if service.state.len() > 11 {
            format!("{}..", &service.state[..9])
        } else {
            service.state.clone()
        };

        // Restart/flap count over the last 24h (see update_service_trends in app.rs).
        let flap_count = app.service_restart_count_24h(&service.unit);
        let flap_text = if flap_count > 0 { flap_count.to_string() } else { "-".to_string() };
        let flap_color = if flap_count as u64 >= app.config.service_flap_threshold_24h {
            color::RED
        } else if flap_count > 0 {
            color::YELLOW
        } else {
            color::TEXT_DIM
        };

        // Memory growth trend indicator, from a rolling linear-regression slope.
        let (trend_char, trend_color) = match app.service_memory_growth(&service.unit) {
            Some((rate_mb_per_hour, still_growing)) if still_growing && rate_mb_per_hour > app.config.service_mem_growth_mb_per_hour => {
                ("\u{2191}", color::RED) // sustained leak-rate growth
            }
            Some((rate_mb_per_hour, _)) if rate_mb_per_hour > 1.0 => ("\u{2197}", color::YELLOW), // mild growth
            Some((rate_mb_per_hour, _)) if rate_mb_per_hour < -1.0 => ("\u{2193}", color::GREEN), // shrinking
            Some(_) => ("\u{2192}", color::TEXT_DIM), // flat
            None => ("\u{00b7}", color::TEXT_DIM), // not enough samples yet
        };

        service_lines.push(Line::from(vec![
            Span::styled(format!("{:<24}", unit_display), Style::default().fg(color::CYAN)),
            Span::styled(format!("{:<6}", status_icon), Style::default().fg(status_color)),
            Span::styled(format!(" {:<11}", state_display), Style::default().fg(color::TEXT)),
            Span::styled(format!(" {:>3}", flap_text), Style::default().fg(flap_color)),
            Span::styled(format!(" {} ", memory_text), Style::default().fg(color::YELLOW)),
            Span::styled(format!(" {}", trend_char), Style::default().fg(trend_color)),
            Span::styled(format!(" {}", uptime_text), Style::default().fg(color::TEXT_DIM)),
        ]));
    }

    f.render_widget(gauge, inner_layout[0]);

    f.render_widget(
        Paragraph::new(Line::from(proc_spans))
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(color::BG_DARK)),
        inner_layout[1],
    );

    f.render_widget(
        Paragraph::new(service_lines).style(Style::default().bg(color::BG_DARK)),
        inner_layout[2],
    );
}

// ── Host Resources ───────────────────────────────────────────────────────────

fn format_bytes(bps: f64) -> String {
    if bps >= 1_000_000_000.0 {
        format!("{:.1} GB/s", bps / 1_000_000_000.0)
    } else if bps >= 1_000_000.0 {
        format!("{:.1} MB/s", bps / 1_000_000.0)
    } else if bps >= 1_000.0 {
        format!("{:.1} KB/s", bps / 1_000.0)
    } else {
        format!("{:.0} B/s", bps)
    }
}

fn format_bytes_total(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.1} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{} B", bytes)
    }
}

fn draw_host_row(f: &mut Frame, area: Rect, app: &App) {
    let mem_total_kb = app.system_info.mem_total_kb.max(1);
    let mem_avail_kb = app.system_info.mem_avail_kb;
    let mem_pct = ((mem_total_kb - mem_avail_kb) as f64 / mem_total_kb as f64) * 100.0;

    let cpu_count = app.config_snapshot.as_ref().map(|c| c.cpu_count).unwrap_or(1) as f64;
    let load_pct = (app.system_info.load_1 / cpu_count * 100.0).min(100.0).max(0.0) as u16;

    let net_rate_str = format!(
        "Net: \u{25bc}{} \u{25b2}{}",
        format_bytes(app.system_info.network_rx_rate),
        format_bytes(app.system_info.network_tx_rate),
    );

    let title = Line::from(vec![
        Span::styled(" Host Resources ", Style::default().fg(color::TEXT_TITLE)),
        Span::styled(
            format!("Load: {:.1}/{:.1}/{:.1}  Mem: {:.0}%  {}", app.system_info.load_1, app.system_info.load_5, app.system_info.load_15, mem_pct, net_rate_str),
            Style::default().fg(color::TEXT_DIM),
        ),
    ]);
    let block = make_block_line(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let inner_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 3), Constraint::Ratio(1, 3), Constraint::Ratio(1, 3)])
        .split(inner);

    let load_gauge = Gauge::default()
        .gauge_style(pct_gauge_style(load_pct as f64))
        .percent(load_pct)
        .label(format!(
            "Load: {:.1}/{:.1}/{:.1}  ({}-core host  {:.0}%)",
            app.system_info.load_1, app.system_info.load_5, app.system_info.load_15,
            cpu_count as u32, load_pct,
        ));

    let mem_total_mb = mem_total_kb / 1024;
    let mem_used_mb = (mem_total_kb.saturating_sub(mem_avail_kb)) / 1024;
    let mem_gauge = Gauge::default()
        .gauge_style(pct_gauge_style(mem_pct))
        .percent((mem_pct.round() as u16).min(100))
        .label(format!(
            "Mem: {} MB / {} MB ({:.0}%)  Cache disk: {:.0}%  PKI disk: {:.0}%",
            mem_used_mb, mem_total_mb, mem_pct,
            app.system_info.cache_disk_usage_pct,
            app.system_info.pki_disk_usage_pct,
        ));

    let net_rx_rate = app.system_info.network_rx_rate;
    let net_tx_rate = app.system_info.network_tx_rate;
    let net_rx_total = app.system_info.network_rx_bytes;
    let net_tx_total = app.system_info.network_tx_bytes;
    let net_activity_pct = ((net_rx_rate + net_tx_rate) / 1_000_000_000.0 * 100.0).min(100.0) as u16;
    let net_color = if net_rx_rate + net_tx_rate > 1_000_000.0 { color::CYAN } else { color::TEXT_DIM };
    let net_gauge = Gauge::default()
        .gauge_style(Style::default().fg(net_color).bg(color::BG_MID))
        .percent(net_activity_pct)
        .label(format!(
            "\u{25bc} {}  \u{25b2} {}  (total: {} / {})",
            format_bytes(net_rx_rate),
            format_bytes(net_tx_rate),
            format_bytes_total(net_rx_total),
            format_bytes_total(net_tx_total),
        ));

    f.render_widget(load_gauge, inner_layout[0]);
    f.render_widget(mem_gauge, inner_layout[1]);
    f.render_widget(net_gauge, inner_layout[2]);
}

fn format_uptime(secs: u64) -> String {
    if secs < 60 { return "0s".to_string(); }
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let minutes = (secs % 3600) / 60;
    let mut parts = vec![];
    if days > 0 { parts.push(format!("{}d", days)); }
    if hours > 0 { parts.push(format!("{}h", hours)); }
    if minutes > 0 { parts.push(format!("{}m", minutes)); }
    parts.join(" ")
}

// ── ZMQ / Network & Latency ─────────────────────────────────────────────────

fn draw_zmq_row(f: &mut Frame, area: Rect, app: &App) {
    let lat = app.zmq_latency_ms;
    let lat_color = if lat <= 0.0 {
        color::TEXT_DIM
    } else if lat < 10.0 {
        color::GREEN
    } else if lat < 50.0 {
        color::YELLOW
    } else {
        color::ORANGE
    };

    let peak_lat = app.zmq_latency_history.iter().cloned().fold(0.0_f64, f64::max);

    // Build title — include peak if the panel is wide enough
    let title = if area.width >= 40 {
        Line::from(vec![
            Span::styled(" ZMQ ", Style::default().fg(color::TEXT_TITLE)),
            Span::styled("\u{25bc} ", Style::default().fg(color::CYAN)),
            Span::styled(format!("{:.1}ms ", lat.max(0.0)), Style::default().fg(lat_color).add_modifier(Modifier::BOLD)),
            Span::styled("pk:", Style::default().fg(color::TEXT_LABEL)),
            Span::styled(format!("{:.1}ms", peak_lat), Style::default().fg(color::TEXT_DIM)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" ZMQ ", Style::default().fg(color::TEXT_TITLE)),
            Span::styled("\u{25bc} ", Style::default().fg(color::CYAN)),
            Span::styled(format!("{:.1}ms", lat.max(0.0)), Style::default().fg(lat_color).add_modifier(Modifier::BOLD)),
        ])
    };

    let block = make_block_line(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let pub_port = app.config_snapshot.as_ref().map(|c| c.publish_port).unwrap_or(4505);
    let ret_port = app.config_snapshot.as_ref().map(|c| c.ret_port).unwrap_or(4506);
    let api_port = app.config_snapshot.as_ref().map(|c| c.api_port).unwrap_or(0);

    let pub_established = app.zmq_connections.iter().filter(|c| c.local_port == pub_port && c.state == "ESTABLISHED").count();
    let pub_total = app.zmq_connections.iter().filter(|c| c.local_port == pub_port).count();
    let ret_established = app.zmq_connections.iter().filter(|c| c.local_port == ret_port && c.state == "ESTABLISHED").count();
    let ret_total = app.zmq_connections.iter().filter(|c| c.local_port == ret_port).count();

    let lat_label = if lat <= 0.0 {
        "measuring...".to_string()
    } else {
        format!("{:.0} ms", lat)
    };

    // Full-width sparkline with stats bar below
    let inner_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), if inner.height >= 3 { Constraint::Length(1) } else { Constraint::Length(0) }])
        .split(inner);

    let max_lat = app.zmq_latency_history.iter().cloned().fold(0.0_f64, f64::max).max(1.0);
    let lat_idx = app.zmq_latency_idx;
    let ordered_lat: Vec<f64> = (0..2000)
        .map(|i| app.zmq_latency_history[(lat_idx + i) % 2000])
        .collect();
    let spark_width = inner_layout[0].width.max(1) as usize;
    let points = spark_width.min(2000);
    let start = ordered_lat.len().saturating_sub(points);
    let spark_data: Vec<u64> = (0..points)
        .map(|i| {
            let v = ordered_lat[start + i];
            (v / max_lat * 100.0) as u64
        })
        .collect();
    render_sparkline(f, inner_layout[0], &spark_data, Style::default().fg(lat_color), app.sparkline_style);

    // Stats bar below sparkline
    let mut spans: Vec<Span<'static>> = vec![
        Span::styled("  ".to_string(), Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!(":{} pub{} ({})  ", pub_port, pub_established, pub_total), Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(":".to_string(), Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{} ret{} ({})  ", ret_port, ret_established, ret_total), Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD)),
    ];
    if api_port > 0 {
        let api_estab = app.zmq_connections.iter().filter(|c| c.local_port == api_port && c.state == "ESTABLISHED").count();
        let api_total = app.zmq_connections.iter().filter(|c| c.local_port == api_port).count();
        spans.push(Span::styled(format!(":{} api{} ({})  ", api_port, api_estab, api_total), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)));
    }
    spans.push(Span::styled("\u{2502}  ".to_string(), Style::default().fg(color::BORDER)));
    spans.push(Span::styled("\u{25b6} ".to_string(), Style::default().fg(color::TEXT_LABEL)));
    spans.push(Span::styled(lat_label, Style::default().fg(lat_color).add_modifier(Modifier::BOLD)));

    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(color::BG_DARK)),
        inner_layout[1],
    );
}

// ── Event Type Breakdown ────────────────────────────────────────────────────

fn draw_event_breakdown(f: &mut Frame, area: Rect, app: &App) {
    let title = Line::from(Span::styled(" Event Types ", Style::default().fg(color::TEXT_TITLE)));
    let block = make_block_line(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let bd = &app.event_breakdown;
    let cats = [
        ("Auth",  bd.auth,  color::YELLOW),
        ("Job",   bd.job_return, color::CYAN),
        ("Start", bd.start, color::GREEN),
        ("Beacon", bd.beacon, color::PURPLE),
        ("Other", bd.other, color::TEXT_DIM),
    ];
    let max_count = cats.iter().map(|(_, c, _)| *c).max().unwrap_or(1).max(1);

    let mut lines: Vec<Line> = vec![];
    if bd.total > 0 {
        for (label, count, color) in &cats {
            let bar_max = inner.width.saturating_sub(14);
            let bar_len = ((*count as f64 / max_count as f64) * bar_max as f64).round() as usize;
            let bar = "\u{2588}".repeat(bar_len);
            let mut spans = vec![
                Span::styled(format!(" {:<6}", label), Style::default().fg(*color).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{:>5} ", count), Style::default().fg(color::TEXT_DIM)),
                Span::styled(bar, Style::default().fg(*color)),
            ];
            if *label == "Auth" {
                spans.push(Span::styled(format!(" ({:.1}/s)", app.current_auth_rate), Style::default().fg(color::TEXT_DIM)));
            }
            lines.push(Line::from(spans));
        }
    } else {
        lines.push(Line::from(vec![
            Span::styled(" waiting for events...", Style::default().fg(color::TEXT_DIM)),
        ]));
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}

// ── Event Rate ───────────────────────────────────────────────────────────────

fn draw_events_row(f: &mut Frame, area: Rect, app: &App) {
    let peak_rate = app.event_rate_history.iter().cloned().fold(0.0_f64, f64::max);

    // Build title — include peak + total if wide enough
    let title = if area.width >= 40 {
        Line::from(vec![
            Span::styled(" Events ", Style::default().fg(color::TEXT_TITLE)),
            Span::styled("\u{25b6} ", Style::default().fg(color::CYAN)),
            Span::styled(format!("{:.1}/s ", app.events_per_sec), Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD)),
            Span::styled("pk:", Style::default().fg(color::TEXT_LABEL)),
            Span::styled(format!("{:.1}/s ", peak_rate), Style::default().fg(color::TEXT_DIM)),
            Span::styled("tot: ", Style::default().fg(color::TEXT_LABEL)),
            Span::styled(format!("{}", app.events_total), Style::default().fg(color::TEXT_DIM)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" Events ", Style::default().fg(color::TEXT_TITLE)),
            Span::styled("\u{25b6} ", Style::default().fg(color::CYAN)),
            Span::styled(format!("{:.1}/s", app.events_per_sec), Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD)),
        ])
    };

    let block = make_block_line(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let inner_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), if inner.height >= 3 { Constraint::Length(1) } else { Constraint::Length(0) }])
        .split(inner);

    let max_rate = app.event_rate_history.iter().cloned().fold(0.0_f64, f64::max).max(0.1);
    let idx = app.event_rate_idx;
    let ordered: Vec<f64> = (0..2000)
        .map(|i| app.event_rate_history[(idx + i) % 2000])
        .collect();
    let spark_width = inner_layout[0].width.max(1) as usize;
    let points = spark_width.min(2000);
    let start = ordered.len().saturating_sub(points);
    let spark_data: Vec<u64> = (0..points)
        .map(|i| {
            let v = ordered[start + i];
            (v / max_rate * 100.0) as u64
        })
        .collect();

    render_sparkline(f, inner_layout[0], &spark_data, Style::default().fg(color::CYAN), app.sparkline_style);

    let last_seen = if app.secs_since_last_event == u64::MAX {
        "\u{2014}".to_string()
    } else {
        format!("{}s ago", app.secs_since_last_event)
    };

    let stats_line = Line::from(vec![
        Span::styled("  Rate: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{:.1}/s  ", app.events_per_sec), Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD)),
        Span::styled("Total: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{}  ", app.events_total), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)),
        Span::styled("Last: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(last_seen, Style::default().fg(
            if app.secs_since_last_event < 5 { color::GREEN }
            else if app.secs_since_last_event < 30 { color::YELLOW }
            else { color::ORANGE }
        )),
    ]);
    f.render_widget(
        Paragraph::new(stats_line).style(Style::default().bg(color::BG_DARK)),
        inner_layout[1],
    );
}

// ── Minion Activity Matrix ───────────────────────────────────────────────────

fn draw_activity_matrix(f: &mut Frame, area: Rect, app: &App) {
    let elapsed = app.last_activity_at.elapsed().as_secs();
    let current_rate = app.current_sec_returns;

    // Build title
    let title = if area.width >= 40 {
        let last_str = if app.last_activity_minion.is_empty() {
            "waiting...".to_string()
        } else {
            format!("{}", app.last_activity_minion)
        };
        Line::from(vec![
            Span::styled(" Minions ", Style::default().fg(color::TEXT_TITLE)),
            Span::styled("\u{25c9} ", Style::default().fg(color::GREEN)),
            Span::styled(format!("{}/s ", current_rate), Style::default().fg(if current_rate > 0 { color::GREEN } else { color::TEXT_DIM }).add_modifier(Modifier::BOLD)),
            Span::styled("last: ", Style::default().fg(color::TEXT_LABEL)),
            Span::styled(last_str, Style::default().fg(color::TEXT_DIM)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" Minions ", Style::default().fg(color::TEXT_TITLE)),
            Span::styled("\u{25c9} ", Style::default().fg(color::GREEN)),
            Span::styled(format!("{}/s", current_rate), Style::default().fg(if current_rate > 0 { color::GREEN } else { color::TEXT_DIM }).add_modifier(Modifier::BOLD)),
        ])
    };

    let block = make_block_line(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Full-width sparkline with stats bar below
    let inner_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), if inner.height >= 3 { Constraint::Length(1) } else { Constraint::Length(0) }])
        .split(inner);

    let max_val = app.activity_matrix.iter().cloned().max().unwrap_or(0).max(1);
    let act_idx = app.activity_matrix_idx;
    let ordered_act: Vec<u64> = (0..2000)
        .map(|i| app.activity_matrix[(act_idx + i) % 2000])
        .collect();
    let spark_width = inner_layout[0].width.max(1) as usize;
    let points = spark_width.min(2000);
    let start = ordered_act.len().saturating_sub(points);
    let spark_data: Vec<u64> = (0..points)
        .map(|i| {
            let v = ordered_act[start + i];
            (v * 100 / max_val) as u64
        })
        .collect();

    render_sparkline(f, inner_layout[0], &spark_data, Style::default().fg(color::GREEN), app.sparkline_style);

    let activity_label = if app.last_activity_minion.is_empty() {
        "  waiting for minion returns...".to_string()
    } else if elapsed < 60 {
        format!(
            "  last: \u{2190} {} \u{2192} {}  ({}s ago)",
            app.last_activity_minion, app.last_activity_fun, elapsed,
        )
    } else {
        format!(
            "  last: \u{2190} {} \u{2192} {}  ({}m ago)",
            app.last_activity_minion, app.last_activity_fun, elapsed / 60,
        )
    };
    let rate_color = if current_rate > 0 { color::GREEN } else { color::TEXT_DIM };

    let mut total_returns: u64 = 0;
    for v in &app.activity_matrix { total_returns += v; }

    let stats_line = Line::from(vec![
        Span::styled(activity_label, Style::default().fg(rate_color)),
        Span::styled(format!("  total: {}", total_returns), Style::default().fg(color::TEXT_DIM)),
    ]);
    f.render_widget(
        Paragraph::new(stats_line).style(Style::default().bg(color::BG_DARK)),
        inner_layout[1],
    );
}

// ── Job Pulse ────────────────────────────────────────────────────────────────

fn draw_job_pulse(f: &mut Frame, area: Rect, app: &App) {
    let active_count = app.jobs.len();

    let title = Line::from(vec![
        Span::styled(" Job Pulse ", Style::default().fg(color::TEXT_TITLE)),
        Span::styled(format!("Active: {}", active_count), Style::default().fg(if active_count > 0 { color::CYAN } else { color::TEXT_DIM })),
    ]);
    let block = make_block_line(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let success = app.job_success_pct().map(|p| format!("{:.0}%", p)).unwrap_or_else(|| "--".into());
    let stuck = app.job_stuck_count();
    let (p50, p95) = app.job_completion_latency_p50_p95().unwrap_or((0.0, 0.0));
    let partial = app.job_partial_pct().map(|p| format!("{:.0}%", p)).unwrap_or_else(|| "--".into());

    let mut lines: Vec<Line<'static>> = vec![Line::from(vec![
        Span::styled("  success:", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(success, Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(" stuck:", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(stuck.to_string(), Style::default().fg(if stuck > 0 { color::ORANGE } else { color::TEXT_DIM }).add_modifier(Modifier::BOLD)),
        Span::styled(" p50/p95:", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{:.1}s/{:.1}s", p50, p95), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)),
        Span::styled(" partial:", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(partial, Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
    ])];

    if app.jobs.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no active jobs",
            Style::default().fg(color::TEXT_DIM),
        )));
        f.render_widget(
            Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
            inner,
        );
        return;
    }

    // Show up to 3 latest jobs, sorted by most recent
    let count = app.jobs.len().min(3);
    for job in app.jobs.iter().rev().take(count) {
        let progress = if job.target_count > 0 {
            let pct = (job.returned as f64 / job.target_count as f64).min(1.0);
            let filled = (pct * 10.0).round() as usize;
            let empty = 10usize.saturating_sub(filled);
            format!(
                "{}{} {}/{}",
                "\u{2588}".repeat(filled),
                "\u{2591}".repeat(empty),
                job.returned, job.target_count,
            )
        } else {
            format!("{}?", job.returned)
        };
        let color = if job.target_count > 0 && job.returned >= job.target_count {
            color::GREEN
        } else {
            color::YELLOW
        };

        let fun_short = if job.fun.len() > 35 {
            format!("{}..", &job.fun[..33])
        } else {
            job.fun.clone()
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", fun_short), Style::default().fg(color::CYAN)),
            Span::styled(progress, Style::default().fg(color).add_modifier(Modifier::BOLD)),
        ]));
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}

// ─── Shared sparkline renderer (bars or braille dots) ─────────────────────

fn render_sparkline(f: &mut Frame, area: Rect, data: &[u64], style: Style, mode: SparklineStyle) {
    match mode {
        SparklineStyle::Bars => {
            let sp = Sparkline::default()
                .block(Block::default())
                .style(style)
                .data(data);
            f.render_widget(sp, area);
        }
        SparklineStyle::Dots => {
            // One braille char per data value: all 8 dots represent 9 height
            // levels (0 = empty, 8 = full).  This fills the full area width.
            let total_pixels = (area.height as u32).saturating_mul(8).max(1);
            let ncols = area.width as usize;
            if ncols == 0 || area.height == 0 { return; }

            let mut lines: Vec<Line> = Vec::with_capacity(area.height as usize);
            for br in 0..area.height {
                let mut line = String::with_capacity(ncols);
                let row_from_bottom = (area.height - 1 - br) as u32;
                for &v in data.iter().take(ncols) {
                    let threshold = ((v.min(100) as u32).saturating_mul(total_pixels) + 99) / 100;
                    let mut bits = 0u8;
                    // Braille bit->dot layout is (bit0 bit3 / bit1 bit4 / bit2 bit5 / bit6 bit7),
                    // i.e. row 0 (top) = bits 0,3 through row 3 (bottom) = bits 6,7. Filling bits
                    // in raw numeric order therefore lights the top-left dot first; walk this
                    // fill order instead so low values light the bottom row like Bars mode does.
                    const FILL_ORDER: [u8; 8] = [6, 7, 2, 5, 1, 4, 0, 3];
                    for (level, &bit_idx) in FILL_ORDER.iter().enumerate() {
                        let pixel_pos = row_from_bottom.saturating_mul(8) + level as u32;
                        if pixel_pos < threshold {
                            bits |= 1 << bit_idx;
                        }
                    }
                    let ch = if bits == 0 { ' ' } else { char::from_u32(0x2800 + bits as u32).unwrap_or(' ') };
                    line.push(ch);
                }
                lines.push(Line::from(Span::styled(line, style)));
            }
            f.render_widget(Paragraph::new(lines), area);
        }
    }
}
