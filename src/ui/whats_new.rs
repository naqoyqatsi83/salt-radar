//! The help overlay's "What's new" page: CHANGELOG.md, embedded at build
//! time so packaged installs (which never see the GitHub release notes)
//! can read it, rendered for the terminal.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::color;

const CHANGELOG: &str = include_str!("../../CHANGELOG.md");

#[derive(Debug, PartialEq)]
enum Block {
    Release(String),
    Section(String),
    Bullet(String),
    Para(String),
}

/// Splits the changelog into blocks, joining hard-wrapped lines back into
/// their bullet or paragraph. The preamble before the first release and
/// releases with nothing in them (an empty `[Unreleased]`) are left out.
fn blocks(src: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut after_blank = true;
    for line in src.lines().map(str::trim_end) {
        if let Some(h) = line.strip_prefix("## ") {
            out.push(Block::Release(h.to_string()));
        } else if out.is_empty() {
        } else if let Some(h) = line.strip_prefix("### ") {
            out.push(Block::Section(h.to_string()));
        } else if let Some(b) = line.strip_prefix("- ") {
            out.push(Block::Bullet(b.to_string()));
        } else if !line.is_empty() {
            match out.last_mut() {
                Some(Block::Bullet(t) | Block::Para(t)) if !after_blank => {
                    t.push(' ');
                    t.push_str(line.trim());
                }
                _ => out.push(Block::Para(line.trim().to_string())),
            }
        }
        after_blank = line.is_empty();
    }
    let mut kept = Vec::with_capacity(out.len());
    let mut iter = out.into_iter().peekable();
    while let Some(b) = iter.next() {
        if matches!(b, Block::Release(_)) && matches!(iter.peek(), None | Some(Block::Release(_))) {
            continue;
        }
        kept.push(b);
    }
    kept
}

/// `[text](url)` -> `text`.
fn strip_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find("](").map(|i| open + i) else { break };
        let Some(end) = rest[close..].find(')').map(|i| close + i) else { break };
        out.push_str(&rest[..open]);
        out.push_str(&rest[open + 1..close]);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Inline markdown -> styled runs: `code` and **bold**.
fn inline(text: &str, base: Style) -> Vec<(String, Style)> {
    let text = strip_links(text);
    let code_style = Style::default().fg(color::cyan());
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut code, mut bold) = (false, false);
    let mut chars = text.chars().peekable();
    let style = |code: bool, bold: bool| {
        if code { code_style } else if bold { base.add_modifier(Modifier::BOLD) } else { base }
    };
    while let Some(c) = chars.next() {
        let toggle = if c == '`' {
            Some(true)
        } else if c == '*' && !code && chars.peek() == Some(&'*') {
            chars.next();
            Some(false)
        } else {
            None
        };
        match toggle {
            Some(is_code) => {
                if !cur.is_empty() {
                    out.push((std::mem::take(&mut cur), style(code, bold)));
                }
                if is_code { code = !code } else { bold = !bold }
            }
            None => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push((cur, style(code, bold)));
    }
    out
}

