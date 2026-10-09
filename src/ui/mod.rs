mod dashboard;
mod minions;
mod events;
mod jobs;
mod alerts_view;
mod pulse;
mod minion_events;
mod api_view;
mod table;

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

// ─── Theme colors ──────────────────────────────────────────────────────────

pub mod color {
    use std::cell::Cell;

    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::Line;

    /// Defines `Theme` (one field per color role, named as in `.theme`
    /// files), the built-in `Theme::DEFAULT`, the key lookup used when
    /// parsing a theme file, and one accessor per role reading the active
    /// theme (e.g. `color::main_fg()`).
    macro_rules! theme {
        ($($key:ident: $r:literal, $g:literal, $b:literal;)*) => {
            #[derive(Clone, Copy, Debug, PartialEq)]
            pub struct Theme { $(pub $key: Color,)* }

            impl Theme {
                pub const DEFAULT: Theme = Theme { $($key: Color::Rgb($r, $g, $b),)* };
                pub const KEYS: &'static [&'static str] = &[$(stringify!($key),)*];

                pub fn slot(&mut self, key: &str) -> Option<&mut Color> {
                    match key {
                        $(stringify!($key) => Some(&mut self.$key),)*
                        _ => None,
                    }
                }
            }

            $(pub fn $key() -> Color { THEME.with(|t| t.get().$key) })*
        };
    }

    theme! {
        main_bg: 8, 12, 24;
        meter_bg: 10, 14, 28;
        bar_bg: 18, 25, 45;
        popup_bg: 10, 14, 28;
        selected_bg: 34, 54, 96;
        border: 30, 50, 85;
        border_focus: 60, 100, 180;
        div_line: 35, 50, 80;
        main_fg: 170, 185, 210;
        inactive_fg: 80, 100, 135;
        label_fg: 90, 105, 135;
        title: 200, 220, 255;
        hi_fg: 200, 210, 230;
        header_fg: 100, 140, 180;
        subtitle_fg: 130, 150, 190;
        version_fg: 90, 150, 210;
        hint_fg: 95, 108, 135;
        badge_bg: 80, 55, 0;
        green: 80, 200, 120;
        cyan: 80, 200, 255;
        yellow: 255, 200, 80;
        orange: 255, 180, 80;
        red: 255, 80, 80;
        purple: 180, 120, 255;
        gradient_start: 80, 200, 120;
        gradient_mid: 255, 200, 80;
        gradient_high: 255, 140, 60;
        gradient_end: 255, 80, 80;
        mem_gradient_start: 80, 200, 255;
        mem_gradient_mid: 100, 150, 255;
        mem_gradient_high: 255, 160, 60;
        mem_gradient_end: 255, 80, 80;
        pulse_fresh: 120, 255, 120;
        pulse_aging: 255, 165, 0;
        tab_active_bg: 30, 42, 70;
        tab_active_fg: 255, 220, 120;
        tab_inactive_fg: 80, 100, 135;
        tab_separator: 40, 55, 80;
        highlight_1: 50, 130, 255;
        highlight_2: 255, 45, 200;
        highlight_3: 0, 230, 170;
        highlight_4: 170, 80, 255;
    }

    thread_local! {
        static THEME: Cell<Theme> = const { Cell::new(Theme::DEFAULT) };
    }

    pub fn set_theme(theme: Theme) {
        THEME.with(|t| t.set(theme));
    }

    /// Color for a level `t` in 0.0..=1.0 (0 = idle, 1 = maxed out), blended
    /// smoothly across the theme's gradient_* stops — green, yellow at half,
    /// amber at three quarters, red at the top — so a value hovering near a
    /// boundary shifts shade slightly instead of flipping between colors.
    pub fn gradient(t: f64) -> Color {
        blend([gradient_start(), gradient_mid(), gradient_high(), gradient_end()], t)
    }

    /// `gradient`, but over the mem_gradient_* stops — memory keeps its own
    /// calm (blue by default) low end, as btop's memory meters do.
    pub fn mem_gradient(t: f64) -> Color {
        blend([mem_gradient_start(), mem_gradient_mid(), mem_gradient_high(), mem_gradient_end()], t)
    }

    fn blend(colors: [Color; 4], t: f64) -> Color {
        let stops = [(0.0, colors[0]), (0.5, colors[1]), (0.75, colors[2]), (1.0, colors[3])];
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        let i = stops.iter().rposition(|(at, _)| *at <= t).unwrap_or(0).min(stops.len() - 2);
        let ((a_at, a), (b_at, b)) = (stops[i], stops[i + 1]);
        let f = (t - a_at) / (b_at - a_at);
        match (a, b) {
            (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => {
                let mix = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * f).round() as u8;
                Color::Rgb(mix(ar, br), mix(ag, bg), mix(ab, bb))
            }
            _ => if f < 0.5 { a } else { b },
        }
    }

    pub fn select_bg() -> Style {
        Style::default().bg(selected_bg()).add_modifier(Modifier::BOLD)
    }

    /// Styles a list row as the cursor row. Dim/label text is lifted to
    /// normal text, since those colors sit too close to the selection
    /// background to stay readable on it.
    pub fn selected_row(mut line: Line<'_>) -> Line<'_> {
        let dim = [Some(inactive_fg()), Some(label_fg())];
        for span in line.spans.iter_mut() {
            if dim.contains(&span.style.fg) {
                span.style.fg = Some(hi_fg());
            }
        }
        line.style(select_bg())
    }

    /// Deterministic color for a filter/highlight term — same term always
    /// gets the same color (everywhere, every session), via a simple FNV-1a
    /// hash of the lowercased text rather than assignment order, so it
    /// doesn't shift depending on typing order or which terms are active.
    /// The `highlight_*` colors are kept apart from green/red/yellow/orange,
    /// which already carry pass/fail/warn meaning — a term-colored hostname
    /// shouldn't read as a status indicator.
    pub fn term_color(term: &str) -> Color {
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in term.trim().to_lowercase().bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        let palette = [highlight_1(), highlight_2(), highlight_3(), highlight_4()];
        palette[(hash as usize) % palette.len()]
    }
}

