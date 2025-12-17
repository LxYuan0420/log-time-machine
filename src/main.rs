use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Seek},
    os::unix::fs::MetadataExt,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use clap::Parser;
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
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Sparkline},
    Terminal,
};
use regex::Regex;

const TICK_RATE: Duration = Duration::from_millis(200);
const DEFAULT_MAX_LINES: usize = 1200;
const DEFAULT_MAX_AGE: Duration = Duration::from_secs(20 * 60);
const TIMELINE_BINS: usize = 80;
const TIMELINE_WINDOW: Duration = Duration::from_secs(20 * 60);
const TAIL_SLEEP: Duration = Duration::from_millis(150);

#[derive(Parser, Debug)]
#[command(name = "log-time-machine")]
struct Args {
    /// Tail this file (fallback: mock feed)
    #[arg(long)]
    file: Option<PathBuf>,

    /// Read from stdin instead of a file
    #[arg(long)]
    stdin: bool,

    /// Maximum number of log lines to retain
    #[arg(long, default_value_t = DEFAULT_MAX_LINES)]
    max_lines: usize,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    target: String,
    message: String,
}

impl LogEntry {
    fn to_list_item(&self, selected: bool) -> ListItem<'static> {
        let ts = self.timestamp.format("%H:%M:%S").to_string();
        let mut spans = vec![
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
        ];
        if selected {
            for span in spans.iter_mut() {
                span.style = span.style.add_modifier(Modifier::REVERSED);
            }
        }
        ListItem::new(Line::from(spans))
    }
}

#[derive(Debug, Clone)]
struct Filters {
    info: bool,
    warn: bool,
    error: bool,
    text: Option<String>,
    regex_mode: bool,
    compiled: Option<Regex>,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            info: true,
            warn: true,
            error: true,
            text: None,
            regex_mode: false,
            compiled: None,
        }
    }
}

impl Filters {
    fn matches(&self, entry: &LogEntry) -> bool {
        let level_ok = match entry.level {
            Level::Info => self.info,
            Level::Warn => self.warn,
            Level::Error => self.error,
        };
        if !level_ok {
            return false;
        }
        if let Some(ref text) = self.text {
            if text.is_empty() {
                return true;
            }
            if self.regex_mode {
                if let Some(re) = &self.compiled {
                    return re.is_match(&entry.message);
                }
                return true;
            }
            entry.message.to_lowercase().contains(&text.to_lowercase())
                || entry.target.to_lowercase().contains(&text.to_lowercase())
        } else {
            true
        }
    }

