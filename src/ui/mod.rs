mod dashboard;
mod minions;
mod events;
mod jobs;
mod alerts_view;
mod pulse;
mod master_config;
mod minion_events;
mod api_view;
mod table;
mod whats_new;

pub mod config_editor;
pub mod detail_popup;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
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

    // Search matches: every match softly tinted, the one `n`/`N` is on as a
    // solid block. Built from theme roles that hold in dark and light themes
    // alike: badge_bg is a muted amber meant for hi_fg text on top, and
    // popup_bg text on yellow is the strongest contrast a theme defines.
    pub fn search_match() -> Style {
        Style::default().fg(hi_fg()).bg(badge_bg())
    }

    pub fn search_current() -> Style {
        // A theme with a transparent background has no popup_bg color, which
        // would leave the terminal's own (usually light) text on yellow.
        let fg = match popup_bg() {
            Color::Reset => Color::Black,
            c => c,
        };
        Style::default().fg(fg).bg(yellow()).add_modifier(Modifier::BOLD)
    }

    // Job outcome colors, the same wherever a result is shown (Live Jobs,
    // Job Detail, the minion activity popup, Fail%), following Salt's own
    // highstate output: a zero or a value that doesn't apply is dimmed.
    pub fn outcome_ok() -> Color { green() }
    pub fn outcome_changed() -> Color { cyan() }
    /// A `test=True` run's "would change" (`result: None`).
    pub fn outcome_pending() -> Color { yellow() }
    pub fn outcome_failed() -> Color { red() }
    /// Targeted but no return (yet).
    pub fn outcome_missing() -> Color { orange() }

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

    /// Highlight colors for the terms of one search: each distinct term
    /// (ignoring case) takes the next color in the order the terms were
    /// typed, so terms of one search only share a color past the palette's
    /// six. The `highlight_*` colors come first; green/red/yellow/orange are
    /// left out, as they already carry pass/fail/warn meaning — a
    /// term-colored hostname shouldn't read as a status indicator.
    pub fn term_colors<'a>(terms: impl IntoIterator<Item = &'a str>) -> std::collections::HashMap<String, Color> {
        let palette = [highlight_1(), highlight_2(), highlight_3(), highlight_4(), cyan(), purple()];
        let mut colors = std::collections::HashMap::new();
        for term in terms {
            let key = term.trim().to_lowercase();
            if !key.is_empty() && !colors.contains_key(&key) {
                let color = palette[colors.len() % palette.len()];
                colors.insert(key, color);
            }
        }
        colors
    }

    /// A term's color from `term_colors`.
    pub fn term_color_in(colors: &std::collections::HashMap<String, Color>, term: &str) -> Color {
        colors.get(&term.trim().to_lowercase()).copied().unwrap_or_else(highlight_1)
    }
}

/// Colors every occurrence of any `keywords` entry within `text`, each
/// keyword in its own color (`color::term_colors`); overlapping matches
/// keep the earlier (by start position) keyword's color rather than
/// splitting further. Used for the Events list's filter terms.
pub fn highlight_keywords(text: &str, terms: &[crate::app::HighlightTerm]) -> Vec<Span<'static>> {
    let colors = color::term_colors(terms.iter().map(|t| t.term.as_str()));
    let matches = terms.iter()
        .flat_map(|t| {
            let c = color::term_color_in(&colors, &t.term);
            t.ranges(text).into_iter().map(move |(s, e)| (s, e, c))
        })
        .collect();
    colored_ranges(text, matches)
}

/// Hard-wraps a styled line at `width` columns, continuation rows hanging
/// two columns in from the line's own indent so a wrapped JSON value stays
/// under its key. Unlike `Paragraph`'s word wrap the row count is known up
/// front (`wrapped_rows`), which a panel budgeting its height needs.
pub fn wrap_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![Line::from(spans)];
    }
    let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    let indent = hang_indent(&text, width);
    let mut rows = Vec::new();
    let mut row: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for span in spans {
        let mut chunk = String::new();
        for c in span.content.chars() {
            if used == width {
                if !chunk.is_empty() {
                    row.push(Span::styled(std::mem::take(&mut chunk), span.style));
                }
                rows.push(Line::from(std::mem::replace(&mut row, vec![Span::raw(" ".repeat(indent))])));
                used = indent;
            }
            chunk.push(c);
            used += 1;
        }
        if !chunk.is_empty() {
            row.push(Span::styled(chunk, span.style));
        }
    }
    rows.push(Line::from(row));
    rows
}

/// How many rows `wrap_spans` makes of `text` at `width`.
pub fn wrapped_rows(text: &str, width: usize) -> usize {
    let n = text.chars().count();
    if width == 0 || n <= width {
        return 1;
    }
    1 + (n - width).div_ceil(width - hang_indent(text, width))
}

fn hang_indent(text: &str, width: usize) -> usize {
    (text.len() - text.trim_start_matches(' ').len() + 2).min(width / 2)
}

/// A text input's `prefix` and content with its cursor: the character under
/// it, or a space past the end, in reverse video (readable in any theme).
/// A list's filter bar: the filter being typed (with its cursor) or the one
/// applied, the item count, and a warning when a `/…/` term is matched as
/// plain text instead of a regex. `check_syntax` is false where the agent
/// applies the filter, as Python's regex syntax differs from this one.
pub fn filter_bar(text: &str, typing: bool, cursor: Option<usize>, count: String, check_syntax: bool) -> Option<Line<'static>> {
    if !typing && text.is_empty() {
        return None;
    }
    let style = Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD);
    let mut spans = if typing {
        input_spans(" filter: ", text, cursor, style)
    } else {
        vec![Span::styled(format!(" filter: {} ", text), style)]
    };
    spans.push(Span::styled(if typing { format!("  {}", count) } else { count }, Style::default().fg(color::inactive_fg())));
    let problem = crate::sniff::filter_terms(text).into_iter()
        .find_map(|t| crate::sniff::regex_problem(t, check_syntax).map(|why| (t, why)));
    if let Some((term, why)) = problem {
        spans.push(Span::styled(
            format!("  \u{26a0} {}: {}, matched as text", term, why),
            Style::default().fg(color::red()).add_modifier(Modifier::BOLD),
        ));
    }
    Some(Line::from(spans))
}

