use std::collections::HashSet;
use std::fmt;
use std::ops::{Range, RangeInclusive};
use std::sync::{LazyLock, Mutex};

use unicode_segmentation::UnicodeSegmentation;

use crate::emoji;

/// Writing systems whose letters may form words. Each word takes the alphabet of its first letter.
#[allow(clippy::single_range_in_vec_init)]
const ALPHABETS: [(&str, &[RangeInclusive<char>]); 9] = [
    ("Latin", &['A'..='Z', 'a'..='z', '\u{C0}'..='\u{D6}', '\u{D8}'..='\u{F6}', '\u{F8}'..='\u{17E}']),
    ("Greek", &['\u{370}'..='\u{3FF}']),
    ("Cyrillic", &['\u{400}'..='\u{4FF}']),
    ("Hebrew", &['\u{5D0}'..='\u{5EA}']),
    ("Arabic", &['\u{620}'..='\u{65F}', '\u{66E}'..='\u{6D3}']),
    ("Devanagari", &['\u{900}'..='\u{97F}']),
    ("Thai", &['\u{E01}'..='\u{E5B}']),
    ("Hangul", &['\u{3131}'..='\u{3163}', '\u{3165}'..='\u{318E}', '\u{AC00}'..='\u{D7A3}']),
    (
        "CJK",
        &[
            '\u{3001}'..='\u{302F}',
            '\u{3031}'..='\u{303C}',
            '\u{3041}'..='\u{30FF}',
            '\u{3400}'..='\u{4DBF}',
            '\u{4E00}'..='\u{9FFF}',
            '\u{FF01}'..='\u{FF0F}',
            '\u{FF1A}'..='\u{FF20}',
        ],
    ),
];

/// Typographic punctuation that Apple keyboards insert, and signs that fit next to any alphabet.
const SYMBOLS: [char; 18] = [
    '\u{2018}',
    '\u{2019}',
    '\u{201C}',
    '\u{201D}',
    '\u{2022}',
    '\u{2013}',
    '\u{2014}',
    '\u{2026}',
    '\u{20AC}',
    '\u{20BF}',
    '\u{B7}',
    '\u{22C6}',
    '\u{2664}',
    '\u{2765}',
    '\u{27C1}',
    '\u{27E0}',
    '\u{13080}',
    '\u{1F71B}',
];

