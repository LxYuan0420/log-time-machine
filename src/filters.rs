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
            let needle = text.to_lowercase();
            entry.message.to_lowercase().contains(&needle)
                || entry.target.to_lowercase().contains(&needle)
                || entry.level.label().to_lowercase().contains(&needle)
                || entry
                    .timestamp
                    .format("%Y-%m-%dT%H:%M:%S")
                    .to_string()
                    .to_lowercase()
                    .contains(&needle)
        } else {
            true
        }
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
}
