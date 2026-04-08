use super::*;

/// Coarse operation kind used to group consecutive edits into a single undo
/// entry (so typing a word is one undo step rather than per-char).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum UndoOpKind {
    #[default]
    None,
    TextInsert,
    Delete,
    Paste,
    Cut,
}

#[derive(Clone, Debug)]
pub(super) struct UndoEntry {
    pub(super) text: String,
    pub(super) cursor: Cursor,
    pub(super) selection: Selection,
}

#[derive(Debug)]
pub(super) struct InputState {
    pub(super) editor: Editor<'static>,
    pub(super) last_text: String,
    pub(super) attrs_fingerprint: u64,
    pub(super) multiline: bool,
    pub(super) preferred_cursor_x_px: Option<f32>,
    pub(super) scroll_metrics: EditorScrollMetrics,
    pub(super) last_used_frame: u64,
    pub(super) undo_stack: Vec<UndoEntry>,
    pub(super) redo_stack: Vec<UndoEntry>,
    pub(super) last_undo_op: UndoOpKind,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct EditorScrollMetrics {
    pub(super) current_horizontal_scroll_px: f32,
    pub(super) max_horizontal_scroll_px: f32,
    pub(super) current_vertical_scroll_px: f32,
    pub(super) max_vertical_scroll_px: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ViewerScrollbarTracks {
    pub(super) horizontal: Option<Rect>,
    pub(super) vertical: Option<Rect>,
}

impl ViewerScrollbarTracks {
    pub(super) fn contains(self, pos: Pos2) -> bool {
        self.horizontal.is_some_and(|rect| rect.contains(pos))
            || self.vertical.is_some_and(|rect| rect.contains(pos))
    }
}

pub(super) fn editor_to_string(editor: &Editor<'static>) -> String {
    let mut out = String::new();
    editor.with_buffer(|buffer| {
        for line in &buffer.lines {
            out.push_str(line.text());
            out.push_str(line.ending().as_str());
        }
    });
    out
}

pub(super) fn editor_horizontal_scroll(editor: &Editor<'static>) -> f32 {
    editor.with_buffer(|buffer| buffer.scroll().horizontal.max(0.0))
}

pub(super) fn clamp_cursor_to_editor(editor: &Editor<'static>, cursor: Cursor) -> Cursor {
    editor.with_buffer(|buffer| {
        let line_index = cursor.line.min(buffer.lines.len().saturating_sub(1));
        let line = &buffer.lines[line_index];
        Cursor::new_with_affinity(
            line_index,
            cursor.index.min(line.text().len()),
            cursor.affinity,
        )
    })
}

pub(super) fn clamp_selection_to_editor(
    editor: &Editor<'static>,
    selection: Selection,
) -> Selection {
    match selection {
        Selection::None => Selection::None,
        Selection::Normal(cursor) => Selection::Normal(clamp_cursor_to_editor(editor, cursor)),
        Selection::Line(cursor) => Selection::Line(clamp_cursor_to_editor(editor, cursor)),
        Selection::Word(cursor) => Selection::Word(clamp_cursor_to_editor(editor, cursor)),
    }
}

pub(super) fn selection_anchor(selection: Selection) -> Option<Cursor> {
    match selection {
        Selection::None => None,
        Selection::Normal(cursor) | Selection::Line(cursor) | Selection::Word(cursor) => {
            Some(cursor)
        }
    }
}

pub(super) fn cursor_x_for_layout_cursor(
    buffer: &mut Buffer,
    font_system: &mut FontSystem,
    cursor: Cursor,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> Option<f32> {
    let layout_cursor = buffer.layout_cursor(font_system, cursor)?;
    let line_text = buffer.lines.get(layout_cursor.line)?.text().to_owned();
    let layout = buffer.line_layout(font_system, layout_cursor.line)?;
    let layout_line = layout.get(layout_cursor.layout).or_else(|| layout.last())?;
    let stops = cursor_stops_for_glyphs(
        layout_cursor.line,
        &line_text,
        &layout_line.glyphs,
        fundamentals,
        scale,
    );
    stops
        .into_iter()
        .find(|(stop_cursor, _)| {
            stop_cursor.line == cursor.line && stop_cursor.index == cursor.index
        })
        .map(|(_, x)| x)
        .or_else(|| layout_line.glyphs.is_empty().then_some(0.0))
}

pub(super) fn cursor_for_layout_line_x(
    buffer: &mut Buffer,
    font_system: &mut FontSystem,
    line_i: usize,
    layout_i: usize,
    desired_x: f32,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> Option<Cursor> {
    let line_text = buffer.lines.get(line_i)?.text().to_owned();
    let layout = buffer.line_layout(font_system, line_i)?;
    let layout_line = layout.get(layout_i).or_else(|| layout.last())?;
    let stops =
        cursor_stops_for_glyphs(line_i, &line_text, &layout_line.glyphs, fundamentals, scale);

    if let Some((first_cursor, first_x)) = stops.first().copied()
        && desired_x <= first_x
    {
        return Some(first_cursor);
    }

    for window in stops.windows(2) {
        let (left_cursor, left_x) = window[0];
        let (right_cursor, right_x) = window[1];
        let mid_x = (left_x + right_x) * 0.5;
        if desired_x <= mid_x {
            return Some(left_cursor);
        }
        if desired_x <= right_x {
            return Some(right_cursor);
        }
    }

    stops
        .last()
        .map(|(cursor, _)| *cursor)
        .or_else(|| Some(Cursor::new_with_affinity(line_i, 0, Affinity::After)))
}

pub(super) fn adjacent_visual_layout_position(
    buffer: &mut Buffer,
    font_system: &mut FontSystem,
    cursor: Cursor,
    direction: i32,
) -> Option<(usize, usize)> {
    let mut layout_cursor = buffer.layout_cursor(font_system, cursor)?;
    match direction.cmp(&0) {
        Ordering::Less => {
            if layout_cursor.layout > 0 {
                layout_cursor.layout -= 1;
            } else if layout_cursor.line > 0 {
                layout_cursor.line -= 1;
                let layout_count = buffer.line_layout(font_system, layout_cursor.line)?.len();
                layout_cursor.layout = layout_count.saturating_sub(1);
            } else {
                return None;
            }
        }
        Ordering::Greater => {
            let layout_count = buffer.line_layout(font_system, layout_cursor.line)?.len();
            if layout_cursor.layout + 1 < layout_count {
                layout_cursor.layout += 1;
            } else if layout_cursor.line + 1 < buffer.lines.len() {
                layout_cursor.line += 1;
                layout_cursor.layout = 0;
            } else {
                return None;
            }
        }
        Ordering::Equal => return Some((layout_cursor.line, layout_cursor.layout)),
    }
    Some((layout_cursor.line, layout_cursor.layout))
}

pub(super) fn move_cursor_one_visual_line(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    direction: i32,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    let current_cursor = editor.cursor();
    let desired_x = preferred_cursor_x_px.unwrap_or_else(|| {
        editor
            .with_buffer_mut(|buffer| {
                cursor_x_for_layout_cursor(buffer, font_system, current_cursor, fundamentals, scale)
            })
            .unwrap_or(0.0)
    });
    *preferred_cursor_x_px = Some(desired_x);

    let Some((target_line, target_layout)) = editor.with_buffer_mut(|buffer| {
        adjacent_visual_layout_position(buffer, font_system, current_cursor, direction)
    }) else {
        return false;
    };

    let Some(new_cursor) = editor.with_buffer_mut(|buffer| {
        cursor_for_layout_line_x(
            buffer,
            font_system,
            target_line,
            target_layout,
            desired_x,
            fundamentals,
            scale,
        )
    }) else {
        return false;
    };

    if new_cursor != current_cursor {
        editor.set_cursor(new_cursor);
        true
    } else {
        false
    }
}

pub(super) fn handle_spacing_aware_vertical_motion(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    motion: Motion,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    match motion {
        Motion::Up => move_cursor_one_visual_line(
            font_system,
            editor,
            -1,
            preferred_cursor_x_px,
            fundamentals,
            scale,
        ),
        Motion::Down => move_cursor_one_visual_line(
            font_system,
            editor,
            1,
            preferred_cursor_x_px,
            fundamentals,
            scale,
        ),
        Motion::PageUp | Motion::PageDown | Motion::Vertical(_) => {
            let step_count = editor.with_buffer(|buffer| match motion {
                Motion::PageUp => buffer
                    .size()
                    .1
                    .map(|height| -(height as i32 / buffer.metrics().line_height as i32))
                    .unwrap_or(0),
                Motion::PageDown => buffer
                    .size()
                    .1
                    .map(|height| height as i32 / buffer.metrics().line_height as i32)
                    .unwrap_or(0),
                Motion::Vertical(px) => px / buffer.metrics().line_height as i32,
                _ => 0,
            });
            let direction = step_count.signum();
            let mut moved = false;
            for _ in 0..step_count.unsigned_abs() {
                if !move_cursor_one_visual_line(
                    font_system,
                    editor,
                    direction,
                    preferred_cursor_x_px,
                    fundamentals,
                    scale,
                ) {
                    break;
                }
                moved = true;
            }
            moved
        }
        _ => false,
    }
}

pub(super) fn editor_hit_test(
    editor: &Editor<'static>,
    x: i32,
    y: i32,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> Option<Cursor> {
    editor.with_buffer(|buffer| {
        hit_buffer_with_fundamentals(buffer, x as f32, y as f32, fundamentals, scale)
    })
}

pub(super) fn click_editor_to_pointer(
    editor: &mut Editor<'static>,
    x: i32,
    y: i32,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    *preferred_cursor_x_px = None;
    let old_cursor = editor.cursor();
    let old_selection = editor.selection();
    editor.set_selection(Selection::None);
    if let Some(new_cursor) = editor_hit_test(editor, x, y, fundamentals, scale) {
        editor.set_cursor(new_cursor);
    }
    editor.cursor() != old_cursor || editor.selection() != old_selection
}

pub(super) fn double_click_editor_to_pointer(
    editor: &mut Editor<'static>,
    x: i32,
    y: i32,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    *preferred_cursor_x_px = None;
    let old_cursor = editor.cursor();
    let old_selection = editor.selection();
    editor.set_selection(Selection::None);
    if let Some(new_cursor) = editor_hit_test(editor, x, y, fundamentals, scale) {
        editor.set_cursor(new_cursor);
        editor.set_selection(Selection::Word(editor.cursor()));
    }
    editor.cursor() != old_cursor || editor.selection() != old_selection
}

pub(super) fn triple_click_editor_to_pointer(
    editor: &mut Editor<'static>,
    x: i32,
    y: i32,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    *preferred_cursor_x_px = None;
    let old_cursor = editor.cursor();
    let old_selection = editor.selection();
    editor.set_selection(Selection::None);
    if let Some(new_cursor) = editor_hit_test(editor, x, y, fundamentals, scale) {
        editor.set_cursor(new_cursor);
        editor.set_selection(Selection::Line(editor.cursor()));
    }
    editor.cursor() != old_cursor || editor.selection() != old_selection
}

pub(super) fn drag_editor_selection_to_pointer(
    editor: &mut Editor<'static>,
    x: i32,
    y: i32,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    *preferred_cursor_x_px = None;
    let old_cursor = editor.cursor();
    let old_selection = editor.selection();
    if editor.selection() == Selection::None {
        editor.set_selection(Selection::Normal(editor.cursor()));
    }
    if let Some(new_cursor) = editor_hit_test(editor, x, y, fundamentals, scale) {
        editor.set_cursor(new_cursor);
    }
    editor.cursor() != old_cursor || editor.selection() != old_selection
}

pub(super) fn extend_selection_to_pointer(
    editor: &mut Editor<'static>,
    x: i32,
    y: i32,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    *preferred_cursor_x_px = None;
    let anchor = selection_anchor(editor.selection()).unwrap_or_else(|| editor.cursor());
    let Some(new_cursor) = editor_hit_test(editor, x, y, fundamentals, scale) else {
        return false;
    };

    editor.set_cursor(new_cursor);
    if new_cursor == anchor {
        editor.set_selection(Selection::None);
    } else {
        editor.set_selection(Selection::Normal(anchor));
    }
    true
}

pub(super) fn select_all(editor: &mut Editor<'static>) -> bool {
    let end = editor.with_buffer(|buffer| {
        let Some(line) = buffer.lines.len().checked_sub(1) else {
            return Cursor::new(0, 0);
        };
        Cursor::new(line, buffer.lines[line].text().len())
    });
    editor.set_selection(Selection::Normal(Cursor::new(0, 0)));
    editor.set_cursor(end);
    true
}

pub(super) fn classify_modify_op(event: &TextInputEvent) -> UndoOpKind {
    match event {
        TextInputEvent::Text(t) if !t.is_empty() => UndoOpKind::TextInsert,
        TextInputEvent::Paste(p) if !p.is_empty() => UndoOpKind::Paste,
        TextInputEvent::Cut => UndoOpKind::Cut,
        TextInputEvent::Key {
            key,
            pressed: true,
            modifiers,
        } => {
            let word_delete = (modifiers.alt || modifiers.ctrl || modifiers.mac_cmd)
                && matches!(key, TextKey::Backspace | TextKey::Delete);
            let emacs_delete =
                modifiers.ctrl && matches!(key, TextKey::H | TextKey::K | TextKey::U | TextKey::W);
            if matches!(key, TextKey::Backspace | TextKey::Delete) || word_delete || emacs_delete {
                UndoOpKind::Delete
            } else {
                UndoOpKind::None
            }
        }
        TextInputEvent::PointerButton {
            button: TextPointerButton::Middle,
            pressed: true,
            ..
        } => UndoOpKind::Paste,
        _ => UndoOpKind::None,
    }
}

pub(super) fn is_navigation_event(event: &TextInputEvent) -> bool {
    matches!(
        event,
        TextInputEvent::Key {
            key: TextKey::Left
                | TextKey::Right
                | TextKey::Up
                | TextKey::Down
                | TextKey::Home
                | TextKey::End
                | TextKey::PageUp
                | TextKey::PageDown,
            pressed: true,
            ..
        }
    )
}

pub(super) fn pending_modify_op(events: &[TextInputEvent]) -> UndoOpKind {
    events
        .iter()
        .map(classify_modify_op)
        .find(|op| *op != UndoOpKind::None)
        .unwrap_or(UndoOpKind::None)
}

pub(super) fn push_undo(stack: &mut Vec<UndoEntry>, entry: UndoEntry) {
    if stack.len() >= UNDO_STACK_MAX {
        stack.remove(0);
    }
    stack.push(entry);
}

pub(super) fn handle_editor_key_event(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    key: Key,
    modifiers: egui::Modifiers,
    multiline: bool,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    if modifiers.command && key == Key::A {
        *preferred_cursor_x_px = None;
        return select_all(editor);
    }

    if handle_editor_delete_shortcut(font_system, editor, key, modifiers) {
        *preferred_cursor_x_px = None;
        return true;
    }

    if cfg!(target_os = "macos") && modifiers.ctrl && !modifiers.shift {
        if let Some(motion) = mac_control_motion(key) {
            return handle_editor_motion_key(
                font_system,
                editor,
                key,
                modifiers,
                motion,
                preferred_cursor_x_px,
                fundamentals,
                scale,
            );
        }
    }

    let Some(action) = key_to_action(key, modifiers, multiline) else {
        return false;
    };

    match action {
        Action::Motion(motion) => handle_editor_motion_key(
            font_system,
            editor,
            key,
            modifiers,
            motion,
            preferred_cursor_x_px,
            fundamentals,
            scale,
        ),
        _ => {
            *preferred_cursor_x_px = None;
            editor.borrow_with(font_system).action(action);
            true
        }
    }
}

pub(super) fn handle_read_only_editor_key_event(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    key: Key,
    modifiers: egui::Modifiers,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    if modifiers.command && key == Key::A {
        *preferred_cursor_x_px = None;
        return select_all(editor);
    }

    if cfg!(target_os = "macos") && modifiers.ctrl && !modifiers.shift {
        if let Some(motion) = mac_control_motion(key) {
            return handle_editor_motion_key(
                font_system,
                editor,
                key,
                modifiers,
                motion,
                preferred_cursor_x_px,
                fundamentals,
                scale,
            );
        }
    }

    let Some(action) = key_to_action(key, modifiers, true) else {
        if key == Key::Escape && editor.selection() != Selection::None {
            *preferred_cursor_x_px = None;
            editor.set_selection(Selection::None);
            return true;
        }
        return false;
    };

    match action {
        Action::Motion(motion) => handle_editor_motion_key(
            font_system,
            editor,
            key,
            modifiers,
            motion,
            preferred_cursor_x_px,
            fundamentals,
            scale,
        ),
        Action::Escape => {
            if editor.selection() != Selection::None {
                *preferred_cursor_x_px = None;
                editor.set_selection(Selection::None);
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

pub(super) fn scroll_editor_to_buffer_end(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
) {
    editor.set_selection(Selection::None);
    editor
        .borrow_with(font_system)
        .action(Action::Motion(Motion::BufferEnd));
}

pub(super) fn handle_editor_motion_key(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    key: Key,
    modifiers: egui::Modifiers,
    motion: Motion,
    preferred_cursor_x_px: &mut Option<f32>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> bool {
    if modifiers.shift {
        if editor.selection() == Selection::None {
            editor.set_selection(Selection::Normal(editor.cursor()));
        }
        if motion_uses_preferred_cursor_x(motion) {
            return handle_spacing_aware_vertical_motion(
                font_system,
                editor,
                motion,
                preferred_cursor_x_px,
                fundamentals,
                scale,
            );
        }
        *preferred_cursor_x_px = None;
        editor
            .borrow_with(font_system)
            .action(Action::Motion(motion));
        return true;
    }

    if let Some((start, end)) = editor.selection_bounds() {
        if modifiers.is_none() && key == Key::ArrowLeft {
            *preferred_cursor_x_px = None;
            editor.set_selection(Selection::None);
            editor.set_cursor(start);
            return true;
        }
        if modifiers.is_none() && key == Key::ArrowRight {
            *preferred_cursor_x_px = None;
            editor.set_selection(Selection::None);
            editor.set_cursor(end);
            return true;
        }
        editor.set_selection(Selection::None);
    }

    if motion_uses_preferred_cursor_x(motion) {
        handle_spacing_aware_vertical_motion(
            font_system,
            editor,
            motion,
            preferred_cursor_x_px,
            fundamentals,
            scale,
        )
    } else {
        *preferred_cursor_x_px = None;
        editor
            .borrow_with(font_system)
            .action(Action::Motion(motion));
        true
    }
}

pub(super) fn handle_editor_delete_shortcut(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    key: Key,
    modifiers: egui::Modifiers,
) -> bool {
    match key {
        Key::Backspace if modifiers.mac_cmd => delete_to_motion(font_system, editor, Motion::Home),
        Key::Backspace if modifiers.alt || modifiers.ctrl => {
            delete_to_motion(font_system, editor, Motion::PreviousWord)
        }
        Key::Delete if (!modifiers.shift || !cfg!(target_os = "windows")) && modifiers.mac_cmd => {
            delete_forward_to_motion(font_system, editor, Motion::End)
        }
        Key::Delete
            if (!modifiers.shift || !cfg!(target_os = "windows"))
                && (modifiers.alt || modifiers.ctrl) =>
        {
            delete_forward_to_motion(font_system, editor, Motion::NextWord)
        }
        Key::H if modifiers.ctrl => {
            editor.borrow_with(font_system).action(Action::Backspace);
            true
        }
        Key::K if modifiers.ctrl => delete_forward_to_motion(font_system, editor, Motion::End),
        Key::U if modifiers.ctrl => delete_to_motion(font_system, editor, Motion::Home),
        Key::W if modifiers.ctrl => delete_to_motion(font_system, editor, Motion::PreviousWord),
        _ => false,
    }
}

pub(super) fn delete_to_motion(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    motion: Motion,
) -> bool {
    if editor.delete_selection() {
        return true;
    }

    let end = editor.cursor();
    let Some(start) = cursor_after_motion(font_system, editor, end, motion) else {
        return false;
    };
    delete_cursor_range(editor, start, end)
}

pub(super) fn delete_forward_to_motion(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    motion: Motion,
) -> bool {
    if editor.delete_selection() {
        return true;
    }

    let start = editor.cursor();
    let Some(end) = cursor_after_motion(font_system, editor, start, motion) else {
        return false;
    };
    delete_cursor_range(editor, start, end)
}

pub(super) fn cursor_after_motion(
    font_system: &mut FontSystem,
    editor: &mut Editor<'static>,
    cursor: Cursor,
    motion: Motion,
) -> Option<Cursor> {
    editor.with_buffer_mut(|buffer| {
        let mut borrowed = buffer.borrow_with(font_system);
        borrowed
            .cursor_motion(cursor, None, motion)
            .map(|(next, _)| next)
    })
}

pub(super) fn delete_cursor_range(
    editor: &mut Editor<'static>,
    first: Cursor,
    second: Cursor,
) -> bool {
    if first == second {
        return false;
    }

    let (start, end) = ordered_cursor_pair(first, second);
    editor.set_selection(Selection::None);
    editor.set_cursor(start);
    editor.delete_range(start, end);
    true
}

pub(super) fn ordered_cursor_pair(first: Cursor, second: Cursor) -> (Cursor, Cursor) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
}

pub(super) fn mac_control_motion(key: Key) -> Option<Motion> {
    match key {
        Key::A => Some(Motion::Home),
        Key::E => Some(Motion::End),
        Key::B => Some(Motion::Left),
        Key::F => Some(Motion::Right),
        Key::P => Some(Motion::Up),
        Key::N => Some(Motion::Down),
        _ => None,
    }
}

pub(super) fn key_to_action(
    key: Key,
    modifiers: egui::Modifiers,
    multiline: bool,
) -> Option<Action> {
    match key {
        Key::ArrowLeft => Some(if modifiers.alt || modifiers.ctrl {
            Action::Motion(Motion::PreviousWord)
        } else if modifiers.mac_cmd {
            Action::Motion(Motion::Home)
        } else {
            Action::Motion(Motion::Left)
        }),
        Key::ArrowRight => Some(if modifiers.alt || modifiers.ctrl {
            Action::Motion(Motion::NextWord)
        } else if modifiers.mac_cmd {
            Action::Motion(Motion::End)
        } else {
            Action::Motion(Motion::Right)
        }),
        Key::ArrowUp => Some(if modifiers.command {
            Action::Motion(Motion::BufferStart)
        } else {
            Action::Motion(Motion::Up)
        }),
        Key::ArrowDown => Some(if modifiers.command {
            Action::Motion(Motion::BufferEnd)
        } else {
            Action::Motion(Motion::Down)
        }),
        Key::Home => Some(if modifiers.ctrl {
            Action::Motion(Motion::BufferStart)
        } else {
            Action::Motion(Motion::Home)
        }),
        Key::End => Some(if modifiers.ctrl {
            Action::Motion(Motion::BufferEnd)
        } else {
            Action::Motion(Motion::End)
        }),
        Key::PageUp => Some(Action::Motion(Motion::PageUp)),
        Key::PageDown => Some(Action::Motion(Motion::PageDown)),
        Key::Backspace => Some(Action::Backspace),
        Key::Delete => Some(Action::Delete),
        Key::Escape => Some(Action::Escape),
        Key::Enter if multiline => Some(Action::Enter),
        Key::Tab if multiline => Some(if modifiers.shift {
            Action::Unindent
        } else {
            Action::Indent
        }),
        _ => None,
    }
}

fn motion_uses_preferred_cursor_x(motion: Motion) -> bool {
    matches!(
        motion,
        Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown | Motion::Vertical(_)
    )
}

pub(super) fn measure_buffer_pixels(buffer: &Buffer) -> (usize, usize) {
    let mut max_right = 0.0_f32;
    let mut max_bottom = 0.0_f32;

    for run in buffer.layout_runs() {
        max_bottom = max_bottom.max(run.line_top + run.line_height);
        for glyph in run.glyphs {
            max_right = max_right.max(glyph.x + glyph.w);
        }
    }

    if max_bottom <= 0.0 {
        max_bottom = buffer.metrics().line_height.max(1.0);
    }

    (
        max_right.ceil().max(1.0) as usize,
        max_bottom.ceil().max(1.0) as usize,
    )
}

pub(super) fn measure_borrowed_buffer_scroll_metrics(
    buffer: &mut BorrowedWithFontSystem<'_, Buffer>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> EditorScrollMetrics {
    let metrics = buffer.metrics();
    let scroll = buffer.scroll();
    let mut max_right = 0.0_f32;
    let mut max_bottom = 0.0_f32;
    let mut line_top = 0.0_f32;
    let mut current_vertical_scroll_px = 0.0_f32;
    let line_count = buffer.lines.len();

    for line_i in 0..line_count {
        if line_i == scroll.line {
            current_vertical_scroll_px = line_top + scroll.vertical.max(0.0);
        }

        let line_text = buffer.lines[line_i].text().to_owned();
        let Some(layout_lines) = buffer.line_layout(line_i) else {
            continue;
        };
        for layout_line in layout_lines {
            let line_height = layout_line.line_height_opt.unwrap_or(metrics.line_height);
            max_bottom = max_bottom.max(line_top + line_height);
            let prefixes = collect_glyph_spacing_prefixes_px(
                &line_text,
                &layout_line.glyphs,
                fundamentals,
                scale,
            );
            for (glyph_index, glyph) in layout_line.glyphs.iter().enumerate() {
                max_right = max_right.max(adjusted_glyph_right_px(glyph, prefixes[glyph_index]));
            }
            line_top += line_height;
        }
    }

    if scroll.line >= line_count {
        current_vertical_scroll_px = max_bottom.max(0.0);
    }

    if max_bottom <= 0.0 {
        max_bottom = metrics.line_height.max(1.0);
    }

    let content_width_px = max_right.ceil().max(1.0);
    let content_height_px = max_bottom.ceil().max(1.0);
    let viewport_width_px = buffer.size().0.unwrap_or(content_width_px).max(1.0);
    let viewport_height_px = buffer.size().1.unwrap_or(content_height_px).max(1.0);
    let max_horizontal_scroll_px = (content_width_px - viewport_width_px).max(0.0);
    let max_vertical_scroll_px = (content_height_px - viewport_height_px).max(0.0);

    EditorScrollMetrics {
        current_horizontal_scroll_px: scroll.horizontal.clamp(0.0, max_horizontal_scroll_px),
        max_horizontal_scroll_px,
        current_vertical_scroll_px: current_vertical_scroll_px.clamp(0.0, max_vertical_scroll_px),
        max_vertical_scroll_px,
    }
}

pub(super) fn clamp_borrowed_buffer_scroll(
    buffer: &mut BorrowedWithFontSystem<'_, Buffer>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> EditorScrollMetrics {
    let mut scroll_metrics = measure_borrowed_buffer_scroll_metrics(buffer, fundamentals, scale);
    let mut scroll = buffer.scroll();
    let clamped_horizontal = scroll
        .horizontal
        .clamp(0.0, scroll_metrics.max_horizontal_scroll_px);
    if (clamped_horizontal - scroll.horizontal).abs() > f32::EPSILON {
        scroll.horizontal = clamped_horizontal;
        buffer.set_scroll(scroll);
        buffer.shape_until_scroll(true);
    }
    scroll_metrics.current_horizontal_scroll_px = clamped_horizontal;
    scroll_metrics
}

pub(super) fn viewer_scrollbar_track_rects(
    scroll_style: egui::style::ScrollStyle,
    widget_hovered: bool,
    widget_active: bool,
    content_rect: Rect,
    scroll_metrics: EditorScrollMetrics,
) -> ViewerScrollbarTracks {
    let show_horizontal = scroll_metrics.max_horizontal_scroll_px > f32::EPSILON;
    let show_vertical = scroll_metrics.max_vertical_scroll_px > f32::EPSILON;
    if !show_horizontal && !show_vertical {
        return ViewerScrollbarTracks::default();
    }

    let bar_width = if scroll_style.floating && !widget_hovered && !widget_active {
        scroll_style
            .floating_width
            .max(scroll_style.floating_allocated_width)
            .max(2.0)
    } else {
        scroll_style.bar_width.max(2.0)
    };
    let inner_margin = if scroll_style.floating {
        scroll_style.bar_inner_margin
    } else {
        scroll_style.bar_inner_margin.max(1.0)
    };
    let outer_margin = if scroll_style.floating {
        0.0
    } else {
        scroll_style.bar_outer_margin
    };

    ViewerScrollbarTracks {
        vertical: if show_vertical {
            let min_x = content_rect.max.x - outer_margin - bar_width;
            let max_x = content_rect.max.x - outer_margin;
            let max_y = if show_horizontal {
                content_rect.max.y - outer_margin - bar_width - inner_margin
            } else {
                content_rect.max.y - outer_margin
            };
            let min_y = content_rect.min.y + inner_margin;
            Some(Rect::from_min_max(
                Pos2::new(min_x, min_y),
                Pos2::new(max_x, max_y),
            ))
        } else {
            None
        },
        horizontal: if show_horizontal {
            let min_y = content_rect.max.y - outer_margin - bar_width;
            let max_y = content_rect.max.y - outer_margin;
            let max_x = if show_vertical {
                content_rect.max.x - outer_margin - bar_width - inner_margin
            } else {
                content_rect.max.x - outer_margin
            };
            let min_x = content_rect.min.x + inner_margin;
            Some(Rect::from_min_max(
                Pos2::new(min_x, min_y),
                Pos2::new(max_x, max_y),
            ))
        } else {
            None
        },
    }
}

pub(super) fn viewer_visible_text_rect(
    content_rect: Rect,
    scroll_metrics: EditorScrollMetrics,
) -> Option<Rect> {
    let viewport_width = content_rect.width().max(1.0);
    let viewport_height = content_rect.height().max(1.0);
    let content_width = viewport_width + scroll_metrics.max_horizontal_scroll_px;
    let content_height = viewport_height + scroll_metrics.max_vertical_scroll_px;
    let visible_width =
        (content_width - scroll_metrics.current_horizontal_scroll_px).clamp(0.0, viewport_width);
    let visible_height =
        (content_height - scroll_metrics.current_vertical_scroll_px).clamp(0.0, viewport_height);

    if visible_width <= f32::EPSILON || visible_height <= f32::EPSILON {
        None
    } else {
        Some(Rect::from_min_size(
            content_rect.min,
            egui::vec2(visible_width, visible_height),
        ))
    }
}
