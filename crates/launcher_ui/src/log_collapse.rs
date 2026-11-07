//! Merging of repeated lines in a console, done as the console is drawn.
//!
//! The log itself keeps every line. What a console shows is the merge of them, in two passes:
//!
//! 1. **Consecutive repeats.** A line identical to the one before it is not shown again; it is counted, and the last of
//!    them is shown with `(repeated N more times)` after it.
//! 2. **Patterns.** A short run of lines (up to [`WINDOW`], each run of consecutive repeats counting as one line)
//!    that repeats as a whole is shown once, followed by `(the previous 3 log lines repeated 5 more times)`.
//!
//! Two lines are the same when everything after the date and time is: lines the launcher logs start with
//! `[date][time]`, and a game's with `[HH:MM:SS]` or a dated variant of it, which nearly always differ, so they are left
//! out of the comparison. So are the figures that differ every time a line repeats without it being a different line:
//! a duration (`198ms 49us`) and a memory address (`0x2924e520790`). Anything merged is dated by the latest line it
//! covers, because the lines shown for it are the latest ones.
//!
//! [`Merger`] does this a frame at a time: each frame it takes only the lines that are new since the last, merges them
//! into what it already holds, and gives back what to show. It keeps the recent unmerged runs between frames, so a
//! pattern whose occurrences were logged over many different frames merges just the same as one logged in a single
//! burst; only runs old enough that nothing later can change them are settled and let go. [`collapse`] is the same
//! merge over a whole list at once.

/// The longest pattern, in lines, that is looked for. A burst of one frame's worth of engine log (shader compile, its
/// imports, the timing and the long-frame warning) is already seven.
pub const WINDOW: usize = 12;

/// The longest cycle the first, nesting passes look for. Short cycles are merged first, to a fixed point, so a group
/// that repeats inside a larger repeat is shown as such ("a b" twice, inside "a b c d" three times); only then are the
/// longer cycles up to [`WINDOW`] looked for among what is left. Looking for the long ones straight away would take the
/// flat cycle that covers the most and lose the nesting.
const NESTING_WINDOW: usize = 5;

/// How many of the newest runs [`Merger`] always keeps open. A pattern can be made of patterns, and its second
/// occurrence may still be a long stretch of unmerged runs when the first occurrence is already far back, so what is
/// recent is not settled however far from the end it looks. A pattern whose occurrences are within this many runs of
/// each other merges exactly as it would if the whole log were merged at once; beyond it the older occurrence is final.
const KEEP_OPEN: usize = 512;

/// A run of consecutive identical lines.
struct Entry<'a> {
    /// What is compared: the line without its date and time or colour codes.
    key: &'a str,
    /// The first line of the run, as stored.
    first: &'a str,
    /// The last line of the run, as stored (the latest date and time).
    last: &'a str,
    count: usize,
}

/// Whether `group` (the text between a pair of brackets) is a clock time or a date and time: `13:45:01`, `13:45:01.123`,
/// `2026-11-06T13:45:01Z`, `06Nov2026 13:45:01.123`. A group that also names a level (`13:45:01 INFO`) is not one: the level is
/// part of what is compared.
fn is_time_group(group: &str) -> bool {
    // The clock is the last token after a space or a `T`; whatever precedes it is a date.
    let (date, clock) = match group.rfind([' ', 'T']) {
        Some(at) => (&group[..at], &group[at + 1..]),
        None => ("", group),
    };
    let clock = clock.strip_suffix('Z').unwrap_or(clock).as_bytes();
    let digit = |at: usize| clock.get(at).is_some_and(u8::is_ascii_digit);
    let clock_ok = clock.len() >= 8
        && [0, 1, 3, 4, 6, 7].iter().all(|at| digit(*at))
        && clock[2] == b':'
        && clock[5] == b':'
        && (clock.len() == 8 || (clock[8] == b'.' && clock.len() > 9 && clock[9..].iter().all(u8::is_ascii_digit)));
    clock_ok && date.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '/' | '.' | ' '))
}

/// `line` without a leading `[date][time]`, when it has one, or the leading `[time]` / `[date time]` a game writes.
fn without_stamp(line: &str) -> &str {
    // `[date][time]`, as the launcher writes it.
    let two_groups = (|| {
        let mut rest = line;
        for _ in 0..2 {
            let after = rest.strip_prefix('[')?;
            let close = after.find(']')?;
            rest = &after[close + 1..];
        }
        Some(rest)
    })();
    if let Some(rest) = two_groups {
        return rest;
    }
    // One bracketed time, as a game writes it ("[13:45:01] [Render thread/INFO]: ...").
    if let Some(after) = line.strip_prefix('[')
        && let Some(close) = after.find(']')
        && is_time_group(&after[..close])
    {
        return after[close + 1..].trim_start();
    }
    line
}

