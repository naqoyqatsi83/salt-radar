//! Column layout shared by every list table whose columns are configurable
//! (see types::TableId): widths, padded cells, and a sortable header row
//! that records where each header was drawn for click-to-sort.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::types::{Column, HeaderHit, SortSpec, TableId};

use super::color;

/// Cell widths for a table `avail` characters wide (after the row indent).
/// Too narrow for every column's full width: the widest free-text column is
/// trimmed one step at a time (down to its minimum), so they share the
/// squeeze instead of the rightmost columns falling off the panel. Room to
/// spare: the widest free-text column (the rightmost on a tie) takes it, so
/// long descriptions/tags/args aren't cut short needlessly.
pub fn column_widths(cols: &[Column], avail: usize) -> Vec<usize> {
    let mut widths: Vec<usize> = cols.iter().map(|c| c.width()).collect();
    let mut used: usize = widths.iter().sum();
    while used > avail {
        let widest = cols.iter().zip(&widths).enumerate()
            .filter(|(_, (c, w))| c.min_width().is_some_and(|m| **w > m))
            .max_by_key(|(_, (_, w))| **w)
            .map(|(i, _)| i);
        match widest {
            Some(i) => { widths[i] -= 1; used -= 1; }
            None => break,
        }
    }
    if avail > used {
        let flex = cols.iter().enumerate()
            .filter(|(_, c)| c.min_width().is_some())
            .max_by_key(|(_, c)| c.width())
            .map(|(i, _)| i)
            .or_else(|| cols.iter().rposition(|c| !c.right_aligned()));
        if let Some(i) = flex {
            widths[i] += avail - used;
        }
    }
    widths
}

pub fn fit(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else if w <= 2 {
        s.chars().take(w).collect()
    } else {
        format!("{}..", s.chars().take(w - 2).collect::<String>())
    }
}

/// One cell's text, fitted and padded to `w` including the trailing gap.
pub fn pad(s: &str, w: usize, right: bool) -> String {
    let inner = w.saturating_sub(1);
    let t = fit(s, inner);
    if right { format!("{:>inner$} ", t) } else { format!("{:<inner$} ", t) }
}

pub fn cell(s: &str, w: usize, right: bool, style: Style) -> Span<'static> {
    Span::styled(pad(s, w, right), style)
}

/// A cell whose text is followed by a differently styled suffix (e.g. a
/// `(test)` marker or an `[n]` detail count) that is kept whole: the main
/// text is truncated first to make room for it.
pub fn cell_with_suffix(text: &str, suffix: &str, w: usize, style: Style, suffix_style: Style) -> Vec<Span<'static>> {
    let inner = w.saturating_sub(1);
    let suffix: String = suffix.chars().take(inner).collect();
    let suffix_len = suffix.chars().count();
    let text = fit(text, inner - suffix_len);
    let gap = inner - suffix_len - text.chars().count() + 1;
    vec![
        Span::styled(text, style),
        Span::styled(suffix, suffix_style),
        Span::raw(" ".repeat(gap)),
    ]
}

/// Header row (with the same two-column indent as data rows), also
/// recording each header's screen span into `hits`. `x`/`y` are the screen
/// position of the row's first character.
pub fn header_line(table: TableId, cols: &[Column], widths: &[usize], sort: Option<SortSpec>, x: u16, y: u16, hits: &mut Vec<HeaderHit>) -> Line<'static> {
    let mut spans = vec![Span::raw("  ")];
    let mut cx = x + 2;
    for (&col, &w) in cols.iter().zip(widths) {
        let (label, style) = match sort {
            Some((c, desc)) if c == col => (
                format!("{}{}", col.header(), if desc { "\u{25BC}" } else { "\u{25B2}" }),
                Style::default().fg(color::YELLOW).add_modifier(Modifier::BOLD),
            ),
            _ => (col.header().to_string(), Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
        };
        spans.push(cell(&label, w, col.right_aligned(), style));
        hits.push(HeaderHit { y, x0: cx, x1: cx.saturating_add(w as u16), col, table });
        cx = cx.saturating_add(w as u16);
    }
    Line::from(spans)
}

/// Border hints every configurable table shares.
pub fn hints(table: TableId) -> Vec<(&'static str, String)> {
    let mut hints = Vec::new();
    if table.sortable() {
        hints.push(("o/O", "sort".to_string()));
    }
    hints.push(("v", "columns".to_string()));
    hints
}
