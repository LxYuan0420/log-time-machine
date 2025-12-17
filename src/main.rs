mod app;
mod config;
mod filters;
mod ingest;
mod log_entry;
mod timeline;
mod ui;

use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use crate::config::{Args, SourceConfig, TICK_RATE};

fn main() -> Result<()> {
    let args = Args::parse();
    let source = if args.stdin {
        SourceConfig::Stdin
    } else if let Some(file) = args.file {
        SourceConfig::File(file)
    } else {
        SourceConfig::Mock
    };

    let ingest = ingest::Ingest::new(source.clone());
    let mut app = app::App::new(ingest, args.max_lines, source.label());

    let mut terminal = ui::setup_terminal()?;
    let result = run(&mut terminal, &mut app);
    ui::restore_terminal(&mut terminal)?;
    result
}

fn run(terminal: &mut ui::Term, app: &mut app::App) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

        let timeout = TICK_RATE
            .checked_sub(app.last_tick().elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if app.show_help {
                        match key.code {
                            KeyCode::Char('?') | KeyCode::Esc => app.show_help = false,
                            _ => {}
                        }
                        continue;
                    }
                    let mode_snapshot = app.input_mode().clone();
                    match mode_snapshot {
                        filters::InputMode::Normal => {
                            if handle_normal_key(app, key)? {
                                break Ok(());
                            }
                        }
                        filters::InputMode::FilterText(_) => handle_filter_key(app, key),
                    }
                }
            }
        }

        if app.last_tick().elapsed() >= TICK_RATE {
            app.tick();
        }
    }
}

fn handle_normal_key(app: &mut app::App, key: crossterm::event::KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('c')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            return Ok(true)
        }
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Char(' ') => app.toggle_pause(),
        KeyCode::Char('g') => app.go_live(),
        KeyCode::Char('r') => app.go_live(),
        KeyCode::Up | KeyCode::Char('k') => app.scroll_up(1),
        KeyCode::Down | KeyCode::Char('j') => app.scroll_down(1),
        KeyCode::PageUp => app.scroll_up(8),
        KeyCode::PageDown => app.scroll_down(8),
        KeyCode::Home => app.scroll_up(app.filtered_len()),
        KeyCode::End => app.go_live(),
        KeyCode::Left => app.move_timeline_cursor(1),
        KeyCode::Right => app.move_timeline_cursor(-1),
        KeyCode::Char('1') => app.toggle_level(log_entry::Level::Info),
        KeyCode::Char('2') => app.toggle_level(log_entry::Level::Warn),
        KeyCode::Char('3') => app.toggle_level(log_entry::Level::Error),
        KeyCode::Char('/') => {
            let seed = app.filters().text.clone().unwrap_or_default();
            app.set_input_mode(filters::InputMode::FilterText(seed));
        }
        KeyCode::Char('F') => app.clear_filters(),
        KeyCode::Char('R') => app.set_regex_mode(!app.filters().regex_mode),
        KeyCode::Char('n') => app.jump_error(1),
        KeyCode::Char('p') => app.jump_error(-1),
        KeyCode::Char('b') => app.add_bookmark(),
        KeyCode::Char(']') => app.jump_bookmark(1),
        KeyCode::Char('[') => app.jump_bookmark(-1),
        KeyCode::Char('s') => app.jump_spike(1),
        KeyCode::Char('S') => app.jump_spike(-1),
        KeyCode::Char('A') => app.set_diff_a(),
        KeyCode::Char('B') => app.set_diff_b(),
        KeyCode::Char('?') => app.show_help = !app.show_help,
        _ => {}
    }
    Ok(false)
}

fn handle_filter_key(app: &mut app::App, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Esc => app.set_input_mode(filters::InputMode::Normal),
        KeyCode::Enter => {
            let mode = std::mem::replace(app.input_mode_mut(), filters::InputMode::Normal);
            if let filters::InputMode::FilterText(buf) = mode {
                let text = if buf.is_empty() { None } else { Some(buf) };
                app.set_filter_text(text);
            }
        }
        KeyCode::Backspace => {
            if let filters::InputMode::FilterText(buf) = app.input_mode_mut() {
                buf.pop();
            }
        }
        KeyCode::Char(c) => {
            if let filters::InputMode::FilterText(buf) = app.input_mode_mut() {
                buf.push(c);
            }
        }
        _ => {}
    }
}
