use std::rc::Rc;

use anyhow::Result;
use gpui_kit::App;
use gpui_kit::component::{Theme, ThemeConfig};

const TERMINAL: &str = include_str!("../assets/terminal-theme.json");
/// Monospace fonts that keep `0`/`O`, `1`/`l`/`I` and `rn`/`m` apart.
#[cfg(target_os = "macos")]
const MONO_FONT: &str = "Menlo";
#[cfg(target_os = "linux")]
const MONO_FONT: &str = "DejaVu Sans Mono";

/// Initialises the component library with the green-on-black terminal theme.
pub fn init(cx: &mut App) -> Result<()> {
    gpui_kit::init(cx);
    let mut config: ThemeConfig = serde_json::from_str(TERMINAL)?;
    config.font_family = Some(MONO_FONT.into());
    config.mono_font_family = Some(MONO_FONT.into());
    Theme::update(cx, |theme| theme.apply_config(&Rc::new(config)));
    Ok(())
}
