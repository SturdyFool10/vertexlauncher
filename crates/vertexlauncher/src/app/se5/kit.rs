//! The Vertex widget layer over SturdyEngine5: themed labels, buttons, toggles, sliders, dropdowns, text fields and
//! the settings-row layout every screen uses. Screens call only this, never raw engine styles, so the look stays in
//! one place (the SE5 counterpart of `ui_foundation` + `launcher_ui::ui::style`).
//!
//! Engine widgets keep per-widget animation state; [`Kit`] owns it, keyed by element id, so screens stay plain
//! functions over app state.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::icons::{self, Icon};
use super::images::{ImageCache, ImageSource, Picture};
use vertex_shell::palette::Palette;
use vertex_shell::sturdy::glam::{Vec2, Vec4};
use vertex_shell::sturdy::ui::{
    AlignX, AlignY, AttachPoint, Border, ChildAlignment, Clip, CornerRadius, Direction,
    ElementDesc, ElementScope, FloatTarget, Floating, Padding, Sizing, TextStyle, TextWrap, Ui,
};
use vertex_shell::sturdy::ui_kit::{
    self, AnimationKeepAlive, FocusNavigator, ModalStyle, NavInput, PopupStyle,
};
use vertex_shell::sturdy::widgets::{
    ButtonState, DropdownState, ScrollAreaState, ScrollbarStyle, SliderConfig, SliderInput,
    SliderKey, SliderState, TextEditState, ToggleState,
};

/// Layers of floating UI, lowest to highest. Modals are at 2000 (a few go slightly above), so dropdowns and menus sit
/// above them, and tooltips above those; otherwise anything opened from inside a dialog would draw behind it.
pub const Z_POPUP: i16 = 2500;
pub const Z_TOOLTIP: i16 = 3000;

pub const SPACE_XS: u16 = 4;
pub const SPACE_SM: u16 = 8;
pub const SPACE_MD: u16 = 12;
pub const SPACE_LG: u16 = 16;
pub const SPACE_XL: u16 = 24;
pub const CORNER: f32 = 14.0;
pub const CONTROL_HEIGHT: f32 = 34.0;
/// Dropdowns with more options than this get a search box.
const SEARCHABLE_DROPDOWN_THRESHOLD: usize = 12;

/// Seconds since the UI started: the clock for indeterminate progress bars.
/// Animation state of one [`Kit::collapsible`].
#[derive(Default)]
struct Collapse {
    open: bool,
    /// 0 = closed, 1 = open.
    progress: f32,
    /// The content's height when last laid out, which the animation is a fraction of.
    height: f32,
    last_frame: f64,
}

/// `text` reduced to what a number can contain: digits, at most one `.` when `decimals`, and a leading `-` when
/// `negative`. At most 12 characters.
fn filter_number_text(text: &str, decimals: bool, negative: bool) -> String {
    let mut out = String::new();
    let mut dot = false;
    for ch in text.chars() {
        let keep = match ch {
            '0'..='9' => true,
            '.' if decimals && !dot => {
                dot = true;
                true
            }
            '-' if negative && out.is_empty() => true,
            _ => false,
        };
        if keep && out.len() < 12 {
            out.push(ch);
        }
    }
    out
}

/// A formatted value split into its leading number and what follows ("100%" into "100" and "%").
fn split_number(text: &str) -> (String, String) {
    let end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(text.len());
    (text[..end].to_owned(), text[end..].to_owned())
}

pub fn uptime() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// Draws one text leaf after the streamer-mode name filter, so an account name can never reach the screen. Every
/// piece of text the SE5 UI draws goes through here.
pub fn label(ui: &mut Ui<'_>, text: &str, style: &TextStyle) {
    label_text(ui, text, style);
}

/// Word-wrapped text that can never be wider than `max_width`: wrapping only breaks between words, so a single word
/// (a long URL, a name with no spaces) wider than the box is shortened with an ellipsis instead of overflowing it.
pub fn wrapped_label(ui: &mut Ui<'_>, text: &str, style: &TextStyle, max_width: f32) {
    let style = TextStyle {
        wrap: TextWrap::Words,
        ..style.clone()
    };
    let text = launcher_ui::privacy::scrub(text);
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_ref();
    // Words and the whitespace (spaces, newlines, tabs) between them, the whitespace kept as it was.
    while !rest.is_empty() {
        let split = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (word, tail) = rest.split_at(split);
        // Only words long enough to possibly overflow are worth measuring.
        if word.chars().count() > 8 {
            out.push_str(&ui.ellipsize(&style, word, max_width));
        } else {
            out.push_str(word);
        }
        let gap = tail
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(tail.len());
        out.push_str(&tail[..gap]);
        rest = &tail[gap..];
    }
    ui.text(&out, &style);
}

/// One account name as it shows in streamer mode: the redaction label, on its account's colour when it has one.
fn account_chip(ui: &mut Ui<'_>, shown: &str, id: Option<&str>, style: &TextStyle) {
    let highlight = id.and_then(launcher_ui::account_colors::color_for);
    let Some(color) = highlight else {
        ui.text(shown, style);
        return;
    };
    let channel = |value: u8| f32::from(value) / 255.0;
    let background = Vec4::new(
        channel(color.r()),
        channel(color.g()),
        channel(color.b()),
        1.0,
    );
    let ink = launcher_ui::account_colors::contrasting_color(color);
    let _chip = ui.element(
        &ElementDesc::new()
            .width(Sizing::Fit)
            .height(Sizing::Fit)
            .padding(Padding::symmetric(3, 1))
            .background(background)
            .corner_radius(CornerRadius::all(4.0)),
    );
    ui.text(
        shown,
        &TextStyle {
            color: Vec4::new(channel(ink.r()), channel(ink.g()), channel(ink.b()), 1.0),
            ..style.clone()
        },
    );
}

/// Draws `pieces` (text and hidden accounts) as wrapped lines no wider than `width`: words are placed left to right and
/// a new line starts when the next one does not fit, so an account's coloured name flows with the text around it.
fn draw_flow(
    ui: &mut Ui<'_>,
    pieces: &[launcher_ui::privacy::ScrubPiece],
    style: &TextStyle,
    width: f32,
) {
    use launcher_ui::privacy::{REDACTED_ACCOUNT_LABEL, ScrubPiece};
    enum Token<'a> {
        Word(&'a str),
        Account(Option<&'a str>),
        Break,
    }
    let plain = TextStyle {
        wrap: TextWrap::None,
        ..style.clone()
    };
    // Words keep the space after them, so they measure and draw the way they read.
    let mut tokens = Vec::new();
    for piece in pieces {
        match piece {
            ScrubPiece::Account { id } => tokens.push(Token::Account(id.as_deref())),
            ScrubPiece::Text(text) => {
                for (index, line) in text.split('\n').enumerate() {
                    if index > 0 {
                        tokens.push(Token::Break);
                    }
                    let mut rest = line;
                    while !rest.is_empty() {
                        let word_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                        let gap_end = rest[word_end..]
                            .find(|c: char| !c.is_whitespace())
                            .map_or(rest.len(), |gap| word_end + gap);
                        tokens.push(Token::Word(&rest[..gap_end]));
                        rest = &rest[gap_end..];
                    }
                }
            }
        }
    }
    let width_of = |ui: &Ui<'_>, token: &Token<'_>| match token {
        Token::Word(word) => ui.measure_text(&plain, word).x,
        // The chip's padding is 3 px on each side.
        Token::Account(_) => ui.measure_text(&plain, REDACTED_ACCOUNT_LABEL).x + 6.0,
        Token::Break => 0.0,
    };
    let mut lines: Vec<Vec<&Token<'_>>> = vec![Vec::new()];
    let mut used = 0.0;
    for token in &tokens {
        if matches!(token, Token::Break) {
            lines.push(Vec::new());
            used = 0.0;
            continue;
        }
        let needed = width_of(ui, token);
        if used + needed > width && used > 0.0 {
            lines.push(Vec::new());
            used = 0.0;
        }
        used += needed;
        if let Some(line) = lines.last_mut() {
            line.push(token);
        }
    }
    let _column = ui.element(
        &ElementDesc::new()
            .width(Sizing::Fixed(width))
            .height(Sizing::Fit)
            .direction(Direction::TopToBottom),
    );
    for line in lines {
        let _row = ui.element(
            &ElementDesc::new()
                .width(Sizing::Fit)
                .height(Sizing::Fit)
                .direction(Direction::LeftToRight)
                .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
        );
        for token in line {
            match token {
                Token::Word(word) => ui.text(word, &plain),
                Token::Account(id) => account_chip(ui, REDACTED_ACCOUNT_LABEL, *id, &plain),
                Token::Break => {}
            }
        }
    }
}

fn label_text(ui: &mut Ui<'_>, text: &str, style: &TextStyle) {
    // In streamer mode an account name is drawn as the redaction label on that account's colour. Wrapping text with a
    // known width (a toast, a dialog) flows word by word around the name; unwrapped short text is one row.
    let known_width = TEXT_WIDTH.get().or(MODAL_WIDTH.get());
    if style.wrap != TextWrap::None
        && let Some(width) = known_width
        && let Some(pieces) = launcher_ui::privacy::scrub_pieces(text)
    {
        draw_flow(ui, &pieces, style, width);
        return;
    }
    if text.chars().count() <= 80
        && let Some(pieces) = launcher_ui::privacy::scrub_pieces(text)
    {
        let plain = TextStyle {
            wrap: TextWrap::None,
            ..style.clone()
        };
        let _row = ui.element(
            &ElementDesc::new()
                .width(Sizing::Fit)
                .height(Sizing::Fit)
                .direction(Direction::LeftToRight)
                .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
        );
        for piece in pieces {
            match piece {
                launcher_ui::privacy::ScrubPiece::Text(part) => ui.text(&part, &plain),
                launcher_ui::privacy::ScrubPiece::Account { id } => account_chip(
                    ui,
                    launcher_ui::privacy::REDACTED_ACCOUNT_LABEL,
                    id.as_deref(),
                    &plain,
                ),
            }
        }
        return;
    }
    let text = launcher_ui::privacy::scrub(text);
    match MODAL_WIDTH.get() {
        Some(width) if style.wrap != TextWrap::None => {
            ui.text(&break_long_words(ui, &text, style, width), style)
        }
        _ => ui.text(&text, style),
    }
}

/// The z-index for a floating piece of a widget (scrollbar thumb, focus ring) that normally sits just above the page.
/// Inside a dialog, which is itself a high layer, it has to sit above the dialog instead.
pub fn in_modal_layer(z: i16) -> i16 {
    if MODAL_WIDTH.get().is_some() {
        MODAL_LAYER + z
    } else {
        z
    }
}

/// Base layer of widget parts inside dialogs: above the dialogs (2000, a few at 2100) and below popups.
const MODAL_LAYER: i16 = 2200;

thread_local! {
    /// The inner width of the modal being built, if any. Text inside a dialog may not be wider than it.
    static MODAL_WIDTH: std::cell::Cell<Option<f32>> = const { std::cell::Cell::new(None) };
    /// The width text has where it is drawn, when something other than a dialog sets one (a toast).
    static TEXT_WIDTH: std::cell::Cell<Option<f32>> = const { std::cell::Cell::new(None) };
}

/// Runs `draw` with `width` as the width wrapping text has, so text that mixes in account names can wrap around them.
pub fn with_text_width<R>(width: f32, draw: impl FnOnce() -> R) -> R {
    let previous = TEXT_WIDTH.replace(Some(width));
    let result = draw();
    TEXT_WIDTH.set(previous);
    result
}

/// `text` with any run of non-space characters wider than `max_width` (a long path or URL, which word wrapping cannot
/// split) broken over several lines.
fn break_long_words(ui: &Ui<'_>, text: &str, style: &TextStyle, max_width: f32) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let split = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (word, tail) = rest.split_at(split);
        if word.chars().count() > 12 && ui.measure_text(style, word).x > max_width {
            let mut line = String::new();
            for ch in word.chars() {
                line.push(ch);
                if ui.measure_text(style, &line).x > max_width && line.chars().count() > 1 {
                    line.pop();
                    out.push_str(&line);
                    out.push('\n');
                    line.clear();
                    line.push(ch);
                }
            }
            out.push_str(&line);
        } else {
            out.push_str(word);
        }
        let gap = tail
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(tail.len());
        out.push_str(&tail[..gap]);
        rest = &tail[gap..];
    }
    out
}

