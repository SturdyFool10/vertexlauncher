//! Wording helpers for notifications that describe a changing set of things ("two new capes: A
//! and B"), so every stacked notification reads the same way.
//!
//! Everything here is a pure function over plain strings and numbers, with no UI or notification
//! state, so it is cheap to reuse and to test. Roughly:
//!
//! - **Counts**: [`count_word`], [`count_or_digits`], [`thousands`], [`ordinal_number`],
//!   [`ordinal_word`], [`times_word`], [`vague_quantity`].
//! - **Nouns**: [`pluralize`], [`pluralize_for`], [`counted`], [`counted_digits`],
//!   [`a_or_count`], [`article_for`], [`with_article`], [`possessive`].
//! - **Verb agreement**: [`is_or_are`], [`was_or_were`], [`has_or_have`], [`does_or_do`],
//!   [`this_or_these`], [`it_or_they`].
//! - **Lists**: [`join`] with a [`ListStyle`], [`join_natural`], [`join_or`], [`join_limited`],
//!   [`join_with_repeats`], [`group_repeats`], [`dedupe`].
//! - **Text**: [`capitalize_first`], [`lowercase_first`], [`sentence`], [`quoted`],
//!   [`quoted_all`].
//! - **Time and size**: [`duration_words`], [`duration_approx`], [`time_ago`], [`bytes_words`].
//!
//! # Languages
//!
//! The mechanics that differ between languages (counts, number grouping, ordinals, list joining, durations, sizes, plural
//! selection) are data in the [`locale`] module (from `vertex_i18n`), and the free functions here follow the launcher's
//! current interface language: [`join_natural`], [`thousands`], [`time_ago`], [`bytes_words`] and the rest speak whatever
//! language is selected. Grammar that belongs to the words themselves (noun plurals, articles, possessives, verb
//! agreement) cannot be derived by rule across languages, so the helpers for it ([`pluralize`], [`with_article`],
//! [`is_or_are`], ...) are English-only, for English source text. A message that needs that grammar in every language is
//! written with `vertex_i18n::trn!`, so each language's catalog supplies its own plural forms.

use std::time::Duration;

/// The per-language mechanics; see the module docs.
pub mod locale {
    pub use vertex_i18n::locale::*;
}

pub use vertex_i18n::{ListKind, Locale, MAX_COUNT_WORD, PluralCategory, PluralForms, PluralRule};

// ---------------------------------------------------------------------------------------------
// Counts
// ---------------------------------------------------------------------------------------------

/// A count as a word: "one" through "ten", then "several" so a big number never reads as a
/// precise claim. In the interface language.
pub fn count_word(count: usize) -> String {
    Locale::current().count_word(count)
}

/// A count as a word up to ten, and as exact digits (with thousands separators) above that. Use
/// it when the number matters ("12 accounts"), unlike [`count_word`].
pub fn count_or_digits(count: usize) -> String {
    Locale::current().count_or_digits(count)
}

/// Digits grouped in threes: `12345` becomes "12,345". In the interface language.
pub fn thousands(value: u64) -> String {
    Locale::current().thousands(value)
}

/// An ordinal in digits: "1st", "2nd", "3rd", "4th", "11th", "22nd", "113th".
pub fn ordinal_number(value: u64) -> String {
    Locale::current().ordinal_number(value)
}

/// An ordinal as a word up to "tenth", and in digits beyond that.
pub fn ordinal_word(value: u64) -> String {
    use vertex_i18n::tr_ctx;
    match value {
        1 => tr_ctx!("ordinal", "first"),
        2 => tr_ctx!("ordinal", "second"),
        3 => tr_ctx!("ordinal", "third"),
        4 => tr_ctx!("ordinal", "fourth"),
        5 => tr_ctx!("ordinal", "fifth"),
        6 => tr_ctx!("ordinal", "sixth"),
        7 => tr_ctx!("ordinal", "seventh"),
        8 => tr_ctx!("ordinal", "eighth"),
        9 => tr_ctx!("ordinal", "ninth"),
        10 => tr_ctx!("ordinal", "tenth"),
        _ => return ordinal_number(value),
    }
    .into_owned()
}

