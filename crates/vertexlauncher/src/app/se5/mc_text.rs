//! Draws Minecraft-styled text (`launcher_ui::mc_text` runs): the console's lines and server MOTDs.
//!
//! What the game's text can do, and what is drawn:
//! * colour: yes; a run with no colour of its own takes the surface's text colour (which is where `§r` lands);
//! * bold and italic: the font's own face when it has one (a variable font's weight / slant axes, or an installed
//!   family's bold / italic face; see `fonts::register_styled_faces`), decided per font when the fonts are set up. What
//!   the font cannot make is dropped without fuss: bold falls back to the game's own trick (the run drawn a second time
//!   one pixel to the right), italic to upright. The bundled Maple Mono is a static regular face, so with it italic is
//!   upright and bold is the double-draw;
//! * underline and strikethrough: a one pixel bar along the run's baseline or middle, in the run's colour;
//! * obfuscated (`§k`): as the game does it, every letter except the space is swapped, on every frame, for a random
//!   letter *of the same width*. Which letters have that width depends on the letter and the font: in the game's own
//!   font an `i` can only become the few other narrow marks (and a letter whose width nothing else shares does not
//!   change at all), a wide `m` only a wide one. The same holds here, measured in whichever font the text is drawn in;
//!   in a monospace font every letter can become every other.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use launcher_ui::mc_text::{Span, Style};
use vertex_shell::sturdy::glam::{Vec2, Vec4};
use vertex_shell::sturdy::ui::{
    AttachPoint, ElementDesc, FloatTarget, Floating, Sizing, TextStyle, Ui,
};

use super::fonts;
use super::kit::{self, Kit};

/// The colour a run is drawn in: its own, or `default`.
pub fn color_of(style: &Style, default: Vec4) -> Vec4 {
    style.color.map_or(default, |[r, g, b]| {
        Vec4::new(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, default.w)
    })
}

/// Bumped when the fonts change, so the width tables measured in the old fonts are measured again.
static FONT_EPOCH: AtomicU32 = AtomicU32::new(0);

/// Call when the UI fonts are (re)registered.
pub fn fonts_changed() {
    FONT_EPOCH.fetch_add(1, Ordering::Relaxed);
}

/// Letters obfuscated text can turn into: printable ASCII, Latin-1, Greek, Cyrillic, kana and a block of common CJK,
/// which gives wide fonts something of their width to choose from. Which of them share a width is measured per font.
fn candidate_chars() -> impl Iterator<Item = char> {
    let ranges: [(u32, u32); 7] = [
        (0x21, 0x7E),
        (0xA1, 0xFF),
        (0x391, 0x3C9),
        (0x410, 0x44F),
        (0x3041, 0x3096),
        (0x30A1, 0x30FA),
        (0x4E00, 0x4E7F),
    ];
    ranges
        .into_iter()
        .flat_map(|(from, to)| from..=to)
        .filter_map(char::from_u32)
        // The soft hyphen draws as nothing.
        .filter(|ch| *ch != '\u{AD}')
}

/// The letters of one font and size grouped by their width, the way the game groups glyphs by advance.
struct WidthTable {
    /// Width (rounded up to whole pixels, as the game rounds an advance) -> letters of that width.
    buckets: HashMap<i32, Vec<char>>,
    /// Widths of letters outside the candidates, measured when first met.
    seen: HashMap<char, i32>,
}

impl WidthTable {
    fn build(ui: &Ui<'_>, style: &TextStyle) -> Self {
        let mut buckets: HashMap<i32, Vec<char>> = HashMap::new();
        let mut buffer = [0_u8; 4];
        for ch in candidate_chars() {
            let width = ui.measure_text(style, ch.encode_utf8(&mut buffer)).x;
            // A letter with no width draws nothing, so it is no replacement for anything.
            if width > 0.0 {
                buckets.entry(width.ceil() as i32).or_default().push(ch);
            }
        }
        Self { buckets, seen: HashMap::new() }
    }

    fn width_of(&mut self, ui: &Ui<'_>, style: &TextStyle, ch: char) -> i32 {
        *self.seen.entry(ch).or_insert_with(|| {
            let mut buffer = [0_u8; 4];
            ui.measure_text(style, ch.encode_utf8(&mut buffer)).x.ceil() as i32
        })
    }
}

thread_local! {
    static TABLES: RefCell<HashMap<(u32, u16, u16), WidthTable>> = RefCell::new(HashMap::new());
    static RNG: Cell<u64> = Cell::new(
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0x9E37_79B9_7F4A_7C15, |since| since.as_nanos() as u64) | 1
    );
}

/// A fresh random number on every call (xorshift64*), so every draw of obfuscated text picks again.
fn random() -> u64 {
    RNG.with(|state| {
        let mut x = state.get();
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        state.set(x);
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    })
}

