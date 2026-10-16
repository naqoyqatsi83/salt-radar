use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::types::{CacheJobEntry, Column, HeaderHit, JobEntry, Tab, TableId};

use super::color;
use super::table::{cell, cell_with_suffix, fitted_widths, header_line};

pub fn draw_jobs(f: &mut Frame, area: Rect, app: &App, tab_focused: bool, hits: &mut Vec<HeaderHit>) {
    let layout = super::split_panes(area, app);

    draw_live_jobs_panel(f, layout[0], app, tab_focused, hits);
    draw_cache_panel(f, layout[1], app, tab_focused, hits);
}

fn args_cell(args: &str, test_run: bool, w: usize) -> Vec<Span<'static>> {
    cell_with_suffix(args, test_marker(args, test_run), w, Style::default().fg(color::main_fg()), Style::default().fg(color::inactive_fg()))
}

fn test_marker(args: &str, test_run: bool) -> &'static str {
    if !test_run { "" } else if args.is_empty() { "(test)" } else { " (test)" }
}

/// The longest value of each column sized to its content, over every job
/// listed (`len` gives one job's, `None` for a fixed-width column).
fn longest_values<J>(cols: &[Column], jobs: &[J], len: impl Fn(&J, Column) -> Option<usize>) -> Vec<Option<usize>> {
    cols.iter().map(|&col| jobs.first().and_then(|j| len(j, col)).map(|_| jobs.iter().filter_map(|j| len(j, col)).max().unwrap_or(0))).collect()
}

fn args_len(args: &str, test_run: bool) -> usize {
    args.chars().count() + test_marker(args, test_run).len()
}

fn live_len(job: &&JobEntry, col: Column) -> Option<usize> {
    let digits = |n: usize| n.to_string().len();
    // Runner and wheel jobs show `?` for their counts.
    let count = |n: usize| if job.target_count == 0 { 1 } else { digits(n) };
    Some(match col {
        Column::Target => job.tgt.chars().count(),
        Column::Fun => job.fun.chars().count(),
        Column::Args => args_len(&job.args, job.test_run),
        Column::Source => if job.local { 5 } else { 6 },
        Column::Targeted => count(job.target_count),
        Column::Ok => count(job.succeeded),
        Column::Fail => count(job.failed),
        Column::Wait => count(job.target_count.saturating_sub(job.returned)),
        Column::Started => format_started(job.created_epoch).chars().count(),
        _ => return None,
    })
}

fn cache_len(job: &CacheJobEntry, col: Column) -> Option<usize> {
    Some(match col {
        Column::Target => job.tgt.chars().count(),
        Column::Fun => job.fun.chars().count(),
        Column::Args => args_len(&job.args, job.test_run),
        Column::User => job.user.chars().count(),
        Column::Started => format_started(job.start_epoch).chars().count(),
        _ => return None,
    })
}

fn format_started(epoch: f64) -> String {
    if epoch > 0.0 {
        chrono::DateTime::from_timestamp(epoch as i64, 0)
            .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "?".to_string())
    } else {
        "?".to_string()
    }
}

fn live_cells(job: &JobEntry, col: Column, w: usize) -> Vec<Span<'static>> {
    let right = col.right_aligned();
    let dim = Style::default().fg(color::inactive_fg());
    let count = |n: usize, hot: Style| {
        // Runner and wheel jobs target no minions, so their counts don't apply.
        if job.target_count == 0 {
            cell("?", w, right, dim)
        } else {
            cell(&n.to_string(), w, right, if n > 0 { hot } else { dim })
        }
    };
    let span = match col {
        Column::Jid => cell(&job.jid, w, right, dim),
        Column::Fun => cell(&job.fun, w, right, Style::default().fg(color::cyan())),
        Column::Args => return args_cell(&job.args, job.test_run, w),
        Column::Target => cell(&job.tgt, w, right, Style::default().fg(color::yellow())),
        Column::Source => {
            if job.local {
                cell("local", w, right, Style::default().fg(color::purple()))
            } else {
                cell("master", w, right, Style::default().fg(color::green()))
            }
        }
        Column::Targeted => count(job.target_count, Style::default().fg(color::main_fg())),
        Column::Ok => count(job.succeeded, Style::default().fg(color::outcome_ok()).add_modifier(Modifier::BOLD)),
        Column::Fail => count(job.failed, Style::default().fg(color::outcome_failed()).add_modifier(Modifier::BOLD)),
        Column::Wait => count(job.target_count.saturating_sub(job.returned), Style::default().fg(color::outcome_missing()).add_modifier(Modifier::BOLD)),
        Column::Started => cell(&format_started(job.created_epoch), w, right, dim),
        _ => cell("", w, right, dim),
    };
    vec![span]
}

