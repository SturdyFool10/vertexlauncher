//! Plural selection for every language family CLDR distinguishes, and the plural forms a message supplies.
//!
//! A language is described by one [`PluralRule`], named in its language file (`plural_rule = "east_slavic"`) or chosen
//! from its language code. Counts are whole numbers: the launcher counts things, it does not pluralize fractions.

use serde::{Deserialize, Serialize};

/// The plural categories of CLDR. A language uses only some of them: English has `One` and `Other`, Japanese only
/// `Other`, Russian `One`, `Few`, `Many`, and Arabic all six.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PluralCategory {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

/// Which counts select which [`PluralCategory`]. One variant per rule family in CLDR's plural rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PluralRule {
    /// No grammatical number: Japanese, Chinese, Korean, Vietnamese, Thai, Indonesian, ...
    None,
    /// `One` for 1, `Other` for the rest: English, German, Dutch, Swedish, Greek, Hungarian, ...
    OneOther,
    /// `One` for 0 and 1, `Many` for round millions: French, Portuguese.
    French,
    /// `One` for 1, `Many` for round millions: Spanish, Italian, Catalan.
    Spanish,
    /// `One` for 0 and 1: Hindi, Bengali, Persian, Gujarati, Kannada, Zulu, Amharic.
    Hindi,
    /// `One` for 1 and ending in 1 but not 11: Icelandic, Macedonian.
    Icelandic,
    /// One / few / many by last digits: Russian, Ukrainian, Belarusian.
    EastSlavic,
    /// One / few / other by last digits: Croatian, Serbian, Bosnian.
    SouthSlavic,
    /// One / few / many: Polish.
    Polish,
    /// `One` for 1, `Few` for 2 to 4: Czech, Slovak.
    Czech,
    /// One / two / few by the last two digits: Slovenian, Sorbian.
    Slovenian,
    /// Zero / one / other by last digits: Latvian.
    Latvian,
    /// One / few / other by last digits with a teens exception: Lithuanian.
    Lithuanian,
    /// One / few / other: Romanian, Moldovan.
    Romanian,
    /// Zero / one / two / few / many / other: Arabic.
    Arabic,
    /// One / two / other: Hebrew.
    Hebrew,
    /// Zero / one / two / few / many / other: Welsh.
    Welsh,
    /// One / two / few / many / other: Irish.
    Irish,
    /// One / two / few / many / other: Maltese.
    Maltese,
    /// One / other with the 1-3 and last-digit exception: Filipino, Tagalog.
    Filipino,
}

const RULE_NAMES: &[(&str, PluralRule)] = &[
    ("none", PluralRule::None),
    ("one_other", PluralRule::OneOther),
    ("french", PluralRule::French),
    ("spanish", PluralRule::Spanish),
    ("hindi", PluralRule::Hindi),
    ("icelandic", PluralRule::Icelandic),
    ("east_slavic", PluralRule::EastSlavic),
    ("south_slavic", PluralRule::SouthSlavic),
    ("polish", PluralRule::Polish),
    ("czech", PluralRule::Czech),
    ("slovenian", PluralRule::Slovenian),
    ("latvian", PluralRule::Latvian),
    ("lithuanian", PluralRule::Lithuanian),
    ("romanian", PluralRule::Romanian),
    ("arabic", PluralRule::Arabic),
    ("hebrew", PluralRule::Hebrew),
    ("welsh", PluralRule::Welsh),
    ("irish", PluralRule::Irish),
    ("maltese", PluralRule::Maltese),
    ("filipino", PluralRule::Filipino),
];

