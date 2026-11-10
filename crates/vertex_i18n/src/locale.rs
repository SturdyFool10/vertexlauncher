//! The language-specific half of natural-language output, as data.
//!
//! What differs between languages falls into two groups:
//!
//! - **Mechanics**, the same for every sentence: how a list is joined ("A, B, and C" against "A、B、C"), which plural form
//!   a number selects, how a number is grouped and punctuated, how an ordinal is written, how a duration or size is
//!   phrased. A [`LocaleData`] describes them, and [`Locale`] applies them.
//! - **Vocabulary and grammar** that belongs to the words being used (the plural of a noun, a gendered article). Those
//!   come from the message catalog as [`PluralForms`], selected with [`Locale::select`].
//!
//! A language file's `[locale]` section is a `LocaleData`: any field it leaves out uses the English value, so a new
//! language starts working immediately and each setting can be refined. Plural rules, list patterns, number formats,
//! ordinals and digit sets are all data, which is what lets any language (right-to-left ones, languages with six plural
//! forms, languages that do not group digits in threes) be described without changing code.
//!
//! The built-in non-English data covers the mechanics only and has had no review by native speakers.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::number::{NumberFormat, defaults_for_language};
use crate::plural::{PluralCategory, PluralForms, PluralRule};
use crate::template::render;

/// The largest count spelled out as a word; anything above it reads as "several" (or the language's equivalent).
pub const MAX_COUNT_WORD: usize = 10;

const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 60 * SECONDS_PER_MINUTE;
const SECONDS_PER_DAY: u64 = 24 * SECONDS_PER_HOUR;

/// How a list is combined: "A, B, and C" (`And`) or "A, B, or C" (`Or`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    And,
    Or,
}

/// A unit of time, from the largest to the smallest.
#[derive(Clone, Copy)]
enum TimeUnit {
    Day,
    Hour,
    Minute,
    Second,
}

/// Fixed phrases and number words.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Words {
    /// "zero" through "ten", eleven entries.
    pub count: Vec<String>,
    /// What a count above ten reads as.
    pub several: String,
    pub less_than_a_second: String,
    pub just_now: String,
    /// `{amount}` is a rounded duration.
    pub about: String,
    /// `{span}` is a duration.
    pub ago: String,
    /// `{span}` is a duration.
    pub from_now: String,
    /// The list item standing for hidden items; `{count}`.
    pub remainder: String,
}

impl Default for Words {
    fn default() -> Self {
        Self {
            count: ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"]
                .map(str::to_owned)
                .to_vec(),
            several: "several".to_owned(),
            less_than_a_second: "less than a second".to_owned(),
            just_now: "just now".to_owned(),
            about: "about {amount}".to_owned(),
            ago: "{span} ago".to_owned(),
            from_now: "in {span}".to_owned(),
            remainder: "{count} more".to_owned(),
        }
    }
}

/// Unit names and how a number sits next to one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Units {
    pub day: PluralForms,
    pub hour: PluralForms,
    pub minute: PluralForms,
    pub second: PluralForms,
    pub byte: PluralForms,
    /// Kilo-, mega-, giga- and tera-byte labels.
    pub byte_prefixes: Vec<String>,
    /// A number with a unit noun: `{number}` and `{unit}`.
    pub counted: String,
    /// A scaled size with its label: `{number}` and `{unit}`.
    pub size: String,
    /// How two duration parts join (`{0}` and `{1}`). Unset joins them like a list.
    pub duration_join: Option<String>,
}

impl Default for Units {
    fn default() -> Self {
        Self {
            day: PluralForms::new("day", "days"),
            hour: PluralForms::new("hour", "hours"),
            minute: PluralForms::new("minute", "minutes"),
            second: PluralForms::new("second", "seconds"),
            byte: PluralForms::new("byte", "bytes"),
            byte_prefixes: ["KB", "MB", "GB", "TB"].map(str::to_owned).to_vec(),
            counted: "{number} {unit}".to_owned(),
            size: "{number} {unit}".to_owned(),
            duration_join: None,
        }
    }
}

