use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Sparkline};
use ratatui::Frame;

use crate::app::App;

use super::color;

const RADAR_FRAMES: &[char] = &['\u{25d4}', '\u{25d1}', '\u{25d5}', '\u{25d0}'];
// Frames: ◔ ◑ ◕ ◐

pub fn draw_dashboard(f: &mut Frame, area: Rect, app: &App) {
    let services_height = (4 + app.services.len()) as u16; // 2 block borders + 1 gauge + 1 header + N services
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),           // Header
            Constraint::Length(3),           // Keys & Uptime
            Constraint::Length(services_height), // Processes & Services (exact fit)
            Constraint::Length(4),           // Host Resources
            Constraint::Length(4),           // ZMQ (merged connections + latency, sparkline)
            Constraint::Length(4),           // Event Rate sparkline
            Constraint::Length(3),           // Minion Activity Matrix
            Constraint::Length(3),           // Job Pulse
        ])
        .split(area);

    draw_header(f, layout[0], app);
    draw_keys_row(f, layout[1], app);
    draw_proc_row(f, layout[2], app);
    draw_host_row(f, layout[3], app);
    draw_zmq_row(f, layout[4], app);
    draw_events_row(f, layout[5], app);
    draw_activity_matrix(f, layout[6], app);
    draw_job_pulse(f, layout[7], app);
}

fn make_block(title: String) -> Block<'static> {
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

    let accepted = count_keys_with_status(&app.keys, crate::types::KeyStatus::Accepted);
    let pending = count_keys_with_status(&app.keys, crate::types::KeyStatus::Pending);
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
        Span::styled(" v0.1.0 ", Style::default().fg(color::TEXT_DIM)),
        Span::styled(format!("{}", radar), Style::default().fg(color::CYAN)),
        Span::styled(format!(" {} ", dot), Style::default().fg(dot_color).add_modifier(Modifier::BOLD)),
        Span::styled(format!("Salt {}", version), Style::default().fg(color::TEXT).add_modifier(Modifier::BOLD)),
        Span::styled("  \u{2502}", Style::default().fg(color::BORDER)),
        Span::styled(" host: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{}  ", hostname), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)),
        Span::styled("\u{2502}", Style::default().fg(color::BORDER)),
        Span::styled(" keys: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("A{} ", accepted), Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(format!("P{} ", pending), Style::default().fg(if pending > 0 { color::YELLOW } else { color::TEXT_DIM }).add_modifier(Modifier::BOLD)),
        Span::styled(" \u{2502}", Style::default().fg(color::BORDER)),
        Span::styled(" uptime: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{}", uptime_display), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)),
    ]);

    f.render_widget(Paragraph::new(line).style(Style::default().bg(color::BG_DARK).fg(color::TEXT)), area);
}

// ── Keys & Uptime ────────────────────────────────────────────────────────────

fn draw_keys_row(f: &mut Frame, area: Rect, app: &App) {
    let block = make_block(" Keys & Uptime ".to_string());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let accepted = count_keys_with_status(&app.keys, crate::types::KeyStatus::Accepted);
    let pending = count_keys_with_status(&app.keys, crate::types::KeyStatus::Pending);
    let rejected = count_keys_with_status(&app.keys, crate::types::KeyStatus::Rejected);
    let denied = count_keys_with_status(&app.keys, crate::types::KeyStatus::Denied);

    let line = Line::from(vec![
        Span::styled("  Accepted: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{} ", accepted), Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD)),
        Span::styled("Pending: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{} ", pending), Style::default().fg(if pending > 0 { color::YELLOW } else { color::TEXT_DIM }).add_modifier(Modifier::BOLD)),
        Span::styled("Rejected: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{} ", rejected), Style::default().fg(if rejected > 0 { color::RED } else { color::TEXT_DIM }).add_modifier(Modifier::BOLD)),
        Span::styled("Denied: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{}", denied), Style::default().fg(if denied > 0 { color::ORANGE } else { color::TEXT_DIM }).add_modifier(Modifier::BOLD)),
    ]);

    f.render_widget(Paragraph::new(line).style(Style::default().bg(color::BG_DARK)), inner);
}

// ── Processes & Services ─────────────────────────────────────────────────────

