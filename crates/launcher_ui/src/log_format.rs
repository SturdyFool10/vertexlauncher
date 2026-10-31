//! Reading game and launcher logs for display: which severity a line has (Minecraft, log4j and stack-trace shapes) and
//! how its ANSI colour codes split it into styled pieces. Toolkit-free; the same rules colour the console in any UI.

use ansi_escapers::{
    interpreter::parse_ansi_annotated,
    types::{Color as AnsiColor, SgrAttribute},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

/// Carries stack-trace state from one line to the next.
#[derive(Clone, Copy, Debug, Default)]
pub struct LogParseContext {
    pub in_error_trace: bool,
}

/// The severity of `line`, given the lines before it (`context` is updated for the next line).
pub fn resolve_log_level(line: &str, context: &mut LogParseContext) -> Option<LogLevel> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        context.in_error_trace = false;
        return None;
    }
    if let Some(level) = detect_log_level(line) {
        context.in_error_trace = matches!(level, LogLevel::Error | LogLevel::Fatal);
        return Some(level);
    }
    if is_stacktrace_line(trimmed) || (context.in_error_trace && is_stacktrace_continuation_line(trimmed)) {
        context.in_error_trace = true;
        return Some(LogLevel::Error);
    }
    if trimmed.starts_with('[') {
        context.in_error_trace = false;
    }
    None
}

/// Severity of every line of a log, in order.
pub fn resolve_log_levels<S: AsRef<str>>(lines: &[S]) -> Vec<Option<LogLevel>> {
    let mut context = LogParseContext::default();
    lines.iter().map(|line| resolve_log_level(line.as_ref(), &mut context)).collect()
}

fn detect_log_level(line: &str) -> Option<LogLevel> {
    parse_minecraft_log_level(line).or_else(|| parse_generic_log_level(line))
}

fn is_stacktrace_line(trimmed: &str) -> bool {
    trimmed.starts_with("at ")
        || trimmed.starts_with("Caused by:")
        || trimmed.starts_with("Suppressed:")
        || trimmed.starts_with("Exception in thread ")
        || (trimmed.starts_with("... ") && trimmed.ends_with(" more"))
        || trimmed.contains("Exception:")
        || trimmed.ends_with("Exception")
        || trimmed.contains("Error:")
        || trimmed.ends_with("Error")
}

fn is_stacktrace_continuation_line(trimmed: &str) -> bool {
    trimmed.starts_with('\t')
        || trimmed.starts_with("com.")
        || trimmed.starts_with("net.")
        || trimmed.starts_with("org.")
        || trimmed.starts_with("java.")
        || trimmed.starts_with("javax.")
        || trimmed.starts_with("kotlin.")
        || trimmed.starts_with('#')
}

fn parse_minecraft_log_level(line: &str) -> Option<LogLevel> {
    if !line.starts_with('[') {
        return None;
    }
    let first_close = line.find(']')?;
    if first_close < 2 {
        return None;
    }
    if !looks_like_minecraft_timestamp(&line[1..first_close]) {
        return None;
    }
    let after_timestamp = line.get(first_close + 1..)?;
    if !after_timestamp.starts_with(" [") {
        return None;
    }
    let second = after_timestamp.get(2..)?;
    let second_close = second.find(']')?;
    let thread_and_level = &second[..second_close];
    if let Some((_, level_token)) = thread_and_level.rsplit_once('/')
        && let Some(level) = parse_level_token(level_token)
    {
        return Some(level);
    }
    Some(LogLevel::Info)
}

fn parse_generic_log_level(line: &str) -> Option<LogLevel> {
    for (level, name) in [
        (LogLevel::Fatal, "FATAL"),
        (LogLevel::Error, "ERROR"),
        (LogLevel::Warn, "WARN"),
        (LogLevel::Info, "INFO"),
        (LogLevel::Debug, "DEBUG"),
        (LogLevel::Trace, "TRACE"),
    ] {
        if line.contains(&format!("][{name}]["))
            || line.contains(&format!("][{name}]:"))
            || line.contains(&format!("/{name}]"))
            || line.contains(&format!("/{name}]:"))
        {
            return Some(level);
        }
    }
    None
}