    fn set_text(&mut self, text: Option<String>) -> Result<(), regex::Error> {
        self.text = text;
        if self.regex_mode {
            if let Some(t) = &self.text {
                if t.is_empty() {
                    self.compiled = None;
                } else {
                    self.compiled = Some(Regex::new(t)?);
                }
            }
        } else {
            self.compiled = None;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
enum InputMode {
    Normal,
    FilterText(String),
}

#[derive(Debug, Clone)]
struct Bookmark {
    timestamp: DateTime<Local>,
    label: String,
}

#[derive(Debug)]
enum Ingest {
    Mock(SmallRng),
    Channel(mpsc::Receiver<String>),
}

#[derive(Debug, Clone)]
struct Timeline {
    bins: VecDeque<u64>,
    bin_width: chrono::Duration,
    last_bin_start: DateTime<Local>,
}

impl Timeline {
    fn new(bin_count: usize, window: Duration) -> Self {
        let total_secs = window.as_secs().max(1);
        let bin_secs = (total_secs / bin_count.max(1) as u64).max(1);
        let bin_width = chrono::Duration::seconds(bin_secs as i64);
        let now = Local::now();
        Self {
            bins: VecDeque::from(vec![0; bin_count.max(1)]),
            bin_width,
            last_bin_start: now - bin_width,
        }
    }

    fn record(&mut self, now: DateTime<Local>, count: u64) {
        self.advance(now);
        if let Some(last) = self.bins.back_mut() {
            *last += count;
        }
    }

    fn advance(&mut self, now: DateTime<Local>) {
        if self.bins.is_empty() {
            return;
        }
        while now - self.last_bin_start >= self.bin_width {
            self.bins.pop_front();
            self.bins.push_back(0);
            self.last_bin_start += self.bin_width;
        }
    }

    fn data(&self) -> Vec<u64> {
        self.bins.iter().copied().collect()
    }

    fn range(&self) -> (DateTime<Local>, DateTime<Local>) {
        if self.bins.is_empty() {
            let now = Local::now();
            return (now, now);
        }
        let span = self.bin_width * (self.bins.len() as i32);
        let end = self.last_bin_start + self.bin_width;
        let start = end - span;
        (start, end)
    }

    fn len(&self) -> usize {
        self.bins.len()
    }

    fn bin_start(&self, idx_from_oldest: usize) -> DateTime<Local> {
        let (start, _) = self.range();
        start + self.bin_width * (idx_from_oldest as i32)
    }

    fn bin_index_for(&self, ts: DateTime<Local>) -> Option<usize> {
        let (start, end) = self.range();
        if ts < start || ts >= end {
            return None;
        }
        let offset = ts - start;
        let secs = offset.num_seconds();
        let bin_secs = self.bin_width.num_seconds().max(1);
        let idx = (secs / bin_secs) as usize;
        if idx < self.bins.len() {
            Some(idx)
        } else {
            None
        }
    }
}

#[derive(Debug)]
struct App {
    mode: Mode,
    logs: VecDeque<LogEntry>,
    max_lines: usize,
    max_age: Duration,
    scroll_offset: usize,
    selected_from_end: usize,
    paused_head_len: Option<usize>,
    filters: Filters,
    filter_error: Option<String>,
    input_mode: InputMode,
    bookmarks: Vec<Bookmark>,
    ingest: Ingest,
    timeline: Timeline,
    source_label: String,
    timeline_cursor_from_end: Option<usize>,
    diff_a: Option<DateTime<Local>>,
    diff_b: Option<DateTime<Local>>,
    show_help: bool,
}

impl App {
    fn new(ingest: Ingest, max_lines: usize, max_age: Duration, source_label: String) -> Self {
        Self {
            mode: Mode::Live,
            logs: VecDeque::with_capacity(max_lines),
            max_lines,
            max_age,
            scroll_offset: 0,
            selected_from_end: 0,
            paused_head_len: None,
            filters: Filters::default(),
            filter_error: None,
            input_mode: InputMode::Normal,
            bookmarks: Vec::new(),
            ingest,
            timeline: Timeline::new(TIMELINE_BINS, TIMELINE_WINDOW),
            source_label,
            timeline_cursor_from_end: None,
            diff_a: None,
            diff_b: None,
            show_help: false,
        }
    }

    fn on_tick(&mut self) {
        let now = Local::now();
        let (new_lines, entries) = match &mut self.ingest {
            Ingest::Mock(rng) => {
                let count = rng.gen_range(0..=3);
                let entries = (0..count).map(|_| fake_entry(rng)).collect::<Vec<_>>();
                (count, entries)
            }
            Ingest::Channel(rx) => {
                let mut entries = Vec::new();
                while let Ok(line) = rx.try_recv() {
                    entries.push(parse_line(&line));
                }
                let count = entries.len();
                (count, entries)
            }
        };

        for entry in entries {
            self.push_log(entry);
        }
        self.timeline.record(now, new_lines as u64);
        self.prune(now);
        if matches!(self.mode, Mode::Live) {
            self.scroll_offset = 0;
            self.selected_from_end = 0;
            self.paused_head_len = None;
            self.timeline_cursor_from_end = None;
        } else {
            self.clamp_selection();
        }
    }

    fn push_log(&mut self, entry: LogEntry) {
        if self.logs.len() >= self.max_lines {
            self.logs.pop_front();
        }
        self.logs.push_back(entry);
    }

    fn prune(&mut self, now: DateTime<Local>) {
        while let Some(front) = self.logs.front() {
            if now
                .signed_duration_since(front.timestamp)
                .to_std()
                .unwrap_or_default()
                > self.max_age
            {
                self.logs.pop_front();
            } else {
                break;
            }
        }
        while self.logs.len() > self.max_lines {
            self.logs.pop_front();
        }
    }

    fn filtered_indices(&self) -> Vec<usize> {
        self.logs
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.filters.matches(entry))
            .map(|(idx, _)| idx)
            .collect()
    }

    fn filtered_len(&self) -> usize {
        self.logs
            .iter()
            .filter(|entry| self.filters.matches(entry))
            .count()
    }

    fn visible_logs(&self, max_visible: usize) -> Vec<(usize, &LogEntry)> {
        if max_visible == 0 {
            return Vec::new();
        }
        let filtered = self.filtered_indices();
        if filtered.is_empty() {
            return Vec::new();
        }
        let total = filtered.len();
        let offset = self.scroll_offset.min(total.saturating_sub(1));
        let end = total.saturating_sub(offset);
        let start = end.saturating_sub(max_visible);
        filtered
            .into_iter()
            .enumerate()
            .skip(start)
            .take(end - start)
            .filter_map(|(filtered_idx, log_idx)| {
                self.logs.get(log_idx).map(|entry| (filtered_idx, entry))
            })
            .collect()
    }

    fn toggle_pause(&mut self) {
        match self.mode {
            Mode::Live => {
                self.mode = Mode::Paused;
                self.paused_head_len = Some(self.logs.len());
            }
            Mode::Paused => {
                self.mode = Mode::Live;
                self.scroll_offset = 0;
                self.selected_from_end = 0;
                self.paused_head_len = None;
            }
        };
    }

    fn go_live(&mut self) {
        self.mode = Mode::Live;
        self.scroll_offset = 0;
        self.selected_from_end = 0;
        self.paused_head_len = None;
        self.timeline_cursor_from_end = None;
    }

    fn scroll_up(&mut self, lines: usize) {
        let max_offset = self.filtered_len();
        self.scroll_offset = (self.scroll_offset + lines).min(max_offset);
        self.selected_from_end = self.scroll_offset;
        self.mode = Mode::Paused;
        self.timeline_cursor_from_end = None;
        if self.paused_head_len.is_none() {
            self.paused_head_len = Some(self.logs.len());
        }
        self.clamp_selection();
    }

    fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        self.selected_from_end = self.scroll_offset;
        if self.scroll_offset == 0 {
            self.mode = Mode::Live;
            self.timeline_cursor_from_end = None;
        }
    }

    fn clamp_selection(&mut self) {
        let filtered_len = self.filtered_len();
        if filtered_len == 0 {
            self.selected_from_end = 0;
            return;
        }
        if self.selected_from_end >= filtered_len {
            self.selected_from_end = filtered_len.saturating_sub(1);
        }
    }

    fn toggle_level(&mut self, level: Level) {
        match level {
            Level::Info => self.filters.info = !self.filters.info,
            Level::Warn => self.filters.warn = !self.filters.warn,
            Level::Error => self.filters.error = !self.filters.error,
        }
        self.after_filter_change();
    }

    fn set_regex_mode(&mut self, enabled: bool) {
        self.filters.regex_mode = enabled;
        match self.filters.set_text(self.filters.text.clone()) {
            Ok(_) => self.filter_error = None,
            Err(err) => self.filter_error = Some(err.to_string()),
        }
        self.after_filter_change();
    }

    fn set_filter_text(&mut self, text: Option<String>) {
        match self.filters.set_text(text) {
            Ok(_) => self.filter_error = None,
            Err(err) => self.filter_error = Some(err.to_string()),
        }
        self.after_filter_change();
    }

    fn clear_filters(&mut self) {
        self.filters = Filters::default();
        self.filter_error = None;
        self.after_filter_change();
    }

    fn after_filter_change(&mut self) {
        let filtered_len = self.filtered_len();
        if filtered_len == 0 {
            self.scroll_offset = 0;
            self.selected_from_end = 0;
            return;
        }
        if self.scroll_offset > filtered_len {
            self.scroll_offset = filtered_len;
        }
        if self.selected_from_end >= filtered_len {
            self.selected_from_end = filtered_len.saturating_sub(1);
        }
    }

    fn jump_error(&mut self, direction: i32) {
        let filtered_indices = self.filtered_indices();
        if filtered_indices.is_empty() {
            return;
        }
        let total = filtered_indices.len();
        let current = self.selected_from_end.min(total.saturating_sub(1));
        let current_idx = total.saturating_sub(current + 1);

        let target = if direction > 0 {
            filtered_indices
                .iter()
                .enumerate()
                .skip(current_idx + 1)
                .find(|(_, idx)| {
                    self.logs
                        .get(**idx)
                        .map(|e| e.level == Level::Error)
                        .unwrap_or(false)
                })
        } else {
            filtered_indices
                .iter()
                .enumerate()
                .take(current_idx)
                .rev()
                .find(|(_, idx)| {
                    self.logs
                        .get(**idx)
                        .map(|e| e.level == Level::Error)
                        .unwrap_or(false)
                })
        };

        if let Some((idx, _)) = target {
            let offset_from_end = total.saturating_sub(idx + 1);
            self.scroll_offset = offset_from_end;
            self.selected_from_end = offset_from_end;
            self.mode = Mode::Paused;
            if self.paused_head_len.is_none() {
                self.paused_head_len = Some(self.logs.len());
            }
        }
    }

    fn current_entry(&self) -> Option<&LogEntry> {
        let filtered = self.filtered_indices();
        if filtered.is_empty() {
            return None;
        }
        let total = filtered.len();
        let target_idx = total.saturating_sub(self.selected_from_end + 1);
        filtered.get(target_idx).and_then(|idx| self.logs.get(*idx))
    }

    fn add_bookmark(&mut self) {
        if let Some(entry) = self.current_entry() {
            let label = format!("mark {}", self.bookmarks.len() + 1);
            self.bookmarks.push(Bookmark {
                timestamp: entry.timestamp,
                label,
            });
            self.bookmarks.sort_by_key(|b| b.timestamp);
        }
    }

    fn jump_bookmark(&mut self, direction: i32) {
        if self.bookmarks.is_empty() || self.logs.is_empty() {
            return;
        }
        let filtered_indices = self.filtered_indices();
        if filtered_indices.is_empty() {
            return;
        }
        let current_ts = self
            .current_entry()
            .map(|e| e.timestamp)
            .unwrap_or_else(Local::now);
        let target = if direction > 0 {
            self.bookmarks
                .iter()
                .find(|b| b.timestamp > current_ts)
                .or_else(|| self.bookmarks.first())
        } else {
            self.bookmarks
                .iter()
                .rev()
                .find(|b| b.timestamp < current_ts)
                .or_else(|| self.bookmarks.last())
        };
        if let Some(bm) = target {
            if let Some((idx, _)) = filtered_indices.iter().enumerate().find(|(_, log_idx)| {
                self.logs
                    .get(**log_idx)
                    .map(|entry| entry.timestamp >= bm.timestamp)
                    .unwrap_or(false)
            }) {
                let offset_from_end = filtered_indices.len().saturating_sub(idx + 1);
                self.scroll_offset = offset_from_end;
                self.selected_from_end = offset_from_end;
                self.mode = Mode::Paused;
                if self.paused_head_len.is_none() {
                    self.paused_head_len = Some(self.logs.len());
                }
            }
        }
    }

    fn jump_spike(&mut self, direction: i32) {
        let data = self.timeline.data();
        if data.is_empty() {
            return;
        }
        let max = *data.iter().max().unwrap_or(&0);
        let threshold = std::cmp::max(1, (max as f64 * 0.5).ceil() as u64);
        let len = data.len();
        let current_cursor = self.timeline_cursor_from_end.unwrap_or(0);
        let mut idx_from_oldest = len.saturating_sub(current_cursor + 1);
        if direction > 0 {
            idx_from_oldest = ((idx_from_oldest + 1)..len)
                .find(|&i| data.get(i).copied().unwrap_or(0) >= threshold)
                .unwrap_or(idx_from_oldest);
        } else {
            idx_from_oldest = (0..idx_from_oldest)
                .rev()
                .find(|&i| data.get(i).copied().unwrap_or(0) >= threshold)
                .unwrap_or(idx_from_oldest);
        }
        let new_cursor_from_end = len.saturating_sub(idx_from_oldest + 1);
        self.timeline_cursor_from_end = Some(new_cursor_from_end);
        self.mode = Mode::Paused;
        if self.paused_head_len.is_none() {
            self.paused_head_len = Some(self.logs.len());
        }
        self.jump_to_timeline_cursor();
    }

    fn set_diff_a(&mut self) {
        if let Some(entry) = self.current_entry() {
            self.diff_a = Some(entry.timestamp);
        }
    }

    fn set_diff_b(&mut self) {
        if let Some(entry) = self.current_entry() {
            self.diff_b = Some(entry.timestamp);
        }
    }

    fn diff_summary(&self) -> Option<(usize, usize, usize)> {
        let (a, b) = match (self.diff_a, self.diff_b) {
            (Some(a), Some(b)) => (a.min(b), a.max(b)),
            _ => return None,
        };
        let mut info = 0;
        let mut warn = 0;
        let mut error = 0;
        for entry in self
            .logs
            .iter()
            .filter(|e| e.timestamp >= a && e.timestamp <= b)
        {
            if !self.filters.matches(entry) {
                continue;
            }
            match entry.level {
                Level::Info => info += 1,
                Level::Warn => warn += 1,
                Level::Error => error += 1,
            }
        }
        Some((info, warn, error))
    }

    fn move_timeline_cursor(&mut self, delta: i32) {
        let len = self.timeline.len();
        if len == 0 {
            return;
        }
        let current = self.timeline_cursor_from_end.unwrap_or(0) as i32;
        let max = (len as i32 - 1).max(0);
        let mut next = current.saturating_add(delta);
        if next < 0 {
            next = 0;
        }
        if next > max {
            next = max;
        }
        self.timeline_cursor_from_end = Some(next as usize);
        self.mode = Mode::Paused;
        if self.paused_head_len.is_none() {
            self.paused_head_len = Some(self.logs.len());
        }
        self.jump_to_timeline_cursor();
    }

    fn jump_to_timeline_cursor(&mut self) {
        let Some(cursor) = self.timeline_cursor_from_end else {
            return;
        };
        let len = self.timeline.len();
        if len == 0 {
            return;
        }
        let idx_from_oldest = len.saturating_sub(cursor + 1);
        let bin_start = self.timeline.bin_start(idx_from_oldest);
        let filtered_indices = self.filtered_indices();
        let filtered_len = filtered_indices.len();
        if filtered_len == 0 {
            self.scroll_offset = 0;
            self.selected_from_end = 0;
            return;
        }
        if let Some((idx_in_filtered, _)) =
            filtered_indices.iter().enumerate().find(|(_, log_idx)| {
                self.logs
                    .get(**log_idx)
                    .map(|entry| entry.timestamp >= bin_start)
                    .unwrap_or(false)
            })
        {
            self.scroll_offset = filtered_len.saturating_sub(idx_in_filtered + 1);
            self.selected_from_end = self.scroll_offset;
        } else {
            self.scroll_offset = filtered_len;
            self.selected_from_end = self.scroll_offset;
        }
        self.clamp_selection();
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let source = if args.stdin {
        SourceConfig::Stdin
    } else if let Some(file) = args.file {
        SourceConfig::File(file)
    } else {
        SourceConfig::Mock
    };

    let ingest = match source.clone() {
        SourceConfig::Mock => Ingest::Mock(SmallRng::seed_from_u64(42)),
        SourceConfig::Stdin => Ingest::Channel(spawn_stdin_reader()),
        SourceConfig::File(path) => Ingest::Channel(spawn_file_tail(path)),
    };

    let mut terminal = setup_terminal()?;
    let result = run_app(
        &mut terminal,
        ingest,
        args.max_lines,
        DEFAULT_MAX_AGE,
        source.label(),
    );
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

#[derive(Clone)]
enum SourceConfig {
    Mock,
    File(PathBuf),
    Stdin,
}

impl SourceConfig {
    fn label(&self) -> String {
        match self {
            SourceConfig::Mock => "mock feed".to_string(),
            SourceConfig::Stdin => "stdin".to_string(),
            SourceConfig::File(path) => format!("file: {}", path.display()),
        }
    }
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ingest: Ingest,
    max_lines: usize,
    max_age: Duration,
    source_label: String,
) -> Result<()> {
    let mut app = App::new(ingest, max_lines, max_age, source_label);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|frame| draw_ui(frame, &app))?;

        let timeout = TICK_RATE
            .checked_sub(last_tick.elapsed())
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

                    match &mut app.input_mode {
                        InputMode::Normal => match key.code {
                            KeyCode::Char('q') | KeyCode::Char('c')
                                if key.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                break Ok(())
                            }
                            KeyCode::Char('q') => break Ok(()),
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
                            KeyCode::Char('1') => app.toggle_level(Level::Info),
                            KeyCode::Char('2') => app.toggle_level(Level::Warn),
                            KeyCode::Char('3') => app.toggle_level(Level::Error),
                            KeyCode::Char('/') => {
                                let seed = app.filters.text.clone().unwrap_or_default();
                                app.input_mode = InputMode::FilterText(seed);
                            }
                            KeyCode::Char('F') => app.clear_filters(),
                            KeyCode::Char('R') => app.set_regex_mode(!app.filters.regex_mode),
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
                        },
                        InputMode::FilterText(buf) => match key.code {
                            KeyCode::Esc => app.input_mode = InputMode::Normal,
                            KeyCode::Enter => {
                                let text = if buf.is_empty() {
                                    None
                                } else {
                                    Some(buf.clone())
                                };
                                app.set_filter_text(text);
                                app.input_mode = InputMode::Normal;
                            }
                            KeyCode::Backspace => {
                                buf.pop();
                            }
                            KeyCode::Char(c) => {
                                buf.push(c);
                            }
                            _ => {}
                        },
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
            Constraint::Length(4),
            Constraint::Length(2),
        ])
        .split(frame.size());