/// A dialog. Like `ui_kit::modal`, but its content sits in a column exactly as wide as the panel's inside, so nothing
/// inside can stretch the dialog past its edges or push right-aligned buttons off them, and text in it breaks long
/// unspaced words to fit. Wider-than-window dialogs shrink to the window.
pub fn modal(
    ui: &mut Ui<'_>,
    modal_id: &str,
    style: &ModalStyle,
    dismissible: bool,
    content: impl FnOnce(&mut Ui<'_>),
) -> ui_kit::ModalResponse {
    let mut style = *style;
    let mut inner = None;
    if let Sizing::Fixed(width) = style.width {
        let width = width.min((ui.viewport_size().x - 32.0).max(240.0));
        style.width = Sizing::Fixed(width);
        inner = Some(width - f32::from(style.padding.left) - f32::from(style.padding.right));
    }
    ui_kit::modal(ui, modal_id, &style, dismissible, |ui| {
        let Some(inner) = inner else {
            return content(ui);
        };
        let previous = MODAL_WIDTH.replace(Some(inner));
        // A dialog taller than the window scrolls instead of running off it. The content height is last frame's.
        let body_id = format!("{modal_id}::body");
        let content_id = format!("{modal_id}::content");
        let room = (ui.viewport_size().y - 2.0 * f32::from(style.padding.top) - 48.0).max(160.0);
        let measured = ui
            .element_bounds(&content_id)
            .map_or(0.0, |(_, size)| size.y);
        let scrolls = measured > room;
        {
            let _body = ui.element(&if scrolls {
                ElementDesc::new()
                    .id(body_id)
                    .width(Sizing::Fixed(inner))
                    .height(Sizing::Fixed(room))
                    .direction(Direction::TopToBottom)
                    .clip(Clip::vertical())
            } else {
                ElementDesc::new()
                    .id(body_id)
                    .width(Sizing::Fixed(inner))
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
            });
            let _inner = ui.element(
                &ElementDesc::new()
                    .id(content_id)
                    .width(Sizing::Fixed(inner))
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
                    .child_gap(12),
            );
            content(ui);
        }
        MODAL_WIDTH.set(previous);
    })
}

/// The mouse cursor shown over an element; the later variant wins where two overlap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CursorKind {
    /// The pointing hand: it does something when clicked.
    Pointer,
    /// The text cursor: an editable text box.
    Text,
}

/// Text roles, sized relative to the user's UI font size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextRole {
    Title,
    Heading,
    Body,
    Muted,
    Small,
    OnAccent,
    Danger,
    Warning,
    /// Body text in the strong weight (names, values).
    Strong,
    /// Monospace text (values, paths, code).
    Code,
}

/// What [`Kit::scroll`] returns: the page's scroll area and the column inside it that holds the page. Dropping it closes
/// both (the column first).
pub struct ScrollScope {
    _content: ElementScope,
    _scroll: ElementScope,
}

/// A position in a log: a line, and a byte offset into that line's text (ANSI codes removed).
pub type LogPoint = (usize, usize);

/// A text selection in a log viewer, from one point to another, with its right-click menu.
#[derive(Default)]
pub struct LogSelection {
    /// Where the drag started and where it is now.
    pub anchor: Option<LogPoint>,
    pub head: Option<LogPoint>,
    pub dragging: bool,
    /// The selection moved while the button was down (a plain click clears it instead).
    pub moved: bool,
    pub menu: ui_kit::PopupState,
    /// The line the menu was opened on.
    pub menu_line: usize,
    /// Frames the view has stayed off the bottom while following.
    pub away_frames: u32,
    /// A jump to the newest line was asked for but the log element did not exist yet to receive it.
    pub pending_scroll: bool,
    /// Frames left in which the view is still catching up with a jump, so it is not mistaken for the user leaving.
    pub settle_frames: u8,
}

impl LogSelection {
    /// The selection as an ordered (start, end) pair; `None` while nothing is selected between the two points.
    pub fn range(&self) -> Option<(LogPoint, LogPoint)> {
        let (anchor, head) = (self.anchor?, self.head?);
        (anchor != head).then(|| (anchor.min(head), anchor.max(head)))
    }

    /// A selection exists, or one is being made.
    pub fn active(&self) -> bool {
        self.dragging || self.range().is_some()
    }

    pub fn clear(&mut self) {
        self.anchor = None;
        self.head = None;
        self.dragging = false;
        self.moved = false;
    }
}

/// Width a full-screen scroll page has for its content, from the window rather than from the scroll area (which
/// grows to hold its widest row, so it would keep a layout sized for a bigger window after the window shrinks).
pub fn content_width(ui: &Ui<'_>) -> f32 {
    // Less the window border on both sides.
    ui.viewport_size().x - super::chrome::SIDEBAR_WIDTH - 2.0 - 2.0 * f32::from(SPACE_XL) - 12.0
}

/// [`content_width`], but never wider than what the scroll area `scroll_id` actually measured last frame (minus its
/// padding), so chrome the window estimate does not know about (borders, a scrollbar) cannot push a row past the edge.
pub fn content_width_in(ui: &Ui<'_>, scroll_id: &str) -> f32 {
    let estimate = content_width(ui);
    match ui.element_bounds(scroll_id) {
        Some((_, size)) => estimate.min(size.x - 2.0 * f32::from(SPACE_XL) - 12.0),
        None => estimate,
    }
}

/// How many columns a masonry gallery of `width` pixels gets: three across wherever there is room, fewer on narrow
/// windows.
pub fn masonry_columns(width: f32) -> usize {
    if width < 520.0 {
        1
    } else if width < 760.0 {
        2
    } else {
        3
    }
}

/// Which look a button has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Neutral,
    Primary,
    Danger,
}

/// Reveal progress of one context menu.
#[derive(Clone, Copy, Debug, Default)]
struct MenuAnim {
    /// 0 = folded away, 1 = fully open.
    progress: f32,
    /// Where the panel was last drawn, so it can fold away in place after the menu closed.
    origin: Vec2,
    last_call: f64,
    was_open: bool,
}

/// One row of a [`Kit::context_menu`]: a label, an optional icon at the right, and optional danger styling.
#[derive(Clone, Debug)]
pub struct MenuEntry {
    pub label: String,
    pub icon: Option<Icon>,
    pub danger: bool,
    pub separator_before: bool,
}

impl MenuEntry {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            danger: false,
            separator_before: false,
        }
    }

    #[must_use]
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    #[must_use]
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    #[must_use]
    pub fn separated(mut self) -> Self {
        self.separator_before = true;
        self
    }
}

pub struct Kit {
    pub palette: Palette,
    /// Base UI font size in pixels (`Config::ui_font_size`).
    pub font_size: f32,
    pub typography: config::TypographySettings,
    pub keep_alive: AnimationKeepAlive,
    pub delta_seconds: f32,
    pub images: ImageCache,
    /// Icon render scale from the anti-aliasing setting.
    pub svg_supersample: u32,
    /// The graphics device generation the icon textures were made on; a switch invalidates them.
    pub svg_device_generation: u64,
    /// Which bold/italic faces the current font has, per styled role (see `fonts::StyledFaces`).
    pub styled_faces: super::fonts::StyledFaces,
    /// When the icons are made once more after a device switch; see `apply_fonts`.
    pub svg_rebuild_at: Option<Instant>,
    buttons: HashMap<String, ButtonState>,
    toggles: HashMap<String, ToggleState>,
    sliders: HashMap<String, SliderState>,
    dropdowns: HashMap<String, DropdownState>,
    texts: HashMap<String, TextEditState>,
    areas: HashMap<String, ScrollAreaState>,
    /// The scrollbar being dragged: the scroll element's id and where on the thumb it was grabbed.
    scroll_drag: Option<(String, f32)>,
    /// Set while `button_icon_right` draws, so `button_full` puts the icon after the label.
    icon_after: bool,
    levels: HashMap<String, (usize, usize, Vec<Option<launcher_ui::log_format::LogLevel>>)>,
    /// The selected lines of each log viewer, by viewer id.
    pub log_selections: HashMap<String, LogSelection>,
    /// The merge of each console's lines, by viewer id, carried from frame to frame.
    mergers: HashMap<String, launcher_ui::log_collapse::Merger>,
    tooltips: HashMap<String, f32>,
    /// Open/closed state and search text of each searchable dropdown.
    searches: HashMap<String, (ui_kit::PopupState, String)>,
    /// Open/close progress (0..1) and the last open state of every context menu, for its reveal and fold animation.
    menu_anim: HashMap<String, MenuAnim>,
    /// Open/close progress of every collapsible section, by id.
    collapsibles: HashMap<String, Collapse>,
    /// What the pointer shows over each interactive element declared this frame (see [`CursorKind`]).
    cursor_targets: Vec<(String, CursorKind)>,
    /// Number fields that were being edited last frame, to notice when editing ends.
    numbers_editing: std::collections::HashSet<String>,
    /// Keyboard and gamepad focus: widgets declare themselves focusable as they are drawn.
    pub focus: FocusNavigator,
    /// Gamepad steps for the focused slider this frame.
    slider_steps: i32,
    /// The clipped container being filled, so focus moves can scroll their target into view.
    scroll_container: Option<String>,
    /// A text field had focus last frame / has focus this frame (arrow keys then belong to its caret).
    text_focused_last: bool,
    text_focused_now: bool,
    /// Whether the UI is being driven by a controller, so focus rings show without the keyboard.
    pub controller_active: bool,
    /// The controller's context-menu button (X) was pressed this frame.
    context_pressed: bool,
    /// A popup took this frame's back request, so dialogs must not also treat it as theirs.
    back_consumed: bool,
}

impl Kit {
    pub fn new(palette: Palette, font_size: f32) -> Self {
        Self {
            palette,
            font_size,
            typography: config::TypographySettings::default(),
            keep_alive: AnimationKeepAlive::default(),
            delta_seconds: 0.016,
            images: ImageCache::default(),
            svg_supersample: 2,
            svg_device_generation: 0,
            styled_faces: Default::default(),
            svg_rebuild_at: None,
            buttons: HashMap::new(),
            toggles: HashMap::new(),
            sliders: HashMap::new(),
            dropdowns: HashMap::new(),
            texts: HashMap::new(),
            areas: HashMap::new(),
            scroll_drag: None,
            menu_anim: HashMap::new(),
            collapsibles: HashMap::new(),
            cursor_targets: Vec::new(),
            numbers_editing: std::collections::HashSet::new(),
            icon_after: false,
            levels: HashMap::new(),
            log_selections: HashMap::new(),
            mergers: HashMap::new(),
            tooltips: HashMap::new(),
            searches: HashMap::new(),
            focus: FocusNavigator::default(),
            slider_steps: 0,
            scroll_container: None,
            text_focused_last: false,
            text_focused_now: false,
            controller_active: false,
            context_pressed: false,
            back_consumed: false,
        }
    }

    /// Starts the frame's input: resolves keyboard and gamepad navigation against last frame's focusables.
    /// `gamepad` is the controller's request (direction, activate, back, scroll) and `slider_step` its left/right
    /// presses for a focused slider.
    pub fn begin_input_frame(&mut self, ui: &mut Ui<'_>, gamepad: NavInput, slider_step: i32) {
        self.back_consumed = false;
        self.text_focused_last = self.text_focused_now;
        self.text_focused_now = false;
        self.scroll_container = None;
        self.slider_steps = slider_step;
        // Arrow keys move a text field's caret, so the keyboard only navigates when none is being edited.
        let keyboard = if self.text_focused_last {
            NavInput::default()
        } else {
            NavInput::from_keyboard(&ui.engine())
        };
        let input = NavInput {
            direction: gamepad.direction.or(keyboard.direction),
            activate: gamepad.activate || keyboard.activate,
            back: gamepad.back || keyboard.back,
            scroll: gamepad.scroll + keyboard.scroll,
        };
        if input.direction.is_some() || input.activate {
            self.keep_alive.keep_awake_for(Duration::from_millis(50));
        }
        self.focus.begin_frame(ui, input);
        if self.focus.moved() {
            self.keep_alive.keep_awake_for(Duration::from_millis(50));
        }
    }

