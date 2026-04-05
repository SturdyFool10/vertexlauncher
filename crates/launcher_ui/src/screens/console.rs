use std::hash::Hash;

use egui::{Color32, CornerRadius, Margin, Ui, pos2, vec2};
use textui::{ButtonOptions, RichTextSpan, RichTextStyle, TextUi};

use crate::{
    assets, console,
    ui::{context_menu, style},
};

const ACTION_COPY_SELECTION: &str = "copy_selection";
const FORCE_CONSOLE_TAB_FOCUS_KEY: &str = "console_force_tab_focus";
const CONSOLE_LOG_SCROLL_ID_KEY: &str = "console_log_scroll_area_id";

pub fn request_console_tab_focus(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(FORCE_CONSOLE_TAB_FOCUS_KEY), true));
}

fn take_console_tab_focus_request(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| {
        let key = egui::Id::new(FORCE_CONSOLE_TAB_FOCUS_KEY);
        let v = d.get_temp::<bool>(key).unwrap_or(false);
        if v {
            d.remove::<bool>(key);
        }
        v
    })
}

pub fn console_log_scroll_id(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|d| d.get_temp::<egui::Id>(egui::Id::new(CONSOLE_LOG_SCROLL_ID_KEY)))
}
const ACTION_CLEAR_SELECTION: &str = "clear_selection";
const ACTION_COPY_LINE: &str = "copy_line";
const LOG_SELECTION_AUTOSCROLL_MARGIN: f32 = 32.0;
const LOG_SELECTION_AUTOSCROLL_MAX_SPEED: f32 = 1100.0;

fn log_console_context_menu(message: impl AsRef<str>) {
    eprintln!("[console_context_menu] {}", message.as_ref());
}

fn edge_autoscroll_speed(pointer_pos: f32, min: f32, max: f32, margin: f32) -> f32 {
    if margin <= 0.0 {
        return 0.0;
    }

    let normalized = if pointer_pos < min + margin {
        ((min + margin - pointer_pos) / margin).clamp(0.0, 1.0)
    } else if pointer_pos > max - margin {
        -((pointer_pos - (max - margin)) / margin).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // Smoothstep keeps the ramp gentle near the threshold and stronger at the edge.
    let magnitude = normalized.abs();
    let eased = magnitude * magnitude * (3.0 - 2.0 * magnitude);
    normalized.signum() * eased * LOG_SELECTION_AUTOSCROLL_MAX_SPEED
}

pub fn render(ui: &mut Ui, text_ui: &mut TextUi) {
    let snapshot = console::snapshot();
    let lines = &snapshot.active_lines;
    let viewport_size = vec2(
        ui.available_width().max(1.0),
        ui.available_height().max(1.0),
    );
    ui.allocate_ui_with_layout(
        viewport_size,
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            ui.add_space(style::SPACE_XL);
            let inner_width = (ui.available_width() - style::SPACE_XL * 2.0).max(1.0);
            ui.allocate_ui_with_layout(
                vec2(inner_width, ui.available_height().max(1.0)),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    render_tabs_row(ui, text_ui, &snapshot);
                    ui.add_space(style::SPACE_MD);
                    egui::Frame::new()
                        .fill(ui.visuals().widgets.noninteractive.bg_fill)
                        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                        .corner_radius(CornerRadius::same(style::CORNER_RADIUS_MD))
                        .inner_margin(Margin::same(style::SPACE_MD as i8))
                        .show(ui, |ui| {
                            render_log_buffer(
                                ui,
                                text_ui,
                                "console_scroll_area",
                                lines,
                                "No log entries yet.",
                                true,
                                snapshot.text_redraw_generation,
                            );
                        });
                },
            );
            ui.add_space(style::SPACE_XL);
        },
    );
}

pub(crate) fn render_log_buffer(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    id_source: impl Hash,
    lines: &[String],
    empty_message: &str,
    stick_to_bottom: bool,
    _text_redraw_generation: u64,
) {
    let viewport_height = ui.available_height().max(1.0);
    let text_base_id = ui.make_persistent_id((&id_source, "text"));

    ui.set_min_height(viewport_height);

    if lines.is_empty() {
        let mut empty_style = style::muted(ui);
        empty_style.wrap = false;
        let _ = text_ui.label(ui, (text_base_id, "empty"), empty_message, &empty_style);
        let _ = ui.allocate_exact_size(
            vec2(1.0, (viewport_height - 24.0).max(1.0)),
            egui::Sense::hover(),
        );
        return;
    }

    render_virtualized_log_lines(ui, text_ui, text_base_id, lines, stick_to_bottom);
}