/// How many times something happened: "never", "once", "twice", "three times", ... "several
/// times".
pub fn times_word(count: usize) -> String {
    use vertex_i18n::tr;
    match count {
        0 => tr!("never"),
        1 => tr!("once"),
        2 => tr!("twice"),
        n => tr!("{count} times", count = count_word(n)),
    }
    .into_owned()
}

/// A rough, human way to say a quantity, for when the exact number is noise: "no", "one", "a
/// couple of", "a few", "several", "many", "dozens of", "hundreds of", "thousands of".
pub fn vague_quantity(count: usize) -> String {
    use vertex_i18n::tr_ctx;
    match count {
        0 => tr_ctx!("quantity", "no"),
        1 => tr_ctx!("quantity", "one"),
        2 => tr_ctx!("quantity", "a couple of"),
        3..=5 => tr_ctx!("quantity", "a few"),
        6..=20 => tr_ctx!("quantity", "several"),
        21..=35 => tr_ctx!("quantity", "many"),
        36..=99 => tr_ctx!("quantity", "dozens of"),
        100..=999 => tr_ctx!("quantity", "hundreds of"),
        _ => tr_ctx!("quantity", "thousands of"),
    }
    .into_owned()
}

// ---------------------------------------------------------------------------------------------
// Nouns
// ---------------------------------------------------------------------------------------------

/// Nouns whose plural is the singular.
const UNCHANGING_NOUNS: [&str; 18] = [
    "sheep",
    "fish",
    "deer",
    "series",
    "species",
    "news",
    "moose",
    "aircraft",
    "offspring",
    "information",
    "equipment",
    "software",
    "hardware",
    "feedback",
    "progress",
    "music",
    "data",
    "metadata",
];

const IRREGULAR_PLURALS: [(&str, &str); 28] = [
    ("man", "men"),
    ("woman", "women"),
    ("child", "children"),
    ("person", "people"),
    ("foot", "feet"),
    ("tooth", "teeth"),
    ("mouse", "mice"),
    ("goose", "geese"),
    ("ox", "oxen"),
    ("leaf", "leaves"),
    ("life", "lives"),
    ("knife", "knives"),
    ("wife", "wives"),
    ("half", "halves"),
    ("calf", "calves"),
    ("wolf", "wolves"),
    ("shelf", "shelves"),
    ("thief", "thieves"),
    ("loaf", "loaves"),
    ("self", "selves"),
    ("elf", "elves"),
    ("index", "indices"),
    ("matrix", "matrices"),
    ("vertex", "vertices"),
    ("analysis", "analyses"),
    ("criterion", "criteria"),
    ("phenomenon", "phenomena"),
    ("quiz", "quizzes"),
];

/// Nouns ending in a consonant plus "o" that take "es" (most others, like "photo", take "s").
const O_TAKES_ES: [&str; 6] = ["hero", "potato", "tomato", "echo", "veto", "torpedo"];

/// The plural of an English noun. Handles the regular rules (-s/-x/-z/-ch/-sh take "es",
/// consonant plus "y" becomes "ies"), common irregulars ("child" to "children", "leaf" to
/// "leaves"), nouns that do not change ("sheep", "data"), acronyms ("URL" to "URLs"), and
/// capitalization. For a phrase, only the last word is pluralized: "new cape" becomes "new
/// capes".
pub fn pluralize(noun: &str) -> String {
    let trimmed = noun.trim_end();
    if trimmed.is_empty() {
        return noun.to_owned();
    }
    match trimmed.rfind(char::is_whitespace) {
        Some(split) => {
            let (head, tail) = trimmed.split_at(split + 1);
            format!("{head}{}", pluralize_word(tail))
        }
        None => pluralize_word(trimmed),
    }
}

