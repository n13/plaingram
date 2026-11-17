use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::*;
use tg_core::{Reason, SafeText, code_points};

/// Replaced and marked characters stand out, underlined in a warning-colored box.
pub fn safe_text(text: &SafeText, cx: &App) -> StyledText {
    let replaced = HighlightStyle {
        color: Some(black()),
        background_color: Some(cx.theme().warning),
        font_weight: Some(FontWeight::BOLD),
        underline: Some(UnderlineStyle { thickness: px(1.), color: Some(black()), wavy: false }),
        ..Default::default()
    };
    StyledText::new(text.to_string()).with_highlights(text.replaced_ranges().into_iter().map(|range| (range, replaced)))
}

/// The name, then its warnings.
pub fn name_label(name: &SafeText, cx: &App) -> Div {
    h_flex().gap_2().child(safe_text(name, cx)).children(name_warnings(name, cx))
}

/// One warning per reason listing what was replaced or marked in a name, e.g. `looks like l: I`.
pub fn name_warnings(name: &SafeText, cx: &App) -> Vec<Div> {
    let mut replaced: Vec<_> = name.replaced().collect();
    replaced.sort_unstable();
    replaced.dedup();
    replaced
        .chunk_by(|a, b| a.0 == b.0)
        .map(|group| {
            let graphemes: Vec<_> = group
                .iter()
                .map(|&(reason, grapheme)| match reason {
                    Reason::NotAllowed => format!("{grapheme} {}", code_points(grapheme)),
                    _ => grapheme.to_string(),
                })
                .collect();
            warning(format!("{}: {}", group[0].0, graphemes.join(" ")), cx)
        })
        .collect()
}

/// A red warning badge.
pub fn warning(text: String, cx: &App) -> Div {
    div().px_1().text_xs().bg(cx.theme().danger).text_color(cx.theme().danger_foreground).child(text)
}
