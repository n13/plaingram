use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::{Theme, ThemeConfig};
use gpui_kit::{App, FontFeatures};
use serde_json::{Value, json};

/// Monospace fonts that keep `0`/`O`, `1`/`l`/`I` and `rn`/`m` apart.
#[cfg(target_os = "macos")]
const MONO_FONT: &str = "Menlo";
#[cfg(target_os = "linux")]
const MONO_FONT: &str = "DejaVu Sans Mono";
/// San Francisco on macOS.
const SYSTEM_FONT: &str = ".SystemUIFont";
/// San Francisco's high legibility set, which keeps `I`/`l` and `0`/`O` apart.
const HIGH_LEGIBILITY: &str = "ss06";

/// The colors a theme is made of, as `#rrggbb`.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub name: &'static str,
    light: bool,
    /// San Francisco with its high legibility set instead of a monospace font.
    system_font: bool,
    /// Phosphor glow: halos around message bubbles and darkened edges.
    pub glow: bool,
    background: &'static str,
    sidebar: &'static str,
    title_bar: &'static str,
    popover: &'static str,
    foreground: &'static str,
    /// Secondary text.
    dim: &'static str,
    /// Buttons and badges, with `on_primary` text.
    primary: &'static str,
    bright: &'static str,
    pressed: &'static str,
    on_primary: &'static str,
    border: &'static str,
    input_border: &'static str,
    /// Incoming messages and hovered rows.
    surface: &'static str,
    surface_hover: &'static str,
    surface_active: &'static str,
    muted: &'static str,
    /// Outgoing messages and the open chat.
    accent: &'static str,
    /// The box around marked characters, with `on_alert` text.
    warning: &'static str,
    /// Warning badges and errors, with `on_alert` text.
    danger: &'static str,
    on_alert: &'static str,
}

const CLI: Palette = Palette {
    name: "CLI",
    light: false,
    system_font: false,
    glow: false,
    background: "#030603",
    sidebar: "#020402",
    title_bar: "#000000",
    popover: "#050a05",
    foreground: "#33ff66",
    dim: "#1fa84a",
    primary: "#33ff66",
    bright: "#70ff94",
    pressed: "#1fd14f",
    on_primary: "#000000",
    border: "#145226",
    input_border: "#1f7a3c",
    surface: "#0a1f10",
    surface_hover: "#0f2e18",
    surface_active: "#143d20",
    muted: "#0a1a0e",
    accent: "#103a1d",
    warning: "#ffb000",
    danger: "#ff4040",
    on_alert: "#000000",
};