fn pluralize_word(word: &str) -> String {
    let lower = word.to_lowercase();
    if UNCHANGING_NOUNS.contains(&lower.as_str()) {
        return word.to_owned();
    }
    if let Some((_, plural)) = IRREGULAR_PLURALS
        .iter()
        .find(|(singular, _)| *singular == lower)
    {
        return match_leading_case(word, plural);
    }
    let letters: Vec<char> = word.chars().filter(|c| c.is_alphabetic()).collect();
    let is_acronym = letters.len() >= 2 && letters.iter().all(|c| c.is_uppercase());
    if is_acronym {
        return format!("{word}s");
    }

    let ends_with = |suffix: &str| lower.ends_with(suffix);
    let before_last = lower.chars().rev().nth(1);
    let after_consonant = before_last.is_some_and(|c| !"aeiou".contains(c));
    if ends_with("s") || ends_with("x") || ends_with("z") || ends_with("ch") || ends_with("sh") {
        format!("{word}es")
    } else if ends_with("y") && after_consonant {
        format!("{}ies", &word[..word.len() - 1])
    } else if ends_with("o") && after_consonant && O_TAKES_ES.contains(&lower.as_str()) {
        format!("{word}es")
    } else {
        format!("{word}s")
    }
}

/// `replacement` with the first letter upper-cased when `original` starts with a capital.
fn match_leading_case(original: &str, replacement: &str) -> String {
    if original.chars().next().is_some_and(char::is_uppercase) {
        capitalize_first(replacement)
    } else {
        replacement.to_owned()
    }
}

/// `noun` when `count` is exactly one, otherwise its plural: `pluralize_for(2, "cape")` is
/// "capes".
pub fn pluralize_for(count: usize, noun: &str) -> String {
    if count == 1 {
        noun.to_owned()
    } else {
        pluralize(noun)
    }
}

/// A counted noun with the count spelled out: "no capes", "one cape", "three capes", "several
/// capes" (see [`count_word`]).
pub fn counted(count: usize, noun: &str) -> String {
    match count {
        0 => format!("no {}", pluralize(noun)),
        n => format!("{} {}", count_word(n), pluralize_for(n, noun)),
    }
}

/// A counted noun with the exact count in digits: "0 capes", "1 cape", "12 capes", "1,200 capes".
pub fn counted_digits(count: usize, noun: &str) -> String {
    format!("{} {}", thousands(count as u64), pluralize_for(count, noun))
}

/// "a cape" for exactly one, otherwise [`counted`]: "no capes", "two capes", "several capes".
/// Pair it with an adjective by putting it in the noun: `a_or_count(3, "new cape")` is "three new
/// capes", and `a_or_count(1, "new cape")` is "a new cape".
pub fn a_or_count(count: usize, noun: &str) -> String {
    if count == 1 {
        with_article(noun)
    } else {
        counted(count, noun)
    }
}

/// The indefinite article ("a" or "an") that reads right before `phrase`. It goes by sound, not
/// spelling: "an hour", "a unicorn", "an unimportant detail", "a one-off", "an FAQ", "an 8",
/// "a URL".
pub fn article_for(phrase: &str) -> &'static str {
    let first_word = phrase
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .split(|c: char| c.is_whitespace())
        .next()
        .unwrap_or("");
    if first_word.is_empty() {
        return "a";
    }

    // Numbers: "an 8", "an 11", "an 18", "an 80", but "a 1", "a 7", "a 110".
    if first_word.starts_with(|c: char| c.is_ascii_digit()) {
        let digits: String = first_word
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        return if digits.starts_with('8') || digits == "11" || digits == "18" {
            "an"
        } else {
            "a"
        };
    }

    // Short all-caps words are read letter by letter: "an FAQ", "an HTML", "a URL".
    let letters: Vec<char> = first_word.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() >= 2 && letters.len() <= 5 && letters.iter().all(|c| c.is_uppercase()) {
        return if "AEFHILMNORSX".contains(letters[0]) {
            "an"
        } else {
            "a"
        };
    }

    let lower = first_word.to_lowercase();
    const SILENT_H: [&str; 5] = ["hour", "honest", "honor", "honour", "heir"];
    if SILENT_H.iter().any(|prefix| lower.starts_with(prefix)) {
        return "an";
    }
    // Vowel letters that sound like a consonant ("yoo", "wun").
    const CONSONANT_SOUND: [&str; 11] = [
        "one", "once", "eu", "ewe", "use", "usu", "util", "uter", "ubiq", "usa", "ura",
    ];
    if CONSONANT_SOUND
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        return "a";
    }
    // "uni..." is "yoo-ni" in unicorn, union, unit, but "uhn-i" in unimportant, uninstall.
    if lower.starts_with("uni") {
        const YOO_NI: [&str; 7] = ["unic", "unif", "unio", "unit", "univ", "uniq", "unix"];
        return if YOO_NI.iter().any(|prefix| lower.starts_with(prefix)) {
            "a"
        } else {
            "an"
        };
    }
    if lower.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

