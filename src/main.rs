mod agent;
mod alerts;
mod app;
mod config;
mod sniff;
mod types;
mod ui;

use std::cell::Cell;
use std::io::{self, Write};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind, KeyCode, KeyModifiers, MouseEventKind, MouseButton};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::Terminal;

use app::App;

// Mouse capture, restricted to just the modes salt-radar actually consumes.
//
// crossterm's own EnableMouseCapture also turns on button-drag (1002) and
// any-motion (1003) tracking, which report an escape sequence for *every*
// mouse movement, dragged or not -- salt-radar has no handler for Drag/Moved
// events at all (main's match falls through to `_ => {}` for them), so that
// traffic is pure waste sharing the same stdin stream as keyboard input.
// On a lossy/high-latency link (SSH, a terminal multiplexer, a KVM's input
// handoff -- see the tab-bar/mouse-click bugs already fixed here) more
// escape-sequence volume means more chances for a sequence to be split or
// corrupted, which can leak stray bytes into the keyboard-event path. Only
// requesting click tracking (1000) and extended SGR coordinates (1006) cuts
// that traffic to just what's needed.
fn enable_mouse_capture() -> io::Result<()> {
    io::stdout().write_all(b"\x1b[?1000h\x1b[?1006h")?;
    io::stdout().flush()
}

fn disable_mouse_capture() -> io::Result<()> {
    io::stdout().write_all(b"\x1b[?1006l\x1b[?1000l")?;
    io::stdout().flush()
}

const MAX_INPUT_EVENTS_PER_FRAME: usize = 64;

/// stdout that counts the bytes written through it, for `OutputBudget`.
struct CountingStdout {
    written: Rc<Cell<u64>>,
}

impl Write for CountingStdout {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = io::stdout().write(buf)?;
        self.written.set(self.written.get() + n as u64);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stdout().flush()
    }
}

/// Caps the bytes per second redraws send (`max_output_kbs`, 0 = no cap):
/// a token bucket that refills at the cap, holds up to two seconds' worth,
/// and goes negative after a large frame. A slow link (a session-recording
/// SSH proxy) that falls behind a busy master's scrolling lists stalls the
/// whole connection, the user's other terminal windows included.
struct OutputBudget {
    rate: f64,
    tokens: f64,
    at: Instant,
}

impl OutputBudget {
    fn new(now: Instant) -> Self {
        Self { rate: 0.0, tokens: 0.0, at: now }
    }

    fn refill(&mut self, kbs: u64, now: Instant) {
        self.rate = kbs as f64 * 1024.0;
        let secs = now.saturating_duration_since(self.at).as_secs_f64();
        self.tokens = (self.tokens + self.rate * secs).min(self.rate * 2.0);
        self.at = now;
    }

    fn ready(&self) -> bool {
        self.rate == 0.0 || self.tokens >= 0.0
    }

    fn spend(&mut self, bytes: u64) {
        if self.rate > 0.0 {
            self.tokens -= bytes as f64;
        }
    }

    /// How long until a held redraw may go out.
    fn wait(&self) -> Duration {
        if self.ready() { Duration::ZERO } else { Duration::from_secs_f64(-self.tokens / self.rate) }
    }
}