pub fn input_spans(prefix: &str, text: &str, cursor: Option<usize>, style: Style) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let at = cursor.unwrap_or(chars.len()).min(chars.len());
    let under = chars.get(at).map_or_else(|| " ".to_string(), |c| c.to_string());
    vec![
        Span::styled(format!("{prefix}{}", chars[..at].iter().collect::<String>()), style),
        Span::styled(under, style.add_modifier(Modifier::REVERSED)),
        Span::styled(chars.get(at + 1..).unwrap_or_default().iter().collect::<String>(), style),
    ]
}

/// `highlight_keywords` for the Event Sniffer's terms: each minion, keyword
/// or regex match in its term's color (`color::term_colors`, in the order of
/// the terms).
pub fn highlight_terms(text: &str, terms: &[crate::sniff::SniffTerm]) -> Vec<Span<'static>> {
    let colors = color::term_colors(terms.iter().map(|t| t.color_key()));
    let matches = terms.iter()
        .flat_map(|t| {
            let c = color::term_color_in(&colors, t.color_key());
            t.find_in(text).into_iter().map(move |(s, e)| (s, e, c))
        })
        .collect();
    colored_ranges(text, matches)
}

fn colored_ranges(text: &str, mut matches: Vec<(usize, usize, Color)>) -> Vec<Span<'static>> {
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

/// Whether a right-aligned border title fits beside the left-aligned ones on
/// a border `area_width` wide. ratatui draws right-aligned titles last, so on
/// a border too narrow for both the right one would cover the key hints on
/// the left; callers leave it out instead.
pub fn fits_beside(left_width: usize, right: &Line, area_width: u16) -> bool {
    left_width + right.width() <= usize::from(area_width).saturating_sub(2)
}

pub fn more_below(border_color: Color) -> Line<'static> {
    Line::from(Span::styled(" \u{25BC} more below ", Style::default().fg(border_color))).right_aligned()
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
        block = block.title_bottom(more_below(border_color));
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

/// A tab's left and right panels, split where `Ctrl+←`/`→` left it. Mouse
/// hit-testing goes through here too, so clicks follow the divider.
pub fn split_panes(area: Rect, app: &App) -> [Rect; 2] {
    let pct = app.config.split_pct(app.current_tab);
    Layout::horizontal([Constraint::Percentage(pct), Constraint::Percentage(100 - pct)]).areas(area)
}

/// A minion's lag ratio, like `1.0x` or `12x`.
pub fn format_lag(ratio: f64) -> String {
    if ratio < 10.0 { format!("{:.1}x", ratio) } else { format!("{:.0}x", ratio) }
}

/// Lag ratios up to 1.5x are normal spread between minions; beyond that a
/// minion is noticeably slower than its peers, and from 3x, much slower.
pub fn lag_color(ratio: f64) -> Color {
    if ratio < 1.5 { color::inactive_fg() } else if ratio < 3.0 { color::yellow() } else { color::red() }
}