/// `phrase` with its indefinite article: "a cape", "an hour", "an old cape".
pub fn with_article(phrase: &str) -> String {
    format!("{} {phrase}", article_for(phrase))
}

/// The possessive of a name or noun: "Steve's", and just an apostrophe after an "s": "James'",
/// "capes'".
pub fn possessive(name: &str) -> String {
    let name = name.trim_end();
    if name.ends_with(['s', 'S']) {
        format!("{name}'")
    } else {
        format!("{name}'s")
    }
}

// ---------------------------------------------------------------------------------------------
// Verb agreement
// ---------------------------------------------------------------------------------------------

/// "is" for exactly one, otherwise "are".
pub fn is_or_are(count: usize) -> &'static str {
    if count == 1 { "is" } else { "are" }
}

/// "was" for exactly one, otherwise "were".
pub fn was_or_were(count: usize) -> &'static str {
    if count == 1 { "was" } else { "were" }
}

/// "has" for exactly one, otherwise "have".
pub fn has_or_have(count: usize) -> &'static str {
    if count == 1 { "has" } else { "have" }
}

/// "does" for exactly one, otherwise "do".
pub fn does_or_do(count: usize) -> &'static str {
    if count == 1 { "does" } else { "do" }
}

/// "this" for exactly one, otherwise "these".
pub fn this_or_these(count: usize) -> &'static str {
    if count == 1 { "this" } else { "these" }
}

/// "it" for exactly one, otherwise "they".
pub fn it_or_they(count: usize) -> &'static str {
    if count == 1 { "it" } else { "they" }
}

// ---------------------------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------------------------

/// How [`join`] words a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListStyle {
    /// The word before the last item: "and", "or", "plus".
    pub conjunction: &'static str,
    /// Whether three or more items get a comma before the conjunction: "A, B, and C".
    pub oxford_comma: bool,
    /// Show at most this many items, then summarize the rest as "and N more". A single leftover
    /// item is shown instead, since "and 1 more" is just a worse way to say its name.
    pub max_items: Option<usize>,
    /// What the hidden items are called: "more" ("and 4 more") or "others" ("and 4 others").
    pub remainder_label: &'static str,
}

impl ListStyle {
    /// "A, B, and C".
    pub const AND: Self = Self {
        conjunction: "and",
        oxford_comma: true,
        max_items: None,
        remainder_label: "more",
    };

    /// "A, B, or C".
    pub const OR: Self = Self {
        conjunction: "or",
        ..Self::AND
    };

    /// Shows at most `max` items.
    pub const fn limited(self, max: usize) -> Self {
        Self {
            max_items: Some(max),
            ..self
        }
    }

    /// Drops the comma before the conjunction: "A, B and C".
    pub const fn without_oxford_comma(self) -> Self {
        Self {
            oxford_comma: false,
            ..self
        }
    }

    /// Names the hidden items: `"others"` gives "and 4 others".
    pub const fn remainder(self, label: &'static str) -> Self {
        Self {
            remainder_label: label,
            ..self
        }
    }
}

impl Default for ListStyle {
    fn default() -> Self {
        Self::AND
    }
}

