use std::collections::HashSet;
use std::fmt;
use std::ops::{Range, RangeInclusive};
use std::sync::{LazyLock, Mutex};

use unicode_security::skeleton;
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

/// Names scammers on Telegram pose as most.
const BORROWED_NAMES: [&str; 23] = [
    "Telegram",
    "Durov",
    "Admin",
    "Support",
    "PayPal",
    "Binance",
    "Coinbase",
    "Bybit",
    "Kraken",
    "MetaMask",
    "Trust Wallet",
    "Ledger",
    "Trezor",
    "Tether",
    "Tonkeeper",
    "Revolut",
    "WhatsApp",
    "Instagram",
    "Facebook",
    "Apple",
    "Google",
    "Microsoft",
    "Amazon",
];

/// Lines already logged, so each is reported once.
static REPORTED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);

/// The [`BORROWED_NAMES`] within `folded`, a name from [`SafeText::folded`].
pub fn borrowed_names(folded: &str) -> impl Iterator<Item = &'static str> + '_ {
    static FOLDED: LazyLock<Vec<(&str, String)>> =
        LazyLock::new(|| BORROWED_NAMES.iter().map(|&name| (name, sanitize(name).folded())).collect());
    FOLDED.iter().filter(move |(_, borrowed)| folded.contains(borrowed.as_str())).map(|&(name, _)| name)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    NotAllowed,
    /// A `letter` from another alphabet than the `first` letter of its word, or of its name.
    MixedAlphabets {
        letter: &'static str,
        first: &'static str,
    },
    /// In a name, passes for `like` where it stands, such as the capital `I` in `AIma`; shown as `shown`, which does not.
    LooksLike {
        like: &'static str,
        shown: &'static str,
    },
    /// In a name, a `letter` that looks just like a plain Latin letter or digit, such as Cyrillic `а`: marked in Latin
    /// words, and in other words made only of such letters, which read as Latin.
    Twin {
        letter: &'static str,
    },
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::NotAllowed => f.write_str("suspicious characters"),
            Reason::MixedAlphabets { letter, first } => write!(f, "{letter} in {first} text"),
            Reason::LooksLike { like, .. } => write!(f, "looks like {like}"),
            Reason::Twin { letter } => write!(f, "{letter} that looks like plain Latin"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    Text(String),
    Emoji(&'static str),
    /// Shown as `?`, or marked as its [`Reason`] says; the raw grapheme is only listed in warnings.
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
            Segment::Replaced { reason: Reason::LooksLike { shown, .. }, .. } => shown,
            Segment::Replaced { grapheme, reason: Reason::Twin { .. } } => grapheme,
            Segment::Replaced { .. } => "?",
        }
    }

    fn raw(&self) -> &str {
        match self {
            Segment::Replaced { grapheme, .. } => grapheme,
            segment => segment.as_str(),
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

    /// The texts one after another, each keeping its replaced and marked graphemes.
    pub fn concat(parts: &[&SafeText]) -> SafeText {
        SafeText(parts.iter().flat_map(|part| part.0.iter().cloned()).collect())
    }

    /// What the text looks like, for telling names that could be mistaken for each other: the Unicode (UTS 39)
    /// skeleton of the original text, ignoring case, the dot on `i`, and everything but letters and digits.
    pub fn folded(&self) -> String {
        let raw: String = self.0.iter().map(Segment::raw).collect();
        let look = |text: &str| skeleton(text).collect::<String>().to_lowercase().replace('i', "l");
        look(&look(&raw)).chars().filter(|c| c.is_alphanumeric()).collect()
    }

    /// Byte ranges of the replaced and marked graphemes within the [`Display`](fmt::Display) output.
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
/// so a lone foreign word such as the `Ω` in `Alex Ω` is replaced too, and what passes for something else where it
/// stands is marked, as [`mark_lookalikes`] says.
pub fn sanitize_name(raw: &str) -> SafeText {
    let SafeText(segments) = sanitize_with(raw, false);
    let mut marked = Vec::new();
    for segment in segments {
        match segment {
            Segment::Text(text) => mark_lookalikes(&text, &mut marked),
            segment => marked.push(segment),
        }
    }
    SafeText(marked)
}

fn sanitize_with(raw: &str, per_word: bool) -> SafeText {
    let mut segments = Vec::new();
    let mut first = None;
    for grapheme in raw.graphemes(true) {
        let is_space = grapheme.chars().all(|c| c.is_ascii_whitespace());
        if per_word && is_space {
            first = None;
        }
        match if is_space { None } else { special(grapheme, &mut first) } {
            Some(segment) => segments.push(segment),
            None => push_text(&mut segments, grapheme),
        }
    }
    SafeText(segments)
}

fn push_text(segments: &mut Vec<Segment>, text: &str) {
    match segments.last_mut() {
        Some(Segment::Text(last)) => last.push_str(text),
        _ => segments.push(Segment::Text(text.into())),
    }
}

/// Marks a capital `I` after the start of a word with lowercase letters, a lowercase `l` starting a word or in one
/// without lowercase letters, `0` and `1` beside letters and `O`, `I` and `l` beside digits, a `|` touching a word,
/// `rn`, which reads as `m`, and [`Reason::Twin`] letters. Errs on the side of caution, so names such as `McIntyre`
/// are marked too.
fn mark_lookalikes(text: &str, segments: &mut Vec<Segment>) {
    let chars: Vec<char> = text.chars().collect();
    let mut marks = vec![None; chars.len()];
    let mut start = 0;
    for run in chars.chunk_by(|a, b| a.is_alphanumeric() == b.is_alphanumeric()) {
        let end = start + run.len();
        if run[0].is_alphanumeric() {
            mark_word(run, &mut marks[start..end]);
        }
        for (ix, &c) in run.iter().enumerate() {
            if c == '|' && ((ix == 0 && start > 0) || (ix + 1 == run.len() && end < chars.len())) {
                marks[start + ix] = Some(Reason::LooksLike { like: "l", shown: "|" });
            }
        }
        start = end;
    }
    let mut ix = 0;
    while let Some(&c) = chars.get(ix) {
        match marks[ix] {
            Some(reason) => {
                let len = match reason {
                    Reason::LooksLike { shown, .. } => shown.len(),
                    _ => 1,
                };
                segments.push(replace(&chars[ix..ix + len].iter().collect::<String>(), reason));
                ix += len;
            }
            None => {
                push_text(segments, c.encode_utf8(&mut [0; 4]));
                ix += 1;
            }
        }
    }
}

/// [`mark_lookalikes`] for one run of letters and digits.
fn mark_word(word: &[char], marks: &mut [Option<Reason>]) {
    let lowercase = word.iter().any(|&c| c.is_lowercase() && c != 'l');
    let twins: Vec<_> = word.iter().map(|&c| twin(c)).collect();
    let disguised = word.iter().zip(&twins).all(|(c, twin)| twin.is_some() || !c.is_alphabetic());
    for (ix, &c) in word.iter().enumerate() {
        if let Some(letter) = twins[ix].filter(|&letter| letter == "Latin" || disguised) {
            marks[ix] = Some(Reason::Twin { letter });
            continue;
        }
        let near: Vec<char> =
            [ix.checked_sub(1), Some(ix + 1)].into_iter().flatten().filter_map(|n| word.get(n).copied()).collect();
        let (letters, digits) = (near.iter().any(|c| c.is_alphabetic()), near.iter().any(char::is_ascii_digit));
        let looks = match c {
            '0' if letters && !digits => Some(("O", "0")),
            '1' if letters && !digits => Some(("l", "1")),
            'O' if digits && !letters => Some(("0", "o")),
            'I' if digits && !letters => Some(("1", "i")),
            'l' if digits && !letters => Some(("1", "L")),
            'I' if ix > 0 && lowercase => Some(("l", "i")),
            'l' if ix == 0 || !lowercase => Some(("I", "L")),
            'r' if word.get(ix + 1) == Some(&'n') => Some(("m", "rn")),
            _ => None,
        };
        if let Some((like, shown)) = looks {
            marks[ix] = Some(Reason::LooksLike { like, shown });
        }
    }
}

/// The alphabet of `c` when it is a letter that Unicode lists as looking just like a plain Latin letter or digit.
fn twin(c: char) -> Option<&'static str> {
    let text = &c.to_string();
    let letter = alphabet(text)?;
    (!c.is_ascii() && skeleton(text).all(|t| t.is_ascii_alphanumeric())).then_some(letter)
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

    fn looks(grapheme: &str, like: &'static str, shown: &'static str) -> Segment {
        Replaced { grapheme: grapheme.into(), reason: Reason::LooksLike { like, shown } }
    }

    #[test]
    fn bars_that_pass_for_another_letter_in_names_show_in_the_other_case() {
        assert_eq!(sanitize_name("AIma").segments(), [text("A"), looks("I", "l", "i"), text("ma")]);
        assert_eq!(sanitize_name("ADMlN").segments(), [text("ADM"), looks("l", "I", "L"), text("N")]);
        assert_eq!(sanitize_name("lnstagram").segments(), [looks("l", "I", "L"), text("nstagram")]);
        assert_eq!(sanitize_name("PayPaI paypaI").to_string(), "PayPai paypai");
        assert_eq!(sanitize_name("McIntyre").to_string(), "Mcintyre", "caution over the odd real name");
        assert_eq!(sanitize_name("Al").to_string(), "AL");
        for name in ["Alma", "ALMA", "Ivan", "IKEA", "Bill", "BILL", "AI", "Henry II", "alice_smith"] {
            assert_eq!(sanitize_name(name).segments(), [text(name)], "{name:?}");
        }
    }

    #[test]
    fn digits_among_letters_and_letters_among_digits_are_marked_in_names() {
        assert_eq!(sanitize_name("B0b").segments(), [text("B"), looks("0", "O", "0"), text("b")]);
        assert_eq!(sanitize_name("A1ice Pau1").to_string(), "A1ice Pau1");
        assert_eq!(sanitize_name("A1ice Pau1").replaced_ranges(), [1..2, 9..10]);
        assert_eq!(
            sanitize_name("2O24 l23").segments(),
            [text("2"), looks("O", "0", "o"), text("24 "), looks("l", "1", "L"), text("23")]
        );
        for name in ["Agent007", "john1990", "Win10", "Room 101"] {
            assert_eq!(sanitize_name(name).segments(), [text(name)], "{name:?}");
        }
    }

    #[test]
    fn bars_touching_words_and_rn_are_marked_in_names() {
        assert_eq!(sanitize_name("A|ma").segments(), [text("A"), looks("|", "l", "|"), text("ma")]);
        assert_eq!(sanitize_name("Bob | Work").segments(), [text("Bob | Work")]);
        assert_eq!(sanitize_name("rnike").segments(), [looks("rn", "m", "rn"), text("ike")]);
        assert_eq!(sanitize_name("Bernard").segments(), [text("Be"), looks("rn", "m", "rn"), text("ard")]);
    }

    #[test]
    fn words_of_letters_that_all_look_latin_are_marked_in_names() {
        let twin = |grapheme: &str, letter| Replaced { grapheme: grapheme.into(), reason: Reason::Twin { letter } };
        let app = "\u{0410}\u{0440}\u{0440}";
        assert_eq!(
            sanitize_name(app).segments(),
            [twin("\u{0410}", "Cyrillic"), twin("\u{0440}", "Cyrillic"), twin("\u{0440}", "Cyrillic")]
        );
        assert_eq!(sanitize_name(app).to_string(), app, "shown, but marked");
        assert_eq!(sanitize_name("\u{041C}\u{041E}\u{0421}\u{041A}\u{0412}\u{0410}").replaced().count(), 6);
        assert_eq!(sanitize_name("Al\u{0131}ce").segments(), [text("Al"), twin("\u{0131}", "Latin"), text("ce")]);
        assert_eq!(Reason::Twin { letter: "Cyrillic" }.to_string(), "Cyrillic that looks like plain Latin");
        for name in ["Москва", "Андрей Смоляков", "Café Müller"] {
            assert_eq!(sanitize_name(name).segments(), [text(name)], "{name:?}");
        }
    }

    #[test]
    fn messages_are_not_marked() {
        assert_eq!(
            sanitize("AIma B0b rnike lol \u{0410}\u{0440}\u{0440}").to_string(),
            "AIma B0b rnike lol \u{0410}\u{0440}\u{0440}"
        );
    }

    #[test]
    fn folding_matches_names_that_look_alike() {
        let folded = |name: &str| sanitize_name(name).folded();
        assert_eq!(folded("Alice Smith"), folded("alice_smith_"));
        assert_eq!(folded("a1ice.smith"), folded("AIice Smith"));
        assert_eq!(folded("B0b Jones"), folded("bobjones"));
        assert_eq!(folded("Mike"), folded("rnike"));
        assert_eq!(folded("\u{0412}\u{041E}\u{0412}"), folded("Bob"), "Cyrillic ВОВ");
        assert_eq!(folded("P\u{0430}ypal"), folded("Paypal"), "the replaced Cyrillic а still counts");
        assert_ne!(folded("Alice Smith"), folded("Alice Smyth"));
        assert_eq!(folded("\u{1F60A} ?!"), "");
    }

    #[test]
    fn borrowed_names_are_found_however_they_are_disguised() {
        let borrowed = |name: &str| borrowed_names(&sanitize_name(name).folded()).collect::<Vec<_>>();
        assert_eq!(borrowed("PayPaI Support \u{1F4B0}"), ["Support", "PayPal"]);
        assert_eq!(borrowed("\u{0420}\u{0430}\u{0443}\u{0420}\u{0430}\u{04CF}"), ["PayPal"], "Cyrillic РауРаӏ");
        assert_eq!(borrowed("trust_wa11et"), ["Trust Wallet"]);
        assert_eq!(borrowed("Te1egrarn"), ["Telegram"]);
        assert!(borrowed("Alice Smith").is_empty());
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