/// A duration in ms under 10 s (`85ms`, `2340ms`), whole seconds above.
pub fn format_secs_short(secs: f64) -> String {
    if secs < 10.0 { format!("{:.0}ms", secs * 1000.0) } else { format!("{:.0}s", secs) }
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
/// (left-aligned, alongside the right-aligned "more below" indicator, which
/// gives way when the panel is `width` columns too narrow for both).
pub fn list_block_hinted(title: impl Into<String>, border_style: Style, bg: Color, has_more_above: bool, has_more_below: bool, hints: &[(&str, String)], width: u16) -> Block<'static> {
    let border_color = border_style.fg.unwrap_or(color::border());
    let hint_line = border_hints(hints, border_color);
    let has_more_below = has_more_below && fits_beside(hint_line.width(), &more_below(border_color), width);
    let block = list_block(title, border_style, bg, has_more_above, has_more_below);
    if hints.is_empty() { block } else { block.title_bottom(hint_line.left_aligned()) }
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

pub const TAB_SEPARATOR: &str = " \u{2502} ";

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

    // A popup (or the globally-triggered Config Editor) steals focus
    // from whatever tab is behind it, so that tab's normally-always-bright
    // navigable panel should dim back to the same color as its non-navigable
    // neighbors instead of still reading as the focused element. `popup_tab`
    // is only ever set when a popup opens and is never cleared when one
    // closes (closing just sets `detail_popup` back to `None`), so it must
    // be gated on `detail_popup.is_some()` too — otherwise a tab that ever
    // had a popup opened on it reads as permanently unfocused afterward.
    let popup_active = app.detail_popup.is_some() && app.popup_tab == Some(app.current_tab);
    // The sniffer lives outside `detail_popup` (it keeps running when you
    // leave Events) so it isn't covered by `popup_active` above, but it
    // still overlays/steals focus from the Events tab whenever it's the
    // thing on screen there (i.e. no other Events-tab popup on top of it).
    let sniffer_active = !popup_active && app.current_tab == Tab::Events && app.sniffer.is_some();
    let tab_focused = !popup_active && !sniffer_active && !app.show_help && app.config_editor.is_none();

    let mut hits = Vec::new();
    match app.current_tab {
        Tab::Dashboard => dashboard::draw_dashboard(f, main_layout[1], app),
        Tab::Minions => minions::draw_minions(f, main_layout[1], app, tab_focused, &mut hits),
        Tab::Events => events::draw_events(f, main_layout[1], app, tab_focused, &mut hits),
        Tab::Jobs => jobs::draw_jobs(f, main_layout[1], app, tab_focused, &mut hits),
        Tab::Alerts => alerts_view::draw_alerts(f, main_layout[1], app, tab_focused, &mut hits),
        Tab::Pulse => pulse::draw_pulse(f, main_layout[1], app, tab_focused),
        Tab::Api => api_view::draw_api(f, main_layout[1], app, tab_focused, &mut hits),
        Tab::Master => master_config::draw_master(f, main_layout[1], app, tab_focused),
    }
    app.header_hits = hits;

    draw_key_hints(f, main_layout[2], app);

    // Detail popup overlay
    let dim = app.popup_dim_pct();
    if popup_active || sniffer_active { dim_background(f, dim); }
    if popup_active { detail_popup::draw_detail_popup(f, app); }
    else if sniffer_active { detail_popup::draw_sniffer_overlay(f, app); }

    if app.config_editor.is_some() {
        dim_background(f, dim);
        config_editor::draw_config_editor(f, app);
    }

    // Help overlay
    if app.show_help {
        dim_background(f, dim);
        // The same size as Job Detail, the largest popup.
        let help_area = detail_popup::centered_rect(85, 85, f.area());
        f.render_widget(Clear, help_area);
        draw_help_overlay(f, help_area, app);
    }

    // Save-to-file prompt: opened from inside a popup, so it's the top layer.
    if app.save_prompt.is_some() {
        dim_background(f, dim);
        detail_popup::draw_save_prompt(f, app);
    }

    if app.quit_confirm {
        dim_background(f, dim);
        detail_popup::draw_quit_confirm(f);
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
    let mut right_spans = value_hint("-", format!("{}ms", app.config.tick_ms), "+", color::border());
    if app.config.max_output_kbs > 0 {
        // Yellow while redraws wait for the output cap.
        right_spans.push(Span::styled("\u{2524}", Style::default().fg(color::border())));
        right_spans.push(Span::styled(
            format!(" \u{2264}{}KB/s ", app.config.max_output_kbs),
            Style::default().fg(if app.output_held { color::yellow() } else { color::inactive_fg() }),
        ));
        right_spans.push(Span::styled("\u{251C}", Style::default().fg(color::border())));
    }
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

    let mouse_hint = if app.config.mouse { "mouse off".to_string() } else { "mouse on".to_string() };
    let admin_hint = if app.admin_mode { "admin off".to_string() } else { "admin on".to_string() };
    let global_hints = border_hints(&[
        ("?/F1", "help".to_string()),
        ("F2", mouse_hint),
        ("F3", "config".to_string()),
        ("a", admin_hint),
        ("q", "quit".to_string()),
    ], color::border());

    let upgraded = app.upgraded_from.as_ref().map(|_| Line::from(vec![
        Span::styled("\u{2524}", Style::default().fg(color::border())),
        Span::styled(format!(" updated to v{version} \u{00B7} F1: what's new "), Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
        Span::styled("\u{251C}", Style::default().fg(color::border())),
    ]));
    let top_left_width = left_title.width() + center_title.width();
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::border()))
        .style(Style::default().bg(color::main_bg()))
        .title_top(left_title.left_aligned())
        .title_top(center_title.centered());
    if fits_beside(top_left_width, &right_title, area.width) {
        block = block.title_top(right_title.right_aligned());
    }
    if fits_beside(upgraded.as_ref().map_or(0, Line::width), &global_hints, area.width) {
        block = block.title_bottom(global_hints.right_aligned());
    }
    if let Some(line) = upgraded {
        block = block.title_bottom(line.left_aligned());
    }
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Tab menu (left) and host/uptime (right) share the one content row.
    let mut tab_spans: Vec<Span<'static>> = Vec::new();
    for (i, tab) in Tab::ORDER.iter().enumerate() {
        if i > 0 {
            tab_spans.push(Span::styled(TAB_SEPARATOR, Style::default().fg(color::tab_separator())));
        }
        let label = tab.bar_label();
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
    let label = |s: &'static str| Span::styled(s, Style::default().fg(color::label_fg()));
    let value = |s: String| Span::styled(s, Style::default().fg(color::cyan()).add_modifier(Modifier::BOLD));
    let mut info_spans = vec![label("host: "), value(format!("{} ", hostname))];
    let radar_up = [
        Span::styled("\u{2502} ", Style::default().fg(color::tab_separator())),
        label("radar up: "),
        value(format!("{} ", dashboard::format_duration(app_uptime))),
    ];
    let host_up = [label("| host up: "), value(format!("{} ", dashboard::format_uptime(host_uptime)))];
    // The tabs come first: the info drops host uptime, then radar uptime,
    // to leave them room and a gap.
    let room = (inner.width as usize).saturating_sub(Line::from(tab_spans.clone()).width() + 1);
    let width = |spans: &[Span]| spans.iter().map(|s| s.width()).sum::<usize>();
    let full = width(&info_spans) + width(&radar_up) + if host_uptime > 0 { width(&host_up) } else { 0 };
    if full <= room || width(&info_spans) + width(&radar_up) <= room {
        info_spans.extend(radar_up);
        if host_uptime > 0 && full <= room {
            info_spans.extend(host_up);
        }
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
        let k1 = key::render("Esc", "Cancel");
        let k2 = key::render("Enter", "Start");
        let mut all_spans: Vec<Span<'static>> = Vec::new();
        all_spans.extend(k1);
        all_spans.extend(k2);
        all_spans.extend(input_spans(
            "  Sniff for (minion, glob, keyword or /regex/, comma-separated): ",
            &app.sniffer_input_text,
            app.input_cursor,
            Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD),
        ));
        if let Some(ref err) = app.sniffer_input_error {
            all_spans.push(Span::styled(format!("  \u{2717} {err}"), Style::default().fg(color::red()).add_modifier(Modifier::BOLD)));
        } else if app.sniffer_input_text.is_empty() {
            all_spans.push(Span::styled("  e.g. web-*, nginx, /ret/(db|web)\\d+/i  \u{00B7}  F1: examples", Style::default().fg(color::inactive_fg())));
        }
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
        all_spans.extend(input_spans("  filter: ", &app.filter_text, app.input_cursor, Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)));
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
    if app.config.mouse {
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
                spans.push(Span::styled(format!(" filter: {} ", app.cache_filter_text), Style::default().fg(color::yellow())));
            } else if !app.cache_focus && !app.filter_text_for(Tab::Jobs).is_empty() {
                spans.push(Span::styled(format!(" filter: {} ", app.filter_text_for(Tab::Jobs)), Style::default().fg(color::yellow())));
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
                Tab::Master => (app.master_selected + 1, app.filtered_master_settings().len()),
                _ => (0, 0),
            };
            if total > 0 {
                spans.push(Span::styled(format!(" {}/{} ", cur, total), Style::default().fg(color::cyan())));
            }
            let active_filter = match app.current_tab {
                Tab::Events => app.applied_filter.as_str(),
                Tab::Minions | Tab::Alerts | Tab::Api | Tab::Master => app.filter_text_for(app.current_tab),
                _ => "",
            };
            if !active_filter.is_empty() {
                spans.push(Span::styled(format!(" filter: {} ", active_filter), Style::default().fg(color::yellow())));
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

/// One keyboard shortcut, spelled exactly as in the README's tables (keys in
/// backticks) — the help overlay is drawn from this list and a test checks
/// every entry against the README, so the two can't drift apart.
pub struct Shortcut {
    /// Tab(s) the key applies to, for the "Per tab" group; empty otherwise.
    pub tab: &'static str,
    pub key: &'static str,
    pub action: &'static str,
}

/// Every shortcut, grouped like the README's keyboard tables.
pub const SHORTCUTS: &[(&str, &[Shortcut])] = &[
    ("General", &[
        Shortcut { tab: "", key: "`q`", action: "Quit (asks first unless `confirm_quit = false`; `Ctrl+C` quits at once)" },
        Shortcut { tab: "", key: "`Tab` / `Shift+Tab`", action: "Next / previous tab" },
        Shortcut { tab: "", key: "`1`–`8`", action: "Jump to tab (Dashboard/Minions/Events/Jobs/Alerts/API/Master Config/Pulse)" },
        Shortcut { tab: "", key: "`↑`/`↓`", action: "Scroll / navigate" },
        Shortcut { tab: "", key: "`PgUp`/`PgDn`", action: "Page scroll" },
        Shortcut { tab: "", key: "`Home`/`End`", action: "First/last row" },
        Shortcut { tab: "", key: "`Enter`", action: "Open detail popup" },
        Shortcut { tab: "", key: "`Esc`", action: "Close popup / help" },
        Shortcut { tab: "", key: "`/`", action: "Filter (current tab)" },
        Shortcut { tab: "", key: "`a`", action: "Toggle admin mode" },
        Shortcut { tab: "", key: "`?` / `F1`", action: "Help overlay; `Tab` / `←` / `→` switch its pages (`Shift+Tab` back): Keys, Guide (filter, search and sniffer syntax, panel notes) and What's new (the changelog)" },
        Shortcut { tab: "", key: "`F2`", action: "Toggle mouse mode" },
        Shortcut { tab: "", key: "`F3`", action: "Open Config Editor" },
        Shortcut { tab: "", key: "`+`/`-`", action: "Adjust refresh interval" },
        Shortcut { tab: "", key: "`m`", action: "Toggle sparkline style (Dashboard)" },
        Shortcut { tab: "", key: "`Ctrl+←`/`Ctrl+→`", action: "Move the divider between the two panels (Minions, Events, Jobs, API, Master Config, Pulse)" },
        Shortcut { tab: "", key: "`F4`–`F8`", action: "Toggle CPU/Memory/ZMQ/Events/Minion-activity graphs (Dashboard)" },
    ]),
    ("Per tab", &[
        Shortcut { tab: "Events", key: "`n`", action: "Launch event sniffer" },
        Shortcut { tab: "Events", key: "`s` / `S`", action: "Save the filtered event list to a file (text / NDJSON)" },
        Shortcut { tab: "Minions", key: "`r`", action: "Minion event/return history popup" },
        Shortcut { tab: "Minions", key: "`s`", action: "Fleet scan (admin mode only)" },
        Shortcut { tab: "Minions", key: "`x`", action: "Hide/show non-responsive minions" },
        Shortcut { tab: "Minions", key: "`z`", action: "Collapse/expand timed-out minions into one row (when not hidden)" },
        Shortcut { tab: "Minions", key: "`f`", action: "Toggle whether the Fleet Overview breakdown follows the active list filter" },
        Shortcut { tab: "Minions, Jobs, Alerts, API", key: "`o` / `O`", action: "Sort by the next column / reverse the sort (also: click a column header)" },
        Shortcut { tab: "Minions, Events, Jobs, Alerts, API", key: "`v`", action: "Choose which columns the focused table shows, and their order" },
        Shortcut { tab: "Jobs (Live)", key: "`x`", action: "Show only failed jobs / all jobs" },
        Shortcut { tab: "Jobs (Cache)", key: "`/`", action: "Filter cache entries (server-side)" },
        Shortcut { tab: "Jobs (Cache)", key: "`[`/`]`", action: "Decrease/increase page limit by 500" },
        Shortcut { tab: "Jobs (Cache)", key: "`,`/`.`", action: "Previous/next page" },
        Shortcut { tab: "Jobs (Cache)", key: "`r`", action: "Refresh job cache" },
        Shortcut { tab: "Jobs (Cache)", key: "`←`/`→`", action: "Switch focus between Live Jobs and Cache panels" },
        Shortcut { tab: "Pulse", key: "`↑`/`↓`/`←`/`→`", action: "Navigate the minion grid" },
        Shortcut { tab: "Master Config", key: "`Enter`", action: "The setting's full value, where it's set and Salt's default" },
        Shortcut { tab: "Master Config", key: "`r`", action: "Read the configuration files again" },
        Shortcut { tab: "API", key: "`x`", action: "Hide/show salt-radar's own `GET /stats` poll in the request log (hidden by default — see `hide_stats_polls`)" },
    ]),
    ("Config Editor", &[
        Shortcut { tab: "", key: "`↑`/`↓`", action: "Move between fields" },
        Shortcut { tab: "", key: "`PgUp`/`PgDn`", action: "Page through fields" },
        Shortcut { tab: "", key: "`Home`/`End`", action: "First/last field" },
        Shortcut { tab: "", key: "`Enter`", action: "Edit field / open picker (fixed-option fields) or file browser (path fields) / confirm picker selection" },
        Shortcut { tab: "", key: "`Esc`", action: "Cancel edit or close the picker without changing the value" },
        Shortcut { tab: "", key: "`s`", action: "Save all changes and apply them" },
        Shortcut { tab: "", key: "`←` / `Backspace`", action: "Up one folder (file browser)" },
        Shortcut { tab: "", key: "`.`", action: "Show/hide hidden files (file browser; while not typing a name)" },
        Shortcut { tab: "", key: "`/`", action: "Type the path by hand (file browser)" },
        Shortcut { tab: "", key: "letters, digits", action: "Jump to the first entry starting with what you type (file browser)" },
    ]),
    ("Popups", &[
        Shortcut { tab: "", key: "`y`", action: "Copy to clipboard" },
        Shortcut { tab: "", key: "`Y`", action: "Copy raw JSON (job detail)" },
        Shortcut { tab: "", key: "`s`", action: "Save to file" },
        Shortcut { tab: "", key: "`S`", action: "Save raw JSON to file (job detail; NDJSON in the sniffer)" },
        Shortcut { tab: "", key: "`a`", action: "Save every return in the job to one file (job detail)" },
        Shortcut { tab: "", key: "`A`", action: "Save every return as one JSON object (job detail)" },
        Shortcut { tab: "", key: "`j`/`k`", action: "Next/previous return (job detail)" },
        Shortcut { tab: "", key: "`PgUp`/`PgDn`", action: "Jump 10 returns" },
        Shortcut { tab: "", key: "`Home`/`End`", action: "First/last return (job detail)" },
        Shortcut { tab: "", key: "`Enter`", action: "Enter scroll mode (event/job detail)" },
        Shortcut { tab: "", key: "`/`", action: "Search the return output (job detail, output pane), an event (event detail, raw or pretty view), an alert's items (alert detail) or the help pages, matches highlighted" },
        Shortcut { tab: "", key: "`n`/`N`", action: "Next/previous search match, marked in place (job detail, also from the returns list; event detail; alert detail; help)" },
        Shortcut { tab: "", key: "`f`/`F`", action: "Next/previous failed state, marked in the output (job detail; also from the returns list)" },
        Shortcut { tab: "", key: "`Esc`", action: "Exit scroll mode" },
        Shortcut { tab: "", key: "`Home`/`End`", action: "Jump to start/end of output (scroll mode)" },
        Shortcut { tab: "", key: "`t`", action: "Toggle raw JSON / pretty view (event detail)" },
        Shortcut { tab: "", key: "`h`", action: "Show only events matching keywords or `/regex/` terms (sniffer)" },
        Shortcut { tab: "", key: "`p`", action: "Pause/resume (sniffer)" },
        Shortcut { tab: "", key: "`g`", action: "Fetch grains for this minion (minion detail, admin mode only)" },
    ]),
];

fn draw_help_overlay(f: &mut Frame, area: Rect, app: &mut App) {
    use crate::app::HelpPage;
    let border = Style::default().fg(color::border_focus());
    // `┤ Keys ├┤ Guide ├┤ What's new ├` like the hints in the border, each
    // label clickable. A left-aligned title starts right after the corner.
    let mut spans = vec![Span::styled("\u{2524}", border)];
    let mut x = area.x + 2;
    app.help_page_hits.clear();
    for (i, page) in HelpPage::ALL.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("\u{251C}\u{2524}", border));
            x += 2;
        }
        let label = format!(" {} ", page.title());
        let w = label.chars().count() as u16;
        app.help_page_hits.push((area.y, x, x + w, page));
        x += w;
        spans.push(if page == app.help_page {
            Span::styled(label, Style::default().fg(color::title()).add_modifier(Modifier::BOLD))
        } else {
            Span::styled(label, Style::default().fg(color::inactive_fg()))
        });
    }
    spans.push(Span::styled("\u{251C}", border));
    let pages = Line::from(spans);
    let pages_width = pages.width();
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::border_focus()))
        .style(Style::default().bg(color::popup_bg()))
        .title_top(pages.left_aligned());
    let inner = block.inner(area);
    let lines = match app.help_page {
        HelpPage::Keys => keys_help_lines(),
        HelpPage::Guide => guide_help_lines(),
        HelpPage::WhatsNew => whats_new::lines(inner.width.saturating_sub(1) as usize),
    };
    if std::mem::take(&mut app.help_search_jump) {
        let (_, matches) = detail_popup::highlight_matches(lines.clone(), &app.help_search, None);
        if let Some(i) = crate::app::step_match(&matches, None, app.help_scroll as usize, true) {
            app.help_search_focus = Some(i);
            app.help_scroll = crate::app::help_scroll_to(matches[i]);
        }
    }
    let (lines, matches) = detail_popup::highlight_matches(lines, &app.help_search, app.help_search_focus);
    app.help_search_matches = matches;
    let matches = &app.help_search_matches;
    let focus = app.help_search_focus.filter(|&i| i < matches.len());

    let hints = if app.help_searching {
        vec![("Esc", "cancel search".to_string()), ("Enter", "find".to_string())]
    } else {
        let mut h = vec![("\u{2191}\u{2193}/PgUp/PgDn", "scroll".to_string()), ("/", "search".to_string())];
        if !matches.is_empty() {
            h.push(("n/N", "next/prev".to_string()));
        }
        h.push(("Tab/\u{2190}\u{2192}", "page".to_string()));
        h.push(("Esc", if app.help_search.is_empty() { "close" } else { "clear search" }.to_string()));
        h
    };
    let hint_line = border_hints(&hints, color::border_focus());
    let hint_width = hint_line.width();
    block = block.title_bottom(hint_line.left_aligned());
    if !app.help_search.is_empty() && !app.help_searching {
        let status = match (focus, matches.len()) {
            (_, 0) => "no matches".to_string(),
            (Some(i), n) => format!("match {} of {}", i + 1, n),
            (None, 1) => "1 match".to_string(),
            (None, n) => format!("{n} matches"),
        };
        let status = Line::from(Span::styled(
            format!(" search {}: {} ", app.help_search, status),
            Style::default().fg(if matches.is_empty() { color::red() } else { color::yellow() }),
        ));
        if fits_beside(pages_width, &status, area.width) {
            block = block.title_top(status.right_aligned());
        }
    }
    let max_scroll = (lines.len() as u16).saturating_sub(inner.height);
    app.help_scroll = app.help_scroll.min(max_scroll);
    let scroll = app.help_scroll;
    if scroll < max_scroll && fits_beside(hint_width, &more_below(color::border_focus()), area.width) {
        block = block.title_bottom(more_below(color::border_focus()));
    }
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(color::popup_bg())).scroll((scroll, 0)), inner);
    if app.help_searching {
        let bar = Rect { y: inner.y + inner.height.saturating_sub(1), height: 1.min(inner.height), ..inner };
        f.render_widget(Clear, bar);
        f.render_widget(
            Paragraph::new(Line::from(input_spans(
                "  Search: ",
                &app.help_search,
                app.input_cursor,
                Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD),
            ))).style(Style::default().bg(color::popup_bg())),
            bar,
        );
    }
}

