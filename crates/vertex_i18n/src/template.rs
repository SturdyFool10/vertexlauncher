//! Message templates: text with `{name}` placeholders. `{{` and `}}` write a literal brace. Translators move and repeat
//! placeholders freely, because every one is named, never positional.
//!
//! A placeholder may carry a format after a colon, which reads the value as a number and writes it the way the language
//! writes numbers: `{size:.1}` is one decimal place ("1.5" in English, "1,5" in German) and `{count:n}` groups digits
//! ("12,345"). A value that is not a number is written as it is.

use crate::number::NumberFormat;

/// One piece of a parsed template.
#[derive(Debug, PartialEq, Eq)]
enum Piece<'a> {
    Text(String),
    Placeholder(&'a str),
}

fn parse(template: &str) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut rest = template;
    while let Some(index) = rest.find(['{', '}']) {
        text.push_str(&rest[..index]);
        let after = &rest[index..];
        if let Some(stripped) = after.strip_prefix("{{") {
            text.push('{');
            rest = stripped;
        } else if let Some(stripped) = after.strip_prefix("}}") {
            text.push('}');
            rest = stripped;
        } else if after.starts_with('{')
            && let Some(close) = after.find('}')
            && !after[1..close].contains('{')
        {
            if !text.is_empty() {
                pieces.push(Piece::Text(std::mem::take(&mut text)));
            }
            pieces.push(Piece::Placeholder(after[1..close].trim()));
            rest = &after[close + 1..];
        } else {
            // A stray brace: keep it as it is rather than losing the rest of the message.
            text.push_str(&after[..1]);
            rest = &after[1..];
        }
    }
    text.push_str(rest);
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    pieces
}

/// A placeholder's text split into its name and optional format.
fn split_format(placeholder: &str) -> (&str, Option<&str>) {
    match placeholder.split_once(':') {
        Some((name, format)) => (name.trim(), Some(format.trim())),
        None => (placeholder, None),
    }
}

fn format_value(value: &str, format: &str, numbers: &NumberFormat) -> String {
    if let Some(places) = format.strip_prefix('.').and_then(|places| places.parse::<usize>().ok())
        && let Ok(number) = value.trim().parse::<f64>()
    {
        return numbers.decimal(number, places);
    }
    if format == "n"
        && let Ok(number) = value.trim().parse::<u64>()
    {
        return numbers.integer(number);
    }
    value.to_owned()
}

/// `template` with each `{name}` replaced by its value from `args`, written with the English number format.
pub fn render(template: &str, args: &[(&str, String)]) -> String {
    render_with(template, args, &NumberFormat::default())
}

/// [`render`] with a language's number format for the `{name:.1}` and `{name:n}` formats. A placeholder with no value is
/// left as written, so a mistake in a translation shows up on screen instead of silently dropping information.
pub fn render_with(template: &str, args: &[(&str, String)], numbers: &NumberFormat) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    for piece in parse(template) {
        match piece {
            Piece::Text(text) => out.push_str(&text),
            Piece::Placeholder(placeholder) => {
                let (name, format) = split_format(placeholder);
                match args.iter().find(|(known, _)| *known == name) {
                    Some((_, value)) => match format {
                        Some(format) => out.push_str(&format_value(value, format, numbers)),
                        None => out.push_str(value),
                    },
                    None => {
                        out.push('{');
                        out.push_str(placeholder);
                        out.push('}');
                    }
                }
            }
        }
    }
    out
}

/// The placeholder names a template uses, in order, without repeats.
pub fn placeholders(template: &str) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    for piece in parse(template) {
        if let Piece::Placeholder(placeholder) = piece
            && let name = split_format(placeholder).0
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        pairs.iter().map(|(name, value)| (*name, (*value).to_owned())).collect()
    }

    #[test]
    fn placeholders_are_filled_by_name_in_any_order() {
        let values = args(&[("name", "Ada"), ("count", "3")]);
        assert_eq!(render("{name} has {count} mods", &values), "Ada has 3 mods");
        assert_eq!(render("{count} mods belong to {name}", &values), "3 mods belong to Ada");
        assert_eq!(render("{name}, {name}!", &values), "Ada, Ada!");
    }

    #[test]
    fn missing_values_and_stray_braces_stay_visible() {
        assert_eq!(render("Hello {who}", &[]), "Hello {who}");
        assert_eq!(render("a } b { c", &[]), "a } b { c");
        assert_eq!(render("{{literal}} {x}", &args(&[("x", "1")])), "{literal} 1");
    }

    #[test]
    fn number_formats_follow_the_given_language() {
        let german = crate::number::defaults_for_language("de");
        let values = args(&[("size", "1.5"), ("count", "12345"), ("name", "x")]);
        assert_eq!(render("{size:.1} MiB", &values), "1.5 MiB");
        assert_eq!(render_with("{size:.1} MiB, {count:n} files", &values, &german), "1,5 MiB, 12.345 files");
        assert_eq!(render_with("{name:.1} {count}", &values, &german), "x 12345");
        assert_eq!(placeholders("{size:.1} and {size:n} and {count}"), vec!["size", "count"]);
    }

    #[test]
    fn placeholder_names_are_listed_once() {
        assert_eq!(placeholders("{a} then {b} then {a} {{c}}"), vec!["a", "b"]);
        assert!(placeholders("no placeholders").is_empty());
    }
}
