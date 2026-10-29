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
    draw_setting_source(f, right, setting, app.config.master_wrap);
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

/// A line of the difference between the running master's value and the
/// files'; a changed line carries the byte range that differs from the line
/// it's paired with, when the two have something in common.
#[derive(Debug, PartialEq)]
pub enum DiffLine<'a> {
    Same(&'a str),
    Running(&'a str, Option<(usize, usize)>),
    Files(&'a str, Option<(usize, usize)>),
}

/// The lines of `running` and `files` in order, each one same, only in the
/// running master or only in the files. Within a change the two sides'
/// lines alternate, so each line sits next to the one it replaced.
pub fn diff_lines<'a>(running: &'a str, files: &'a str) -> Vec<DiffLine<'a>> {
    let a: Vec<&str> = running.lines().collect();
    let b: Vec<&str> = files.lines().collect();
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a[head..].iter().rev().zip(b[head..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (ma, mb) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let mut out: Vec<DiffLine> = a[..head].iter().map(|l| DiffLine::Same(l)).collect();
    // Longest common subsequence from each pair of positions to the end. A
    // value too big for the table shows as removed whole, then added whole.
    let (n, m) = (ma.len(), mb.len());
    let lcs = if n * m <= 4_000_000 {
        let mut t = vec![0u32; (n + 1) * (m + 1)];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                t[i * (m + 1) + j] = if ma[i] == mb[j] { t[(i + 1) * (m + 1) + j + 1] + 1 } else { t[(i + 1) * (m + 1) + j].max(t[i * (m + 1) + j + 1]) };
            }
        }
        Some(t)
    } else {
        None
    };
    let (mut i, mut j) = (0, 0);
    let (mut gone, mut came) = (Vec::new(), Vec::new());
    while i < n || j < m {
        let same = i < n && j < m && ma[i] == mb[j] && lcs.is_some();
        if !same {
            let take_running = j == m || (i < n && lcs.as_ref().is_none_or(|t| t[(i + 1) * (m + 1) + j] >= t[i * (m + 1) + j + 1]));
            if take_running {
                gone.push(ma[i]);
                i += 1;
            } else {
                came.push(mb[j]);
                j += 1;
            }
            continue;
        }
        push_change(&mut out, &mut gone, &mut came);
        out.push(DiffLine::Same(ma[i]));
        i += 1;
        j += 1;
    }
    push_change(&mut out, &mut gone, &mut came);
    out.extend(a[a.len() - tail..].iter().map(|l| DiffLine::Same(l)));
    out
}

fn push_change<'a>(out: &mut Vec<DiffLine<'a>>, gone: &mut Vec<&'a str>, came: &mut Vec<&'a str>) {
    for k in 0..gone.len().max(came.len()) {
        let (old, new) = (gone.get(k).copied(), came.get(k).copied());
        let spans = old.zip(new).and_then(|(o, n)| changed_part(o, n));
        if let Some(o) = old {
            out.push(DiffLine::Running(o, spans.map(|s| s.0)));
        }
        if let Some(n) = new {
            out.push(DiffLine::Files(n, spans.map(|s| s.1)));
        }
    }
    gone.clear();
    came.clear();
}

/// The byte ranges of `old` and `new` between what they start and end with
/// in common; None when they have nothing in common.
fn changed_part(old: &str, new: &str) -> Option<((usize, usize), (usize, usize))> {
    let pre: usize = old.chars().zip(new.chars()).take_while(|(x, y)| x == y).map(|(c, _)| c.len_utf8()).sum();
    let room = old.len().min(new.len()) - pre;
    let suf: usize = old[pre..].chars().rev().zip(new[pre..].chars().rev()).take_while(|(x, y)| x == y).map(|(c, _)| c.len_utf8()).scan(0, |used, len| {
        *used += len;
        (*used <= room).then_some(len)
    }).sum();
    (pre + suf > 0).then_some(((pre, old.len() - suf), (pre, new.len() - suf)))
}

/// A line of a setting's difference from the running master: marked `−` or
/// `+`, the part that differs from its paired line in reverse video,
/// wrapped under its own indent (not at all at `width` 0).
pub fn diff_rows(line: &DiffLine, width: usize) -> Vec<Line<'static>> {
    let (mark, text, part, fg) = match *line {
        DiffLine::Same(t) => ("    ", t, None, color::inactive_fg()),
        DiffLine::Running(t, p) => ("  \u{2212} ", t, p, color::red()),
        DiffLine::Files(t, p) => ("  + ", t, p, color::green()),
    };
    let style = Style::default().fg(fg);
    let mut content = Line::from(Span::styled(text.to_string(), style));
    if let Some((start, end)) = part.filter(|(s, e)| s < e) {
        content = super::detail_popup::style_ranges(content, &[(start, end)], |_| Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD));
    }
    super::wrap_spans(content.spans, width.saturating_sub(4))
        .into_iter()
        .enumerate()
        .map(|(i, row)| {
            let lead = if i == 0 { Span::styled(mark, style.add_modifier(Modifier::BOLD)) } else { Span::raw("    ") };
            Line::from([vec![lead], row.spans].concat())
        })
        .collect()
}

