use std::fs::OpenOptions;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

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
const DOWNLOADS: &str = "Downloads";
/// Photos re-encoded for upload, inside the data folder, until Telegram has them.
const UPLOADS: &str = "uploads";

/// TDLib's encrypted database and downloaded files.
pub(crate) fn data_dir() -> Result<PathBuf> {
    home(DATA_DIR)
}

/// A new, unused path for a photo being prepared for upload.
pub(crate) fn upload_path(data_dir: &Path) -> Result<PathBuf> {
    let dir = data_dir.join(UPLOADS);
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let mut id = [0u8; 16];
    getrandom::fill(&mut id)?;
    Ok(dir.join(format!("{}.jpg", id.map(|b| format!("{b:02x}")).concat())))
}

pub fn log_file() -> Result<PathBuf> {
    home(LOG_FILE)
}

/// Writes `contents` to a new file in ~/Downloads and returns its path.
pub(crate) fn save_download(name: &str, contents: &mut impl Read) -> Result<PathBuf> {
    let dir = home(DOWNLOADS)?;
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    save_new(&dir, name, contents)
}

/// Writes `contents` to a new file in `dir`, never over an existing one: a taken name gets " (1)", " (2)"… before
/// its extension. Path separators in `name` become `_`, so the file always lands in `dir`.
fn save_new(dir: &Path, name: &str, contents: &mut impl Read) -> Result<PathBuf> {
    let name: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':') { '_' } else { c }).collect();
    let name = match name.trim_start_matches('.') {
        "" => "file",
        name => name,
    };
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    for copy in 0.. {
        let path = dir.join(match copy {
            0 => name.to_owned(),
            n => format!("{stem} ({n}){extension}"),
        });
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                std::io::copy(contents, &mut file).with_context(|| format!("cannot write {}", path.display()))?;
                quarantine(&path)?;
                return Ok(path);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e).with_context(|| format!("cannot create {}", path.display())),
        }
    }
    unreachable!("an unbounded range always yields")
}

/// Marks a saved file as downloaded from the internet, so macOS checks it with Gatekeeper before running it, and
/// Archive Utility marks what it unpacks from it too.
#[cfg(target_os = "macos")]
fn quarantine(path: &Path) -> Result<()> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let status = std::process::Command::new("/usr/bin/xattr")
        .args(["-w", "com.apple.quarantine", &format!("0081;{now:x};plaingram;")])
        .arg(path)
        .status()?;
    anyhow::ensure!(status.success(), "cannot mark {} as downloaded", path.display());
    Ok(())
}

#[cfg(target_os = "linux")]
fn quarantine(_: &Path) -> Result<()> {
    Ok(())
}

fn home(relative: &str) -> Result<PathBuf> {
    Ok(std::env::home_dir().context("no home directory")?.join(relative))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_never_overwrite_and_stay_in_the_folder() {
        let dir = std::env::temp_dir().join(format!("plaingram-save-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let save = |name: &str| save_new(&dir, name, &mut "data".as_bytes()).unwrap();
        assert_eq!(save("report.zip"), dir.join("report.zip"));
        assert_eq!(save("report.zip"), dir.join("report (1).zip"));
        assert_eq!(save("report.zip"), dir.join("report (2).zip"));
        assert_eq!(save("../../etc/passwd"), dir.join("_.._etc_passwd"));
        assert_eq!(save(".hidden"), dir.join("hidden"));
        assert_eq!(save(""), dir.join("file"));
        assert_eq!(std::fs::read_to_string(dir.join("report (1).zip")).unwrap(), "data");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
