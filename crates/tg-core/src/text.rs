use std::collections::HashSet;
use std::fmt;
use std::ops::{Range, RangeInclusive};
use std::sync::{LazyLock, Mutex};

use unicode_normalization::char::is_combining_mark;
use unicode_segmentation::UnicodeSegmentation;

use crate::emoji;

/// Writing systems whose letters may form words. Each word takes the alphabet of its first letter. Latin leaves out
/// `ĳ Ĳ ĸ Ŀ ŀ ŉ`, compatibility letters that keyboards do not type and that pass for `ij`, `k`, `l` and `'n`.
#[allow(clippy::single_range_in_vec_init)]
const ALPHABETS: [(&str, &[RangeInclusive<char>]); 9] = [
    (
        "Latin",
        &[
            'A'..='Z',
            'a'..='z',
            '\u{C0}'..='\u{D6}',
            '\u{D8}'..='\u{F6}',
            '\u{F8}'..='\u{131}',
            '\u{134}'..='\u{137}',
            '\u{139}'..='\u{13E}',
            '\u{141}'..='\u{148}',
            '\u{14A}'..='\u{17E}',
        ],
    ),
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

/// Combining marks one letter may carry in a row, as in Devanagari `ज़िं` (nukta, vowel sign, anusvara). More stack
/// into a tower that draws over the lines around it.
const MAX_MARKS: usize = 3;

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
            Reason::NotAllowed => f.write_str("suspicious characters"),
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

/// What a name or handle looks like, for telling whether two could be mistaken for each other. Only for comparing;
/// there is no way to display one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Skeleton(String);

impl Skeleton {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
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

    /// The texts one after another, each keeping its replaced graphemes.
    pub fn concat(parts: &[&SafeText]) -> SafeText {
        SafeText(parts.iter().flat_map(|part| part.0.iter().cloned()).collect())
    }

    /// Folds case, punctuation, accents and the look-alike letters in Unicode's confusables list ([UTS #39]), so
    /// `a1ice_smith_`, `Alice Smith`, `Álice` and `Alicе` with a Cyrillic `е` have the same skeleton. Replaced
    /// graphemes count as what they are, not as `?`.
    ///
    /// [UTS #39]: https://www.unicode.org/reports/tr39/#Confusable_Detection
    pub fn skeleton(&self) -> Skeleton {
        let raw: String = self
            .0
            .iter()
            .map(|segment| match segment {
                Segment::Text(text) | Segment::Replaced { grapheme: text, .. } => text.as_str(),
                Segment::Emoji(emoji) => emoji,
            })
            .collect();
        let folded: String = unicode_security::skeleton(&raw.to_lowercase()).flat_map(char::to_lowercase).collect();
        Skeleton(
            folded
                .chars()
                .filter(|c| c.is_alphanumeric())
                .map(|c| match c {
                    '0' => 'o',
                    '1' | 'i' => 'l',
                    c => c,
                })
                .collect(),
        )
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
/// so a lone foreign word such as the `Ω` in `Alex Ω` is replaced too. A name with a word that mixes cases, such as
/// the capital `I` posing as `l` in `AIice`, shows in lowercase, where `i` and `l` differ.
pub fn sanitize_name(raw: &str) -> SafeText {
    match mixes_case(raw) {
        true => sanitize_with(&raw.to_lowercase(), false),
        false => sanitize_with(raw, false),
    }
}

/// Some word, a run of letters, has a capital after its first letter as well as a small letter, as in `AIice`, `ADMlN`
/// or `PayPal`; capitalized and all-caps words do not.
fn mixes_case(raw: &str) -> bool {
    raw.split(|c: char| !c.is_alphabetic())
        .any(|word| word.chars().any(char::is_lowercase) && word.chars().skip(1).any(char::is_uppercase))
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
        if stacks_marks(grapheme) {
            return Some(replace(grapheme, Reason::NotAllowed));
        }
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

/// More than [`MAX_MARKS`] combining marks follow one another somewhere in `grapheme`.
fn stacks_marks(grapheme: &str) -> bool {
    let mut run = 0;
    grapheme.chars().any(|c| {
        run = if is_combining_mark(c) { run + 1 } else { 0 };
        run > MAX_MARKS
    })
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
    fn names_with_a_word_that_mixes_cases_show_in_lowercase() {
        for (raw, shown) in [
            ("AIice Smith", "aiice smith"),
            ("ADMlN", "admln"),
            ("aLICE", "alice"),
            ("PayPal Support", "paypal support"),
            ("AIice_smith", "aiice_smith"),
            ("АндрЕй", "андрей"),
            ("Alice Smith", "Alice Smith"),
            ("ADMIN", "ADMIN"),
            ("Jean-Luc O'Brien", "Jean-Luc O'Brien"),
            ("Андрей ПЕТРОВ", "Андрей ПЕТРОВ"),
        ] {
            assert_eq!(sanitize_name(raw).to_string(), shown, "{raw}");
        }
        assert_eq!(sanitize("AIice").to_string(), "AIice", "messages keep their case");
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
    #[test]
    fn compatibility_latin_letters_are_replaced() {
        for c in ["\u{0132}", "\u{0133}", "\u{0138}", "\u{013F}", "\u{0140}", "\u{0149}"] {
            assert_eq!(sanitize(&format!("Ni{c}o")).segments(), [text("Ni"), replaced(c), text("o")], "{c:?}");
        }
        for raw in [
            "I\u{015F}\u{0131}k Alt\u{0131}n",
            "\u{0141}\u{00F3}d\u{017A}",
            "Ko\u{0161}i\u{0107}",
            "\u{0110}or\u{0111}e",
        ] {
            assert_eq!(sanitize_name(raw).segments(), [text(raw)], "{raw:?}");
        }
    }

    #[test]
    fn letters_carry_at_most_three_combining_marks_in_a_row() {
        for raw in [
            "\u{0E17}\u{0E35}\u{0E48}\u{0E19}\u{0E35}\u{0E48}",
            "\u{0645}\u{064F}\u{062D}\u{064E}\u{0645}\u{0651}\u{064E}\u{062F}",
            "\u{091C}\u{093C}\u{093F}\u{0902}\u{0926}\u{0917}\u{0940}",
            "\u{0938}\u{094D}\u{0924}\u{094D}\u{0930}\u{0940}\u{0902}",
        ] {
            assert_eq!(sanitize_name(raw).segments(), [text(raw)], "{raw:?}");
        }
        for tower in [
            format!("\u{0E01}{}", "\u{0E49}".repeat(4)),
            format!("\u{0E01}{}", "\u{0E48}\u{0E49}".repeat(20)),
            format!("\u{0644}{}", "\u{0651}".repeat(12)),
            format!("\u{0430}{}", "\u{0489}".repeat(5)),
            format!("\u{0915}{}", "\u{0901}".repeat(4)),
        ] {
            assert_eq!(sanitize(&format!("{tower} ok")).segments(), [replaced(&tower), text(" ok")], "{tower:?}");
        }
    }

    #[test]
    fn skeletons_fold_look_alike_names_and_handles() {
        let skeleton = |raw: &str| sanitize_name(raw).skeleton();
        assert_eq!(skeleton("Alice Smith"), skeleton("alice_smith_"));
        assert_eq!(skeleton("a1ice.smith"), skeleton("AIice Smith"));
        assert_eq!(skeleton("B0b Jones"), skeleton("bobjones"));
        assert_eq!(skeleton("Mike"), skeleton("rnike"));
        assert_ne!(skeleton("Alice Smith"), skeleton("Alice Smyth"));
        assert_ne!(skeleton("Alice"), skeleton("Alina"));
        assert!(skeleton("\u{1F60A} ???").is_empty());
        for impostor in [
            "Alic\u{0435}",
            "\u{0410}lice",
            "\u{00C1}lice",
            "Al\u{00EC}ce",
            "\u{FF21}lice",
            "Ali\u{200B}ce",
            "\u{1D400}lice",
            "\u{0391}lice",
        ] {
            assert_eq!(skeleton(impostor), skeleton("Alice"), "{impostor:?}");
        }
        assert_eq!(skeleton("V\u{0131}talik"), skeleton("Vitalik"));
        assert_eq!(skeleton("V\u{00ED}talik"), skeleton("Vitalik"));
        assert_eq!(skeleton("\u{0421}\u{043E}\u{0441}\u{043E}"), skeleton("Coco"));
    }
}
