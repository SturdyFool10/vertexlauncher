use egui::Context;

pub fn sanitize_for_clipboard(text: &str) -> String {
    let needs_work = text.contains('\u{2026}')
        || text.contains('\u{201C}')
        || text.contains('\u{201D}')
        || text.contains('\u{2018}')
        || text.contains('\u{2019}');
    if !needs_work {
        return text.to_owned();
    }
    text.replace('\u{2026}', "...")
        .replace('\u{201C}', "\"")
        .replace('\u{201D}', "\"")
        .replace('\u{2018}', "'")
        .replace('\u{2019}', "'")
}

pub fn apply_smart_quotes(text: &str) -> String {
    if !text.contains('"') && !text.contains('\'') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut prev = ' ';
    for ch in text.chars() {
        match ch {
            '"' => {
                if is_opening_context(prev) {
                    out.push('\u{201C}');
                } else {
                    out.push('\u{201D}');
                }
            }
            '\'' => {
                if is_opening_context(prev) {
                    out.push('\u{2018}');
                } else {
                    out.push('\u{2019}');
                }
            }
            _ => out.push(ch),
        }
        prev = ch;
    }
    out
}

pub(super) fn copy_sanitized(ctx: &Context, text: String) {
    ctx.copy_text(sanitize_for_clipboard(&text));
}

#[inline]
fn is_opening_context(prev: char) -> bool {
    prev.is_whitespace() || matches!(prev, '(' | '[' | '{' | '\u{2014}' | '\u{2013}' | '\0')
}
