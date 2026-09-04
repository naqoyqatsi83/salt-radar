mod dashboard;
mod minions;
mod events;
mod jobs;
mod alerts_view;
mod pulse;
mod minion_events;
mod api_view;

pub mod config_editor;
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

/// A `Block` with `Borders::ALL` and the title text styled in the app's
/// title color (`color::TEXT_TITLE`) — the shared construction every panel
/// title should go through so the color stays consistent regardless of
/// which view renders it, instead of each call site handing `.title()` a
/// plain, unstyled string.
pub fn titled_block(title: impl Into<String>, border_style: Style, bg: Color) -> Block<'static> {
    let border_color = border_style.fg.unwrap_or(color::BORDER);
    Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Line::from(bracket_title(vec![Span::styled(title.into(), Style::default().fg(color::TEXT_TITLE))], border_color)))
        .style(Style::default().bg(bg))
}

/// Windowed-list scroll math shared by every scrollable list panel: given
/// the selected index, the number of visible rows, and the total row
/// count, returns `(scroll_offset, has_more_above, has_more_below)` so
/// every view can show a consistent "more above/below" indicator instead
/// of silently clipping with no way to tell there's more to scroll to.
pub fn scroll_window(selected: usize, visible: usize, total: usize) -> (usize, bool, bool) {
    if visible == 0 || total <= visible {
        return (0, false, false);
    }
    let scroll = if selected > visible / 2 { selected - visible / 2 } else { 0 };
    let scroll = scroll.min(total.saturating_sub(visible));
    (scroll, scroll > 0, scroll + visible < total)
}

/// Like `titled_block`, but for scrollable list panels: appends a "more
/// above" hint to the top title and adds a "more below" hint on the
/// bottom border when the list is windowed in that direction.
pub fn list_block(title: impl Into<String>, border_style: Style, bg: Color, has_more_above: bool, has_more_below: bool) -> Block<'static> {
    let border_color = border_style.fg.unwrap_or(color::BORDER);
    let mut top_spans = bracket_title(vec![Span::styled(title.into(), Style::default().fg(color::TEXT_TITLE))], border_color);
    if has_more_above {
        top_spans.push(Span::styled(" \u{25B2} more above ", Style::default().fg(border_color)));
    }
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(Style::default().bg(bg))
        .title_top(Line::from(top_spans));
    if has_more_below {
        block = block.title_bottom(
            Line::from(Span::styled(" \u{25BC} more below ", Style::default().fg(border_color))).right_aligned(),
        );
    }
    block
}

/// btop-style border hint: `┤ KEY:label ├` with the key that switches the
/// behavior picked out in red, built directly into the border it affects
/// instead of listed in a separate legend. One space pads both sides of
/// every bracketed segment in this border-decoration family — border_hints,
/// border_info, value_hint — consistently; none of them run text up against
/// the ┤/├ glyphs.
pub fn border_hints(hints: &[(&str, String)], border_color: Color) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (key, label) in hints {
        spans.push(Span::styled("\u{2524}", Style::default().fg(border_color)));
        spans.push(Span::styled(format!(" {}", key), Style::default().fg(color::RED).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(":{} ", label), Style::default().fg(color::TEXT_TITLE)));
        spans.push(Span::styled("\u{251C}", Style::default().fg(border_color)));
    }
    Line::from(spans)
}

/// Plain `┤ text ├` border segment for read-only info (no key to
/// highlight) — same bracket/spacing style as `border_hints`, just without
/// the red key portion.
pub fn border_info(text: impl Into<String>, border_color: Color) -> Line<'static> {
    Line::from(bracket_title(vec![Span::styled(text.into(), Style::default().fg(color::TEXT_TITLE))], border_color))
}