/// A conjunction that changes with the next word (Spanish "y" becomes "e" before an "i" sound).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Swap {
    /// The text in the pattern to replace, with its spaces.
    pub from: String,
    pub to: String,
    /// The swap applies when the next item (lowercased, past leading punctuation) starts with one of these...
    pub when_next_starts_with: Vec<String>,
    /// ...unless it starts with one of these.
    pub except: Vec<String>,
}

impl Default for Swap {
    fn default() -> Self {
        Self { from: String::new(), to: String::new(), when_next_starts_with: Vec::new(), except: Vec::new() }
    }
}

/// CLDR-style list patterns; `{0}` is the earlier part and `{1}` the rest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListPatterns {
    /// Exactly two items.
    pub two: String,
    /// The first item and the rest, for three or more.
    pub start: String,
    pub middle: String,
    /// The last two items, for three or more.
    pub end: String,
    pub swaps: Vec<Swap>,
}

impl ListPatterns {
    fn english(word: &str) -> Self {
        Self {
            two: format!("{{0}} {word} {{1}}"),
            start: "{0}, {1}".to_owned(),
            middle: "{0}, {1}".to_owned(),
            end: format!("{{0}}, {word} {{1}}"),
            swaps: Vec::new(),
        }
    }

    fn apply(&self, pattern: &str, first: &str, rest: &str) -> String {
        let mut pattern = pattern.to_owned();
        let next = rest.trim_start_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
        for swap in &self.swaps {
            let wanted = swap.when_next_starts_with.iter().any(|prefix| next.starts_with(prefix.as_str()));
            let excluded = swap.except.iter().any(|prefix| next.starts_with(prefix.as_str()));
            if wanted && !excluded {
                pattern = pattern.replace(&swap.from, &swap.to);
            }
        }
        render(&pattern, &[("0", first.to_owned()), ("1", rest.to_owned())])
    }
}

impl Default for ListPatterns {
    fn default() -> Self {
        Self::english("and")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Lists {
    pub and: ListPatterns,
    pub or: ListPatterns,
}

impl Default for Lists {
    fn default() -> Self {
        Self { and: ListPatterns::english("and"), or: ListPatterns::english("or") }
    }
}

/// Which ordinal template a number takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrdinalRule {
    /// Every number takes `other`.
    Single,
    /// 1st, 2nd, 3rd, 4th, with 11th to 13th as `other`: English.
    #[default]
    English,
    /// Only 1 takes `one`; the rest take `other`: French.
    FirstOnly,
}

/// How an ordinal is written; each template has `{n}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ordinal {
    pub rule: OrdinalRule,
    pub one: String,
    pub two: String,
    pub few: String,
    pub other: String,
}

impl Default for Ordinal {
    fn default() -> Self {
        Self {
            rule: OrdinalRule::English,
            one: "{n}st".to_owned(),
            two: "{n}nd".to_owned(),
            few: "{n}rd".to_owned(),
            other: "{n}th".to_owned(),
        }
    }
}

/// Month and weekday names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Calendar {
    /// January to December.
    pub months_long: Vec<String>,
    pub months_short: Vec<String>,
    /// Monday to Sunday.
    pub weekdays_short: Vec<String>,
}