/// Language codes and the rule they use. Codes not listed fall back to `OneOther`, the commonest rule.
const LANGUAGE_RULES: &[(&str, PluralRule)] = &[
    ("ja", PluralRule::None),
    ("zh", PluralRule::None),
    ("ko", PluralRule::None),
    ("vi", PluralRule::None),
    ("th", PluralRule::None),
    ("id", PluralRule::None),
    ("ms", PluralRule::None),
    ("lo", PluralRule::None),
    ("km", PluralRule::None),
    ("my", PluralRule::None),
    ("jv", PluralRule::None),
    ("fr", PluralRule::French),
    ("pt", PluralRule::French),
    ("es", PluralRule::Spanish),
    ("it", PluralRule::Spanish),
    ("ca", PluralRule::Spanish),
    ("hi", PluralRule::Hindi),
    ("bn", PluralRule::Hindi),
    ("fa", PluralRule::Hindi),
    ("gu", PluralRule::Hindi),
    ("kn", PluralRule::Hindi),
    ("zu", PluralRule::Hindi),
    ("am", PluralRule::Hindi),
    ("is", PluralRule::Icelandic),
    ("mk", PluralRule::Icelandic),
    ("ru", PluralRule::EastSlavic),
    ("uk", PluralRule::EastSlavic),
    ("be", PluralRule::EastSlavic),
    ("hr", PluralRule::SouthSlavic),
    ("sr", PluralRule::SouthSlavic),
    ("bs", PluralRule::SouthSlavic),
    ("pl", PluralRule::Polish),
    ("cs", PluralRule::Czech),
    ("sk", PluralRule::Czech),
    ("sl", PluralRule::Slovenian),
    ("hsb", PluralRule::Slovenian),
    ("dsb", PluralRule::Slovenian),
    ("lv", PluralRule::Latvian),
    ("lt", PluralRule::Lithuanian),
    ("ro", PluralRule::Romanian),
    ("mo", PluralRule::Romanian),
    ("ar", PluralRule::Arabic),
    ("he", PluralRule::Hebrew),
    ("cy", PluralRule::Welsh),
    ("ga", PluralRule::Irish),
    ("mt", PluralRule::Maltese),
    ("fil", PluralRule::Filipino),
    ("tl", PluralRule::Filipino),
];

impl PluralRule {
    /// The rule a language file names, e.g. `"east_slavic"`.
    pub fn from_name(name: &str) -> Option<Self> {
        RULE_NAMES.iter().find(|(known, _)| *known == name).map(|(_, rule)| *rule)
    }

    pub fn name(self) -> &'static str {
        RULE_NAMES.iter().find(|(_, rule)| *rule == self).map_or("one_other", |(name, _)| name)
    }

    /// The usual rule for a language code (`"ru"`, `"pt-BR"`, `"ar_EG"`).
    pub fn for_language(tag: &str) -> Self {
        let language = tag.split(['-', '_', '.', '@']).next().unwrap_or("").to_ascii_lowercase();
        LANGUAGE_RULES.iter().find(|(code, _)| *code == language).map_or(PluralRule::OneOther, |(_, rule)| *rule)
    }

    /// Which category `count` falls in.
    pub fn category(self, count: u64) -> PluralCategory {
        use PluralCategory::{Few, Many, One, Other, Two, Zero};
        let (n10, n100) = (count % 10, count % 100);
        let round_million = count != 0 && count % 1_000_000 == 0;
        match self {
            PluralRule::None => Other,
            PluralRule::OneOther => if count == 1 { One } else { Other },
            PluralRule::French => match count {
                0 | 1 => One,
                _ if round_million => Many,
                _ => Other,
            },
            PluralRule::Spanish => match count {
                1 => One,
                _ if round_million => Many,
                _ => Other,
            },
            PluralRule::Hindi => if count <= 1 { One } else { Other },
            PluralRule::Icelandic => if n10 == 1 && n100 != 11 { One } else { Other },
            PluralRule::EastSlavic => {
                if n10 == 1 && n100 != 11 {
                    One
                } else if (2..=4).contains(&n10) && !(12..=14).contains(&n100) {
                    Few
                } else {
                    Many
                }
            }
            PluralRule::SouthSlavic => {
                if n10 == 1 && n100 != 11 {
                    One
                } else if (2..=4).contains(&n10) && !(12..=14).contains(&n100) {
                    Few
                } else {
                    Other
                }
            }
            PluralRule::Polish => {
                if count == 1 {
                    One
                } else if (2..=4).contains(&n10) && !(12..=14).contains(&n100) {
                    Few
                } else {
                    Many
                }
            }
            PluralRule::Czech => match count {
                1 => One,
                2..=4 => Few,
                _ => Other,
            },
            PluralRule::Slovenian => match n100 {
                1 => One,
                2 => Two,
                3 | 4 => Few,
                _ => Other,
            },
            PluralRule::Latvian => {
                if n10 == 0 || (11..=19).contains(&n100) {
                    Zero
                } else if n10 == 1 && n100 != 11 {
                    One
                } else {
                    Other
                }
            }
            PluralRule::Lithuanian => {
                if n10 == 1 && !(11..=19).contains(&n100) {
                    One
                } else if (2..=9).contains(&n10) && !(11..=19).contains(&n100) {
                    Few
                } else {
                    Other
                }
            }
            PluralRule::Romanian => {
                if count == 1 {
                    One
                } else if count == 0 || (2..=19).contains(&n100) {
                    Few
                } else {
                    Other
                }
            }
            PluralRule::Arabic => match count {
                0 => Zero,
                1 => One,
                2 => Two,
                _ if (3..=10).contains(&n100) => Few,
                _ if (11..=99).contains(&n100) => Many,
                _ => Other,
            },
            PluralRule::Hebrew => match count {
                1 => One,
                2 => Two,
                _ => Other,
            },
            PluralRule::Welsh => match count {
                0 => Zero,
                1 => One,
                2 => Two,
                3 => Few,
                6 => Many,
                _ => Other,
            },
            PluralRule::Irish => match count {
                1 => One,
                2 => Two,
                3..=6 => Few,
                7..=10 => Many,
                _ => Other,
            },
            PluralRule::Maltese => {
                if count == 1 {
                    One
                } else if count == 2 {
                    Two
                } else if count == 0 || (3..=10).contains(&n100) {
                    Few
                } else if (11..=19).contains(&n100) {
                    Many
                } else {
                    Other
                }
            }
            PluralRule::Filipino => {
                if matches!(count, 1..=3) || !matches!(n10, 4 | 6 | 9) {
                    One
                } else {
                    Other
                }
            }
        }
    }
}