/// Colors every occurrence of any `keywords` entry within `text`, each
/// keyword getting its own stable `color::term_color`; overlapping matches
/// keep the earlier (by start position) keyword's color rather than
/// splitting further. Shared by the Event Sniffer's keyword highlighting and
/// the Events list's filter-term highlighting so both stay visually
/// consistent — the same term is always the same color in both places.
pub fn highlight_keywords(text: &str, keywords: &[String]) -> Vec<Span<'static>> {
    if keywords.is_empty() {
        return vec![Span::styled(text.to_string(), Style::default().fg(color::main_fg()))];
    }
    let lower_text = text.to_lowercase();
    let mut matches: Vec<(usize, usize, Color)> = Vec::new();
    for kw in keywords {
        if kw.is_empty() { continue; }
        let lower_kw = kw.to_lowercase();
        let kw_color = color::term_color(kw);
        let mut start = 0;
        while let Some(pos) = lower_text[start..].find(&lower_kw) {
            let abs_start = start + pos;
            let abs_end = abs_start + kw.len();
            matches.push((abs_start, abs_end, kw_color));
            start = abs_end;
        }
    }
    if matches.is_empty() {
        return vec![Span::styled(text.to_string(), Style::default().fg(color::main_fg()))];
    }
    matches.sort_by_key(|m| m.0);
    let mut merged: Vec<(usize, usize, Color)> = Vec::new();
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
    for (start, end, col) in &merged {
        if *start > pos {
            spans.push(Span::styled(text[pos..*start].to_string(), Style::default().fg(color::main_fg())));
        }
        spans.push(Span::styled(
            text[*start..*end].to_string(),
            Style::default().fg(*col).add_modifier(Modifier::BOLD),
        ));
        pos = *end;
    }
    if pos < text.len() {
        spans.push(Span::styled(text[pos..].to_string(), Style::default().fg(color::main_fg())));
    }
    spans
}

