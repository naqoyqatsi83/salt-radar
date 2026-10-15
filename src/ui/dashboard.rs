use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Sparkline, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::types::{SparklineStyle, SaltProcessRole};

use super::color;

pub(super) const RADAR_FRAMES: &[char] = &['\u{25d4}', '\u{25d1}', '\u{25d5}', '\u{25d0}'];
// Frames: ◔ ◑ ◕ ◐

// Services / Minion Key Status / Event Types horizontal split, out of 5 parts.
// Key Status only ever shows 4 fixed-format count rows + a churn line, so it
// needs far less width than Services (gauge + per-role mem + service list) or
// Event Types; used both for the actual row layout and for estimating the
// Services column's wrapped height below, which must stay in sync with it.
const MID_ROW_RATIO: [u32; 3] = [2, 1, 2];

/// The Dashboard's bottom sparkline panels, each independently toggleable
/// via config.show_*_graph and orderable via config.graph_order (Config
/// Editor's [dashboard] section). `ALL`'s order is the fallback default and
/// also what any panel key missing from graph_order gets appended in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SparkPanel {
    Cpu,
    Mem,
    Zmq,
    Events,
    Activity,
}

impl SparkPanel {
    const ALL: [SparkPanel; 5] = [SparkPanel::Cpu, SparkPanel::Mem, SparkPanel::Zmq, SparkPanel::Events, SparkPanel::Activity];

    fn key(self) -> &'static str {
        match self {
            SparkPanel::Cpu => "cpu",
            SparkPanel::Mem => "mem",
            SparkPanel::Zmq => "zmq",
            SparkPanel::Events => "events",
            SparkPanel::Activity => "activity",
        }
    }

    fn from_key(key: &str) -> Option<SparkPanel> {
        match key.trim().to_ascii_lowercase().as_str() {
            "cpu" => Some(SparkPanel::Cpu),
            "mem" | "memory" => Some(SparkPanel::Mem),
            "zmq" => Some(SparkPanel::Zmq),
            "events" => Some(SparkPanel::Events),
            "activity" => Some(SparkPanel::Activity),
            _ => None,
        }
    }

    fn enabled(self, app: &App) -> bool {
        match self {
            SparkPanel::Zmq => app.config.show_zmq_graph,
            SparkPanel::Events => app.config.show_events_graph,
            SparkPanel::Activity => app.config.show_activity_graph,
            SparkPanel::Cpu => app.config.show_cpu_graph,
            SparkPanel::Mem => app.config.show_mem_graph,
        }
    }

    fn draw(self, f: &mut Frame, area: Rect, app: &App) {
        match self {
            SparkPanel::Zmq => draw_zmq_row(f, area, app),
            SparkPanel::Events => draw_events_row(f, area, app),
            SparkPanel::Activity => draw_activity_matrix(f, area, app),
            SparkPanel::Cpu => draw_cpu_row(f, area, app),
            SparkPanel::Mem => draw_mem_row(f, area, app),
        }
    }
}

/// Parses config.graph_order (comma-separated panel keys) into the full
/// panel list in that order — unrecognized/duplicate tokens are dropped,
/// and any panel missing from the string is appended afterward in `ALL`'s
/// default order, so a partial or malformed value never loses a panel.
fn ordered_panels(order: &str) -> Vec<SparkPanel> {
    let mut seen = std::collections::HashSet::new();
    let mut result: Vec<SparkPanel> = order
        .split(',')
        .filter_map(SparkPanel::from_key)
        .filter(|p| seen.insert(p.key()))
        .collect();
    for p in SparkPanel::ALL {
        if seen.insert(p.key()) {
            result.push(p);
        }
    }
    result
}

fn enabled_spark_panels(app: &App) -> Vec<SparkPanel> {
    ordered_panels(&app.config.graph_order).into_iter().filter(|p| p.enabled(app)).collect()
}

fn draw_no_graphs_hint(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  No graphs enabled — toggle show_*_graph in Config Editor ('F3')",
            Style::default().fg(color::inactive_fg()),
        )))
        .style(Style::default().bg(color::main_bg())),
        area,
    );
}

/// The Dashboard's middle row (Services / Minion Key Status / Event Types),
/// each independently toggleable via config.show_services / show_key_status
/// / show_event_types. Weights re-normalize over whichever subset is
/// actually enabled, so hiding one column doesn't leave a gap — the
/// remaining columns just split the row's width between them.
#[derive(Clone, Copy)]
enum MidPanel {
    Services,
    KeyStatus,
    EventTypes,
}

impl MidPanel {
    fn enabled(self, app: &App) -> bool {
        match self {
            MidPanel::Services => app.config.show_services,
            MidPanel::KeyStatus => app.config.show_key_status,
            MidPanel::EventTypes => app.config.show_event_types,
        }
    }

    fn weight(self) -> u32 {
        match self {
            MidPanel::Services => MID_ROW_RATIO[0],
            MidPanel::KeyStatus => MID_ROW_RATIO[1],
            MidPanel::EventTypes => MID_ROW_RATIO[2],
        }
    }

    fn draw(self, f: &mut Frame, area: Rect, app: &App) {
        match self {
            MidPanel::Services => draw_proc_row(f, area, app),
            MidPanel::KeyStatus => draw_key_status(f, area, app),
            MidPanel::EventTypes => draw_event_breakdown(f, area, app),
        }
    }
}

fn enabled_mid_panels(app: &App) -> Vec<MidPanel> {
    [MidPanel::Services, MidPanel::KeyStatus, MidPanel::EventTypes]
        .into_iter()
        .filter(|p| p.enabled(app))
        .collect()
}