/// Heading of the help section `F1` jumps to from the sniffer's prompts.
pub const SNIFFER_HELP_TITLE: &str = "Event Sniffer terms";

/// The help Guide page's line holding a section's heading.
pub fn guide_section_line(title: &str) -> Option<usize> {
    guide_help_lines().iter().position(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>().trim() == title)
}

/// A help section explaining a kind of input: a heading, an intro line, one
/// `kind  example  meaning` row per kind of term, then notes.
fn term_help_section(title: &str, intro: &str, rows: &[(&str, &str, &str)], notes: &[&str]) -> Vec<Line<'static>> {
    let kind_w = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0);
    let example_w = rows.iter().map(|r| r.1.chars().count()).max().unwrap_or(0);
    let note = |text: &str| Line::from(Span::styled(format!("  {text}"), Style::default().fg(color::inactive_fg())));
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(format!("  {title}"), Style::default().fg(color::title()).add_modifier(Modifier::BOLD))),
        note(intro),
    ];
    for (kind, example, what) in rows {
        lines.push(Line::from(vec![
            Span::styled(format!("  {kind:<kind_w$}  "), Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{example:<example_w$}  "), Style::default().fg(color::cyan())),
            Span::styled(what.to_string(), Style::default().fg(color::main_fg())),
        ]));
    }
    lines.extend(notes.iter().map(|n| note(n)));
    lines
}