/// Items as an English list, worded by `style`. Empty input gives an empty string.
pub fn join<S: AsRef<str>>(items: &[S], style: ListStyle) -> String {
    let mut parts: Vec<String> = items.iter().map(|item| item.as_ref().to_owned()).collect();
    if let Some(max) = style.max_items
        && parts.len() > max.saturating_add(1)
    {
        let hidden = parts.len() - max;
        parts.truncate(max);
        parts.push(format!("{hidden} {}", style.remainder_label));
    }

    match parts.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [first, second] => format!("{first} {} {second}", style.conjunction),
        [init @ .., last] => {
            let comma = if style.oxford_comma { "," } else { "" };
            format!("{}{comma} {} {last}", init.join(", "), style.conjunction)
        }
    }
}

/// Items as a list in the interface language: "A", "A and B", "A, B, and C".
pub fn join_natural<S: AsRef<str>>(items: &[S]) -> String {
    Locale::current().join(items, ListKind::And)
}

/// Items as an either/or list in the interface language: "A", "A or B", "A, B, or C".
pub fn join_or<S: AsRef<str>>(items: &[S]) -> String {
    Locale::current().join(items, ListKind::Or)
}

/// Like [`join_natural`], but shows at most `max` items and folds the rest into "and N more".
pub fn join_limited<S: AsRef<str>>(items: &[S], max: usize) -> String {
    Locale::current().join_limited(items, ListKind::And, max)
}

/// Each distinct item with how many times it appears, in order of first appearance.
pub fn group_repeats<S: AsRef<str>>(items: &[S]) -> Vec<(String, usize)> {
    let mut groups: Vec<(String, usize)> = Vec::new();
    for item in items {
        let item = item.as_ref();
        match groups.iter_mut().find(|(seen, _)| seen == item) {
            Some((_, count)) => *count += 1,
            None => groups.push((item.to_owned(), 1)),
        }
    }
    groups
}

/// Distinct items, in order of first appearance.
pub fn dedupe<S: AsRef<str>>(items: &[S]) -> Vec<String> {
    group_repeats(items)
        .into_iter()
        .map(|(item, _)| item)
        .collect()
}

/// A list that notes repeats instead of listing them again: `["A", "B", "A", "A"]` becomes "A
/// (three times) and B".
pub fn join_with_repeats<S: AsRef<str>>(items: &[S], style: ListStyle) -> String {
    let described: Vec<String> = group_repeats(items)
        .into_iter()
        .map(|(item, count)| {
            if count == 1 {
                item
            } else {
                format!("{item} ({})", times_word(count))
            }
        })
        .collect();
    join(&described, style)
}

// ---------------------------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------------------------

/// `text` with its first letter upper-cased.
pub fn capitalize_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `text` with its first letter lower-cased, for dropping a sentence into the middle of another.
pub fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `text` as a sentence: trimmed, capitalized, and ended with a period unless it already ends in
/// punctuation.
pub fn sentence(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let capitalized = capitalize_first(trimmed);
    if capitalized.ends_with(['.', '!', '?', '…']) {
        capitalized
    } else {
        format!("{capitalized}.")
    }
}

/// `text` in typographic double quotes.
pub fn quoted(text: &str) -> String {
    format!("\u{201c}{text}\u{201d}")
}

/// Every item in typographic double quotes, ready for [`join`].
pub fn quoted_all<S: AsRef<str>>(items: &[S]) -> Vec<String> {
    items.iter().map(|item| quoted(item.as_ref())).collect()
}

// ---------------------------------------------------------------------------------------------
// Time and size
// ---------------------------------------------------------------------------------------------

/// A duration with its two largest non-zero units: "less than a second", "45 seconds", "2
/// minutes and 5 seconds", "1 hour and 30 minutes", "3 days and 4 hours". English; see
/// [`Locale::duration_words`].
pub fn duration_words(duration: Duration) -> String {
    Locale::current().duration_words(duration)
}

