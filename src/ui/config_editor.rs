use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, ConfigEditorState};
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
    let block = super::titled_block(" Config Editor ", Style::default().fg(color::BORDER), color::BG_DARK)
        .title_bottom(super::border_info(format!("saved to: {}", config_path)).left_aligned());
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
                    Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD),
                )));
            }
            ConfigFieldKind::SaveButton => {
                let is_sel = field_idx == state.selected;
                let sel_style = if is_sel { color::select_bg(app.high_contrast) } else { Style::default().bg(color::BG_DARK) };
                lines.push(Line::from(""));
                if is_sel { sel_line = lines.len(); }
                lines.push(Line::from(
                    Span::styled(
                        format!("  \u{25B8} Save & Restart"),
                        Style::default().fg(color::GREEN).add_modifier(Modifier::BOLD),
                    ),
                ).style(sel_style));
                if is_sel {
                    lines.push(Line::from(Span::styled(
                        "    Enter to save and restart",
                        Style::default().fg(color::TEXT_DIM),
                    )));
                }
            }
            ConfigFieldKind::Choice(options) => {
                let is_sel = field_idx == state.selected;
                let editing = is_sel && state.editing;
                let sel_style = if is_sel { color::select_bg(app.high_contrast) } else { Style::default().bg(color::BG_DARK) };

                if is_sel { sel_line = lines.len(); }
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {:<label_w$}", field.label, label_w = label_w),
                        sel_style.fg(if is_sel { color::TEXT } else { color::TEXT_LABEL }).add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() }),
                    ),
                    Span::styled(
                        field.label_value.clone(),
                        sel_style.fg(if editing { color::YELLOW } else { color::TEXT }),
                    ),
                ]));
                if editing {
                    for (i, opt) in options.iter().enumerate() {
                        let opt_sel = i == state.choice_idx;
                        let opt_style = if opt_sel { color::select_bg(app.high_contrast) } else { Style::default().bg(color::BG_DARK) };
                        let marker = if opt_sel { "\u{25B8} " } else { "  " };
                        lines.push(Line::from(Span::styled(
                            format!("      {}{}", marker, opt),
                            opt_style.fg(if opt_sel { color::YELLOW } else { color::TEXT_DIM }).add_modifier(if opt_sel { Modifier::BOLD } else { Modifier::empty() }),
                        )));
                    }
                }
            }
            _ => {
                let is_sel = field_idx == state.selected;
                let editing = is_sel && state.editing;
                let sel_style = if is_sel { color::select_bg(app.high_contrast) } else { Style::default().bg(color::BG_DARK) };

                let display_val = if editing {
                    let cursor = if state.input.len() % 2 == 0 { "\u{2588}" } else { " " };
                    format!("{}{}", state.input, cursor)
                } else {
                    field.label_value.clone()
                };

                if is_sel { sel_line = lines.len(); }
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {:<label_w$}", field.label, label_w = label_w),
                        sel_style.fg(if is_sel { color::TEXT } else { color::TEXT_LABEL }).add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() }),
                    ),
                    Span::styled(
                        display_val,
                        sel_style.fg(if editing { color::YELLOW } else { color::TEXT }),
                    ),
                ]));
            }
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Esc to close  \u{b7} \u{2191}\u{2193} to move  \u{b7} Enter to edit  \u{b7} Save & Restart to apply",
        Style::default().fg(color::TEXT_DIM),
    )));

    if let Some(ref err) = state.save_error {
        lines.push(Line::from(Span::styled(
            format!("  Error: {}", err),
            Style::default().fg(color::RED).add_modifier(Modifier::BOLD),
        )));
    }

    let visible = inner.height as usize;
    let (scroll, _, _) = super::scroll_window(sel_line, visible, lines.len());

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)).scroll((scroll as u16, 0)),
        inner,
    );
}

// Re-export ConfigFieldKind from the app module for use here
pub use crate::app::ConfigFieldKind;