/// What the list filters (`/`) and the output / help searches take.
fn filter_help_lines() -> Vec<Line<'static>> {
    term_help_section(
        "Filter and search terms",
        "/ on a list filters it by comma-separated terms; a row matching any term stays.",
        &[
            ("plain", "nginx", "the text anywhere, any case"),
            ("glob", "web-*  db?", "a whole minion ID, like Salt's targeting"),
            ("regex", "/web-0[1-5]$/", "a regex, case-sensitive; /…/i ignores case"),
            ("several", "web-*, /^db-0[0-9]$/", "any of the terms matches"),
            ("search", "/Result: (False|None)/", "output and help search: one query"),
        ],
        &[
            "Text that isn't a complete /…/ is plain, so /etc/salt/minion finds the path.",
            "A /…/ that isn't a usable regex (/x/g, look-around) is matched as text, flagged ⚠.",
            "Filter terms get a color each; search matches are tinted, the current one solid.",
            "Job Cache filters on the master: globs match the target, regexes use Python's re.",
        ],
    )
}

/// What can be typed when starting the sniffer (`n`) or narrowing it (`h`),
/// one example per kind of term.
fn sniffer_help_lines() -> Vec<Line<'static>> {
    term_help_section(
        SNIFFER_HELP_TITLE,
        "n on Events starts the sniffer; it captures events matching any term.",
        &[
            ("minion", "web01", "its events, returns and the jobs targeting it"),
            ("glob", "web-*  db?", "the same for each known minion it matches"),
            ("keyword", "nginx", "anywhere in the tag or JSON, any case"),
            ("regex", r"/salt/job/\d+/ret/db\d+/", "a tag or JSON line matching it"),
            ("regex (i)", "/error|timed out/i", "the same, ignoring case"),
            ("several", "web-*, nginx, /fail(ed)?/i", "any of the terms matches"),
            ("h (narrow)", "/rocky9-1[0-9]/, timeout", "in a running sniffer: show only matches"),
        ],
        &[
            "Keywords and regexes match the tag and pretty JSON line by line.",
            "Inside /…/, commas and slashes belong to the regex. Empty h shows all.",
            "Each term colors its matches; an invalid regex shows its error.",
        ],
    )
}