/// A noun or phrase in each plural form a message needs, chosen with [`PluralRule::category`].
///
/// In a language file: `[plurals."{count} mods"]` with `one = "..."`, `few = "..."`, `other = "..."`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PluralForms {
    pub zero: Option<String>,
    pub one: Option<String>,
    pub two: Option<String>,
    pub few: Option<String>,
    pub many: Option<String>,
    pub other: String,
}

impl PluralForms {
    /// A singular and a plural form, which is all most languages need.
    pub fn new(one: impl Into<String>, other: impl Into<String>) -> Self {
        Self { one: Some(one.into()), ..Self::other_only(other) }
    }

    /// One form for every count, for languages without grammatical number.
    pub fn other_only(other: impl Into<String>) -> Self {
        Self { other: other.into(), ..Self::default() }
    }

    /// A form used for exactly zero ("no capes"), in any language.
    pub fn with_zero(mut self, form: impl Into<String>) -> Self {
        self.zero = Some(form.into());
        self
    }

    pub fn with_two(mut self, form: impl Into<String>) -> Self {
        self.two = Some(form.into());
        self
    }

    pub fn with_few(mut self, form: impl Into<String>) -> Self {
        self.few = Some(form.into());
        self
    }

    pub fn with_many(mut self, form: impl Into<String>) -> Self {
        self.many = Some(form.into());
        self
    }

