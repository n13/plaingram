use std::collections::HashMap;
use std::sync::LazyLock;

/// Unicode's list of every recommended emoji, from https://www.unicode.org/Public/emoji/latest/emoji-test.txt
const EMOJI_TEST: &str = include_str!("../data/emoji-test.txt");
const VS16: char = '\u{FE0F}';
/// Emoji that read as letters or digits: 🅰 🅱 🅾 🅿 Ⓜ ℹ 🔟 and keycaps such as 1️⃣.
const LOOKALIKES: [char; 8] =
    ['\u{1F170}', '\u{1F171}', '\u{1F17E}', '\u{1F17F}', '\u{24C2}', '\u{2139}', '\u{1F51F}', '\u{20E3}'];

/// Each emoji keyed without VS16, mapped to its fully-qualified form.
static EMOJI: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    forms()
        .filter(|(form, status)| matches!(*status, "fully-qualified" | "component") && !form.contains(LOOKALIKES))
        .map(|(form, _)| (without_vs16(&form), form))
        .collect()
});

/// The fully-qualified form of `grapheme` if it is an emoji, with or without VS16.
pub(crate) fn canonical(grapheme: &str) -> Option<&'static str> {
    EMOJI.get(&without_vs16(grapheme)).map(String::as_str)
}

fn without_vs16(grapheme: &str) -> String {
    grapheme.chars().filter(|&c| c != VS16).collect()
}

/// Each emoji form in `emoji-test.txt` with its status, e.g. `("❤", "unqualified")`.
fn forms() -> impl Iterator<Item = (String, &'static str)> {
    EMOJI_TEST.lines().filter(|line| !line.is_empty() && !line.starts_with('#')).map(|line| {
        let (code_points, rest) = line.split_once(';').expect("emoji-test.txt line has no status");
        let form = code_points
            .split_whitespace()
            .map(|cp| u32::from_str_radix(cp, 16).ok().and_then(char::from_u32).expect("invalid code point"))
            .collect();
        (form, rest.split('#').next().unwrap_or_default().trim())
    })
}

#[cfg(test)]
mod tests {
    use unicode_segmentation::UnicodeSegmentation;

    use super::*;
    use crate::text::{Segment, sanitize};

    #[test]
    fn every_unicode_emoji_form_passes_in_canonical_form() {
        let mut count = 0;
        for (form, _) in forms().filter(|(form, _)| !form.contains(LOOKALIKES)) {
            assert_eq!(form.graphemes(true).count(), 1, "{form:?}");
            let Some(&Segment::Emoji(emoji)) = sanitize(&form).segments().first() else { panic!("{form:?}") };
            assert_eq!(without_vs16(emoji), without_vs16(&form));
            count += 1;
        }
        assert!(count > 5000, "only {count} emoji");
    }

    #[test]
    fn letter_and_digit_lookalikes_are_not_emoji() {
        for lookalike in [
            "\u{1F170}\u{FE0F}",
            "\u{1F17F}",
            "\u{24C2}\u{FE0F}",
            "\u{2139}",
            "1\u{FE0F}\u{20E3}",
            "\u{1F51F}",
            "\u{1F1E9}",
        ] {
            assert_eq!(canonical(lookalike), None, "{lookalike:?}");
        }
    }
}
