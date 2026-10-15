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

/// Cell widths for a table whose free-text columns are sized to what they
/// hold: `longest[i]` is the longest value in column `i`, `None` for a
/// fixed-width one. When everything fits, each column is as wide as its
/// content (or header), and the room left widens the gaps between columns
/// evenly, up to `MAX_EXTRA_GAP` each, the rest staying at the end of the
/// row, so a short column doesn't spread the table out and a wide panel
/// doesn't cram it. When it doesn't fit, free-text columns are trimmed down
/// to their minimum in `trim_order`.
pub fn fitted_widths(cols: &[Column], avail: usize, longest: &[Option<usize>]) -> Vec<usize> {
    // +1 for a sort marker after the header, +1 for the gap after the cell.
    let mut widths: Vec<usize> = cols.iter().zip(longest).map(|(c, longest)| match longest {
        Some(n) => (*n).max(c.header().chars().count() + 1) + 1,
        None => c.width(),
    }).collect();
    let mut used: usize = widths.iter().sum();
    while used > avail {
        let next = cols.iter().zip(&widths).enumerate()
            .filter(|(_, (c, w))| c.min_width().is_some_and(|m| **w > m))
            .min_by_key(|(_, (c, w))| (trim_order(**c, **w), std::cmp::Reverse(**w)))
            .map(|(i, _)| i);
        match next {
            Some(i) => { widths[i] -= 1; used -= 1; }
            None => break,
        }
    }
    if let Some(extra) = avail.saturating_sub(used).checked_div(cols.len()) {
        widths.iter_mut().for_each(|w| *w += extra.min(MAX_EXTRA_GAP));
    }
    widths
}

/// The most `fitted_widths` widens a gap between two columns by.
const MAX_EXTRA_GAP: usize = 3;

/// Which free-text columns `fitted_widths` trims first (lowest first):
/// what's least needed to tell rows apart goes first (OS), the minion ID
/// last.
fn trim_order(col: Column, _w: usize) -> u8 {
    match col {
        Column::Os => 0,
        Column::SaltVersion => 1,
        Column::Arch => 2,
        Column::MinionId => 4,
        _ => 3,
    }
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
                Style::default().fg(color::yellow()).add_modifier(Modifier::BOLD),
            ),
            _ => (col.header().to_string(), Style::default().fg(color::label_fg()).add_modifier(Modifier::BOLD)),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(table: TableId) -> Vec<Column> {
        Column::parse_list(table.default_columns(), table)
    }

    #[test]
    fn column_widths_fill_the_available_width() {
        for table in [TableId::LiveJobs, TableId::JobCache, TableId::Minions, TableId::Events, TableId::Alerts, TableId::ApiLog] {
            let c = cols(table);
            let natural: usize = c.iter().map(|c| c.width()).sum();
            for avail in [natural, natural + 37] {
                assert_eq!(column_widths(&c, avail).iter().sum::<usize>(), avail, "{:?} at {}", table, avail);
            }
        }
    }

    #[test]
    fn narrow_panels_shrink_text_columns_but_not_below_their_minimum() {
        let c = cols(TableId::JobCache);
        let natural: usize = c.iter().map(|c| c.width()).sum();
        let w = column_widths(&c, natural - 10);
        assert_eq!(w.iter().sum::<usize>(), natural - 10);
        for (col, width) in c.iter().zip(&w) {
            match col.min_width() {
                Some(min) => assert!(*width >= min),
                None => assert_eq!(*width, col.width(), "fixed-width columns never shrink"),
            }
        }
    }

    #[test]
    fn fitted_widths_follow_content_and_trim_the_id_last() {
        let c = [Column::MinionId, Column::KeyStatus, Column::Os, Column::SaltVersion, Column::Arch];
        // A 30-character FQDN, "Debian-12", "3006.16", AIX's "PowerPC_POWER10".
        let longest = [Some(30), None, Some(9), Some(7), Some(15)];
        // Room to spare: content widths, each gap 3 wider, the rest of the row left empty.
        assert_eq!(fitted_widths(&c, 200, &longest), vec![34, 13, 13, 11, 19]);
        // A little room: shared evenly.
        assert_eq!(fitted_widths(&c, 85, &longest), vec![33, 12, 12, 10, 18]);
        // Short values still fit their header and a sort marker.
        assert_eq!(fitted_widths(&c, 32, &[Some(2), None, Some(1), Some(1), Some(1)]), vec![4, 10, 4, 6, 6]);
        // Too narrow: OS, then Salt, then Arch give way, each down to its minimum.
        assert_eq!(fitted_widths(&c, 73, &longest), vec![31, 10, 8, 8, 16]);
        assert_eq!(fitted_widths(&c, 71, &longest), vec![31, 10, 8, 6, 16]);
        assert_eq!(fitted_widths(&c, 66, &longest), vec![31, 10, 8, 6, 11]);
        // Only then the ID.
        let tight = fitted_widths(&c, 50, &longest);
        assert_eq!(tight, vec![20, 10, 8, 6, 6]);
    }

    #[test]
    fn fit_and_pad_truncate_with_dots() {
        assert_eq!(fit("short", 10), "short");
        assert_eq!(fit("state.apply", 8), "state...");
        assert_eq!(fit("abc", 2), "ab");
        assert_eq!(pad("ok", 5, false), "ok   ");
        assert_eq!(pad("7", 4, true), "  7 ");
    }
}
