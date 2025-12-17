use std::{fs, path::PathBuf, time::Duration};

use clap::Parser;
use serde::Deserialize;

pub const TICK_RATE: Duration = Duration::from_millis(200);
pub const DEFAULT_MAX_LINES: usize = 1200;
pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(20 * 60);
pub const TIMELINE_BINS: usize = 80;
pub const TIMELINE_WINDOW: Duration = Duration::from_secs(20 * 60);
pub const TAIL_SLEEP: Duration = Duration::from_millis(150);

#[derive(Parser, Debug)]
#[command(name = "log-time-machine")]
pub struct Args {
    /// Tail this file (fallback: mock feed)
    #[arg(long)]
    pub file: Option<PathBuf>,

    /// Read from stdin instead of a file
    #[arg(long)]
    pub stdin: bool,

    /// Maximum number of log lines to retain
    #[arg(long)]
    pub max_lines: Option<usize>,
}

#[derive(Clone)]
pub enum SourceConfig {
    Mock,
    File(PathBuf),
    Stdin,
}

impl SourceConfig {
    pub fn label(&self) -> String {
        match self {
            SourceConfig::Mock => "mock feed".to_string(),
            SourceConfig::Stdin => "stdin".to_string(),
            SourceConfig::File(path) => format!("file: {}", path.display()),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct FileConfig {
    pub max_lines: Option<usize>,
}

impl FileConfig {
    fn load_from(path: &PathBuf) -> Option<Self> {
        let contents = fs::read_to_string(path).ok()?;
        toml::from_str(&contents).ok()
    }
}

#[derive(Debug)]
pub struct AppConfig {
    pub max_lines: usize,
}

impl AppConfig {
    pub fn load(args: &Args) -> Self {
        let config_path = std::env::var("LOGTM_CONFIG")
            .map(PathBuf::from)
            .ok()
            .unwrap_or_else(|| {
                dirs::config_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("logtm/config.toml")
            });
        let file_cfg = FileConfig::load_from(&config_path);
        let max_lines = args
            .max_lines
            .or_else(|| file_cfg.as_ref().and_then(|c| c.max_lines))
            .unwrap_or(DEFAULT_MAX_LINES);
        AppConfig { max_lines }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn config_merges_defaults() {
        let args = Args {
            file: None,
            stdin: false,
            max_lines: None,
        };
        let cfg = AppConfig::load(&args);
        assert_eq!(cfg.max_lines, DEFAULT_MAX_LINES);
    }

    #[test]
    fn config_reads_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "max_lines = 42").unwrap();
        std::env::set_var("LOGTM_CONFIG", &path);
        let args = Args {
            file: None,
            stdin: false,
            max_lines: None,
        };
        let cfg = AppConfig::load(&args);
        assert_eq!(cfg.max_lines, 42);
        std::env::remove_var("LOGTM_CONFIG");
    }
}