pub fn draw_dashboard(f: &mut Frame, area: Rect, app: &App) {
    // Small terminal fallback: vertical stacked layout
    if area.height < 26 {
        draw_dashboard_compact(f, area, app);
        return;
    }

    let body = area;

    let mid_panels = enabled_mid_panels(app);

    // Top section: fixed-height info panels
    // Bottom section: stacked full-width sparklines fill remaining space
    let key_status_h = 8u16; // 4 key rows + separator + total + 2 border
    // Services panel's width depends on how many mid-row columns are
    // actually enabled; mirror that split just for its width so the
    // proc-row estimate matches what draw_proc_row actually renders with,
    // rather than assuming a fixed 1 line.
    let services_h = if mid_panels.iter().any(|p| matches!(p, MidPanel::Services)) {
        let total_weight: u32 = mid_panels.iter().map(|p| p.weight()).sum();
        let services_col_width = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(mid_panels.iter().map(|p| Constraint::Ratio(p.weight(), total_weight)).collect::<Vec<_>>())
            .split(Rect::new(0, 0, body.width, 1))[mid_panels.iter().position(|p| matches!(p, MidPanel::Services)).unwrap()]
            .width
            .saturating_sub(2); // block borders
        let proc_rows = proc_rows_for(&group_processes(&app.processes), services_col_width);
        (4 + proc_rows + app.services.len() as u16).min(25) // gauge + proc lines + header + N services + 2 border
    } else {
        0
    };
    let mid_h = key_status_h.max(services_h);

    // salt-api gets a row only when it's actually present on this master —
    // most masters don't run it, and reserving space for an always-empty
    // panel would just shrink everything else for no reason.
    let show_api_row = app.config.show_salt_api && app.processes.iter().any(|p| p.role == SaltProcessRole::Api);

    let mut constraints = vec![];
    if app.config.show_host_resources {
        constraints.push(Constraint::Length(5)); // Host Resources
    }
    if !mid_panels.is_empty() {
        constraints.push(Constraint::Length(mid_h)); // Services + Key Status + Event Types (exact-fit)
    }
    if app.config.show_job_pulse || show_api_row {
        // Job Pulse and salt-api are both a KPI line + up to 3 rows -- same
        // fixed height -- and neither needs anywhere near the terminal's
        // full width, so they share one row side by side instead of each
        // claiming a full-width row of their own.
        constraints.push(Constraint::Length(4));
    }
    constraints.push(Constraint::Min(10)); // Sparklines (at least 10 lines = 2 per panel × 5 default-enabled panels)

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(body);

    let mut next = 0;
    if app.config.show_host_resources {
        draw_host_row(f, sections[next], app);
        next += 1;
    }

    if !mid_panels.is_empty() {
        let total_weight: u32 = mid_panels.iter().map(|p| p.weight()).sum();
        let mid = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(mid_panels.iter().map(|p| Constraint::Ratio(p.weight(), total_weight)).collect::<Vec<_>>())
            .split(sections[next]);
        for (panel, area) in mid_panels.iter().zip(mid.iter()) {
            panel.draw(f, *area, app);
        }
        next += 1;
    }

    if app.config.show_job_pulse || show_api_row {
        if app.config.show_job_pulse && show_api_row {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(sections[next]);
            draw_job_pulse(f, cols[0], app);
            draw_api_row(f, cols[1], app);
        } else if app.config.show_job_pulse {
            draw_job_pulse(f, sections[next], app);
        } else {
            draw_api_row(f, sections[next], app);
        }
        next += 1;
    }

    // Bottom: stacked full-width sparklines, one per enabled panel, for
    // time-axis correlation.
    let panels = enabled_spark_panels(app);
    if panels.is_empty() {
        draw_no_graphs_hint(f, sections[next]);
    } else {
        let n = panels.len() as u32;
        let constraints: Vec<Constraint> = panels.iter().map(|_| Constraint::Ratio(1, n)).collect();
        let sparklines = Layout::default().direction(Direction::Vertical).constraints(constraints).split(sections[next]);
        for (panel, area) in panels.into_iter().zip(sparklines.iter()) {
            panel.draw(f, *area, app);
        }
    }
}

/// Fallback for small terminals: vertical stacked layout (same as original).
/// Key Status and Event Types don't get their own row in this layout (they
/// never have — compact mode already dropped them for space), so only
/// Services' own toggle applies here; Key Status/Event Types' toggles just
/// have no effect on a terminal this small.
fn draw_dashboard_compact(f: &mut Frame, area: Rect, app: &App) {
    let proc_rows = proc_rows_for(&group_processes(&app.processes), area.width.saturating_sub(2));
    let services_height = 4 + proc_rows + app.services.len() as u16;
    let panels = enabled_spark_panels(app);
    let show_api_row = app.config.show_salt_api && app.processes.iter().any(|p| p.role == SaltProcessRole::Api);

    let mut constraints = vec![];
    if app.config.show_host_resources {
        constraints.push(Constraint::Length(5));
    }
    if app.config.show_services {
        constraints.push(Constraint::Length(services_height));
    }
    constraints.extend(panels.iter().map(|_| Constraint::Length(4)));
    if app.config.show_job_pulse || show_api_row {
        constraints.push(Constraint::Length(4)); // Job Pulse + salt-api, side by side
    }

    let layout = Layout::default().direction(Direction::Vertical).constraints(constraints).split(area);

    let mut next = 0;
    if app.config.show_host_resources {
        draw_host_row(f, layout[next], app);
        next += 1;
    }
    if app.config.show_services {
        draw_proc_row(f, layout[next], app);
        next += 1;
    }
    for panel in panels {
        panel.draw(f, layout[next], app);
        next += 1;
    }
    if app.config.show_job_pulse || show_api_row {
        if app.config.show_job_pulse && show_api_row {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(layout[next]);
            draw_job_pulse(f, cols[0], app);
            draw_api_row(f, cols[1], app);
        } else if app.config.show_job_pulse {
            draw_job_pulse(f, layout[next], app);
        } else {
            draw_api_row(f, layout[next], app);
        }
    }
}

fn make_block_line(title: Line<'static>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::border()))
        .title(Line::from(super::bracket_title(title.spans, color::border())))
        .style(Style::default().bg(color::main_bg()))
}

