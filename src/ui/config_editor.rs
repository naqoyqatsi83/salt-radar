use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
use super::color;
use super::detail_popup;

pub fn draw_config_editor(f: &mut Frame, app: &App) {
    let state = match app.detail_popup {
        Some(crate::app::DetailPopup::ConfigEditor(ref s)) => s,
        _ => return,
    };

    let area = detail_popup::centered_rect(68, 70, f.area());
    f.render_widget(Clear, area);

    let config_path = std::fs::canonicalize(&app.config_path)
        .unwrap_or_else(|_| app.config_path.clone())
        .display()
        .to_string();
    let editing_kind = if state.editing { state.fields.get(state.selected).map(|f| f.kind) } else { None };
    let hints: Vec<(&str, String)> = if let Some(ConfigFieldKind::Columns(_)) = editing_kind {
        vec![
            ("Esc", "cancel".to_string()),
            ("\u{2191}\u{2193}", "select".to_string()),
            ("+/-", "move column".to_string()),
            ("Space", "show/hide".to_string()),
            ("Enter", if state.quick { "save" } else { "confirm" }.to_string()),
        ]
    } else if let Some(ConfigFieldKind::Order) = editing_kind {
        vec![
            ("Esc", "cancel".to_string()),
            ("\u{2191}\u{2193}", "select".to_string()),
            ("+/-", "move item".to_string()),
            ("Enter", "confirm".to_string()),
        ]
    } else {
        vec![
            ("Esc/q", "close".to_string()),
            ("\u{2191}\u{2193}", "move".to_string()),
            ("PgUp/PgDn", "page".to_string()),
            ("Home/End", "top/bottom".to_string()),
            ("Enter", "edit".to_string()),
            ("s", "save".to_string()),
        ]
    };
    let title = if state.dirty() { " Config Editor \u{25CF} unsaved changes " } else { " Config Editor " };
    let block = super::titled_block(title, Style::default().fg(color::border_focus()), color::main_bg())
        .title_bottom(super::border_hints(&hints, color::border_focus()).left_aligned())
        .title_bottom(super::border_info(format!("saved to: {}", config_path), color::border_focus()).right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    let mut sel_line: usize = 0;

    // Label column width is derived from the longest label actually present
    // (Section/SaveButton rows don't use this two-column layout) instead of a
    // hardcoded width that several labels — e.g. service_mem_growth_mb_per_hour
    // at 31 chars — already ran past, jamming the value against the label
    // with no separating space.
    let label_w = state.fields.iter()
        .filter(|f| !matches!(f.kind, ConfigFieldKind::Section | ConfigFieldKind::SaveButton))
        .map(|f| f.label.len())
        .max()
        .unwrap_or(22)
        + 2;

    for (field_idx, field) in state.fields.iter().enumerate() {
        match field.kind {
            ConfigFieldKind::Section => {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("  {}", field.label),
                    Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD),
                )));
            }
            ConfigFieldKind::SaveButton => {
                let is_sel = field_idx == state.selected;
                let sel_style = if is_sel { color::select_bg() } else { Style::default().bg(color::main_bg()) };
                lines.push(Line::from(""));
                if is_sel { sel_line = lines.len(); }
                lines.push(Line::from(
                    Span::styled(
                        format!("  \u{25B8} Save & Restart"),
                        Style::default().fg(color::green()).add_modifier(Modifier::BOLD),
                    ),
                ).style(sel_style));
                if is_sel {
                    lines.push(Line::from(Span::styled(
                        "    'Enter' to save and restart",
                        Style::default().fg(color::inactive_fg()),
                    )));
                }
            }
            ConfigFieldKind::Choice(_) | ConfigFieldKind::Theme => {
                let options = state.options(field.kind).unwrap_or_default();
                let is_sel = field_idx == state.selected;
                let editing = is_sel && state.editing;
                let sel_style = if is_sel { color::select_bg() } else { Style::default().bg(color::main_bg()) };

                if is_sel { sel_line = lines.len(); }
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {:<label_w$}", field.label, label_w = label_w),
                        sel_style.fg(if is_sel { color::main_fg() } else { color::label_fg() }).add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() }),
                    ),
                    Span::styled(
                        field.label_value.clone(),
                        sel_style.fg(if editing { color::yellow() } else { color::main_fg() }),
                    ),
                ]));
                if editing {
                    for (i, opt) in options.iter().enumerate() {
                        let opt_sel = i == state.choice_idx;
                        if opt_sel { sel_line = lines.len(); }
                        let opt_style = if opt_sel { color::select_bg() } else { Style::default().bg(color::main_bg()) };
                        let marker = if opt_sel { "\u{25B8} " } else { "  " };
                        lines.push(Line::from(Span::styled(
                            format!("      {}{}", marker, opt),
                            opt_style.fg(if opt_sel { color::yellow() } else { color::inactive_fg() }).add_modifier(if opt_sel { Modifier::BOLD } else { Modifier::empty() }),
                        )));
                    }
                }
            }
            ConfigFieldKind::Columns(all) => {
                let is_sel = field_idx == state.selected;
                let editing = is_sel && state.editing;
                let sel_style = if is_sel { color::select_bg() } else { Style::default().bg(color::main_bg()) };

                if is_sel { sel_line = lines.len(); }
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {:<label_w$}", field.label, label_w = label_w),
                        sel_style.fg(if is_sel { color::main_fg() } else { color::label_fg() }).add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() }),
                    ),
                    Span::styled(
                        field.label_value.clone(),
                        sel_style.fg(if editing { color::yellow() } else { color::main_fg() }),
                    ),
                ]));
                if editing {
                    let (items, shown) = crate::app::column_editor_items(&field.label_value, all);
                    for (i, item) in items.iter().enumerate() {
                        let item_sel = i == state.choice_idx;
                        let item_style = if item_sel { color::select_bg() } else { Style::default().bg(color::main_bg()) };
                        let marker = if item_sel { "\u{25B8} " } else { "  " };
                        let check = if i < shown { "[x]" } else { "[ ]" };
                        let fg = if item_sel { color::yellow() } else if i < shown { color::main_fg() } else { color::inactive_fg() };
                        if item_sel { sel_line = lines.len(); }
                        lines.push(Line::from(Span::styled(
                            format!("      {}{} {}", marker, check, item),
                            item_style.fg(fg).add_modifier(if item_sel { Modifier::BOLD } else { Modifier::empty() }),
                        )));
                    }
                }
            }
            ConfigFieldKind::Order => {
                let is_sel = field_idx == state.selected;
                let editing = is_sel && state.editing;
                let sel_style = if is_sel { color::select_bg() } else { Style::default().bg(color::main_bg()) };

                if is_sel { sel_line = lines.len(); }
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {:<label_w$}", field.label, label_w = label_w),
                        sel_style.fg(if is_sel { color::main_fg() } else { color::label_fg() }).add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() }),
                    ),
                    Span::styled(
                        field.label_value.clone(),
                        sel_style.fg(if editing { color::yellow() } else { color::main_fg() }),
                    ),
                ]));
                if editing {
                    for (i, item) in field.label_value.split(',').enumerate() {
                        let item_sel = i == state.choice_idx;
                        let item_style = if item_sel { color::select_bg() } else { Style::default().bg(color::main_bg()) };
                        let marker = if item_sel { "\u{25B8} " } else { "  " };
                        lines.push(Line::from(Span::styled(
                            format!("      {}{}", marker, item),
                            item_style.fg(if item_sel { color::yellow() } else { color::inactive_fg() }).add_modifier(if item_sel { Modifier::BOLD } else { Modifier::empty() }),
                        )));
                    }
                }
            }
            _ => {
                let is_sel = field_idx == state.selected;
                let editing = is_sel && state.editing;
                let sel_style = if is_sel { color::select_bg() } else { Style::default().bg(color::main_bg()) };

                let display_val = if editing {
                    let cursor = if state.input.len() % 2 == 0 { "\u{2588}" } else { " " };
                    format!("{}{}", state.input, cursor)
                } else {
                    field.label_value.clone()
                };
                let missing = match field.kind {
                    ConfigFieldKind::Path(kind) if !editing => !kind.exists(&field.label_value),
                    _ => false,
                };

                if is_sel { sel_line = lines.len(); }
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {:<label_w$}", field.label, label_w = label_w),
                        sel_style.fg(if is_sel { color::main_fg() } else { color::label_fg() }).add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() }),
                    ),
                    Span::styled(
                        display_val,
                        sel_style.fg(if editing { color::yellow() } else { color::main_fg() }),
                    ),
                    Span::styled(
                        if missing { "  \u{2717} not found" } else { "" },
                        sel_style.fg(color::red()).add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
        }
    }

    if state.confirm_discard {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Unsaved changes \u{2014} 's' to save, Esc/q again to discard them",
            Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD),
        )));
        sel_line = lines.len() - 1;
    }

    if let Some(ref err) = state.save_error {
        lines.push(Line::from(Span::styled(
            format!("  Error: {}", err),
            Style::default().fg(color::red()).add_modifier(Modifier::BOLD),
        )));
    }

    let visible = inner.height as usize;
    let (scroll, _, _) = super::scroll_window(sel_line, visible, lines.len());

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::main_bg())).scroll((scroll as u16, 0)),
        inner,
    );

    if let Some(ref b) = state.browser {
        super::dim_background(f, app.config.popup_dim_pct);
        draw_path_browser(f, b, state.fields.get(state.selected).map_or("", |f| f.label));
    }
}