/// Wraps a panel's title content — its label plus any dynamic stats bundled
/// into it — in a single `┤ ... ├` bracket, space-padded on both sides. Every
/// piece of info a frame's top border carries should go through this, so it
/// reads the same as the `┤ key:hint ├` segments already used in the bottom
/// border (border_hints) instead of running as bare text against the border.
/// `border_color` should match whatever color the panel's own border is
/// currently drawn in (`color::BORDER` or, while focused/interactive,
/// `color::BORDER_HI`) so the bracket doesn't read as a dimmer color than
/// the border it's embedded in.
pub fn bracket_title(mut spans: Vec<Span<'static>>, border_color: Color) -> Vec<Span<'static>> {
    // Callers built their content for the old bare-text title (with its own
    // leading/trailing padding); strip that so the bracket's own one-space
    // padding doesn't end up doubled.
    if let Some(first) = spans.first_mut() {
        first.content = std::borrow::Cow::Owned(first.content.trim_start().to_string());
    }
    if let Some(last) = spans.last_mut() {
        last.content = std::borrow::Cow::Owned(last.content.trim_end().to_string());
    }
    let mut wrapped = Vec::with_capacity(spans.len() + 2);
    wrapped.push(Span::styled("\u{2524} ", Style::default().fg(border_color)));
    wrapped.append(&mut spans);
    wrapped.push(Span::styled(" \u{251C}", Style::default().fg(border_color)));
    wrapped
}

/// btop-style adjustable-value hint: `┤ -VALUE+ ├` with the `-`/`+` keys
/// that step the value picked out in red, the live value shown plain
/// between them (e.g. how btop shows its own update-interval in the box
/// border).
pub fn value_hint(dec_key: &str, value: impl Into<String>, inc_key: &str, border_color: Color) -> Vec<Span<'static>> {
    vec![
        Span::styled("\u{2524}", Style::default().fg(border_color)),
        Span::styled(format!(" {}", dec_key), Style::default().fg(color::RED).add_modifier(Modifier::BOLD)),
        Span::styled(value.into(), Style::default().fg(color::TEXT_TITLE)),
        Span::styled(inc_key.to_string(), Style::default().fg(color::RED).add_modifier(Modifier::BOLD)),
        Span::styled(" \u{251C}", Style::default().fg(border_color)),
    ]
}

/// Like `titled_block`, with btop-style hints embedded in the bottom border.
pub fn titled_block_hinted(title: impl Into<String>, border_style: Style, bg: Color, hints: &[(&str, String)]) -> Block<'static> {
    let border_color = border_style.fg.unwrap_or(color::BORDER);
    let block = titled_block(title, border_style, bg);
    if hints.is_empty() { block } else { block.title_bottom(border_hints(hints, border_color).left_aligned()) }
}

/// Like `list_block`, with btop-style hints embedded in the bottom border
/// (left-aligned, alongside the right-aligned "more below" indicator).
pub fn list_block_hinted(title: impl Into<String>, border_style: Style, bg: Color, has_more_above: bool, has_more_below: bool, hints: &[(&str, String)]) -> Block<'static> {
    let border_color = border_style.fg.unwrap_or(color::BORDER);
    let block = list_block(title, border_style, bg, has_more_above, has_more_below);
    if hints.is_empty() { block } else { block.title_bottom(border_hints(hints, border_color).left_aligned()) }
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
            Constraint::Length(3),  // Title/tab frame (bordered)
            Constraint::Min(5),     // Tab content
            Constraint::Length(1),  // Status bar / key hints
        ])
        .split(f.area());

    draw_title_frame(f, main_layout[0], app);

    // A popup (including the globally-triggered Config Editor) steals focus
    // from whatever tab is behind it, so that tab's normally-always-bright
    // navigable panel should dim back to the same color as its non-navigable
    // neighbors instead of still reading as the focused element. `popup_tab`
    // is only ever set when a popup opens and is never cleared when one
    // closes (closing just sets `detail_popup` back to `None`), so it must
    // be gated on `detail_popup.is_some()` too — otherwise a tab that ever
    // had a popup opened on it reads as permanently unfocused afterward.
    let popup_active = app.detail_popup.is_some()
        && (app.popup_tab == Some(app.current_tab) || matches!(app.detail_popup, Some(DetailPopup::ConfigEditor(_))));
    let tab_focused = !popup_active && !app.show_help;

    match app.current_tab {
        Tab::Dashboard => dashboard::draw_dashboard(f, main_layout[1], app),
        Tab::Minions => minions::draw_minions(f, main_layout[1], app, tab_focused),
        Tab::Events => events::draw_events(f, main_layout[1], app, tab_focused),
        Tab::Jobs => jobs::draw_jobs(f, main_layout[1], app, tab_focused),
        Tab::Alerts => alerts_view::draw_alerts(f, main_layout[1], app, tab_focused),
        Tab::Pulse => pulse::draw_pulse(f, main_layout[1], app, tab_focused),
        Tab::Api => api_view::draw_api(f, main_layout[1], app, tab_focused),
    }

    draw_key_hints(f, main_layout[2], app);

    // Detail popup overlay
    if popup_active { detail_popup::draw_detail_popup(f, app); }

    // Help overlay
    if app.show_help {
        let help_area = detail_popup::centered_rect(60, 65, f.area());
        f.render_widget(Clear, help_area);
        draw_help_overlay(f, help_area);
    }
}

