use std::rc::Rc;

use anyhow::Result;
use gpui_kit::App;
use gpui_kit::component::{Theme, ThemeConfig};

const TERMINAL: &str = include_str!("../assets/terminal-theme.json");

/// Initialises the component library with the green-on-black terminal theme.
pub fn init(cx: &mut App) -> Result<()> {
    gpui_kit::init(cx);
    let config: ThemeConfig = serde_json::from_str(TERMINAL)?;
    Theme::update(cx, |theme| theme.apply_config(&Rc::new(config)));
    Ok(())
}
