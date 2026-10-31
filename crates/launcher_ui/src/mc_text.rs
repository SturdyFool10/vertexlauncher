//! Minecraft's text formatting, as the Java game reads it, for anything in the launcher that shows game text: the
//! console, server MOTDs, names the game lets you format.
//!
//! Two inputs end up as the same [`Span`]s:
//!
//! * **Legacy section-sign codes** (`§a`, `§l`, ...). The rules are the game's own: the letter is case-insensitive;
//!   `§0`-`§f` pick a colour and, like the game's chat, clear the bold/italic/underline/strikethrough/obfuscated set
//!   before it; `§k` `§l` `§m` `§n` `§o` add obfuscated, bold, strikethrough, underline and italic; `§r` goes back to the
//!   style the text started with; an unknown code after the sign is dropped together with the sign; a sign at the very end
//!   is dropped. Beyond the game, the BungeeCord/Spigot hex form `§x§R§R§G§G§B§B` (what servers' MOTD plugins write for
//!   `#RRGGBB`) is understood as a colour.
//! * **Chat components** (the JSON of a server's status `description`): a string, an array, or an object with `text`
//!   (or `translate`/`score`/`selector`/`keybind`), `color` (a name or `#RRGGBB`), the five formatting flags, and
//!   `extra` children that inherit the parent's style. Section-sign codes inside a component's text are applied on top
//!   of that component's style, and `§r` there returns to it.
//!
//! A colour of `None` means "whatever the surface's own text colour is". `§r` in a console line therefore lands on the
//! line's colour (its log level's, or the one its ANSI escape set), not on a fixed white.

use serde_json::Value;

/// How a run of text is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    /// `None` = the surface's default colour.
    pub color: Option<[u8; 3]>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub obfuscated: bool,
}

impl Style {
    /// Style with `color` and nothing else.
    pub fn with_color(color: Option<[u8; 3]>) -> Self {
        Self { color, ..Self::default() }
    }

    fn formatted(&self) -> bool {
        self.bold || self.italic || self.underline || self.strikethrough || self.obfuscated
    }
}

/// A run of text in one style.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

/// The game's sixteen chat colours, by the character after the section sign.
pub fn legacy_color(code: char) -> Option<[u8; 3]> {
    Some(match code.to_ascii_lowercase() {
        '0' => [0x00, 0x00, 0x00],
        '1' => [0x00, 0x00, 0xAA],
        '2' => [0x00, 0xAA, 0x00],
        '3' => [0x00, 0xAA, 0xAA],
        '4' => [0xAA, 0x00, 0x00],
        '5' => [0xAA, 0x00, 0xAA],
        '6' => [0xFF, 0xAA, 0x00],
        '7' => [0xAA, 0xAA, 0xAA],
        '8' => [0x55, 0x55, 0x55],
        '9' => [0x55, 0x55, 0xFF],
        'a' => [0x55, 0xFF, 0x55],
        'b' => [0x55, 0xFF, 0xFF],
        'c' => [0xFF, 0x55, 0x55],
        'd' => [0xFF, 0x55, 0xFF],
        'e' => [0xFF, 0xFF, 0x55],
        'f' => [0xFF, 0xFF, 0xFF],
        _ => return None,
    })
}

/// A chat component's colour name, or `#RRGGBB`.
fn named_color(name: &str) -> Option<[u8; 3]> {
    let code = match name {
        "black" => '0',
        "dark_blue" => '1',
        "dark_green" => '2',
        "dark_aqua" => '3',
        "dark_red" => '4',
        "dark_purple" => '5',
        "gold" => '6',
        "gray" => '7',
        "dark_gray" => '8',
        "blue" => '9',
        "green" => 'a',
        "aqua" => 'b',
        "red" => 'c',
        "light_purple" => 'd',
        "yellow" => 'e',
        "white" => 'f',
        _ => {
            let hex = name.strip_prefix('#')?;
            if hex.len() != 6 {
                return None;
            }
            let value = u32::from_str_radix(hex, 16).ok()?;
            return Some([(value >> 16) as u8, (value >> 8) as u8, value as u8]);
        }
    };
    legacy_color(code)
}

/// The style after the format code `code` is applied on top of `style`; `base` is what `§r` returns to. `None` for a
/// code the game does not know.
fn apply_code(style: Style, base: Style, code: char) -> Option<Style> {
    let lower = code.to_ascii_lowercase();
    if let Some(color) = legacy_color(lower) {
        // A colour clears the formatting set before it.
        return Some(Style { color: Some(color), ..Style::default() });
    }
    let mut next = style;
    match lower {
        'k' => next.obfuscated = true,
        'l' => next.bold = true,
        'm' => next.strikethrough = true,
        'n' => next.underline = true,
        'o' => next.italic = true,
        'r' => next = base,
        _ => return None,
    }
    Some(next)
}

