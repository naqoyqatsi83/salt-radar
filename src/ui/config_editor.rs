use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
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

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color::BORDER))
        .title(" Config Editor ")
        .style(Style::default().bg(color::BG_DARK));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

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

                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {:<22}", field.label),
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

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::BG_DARK)),
        inner,
    );
}

// Re-export ConfigFieldKind from the app module for use here
pub use crate::app::ConfigFieldKind;
