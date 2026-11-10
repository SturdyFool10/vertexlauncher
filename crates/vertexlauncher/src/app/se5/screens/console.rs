//! Console: the launcher log and one tab per running instance, from `launcher_ui::console` (the same store the egui
//! console reads). Only the visible lines are built, so huge game logs stay cheap.


use vertex_i18n::tr;
use launcher_ui::console;
use launcher_ui::log_format::{self, LogLevel};
use vertex_shell::sturdy::MouseButton;
use vertex_shell::sturdy::glam::{Vec2, Vec4};
use vertex_shell::sturdy::ui::{
    AlignX, AlignY, AttachPoint, ChildAlignment, Clip, CornerRadius, Direction, ElementDesc, FloatTarget, Floating, Padding,
    Sector, Sizing, TextStyle, TextWrap, Ui,
};
use vertex_shell::sturdy::ui_kit;

use super::super::AppState;
use super::super::icons::Icon;
use super::super::kit::{self, ButtonKind, Kit, LogPoint, MenuEntry, TextRole};
use super::super::mc_text;

pub const LINES_ID: &str = "console::lines";

/// What one console tab remembers about how it is being read. Each console (Vertex's own, each game's) has its own,
/// kept in memory for as long as the tab exists, so following one never changes another.
struct TabView {
    /// Keep the newest line in view as lines arrive. A new console starts out following.
    follow: bool,
    /// Following has caught up with the bottom since it was enabled. Until then, being scrolled up is not the user
    /// leaving: they enabled follow from there, and the next line moves the view down.
    engaged: bool,
    last_line_count: usize,
}

impl Default for TabView {
    fn default() -> Self {
        Self {
            follow: true,
            engaged: false,
            last_line_count: 0,
        }
    }
}

#[derive(Default)]
pub struct ConsoleView {
    tabs: std::collections::HashMap<String, TabView>,
    /// The log element of the tab on screen, for the controller's scroll stick.
    active_lines_id: String,
}

impl ConsoleView {
    /// The element id of the log being shown.
    pub fn active_lines_id(&self) -> &str {
        if self.active_lines_id.is_empty() {
            LINES_ID
        } else {
            &self.active_lines_id
        }
    }
}

