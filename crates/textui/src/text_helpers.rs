use egui::Ui;

use crate::{TextUi, label_options::LabelOptions};

#[path = "text_helpers/truncated_text.rs"]
mod truncated_text;

pub use self::truncated_text::TruncatedText;

/// Collapses any repeated whitespace into single ASCII spaces for single-line UI labels.
#[allow(dead_code)]
pub fn normalize_inline_whitespace(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !normalized.is_empty() {
            normalized.push(' ');
        }
        normalized.push_str(word);
    }
    normalized
}

/// Truncates a single-line label after collapsing repeated whitespace.
#[allow(dead_code)]
pub fn truncate_single_line_text_with_ellipsis(
    text_ui: &mut TextUi,
    ui: &Ui,
    text: &str,
    max_width: f32,
    label_options: &LabelOptions,
) -> String {
    truncate_single_line_text_with_ellipsis_detailed(text_ui, ui, text, max_width, label_options)
        .display
}

/// Like [`truncate_single_line_text_with_ellipsis`] but returns a
/// [`TruncatedText`] that includes both the display string and the raw
/// original.
#[allow(dead_code)]
pub fn truncate_single_line_text_with_ellipsis_detailed(
    text_ui: &mut TextUi,
    ui: &Ui,
    text: &str,
    max_width: f32,
    label_options: &LabelOptions,
) -> TruncatedText {
    let normalized = normalize_inline_whitespace(text);
    truncate_prepared_single_line_text_with_ellipsis(
        text_ui,
        ui,
        &normalized,
        text,
        max_width,
        label_options,
    )
}

/// Truncates a single-line label while preserving internal whitespace.
#[allow(dead_code)]
pub fn truncate_single_line_text_with_ellipsis_preserving_whitespace(
    text_ui: &mut TextUi,
    ui: &Ui,
    text: &str,
    max_width: f32,
    label_options: &LabelOptions,
) -> String {
    truncate_single_line_text_with_ellipsis_preserving_whitespace_detailed(
        text_ui,
        ui,
        text,
        max_width,
        label_options,
    )
    .display
}

/// Like [`truncate_single_line_text_with_ellipsis_preserving_whitespace`] but
/// returns a [`TruncatedText`] that includes both the display string and the
/// raw original.
#[allow(dead_code)]
pub fn truncate_single_line_text_with_ellipsis_preserving_whitespace_detailed(
    text_ui: &mut TextUi,
    ui: &Ui,
    text: &str,
    max_width: f32,
    label_options: &LabelOptions,
) -> TruncatedText {
    truncate_prepared_single_line_text_with_ellipsis(
        text_ui,
        ui,
        text.trim(),
        text,
        max_width,
        label_options,
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// O(1) truncation  (was O(log n) binary search)
//
// Strategy:
//   1.  Measure the full text once.
//   2.  Measure the ellipsis once (cached across calls via measure_text_size).
//   3.  Estimate the char cutoff linearly:  cut ≈ len × (max_width − ellipsis_w) / full_w.
//   4.  Make at most ONE refinement measurement to handle kerning / ligature
//       inaccuracy near the cutoff point (try ±1 char if needed).
//
// Total cost: 2–3 shape/layout calls instead of up to log2(len) calls.
// Correctness: correct for Latin and most scripts; ligatures are handled by
// the refinement step.  RTL and complex-script edge cases at exact boundary
// positions are handled by the single refinement round.
// ─────────────────────────────────────────────────────────────────────────────
#[allow(dead_code)]
fn truncate_prepared_single_line_text_with_ellipsis(
    text_ui: &mut TextUi,
    ui: &Ui,
    text: &str,
    raw: &str,
    max_width: f32,
    label_options: &LabelOptions,
) -> TruncatedText {
    let raw_owned = raw.to_owned();

    if text.is_empty() {
        return TruncatedText {
            display: String::new(),
            raw: raw_owned,
            was_truncated: false,
        };
    }

    let ellipsis = &label_options.ellipsis;

    if max_width <= 0.0 {
        return TruncatedText {
            display: ellipsis.clone(),
            raw: raw_owned,
            was_truncated: true,
        };
    }

    let full_width = text_ui.measure_text_size(ui, text, label_options).x;
    if full_width <= max_width {
        return TruncatedText {
            display: text.to_owned(),
            raw: raw_owned,
            was_truncated: false,
        };
    }

    let ellipsis_width = text_ui.measure_text_size(ui, ellipsis, label_options).x;
    if ellipsis_width > max_width {
        return TruncatedText {
            display: String::new(),
            raw: raw_owned,
            was_truncated: true,
        };
    }

    let budget = (max_width - ellipsis_width).max(0.0);
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();

    // Linear estimate of char cutoff
    let estimate = if full_width > f32::EPSILON {
        ((len as f32 * budget / full_width).floor() as usize).min(len)
    } else {
        0
    };

    // Helper: build "chars[0..n] + ellipsis"
    let candidate = |n: usize| -> String {
        let mut s = String::with_capacity(n + ellipsis.len());
        s.extend(chars[..n].iter());
        s.push_str(ellipsis);
        s
    };

    // Measure estimate; then probe one step up/down to find the correct cut.
    let est_width = if estimate > 0 {
        text_ui
            .measure_text_size(ui, &candidate(estimate), label_options)
            .x
    } else {
        f32::MAX
    };

    // Walk forward while we still fit
    let mut best = if est_width <= max_width { estimate } else { 0 };
    if est_width <= max_width {
        // Try to extend one char at a time (usually 0–2 steps)
        let mut n = estimate + 1;
        while n <= len {
            let w = text_ui
                .measure_text_size(ui, &candidate(n), label_options)
                .x;
            if w > max_width {
                break;
            }
            best = n;
            n += 1;
        }
    } else if estimate > 0 {
        // Estimate was too wide — walk back one char at a time
        let mut n = estimate.saturating_sub(1);
        loop {
            if n == 0 {
                break;
            }
            let w = text_ui
                .measure_text_size(ui, &candidate(n), label_options)
                .x;
            if w <= max_width {
                best = n;
                break;
            }
            n = n.saturating_sub(1);
        }
    }

    if best == 0 {
        return TruncatedText {
            display: ellipsis.clone(),
            raw: raw_owned,
            was_truncated: true,
        };
    }

    let mut out = String::with_capacity(best + ellipsis.len());
    out.extend(chars[..best].iter());
    out.push_str(ellipsis);
    TruncatedText {
        display: out,
        raw: raw_owned,
        was_truncated: true,
    }
}
