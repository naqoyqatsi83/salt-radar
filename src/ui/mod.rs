mod dashboard;
mod minions;
mod events;
mod jobs;
mod alerts_view;

pub mod detail_popup;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::app::DetailPopup;
use crate::types::Tab;

// ─── Theme colors (matching psnet's dark palette) ──────────────────────────

pub mod color {
    use ratatui::style::Color;

    // Background
    pub const BG_DARK: Color = Color::Rgb(8, 12, 24);
    pub const BG_MID: Color = Color::Rgb(10, 14, 28);
    pub const BG_LIGHT: Color = Color::Rgb(18, 25, 45);
    pub const BG_POPUP: Color = Color::Rgb(10, 14, 28);

    // Borders
    pub const BORDER: Color = Color::Rgb(30, 50, 85);
    pub const BORDER_HI: Color = Color::Rgb(60, 100, 180);

    // Text
    pub const TEXT: Color = Color::Rgb(170, 185, 210);
    pub const TEXT_DIM: Color = Color::Rgb(80, 100, 135);
    pub const TEXT_LABEL: Color = Color::Rgb(90, 105, 135);
    pub const TEXT_TITLE: Color = Color::Rgb(200, 220, 255);
    pub const TEXT_HI: Color = Color::Rgb(200, 210, 230);

    // Accents
    pub const GREEN: Color = Color::Rgb(80, 200, 120);
    pub const CYAN: Color = Color::Rgb(80, 200, 255);
    pub const YELLOW: Color = Color::Rgb(255, 200, 80);
    pub const ORANGE: Color = Color::Rgb(255, 180, 80);
    pub const RED: Color = Color::Rgb(255, 80, 80);
    pub const PURPLE: Color = Color::Rgb(180, 120, 255);

    // Tab
    pub const TAB_ACTIVE_BG: Color = Color::Rgb(30, 42, 70);
    pub const TAB_ACTIVE_FG: Color = Color::Rgb(255, 220, 120);
    pub const TAB_INACTIVE: Color = Color::Rgb(80, 100, 135);
    pub const TAB_SEPARATOR: Color = Color::Rgb(40, 55, 80);

    pub fn select_bg(high_contrast: bool) -> ratatui::style::Style {
        if high_contrast {
            ratatui::style::Style::default().bg(Color::Rgb(40, 55, 80)).add_modifier(ratatui::style::Modifier::BOLD)
        } else {
            ratatui::style::Style::default().bg(BG_LIGHT)
        }
    }
}

pub mod key {
    use ratatui::style::{Color, Modifier, Style};

    pub fn render(key: &str, desc: &str) -> Vec<ratatui::text::Span<'static>> {
        vec![
            ratatui::text::Span::styled(
                format!(" {} ", key),
                Style::default()
                    .fg(Color::Rgb(255, 200, 80))
                    .add_modifier(Modifier::BOLD),
            ),
            ratatui::text::Span::styled(
                format!("{} ", desc),
                Style::default().fg(Color::Rgb(95, 108, 135)),
            ),
        ]
    }
}

// ─── Master draw ───────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, app: &mut App) {
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // Title bar
            Constraint::Length(1),  // Tab menu
            Constraint::Min(5),     // Tab content
            Constraint::Length(1),  // Status bar / key hints
        ])
        .split(f.area());

    draw_title_bar(f, main_layout[0], app);
    draw_tab_menu(f, main_layout[1], app);

    match app.current_tab {
        Tab::Dashboard => dashboard::draw_dashboard(f, main_layout[2], app),
        Tab::Minions => minions::draw_minions(f, main_layout[2], app),
        Tab::Events => events::draw_events(f, main_layout[2], app),
        Tab::Jobs => jobs::draw_jobs(f, main_layout[2], app),
        Tab::Alerts => alerts_view::draw_alerts(f, main_layout[2], app),
    }

    draw_key_hints(f, main_layout[3], app);

    // Detail popup overlay
    detail_popup::draw_detail_popup(f, app);

    // Help overlay
    if app.show_help {
        let help_area = detail_popup::centered_rect(60, 50, f.area());
        f.render_widget(Clear, help_area);
        draw_help_overlay(f, help_area);
    }
}