/// `text` with the figures that change from one repeat of a line to the next replaced by placeholders: durations (`198ms`,
/// `1.5s`, and runs of them like `198ms 49us 300ns`) and hexadecimal addresses (`0x2924e520790`). Everything else, numbers
/// included, is compared as it is: `file 1 of 10` and `file 2 of 10` are different lines.
fn normalize_volatile(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    // A figure only counts at the start of a token, so `x264ms` or `abc123s` are left alone.
    let at_boundary = |i: usize| i == 0 || !chars[i - 1].is_alphanumeric();
    // Reads `<digits>[.<digits>]<unit>` at `from`; returns where it ends.
    let duration_at = |from: usize| -> Option<usize> {
        let mut j = from;
        while j < chars.len() && chars[j].is_ascii_digit() {
            j += 1;
        }
        if j == from {
            return None;
        }
        if j + 1 < chars.len() && chars[j] == '.' && chars[j + 1].is_ascii_digit() {
            j += 1;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
        }
        // The longest unit that fits, so `ms` is not read as `m` + junk.
        for unit in ["ms", "ns", "us", "\u{b5}s", "s"] {
            let u: Vec<char> = unit.chars().collect();
            if chars[j..].starts_with(&u) && !chars.get(j + u.len()).is_some_and(|c| c.is_alphanumeric()) {
                return Some(j + u.len());
            }
        }
        None
    };
    while i < chars.len() {
        if at_boundary(i) {
            if chars[i] == '0' && matches!(chars.get(i + 1), Some('x' | 'X')) {
                let mut j = i + 2;
                while j < chars.len() && chars[j].is_ascii_hexdigit() {
                    j += 1;
                }
                if j - (i + 2) >= 4 && !chars.get(j).is_some_and(|c| c.is_alphanumeric()) {
                    out.push_str("<addr>");
                    i = j;
                    continue;
                }
            }
            if chars[i].is_ascii_digit()
                && let Some(mut end) = duration_at(i)
            {
                // `198ms 49us 300ns`: further durations after single spaces belong to the same figure.
                while end + 1 < chars.len() && chars[end] == ' ' && chars[end + 1].is_ascii_digit() {
                    match duration_at(end + 1) {
                        Some(next) => end = next,
                        None => break,
                    }
                }
                out.push_str("<time>");
                i = end;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// The `[date][time]` at the start of `line`, if it has one.
fn stamp(line: &str) -> &str {
    let rest = without_stamp(line);
    &line[..line.len() - rest.len()]
}

fn key_of(line: &str) -> String {
    normalize_volatile(without_stamp(&crate::log_format::strip_ansi(line)))
}

/// How severe the `[LEVEL]` after the stamp is (lower is worse), or `None` when the line has none.
fn severity(line: &str) -> Option<(usize, &'static str)> {
    let plain = crate::log_format::strip_ansi(line);
    let rest = without_stamp(&plain);
    ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"]
        .iter()
        .enumerate()
        .find(|(_, name)| rest.starts_with(&format!("[{name}]")))
        .map(|(rank, name)| (rank, *name))
}

/// `lines` as a console shows them.
pub fn collapse(lines: &[String]) -> Vec<String> {
    let keys: Vec<String> = lines.iter().map(|line| key_of(line)).collect();
    let mut entries: Vec<Entry<'_>> = Vec::new();
    for (line, key) in lines.iter().zip(&keys) {
        match entries.last_mut() {
            Some(entry) if entry.key == key => {
                entry.last = line;
                entry.count += 1;
            }
            _ => entries.push(Entry { key, first: line, last: line, count: 1 }),
        }
    }
    merge(&entries).0.into_iter().flat_map(|(_, lines)| lines).collect()
}

/// One thing the pattern pass works with: a run of identical lines, or a pattern already found. A pattern counts as a
/// single line for the next pass, whatever it covers and however many times it repeated, so a pattern of patterns is
/// found the same way as a pattern of lines.
#[derive(Clone)]
enum Unit<'a> {
    Run { entry: usize, key: &'a str, first: &'a str, last: &'a str, count: usize },
    Pattern { cycle: Vec<Unit<'a>>, repetitions: usize, latest: &'a str, key: String, entries: usize },
}

impl<'a> Unit<'a> {
    fn key(&self) -> &str {
        match self {
            Unit::Run { key, .. } => key,
            Unit::Pattern { key, .. } => key,
        }
    }

    /// How many runs of the log this covers.
    fn entries(&self) -> usize {
        match self {
            Unit::Run { .. } => 1,
            // Counted when the pattern was found: its repetitions can differ in size (a nested pattern repeated twice in
            // one and seven times in the next), so the first cycle's size times the repetitions is not it.
            Unit::Pattern { entries, .. } => *entries,
        }
    }

    /// The last line this covers, which has the latest date and time.
    fn latest(&self) -> &'a str {
        match self {
            Unit::Run { last, .. } => last,
            Unit::Pattern { latest, .. } => latest,
        }
    }

    fn severity(&self) -> Option<(usize, &'static str)> {
        match self {
            Unit::Run { first, .. } => severity(first),
            Unit::Pattern { cycle, .. } => cycle.iter().filter_map(Unit::severity).min(),
        }
    }

    fn render(&self, out: &mut Vec<String>) {
        match self {
            Unit::Run { first, last, count, .. } => {
                out.push((*first).to_owned());
                if *count > 1 {
                    let more = count - 1;
                    out.push(format!("{last} (repeated {more} more time{})", if more == 1 { "" } else { "s" }));
                }
            }
            Unit::Pattern { cycle, repetitions, latest, .. } => {
                for unit in cycle {
                    unit.render(out);
                }
                let len = cycle.len();
                let level = self.severity().map_or("INFO", |(_, level)| level);
                out.push(format!(
                    "{}[{level}]: (the previous {len} log line{} repeated {repetitions} more time{})",
                    stamp(latest),
                    if len == 1 { "" } else { "s" },
                    if *repetitions == 1 { "" } else { "s" }
                ));
            }
        }
    }
}

/// One pass: wherever a cycle of up to WINDOW units repeats right after itself, the cycle and its repeats become one
/// pattern unit. At each place the cycle covering the most is taken (the shorter on a tie). Whether anything changed
/// is returned too.
fn pattern_pass<'a>(units: Vec<Unit<'a>>, window: usize) -> (Vec<Unit<'a>>, bool) {
    let mut out = Vec::new();
    let mut changed = false;
    let mut at = 0;
    while at < units.len() {
        let mut best: Option<(usize, usize)> = None;
        for len in 1..=window {
            if at + 2 * len > units.len() {
                break;
            }
            let same_as_next_cycle = |start: usize| (0..len).all(|i| units[start + i].key() == units[at + i].key());
            if !same_as_next_cycle(at + len) {
                continue;
            }
            let mut repetitions = 1;
            while at + (repetitions + 2) * len <= units.len() && same_as_next_cycle(at + (repetitions + 1) * len) {
                repetitions += 1;
            }
            if best.is_none_or(|(best_len, best_reps)| len * (repetitions + 1) > best_len * (best_reps + 1)) {
                best = Some((len, repetitions));
            }
        }
        match best {
            Some((len, repetitions)) => {
                let cycle: Vec<Unit<'a>> = units[at..at + len].to_vec();
                let key = format!(
                    "\u{1}[{}]",
                    cycle.iter().map(Unit::key).collect::<Vec<_>>().join("\u{2}")
                );
                let latest = units[at + (repetitions + 1) * len - 1].latest();
                let entries = units[at..at + (repetitions + 1) * len].iter().map(Unit::entries).sum();
                out.push(Unit::Pattern { cycle, repetitions, latest, key, entries });
                at += (repetitions + 1) * len;
                changed = true;
            }
            None => {
                out.push(units[at].clone());
                at += 1;
            }
        }
    }
    (out, changed)
}

/// The earliest entry that more lines could still change the merge from, looking at one level's `units`: a cycle that
/// is still repeating at the end of them might yet cover more (and then be preferred over what was chosen before it),
/// and a cycle could begin within a window or two of the end. Everything before it is settled.
fn unsettled_from(units: &[Unit<'_>]) -> usize {
    let n = units.len();
    let mut from = n.saturating_sub(2 * WINDOW);
    for len in 1..=WINDOW {
        if n <= len {
            break;
        }
        // The start of the run of units, ending at the end, that each equal the one a cycle later.
        let mut start = n - len;
        while start > 0 && units[start - 1].key() == units[start - 1 + len].key() {
            start -= 1;
        }
        from = from.min(start);
    }
    units[..from].iter().map(Unit::entries).sum()
}

/// The lines to show for `entries`, in groups: each group is what one top-level unit became, with the number of
/// entries covered up to and including it. Also the number of entries from which more lines could still change what
/// is shown (see [`unsettled_from`]), over every level of the merge.
fn merge(entries: &[Entry<'_>]) -> (Vec<(usize, Vec<String>)>, usize) {
    let mut units: Vec<Unit<'_>> = entries
        .iter()
        .enumerate()
        .map(|(entry, e)| Unit::Run { entry, key: e.key, first: e.first, last: e.last, count: e.count })
        .collect();
    // A pattern is a line to the next pass, so patterns of patterns are found too, until nothing more merges.
    let mut unsettled = entries.len();
    for window in [NESTING_WINDOW, WINDOW] {
        loop {
            unsettled = unsettled.min(unsettled_from(&units));
            let (next, changed) = pattern_pass(units, window);
            units = next;
            if !changed {
                break;
            }
        }
    }
    let mut covered = 0;
    let groups = units
        .iter()
        .map(|unit| {
            covered += unit.entries();
            let mut lines = Vec::new();
            unit.render(&mut lines);
            (covered, lines)
        })
        .collect();
    (groups, unsettled)
}

/// A run of consecutive identical lines that [`Merger`] holds on to between frames.
struct HeldEntry {
    key: String,
    first: String,
    last: String,
    count: usize,
}

/// Merges a console's lines a frame at a time (see the module notes).
#[derive(Default)]
pub struct Merger {
    /// How many lines of the log (counting the ones since trimmed off its front) have been taken in.
    consumed: u64,
    /// The runs not yet settled, oldest first.
    held: Vec<HeldEntry>,
    /// What to show: the settled lines, then the lines for the runs still held.
    shown: std::sync::Arc<Vec<String>>,
    /// How many of `shown` are settled.
    settled: usize,
}

impl Merger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes in the lines that are new since the last call and returns what to show. `lines` is the log as stored and
    /// `first_index` is the position, among every line the log has ever had, of `lines[0]`.
    pub fn update(&mut self, lines: &[String], first_index: u64) -> std::sync::Arc<Vec<String>> {
        let end = first_index + lines.len() as u64;
        if self.consumed < first_index || self.consumed > end {
            // The lines it had not yet taken in are gone, or the log was emptied: start again from what there is.
            *self = Self::default();
            self.consumed = first_index;
        }
        let fresh = &lines[(self.consumed - first_index) as usize..];
        if fresh.is_empty() && !(self.shown.is_empty() && self.held.is_empty()) {
            return std::sync::Arc::clone(&self.shown);
        }
        for line in fresh {
            let key = key_of(line);
            match self.held.last_mut() {
                Some(entry) if entry.key == key => {
                    entry.last = line.clone();
                    entry.count += 1;
                }
                _ => self.held.push(HeldEntry { key, first: line.clone(), last: line.clone(), count: 1 }),
            }
        }
        self.consumed = end;

        let entries: Vec<Entry<'_>> = self
            .held
            .iter()
            .map(|held| Entry { key: &held.key, first: &held.first, last: &held.last, count: held.count })
            .collect();
        let (groups, unsettled) = merge(&entries);
        let unsettled = unsettled.min(entries.len().saturating_sub(KEEP_OPEN));
        // Groups that end before anything more lines could change are settled and let go. (A cycle still repeating at
        // the end can come to cover more and displace what was chosen before it, so "far from the end" is not enough.)
        let settled_groups = groups.iter().take_while(|(end, _)| *end <= unsettled).count();
        let settled_entries = settled_groups.checked_sub(1).map_or(0, |last| groups[last].0);
        let shown = std::sync::Arc::make_mut(&mut self.shown);
        shown.truncate(self.settled);
        for (_, lines) in &groups[..settled_groups] {
            shown.extend(lines.iter().cloned());
        }
        self.settled = shown.len();
        for (_, lines) in &groups[settled_groups..] {
            shown.extend(lines.iter().cloned());
        }
        drop(entries);
        self.held.drain(..settled_entries.min(self.held.len()));
        std::sync::Arc::clone(&self.shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(messages: &[&str]) -> Vec<String> {
        messages
            .iter()
            .enumerate()
            .map(|(second, message)| format!("[2026-10-21][00:00:{second:02}][INFO][m]: {message}"))
            .collect()
    }

    /// The message of each shown line, without its stamp and level.
    fn shown(messages: &[&str]) -> Vec<String> {
        collapse(&lines(messages))
            .into_iter()
            .map(|line| line.split("]: ").last().unwrap_or("").to_owned())
            .collect()
    }

    #[test]
    fn consecutive_repeats_become_one_line_and_a_count() {
        assert_eq!(shown(&["a", "a", "a", "b"]), ["a", "a (repeated 2 more times)", "b"]);
        assert_eq!(shown(&["a", "a"]), ["a", "a (repeated 1 more time)"]);
    }

    #[test]
    fn a_repeating_pattern_is_shown_once_with_its_count() {
        assert_eq!(
            shown(&["a", "b", "c", "a", "b", "c", "a", "b", "c", "d"]),
            ["a", "b", "c", "(the previous 3 log lines repeated 2 more times)", "d"]
        );
    }

    #[test]
    fn a_collapsed_run_counts_as_one_line_whatever_its_length() {
        assert_eq!(
            shown(&["a", "a", "a", "b", "a", "b", "b", "b", "a", "b", "z"]),
            ["a", "a (repeated 2 more times)", "b", "(the previous 2 log lines repeated 2 more times)", "z"]
        );
    }

    #[test]
    fn a_pattern_that_ends_the_log_is_still_counted() {
        assert_eq!(shown(&["a", "b", "a", "b", "a", "b"]), ["a", "b", "(the previous 2 log lines repeated 2 more times)"]);
    }

    #[test]
    fn a_false_start_is_shown_in_order() {
        assert_eq!(shown(&["a", "b", "a", "c"]), ["a", "b", "a", "c"]);
    }

    #[test]
    fn a_partial_repetition_follows_the_count() {
        assert_eq!(
            shown(&["a", "b", "a", "b", "a", "x"]),
            ["a", "b", "(the previous 2 log lines repeated 1 more time)", "a", "x"]
        );
    }

    #[test]
    fn a_cycle_longer_than_the_window_is_not_merged() {
        let cycle = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m"];
        let mut messages = Vec::new();
        messages.extend(cycle);
        messages.extend(cycle);
        assert_eq!(shown(&messages).len(), 26);
    }

    #[test]
    fn merged_lines_are_dated_by_the_latest_line_they_cover() {
        let merged = collapse(&lines(&["a", "a", "a", "b"]));
        assert!(merged[0].starts_with("[2026-10-21][00:00:00]"));
        assert!(merged[1].starts_with("[2026-10-21][00:00:02]"), "{merged:?}");
        let merged = collapse(&lines(&["a", "b", "a", "b"]));
        assert!(merged[2].starts_with("[2026-10-21][00:00:03]"), "the count line: {merged:?}");
    }

    #[test]
    fn the_date_and_time_are_not_compared_but_the_level_is() {
        let same = vec![
            "[2026-10-21][00:00:01][INFO][m]: a".to_owned(),
            "[2026-10-22][09:59:59][INFO][m]: a".to_owned(),
        ];
        assert_eq!(collapse(&same).len(), 2);
        let different = vec![
            "[2026-10-21][00:00:01][INFO][m]: a".to_owned(),
            "[2026-10-21][00:00:01][WARN][m]: a".to_owned(),
        ];
        assert_eq!(collapse(&different).len(), 2);
        assert!(collapse(&different)[1].contains("[WARN]"));
    }

    #[test]
    fn lines_without_a_stamp_are_compared_whole() {
        let game = vec!["same".to_owned(), "same".to_owned(), "other".to_owned()];
        assert_eq!(collapse(&game), ["same", "same (repeated 1 more time)", "other"]);
    }

    #[test]
    fn nothing_to_merge_is_returned_as_it_is() {
        let plain = lines(&["a", "b", "c", "d"]);
        assert_eq!(collapse(&plain), plain);
    }

    /// What `Merger` shows after being given `messages` in the groups `frames` says (one call per group).
    fn merged_over_frames(messages: &[&str], frames: &[usize]) -> Vec<String> {
        let all = lines(messages);
        let mut merger = Merger::new();
        let mut taken = 0;
        let mut shown = Vec::new();
        for &frame in frames {
            taken += frame;
            shown = merger.update(&all[..taken], 0).to_vec();
        }
        shown
    }

    #[test]
    fn a_pattern_logged_over_many_frames_merges_like_one_logged_at_once() {
        let messages = ["a", "b", "c", "a", "b", "c", "a", "b", "c", "a", "b", "c", "d"];
        let at_once = collapse(&lines(&messages));
        // One line per frame, then in uneven bursts, then all at once.
        assert_eq!(merged_over_frames(&messages, &[1; 13]), at_once);
        assert_eq!(merged_over_frames(&messages, &[2, 5, 1, 3, 2]), at_once);
        assert_eq!(merged_over_frames(&messages, &[13]), at_once);
    }

    #[test]
    fn every_split_into_frames_shows_the_same_as_the_whole() {
        let messages = [
            "a", "a", "b", "c", "a", "b", "c", "c", "a", "b", "c", "x", "y", "x", "y", "x", "y", "z", "z", "z", "q",
        ];
        let at_once = collapse(&lines(&messages));
        for first in 1..messages.len() {
            for second in 1..messages.len() - first + 1 {
                let rest = messages.len() - first - second;
                let frames: Vec<usize> = [first, second, rest].into_iter().filter(|n| *n > 0).collect();
                assert_eq!(merged_over_frames(&messages, &frames), at_once, "frames {frames:?}");
            }
        }
    }

    #[test]
    fn old_runs_are_let_go_but_what_is_shown_stays_right() {
        let mut messages = Vec::new();
        for n in 0..1500 {
            messages.push(format!("line {n}"));
        }
        let owned: Vec<&str> = messages.iter().map(String::as_str).collect();
        let all = lines(&owned);
        let mut merger = Merger::new();
        for taken in (1..=all.len()).step_by(7) {
            merger.update(&all[..taken], 0);
        }
        let shown = merger.update(&all, 0);
        assert_eq!(shown.to_vec(), all);
        assert!(
            merger.held.len() <= KEEP_OPEN + 2 * WINDOW + 1,
            "settled runs are not kept: {} held of {}",
            merger.held.len(),
            all.len()
        );
    }

    #[test]
    fn lines_trimmed_off_the_front_do_not_disturb_the_merge() {
        let all = lines(&["a", "a", "b", "c", "d", "e", "f", "g"]);
        let mut merger = Merger::new();
        merger.update(&all[..4], 0);
        // The log dropped its first two lines and gained four: lines 2.. are now stored.
        let shown = merger.update(&all[2..], 2);
        assert_eq!(shown.last().map(|line| line.ends_with("g")), Some(true), "{shown:?}");
        assert!(shown.iter().any(|line| line.ends_with("]: f")));
    }

    #[test]
    fn a_log_that_was_emptied_starts_over() {
        let all = lines(&["a", "b"]);
        let mut merger = Merger::new();
        merger.update(&all, 0);
        let shown = merger.update(&[], 0);
        assert!(shown.is_empty(), "{shown:?}");
        let shown = merger.update(&all[..1], 0);
        assert_eq!(shown.len(), 1);
    }

    /// The messages of a stream like the engine's resize log: a blur line, then a three-line group that repeats, with
    /// a stray blur line between runs of it.
    fn engine_stream() -> Vec<&'static str> {
        let mut stream = vec!["B"];
        for repeats in [2, 2, 7] {
            for _ in 0..repeats {
                stream.extend(["P", "B", "S"]);
            }
            if repeats != 7 {
                stream.push("B");
            }
        }
        stream.extend(["P", "S", "P"]);
        stream
    }

    #[test]
    fn a_collapsed_group_counts_as_one_line_when_looking_for_the_next_pattern() {
        let merged: Vec<String> = collapse(&lines(&engine_stream()))
            .into_iter()
            .map(|line| line.split("]: ").last().unwrap_or("").to_owned())
            .collect();
        // B, then (P B S ×3 runs) twice more, all one pattern of "a B and a collapsed group" repeated: the first
        // cycle is shown, then the count. The stream's tail (P S P) follows.
        assert_eq!(
            merged,
            [
                "B",
                "P",
                "B",
                "S",
                "(the previous 3 log lines repeated 1 more time)",
                "(the previous 2 log lines repeated 2 more times)",
                "P",
                "S",
                "P"
            ]
        );
    }

    #[test]
    fn that_stream_merges_the_same_whenever_it_arrives() {
        let stream = engine_stream();
        let at_once = collapse(&lines(&stream));
        assert_eq!(merged_over_frames(&stream, &vec![1; stream.len()]), at_once);
        for chunk in [2, 3, 4, 5, 7, 11] {
            let mut frames = vec![chunk; stream.len() / chunk];
            if stream.len() % chunk > 0 {
                frames.push(stream.len() % chunk);
            }
            assert_eq!(merged_over_frames(&stream, &frames), at_once, "in chunks of {chunk}");
        }
    }

    #[test]
    fn patterns_of_patterns_nest() {
        // (a b a b c d e) x3: seven lines is longer than the window, but the inner "a b" pair repeats, and a pair
        // counts as one line, so what repeats is four "lines": the pair, c, d and e.
        let mut stream = Vec::new();
        for _ in 0..3 {
            stream.extend(["a", "b", "a", "b", "c", "d", "e"]);
        }
        assert_eq!(
            shown(&stream),
            [
                "a",
                "b",
                "(the previous 2 log lines repeated 1 more time)",
                "c",
                "d",
                "e",
                "(the previous 4 log lines repeated 2 more times)"
            ]
        );
    }

    #[test]
    fn the_longest_cover_wins_over_a_shorter_cycle_inside_it() {
        // (a b a b c) x3: the five-line cycle covers all fifteen; the "a b" pair inside it covers only four.
        let mut stream = Vec::new();
        for _ in 0..3 {
            stream.extend(["a", "b", "a", "b", "c"]);
        }
        assert_eq!(shown(&stream), ["a", "b", "a", "b", "c", "(the previous 5 log lines repeated 2 more times)"]);
    }

    #[test]
    fn the_cycle_that_covers_the_most_is_taken() {
        // a b a b a b a b : a cycle of 2 repeated 3 times covers all 8; a cycle of 4 repeated once covers the same,
        // and the shorter is taken.
        assert_eq!(shown(&["a", "b", "a", "b", "a", "b", "a", "b"]), ["a", "b", "(the previous 2 log lines repeated 3 more times)"]);
    }

    #[test]
    fn a_long_stream_of_nested_patterns_of_different_sizes_merges_the_same_in_any_frames() {
        // Many blocks like the engine's: a stray line then a three-line group repeated a different number of times
        // each block. The repetitions of the outer pattern are different sizes, which once made the merger release
        // more runs than it held.
        let mut stream = Vec::new();
        for block in 0..60 {
            stream.push("B");
            for _ in 0..(2 + block % 7) {
                stream.extend(["P", "B", "S"]);
            }
            stream.push(if block % 11 == 10 { "odd" } else { "B" });
        }
        let at_once = collapse(&lines(&stream));
        for chunk in [1, 2, 3, 5, 8, 13, 21, 55] {
            let mut frames = vec![chunk; stream.len() / chunk];
            if stream.len() % chunk > 0 {
                frames.push(stream.len() % chunk);
            }
            assert_eq!(merged_over_frames(&stream, &frames), at_once, "in chunks of {chunk}");
        }
    }

    #[test]
    fn random_noisy_streams_merge_the_same_in_any_frames_and_never_overrun() {
        // Streams in the style of the engine's resize log: a few kinds of line, in groups that repeat a varying number
        // of times with strays between, as one long log fed a few lines at a time.
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = move |bound: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % bound
        };
        for case in 0..12 {
            let mut stream: Vec<&'static str> = Vec::new();
            while stream.len() < 2400 {
                let group: &[&'static str] = match next(4) {
                    0 => &["P", "B", "S"],
                    1 => &["B", "P", "B", "S"],
                    2 => &["P", "S"],
                    _ => &["B"],
                };
                for _ in 0..1 + next(9) {
                    stream.extend(group);
                }
                if next(3) == 0 {
                    stream.push(["x", "y", "B"][next(3) as usize]);
                }
            }
            let at_once = collapse(&lines(&stream));
            for chunk in [1_usize, 3, 7, 40] {
                let all = lines(&stream);
                let mut merger = Merger::new();
                let mut shown = Vec::new();
                let mut taken = 0;
                while taken < all.len() {
                    taken = (taken + chunk).min(all.len());
                    shown = merger.update(&all[..taken], 0).to_vec();
                }
                assert_eq!(shown, at_once, "case {case}, chunks of {chunk}");
            }
        }
    }

    /// One frame of engine log, as it appears in the console: a shader compile with its imports, and the long-frame
    /// warning. Seven lines, whose only differences from one frame to the next are the durations.
    fn engine_frame(frame: usize, at: usize) -> Vec<String> {
        let stamped = |level: &str, offset: usize, body: String| {
            format!("[2026-11-04][07:49:{:02}][{level}][sturdy]: {body}", (at + offset) % 60)
        };
        vec![
            stamped("TRACE", 0, "Slang: 'Shaders/fullscreen_tonemap.slang' not found on disk, using the embedded copy.".into()),
            stamped("TRACE", 0, "Slang: import 'Shaders/sturdy_common.slang' not found on disk, using the embedded copy.".into()),
            stamped("TRACE", 0, "Slang: import 'Shaders/sturdy_tonemap_agx.slang' not found on disk, using the embedded copy.".into()),
            stamped("TRACE", 0, "Slang: import 'Shaders/sturdy_tonemap_hermite.slang' not found on disk, using the embedded copy.".into()),
            stamped("TRACE", 0, "Slang: import 'Shaders/sturdy_tonemap_psychov.slang' not found on disk, using the embedded copy.".into()),
            stamped("TRACE", 1, format!("Slang: compiled 'fullscreen_tonemap' (2 entry point(s)) in {}ms {}us {}ns", 190 + frame, 10 + frame * 7, 300 + frame)),
            stamped("WARN", 1, format!("Long frame detected: {}ms {}us {}ns", 280 + frame, 5 + frame, 100 + frame * 3)),
        ]
    }

    #[test]
    fn durations_and_addresses_do_not_make_lines_different() {
        let stream: Vec<String> = (0..6).flat_map(|frame| engine_frame(frame, frame * 2)).collect();
        let merged = collapse(&stream);
        assert_eq!(merged.len(), 8, "{merged:#?}");
        assert!(merged[7].ends_with("(the previous 7 log lines repeated 5 more times)"), "{}", merged[7]);
        // The level of the cycle is its worst line's.
        assert!(merged[7].contains("[WARN]"), "{}", merged[7]);
        // Figures that are not a duration stay part of the line.
        let files = vec!["[2026-11-04][07:49:00][INFO]: file 1 of 10".to_owned(), "[2026-11-04][07:49:01][INFO]: file 2 of 10".to_owned()];
        assert_eq!(collapse(&files).len(), 2);
        let addresses = vec![
            "[2026-11-04][07:49:00][INFO]: window wrapper=0x292450e5440".to_owned(),
            "[2026-11-04][07:49:01][INFO]: window wrapper=0x7fb1fc061690".to_owned(),
        ];
        // The same line twice with different addresses: the line and its "repeated 1 more time", not two lines.
        assert!(collapse(&addresses)[1].ends_with("(repeated 1 more time)"), "{:?}", collapse(&addresses));
    }

    #[test]
    fn a_repeating_pattern_with_changing_figures_merges_the_same_in_any_frames() {
        let stream: Vec<String> = (0..40).flat_map(|frame| engine_frame(frame, frame)).collect();
        let at_once = collapse(&stream);
        assert_eq!(at_once.len(), 8, "{at_once:#?}");
        for chunk in [1_usize, 2, 3, 5, 7, 11, 50, 280] {
            let mut merger = Merger::new();
            let mut shown = Vec::new();
            let mut taken = 0;
            while taken < stream.len() {
                taken = (taken + chunk).min(stream.len());
                shown = merger.update(&stream[..taken], 0).to_vec();
            }
            assert_eq!(shown, at_once, "chunks of {chunk}");
        }
    }

    #[test]
    fn repeats_that_arrive_later_extend_what_was_already_counted() {
        // A pattern repeated, then later (other frames, with an unrelated line in between) repeated again: each stretch is
        // counted, and a pattern of those stretches is counted in turn.
        let mut stream: Vec<String> = Vec::new();
        let mut merger = Merger::new();
        let mut shown = Vec::new();
        for round in 0..4 {
            for frame in 0..5 {
                stream.extend(engine_frame(frame, round * 10 + frame));
                shown = merger.update(&stream, 0).to_vec();
            }
            stream.push(format!("[2026-11-04][07:50:{round:02}][INFO][m]: separator"));
            shown = merger.update(&stream, 0).to_vec();
        }
        assert_eq!(shown, collapse(&stream));
        // Four identical rounds are one pattern of "seven lines x5, then a separator" repeated 3 more times, however many
        // frames it took to log.
        let last = shown.last().expect("something is shown");
        assert!(last.ends_with("(the previous 2 log lines repeated 3 more times)"), "{shown:#?}");
    }

    #[test]
    fn a_games_own_time_stamp_is_not_compared() {
        let game = vec![
            "[13:45:01] [Render thread/INFO]: Reloading ResourceManager".to_owned(),
            "[13:45:02] [Render thread/INFO]: Reloading ResourceManager".to_owned(),
            "[13:45:09] [Render thread/WARN]: Reloading ResourceManager".to_owned(),
        ];
        assert_eq!(
            collapse(&game),
            [
                "[13:45:01] [Render thread/INFO]: Reloading ResourceManager",
                "[13:45:02] [Render thread/INFO]: Reloading ResourceManager (repeated 1 more time)",
                "[13:45:09] [Render thread/WARN]: Reloading ResourceManager",
            ]
        );
        let dated = vec![
            "[06Nov2026 13:45:01.123] [main/INFO]: x".to_owned(),
            "[06Nov2026 13:45:05.999] [main/INFO]: x".to_owned(),
            "[2026-11-06T13:45:07Z] [main/INFO]: x".to_owned(),
        ];
        assert_eq!(collapse(&dated).len(), 2, "{:?}", collapse(&dated));
    }
}