pub fn render(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let snapshot = console::snapshot();
    // Closed consoles forget their state.
    state
        .console
        .tabs
        .retain(|id, _| snapshot.tabs.iter().any(|tab| tab.id == *id));
    let tab_id = snapshot.active_tab_id.clone();
    // Each console is its own log element, so its scroll position and text selection are its own too.
    let lines_id = format!("{LINES_ID}::{tab_id}");
    let shown_level = if console::is_default_tab(&tab_id) {
        state.config.console_log_level()
    } else {
        state.config.game_console_log_level()
    };
    let min_level = match shown_level {
        config::ConsoleLogLevel::Error => log_format::LogLevel::Error,
        config::ConsoleLogLevel::Warn => log_format::LogLevel::Warn,
        config::ConsoleLogLevel::Info => log_format::LogLevel::Info,
        config::ConsoleLogLevel::Debug => log_format::LogLevel::Debug,
        config::ConsoleLogLevel::Trace => log_format::LogLevel::Trace,
    };
    state.console.active_lines_id = lines_id.clone();
    let _page = ui.element(
        &ElementDesc::new()
            .width(Sizing::Grow)
            .height(Sizing::Grow)
            .direction(Direction::TopToBottom)
            .child_gap(kit::SPACE_MD)
            .padding(Padding::all(kit::SPACE_XL)),
    );
    {
        let _header = kit.row(ui, kit::SPACE_SM);
        kit.fill(ui);
        let tab = state.console.tabs.entry(tab_id.clone()).or_default();
        let mut follow = tab.follow;
        kit.text(ui, TextRole::Muted, &tr!("Follow output"));
        if let Some(next) = kit.switch(ui, "console::follow", follow, true) {
            follow = next;
        }
        tab.follow = follow;
        if kit.button(
            ui,
            "console::copy",
            &tr!("Copy all"),
            ButtonKind::Neutral,
            !snapshot.active_lines.is_empty(),
        ) {
            let text = snapshot
                .active_lines
                .iter()
                .map(|line| log_format::plain_text(line))
                .collect::<Vec<_>>()
                .join("\n");
            let _ = ui
                .engine()
                .primary_window_control()
                .map(|mut window| window.set_clipboard_text(&text));
            launcher_ui::notification::info!("console", "Copied {count} console lines.", count = snapshot.active_lines.len());
        }
    }
    {
        let _tabs = kit.row(ui, kit::SPACE_XS);
        for tab in &snapshot.tabs {
            let id = format!("console::tab::{}", tab.id);
            let selected = tab.id == snapshot.active_tab_id;
            let background = if selected {
                kit.palette.primary.with_w(0.28)
            } else if ui.hovered(&id) {
                kit.palette.hovered_fill()
            } else {
                kit.palette.bg_light
            };
            {
                let _tab = ui.element(
                    &ElementDesc::new()
                        .id(id.clone())
                        .width(Sizing::Fit)
                        .height(Sizing::Fixed(30.0))
                        .padding(Padding::symmetric(kit::SPACE_MD, 0))
                        .child_gap(kit::SPACE_SM)
                        .align(ChildAlignment::new(AlignX::Left, AlignY::Center))
                        .background(background)
                        .corner_radius(CornerRadius::all(8.0)),
                );
                kit.text_clipped(ui, TextRole::Body, &tab.label, 220.0);
                if tab.can_close {
                    let close = format!("{id}::close");
                    {
                        let _close = ui.element(
                            &ElementDesc::new()
                                .id(close.clone())
                                .width(Sizing::Fixed(18.0))
                                .height(Sizing::Fixed(18.0))
                                .align(ChildAlignment::center()),
                        );
                        kit.text(ui, TextRole::Small, "✕");
                    }
                    if kit.clicked_plain(ui, &close) {
                        console::close_tab(&tab.id);
                        continue;
                    }
                }
            }
            if kit.clicked(ui, &id) {
                console::set_active_tab(&tab.id);
            }
        }
    }

    // Shown with repeats merged, a frame at a time; "Copy all" above still copies every line.
    let merged = kit.merged_log(&lines_id, &snapshot.active_lines, snapshot.first_line_index);
    let shown_lines = kit.leveled_log(&lines_id, &merged, min_level);
    let tab = state.console.tabs.entry(tab_id).or_default();
    let follow = tab.follow;
    let (count, left_bottom, at_bottom) = log_lines_view(
        ui,
        kit,
        &lines_id,
        &shown_lines,
        &tr!("Nothing logged yet."),
        follow,
        tab.engaged,
        tab.last_line_count,
    );
    tab.last_line_count = count;
    tab.engaged = follow && (tab.engaged || at_bottom);
    if follow && left_bottom {
        // The user scrolled up to read something; following would drag the view back down under them.
        tab.follow = false;
    }
}

/// A virtualised, colour-coded log viewer: only the visible lines are built. Lines are coloured by their ANSI codes
/// or, failing that, their severity. Returns the number of lines shown.
pub fn log_lines(ui: &mut Ui<'_>, kit: &mut Kit, id: &str, lines: &[String], empty_text: &str) {
    let _ = log_lines_view(ui, kit, id, lines, empty_text, false, false, lines.len());
}