/// The help Keys page: every keyboard shortcut, grouped.
fn keys_help_lines() -> Vec<Line<'static>> {
    let key_style = Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD);
    let mut lines = vec![
        Line::from(Span::styled("  Keys", Style::default().fg(color::title()).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("  \u{2500}".to_string() + &"\u{2500}".repeat(15), Style::default().fg(color::div_line()))),
        Line::from(Span::styled("  Most shortcuts are also shown live in the border of the panel they affect.", Style::default().fg(color::inactive_fg()))),
    ];
    let shown = |text: &str| text.replace('`', "");
    for (group, shortcuts) in SHORTCUTS {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(format!("  {}", group), Style::default().fg(color::title()).add_modifier(Modifier::BOLD))));
        let key_w = shortcuts.iter().map(|s| shown(s.key).chars().count()).max().unwrap_or(0);
        for s in shortcuts.iter() {
            let mut spans = vec![Span::styled(format!("  {:<w$}  ", shown(s.key), w = key_w), key_style)];
            if !s.tab.is_empty() {
                spans.push(Span::styled(format!("{}: ", s.tab), Style::default().fg(color::label_fg())));
            }
            spans.push(Span::styled(shown(s.action), Style::default().fg(color::main_fg())));
            lines.push(Line::from(spans));
        }
    }
    lines
}

