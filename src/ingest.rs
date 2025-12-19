use std::{
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Seek},
    path::PathBuf,
    sync::mpsc,
    thread,
};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

use anyhow::Context;
use rand::{rngs::SmallRng, Rng, SeedableRng};

use crate::{
    config::{SourceConfig, TailStart, TAIL_SLEEP},
    log_entry::{fake_entry, parse_line, LogEntry},
};

#[derive(Debug)]
pub enum Ingest {
    Mock(SmallRng),
    Channel(mpsc::Receiver<String>),
}

impl Ingest {
    pub fn new(source: SourceConfig) -> Self {
        match source {
            SourceConfig::Mock => Ingest::Mock(SmallRng::seed_from_u64(42)),
            SourceConfig::Stdin => Ingest::Channel(spawn_stdin_reader()),
            SourceConfig::File { path, start } => Ingest::Channel(spawn_file_tail(path, start)),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FileId {
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(windows)]
    volume: u64,
    #[cfg(windows)]
    file_index: u64,
    #[cfg(not(any(unix, windows)))]
    len: u64,
}

impl From<&File> for FileId {
    fn from(value: &File) -> Self {
        let meta = value.metadata().expect("metadata");
        #[cfg(unix)]
        {
            return FileId {
                dev: meta.dev(),
                ino: meta.ino(),
            };
        }
        #[cfg(windows)]
        {
            return FileId {
                volume: meta.volume_serial_number().unwrap_or_default(),
                file_index: meta.file_index().unwrap_or_default(),
            };
        }
        #[cfg(not(any(unix, windows)))]
        {
            return FileId { len: meta.len() };
        }
    }
}

impl FileId {
    fn matches(&self, other: &File) -> bool {
        let other_id = FileId::from(other);
        #[cfg(unix)]
        {
            return self.dev == other_id.dev && self.ino == other_id.ino;
        }
        #[cfg(windows)]
        {
            return self.volume == other_id.volume && self.file_index == other_id.file_index;
        }
        #[cfg(not(any(unix, windows)))]
        {
            return self.len == other_id.len;
        }
    }
}

pub fn drain_ingest(ingest: &mut Ingest) -> Vec<LogEntry> {
    match ingest {
        Ingest::Mock(rng) => {
            let count = rng.gen_range(0..=3);
            (0..count).map(|_| fake_entry(rng)).collect()
        }
        Ingest::Channel(rx) => {
            let mut entries = Vec::new();
            while let Ok(line) = rx.try_recv() {
                entries.push(parse_line(&line));
            }
            entries
        }
    }
}

fn spawn_stdin_reader() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    rx
}

fn spawn_file_tail(path: PathBuf, start: TailStart) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reopen_start = start;
        loop {
            match open_reader(&path, reopen_start) {
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
            reopen_start = TailStart::Beginning;
        }
    });
    rx
}

fn open_reader(path: &PathBuf, start: TailStart) -> anyhow::Result<(BufReader<File>, u64, FileId)> {
    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let file_id = FileId::from(&file);
    let mut reader = BufReader::new(file);
    let pos = match start {
        TailStart::Beginning => reader
            .get_mut()
            .seek(io::SeekFrom::Start(0))
            .context("seek to start")?,
        TailStart::End => reader
            .get_mut()
            .seek(io::SeekFrom::End(0))
            .context("seek to end")?,
    };
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
