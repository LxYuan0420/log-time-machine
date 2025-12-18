use regex::Regex;

use crate::log_entry::{Level, LogEntry};

#[derive(Debug, Clone)]
pub struct Filters {
    pub info: bool,
    pub warn: bool,
    pub error: bool,
    pub text: Option<String>,
    pub regex_mode: bool,
    pub compiled: Option<Regex>,
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
    pub fn matches(&self, entry: &LogEntry) -> bool {
        let level_ok = match entry.level {
            Level::Info => self.info,
            Level::Warn => self.warn,
            Level::Error => self.error,
        };
        if !level_ok {
            return false;
        }
        let Some(ref text) = self.text else {
            return true;
        };
        if text.is_empty() {
            return true;
        }

        let haystack = format!(
            "{} {} {} {}",
            entry.timestamp.format("%Y-%m-%dT%H:%M:%S"),
            entry.level.label(),
            entry.target,
            entry.message
        );

        if self.regex_mode {
            if let Some(re) = &self.compiled {
                return re.is_match(&haystack);
            }
            return true;
        }

        let needle = text.to_lowercase();
        haystack.to_lowercase().contains(&needle)
    }

    pub fn set_text(&mut self, text: Option<String>) -> Result<(), regex::Error> {
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
pub enum InputMode {
    Normal,
    FilterText(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    #[test]
    fn filters_support_regex_across_fields() {
        let entry = LogEntry {
            timestamp: Local::now(),
            level: Level::Warn,
            target: "api".to_string(),
            message: "timeout while calling upstream".to_string(),
        };
        let mut filters = Filters::default();
        filters.regex_mode = true;
        filters
            .set_text(Some("WARN.*api.*timeout".to_string()))
            .unwrap();
        assert!(filters.matches(&entry));
    }

    #[test]
    fn filters_match_all_fields_with_text() {
        let entry = LogEntry {
            timestamp: Local::now(),
            level: Level::Info,
            target: "ingest".to_string(),
            message: "ingest worker started".to_string(),
        };
        let mut filters = Filters::default();
        filters.set_text(Some("ingest worker".to_string())).unwrap();
        assert!(filters.matches(&entry));
    }

    #[test]
    fn filters_match_timestamp_and_level() {
        let entry = LogEntry {
            timestamp: Local::now(),
            level: Level::Error,
            target: "db".to_string(),
            message: "failed to commit".to_string(),
        };
        let mut filters = Filters::default();
        filters.set_text(Some("error db".to_string())).unwrap();
        assert!(filters.matches(&entry));
    }
}