/// Splits `text` at its section-sign codes. `base` is the style the text starts in and `§r` returns to. Adjacent runs of
/// one style are joined, and empty runs are left out.
pub fn parse_legacy(text: &str, base: Style) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let mut style = base;
    let mut run = String::new();
    let flush = |run: &mut String, style: Style, spans: &mut Vec<Span>| {
        if run.is_empty() {
            return;
        }
        match spans.last_mut() {
            Some(last) if last.style == style => last.text.push_str(run),
            _ => spans.push(Span { text: std::mem::take(run), style }),
        }
        run.clear();
    };
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '§' {
            run.push(ch);
            continue;
        }
        // A sign with nothing after it is dropped.
        let Some(code) = chars.next() else { break };
        flush(&mut run, style, &mut spans);
        if code.eq_ignore_ascii_case(&'x') {
            // `§x§R§R§G§G§B§B`: six more sign+digit pairs make a hex colour; anything shorter is just an unknown code.
            let mut ahead = chars.clone();
            let mut digits = String::new();
            for _ in 0..6 {
                match (ahead.next(), ahead.next()) {
                    (Some('§'), Some(digit)) if digit.is_ascii_hexdigit() => digits.push(digit),
                    _ => break,
                }
            }
            if digits.len() == 6 {
                chars = ahead;
                let value = u32::from_str_radix(&digits, 16).unwrap_or(0);
                style = Style { color: Some([(value >> 16) as u8, (value >> 8) as u8, value as u8]), ..Style::default() };
            }
            continue;
        }
        if let Some(next) = apply_code(style, base, code) {
            style = next;
        }
        // An unknown code is dropped with its sign.
    }
    flush(&mut run, style, &mut spans);
    spans
}

/// `text` with its section-sign codes removed and nothing else changed.
pub fn strip_codes(text: &str) -> String {
    if !text.contains('§') {
        return text.to_owned();
    }
    plain(&parse_legacy(text, Style::default()))
}

/// The text of `spans`, without styling.
pub fn plain(spans: &[Span]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
}

/// `spans` cut into lines at their newlines (a MOTD is two). Empty trailing lines are kept: they are part of the text.
pub fn split_lines(spans: &[Span]) -> Vec<Vec<Span>> {
    let mut lines: Vec<Vec<Span>> = vec![Vec::new()];
    for span in spans {
        let mut parts = span.text.split('\n');
        if let Some(first) = parts.next()
            && !first.is_empty()
        {
            lines.last_mut().expect("lines is never empty").push(Span { text: first.to_owned(), style: span.style });
        }
        for part in parts {
            lines.push(Vec::new());
            if !part.is_empty() {
                lines.last_mut().expect("just pushed").push(Span { text: part.to_owned(), style: span.style });
            }
        }
    }
    lines
}

/// A status response's `description` (or any chat component) as styled runs.
pub fn from_component(value: &Value) -> Vec<Span> {
    let mut spans = Vec::new();
    push_component(value, Style::default(), &mut spans);
    spans
}

fn flag(object: &serde_json::Map<String, Value>, key: &str, inherited: bool) -> bool {
    match object.get(key) {
        Some(Value::Bool(value)) => *value,
        // Older servers write the flags as the strings "true" / "false".
        Some(Value::String(value)) => value.eq_ignore_ascii_case("true"),
        _ => inherited,
    }
}

fn push_component(value: &Value, inherited: Style, out: &mut Vec<Span>) {
    match value {
        Value::String(text) => out.extend(parse_legacy(text, inherited)),
        Value::Array(parts) => {
            // The first element's style is the style of the rest, the way the game treats a component list.
            let mut parts = parts.iter();
            let Some(first) = parts.next() else { return };
            push_component(first, inherited, out);
            let style = component_style(first, inherited);
            for part in parts {
                push_component(part, style, out);
            }
        }
        Value::Object(object) => {
            let style = component_style(value, inherited);
            if let Some(text) = component_text(object) {
                out.extend(parse_legacy(&text, style));
            }
            if let Some(Value::Array(children)) = object.get("extra") {
                for child in children {
                    push_component(child, style, out);
                }
            }
        }
        Value::Number(number) => out.extend(parse_legacy(&number.to_string(), inherited)),
        Value::Bool(value) => out.extend(parse_legacy(&value.to_string(), inherited)),
        Value::Null => {}
    }
}

/// The style a component sets on top of `inherited`.
fn component_style(value: &Value, inherited: Style) -> Style {
    let Value::Object(object) = value else { return inherited };
    let color = match object.get("color").and_then(Value::as_str) {
        Some("reset") => None,
        Some(name) => named_color(name).or(inherited.color),
        None => inherited.color,
    };
    Style {
        color,
        bold: flag(object, "bold", inherited.bold),
        italic: flag(object, "italic", inherited.italic),
        underline: flag(object, "underlined", inherited.underline),
        strikethrough: flag(object, "strikethrough", inherited.strikethrough),
        obfuscated: flag(object, "obfuscated", inherited.obfuscated),
    }
}