    render_header(frame, chunks[0], app);
    render_logs(frame, chunks[1], app);
    render_timeline(frame, chunks[2], app);
    render_status(frame, chunks[3], app);

    if app.show_help {
        let area = centered_rect(70, 60, frame.size());
        frame.render_widget(Clear, area);
        let help = Paragraph::new(vec![
            Line::from("Keys:"),
            Line::from(" q/ctrl-c quit | space pause | g/end go live | arrows/pgup/pgdn scroll"),
            Line::from(" left/right timeline | s/S jump spikes"),
            Line::from(
                " / filter | R toggle regex | F clear | 1/2/3 toggle levels | n/p next/prev error",
            ),
            Line::from(" b add bookmark | ]/[ next/prev bookmark"),
            Line::from(" A/B set diff markers"),
            Line::from(""),
            Line::from("While scrolling up we auto-pause; queued lines show as +N."),
            Line::from("Timeline cursor moves with left/right; markers show bookmarks and cursor."),
            Line::from("Press ? or Esc to close this help."),
        ])
        .block(
            Block::default()
                .title("Help")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta)),
        );
        frame.render_widget(help, area);
    }
}

fn render_header(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &App) {
    let header = Paragraph::new(vec![
        Line::from(format!(
            "Source: {}   Mode: {}",
            app.source_label,
            app.mode.label()
        )),
        Line::from(
            "q: quit  space: pause/resume  arrows: scroll  left/right: timeline  g/end: go live  ?: help",
        ),
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

    let selected_idx_from_end = app.selected_from_end;
    let filtered_total = app.filtered_len();
    let items: Vec<ListItem> = visible_logs
        .into_iter()
        .map(|(filtered_idx, entry)| {
            let selected = filtered_total.saturating_sub(filtered_idx + 1) == selected_idx_from_end;
            entry.to_list_item(selected)
        })
        .collect();

    let list = List::new(items).block(Block::default().title("Logs").borders(Borders::ALL));
    frame.render_widget(list, area);
}

fn render_timeline(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &App) {
    let data: Vec<u64> = app.timeline.data();
    let max_value = data.iter().copied().max().unwrap_or(1);
    let (start, end) = app.timeline.range();
    let cursor_text = app.timeline_cursor_from_end.map(|cursor| {
        let len = app.timeline.len();
        let idx_from_oldest = len.saturating_sub(cursor + 1);
        let ts = app.timeline.bin_start(idx_from_oldest);
        format!("  cursor: {}", ts.format("%H:%M:%S"))
    });
    let title = format!(
        "Activity timeline ({} - {}){}",
        start.format("%H:%M:%S"),
        end.format("%H:%M:%S"),
        cursor_text.unwrap_or_default()
    );

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(1).max(1)),
            Constraint::Length(1),
        ])
        .split(area);

    let sparkline = Sparkline::default()
        .block(Block::default().title(title).borders(Borders::ALL))
        .data(&data)
        .max(max_value)
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(sparkline, parts[0]);

    let mut marks = vec!['.'; data.len()];
    if let Some(cursor) = app.timeline_cursor_from_end {
        let len = data.len();
        if len > 0 {
            let idx_from_oldest = len.saturating_sub(cursor + 1);
            if idx_from_oldest < marks.len() {
                marks[idx_from_oldest] = '^';
            }
        }
    }
    for bm in &app.bookmarks {
        if let Some(idx) = app.timeline.bin_index_for(bm.timestamp) {
            if let Some(slot) = marks.get_mut(idx) {
                *slot = if *slot == '^' { '#' } else { '*' };
            }
        }
    }
    if let Some(a) = app.diff_a {
        if let Some(idx) = app.timeline.bin_index_for(a) {
            if let Some(slot) = marks.get_mut(idx) {
                *slot = 'A';
            }
        }
    }
    if let Some(b) = app.diff_b {
        if let Some(idx) = app.timeline.bin_index_for(b) {
            if let Some(slot) = marks.get_mut(idx) {
                *slot = 'B';
            }
        }
    }

    let marker_str: String = marks.iter().collect();
    let trimmed = if marker_str.len() > parts[1].width as usize {
        marker_str
            .chars()
            .take(parts[1].width as usize)
            .collect::<String>()
    } else {
        marker_str
    };
    let markers = Paragraph::new(trimmed).block(
        Block::default()
            .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(markers, parts[1]);
}