/// Returns the number of lines shown, whether the view left the bottom while following (only once `engaged`), and
/// whether the view currently shows the bottom.
fn log_lines_view(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    id: &str,
    lines: &[String],
    empty_text: &str,
    follow: bool,
    engaged: bool,
    last_count: usize,
) -> (usize, bool, bool) {
    let line_style = TextStyle {
        wrap: TextWrap::None,
        ..kit.style(TextRole::Body)
    };
    let row_height = (f32::from(line_style.font_size) * 1.45).ceil();
    let levels = kit.log_levels(id, lines);
    let mut selection = kit.log_selections.remove(id).unwrap_or_default();
    // Keep the selection inside the log when lines are cleared.
    if selection.range().is_some_and(|(_, end)| end.0 >= lines.len()) {
        selection.clear();
    }
    let selected = selection.range();
    // The line under the pointer and the byte offset in it the pointer is at; likewise for a press.
    let mut hovered_point: Option<LogPoint> = None;
    let mut pressed_point: Option<LogPoint> = None;
    let selection_color = kit.palette.primary.with_w(0.40);
    // The horizontal extent of the selection on a line, in pixels from the line's left edge: it ends at the text it
    // covers, and a line the selection runs on from reaches a little past its end, as a newline would.
    let extent = |ui: &Ui<'_>, index: usize| -> Option<(f32, f32)> {
        let (start, end) = selected?;
        if index < start.0 || index > end.0 || index >= lines.len() {
            return None;
        }
        let plain = log_format::plain_text(&lines[index]);
        let from = (if index == start.0 { start.1 } else { 0 }).min(plain.len());
        let to = (if index == end.0 { end.1 } else { plain.len() }).min(plain.len());
        let at = |upto: usize| ui.measure_text(&line_style, plain.get(..upto).unwrap_or_default()).x;
        let x0 = at(from);
        let mut x1 = at(to);
        if index < end.0 && to >= plain.len() {
            x1 += 6.0;
        }
        // Whole pixels: the pieces of the selection meet along shared edges, and an edge inside a pixel would be
        // painted by two pieces that each cover part of it, which shows as a faint line.
        let (x0, x1) = (x0.round(), x1.round());
        (x1 > x0).then_some((x0, x1))
    };
    let mut menu_request: Option<(usize, Vec2)> = None;
    {
        // The engine clips to a rectangle, never to rounded corners, so text that scrolls past the edge would show in the
        // corner squares outside the rounded shape. The rounded background is therefore its own element, and the clipped
        // list sits inside it, inset so its corners fall within the arc (see `kit::rounded_clip_inset`). The list's own
        // padding is reduced by the same amount, so where the text sits does not change.
        let inset = kit::rounded_clip_inset(kit::CORNER);
        let _frame = ui.element(
            &ElementDesc::new()
                .width(Sizing::Grow)
                .height(Sizing::Grow)
                .padding(Padding::all(inset))
                .background(kit.palette.bg_dark)
                .corner_radius(CornerRadius::all(kit::CORNER)),
        );
        let _list = ui.element(
            &ElementDesc::new()
                .id(id.into())
                .width(Sizing::Grow)
                .height(Sizing::Grow)
                .direction(Direction::TopToBottom)
                .padding(Padding::symmetric(
                    kit::SPACE_MD.saturating_sub(inset),
                    kit::SPACE_SM.saturating_sub(inset),
                ))
                .clip(Clip::both()),
        );
        kit.scrollbar(ui, id);
        // Log lines are selectable text, so the pointer is the text cursor over them, as over text in a web page.
        kit.cursor_target(id, kit::CursorKind::Text);
        if lines.is_empty() {
            kit.text(ui, TextRole::Muted, empty_text);
        }
        let visible = ui_kit::visible_rows(ui, id, row_height, lines.len(), 8);
        ui_kit::spacer(ui, visible.before);
        for index in visible.rows.clone() {
            let level = levels.get(index).copied().flatten();
            let default_color = match level {
                Some(LogLevel::Fatal | LogLevel::Error) => kit.palette.danger,
                Some(LogLevel::Warn) => kit.palette.warning,
                Some(LogLevel::Info) => kit.palette.info,
                Some(LogLevel::Debug | LogLevel::Trace) => kit.palette.text_muted,
                None => kit.palette.text,
            };
            let row_id = format!("{id}::row::{index}");
            {
                let _row = ui.element(
                    &ElementDesc::new()
                        .id(row_id.clone())
                        .width(Sizing::Grow)
                        .height(Sizing::Fixed(row_height))
                        .direction(Direction::LeftToRight)
                        .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
                );
                // ANSI colours and Minecraft section-sign codes together; a run with no colour (and `§r`) is the line's
                // own colour, which is the level's.
                let runs = log_format::styled_runs(&lines[index]);
                // Whole runs, never split at the selection: cutting the text there would cut a ligature in two.
                mc_text::draw(
                    ui,
                    kit,
                    &row_id,
                    &runs,
                    &TextStyle {
                        color: default_color,
                        ..line_style
                    },
                );
            }
            // Where in the line the pointer is: the text's own width decides which character it is between.
            let point_in_line = |ui: &Ui<'_>| {
                let origin = ui.element_bounds(&row_id).map_or(0.0, |(position, _)| position.x);
                let plain = log_format::plain_text(&lines[index]);
                let at = ui.hit_test_text(&line_style, &plain, ui.pointer_position().x - origin);
                (index, at.min(plain.len()))
            };
            if ui.hovered(&row_id) {
                hovered_point = Some(point_in_line(ui));
            }
            if ui_kit::pressed(ui, &row_id, MouseButton::Left) {
                pressed_point = Some(point_in_line(ui));
            }
            if let Some(at) = kit.secondary(ui, &row_id) {
                menu_request = Some((index, at));
            }
        }
        // The selection is one shape across the lines it covers, not one block per line: blocks stacked along a shared
        // edge show a seam there.
        let extents: Vec<Option<(f32, f32)>> =
            visible.rows.clone().map(|index| extent(ui, index)).collect();
        if extents.iter().any(Option::is_some) {
            let anchor = format!("{id}::row::{}", visible.rows.start);
            selection_overlay(ui, &anchor, &extents, row_height, selection_color);
        }
        ui_kit::spacer(ui, visible.after);
    }

    // Drag across text to select it; a plain click clears the selection.
    if let Some(point) = pressed_point {
        selection.anchor = Some(point);
        selection.head = Some(point);
        selection.dragging = true;
        selection.moved = false;
    }
    if selection.dragging {
        if ui.pointer_down() {
            if let Some(point) = hovered_point
                && selection.head != Some(point)
            {
                selection.head = Some(point);
                selection.moved = true;
            }
        } else {
            selection.dragging = false;
            if !selection.moved {
                selection.clear();
            }
        }
    }
    let menu_id = format!("{id}::menu");
    if let Some((line, at)) = menu_request {
        selection.menu_line = line;
        kit.open_context_menu(&menu_id, &mut selection.menu, at);
    }
    let has_selection = selection.range().is_some();
    let items: Vec<MenuEntry> = if has_selection {
        vec![
            MenuEntry::new(tr!("Copy selection")).icon(Icon::Copy),
            MenuEntry::new(tr!("Clear selection")).icon(Icon::Close),
        ]
    } else {
        vec![MenuEntry::new(tr!("Copy line")).icon(Icon::Copy)]
    };
    if let Some(choice) = kit.context_menu(ui, &menu_id, &mut selection.menu, &items) {
        // Copies the text from `start` to `end`, partial first and last lines included.
        let copy = |ui: &mut Ui<'_>, start: LogPoint, end: LogPoint, what: &str| {
            let last = lines.len().saturating_sub(1);
            let text = (start.0.min(last)..=end.0.min(last))
                .map(|index| {
                    let plain = log_format::plain_text(&lines[index]);
                    let from = if index == start.0 { start.1.min(plain.len()) } else { 0 };
                    let to = if index == end.0 { end.1.min(plain.len()) } else { plain.len() };
                    plain.get(from..to).unwrap_or_default().to_owned()
                })
                .collect::<Vec<_>>()
                .join("\n");
            let _ = ui
                .engine()
                .primary_window_control()
                .map(|mut window| window.set_clipboard_text(&text));
            launcher_ui::notification::info!("console", "Copied {what}.", what = what);
        };
        match (has_selection, choice, selection.range()) {
            (true, 0, Some((start, end))) if !lines.is_empty() => {
                copy(ui, start, end, &tr!("the selection"))
            }
            (true, _, _) => selection.clear(),
            (false, _, _) if !lines.is_empty() => {
                let line = selection.menu_line.min(lines.len() - 1);
                copy(ui, (line, 0), (line, usize::MAX), &tr!("the line"));
            }
            _ => {}
        }
    }
    let selecting = selection.active();
    // Following means the last line is on screen. The user leaves it by an action: the wheel scrolling up over the log,
    // or dragging its scrollbar. A view that is merely short of the bottom is a jump that fell behind the log growing
    // (the engine clamps a jump to the size the log had a frame ago), so it jumps again instead of giving up.
    let at_bottom = ui.scroll_metrics(id).is_none_or(|m| {
        m.content_size.y <= m.container_size.y + 1.0
            || (-m.offset.y) + m.container_size.y >= m.content_size.y - 2.0
    });
    let wheel_up = ui.hovered(id) && ui.engine().input().wheel_delta().y > 0.0;
    let user_left = wheel_up || kit.scrollbar_dragging(id);
    selection.settle_frames = selection.settle_frames.saturating_sub(1);
    let fell_behind = follow && engaged && !at_bottom && !user_left && selection.settle_frames == 0;
    let left_bottom = follow && engaged && user_left;
    let mut pending_scroll = selection.pending_scroll;
    let mut settle_frames = selection.settle_frames;
    // Following the newest line would scroll the selection away, so it pauses while one exists.
    if follow && !selecting && (lines.len() != last_count || pending_scroll || fell_behind) {
        // Scroll offsets are negative when scrolled down; a huge value clamps to the end. The engine refuses while the
        // log element has not been through a frame yet (a console just opened), so the jump is retried until it takes.
        let jumped = ui.set_scroll_offset(id, Vec2::new(0.0, -1.0e9));
        pending_scroll = !jumped;
        settle_frames = 6;
    }
    selection.pending_scroll = pending_scroll;
    selection.settle_frames = settle_frames;
    kit.log_selections.insert(id.to_owned(), selection);
    (lines.len(), left_bottom, at_bottom)
}