/// `text` as the game draws obfuscated text: each letter other than a space becomes a random letter of the same width
/// in `style`'s font (the width rounded up to whole pixels). A letter whose width nothing else has stays as it is, and
/// one with only a few of its width cycles through just those: the replacement depends on what it replaces.
/// Called on every frame, and picks again each time.
fn scramble(ui: &Ui<'_>, style: &TextStyle, text: &str) -> String {
    let key = (FONT_EPOCH.load(Ordering::Relaxed), style.font_id, style.font_size);
    TABLES.with(|tables| {
        let mut tables = tables.borrow_mut();
        let table = tables.entry(key).or_insert_with(|| WidthTable::build(ui, style));
        text.chars()
            .map(|ch| {
                if ch.is_whitespace() {
                    return ch;
                }
                let width = table.width_of(ui, style, ch);
                match table.buckets.get(&width) {
                    Some(pool) if pool.len() > 1 => pool[(random() % pool.len() as u64) as usize],
                    // Alone at its width: nothing to swap it for.
                    _ => ch,
                }
            })
            .collect()
    })
}

/// The font a run is drawn with and whether the bold has to be faked. `font_id` is the role's regular font; the styled
/// faces the font really has (`kit.styled_faces`) are used, and an attribute no face carries is left out.
fn face_for(kit: &Kit, font_id: u16, style: &Style) -> (u16, bool) {
    let Some(slot) = fonts::styled_slot(font_id) else {
        return (font_id, style.bold);
    };
    let have = kit.styled_faces[slot];
    let pick = |variant: usize| fonts::styled_face_id(slot, variant);
    match (style.bold, style.italic) {
        (false, false) => (font_id, false),
        (true, false) => {
            if have[0] { (pick(0), false) } else { (font_id, true) }
        }
        (false, true) => {
            if have[1] { (pick(1), false) } else { (font_id, false) }
        }
        (true, true) => {
            if have[2] {
                (pick(2), false)
            } else if have[1] {
                // Italic is the face; the bold is faked on top of it.
                (pick(1), true)
            } else if have[0] {
                (pick(0), false)
            } else {
                (font_id, true)
            }
        }
    }
}

/// Whether `spans` change on their own (obfuscated runs), so the screen has to keep redrawing.
pub fn animates(spans: &[Span]) -> bool {
    spans.iter().any(|span| span.style.obfuscated)
}

/// `spans` cut so the line is no wider than `max_width` when drawn with `base`: the run that would run over is
/// shortened with an ellipsis and the rest dropped.
pub fn fit(ui: &Ui<'_>, kit: &Kit, spans: &[Span], base: &TextStyle, max_width: f32) -> Vec<Span> {
    let mut out = Vec::with_capacity(spans.len());
    let mut used = 0.0_f32;
    for span in spans {
        // Measured in the face the run is drawn with: an italic or bold face can be wider.
        let style = TextStyle { font_id: face_for(kit, base.font_id, &span.style).0, ..*base };
        let width = ui.measure_text(&style, &span.text).x;
        if used + width <= max_width {
            used += width;
            out.push(span.clone());
            continue;
        }
        let left = (max_width - used).max(0.0);
        out.push(Span { text: ui.ellipsize(&style, &span.text, left), style: span.style });
        break;
    }
    out
}

/// Draws `spans` one after another in the row that is open. `id` names the runs (each needs an id of its own for its
/// decorations); `base` gives the font, size and, for runs with no colour, the colour.
pub fn draw(ui: &mut Ui<'_>, kit: &Kit, id: &str, spans: &[Span], base: &TextStyle) {
    for (index, span) in spans.iter().enumerate() {
        let (font_id, fake_bold) = face_for(kit, base.font_id, &span.style);
        let style = TextStyle { color: color_of(&span.style, base.color), font_id, ..*base };
        let shown = if span.style.obfuscated {
            scramble(ui, &style, &span.text)
        } else {
            span.text.clone()
        };
        let s = &span.style;
        if !(fake_bold || s.underline || s.strikethrough) {
            kit::label(ui, &shown, &style);
            continue;
        }
        let run_id = format!("{id}::run::{index}");
        let _run = ui.element(&ElementDesc::new().id(run_id.clone()).width(Sizing::Fit).height(Sizing::Fit));
        kit::label(ui, &shown, &style);
        let float = |attach: AttachPoint, offset: Vec2| {
            Some(Floating {
                element_attach_point: attach,
                parent_attach_point: attach,
                offset,
                z_index: kit::in_modal_layer(1),
                capture_pointer: false,
                ..Floating::new(FloatTarget::Parent)
            })
        };
        if fake_bold {
            let _bold = ui.element(
                &ElementDesc::new()
                    .id(format!("{run_id}::bold"))
                    .width(Sizing::Fit)
                    .height(Sizing::Fit)
                    .floating(float(AttachPoint::LeftTop, Vec2::new(1.0, 0.0))),
            );
            kit::label(ui, &shown, &style);
        }
        let bar = |ui: &mut Ui<'_>, which: &str, attach: AttachPoint, y: f32| {
            drop(ui.element(
                &ElementDesc::new()
                    .id(format!("{run_id}::{which}"))
                    .width(Sizing::Percent(1.0))
                    .height(Sizing::Fixed(1.0))
                    .background(style.color)
                    .floating(float(attach, Vec2::new(0.0, y))),
            ));
        };
        if s.underline {
            let drop_below = (f32::from(base.font_size) * 0.12).round();
            bar(ui, "underline", AttachPoint::LeftBottom, -drop_below);
        }
        if s.strikethrough {
            bar(ui, "strike", AttachPoint::LeftCenter, 0.0);
        }
    }
    // The game picks new letters on every frame it draws, so while obfuscated text is on screen, the next frame is asked
    // for as soon as this one is done.
    if animates(spans) {
        vertex_shell::wake();
    }
}
