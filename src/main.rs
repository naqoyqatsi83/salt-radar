#![allow(dead_code)]

mod agent;
mod alerts;
mod app;
mod config;
mod collectors;
mod sniff;
mod types;
mod ui;

use std::io;
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
    use std::io::Write;
    io::stdout().write_all(b"\x1b[?1000h\x1b[?1006h")?;
    io::stdout().flush()
}

fn disable_mouse_capture() -> io::Result<()> {
    use std::io::Write;
    io::stdout().write_all(b"\x1b[?1006l\x1b[?1000l")?;
    io::stdout().flush()
}

const MAX_INPUT_EVENTS_PER_FRAME: usize = 64;

fn main() -> io::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    // Enable mouse capture for click/scroll support
    enable_mouse_capture()?;

    let mut app = App::new();

    // Draw first frame immediately
    terminal.draw(|f| {
        app.last_frame_size = f.area();
        ui::draw(f, &mut app);
    })?;

    let mut last_tick = Instant::now();
    let mut last_tab = app.current_tab;
    let mut need_redraw = true;

    'main: loop {
        if need_redraw {
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
            need_redraw = false;
        }

        let tick_rate = Duration::from_millis(app.tick_interval_ms);
        let remaining = tick_rate.checked_sub(last_tick.elapsed()).unwrap_or(Duration::ZERO);
        let timeout = if app.needs_fast_poll() {
            remaining.min(Duration::from_millis(100))
        } else {
            remaining
        };

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
                            if app.handle_key(code) {
                                break 'main;
                            }
                            need_redraw = true;
                        }
                    }
                    Event::Resize(_, _) => {
                        need_redraw = true;
                    }
                    Event::Mouse(mouse) if app.mouse_enabled => {
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
        }

        if app.needs_mouse_toggle {
            app.needs_mouse_toggle = false;
            if app.mouse_enabled {
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