    /// Tells the kit the controller asked for the focused element's context menu this frame.
    pub fn set_context_pressed(&mut self, pressed: bool) {
        self.context_pressed = pressed;
    }

    /// Where the context menu of element `id` should open, if it was asked for this frame: at the pointer on a
    /// right-click, or under the element when the controller's context button (or Shift+F10) is pressed while it has
    /// focus.
    pub fn secondary(&mut self, ui: &mut Ui<'_>, id: &str) -> Option<Vec2> {
        if ui_kit::pressed(ui, id, vertex_shell::sturdy::MouseButton::Right) {
            return Some(ui.pointer_position());
        }
        if self.context_pressed && self.focus.is_focused(id) {
            let (origin, size) = ui.element_bounds(id)?;
            return Some(Vec2::new(origin.x + size.x * 0.5, origin.y + size.y));
        }
        None
    }

    /// Whether a back request (Escape, controller B) came in this frame; dialogs treat it as a dismissal.
    pub fn back_pressed(&self) -> bool {
        self.focus.back_pressed() && !self.back_consumed
    }

    /// Marks this frame's back request as used, so an open dialog stays open.
    pub fn consume_back(&mut self) {
        self.back_consumed = true;
    }

    /// Closes every searchable dropdown's popup; true if one was open.
    pub fn close_search_popups(&mut self) -> bool {
        let mut closed = false;
        for (menu, _) in self.searches.values_mut() {
            if menu.is_open() {
                menu.close();
                closed = true;
            }
        }
        closed
    }

    /// Whether the focused element is a slider (left/right then change its value instead of moving focus).
    pub fn focused_is_slider(&self) -> bool {
        self.focus
            .focused()
            .is_some_and(|id| self.sliders.contains_key(id))
    }

    /// Declares `id` focusable and returns whether it has focus now. Anything the user can act on is focusable, so this
    /// is also what gives it the pointing-hand cursor.
    pub fn focusable(&mut self, id: &str) -> bool {
        self.cursor_target(id, CursorKind::Pointer);
        self.focusable_without_cursor(id)
    }

    fn focusable_without_cursor(&mut self, id: &str) -> bool {
        let container = self.scroll_container.clone();
        self.focus.focusable(id, container.as_deref())
    }

    /// Whether the scrollbar thumb of the scroll area `id` is being dragged.
    pub fn scrollbar_dragging(&self, id: &str) -> bool {
        self.scroll_drag
            .as_ref()
            .is_some_and(|(dragging, _)| dragging == id)
    }

    /// A click on `id` or its activation from the keyboard or controller. Also declares it focusable.
    pub fn clicked(&mut self, ui: &mut Ui<'_>, id: &str) -> bool {
        if self.focusable(id) {
            self.focus_ring(ui, id);
        }
        ui.clicked(id) || self.focus.activated(id)
    }

    /// [`Kit::clicked`] for an element that is not part of keyboard navigation (a close button inside a card that is
    /// itself focusable, say): a plain click, with the pointing-hand cursor.
    pub fn clicked_plain(&mut self, ui: &mut Ui<'_>, id: &str) -> bool {
        self.cursor_target(id, CursorKind::Pointer);
        ui.clicked(id)
    }

    /// [`Kit::clicked`] for the window's own controls (minimize, maximize, close), which keep the ordinary cursor.
    pub fn clicked_with_default_cursor(&mut self, ui: &mut Ui<'_>, id: &str) -> bool {
        if self.focusable_without_cursor(id) {
            self.focus_ring(ui, id);
        }
        ui.clicked(id) || self.focus.activated(id)
    }

    // -- Mouse cursor -----------------------------------------------------------------------------------------------
    //
    // One rule for the whole UI. Interactive elements register themselves while they are built (`focusable` and
    // `clicked_plain` for things to click, the text fields for things to type in); `resolve_cursor`, run once at the
    // end of the frame, picks the cursor from whichever of them the pointer is over: the text cursor over an editable
    // text box, the pointing hand over anything that does something when clicked, the ordinary arrow everywhere else.
    // Nothing sets a cursor on an element directly.

    /// Registers `id` as an element the pointer should show `kind` over.
    pub fn cursor_target(&mut self, id: &str, kind: CursorKind) {
        self.cursor_targets.push((id.to_owned(), kind));
    }

    /// Chooses the cursor for this frame from the elements registered during it. Call once, after the UI is built.
    pub fn resolve_cursor(&mut self, ui: &mut Ui<'_>) {
        // The text cursor wins over the hand where a field sits inside something clickable.
        let kind = self
            .cursor_targets
            .iter()
            .filter(|(id, _)| ui.hovered(id))
            .map(|(_, kind)| *kind)
            .max();
        self.cursor_targets.clear();
        vertex_shell::set_cursor(match kind {
            Some(CursorKind::Text) => vertex_shell::PointerCursor::Text,
            Some(CursorKind::Pointer) => vertex_shell::PointerCursor::Hand,
            None => vertex_shell::PointerCursor::Arrow,
        });
    }

    /// Outlines the element `id` (drawn on top of it, from its last known size) while it has keyboard or controller
    /// focus. For elements that are not widgets of their own.
    pub fn focus_ring(&mut self, ui: &mut Ui<'_>, id: &str) {
        let Some((_, size)) = ui.element_bounds(id) else {
            return;
        };
        let _ring = ui.element(
            &ElementDesc::new()
                .id(format!("{id}::focus-ring"))
                .width(Sizing::Fixed(size.x))
                .height(Sizing::Fixed(size.y))
                .corner_radius(CornerRadius::all(10.0))
                .border(Border::all(self.palette.highlight, 2))
                .floating(Some(Floating {
                    element_attach_point: AttachPoint::LeftTop,
                    parent_attach_point: AttachPoint::LeftTop,
                    z_index: in_modal_layer(20),
                    capture_pointer: false,
                    ..Floating::new(FloatTarget::Element(id.to_owned()))
                })),
        );
    }

    /// The ring drawn around the focused element, or no border when it is not focused.
    pub fn focus_border(&self, focused: bool) -> Border {
        if focused {
            Border::all(self.palette.highlight, 2)
        } else {
            Border::default()
        }
    }

    /// Keeps frames coming for `duration` (animations, polling a background task).
    pub fn keep_awake_for(&mut self, duration: Duration) {
        self.keep_alive.keep_awake_for(duration);
    }

    /// The size of body text in pixels, with the launcher-wide UI font size applied (18 is neutral).
    fn px(&self, scale: f32) -> u16 {
        let body = self.typography.resolve(config::TextRole::Body).size * self.font_size / 18.0;
        (body * scale).round().clamp(8.0, 96.0) as u16
    }

    /// The config text role a kit role is drawn with.
    fn config_role(role: TextRole) -> config::TextRole {
        match role {
            TextRole::Title => config::TextRole::PageHeading,
            TextRole::Heading => config::TextRole::SectionHeading,
            TextRole::Body | TextRole::Muted | TextRole::Danger | TextRole::Warning => {
                config::TextRole::Body
            }
            TextRole::Strong => config::TextRole::BodyStrong,
            TextRole::Code => config::TextRole::Code,
            TextRole::Small => config::TextRole::Caption,
            TextRole::OnAccent => config::TextRole::Button,
        }
    }

    pub fn style(&self, role: TextRole) -> TextStyle {
        let p = &self.palette;
        let color = match role {
            TextRole::Title
            | TextRole::Heading
            | TextRole::Body
            | TextRole::Strong
            | TextRole::Code => p.text,
            TextRole::Muted | TextRole::Small => p.text_muted,
            TextRole::OnAccent => p.on_primary,
            TextRole::Danger => p.danger,
            TextRole::Warning => p.warning,
        };
        let config_role = Self::config_role(role);
        let resolved = self.typography.resolve(config_role);
        let scale = self.font_size / 18.0;
        TextStyle {
            color,
            font_id: super::fonts::role_font_id(config_role),
            font_size: (resolved.size * scale).round().clamp(8.0, 96.0) as u16,
            line_height: (resolved.line_height * scale).round().clamp(0.0, 200.0) as u16,
            letter_spacing: resolved.letter_spacing.max(0.0).round() as u16,
            ..TextStyle::default()
        }
    }

    /// The engine font id text inputs and dropdown options use.
    fn input_font(&self) -> u16 {
        super::fonts::role_font_id(config::TextRole::Input)
    }

    pub fn text(&self, ui: &mut Ui<'_>, role: TextRole, text: &str) {
        label(ui, text, &self.style(role));
    }

    /// Text that wraps at word boundaries to the width its container gives it.
    pub fn text_wrapped(&self, ui: &mut Ui<'_>, role: TextRole, text: &str) {
        label(
            ui,
            text,
            &TextStyle {
                wrap: TextWrap::Words,
                ..self.style(role)
            },
        );
    }

    /// Text on a single line, cut with an ellipsis at `max_width`.
    pub fn text_clipped(&self, ui: &mut Ui<'_>, role: TextRole, text: &str, max_width: f32) {
        let style = TextStyle {
            wrap: TextWrap::None,
            ..self.style(role)
        };
        let text = launcher_ui::privacy::scrub(text);
        let shown = ui.ellipsize(&style, &text, max_width);
        ui.text(&shown, &style);
    }

    /// [`text_clipped`](Self::text_clipped) for text inside the element `container_id`: it is cut with an ellipsis at
    /// whatever width that element was given last frame, so it follows the window instead of a fixed number. The
    /// container should grow to fill its row and clip horizontally (so it can shrink below its content); `fallback` is
    /// used on the first frame, before it has been measured.
    pub fn text_fitted(
        &self,
        ui: &mut Ui<'_>,
        role: TextRole,
        text: &str,
        container_id: &str,
        fallback: f32,
    ) {
        let width = ui
            .element_bounds(container_id)
            .map_or(fallback, |(_, size)| size.x - 1.0)
            .max(0.0);
        self.text_clipped(ui, role, text, width);
    }

    /// Text that may mention accounts (see `launcher_ui::account_text`): names read as plain text, and in streamer mode
    /// as "Hidden Account" highlighted in the account's chosen colour. Text without a mention is an ordinary label.
    pub fn account_label(&self, ui: &mut Ui<'_>, role: TextRole, text: &str, streamer_mode: bool) {
        use launcher_ui::account_text::{self, Segment};
        if !account_text::has_mentions(text) {
            return self.text(ui, role, text);
        }
        let style = self.style(role);
        let _row = ui.element(
            &ElementDesc::new()
                .width(Sizing::Fit)
                .height(Sizing::Fit)
                .direction(Direction::LeftToRight)
                .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
        );
        for segment in account_text::segments(text) {
            match segment {
                Segment::Text(text) => label_text(ui, text, &style),
                Segment::Account { id, name } => {
                    let shown = account_text::display_name(name, streamer_mode);
                    let highlight = streamer_mode
                        .then(|| launcher_ui::account_colors::color_for(id))
                        .flatten();
                    match highlight {
                        Some(_) => account_chip(ui, shown, Some(id), &style),
                        None => label_text(ui, shown, &style),
                    }
                }
            }
        }
    }

    pub fn text_colored(&self, ui: &mut Ui<'_>, text: &str, color: Vec4, scale: f32) {
        label(
            ui,
            text,
            &TextStyle {
                color,
                font_size: self.px(scale),
                font_id: super::fonts::role_font_id(config::TextRole::Body),
                ..TextStyle::default()
            },
        );
    }

    /// A themed icon `size` pixels square.
    pub fn icon(&self, ui: &mut Ui<'_>, icon: Icon, size: f32, color: Vec4) {
        icons::draw(ui, icon, size, color, self.svg_supersample);
    }

    /// What the console `id` shows: its lines with repeats merged (`launcher_ui::log_collapse`). Each frame only the
    /// lines new since the last are taken in, merged into what the console already holds, so a pattern spread over many
    /// frames merges just the same. `first_index` is the position of `lines[0]` among every line the console has had.
    pub fn merged_log(&mut self, id: &str, lines: &[String], first_index: u64) -> Arc<Vec<String>> {
        self.mergers
            .entry(id.to_owned())
            .or_default()
            .update(lines, first_index)
    }

