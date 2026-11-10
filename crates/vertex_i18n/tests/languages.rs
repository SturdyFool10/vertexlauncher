//! Languages described entirely as data: nothing here needs a code change in the launcher.

use std::time::Duration;

use vertex_i18n::{ListKind, Locale, LocaleData, PluralForms};

fn locale(source: &str) -> Locale {
    Locale::from_data(toml::from_str::<LocaleData>(source).expect("locale data parses"))
}

const RUSSIAN: &str = r#"
tag = "ru"

[words]
count = ["ноль", "один", "два", "три", "четыре", "пять", "шесть", "семь", "восемь", "девять", "десять"]
several = "несколько"
just_now = "только что"
ago = "{span} назад"

[units]
day = { one = "день", few = "дня", many = "дней", other = "дня" }
hour = { one = "час", few = "часа", many = "часов", other = "часа" }
minute = { one = "минута", few = "минуты", many = "минут", other = "минуты" }
second = { one = "секунда", few = "секунды", many = "секунд", other = "секунды" }

[lists.and]
two = "{0} и {1}"
start = "{0}, {1}"
middle = "{0}, {1}"
end = "{0} и {1}"
"#;

const ARABIC: &str = r#"
tag = "ar"

[number]
decimal = "٫"
group = "٬"
digits = "٠١٢٣٤٥٦٧٨٩"

[units]
minute = { zero = "لا دقائق", one = "دقيقة", two = "دقيقتان", few = "دقائق", many = "دقيقة", other = "دقيقة" }
second = { one = "ثانية", two = "ثانيتان", few = "ثوان", many = "ثانية", other = "ثانية" }
"#;

const HINDI: &str = r#"
tag = "hi"
"#;

#[test]
fn a_slavic_language_picks_three_forms_and_declines_durations() {
    let russian = locale(RUSSIAN);
    assert_eq!(russian.duration_words(Duration::from_secs(61)), "1 минута и 1 секунда");
    assert_eq!(russian.duration_words(Duration::from_secs(2 * 60 + 5)), "2 минуты и 5 секунд");
    assert_eq!(russian.duration_words(Duration::from_secs(21 * 60)), "21 минута");
    assert_eq!(russian.duration_words(Duration::from_secs(11 * 3_600)), "11 часов");
    assert_eq!(russian.time_ago(Duration::from_secs(3 * 3_600)), "3 часа назад");
    assert_eq!(russian.time_ago(Duration::from_secs(1)), "только что");
    assert_eq!(russian.join(&["A", "B", "C"], ListKind::And), "A, B и C");
    // Russian groups with a no-break space and uses a decimal comma without any further data.
    assert_eq!(russian.thousands(1_234_567), "1\u{a0}234\u{a0}567");
    assert_eq!(russian.decimal(2.5, 1), "2,5");
}

#[test]
fn a_six_form_language_with_its_own_digits() {
    let arabic = locale(ARABIC);
    assert_eq!(arabic.thousands(12_345), "١٢٬٣٤٥");
    assert_eq!(arabic.decimal(1.5, 1), "١٫٥");
    assert_eq!(arabic.duration_words(Duration::from_secs(120)), "٢ دقيقتان");
    assert_eq!(arabic.duration_words(Duration::from_secs(5)), "٥ ثوان");
    assert_eq!(arabic.duration_words(Duration::from_secs(60 * 25)), "٢٥ دقيقة");
    let forms = PluralForms::new("one", "other").with_two("two").with_few("few").with_many("many").with_zero("zero");
    assert_eq!(arabic.select(&forms, 0), "zero");
    assert_eq!(arabic.select(&forms, 2), "two");
    assert_eq!(arabic.select(&forms, 7), "few");
    assert_eq!(arabic.select(&forms, 42), "many");
    assert_eq!(arabic.select(&forms, 100), "other");
}

#[test]
fn indian_digit_grouping_comes_from_the_language_code() {
    let hindi = locale(HINDI);
    assert_eq!(hindi.thousands(1_234_567), "12,34,567");
    assert_eq!(hindi.plural_category(0), vertex_i18n::PluralCategory::One);
}

#[test]
fn a_language_that_says_nothing_falls_back_to_english_wording_with_its_own_rules() {
    let polish = locale("tag = \"pl\"");
    assert_eq!(polish.plural_category(22), vertex_i18n::PluralCategory::Few);
    assert_eq!(polish.plural_category(5), vertex_i18n::PluralCategory::Many);
    assert_eq!(polish.thousands(12_345), "12\u{a0}345");
    assert_eq!(polish.count_word(3), "three");
}