#[derive(Clone, Debug, Default)]
struct VirtualLogViewerState {
    max_line_width: f32,
    follow_bottom: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct LogSelectionCursor {
    line: usize,
    char_index: usize,
}

#[derive(Clone, Debug, Default)]
struct LogSelectionState {
    anchor: Option<LogSelectionCursor>,
    head: Option<LogSelectionCursor>,
    dragging: bool,
}

impl LogSelectionState {
    fn normalized(&self) -> Option<(LogSelectionCursor, LogSelectionCursor)> {
        let anchor = self.anchor?;
        let head = self.head.unwrap_or(anchor);
        if anchor <= head {
            Some((anchor, head))
        } else {
            Some((head, anchor))
        }
    }

    fn has_selection(&self) -> bool {
        matches!(self.normalized(), Some((start, end)) if start != end)
    }

    fn clear(&mut self) {
        self.anchor = None;
        self.head = None;
        self.dragging = false;
    }
}

#[derive(Clone)]
struct VisibleLogRowHit {
    line_index: usize,
    rect: egui::Rect,
    text_rect: egui::Rect,
    galley: std::sync::Arc<egui::Galley>,
    line_len_chars: usize,
}

fn virtual_log_line_options(ui: &Ui, level: Option<LogLevel>) -> textui::LabelOptions {
    let mut options = style::body(ui);
    options.wrap = false;
    options.color = color_for_level(ui, level);
    options.weight = if matches!(level, Some(LogLevel::Error | LogLevel::Fatal)) {
        700
    } else {
        400
    };
    options
}

fn warm_log_parse_context(lines: &[String], first_visible_line: usize) -> LogParseContext {
    const LOOKBACK_LINES: usize = 64;

    let start = first_visible_line.saturating_sub(LOOKBACK_LINES);
    let mut context = LogParseContext::default();

    for line in &lines[start..first_visible_line] {
        let _ = resolve_log_level(line, &mut context);
    }

    context
}

fn log_char_count(text: &str) -> usize {
    text.chars().count()
}

fn log_char_to_byte_index(text: &str, char_index: usize) -> usize {
    if char_index == 0 {
        return 0;
    }

    text.char_indices()
        .nth(char_index)
        .map(|(byte_index, _)| byte_index)
        .unwrap_or(text.len())
}

fn slice_log_chars(text: &str, start: usize, end: usize) -> &str {
    let start = start.min(log_char_count(text));
    let end = end.min(log_char_count(text));
    let start_byte = log_char_to_byte_index(text, start);
    let end_byte = log_char_to_byte_index(text, end);
    &text[start_byte..end_byte]
}

fn selected_log_text(lines: &[String], selection: &LogSelectionState) -> Option<String> {
    let (start, end) = selection.normalized()?;
    if start == end {
        return None;
    }

    let mut out = String::new();

    for line_index in start.line..=end.line {
        let line = lines.get(line_index)?;
        let line_chars = log_char_count(line);

        let from = if line_index == start.line {
            start.char_index.min(line_chars)
        } else {
            0
        };

        let to = if line_index == end.line {
            end.char_index.min(line_chars)
        } else {
            line_chars
        };

        if to > from {
            out.push_str(slice_log_chars(line, from, to));
        }

        if line_index != end.line {
            out.push('\n');
        }
    }

    if out.is_empty() { None } else { Some(out) }
}

fn selection_fill_color(ui: &Ui) -> egui::Color32 {
    ui.visuals().selection.bg_fill.linear_multiply(0.55)
}

fn galley_font_id(options: &textui::LabelOptions) -> egui::FontId {
    if options.monospace {
        egui::FontId::monospace(options.font_size)
    } else {
        egui::FontId::proportional(options.font_size)
    }
}

fn row_contains_text(row: &VisibleLogRowHit, pointer_pos: egui::Pos2) -> bool {
    row.line_len_chars > 0 && row.text_rect.contains(pointer_pos)
}

fn clamp_log_cursor_to_row(row: &VisibleLogRowHit, pointer_pos: egui::Pos2) -> LogSelectionCursor {
    let local_x = (pointer_pos.x - row.rect.min.x).clamp(0.0, row.rect.width().max(0.0));
    let local_y = (pointer_pos.y - row.rect.min.y).clamp(0.0, row.rect.height().max(0.0));
    let cursor = row.galley.cursor_from_pos(vec2(local_x, local_y));
    LogSelectionCursor {
        line: row.line_index,
        char_index: cursor.index.min(row.line_len_chars),
    }
}

fn cursor_from_visible_rows(
    rows: &[VisibleLogRowHit],
    pointer_pos: egui::Pos2,
) -> Option<LogSelectionCursor> {
    let first = rows.first()?;
    let last = rows.last()?;

    if let Some(row) = rows.iter().find(|row| row.rect.contains(pointer_pos)) {
        return Some(clamp_log_cursor_to_row(row, pointer_pos));
    }

    if pointer_pos.y <= first.rect.min.y {
        return Some(clamp_log_cursor_to_row(
            first,
            pos2(pointer_pos.x, first.rect.center().y),
        ));
    }

    if pointer_pos.y >= last.rect.max.y {
        return Some(clamp_log_cursor_to_row(
            last,
            pos2(pointer_pos.x, last.rect.center().y),
        ));
    }

    let nearest = rows.iter().min_by(|a, b| {
        let da = (a.rect.center().y - pointer_pos.y).abs();
        let db = (b.rect.center().y - pointer_pos.y).abs();
        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
    })?;

    Some(clamp_log_cursor_to_row(nearest, pointer_pos))
}

fn paint_log_selection_for_line(
    ui: &Ui,
    rect: egui::Rect,
    galley: &egui::Galley,
    line_text: &str,
    line_index: usize,
    selection: &LogSelectionState,
) {
    let Some((start, end)) = selection.normalized() else {
        return;
    };

    if line_index < start.line || line_index > end.line {
        return;
    }

    let line_chars = log_char_count(line_text);
    let from = if line_index == start.line {
        start.char_index.min(line_chars)
    } else {
        0
    };
    let to = if line_index == end.line {
        end.char_index.min(line_chars)
    } else {
        line_chars
    };

    if to <= from {
        return;
    }

    let start_cursor = egui::text::CCursor::new(from);
    let end_cursor = egui::text::CCursor::new(to);
    let start_pos = galley.pos_from_cursor(start_cursor);
    let end_pos = galley.pos_from_cursor(end_cursor);

    let min_x = rect.min.x + start_pos.min.x.min(end_pos.min.x);
    let max_x = rect.min.x + start_pos.max.x.max(end_pos.max.x);

    let selection_rect = egui::Rect::from_min_max(
        pos2(min_x, rect.min.y),
        pos2(max_x.max(min_x + 1.0), rect.max.y),
    );

    ui.painter()
        .rect_filled(selection_rect, 0.0, selection_fill_color(ui));
}

fn render_virtualized_log_lines(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    text_base_id: egui::Id,
    lines: &[String],
    stick_to_bottom: bool,
) {
    let body_style = style::body(ui);
    let row_height = body_style.line_height.max(1.0);
    let state_id = ui.make_persistent_id((text_base_id, "virtual_log_state"));
    let selection_id = ui.make_persistent_id((text_base_id, "virtual_log_selection"));
    let viewport_id = ui.make_persistent_id((text_base_id, "virtual_log_viewport"));
    let scroll_id = ui.make_persistent_id((text_base_id, "virtual_log_scroll"));
    let menu_source_id = ui.make_persistent_id((text_base_id, "context_menu_source"));
    let menu_line_id = ui.make_persistent_id((text_base_id, "context_menu_line"));

    let mut viewer_state = ui.ctx().data_mut(|data| {
        data.get_temp::<VirtualLogViewerState>(state_id)
            .unwrap_or_default()
    });
    let mut selection_state = ui.ctx().data_mut(|data| {
        data.get_temp::<LogSelectionState>(selection_id)
            .unwrap_or_default()
    });

    if let Some(action) = context_menu::take_invocation(ui.ctx(), menu_source_id) {
        log_console_context_menu(format!("received invocation action={}", action));
        match action.as_str() {
            ACTION_COPY_SELECTION => {
                if let Some(text) = selected_log_text(lines, &selection_state) {
                    ui.ctx().copy_text(text);
                }
            }
            ACTION_CLEAR_SELECTION => {
                selection_state.clear();
            }
            ACTION_COPY_LINE => {
                let maybe_line_index = ui
                    .ctx()
                    .data_mut(|data| data.get_temp::<Option<usize>>(menu_line_id))
                    .flatten();
                if let Some(line_index) = maybe_line_index {
                    if let Some(line) = lines.get(line_index) {
                        ui.ctx().copy_text(line.clone());
                    }
                }
            }
            _ => {}
        }
    }

    let mut clear_selection = false;
    let mut viewport_has_focus = false;
    let selection_active = selection_state.dragging || selection_state.has_selection();
    let previous_scroll_state =
        egui::scroll_area::State::load(ui.ctx(), scroll_id).unwrap_or_default();
    if !stick_to_bottom {
        viewer_state.follow_bottom = false;
    } else if viewer_state.follow_bottom == false
        && previous_scroll_state.offset == egui::Vec2::ZERO
    {
        viewer_state.follow_bottom = true;
    }
    let allow_stick_to_bottom = stick_to_bottom && viewer_state.follow_bottom && !selection_active;

    let scroll_output = egui::ScrollArea::both()
        .id_salt((text_base_id, "virtual_log_scroll"))
        .auto_shrink([false, false])
        .stick_to_bottom(allow_stick_to_bottom)
        .show_viewport(ui, |ui, viewport| {
            let total_rows = lines.len();
            let visible_rows = ((viewport.height() / row_height).ceil() as usize).max(1);
            let overscan = visible_rows.max(8);

            // When stick_to_bottom is active and we are at (or past) the computed
            // bottom offset, pin first_row directly from total_rows instead of
            // dividing viewport.min.y by row_height.  The viewport offset is derived
            // from the *previous* frame's content measurement, so floating-point drift
            // causes it to land just above or just below the true bottom on alternating
            // frames, which flips first_row by ±1 and produces the visible jitter.
            let max_scroll_y = (total_rows as f32 * row_height - viewport.height()).max(0.0);
            let at_bottom = allow_stick_to_bottom && viewport.min.y >= max_scroll_y - row_height;
            let first_row = if at_bottom {
                total_rows.saturating_sub(visible_rows)
            } else {
                ((viewport.min.y / row_height).floor().max(0.0) as usize)
                    .min(total_rows.saturating_sub(1))
            };
            let last_row = (first_row + visible_rows + overscan).min(total_rows);

            let top_space = first_row as f32 * row_height;
            let bottom_space = total_rows.saturating_sub(last_row) as f32 * row_height;

            let mut parse_context = warm_log_parse_context(lines, first_row);
            let mut row_hits: Vec<VisibleLogRowHit> =
                Vec::with_capacity(last_row.saturating_sub(first_row));

            let clip_rect = ui.clip_rect();
            let viewport_response =
                ui.interact(clip_rect, viewport_id, egui::Sense::click_and_drag());
            viewport_has_focus = viewport_response.has_focus();

            ui.set_min_width(viewer_state.max_line_width.max(viewport.width()).max(1.0));
            ui.add_space(top_space);

            for (offset, line) in lines[first_row..last_row].iter().enumerate() {
                let line_index = first_row + offset;
                let level = resolve_log_level(line, &mut parse_context);
                let options = virtual_log_line_options(ui, level);
                let spans = [RichTextSpan {
                    text: line.clone(),
                    style: RichTextStyle {
                        color: options.color,
                        monospace: options.monospace,
                        italic: options.italic,
                        weight: options.weight,
                    },
                }];

                let texture = text_ui.prepare_rich_text_texture(
                    ui.ctx(),
                    (text_base_id, "virtual_line", line_index),
                    &spans,
                    &options,
                    None,
                );

                viewer_state.max_line_width = viewer_state
                    .max_line_width
                    .max(texture.size_points.x.ceil().max(1.0));

                let desired_width = viewer_state.max_line_width.max(viewport.width()).max(1.0);
                let desired_height = row_height;

                let galley = ui.painter().layout_no_wrap(
                    line.clone(),
                    galley_font_id(&options),
                    Color32::TRANSPARENT,
                );

                let (rect, _) = ui.allocate_exact_size(
                    vec2(desired_width, desired_height),
                    egui::Sense::hover(),
                );

                let text_rect = egui::Rect::from_min_size(rect.min, texture.size_points);

                row_hits.push(VisibleLogRowHit {
                    line_index,
                    rect,
                    text_rect,
                    galley: galley.clone(),
                    line_len_chars: log_char_count(line),
                });

                paint_log_selection_for_line(ui, rect, &galley, line, line_index, &selection_state);
                texture.paint(ui, text_rect);
            }

            let mut current_hovered_line: Option<usize> = None;
            if let Some(pointer_pos) = ui.input(|i| i.pointer.hover_pos()) {
                if let Some(row) = row_hits.iter().find(|row| row_contains_text(row, pointer_pos)) {
                    current_hovered_line = Some(row.line_index);
                    ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Text);
                }
            }

            if selection_active {
                let (modifiers, smooth_scroll_delta, pointer_pos) = ui.input(|i| {
                    (
                        i.modifiers,
                        i.smooth_scroll_delta,
                        i.pointer.hover_pos().or_else(|| i.pointer.latest_pos()),
                    )
                });
                let pointer_over_viewport =
                    pointer_pos.is_some_and(|pointer_pos| clip_rect.contains(pointer_pos));
                if pointer_over_viewport {
                    let vertical_scroll_delta = smooth_scroll_delta.y;
                    let horizontal_scroll_delta = if smooth_scroll_delta.x.abs() > f32::EPSILON {
                        smooth_scroll_delta.x
                    } else if modifiers.shift && smooth_scroll_delta.y.abs() > f32::EPSILON {
                        smooth_scroll_delta.y
                    } else {
                        0.0
                    };
                    let horizontal_uses_vertical_wheel = modifiers.shift
                        && smooth_scroll_delta.x.abs() <= f32::EPSILON
                        && horizontal_scroll_delta.abs() > f32::EPSILON;
                    let scroll_delta = vec2(
                        if horizontal_scroll_delta.abs() > f32::EPSILON {
                            -horizontal_scroll_delta
                        } else {
                            0.0
                        },
                        if !horizontal_uses_vertical_wheel
                            && vertical_scroll_delta.abs() > f32::EPSILON
                        {
                            -vertical_scroll_delta
                        } else {
                            0.0
                        },
                    );
                    if scroll_delta.x.abs() > f32::EPSILON || scroll_delta.y.abs() > f32::EPSILON
                    {
                        ui.scroll_with_delta_animation(
                            scroll_delta,
                            egui::style::ScrollAnimation::none(),
                        );
                    }
                }
            }

            if viewport_response.secondary_clicked() {
                viewport_response.request_focus();
                viewport_has_focus = true;
                let latest_pos = ui.input(|i| i.pointer.latest_pos());
                let interact_pos = ui.input(|i| i.pointer.interact_pos());
                let press_origin = ui.input(|i| i.pointer.press_origin());
                let anchor_pos = latest_pos.or(interact_pos).or(press_origin);

                log_console_context_menu(format!(
                    "secondary_clicked hovered_line={:?} selection_active={} latest_pos={:?} interact_pos={:?} press_origin={:?} clip_rect={:?}",
                    current_hovered_line,
                    selection_state.has_selection(),
                    latest_pos,
                    interact_pos,
                    press_origin,
                    clip_rect,
                ));

                if let Some(anchor_pos) = anchor_pos {
                    let selection_active = selection_state.has_selection();

                    let items = if selection_active {
                        vec![
                            context_menu::ContextMenuItem::new_with_icon(
                                ACTION_COPY_SELECTION,
                                "Copy selection",
                                assets::COPY_SVG,
                            ),
                            context_menu::ContextMenuItem::new_with_icon(
                                ACTION_CLEAR_SELECTION,
                                "Clear selection",
                                assets::X_SVG,
                            ),
                        ]
                    } else if let Some(line_index) = current_hovered_line {
                        ui.ctx().data_mut(|data| data.insert_temp(menu_line_id, Some(line_index)));
                        vec![context_menu::ContextMenuItem::new_with_icon(
                            ACTION_COPY_LINE,
                            "Copy line",
                            assets::COPY_SVG,
                        )]
                    } else {
                        Vec::new()
                    };

                    log_console_context_menu(format!(
                        "requesting menu anchor=({:.1}, {:.1}) item_count={}",
                        anchor_pos.x,
                        anchor_pos.y,
                        items.len(),
                    ));

                    if !items.is_empty() {
                        context_menu::request(
                            ui.ctx(),
                            context_menu::ContextMenuRequest::new(
                                menu_source_id,
                                anchor_pos,
                                items,
                            ),
                        );
                    }
                } else {
                    log_console_context_menu("secondary_clicked but no pointer position was available");
                }
            }

            if viewport_response.clicked() {
                viewport_response.request_focus();
                viewport_has_focus = true;
                if let Some(pointer_pos) = viewport_response.interact_pointer_pos() {
                    if let Some(cursor) = cursor_from_visible_rows(&row_hits, pointer_pos) {
                        selection_state.anchor = Some(cursor);
                        selection_state.head = Some(cursor);
                        selection_state.dragging = false;
                    } else {
                        clear_selection = true;
                    }
                } else {
                    clear_selection = true;
                }
            }

            if viewport_response.drag_started() {
                viewport_response.request_focus();
                viewport_has_focus = true;
                if let Some(pointer_pos) = ui.input(|i| i.pointer.interact_pos()) {
                    if let Some(cursor) = cursor_from_visible_rows(&row_hits, pointer_pos) {
                        selection_state.anchor = Some(cursor);
                        selection_state.head = Some(cursor);
                        selection_state.dragging = true;
                    }
                }
            }

            if selection_state.dragging && ui.input(|i| i.pointer.primary_down()) {
                if let Some(pointer_pos) =
                    ui.input(|i| i.pointer.latest_pos().or_else(|| i.pointer.interact_pos()))
                {
                    if let Some(cursor) = cursor_from_visible_rows(&row_hits, pointer_pos) {
                        selection_state.head = Some(cursor);
                    }
                    let vertical_margin =
                        LOG_SELECTION_AUTOSCROLL_MARGIN.min(clip_rect.height() * 0.25);
                    let horizontal_margin =
                        LOG_SELECTION_AUTOSCROLL_MARGIN.min(clip_rect.width() * 0.25);
                    if vertical_margin > 0.0 || horizontal_margin > 0.0 {
                        let dt = ui.input(|i| i.stable_dt).max(1.0 / 240.0);
                        let autoscroll_delta_x = edge_autoscroll_speed(
                            pointer_pos.x,
                            clip_rect.left(),
                            clip_rect.right(),
                            horizontal_margin,
                        ) * dt;
                        let autoscroll_delta_y = edge_autoscroll_speed(
                            pointer_pos.y,
                            clip_rect.top(),
                            clip_rect.bottom(),
                            vertical_margin,
                        ) * dt;
                        if autoscroll_delta_x.abs() > f32::EPSILON
                            || autoscroll_delta_y.abs() > f32::EPSILON
                        {
                            ui.scroll_with_delta_animation(
                                vec2(autoscroll_delta_x, autoscroll_delta_y),
                                egui::style::ScrollAnimation::none(),
                            );
                            ui.ctx().request_repaint();
                        }
                    }
                }
            }

            if selection_state.dragging && !ui.input(|i| i.pointer.primary_down()) {
                selection_state.dragging = false;
            }

            ui.add_space(bottom_space);
        });
    textui::make_gamepad_scrollable(ui.ctx(), &scroll_output);
    ui.ctx().data_mut(|d| {
        d.insert_temp(egui::Id::new(CONSOLE_LOG_SCROLL_ID_KEY), scroll_output.id);
    });
    let max_offset_y = (scroll_output.content_size.y - scroll_output.inner_rect.height()).max(0.0);
    let at_bottom_after_scroll = scroll_output.state.offset.y >= max_offset_y - row_height;
    if selection_active {
        viewer_state.follow_bottom = false;
    } else if at_bottom_after_scroll {
        viewer_state.follow_bottom = stick_to_bottom;
    } else if scroll_output.state.offset != previous_scroll_state.offset {
        viewer_state.follow_bottom = false;
    } else if !stick_to_bottom {
        viewer_state.follow_bottom = false;
    }