fn render_status(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &App) {
    let queued = app
        .paused_head_len
        .map(|start| app.logs.len().saturating_sub(start))
        .unwrap_or(0);
    let levels = format!(
        "levels: {}{}{}",
        if app.filters.info { "I" } else { "i" },
        if app.filters.warn { "W" } else { "w" },
        if app.filters.error { "E" } else { "e" }
    );
    let filter_text = match &app.filters.text {
        Some(text) if !text.is_empty() => {
            if app.filters.regex_mode {
                format!("filter: /{text}/")
            } else {
                format!("filter: \"{text}\"")
            }
        }
        _ => "filter: none".to_string(),
    };
    let input_hint = match &app.input_mode {
        InputMode::FilterText(buf) => format!("typing filter: {buf}_"),
        InputMode::Normal => "".to_string(),
    };
    let bookmarks = format!("bookmarks: {}", app.bookmarks.len());

    let mut lines = vec![Line::from(vec![
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
        Span::raw(" · "),
        Span::raw(if app.scroll_offset > 0 {
            format!("INSPECTING -{}", app.scroll_offset)
        } else {
            "live view".to_string()
        }),
        Span::raw(" · "),
        Span::raw(format!("queued: +{queued}")),
        Span::raw(" · "),
        Span::raw(levels),
        Span::raw(" · "),
        Span::raw(filter_text),
        Span::raw(" · "),
        Span::raw(input_hint),
        Span::raw(" · "),
        Span::raw(bookmarks),
        Span::raw(" · "),
        Span::raw("keys: pgup/pgdn scroll, left/right timeline, / filter, F clear, R regex, b add mark, ]/[ jump mark, n/p error, s/S spike, A/B diff"),
    ])];

    if !app.bookmarks.is_empty() {
        let labels: Vec<String> = app.bookmarks.iter().map(|b| b.label.clone()).collect();
        lines.push(Line::from(format!("Bookmarks: {}", labels.join(", "))));
    }

    if let Some(err) = &app.filter_error {
        lines.push(Line::from(Span::styled(
            format!("filter error: {err}"),
            Style::default().fg(Color::Red),
        )));
    }

    if let (Some(a), Some(b)) = (app.diff_a, app.diff_b) {
        lines.push(Line::from(format!(
            "Diff A..B: A={}  B={}",
            a.format("%H:%M:%S"),
            b.format("%H:%M:%S")
        )));
        if let Some((info, warn, error)) = app.diff_summary() {
            lines.push(Line::from(format!(
                "Counts in range (filtered): info={info} warn={warn} error={error}"
            )));
        }
    }

    let status = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(status, area);
}