fn draw_path_browser(f: &mut Frame, b: &PathBrowser, label: &str) {
    let area = detail_popup::centered_rect(60, 70, f.area());
    f.render_widget(Clear, area);
    let what = match b.kind { PathKind::File => "file", PathKind::Dir => "directory" };
    let hints: Vec<(&str, String)> = vec![
        ("Enter", if b.kind == PathKind::File { "open/pick" } else { "open" }.to_string()),
        ("\u{2190}", "up".to_string()),
        (".", if b.show_hidden { "hide hidden" } else { "show hidden" }.to_string()),
        ("e", "type path".to_string()),
        ("Esc", "cancel".to_string()),
    ];
    let block = super::titled_block(format!(" {}: select a {} ", label, what), Style::default().fg(color::border_focus()), color::popup_bg())
        .title_bottom(super::border_hints(&hints, color::border_focus()).left_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![
        Line::from(Span::styled(format!("  {}", b.dir.display()), Style::default().fg(color::cyan()).add_modifier(Modifier::BOLD))),
        Line::from(""),
    ];
    let header = lines.len();
    for (i, entry) in b.entries.iter().enumerate() {
        let (text, fg) = match entry {
            BrowserEntry::SelectHere => ("  \u{2713} select this directory".to_string(), color::green()),
            BrowserEntry::Parent => ("  ../".to_string(), color::inactive_fg()),
            BrowserEntry::Dir(n) => (format!("  {}/", n), color::cyan()),
            BrowserEntry::File(n) => (format!("  {}", n), color::main_fg()),
        };
        let line = Line::from(Span::styled(text, Style::default().fg(fg)));
        lines.push(if i == b.selected { color::selected_row(line) } else { line });
    }
    if b.entries.iter().all(|e| matches!(e, BrowserEntry::SelectHere | BrowserEntry::Parent)) {
        lines.push(Line::from(Span::styled("  (empty)", Style::default().fg(color::inactive_fg()))));
    }
    let visible = inner.height as usize;
    let (scroll, _, _) = super::scroll_window(header + b.selected, visible, lines.len());
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::popup_bg())).scroll((scroll as u16, 0)),
        inner,
    );
}

// Re-export ConfigFieldKind from the app module for use here
pub use crate::app::{BrowserEntry, ConfigFieldKind, PathBrowser, PathKind};
