//! Renders `assets/icon.svg` into a macOS iconset directory for `iconutil`.

use std::{env, fs};

use anyhow::{Context, Result};
use resvg::{tiny_skia, usvg};

const SVG: &str = include_str!("../assets/icon.svg");

fn main() -> Result<()> {
    let dir = env::args().nth(1).context("usage: icon <AppIcon.iconset>")?;
    fs::create_dir_all(&dir)?;
    let tree = usvg::Tree::from_str(SVG, &usvg::Options::default())?;
    for size in [16, 32, 128, 256, 512] {
        for (scale, suffix) in [(1, ""), (2, "@2x")] {
            let side = size * scale;
            let mut pixmap = tiny_skia::Pixmap::new(side, side).context("icon size must be non-zero")?;
            let zoom = side as f32 / tree.size().width();
            resvg::render(&tree, tiny_skia::Transform::from_scale(zoom, zoom), &mut pixmap.as_mut());
            pixmap.save_png(format!("{dir}/icon_{size}x{size}{suffix}.png"))?;
        }
    }
    Ok(())
}