fn make_block_line_hinted(title: Line<'static>, hints: &[(&str, String)]) -> Block<'static> {
    let block = make_block_line(title);
    if hints.is_empty() { block } else { block.title_bottom(super::border_hints(hints, color::border()).left_aligned()) }
}

/// Each sparkline panel's own hide toggle (its existing F-key), plus the
/// cross-panel bars/dots style toggle embedded once, on whichever panel
/// happens to be first among the currently-enabled ones.
fn spark_panel_hints(panel: SparkPanel, app: &App) -> Vec<(&'static str, String)> {
    let fkey = match panel {
        SparkPanel::Cpu => "F4",
        SparkPanel::Mem => "F5",
        SparkPanel::Zmq => "F6",
        SparkPanel::Events => "F7",
        SparkPanel::Activity => "F8",
    };
    let mut hints = vec![(fkey, "hide".to_string())];
    if enabled_spark_panels(app).first() == Some(&panel) {
        let style_label = if app.config.sparkline_style() == SparklineStyle::Dots { "bars" } else { "dots" };
        hints.push(("m", style_label.to_string()));
    }
    hints
}

fn count_keys_with_status(keys: &[crate::types::KeyInfo], status: crate::types::KeyStatus) -> usize {
    keys.iter().filter(|k| k.status == status).count()
}

pub(super) fn format_duration(secs: u64) -> String {
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

fn pct_color(pct: f64) -> Color {
    color::gradient(pct / 100.0)
}

fn mem_color(pct: f64) -> Color {
    color::mem_gradient(pct / 100.0)
}

/// Whether the agent has reported host figures yet (load, memory, CPU). Until
/// then they're all zero — e.g. the agent is down or still starting — and must
/// read "n/a", not 0% load and a full "100%" memory bar from dividing by zero.
fn host_data(app: &App) -> bool {
    app.system_info.mem_total_kb > 0
}

/// How a sparkline is colored: one fixed color, or btop-style by level —
/// each row by its height on the graph's fixed 0–100 scale, so a bar's top
/// shows how high it reached and history never recolors as new values come in.
enum GraphColor {
    Solid(Color),
    ByLevel(fn(f64) -> Color),
}

/// Position of a latency on the ZMQ graph's log scale, 0.0..=1.0, with
/// `ceiling` at the top — log so a healthy few-ms figure still shows instead
/// of hugging the floor next to the ceiling (1 ms ≈ 15%, 10 ms ≈ 50% of a
/// 100 ms scale).
fn latency_level(ms: f64, ceiling: f64) -> f64 {
    ((1.0 + ms.max(0.0)).ln() / (1.0 + ceiling.max(1.0)).ln()).clamp(0.0, 1.0)
}

fn pct_gauge_style(pct: f64) -> Style {
    Style::default().fg(pct_color(pct)).bg(color::meter_bg())
}

fn mem_gauge_style(pct: f64) -> Style {
    Style::default().fg(mem_color(pct)).bg(color::meter_bg())
}

// ── Minion Key Status ───────────────────────────────────────────────────────

fn draw_key_status(f: &mut Frame, area: Rect, app: &App) {
    let accepted = count_keys_with_status(&app.keys, crate::types::KeyStatus::Accepted);
    let pending = count_keys_with_status(&app.keys, crate::types::KeyStatus::Pending);
    let rejected = count_keys_with_status(&app.keys, crate::types::KeyStatus::Rejected);
    let denied = count_keys_with_status(&app.keys, crate::types::KeyStatus::Denied);

    let churn = app.pending_key_churn_per_min();
    let title = Line::from(vec![
        Span::styled(" Minion Key Status ", Style::default().fg(color::title())),
        Span::styled(
            format!("new pending:{}/min", churn),
            Style::default().fg(if churn > 0 { color::yellow() } else { color::inactive_fg() }),
        ),
    ]);
    let block = make_block_line(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let cols: [(u16, &str, usize, Color); 4] = [
        (1, "Accepted", accepted, color::green()),
        (2, "Denied", denied, if denied > 0 { color::red() } else { color::inactive_fg() }),
        (3, "Unaccepted", pending, if pending > 0 { color::yellow() } else { color::inactive_fg() }),
        (4, "Rejected", rejected, if rejected > 0 { color::red() } else { color::inactive_fg() }),
    ];

    let mut lines = Vec::new();
    for (n, label, count, fg) in &cols {
        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", n), Style::default().fg(color::label_fg())),
            Span::styled(format!("{:<15}", *label), Style::default().fg(color::main_fg())),
            Span::styled(format!("{}", count), Style::default().fg(*fg).add_modifier(Modifier::BOLD)),
        ]));
    }

    // total keys as last row
    let total_keys = accepted + pending + rejected + denied;
    if total_keys > 0 {
        lines.push(Line::from(Span::styled(
            "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}".to_string(),
            Style::default().fg(color::border()),
        )));
        lines.push(Line::from(vec![
            Span::styled("    Total keys  ", Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{}", total_keys), Style::default().fg(color::cyan()).add_modifier(Modifier::BOLD)),
        ]));
    }

    f.render_widget(Paragraph::new(lines).style(Style::default().bg(color::main_bg())), inner);
}

// ── Processes & Services ─────────────────────────────────────────────────────

/// Group processes by role for the Services panel's per-role memory line.
/// Returns (role_label, total_rss_kb, process_count) in first-seen order.
fn group_processes(processes: &[crate::types::SaltProcess]) -> Vec<(String, u64, usize)> {
    let mut groups: Vec<(String, u64, usize)> = vec![];
    for p in processes {
        // Unknown means the agent's classifier couldn't match this pid to a
        // Salt process at all -- it isn't a Salt sub-role to display, it's
        // (should the agent's own filtering ever regress) an unrelated
        // process on the host that must not be counted as "Salt RSS".
        let label = match p.role {
            crate::types::SaltProcessRole::Master => "Master",
            crate::types::SaltProcessRole::Api => "API",
            crate::types::SaltProcessRole::Worker => "Worker",
            crate::types::SaltProcessRole::Syndic => "Syndic",
            crate::types::SaltProcessRole::Minion => "Minion",
            crate::types::SaltProcessRole::EventPublisher => "EventPub",
            crate::types::SaltProcessRole::ReqServer => "ReqSrv",
            crate::types::SaltProcessRole::Unknown => continue,
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
    // Unknown-role entries are excluded -- the agent already only reports
    // actual Salt processes, but this guards against ever again silently
    // summing every process on the host (VS Code, a browser, ...) into
    // "Salt RSS", which is how that figure previously read over 100%.
    let salt_processes = app.processes.iter().filter(|p| p.role != crate::types::SaltProcessRole::Unknown);
    let total_rss_used: u64 = salt_processes.clone().map(|p| p.rss_kb).sum();
    let mem_pct = if host_data(app) {
        (total_rss_used as f64 / app.system_info.mem_total_kb as f64) * 100.0
    } else {
        0.0
    };

    let proc_count = salt_processes.count();

    let title = Line::from(vec![
        Span::styled(" Services ", Style::default().fg(color::title())),
        if host_data(app) {
            Span::styled(format!("Mem: {:.0}% ", mem_pct), Style::default().fg(mem_color(mem_pct)))
        } else {
            Span::styled("Mem: n/a ", Style::default().fg(color::inactive_fg()))
        },
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
            "API" => color::purple(),
            "Master" => color::green(),
            "Worker" => color::cyan(),
            _ => color::main_fg(),
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
    // Accepted keys, as Minion Key Status counts them.
    let minions = match count_keys_with_status(&app.keys, crate::types::KeyStatus::Accepted) {
        0 => app.pulse_minions_count(),
        n => n,
    };
    let gauge = Gauge::default()
        .gauge_style(mem_gauge_style(mem_pct))
        .percent((mem_pct.round() as u16).min(100))
        .label(format!(
            "Salt RSS {} MB ({} proc)  {} minions{}",
            total_rss_used / 1024, proc_count, minions, workers_label,
        ));

    let mut service_lines: Vec<Line<'static>> = vec![];

    service_lines.push(Line::from(vec![
        Span::styled("Unit                    ", Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD)),
        Span::styled("Status", Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD)),
        Span::styled(" State      ", Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD)),
        Span::styled(" Flp", Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD)),
        Span::styled(" Memory   ", Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD)),
        Span::styled(" T", Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD)),
        Span::styled(" Uptime", Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD)),
    ]));

    for service in &app.services {
        let status_icon = if service.active { "\u{25cf}" } else { "\u{25cb}" };
        let status_color = if service.active { color::green() } else { color::red() };

        // Combined string is right-aligned as a whole (rather than padding
        // just the number to a 3-digit budget) so multi-GB services don't
        // overflow the column — up to 5 digits (99999 MB) fits.
        let memory_mb = service.memory_kb / 1024;
        let memory_text = if memory_mb > 0 {
            format!("{} MB", memory_mb)
        } else {
            "--".to_string()
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
            color::red()
        } else if flap_count > 0 {
            color::yellow()
        } else {
            color::inactive_fg()
        };

        // Memory growth trend indicator, from a rolling linear-regression slope.
        let (trend_char, trend_color) = match app.service_memory_growth(&service.unit) {
            Some((rate_mb_per_hour, still_growing)) if still_growing && rate_mb_per_hour > app.config.service_mem_growth_mb_per_hour => {
                ("\u{2191}", color::red()) // sustained leak-rate growth
            }
            Some((rate_mb_per_hour, _)) if rate_mb_per_hour > 1.0 => ("\u{2197}", color::yellow()), // mild growth
            Some((rate_mb_per_hour, _)) if rate_mb_per_hour < -1.0 => ("\u{2193}", color::green()), // shrinking
            Some(_) => ("\u{2192}", color::inactive_fg()), // flat
            None => ("\u{00b7}", color::inactive_fg()), // not enough samples yet
        };

        service_lines.push(Line::from(vec![
            Span::styled(format!("{:<24}", unit_display), Style::default().fg(color::cyan())),
            Span::styled(format!("{:>6}", status_icon), Style::default().fg(status_color)),
            Span::styled(format!(" {:<11}", state_display), Style::default().fg(color::main_fg())),
            Span::styled(format!(" {:>3}", flap_text), Style::default().fg(flap_color)),
            Span::styled(format!(" {:>8} ", memory_text), Style::default().fg(color::yellow())),
            Span::styled(format!(" {}", trend_char), Style::default().fg(trend_color)),
            Span::styled(format!(" {}", uptime_text), Style::default().fg(color::inactive_fg())),
        ]));
    }

    f.render_widget(gauge, inner_layout[0]);

    f.render_widget(
        Paragraph::new(Line::from(proc_spans))
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(color::main_bg())),
        inner_layout[1],
    );

    f.render_widget(
        Paragraph::new(service_lines).style(Style::default().bg(color::main_bg())),
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
    if !host_data(app) {
        let title = Line::from(vec![
            Span::styled(" Host Resources ", Style::default().fg(color::title())),
            Span::styled("waiting for agent\u{2026} ", Style::default().fg(color::inactive_fg())),
        ]);
        let block = make_block_line(title);
        let inner = block.inner(area);
        f.render_widget(block, area);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Ratio(1, 3), Constraint::Ratio(1, 3), Constraint::Ratio(1, 3)])
            .split(inner);
        for (row, label) in rows.iter().zip(["Load: n/a", "Mem: n/a", "Net: n/a"]) {
            let gauge = Gauge::default()
                .gauge_style(Style::default().fg(color::inactive_fg()).bg(color::meter_bg()))
                .percent(0)
                .label(label);
            f.render_widget(gauge, *row);
        }
        return;
    }
    let mem_total_kb = app.system_info.mem_total_kb;
    let mem_avail_kb = app.system_info.mem_avail_kb;
    let mem_pct = ((mem_total_kb - mem_avail_kb) as f64 / mem_total_kb as f64) * 100.0;

    // The core count comes with the agent's config snapshot; without it there
    // is no meaningful load percentage, only the raw load averages.
    let cpu_count = app.config_snapshot.as_ref().map(|c| c.cpu_count).filter(|&n| n > 0);
    let load_pct = cpu_count.map_or(0, |n| (app.system_info.load_1 / n as f64 * 100.0).clamp(0.0, 100.0) as u16);

    let net_rate_str = format!(
        "Net: \u{25bc}{} \u{25b2}{}",
        format_bytes(app.system_info.network_rx_rate),
        format_bytes(app.system_info.network_tx_rate),
    );

    let title = Line::from(vec![
        Span::styled(" Host Resources ", Style::default().fg(color::title())),
        Span::styled(
            format!("Load: {:.1}/{:.1}/{:.1}  Mem: {:.0}%  {}", app.system_info.load_1, app.system_info.load_5, app.system_info.load_15, mem_pct, net_rate_str),
            Style::default().fg(color::inactive_fg()),
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
        .label(match cpu_count {
            Some(n) => format!(
                "Load: {:.1}/{:.1}/{:.1}  ({}-core host  {:.0}%)",
                app.system_info.load_1, app.system_info.load_5, app.system_info.load_15, n, load_pct,
            ),
            None => format!(
                "Load: {:.1}/{:.1}/{:.1}",
                app.system_info.load_1, app.system_info.load_5, app.system_info.load_15,
            ),
        });

    let mem_total_mb = mem_total_kb / 1024;
    let mem_used_mb = (mem_total_kb.saturating_sub(mem_avail_kb)) / 1024;
    let mem_gauge = Gauge::default()
        .gauge_style(mem_gauge_style(mem_pct))
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
    let net_color = if net_rx_rate + net_tx_rate > 1_000_000.0 { color::cyan() } else { color::inactive_fg() };
    let net_gauge = Gauge::default()
        .gauge_style(Style::default().fg(net_color).bg(color::meter_bg()))
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

pub(super) fn format_uptime(secs: u64) -> String {
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
    let ceiling = app.config.zmq_latency_ceiling();
    let lat_color = if app.zmq_latency_seen { color::gradient(latency_level(lat, ceiling)) } else { color::inactive_fg() };

    let peak_lat = app.zmq_latency_history.iter().cloned().fold(0.0_f64, f64::max);

    // Label and "no data" hint depend on which source is configured to feed
    // this KPI (config.zmq_latency_source) — see App::zmq_latency_ms doc.
    let (source_label, na_hint) = match app.config.zmq_latency_source.as_str() {
        "master_stats" => ("master exec", "n/a (needs master_stats: True)"),
        "job_ping" => ("job ping", "n/a (waiting for round-trip)"),
        _ => ("agent IPC", "measuring..."),
    };
    let lat_display = if app.zmq_latency_seen { format!("{:.1}ms", lat) } else { na_hint.to_string() };
    let peak_label = if app.zmq_latency_seen { format!("{:.1}ms", peak_lat) } else { "n/a".to_string() };

    let pub_port = app.config_snapshot.as_ref().map(|c| c.publish_port).unwrap_or(4505);
    let ret_port = app.config_snapshot.as_ref().map(|c| c.ret_port).unwrap_or(4506);
    let api_port = app.config_snapshot.as_ref().map(|c| c.api_port).unwrap_or(0);

    let pub_established = app.zmq_connections.iter().filter(|c| c.local_port == pub_port && c.state == "ESTABLISHED").count();
    let pub_total = app.zmq_connections.iter().filter(|c| c.local_port == pub_port).count();
    let ret_established = app.zmq_connections.iter().filter(|c| c.local_port == ret_port && c.state == "ESTABLISHED").count();
    let ret_total = app.zmq_connections.iter().filter(|c| c.local_port == ret_port).count();

    // Build title — richer detail as the panel grows wide enough to fit it.
    let title = if area.width >= 90 {
        let mut spans = vec![
            Span::styled(" ZMQ ", Style::default().fg(color::title())),
            Span::styled("\u{25bc} ", Style::default().fg(color::cyan())),
            Span::styled(format!("{} ", lat_display), Style::default().fg(lat_color).add_modifier(Modifier::BOLD)),
            Span::styled("pk:", Style::default().fg(color::label_fg())),
            Span::styled(format!("{} ", peak_label), Style::default().fg(color::inactive_fg())),
            Span::styled(format!("({}, scale {:.0}ms) ", source_label, ceiling), Style::default().fg(color::label_fg())),
            Span::styled(format!(":{} pub {}/{} ", pub_port, pub_established, pub_total), Style::default().fg(color::green()).add_modifier(Modifier::BOLD)),
            Span::styled(format!(":{} ret {}/{}", ret_port, ret_established, ret_total), Style::default().fg(color::green()).add_modifier(Modifier::BOLD)),
        ];
        if api_port > 0 {
            let api_estab = app.zmq_connections.iter().filter(|c| c.local_port == api_port && c.state == "ESTABLISHED").count();
            let api_total = app.zmq_connections.iter().filter(|c| c.local_port == api_port).count();
            spans.push(Span::styled(format!(" :{} api {}/{}", api_port, api_estab, api_total), Style::default().fg(color::cyan()).add_modifier(Modifier::BOLD)));
        }
        Line::from(spans)
    } else if area.width >= 40 {
        Line::from(vec![
            Span::styled(" ZMQ ", Style::default().fg(color::title())),
            Span::styled("\u{25bc} ", Style::default().fg(color::cyan())),
            Span::styled(format!("{} ", lat_display), Style::default().fg(lat_color).add_modifier(Modifier::BOLD)),
            Span::styled("pk:", Style::default().fg(color::label_fg())),
            Span::styled(peak_label, Style::default().fg(color::inactive_fg())),
        ])
    } else {
        Line::from(vec![
            Span::styled(" ZMQ ", Style::default().fg(color::title())),
            Span::styled("\u{25bc} ", Style::default().fg(color::cyan())),
            Span::styled(lat_display, Style::default().fg(lat_color).add_modifier(Modifier::BOLD)),
        ])
    };

    let block = make_block_line_hinted(title, &spark_panel_hints(SparkPanel::Zmq, app));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lat_idx = app.zmq_latency_idx;
    let ordered_lat: Vec<f64> = (0..2000)
        .map(|i| app.zmq_latency_history[(lat_idx + i) % 2000])
        .collect();
    let spark_width = inner.width.max(1) as usize;
    let points = spark_width.min(2000);
    let start = ordered_lat.len().saturating_sub(points);
    let spark_data: Vec<u64> = (0..points)
        .map(|i| {
            (latency_level(ordered_lat[start + i], ceiling) * 100.0).round() as u64
        })
        .collect();
    render_sparkline(f, inner, &spark_data, GraphColor::ByLevel(color::gradient), app.config.sparkline_style());
}

// ── Event Type Breakdown ────────────────────────────────────────────────────

fn draw_event_breakdown(f: &mut Frame, area: Rect, app: &App) {
    let title = Line::from(vec![
        Span::styled(" Event Types ", Style::default().fg(color::title())),
        Span::styled(format!("auth: {:.1}/s", app.current_auth_rate), Style::default().fg(color::inactive_fg())),
    ]);
    let block = make_block_line(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let bd = &app.event_breakdown;
    let cats = [
        ("Auth",  bd.auth,  color::yellow()),
        ("Job",   bd.job_return, color::cyan()),
        ("Start", bd.start, color::green()),
        ("Beacon", bd.beacon, color::purple()),
        ("Refresh", bd.refresh, color::orange()),
        ("Other", bd.other, color::inactive_fg()),
    ];
    let max_count = cats.iter().map(|(_, c, _)| *c).max().unwrap_or(1).max(1);
    // Sized to the widest count actually present (min 5, matching the old
    // fixed width) instead of a hardcoded 5 digits — once any category
    // passes 99999, a fixed width let that count's digits run into the bar
    // column, and every row's bar started at a different column depending
    // on its own count's digit width.
    let count_width = cats.iter().map(|(_, c, _)| c.to_string().len()).max().unwrap_or(1).max(5);
    let prefix_width = 1 + 7 + count_width + 1; // leading space + label + count + trailing space

    let mut lines: Vec<Line> = vec![];
    if bd.total > 0 {
        for (label, count, color) in &cats {
            let bar_max = inner.width.saturating_sub(prefix_width as u16);
            let bar_len = ((*count as f64 / max_count as f64) * bar_max as f64).round() as usize;
            let bar = "\u{2588}".repeat(bar_len);
            let spans = vec![
                Span::styled(format!(" {:<7}", label), Style::default().fg(*color).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{:>width$} ", count, width = count_width), Style::default().fg(color::inactive_fg())),
                Span::styled(bar, Style::default().fg(*color)),
            ];
            lines.push(Line::from(spans));
        }
    } else {
        lines.push(Line::from(vec![
            Span::styled(" waiting for events...", Style::default().fg(color::inactive_fg())),
        ]));
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::main_bg())),
        inner,
    );
}

// ── Event Rate ───────────────────────────────────────────────────────────────

fn draw_events_row(f: &mut Frame, area: Rect, app: &App) {
    let peak_rate = app.event_rate_history.iter().cloned().fold(0.0_f64, f64::max);
    let last_seen = if app.secs_since_last_event == u64::MAX {
        "\u{2014}".to_string()
    } else {
        format!("{}s ago", app.secs_since_last_event)
    };
    let last_color = if app.secs_since_last_event < 5 { color::green() }
        else if app.secs_since_last_event < 30 { color::yellow() }
        else { color::orange() };

    // Build title — richer detail as the panel grows wide enough to fit it.
    let title = if area.width >= 70 {
        Line::from(vec![
            Span::styled(" Events ", Style::default().fg(color::title())),
            Span::styled("\u{25b6} ", Style::default().fg(color::cyan())),
            Span::styled(format!("{:.1}/s ", app.events_per_sec), Style::default().fg(color::green()).add_modifier(Modifier::BOLD)),
            Span::styled("pk:", Style::default().fg(color::label_fg())),
            Span::styled(format!("{:.1}/s ", peak_rate), Style::default().fg(color::inactive_fg())),
            Span::styled("tot:", Style::default().fg(color::label_fg())),
            Span::styled(format!("{} ", app.events_total), Style::default().fg(color::inactive_fg())),
            Span::styled("last:", Style::default().fg(color::label_fg())),
            Span::styled(last_seen, Style::default().fg(last_color)),
        ])
    } else if area.width >= 40 {
        Line::from(vec![
            Span::styled(" Events ", Style::default().fg(color::title())),
            Span::styled("\u{25b6} ", Style::default().fg(color::cyan())),
            Span::styled(format!("{:.1}/s ", app.events_per_sec), Style::default().fg(color::green()).add_modifier(Modifier::BOLD)),
            Span::styled("pk:", Style::default().fg(color::label_fg())),
            Span::styled(format!("{:.1}/s ", peak_rate), Style::default().fg(color::inactive_fg())),
            Span::styled("tot: ", Style::default().fg(color::label_fg())),
            Span::styled(format!("{}", app.events_total), Style::default().fg(color::inactive_fg())),
        ])
    } else {
        Line::from(vec![
            Span::styled(" Events ", Style::default().fg(color::title())),
            Span::styled("\u{25b6} ", Style::default().fg(color::cyan())),
            Span::styled(format!("{:.1}/s", app.events_per_sec), Style::default().fg(color::green()).add_modifier(Modifier::BOLD)),
        ])
    };

    let block = make_block_line_hinted(title, &spark_panel_hints(SparkPanel::Events, app));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let max_rate = app.event_rate_history.iter().cloned().fold(0.0_f64, f64::max).max(0.1);
    let idx = app.event_rate_idx;
    let ordered: Vec<f64> = (0..2000)
        .map(|i| app.event_rate_history[(idx + i) % 2000])
        .collect();
    let spark_width = inner.width.max(1) as usize;
    let points = spark_width.min(2000);
    let start = ordered.len().saturating_sub(points);
    let spark_data: Vec<u64> = (0..points)
        .map(|i| {
            let v = ordered[start + i];
            (v / max_rate * 100.0) as u64
        })
        .collect();

    render_sparkline(f, inner, &spark_data, GraphColor::Solid(color::cyan()), app.config.sparkline_style());
}

// ── Minion Activity Matrix ───────────────────────────────────────────────────

fn draw_activity_matrix(f: &mut Frame, area: Rect, app: &App) {
    let elapsed = app.last_activity_at.elapsed().as_secs();
    let current_rate = app.current_sec_returns;
    let rate_color = if current_rate > 0 { color::green() } else { color::inactive_fg() };

    let mut total_returns: u64 = 0;
    for v in &app.activity_matrix { total_returns += v; }

    // Build title — richer detail as the panel grows wide enough to fit it.
    let title = if area.width >= 70 {
        let last_str = if app.last_activity_minion.is_empty() {
            "waiting for minion returns...".to_string()
        } else if elapsed < 60 {
            format!("\u{2190} {} \u{2192} {} ({}s ago)", app.last_activity_minion, app.last_activity_fun, elapsed)
        } else {
            format!("\u{2190} {} \u{2192} {} ({}m ago)", app.last_activity_minion, app.last_activity_fun, elapsed / 60)
        };
        Line::from(vec![
            Span::styled(" Minions ", Style::default().fg(color::title())),
            Span::styled("\u{25c9} ", Style::default().fg(color::green())),
            Span::styled(format!("{}/s ", current_rate), Style::default().fg(rate_color).add_modifier(Modifier::BOLD)),
            Span::styled("last: ", Style::default().fg(color::label_fg())),
            Span::styled(format!("{} ", last_str), Style::default().fg(color::inactive_fg())),
            Span::styled("total:", Style::default().fg(color::label_fg())),
            Span::styled(format!("{}", total_returns), Style::default().fg(color::inactive_fg())),
        ])
    } else if area.width >= 40 {
        let last_str = if app.last_activity_minion.is_empty() {
            "waiting...".to_string()
        } else {
            app.last_activity_minion.to_string()
        };
        Line::from(vec![
            Span::styled(" Minions ", Style::default().fg(color::title())),
            Span::styled("\u{25c9} ", Style::default().fg(color::green())),
            Span::styled(format!("{}/s ", current_rate), Style::default().fg(rate_color).add_modifier(Modifier::BOLD)),
            Span::styled("last: ", Style::default().fg(color::label_fg())),
            Span::styled(last_str, Style::default().fg(color::inactive_fg())),
        ])
    } else {
        Line::from(vec![
            Span::styled(" Minions ", Style::default().fg(color::title())),
            Span::styled("\u{25c9} ", Style::default().fg(color::green())),
            Span::styled(format!("{}/s", current_rate), Style::default().fg(rate_color).add_modifier(Modifier::BOLD)),
        ])
    };

    let block = make_block_line_hinted(title, &spark_panel_hints(SparkPanel::Activity, app));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let max_val = app.activity_matrix.iter().cloned().max().unwrap_or(0).max(1);
    let act_idx = app.activity_matrix_idx;
    let ordered_act: Vec<u64> = (0..2000)
        .map(|i| app.activity_matrix[(act_idx + i) % 2000])
        .collect();
    let spark_width = inner.width.max(1) as usize;
    let points = spark_width.min(2000);
    let start = ordered_act.len().saturating_sub(points);
    let spark_data: Vec<u64> = (0..points)
        .map(|i| {
            let v = ordered_act[start + i];
            (v * 100 / max_val) as u64
        })
        .collect();

    render_sparkline(f, inner, &spark_data, GraphColor::Solid(color::green()), app.config.sparkline_style());
}

// ── CPU / Memory history ────────────────────────────────────────────────────
// Optional Dashboard sparkline panels (config.show_cpu_graph / show_mem_graph)
// giving CPU load and Memory utilization the same rolling-history treatment
// draw_zmq_row/draw_events_row already give latency/event-rate — unlike those,
// the values are already percentages, so the sparkline is rendered on a fixed
// 0-100 scale instead of auto-scaling to the window's own peak (auto-scaling
// a percentage would visually exaggerate a fleet that never gets very busy).

fn draw_cpu_row(f: &mut Frame, area: Rect, app: &App) {
    let current_pct = app.system_info.cpu_percent;
    let (current, current_fg) = if host_data(app) {
        (format!("{:.0}%", current_pct), pct_color(current_pct))
    } else {
        ("n/a".to_string(), color::inactive_fg())
    };
    let peak_pct = app.cpu_history.iter().cloned().fold(0.0_f64, f64::max);

    let idx = app.cpu_history_idx;
    let ordered: Vec<f64> = (0..2000).map(|i| app.cpu_history[(idx + i) % 2000]).collect();
    let n = (app.cpu_mem_samples as usize).min(ordered.len());
    let avg_pct = if n > 0 { ordered[ordered.len() - n..].iter().sum::<f64>() / n as f64 } else { 0.0 };

    let title = if area.width >= 70 {
        Line::from(vec![
            Span::styled(" CPU ", Style::default().fg(color::title())),
            Span::styled(format!("{} ", current), Style::default().fg(current_fg).add_modifier(Modifier::BOLD)),
            Span::styled("pk:", Style::default().fg(color::label_fg())),
            Span::styled(format!("{:.0}% ", peak_pct), Style::default().fg(color::inactive_fg())),
            Span::styled("avg:", Style::default().fg(color::label_fg())),
            Span::styled(format!("{:.0}% ", avg_pct), Style::default().fg(color::inactive_fg())),
            Span::styled("load:", Style::default().fg(color::label_fg())),
            Span::styled(
                if host_data(app) { format!("{:.1}/{:.1}/{:.1}", app.system_info.load_1, app.system_info.load_5, app.system_info.load_15) } else { "n/a".to_string() },
                Style::default().fg(color::inactive_fg()),
            ),
        ])
    } else if area.width >= 40 {
        Line::from(vec![
            Span::styled(" CPU ", Style::default().fg(color::title())),
            Span::styled(format!("{} ", current), Style::default().fg(current_fg).add_modifier(Modifier::BOLD)),
            Span::styled("pk:", Style::default().fg(color::label_fg())),
            Span::styled(format!("{:.0}%", peak_pct), Style::default().fg(color::inactive_fg())),
        ])
    } else {
        Line::from(vec![
            Span::styled(" CPU ", Style::default().fg(color::title())),
            Span::styled(current.clone(), Style::default().fg(current_fg).add_modifier(Modifier::BOLD)),
        ])
    };

    let block = make_block_line_hinted(title, &spark_panel_hints(SparkPanel::Cpu, app));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let spark_width = inner.width.max(1) as usize;
    let points = spark_width.min(ordered.len());
    let start = ordered.len().saturating_sub(points);
    let spark_data: Vec<u64> = (start..ordered.len()).map(|i| ordered[i].round() as u64).collect();

    render_sparkline(f, inner, &spark_data, GraphColor::ByLevel(color::gradient), app.config.sparkline_style());
}

fn draw_mem_row(f: &mut Frame, area: Rect, app: &App) {
    let mem_total_kb = app.system_info.mem_total_kb.max(1);
    let have = host_data(app);
    let mem_avail_kb = app.system_info.mem_avail_kb;
    let current_pct = ((mem_total_kb.saturating_sub(mem_avail_kb)) as f64 / mem_total_kb as f64 * 100.0).clamp(0.0, 100.0);
    let peak_pct = app.mem_history.iter().cloned().fold(0.0_f64, f64::max);

    let idx = app.mem_history_idx;
    let ordered: Vec<f64> = (0..2000).map(|i| app.mem_history[(idx + i) % 2000]).collect();
    let n = (app.cpu_mem_samples as usize).min(ordered.len());
    let avg_pct = if n > 0 { ordered[ordered.len() - n..].iter().sum::<f64>() / n as f64 } else { 0.0 };
    let mem_used_mb = mem_total_kb.saturating_sub(mem_avail_kb) / 1024;
    let mem_total_mb = mem_total_kb / 1024;
    let (current, current_fg) = if have {
        (format!("{:.0}%", current_pct), mem_color(current_pct))
    } else {
        ("n/a".to_string(), color::inactive_fg())
    };
    let used = if have { format!("{} MB / {} MB", mem_used_mb, mem_total_mb) } else { "n/a".to_string() };

    let title = if area.width >= 70 {
        Line::from(vec![
            Span::styled(" Memory ", Style::default().fg(color::title())),
            Span::styled(format!("{} ", current), Style::default().fg(current_fg).add_modifier(Modifier::BOLD)),
            Span::styled("pk:", Style::default().fg(color::label_fg())),
            Span::styled(format!("{:.0}% ", peak_pct), Style::default().fg(color::inactive_fg())),
            Span::styled("avg:", Style::default().fg(color::label_fg())),
            Span::styled(format!("{:.0}% ", avg_pct), Style::default().fg(color::inactive_fg())),
            Span::styled("used:", Style::default().fg(color::label_fg())),
            Span::styled(used, Style::default().fg(color::inactive_fg())),
        ])
    } else if area.width >= 40 {
        Line::from(vec![
            Span::styled(" Memory ", Style::default().fg(color::title())),
            Span::styled(format!("{} ", current), Style::default().fg(current_fg).add_modifier(Modifier::BOLD)),
            Span::styled("pk:", Style::default().fg(color::label_fg())),
            Span::styled(format!("{:.0}%", peak_pct), Style::default().fg(color::inactive_fg())),
        ])
    } else {
        Line::from(vec![
            Span::styled(" Memory ", Style::default().fg(color::title())),
            Span::styled(current.clone(), Style::default().fg(current_fg).add_modifier(Modifier::BOLD)),
        ])
    };

    let block = make_block_line_hinted(title, &spark_panel_hints(SparkPanel::Mem, app));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let spark_width = inner.width.max(1) as usize;
    let points = spark_width.min(ordered.len());
    let start = ordered.len().saturating_sub(points);
    let spark_data: Vec<u64> = (start..ordered.len()).map(|i| ordered[i].round() as u64).collect();

    render_sparkline(f, inner, &spark_data, GraphColor::ByLevel(color::mem_gradient), app.config.sparkline_style());
}

// ── Job Pulse ────────────────────────────────────────────────────────────────

fn draw_job_pulse(f: &mut Frame, area: Rect, app: &App) {
    let active_count = app.jobs.len();

    let title = Line::from(vec![
        Span::styled(" Job Pulse ", Style::default().fg(color::title())),
        Span::styled(format!("Active: {}", active_count), Style::default().fg(if active_count > 0 { color::cyan() } else { color::inactive_fg() })),
    ]);
    let block = make_block_line(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let success = app.job_success_pct().map(|p| format!("{:.0}%", p)).unwrap_or_else(|| "--".into());
    let stuck = app.job_stuck_count();
    let (p50, p95) = app.job_completion_latency_p50_p95().unwrap_or((0.0, 0.0));
    let partial = app.job_partial_pct().map(|p| format!("{:.0}%", p)).unwrap_or_else(|| "--".into());

    let mut lines: Vec<Line<'static>> = vec![Line::from(vec![
        Span::styled("  success:", Style::default().fg(color::label_fg())),
        Span::styled(success, Style::default().fg(color::green()).add_modifier(Modifier::BOLD)),
        Span::styled(" stuck:", Style::default().fg(color::label_fg())),
        Span::styled(stuck.to_string(), Style::default().fg(if stuck > 0 { color::orange() } else { color::inactive_fg() }).add_modifier(Modifier::BOLD)),
        Span::styled(" p50/p95:", Style::default().fg(color::label_fg())),
        Span::styled(format!("{:.1}s/{:.1}s", p50, p95), Style::default().fg(color::cyan()).add_modifier(Modifier::BOLD)),
        Span::styled(" partial:", Style::default().fg(color::label_fg())),
        Span::styled(partial, Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
    ])];

    if app.jobs.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no active jobs",
            Style::default().fg(color::inactive_fg()),
        )));
        f.render_widget(
            Paragraph::new(lines).style(Style::default().bg(color::main_bg())),
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
            color::green()
        } else {
            color::yellow()
        };

        let fun_short = if job.fun.len() > 35 {
            format!("{}..", &job.fun[..33])
        } else {
            job.fun.clone()
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", fun_short), Style::default().fg(color::cyan())),
            Span::styled(progress, Style::default().fg(color).add_modifier(Modifier::BOLD)),
        ]));
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::main_bg())),
        inner,
    );
}