    /// Severity of each line of a log, recomputed only when the lines change (same buffer, same length = same levels).
    pub fn log_levels(
        &mut self,
        id: &str,
        lines: &[String],
    ) -> Vec<Option<launcher_ui::log_format::LogLevel>> {
        let key = (lines.as_ptr() as usize, lines.len());
        if let Some((ptr, len, levels)) = self.levels.get(id)
            && (*ptr, *len) == key
        {
            return levels.clone();
        }
        let levels = launcher_ui::log_format::resolve_log_levels(lines);
        self.levels
            .insert(id.to_owned(), (key.0, key.1, levels.clone()));
        levels
    }

    /// Draws a loaded picture (or a placeholder while it loads or if it failed) in a `width` x `height` box, cropped
    /// to fill with rounded corners.
    #[allow(clippy::too_many_arguments)]
    pub fn image(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        key: &str,
        source: impl FnOnce() -> ImageSource,
        width: f32,
        height: f32,
        radius: f32,
    ) {
        let picture: Option<Picture> =
            self.images
                .get(key, source, (width.max(height) * 2.0) as u32);
        let desc = ElementDesc::new()
            .id(id.into())
            .width(Sizing::Fixed(width))
            .height(Sizing::Fixed(height))
            .corner_radius(CornerRadius::all(radius));
        match picture {
            Some(picture) => ui.image(&desc, picture.texture),
            None => {
                if self.images.loading() {
                    self.keep_alive.keep_awake_for(Duration::from_millis(100));
                }
                drop(ui.element(&desc.background(self.palette.bg_light)));
            }
        }
    }

    // -- Layout ---------------------------------------------------------------------------------------------------