fn cache_cells(job: &CacheJobEntry, col: Column, w: usize) -> Vec<Span<'static>> {
    let right = col.right_aligned();
    let dim = Style::default().fg(color::inactive_fg());
    let span = match col {
        Column::Jid => cell(&job.jid, w, right, dim),
        Column::User => cell(&job.user, w, right, Style::default().fg(color::purple())),
        Column::Started => cell(&format_started(job.start_epoch), w, right, dim),
        Column::Target => cell(&job.tgt, w, right, Style::default().fg(color::yellow())),
        Column::Fun => cell(&job.fun, w, right, Style::default().fg(color::cyan())),
        Column::Args => return args_cell(&job.args, job.test_run, w),
        _ => cell("", w, right, dim),
    };
    vec![span]
}

fn panel_hints(table: TableId, focused: bool, extra: &[(&'static str, &'static str)], arrow: (&'static str, &'static str)) -> Vec<(&'static str, String)> {
    let mut hints: Vec<(&str, String)> = extra.iter().map(|(k, v)| (*k, v.to_string())).collect();
    hints.extend(super::table::hints(table));
    if focused {
        hints.push((arrow.0, arrow.1.to_string()));
    }
    hints
}

fn draw_live_jobs_panel(f: &mut Frame, area: Rect, app: &App, tab_focused: bool, hits: &mut Vec<HeaderHit>) {
    let focused = tab_focused && !app.cache_focus;
    let border_style = if focused {
        Style::default().fg(color::border_focus())
    } else {
        Style::default().fg(color::border())
    };

    // Filtered once per frame, not once per visible row (#104).
    let filtered = app.filtered_jobs();
    let total = filtered.len();
    let selected = app.job_selected.min(total.saturating_sub(1));
    let visible = (area.height as usize).saturating_sub(2).saturating_sub(2);
    let (scroll, has_more_above, has_more_below) = super::scroll_window(selected, visible, total);

    let hints = panel_hints(TableId::LiveJobs, focused, &[
        ("/", "filter"),
        ("x", if app.jobs_failed_only { "all" } else { "failed only" }),
        ("Enter", "detail"),
        ("PgUp/PgDn", "page"),
        ("Home/End", "top/bottom"),
    ], ("\u{2192}", "cache"));
    let block = super::list_block_hinted(if app.jobs_failed_only { format!(" Live Jobs (failed only: {}/{}) ", total, app.jobs.len()) } else { format!(" Live Jobs ({}) ", app.jobs.len()) }, border_style, color::main_bg(), has_more_above, has_more_below, &hints, area.width);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // Filter bar
    let (filter, count) = if app.is_filtering {
        (app.filter_text.as_str(), format!("{} items", total))
    } else {
        (app.filter_text_for(Tab::Jobs), format!("{} items", total))
    };
    if let Some(bar) = super::filter_bar(filter, app.is_filtering, app.input_cursor, count, true) {
        lines.push(bar);
    }

    if app.jobs.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No active jobs.",
            Style::default().fg(color::inactive_fg()),
        )));
    } else {
        let cols = app.columns(TableId::LiveJobs);
        let widths = fitted_widths(&cols, (inner.width as usize).saturating_sub(2), &longest_values(&cols, &filtered, live_len));
        lines.push(header_line(TableId::LiveJobs, &cols, &widths, app.sort_of(TableId::LiveJobs), inner.x, inner.y + lines.len() as u16, hits));

        if total == 0 {
            lines.push(Line::from(Span::styled(
                "  No jobs match the filter.",
                Style::default().fg(color::inactive_fg()),
            )));
        } else {
            for (idx, &job) in filtered.iter().enumerate().skip(scroll).take(visible) {
                let mut row = vec![Span::raw("  ")];
                for (&col, &w) in cols.iter().zip(&widths) {
                    row.extend(live_cells(job, col, w));
                }
                let line = Line::from(row);
                lines.push(if idx == selected { color::selected_row(line) } else { line });
            }
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::main_bg())),
        inner,
    );
}