/// A `Block` with `Borders::ALL` and the title text styled in the app's
/// title color (`color::title()`) — the shared construction every panel
/// title should go through so the color stays consistent regardless of
/// which view renders it, instead of each call site handing `.title()` a
/// plain, unstyled string.
pub fn titled_block(title: impl Into<String>, border_style: Style, bg: Color) -> Block<'static> {
    let border_color = border_style.fg.unwrap_or(color::border());
    Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Line::from(bracket_title(vec![Span::styled(title.into(), Style::default().fg(color::title()))], border_color)))
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
    let scroll = selected.saturating_sub(visible / 2);
    let scroll = scroll.min(total.saturating_sub(visible));
    (scroll, scroll > 0, scroll + visible < total)
}

/// Like `titled_block`, but for scrollable list panels: appends a "more
/// above" hint to the top title and adds a "more below" hint on the
/// bottom border when the list is windowed in that direction.
pub fn list_block(title: impl Into<String>, border_style: Style, bg: Color, has_more_above: bool, has_more_below: bool) -> Block<'static> {
    let border_color = border_style.fg.unwrap_or(color::border());
    let mut top_spans = bracket_title(vec![Span::styled(title.into(), Style::default().fg(color::title()))], border_color);
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
        spans.push(Span::styled(format!(" {}", key), Style::default().fg(color::red()).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(":{} ", label), Style::default().fg(color::title())));
        spans.push(Span::styled("\u{251C}", Style::default().fg(border_color)));
    }
    Line::from(spans)
}

/// Plain `┤ text ├` border segment for read-only info (no key to
/// highlight) — same bracket/spacing style as `border_hints`, just without
/// the red key portion.
pub fn border_info(text: impl Into<String>, border_color: Color) -> Line<'static> {
    Line::from(bracket_title(vec![Span::styled(text.into(), Style::default().fg(color::title()))], border_color))
}

/// Wraps a panel's title content — its label plus any dynamic stats bundled
/// into it — in a single `┤ ... ├` bracket, space-padded on both sides. Every
/// piece of info a frame's top border carries should go through this, so it
/// reads the same as the `┤ key:hint ├` segments already used in the bottom
/// border (border_hints) instead of running as bare text against the border.
/// `border_color` should match whatever color the panel's own border is
/// currently drawn in (`color::border()` or, while focused/interactive,
/// `color::border_focus()`) so the bracket doesn't read as a dimmer color than
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
        Span::styled(format!(" {}", dec_key), Style::default().fg(color::red()).add_modifier(Modifier::BOLD)),
        Span::styled(value.into(), Style::default().fg(color::title())),
        Span::styled(inc_key.to_string(), Style::default().fg(color::red()).add_modifier(Modifier::BOLD)),
        Span::styled(" \u{251C}", Style::default().fg(border_color)),
    ]
}

/// Like `titled_block`, with btop-style hints embedded in the bottom border.
pub fn titled_block_hinted(title: impl Into<String>, border_style: Style, bg: Color, hints: &[(&str, String)]) -> Block<'static> {
    let border_color = border_style.fg.unwrap_or(color::border());
    let block = titled_block(title, border_style, bg);
    if hints.is_empty() { block } else { block.title_bottom(border_hints(hints, border_color).left_aligned()) }
}

/// Like `list_block`, with btop-style hints embedded in the bottom border
/// (left-aligned, alongside the right-aligned "more below" indicator).
pub fn list_block_hinted(title: impl Into<String>, border_style: Style, bg: Color, has_more_above: bool, has_more_below: bool, hints: &[(&str, String)]) -> Block<'static> {
    let border_color = border_style.fg.unwrap_or(color::border());
    let block = list_block(title, border_style, bg, has_more_above, has_more_below);
    if hints.is_empty() { block } else { block.title_bottom(border_hints(hints, border_color).left_aligned()) }
}

pub mod key {
    use ratatui::style::{Modifier, Style};

    pub fn render(key: &str, desc: &str) -> Vec<ratatui::text::Span<'static>> {
        vec![
            ratatui::text::Span::styled(
                format!(" {} ", key),
                Style::default()
                    .fg(super::color::yellow())
                    .add_modifier(Modifier::BOLD),
            ),
            ratatui::text::Span::styled(
                format!("{} ", desc),
                Style::default().fg(super::color::hint_fg()),
            ),
        ]
    }
}