/// Radius of the selection's rounded corners, in pixels.
const SELECTION_RADIUS: f32 = 6.0;

/// Draws the selection over the lines whose extents are `extents` (`None` where a line is not selected), the first
/// being the row `anchor`, each `height` tall.
///
/// The shape is cut into vertical strips, one per stretch between two line ends, each a single rectangle down every
/// run of lines that reaches across it. Nothing is painted twice and no two rectangles share a horizontal edge, so the
/// translucent colour shows no seam or doubled band. Corners are rounded as the sign-in QR code rounds its merged
/// shapes: a corner that is an outer (convex) corner of the whole shape is rounded, one that continues into a
/// neighbouring rectangle is left alone, and where two rectangles meet to leave a notch the notch is filled with a
/// rounded inner (concave) corner.
fn selection_overlay(
    ui: &mut Ui<'_>,
    anchor: &str,
    extents: &[Option<(f32, f32)>],
    height: f32,
    color: Vec4,
) {
    use std::collections::BTreeSet;
    use std::f32::consts::{FRAC_PI_2, PI};
    const PROBE: f32 = 0.25;
    let rows = extents.len();
    // Whether the selection covers the point (a point on an edge counts as outside).
    let covered = |x: f32, y: f32| -> bool {
        let row = (y / height).floor();
        row >= 0.0
            && (row as usize) < rows
            && extents[row as usize].is_some_and(|(from, to)| x > from && x < to)
    };
    let float = |x: f32, y: f32| {
        Some(Floating {
            element_attach_point: AttachPoint::LeftTop,
            parent_attach_point: AttachPoint::LeftTop,
            offset: Vec2::new(x, y),
            z_index: kit::in_modal_layer(1),
            capture_pointer: false,
            clip_to_target: true,
            ..Floating::new(FloatTarget::Element(anchor.to_owned()))
        })
    };

    // Every piece has its own id: the layout engine rejects two elements with the same id in a frame, and the ones it
    // makes up for anonymous elements are only as unique as their position.
    let mut piece = 0_u32;
    let mut piece_id = |kind: &str| {
        piece += 1;
        format!("{anchor}::selection::{kind}::{piece}")
    };

    let mut edges: Vec<i32> = extents
        .iter()
        .flatten()
        .flat_map(|(from, to)| [*from as i32, *to as i32])
        .collect();
    edges.sort_unstable();
    edges.dedup();

    // Notches to fill: the corner point and which way the notch lies from it.
    let mut notches: BTreeSet<(i32, i32, i8, i8)> = BTreeSet::new();
    for pair in edges.windows(2) {
        let (left, right) = (pair[0] as f32, pair[1] as f32);
        let mut run_start = None;
        for line in 0..=rows {
            let reaches = line < rows
                && extents[line].is_some_and(|(from, to)| from <= left && to >= right);
            match (reaches, run_start) {
                (true, None) => run_start = Some(line),
                (false, Some(start)) => {
                    run_start = None;
                    let (top, bottom) = (start as f32 * height, line as f32 * height);
                    let (width, tall) = (right - left, bottom - top);
                    let round = SELECTION_RADIUS.min(width / 2.0).min(tall / 2.0);
                    // Each corner, with the direction its rectangle lies in from it.
                    let mut radii = [0.0_f32; 4];
                    for (slot, (px, py, sx, sy)) in [
                        (left, top, 1.0, 1.0),
                        (right, top, -1.0, 1.0),
                        (left, bottom, 1.0, -1.0),
                        (right, bottom, -1.0, -1.0),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        // The cells across each edge at this corner, and diagonally across it.
                        let across_vertical = covered(px - sx * PROBE, py + sy * PROBE);
                        let across_horizontal = covered(px + sx * PROBE, py - sy * PROBE);
                        let diagonal = covered(px - sx * PROBE, py - sy * PROBE);
                        if !across_vertical && !across_horizontal {
                            radii[slot] = round;
                        } else if [across_vertical, across_horizontal, diagonal]
                            .iter()
                            .filter(|covered| **covered)
                            .count()
                            == 2
                        {
                            // Three of the four cells around this point are covered (this rectangle and two of the
                            // others): an inner corner, with the notch in the fourth. Which of them is empty says
                            // which way the notch lies; every rectangle meeting here finds the same one.
                            let notch = if !across_vertical {
                                (-sx, sy)
                            } else if !across_horizontal {
                                (sx, -sy)
                            } else {
                                (-sx, -sy)
                            };
                            notches.insert((px as i32, py as i32, notch.0 as i8, notch.1 as i8));
                        }
                    }
                    drop(ui.element(
                        &ElementDesc::new()
                            .id(piece_id("rect"))
                            .width(Sizing::Fixed(width))
                            .height(Sizing::Fixed(tall))
                            .background(color)
                            .corner_radius(CornerRadius {
                                top_left: radii[0],
                                top_right: radii[1],
                                bottom_left: radii[2],
                                bottom_right: radii[3],
                            })
                            .floating(float(left, top)),
                    ));
                }
                _ => {}
            }
        }
    }

    // The inner corners: a square filling the notch with a quarter circle cut out of it, drawn as a ring segment inside
    // a parent that clips to the square so nothing is painted outside it.
    for (px, py, dx, dy) in notches {
        let (px, py) = (px as f32, py as f32);
        let (dx, dy) = (f32::from(dx), f32::from(dy));
        // As large as fits in the empty space, up to the usual radius.
        let Some(r) = (1..=SELECTION_RADIUS as i32).rev().map(|r| r as f32).find(|r| {
            !covered(px + dx * r * 0.5, py + dy * r * 0.5)
                && !covered(px + dx * (r - PROBE), py + dy * (r - PROBE))
        }) else {
            continue;
        };
        let origin = Vec2::new(px.min(px + dx * r), py.min(py + dy * r));
        // The circle is centred on the square's corner farthest from the notch's tip.
        let center = Vec2::new(px + dx * r, py + dy * r) - origin;
        // A corner on top of its vertex (the notch above it) paints a row of pixels that the rectangle below also
        // paints, so the square sits one pixel higher. The circle moves with it.
        let origin = if dy < 0.0 { origin - Vec2::new(0.0, 1.0) } else { origin };
        let from = match (dx < 0.0, dy < 0.0) {
            (true, true) => 0.0,
            (true, false) => -FRAC_PI_2,
            (false, true) => FRAC_PI_2,
            (false, false) => PI,
        };
        let square_id = piece_id("notch");
        let _square = ui.element(
            &ElementDesc::new()
                .id(square_id.clone())
                .width(Sizing::Fixed(r))
                .height(Sizing::Fixed(r))
                .clip(Clip::both())
                .floating(float(origin.x, origin.y)),
        );
        ui.fill_sectors(
            &ElementDesc::new()
                .id(format!("{square_id}::fill"))
                .width(Sizing::Fixed(r))
                .height(Sizing::Fixed(r)),
            &[Sector {
                center,
                inner_radius: r,
                outer_radius: r * 1.5,
                start_angle: from,
                end_angle: from + FRAC_PI_2,
                color,
                feather_px: 0.75,
            }],
        );
    }
}
