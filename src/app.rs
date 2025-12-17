use std::{
    collections::{HashMap, VecDeque},
    fs,
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

pub struct DiffStats {
    pub total: usize,
    pub info: usize,
    pub warn: usize,
    pub error: usize,
    pub top_targets: Vec<(String, usize)>,
}

pub struct App {
    pub mode: Mode,
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
            self.push_log(entry);
        }
        self.timeline.record(now, info, warn, error);
        self.prune(now);
        if matches!(self.mode, Mode::Live) {
            self.scroll_offset = 0;
            self.selected_from_end = 0;
            self.paused_head_len = None;
            self.timeline_cursor_from_end = None;
        } else {
            if let Some(prev) = self.paused_head_len {
                let added = self.logs.len().saturating_sub(prev);
                if added > 0 {
                    self.scroll_offset += added;
                    self.selected_from_end += added;
                    self.paused_head_len = Some(self.logs.len());
                }
            }
            self.clamp_selection();
        }
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
                self.scroll_offset = 0;
                self.selected_from_end = 0;
                self.paused_head_len = None;
                self.timeline_cursor_from_end = None;
            }
        };
    }

    pub fn go_live(&mut self) {
        self.mode = Mode::Live;
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
            }
        }
    }

    pub fn jump_spike(&mut self, direction: i32) {
        let data = self.timeline.data();
        if data.is_empty() {
            return;
        }
        let max = data
            .iter()
            .map(|b| b.info + b.warn + b.error)
            .max()
            .unwrap_or(0);
        let threshold = std::cmp::max(1, (max as f64 * 0.5).ceil() as u64);
        let len = data.len();
        let current_cursor = self.timeline_cursor_from_end.unwrap_or(0);
        let mut idx_from_oldest = len.saturating_sub(current_cursor + 1);
        if direction > 0 {
            idx_from_oldest = ((idx_from_oldest + 1)..len)
                .find(|&i| {
                    data.get(i)
                        .map(|b| b.info + b.warn + b.error >= threshold)
                        .unwrap_or(false)
                })
                .unwrap_or(idx_from_oldest);
        } else {
            idx_from_oldest = (0..idx_from_oldest)
                .rev()
                .find(|&i| {
                    data.get(i)
                        .map(|b| b.info + b.warn + b.error >= threshold)
                        .unwrap_or(false)
                })
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

    pub fn set_diff_a(&mut self) {
        if let Some(entry) = self.current_entry() {
            self.diff_a = Some(entry.timestamp);
            self.last_notice = Some("Set marker A".to_string());
        } else {
            self.last_notice = Some("No entry to mark as A".to_string());
        }
    }

    pub fn set_diff_b(&mut self) {
        if let Some(entry) = self.current_entry() {
            self.diff_b = Some(entry.timestamp);
            self.last_notice = Some("Set marker B".to_string());
        } else {
            self.last_notice = Some("No entry to mark as B".to_string());
        }
    }

    pub fn clear_diff(&mut self) {
        self.diff_a = None;
        self.diff_b = None;
        self.last_notice = Some("Cleared diff markers".to_string());
    }

    pub fn diff_summary(&self) -> Option<DiffStats> {
        let (a, b) = match (self.diff_a, self.diff_b) {
            (Some(a), Some(b)) => (a.min(b), a.max(b)),
            _ => return None,
        };
        let mut info = 0;
        let mut warn = 0;
        let mut error = 0;
        let mut targets: HashMap<String, usize> = HashMap::new();
        let mut total = 0;
        for entry in self
            .logs
            .iter()
            .filter(|e| e.timestamp >= a && e.timestamp <= b)
        {
            if !self.filters.matches(entry) {
                continue;
            }
            total += 1;
            match entry.level {
                Level::Info => info += 1,
                Level::Warn => warn += 1,
                Level::Error => error += 1,
            }
            *targets.entry(entry.target.clone()).or_default() += 1;
        }
        let mut top_targets: Vec<(String, usize)> = targets.into_iter().collect();
        top_targets.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        top_targets.truncate(5);
        Some(DiffStats {
            total,
            info,
            warn,
            error,
            top_targets,
        })
    }

    pub fn export_diff(&mut self) {
        let (a, b) = match (self.diff_a, self.diff_b) {
            (Some(a), Some(b)) => (a.min(b), a.max(b)),
            _ => {
                self.last_notice = Some("Set A and B before exporting".to_string());
                return;
            }
        };
        let filtered: Vec<&LogEntry> = self
            .logs
            .iter()
            .filter(|e| e.timestamp >= a && e.timestamp <= b)
            .filter(|e| self.filters.matches(e))
            .collect();
        if filtered.is_empty() {
            self.last_notice = Some("No lines in diff range (after filters)".to_string());
            return;
        }
        let path = format!(
            "/tmp/logtm_diff_{}.log",
            Local::now().format("%Y%m%d%H%M%S")
        );
        let body = filtered
            .iter()
            .map(|e| {
                format!(
                    "{} {:5} {:<7} {}",
                    e.timestamp.to_rfc3339(),
                    e.level.label(),
                    e.target,
                    e.message
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        match fs::write(&path, body) {
            Ok(_) => self.last_notice = Some(format!("Exported diff slice to {path}")),
            Err(err) => self.last_notice = Some(format!("Export failed: {err}")),
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

    pub fn diff_a(&self) -> Option<DateTime<Local>> {
        self.diff_a
    }

    pub fn diff_b(&self) -> Option<DateTime<Local>> {
        self.diff_b
    }

    pub fn paused_head_len(&self) -> Option<usize> {
        self.paused_head_len
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