// ─── Title bar ─────────────────────────────────────────────────────────────

fn draw_title_bar(f: &mut Frame, area: Rect, app: &App) {
    let version = env!("CARGO_PKG_VERSION");
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let title = Line::from(vec![
        Span::styled(
            format!(" \u{25C8} salt-radar v{}", version),
            Style::default()
                .fg(color::CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  Salt Master Monitor",
            Style::default().fg(Color::Rgb(130, 150, 190)),
        ),
        Span::styled(
            " \u{2502} ",
            Style::default().fg(Color::Rgb(35, 50, 75)),
        ),
        Span::styled(
            format!("{}", app.config_snapshot.as_ref().map(|c| &c.salt_version).unwrap_or(&"?".to_string())),
            Style::default().fg(Color::Rgb(90, 150, 210)),
        ),
        Span::styled(
            " \u{2502} ",
            Style::default().fg(Color::Rgb(35, 50, 75)),
        ),
        Span::styled(
            format!("alerts: {}", app.alert_engine.active_count()),
            Style::default().fg(if app.alert_engine.active_count() > 0 {
                color::ORANGE
            } else {
                color::GREEN
            }).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {} ", now),
            Style::default().fg(Color::Rgb(85, 95, 120)),
        ),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER))
        .style(Style::default().bg(color::BG_DARK));

    f.render_widget(Paragraph::new(title).block(block), area);
}

// ─── Tab menu ──────────────────────────────────────────────────────────────

fn draw_tab_menu(f: &mut Frame, area: Rect, app: &App) {
    let tabs = [Tab::Dashboard, Tab::Minions, Tab::Events, Tab::Jobs, Tab::Alerts];
    let mut tab_spans: Vec<Span<'static>> = Vec::new();

    for (i, tab) in tabs.iter().enumerate() {
        if i > 0 {
            tab_spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::TAB_SEPARATOR)));
        }
        let label = format!(" {} {} ", i + 1, tab.label());
        if *tab == app.current_tab {
            tab_spans.push(Span::styled(
                label,
                Style::default()
                    .fg(color::TAB_ACTIVE_FG)
                    .bg(color::TAB_ACTIVE_BG)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            tab_spans.push(Span::styled(
                label,
                Style::default().fg(color::TAB_INACTIVE),
            ));
        }
    }

    let tab_line = Paragraph::new(Line::from(tab_spans))
        .style(Style::default().bg(color::BG_LIGHT));
    f.render_widget(tab_line, area);
}

// ─── Key hints ─────────────────────────────────────────────────────────────

