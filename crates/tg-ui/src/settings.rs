use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::theme::THEMES;

/// What Settings keeps between runs, as JSON in plaingram's data folder.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub show_fps: bool,
    /// Dark lines over the window, like a CRT's.
    pub scanlines: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { theme: THEMES[0].name.into(), show_fps: false, scanlines: false }
    }
}

impl Settings {
    /// The settings saved at `path`, or the defaults before anything was saved.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(json) => serde_json::from_str(&json).with_context(|| format!("cannot read {}", path.display())),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("cannot open {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let dir = path.parent().context("settings file has no folder")?;
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("cannot write {}", path.display()))
    }
}
