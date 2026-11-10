//! How a language writes numbers: grouping, the decimal mark and the digit set.

use serde::{Deserialize, Serialize};

/// How digits are grouped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Grouping {
    /// Groups of three: 1,234,567.
    #[default]
    Western,
    /// A group of three, then groups of two: 12,34,567 (India).
    Indian,
    /// Never grouped.
    None,
}

/// Number formatting for one language. Every field has the English value as its default, so a language file only
/// names what differs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NumberFormat {
    pub decimal: String,
    pub group: String,
    pub grouping: Grouping,
    /// The fewest digits a number needs before it is grouped at all (Spanish groups from five: `1234`, `12.345`).
    pub group_from: usize,
    /// The ten digits, zero first, for languages that do not use 0-9 (`"٠١٢٣٤٥٦٧٨٩"`). Empty keeps 0-9.
    pub digits: String,
}

impl Default for NumberFormat {
    fn default() -> Self {
        Self { decimal: ".".to_owned(), group: ",".to_owned(), grouping: Grouping::Western, group_from: 4, digits: String::new() }
    }
}

impl NumberFormat {
    /// `value` in the language's digits.
    pub fn native_digits(&self, ascii: &str) -> String {
        let digits: Vec<char> = self.digits.chars().collect();
        if digits.len() != 10 {
            return ascii.to_owned();
        }
        ascii.chars().map(|c| c.to_digit(10).map_or(c, |digit| digits[digit as usize])).collect()
    }

    /// A whole number with digit groups: "12,345" in English, "12.345" in German, "1,23,456" in India.
    pub fn integer(&self, value: u64) -> String {
        let digits = value.to_string();
        if self.grouping == Grouping::None || digits.len() < self.group_from {
            return self.native_digits(&digits);
        }
        let mut grouped = String::with_capacity(digits.len() + digits.len() / 2);
        let len = digits.len();
        for (index, digit) in digits.chars().enumerate() {
            let from_right = len - index;
            let boundary = match self.grouping {
                Grouping::Western => from_right % 3 == 0,
                Grouping::Indian => from_right == 3 || (from_right > 3 && (from_right - 3) % 2 == 0),
                Grouping::None => false,
            };
            if index > 0 && boundary {
                grouped.push_str(&self.group);
            }
            grouped.push(digit);
        }
        self.native_digits(&grouped)
    }

    /// `value` with exactly `places` decimals, using the language's decimal mark and digits.
    pub fn decimal(&self, value: f64, places: usize) -> String {
        self.native_digits(&format!("{value:.places$}").replace('.', &self.decimal))
    }
}

/// The number format a language code usually uses, so a language file only needs to name exceptions. Languages not
/// listed use English punctuation.
pub fn defaults_for_language(tag: &str) -> NumberFormat {
    let language = tag.split(['-', '_', '.', '@']).next().unwrap_or("").to_ascii_lowercase();
    let nbsp = "\u{202f}";
    let (decimal, group, group_from, grouping) = match language.as_str() {
        "de" | "nl" | "id" | "tr" | "da" | "el" | "ro" | "hr" | "sl" | "sr" | "is" | "it" | "vi" => (",", ".", 4, Grouping::Western),
        "es" => (",", ".", 5, Grouping::Western),
        "pt" => (",", ".", 4, Grouping::Western),
        "pl" => (",", "\u{a0}", 5, Grouping::Western),
        "fr" => (",", nbsp, 4, Grouping::Western),
        "ru" | "uk" | "be" | "cs" | "sk" | "bg" | "fi" | "sv" | "nb" | "no" | "hu" | "lt" | "lv" | "et" => (",", "\u{a0}", 4, Grouping::Western),
        "hi" | "bn" | "gu" | "kn" | "ta" | "te" | "mr" => (".", ",", 4, Grouping::Indian),
        _ => (".", ",", 4, Grouping::Western),
    };
    NumberFormat { decimal: decimal.to_owned(), group: group.to_owned(), grouping, group_from, digits: String::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_follow_the_format() {
        let english = NumberFormat::default();
        assert_eq!(english.integer(999), "999");
        assert_eq!(english.integer(1_234), "1,234");
        assert_eq!(english.integer(1_234_567), "1,234,567");
        let indian = NumberFormat { grouping: Grouping::Indian, ..NumberFormat::default() };
        assert_eq!(indian.integer(1_234_567), "12,34,567");
        assert_eq!(indian.integer(123_456), "1,23,456");
        assert_eq!(indian.integer(12_345), "12,345");
        let spanish = defaults_for_language("es");
        assert_eq!(spanish.integer(1_234), "1234");
        assert_eq!(spanish.integer(12_345), "12.345");
        assert_eq!(defaults_for_language("fr").integer(1_234_567), "1\u{202f}234\u{202f}567");
        let none = NumberFormat { grouping: Grouping::None, ..NumberFormat::default() };
        assert_eq!(none.integer(1_234_567), "1234567");
    }

    #[test]
    fn decimals_and_native_digits() {
        assert_eq!(NumberFormat::default().decimal(1.5, 1), "1.5");
        assert_eq!(defaults_for_language("de").decimal(1.5, 1), "1,5");
        let arabic = NumberFormat { decimal: "٫".to_owned(), group: "٬".to_owned(), digits: "٠١٢٣٤٥٦٧٨٩".to_owned(), ..NumberFormat::default() };
        assert_eq!(arabic.integer(1_234), "١٬٢٣٤");
        assert_eq!(arabic.decimal(2.5, 1), "٢٫٥");
    }
}