// ─── Master draw ───────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, app: &mut App) {
    app.sync_theme();
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
    // The sniffer lives outside `detail_popup` (it keeps running when you
    // leave Events) so it isn't covered by `popup_active` above, but it
    // still overlays/steals focus from the Events tab whenever it's the
    // thing on screen there (i.e. no other Events-tab popup on top of it).
    let sniffer_active = !popup_active && app.current_tab == Tab::Events && app.sniffer.is_some();
    let tab_focused = !popup_active && !sniffer_active && !app.show_help;

    let mut hits = Vec::new();
    match app.current_tab {
        Tab::Dashboard => dashboard::draw_dashboard(f, main_layout[1], app),
        Tab::Minions => minions::draw_minions(f, main_layout[1], app, tab_focused, &mut hits),
        Tab::Events => events::draw_events(f, main_layout[1], app, tab_focused, &mut hits),
        Tab::Jobs => jobs::draw_jobs(f, main_layout[1], app, tab_focused, &mut hits),
        Tab::Alerts => alerts_view::draw_alerts(f, main_layout[1], app, tab_focused, &mut hits),
        Tab::Pulse => pulse::draw_pulse(f, main_layout[1], app, tab_focused),
        Tab::Api => api_view::draw_api(f, main_layout[1], app, tab_focused, &mut hits),
    }
    app.header_hits = hits;

    draw_key_hints(f, main_layout[2], app);

    // Detail popup overlay
    let dim = app.config.popup_dim_pct;
    if popup_active || sniffer_active { dim_background(f, dim); }
    if popup_active { detail_popup::draw_detail_popup(f, app); }
    else if sniffer_active { detail_popup::draw_sniffer_overlay(f, app); }

    // Help overlay
    if app.show_help {
        dim_background(f, dim);
        let help_area = detail_popup::centered_rect(60, 65, f.area());
        f.render_widget(Clear, help_area);
        draw_help_overlay(f, help_area);
    }

    // Save-to-file prompt: opened from inside a popup, so it's the top layer.
    if app.save_prompt.is_some() {
        dim_background(f, dim);
        detail_popup::draw_save_prompt(f, app);
    }
}

