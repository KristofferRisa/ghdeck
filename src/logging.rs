//! File logging. The TUI owns the terminal, so everything goes to
//! `$XDG_STATE_HOME/ghdeck/ghdeck.log` (default `~/.local/state/ghdeck/ghdeck.log`).

use std::{fs, path::PathBuf, sync::Mutex};

use tracing_subscriber::EnvFilter;

use crate::config;

/// Keep one previous file once the log grows past this size.
const MAX_BYTES: u64 = 5 * 1024 * 1024;

pub fn path() -> PathBuf {
    config::state_dir().join("ghdeck.log")
}

/// Starts logging at `level` unless `GHDECK_LOG` holds a filter (e.g. `debug`,
/// `ghdeck=trace`). Returns the log file path, or None when the file cannot be opened.
pub fn init(level: &str) -> Option<PathBuf> {
    let path = path();
    fs::create_dir_all(path.parent()?).ok()?;
    if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = fs::rename(&path, path.with_extension("log.1"));
    }
    let file = fs::OpenOptions::new().create(true).append(true).open(&path).ok()?;
    let filter = EnvFilter::try_from_env("GHDECK_LOG")
        .or_else(|_| EnvFilter::try_new(format!("warn,ghdeck={level}")))
        .unwrap_or_else(|_| EnvFilter::new("warn,ghdeck=info"));
    tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .with_target(false)
        .with_env_filter(filter)
        .try_init()
        .ok()?;
    Some(path)
}