fn draw_cache_panel(f: &mut Frame, area: Rect, app: &App, tab_focused: bool, hits: &mut Vec<HeaderHit>) {
    let focused = tab_focused && app.cache_focus;
    let border_style = if focused {
        Style::default().fg(color::border_focus())
    } else {
        Style::default().fg(color::border())
    };

    let total_hint = if app.cache_loaded {
        let page = if app.cache_total_count > 0 {
            (app.cache_offset / app.cache_max_results) + 1
        } else {
            0
        };
        format!("{} found (pg {}, limit {})", app.cache_total_count, page, app.cache_max_results)
    } else if app.cache_loading {
        match app.cache_load_progress {
            Some(loaded) => format!("loading... {}", loaded),
            None => "loading...".to_string(),
        }
    } else {
        format!("{}", app.jobs_cache.len())
    };
    let total = app.filtered_cache_len();
    let selected = app.cache_selected.min(total.saturating_sub(1));
    let visible = (area.height as usize).saturating_sub(2).saturating_sub(2);
    let (scroll, has_more_above, has_more_below) = super::scroll_window(selected, visible, total);

    let hints = panel_hints(TableId::JobCache, focused, &[
        ("/", "filter"),
        ("Enter", "detail"),
        ("PgUp/PgDn", "scroll"),
        ("Home/End", "top/bottom"),
        ("[ ]", "limit"),
        (", .", "page"),
        ("r", "refresh"),
    ], ("\u{2190}", "live"));
    let block = super::list_block_hinted(format!(" Job Cache ({}) ", total_hint), border_style, color::main_bg(), has_more_above, has_more_below, &hints, area.width);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // Loading state — a live "N jobs read" count from the agent's
    // incremental progress messages, where available (the fast local_cache
    // walk reports it; the salt-run fallback for other job-cache backends
    // doesn't, so this just falls back to a plain "Loading..." line).
    if app.cache_loading && !app.cache_loaded {
        let elapsed = app.cache_loading_at.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0);
        let mut spans = vec![Span::styled("  Loading job cache... ", Style::default().fg(color::inactive_fg()))];
        if let Some(loaded) = app.cache_load_progress {
            spans.push(Span::styled(format!("{}", loaded), Style::default().fg(color::title()).add_modifier(Modifier::BOLD)));
            spans.push(Span::styled(" jobs read", Style::default().fg(color::inactive_fg())));
        }
        spans.push(Span::styled(format!(" ({:.0}s)", elapsed), Style::default().fg(color::inactive_fg())));
        lines.push(Line::from(spans));
        f.render_widget(
            Paragraph::new(lines).style(Style::default().bg(color::main_bg())),
            inner,
        );
        return;
    }

    // Error state
    if let Some(ref err) = app.cache_load_error {
        lines.push(Line::from(Span::styled(
            format!("  Error: {}", err),
            Style::default().fg(color::red()),
        )));
        f.render_widget(
            Paragraph::new(lines).style(Style::default().bg(color::main_bg())),
            inner,
        );
        return;
    }

    // Filter bar with total_count context
    let (filter, count) = if app.cache_is_filtering {
        (&app.cache_filter_text, format!("{} items", total))
    } else {
        (&app.cache_server_filter, format!("{} items (of {} total)", total, app.cache_total_count))
    };
    if let Some(bar) = super::filter_bar(filter, app.cache_is_filtering, app.input_cursor, count, false) {
        lines.push(bar);
    }

    if app.jobs_cache.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No cached jobs.",
            Style::default().fg(color::inactive_fg()),
        )));
    } else {
        let cols = app.columns(TableId::JobCache);
        let cached: Vec<&CacheJobEntry> = app.jobs_cache.iter().collect();
        let widths = fitted_widths(&cols, (inner.width as usize).saturating_sub(2), &longest_values(&cols, &cached, |j, col| cache_len(j, col)));
        lines.push(header_line(TableId::JobCache, &cols, &widths, app.sort_of(TableId::JobCache), inner.x, inner.y + lines.len() as u16, hits));

        if total == 0 {
            lines.push(Line::from(Span::styled(
                "  No cached jobs match the filter.",
                Style::default().fg(color::inactive_fg()),
            )));
        } else {
            for idx in scroll..(scroll + visible).min(total) {
                let job = match app.get_filtered_cache_job(idx) {
                    Some(j) => j,
                    None => break,
                };
                let mut row = vec![Span::raw("  ")];
                for (&col, &w) in cols.iter().zip(&widths) {
                    row.extend(cache_cells(job, col, w));
                }
                let line = Line::from(row);
                lines.push(if idx == selected { color::selected_row(line) } else { line });
            }
        }
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(color::main_bg())),
        inner,
    );
}