/// Fades everything already drawn toward the theme's background by
/// `dim_pct` (the `popup_dim_pct` setting) so the popup rendered next stands
/// out — darker on a dark theme, washed out on a light one. Called once per
/// overlay layer, so a nested popup fades the one beneath it as well. A
/// transparent (terminal-default) background fades toward black;
/// terminal-default and palette colors can't be blended (their real value
/// depends on the terminal's theme), so their foreground falls back to
/// `Modifier::DIM`.
pub(crate) fn dim_background(f: &mut Frame, dim_pct: u64) {
    if dim_pct == 0 { return; }
    let keep = 100 - dim_pct.min(100) as u16;
    let (tr, tg, tb) = match color::main_bg() {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (0, 0, 0),
    };
    let mix = |c: u8, t: u8| ((c as u16 * keep + t as u16 * (100 - keep)) / 100) as u8;
    let fade = |r, g, b| Color::Rgb(mix(r, tr), mix(g, tg), mix(b, tb));
    for cell in f.buffer_mut().content.iter_mut() {
        match cell.fg {
            Color::Rgb(r, g, b) => { cell.fg = fade(r, g, b); }
            _ => { cell.modifier.insert(Modifier::DIM); }
        }
        if let Color::Rgb(r, g, b) = cell.bg {
            cell.bg = fade(r, g, b);
        }
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
    let dot_color = if connected { color::green() } else { color::red() };
    let master_version = app.config_snapshot.as_ref().map(|c| c.salt_version.as_str()).unwrap_or("?");

    let left_title = Line::from(vec![
        Span::styled(
            format!(" \u{25C8} salt-radar v{} ", version),
            Style::default().fg(color::cyan()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{}", radar), Style::default().fg(color::cyan())),
        Span::styled(format!(" {} ", dot), Style::default().fg(dot_color).add_modifier(Modifier::BOLD)),
        Span::styled("Salt Master Monitor ", Style::default().fg(color::subtitle_fg())),
        Span::styled("\u{2502} ", Style::default().fg(color::div_line())),
        Span::styled(format!("Salt {} ", master_version), Style::default().fg(color::version_fg())),
    ]);
    let center_title = Line::from(Span::styled(
        format!(" {} ", now),
        Style::default().fg(color::title()),
    ));
    let dropped = app.agent_backlog_dropped.load(std::sync::atomic::Ordering::Relaxed);
    let mut right_spans = value_hint("-", format!("{}ms", app.tick_interval_ms), "+", color::border());
    if dropped > 0 {
        right_spans.push(Span::styled("\u{2524}", Style::default().fg(color::border())));
        right_spans.push(Span::styled(
            format!(" backlog dropped: {} ", dropped),
            Style::default().fg(color::red()).add_modifier(Modifier::BOLD),
        ));
        right_spans.push(Span::styled("\u{251C}", Style::default().fg(color::border())));
    }
    right_spans.push(Span::styled("\u{2524}", Style::default().fg(color::border())));
    right_spans.push(Span::styled(
        format!(" alerts: {} ", app.alert_engine.active_count()),
        Style::default().fg(if app.alert_engine.active_count() > 0 { color::orange() } else { color::green() })
            .add_modifier(Modifier::BOLD),
    ));
    right_spans.push(Span::styled("\u{251C}", Style::default().fg(color::border())));
    let right_title = Line::from(right_spans);

    let mouse_hint = if app.mouse_enabled { "mouse off".to_string() } else { "mouse on".to_string() };
    let admin_hint = if app.admin_mode { "admin off".to_string() } else { "admin on".to_string() };
    let global_hints = border_hints(&[
        ("?/F1", "help".to_string()),
        ("F2", mouse_hint),
        ("F3", "config".to_string()),
        ("a", admin_hint),
        ("q", "quit".to_string()),
    ], color::border());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::border()))
        .style(Style::default().bg(color::main_bg()))
        .title_top(left_title.left_aligned())
        .title_top(center_title.centered())
        .title_top(right_title.right_aligned())
        .title_bottom(global_hints.right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Tab menu (left) and host/uptime (right) share the one content row.
    let tabs = [Tab::Dashboard, Tab::Minions, Tab::Events, Tab::Jobs, Tab::Alerts, Tab::Api, Tab::Pulse];
    let mut tab_spans: Vec<Span<'static>> = Vec::new();
    for (i, tab) in tabs.iter().enumerate() {
        if i > 0 {
            tab_spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::tab_separator())));
        }
        let label = format!(" {} {} ", i + 1, tab.label());
        if *tab == app.current_tab {
            tab_spans.push(Span::styled(
                label,
                Style::default().fg(color::tab_active_fg()).bg(color::tab_active_bg()).add_modifier(Modifier::BOLD),
            ));
        } else {
            tab_spans.push(Span::styled(label, Style::default().fg(color::tab_inactive_fg())));
        }
    }

    let hostname = app.config_snapshot.as_ref().map(|c| c.hostname.as_str()).unwrap_or("?");
    let app_uptime = app.session_start.elapsed().as_secs();
    let host_uptime = app.system_info.uptime_secs;
    let mut info_spans = vec![
        Span::styled("host: ", Style::default().fg(color::label_fg())),
        Span::styled(format!("{} ", hostname), Style::default().fg(color::cyan()).add_modifier(Modifier::BOLD)),
        Span::styled("\u{2502} ", Style::default().fg(color::tab_separator())),
        Span::styled("radar up: ", Style::default().fg(color::label_fg())),
        Span::styled(format!("{} ", dashboard::format_duration(app_uptime)), Style::default().fg(color::cyan()).add_modifier(Modifier::BOLD)),
    ];
    if host_uptime > 0 {
        info_spans.push(Span::styled("| host up: ", Style::default().fg(color::label_fg())));
        info_spans.push(Span::styled(format!("{} ", dashboard::format_uptime(host_uptime)), Style::default().fg(color::cyan()).add_modifier(Modifier::BOLD)));
    }
    let info_line = Line::from(info_spans);
    let info_width = (info_line.width() as u16).min(inner.width);

    let content = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(info_width)])
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(tab_spans)).style(Style::default().bg(color::bar_bg())),
        content[0],
    );
    f.render_widget(
        Paragraph::new(info_line).style(Style::default().bg(color::bar_bg())),
        content[1],
    );
}

// ─── Key hints ─────────────────────────────────────────────────────────────

