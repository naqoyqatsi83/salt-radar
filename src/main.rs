#![allow(dead_code)]

mod agent;
mod alerts;
mod app;
mod config;
mod collectors;
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

fn main() -> io::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    // Enable mouse capture for click/scroll support
    use crossterm::event::{EnableMouseCapture, DisableMouseCapture};
    io::stdout().execute(EnableMouseCapture)?;

    let mut app = App::new();

    // Draw first frame immediately
    terminal.draw(|f| {
        app.last_frame_size = f.area();
        ui::draw(f, &mut app);
    })?;

    let mut last_tick = Instant::now();
    let mut last_tab = app.current_tab;
    let mut need_redraw = true;

    loop {
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

        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Press {
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('C'))
                        {
                            break;
                        }
                        // Ctrl+H = ^H = backspace for terminals that send ^H instead of DEL
                        let code = if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('h') {
                            KeyCode::Backspace
                        } else {
                            key.code
                        };
                        if app.handle_key(code) {
                            break;
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
                            if app.handle_key(KeyCode::Up) { break; }
                            need_redraw = true;
                        }
                        MouseEventKind::ScrollDown => {
                            if app.handle_key(KeyCode::Down) { break; }
                            need_redraw = true;
                        }
                        MouseEventKind::ScrollLeft => {
                            if app.handle_key(KeyCode::Left) { break; }
                            need_redraw = true;
                        }
                        MouseEventKind::ScrollRight => {
                            if app.handle_key(KeyCode::Right) { break; }
                            need_redraw = true;
                        }
                        MouseEventKind::Down(MouseButton::Left) => {
                            if app.handle_mouse_click(mouse.column, mouse.row, app.last_frame_size) {
                                break;
                            }
                            need_redraw = true;
                        }
                        MouseEventKind::Down(MouseButton::Right) => {
                            if app.detail_popup.is_some() && app.popup_tab == Some(app.current_tab) {
                                if !matches!(app.detail_popup, Some(crate::app::DetailPopup::AdminConfirm)) {
                                    app.detail_popup = None;
                                    need_redraw = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        if app.needs_mouse_toggle {
            app.needs_mouse_toggle = false;
            if app.mouse_enabled {
                io::stdout().execute(EnableMouseCapture)?;
            } else {
                io::stdout().execute(DisableMouseCapture)?;
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
    io::stdout().execute(DisableMouseCapture)?;
    io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}