fn draw_key_hints(f: &mut Frame, area: Rect, app: &App) {
    if app.sniffer_input_active {
        let cursor = if app.sniffer_input_text.len() % 2 == 0 { '|' } else { ' ' };
        let k1 = key::render("Esc", "Cancel");
        let k2 = key::render("Enter", "Start");
        let mut all_spans: Vec<Span<'static>> = Vec::new();
        all_spans.extend(k1);
        all_spans.extend(k2);
        all_spans.push(Span::styled(
            format!("  Minion IDs (comma-separated): {} {}", app.sniffer_input_text, cursor),
            Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
        ));
        let paragraph = Paragraph::new(Line::from(all_spans))
            .style(Style::default().bg(color::BG_LIGHT));
        f.render_widget(paragraph, area);
        return;
    }

    if app.is_filtering {
        let spans = vec![
            key::render("Esc", "Cancel"),
            key::render("Enter", "Apply"),
        ];
        let mut all_spans: Vec<Span<'static>> = Vec::new();
        for s in spans { all_spans.extend(s); }
        all_spans.push(Span::styled(
            format!("  filter: /{} {}", app.filter_text, if app.filter_text.len() % 2 == 0 { '|' } else { ' ' }),
            Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
        ));
        let paragraph = Paragraph::new(Line::from(all_spans))
            .style(Style::default().bg(color::BG_LIGHT));
        f.render_widget(paragraph, area);
        return;
    }

    if app.detail_popup.is_some() {
        if app.admin_mode_pending {
            let mut spans = key::render("Enter", "OK");
            spans.extend(key::render("Esc/q/a", "Cancel"));
            let paragraph = Paragraph::new(Line::from(spans))
                .style(Style::default().bg(color::BG_LIGHT));
            f.render_widget(paragraph, area);
            return;
        }
        if let Some(DetailPopup::EventSniffer(ref state)) = app.detail_popup {
            let mut spans = key::render("Esc/q", "Close");
            spans.extend(key::render("\u{2191}\u{2193}", "Scroll"));
            spans.extend(key::render("Enter", "Detail"));
            if state.paused {
                spans.extend(key::render("p", "Resume"));
            } else {
                spans.extend(key::render("p", "Pause"));
            }
            spans.extend(key::render("c", "Contrast"));
            spans.extend(key::render("y", "Copy"));
            let paragraph = Paragraph::new(Line::from(spans))
                .style(Style::default().bg(color::BG_LIGHT));
            f.render_widget(paragraph, area);
            return;
        }
        let is_event = matches!(app.detail_popup, Some(DetailPopup::SaltEvent(_)));
        let is_alert = matches!(app.detail_popup, Some(DetailPopup::Alert(_)));
        let mut spans = key::render("Esc", "Close");
        if is_event || is_alert {
            spans.extend(key::render("y", "Copy"));
        }
        let paragraph = Paragraph::new(Line::from(spans))
            .style(Style::default().bg(color::BG_LIGHT));
        f.render_widget(paragraph, area);
        return;
    }

    let common_keys = vec![
        key::render("q", "Quit"),
        key::render("Tab", "Switch"),
        key::render("\u{2191}\u{2193}", "Select"),
        key::render("?", "Help"),
        key::render("c", "Contrast"),
    ];

    let tab_keys = match app.current_tab {
        Tab::Dashboard => {
            let mut keys = vec![
                key::render("1-5", "Tabs"),
                key::render("+/-", "Interval"),
            ];
            keys.push(key::render("a", "Admin"));
            keys
        }
        Tab::Minions => {
            let mut keys = vec![key::render("/", "Filter"), key::render("Enter", "Detail")];
            keys.push(key::render("a", "Admin"));
            if app.fleet_data.is_some() {
                keys.push(key::render("x", "Hide unresponsive"));
            }
            if app.admin_mode {
                keys.push(key::render("s", "Scan fleet"));
            }
            keys
        }
        Tab::Events => {
            let mut keys = vec![
                key::render("/", "Filter"),
                key::render("Enter", "Detail"),
                key::render("n", "Sniffer"),
            ];
            keys.push(key::render("a", "Admin"));
            keys
        }
        _ => {
            let mut keys = vec![
                key::render("/", "Filter"),
                key::render("Enter", "Detail"),
            ];
            keys.push(key::render("a", "Admin"));
            keys
        }
    };

    let mut spans: Vec<Span<'static>> = Vec::new();
    for s in common_keys {
        spans.extend(s);
    }
    spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::TAB_SEPARATOR)));
    for s in tab_keys {
        spans.extend(s);
    }

    // Agent status on the right
    spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::TAB_SEPARATOR)));
    if app.agent.connected {
        spans.push(Span::styled(" agent: ok ", Style::default().fg(color::GREEN)));
    } else {
        spans.push(Span::styled(" agent: down ", Style::default().fg(color::RED)));
    }

    // Admin mode indicator (always visible, highlights when active)
    spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::TAB_SEPARATOR)));
    if app.admin_mode {
        let label = if app.fleet_scanning { " ADMIN MODE: ON (scanning) " } else { " ADMIN MODE: ON " };
        spans.push(Span::styled(label, Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)));
    } else {
        spans.push(Span::styled(" admin mode: off ", Style::default().fg(color::TEXT_DIM)));
    }

    // Contrast indicator
    spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::TAB_SEPARATOR)));
    if app.high_contrast {
        spans.push(Span::styled(" contrast: ON ", Style::default().fg(color::TEXT_HI).bg(Color::Rgb(80, 55, 0)).add_modifier(Modifier::BOLD)));
    } else {
        spans.push(Span::styled(" contrast: off ", Style::default().fg(color::TEXT_DIM)));
    }

    // Selection position on list tabs
    match app.current_tab {
        Tab::Dashboard => {}
        _ => {
            spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::TAB_SEPARATOR)));
            let (cur, total) = match app.current_tab {
                Tab::Minions => (app.minion_selected + 1, app.filtered_minions_len()),
                Tab::Events => (app.event_selected + 1, app.filtered_events_len()),
                Tab::Jobs => (app.job_selected + 1, app.filtered_jobs_len()),
                Tab::Alerts => (app.alert_selected + 1, app.filtered_alerts_len()),
                _ => (0, 0),
            };
            if total > 0 {
                spans.push(Span::styled(format!(" {}/{} ", cur, total), Style::default().fg(color::CYAN)));
            }
            if !app.filter_text.is_empty() {
                spans.push(Span::styled(format!(" filtered: /{}/ ", app.filter_text), Style::default().fg(color::YELLOW)));
            }
        }
    }

    let paragraph = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(color::BG_LIGHT));
    f.render_widget(paragraph, area);
}