// ─── Title/tab frame ───────────────────────────────────────────────────────

/// App identity, master info, clock, and alert count live in the bordered
/// frame's title (left/center/right), so they're visible on every tab
/// without duplicating them again in a per-tab header. The tab menu and
/// host/uptime info share the one content row inside the frame.
fn draw_title_frame(f: &mut Frame, area: Rect, app: &App) {
    let version = env!("CARGO_PKG_VERSION");
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let radar = dashboard::RADAR_FRAMES[app.header_radar_frame as usize];
    let connected = app.agent.connected;
    let dot = if connected { "\u{25cf}" } else { "\u{25cb}" };
    let dot_color = if connected { color::GREEN } else { color::RED };
    let master_version = app.config_snapshot.as_ref().map(|c| c.salt_version.as_str()).unwrap_or("?");

    let left_title = Line::from(vec![
        Span::styled(
            format!(" \u{25C8} salt-radar v{} ", version),
            Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{}", radar), Style::default().fg(color::CYAN)),
        Span::styled(format!(" {} ", dot), Style::default().fg(dot_color).add_modifier(Modifier::BOLD)),
        Span::styled("Salt Master Monitor ", Style::default().fg(Color::Rgb(130, 150, 190))),
        Span::styled("\u{2502} ", Style::default().fg(Color::Rgb(35, 50, 75))),
        Span::styled(format!("Salt {} ", master_version), Style::default().fg(Color::Rgb(90, 150, 210))),
    ]);
    let center_title = Line::from(Span::styled(
        format!(" {} ", now),
        Style::default().fg(color::TEXT_TITLE),
    ));
    let dropped = app.agent_backlog_dropped.load(std::sync::atomic::Ordering::Relaxed);
    let mut right_spans = value_hint("-", format!("{}ms", app.tick_interval_ms), "+", color::BORDER);
    if dropped > 0 {
        right_spans.push(Span::styled("\u{2524}", Style::default().fg(color::BORDER)));
        right_spans.push(Span::styled(
            format!(" backlog dropped: {} ", dropped),
            Style::default().fg(color::RED).add_modifier(Modifier::BOLD),
        ));
        right_spans.push(Span::styled("\u{251C}", Style::default().fg(color::BORDER)));
    }
    right_spans.push(Span::styled("\u{2524}", Style::default().fg(color::BORDER)));
    right_spans.push(Span::styled(
        format!(" alerts: {} ", app.alert_engine.active_count()),
        Style::default().fg(if app.alert_engine.active_count() > 0 { color::ORANGE } else { color::GREEN })
            .add_modifier(Modifier::BOLD),
    ));
    right_spans.push(Span::styled("\u{251C}", Style::default().fg(color::BORDER)));
    let right_title = Line::from(right_spans);

    let mouse_hint = if app.mouse_enabled { "mouse off".to_string() } else { "mouse on".to_string() };
    let admin_hint = if app.admin_mode { "admin off".to_string() } else { "admin on".to_string() };
    let contrast_hint = if app.high_contrast { "contrast off".to_string() } else { "contrast on".to_string() };
    let global_hints = border_hints(&[
        ("?/F1", "help".to_string()),
        ("F2", mouse_hint),
        ("F3", "config".to_string()),
        ("c", contrast_hint),
        ("a", admin_hint),
        ("q", "quit".to_string()),
    ], color::BORDER);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER))
        .style(Style::default().bg(color::BG_DARK))
        .title_top(left_title.left_aligned())
        .title_top(center_title.centered())
        .title_top(right_title.right_aligned())
        .title_bottom(global_hints.right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Tab menu (left) and host/uptime (right) share the one content row.
    let tabs = [Tab::Dashboard, Tab::Minions, Tab::Events, Tab::Jobs, Tab::Alerts, Tab::Pulse, Tab::Api];
    let mut tab_spans: Vec<Span<'static>> = Vec::new();
    for (i, tab) in tabs.iter().enumerate() {
        if i > 0 {
            tab_spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::TAB_SEPARATOR)));
        }
        let label = format!(" {} {} ", i + 1, tab.label());
        if *tab == app.current_tab {
            tab_spans.push(Span::styled(
                label,
                Style::default().fg(color::TAB_ACTIVE_FG).bg(color::TAB_ACTIVE_BG).add_modifier(Modifier::BOLD),
            ));
        } else {
            tab_spans.push(Span::styled(label, Style::default().fg(color::TAB_INACTIVE)));
        }
    }

    let hostname = app.config_snapshot.as_ref().map(|c| c.hostname.as_str()).unwrap_or("?");
    let app_uptime = app.session_start.elapsed().as_secs();
    let host_uptime = app.system_info.uptime_secs;
    let mut info_spans = vec![
        Span::styled("host: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{} ", hostname), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)),
        Span::styled("\u{2502} ", Style::default().fg(color::TAB_SEPARATOR)),
        Span::styled("radar up: ", Style::default().fg(color::TEXT_LABEL)),
        Span::styled(format!("{} ", dashboard::format_duration(app_uptime)), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)),
    ];
    if host_uptime > 0 {
        info_spans.push(Span::styled("| host up: ", Style::default().fg(color::TEXT_LABEL)));
        info_spans.push(Span::styled(format!("{} ", dashboard::format_uptime(host_uptime)), Style::default().fg(color::CYAN).add_modifier(Modifier::BOLD)));
    }
    let info_line = Line::from(info_spans);
    let info_width = (info_line.width() as u16).min(inner.width);

    let content = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(info_width)])
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(tab_spans)).style(Style::default().bg(color::BG_LIGHT)),
        content[0],
    );
    f.render_widget(
        Paragraph::new(info_line).style(Style::default().bg(color::BG_LIGHT)),
        content[1],
    );
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

    // Popup shortcuts now live embedded in the popup's own border (same
    // btop style as every panel) instead of duplicated here, so an open
    // popup no longer blocks this bar — the persistent status stays visible
    // underneath it, consistent with every other view.

    // Shortcut hints now live embedded in each panel's own border (btop
    // style) and the title frame's global hint row — this bar is left for
    // persistent status indicators only.
    let mut spans: Vec<Span<'static>> = Vec::new();

    // Agent status
    if app.agent.connected {
        spans.push(Span::styled(" agent: ok ", Style::default().fg(color::GREEN)));
    } else {
        spans.push(Span::styled(" agent: down ", Style::default().fg(color::RED)));
    }

    // Transient clipboard message (auto-clears after 8 seconds)
    if let Some((ref msg, _)) = app.clipboard_msg {
        spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::TAB_SEPARATOR)));
        spans.push(Span::styled(
            msg.clone(),
            Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
        ));
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

    // Mouse mode indicator — the border hint (`F2:mouse on`) only names
    // the key, not whether it's currently on or off; this is the actual
    // current state, same convention as contrast/admin mode.
    spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::TAB_SEPARATOR)));
    if app.mouse_enabled {
        spans.push(Span::styled(" mouse: ON ", Style::default().fg(color::TEXT_HI).bg(Color::Rgb(80, 55, 0)).add_modifier(Modifier::BOLD)));
    } else {
        spans.push(Span::styled(" mouse: off ", Style::default().fg(color::TEXT_DIM)));
    }

    // Selection position on list tabs
    match app.current_tab {
        Tab::Dashboard => {}
        Tab::Jobs => {
            spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::TAB_SEPARATOR)));
            let focus_label = if app.cache_focus { "Cache" } else { "Live" };
            let focus_color = if app.cache_focus { color::CYAN } else { color::GREEN };
            spans.push(Span::styled(
                format!(" {} ", focus_label),
                Style::default().fg(focus_color).add_modifier(Modifier::BOLD),
            ));
            let (cur, total) = if app.cache_focus {
                (app.cache_selected + 1, app.filtered_cache_len())
            } else {
                (app.job_selected + 1, app.filtered_jobs_len())
            };
            if total > 0 {
                spans.push(Span::styled(format!(" {}/{} ", cur, total), Style::default().fg(color::CYAN)));
            }
            if app.cache_focus && !app.cache_filter_text.is_empty() {
                spans.push(Span::styled(format!(" filtered: /{}/ ", app.cache_filter_text), Style::default().fg(color::YELLOW)));
            } else if !app.cache_focus && !app.filter_text.is_empty() {
                spans.push(Span::styled(format!(" filtered: /{}/ ", app.filter_text), Style::default().fg(color::YELLOW)));
            }
        }
        _ => {
            spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::TAB_SEPARATOR)));
            let (cur, total) = match app.current_tab {
                Tab::Minions => (app.minion_selected + 1, app.filtered_minions_len()),
                Tab::Events => (app.event_selected + 1, app.filtered_events_len()),
                Tab::Alerts => (app.alert_selected + 1, app.filtered_alerts_len()),
                Tab::Pulse => (app.pulse_selected + 1, app.pulse_minions_count()),
                Tab::Api => (app.api_log_selected + 1, app.filtered_api_log_len()),
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
        Line::from(Span::styled("  Most shortcuts are also shown live in the border of the panel they affect.", Style::default().fg(color::TEXT_DIM))),
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
            Span::styled("  1-7", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
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
            Span::styled("  Adjust refresh interval", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  m", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Toggle sparkline style (Bars/Dots, Dashboard)", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  F4-F8", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Toggle CPU/Mem/ZMQ/Events/Minions graphs (Dashboard, saved on quit)", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  ?/F1", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
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
            Span::styled("  r", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Minion event history (Minions tab)", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  s", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Fleet scan (admin mode, Minions tab)", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  x", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Hide/show non-responsive minions (Minions tab)", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  \u{2191}\u{2193}\u{2190}\u{2192}", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Navigate grid (Pulse tab)", Style::default().fg(color::TEXT)),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Dashboard KPIs", Style::default().fg(color::TEXT_TITLE).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  new pending", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  Minion Key Status: pending keys registered in the last 60s", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  ZMQ latency", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  source is configurable: zmq_latency_source in [dashboard]", Style::default().fg(color::TEXT)),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Pulse Status (Details panel)", Style::default().fg(color::TEXT_TITLE).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  Connected", Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD)),
            Span::styled("  most recent liveness probe (~every 5 ticks) found this minion has a cache entry", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  Unknown", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  no successful probe result yet (e.g. just discovered) \u{2014} not an error", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  Timed out", Style::default().fg(color::RED).add_modifier(Modifier::BOLD)),
            Span::styled("  flagged unresponsive by fleet-scan/staleness tracking", Style::default().fg(color::TEXT)),
        ]),
        Line::from(""),
        Line::from(Span::styled("  API tab", Style::default().fg(color::TEXT_TITLE).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  requires", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  rest_cherrypy: {collect_stats: True, stats_disable_auth: True} on the master", Style::default().fg(color::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("  request log", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  best-effort: only sees each worker thread's latest request per poll", Style::default().fg(color::TEXT)),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Minion Detail", Style::default().fg(color::TEXT_TITLE).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  Last Seen", Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled("  newer of: Salt's own minion-cache mtime, or the last /ret/ event seen live", Style::default().fg(color::TEXT)),
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
