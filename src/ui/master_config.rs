use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::types::{ConfigSetting, MasterConfig, SettingKind, SourceRelation, Tab};

use super::color;
use super::table::cell;

pub fn draw_master(f: &mut Frame, area: Rect, app: &App, focused: bool) {
    let [left, right] = super::split_panes(area, app);
    let rows = app.filtered_master_settings();
    draw_master_list(f, left, app, focused, &rows);
    let selected = rows.get(app.master_selected.min(rows.len().saturating_sub(1)));
    let setting = app.master_config.as_ref().zip(selected).map(|(mc, &i)| (mc, &mc.settings[i]));
    draw_setting_source(f, right, setting);
}

pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A block's lines for a title: `line 3`, `lines 5–13`, or nothing when the
/// agent couldn't place it (start 0).
pub fn line_span(start: usize, end: usize) -> String {
    match end.saturating_sub(1) {
        _ if start == 0 => String::new(),
        last if last > start => format!("lines {}\u{2013}{}", start, last),
        _ => format!("line {}", start),
    }
}

fn kind_colors(s: &ConfigSetting) -> (Style, Style) {
    let value = Style::default().fg(color::cyan());
    match s.kind {
        SettingKind::Explicit => (Style::default().fg(color::main_fg()).add_modifier(Modifier::BOLD), value),
        SettingKind::Unknown if s.suggest.is_some() => (Style::default().fg(color::red()).add_modifier(Modifier::BOLD), value),
        SettingKind::Unknown => (Style::default().fg(color::purple()).add_modifier(Modifier::BOLD), value),
        SettingKind::Ignored => (Style::default().fg(color::red()).add_modifier(Modifier::BOLD), Style::default().fg(color::inactive_fg())),
        SettingKind::Derived => (Style::default().fg(color::yellow()), Style::default().fg(color::main_fg())),
        SettingKind::Default => (Style::default().fg(color::inactive_fg()), Style::default().fg(color::inactive_fg())),
    }
}

/// Where a setting comes from, in a few characters: the winning file and
/// line, `+N` when other files set it too.
fn source_label(mc: &MasterConfig, s: &ConfigSetting) -> String {
    match s.winner() {
        Some(src) => {
            let name = mc.files.get(src.file).map_or("?", |f| file_name(&f.path));
            let at = if src.start > 0 { format!(":{}", src.start) } else { String::new() };
            let more = if s.sources.len() > 1 { format!(" +{}", s.sources.len() - 1) } else { String::new() };
            format!("{}{}{}", name, at, more)
        }
        None => s.kind.label().to_string(),
    }
}

/// `master.d/60-b.conf:9`-style name of a source, relative to the
/// configuration directory.
pub fn source_place(mc: &MasterConfig, file: usize, start: usize) -> String {
    let path = mc.files.get(file).map_or("?", |f| f.path.as_str());
    let dir = mc.conf_file.rsplit_once('/').map_or("", |(d, _)| d);
    let shown = path.strip_prefix(dir).map_or(path, |p| p.trim_start_matches('/'));
    if start > 0 { format!("{}:{}", shown, start) } else { shown.to_string() }
}