/// Lines already logged, so each is reported once.
static REPORTED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    NotAllowed,
    /// A `letter` from another alphabet than the `first` letter of its word, or of its name.
    MixedAlphabets {
        letter: &'static str,
        first: &'static str,
    },
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::NotAllowed => f.write_str("not on the allowlist"),
            Reason::MixedAlphabets { letter, first } => write!(f, "{letter} in {first} text"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    Text(String),
    Emoji(&'static str),
    /// Displayed as `?`; the raw grapheme is only shown in warnings.
    Replaced {
        grapheme: String,
        reason: Reason,
    },
}

impl Segment {
    pub fn as_str(&self) -> &str {
        match self {
            Segment::Text(text) => text,
            Segment::Emoji(emoji) => emoji,
            Segment::Replaced { .. } => "?",
        }
    }
}

/// Text that has passed the allowlist. Only [`sanitize`] can create one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SafeText(Vec<Segment>);

impl SafeText {
    pub fn segments(&self) -> &[Segment] {
        &self.0
    }

    pub fn has_replacements(&self) -> bool {
        self.replaced().next().is_some()
    }

    /// Each replaced grapheme with the reason, in order.
    pub fn replaced(&self) -> impl Iterator<Item = (Reason, &str)> {
        self.0.iter().filter_map(|segment| match segment {
            Segment::Replaced { grapheme, reason } => Some((*reason, grapheme.as_str())),
            _ => None,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Byte ranges of the replacement `?`s within the [`Display`](fmt::Display) output.
    pub fn replaced_ranges(&self) -> Vec<Range<usize>> {
        let mut end = 0;
        self.0
            .iter()
            .filter_map(|segment| {
                let start = end;
                end += segment.as_str().len();
                matches!(segment, Segment::Replaced { .. }).then_some(start..end)
            })
            .collect()
    }
}

impl fmt::Display for SafeText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|segment| f.write_str(segment.as_str()))
    }
}

pub fn is_allowed(raw: &str) -> bool {
    !sanitize(raw).has_replacements()
}

/// Keeps ASCII digits, punctuation and whitespace, [`SYMBOLS`], Unicode emoji other than letter lookalikes, and
/// letters from the alphabet of their word's first letter; every other grapheme becomes [`Segment::Replaced`].
/// Words are separated by ASCII whitespace. Deliberately no Unicode normalization, so lookalikes such as
/// fullwidth or mathematical letters stay visible.
pub fn sanitize(raw: &str) -> SafeText {
    sanitize_with(raw, true)
}

/// Like [`sanitize`] for names of people and chats, but the first letter decides the alphabet of the whole name,
/// so a lone foreign word such as the `Ω` in `Alex Ω` is replaced too. Logs each flagged name once.
pub fn sanitize_name(raw: &str) -> SafeText {
    let name = sanitize_with(raw, false);
    if name.has_replacements() {
        report(format!("flagged name: {raw:?}"));
    }
    name
}

fn sanitize_with(raw: &str, per_word: bool) -> SafeText {
    let mut segments = Vec::new();
    let mut first = None;
    for grapheme in raw.graphemes(true) {
        let is_space = grapheme.chars().all(|c| c.is_ascii_whitespace());
        if per_word && is_space {
            first = None;
        }
        match (if is_space { None } else { special(grapheme, &mut first) }, segments.last_mut()) {
            (Some(segment), _) => segments.push(segment),
            (None, Some(Segment::Text(text))) => text.push_str(grapheme),
            (None, _) => segments.push(Segment::Text(grapheme.into())),
        }
    }
    SafeText(segments)
}

/// `None` for plain text. `first` is the alphabet of the first letter of the current word or name.
fn special(grapheme: &str, first: &mut Option<&'static str>) -> Option<Segment> {
    if let Some(letter) = alphabet(grapheme) {
        let first = *first.get_or_insert(letter);
        return (letter != first).then(|| replace(grapheme, Reason::MixedAlphabets { letter, first }));
    }
    if grapheme.chars().all(|c| (c.is_ascii_graphic() && !c.is_ascii_alphabetic()) || SYMBOLS.contains(&c)) {
        return None;
    }
    Some(emoji::canonical(grapheme).map_or_else(|| replace(grapheme, Reason::NotAllowed), Segment::Emoji))
}

fn alphabet(grapheme: &str) -> Option<&'static str> {
    ALPHABETS
        .iter()
        .find(|(_, ranges)| grapheme.chars().all(|c| ranges.iter().any(|range| range.contains(&c))))
        .map(|(name, _)| *name)
}

fn replace(grapheme: &str, reason: Reason) -> Segment {
    report(format!("{reason}: {} {grapheme:?}", code_points(grapheme)));
    Segment::Replaced { grapheme: grapheme.into(), reason }
}

fn report(line: String) {
    if REPORTED.lock().expect("report lock poisoned").insert(line.clone()) {
        tracing::info!("{line}");
    }
}

