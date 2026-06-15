use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Sparkline};
use ratatui::Frame;

use crate::app::App;

use super::color;

pub fn draw_dashboard(f: &mut Frame, area: Rect, app: &App) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),   // Keys & Uptime
            Constraint::Length(4),   // Processes & Services
            Constraint::Length(4),   // Host Resources
            Constraint::Length(3),   // ZMQ
            Constraint::Length(3),   // Config
            Constraint::Length(4),   // Event Rate sparkline
        ])
        .split(area);

    draw_keys_row(f, layout[0], app);
    draw_proc_row(f, layout[1], app);
    draw_host_row(f, layout[2], app);
    draw_zmq_row(f, layout[3], app);
    draw_config_row(f, layout[4], app);
    draw_events_row(f, layout[5], app);
}

fn make_block(_title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER))
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

fn draw_keys_row(f: &mut Frame, area: Rect, app: &App) {
    let block = make_block(" Keys & Uptime ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let accepted = count_keys_with_status(&app.keys, crate::types::KeyStatus::Accepted);
    let pending = count_keys_with_status(&app.keys, crate::types::KeyStatus::Pending);
    let rejected = count_keys_with_status(&app.keys, crate::types::KeyStatus::Rejected);
    let denied = count_keys_with_status(&app.keys, crate::types::KeyStatus::Denied);
    let uptime = app.session_start.elapsed().as_secs();

    let line = Line::from(vec![
        Span::styled("  Accepted: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{} ", accepted), Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD)),
        Span::styled("Pending: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{} ", pending), Style::default().fg(if pending > 0 { color::YELLOW } else { color::TEXT_DIM }).add_modifier(Modifier::BOLD)),
        Span::styled("Rejected: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{} ", rejected), Style::default().fg(if rejected > 0 { color::RED } else { color::TEXT_DIM }).add_modifier(Modifier::BOLD)),
        Span::styled("Denied: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{}  ", denied), Style::default().fg(if denied > 0 { color::ORANGE } else { color::TEXT_DIM }).add_modifier(Modifier::BOLD)),
        Span::styled("\u{2502}  ", Style::default().fg(color::BORDER)),
        Span::styled("Uptime: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{}", format_duration(uptime)), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)),
    ]);

    f.render_widget(Paragraph::new(line).style(Style::default().bg(color::BG_DARK)), inner);
}

fn draw_proc_row(f: &mut Frame, area: Rect, app: &App) {
    let block = make_block(" Processes & Services ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut master_rss = 0u64;
    let mut worker_count = 0usize;
    let mut api_rss = 0u64;
    let mut api_count = 0usize;
    for p in &app.processes {
        match p.role {
            crate::types::SaltProcessRole::Master | crate::types::SaltProcessRole::Worker => {
                master_rss += p.rss_kb;
                worker_count += 1;
            }
            crate::types::SaltProcessRole::API => { api_rss += p.rss_kb; api_count += 1; }
            _ => {}
        }
    }

    let total_rss_used: u64 = app.processes.iter().map(|p| p.rss_kb).sum();
    let mem_total_kb = app.system_info.mem_total_kb.max(1);
    let mem_pct = (total_rss_used as f64 / mem_total_kb as f64) * 100.0;

    let event_worker_rss: u64 = app.processes.iter()
        .filter(|p| matches!(p.role, crate::types::SaltProcessRole::EventPublisher | crate::types::SaltProcessRole::ReqServer))
        .map(|p| p.rss_kb).sum();

    let gauge_label = format!(
        "Master {} MB ({} wrk)  API {} MB ({} api)  Event/Req {} MB  {} minions tracked",
        master_rss / 1024, worker_count,
        api_rss / 1024, api_count,
        event_worker_rss / 1024,
        app.minion_info.len(),
    );

    let gauge = Gauge::default()
        .gauge_style(pct_gauge_style(mem_pct))
        .percent((mem_pct.round() as u16).min(100))
        .label(gauge_label);

    // Service status line
    let master_svc = app.services.iter().find(|s| s.unit == "salt-master");
    let api_svc = app.services.iter().find(|s| s.unit == "salt-api");

    let mut svc_spans: Vec<Span<'static>> = vec![
        Span::styled("  ".to_string(), Style::default().fg(color::BG_DARK)),
        Span::styled(
            format!("salt-master: {}", master_svc.map(|s| if s.active { "\u{25cf} active" } else { "\u{25cb} down" }).unwrap_or("\u{25cb} n/a")),
            Style::default().fg(master_svc.map(|s| if s.active { color::GREEN } else { color::RED }).unwrap_or(color::TEXT_DIM)),
        ),
        Span::styled("  ".to_string(), Style::default().fg(color::TEXT_DIM)),
        Span::styled(
            format!("salt-api: {}", api_svc.map(|s| if s.active { "\u{25cf} active" } else { "\u{25cb} down" }).unwrap_or("\u{25cb} n/a")),
            Style::default().fg(api_svc.map(|s| if s.active { color::GREEN } else { color::RED }).unwrap_or(color::TEXT_DIM)),
        ),
    ];

    if app.processes.iter().any(|p| p.role == crate::types::SaltProcessRole::Syndic) {
        svc_spans.push(Span::styled("  syndic: \u{25cf} running".to_string(), Style::default().fg(color::CYAN)));
    }
    // Add alert count on the right
    let a = app.alert_engine.active_count();
    svc_spans.push(Span::styled(format!("  \u{2502}  "), Style::default().fg(color::BORDER)));
    svc_spans.push(Span::styled("Alerts: ".to_string(), Style::default().fg(color::TEXT_LABEL)));
    svc_spans.push(Span::styled(
        format!("{}", a),
        Style::default().fg(if a > 0 { color::ORANGE } else { color::GREEN }).add_modifier(Modifier::BOLD),
    ));

    let inner_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(inner);

    f.render_widget(gauge, inner_layout[0]);
    f.render_widget(Paragraph::new(Line::from(svc_spans)).style(Style::default().bg(color::BG_DARK)), inner_layout[1]);
}

