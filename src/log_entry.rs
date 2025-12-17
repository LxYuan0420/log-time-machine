use chrono::{DateTime, Local};
use rand::{rngs::SmallRng, Rng};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }

    pub fn color(self) -> ratatui::style::Color {
        match self {
            Level::Info => ratatui::style::Color::White,
            Level::Warn => ratatui::style::Color::Yellow,
            Level::Error => ratatui::style::Color::Red,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: DateTime<Local>,
    pub level: Level,
    pub target: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct JsonLog {
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

pub fn parse_line(line: &str) -> LogEntry {
    if let Some(entry) = parse_json_log(line) {
        return entry;
    }

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

fn parse_json_log(line: &str) -> Option<LogEntry> {
    let json: JsonLog = serde_json::from_str(line).ok()?;
    let timestamp = json
        .timestamp
        .as_deref()
        .or(json.msg.as_deref())
        .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
        .map(|dt| dt.with_timezone(&Local))
        .unwrap_or_else(Local::now);
    let level = json
        .level
        .as_deref()
        .and_then(|lvl| match lvl.to_ascii_uppercase().as_str() {
            "INFO" => Some(Level::Info),
            "WARN" | "WARNING" => Some(Level::Warn),
            "ERROR" | "ERR" | "FATAL" => Some(Level::Error),
            _ => None,
        })
        .unwrap_or(Level::Info);
    let target = json.target.unwrap_or_else(|| "log".to_string());
    let message = json
        .message
        .or(json.msg)
        .unwrap_or_else(|| "<missing>".to_string());
    Some(LogEntry {
        timestamp,
        level,
        target,
        message,
    })
}

pub fn fake_entry(rng: &mut SmallRng) -> LogEntry {
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
}
