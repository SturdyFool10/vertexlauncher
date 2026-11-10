//! How the launcher shows a moment in time (the "Taken at ..." card on screenshots, and whatever else wants it) and how
//! long ago it was. Times are signed 64-bit seconds since the Unix epoch, so there is no 2038 limit, and the calendar
//! maths is `jiff`'s, which covers years -9999 to 9999 and the system time zone database.

use jiff::{Span, Timestamp, Unit, Zoned, tz::TimeZone};
use serde::{Deserialize, Serialize};

macro_rules! choice {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $label:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub const fn label(self) -> &'static str {
                match self { $($name::$variant => $label),+ }
            }
        }
    };
}

choice! {
    /// A ready-made date layout from somewhere in the world, or `Custom` to build one from the settings below it. The
    /// examples are for 10 November 2026.
    DateFormat {
        Custom => "Custom (set it below)",
        UnitedStates => "United States (11/10/2026)",
        UnitedStatesLong => "United States, long (November 10, 2026)",
        Europe => "UK, France, Spain, Italy, Brazil, Australia (10/11/2026)",
        EuropeLong => "UK and Europe, long (10 November 2026)",
        Germany => "Germany, Russia, Poland, Turkey (10.11.2026)",
        Netherlands => "Netherlands, Belgium (10-11-2026)",
        India => "India, Bangladesh (10-11-2026, day first)",
        Iso => "ISO 8601, Sweden, Canada, South Africa (2026-11-10)",
        Japan => "Japan, China (2026年11月10日)",
        Korea => "South Korea (2026. 11. 10.)",
        Thailand => "Thailand, Buddhist Era (10/11/2569)",
        Taiwan => "Taiwan, Minguo calendar (115/11/10)",
    }
}

choice! {
    /// The order the parts of a date come in.
    DateOrder {
        MonthDayYear => "Month, day, year",
        DayMonthYear => "Day, month, year",
        YearMonthDay => "Year, month, day (ISO 8601 order)",
    }
}

choice! {
    /// What goes between the numbers of a numeric date.
    DateSeparator {
        Dash => "Dash (-)",
        Slash => "Slash (/)",
        Dot => "Dot (.)",
        Space => "Space",
    }
}

choice! {
    /// How the month is written.
    MonthStyle {
        Numeric => "Number (11)",
        Short => "Short name (Nov)",
        Long => "Full name (November)",
    }
}

choice! {
    /// 12-hour or 24-hour clock.
    ClockStyle {
        Hour12 => "12-hour",
        Hour24 => "24-hour",
    }
}

choice! {
    /// How AM and PM are written on a 12-hour clock.
    MeridiemStyle {
        Upper => "AM / PM",
        Lower => "am / pm",
        UpperDotted => "A.M. / P.M.",
        LowerDotted => "a.m. / p.m.",
    }
}

choice! {
    /// What goes between hours, minutes and seconds.
    TimeSeparator {
        Colon => "Colon (:)",
        Dot => "Dot (.)",
    }
}

choice! {
    /// Which time zone times are shown in.
    DisplayTimeZone {
        Local => "This computer's time zone",
        Utc => "UTC",
    }
}

/// Every setting of the "Displayed Time" section.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayedTime {
    /// The "Taken at ..." card over a screenshot.
    pub show_screenshot_info: bool,
    /// A regional date layout; anything but `Custom` overrides the date settings that follow (not the weekday).
    pub date_format: DateFormat,
    /// Count years in the Human Era (Holocene) calendar, the Gregorian year plus 10,000, and mark them `HE`. Only for
    /// layouts where `DateFormat::keeps_gregorian_year`.
    pub human_era: bool,
    pub date_order: DateOrder,
    pub date_separator: DateSeparator,
    pub month_style: MonthStyle,
    pub four_digit_year: bool,
    /// Zero-pad the day and a numeric month (`03` rather than `3`).
    pub pad_date: bool,
    /// Put the day of the week in front of the date.
    pub show_weekday: bool,
    pub clock: ClockStyle,
    pub pad_hour: bool,
    pub show_seconds: bool,
    pub meridiem: MeridiemStyle,
    pub time_separator: TimeSeparator,
    pub time_zone: DisplayTimeZone,
    /// The "Taken 2 days 3 hours ago" line.
    pub show_relative: bool,
    /// The most units the relative line shows, largest first (1 to 5: years, days, hours, minutes, seconds).
    pub relative_max_units: u8,
    /// Leave out units that are zero (otherwise every unit from the largest one down is shown).
    pub relative_hide_zero: bool,
    /// `1y 2d 3h` rather than `1 year 2 days 3 hours`.
    pub relative_abbreviate: bool,
}