    if clear_selection {
        selection_state.clear();
    }

    let copy_requested = selection_state.has_selection()
        && ui.input_mut(|i| {
            let command_shortcut =
                egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::C);
            let ctrl_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::C);
            let shortcut_copy =
                i.consume_shortcut(&command_shortcut) || i.consume_shortcut(&ctrl_shortcut);
            let logical_copy = i.consume_key(egui::Modifiers::NONE, egui::Key::Copy);
            let mut event_copy = false;
            i.events.retain(|event| {
                let is_copy = matches!(event, egui::Event::Copy);
                event_copy |= is_copy;
                !is_copy
            });
            shortcut_copy || logical_copy || event_copy
        });

    if copy_requested {
        if let Some(text) = selected_log_text(lines, &selection_state) {
            ui.ctx().copy_text(text);
        }
    }

    ui.ctx().data_mut(|data| {
        data.insert_temp(state_id, viewer_state);
        data.insert_temp(selection_id, selection_state);
    });
}

fn render_tabs_row(ui: &mut Ui, text_ui: &mut TextUi, snapshot: &console::ConsoleSnapshot) {
    let want_tab_focus = take_console_tab_focus_request(ui.ctx());
    egui::ScrollArea::horizontal()
        .id_salt("console_tabs")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(style::SPACE_SM, style::SPACE_SM);
                for (tab_idx, tab) in snapshot.tabs.iter().enumerate() {
                    let selected = tab.id == snapshot.active_tab_id;
                    let fill = if selected {
                        ui.visuals().selection.bg_fill
                    } else {
                        ui.visuals().widgets.inactive.weak_bg_fill
                    };
                    let stroke = if selected {
                        ui.visuals().selection.stroke
                    } else {
                        ui.visuals().widgets.inactive.bg_stroke
                    };
                    egui::Frame::new()
                        .fill(fill)
                        .stroke(stroke)
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = style::SPACE_XS;
                                let mut label_style = style::body(ui);
                                label_style.wrap = false;
                                label_style.weight = if selected { 700 } else { 500 };
                                let label_response = text_ui.clickable_label(
                                    ui,
                                    ("console_tab_label", tab.id.as_str()),
                                    tab.label.as_str(),
                                    &label_style,
                                );
                                if label_response.clicked() {
                                    console::set_active_tab(tab.id.as_str());
                                }
                                if want_tab_focus && tab_idx == 0 {
                                    label_response.request_focus();
                                }

                                if tab.can_close {
                                    let close_style = ButtonOptions {
                                        min_size: vec2(26.0, 26.0),
                                        corner_radius: style::CORNER_RADIUS_SM,
                                        padding: vec2(0.0, 0.0),
                                        text_color: ui.visuals().text_color(),
                                        fill: ui.visuals().widgets.inactive.weak_bg_fill,
                                        fill_hovered: ui.visuals().widgets.hovered.weak_bg_fill,
                                        fill_active: ui.visuals().widgets.active.weak_bg_fill,
                                        fill_selected: ui.visuals().widgets.open.weak_bg_fill,
                                        stroke: ui.visuals().widgets.inactive.bg_stroke,
                                        font_size: 20.0,
                                        line_height: 20.0,
                                    };
                                    let close_response = text_ui.button(
                                        ui,
                                        ("console_tab_close", tab.id.as_str()),
                                        "×",
                                        &close_style,
                                    );
                                    if close_response.clicked() {
                                        let _ = console::close_tab(tab.id.as_str());
                                    }
                                }
                            });
                        });
                }
            });
        });
}