    /// The form `count` selects under `rule`. An explicit zero form wins for exactly zero; a category with no form of
    /// its own uses `other`.
    pub fn select(&self, rule: PluralRule, count: u64) -> &str {
        if count == 0
            && let Some(zero) = self.zero.as_deref()
        {
            return zero;
        }
        let chosen = match rule.category(count) {
            PluralCategory::Zero => &self.zero,
            PluralCategory::One => &self.one,
            PluralCategory::Two => &self.two,
            PluralCategory::Few => &self.few,
            PluralCategory::Many => &self.many,
            PluralCategory::Other => return &self.other,
        };
        chosen.as_deref().unwrap_or(&self.other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PluralCategory::{Few, Many, One, Other, Two, Zero};

    #[test]
    fn rules_follow_cldr_for_whole_numbers() {
        for (rule, count, expected) in [
            (PluralRule::OneOther, 1, One),
            (PluralRule::OneOther, 0, Other),
            (PluralRule::French, 0, One),
            (PluralRule::French, 2_000_000, Many),
            (PluralRule::Spanish, 1_000_000, Many),
            (PluralRule::EastSlavic, 1, One),
            (PluralRule::EastSlavic, 21, One),
            (PluralRule::EastSlavic, 11, Many),
            (PluralRule::EastSlavic, 3, Few),
            (PluralRule::EastSlavic, 12, Many),
            (PluralRule::EastSlavic, 25, Many),
            (PluralRule::Polish, 1, One),
            (PluralRule::Polish, 22, Few),
            (PluralRule::Polish, 12, Many),
            (PluralRule::Polish, 5, Many),
            (PluralRule::Czech, 3, Few),
            (PluralRule::Czech, 5, Other),
            (PluralRule::Slovenian, 101, One),
            (PluralRule::Slovenian, 102, Two),
            (PluralRule::Slovenian, 103, Few),
            (PluralRule::Latvian, 0, Zero),
            (PluralRule::Latvian, 21, One),
            (PluralRule::Lithuanian, 31, One),
            (PluralRule::Lithuanian, 12, Other),
            (PluralRule::Lithuanian, 7, Few),
            (PluralRule::Romanian, 0, Few),
            (PluralRule::Romanian, 19, Few),
            (PluralRule::Romanian, 20, Other),
            (PluralRule::Arabic, 0, Zero),
            (PluralRule::Arabic, 2, Two),
            (PluralRule::Arabic, 7, Few),
            (PluralRule::Arabic, 25, Many),
            (PluralRule::Arabic, 100, Other),
            (PluralRule::Hebrew, 2, Two),
            (PluralRule::Welsh, 6, Many),
            (PluralRule::Irish, 8, Many),
            (PluralRule::Hindi, 0, One),
            (PluralRule::Icelandic, 21, One),
            (PluralRule::None, 1, Other),
        ] {
            assert_eq!(rule.category(count), expected, "{rule:?} {count}");
        }
    }

    #[test]
    fn language_codes_pick_their_rule() {
        assert_eq!(PluralRule::for_language("ru"), PluralRule::EastSlavic);
        assert_eq!(PluralRule::for_language("pt-BR"), PluralRule::French);
        assert_eq!(PluralRule::for_language("ar_EG.UTF-8"), PluralRule::Arabic);
        assert_eq!(PluralRule::for_language("xx"), PluralRule::OneOther);
        for (name, rule) in RULE_NAMES {
            assert_eq!(PluralRule::from_name(name), Some(*rule));
            assert_eq!(rule.name(), *name);
        }
    }

    #[test]
    fn forms_select_by_the_languages_own_rules() {
        let capes = PluralForms::new("cape", "capes");
        assert_eq!(capes.select(PluralRule::OneOther, 1), "cape");
        assert_eq!(capes.select(PluralRule::OneOther, 0), "capes");
        assert_eq!(capes.select(PluralRule::French, 0), "cape");
        let with_zero = PluralForms::new("cape", "capes").with_zero("no capes");
        assert_eq!(with_zero.select(PluralRule::OneOther, 0), "no capes");
        assert_eq!(with_zero.select(PluralRule::French, 0), "no capes");
        let with_two = PluralForms::new("cape", "capes").with_two("a pair of capes");
        assert_eq!(with_two.select(PluralRule::OneOther, 2), "capes");
        assert_eq!(with_two.select(PluralRule::Arabic, 2), "a pair of capes");
        let millions = PluralForms::new("cape", "capes").with_many("de capes");
        assert_eq!(millions.select(PluralRule::Spanish, 1_000_000), "de capes");
        assert_eq!(millions.select(PluralRule::Spanish, 999), "capes");
        let counter = PluralForms::other_only("枚");
        assert_eq!(counter.select(PluralRule::None, 1), "枚");
        let russian = PluralForms::new("мод", "модов").with_few("мода").with_many("модов");
        assert_eq!(russian.select(PluralRule::EastSlavic, 2), "мода");
        assert_eq!(russian.select(PluralRule::EastSlavic, 5), "модов");
        assert_eq!(russian.select(PluralRule::EastSlavic, 21), "мод");
    }
}