pub const RELATIVE_MAX_UNITS_MIN: u8 = 1;
pub const RELATIVE_MAX_UNITS_MAX: u8 = 5;

impl Default for DisplayedTime {
    fn default() -> Self {
        Self {
            show_screenshot_info: true,
            date_format: DateFormat::Custom,
            human_era: false,
            date_order: DateOrder::MonthDayYear,
            date_separator: DateSeparator::Space,
            month_style: MonthStyle::Numeric,
            four_digit_year: true,
            pad_date: true,
            show_weekday: false,
            clock: ClockStyle::Hour12,
            pad_hour: true,
            show_seconds: true,
            meridiem: MeridiemStyle::Upper,
            time_separator: TimeSeparator::Colon,
            time_zone: DisplayTimeZone::Local,
            show_relative: true,
            relative_max_units: RELATIVE_MAX_UNITS_MAX,
            relative_hide_zero: true,
            relative_abbreviate: false,
        }
    }
}

/// Signed seconds since the Unix epoch for `time` (negative before 1970).
pub fn unix_seconds(time: std::time::SystemTime) -> i64 {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_secs()).map_or(i64::MIN, |seconds| -seconds),
    }
}

/// The current time as signed Unix seconds.
pub fn unix_now() -> i64 {
    unix_seconds(std::time::SystemTime::now())
}

/// When the screenshot at `path` was taken: from a Minecraft-style file name (`2026-11-10_21.15.30.png`, written in the
/// computer's own time zone) when it has one, otherwise from the file's modified time.
pub fn screenshot_taken_at(path: &std::path::Path) -> Option<i64> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(taken_at_from_file_stem)
        .or_else(|| std::fs::metadata(path).and_then(|meta| meta.modified()).ok().map(unix_seconds))
}

fn taken_at_from_file_stem(stem: &str) -> Option<i64> {
    // `yyyy-mm-dd_hh.mm.ss`, optionally followed by `_1` for a second screenshot in the same second.
    let (date, rest) = stem.split_once('_')?;
    let time = rest.split('_').next()?;
    let mut date = date.split('-').map(|part| part.parse::<i16>().ok());
    let mut time = time.split('.').map(|part| part.parse::<i8>().ok());
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    let (hour, minute, second) = (time.next()??, time.next()??, time.next()??);
    if date.next().is_some() || time.next().is_some() {
        return None;
    }
    let civil = jiff::civil::DateTime::new(year, i8::try_from(month).ok()?, i8::try_from(day).ok()?, hour, minute, second, 0).ok()?;
    Some(civil.to_zoned(TimeZone::system()).ok()?.timestamp().as_second())
}

impl DateFormat {
    /// Whether the layout counts years the usual (Gregorian) way, so the Human Era option can apply to it. Thailand and
    /// Taiwan already count from a different year.
    pub const fn keeps_gregorian_year(self) -> bool {
        !matches!(self, DateFormat::Thailand | DateFormat::Taiwan)
    }
}