fn draw_key_hints(f: &mut Frame, area: Rect, app: &App) {
    if app.sniffer_input_active {
        let cursor = if app.sniffer_input_text.len().is_multiple_of(2) { '|' } else { ' ' };
        let k1 = key::render("Esc", "Cancel");
        let k2 = key::render("Enter", "Start");
        let mut all_spans: Vec<Span<'static>> = Vec::new();
        all_spans.extend(k1);
        all_spans.extend(k2);
        all_spans.push(Span::styled(
            format!("  Minion IDs (comma-separated): {} {}", app.sniffer_input_text, cursor),
            Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD),
        ));
        let paragraph = Paragraph::new(Line::from(all_spans))
            .style(Style::default().bg(color::bar_bg()));
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
            format!("  filter: /{} {}", app.filter_text, if app.filter_text.len().is_multiple_of(2) { '|' } else { ' ' }),
            Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD),
        ));
        let paragraph = Paragraph::new(Line::from(all_spans))
            .style(Style::default().bg(color::bar_bg()));
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
        spans.push(Span::styled(" agent: ok ", Style::default().fg(color::green())));
    } else {
        spans.push(Span::styled(" agent: down ", Style::default().fg(color::red())));
    }

    // Transient clipboard message (auto-clears after 8 seconds)
    if let Some((ref msg, _)) = app.clipboard_msg {
        spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::tab_separator())));
        spans.push(Span::styled(
            msg.clone(),
            Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD),
        ));
    }

    // Admin mode indicator (always visible, highlights when active)
    spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::tab_separator())));
    if app.admin_mode {
        let label = if app.fleet_scanning { " ADMIN MODE: ON (scanning) " } else { " ADMIN MODE: ON " };
        spans.push(Span::styled(label, Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)));
    } else {
        spans.push(Span::styled(" admin mode: off ", Style::default().fg(color::inactive_fg())));
    }

    // Mouse mode indicator — the border hint (`F2:mouse on`) only names
    // the key, not whether it's currently on or off; this is the actual
    // current state, same convention as admin mode.
    spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::tab_separator())));
    if app.mouse_enabled {
        spans.push(Span::styled(" mouse: ON ", Style::default().fg(color::hi_fg()).bg(color::badge_bg()).add_modifier(Modifier::BOLD)));
    } else {
        spans.push(Span::styled(" mouse: off ", Style::default().fg(color::inactive_fg())));
    }

    // Selection position on list tabs
    match app.current_tab {
        Tab::Dashboard => {}
        Tab::Jobs => {
            spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::tab_separator())));
            let focus_label = if app.cache_focus { "Cache" } else { "Live" };
            let focus_color = if app.cache_focus { color::cyan() } else { color::green() };
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
                spans.push(Span::styled(format!(" {}/{} ", cur, total), Style::default().fg(color::cyan())));
            }
            if app.cache_focus && !app.cache_filter_text.is_empty() {
                spans.push(Span::styled(format!(" filtered: /{}/ ", app.cache_filter_text), Style::default().fg(color::yellow())));
            } else if !app.cache_focus && !app.filter_text_for(Tab::Jobs).is_empty() {
                spans.push(Span::styled(format!(" filtered: /{}/ ", app.filter_text_for(Tab::Jobs)), Style::default().fg(color::yellow())));
            }
        }
        _ => {
            spans.push(Span::styled(" \u{2502} ", Style::default().fg(color::tab_separator())));
            let (cur, total) = match app.current_tab {
                Tab::Minions => (app.minion_selected + 1, app.filtered_minions_len()),
                Tab::Events => (app.event_selected + 1, app.filtered_events_len()),
                Tab::Alerts => (app.alert_selected + 1, app.filtered_alerts_len()),
                Tab::Pulse => (app.pulse_selected + 1, app.pulse_minions_count()),
                Tab::Api => (app.api_log_selected + 1, app.filtered_api_log_len()),
                _ => (0, 0),
            };
            if total > 0 {
                spans.push(Span::styled(format!(" {}/{} ", cur, total), Style::default().fg(color::cyan())));
            }
            let active_filter = match app.current_tab {
                Tab::Events => app.applied_filter.as_str(),
                Tab::Minions | Tab::Alerts | Tab::Api => app.filter_text_for(app.current_tab),
                _ => "",
            };
            if !active_filter.is_empty() {
                spans.push(Span::styled(format!(" filtered: /{}/ ", active_filter), Style::default().fg(color::yellow())));
            }
        }
    }

    let paragraph = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(color::bar_bg()));
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
                .fg(color::header_fg())
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
        Line::from(Span::styled("  Help", Style::default().fg(color::title()).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}", Style::default().fg(color::div_line()))),
        Line::from(Span::styled("  Most shortcuts are also shown live in the border of the panel they affect.", Style::default().fg(color::inactive_fg()))),
        Line::from(""),
        Line::from(vec![
            Span::styled("  q", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Quit", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  Tab", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Switch tab", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  1-7", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Jump to tab", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  \u{2191}\u{2193}", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Scroll", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  PgUp/PgDn", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Page scroll", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  Enter", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Show detail popup", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  Esc", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Close popup / help", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  +/-", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Adjust refresh interval", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  m", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Toggle sparkline style (Bars/Dots, Dashboard)", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  F4-F8", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Toggle CPU/Mem/ZMQ/Events/Minions graphs (Dashboard, saved on quit)", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ?/F1", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Toggle this help", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  a", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Toggle admin mode", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  n", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Event sniffer (Events tab)", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  r", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Minion event history (Minions tab)", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  s", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Fleet scan (admin mode, Minions tab)", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  x", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Hide/show non-responsive minions (Minions tab)", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  o/O", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Sort by next column / reverse sort (list tables; or click a header)", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  v", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Choose and reorder the focused table's columns", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  \u{2191}\u{2193}\u{2190}\u{2192}", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Navigate grid (Pulse tab)", Style::default().fg(color::main_fg())),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Dashboard KPIs", Style::default().fg(color::title()).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  new pending", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Minion Key Status: pending keys registered in the last 60s", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ZMQ latency", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  source is configurable: zmq_latency_source in [dashboard]", Style::default().fg(color::main_fg())),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Pulse Status (Details panel)", Style::default().fg(color::title()).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  Connected", Style::default().fg(color::green()).add_modifier(Modifier::BOLD)),
            Span::styled("  most recent liveness probe (~every 5 ticks) found this minion has a cache entry", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  Unknown", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  no successful probe result yet (e.g. just discovered) \u{2014} not an error", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  Timed out", Style::default().fg(color::red()).add_modifier(Modifier::BOLD)),
            Span::styled("  flagged unresponsive by fleet-scan/staleness tracking", Style::default().fg(color::main_fg())),
        ]),
        Line::from(""),
        Line::from(Span::styled("  API tab", Style::default().fg(color::title()).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  requires", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  rest_cherrypy: {collect_stats: True} on the master, plus stats_disable_auth: True", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  (unauthenticated /stats) or stats_auth_mode = plaintext/pillar (Config Editor)", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  request log", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  best-effort: only sees each worker thread's latest request per poll", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  x", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  hide/show salt-radar's own GET /stats poll in the request log (on by default)", Style::default().fg(color::main_fg())),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Job Detail", Style::default().fg(color::title()).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  state detail", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  always full/verbose, overriding the master's own state_output/state_verbose", Style::default().fg(color::main_fg())),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Minion Detail", Style::default().fg(color::title()).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  Last Seen", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  newer of: Salt's own minion-cache mtime, or the last /ret/ event seen live", Style::default().fg(color::main_fg())),
        ]),
        Line::from(""),
        Line::from(Span::styled(format!("  salt-radar v{} \u{2014} Salt Master Monitor", env!("CARGO_PKG_VERSION")), Style::default().fg(color::inactive_fg()))),
        Line::from(Span::styled("  Read-only TUI. All data sourced from /proc, systemctl,", Style::default().fg(color::inactive_fg()))),
        Line::from(Span::styled("  and Salt's own APIs via the bundled Python agent.", Style::default().fg(color::inactive_fg()))),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::border_focus()))
        .style(Style::default().bg(color::popup_bg()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(color::popup_bg())), inner);
}

// ─── Sub-view modules ──────────────────────────────────────────────────────