/// `U+` notation for each code point, e.g. `U+1F44D U+1F3FD`.
pub fn code_points(grapheme: &str) -> String {
    let code_points: Vec<_> = grapheme.chars().map(|c| format!("U+{:04X}", u32::from(c))).collect();
    code_points.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use Segment::*;

    fn text(s: &str) -> Segment {
        Text(s.to_string())
    }

    fn replaced(grapheme: &str) -> Segment {
        Replaced { grapheme: grapheme.into(), reason: Reason::NotAllowed }
    }

    fn mixed(grapheme: &str, letter: &'static str, first: &'static str) -> Segment {
        Replaced { grapheme: grapheme.into(), reason: Reason::MixedAlphabets { letter, first } }
    }

    #[test]
    fn printable_ascii_passes_unchanged() {
        let raw: String = (0x20u8..=0x7E).map(char::from).chain("\t\n\r\n\x0C".chars()).collect();
        let safe = sanitize(&raw);
        assert_eq!(safe.segments(), [text(&raw)]);
        assert!(!safe.has_replacements());
    }

    #[test]
    fn typographic_punctuation_and_symbols_pass() {
        let raw =
            "\u{2022} We\u{2019}ve \u{201C}shipped\u{201D} \u{2018}it\u{2019} \u{2013} 1\u{2014}2\u{2026} 5\u{20AC}";
        assert_eq!(sanitize(raw).segments(), [text(raw)]);
        assert_eq!(sanitize("\u{2019}\u{0301}").segments(), [replaced("\u{2019}\u{0301}")]);
    }

    #[test]
    fn words_written_in_one_alphabet_pass() {
        for raw in ["Café Müller", "Привет, мир!", "你好，世界。", "안녕 ㅋㅋ", "Ελλάδα", "Привет world"]
        {
            assert_eq!(sanitize(raw).segments(), [text(raw)], "{raw:?}");
        }
    }

    #[test]
    fn letters_from_another_alphabet_than_the_first_are_replaced() {
        assert_eq!(
            sanitize("P\u{0430}ypal").segments(),
            [text("P"), mixed("\u{0430}", "Cyrillic", "Latin"), text("ypal")]
        );
        assert_eq!(sanitize("(\u{0420}ay) pay").to_string(), "(\u{0420}??) pay");
        assert_eq!(sanitize("Москва/Moscow").to_string(), "Москва/??????");
        assert_eq!(sanitize("我用iPhone 很好").to_string(), "我用?????? 很好");
    }

    #[test]
    fn names_take_one_alphabet_across_words() {
        assert_eq!(sanitize("Alex \u{03A9}").segments(), [text("Alex \u{03A9}")]);
        assert_eq!(sanitize_name("Alex \u{03A9}").segments(), [text("Alex "), mixed("\u{03A9}", "Greek", "Latin")]);
        assert_eq!(sanitize_name("\u{0410}lice Smith").to_string(), "\u{0410}???? ?????");
        assert_eq!(sanitize_name("Андрей Смоляков \u{1F4A6}").to_string(), "Андрей Смоляков \u{1F4A6}");
        assert_eq!(Reason::MixedAlphabets { letter: "Greek", first: "Latin" }.to_string(), "Greek in Latin text");
    }

    #[test]
    fn letter_lookalikes_outside_the_alphabets_are_replaced() {
        assert_eq!(sanitize("\u{FF21}dmin").to_string(), "?dmin");
        assert_eq!(sanitize("\u{1D400}lice").to_string(), "?lice");
        assert_eq!(sanitize("\u{1D00}dmin").to_string(), "?dmin");
        assert_eq!(sanitize("\u{3164}").to_string(), "?");
    }

    #[test]
    fn replaced_ranges_point_at_display_output() {
        let safe = sanitize("\u{1F602}P\u{0430}y\u{200B}");
        assert_eq!(safe.to_string(), "\u{1F602}P?y?");
        assert_eq!(safe.replaced_ranges(), [5..6, 7..8]);
        assert!(is_allowed("ok \u{1F602}\n") && !is_allowed("P\u{0430}y"));
        let replaced: Vec<_> = safe.replaced().map(|(_, grapheme)| grapheme).collect();
        assert_eq!(replaced, ["\u{0430}", "\u{200B}"]);
        assert_eq!(code_points("\u{1F44D}\u{1F3FD}"), "U+1F44D U+1F3FD");
    }

    #[test]
    fn invisible_and_control_characters_are_replaced() {
        for c in ["\u{200B}", "\u{202E}", "\u{00A0}", "\u{2028}", "\u{0000}", "\u{1B}", "\u{7F}"] {
            assert_eq!(sanitize(&format!("a{c}b")).segments(), [text("a"), replaced(c), text("b")], "{c:?}");
        }
    }

    #[test]
    fn whole_grapheme_becomes_one_replacement() {
        assert_eq!(sanitize("e\u{0301}").segments(), [replaced("e\u{0301}")]);
        assert_eq!(sanitize("1\u{FE0F}\u{20E3}").segments(), [replaced("1\u{FE0F}\u{20E3}")]);
        assert_eq!(sanitize("a\u{FE0F}").segments(), [replaced("a\u{FE0F}")]);
    }

    #[test]
    fn emoji_pass_in_canonical_form() {
        assert_eq!(sanitize("hi \u{1F602}").segments(), [text("hi "), Emoji("\u{1F602}")]);
        assert_eq!(sanitize("\u{2764}").segments(), [Emoji("\u{2764}\u{FE0F}")]);
        assert_eq!(sanitize("\u{2764}\u{FE0F}").segments(), [Emoji("\u{2764}\u{FE0F}")]);
        assert_eq!(sanitize("\u{1F602}\u{FE0F}").segments(), [Emoji("\u{1F602}")]);
        for emoji in
            ["\u{1F1E9}\u{1F1EA}", "\u{1F44D}\u{1F3FD}", "\u{1F926}\u{200D}\u{2642}\u{FE0F}", "\u{1F6CB}\u{FE0F}"]
        {
            assert_eq!(sanitize(emoji).segments(), [Emoji(emoji)], "{emoji:?}");
        }
    }
}