/// A regional date layout (anything but `Custom`) for a year, month and day.
fn preset_date(format: DateFormat, year: i32, month: usize, day: i8) -> String {
    match format {
        DateFormat::Custom | DateFormat::UnitedStates => format!("{month:02}/{day:02}/{year}"),
        DateFormat::UnitedStatesLong => format!("{} {day}, {year}", LONG_MONTHS[month - 1]),
        DateFormat::Europe => format!("{day:02}/{month:02}/{year}"),
        DateFormat::EuropeLong => format!("{day} {} {year}", LONG_MONTHS[month - 1]),
        DateFormat::Germany => format!("{day:02}.{month:02}.{year}"),
        DateFormat::Netherlands | DateFormat::India => format!("{day:02}-{month:02}-{year}"),
        DateFormat::Iso => format!("{year:04}-{month:02}-{day:02}"),
        DateFormat::Japan => format!("{year}年{month:02}月{day:02}日"),
        DateFormat::Korea => format!("{year}. {month:02}. {day:02}."),
        // The Thai Buddhist Era runs 543 years ahead of the Gregorian count.
        DateFormat::Thailand => format!("{day:02}/{month:02}/{}", year + 543),
        // The Republic of China (Minguo) calendar counts from 1912.
        DateFormat::Taiwan => format!("{}/{month:02}/{day:02}", year - 1911),
    }
}

const SHORT_MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const LONG_MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];
const SHORT_DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

impl DisplayedTime {
    fn zone(&self) -> TimeZone {
        match self.time_zone {
            DisplayTimeZone::Local => TimeZone::system(),
            DisplayTimeZone::Utc => TimeZone::UTC,
        }
    }

    fn zoned(&self, unix_seconds: i64) -> Option<Zoned> {
        Some(Timestamp::from_second(unix_seconds).ok()?.to_zoned(self.zone()))
    }

    /// `11-10, 2026`, `Nov 10, 2026`, `2026-11-10`, ... per the date settings.
    pub fn format_date(&self, unix_seconds: i64) -> Option<String> {
        let at = self.zoned(unix_seconds)?;
        let (year, month, day) = (i32::from(at.year()), at.month() as usize, at.day());
        let human_era = self.human_era && self.date_format.keeps_gregorian_year();
        let year = if human_era { year + 10_000 } else { year };
        let mut date = match self.date_format {
            DateFormat::Custom => self.custom_date(year, month, day),
            preset => preset_date(preset, year, month, day),
        };
        if human_era {
            date.push_str(" HE");
        }
        Some(if self.show_weekday {
            let weekday = SHORT_DAYS[at.weekday().to_monday_zero_offset() as usize];
            format!("{weekday}, {date}")
        } else {
            date
        })
    }

    /// The date from the order, separator, month style and padding settings.
    fn custom_date(&self, year: i32, month: usize, day: i8) -> String {
        let year = if self.four_digit_year { format!("{year}") } else { format!("{:02}", year.rem_euclid(100)) };
        let day = if self.pad_date { format!("{day:02}") } else { day.to_string() };
        let separator = match self.date_separator {
            DateSeparator::Dash => "-",
            DateSeparator::Slash => "/",
            DateSeparator::Dot => ".",
            DateSeparator::Space => " ",
        };
        let month_text = match self.month_style {
            MonthStyle::Numeric if self.pad_date => format!("{month:02}"),
            MonthStyle::Numeric => month.to_string(),
            MonthStyle::Short => SHORT_MONTHS[month - 1].to_owned(),
            MonthStyle::Long => LONG_MONTHS[month - 1].to_owned(),
        };
        let named = self.month_style != MonthStyle::Numeric;
        let date = match (self.date_order, named) {
            (DateOrder::MonthDayYear, _) => {
                let between = if named { " " } else { separator };
                format!("{month_text}{between}{day}, {year}")
            }
            (DateOrder::DayMonthYear, true) => format!("{day} {month_text} {year}"),
            (DateOrder::DayMonthYear, false) => format!("{day}{separator}{month_text}{separator}{year}"),
            (DateOrder::YearMonthDay, true) => format!("{year} {month_text} {day}"),
            (DateOrder::YearMonthDay, false) => format!("{year}{separator}{month_text}{separator}{day}"),
        };
        date
    }

