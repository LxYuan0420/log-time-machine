use std::{path::PathBuf, time::Duration};

use clap::Parser;

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
    #[arg(long, default_value_t = DEFAULT_MAX_LINES)]
    pub max_lines: usize,
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