fn parse_level_token(token: &str) -> Option<LogLevel> {
    match token.trim() {
        "TRACE" => Some(LogLevel::Trace),
        "DEBUG" => Some(LogLevel::Debug),
        "INFO" => Some(LogLevel::Info),
        "WARN" => Some(LogLevel::Warn),
        "ERROR" => Some(LogLevel::Error),
        "FATAL" => Some(LogLevel::Fatal),
        _ => None,
    }
}

fn looks_like_minecraft_timestamp(value: &str) -> bool {
    let mut parts = value.split(':');
    let (Some(hours), Some(minutes), Some(seconds)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    [hours, minutes, seconds].iter().all(|part| part.len() == 2 && part.as_bytes().iter().all(u8::is_ascii_digit))
}

/// One run of a line with the same style.
#[derive(Clone, Debug, PartialEq)]
pub struct StyledSegment {
    pub text: String,
    /// `None` = the line's default colour.
    pub foreground: Option<[u8; 3]>,
    pub background: Option<[u8; 3]>,
    pub underline: bool,
    pub strikethrough: bool,
}

/// `line` split at its ANSI colour codes. A line without escapes is one plain segment.
pub fn ansi_segments(line: &str) -> Vec<StyledSegment> {
    if !line.contains('\x1b') {
        return vec![StyledSegment { text: line.to_owned(), foreground: None, background: None, underline: false, strikethrough: false }];
    }
    let parsed = parse_ansi_annotated(line);
    let text = parsed.text;
    // Boundaries where any span starts or ends.
    let mut cuts: Vec<usize> = vec![0, text.len()];
    for span in &parsed.spans {
        cuts.push(span.start.min(text.len()));
        cuts.push(span.end.min(text.len()));
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut segments = Vec::new();
    for window in cuts.windows(2) {
        let (start, end) = (window[0], window[1]);
        if start >= end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            continue;
        }
        let mut segment = StyledSegment {
            text: text[start..end].to_owned(),
            foreground: None,
            background: None,
            underline: false,
            strikethrough: false,
        };
        for span in parsed.spans.iter().filter(|span| span.start <= start && span.end >= end) {
            for code in &span.codes {
                match code {
                    SgrAttribute::Foreground(color) => segment.foreground = Some(ansi_rgb(color)),
                    SgrAttribute::Background(color) => segment.background = Some(ansi_rgb(color)),
                    SgrAttribute::Underline => segment.underline = true,
                    SgrAttribute::CrossedOut => segment.strikethrough = true,
                    SgrAttribute::Reset => {
                        segment.foreground = None;
                        segment.background = None;
                        segment.underline = false;
                        segment.strikethrough = false;
                    }
                    _ => {}
                }
            }
        }
        segments.push(segment);
    }
    segments
}

/// The text of `line` without ANSI escapes.
pub fn strip_ansi(line: &str) -> String {
    if line.contains('\x1b') { parse_ansi_annotated(line).text } else { line.to_owned() }
}

/// `line` as styled runs: split at its ANSI colour codes, and each of those at the Minecraft section-sign codes in it
/// (see [`crate::mc_text`]). A run's colour of `None` is the line's own colour, and `§r` goes back to what the ANSI
/// code (or the line) set, not to white.
pub fn styled_runs(line: &str) -> Vec<crate::mc_text::Span> {
    let mut runs = Vec::new();
    for segment in ansi_segments(line) {
        let base = crate::mc_text::Style {
            color: segment.foreground,
            underline: segment.underline,
            strikethrough: segment.strikethrough,
            ..Default::default()
        };
        runs.extend(crate::mc_text::parse_legacy(&segment.text, base));
    }
    runs
}

/// The text of `line` as it reads once the ANSI escapes and the section-sign codes are taken out: what is shown,
/// selected and copied.
pub fn plain_text(line: &str) -> String {
    if !line.contains('\x1b') && !line.contains('§') {
        return line.to_owned();
    }
    crate::mc_text::plain(&styled_runs(line))
}

/// xterm's default palette for the 16 named colours, then the 6x6x6 cube and the grey ramp.
fn ansi_rgb(color: &AnsiColor) -> [u8; 3] {
    match color {
        AnsiColor::Black => [0x4C, 0x4C, 0x4C],
        AnsiColor::Red => [0xC5, 0x0F, 0x1F],
        AnsiColor::Green => [0x13, 0xA1, 0x0E],
        AnsiColor::Yellow => [0xC1, 0x9C, 0x00],
        AnsiColor::Blue => [0x00, 0x37, 0xDA],
        AnsiColor::Magenta => [0x88, 0x17, 0x98],
        AnsiColor::Cyan => [0x3A, 0x96, 0xDD],
        AnsiColor::White => [0xCC, 0xCC, 0xCC],
        AnsiColor::BrightBlack => [0x76, 0x76, 0x76],
        AnsiColor::BrightRed => [0xE7, 0x48, 0x56],
        AnsiColor::BrightGreen => [0x16, 0xC6, 0x0C],
        AnsiColor::BrightYellow => [0xF9, 0xF1, 0xA5],
        AnsiColor::BrightBlue => [0x3B, 0x78, 0xFF],
        AnsiColor::BrightMagenta => [0xB4, 0x00, 0x9E],
        AnsiColor::BrightCyan => [0x61, 0xD6, 0xD6],
        AnsiColor::BrightWhite => [0xF2, 0xF2, 0xF2],
        AnsiColor::AnsiValue(index) => ansi_256_rgb(*index),
        AnsiColor::Rgb24 { r, g, b } => [*r, *g, *b],
    }
}

fn ansi_256_rgb(index: u8) -> [u8; 3] {
    match index {
        0..=15 => ansi_rgb(&match index {
            0 => AnsiColor::Black,
            1 => AnsiColor::Red,
            2 => AnsiColor::Green,
            3 => AnsiColor::Yellow,
            4 => AnsiColor::Blue,
            5 => AnsiColor::Magenta,
            6 => AnsiColor::Cyan,
            7 => AnsiColor::White,
            8 => AnsiColor::BrightBlack,
            9 => AnsiColor::BrightRed,
            10 => AnsiColor::BrightGreen,
            11 => AnsiColor::BrightYellow,
            12 => AnsiColor::BrightBlue,
            13 => AnsiColor::BrightMagenta,
            14 => AnsiColor::BrightCyan,
            _ => AnsiColor::BrightWhite,
        }),
        16..=231 => {
            let value = u32::from(index) - 16;
            let channel = |c: u32| if c == 0 { 0u8 } else { (55 + c * 40) as u8 };
            [channel(value / 36), channel((value / 6) % 6), channel(value % 6)]
        }
        232..=255 => {
            let grey = (8 + (u32::from(index) - 232) * 10).min(255) as u8;
            [grey, grey, grey]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minecraft_lines_carry_their_level() {
        let mut context = LogParseContext::default();
        assert_eq!(resolve_log_level("[12:30:45] [main/WARN]: careful", &mut context), Some(LogLevel::Warn));
        assert_eq!(resolve_log_level("[12:30:46] [Render thread/ERROR]: boom", &mut context), Some(LogLevel::Error));
        // A stack trace after an error stays an error.
        assert_eq!(resolve_log_level("\tat net.minecraft.Foo.bar(Foo.java:1)", &mut context), Some(LogLevel::Error));
        assert_eq!(resolve_log_level("[12:30:47] [main/INFO]: fine", &mut context), Some(LogLevel::Info));
        assert_eq!(resolve_log_level("plain text", &mut context), None);
    }

    #[test]
    fn ansi_codes_split_a_line_into_coloured_segments() {
        let segments = ansi_segments("A\x1b[31mB\x1b[0mC");
        let text: Vec<&str> = segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, ["A", "B", "C"]);
        assert_eq!(segments[0].foreground, None);
        assert_eq!(segments[1].foreground, Some([0xC5, 0x0F, 0x1F]));
        assert_eq!(strip_ansi("A\x1b[31mB\x1b[0mC"), "ABC");
    }

    #[test]
    fn section_codes_in_a_console_line_reset_to_the_lines_own_colour() {
        let runs = styled_runs("[CHAT] §aGreen§r back");
        assert_eq!(crate::mc_text::plain(&runs), "[CHAT] Green back");
        // The line has no ANSI colour, so "back" is the line's own colour (None), not white.
        assert_eq!(runs.last().unwrap().style.color, None);
        assert_eq!(plain_text("[CHAT] §aGreen§r back"), "[CHAT] Green back");
        // With an ANSI colour set, §r returns to that.
        let runs = styled_runs("\x1b[31m§aGreen§rRed\x1b[0m");
        assert_eq!(runs[0].style.color, Some([0x55, 0xFF, 0x55]));
        assert_eq!(runs[1].style.color, ansi_segments("\x1b[31mx\x1b[0m")[0].foreground);
    }
}