/// salt-api's own in-memory request stats (rest_cherrypy's `collect_stats`
/// option, polled over its /stats endpoint — see agent's `cmd_get_api_stats`).
/// Only drawn at all when a salt-api process is actually detected running;
/// even then, the stats themselves need `collect_stats: True` set on the
/// master, plus either `stats_disable_auth: True` (the default,
/// unauthenticated) or a `stats_auth_mode` set in the Config Editor (an
/// eauth login instead) — so this doubles as a nudge to turn one of those
/// on.
fn draw_api_row(f: &mut Frame, area: Rect, app: &App) {
    let stats = &app.api_stats;

    let title = Line::from(vec![
        Span::styled(" salt-api ", Style::default().fg(color::title())),
        if stats.reachable {
            // Recent rate (App::api_requests_per_sec_recent), not cpstats'
            // own Requests/Second -- that's a lifetime average since salt-api
            // started, which barely moves once uptime is more than a few
            // minutes and would make this look frozen on a long-running server.
            Span::styled(format!("{:.1} req/s", app.api_requests_per_sec_recent), Style::default().fg(color::cyan()))
        } else {
            Span::styled("stats unavailable", Style::default().fg(color::inactive_fg()))
        },
    ]);
    let block = make_block_line(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if !stats.reachable {
        let hint = if !stats.configured {
            "  rest_cherrypy not configured".to_string()
        } else if app.config.api_stats_auth_mode != "none" {
            format!("  {}:{} unreachable — {}", stats.scheme, stats.port, if stats.error.is_empty() { "check stats_auth_mode settings ('F3')" } else { stats.error.as_str() })
        } else {
            format!(
                "  {}:{} unreachable — set rest_cherrypy: {{collect_stats: True, stats_disable_auth: True}}",
                stats.scheme, stats.port,
            )
        };
        f.render_widget(
            Paragraph::new(vec![Line::from(Span::styled(hint, Style::default().fg(color::inactive_fg())))])
                .style(Style::default().bg(color::main_bg()))
                .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }

    let err_pct = app.api_error_pct();
    let err_str = err_pct.map(|p| format!("{:.0}%", p)).unwrap_or_else(|| "--".into());
    let err_color = match err_pct {
        Some(p) if p >= 10.0 => color::red(),
        Some(p) if p > 0.0 => color::yellow(),
        Some(_) => color::green(),
        None => color::inactive_fg(),
    };
    let login_p95 = app.api_login_latency_p50_p95().map(|(_, p95)| p95);
    let login_str = login_p95.map(|s| format!("{:.2}s", s)).unwrap_or_else(|| "--".into());
    let login_color = match login_p95 {
        Some(s) if s >= 3.0 => color::red(),
        Some(s) if s >= 1.0 => color::yellow(),
        Some(_) => color::green(),
        None => color::inactive_fg(),
    };

    let mut lines: Vec<Line<'static>> = vec![Line::from(vec![
        Span::styled("  current:", Style::default().fg(color::label_fg())),
        Span::styled(stats.current_requests.to_string(), Style::default().fg(color::cyan()).add_modifier(Modifier::BOLD)),
        Span::styled(" total:", Style::default().fg(color::label_fg())),
        Span::styled(stats.total_requests.to_string(), Style::default().fg(color::inactive_fg())),
        Span::styled(" errors:", Style::default().fg(color::label_fg())),
        Span::styled(err_str, Style::default().fg(err_color).add_modifier(Modifier::BOLD)),
        Span::styled(" login p95:", Style::default().fg(color::label_fg())),
        Span::styled(login_str, Style::default().fg(login_color).add_modifier(Modifier::BOLD)),
        Span::styled(" in/out:", Style::default().fg(color::label_fg())),
        Span::styled(
            format!("{} / {}", format_bytes(stats.bytes_read_per_sec), format_bytes(stats.bytes_written_per_sec)),
            Style::default().fg(color::inactive_fg()),
        ),
    ])];

    // Most recent request per CherryPy worker thread — a live snapshot of
    // in-flight/just-finished activity, not a request log (see the doc
    // comment on cmd_get_api_stats for why nothing here is ever logged).
    let mut recent: Vec<&crate::types::ApiRequestRecord> = stats.requests.iter()
        .filter(|r| !r.request_line.is_empty())
        .collect();
    recent.sort_by(|a, b| b.processing_time_secs.partial_cmp(&a.processing_time_secs).unwrap_or(std::cmp::Ordering::Equal));
    if recent.is_empty() {
        lines.push(Line::from(Span::styled("  no requests yet", Style::default().fg(color::inactive_fg()))));
    } else {
        for rec in recent.iter().take(3) {
            let status_color = match rec.response_status.split_whitespace().next() {
                Some(s) if s.starts_with('2') => color::green(),
                Some(s) if s.starts_with('4') || s.starts_with('5') => color::red(),
                _ => color::inactive_fg(),
            };
            let line_short = if rec.request_line.len() > 40 {
                format!("{}..", &rec.request_line[..38])
            } else {
                rec.request_line.clone()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<42}", line_short), Style::default().fg(color::cyan())),
                Span::styled(format!("{:<4}", rec.response_status), Style::default().fg(status_color)),
                Span::styled(format!(" {:.2}s", rec.processing_time_secs), Style::default().fg(color::inactive_fg())),
            ]));
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::main_bg())),
        inner,
    );
}