impl Default for Calendar {
    fn default() -> Self {
        let own = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect();
        Self {
            months_long: own(&[
                "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
                "December",
            ]),
            months_short: own(&["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]),
            weekdays_short: own(&["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]),
        }
    }
}

/// Everything about how one language handles counts, lists, numbers, times and sizes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocaleData {
    /// The language code ("en", "pt-BR").
    pub tag: String,
    /// A rule name (`"east_slavic"`, see [`PluralRule::from_name`]). Unset uses the usual rule for the tag.
    pub plural_rule: Option<String>,
    /// Unset uses the usual formatting for the tag.
    pub number: Option<NumberFormat>,
    pub words: Words,
    pub units: Units,
    pub lists: Lists,
    pub ordinal: Ordinal,
    pub calendar: Calendar,
}

impl Default for LocaleData {
    fn default() -> Self {
        Self {
            tag: "en".to_owned(),
            plural_rule: None,
            number: None,
            words: Words::default(),
            units: Units::default(),
            lists: Lists::default(),
            ordinal: Ordinal::default(),
            calendar: Calendar::default(),
        }
    }
}

/// A language's mechanics, ready to use. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Locale {
    data: Arc<LocaleData>,
    rule: PluralRule,
    number: NumberFormat,
}

impl PartialEq for Locale {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
    }
}

const BUILTIN_SOURCES: [&str; 4] = [
    include_str!("../data/es.toml"),
    include_str!("../data/fr.toml"),
    include_str!("../data/de.toml"),
    include_str!("../data/ja.toml"),
];

fn builtin() -> &'static [Locale] {
    static BUILTIN: OnceLock<Vec<Locale>> = OnceLock::new();
    BUILTIN.get_or_init(|| {
        let mut all = vec![Locale::from_data(LocaleData::default())];
        for source in BUILTIN_SOURCES {
            match toml::from_str::<LocaleData>(source) {
                Ok(data) => all.push(Locale::from_data(data)),
                Err(err) => tracing::error!(target: "vertex_i18n", "A built-in locale file is invalid: {err}"),
            }
        }
        all
    })
}

impl Default for Locale {
    fn default() -> Self {
        Self::english()
    }
}

impl Locale {
    /// A locale from its data.
    pub fn from_data(data: LocaleData) -> Self {
        let rule = data.plural_rule.as_deref().and_then(PluralRule::from_name).unwrap_or_else(|| PluralRule::for_language(&data.tag));
        let number = data.number.clone().unwrap_or_else(|| defaults_for_language(&data.tag));
        Self { data: Arc::new(data), rule, number }
    }

    pub fn english() -> Self {
        builtin()[0].clone()
    }

    /// The built-in mechanics for `tag`'s language ("en", "es_MX", "ja_JP.UTF-8"), if the launcher ships any.
    pub fn builtin(tag: &str) -> Option<Self> {
        let language = tag.split(['-', '_', '.', '@']).next()?.to_ascii_lowercase();
        builtin().iter().find(|locale| locale.data.tag == language).cloned()
    }

    /// Every locale whose mechanics ship with the launcher.
    pub fn builtins() -> Vec<Locale> {
        builtin().to_vec()
    }

    /// The active language's locale; English until a language is chosen (see [`crate::set_language`]).
    pub fn current() -> Self {
        crate::current_locale()
    }

    /// The locale for a locale tag, from a loaded language or the built-in set.
    pub fn from_tag(tag: &str) -> Option<Self> {
        crate::locale_for_tag(tag).or_else(|| Self::builtin(tag))
    }

    /// The language of the user's environment (`LC_ALL`, `LC_MESSAGES`, then `LANG`), if it has a locale here. The first
    /// variable that is set decides.
    pub fn detect_system() -> Option<Self> {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
            .and_then(|value| Self::from_tag(&value))
    }

    pub fn data(&self) -> &LocaleData {
        &self.data
    }

    /// The language code: "en", "es", "fr", "de", "ja", ...
    pub fn tag(&self) -> &str {
        &self.data.tag
    }

    pub fn plural_rule(&self) -> PluralRule {
        self.rule
    }

    pub fn number_format(&self) -> &NumberFormat {
        &self.number
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Plurals
    // -----------------------------------------------------------------------------------------------------------------

    /// Which plural form a whole number selects, per CLDR.
    pub fn plural_category(&self, count: u64) -> PluralCategory {
        self.rule.category(count)
    }

    /// The form of `forms` that `count` selects. An explicit zero form wins for exactly zero.
    pub fn select<'a>(&self, forms: &'a PluralForms, count: u64) -> &'a str {
        forms.select(self.rule, count)
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Numbers
    // -----------------------------------------------------------------------------------------------------------------