    /// `02:15:30 PM`, `14.15`, ... per the time settings.
    pub fn format_time(&self, unix_seconds: i64) -> Option<String> {
        let at = self.zoned(unix_seconds)?;
        let (hour, minute, second) = (at.hour(), at.minute(), at.second());
        let separator = match self.time_separator {
            TimeSeparator::Colon => ":",
            TimeSeparator::Dot => ".",
        };
        let shown_hour = match self.clock {
            ClockStyle::Hour24 => hour,
            ClockStyle::Hour12 => match hour % 12 {
                0 => 12,
                other => other,
            },
        };
        let hour_text = if self.pad_hour { format!("{shown_hour:02}") } else { shown_hour.to_string() };
        let mut text = format!("{hour_text}{separator}{minute:02}");
        if self.show_seconds {
            text.push_str(&format!("{separator}{second:02}"));
        }
        if self.clock == ClockStyle::Hour12 {
            let pm = hour >= 12;
            text.push(' ');
            text.push_str(match (self.meridiem, pm) {
                (MeridiemStyle::Upper, false) => "AM",
                (MeridiemStyle::Upper, true) => "PM",
                (MeridiemStyle::Lower, false) => "am",
                (MeridiemStyle::Lower, true) => "pm",
                (MeridiemStyle::UpperDotted, false) => "A.M.",
                (MeridiemStyle::UpperDotted, true) => "P.M.",
                (MeridiemStyle::LowerDotted, false) => "a.m.",
                (MeridiemStyle::LowerDotted, true) => "p.m.",
            });
        }
        Some(text)
    }

    /// `Taken at 11-10, 2026 at 02:15:30 PM`.
    pub fn taken_at(&self, unix_seconds: i64) -> String {
        match (self.format_date(unix_seconds), self.format_time(unix_seconds)) {
            (Some(date), Some(time)) => format!("Taken at {date} at {time}"),
            _ => "Taken at an unknown time".to_owned(),
        }
    }

    /// `1 year 2 days 3 hours 4 seconds ago`, or `... from now` for a time that has not come yet.
    pub fn time_ago(&self, then_unix_seconds: i64, now_unix_seconds: i64) -> String {
        let Some(delta) = self.relative_units(then_unix_seconds, now_unix_seconds) else {
            return "At an unknown time".to_owned();
        };
        let future = then_unix_seconds > now_unix_seconds;
        if delta.is_empty() {
            return "Just now".to_owned();
        }
        format!("{} {}", delta, if future { "from now" } else { "ago" })
    }

