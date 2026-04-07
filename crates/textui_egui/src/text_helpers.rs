use egui::Ui;
use textui::TextUi;

use crate::LabelOptions;

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
    let normalized = normalize_inline_whitespace(text);
    truncate_prepared_single_line_text_with_ellipsis(
        text_ui,
        ui,
        normalized.as_str(),
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
    truncate_prepared_single_line_text_with_ellipsis(
        text_ui,
        ui,
        text.trim(),
        max_width,
        label_options,
    )
}

fn truncate_prepared_single_line_text_with_ellipsis(
    text_ui: &mut TextUi,
    ui: &Ui,
    text: &str,
    max_width: f32,
    label_options: &LabelOptions,
) -> String {
    if text.is_empty() {
        return String::new();
    }

    if max_width <= 0.0 {
        return "...".to_owned();
    }

    let full_width = text_ui
        .measure_text_size_at_scale(
            ui.ctx().pixels_per_point(),
            text,
            &label_options.to_text_label_options(),
        )
        .x;
    if full_width <= max_width {
        return text.to_owned();
    }

    const ELLIPSIS: &str = "...";
    let ellipsis_width = text_ui
        .measure_text_size_at_scale(
            ui.ctx().pixels_per_point(),
            ELLIPSIS,
            &label_options.to_text_label_options(),
        )
        .x;
    if ellipsis_width > max_width {
        return String::new();
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
        let mut s = String::with_capacity(n + ELLIPSIS.len());
        s.extend(chars[..n].iter());
        s.push_str(ELLIPSIS);
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
        return ELLIPSIS.to_owned();
    }

    let mut out = String::with_capacity(best + ELLIPSIS.len());
    out.extend(chars[..best].iter());
    out.push_str(ELLIPSIS);
    out
}