/// What a component says in words. The launcher has no language files, so a translatable component shows its
/// `fallback` when it has one, and its key otherwise; a score shows its value, a selector its pattern, a keybind its key.
fn component_text(object: &serde_json::Map<String, Value>) -> Option<String> {
    if let Some(text) = object.get("text") {
        return match text {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(value) => Some(value.to_string()),
            _ => None,
        };
    }
    if let Some(key) = object.get("translate").and_then(Value::as_str) {
        return Some(object.get("fallback").and_then(Value::as_str).unwrap_or(key).to_owned());
    }
    if let Some(score) = object.get("score").and_then(Value::as_object) {
        return score.get("value").and_then(Value::as_str).map(str::to_owned);
    }
    if let Some(selector) = object.get("selector").and_then(Value::as_str) {
        return Some(selector.to_owned());
    }
    object.get("keybind").and_then(Value::as_str).map(str::to_owned)
}

/// Whether any run of `spans` needs more than a colour to draw.
pub fn has_formatting(spans: &[Span]) -> bool {
    spans.iter().any(|span| span.style.formatted())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn texts(spans: &[Span]) -> Vec<&str> {
        spans.iter().map(|span| span.text.as_str()).collect()
    }

    #[test]
    fn colours_and_formats_apply_in_order() {
        let spans = parse_legacy("§aGreen §lbold§r plain", Style::default());
        assert_eq!(texts(&spans), ["Green ", "bold", " plain"]);
        assert_eq!(spans[0].style.color, Some([0x55, 0xFF, 0x55]));
        assert!(spans[1].style.bold && spans[1].style.color == Some([0x55, 0xFF, 0x55]));
        assert_eq!(spans[2].style, Style::default());
    }

    #[test]
    fn a_colour_clears_the_formatting_before_it() {
        let spans = parse_legacy("§l§nA§cB", Style::default());
        assert!(spans[0].style.bold && spans[0].style.underline);
        assert_eq!(spans[1].style, Style { color: Some([0xFF, 0x55, 0x55]), ..Style::default() });
    }

    #[test]
    fn codes_are_case_insensitive_and_every_format_is_known() {
        let spans = parse_legacy("§KA§LB§MC§ND§OE", Style::default());
        let last = spans.last().unwrap().style;
        assert!(last.obfuscated && last.bold && last.strikethrough && last.underline && last.italic);
    }

    #[test]
    fn reset_returns_to_the_base_style_not_to_white() {
        let base = Style::with_color(Some([1, 2, 3]));
        let spans = parse_legacy("§cred§rback", base);
        assert_eq!(spans[0].style.color, Some([0xFF, 0x55, 0x55]));
        assert_eq!(spans[1].style, base);
    }

    #[test]
    fn unknown_and_trailing_codes_are_dropped() {
        assert_eq!(plain(&parse_legacy("a§zb§", Style::default())), "ab");
        assert_eq!(strip_codes("§6Gold §lBold"), "Gold Bold");
        assert_eq!(strip_codes("no codes"), "no codes");
    }

    #[test]
    fn bungeecord_hex_colours_are_read() {
        let spans = parse_legacy("§x§f§f§8§0§0§0hot", Style::default());
        assert_eq!(spans[0].style.color, Some([0xFF, 0x80, 0x00]));
        assert_eq!(plain(&spans), "hot");
        // A short one is just an unknown code: the `x` goes, and what follows is read as ordinary codes.
        assert_eq!(plain(&parse_legacy("§x§1§2x", Style::default())), "x");
    }

    #[test]
    fn components_inherit_style_into_extra() {
        let value = json!({"text": "A", "color": "gold", "bold": true, "extra": [
            {"text": "B", "color": "#336699"},
            {"text": "C", "bold": false}
        ]});
        let spans = from_component(&value);
        assert_eq!(texts(&spans), ["A", "B", "C"]);
        assert_eq!(spans[0].style.color, Some([0xFF, 0xAA, 0x00]));
        assert_eq!(spans[1].style.color, Some([0x33, 0x66, 0x99]));
        assert!(spans[1].style.bold);
        assert!(!spans[2].style.bold && spans[2].style.color == Some([0xFF, 0xAA, 0x00]));
    }

    #[test]
    fn section_signs_inside_a_component_reset_to_that_component() {
        let value = json!({"text": "§lA§rB", "color": "red"});
        let spans = from_component(&value);
        let red = Some([0xFF, 0x55, 0x55]);
        assert!(spans[0].style.bold && spans[0].style.color == red);
        assert!(!spans[1].style.bold && spans[1].style.color == red);
    }

    #[test]
    fn plain_strings_and_arrays_are_components_too() {
        assert_eq!(plain(&from_component(&json!("§aHi"))), "Hi");
        assert_eq!(plain(&from_component(&json!(["a", {"text": "b"}, "c"]))), "abc");
        assert_eq!(plain(&from_component(&json!({"translate": "x.y", "fallback": "Fallback"}))), "Fallback");
        assert_eq!(plain(&from_component(&json!({"translate": "x.y"}))), "x.y");
    }

    #[test]
    fn a_motd_splits_into_its_two_lines() {
        let spans = parse_legacy("§aLine one\n§bLine two", Style::default());
        let lines = split_lines(&spans);
        assert_eq!(lines.len(), 2);
        assert_eq!(plain(&lines[0]), "Line one");
        assert_eq!(lines[1][0].style.color, Some([0x55, 0xFF, 0xFF]));
    }
}