/// Word-wraps styled runs to `width`, the first line after `first` and the
/// rest after `indent` (as wide as `first`), so bullets hang.
pub(super) fn wrap(runs: Vec<(String, Style)>, width: usize, first: Span<'static>, indent: &str) -> Vec<Line<'static>> {
    // Words, each a group of runs with no space between them (`code`, etc.).
    let mut words: Vec<Vec<(String, Style)>> = Vec::new();
    let mut space_before = true;
    for (text, style) in runs {
        for (i, seg) in text.split(' ').enumerate() {
            if i > 0 {
                space_before = true;
            }
            if seg.is_empty() {
                continue;
            }
            match words.last_mut() {
                Some(w) if !space_before => w.push((seg.to_string(), style)),
                _ => words.push(vec![(seg.to_string(), style)]),
            }
            space_before = false;
        }
    }
    let avail = width.saturating_sub(indent.chars().count()).max(10);
    let mut lines = Vec::new();
    let mut cur: Vec<Span<'static>> = vec![first];
    let mut used = 0;
    for word in words {
        let w: usize = word.iter().map(|(t, _)| t.chars().count()).sum();
        if used > 0 && used + 1 + w > avail {
            lines.push(Line::from(std::mem::replace(&mut cur, vec![Span::raw(indent.to_string())])));
            used = 0;
        }
        if used > 0 {
            cur.push(Span::raw(" "));
            used += 1;
        }
        cur.extend(word.into_iter().map(|(t, s)| Span::styled(t, s)));
        used += w;
    }
    lines.push(Line::from(cur));
    lines
}

/// `[0.7.6] - 2026-10-10` -> (`0.7.6`, `2026-10-10`).
fn release_title(h: &str) -> (String, String) {
    let (name, date) = h.split_once(" - ").unwrap_or((h, ""));
    (name.trim_matches(['[', ']']).to_string(), date.trim().to_string())
}

pub fn lines(width: usize) -> Vec<Line<'static>> {
    let text = Style::default().fg(color::main_fg());
    let mut out = vec![
        Line::from(Span::styled("  What's new", Style::default().fg(color::title()).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("  \u{2500}".to_string() + &"\u{2500}".repeat(15), Style::default().fg(color::div_line()))),
    ];
    for block in blocks(CHANGELOG) {
        match block {
            Block::Release(h) => {
                let (name, date) = release_title(&h);
                out.push(Line::from(""));
                let mut spans = vec![
                    Span::styled(format!("  {name}"), Style::default().fg(color::cyan()).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("  {date}"), Style::default().fg(color::inactive_fg())),
                ];
                if name == env!("CARGO_PKG_VERSION") {
                    spans.push(Span::styled("  (running)", Style::default().fg(color::green())));
                }
                out.push(Line::from(spans));
            }
            Block::Section(h) => {
                out.push(Line::from(Span::styled(format!("  {h}"), Style::default().fg(color::title()).add_modifier(Modifier::BOLD))));
            }
            Block::Bullet(t) => {
                let bullet = Span::styled("    \u{2022} ", Style::default().fg(color::label_fg()));
                out.extend(wrap(inline(&t, text), width, bullet, "      "));
            }
            Block::Para(t) => {
                out.extend(wrap(inline(&t, Style::default().fg(color::inactive_fg())), width, Span::raw("    "), "    "));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_join_wrapped_lines_and_skip_empty_releases() {
        let src = "# Changelog\n\nPreamble.\n\n## [Unreleased]\n\n## [1.0.0] - 2026-01-01\n\nIntro\ncontinued.\n\n### Added\n\n- One\n  more\n- Two\n";
        assert_eq!(blocks(src), vec![
            Block::Release("[1.0.0] - 2026-01-01".into()),
            Block::Para("Intro continued.".into()),
            Block::Section("Added".into()),
            Block::Bullet("One more".into()),
            Block::Bullet("Two".into()),
        ]);
    }

    #[test]
    fn inline_markup() {
        let base = Style::default();
        let runs = inline("Press `F1` in **Help**, see [docs](http://x)", base);
        let texts: Vec<&str> = runs.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(texts, ["Press ", "F1", " in ", "Help", ", see docs"]);
        assert!(runs[3].1.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn wrap_hangs_and_keeps_glued_runs_together() {
        let runs = vec![("aaa bbb ".to_string(), Style::default()), ("ccc".to_string(), Style::default()), (", ddd".to_string(), Style::default())];
        let plain: Vec<String> = wrap(runs, 12, Span::raw("- "), "  ")
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        assert_eq!(plain, ["- aaa bbb", "  ccc, ddd"]);
    }

    #[test]
    fn embedded_changelog_renders() {
        assert!(lines(80).len() > 20);
    }
}
