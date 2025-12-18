use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use chrono::{DateTime, Local};

use crate::{
    config::{DEFAULT_MAX_AGE, TIMELINE_BINS, TIMELINE_WINDOW},
    filters::{Filters, InputMode},
    ingest::{drain_ingest, Ingest},
    log_entry::{Level, LogEntry},
    timeline::Timeline,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Live,
    Paused,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Live => "LIVE",
            Mode::Paused => "PAUSED",
        }
    }

    pub fn color(self) -> ratatui::style::Color {
        match self {
            Mode::Live => ratatui::style::Color::Green,
            Mode::Paused => ratatui::style::Color::Yellow,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Bookmark {
    pub timestamp: DateTime<Local>,
    pub label: String,
}

pub struct App {
    pub mode: Mode,
    logs: VecDeque<LogEntry>,
    max_lines: usize,
    max_age: Duration,
    scroll_offset: usize,
    selected_from_end: usize,
    paused_head_len: Option<usize>,
    paused_buffer: Vec<LogEntry>,
    filters: Filters,
    filter_error: Option<String>,
    input_mode: InputMode,
    bookmarks: Vec<Bookmark>,
    ingest: Ingest,
    timeline: Timeline,
    source_label: String,
    timeline_cursor_from_end: Option<usize>,
    pub show_help: bool,
    last_tick: Instant,
    last_notice: Option<String>,
}

impl App {
    pub fn new(ingest: Ingest, max_lines: usize, source_label: String) -> Self {
        Self {
            mode: Mode::Live,
            logs: VecDeque::with_capacity(max_lines),
            max_lines,
            max_age: DEFAULT_MAX_AGE,
            scroll_offset: 0,
            selected_from_end: 0,
            paused_head_len: None,
            paused_buffer: Vec::new(),
            filters: Filters::default(),
            filter_error: None,
            input_mode: InputMode::Normal,
            bookmarks: Vec::new(),
            ingest,
            timeline: Timeline::new(TIMELINE_BINS, TIMELINE_WINDOW),
            source_label,
            timeline_cursor_from_end: None,
            show_help: false,
            last_tick: Instant::now(),
            last_notice: None,
        }
    }

    pub fn tick(&mut self) {
        let now = Local::now();
        let new_entries = drain_ingest(&mut self.ingest);
        let mut info = 0;
        let mut warn = 0;
        let mut error = 0;
        for entry in new_entries {
            match entry.level {
                Level::Info => info += 1,
                Level::Warn => warn += 1,
                Level::Error => error += 1,
            }
            if matches!(self.mode, Mode::Paused) {
                self.paused_buffer.push(entry);
            } else {
                self.push_log(entry);
            }
        }
        self.timeline.record(now, info, warn, error);
        if matches!(self.mode, Mode::Paused) {
            self.last_tick = Instant::now();
            return;
        }
        self.flush_pending();
        self.prune(now);
        self.scroll_offset = 0;
        self.selected_from_end = 0;
        self.paused_head_len = None;
        self.timeline_cursor_from_end = None;
        self.clamp_selection();
        self.last_tick = Instant::now();
    }

    pub fn last_tick(&self) -> Instant {
        self.last_tick
    }

    pub fn toggle_pause(&mut self) {
        match self.mode {
            Mode::Live => {
                self.mode = Mode::Paused;
                self.paused_head_len = Some(self.logs.len());
            }
            Mode::Paused => {
                self.mode = Mode::Live;
                self.flush_pending();
                self.scroll_offset = 0;
                self.selected_from_end = 0;
                self.paused_head_len = None;
                self.timeline_cursor_from_end = None;
            }
        };
    }

    pub fn go_live(&mut self) {
        self.mode = Mode::Live;
        self.flush_pending();
        self.scroll_offset = 0;
        self.selected_from_end = 0;
        self.paused_head_len = None;
        self.timeline_cursor_from_end = None;
    }

    pub fn scroll_up(&mut self, lines: usize) {
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

    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        self.selected_from_end = self.scroll_offset;
        if self.scroll_offset == 0 {
            self.mode = Mode::Live;
            self.timeline_cursor_from_end = None;
        }
    }

    pub fn move_timeline_cursor(&mut self, delta: i32) {
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

    pub fn toggle_level(&mut self, level: Level) {
        match level {
            Level::Info => self.filters.info = !self.filters.info,
            Level::Warn => self.filters.warn = !self.filters.warn,
            Level::Error => self.filters.error = !self.filters.error,
        }
        self.after_filter_change();
    }

    pub fn set_regex_mode(&mut self, enabled: bool) {
        self.filters.regex_mode = enabled;
        match self.filters.set_text(self.filters.text.clone()) {
            Ok(_) => self.filter_error = None,
            Err(err) => self.filter_error = Some(err.to_string()),
        }
        self.after_filter_change();
    }

    pub fn set_filter_text(&mut self, text: Option<String>) {
        match self.filters.set_text(text) {
            Ok(_) => self.filter_error = None,
            Err(err) => self.filter_error = Some(err.to_string()),
        }
        self.after_filter_change();
    }

    pub fn clear_filters(&mut self) {
        self.filters = Filters::default();
        self.filter_error = None;
        self.after_filter_change();
        self.last_notice = Some("Filters cleared".to_string());
    }

    pub fn jump_error(&mut self, direction: i32) {
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

    pub fn add_bookmark(&mut self) {
        if let Some(entry) = self.current_entry() {
            let label = format!("mark {}", self.bookmarks.len() + 1);
            self.bookmarks.push(Bookmark {
                timestamp: entry.timestamp,
                label,
            });
            self.bookmarks.sort_by_key(|b| b.timestamp);
            if let Some(last) = self.bookmarks.last() {
                self.last_notice = Some(format!(
                    "Added bookmark {} @ {}",
                    last.label, last.timestamp
                ));
            }
        }
    }

    pub fn jump_bookmark(&mut self, direction: i32) {
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
                self.last_notice = Some(format!("Jumped to {}", bm.label));
            }
        }
    }

    pub fn visible_logs(&self, max_visible: usize) -> Vec<(usize, &LogEntry)> {
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

    pub fn filtered_len(&self) -> usize {
        self.logs
            .iter()
            .filter(|entry| self.filters.matches(entry))
            .count()
    }

    pub fn selected_from_end(&self) -> usize {
        self.selected_from_end
    }

    pub fn timeline_cursor_from_end(&self) -> Option<usize> {
        self.timeline_cursor_from_end
    }

    pub fn filters(&self) -> &Filters {
        &self.filters
    }

    pub fn filter_error(&self) -> Option<&String> {
        self.filter_error.as_ref()
    }

    pub fn input_mode(&self) -> &InputMode {
        &self.input_mode
    }

    pub fn bookmarks(&self) -> &Vec<Bookmark> {
        &self.bookmarks
    }

    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    pub fn queued_len(&self) -> usize {
        self.paused_head_len
            .map(|head| {
                self.logs
                    .len()
                    .saturating_sub(head)
                    .saturating_add(self.paused_buffer.len())
            })
            .unwrap_or(0)
    }

    pub fn total_logs(&self) -> usize {
        self.logs.len()
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn source_label(&self) -> &str {
        &self.source_label
    }

    pub fn last_notice(&self) -> Option<&String> {
        self.last_notice.as_ref()
    }

    pub fn current_bookmark_position(&self) -> Option<(usize, &Bookmark)> {
        let entry_ts = self.current_entry()?.timestamp;
        let mut candidate: Option<(usize, &Bookmark)> = None;
        for (idx, bm) in self.bookmarks.iter().enumerate() {
            if bm.timestamp <= entry_ts {
                candidate = Some((idx, bm));
            } else {
                break;
            }
        }
        candidate.or_else(|| self.bookmarks.first().map(|bm| (0, bm)))
    }

    pub fn input_mode_mut(&mut self) -> &mut InputMode {
        &mut self.input_mode
    }

    pub fn set_input_mode(&mut self, mode: InputMode) {
        self.input_mode = mode;
    }

    fn filtered_indices(&self) -> Vec<usize> {
        self.logs
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.filters.matches(entry))
            .map(|(idx, _)| idx)
            .collect()
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

    fn current_entry(&self) -> Option<&LogEntry> {
        let filtered = self.filtered_indices();
        if filtered.is_empty() {
            return None;
        }
        let total = filtered.len();
        let target_idx = total.saturating_sub(self.selected_from_end + 1);
        filtered.get(target_idx).and_then(|idx| self.logs.get(*idx))
    }

    fn flush_pending(&mut self) {
        if self.paused_buffer.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.paused_buffer);
        for entry in pending {
            self.push_log(entry);
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
}