pub static THEMES: [Palette; 8] = [
    CLI,
    Palette { name: "CLI proportional", system_font: true, ..CLI },
    Palette {
        name: "Orange glow",
        glow: true,
        background: "#0a0600",
        sidebar: "#070400",
        popover: "#0d0800",
        foreground: "#ffb000",
        dim: "#c07c00",
        primary: "#ffb000",
        bright: "#ffcc4d",
        pressed: "#e09a00",
        border: "#523600",
        input_border: "#7a5000",
        surface: "#1f1400",
        surface_hover: "#2e1e00",
        surface_active: "#3d2800",
        muted: "#1a1100",
        accent: "#3a2600",
        warning: "#33d6ff",
        danger: "#ff3b30",
        ..CLI
    },
    Palette {
        name: "Paper white",
        background: "#0c0c0c",
        sidebar: "#080808",
        popover: "#111111",
        foreground: "#e8e8de",
        dim: "#9a9a92",
        primary: "#e8e8de",
        bright: "#ffffff",
        pressed: "#c8c8be",
        border: "#3a3a36",
        input_border: "#5a5a54",
        surface: "#1c1c1a",
        surface_hover: "#262624",
        surface_active: "#30302e",
        muted: "#161614",
        accent: "#2c2c28",
        danger: "#ff4d4d",
        ..CLI
    },
    Palette {
        name: "DOS blue",
        background: "#0000aa",
        sidebar: "#000088",
        title_bar: "#000077",
        popover: "#00008a",
        foreground: "#ffffff",
        dim: "#aaaaaa",
        primary: "#55ffff",
        bright: "#aaffff",
        pressed: "#00cccc",
        border: "#5555ff",
        input_border: "#5555ff",
        surface: "#1a1ac4",
        surface_hover: "#2a2ad0",
        surface_active: "#3a3ad8",
        muted: "#00009a",
        accent: "#00608a",
        warning: "#ffff55",
        danger: "#ff5555",
        ..CLI
    },
    Palette {
        name: "C64",
        background: "#352879",
        sidebar: "#2c2068",
        title_bar: "#241a58",
        popover: "#3a2d84",
        foreground: "#c8c0ff",
        dim: "#b8b0f4",
        primary: "#c8c0ff",
        bright: "#e4e0ff",
        pressed: "#a8a0f0",
        on_primary: "#352879",
        border: "#6c5eb5",
        input_border: "#8a7cd0",
        surface: "#43369a",
        surface_hover: "#4c3fa6",
        surface_active: "#5547b0",
        muted: "#30246e",
        accent: "#503f9e",
        warning: "#edf171",
        danger: "#ff7a70",
        ..CLI
    },
    Palette {
        name: "Workbench",
        background: "#0055aa",
        sidebar: "#004c99",
        title_bar: "#00468c",
        popover: "#0060bb",
        foreground: "#ffffff",
        dim: "#b8d4f0",
        primary: "#ff8800",
        bright: "#ffaa44",
        pressed: "#dd7000",
        border: "#001a40",
        input_border: "#66a3e0",
        surface: "#00407f",
        surface_hover: "#00376e",
        surface_active: "#002f5f",
        muted: "#004d9c",
        accent: "#1a6cc4",
        warning: "#ffdd00",
        danger: "#ff3040",
        ..CLI
    },
    Palette {
        name: "1-bit Mac",
        light: true,
        system_font: true,
        background: "#ffffff",
        sidebar: "#ffffff",
        title_bar: "#ffffff",
        popover: "#ffffff",
        foreground: "#000000",
        dim: "#555555",
        primary: "#000000",
        bright: "#333333",
        pressed: "#000000",
        on_primary: "#ffffff",
        border: "#000000",
        input_border: "#000000",
        surface: "#ebebeb",
        surface_hover: "#dddddd",
        surface_active: "#cccccc",
        muted: "#f2f2f2",
        accent: "#dcdcdc",
        warning: "#cc5500",
        danger: "#ff3b30",
        ..CLI
    },
];