    /// A count as a word: "one" through "ten", then "several" so a big number never reads as a precise claim. Words for
    /// 1-10 use the plain counting form; gendered languages need the message's own catalog when the noun's gender matters.
    pub fn count_word(&self, count: usize) -> String {
        self.data.words.count.get(count).cloned().unwrap_or_else(|| self.data.words.several.clone())
    }

    /// A count as a word up to ten, and as exact digits above that.
    pub fn count_or_digits(&self, count: usize) -> String {
        if count <= MAX_COUNT_WORD { self.count_word(count) } else { self.thousands(count as u64) }
    }

    /// Digits grouped the way the language groups them: "12,345" in English, "12.345" in German, "12 345" in French.
    pub fn thousands(&self, value: u64) -> String {
        self.number.integer(value)
    }

    /// `value` with exactly `places` decimals, using the language's decimal mark.
    pub fn decimal(&self, value: f64, places: usize) -> String {
        self.number.decimal(value, places)
    }

    /// An ordinal in digits: "3rd", "3.º", "3e", "3.", "第3".
    pub fn ordinal_number(&self, value: u64) -> String {
        let ordinal = &self.data.ordinal;
        let template = match ordinal.rule {
            OrdinalRule::Single => &ordinal.other,
            OrdinalRule::FirstOnly if value == 1 => &ordinal.one,
            OrdinalRule::FirstOnly => &ordinal.other,
            OrdinalRule::English => match (value % 100, value % 10) {
                (11..=13, _) => &ordinal.other,
                (_, 1) => &ordinal.one,
                (_, 2) => &ordinal.two,
                (_, 3) => &ordinal.few,
                _ => &ordinal.other,
            },
        };
        render(template, &[("n", self.number.native_digits(&value.to_string()))])
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Calendar
    // -----------------------------------------------------------------------------------------------------------------

    /// The full name of month `month` (1 to 12).
    pub fn month_long(&self, month: usize) -> String {
        self.data.calendar.months_long.get(month.wrapping_sub(1)).cloned().unwrap_or_else(|| month.to_string())
    }

    /// The short name of month `month` (1 to 12).
    pub fn month_short(&self, month: usize) -> String {
        self.data.calendar.months_short.get(month.wrapping_sub(1)).cloned().unwrap_or_else(|| month.to_string())
    }

    /// The short name of a weekday, counting from Monday as 0.
    pub fn weekday_short(&self, weekday: usize) -> String {
        self.data.calendar.weekdays_short.get(weekday).cloned().unwrap_or_default()
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Lists
    // -----------------------------------------------------------------------------------------------------------------

    /// Items as a list in this language: "A, B, and C", "A, B y C", "A、B、C".
    pub fn join<S: AsRef<str>>(&self, items: &[S], kind: ListKind) -> String {
        let parts: Vec<&str> = items.iter().map(AsRef::as_ref).collect();
        self.join_parts(&parts, kind)
    }

    /// Like [`Locale::join`], but shows at most `max` items and folds the rest into one "N more" item. A single leftover
    /// item is named instead.
    pub fn join_limited<S: AsRef<str>>(&self, items: &[S], kind: ListKind, max: usize) -> String {
        let mut parts: Vec<String> = items.iter().map(|item| item.as_ref().to_owned()).collect();
        if parts.len() > max.saturating_add(1) {
            let hidden = parts.len() - max;
            parts.truncate(max);
            parts.push(self.remainder_item(hidden));
        }
        self.join(&parts, kind)
    }

    /// The list item that stands for `hidden` omitted items: "3 more", "3 más", "他3件".
    pub fn remainder_item(&self, hidden: usize) -> String {
        render(&self.data.words.remainder, &[("count", self.thousands(hidden as u64))])
    }

    fn join_parts(&self, parts: &[&str], kind: ListKind) -> String {
        let patterns = match kind {
            ListKind::And => &self.data.lists.and,
            ListKind::Or => &self.data.lists.or,
        };
        match parts {
            [] => String::new(),
            [only] => (*only).to_owned(),
            [first, second] => patterns.apply(&patterns.two, first, second),
            [first, middle @ .., penultimate, last] => {
                let mut tail = patterns.apply(&patterns.end, penultimate, last);
                for item in middle.iter().rev() {
                    tail = patterns.apply(&patterns.middle, item, &tail);
                }
                patterns.apply(&patterns.start, first, &tail)
            }
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Time and size
    // -----------------------------------------------------------------------------------------------------------------

    fn time_unit(&self, unit: TimeUnit) -> &PluralForms {
        let units = &self.data.units;
        match unit {
            TimeUnit::Day => &units.day,
            TimeUnit::Hour => &units.hour,
            TimeUnit::Minute => &units.minute,
            TimeUnit::Second => &units.second,
        }
    }

    /// "5 seconds", "1 día", "3日": a number with its unit noun, spaced as the language spaces it.
    fn counted_unit(&self, count: u64, forms: &PluralForms) -> String {
        render(
            &self.data.units.counted,
            &[("number", self.thousands(count)), ("unit", self.select(forms, count).to_owned())],
        )
    }

    /// A duration with its two largest non-zero units: "2 minutes and 5 seconds".
    pub fn duration_words(&self, duration: Duration) -> String {
        let total = duration.as_secs();
        if total == 0 {
            return self.data.words.less_than_a_second.clone();
        }
        let units = [
            (total / SECONDS_PER_DAY, TimeUnit::Day),
            (total % SECONDS_PER_DAY / SECONDS_PER_HOUR, TimeUnit::Hour),
            (total % SECONDS_PER_HOUR / SECONDS_PER_MINUTE, TimeUnit::Minute),
            (total % SECONDS_PER_MINUTE, TimeUnit::Second),
        ];
        let parts: Vec<String> = units
            .into_iter()
            .filter(|(value, _)| *value > 0)
            .take(2)
            .map(|(value, unit)| self.counted_unit(value, self.time_unit(unit)))
            .collect();
        match (&self.data.units.duration_join, parts.as_slice()) {
            (Some(join), [first, second]) => render(join, &[("0", first.clone()), ("1", second.clone())]),
            _ => self.join(&parts, ListKind::And),
        }
    }

    fn about(&self, amount: &str) -> String {
        render(&self.data.words.about, &[("amount", amount.to_owned())])
    }

    /// A duration as one rounded unit, with "about" when rounding changed it.
    pub fn duration_approx(&self, duration: Duration) -> String {
        let total = duration.as_secs();
        if total == 0 {
            return self.data.words.less_than_a_second.clone();
        }
        let (unit_seconds, unit) = match total {
            0..SECONDS_PER_MINUTE => (1, TimeUnit::Second),
            SECONDS_PER_MINUTE..SECONDS_PER_HOUR => (SECONDS_PER_MINUTE, TimeUnit::Minute),
            SECONDS_PER_HOUR..SECONDS_PER_DAY => (SECONDS_PER_HOUR, TimeUnit::Hour),
            _ => (SECONDS_PER_DAY, TimeUnit::Day),
        };
        let rounded = (total + unit_seconds / 2) / unit_seconds;
        let amount = self.counted_unit(rounded, self.time_unit(unit));
        if total % unit_seconds == 0 { amount } else { self.about(&amount) }
    }

    /// How long ago something happened: "just now" within five seconds, then "about 5 minutes ago", "hace 2 horas".
    pub fn time_ago(&self, elapsed: Duration) -> String {
        if elapsed < Duration::from_secs(5) {
            return self.data.words.just_now.clone();
        }
        render(&self.data.words.ago, &[("span", self.duration_approx(elapsed))])
    }

    /// How far off something is: "in 5 minutes", "dentro de 2 horas".
    pub fn time_from_now(&self, remaining: Duration) -> String {
        if remaining < Duration::from_secs(5) {
            return self.data.words.just_now.clone();
        }
        render(&self.data.words.from_now, &[("span", self.duration_approx(remaining))])
    }

    /// A byte count for people: "512 bytes", "1.5 KB", "20 MB". Units step by 1024.
    pub fn bytes_words(&self, bytes: u64) -> String {
        if bytes < 1024 {
            return self.counted_unit(bytes, &self.data.units.byte);
        }
        let prefixes = &self.data.units.byte_prefixes;
        let mut value = bytes as f64 / 1024.0;
        let mut index = 0;
        while index + 1 < prefixes.len() && value >= 1024.0 {
            value /= 1024.0;
            index += 1;
        }
        let number = if value < 10.0 {
            let text = self.decimal(value, 1);
            // "1,0" becomes "1": drop a trailing ".0" in the language's own punctuation.
            let suffix = format!("{}{}", self.number.decimal, self.number.native_digits("0"));
            text.strip_suffix(&suffix).map_or(text.clone(), str::to_owned)
        } else {
            self.decimal(value, 0)
        };
        let unit = prefixes.get(index).cloned().unwrap_or_default();
        render(&self.data.units.size, &[("number", number), ("unit", unit)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plural::PluralCategory;

    fn en() -> Locale {
        Locale::english()
    }
    fn es() -> Locale {
        Locale::builtin("es").unwrap()
    }
    fn fr() -> Locale {
        Locale::builtin("fr").unwrap()
    }
    fn de() -> Locale {
        Locale::builtin("de").unwrap()
    }
    fn ja() -> Locale {
        Locale::builtin("ja").unwrap()
    }

    fn secs(value: u64) -> Duration {
        Duration::from_secs(value)
    }

    #[test]
    fn plural_categories_follow_cldr() {
        use PluralCategory::{Many, One, Other};
        for (locale, count, expected) in [
            (en(), 0, Other),
            (en(), 1, One),
            (en(), 2, Other),
            (de(), 1, One),
            (de(), 0, Other),
            (es(), 1, One),
            (es(), 5, Other),
            (es(), 1_000_000, Many),
            (fr(), 0, One),
            (fr(), 1, One),
            (fr(), 2, Other),
            (fr(), 2_000_000, Many),
            (ja(), 1, Other),
            (ja(), 0, Other),
        ] {
            assert_eq!(
                locale.plural_category(count),
                expected,
                "{locale:?} {count}"
            );
        }
    }

    #[test]
    fn plural_forms_select_by_the_languages_own_rules() {
        let capes = PluralForms::new("cape", "capes");
        assert_eq!(en().select(&capes, 1), "cape");
        assert_eq!(en().select(&capes, 0), "capes");
        assert_eq!(en().select(&capes, 5), "capes");
        // fr() treats zero as singular.
        let french = PluralForms::new("cape", "capes");
        assert_eq!(fr().select(&french, 0), "cape");
        // An explicit zero form wins for exactly zero, in any language.
        let with_zero = PluralForms::new("cape", "capes").with_zero("no capes");
        assert_eq!(en().select(&with_zero, 0), "no capes");
        assert_eq!(fr().select(&with_zero, 0), "no capes");
        // Forms a language never selects are ignored.
        let with_two = PluralForms::new("cape", "capes").with_two("a pair of capes");
        assert_eq!(en().select(&with_two, 2), "capes");
        // es() and fr() use "many" for round millions.
        let millions = PluralForms::new("cape", "capes").with_many("de capes");
        assert_eq!(es().select(&millions, 1_000_000), "de capes");
        assert_eq!(es().select(&millions, 999), "capes");
        // No grammatical number: one form for everything.
        let counter = PluralForms::other_only("枚");
        assert_eq!(ja().select(&counter, 1), "枚");
        assert_eq!(ja().select(&counter, 9), "枚");
    }

    #[test]
    fn count_words_exist_in_every_language() {
        for (locale, two, ten, several) in [
            (en(), "two", "ten", "several"),
            (es(), "dos", "diez", "varios"),
            (fr(), "deux", "dix", "plusieurs"),
            (de(), "zwei", "zehn", "mehrere"),
            (ja(), "二", "十", "数"),
        ] {
            assert_eq!(locale.count_word(2), two, "{locale:?}");
            assert_eq!(locale.count_word(10), ten, "{locale:?}");
            assert_eq!(locale.count_word(11), several, "{locale:?}");
        }
        assert_eq!(de().count_or_digits(7), "sieben");
        assert_eq!(de().count_or_digits(1_500), "1.500");
    }

    #[test]
    fn lists_use_each_languages_conjunction_and_punctuation() {
        let abc = ["A", "B", "C"];
        let ab = ["A", "B"];
        for (locale, kind, three, two) in [
            (en(), ListKind::And, "A, B, and C", "A and B"),
            (en(), ListKind::Or, "A, B, or C", "A or B"),
            (es(), ListKind::And, "A, B y C", "A y B"),
            (es(), ListKind::Or, "A, B o C", "A o B"),
            (fr(), ListKind::And, "A, B et C", "A et B"),
            (fr(), ListKind::Or, "A, B ou C", "A ou B"),
            (de(), ListKind::And, "A, B und C", "A und B"),
            (de(), ListKind::Or, "A, B oder C", "A oder B"),
            (ja(), ListKind::And, "A、B、C", "A、B"),
            (ja(), ListKind::Or, "A、B、またはC", "AまたはB"),
        ] {
            assert_eq!(locale.join(&abc, kind), three, "{locale:?} {kind:?}");
            assert_eq!(locale.join(&ab, kind), two, "{locale:?} {kind:?}");
            assert_eq!(locale.join(&["A"], kind), "A");
            assert_eq!(locale.join::<&str>(&[], kind), "");
        }
    }

    #[test]
    fn spanish_conjunctions_avoid_repeated_vowel_sounds() {
        assert_eq!(
            es().join(&["Pan", "Irene"], ListKind::And),
            "Pan e Irene"
        );
        assert_eq!(es().join(&["Pan", "Hilo"], ListKind::And), "Pan e Hilo");
        assert_eq!(
            es().join(&["Sol", "Hierro"], ListKind::And),
            "Sol y Hierro"
        );
        assert_eq!(
            es().join(&["Sol", "Hielo"], ListKind::And),
            "Sol y Hielo"
        );
        assert_eq!(es().join(&["Uno", "Oro"], ListKind::Or), "Uno u Oro");
        assert_eq!(
            es().join(&["Uno", "Hombre"], ListKind::Or),
            "Uno u Hombre"
        );
        assert_eq!(es().join(&["Uno", "Dos"], ListKind::Or), "Uno o Dos");
    }

    #[test]
    fn long_lists_fold_the_rest_into_a_localized_count() {
        let items = ["A", "B", "C", "D", "E", "F"];
        for (locale, expected) in [
            (en(), "A, B, C, and 3 more"),
            (es(), "A, B, C y 3 más"),
            (fr(), "A, B, C et 3 autres"),
            (de(), "A, B, C und 3 weitere"),
            (ja(), "A、B、C、他3件"),
        ] {
            assert_eq!(locale.join_limited(&items, ListKind::And, 3), expected);
        }
        // A single leftover is named, not counted.
        assert_eq!(
            en().join_limited(&items[..4], ListKind::And, 3),
            "A, B, C, and D"
        );
    }

    #[test]
    fn numbers_are_grouped_and_punctuated_per_language() {
        assert_eq!(en().thousands(1_234), "1,234");
        assert_eq!(en().thousands(999), "999");
        assert_eq!(de().thousands(1_234_567), "1.234.567");
        assert_eq!(fr().thousands(1_234_567), "1\u{202f}234\u{202f}567");
        assert_eq!(ja().thousands(1_234), "1,234");
        // es() leaves four-digit numbers alone.
        assert_eq!(es().thousands(1_234), "1234");
        assert_eq!(es().thousands(12_345), "12.345");

        assert_eq!(en().decimal(1.5, 1), "1.5");
        assert_eq!(de().decimal(1.5, 1), "1,5");
        assert_eq!(fr().decimal(2.25, 2), "2,25");
    }

    #[test]
    fn ordinals_are_written_per_language() {
        assert_eq!(en().ordinal_number(22), "22nd");
        assert_eq!(en().ordinal_number(13), "13th");
        assert_eq!(es().ordinal_number(3), "3.º");
        assert_eq!(fr().ordinal_number(1), "1er");
        assert_eq!(fr().ordinal_number(2), "2e");
        assert_eq!(de().ordinal_number(3), "3.");
        assert_eq!(ja().ordinal_number(3), "第3");
    }

    #[test]
    fn durations_name_their_units_in_each_language() {
        assert_eq!(en().duration_words(secs(125)), "2 minutes and 5 seconds");
        assert_eq!(es().duration_words(secs(125)), "2 minutos y 5 segundos");
        assert_eq!(es().duration_words(secs(1)), "1 segundo");
        assert_eq!(fr().duration_words(secs(3_600)), "1 heure");
        assert_eq!(fr().duration_words(secs(5_400)), "1 heure et 30 minutes");
        assert_eq!(de().duration_words(secs(125)), "2 Minuten und 5 Sekunden");
        assert_eq!(de().duration_words(secs(86_400)), "1 Tag");
        assert_eq!(ja().duration_words(secs(125)), "2分5秒");
        assert_eq!(ja().duration_words(secs(5_400)), "1時間30分");
        for (locale, expected) in [
            (en(), "less than a second"),
            (es(), "menos de un segundo"),
            (fr(), "moins d'une seconde"),
            (de(), "weniger als eine Sekunde"),
            (ja(), "1秒未満"),
        ] {
            assert_eq!(locale.duration_words(Duration::from_millis(300)), expected);
            assert_eq!(locale.duration_approx(Duration::ZERO), expected);
        }
    }

    #[test]
    fn approximate_durations_and_time_ago_follow_each_languages_phrasing() {
        assert_eq!(en().duration_approx(secs(310)), "about 5 minutes");
        assert_eq!(
            es().duration_approx(secs(310)),
            "aproximadamente 5 minutos"
        );
        assert_eq!(fr().duration_approx(secs(300)), "5 minutes");
        assert_eq!(fr().duration_approx(secs(310)), "environ 5 minutes");
        assert_eq!(de().duration_approx(secs(310)), "etwa 5 Minuten");
        assert_eq!(ja().duration_approx(secs(310)), "約5分");

        assert_eq!(en().time_ago(secs(45)), "45 seconds ago");
        assert_eq!(es().time_ago(secs(45)), "hace 45 segundos");
        assert_eq!(fr().time_ago(secs(7_200)), "il y a 2 heures");
        assert_eq!(de().time_ago(secs(45)), "vor 45 Sekunden");
        assert_eq!(ja().time_ago(secs(300)), "5分前");
        assert_eq!(ja().time_ago(secs(310)), "約5分前");
        for (locale, expected) in [
            (en(), "just now"),
            (es(), "ahora mismo"),
            (fr(), "à l'instant"),
            (de(), "gerade eben"),
            (ja(), "たった今"),
        ] {
            assert_eq!(locale.time_ago(secs(2)), expected);
        }
    }

    #[test]
    fn byte_sizes_use_local_units_and_decimal_marks() {
        assert_eq!(en().bytes_words(1), "1 byte");
        assert_eq!(en().bytes_words(1_536), "1.5 KB");
        assert_eq!(es().bytes_words(20 * 1024 * 1024), "20 MB");
        assert_eq!(es().bytes_words(1_536), "1,5 KB");
        assert_eq!(fr().bytes_words(0), "0 octet", "French zero is singular");
        assert_eq!(fr().bytes_words(512), "512 octets");
        assert_eq!(fr().bytes_words(1_536), "1,5 Ko");
        assert_eq!(de().bytes_words(1_536), "1,5 KB");
        assert_eq!(de().bytes_words(512), "512 Bytes");
        assert_eq!(ja().bytes_words(1), "1バイト");
        assert_eq!(ja().bytes_words(512), "512バイト");
        assert_eq!(en().bytes_words(2 * 1024), "2 KB");
    }
}
