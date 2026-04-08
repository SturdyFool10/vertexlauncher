use egui::Ui;
use textui::TextUi;

use crate::LabelOptions;

/// Result of a text truncation operation, providing both the display string
/// (with an ellipsis appended) and the raw, untruncated original text.
#[derive(Clone, Debug)]
pub struct TruncatedText {
    /// The text as it should be rendered, with an ellipsis appended when
    /// the original text was too wide.
    pub display: String,
    /// The original, unmodified text before truncation.
    pub raw: String,
    /// `true` when the text was shortened and an ellipsis was appended.
    pub was_truncated: bool,
}

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

    let full_width = text_ui
        .measure_text_size_at_scale(
            ui.ctx().pixels_per_point(),
            text,
            &label_options.to_text_label_options(),
        )
        .x;
    if full_width <= max_width {
        return TruncatedText {
            display: text.to_owned(),
            raw: raw_owned,
            was_truncated: false,
        };
    }

    let ellipsis_width = text_ui
        .measure_text_size_at_scale(
            ui.ctx().pixels_per_point(),
            ellipsis,
            &label_options.to_text_label_options(),
        )
        .x;
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
    let estimate = if full_width > f32::EPSILON {
        ((len as f32 * budget / full_width).floor() as usize).min(len)
    } else {
        0
    };

    let candidate = |n: usize| -> String {
        let mut s = String::with_capacity(n + ellipsis.len());
        s.extend(chars[..n].iter());
        s.push_str(ellipsis);
        s
    };

    let measure = |text_ui: &mut TextUi, text: &str, label_options: &LabelOptions| {
        text_ui
            .measure_text_size_at_scale(
                ui.ctx().pixels_per_point(),
                text,
                &label_options.to_text_label_options(),
            )
            .x
    };

    let est_width = if estimate > 0 {
        measure(text_ui, &candidate(estimate), label_options)
    } else {
        f32::MAX
    };

    let mut best = if est_width <= max_width { estimate } else { 0 };
    if est_width <= max_width {
        let mut n = estimate + 1;
        while n <= len {
            let w = measure(text_ui, &candidate(n), label_options);
            if w > max_width {
                break;
            }
            best = n;
            n += 1;
        }
    } else if estimate > 0 {
        let mut n = estimate.saturating_sub(1);
        loop {
            if n == 0 {
                break;
            }
            let w = measure(text_ui, &candidate(n), label_options);
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