// ─── Reusable table helper ─────────────────────────────────────────────────

pub fn render_table_header(f: &mut Frame, area: Rect, columns: &[(&str, u16)], bg: Color) {
    let mut spans = Vec::new();
    for (i, (label, width)) in columns.iter().enumerate() {
        let w = *width as usize;
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        let padded = if label.len() > w {
            format!("{:.width$}", label, width = w)
        } else {
            format!("{:width$}", label, width = w)
        };
        spans.push(Span::styled(
            padded,
            Style::default()
                .fg(Color::Rgb(100, 140, 180))
                .add_modifier(Modifier::BOLD),
        ));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(bg)),
        area,
    );
}

// ─── Help overlay ──────────────────────────────────────────────────────────

fn draw_help_overlay(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(Span::styled("  Help", Style::default().fg(color::TEXT_TITLE).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}", Style::default().fg(Color::Rgb(35, 50, 80)))),
        Line::from(""),
        Line::from(vec![
            Span::styled("  q", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Quit", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  Tab", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Switch tab", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  1-5", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Jump to tab", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  \u{2191}\u{2193}", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Scroll", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  PgUp/PgDn", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Page scroll", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  Enter", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Show detail popup", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  Esc", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Close popup / help", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  +/-", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Adjust refresh interval (Dashboard)", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  ?/h", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Toggle this help", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  a", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Toggle admin mode", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  c", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Toggle high-contrast selection", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  n", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Event sniffer (Events tab)", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  s", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Fleet scan (admin mode, Minions tab)", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  x", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Hide/show non-responsive minions (Minions tab)", Style::default().fg(color::TEXT)),
        ]),
        Line::from(""),
        Line::from(Span::styled(format!("  salt-radar v{} \u{2014} Salt Master Monitor", env!("CARGO_PKG_VERSION")), Style::default().fg(color::TEXT_DIM))),
        Line::from(Span::styled("  Read-only TUI. All data sourced from /proc, systemctl,", Style::default().fg(color::TEXT_DIM))),
        Line::from(Span::styled("  and Salt's own APIs via the bundled Python agent.", Style::default().fg(color::TEXT_DIM))),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER_HI))
        .style(Style::default().bg(color::BG_POPUP));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(color::BG_POPUP)), inner);
}

// ─── Sub-view modules ──────────────────────────────────────────────────────
