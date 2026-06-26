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

use crossterm::event::{self, Event, KeyEventKind, KeyCode, KeyModifiers};
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
                _ => {}
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

    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}