#[derive(Debug, Clone, Copy)]
struct FileId {
    dev: u64,
    ino: u64,
}

impl From<&File> for FileId {
    fn from(value: &File) -> Self {
        let meta = value.metadata().expect("metadata");
        FileId {
            dev: meta.dev(),
            ino: meta.ino(),
        }
    }
}

impl FileId {
    fn matches(&self, other: &File) -> bool {
        let other_id = FileId::from(other);
        self.dev == other_id.dev && self.ino == other_id.ino
    }
}

fn spawn_stdin_reader() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines().flatten() {
            let _ = tx.send(line);
        }
    });
    rx
}

fn spawn_file_tail(path: PathBuf) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || loop {
        match open_reader(&path) {
            Ok((mut reader, mut pos, file_id)) => loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        thread::sleep(TAIL_SLEEP);
                        if should_reopen(&path, pos, &file_id) {
                            break;
                        }
                    }
                    Ok(n) => {
                        pos += n as u64;
                        let trimmed = line.trim_end_matches(&['\n', '\r'][..]).to_string();
                        let _ = tx.send(trimmed);
                    }
                    Err(_) => {
                        break;
                    }
                }
            },
            Err(_) => {
                thread::sleep(TAIL_SLEEP);
            }
        }
    });
    rx
}