fn draw_host_row(f: &mut Frame, area: Rect, app: &App) {
    let block = make_block(" Host Resources ");
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

    // Load gauge
    let load_gauge = Gauge::default()
        .gauge_style(pct_gauge_style(load_pct as f64))
        .percent(load_pct)
        .label(format!(
            "Load: {:.1}/{:.1}/{:.1}  ({}-core host  {:.0}%)",
            app.system_info.load_1, app.system_info.load_5, app.system_info.load_15,
            cpu_count as u32, load_pct,
        ));

    // Memory gauge with disk appended as text
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

fn draw_zmq_row(f: &mut Frame, area: Rect, app: &App) {
    let block = make_block(" Network Connections ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let pub_port = app.config_snapshot.as_ref().map(|c| c.publish_port).unwrap_or(4505);
    let ret_port = app.config_snapshot.as_ref().map(|c| c.ret_port).unwrap_or(4506);
    let api_port = app.config_snapshot.as_ref().map(|c| c.api_port).unwrap_or(0);

    let pub_established = app.zmq_connections.iter().filter(|c| c.local_port == pub_port && c.state == "ESTABLISHED").count();
    let pub_total = app.zmq_connections.iter().filter(|c| c.local_port == pub_port).count();
    let ret_established = app.zmq_connections.iter().filter(|c| c.local_port == ret_port && c.state == "ESTABLISHED").count();
    let ret_total = app.zmq_connections.iter().filter(|c| c.local_port == ret_port).count();

    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(format!("  :{} publish ", pub_port), Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{} estab", pub_established), Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" ({})  ", pub_total), Style::default().fg(color::TEXT_DIM)),
        Span::styled(format!(":{} return ", ret_port), Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{} estab", ret_established), Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" ({})", ret_total), Style::default().fg(color::TEXT_DIM)),
    ];

    if api_port > 0 {
        let api_estab = app.zmq_connections.iter().filter(|c| c.local_port == api_port && c.state == "ESTABLISHED").count();
        let api_total = app.zmq_connections.iter().filter(|c| c.local_port == api_port).count();
        spans.push(Span::styled(format!("  :{} api ", api_port), Style::default().fg(color::TEXT_LABEL)));
        spans.push(Span::styled(format!("{} estab", api_estab), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(" ({})", api_total), Style::default().fg(color::TEXT_DIM)));
    }

    f.render_widget(Paragraph::new(Line::from(spans)).style(Style::default().bg(color::BG_DARK)), inner);
}

fn draw_config_row(f: &mut Frame, area: Rect, app: &App) {
    let block = make_block(" Config ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    if let Some(ref cfg) = app.config_snapshot {
        let reactors = if cfg.reactor_count > 0 { format!("{} reactors", cfg.reactor_count) } else { "no reactors".to_string() };
        let fs_backends = if cfg.fileserver_backends.is_empty() { "roots".to_string() } else { cfg.fileserver_backends.join(", ") };

        let text = format!(
            "  threads: {} on {} cores  transport: {}  {}  fileserver: [{}]  job_cache: {}  pillar_cache: {}  presence: {}",
            cfg.worker_threads.map(|v| v.to_string()).unwrap_or_else(|| "?".to_string()),
            cfg.cpu_count,
            cfg.transport,
            reactors,
            fs_backends,
            cfg.master_job_cache,
            if cfg.pillar_cache { "on" } else { "off" },
            if cfg.presence_events { "enabled" } else { "disabled" },
        );

        f.render_widget(
            Paragraph::new(Line::from(Span::styled(text, Style::default().fg(color::TEXT))))
                .style(Style::default().bg(color::BG_DARK)),
            inner,
        );
    } else {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled("  waiting for config data...", Style::default().fg(color::TEXT_DIM))))
                .style(Style::default().bg(color::BG_DARK)),
            inner,
        );
    }
}

fn draw_events_row(f: &mut Frame, area: Rect, app: &App) {
    let block = make_block(" Event Rate (60s) ");
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
        Span::styled("Peak: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{:.1}/s  ", max_rate), Style::default().fg(color::CYAN)),
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
    f.render_widget(Paragraph::new(stats_line).style(Style::default().bg(color::BG_DARK)), inner_layout[1]);
}