/// How many lines only the running master has, and only the files.
pub fn diff_counts(diff: &[DiffLine]) -> (usize, usize) {
    diff.iter().fold((0, 0), |(r, f), l| match l {
        DiffLine::Running(..) => (r + 1, f),
        DiffLine::Files(..) => (r, f + 1),
        DiffLine::Same(_) => (r, f),
    })
}

/// The changed lines of a difference, a `⋯` between changes that aren't next
/// to each other, in at most `budget` rows: the last says what `Enter`
/// shows, or how many changed lines didn't fit.
fn changed_rows(diff: &[DiffLine], width: usize, budget: usize) -> Vec<Line<'static>> {
    let dim = Style::default().fg(color::inactive_fg());
    let mut changes: Vec<Vec<Line<'static>>> = Vec::new();
    let mut gap = false;
    for line in diff {
        if matches!(line, DiffLine::Same(_)) {
            gap = !changes.is_empty();
            continue;
        }
        let mut rows = diff_rows(line, width);
        if std::mem::take(&mut gap) {
            rows.insert(0, Line::from(Span::styled("    \u{22ef}", dim)));
        }
        changes.push(rows);
    }
    let mut out = Vec::new();
    let mut shown = 0;
    for rows in &changes {
        if out.len() + rows.len() + 1 > budget {
            break;
        }
        out.extend(rows.iter().cloned());
        shown += 1;
    }
    out.push(Line::from(if shown < changes.len() {
        vec![Span::styled(format!("  \u{2026} {} more changed lines; Enter shows the difference", changes.len() - shown), dim)]
    } else {
        vec![
            Span::styled("  \u{2212}", Style::default().fg(color::red())),
            Span::styled(" running master  ", dim),
            Span::styled("+", Style::default().fg(color::green())),
            Span::styled(" after restart \u{b7} Enter shows it in context", dim),
        ]
    }));
    out
}