    pub fn row(&self, ui: &mut Ui<'_>, gap: u16) -> ElementScope {
        ui.element(
            &ElementDesc::new()
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::LeftToRight)
                .child_gap(gap)
                .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
        )
    }

    pub fn column(&self, ui: &mut Ui<'_>, gap: u16) -> ElementScope {
        ui.element(
            &ElementDesc::new()
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::TopToBottom)
                .child_gap(gap),
        )
    }

    /// Flexible empty space that pushes the following siblings to the far end of a row.
    pub fn fill(&self, ui: &mut Ui<'_>) {
        drop(
            ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fixed(1.0)),
            ),
        );
    }

    pub fn gap(&self, ui: &mut Ui<'_>, size: f32) {
        drop(
            ui.element(
                &ElementDesc::new()
                    .width(Sizing::Fixed(size))
                    .height(Sizing::Fixed(size)),
            ),
        );
    }

    /// A vertical scroll container that fills its parent, with a draggable scrollbar on its right edge.
    pub fn scroll(&mut self, ui: &mut Ui<'_>, id: &str, gap: u16, padding: u16) -> ScrollScope {
        self.scroll_container = Some(id.to_owned());
        // The scroll area has exactly one child, and no child gap of its own. The layout engine leaves a scroll
        // container's children out of its minimum size, except for the gaps between them (a known quirk of its sizing),
        // so with the page's rows as direct children, 16 px each, the area's minimum height grew with the number of
        // rows, could not be shrunk to the window, and ran past the bottom of it. The gapped column is that one child.
        let scroll = ui.element(
            &ElementDesc::new()
                .id(id.into())
                .width(Sizing::Grow)
                .height(Sizing::Grow)
                .direction(Direction::TopToBottom)
                .padding(Padding::all(padding))
                .clip(Clip::vertical()),
        );
        self.scrollbar(ui, id);
        let content = ui.element(
            &ElementDesc::new()
                .id(format!("{id}::content"))
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::TopToBottom)
                .child_gap(gap),
        );
        ScrollScope {
            _content: content,
            _scroll: scroll,
        }
    }

    /// The scrollbar for the clipped element `id`, drawn from last frame's scroll metrics and dragged by grabbing its
    /// thumb. Call it from inside the element.
    pub fn scrollbar(&mut self, ui: &mut Ui<'_>, id: &str) {
        const MARGIN: f32 = 4.0;
        let Some(metrics) = ui.scroll_metrics(id) else {
            return;
        };
        let Some((origin, size)) = ui.element_bounds(id) else {
            return;
        };
        if !metrics.vertical || metrics.content_size.y <= metrics.container_size.y + 1.0 {
            self.scroll_drag = None;
            return;
        }
        let track = (size.y - MARGIN * 2.0).max(1.0);
        let max_offset = metrics.content_size.y - metrics.container_size.y;
        let thumb_height =
            (track * metrics.container_size.y / metrics.content_size.y).clamp(28.0, track);
        let travel = (track - thumb_height).max(1.0);
        let thumb_id = format!("{id}::thumb");
        let pointer = ui.pointer_position();
        if !ui.pointer_down() {
            self.scroll_drag = None;
        } else if self.scroll_drag.is_none() && ui.hovered(&thumb_id) {
            let current = (-metrics.offset.y).clamp(0.0, max_offset) / max_offset * travel;
            self.scroll_drag = Some((id.to_owned(), pointer.y - (origin.y + MARGIN + current)));
        }
        let mut offset = (-metrics.offset.y).clamp(0.0, max_offset);
        if let Some((dragging, grab)) = &self.scroll_drag
            && dragging == id
        {
            let thumb_top = (pointer.y - origin.y - MARGIN - grab).clamp(0.0, travel);
            offset = thumb_top / travel * max_offset;
            ui.set_scroll_offset(id, Vec2::new(0.0, -offset));
            self.keep_alive.keep_awake_for(Duration::from_millis(50));
        }
        let thumb_top = offset / max_offset * travel;
        let active = self
            .scroll_drag
            .as_ref()
            .is_some_and(|(dragging, _)| dragging == id);
        let alpha = if active || ui.hovered(&thumb_id) {
            0.7
        } else {
            0.35
        };
        let _thumb = ui.element(
            &ElementDesc::new()
                .id(thumb_id)
                .width(Sizing::Fixed(6.0))
                .height(Sizing::Fixed(thumb_height))
                .background(self.palette.text_muted.with_w(alpha))
                .corner_radius(CornerRadius::all(3.0))
                .floating(Some(Floating {
                    element_attach_point: AttachPoint::RightTop,
                    parent_attach_point: AttachPoint::RightTop,
                    offset: Vec2::new(-3.0, MARGIN + thumb_top),
                    z_index: in_modal_layer(100),
                    capture_pointer: true,
                    // Otherwise the thumb draws over everything and ignores the clips around its list: a collapsing
                    // section, or the page's own scroll area.
                    clip_to_target: true,
                    ..Floating::new(FloatTarget::Element(id.to_owned()))
                })),
        );
    }

    /// A raised panel (settings section, instance card, dialog body).
    pub fn card(&self, ui: &mut Ui<'_>, id: &str, gap: u16) -> ElementScope {
        let p = &self.palette;
        ui.element(
            &ElementDesc::new()
                .id(id.into())
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::TopToBottom)
                .child_gap(gap)
                .padding(Padding::all(SPACE_LG))
                .background(p.bg)
                .corner_radius(CornerRadius::all(CORNER))
                .border(Border::all(p.border_muted, 1)),
        )
    }

    /// A section heading with its description underneath.
    pub fn section_header(&self, ui: &mut Ui<'_>, heading: &str, description: &str) {
        let _col = self.column(ui, SPACE_XS);
        self.text(ui, TextRole::Heading, heading);
        if !description.is_empty() {
            self.text(ui, TextRole::Muted, description);
        }
    }

    // -- Buttons --------------------------------------------------------------------------------------------------

    fn button_style(&self, kind: ButtonKind) -> vertex_shell::sturdy::widgets::ButtonStyle {
        match kind {
            ButtonKind::Neutral => self.palette.button(),
            ButtonKind::Primary => self.palette.primary_button(),
            ButtonKind::Danger => self.palette.danger_button(),
        }
    }

    /// A text button sized to its label. Returns true when clicked.
    pub fn button(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        label: &str,
        kind: ButtonKind,
        enabled: bool,
    ) -> bool {
        self.button_sized(ui, id, label, kind, enabled, Sizing::Fit)
    }

    pub fn button_sized(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        label: &str,
        kind: ButtonKind,
        enabled: bool,
        width: Sizing,
    ) -> bool {
        self.button_full(ui, id, None, label, kind, enabled, width)
    }

    /// A button with an icon to the left of its centred label.
    #[allow(clippy::too_many_arguments)]
    pub fn button_icon(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        icon: Icon,
        label: &str,
        kind: ButtonKind,
        enabled: bool,
        width: Sizing,
    ) -> bool {
        self.button_full(ui, id, Some(icon), label, kind, enabled, width)
    }

    /// `button_icon` with the icon after the label.
    #[allow(clippy::too_many_arguments)]
    pub fn button_icon_right(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        icon: Icon,
        label: &str,
        kind: ButtonKind,
        enabled: bool,
        width: Sizing,
    ) -> bool {
        self.icon_after = true;
        let clicked = self.button_full(ui, id, Some(icon), label, kind, enabled, width);
        self.icon_after = false;
        clicked
    }

    #[allow(clippy::too_many_arguments)]
    fn button_full(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        icon: Option<Icon>,
        label: &str,
        kind: ButtonKind,
        enabled: bool,
        width: Sizing,
    ) -> bool {
        let style = self.button_style(kind);
        let text = match kind {
            ButtonKind::Neutral => self.style(TextRole::Body),
            ButtonKind::Primary | ButtonKind::Danger => self.style(TextRole::OnAccent),
        };
        let text = if enabled {
            text
        } else {
            TextStyle {
                color: text.color.with_w(0.5),
                ..text
            }
        };
        let focused = enabled && self.focusable(id);
        let activated = enabled && self.focus.activated(id);
        let ring = self.focus_border(focused);
        let state = self
            .buttons
            .entry(id.to_owned())
            .or_insert_with(ButtonState::new);
        let (result, scope) = ui.button(
            &ElementDesc::new()
                .id(id.into())
                .width(width)
                .height(Sizing::Fixed(CONTROL_HEIGHT))
                .padding(Padding::symmetric(SPACE_LG, 0))
                .direction(Direction::LeftToRight)
                .child_gap(SPACE_SM)
                .align(ChildAlignment::center())
                .border(ring),
            state,
            &style,
            enabled,
        );
        if let Some(scope) = scope {
            let after = self.icon_after;
            if let (Some(icon), false) = (icon, after) {
                self.icon(ui, icon, 16.0, text.color);
            }
            self::label(ui, label, &text);
            if let (Some(icon), true) = (icon, after) {
                self.icon(ui, icon, 16.0, text.color);
            }
            drop(scope);
        }
        enabled && (result.clicked || activated)
    }

    /// A square icon-only button. Returns true when clicked.
    pub fn icon_button(&mut self, ui: &mut Ui<'_>, id: &str, icon: Icon, size: f32) -> bool {
        let focused = self.focusable(id);
        let ring = self.focus_border(focused);
        let hovered = ui.hovered(id);
        {
            let _button = ui.element(
                &ElementDesc::new()
                    .id(id.into())
                    .width(Sizing::Fixed(size))
                    .height(Sizing::Fixed(size))
                    .align(ChildAlignment::center())
                    .background(if hovered {
                        self.palette.hovered_fill()
                    } else {
                        Vec4::ZERO
                    })
                    .corner_radius(CornerRadius::all(8.0))
                    .border(ring),
            );
            self.icon(ui, icon, size * 0.55, self.palette.text);
        }
        ui.clicked(id) || self.focus.activated(id)
    }

    /// Two buttons that look like one: a wide main action and a narrow `+ ▾` menu opener on its right. Returns
    /// (main clicked, menu clicked).
    pub fn split_button(
        &mut self,
        ui: &mut Ui<'_>,
        main_id: &str,
        label: &str,
        menu_id: &str,
        main_enabled: bool,
    ) -> (bool, bool) {
        let fill = self.palette.primary;
        let ink = self.palette.on_primary;
        let main_focused = self.focusable(main_id);
        let menu_focused = self.focusable(menu_id);
        let main_hover = main_enabled && ui.hovered(main_id);
        let menu_hover = ui.hovered(menu_id);
        let shade = |hovered: bool, enabled: bool| {
            let base = if hovered {
                fill.lerp(Vec4::new(1.0, 1.0, 1.0, fill.w), 0.18)
            } else {
                fill
            };
            if enabled {
                base
            } else {
                base.with_w(base.w * 0.5)
            }
        };
        let radius = 10.0;
        {
            let _row = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fixed(CONTROL_HEIGHT))
                    .direction(Direction::LeftToRight)
                    .child_gap(1),
            );
            {
                let _main = ui.element(
                    &ElementDesc::new()
                        .id(main_id.into())
                        .width(Sizing::Grow)
                        .height(Sizing::Grow)
                        .padding(Padding::symmetric(SPACE_LG, 0))
                        .align(ChildAlignment::center())
                        .background(shade(main_hover, main_enabled))
                        .corner_radius(CornerRadius {
                            top_left: radius,
                            bottom_left: radius,
                            top_right: 0.0,
                            bottom_right: 0.0,
                        })
                        .border(self.focus_border(main_focused)),
                );
                label_text(
                    ui,
                    label,
                    &TextStyle {
                        color: ink,
                        ..self.style(TextRole::OnAccent)
                    },
                );
            }
            {
                let _menu = ui.element(
                    &ElementDesc::new()
                        .id(menu_id.into())
                        .width(Sizing::Fixed(CONTROL_HEIGHT + 18.0))
                        .height(Sizing::Grow)
                        .direction(Direction::LeftToRight)
                        .child_gap(2)
                        .align(ChildAlignment::center())
                        .background(shade(menu_hover, true))
                        .corner_radius(CornerRadius {
                            top_left: 0.0,
                            bottom_left: 0.0,
                            top_right: radius,
                            bottom_right: radius,
                        })
                        .border(self.focus_border(menu_focused)),
                );
                self.icon(ui, Icon::Plus, 16.0, ink);
                self.icon(ui, Icon::ChevronDown, 12.0, ink);
            }
        }
        let main = main_enabled && (ui.clicked(main_id) || self.focus.activated(main_id));
        let menu = ui.clicked(menu_id) || self.focus.activated(menu_id);
        (main, menu)
    }

    /// A square icon-only sidebar entry; hover shows the name through a tooltip the caller attaches.
    pub fn nav_icon(&mut self, ui: &mut Ui<'_>, id: &str, icon: Icon, selected: bool) -> bool {
        self.nav_thumbnail(ui, id, selected, |ui, kit| {
            kit.icon(
                ui,
                icon,
                22.0,
                if selected {
                    kit.palette.text
                } else {
                    kit.palette.text_muted
                },
            )
        })
    }

    /// A square sidebar entry holding whatever `content` draws (an icon, an instance thumbnail).
    pub fn nav_thumbnail(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        selected: bool,
        content: impl FnOnce(&mut Ui<'_>, &mut Self),
    ) -> bool {
        let focused = self.focusable(id);
        let ring = self.focus_border(focused);
        let hovered = ui.hovered(id);
        let background = if selected {
            self.palette.primary.with_w(0.28)
        } else if hovered {
            self.palette.hovered_fill()
        } else {
            Vec4::ZERO
        };
        {
            let _item = ui.element(
                &ElementDesc::new()
                    .id(id.into())
                    .width(Sizing::Grow)
                    .height(Sizing::Fixed(44.0))
                    .align(ChildAlignment::center())
                    .background(background)
                    .corner_radius(CornerRadius::all(12.0))
                    .border(ring),
            );
            content(ui, self);
        }
        ui.clicked(id) || self.focus.activated(id)
    }

    /// A flat, borderless clickable row (sidebar entries, menu items). `selected` draws the accent tint.
    pub fn nav_item(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        icon: Option<Icon>,
        label: &str,
        selected: bool,
    ) -> bool {
        self.nav_item_ex(ui, id, icon, label, selected, true)
    }

    /// `nav_item`; with `truncate` false the label is never shortened, so the menu around it must be as wide as it.
    pub fn nav_item_ex(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        icon: Option<Icon>,
        label: &str,
        selected: bool,
        truncate: bool,
    ) -> bool {
        let focused = self.focusable(id);
        let ring = self.focus_border(focused);
        let p = &self.palette;
        let background = if selected {
            p.primary.with_w(0.28)
        } else if ui.hovered(id) {
            p.hovered_fill()
        } else {
            Vec4::ZERO
        };
        {
            let _item = ui.element(
                &ElementDesc::new()
                    .id(id.into())
                    .width(Sizing::Grow)
                    .height(Sizing::Fixed(CONTROL_HEIGHT))
                    .padding(Padding::symmetric(SPACE_MD, 0))
                    .child_gap(SPACE_SM)
                    .align(ChildAlignment::new(AlignX::Left, AlignY::Center))
                    .background(background)
                    .corner_radius(CornerRadius::all(10.0))
                    .border(ring),
            );
            if let Some(icon) = icon {
                self.icon(
                    ui,
                    icon,
                    18.0,
                    if selected {
                        self.palette.text
                    } else {
                        self.palette.text_muted
                    },
                );
            }
            let style = TextStyle {
                wrap: TextWrap::None,
                ..self.style(TextRole::Body)
            };
            let label = launcher_ui::privacy::scrub(label);
            if truncate {
                let shown =
                    ui.ellipsize(&style, &label, if icon.is_some() { 160.0 } else { 190.0 });
                ui.text(&shown, &style);
            } else {
                ui.text(&label, &style);
            }
        }
        ui.clicked(id) || self.focus.activated(id)
    }

    /// Whether context menu `id` is still on screen, including while it folds away after closing.
    pub fn menu_visible(&self, id: &str) -> bool {
        self.menu_anim
            .get(id)
            .is_some_and(|anim| anim.progress > 0.0)
    }

    /// The right-click menu: rows with their icon at the right, danger rows in red, opening at the pointer with the
    /// panel unfolding and the rows sliding in, and folding away again on close. Returns the chosen row.
    pub fn context_menu(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        state: &mut ui_kit::PopupState,
        items: &[MenuEntry],
    ) -> Option<usize> {
        const ROW_HEIGHT: f32 = 30.0;
        const SEPARATOR_HEIGHT: f32 = 9.0;
        const SLIDE: f32 = 14.0;
        const COLLAPSED: f32 = 2.0;
        const OPEN_SECONDS: f32 = 0.16;
        const CLOSE_SECONDS: f32 = 0.12;
        let delta = self.delta_seconds.clamp(0.0, 0.1);
        let now = uptime();
        let open = state.is_open();
        let bounds = ui.element_bounds(id);
        let entry = self.menu_anim.entry(id.to_owned()).or_default();
        if open && !entry.was_open && now - entry.last_call > 0.1 {
            // Opened after the menu had stopped being drawn: unfold from the start.
            entry.progress = 0.0;
        }
        entry.was_open = open;
        entry.last_call = now;
        if open {
            if let Some((origin, _)) = bounds {
                entry.origin = origin;
            }
            entry.progress = (entry.progress + delta / OPEN_SECONDS).min(1.0);
        } else {
            entry.progress = (entry.progress - delta / CLOSE_SECONDS).max(0.0);
        }
        let (progress, origin) = (entry.progress, entry.origin);
        if !open && progress <= 0.0 {
            return None;
        }
        if (open && progress < 1.0) || (!open && progress > 0.0) {
            self.keep_alive.keep_awake_for(Duration::from_millis(32));
        }
        let eased = if progress < 0.5 {
            2.0 * progress * progress
        } else {
            1.0 - (-2.0 * progress + 2.0).powi(2) / 2.0
        };
        let full: f32 = items
            .iter()
            .map(|item| {
                ROW_HEIGHT
                    + if item.separator_before {
                        SEPARATOR_HEIGHT
                    } else {
                        0.0
                    }
            })
            .sum();
        let height = COLLAPSED + (full - COLLAPSED) * eased;
        let slide = ((1.0 - eased) * SLIDE) as u16;

        let style = PopupStyle {
            background: self.palette.bg_dark,
            border: self.palette.border_muted,
            corner_radius: 10.0,
            padding: Padding::all(8),
            z_index: Z_POPUP,
            ..PopupStyle::default()
        };
        // Closing: a fresh popup pinned where the panel was, so it folds away without reacting to clicks.
        let mut shown = if open {
            state.clone()
        } else {
            let mut ghost = ui_kit::PopupState::default();
            ghost.open_at(origin);
            ghost
        };
        let mut chosen = None;
        ui_kit::popup(ui, id, id, &mut shown, &style, |ui, popup| {
            let _clip = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Fit)
                    .height(Sizing::Fixed(height))
                    .direction(Direction::TopToBottom)
                    .padding(Padding {
                        left: 0,
                        right: 0,
                        top: slide,
                        bottom: 0,
                    })
                    .clip(Clip::vertical()),
            );
            drop(
                ui.element(
                    &ElementDesc::new()
                        .width(Sizing::Fixed(124.0))
                        .height(Sizing::Fixed(0.0)),
                ),
            );
            for (index, item) in items.iter().enumerate() {
                if item.separator_before {
                    let _gap = ui.element(
                        &ElementDesc::new()
                            .width(Sizing::Grow)
                            .height(Sizing::Fixed(SEPARATOR_HEIGHT))
                            .align(ChildAlignment::center()),
                    );
                    drop(
                        ui.element(
                            &ElementDesc::new()
                                .width(Sizing::Grow)
                                .height(Sizing::Fixed(1.0))
                                .background(self.palette.border_muted),
                        ),
                    );
                }
                let row_id = format!("{id}::{index}");
                self.cursor_target(&row_id, CursorKind::Pointer);
                let hovered = open && ui.hovered(&row_id);
                let ink = if item.danger {
                    self.palette.danger
                } else {
                    self.palette.text
                };
                let fill = if !hovered {
                    Vec4::ZERO
                } else if item.danger {
                    self.palette.danger.with_w(0.16)
                } else {
                    self.palette.hovered_fill()
                };
                {
                    let _row = ui.element(
                        &ElementDesc::new()
                            .id(row_id.clone())
                            .width(Sizing::Grow)
                            .height(Sizing::Fixed(ROW_HEIGHT))
                            .padding(Padding::symmetric(10, 0))
                            .child_gap(SPACE_MD)
                            .align(ChildAlignment::new(AlignX::Left, AlignY::Center))
                            .background(fill)
                            .corner_radius(CornerRadius::all(6.0)),
                    );
                    self::label(
                        ui,
                        &item.label,
                        &TextStyle {
                            color: ink,
                            wrap: TextWrap::None,
                            ..self.style(TextRole::Body)
                        },
                    );
                    self.fill(ui);
                    if let Some(icon) = item.icon {
                        self.icon(ui, icon, 16.0, ink);
                    }
                }
                if open && ui.clicked(&row_id) {
                    chosen = Some(index);
                    popup.close();
                }
            }
        });
        if open {
            // The popup counts frames since opening (a click is only "outside" after the first), so it must keep its
            // own state from frame to frame.
            *state = shown;
            if chosen.is_some() {
                state.close();
            }
        }
        chosen
    }

    /// A tab button: as wide as its label (never truncated), the label centred, the selected one tinted.
    pub fn tab(&mut self, ui: &mut Ui<'_>, id: &str, label: &str, selected: bool) -> bool {
        self.tab_sized(ui, id, label, selected, Sizing::Fit)
    }

    /// A tab that takes an equal share of its row: the tabs of a row all use `Sizing::Grow`.
    pub fn tab_sized(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        label: &str,
        selected: bool,
        width: Sizing,
    ) -> bool {
        let focused = self.focusable(id);
        let ring = self.focus_border(focused);
        let hovered = ui.hovered(id);
        let background = if selected {
            self.palette.primary.with_w(0.28)
        } else if hovered {
            self.palette.hovered_fill()
        } else {
            self.palette.bg_light
        };
        {
            let _tab = ui.element(
                &ElementDesc::new()
                    .id(id.into())
                    .width(width)
                    .height(Sizing::Fixed(CONTROL_HEIGHT))
                    .padding(Padding::symmetric(SPACE_LG, 0))
                    .align(ChildAlignment::center())
                    .background(background)
                    .corner_radius(CornerRadius::all(10.0))
                    .border(ring),
            );
            self.text(ui, TextRole::Body, label);
        }
        ui.clicked(id) || self.focus.activated(id)
    }

    // -- Inputs ---------------------------------------------------------------------------------------------------

    /// A switch; returns the new value when the user flipped it.
    pub fn switch(&mut self, ui: &mut Ui<'_>, id: &str, on: bool, enabled: bool) -> Option<bool> {
        let style = self.palette.toggle();
        let focused = enabled && self.focusable(id);
        let activated = enabled && self.focus.activated(id);
        let ring = self.focus_border(focused);
        let state = self
            .toggles
            .entry(id.to_owned())
            .or_insert_with(ToggleState::new);
        let result = ui.switch(
            &ElementDesc::new()
                .id(id.into())
                .width(Sizing::Fixed(46.0))
                .height(Sizing::Fixed(24.0))
                .corner_radius(CornerRadius::all(12.0))
                .border(ring),
            state,
            &style,
            on,
            enabled,
        );
        (enabled && (result.clicked || activated)).then_some(!on)
    }

    /// A slider over `min..=max`; returns the new value while it changes.
    #[allow(clippy::too_many_arguments)]
    pub fn slider(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        value: f64,
        min: f64,
        max: f64,
        step: Option<f64>,
        enabled: bool,
    ) -> Option<f64> {
        self.slider_sized(ui, id, value, min, max, step, enabled, 260.0)
    }

    /// [`Kit::slider`] with an explicit track width.
    #[allow(clippy::too_many_arguments)]
    pub fn slider_sized(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        value: f64,
        min: f64,
        max: f64,
        step: Option<f64>,
        enabled: bool,
        width: f32,
    ) -> Option<f64> {
        let style = self.palette.slider();
        let focused = enabled && self.focusable(id);
        // Controller left/right adjust the focused slider; the engine handles the keyboard itself.
        let mut input = SliderInput::default();
        if focused && self.slider_steps != 0 {
            let key = if self.slider_steps > 0 {
                SliderKey::Increment
            } else {
                SliderKey::Decrement
            };
            input.keys = vec![key; self.slider_steps.unsigned_abs() as usize];
        }
        let state = self
            .sliders
            .entry(id.to_owned())
            .or_insert_with(SliderState::new);
        let result = ui.slider(
            &ElementDesc::new()
                .id(id.into())
                .width(Sizing::Fixed(width))
                .height(Sizing::Fixed(22.0)),
            &SliderConfig {
                min,
                max,
                step,
                ..SliderConfig::range(min, max)
            },
            &style,
            state,
            value,
            &input,
            enabled,
        );
        (result.changed && (result.value - value).abs() > f64::EPSILON).then_some(result.value)
    }

    /// A dropdown of `options`; returns the newly chosen index.
    pub fn dropdown(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        selected: usize,
        options: &[String],
        width: f32,
    ) -> Option<usize> {
        if options.is_empty() {
            return None;
        }
        // One look for every dropdown; long lists (fonts, Minecraft versions) also get a search box.
        self.search_dropdown(ui, id, selected, options, width)
    }

    /// The dropdown every list uses: a virtualised list under the trigger, with a search box once the list is long
    /// enough to need one. Returns the chosen index.
    pub fn search_dropdown(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        selected: usize,
        options: &[String],
        width: f32,
    ) -> Option<usize> {
        self.dropdown_sized(ui, id, selected, options, Sizing::Fixed(width))
    }

    /// A dropdown that takes its share of a row: with `Sizing::Grow`, dropdowns side by side split the row equally.
    pub fn dropdown_grow(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        selected: usize,
        options: &[String],
    ) -> Option<usize> {
        if options.is_empty() {
            return None;
        }
        self.dropdown_sized(ui, id, selected, options, Sizing::Grow)
    }

    fn dropdown_sized(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        selected: usize,
        options: &[String],
        sizing: Sizing,
    ) -> Option<usize> {
        const ROW_HEIGHT: f32 = 30.0;
        const LIST_HEIGHT: f32 = 300.0;
        let trigger = format!("{id}::trigger");
        // A growing trigger's width is last frame's; its text is cut to that, so the text never decides the width and
        // siblings keep splitting the row equally.
        let width = match sizing {
            Sizing::Fixed(width) => width,
            _ => ui
                .element_bounds(&trigger)
                .map_or(160.0, |(_, size)| size.x),
        };
        let focused = self.focusable(&trigger);
        let activated = self.focus.activated(&trigger);
        let ring = self.focus_border(focused);
        let current = options
            .get(selected)
            .map_or("Select...", String::as_str)
            .to_owned();
        let hovered = ui.hovered(&trigger);
        {
            let _button = ui.element(
                &ElementDesc::new()
                    .id(trigger.clone())
                    .width(sizing)
                    .height(Sizing::Fixed(CONTROL_HEIGHT))
                    .padding(Padding::symmetric(SPACE_MD, 0))
                    .child_gap(SPACE_SM)
                    .align(ChildAlignment::new(AlignX::Left, AlignY::Center))
                    .background(if hovered {
                        self.palette.hovered_fill()
                    } else {
                        self.palette.bg_light
                    })
                    .corner_radius(CornerRadius::all(8.0))
                    .border(if focused {
                        ring
                    } else {
                        Border::all(self.palette.border_muted, 1)
                    }),
            );
            let text = TextStyle {
                wrap: TextWrap::None,
                ..self.style(TextRole::Body)
            };
            let shown = ui.ellipsize(
                &text,
                &launcher_ui::privacy::scrub(&current),
                (width - 52.0).max(20.0),
            );
            ui.text(&shown, &text);
            self.icon(ui, Icon::ChevronDown, 14.0, self.palette.text_muted);
        }
        let (mut menu, mut query) = self.searches.remove(id).unwrap_or_default();
        if ui.clicked(&trigger) || activated {
            if menu.is_open() {
                menu.close();
            } else {
                menu.open();
                query.clear();
            }
        }
        let was_open = menu.is_open();
        let searchable = options.len() > SEARCHABLE_DROPDOWN_THRESHOLD;
        let style = PopupStyle {
            background: self.palette.bg_dark,
            border: self.palette.border_muted,
            corner_radius: 10.0,
            padding: Padding::all(SPACE_SM),
            z_index: Z_POPUP,
            ..PopupStyle::default()
        };
        let mut chosen = None;
        let popup_id = format!("{id}::popup");
        ui_kit::popup(ui, &popup_id, &trigger, &mut menu, &style, |ui, popup| {
            let _column = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Fixed(width.max(260.0)))
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
                    .child_gap(SPACE_XS),
            );
            let field = format!("{id}::search");
            if searchable {
                self.text_field(
                    ui,
                    &field,
                    &mut query,
                    "Type to search",
                    false,
                    Sizing::Grow,
                );
                if !was_open || query.is_empty() && !ui.has_focus(&field) && !ui.focused() {
                    ui.focus(&field);
                }
            }
            let needles: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
            let matches: Vec<usize> = options
                .iter()
                .enumerate()
                .filter(|(_, option)| {
                    let lower = option.to_lowercase();
                    needles.iter().all(|needle| lower.contains(needle.as_str()))
                })
                .map(|(index, _)| index)
                .collect();
            let list_id = format!("{id}::list");
            {
                let _list = ui.element(
                    &ElementDesc::new()
                        .id(list_id.clone())
                        .width(Sizing::Grow)
                        .height(Sizing::Fixed(
                            LIST_HEIGHT.min(matches.len().max(1) as f32 * ROW_HEIGHT),
                        ))
                        .direction(Direction::TopToBottom)
                        .clip(Clip::both()),
                );
                self.scrollbar(ui, &list_id);
                if matches.is_empty() {
                    self.text(ui, TextRole::Muted, "No matches found.");
                }
                let visible = ui_kit::visible_rows(ui, &list_id, ROW_HEIGHT, matches.len(), 4);
                ui_kit::spacer(ui, visible.before);
                for row in visible.rows.clone() {
                    let index = matches[row];
                    let item = format!("{id}::option::{index}");
                    let _row = ui.element(
                        &ElementDesc::new()
                            .width(Sizing::Grow)
                            .height(Sizing::Fixed(ROW_HEIGHT)),
                    );
                    if self.nav_item_ex(ui, &item, None, &options[index], index == selected, false)
                    {
                        chosen = Some(index);
                        popup.close();
                    }
                }
                ui_kit::spacer(ui, visible.after);
            }
        });
        self.searches.insert(id.to_owned(), (menu, query));
        chosen.filter(|index| *index != selected)
    }

    /// A dropdown over an enum's values.
    pub fn choice<T: Copy + PartialEq>(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        current: T,
        all: &[T],
        label: impl Fn(T) -> String,
        width: f32,
    ) -> Option<T> {
        let options: Vec<String> = all.iter().map(|value| label(*value)).collect();
        let selected = all.iter().position(|value| *value == current).unwrap_or(0);
        self.dropdown(ui, id, selected, &options, width)
            .map(|index| all[index])
    }

    /// A text field for a number: it refuses every character that no valid value contains, as it is typed or pasted
    /// (digits; one `.` when `decimals`; a leading `-` when `negative`). Otherwise it is [`Kit::text_field_ex`].
    pub fn number_text_field(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        value: &mut String,
        width: f32,
        decimals: bool,
        negative: bool,
    ) -> vertex_shell::sturdy::widgets::TextInputResult {
        let result = self.text_field_ex(ui, id, value, "", false, Sizing::Fixed(width));
        if result.changed {
            let filtered = filter_number_text(value, decimals, negative);
            if filtered != *value {
                if let Some(state) = self.texts.get_mut(id) {
                    state.set_text(&filtered);
                }
                *value = filtered;
            }
        }
        result
    }

    /// Removes from the field `id` (and from `value`) every character `allowed` rejects, so a text box for a value
    /// with a known alphabet never holds a symbol that value cannot contain. Call it right after the field.
    pub fn restrict_text(&mut self, id: &str, value: &mut String, allowed: impl Fn(char) -> bool) {
        if value.chars().all(&allowed) {
            return;
        }
        let filtered: String = value.chars().filter(|ch| allowed(*ch)).collect();
        if let Some(state) = self.texts.get_mut(id) {
            state.set_text(&filtered);
        }
        *value = filtered;
    }

    /// A number box showing `shown`. Returns the number typed once the user finishes (Enter, or leaving the box);
    /// until then the box holds what they are typing, and afterwards it shows `shown` again.
    pub fn number_field(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        shown: &str,
        width: f32,
        decimals: bool,
        negative: bool,
    ) -> Option<f64> {
        let mut text = shown.to_owned();
        let result = self.number_text_field(ui, id, &mut text, width, decimals, negative);
        let editing = result.focused || ui.has_focus(id);
        let was_editing = if editing {
            !self.numbers_editing.insert(id.to_owned())
        } else {
            self.numbers_editing.remove(id)
        };
        if result.submitted || (was_editing && !editing) {
            let typed = self.texts.get(id).map(TextEditState::text)?;
            return typed.trim().parse::<f64>().ok().filter(|n| n.is_finite());
        }
        None
    }

    /// A single-line text field bound to `value`. Returns true when the text changed (and `value` was updated).
    /// The field keeps its own edit state; it re-reads `value` whenever it is not focused, so outside changes show.
    pub fn text_field(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        value: &mut String,
        placeholder: &str,
        masked: bool,
        width: Sizing,
    ) -> bool {
        self.text_field_ex(ui, id, value, placeholder, masked, width)
            .changed
    }

    /// [`Kit::text_field`] that also reports Enter and focus.
    pub fn text_field_ex(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        value: &mut String,
        placeholder: &str,
        masked: bool,
        width: Sizing,
    ) -> vertex_shell::sturdy::widgets::TextInputResult {
        let mut style = self.palette.text_input(
            self.input_font(),
            self.typography
                .resolve(config::TextRole::Input)
                .size
                .mul_add(self.font_size / 18.0, 0.0)
                .round() as u16,
        );
        style.mask_characters = masked;
        let nav_focused = self.focusable(id);
        self.cursor_target(id, CursorKind::Text);
        let activated = self.focus.activated(id);
        let state = self
            .texts
            .entry(id.to_owned())
            .or_insert_with(|| TextEditState::with_text(value));
        if !state.focused() && state.text() != *value {
            state.set_text(value);
        }
        if activated {
            ui.focus(id);
        }
        let result = ui.text_field(
            &ElementDesc::new()
                .id(id.into())
                .width(width)
                .height(Sizing::Fixed(CONTROL_HEIGHT))
                // The field's text is its child: centre it vertically in the box, with a little room at the sides.
                .padding(Padding::symmetric(SPACE_SM, 0))
                .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
            &style,
            state,
            placeholder,
            true,
        );
        if result.changed {
            *value = state.text();
        }
        let _ = nav_focused;
        // The field is being edited when the widget says so (a click focuses it inside the engine without touching
        // the UI's keyboard focus, which only `ui.focus` sets): arrows must then move its caret, not the page's focus.
        if result.focused || ui.has_focus(id) {
            self.text_focused_now = true;
        }
        result
    }

    /// Whether the element `id` was on screen (inside `scroll_id`'s viewport, with `margin` to spare) as of the last
    /// frame. Unknown elements count as visible so first frames load their images.
    pub fn near_viewport(&self, ui: &Ui<'_>, scroll_id: &str, id: &str, margin: f32) -> bool {
        let (Some(metrics), Some((position, size))) =
            (ui.scroll_metrics(scroll_id), ui.element_bounds(id))
        else {
            return true;
        };
        let Some((origin, _)) = ui.element_bounds(scroll_id) else {
            return true;
        };
        let top = position.y - origin.y;
        top + size.y >= -margin && top <= metrics.container_size.y + margin
    }

    /// A section that expands and collapses with an animation. The header is the label on the left, a gap, a rule that
    /// takes the remaining width, and a chevron that points right when closed and turns clockwise to down when open; a rule of the same
    /// look closes the section's bottom. `body` is not built while the section is closed.
    pub fn collapsible(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        label: &str,
        body: impl FnOnce(&mut Ui<'_>, &mut Kit),
    ) {
        const CHEVRON: f32 = 18.0;
        let header_id = format!("{id}::header");
        let was_open = self.collapsibles.get(id).is_some_and(|c| c.open);
        let open = if self.clicked(ui, &header_id) {
            !was_open
        } else {
            was_open
        };
        let (progress, eased) = self.animate_collapse(id, open);

        let hovered = ui.hovered(&header_id);
        {
            let _header = ui.element(
                &ElementDesc::new()
                    .id(header_id.clone())
                    .width(Sizing::Grow)
                    .height(Sizing::Fixed(30.0))
                    .direction(Direction::LeftToRight)
                    .child_gap(SPACE_XS)
                    .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
            );
            self.text(ui, TextRole::Heading, label);
            self.gap(ui, f32::from(SPACE_XS));
            self.fill_rule(ui, if hovered { self.palette.border } else { self.palette.border_muted });
            // Pointing right when closed, turning clockwise a quarter-turn to point down when open. The down chevron
            // turned 270 degrees clockwise points right; 360 is down again.
            let degrees = 270.0 + 90.0 * eased;
            icons::warm_turns(Icon::ChevronDown, 270.0, 360.0);
            icons::draw_turned(
                ui,
                Icon::ChevronDown,
                degrees,
                CHEVRON,
                if hovered {
                    self.palette.text
                } else {
                    self.palette.text_muted
                },
                self.svg_supersample,
            );
        }
        self.collapse_body(ui, id, progress, eased, SPACE_MD, |ui, kit| {
            body(ui, kit);
            kit.fill_rule(ui, kit.palette.border_muted);
        });
    }

    /// Moves the open/close animation of section `id` toward `open` and returns its (progress, eased progress), both
    /// 0 when closed and 1 when open. A section seen for the first time starts at its target, with no animation.
    pub fn animate_collapse(&mut self, id: &str, open: bool) -> (f32, f32) {
        const SECONDS: f32 = 0.22;
        let now = uptime();
        let target = if open { 1.0 } else { 0.0 };
        let entry = self
            .collapsibles
            .entry(id.to_owned())
            .or_insert_with(|| Collapse {
                progress: target,
                last_frame: now,
                ..Collapse::default()
            });
        entry.open = open;
        let delta = (now - entry.last_frame).clamp(0.0, 0.1) as f32;
        entry.last_frame = now;
        if entry.progress < target {
            entry.progress = (entry.progress + delta / SECONDS).min(target);
        } else if entry.progress > target {
            entry.progress = (entry.progress - delta / SECONDS).max(target);
        }
        let progress = entry.progress;
        if progress != target {
            self.keep_awake_for(Duration::from_millis(32));
        }
        let eased = if progress < 0.5 {
            2.0 * progress * progress
        } else {
            1.0 - (-2.0 * progress + 2.0).powi(2) / 2.0
        };
        (progress, eased)
    }

    /// The part of a section that reveals: nothing while closed, a clip whose height follows `eased` while moving, and
    /// the plain content when open. `gap` is the space between the content's children.
    pub fn collapse_body(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        progress: f32,
        eased: f32,
        gap: u16,
        body: impl FnOnce(&mut Ui<'_>, &mut Kit),
    ) {
        if progress <= 0.0 {
            return;
        }
        let content_id = format!("{id}::content");
        let known_height = self.collapsibles.get(id).map_or(0.0, |c| c.height);
        // Fully open: natural height, so the body can change size. Otherwise a clip whose height follows the animation,
        // from the content height measured on an earlier frame.
        let clip = if progress >= 1.0 {
            ElementDesc::new().height(Sizing::Fit)
        } else {
            ElementDesc::new()
                .height(Sizing::Fixed((known_height * eased).max(1.0)))
                .clip(Clip::vertical())
        };
        {
            let _clip = ui.element(&clip.id(format!("{id}::clip")).width(Sizing::Grow));
            let _content = ui.element(
                &ElementDesc::new()
                    .id(content_id.clone())
                    .width(Sizing::Grow)
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
                    .child_gap(gap),
            );
            body(ui, self);
        }
        if let Some((_, size)) = ui.element_bounds(&content_id) {
            self.collapsibles.entry(id.to_owned()).or_default().height = size.y;
        }
    }

    /// A one-pixel rule that takes the rest of its row's width, or a full-width one in a column.
    fn fill_rule(&self, ui: &mut Ui<'_>, color: Vec4) {
        drop(
            ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fixed(1.0))
                    .background(color),
            ),
        );
    }

    /// A removable chip (a search tag). Returns true when its close button was clicked.
    pub fn chip(&mut self, ui: &mut Ui<'_>, id: &str, label: &str) -> bool {
        let close = format!("{id}::close");
        self.cursor_target(&close, CursorKind::Pointer);
        {
            let _chip = ui.element(
                &ElementDesc::new()
                    .id(id.into())
                    .width(Sizing::Fit)
                    .height(Sizing::Fixed(28.0))
                    .padding(Padding::symmetric(SPACE_MD, 0))
                    .child_gap(SPACE_SM)
                    .align(ChildAlignment::new(AlignX::Left, AlignY::Center))
                    .background(self.palette.primary.with_w(0.22))
                    .corner_radius(CornerRadius::all(14.0)),
            );
            label_text(ui, label, &self.style(TextRole::Body));
            {
                let _close = ui.element(
                    &ElementDesc::new()
                        .id(close.clone())
                        .width(Sizing::Fixed(16.0))
                        .height(Sizing::Fixed(16.0))
                        .align(ChildAlignment::center()),
                );
                self.icon(ui, Icon::Close, 12.0, self.palette.text_muted);
            }
        }
        ui.clicked(&close)
    }

    /// Removable chips laid out in as many rows as the width needs, so a long tag list wraps instead of overflowing.
    /// `id` names the wrapping column (its last-frame width decides the row breaks). Returns the index of a chip whose
    /// close button was clicked.
    pub fn chip_rows(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        tags: &[String],
    ) -> Option<usize> {
        let style = self.style(TextRole::Body);
        let available = ui.element_bounds(id).map_or(600.0, |(_, size)| size.x);
        // Padding on both sides, the gap before the close button, and the button itself.
        let chrome = 2.0 * f32::from(SPACE_MD) + f32::from(SPACE_SM) + 16.0;
        let gap = f32::from(SPACE_SM);
        let mut rows: Vec<Vec<usize>> = Vec::new();
        let mut used = 0.0_f32;
        for (index, tag) in tags.iter().enumerate() {
            let width = ui
                .measure_text(&style, &launcher_ui::privacy::scrub(tag))
                .x
                + chrome;
            match rows.last_mut() {
                Some(row) if used + gap + width <= available => {
                    row.push(index);
                    used += gap + width;
                }
                _ => {
                    rows.push(vec![index]);
                    used = width;
                }
            }
        }
        let mut removed = None;
        let _column = ui.element(
            &ElementDesc::new()
                .id(id.into())
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::TopToBottom)
                .child_gap(SPACE_SM),
        );
        for row in rows {
            let _row = self.row(ui, SPACE_SM);
            for index in row {
                if self.chip(ui, &format!("{id}::{index}"), &tags[index]) {
                    removed = Some(index);
                }
            }
        }
        removed
    }

    /// A multi-line text box bound to `value`; `lines` sets its height. Returns true when the text changed.
    pub fn text_area(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        value: &mut String,
        placeholder: &str,
        lines: u16,
    ) -> bool {
        let mut style = self.palette.text_input(
            self.input_font(),
            self.typography
                .resolve(config::TextRole::Input)
                .size
                .mul_add(self.font_size / 18.0, 0.0)
                .round() as u16,
        );
        style.wrap_lines = true;
        let height = f32::from(lines) * f32::from(self.px(1.0)) * 1.5 + 16.0;
        let scrollbar = ScrollbarStyle {
            thumb_color: self.palette.text_muted.with_w(0.35),
            ..ScrollbarStyle::default()
        };
        self.cursor_target(id, CursorKind::Text);
        let state = self
            .texts
            .entry(id.to_owned())
            .or_insert_with(|| TextEditState::with_text(value));
        if !state.focused() && state.text() != *value {
            state.set_text(value);
        }
        let area = self
            .areas
            .entry(id.to_owned())
            .or_insert_with(ScrollAreaState::new);
        let result = ui.text_area(
            &ElementDesc::new()
                .id(id.into())
                .width(Sizing::Grow)
                .height(Sizing::Fixed(height)),
            &style,
            &scrollbar,
            state,
            area,
            placeholder,
            true,
        );
        if result.changed {
            *value = state.text();
        }
        result.changed
    }

    /// A checkbox with a label to its right. Returns the new value when flipped.
    pub fn checkbox(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        label: &str,
        checked: bool,
    ) -> Option<bool> {
        let style = self.palette.toggle();
        let focused = self.focusable(id);
        let activated = self.focus.activated(id);
        let ring = self.focus_border(focused);
        let state = self
            .toggles
            .entry(id.to_owned())
            .or_insert_with(ToggleState::new);
        let mut flipped = false;
        {
            let _row = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fixed(28.0))
                    .direction(Direction::LeftToRight)
                    .child_gap(SPACE_SM)
                    .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
            );
            let result = ui.checkbox(
                &ElementDesc::new()
                    .id(id.into())
                    .width(Sizing::Fixed(22.0))
                    .height(Sizing::Fixed(22.0))
                    .border(ring),
                state,
                &style,
                checked,
                0,
                true,
            );
            flipped = result.clicked || activated;
            label_text(ui, label, &self.style(TextRole::Body));
        }
        flipped.then_some(!checked)
    }

    /// A radio option with a label to its right. Returns true when it was clicked.
    pub fn radio(&mut self, ui: &mut Ui<'_>, id: &str, label: &str, selected: bool) -> bool {
        let style = self.palette.toggle();
        let focused = self.focusable(id);
        let activated = self.focus.activated(id);
        let ring = self.focus_border(focused);
        let state = self
            .toggles
            .entry(id.to_owned())
            .or_insert_with(ToggleState::new);
        let mut clicked = false;
        {
            let _row = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fixed(28.0))
                    .direction(Direction::LeftToRight)
                    .child_gap(SPACE_SM)
                    .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
            );
            let result = ui.radio_button(
                &ElementDesc::new()
                    .id(id.into())
                    .width(Sizing::Fixed(22.0))
                    .height(Sizing::Fixed(22.0))
                    .border(ring),
                state,
                &style,
                selected,
                true,
            );
            clicked = result.clicked || activated;
            label_text(ui, label, &self.style(TextRole::Body));
        }
        clicked
    }

    /// Text the user can select and copy but not edit (paths, codes, log lines).
    pub fn selectable_text(&mut self, ui: &mut Ui<'_>, id: &str, value: &str, width: Sizing) {
        let mut style = self.palette.text_input(
            self.input_font(),
            self.typography
                .resolve(config::TextRole::Input)
                .size
                .mul_add(self.font_size / 18.0, 0.0)
                .round() as u16,
        );
        style.read_only = true;
        let state = self
            .texts
            .entry(id.to_owned())
            .or_insert_with(|| TextEditState::with_text(value));
        if state.text() != value {
            state.set_text(value);
        }
        let _ = ui.text_field(
            &ElementDesc::new()
                .id(id.into())
                .width(width)
                .height(Sizing::Fixed(CONTROL_HEIGHT))
                .padding(Padding::symmetric(SPACE_SM, 0))
                .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
            &style,
            state,
            "",
            true,
        );
    }

    /// Shows `text` as a tooltip while the element `anchor_id` is hovered. It sits to the right of the pointer when the
    /// whole text fits there without wrapping, else on whichever side has more room (the right on a tie); it wraps to
    /// fit that room so nothing leaves the window, and is kept vertically inside the window too.
    pub fn tooltip(&mut self, ui: &mut Ui<'_>, anchor_id: &str, text: &str) {
        const DELAY_SECONDS: f32 = 0.5;
        const GAP: f32 = 16.0; // clears the cursor itself
        const MARGIN: f32 = 8.0; // from the window's edge
        const MAX_WIDTH: f32 = 360.0;
        const PAD_X: u16 = 9;
        const PAD_Y: u16 = 6;

        if !ui.hovered(anchor_id) {
            self.tooltips.remove(anchor_id);
            return;
        }
        let waited = self.tooltips.entry(anchor_id.to_owned()).or_insert(0.0);
        *waited += self.delta_seconds;
        if *waited < DELAY_SECONDS {
            self.keep_alive.keep_awake_for(Duration::from_secs_f32(
                (DELAY_SECONDS - *waited).max(0.016),
            ));
            return;
        }

        let pointer = ui.pointer_position();
        let viewport = ui.viewport_size();
        let room_right = viewport.x - pointer.x - GAP - MARGIN;
        let room_left = pointer.x - GAP - MARGIN;

        let style = TextStyle {
            wrap: TextWrap::Words,
            ..self.style(TextRole::Small)
        };
        let shown = launcher_ui::privacy::scrub(text);
        // The widest line as it would be drawn unwrapped, and the box that takes.
        let natural = shown
            .lines()
            .map(|line| ui.measure_text(&style, line).x)
            .fold(0.0_f32, f32::max);
        let pad = f32::from(PAD_X) * 2.0;
        let wanted = (natural + pad + 2.0).min(MAX_WIDTH);
        // Right if the whole text fits there without wrapping; otherwise whichever side has more room (right on a tie).
        let on_right = room_right >= wanted || room_right >= room_left;
        let room = if on_right { room_right } else { room_left };
        let width = wanted.min(room.max(80.0)).max(40.0);

        // Height from last frame's layout of this very tooltip (it wraps, so it is not known before), to keep it inside
        // the window vertically.
        let tip_id = format!("{anchor_id}::tooltip");
        let height = ui.element_bounds(&tip_id).map_or(28.0, |(_, size)| size.y);
        let x = if on_right {
            pointer.x + GAP
        } else {
            pointer.x - GAP - width
        };
        let y =
            (pointer.y - height * 0.5).clamp(MARGIN, (viewport.y - height - MARGIN).max(MARGIN));

        let mut floating = Floating::new(FloatTarget::Root);
        floating.element_attach_point = AttachPoint::LeftTop;
        floating.parent_attach_point = AttachPoint::LeftTop;
        floating.offset = Vec2::new(x.max(0.0), y);
        floating.z_index = Z_TOOLTIP;
        floating.capture_pointer = false;
        {
            let _tip = ui.element(
                &ElementDesc::new()
                    .id(tip_id)
                    .width(Sizing::Fixed(width))
                    .height(Sizing::Fit)
                    .padding(Padding::symmetric(PAD_X, PAD_Y))
                    .background(self.palette.bg_dark)
                    .corner_radius(CornerRadius::all(6.0))
                    .border(Border::all(self.palette.border_muted, 1))
                    .direction(Direction::TopToBottom)
                    .floating(Some(floating)),
            );
            self::label(ui, text, &style);
        }
        // Follows the pointer while it moves over the anchor.
        self.keep_alive.keep_awake_for(Duration::from_millis(16));
    }

    // -- Settings rows --------------------------------------------------------------------------------------------

    /// Label (+ info tooltip) on the left, control on the right: the shape of every settings row.
    pub fn setting_row(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        label: &str,
        info: Option<&str>,
        control: impl FnOnce(&mut Self, &mut Ui<'_>),
    ) {
        let label_id = format!("{id}::label");
        let info_id = format!("{id}::info");
        {
            let _row = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fit)
                    .direction(Direction::LeftToRight)
                    .child_gap(SPACE_MD)
                    .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
            );
            {
                let _label = ui.element(
                    &ElementDesc::new()
                        .id(label_id.clone())
                        .width(Sizing::Grow)
                        .height(Sizing::Fit)
                        .direction(Direction::TopToBottom),
                );
                self.text(ui, TextRole::Body, label);
            }
            if info.is_some() {
                let _badge = ui.element(
                    &ElementDesc::new()
                        .id(info_id.clone())
                        .width(Sizing::Fixed(16.0))
                        .height(Sizing::Fixed(16.0))
                        .align(ChildAlignment::new(AlignX::Center, AlignY::Center)),
                );
                self.icon(ui, Icon::Info, 14.0, self.palette.text_muted);
            }
            control(self, ui);
        }
        if let Some(info) = info {
            self.tooltip(ui, &info_id, info);
            self.tooltip(ui, &label_id, info);
        }
    }

    pub fn toggle_setting(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        label: &str,
        info: Option<&str>,
        value: &mut bool,
    ) -> bool {
        self.toggle_setting_ex(ui, id, label, info, value, true)
    }

    /// [`Kit::toggle_setting`] that can be greyed out.
    pub fn toggle_setting_ex(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        label: &str,
        info: Option<&str>,
        value: &mut bool,
        enabled: bool,
    ) -> bool {
        let mut changed = false;
        self.setting_row(ui, id, label, info, |kit, ui| {
            if let Some(next) = kit.switch(ui, id, *value, enabled) {
                *value = next;
                changed = true;
            }
        });
        changed
    }

    #[allow(clippy::too_many_arguments)]
    pub fn slider_setting(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        label: &str,
        info: Option<&str>,
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        format: impl Fn(f64) -> String,
    ) -> Option<f64> {
        self.slider_setting_ex(ui, id, label, info, value, min, max, step, true, format)
    }

    /// [`Kit::slider_setting`] that can be greyed out.
    #[allow(clippy::too_many_arguments)]
    pub fn slider_setting_ex(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        label: &str,
        info: Option<&str>,
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        enabled: bool,
        format: impl Fn(f64) -> String,
    ) -> Option<f64> {
        let mut out = None;
        let (number, suffix) = split_number(&format(value));
        // What the box shows against what the slider holds: a percentage shows 50 for 0.5. The scale is read off the
        // formatter at the top of the range.
        let scale = split_number(&format(max))
            .0
            .parse::<f64>()
            .ok()
            .filter(|shown| max != 0.0 && shown.is_finite() && *shown != 0.0)
            .map_or(1.0, |shown| shown / max);
        let decimals = step <= 0.0 || step.fract() != 0.0 || number.contains('.');
        let negative = min < 0.0;
        let step_opt = (step > 0.0).then_some(step);
        let mut typed = None;
        let mut field = |kit: &mut Self, ui: &mut Ui<'_>| {
            if enabled {
                typed = kit.number_field(ui, &format!("{id}::value"), &number, 84.0, decimals, negative);
            } else {
                kit.text(ui, TextRole::Muted, &number);
            }
            if !suffix.is_empty() {
                kit.text(ui, TextRole::Muted, suffix.trim());
            }
        };
        // A dialog is too narrow for label, number box and a 260px track side by side: the label would be squeezed
        // to a sliver. There the track gets its own full-width line under the label and number.
        if let Some(width) = MODAL_WIDTH.get() {
            self.setting_row(ui, id, label, info, &mut field);
            // Inset by the thumb's radius so it is not cut off at the ends.
            out = self.slider_sized(ui, id, value, min, max, step_opt, enabled, (width - 12.0).max(120.0));
        } else {
            let mut dragged = None;
            self.setting_row(ui, id, label, info, |kit, ui| {
                field(kit, ui);
                dragged = kit.slider(ui, id, value, min, max, step_opt, enabled);
            });
            out = dragged;
        }
        if out.is_none()
            && let Some(entered) = typed
        {
            let mut next = (entered / scale).clamp(min, max);
            if let Some(step) = step_opt {
                next = (((next - min) / step).round() * step + min).clamp(min, max);
            }
            if (next - value).abs() > f64::EPSILON {
                out = Some(next);
            }
        }
        out
    }

    pub fn choice_setting<T: Copy + PartialEq>(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        label: &str,
        info: Option<&str>,
        current: T,
        all: &[T],
        label_of: impl Fn(T) -> String,
    ) -> Option<T> {
        let mut out = None;
        self.setting_row(ui, id, label, info, |kit, ui| {
            out = kit.choice(ui, id, current, all, label_of, 280.0);
        });
        out
    }

    pub fn text_setting(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        label: &str,
        info: Option<&str>,
        value: &mut String,
        placeholder: &str,
        masked: bool,
    ) -> bool {
        let mut changed = false;
        self.setting_row(ui, id, label, info, |kit, ui| {
            changed = kit.text_field(ui, id, value, placeholder, masked, Sizing::Fixed(320.0));
        });
        changed
    }

    /// A thin horizontal divider.
    pub fn separator(&self, ui: &mut Ui<'_>) {
        drop(
            ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fixed(1.0))
                    .background(self.palette.border_muted),
            ),
        );
    }

    /// A determinate (`Some`) or indeterminate (`None`) progress bar.
    pub fn progress(
        &mut self,
        ui: &mut Ui<'_>,
        id: &str,
        progress: Option<f32>,
        time_seconds: f64,
    ) {
        let style = ui_kit::ProgressStyle {
            track: self.palette.bg_light,
            fill: self.palette.primary,
            ..ui_kit::ProgressStyle::default()
        };
        let desc = ElementDesc::new()
            .id(id.into())
            .width(Sizing::Grow)
            .height(Sizing::Fixed(6.0));
        if let Some(next) = ui_kit::progress_bar(ui, &desc, progress, &style, time_seconds) {
            self.keep_alive.keep_awake_for(next * 4);
        }
    }

    /// Drops widget state for ids that start with `prefix` (a screen or dialog that closed).
    pub fn forget(&mut self, prefix: &str) {
        self.buttons.retain(|id, _| !id.starts_with(prefix));
        self.toggles.retain(|id, _| !id.starts_with(prefix));
        self.sliders.retain(|id, _| !id.starts_with(prefix));
        self.dropdowns.retain(|id, _| !id.starts_with(prefix));
        self.texts.retain(|id, _| !id.starts_with(prefix));
        self.areas.retain(|id, _| !id.starts_with(prefix));
        self.tooltips.retain(|id, _| !id.starts_with(prefix));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_selection_ranges_are_ordered_whichever_way_they_were_dragged() {
        let mut selection = LogSelection::default();
        assert_eq!(selection.range(), None);
        assert!(!selection.active());
        selection.anchor = Some((9, 2));
        selection.head = Some((4, 7));
        assert_eq!(selection.range(), Some(((4, 7), (9, 2))));
        assert!(selection.active());
        // Within one line the order is by position in the text.
        selection.anchor = Some((3, 10));
        selection.head = Some((3, 4));
        assert_eq!(selection.range(), Some(((3, 4), (3, 10))));
        // The same point twice selects nothing.
        selection.head = Some((3, 10));
        assert_eq!(selection.range(), None);
        selection.head = Some((4, 7));
        selection.clear();
        assert_eq!(selection.range(), None);
        assert!(!selection.active());
    }

    #[test]
    fn a_selection_being_dragged_counts_as_active_before_it_has_a_range() {
        let selection = LogSelection {
            dragging: true,
            ..LogSelection::default()
        };
        assert!(selection.active());
        assert_eq!(selection.range(), None);
    }
}