/// A duration as one rounded unit, with "about" when rounding changed it: "less than a second",
/// "45 seconds", "about 5 minutes", "2 hours", "about 3 days". English; see
/// [`Locale::duration_approx`].
pub fn duration_approx(duration: Duration) -> String {
    Locale::current().duration_approx(duration)
}

/// How long ago something happened: "just now" within five seconds, then "45 seconds ago",
/// "about 5 minutes ago", "2 hours ago". In the interface language.
pub fn time_ago(elapsed: Duration) -> String {
    Locale::current().time_ago(elapsed)
}

/// A byte count for people: "0 bytes", "1 byte", "512 bytes", "1.5 KB", "20 MB", "3.2 GB".
/// Units step by 1024. In the interface language.
pub fn bytes_words(bytes: u64) -> String {
    Locale::current().bytes_words(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn counts_are_words_until_ten_then_several() {
        assert_eq!(count_word(2), "two");
        assert_eq!(count_word(10), "ten");
        assert_eq!(count_word(11), "several");
        assert_eq!(MAX_COUNT_WORD, 10);
    }

    #[test]
    fn exact_counts_switch_to_digits_with_separators() {
        assert_eq!(count_or_digits(7), "seven");
        assert_eq!(count_or_digits(11), "11");
        assert_eq!(count_or_digits(12_345), "12,345");
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn ordinals() {
        for (value, expected) in [
            (1, "1st"),
            (2, "2nd"),
            (3, "3rd"),
            (4, "4th"),
            (11, "11th"),
            (12, "12th"),
            (13, "13th"),
            (21, "21st"),
            (22, "22nd"),
            (101, "101st"),
            (111, "111th"),
            (113, "113th"),
        ] {
            assert_eq!(ordinal_number(value), expected);
        }
        assert_eq!(ordinal_word(1), "first");
        assert_eq!(ordinal_word(10), "tenth");
        assert_eq!(ordinal_word(11), "11th");
        assert_eq!(ordinal_word(0), "0th");
    }

    #[test]
    fn times_and_vague_quantities() {
        assert_eq!(times_word(0), "never");
        assert_eq!(times_word(1), "once");
        assert_eq!(times_word(2), "twice");
        assert_eq!(times_word(3), "three times");
        assert_eq!(times_word(40), "several times");
        assert_eq!(vague_quantity(0), "no");
        assert_eq!(vague_quantity(2), "a couple of");
        assert_eq!(vague_quantity(4), "a few");
        assert_eq!(vague_quantity(12), "several");
        assert_eq!(vague_quantity(30), "many");
        assert_eq!(vague_quantity(60), "dozens of");
        assert_eq!(vague_quantity(500), "hundreds of");
        assert_eq!(vague_quantity(5_000), "thousands of");
    }

    #[test]
    fn regular_plurals() {
        for (singular, plural) in [
            ("cape", "capes"),
            ("screenshot", "screenshots"),
            ("class", "classes"),
            ("box", "boxes"),
            ("buzz", "buzzes"),
            ("match", "matches"),
            ("dash", "dashes"),
            ("entry", "entries"),
            ("key", "keys"),
            ("day", "days"),
            ("photo", "photos"),
            ("hero", "heroes"),
            ("potato", "potatoes"),
        ] {
            assert_eq!(pluralize(singular), plural, "{singular}");
        }
    }

    #[test]
    fn irregular_and_unchanging_plurals() {
        for (singular, plural) in [
            ("child", "children"),
            ("person", "people"),
            ("leaf", "leaves"),
            ("knife", "knives"),
            ("vertex", "vertices"),
            ("quiz", "quizzes"),
            ("sheep", "sheep"),
            ("data", "data"),
            ("software", "software"),
        ] {
            assert_eq!(pluralize(singular), plural, "{singular}");
        }
    }

    #[test]
    fn plurals_keep_case_and_pluralize_the_last_word() {
        assert_eq!(pluralize("Child"), "Children");
        assert_eq!(pluralize("Vertex"), "Vertices");
        assert_eq!(pluralize("URL"), "URLs");
        assert_eq!(pluralize("new cape"), "new capes");
        assert_eq!(pluralize("login token"), "login tokens");
        assert_eq!(pluralize(""), "");
    }

    #[test]
    fn counted_nouns() {
        assert_eq!(pluralize_for(1, "cape"), "cape");
        assert_eq!(pluralize_for(0, "cape"), "capes");
        assert_eq!(counted(0, "cape"), "no capes");
        assert_eq!(counted(1, "cape"), "one cape");
        assert_eq!(counted(3, "cape"), "three capes");
        assert_eq!(counted(11, "cape"), "several capes");
        assert_eq!(counted_digits(0, "cape"), "0 capes");
        assert_eq!(counted_digits(1, "cape"), "1 cape");
        assert_eq!(counted_digits(1_200, "cape"), "1,200 capes");
        assert_eq!(a_or_count(1, "new cape"), "a new cape");
        assert_eq!(a_or_count(2, "new cape"), "two new capes");
        assert_eq!(a_or_count(0, "new cape"), "no new capes");
        assert_eq!(a_or_count(1, "old cape"), "an old cape");
    }

    #[test]
    fn articles_follow_sound_not_spelling() {
        for (phrase, article) in [
            ("cape", "a"),
            ("account", "an"),
            ("hour", "an"),
            ("honest mistake", "an"),
            ("hero", "a"),
            ("unicorn", "a"),
            ("university", "a"),
            ("unimportant detail", "an"),
            ("user", "a"),
            ("one-off", "a"),
            ("umbrella", "an"),
            ("European cape", "a"),
            ("FAQ", "an"),
            ("HTML page", "an"),
            ("URL", "a"),
            ("8 capes", "an"),
            ("11 capes", "an"),
            ("110 capes", "a"),
            ("7 capes", "a"),
            ("\"quoted\"", "a"),
            ("", "a"),
        ] {
            assert_eq!(article_for(phrase), article, "{phrase:?}");
        }
        assert_eq!(with_article("account"), "an account");
    }

    #[test]
    fn possessives() {
        assert_eq!(possessive("Steve"), "Steve's");
        assert_eq!(possessive("James"), "James'");
        assert_eq!(possessive("capes"), "capes'");
    }

    #[test]
    fn verb_agreement() {
        assert_eq!(is_or_are(1), "is");
        assert_eq!(is_or_are(0), "are");
        assert_eq!(is_or_are(3), "are");
        assert_eq!(was_or_were(1), "was");
        assert_eq!(was_or_were(2), "were");
        assert_eq!(has_or_have(1), "has");
        assert_eq!(has_or_have(2), "have");
        assert_eq!(does_or_do(1), "does");
        assert_eq!(does_or_do(2), "do");
        assert_eq!(this_or_these(1), "this");
        assert_eq!(this_or_these(2), "these");
        assert_eq!(it_or_they(1), "it");
        assert_eq!(it_or_they(2), "they");
    }

    #[test]
    fn lists_read_naturally() {
        assert_eq!(join_natural(&names(&[])), "");
        assert_eq!(join_natural(&names(&["A"])), "A");
        assert_eq!(join_natural(&names(&["A", "B"])), "A and B");
        assert_eq!(join_natural(&names(&["A", "B", "C"])), "A, B, and C");
        assert_eq!(join_or(&["A", "B", "C"]), "A, B, or C");
        assert_eq!(
            join(&["A", "B", "C"], ListStyle::AND.without_oxford_comma()),
            "A, B and C"
        );
        assert_eq!(
            join(
                &["A", "B", "C"],
                ListStyle {
                    conjunction: "plus",
                    ..ListStyle::AND
                }
            ),
            "A, B, plus C"
        );
    }

    #[test]
    fn long_lists_can_be_cut_short() {
        let items = names(&["A", "B", "C", "D", "E", "F"]);
        assert_eq!(join_limited(&items, 3), "A, B, C, and 3 more");
        assert_eq!(
            join(&items, ListStyle::AND.limited(2).remainder("others")),
            "A, B, and 4 others"
        );
        assert_eq!(
            join_limited(&items[..4], 3),
            "A, B, C, and D",
            "one leftover is named"
        );
        assert_eq!(join_limited(&items[..3], 3), "A, B, and C");
        assert_eq!(join_limited(&items, 0), "6 more");
    }

    #[test]
    fn repeats_are_grouped_and_described() {
        let items = names(&["A", "B", "A", "A", "C"]);
        assert_eq!(
            group_repeats(&items),
            vec![
                ("A".to_owned(), 3),
                ("B".to_owned(), 1),
                ("C".to_owned(), 1)
            ]
        );
        assert_eq!(dedupe(&items), names(&["A", "B", "C"]));
        assert_eq!(
            join_with_repeats(&items, ListStyle::AND),
            "A (three times), B, and C"
        );
        assert_eq!(
            join_with_repeats(&names(&["A", "A"]), ListStyle::AND),
            "A (twice)"
        );
    }

    #[test]
    fn text_helpers() {
        assert_eq!(capitalize_first("steve"), "Steve");
        assert_eq!(capitalize_first(""), "");
        assert_eq!(lowercase_first("Steve"), "steve");
        assert_eq!(sentence("  login renewed "), "Login renewed.");
        assert_eq!(sentence("done!"), "Done!");
        assert_eq!(sentence("really?"), "Really?");
        assert_eq!(sentence("   "), "");
        assert_eq!(quoted("A"), "\u{201c}A\u{201d}");
        assert_eq!(
            join_natural(&quoted_all(&["A", "B"])),
            "\u{201c}A\u{201d} and \u{201c}B\u{201d}"
        );
    }

    #[test]
    fn durations_use_their_two_largest_units() {
        let secs = Duration::from_secs;
        assert_eq!(
            duration_words(Duration::from_millis(400)),
            "less than a second"
        );
        assert_eq!(duration_words(secs(1)), "1 second");
        assert_eq!(duration_words(secs(45)), "45 seconds");
        assert_eq!(duration_words(secs(125)), "2 minutes and 5 seconds");
        assert_eq!(duration_words(secs(3_600)), "1 hour");
        assert_eq!(duration_words(secs(5_400)), "1 hour and 30 minutes");
        assert_eq!(duration_words(secs(3_605)), "1 hour and 5 seconds");
        assert_eq!(
            duration_words(secs(3 * 86_400 + 4 * 3_600 + 9)),
            "3 days and 4 hours"
        );
    }

    #[test]
    fn approximate_durations_round_to_one_unit() {
        let secs = Duration::from_secs;
        assert_eq!(duration_approx(Duration::ZERO), "less than a second");
        assert_eq!(duration_approx(secs(45)), "45 seconds");
        assert_eq!(duration_approx(secs(300)), "5 minutes");
        assert_eq!(duration_approx(secs(310)), "about 5 minutes");
        assert_eq!(duration_approx(secs(7_200)), "2 hours");
        assert_eq!(duration_approx(secs(2 * 86_400 + 50_000)), "about 3 days");
    }

    #[test]
    fn time_ago_is_just_now_then_approximate() {
        assert_eq!(time_ago(Duration::from_secs(2)), "just now");
        assert_eq!(time_ago(Duration::from_secs(45)), "45 seconds ago");
        assert_eq!(time_ago(Duration::from_secs(310)), "about 5 minutes ago");
    }

    #[test]
    fn byte_sizes() {
        assert_eq!(bytes_words(0), "0 bytes");
        assert_eq!(bytes_words(1), "1 byte");
        assert_eq!(bytes_words(512), "512 bytes");
        assert_eq!(bytes_words(1024), "1 KB");
        assert_eq!(bytes_words(1536), "1.5 KB");
        assert_eq!(bytes_words(20 * 1024 * 1024), "20 MB");
        assert_eq!(bytes_words(3 * 1024 * 1024 * 1024 + 214_748_365), "3.2 GB");
    }
}