fn main() -> io::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let written = Rc::new(Cell::new(0u64));
    let backend = ratatui::backend::CrosstermBackend::new(CountingStdout { written: written.clone() });
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    if app.config.mouse {
        enable_mouse_capture()?;
    }

    // Draw first frame immediately
    terminal.draw(|f| {
        app.last_frame_size = f.area();
        ui::draw(f, &mut app);
    })?;

    let mut last_tick = Instant::now();
    let mut last_tab = app.current_tab;
    let mut need_redraw = true;
    // Input redraws at once; ticks and streams wait for the output budget.
    let mut input_redraw = false;
    let mut budget = OutputBudget::new(Instant::now());

    'main: loop {
        budget.refill(app.config.max_output_kbs, Instant::now());
        app.output_held = need_redraw && !input_redraw && !budget.ready();
        if need_redraw && !app.output_held {
            let before = written.get();
            if app.needs_terminal_clear {
                terminal.clear()?;
                app.needs_terminal_clear = false;
                last_tab = app.current_tab;
            } else if app.current_tab != last_tab {
                terminal.clear()?;
                last_tab = app.current_tab;
            }
            terminal.draw(|f| {
                app.last_frame_size = f.area();
                ui::draw(f, &mut app);
            })?;
            budget.spend(written.get() - before);
            need_redraw = false;
            input_redraw = false;
        }

        let tick_rate = Duration::from_millis(app.config.tick_ms);
        let remaining = tick_rate.checked_sub(last_tick.elapsed()).unwrap_or(Duration::ZERO);
        let mut timeout = if app.needs_fast_poll() {
            remaining.min(Duration::from_millis(100))
        } else {
            remaining
        };
        if app.output_held {
            timeout = timeout.min(budget.wait());
        }

        // Handle every input event already queued before redrawing: under a
        // busy event bus each redraw and drain is expensive, so paying for
        // one per key-repeat made input lag pile up (#104).
        if event::poll(timeout)? {
            let mut handled = 0;
            loop {
                match event::read()? {
                    Event::Key(key) => {
                        if key.kind == KeyEventKind::Press {
                            if key.modifiers.contains(KeyModifiers::CONTROL)
                                && (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('C'))
                            {
                                break 'main;
                            }
                            // Ctrl+H = ^H = backspace for terminals that send ^H instead of DEL
                            let code = if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('h') {
                                KeyCode::Backspace
                            } else {
                                key.code
                            };
                            let ctrl_arrow = key.modifiers.contains(KeyModifiers::CONTROL)
                                && matches!(key.code, KeyCode::Left | KeyCode::Right);
                            if !(ctrl_arrow && app.move_split(key.code == KeyCode::Right)) && app.handle_key(code) {
                                break 'main;
                            }
                            need_redraw = true;
                        }
                    }
                    Event::Resize(_, _) => {
                        need_redraw = true;
                    }
                    Event::Mouse(mouse) if app.config.mouse => {
                        match mouse.kind {
                            MouseEventKind::ScrollUp => {
                                if app.handle_key(KeyCode::Up) { break 'main; }
                                need_redraw = true;
                            }
                            MouseEventKind::ScrollDown => {
                                if app.handle_key(KeyCode::Down) { break 'main; }
                                need_redraw = true;
                            }
                            MouseEventKind::ScrollLeft => {
                                if app.handle_key(KeyCode::Left) { break 'main; }
                                need_redraw = true;
                            }
                            MouseEventKind::ScrollRight => {
                                if app.handle_key(KeyCode::Right) { break 'main; }
                                need_redraw = true;
                            }
                            MouseEventKind::Down(MouseButton::Left) => {
                                if app.handle_mouse_click(mouse.column, mouse.row, app.last_frame_size) {
                                    break 'main;
                                }
                                need_redraw = true;
                                // Click hit-testing reads the last drawn
                                // layout, so redraw before the next one.
                                break;
                            }
                            MouseEventKind::Down(MouseButton::Right) if app.quit_confirm => {
                                app.quit_confirm = false;
                                need_redraw = true;
                            }
                            MouseEventKind::Down(MouseButton::Right) if app.show_help => {
                                app.show_help = false;
                                need_redraw = true;
                            }
                            MouseEventKind::Down(MouseButton::Right) if app.config_editor.is_some() => {
                                if app.handle_key(KeyCode::Esc) { break 'main; }
                                need_redraw = true;
                            }
                            MouseEventKind::Down(MouseButton::Right) => {
                                app.detail_popup = None;
                                // A sniffer, if any, isn't stashed anywhere -- it
                                // lives in `app.sniffer` independent of
                                // `detail_popup` and reappears on its own.
                                if let Some((mid, sel)) = app.return_to_minion_events.take() {
                                    app.detail_popup = Some(crate::app::DetailPopup::MinionEvents {
                                        minion_id: mid, selected: sel,
                                    });
                                    app.popup_tab = Some(app.current_tab);
                                }
                                need_redraw = true;
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
                handled += 1;
                if handled >= MAX_INPUT_EVENTS_PER_FRAME || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
            input_redraw = need_redraw;
        }

        if app.needs_mouse_toggle {
            app.needs_mouse_toggle = false;
            if app.config.mouse {
                enable_mouse_capture()?;
            } else {
                disable_mouse_capture()?;
            }
        }

        // Fast poll: drain streaming buffers
        if app.fast_poll() {
            need_redraw = true;
        }

        if last_tick.elapsed() >= tick_rate {
            app.update();
            last_tick = Instant::now();
            need_redraw = true;
        }
    }

    app.cleanup();
    disable_raw_mode()?;
    disable_mouse_capture()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_budget_holds_redraws_until_the_cap_refills() {
        let t0 = Instant::now();
        let mut b = OutputBudget::new(t0);
        b.refill(0, t0);
        b.spend(1_000_000);
        assert!(b.ready(), "no cap never holds");

        b.refill(4, t0);
        assert!(b.ready());
        b.spend(6 * 1024);
        assert!(!b.ready(), "a frame over the budget holds the next one");
        assert_eq!(b.wait(), Duration::from_millis(1500), "6 KB at 4 KB/s");
        b.refill(4, t0 + Duration::from_millis(1500));
        assert!(b.ready());

        b.refill(4, t0 + Duration::from_secs(60));
        b.spend(9 * 1024);
        assert!(!b.ready(), "an idle minute saves up only two seconds' worth");
    }
}