fn kind_colors(s: &ConfigSetting) -> (Style, Style) {
    let value = Style::default().fg(color::cyan());
    match s.kind {
        SettingKind::Explicit => (Style::default().fg(color::main_fg()).add_modifier(Modifier::BOLD), value),
        SettingKind::Module if !s.unknown_nested.is_empty() => (Style::default().fg(color::red()).add_modifier(Modifier::BOLD), value),
        SettingKind::Module => (Style::default().fg(color::main_fg()).add_modifier(Modifier::BOLD), value),
        SettingKind::Unknown if s.suggest.is_some() => (Style::default().fg(color::red()).add_modifier(Modifier::BOLD), value),
        SettingKind::Unknown if !s.notes.is_empty() => (Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD), value),
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

/// What became of `r`, ahead of the counts so a narrow panel keeps it: the
/// list looks the same before and after a reload that changed nothing.
fn reload_status(app: &App) -> String {
    if app.master_reload_id.is_some() {
        " \u{b7} reloading\u{2026}".to_string()
    } else if let (Some(e), Some(_)) = (&app.master_config_error, &app.master_config) {
        format!(" \u{26a0} reload failed: {}", e)
    } else if app.master_reloaded_at.is_some_and(|t| t.elapsed().as_secs() < 3) {
        " \u{2713} reloaded".to_string()
    } else {
        String::new()
    }
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
            let not_core: usize = mc.settings.iter().map(ConfigSetting::ignored_keys).sum();
            let pending = mc.settings.iter().filter(|s| s.restart_pending(mc)).count();
            let rejected = mc.files.iter().filter(|file| file.error.is_some()).count();
            // Warnings first: on a narrow panel the counts are what gets cut.
            // Without the running master's options, ↻ only goes by file times.
            let by_time = if mc.running_compared { "" } else { " (file times only)" };
            format!(
                " Master Config{}{}{} \u{b7} {} set, {} module, {} not core, {} derived, {} default ",
                reload_status(app),
                if rejected > 0 { format!(" \u{26a0} {} file(s) rejected", rejected) } else { String::new() },
                if pending > 0 { format!(" \u{21bb} {} on restart{}", pending, by_time) } else { String::new() },
                count(SettingKind::Explicit), count(SettingKind::Module), not_core, count(SettingKind::Derived), count(SettingKind::Default),
            )
        }
        None => format!(" Master Config{} ", reload_status(app)),
    };
    let block = super::list_block_hinted(title, border_style, color::main_bg(), has_more_above, has_more_below, &hints, area.width);
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
/// file that sets it with the setting's block marked, its long lines
/// wrapped under their line number when `wrap`.
fn draw_setting_source(f: &mut Frame, area: Rect, setting: Option<(&MasterConfig, &ConfigSetting)>, wrap: bool) {
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
    let hints = [("w", if wrap { "no wrap" } else { "wrap" }.to_string())];
    let block = super::titled_block_hinted(title, Style::default().fg(color::border()), color::main_bg(), &hints);
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
            Some(meant) => note(format!("Nothing in Salt reads it \u{2014} did you mean {}?", meant), Style::default().fg(color::red())),
            None if !s.notes.is_empty() => note(s.notes.join("; "), Style::default().fg(color::yellow())),
            None => note("Nothing in Salt reads it: a typo, or an option of a module that isn't installed".to_string(), Style::default().fg(color::purple())),
        }),
        SettingKind::Module => {
            if let Some(about) = &s.about {
                lines.push(note(format!("Not salt-master's own: {}", about), Style::default().fg(color::cyan())));
            }
            for (key, suggest) in &s.unknown_nested {
                let hint = suggest.as_ref().map_or(String::new(), |k| format!(" \u{2014} did you mean {}?", k));
                lines.push(note(format!("{}: not a {} option, ignored{}", key, s.key, hint), Style::default().fg(color::red())));
            }
            for remark in &s.notes {
                lines.push(note(remark.clone(), Style::default().fg(color::yellow())));
            }
        }
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
    if !s.sources.is_empty() {
        match s.default.as_ref() {
            Some(d) if *d == s.full => lines.push(note("Same as Salt's default".to_string(), dim)),
            Some(d) if !d.contains('\n') => lines.push(note(format!("Salt's default: {}", d), Style::default().fg(color::yellow()))),
            Some(_) => lines.push(note("Differs from Salt's default (Enter shows it)".to_string(), Style::default().fg(color::yellow()))),
            None => {}
        }
    }
    if let Some((file, _, _)) = winner {
        if let Some(error) = &file.error {
            lines.push(note(format!("\u{26a0} Salt rejected this file: {}", error), Style::default().fg(color::red()).add_modifier(Modifier::BOLD)));
        }
    }
    let restart = Style::default().fg(color::yellow());
    match (s.restart, mc.running_compared) {
        (Some(state), _) => {
            lines.push(note(format!("\u{21bb} May change on restart: {}", state.describe()), restart));
            match s.running_full.as_ref().filter(|_| s.has_running_diff()) {
                // Up to half the panel, the rest stays the file's.
                Some(running) => lines.extend(changed_rows(&diff_lines(running, &s.full), if wrap { inner.width as usize } else { 0 }, (inner.height as usize / 2).max(4))),
                None => lines.push(note(format!("  running master: {}", s.running_shown()), restart)),
            }
        }
        (None, false) if s.restart_pending(mc) => lines.push(note(
            format!("\u{21bb} Set in a file changed after salt-master started: may change on restart (running master not asked: {})", mc.running_error.as_deref().unwrap_or("unknown")),
            restart,
        )),
        _ => {}
    }
    // Notes always wrap, hanging under their own text.
    let width = inner.width as usize;
    let mut lines: Vec<Line> = lines.into_iter().flat_map(|line| match line.spans.as_slice() {
        [only] if line.width() > width => {
            let text = only.content.trim_start();
            let lead = only.content.len() - text.len();
            super::whats_new::wrap(vec![(text.to_string(), only.style)], width, Span::raw(" ".repeat(lead)), &" ".repeat(lead + 2))
        }
        _ => vec![line],
    }).collect();
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
            let code_w = if wrap { width.saturating_sub(gutter + 2) } else { 0 };
            let end_row = lines.len() + room;
            for (n, line) in text.iter().enumerate().skip(first) {
                if lines.len() >= end_row {
                    break;
                }
                let number = n + 1;
                let inside = start > 0 && number >= start && number < end;
                let at_fault = number == file.error_line;
                let marker = Span::styled(if inside || at_fault { "\u{258c}" } else { " " }, Style::default().fg(if at_fault { color::red() } else { color::border_focus() }));
                let body = if line.trim_start().starts_with('#') { dim } else { Style::default().fg(color::main_fg()) };
                let rows = super::wrap_spans(vec![Span::styled(line.to_string(), if inside { body.bg(color::bar_bg()) } else { body })], code_w);
                for (i, row) in rows.into_iter().enumerate() {
                    let number = if i == 0 { format!("{:>w$} ", number, w = gutter) } else { " ".repeat(gutter + 1) };
                    lines.push(Line::from([vec![marker.clone(), Span::styled(number, dim)], row.spans].concat()));
                }
            }
            lines.truncate(end_row);
        }
        None => {
            let code_w = if wrap { width } else { 0 };
            if s.kind == SettingKind::Derived {
                lines.push(note("Value:".to_string(), label));
                lines.extend(s.full.lines().flat_map(|l| super::wrap_spans(vec![Span::styled(format!("   {}", l), Style::default().fg(color::main_fg()))], code_w)));
            }
            if let Some(default) = &s.default {
                lines.push(note("Salt's default:".to_string(), label));
                lines.extend(default.lines().flat_map(|l| super::wrap_spans(vec![Span::styled(format!("   {}", l), dim)], code_w)));
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

    #[test]
    fn diff_pairs_each_changed_line_with_the_one_it_replaced() {
        use DiffLine::*;
        let running = "base:\n- /srv/salt\n- /srv/formulas/nginx\n- /srv/users\ndev:\n- /srv/dev";
        let files = "base:\n- /srv/salt\n- /srv/formulas/nginx-v2\n- /srv/users\ndev:\n- /srv/dev\n- /srv/extra";
        assert_eq!(diff_lines(running, files), [
            Same("base:"),
            Same("- /srv/salt"),
            Running("- /srv/formulas/nginx", Some((21, 21))),
            Files("- /srv/formulas/nginx-v2", Some((21, 24))),
            Same("- /srv/users"),
            Same("dev:"),
            Same("- /srv/dev"),
            Files("- /srv/extra", None),
        ]);
        assert_eq!(diff_counts(&diff_lines(running, files)), (1, 2));
    }

    #[test]
    fn changed_part_stops_where_prefix_and_suffix_would_overlap() {
        assert_eq!(changed_part("aXa", "aXaXa"), Some(((3, 3), (3, 5))));
        assert_eq!(changed_part("n=3", "n=4"), Some(((2, 3), (2, 3))));
        assert_eq!(changed_part("abc", "xyz"), None);
        assert_eq!(changed_part("é1", "é2"), Some(((2, 3), (2, 3))));
    }

    #[test]
    fn changed_rows_mark_gaps_and_count_what_doesnt_fit() {
        let text = |rows: Vec<Line>| rows.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()).collect::<Vec<_>>();
        let diff = diff_lines("a\nb\nc\nd", "A\nb\nc\nD");
        assert_eq!(text(changed_rows(&diff, 40, 10)), [
            "  \u{2212} a", "  + A", "    \u{22ef}", "  \u{2212} d", "  + D",
            "  \u{2212} running master  + after restart \u{b7} Enter shows it in context",
        ]);
        assert_eq!(text(changed_rows(&diff, 40, 4)), ["  \u{2212} a", "  + A", "  \u{2026} 2 more changed lines; Enter shows the difference"]);
    }

    #[test]
    fn diff_of_unrelated_values_removes_then_adds() {
        use DiffLine::*;
        assert_eq!(diff_lines("a\nb", "c"), [Running("a", None), Files("c", None), Running("b", None)]);
    }
}
