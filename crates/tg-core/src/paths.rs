use std::path::PathBuf;

use anyhow::{Context, Result};

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("plaingram supports macOS and Linux");

#[cfg(target_os = "macos")]
const DATA_DIR: &str = "Library/Application Support/plaingram";
#[cfg(target_os = "macos")]
const LOG_FILE: &str = "Library/Logs/plaingram.log";
#[cfg(target_os = "linux")]
const DATA_DIR: &str = ".local/share/plaingram";
#[cfg(target_os = "linux")]
const LOG_FILE: &str = ".local/state/plaingram/plaingram.log";

/// TDLib's encrypted database and downloaded files.
pub(crate) fn data_dir() -> Result<PathBuf> {
    home(DATA_DIR)
}

pub fn log_file() -> Result<PathBuf> {
    home(LOG_FILE)
}

fn home(relative: &str) -> Result<PathBuf> {
    Ok(std::env::home_dir().context("no home directory")?.join(relative))
}
