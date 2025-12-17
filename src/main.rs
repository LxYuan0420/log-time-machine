use std::{
    collections::VecDeque,
    io,
    time::{Duration, Instant},
};

use anyhow::Result;
use chrono::{DateTime, Local};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use rand::{rngs::SmallRng, Rng, SeedableRng};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Sparkline},
    Terminal,
};

const TICK_RATE: Duration = Duration::from_millis(200);
const MAX_LOG_LINES: usize = 400;
const TIMELINE_BINS: usize = 80;

fn main() -> Result<()> {
    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal);
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Live,
    Paused,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Mode::Live => "LIVE",
            Mode::Paused => "PAUSED",
        }
    }

    fn color(self) -> Color {
        match self {
            Mode::Live => Color::Green,
            Mode::Paused => Color::Yellow,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }

    fn color(self) -> Color {
        match self {
            Level::Info => Color::White,
            Level::Warn => Color::Yellow,
            Level::Error => Color::Red,
        }
    }
}

#[derive(Debug, Clone)]
struct LogEntry {
    timestamp: DateTime<Local>,
    level: Level,
    target: &'static str,
    message: String,
}

impl LogEntry {
    fn to_list_item(&self) -> ListItem<'static> {
        let ts = self.timestamp.format("%H:%M:%S").to_string();
        let line = Line::from(vec![
            Span::styled(
                format!("{ts} "),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ),
            Span::styled(
                format!("{:5}", self.level.label()),
                Style::default()
                    .fg(self.level.color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:<7}", self.target),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(" "),
            Span::raw(self.message.clone()),
        ]);
        ListItem::new(line)
    }
}

struct App {
    mode: Mode,
    logs: VecDeque<LogEntry>,
    max_logs: usize,
    scroll_offset: usize,
    rng: SmallRng,
    timeline: VecDeque<u64>,
}

impl App {
    fn new() -> Self {
        Self {
            mode: Mode::Live,
            logs: VecDeque::with_capacity(MAX_LOG_LINES),
            max_logs: MAX_LOG_LINES,
            scroll_offset: 0,
            rng: SmallRng::seed_from_u64(42),
            timeline: VecDeque::from(vec![0; TIMELINE_BINS]),
        }
    }

    fn on_tick(&mut self) {
        let new_lines = self.generate_fake_logs();
        self.push_timeline(new_lines as u64);
        if matches!(self.mode, Mode::Live) {
            self.scroll_offset = 0;
        }
    }

    fn generate_fake_logs(&mut self) -> usize {
        let new_lines = self.rng.gen_range(0..=3);
        for _ in 0..new_lines {
            let entry = self.fake_entry();
            self.push_log(entry);
        }
        new_lines
    }

    fn push_log(&mut self, entry: LogEntry) {
        if self.logs.len() >= self.max_logs {
            self.logs.pop_front();
        }
        self.logs.push_back(entry);
    }

    fn push_timeline(&mut self, value: u64) {
        if self.timeline.len() >= TIMELINE_BINS {
            self.timeline.pop_front();
        }
        self.timeline.push_back(value);
    }

    fn toggle_pause(&mut self) {
        self.mode = match self.mode {
            Mode::Live => Mode::Paused,
            Mode::Paused => Mode::Live,
        };
        if matches!(self.mode, Mode::Live) {
            self.scroll_offset = 0;
        }
    }

    fn go_live(&mut self) {
        self.mode = Mode::Live;
        self.scroll_offset = 0;
    }

    fn reset_scroll(&mut self) {
        self.scroll_offset = 0;
    }

    fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = (self.scroll_offset + lines).min(self.logs.len());
        self.mode = Mode::Paused;
    }

    fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        if self.scroll_offset == 0 && matches!(self.mode, Mode::Paused) {
            self.mode = Mode::Live;
        }
    }

    fn visible_logs(&self, max_visible: usize) -> Vec<&LogEntry> {
        if max_visible == 0 || self.logs.is_empty() {
            return Vec::new();
        }
        let total = self.logs.len();
        let clamped_offset = self.scroll_offset.min(total);
        let end = total.saturating_sub(clamped_offset);
        let start = end.saturating_sub(max_visible);
        self.logs.iter().skip(start).take(end - start).collect()
    }

    fn fake_entry(&mut self) -> LogEntry {
        let level_roll: u8 = self.rng.gen_range(0..100);
        let level = match level_roll {
            0..=65 => Level::Info,
            66..=88 => Level::Warn,
            _ => Level::Error,
        };
        let target = COMPONENTS[self.rng.gen_range(0..COMPONENTS.len())];
        let base_msg = match level {
            Level::Info => INFO_MESSAGES[self.rng.gen_range(0..INFO_MESSAGES.len())],
            Level::Warn => WARN_MESSAGES[self.rng.gen_range(0..WARN_MESSAGES.len())],
            Level::Error => ERROR_MESSAGES[self.rng.gen_range(0..ERROR_MESSAGES.len())],
        };
        let detail_id: u16 = self.rng.gen_range(1000..9999);
        let message = format!("{base_msg} target={target} req={detail_id}");
        LogEntry {
            timestamp: Local::now(),
            level,
            target,
            message,
        }
    }
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = App::new();
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|frame| draw_ui(frame, &app))?;

        let timeout = TICK_RATE
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Char('c')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            break Ok(())
                        }
                        KeyCode::Char('q') => break Ok(()),
                        KeyCode::Char(' ') => app.toggle_pause(),
                        KeyCode::Char('g') => app.go_live(),
                        KeyCode::Char('r') => {
                            app.reset_scroll();
                            app.go_live();
                        }
                        KeyCode::Up | KeyCode::Char('k') => app.scroll_up(1),
                        KeyCode::Down | KeyCode::Char('j') => app.scroll_down(1),
                        KeyCode::PageUp => app.scroll_up(8),
                        KeyCode::PageDown => app.scroll_down(8),
                        KeyCode::Home => app.scroll_up(app.logs.len()),
                        KeyCode::End => app.go_live(),
                        _ => {}
                    }
                }
            }
        }

        if last_tick.elapsed() >= TICK_RATE {
            app.on_tick();
            last_tick = Instant::now();
        }
    }
}

fn draw_ui(frame: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(frame.size());

    render_header(frame, chunks[0]);
    render_logs(frame, chunks[1], app);
    render_timeline(frame, chunks[2], app);
    render_status(frame, chunks[3], app);
}

fn render_header(frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
    let header = Paragraph::new(vec![
        Line::from("Log Time Machine (mock feed)"),
        Line::from("q: quit  space: pause/resume  arrows: scroll  g: go live"),
    ])
    .block(
        Block::default()
            .title("Overview")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(header, area);
}

fn render_logs(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &App) {
    let max_visible = area.height.saturating_sub(2) as usize;
    let visible_logs = app.visible_logs(max_visible);
    let items: Vec<ListItem> = visible_logs
        .into_iter()
        .map(|entry| entry.to_list_item())
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title("Logs (mock stream)")
            .borders(Borders::ALL),
    );
    frame.render_widget(list, area);
}

fn render_timeline(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &App) {
    let data: Vec<u64> = app.timeline.iter().copied().collect();
    let max_value = data.iter().copied().max().unwrap_or(1);
    let sparkline = Sparkline::default()
        .block(
            Block::default()
                .title("Activity timeline (new lines per tick)")
                .borders(Borders::ALL),
        )
        .data(&data)
        .max(max_value)
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(sparkline, area);
}

fn render_status(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &App) {
    let status_line = Line::from(vec![
        Span::styled(
            format!(" {} ", app.mode.label()),
            Style::default()
                .fg(app.mode.color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" · "),
        Span::styled(
            format!("logs buffered: {}", app.logs.len()),
            Style::default().fg(Color::Gray),
        ),
        Span::raw(" · "),
        Span::raw(format!("scroll offset: {}", app.scroll_offset)),
        Span::raw(" · keys: pgup/pgdn to scroll faster, r to reset"),
    ]);

    let status = Paragraph::new(status_line).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(status, area);
}

const COMPONENTS: &[&str] = &["http", "db", "cache", "worker", "auth", "search"];
const INFO_MESSAGES: &[&str] = &[
    "GET /health 200",
    "job completed successfully",
    "cache warm completed",
    "user session refreshed",
    "metrics flushed",
];
const WARN_MESSAGES: &[&str] = &[
    "cache miss rate spiked",
    "retrying request",
    "slow query detected",
    "upstream took too long",
    "backoff applied",
];
const ERROR_MESSAGES: &[&str] = &[
    "database transaction deadlock",
    "timeout talking to upstream",
    "panic in worker thread",
    "failed to commit offset",
    "permission denied accessing key",
];