impl Palette {
    /// The theme called `name`.
    pub fn named(name: &str) -> Option<&'static Palette> {
        THEMES.iter().find(|theme| theme.name == name)
    }

    /// Font features for the whole window: high legibility where the theme uses the system font.
    pub fn font_features(&self) -> FontFeatures {
        FontFeatures(Arc::new(if self.system_font { vec![(HIGH_LEGIBILITY.into(), 1)] } else { Vec::new() }))
    }

    /// Every color of the component library, derived from the palette.
    fn colors(&self) -> Value {
        let alpha = |color: &str, alpha: &str| format!("{color}{alpha}");
        json!({
            "background": self.background,
            "foreground": self.foreground,
            "border": self.border,
            "window.border": self.border,
            "caret": self.foreground,
            "ring": self.foreground,
            "selection.background": alpha(self.foreground, "40"),
            "overlay": if self.light { "#00000066" } else { "#000000b0" },
            "muted.background": self.muted,
            "muted.foreground": self.dim,
            "primary.background": self.primary,
            "primary.foreground": self.on_primary,
            "primary.hover.background": self.bright,
            "primary.active.background": self.pressed,
            "secondary.background": self.surface,
            "secondary.foreground": self.foreground,
            "secondary.hover.background": self.surface_hover,
            "secondary.active.background": self.surface_active,
            "accent.background": self.accent,
            "accent.foreground": self.foreground,
            "warning.background": self.warning,
            "warning.foreground": self.on_alert,
            "danger.background": self.danger,
            "danger.foreground": self.on_alert,
            "success.background": self.primary,
            "success.foreground": self.on_primary,
            "input.border": self.input_border,
            "link.foreground": self.primary,
            "link.hover.foreground": self.bright,
            "link.active.foreground": self.pressed,
            "popover.background": self.popover,
            "popover.foreground": self.foreground,
            "list.background": self.background,
            "list.even.background": self.background,
            "list.hover.background": self.surface,
            "list.active.background": self.accent,
            "list.active.border": self.primary,
            "sidebar.background": self.sidebar,
            "sidebar.foreground": self.foreground,
            "sidebar.border": self.border,
            "sidebar.accent.background": self.accent,
            "sidebar.accent.foreground": self.foreground,
            "tab_bar.background": self.sidebar,
            "tab.background": self.sidebar,
            "tab.foreground": self.dim,
            "tab.active.background": self.sidebar,
            "tab.active.foreground": self.foreground,
            "title_bar.background": self.title_bar,
            "title_bar.border": self.border,
            "scrollbar.background": "#00000000",
            "scrollbar.thumb.background": alpha(self.input_border, "99"),
            "scrollbar.thumb.hover.background": alpha(self.foreground, "aa"),
        })
    }

    /// Makes this the theme of every window.
    pub fn apply(&self, cx: &mut App) {
        let config = self.config();
        Theme::update(cx, |theme| theme.apply_config(&Rc::new(config)));
        cx.refresh_windows();
    }

    fn config(&self) -> ThemeConfig {
        let mut config: ThemeConfig = serde_json::from_value(json!({
            "name": self.name,
            "mode": if self.light { "light" } else { "dark" },
            "font.size": 14,
            "mono_font.size": 14,
            "radius": 0,
            "radius.lg": 0,
            "shadow": false,
            "colors": self.colors(),
        }))
        .expect("every palette makes a theme");
        config.font_family = Some(if self.system_font { SYSTEM_FONT } else { MONO_FONT }.into());
        config.mono_font_family = Some(MONO_FONT.into());
        config
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Hsla, rgb};

    use super::*;

    fn color(hex: &str) -> Hsla {
        rgb(u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap()).into()
    }

    /// WCAG's contrast ratio, from 1:1 to 21:1.
    fn contrast(a: &str, b: &str) -> f32 {
        let luminance = |hex: &str| {
            let rgba = color(hex).to_rgb();
            let [r, g, b] = [rgba.r, rgba.g, rgba.b]
                .map(|c| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) });
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// Text meets WCAG AA everywhere it sits, AAA for messages on the window background, and the box around marked
    /// characters stands out from everything it sits on.
    #[test]
    fn every_theme_is_legible() {
        for theme in &THEMES {
            let check = |what: &str, front: &str, back: &str, min: f32| {
                let ratio = contrast(front, back);
                assert!(ratio >= min, "{}: {what} {front} on {back} is {ratio:.1}:1, under {min}:1", theme.name);
            };
            check("text", theme.foreground, theme.background, 7.);
            for back in [theme.sidebar, theme.title_bar, theme.popover, theme.surface, theme.accent] {
                check("text", theme.foreground, back, 4.5);
            }
            for back in [theme.background, theme.sidebar, theme.surface] {
                check("secondary text", theme.dim, back, 4.5);
            }
            check("button text", theme.on_primary, theme.primary, 4.5);
            check("marked characters", theme.on_alert, theme.warning, 4.5);
            check("warnings", theme.on_alert, theme.danger, 4.5);
            for back in [theme.background, theme.surface, theme.accent] {
                check("the box around marked characters", theme.warning, back, 3.);
            }
        }
    }

    /// Colored text never shares the hue of the box around marked characters, which would hide the box.
    #[test]
    fn marked_characters_stand_apart_from_text() {
        for theme in &THEMES {
            let (text, marked) = (color(theme.foreground), color(theme.warning));
            let apart = (text.h - marked.h).abs().min(1. - (text.h - marked.h).abs()) * 360.;
            assert!(text.s < 0.25 || apart >= 30., "{}: hues {apart:.0}° apart", theme.name);
        }
    }

    #[test]
    fn every_palette_makes_a_theme_named_after_it() {
        for theme in &THEMES {
            assert_eq!(theme.config().name, theme.name);
            assert!(std::ptr::eq(Palette::named(theme.name).unwrap(), theme));
        }
    }
}