/// The help Guide page: the syntax of filters, searches and the sniffer,
/// then notes on what some panels show.
fn guide_help_lines() -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled("  Guide", Style::default().fg(color::title()).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("  \u{2500}".to_string() + &"\u{2500}".repeat(15), Style::default().fg(color::div_line()))),
        Line::from(Span::styled("  What filters, searches and the sniffer take, and what some panels show.", Style::default().fg(color::inactive_fg()))),
    ];
    lines.extend(filter_help_lines());
    lines.extend(sniffer_help_lines());
    lines.extend([
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
        Line::from(vec![
            Span::styled("  OK / FAIL", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Salt's success and retcode, plus a failed state or an ERROR: string Salt missed;", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  Live Jobs, Job Detail and Fail% all use this rule (an empty result is OK)", Style::default().fg(color::main_fg())),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Minion Detail", Style::default().fg(color::title()).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  Last Seen", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  the minion's last job return seen live, since salt-radar started", Style::default().fg(color::main_fg())),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Minions list", Style::default().fg(color::title()).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  one row per key", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  adds up to Minion Key Status; pending, rejected and denied keys too", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  listed twice", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  an ID with an accepted and a denied key (it came back with a new key)", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  Lag", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  the minion's return time ÷ the job's median, median of its last 50 jobs:", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  1.0x typical, yellow from 1.5x, red from 3x, whatever the jobs were", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  rated once all targets returned or after job_straggler_secs, later returns", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  against that median; times under 0.5 s count as 0.5 s, jobs where both the", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  minion and the median took under 0.5 s aren't rated (\u{2014} until one is)", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  Ping", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  time to the minion's latest test.ping return, whoever sent it", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  both fill in from jobs seen while salt-radar runs; v shows the columns", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  Proto", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  auth protocol by Salt version: v0 before the March 2022 security releases,", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  v2 from 3002.8, 3003.4, 3004.1 and 3005, v3 from 3006.12, 3007.4 and 3008;", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  masters from 3006.17, 3007.9 and 3008 reject v0 and v2 by default (minimum_auth_version)", Style::default().fg(color::main_fg())),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Master Config tab", Style::default().fg(color::title()).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  load order", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  /etc/salt/master, then master.d/*.conf (sorted), then include:", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  list", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  settings set in a file first (load order), then derived, then defaults", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  right panel", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  the file the setting comes from, its block marked, and what each file", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  setting it does; Enter shows each file's own value, then the result", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  / and r", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  filter by name, value, file or kind (set, module, not core, restart, ...);", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  r reads the files again (also at start and every config_secs)", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  set", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  in a file; Source is the file and line that wins, +N when others set it too", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  replaces", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  a master.d file replaces the main file's whole value: nested keys it", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  doesn't set are dropped (named); include: files replace master.d ones", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  merged", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  between master.d files dicts merge and lists add up; other values override", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ignored", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  in a file Salt rejected (a key twice is enough): the whole file is skipped", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  module", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  not salt-master's own, but read by salt-api (rest_cherrypy, whose options", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  are checked too), a module Salt ships (auth.ldap.*, mysql.*, vault) or a", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  minion, syndic or salt-ssh option", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  not core", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  in a file, but nothing in Salt reads it: a typo, which Salt ignores; a near", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  miss names the setting it's probably meant to be", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  derived", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  in no file, but not Salt's default either: worked out at startup", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  default", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  Salt's default, in no file", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  \u{21bb}", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  differs from what the running master holds: may change on restart (changed", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  after it started, restored with an old file time, or removed from the files)", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  <redacted>", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  values of credential-looking keys (*pass*, *secret*, *token, *_key), nested", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("  ones too, and the password or token in any URL, blanked by the agent", Style::default().fg(color::main_fg())),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Events Preview", Style::default().fg(color::title()).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  long lines", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  wrap under their key; Enter opens the whole event", Style::default().fg(color::main_fg())),
        ]),
        Line::from(vec![
            Span::styled("  filter match", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD)),
            Span::styled("  highlighted; the body starts at the first one when it would be cut off", Style::default().fg(color::main_fg())),
        ]),
        Line::from(""),
        Line::from(Span::styled(format!("  salt-radar v{} \u{2014} Salt Master Monitor", env!("CARGO_PKG_VERSION")), Style::default().fg(color::inactive_fg()))),
        Line::from(Span::styled("  Read-only TUI. All data sourced from /proc, systemctl,", Style::default().fg(color::inactive_fg()))),
        Line::from(Span::styled("  and Salt's own APIs via the bundled Python agent.", Style::default().fg(color::inactive_fg()))),
    ]);
    lines
}