    /// The gap between the two moments as the units the settings ask for. `None` when either is out of range.
    fn relative_units(&self, then: i64, now: i64) -> Option<String> {
        let zone = self.zone();
        let (earlier, later) = (then.min(now), then.max(now));
        let earlier = Timestamp::from_second(earlier).ok()?.to_zoned(zone.clone());
        let later = Timestamp::from_second(later).ok()?.to_zoned(zone);
        // Whole calendar years first, then everything else from that point as days and below (no months).
        let years = earlier.until((Unit::Year, &later)).ok()?.get_years();
        let after_years = earlier.checked_add(Span::new().years(i64::from(years))).ok()?;
        let rest = after_years.until((Unit::Day, &later)).ok()?;
        let mut parts: Vec<(i64, [&str; 3])> = vec![
            (i64::from(years), ["year", "years", "y"]),
            (i64::from(rest.get_days()), ["day", "days", "d"]),
            (i64::from(rest.get_hours()), ["hour", "hours", "h"]),
            (rest.get_minutes(), ["minute", "minutes", "m"]),
            (rest.get_seconds(), ["second", "seconds", "s"]),
        ];
        let Some(first) = parts.iter().position(|(value, _)| *value != 0) else {
            return Some(String::new());
        };
        parts.drain(..first);
        if self.relative_hide_zero {
            parts.retain(|(value, _)| *value != 0);
        }
        parts.truncate(usize::from(self.relative_max_units.clamp(RELATIVE_MAX_UNITS_MIN, RELATIVE_MAX_UNITS_MAX)));
        Some(
            parts
                .iter()
                .map(|(value, names)| {
                    if self.relative_abbreviate {
                        format!("{value}{}", names[2])
                    } else {
                        format!("{value} {}", names[usize::from(*value != 1)])
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc() -> DisplayedTime {
        DisplayedTime { time_zone: DisplayTimeZone::Utc, ..DisplayedTime::default() }
    }

    // 2026-11-10 21:15:30 UTC
    const SAMPLE: i64 = 1_794_345_330;

    #[test]
    fn default_matches_the_requested_layout() {
        assert_eq!(utc().taken_at(SAMPLE), "Taken at 11 10, 2026 at 09:15:30 PM");
    }

    #[test]
    fn date_and_clock_settings_apply() {
        let settings = DisplayedTime {
            date_order: DateOrder::YearMonthDay,
            date_separator: DateSeparator::Dash,
            clock: ClockStyle::Hour24,
            show_seconds: false,
            ..utc()
        };
        assert_eq!(settings.taken_at(SAMPLE), "Taken at 2026-11-10 at 21:15");
        let named = DisplayedTime {
            month_style: MonthStyle::Long,
            meridiem: MeridiemStyle::LowerDotted,
            pad_hour: false,
            show_weekday: true,
            ..utc()
        };
        assert_eq!(named.taken_at(SAMPLE), "Taken at Tue, November 10, 2026 at 9:15:30 p.m.");
    }

    #[test]
    fn minecraft_screenshot_names_give_a_time() {
        assert!(taken_at_from_file_stem("2026-11-10_21.15.30").is_some());
        assert!(taken_at_from_file_stem("2026-11-10_21.15.30_1").is_some());
        assert_eq!(taken_at_from_file_stem("my screenshot"), None);
        assert_eq!(taken_at_from_file_stem("2026-13-10_21.15.30"), None);
    }

    #[test]
    fn regional_formats_write_the_same_day_their_own_way() {
        let write = |date_format| DisplayedTime { date_format, ..utc() }.format_date(SAMPLE).unwrap();
        assert_eq!(write(DateFormat::UnitedStates), "11/10/2026");
        assert_eq!(write(DateFormat::UnitedStatesLong), "November 10, 2026");
        assert_eq!(write(DateFormat::Europe), "10/11/2026");
        assert_eq!(write(DateFormat::EuropeLong), "10 November 2026");
        assert_eq!(write(DateFormat::Germany), "10.11.2026");
        assert_eq!(write(DateFormat::Iso), "2026-11-10");
        assert_eq!(write(DateFormat::Japan), "2026年11月10日");
        assert_eq!(write(DateFormat::Korea), "2026. 11. 10.");
        assert_eq!(write(DateFormat::Thailand), "10/11/2569");
        assert_eq!(write(DateFormat::Taiwan), "115/11/10");
    }

    #[test]
    fn human_era_adds_ten_thousand_years_where_the_year_is_the_usual_one() {
        let era = |date_format| DisplayedTime { date_format, human_era: true, ..utc() }.format_date(SAMPLE).unwrap();
        assert_eq!(era(DateFormat::Custom), "11 10, 12026 HE");
        assert_eq!(era(DateFormat::Iso), "12026-11-10 HE");
        assert_eq!(era(DateFormat::EuropeLong), "10 November 12026 HE");
        // These count years their own way, so the option leaves them alone.
        assert_eq!(era(DateFormat::Thailand), "10/11/2569");
        assert_eq!(era(DateFormat::Taiwan), "115/11/10");
    }

    #[test]
    fn times_past_2038_and_far_out_still_format() {
        // Both are well beyond a 32-bit time_t.
        assert!(utc().format_date(7_000_000_000).unwrap().ends_with("2191"));
        assert!(utc().format_date(60_000_000_000).unwrap().ends_with("3871"));
        assert_eq!(utc().format_date(i64::MAX), None);
    }

    #[test]
    fn relative_time_counts_calendar_years_and_smaller_units() {
        let then = SAMPLE;
        let now = then + 2 * 366 * 86_400 + 3 * 3_600 + 4;
        assert_eq!(utc().time_ago(then, now), "2 years 1 day 3 hours 4 seconds ago");
        assert_eq!(utc().time_ago(now, then), "2 years 1 day 3 hours 4 seconds from now");
        let tight = DisplayedTime { relative_max_units: 2, relative_abbreviate: true, ..utc() };
        assert_eq!(tight.time_ago(then, now), "2y 1d ago");
        assert_eq!(utc().time_ago(then, then), "Just now");
        let zeros = DisplayedTime { relative_hide_zero: false, ..utc() };
        assert_eq!(zeros.time_ago(then, then + 3_604), "1 hour 0 minutes 4 seconds ago");
    }
}