fn open_reader(path: &PathBuf) -> Result<(BufReader<File>, u64, FileId)> {
    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let file_id = FileId::from(&file);
    let mut reader = BufReader::new(file);
    let pos = reader
        .get_mut()
        .seek(io::SeekFrom::Start(0))
        .context("seek to start")?;
    Ok((reader, pos, file_id))
}

fn should_reopen(path: &PathBuf, pos: u64, file_id: &FileId) -> bool {
    if let Ok(file) = OpenOptions::new().read(true).open(path) {
        if !file_id.matches(&file) {
            return true;
        }
        if let Ok(meta) = file.metadata() {
            if meta.len() < pos {
                return true;
            }
        }
    } else {
        return true;
    }
    false
}

fn parse_line(line: &str) -> LogEntry {
    let mut parts = line.split_whitespace();
    let timestamp = parts
        .next()
        .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
        .map(|dt| dt.with_timezone(&Local))
        .unwrap_or_else(Local::now);

    let level = parts
        .next()
        .and_then(|lvl| match lvl.to_ascii_uppercase().as_str() {
            "INFO" => Some(Level::Info),
            "WARN" | "WARNING" => Some(Level::Warn),
            "ERROR" | "ERR" | "FATAL" => Some(Level::Error),
            _ => None,
        })
        .unwrap_or(Level::Info);

    let target = parts.next().unwrap_or("log").to_string();
    let message = parts.collect::<Vec<&str>>().join(" ");

    LogEntry {
        timestamp,
        level,
        target,
        message,
    }
}