fn draw_master_list(f: &mut Frame, area: Rect, app: &App, focused: bool, rows: &[usize]) {
    let border_style = Style::default().fg(if focused { color::border_focus() } else { color::border() });
    let total = rows.len();
    let selected = app.master_selected.min(total.saturating_sub(1));
    let visible = (area.height as usize).saturating_sub(2).saturating_sub(2);
    let (scroll, has_more_above, has_more_below) = super::scroll_window(selected, visible, total);

    let hints: Vec<(&str, String)> = vec![
        ("/", "filter".to_string()),
        ("Enter", "value".to_string()),
        ("r", "reload".to_string()),
        ("PgUp/PgDn", "page".to_string()),
    ];
    let title = match &app.master_config {
        Some(mc) => {
            let count = |k: SettingKind| mc.settings.iter().filter(|s| s.kind == k).count();
            let pending = mc.settings.iter().filter(|s| s.restart_pending(mc)).count();
            let rejected = mc.files.iter().filter(|file| file.error.is_some()).count();
            // Warnings first: on a narrow panel the counts are what gets cut.
            format!(
                " Master Config{}{} \u{b7} {} set, {} not core, {} derived, {} default ",
                if rejected > 0 { format!(" \u{26a0} {} file(s) rejected", rejected) } else { String::new() },
                if pending > 0 { format!(" \u{21bb} {} on restart", pending) } else { String::new() },
                count(SettingKind::Explicit), count(SettingKind::Unknown), count(SettingKind::Derived), count(SettingKind::Default),
            )
        }
        None => " Master Config ".to_string(),
    };
    let block = super::list_block_hinted(title, border_style, color::main_bg(), has_more_above, has_more_below, &hints);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    if let Some(bar) = super::filter_bar(app.filter_text_for(Tab::Master), app.is_filtering, app.input_cursor, format!("{} items", total), true) {
        lines.push(bar);
    }

    let Some(mc) = &app.master_config else {
        let msg = match (&app.master_config_error, app.master_config_loading) {
            (Some(e), _) => format!("  Couldn't read the master configuration: {}", e),
            (None, true) => "  Reading the master configuration\u{2026}".to_string(),
            (None, false) => "  Not loaded yet.".to_string(),
        };
        lines.push(Line::from(Span::styled(msg, Style::default().fg(color::inactive_fg()))));
        f.render_widget(Paragraph::new(lines).style(Style::default().bg(color::main_bg())), inner);
        return;
    };

    let width = (inner.width as usize).saturating_sub(2);
    let key_w = rows.iter().map(|&i| mc.settings[i].key.chars().count()).max().unwrap_or(3).clamp(3, 26) + 1;
    let source_w = rows.iter().map(|&i| source_label(mc, &mc.settings[i]).chars().count() + 1).max().unwrap_or(7).clamp(7, 29);
    let value_w = width.saturating_sub(key_w + source_w + 1).max(8);
    let header = Style::default().fg(color::header_fg()).add_modifier(Modifier::BOLD);
    lines.push(Line::from(vec![
        Span::raw("  "),
        cell("Key", key_w, false, header),
        cell("Value", value_w, false, header),
        Span::raw(" "),
        cell("Source", source_w, false, header),
    ]));

    if total == 0 {
        lines.push(Line::from(Span::styled("  No settings match the filter.", Style::default().fg(color::inactive_fg()))));
    }
    let filter = app.filter_text_for(Tab::Master);
    for (pos, &i) in rows.iter().enumerate().skip(scroll).take(visible) {
        let s = &mc.settings[i];
        let (key_style, value_style) = kind_colors(s);
        let key = super::detail_popup::style_ranges(
            Line::from(cell(&s.key, key_w, false, key_style)),
            &crate::app::filter_match_ranges(&s.key, filter),
            |_| color::search_match(),
        );
        let mut spans = vec![if s.restart_pending(mc) {
            Span::styled("\u{21bb} ", Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD))
        } else {
            Span::raw("  ")
        }];
        spans.extend(key.spans);
        spans.push(cell(&s.line, value_w, false, value_style));
        spans.push(Span::raw(" "));
        spans.push(cell(&source_label(mc, s), source_w, false, Style::default().fg(color::inactive_fg())));
        let line = Line::from(spans);
        lines.push(if pos == selected { color::selected_row(line) } else { line });
    }
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(color::main_bg())), inner);
}