// ─── Shared sparkline renderer (bars or braille dots) ─────────────────────

fn render_sparkline(f: &mut Frame, area: Rect, data: &[u64], fill: GraphColor, mode: SparklineStyle) {
    let style = Style::default().fg(match fill {
        GraphColor::Solid(c) => c,
        GraphColor::ByLevel(level_color) => level_color(0.0),
    });
    match mode {
        SparklineStyle::Bars => {
            // Every caller already normalizes its data into a fixed 0-100
            // domain (either a raw percentage, or scaled against the panel's
            // own rolling peak) — same assumption Dots mode below already
            // makes via `v.min(100)`. Without an explicit `.max(100)` here,
            // ratatui's Sparkline auto-scales bars to the *visible window's*
            // own max, so e.g. a CPU panel sitting at a real 3% (with no
            // higher value currently on screen) would render as a full-height
            // bar instead of a mostly-empty one.
            let sp = Sparkline::default()
                .block(Block::default())
                .style(style)
                .max(100)
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
                    let threshold = (v.min(100) as u32).saturating_mul(total_pixels).div_ceil(100);
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
    if let GraphColor::ByLevel(level_color) = fill {
        let rows = area.height.max(1);
        for row in 0..area.height {
            let from_bottom = rows - 1 - row;
            let level = if rows > 1 { from_bottom as f64 / (rows - 1) as f64 } else { 0.0 };
            let fg = level_color(level);
            for x in area.left()..area.right() {
                if let Some(cell) = f.buffer_mut().cell_mut((x, area.y + row)) {
                    if cell.symbol() != " " {
                        cell.set_fg(fg);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_scale_is_logarithmic_up_to_the_ceiling() {
        assert_eq!(latency_level(0.0, 100.0), 0.0);
        assert!((latency_level(1.0, 100.0) - 0.15).abs() < 0.01);
        assert!((latency_level(10.0, 100.0) - 0.52).abs() < 0.01);
        assert!((latency_level(50.0, 100.0) - 0.85).abs() < 0.01);
        assert_eq!(latency_level(100.0, 100.0), 1.0);
        assert_eq!(latency_level(5000.0, 100.0), 1.0, "slower than the ceiling clips to the top");
        assert_eq!(latency_level(-3.0, 100.0), 0.0);
    }
}