fn fake_entry(rng: &mut SmallRng) -> LogEntry {
    let level_roll: u8 = rng.gen_range(0..100);
    let level = match level_roll {
        0..=65 => Level::Info,
        66..=88 => Level::Warn,
        _ => Level::Error,
    };
    let target = COMPONENTS[rng.gen_range(0..COMPONENTS.len())];
    let base_msg = match level {
        Level::Info => INFO_MESSAGES[rng.gen_range(0..INFO_MESSAGES.len())],
        Level::Warn => WARN_MESSAGES[rng.gen_range(0..WARN_MESSAGES.len())],
        Level::Error => ERROR_MESSAGES[rng.gen_range(0..ERROR_MESSAGES.len())],
    };
    let detail_id: u16 = rng.gen_range(1000..9999);
    let message = format!("{base_msg} target={target} req={detail_id}");
    LogEntry {
        timestamp: Local::now(),
        level,
        target: target.to_string(),
        message,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_understands_timestamp_and_level() {
        let entry = parse_line("2024-12-17T12:00:00Z ERROR db deadlock retry txn=7 attempt=1");
        assert_eq!(entry.level, Level::Error);
        assert_eq!(entry.target, "db");
        assert!(entry.message.contains("deadlock"));
    }

    #[test]
    fn filters_support_regex() {
        let entry = LogEntry {
            timestamp: Local::now(),
            level: Level::Error,
            target: "db".to_string(),
            message: "deadlock retry txn=7 attempt=1".to_string(),
        };
        let mut filters = Filters::default();
        filters.regex_mode = true;
        filters
            .set_text(Some("deadlock.*txn=7".to_string()))
            .unwrap();
        assert!(filters.matches(&entry));
    }

    #[test]
    fn timeline_tracks_bins() {
        let mut timeline = Timeline::new(5, Duration::from_secs(5));
        let now = Local::now();
        timeline.record(now, 3);
        timeline.record(now + chrono::Duration::seconds(6), 2);
        assert_eq!(timeline.data().len(), 5);
        assert!(timeline.data().iter().any(|v| *v >= 2));
    }
}

fn centered_rect(
    percent_x: u16,
    percent_y: u16,
    r: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