fn color_for_level(ui: &Ui, level: Option<LogLevel>) -> egui::Color32 {
    match level {
        Some(LogLevel::Fatal | LogLevel::Error) => ui.visuals().error_fg_color,
        Some(LogLevel::Warn) => ui.visuals().warn_fg_color,
        Some(LogLevel::Info) => ui.visuals().hyperlink_color,
        Some(LogLevel::Debug | LogLevel::Trace) => ui.visuals().weak_text_color(),
        None => ui.visuals().text_color(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

#[derive(Clone, Debug, Default)]
struct LogParseContext {
    in_error_trace: bool,
}

fn resolve_log_level(line: &str, context: &mut LogParseContext) -> Option<LogLevel> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        context.in_error_trace = false;
        return None;
    }

    if let Some(level) = detect_log_level(line) {
        context.in_error_trace = matches!(level, LogLevel::Error | LogLevel::Fatal);
        return Some(level);
    }

    if is_stacktrace_line(trimmed)
        || (context.in_error_trace && is_stacktrace_continuation_line(trimmed))
    {
        context.in_error_trace = true;
        return Some(LogLevel::Error);
    }

    if trimmed.starts_with('[') {
        context.in_error_trace = false;
    }
    None
}

fn detect_log_level(line: &str) -> Option<LogLevel> {
    if let Some(level) = parse_minecraft_log_level(line) {
        return Some(level);
    }
    parse_generic_log_level(line)
}

fn is_stacktrace_line(trimmed: &str) -> bool {
    trimmed.starts_with("at ")
        || trimmed.starts_with("Caused by:")
        || trimmed.starts_with("Suppressed:")
        || trimmed.starts_with("Exception in thread ")
        || (trimmed.starts_with("... ") && trimmed.ends_with(" more"))
        || trimmed.contains("Exception:")
        || trimmed.ends_with("Exception")
        || trimmed.contains("Error:")
        || trimmed.ends_with("Error")
}

fn is_stacktrace_continuation_line(trimmed: &str) -> bool {
    trimmed.starts_with('\t')
        || trimmed.starts_with("com.")
        || trimmed.starts_with("net.")
        || trimmed.starts_with("org.")
        || trimmed.starts_with("java.")
        || trimmed.starts_with("javax.")
        || trimmed.starts_with("kotlin.")
        || trimmed.starts_with('#')
}

fn parse_minecraft_log_level(line: &str) -> Option<LogLevel> {
    if !line.starts_with('[') {
        return None;
    }
    let first_close = line.find(']')?;
    if first_close < 2 {
        return None;
    }
    let timestamp = &line[1..first_close];
    if !looks_like_minecraft_timestamp(timestamp) {
        return None;
    }
    let after_timestamp = line.get(first_close + 1..)?;
    if !after_timestamp.starts_with(" [") {
        return None;
    }
    let second = after_timestamp.get(2..)?;
    let second_close = second.find(']')?;
    let thread_and_level = &second[..second_close];
    if let Some((_, level_token)) = thread_and_level.rsplit_once('/')
        && let Some(level) = parse_level_token(level_token)
    {
        return Some(level);
    }

    Some(LogLevel::Info)
}

fn parse_generic_log_level(line: &str) -> Option<LogLevel> {
    for (token, level) in [
        ("FATAL", LogLevel::Fatal),
        ("ERROR", LogLevel::Error),
        ("WARN", LogLevel::Warn),
        ("INFO", LogLevel::Info),
        ("DEBUG", LogLevel::Debug),
        ("TRACE", LogLevel::Trace),
    ] {
        if line.contains(&format!("][{token}]["))
            || line.contains(&format!("][{token}]:"))
            || line.contains(&format!("/{token}]"))
            || line.contains(&format!("/{token}]:"))
        {
            return Some(level);
        }
    }
    None
}

fn parse_level_token(token: &str) -> Option<LogLevel> {
    match token.trim() {
        "TRACE" => Some(LogLevel::Trace),
        "DEBUG" => Some(LogLevel::Debug),
        "INFO" => Some(LogLevel::Info),
        "WARN" => Some(LogLevel::Warn),
        "ERROR" => Some(LogLevel::Error),
        "FATAL" => Some(LogLevel::Fatal),
        _ => None,
    }
}

fn looks_like_minecraft_timestamp(value: &str) -> bool {
    let mut parts = value.split(':');
    let Some(hours) = parts.next() else {
        return false;
    };
    let Some(minutes) = parts.next() else {
        return false;
    };
    let Some(seconds) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    [hours, minutes, seconds]
        .iter()
        .all(|part| part.len() == 2 && part.as_bytes().iter().all(u8::is_ascii_digit))
}