// ─── Sub-view modules ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span};

    fn bottom_border(block: Block<'static>, width: u16) -> String {
        let area = Rect::new(0, 0, width, 3);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        ratatui::widgets::Widget::render(block, area, &mut buf);
        (0..width).map(|x| buf[(x, 2)].symbol().to_string()).collect()
    }

    #[test]
    fn more_below_gives_way_to_the_hints_on_a_narrow_panel() {
        let hints = [("/", "filter".to_string()), ("Enter", "detail".to_string())];
        let wide = bottom_border(list_block_hinted(" Jobs ", Style::default(), Color::Reset, false, true, &hints, 60), 60);
        assert!(wide.contains("Enter:detail") && wide.contains("more below"), "{wide}");
        let narrow = bottom_border(list_block_hinted(" Jobs ", Style::default(), Color::Reset, false, true, &hints, 34), 34);
        assert!(narrow.contains("Enter:detail") && !narrow.contains("more"), "{narrow}");
    }

    #[test]
    fn wrap_spans_hangs_under_the_key_and_keeps_styles() {
        let red = Style::default().fg(Color::Red);
        let spans = vec![Span::raw("    \"tgt\": \""), Span::styled("web01,web02,web03", red), Span::raw("\",")];
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        let rows = wrap_spans(spans, 16);
        let plain: Vec<String> = rows.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect()).collect();
        assert_eq!(plain, ["    \"tgt\": \"web0", "      1,web02,we", "      b03\","]);
        assert_eq!(rows.len(), wrapped_rows(&text, 16));
        assert!(rows[1].spans.iter().any(|s| s.content == "1,web02,we" && s.style == red), "a style carries across a break");
        assert_eq!(wrapped_rows("short", 16), 1);
        assert_eq!(wrap_spans(vec![Span::raw("exactly 16 chars")], 16).len(), 1);
    }

    #[test]
    fn help_pages_split_keys_from_the_guide() {
        use crate::app::HelpPage;
        assert_eq!(HelpPage::Keys.step(true), HelpPage::Guide);
        assert_eq!(HelpPage::WhatsNew.step(true), HelpPage::Keys);
        assert_eq!(HelpPage::Keys.step(false), HelpPage::WhatsNew);
        let text = |lines: Vec<Line>| lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()).collect::<Vec<_>>().join("
");
        let keys = text(keys_help_lines());
        let guide = text(guide_help_lines());
        assert!(!keys.contains("Filter and search terms") && !keys.contains("Dashboard KPIs"));
        assert!(guide.contains("Filter and search terms") && guide.contains(SNIFFER_HELP_TITLE) && guide.contains("Dashboard KPIs"));
        assert!(guide_section_line(SNIFFER_HELP_TITLE).is_some());
    }

    #[test]
    fn term_colors_differ_within_a_search_until_the_palette_runs_out() {
        let colors = color::term_colors(["ubuntu22", "ubuntu24", "UBUNTU22", "ubuntu26"]);
        assert_eq!(colors.len(), 3);
        let c = |t| color::term_color_in(&colors, t);
        assert!(c("ubuntu22") != c("ubuntu24") && c("ubuntu24") != c("ubuntu26") && c("ubuntu22") != c("ubuntu26"));
        let many = color::term_colors(["a", "b", "c", "d", "e", "f", "g"]);
        assert_eq!(color::term_color_in(&many, "g"), color::term_color_in(&many, "a"));
    }

    #[test]
    fn every_help_shortcut_is_a_readme_row() {
        let readme = include_str!("../../README.md");
        for (group, shortcuts) in SHORTCUTS {
            for sc in shortcuts.iter() {
                let row = if sc.tab.is_empty() {
                    format!("| {} | {} |", sc.key, sc.action)
                } else {
                    format!("| {} | {} | {} |", sc.tab, sc.key, sc.action)
                };
                assert!(readme.contains(&row), "{}: README has no row {}", group, row);
            }
        }
    }

    #[test]
    fn scroll_window_never_scrolls_a_list_that_fits() {
        for sel in 0..15 {
            assert_eq!(scroll_window(sel, 40, 15), (0, false, false));
        }
    }

    #[test]
    fn scroll_window_centers_then_clamps_at_the_end() {
        assert_eq!(scroll_window(0, 10, 100), (0, false, true));
        assert_eq!(scroll_window(50, 10, 100), (45, true, true));
        assert_eq!(scroll_window(99, 10, 100), (90, true, false));
    }

    #[test]
    fn gradient_hits_its_stops_and_blends_between_them() {
        let t = color::Theme::DEFAULT;
        assert_eq!(color::gradient(0.0), t.gradient_start);
        assert_eq!(color::gradient(0.5), t.gradient_mid);
        assert_eq!(color::gradient(0.75), t.gradient_high);
        assert_eq!(color::gradient(1.0), t.gradient_end);
        assert_eq!(color::gradient(-1.0), t.gradient_start);
        assert_eq!(color::gradient(7.0), t.gradient_end);
        assert_eq!(color::gradient(f64::NAN), t.gradient_start);
        let (Color::Rgb(r0, ..), Color::Rgb(r1, ..), Color::Rgb(r, ..)) = (t.gradient_start, t.gradient_mid, color::gradient(0.25)) else { panic!() };
        assert!(r0.min(r1) <= r && r <= r0.max(r1), "a quarter of the way is between start and mid");
        assert_eq!(color::mem_gradient(0.0), t.mem_gradient_start);
        assert_eq!(color::mem_gradient(1.0), t.mem_gradient_end);
    }

    #[test]
    fn selected_row_lifts_dim_text_and_keeps_other_colors() {
        let line = Line::from(vec![
            Span::styled("dim", Style::default().fg(color::inactive_fg())),
            Span::styled("ok", Style::default().fg(color::green())),
        ]);
        let row = color::selected_row(line);
        assert_eq!(row.spans[0].style.fg, Some(color::hi_fg()));
        assert_eq!(row.spans[1].style.fg, Some(color::green()));
        assert_eq!(row.style.bg, Some(color::selected_bg()));
        assert!(row.style.add_modifier.contains(Modifier::BOLD));
    }
}