fn draw_proc_row(f: &mut Frame, area: Rect, app: &App) {
    let block = make_block(" Services ".to_string());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let total_rss_used: u64 = app.processes.iter().map(|p| p.rss_kb).sum();
    let mem_total_kb = app.system_info.mem_total_kb.max(1);
    let mem_pct = (total_rss_used as f64 / mem_total_kb as f64) * 100.0;

    let mut master_rss = 0u64;
    let mut worker_count = 0usize;
    for p in &app.processes {
        match p.role {
            crate::types::SaltProcessRole::Master | crate::types::SaltProcessRole::Worker => {
                master_rss += p.rss_kb;
                worker_count += 1;
            }
            _ => {}
        }
    }

    let inner_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1 + app.services.len() as u16)])
        .split(inner);

    let gauge = Gauge::default()
        .gauge_style(pct_gauge_style(mem_pct))
        .percent((mem_pct.round() as u16).min(100))
        .label(format!(
            "Salt RSS {} MB ({} proc)  {} minions",
            master_rss / 1024, worker_count, app.minion_info.len(),
        ));

    // Service status - display all discovered salt-* services with memory usage
    let mut service_lines: Vec<Line<'static>> = vec![];

    // Display header for service columns
    service_lines.push(Line::from(vec![
        Span::styled("Unit            ", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        Span::styled("Status", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        Span::styled(" State            ", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        Span::styled(" Memory ", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        Span::styled(" Uptime", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
    ]));

    for service in &app.services {
        let status_icon = if service.active { "●" } else { "○" };
        let status_color = if service.active { color::GREEN } else { color::RED };

        // Format memory usage from ServiceHealth
        let memory_mb = service.memory_kb / 1024;
        let memory_text = if memory_mb > 0 {
            format!("{} MB", memory_mb)
        } else {
            "--".to_string()
        };

        // Format uptime from service data
        let uptime_text = if service.uptime_seconds > 0 {
            let secs = service.uptime_seconds;
            if secs >= 3600 {
                format!("{}h", secs / 3600)
            } else if secs >= 60 {
                format!("{}m", secs / 60)
            } else {
                format!("{}s", secs)
            }
        } else {
            "--".to_string()
        };

        service_lines.push(Line::from(vec![
            Span::styled(format!("{:<16}", service.unit), Style::default().fg(color::CYAN)),
            Span::styled(format!(" {:<6}", status_icon), Style::default().fg(status_color)),
            Span::styled(format!(" {:<17}", service.state), Style::default().fg(color::TEXT)),
            Span::styled(format!(" {:>6} ", memory_text), Style::default().fg(color::YELLOW)),
            Span::styled(format!(" {:>6}", uptime_text), Style::default().fg(color::TEXT_DIM)),
        ]));
    }

    f.render_widget(gauge, inner_layout[0]);
    f.render_widget(
        Paragraph::new(service_lines).style(Style::default().bg(color::BG_DARK)),
        inner_layout[1],
    );
}

// ── Host Resources ───────────────────────────────────────────────────────────

fn draw_host_row(f: &mut Frame, area: Rect, app: &App) {
    let block = make_block(" Host Resources ".to_string());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mem_total_kb = app.system_info.mem_total_kb.max(1);
    let mem_avail_kb = app.system_info.mem_avail_kb;
    let mem_used_mb = (mem_total_kb.saturating_sub(mem_avail_kb)) / 1024;
    let mem_total_mb = mem_total_kb / 1024;
    let mem_pct = ((mem_total_kb - mem_avail_kb) as f64 / mem_total_kb as f64) * 100.0;

    let cpu_count = app.config_snapshot.as_ref().map(|c| c.cpu_count).unwrap_or(1) as f64;
    let load_pct = (app.system_info.load_1 / cpu_count * 100.0).min(100.0).max(0.0) as u16;

    let inner_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(inner);

    let load_gauge = Gauge::default()
        .gauge_style(pct_gauge_style(load_pct as f64))
        .percent(load_pct)
        .label(format!(
            "Load: {:.1}/{:.1}/{:.1}  ({}-core host  {:.0}%)",
            app.system_info.load_1, app.system_info.load_5, app.system_info.load_15,
            cpu_count as u32, load_pct,
        ));

    let mem_gauge = Gauge::default()
        .gauge_style(pct_gauge_style(mem_pct))
        .percent((mem_pct.round() as u16).min(100))
        .label(format!(
            "Mem: {} MB / {} MB ({:.0}%)  Cache disk: {:.0}%  PKI disk: {:.0}%",
            mem_used_mb, mem_total_mb, mem_pct,
            app.system_info.cache_disk_usage_pct,
            app.system_info.pki_disk_usage_pct,
        ));

    f.render_widget(load_gauge, inner_layout[0]);
    f.render_widget(mem_gauge, inner_layout[1]);
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
    let block = make_block(" Network & Latency ".to_string());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let pub_port = app.config_snapshot.as_ref().map(|c| c.publish_port).unwrap_or(4505);
    let ret_port = app.config_snapshot.as_ref().map(|c| c.ret_port).unwrap_or(4506);
    let api_port = app.config_snapshot.as_ref().map(|c| c.api_port).unwrap_or(0);

    let pub_established = app.zmq_connections.iter().filter(|c| c.local_port == pub_port && c.state == "ESTABLISHED").count();
    let pub_total = app.zmq_connections.iter().filter(|c| c.local_port == pub_port).count();
    let ret_established = app.zmq_connections.iter().filter(|c| c.local_port == ret_port && c.state == "ESTABLISHED").count();
    let ret_total = app.zmq_connections.iter().filter(|c| c.local_port == ret_port).count();

    // Latency info
    let lat = app.zmq_latency_ms;
    let (lat_color, lat_label) = if lat <= 0.0 {
        (color::TEXT_DIM, "measuring...".to_string())
    } else if lat < 10.0 {
        (color::GREEN, format!("{:.0} ms", lat))
    } else if lat < 50.0 {
        (color::YELLOW, format!("{:.0} ms", lat))
    } else {
        (color::ORANGE, format!("{:.0} ms", lat))
    };

    let inner_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(inner);

    // Line 1: Latency sparkline (consistent with Event Rate, Minion Activity)
    let max_lat = app.zmq_latency_history.iter().cloned().fold(0.0_f64, f64::max).max(1.0);
    let spark_data: Vec<u64> = app.zmq_latency_history.iter()
        .map(|v| (v / max_lat * 100.0) as u64)
        .collect();
    let sparkline = Sparkline::default()
        .block(Block::default())
        .style(Style::default().fg(lat_color))
        .data(&spark_data);
    f.render_widget(sparkline, inner_layout[0]);

    // Line 2: connections + latency
    let mut spans: Vec<Span<'static>> = vec![
        Span::styled("  :".to_string(), Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{} pub{} ({})  ", pub_port, pub_established, pub_total), Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD)),
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

// ── Event Rate ───────────────────────────────────────────────────────────────

fn draw_events_row(f: &mut Frame, area: Rect, app: &App) {
    let block = make_block(" Event Rate (60s) ".to_string());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let max_rate = app.event_rate_history.iter().cloned().fold(0.0_f64, f64::max).max(0.1);
    let spark_data: Vec<u64> = app.event_rate_history.iter()
        .map(|v| (v / max_rate * 100.0) as u64)
        .collect();

    let last_seen = if app.secs_since_last_event == u64::MAX {
        "\u{2014}".to_string()
    } else {
        format!("{}s ago", app.secs_since_last_event)
    };

    let inner_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(inner);

    let sparkline = Sparkline::default()
        .block(Block::default())
        .style(Style::default().fg(color::CYAN))
        .data(&spark_data);

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

    f.render_widget(sparkline, inner_layout[0]);
    f.render_widget(
        Paragraph::new(stats_line).style(Style::default().bg(color::BG_DARK)),
        inner_layout[1],
    );
}

// ── Minion Activity Matrix ───────────────────────────────────────────────────

fn draw_activity_matrix(f: &mut Frame, area: Rect, app: &App) {
    let block = make_block(" Minion Activity (60s) ".to_string());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let max_val = app.activity_matrix.iter().cloned().max().unwrap_or(0).max(1);
    let spark_data: Vec<u64> = app.activity_matrix.iter()
        .map(|v| (v * 100 / max_val) as u64)
        .collect();

    let inner_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(inner);

    let sparkline = Sparkline::default()
        .block(Block::default())
        .style(Style::default().fg(color::GREEN))
        .data(&spark_data);

    let elapsed = app.last_activity_at.elapsed().as_secs();
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
    let rate_color = if app.current_sec_returns > 0 { color::GREEN } else { color::TEXT_DIM };

    let mut total_returns: u64 = 0;
    for v in &app.activity_matrix { total_returns += v; }

    let stats_line = Line::from(vec![
        Span::styled(activity_label, Style::default().fg(rate_color)),
        Span::styled(format!("  total: {}", total_returns), Style::default().fg(color::TEXT_DIM)),
    ]);

    f.render_widget(sparkline, inner_layout[0]);
    f.render_widget(
        Paragraph::new(stats_line).style(Style::default().bg(color::BG_DARK)),
        inner_layout[1],
    );
}

// ── Job Pulse ────────────────────────────────────────────────────────────────

fn draw_job_pulse(f: &mut Frame, area: Rect, app: &App) {
    let block = make_block(" Job Pulse ".to_string());
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.jobs.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  no active jobs",
                Style::default().fg(color::TEXT_DIM),
            ))).style(Style::default().bg(color::BG_DARK)),
            inner,
        );
        return;
    }

    // Show up to 4 latest jobs, sorted by most recent (reverse order)
    let count = app.jobs.len().min(4);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(count);
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