/// The right panel: what the setting is and where it comes from, then the
/// file that sets it with the setting's block marked.
fn draw_setting_source(f: &mut Frame, area: Rect, setting: Option<(&MasterConfig, &ConfigSetting)>) {
    let Some((mc, s)) = setting else {
        let block = super::titled_block_hinted(" Source ", Style::default().fg(color::border()), color::main_bg(), &[]);
        f.render_widget(block, area);
        return;
    };
    let winner = s.winner().and_then(|src| mc.files.get(src.file).map(|fl| (fl, src.start, src.end)));
    let title = match winner {
        Some((file, start, end)) if start > 0 => format!(" {} \u{b7} {} ", file.path, line_span(start, end)),
        Some((file, _, _)) => format!(" {} ", file.path),
        None => format!(" {} ", s.kind.label()),
    };
    let block = super::titled_block_hinted(title, Style::default().fg(color::border()), color::main_bg(), &[]);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let label = Style::default().fg(color::label_fg());
    let dim = Style::default().fg(color::inactive_fg());
    let (key_style, _) = kind_colors(s);
    let mut lines = vec![Line::from(vec![
        Span::styled(format!(" {}", s.key), key_style),
        Span::styled(format!("  {}", s.kind.label()), dim),
    ])];
    let note = |text: String, style: Style| Line::from(Span::styled(format!(" {}", text), style));
    match s.kind {
        SettingKind::Unknown => lines.push(match &s.suggest {
            Some(core) => note(format!("Not a Salt core setting \u{2014} did you mean {}?", core), Style::default().fg(color::red())),
            None => note("Not a Salt core setting: a module's own option, or a typo".to_string(), Style::default().fg(color::purple())),
        }),
        SettingKind::Derived => lines.push(note("In no file; Salt worked this out at startup".to_string(), Style::default().fg(color::yellow()))),
        SettingKind::Default => lines.push(note("In no file; Salt's default".to_string(), dim)),
        SettingKind::Ignored => lines.push(note("Only in a file Salt rejected: Salt's default is in use".to_string(), Style::default().fg(color::red()))),
        SettingKind::Explicit => {}
    }
    if s.sources.len() > 1 || s.kind == SettingKind::Ignored {
        let width = s.sources.iter().map(|src| source_place(mc, src.file, src.start).chars().count()).max().unwrap_or(0);
        for src in &s.sources {
            let style = match src.relation {
                SourceRelation::Ignored => Style::default().fg(color::red()),
                SourceRelation::Replaces => Style::default().fg(color::yellow()),
                _ => dim,
            };
            lines.push(Line::from(vec![
                Span::styled(format!(" {:<w$}  ", source_place(mc, src.file, src.start), w = width), label),
                Span::styled(src.relation.describe(), style),
            ]));
            if !src.dropped.is_empty() {
                lines.push(note(format!("  drops {}", src.dropped.join(", ")), Style::default().fg(color::yellow())));
            }
        }
    }
    if s.redacted {
        lines.push(note("Credentials in it are shown as <redacted>".to_string(), dim));
    }
    if let Some((file, _, _)) = winner {
        if let Some(error) = &file.error {
            lines.push(note(format!("\u{26a0} Salt rejected this file: {}", error), Style::default().fg(color::red()).add_modifier(Modifier::BOLD)));
        }
    }
    if s.restart_pending(mc) {
        lines.push(note("\u{21bb} Set in a file changed after salt-master started: may change on restart".to_string(), Style::default().fg(color::yellow())));
    }
    lines.push(Line::from(""));

    let room = (inner.height as usize).saturating_sub(lines.len());
    match winner {
        Some((file, start, end)) => {
            let text: Vec<&str> = file.text.lines().collect();
            let gutter = text.len().max(1).to_string().len();
            let first = if start > 0 {
                let block_len = end.saturating_sub(start);
                let lead = if block_len + 2 <= room { (room - block_len) / 3 } else { 1 };
                start.saturating_sub(1 + lead).min(text.len().saturating_sub(room))
            } else { 0 };
            for (n, line) in text.iter().enumerate().skip(first).take(room) {
                let number = n + 1;
                let inside = start > 0 && number >= start && number < end;
                let at_fault = number == file.error_line;
                let marker = if at_fault { color::red() } else { color::border_focus() };
                let mut spans = vec![
                    Span::styled(if inside || at_fault { "\u{258c}" } else { " " }, Style::default().fg(marker)),
                    Span::styled(format!("{:>w$} ", number, w = gutter), dim),
                ];
                let body = if line.trim_start().starts_with('#') { dim } else { Style::default().fg(color::main_fg()) };
                spans.push(Span::styled(line.to_string(), if inside { body.bg(color::bar_bg()) } else { body }));
                lines.push(Line::from(spans));
            }
        }
        None => {
            if s.kind == SettingKind::Derived {
                lines.push(note("Value:".to_string(), label));
                lines.extend(s.full.lines().map(|l| Line::from(Span::styled(format!("   {}", l), Style::default().fg(color::main_fg())))));
            }
            if let Some(default) = &s.default {
                lines.push(note("Salt's default:".to_string(), label));
                lines.extend(default.lines().map(|l| Line::from(Span::styled(format!("   {}", l), dim))));
            }
        }
    }
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(color::main_bg())), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_spans_read_naturally() {
        assert_eq!([line_span(3, 4), line_span(5, 14), line_span(0, 0)], ["line 3", "lines 5\u{2013}13", ""]);
    }
}
