use super::*;

impl TextUi {
    #[doc(hidden)]
    pub fn egui_singleline_input(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &mut String,
        options: &InputOptions,
    ) -> Response {
        self.input_widget(ui, id_source, text, options, false)
    }

    #[doc(hidden)]
    pub fn egui_multiline_input(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &mut String,
        options: &InputOptions,
    ) -> Response {
        self.input_widget(ui, id_source, text, options, true)
    }

    #[doc(hidden)]
    pub fn egui_multiline_rich_viewer(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        spans: &[RichTextSpan],
        options: &InputOptions,
        stick_to_bottom: bool,
        wrap: bool,
    ) -> Response {
        let id = ui.make_persistent_id(id_source).with("textui_rich_viewer");
        let width = options
            .desired_width
            .unwrap_or_else(|| ui.available_width())
            .max(options.min_width);
        let min_height = options.line_height + (options.padding.y * 2.0);
        let height = (options.line_height * options.desired_rows.max(1) as f32
            + options.padding.y * 2.0)
            .max(min_height);

        let desired_size = egui::vec2(width, height);
        let rect = ui.allocate_space(desired_size).1;
        let mut response = ui.interact(rect, id, Sense::click_and_drag());

        let has_focus = response.has_focus();
        if has_focus {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        tab: true,
                        escape: false,
                    },
                );
            });
        }
        let scale = ui.ctx().pixels_per_point();
        let content_rect = rect.shrink2(options.padding);
        let content_width_px = (content_rect.width() * scale).max(1.0);
        let content_height_px = (content_rect.height() * scale).max(1.0);
        let text = spans
            .iter()
            .map(|span| span.text.as_str())
            .collect::<String>();
        let attrs_fingerprint = self.rich_viewer_attrs_fingerprint(spans, options, scale, wrap);

        let mut state = self
            .input_states
            .remove(&id)
            .unwrap_or_else(|| Self::new_input_state(&mut self.font_system, &text, true));

        let needs_text_sync =
            state.last_text != text || state.attrs_fingerprint != attrs_fingerprint;
        if needs_text_sync {
            state.scroll_metrics = self.replace_editor_rich_text(
                &mut state.editor,
                spans,
                options,
                content_width_px,
                content_height_px,
                scale,
                wrap,
            );
            state.last_text = text.clone();
            state.attrs_fingerprint = attrs_fingerprint;
            state.preferred_cursor_x_px = None;
            if stick_to_bottom && !has_focus && !response.hovered() {
                scroll_editor_to_buffer_end(&mut self.font_system, &mut state.editor);
            }
        } else {
            state.scroll_metrics = self.configure_viewer(
                &mut state.editor,
                options,
                content_width_px,
                content_height_px,
                scale,
                wrap,
            );
        }

        let pointer_pos = response.interact_pointer_pos();
        let scrollbar_tracks = viewer_scrollbar_track_rects(
            ui.style().spacing.scroll,
            response.hovered(),
            response.is_pointer_button_down_on(),
            content_rect,
            state.scroll_metrics,
        );
        let pointer_over_scrollbar = pointer_pos.is_some_and(|pos| scrollbar_tracks.contains(pos));
        let pointer_over_text = pointer_pos.is_some_and(|pos| {
            viewer_visible_text_rect(content_rect, state.scroll_metrics)
                .is_some_and(|text_rect| text_rect.contains(pos))
        }) && !pointer_over_scrollbar;
        let pointer_pressed_on_widget =
            ui.ctx().input(|i| i.pointer.primary_pressed()) && response.is_pointer_button_down_on();

        if (response.clicked() || pointer_pressed_on_widget) && !pointer_over_scrollbar {
            response.request_focus();
        }

        if pointer_over_text {
            ui.output_mut(|o| {
                o.cursor_icon = egui::CursorIcon::Text;
                o.mutable_text_under_cursor = true;
            });
        }

        let pointer_interacted = !pointer_over_scrollbar
            && (pointer_pressed_on_widget
                || response.clicked()
                || response.double_clicked()
                || response.triple_clicked()
                || response.drag_started()
                || response.dragged());

        let mut state_changed = if has_focus || response.hovered() || pointer_interacted {
            self.handle_viewer_events(
                ui,
                &response,
                &mut state.editor,
                content_rect,
                scale,
                &mut state.preferred_cursor_x_px,
                &options.fundamentals,
                has_focus,
                pointer_over_scrollbar,
                &mut state.scroll_metrics,
            )
        } else {
            false
        };

        let frame_fill = if has_focus {
            options
                .background_color_focused
                .or(options.background_color_hovered)
                .unwrap_or(options.background_color)
        } else if response.hovered() {
            options
                .background_color_hovered
                .unwrap_or(options.background_color)
        } else {
            options.background_color
        };
        let frame_stroke = if has_focus {
            options
                .stroke_focused
                .or(options.stroke_hovered)
                .unwrap_or(options.stroke)
        } else if response.hovered() {
            options.stroke_hovered.unwrap_or(options.stroke)
        } else {
            options.stroke
        };
        let corner_radius = CornerRadius::same(options.corner_radius);

        ui.painter().rect_filled(rect, corner_radius, frame_fill);
        ui.painter()
            .rect_stroke(rect, corner_radius, frame_stroke, egui::StrokeKind::Inside);

        {
            let painter = ui
                .painter()
                .with_clip_rect(content_rect.intersect(ui.clip_rect()));
            self.paint_editor_gpu(
                &painter,
                content_rect,
                &state.editor,
                options,
                scale,
                false,
                true,
            );
        }

        state_changed |= self.sync_viewer_scrollbars(
            ui,
            id,
            &mut state.editor,
            content_rect,
            scale,
            &options.fundamentals,
            &mut state.scroll_metrics,
        );

        self.input_states.insert(id, state);
        if state_changed {
            response.mark_changed();
        }
        apply_gamepad_scroll_if_focused(ui, &response);

        response
    }

    pub(crate) fn input_widget(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &mut String,
        options: &InputOptions,
        multiline: bool,
    ) -> Response {
        let id = ui.make_persistent_id(id_source).with("textui_input");
        let width = options
            .desired_width
            .unwrap_or_else(|| ui.available_width())
            .max(options.min_width);

        let min_height = options.line_height + (options.padding.y * 2.0);
        let height = if multiline {
            (options.line_height * options.desired_rows.max(2) as f32 + options.padding.y * 2.0)
                .max(min_height)
        } else {
            min_height
        };

        let desired_size = egui::vec2(width, height);
        let rect = ui.allocate_space(desired_size).1;
        let mut response = ui.interact(rect, id, Sense::click_and_drag());

        if response.hovered() {
            ui.output_mut(|o| {
                o.cursor_icon = egui::CursorIcon::Text;
                o.mutable_text_under_cursor = true;
            });
        }

        if response.clicked() {
            response.request_focus();
        }

        let has_focus = response.has_focus();
        if has_focus {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        tab: multiline,
                        escape: false,
                    },
                );
            });
        }
        let scale = ui.ctx().pixels_per_point();
        let content_rect = rect.shrink2(options.padding);
        let content_width_px = (content_rect.width() * scale).max(1.0);
        let content_height_px = (content_rect.height() * scale).max(1.0);
        let attrs_fingerprint = self.input_attrs_fingerprint(options, scale);

        let mut state = self
            .input_states
            .remove(&id)
            .unwrap_or_else(|| Self::new_input_state(&mut self.font_system, text, multiline));

        if state.multiline != multiline {
            state = Self::new_input_state(&mut self.font_system, text, multiline);
        }

        let needs_text_sync = !has_focus && state.last_text != *text;
        let needs_attrs_sync = state.attrs_fingerprint != attrs_fingerprint;
        if needs_text_sync || needs_attrs_sync {
            state.scroll_metrics = self.replace_editor_text(
                &mut state.editor,
                text,
                options,
                multiline,
                content_width_px,
                content_height_px,
                scale,
            );
            state.last_text.clone_from(text);
            state.attrs_fingerprint = attrs_fingerprint;
            state.preferred_cursor_x_px = None;
        }

        state.scroll_metrics = self.configure_editor(
            &mut state.editor,
            options,
            multiline,
            content_width_px,
            content_height_px,
            scale,
        );

        let pointer_interacted = response.clicked()
            || response.double_clicked()
            || response.triple_clicked()
            || response.dragged();

        let mut changed = false;
        if has_focus || pointer_interacted {
            let (undo_pressed, redo_pressed) = if has_focus {
                ui.input(|i| {
                    let undo = i.key_pressed(Key::Z) && i.modifiers.command && !i.modifiers.shift;
                    let redo = (i.key_pressed(Key::Y) && i.modifiers.command)
                        || (i.key_pressed(Key::Z) && i.modifiers.command && i.modifiers.shift);
                    (undo, redo)
                })
            } else {
                (false, false)
            };

            if undo_pressed {
                if let Some(UndoEntry {
                    text: undo_text,
                    cursor: undo_cursor,
                    selection: undo_sel,
                }) = state.undo_stack.pop()
                {
                    let snap = UndoEntry {
                        text: editor_to_string(&state.editor),
                        cursor: state.editor.cursor(),
                        selection: state.editor.selection(),
                    };
                    state.redo_stack.push(snap);
                    state.scroll_metrics = self.replace_editor_text(
                        &mut state.editor,
                        &undo_text,
                        options,
                        multiline,
                        content_width_px,
                        content_height_px,
                        scale,
                    );
                    state
                        .editor
                        .set_cursor(clamp_cursor_to_editor(&state.editor, undo_cursor));
                    state
                        .editor
                        .set_selection(clamp_selection_to_editor(&state.editor, undo_sel));
                    state.last_text = undo_text;
                    state.last_undo_op = UndoOpKind::None;
                    state.preferred_cursor_x_px = None;
                    changed = true;
                }
            } else if redo_pressed {
                if let Some(UndoEntry {
                    text: redo_text,
                    cursor: redo_cursor,
                    selection: redo_sel,
                }) = state.redo_stack.pop()
                {
                    let snap = UndoEntry {
                        text: editor_to_string(&state.editor),
                        cursor: state.editor.cursor(),
                        selection: state.editor.selection(),
                    };
                    push_undo(&mut state.undo_stack, snap);
                    state.scroll_metrics = self.replace_editor_text(
                        &mut state.editor,
                        &redo_text,
                        options,
                        multiline,
                        content_width_px,
                        content_height_px,
                        scale,
                    );
                    state
                        .editor
                        .set_cursor(clamp_cursor_to_editor(&state.editor, redo_cursor));
                    state
                        .editor
                        .set_selection(clamp_selection_to_editor(&state.editor, redo_sel));
                    state.last_text = redo_text;
                    state.last_undo_op = UndoOpKind::None;
                    state.preferred_cursor_x_px = None;
                    changed = true;
                }
            } else {
                if has_focus {
                    let pending_op = pending_modify_op(&self.frame_events);
                    if pending_op != UndoOpKind::None {
                        let should_push = matches!(pending_op, UndoOpKind::Paste | UndoOpKind::Cut)
                            || state.last_undo_op != pending_op;
                        if should_push {
                            push_undo(
                                &mut state.undo_stack,
                                UndoEntry {
                                    text: editor_to_string(&state.editor),
                                    cursor: state.editor.cursor(),
                                    selection: state.editor.selection(),
                                },
                            );
                            state.redo_stack.clear();
                        }
                        state.last_undo_op = pending_op;
                    } else if self.frame_events.iter().any(is_navigation_event) {
                        state.last_undo_op = UndoOpKind::None;
                    }
                }

                changed |= self.handle_input_events(
                    ui,
                    &response,
                    &mut state.editor,
                    multiline,
                    content_rect,
                    scale,
                    &mut state.preferred_cursor_x_px,
                    &options.fundamentals,
                    has_focus,
                    &mut state.scroll_metrics,
                );
            }

            if !multiline && ui.input(|i| i.key_pressed(Key::Enter)) {
                response.surrender_focus();
            }
        }

        let mut ctx_cut = false;
        let mut ctx_copy = false;
        let mut ctx_paste = false;
        let mut ctx_select_all = false;
        response.context_menu(|menu| {
            let has_selection = state.editor.selection() != Selection::None;
            let button_options = ButtonOptions {
                font_size: 14.0,
                line_height: 18.0,
                text_color: menu.visuals().text_color(),
                fill: Color32::TRANSPARENT,
                fill_hovered: menu.visuals().widgets.hovered.bg_fill,
                fill_active: menu.visuals().widgets.active.bg_fill,
                fill_selected: Color32::TRANSPARENT,
                stroke: egui::Stroke::NONE,
                corner_radius: 4,
                padding: egui::vec2(8.0, 4.0),
                min_size: egui::vec2(menu.available_width().max(96.0), 26.0),
            };
            if menu
                .add_enabled_ui(has_selection, |menu| {
                    self.button(menu, "textui_input_context_cut", "Cut", &button_options)
                })
                .inner
                .clicked()
            {
                ctx_cut = true;
                menu.close();
            }
            if menu
                .add_enabled_ui(has_selection, |menu| {
                    self.button(menu, "textui_input_context_copy", "Copy", &button_options)
                })
                .inner
                .clicked()
            {
                ctx_copy = true;
                menu.close();
            }
            if self
                .button(menu, "textui_input_context_paste", "Paste", &button_options)
                .clicked()
            {
                ctx_paste = true;
                menu.close();
            }
            menu.separator();
            if self
                .button(
                    menu,
                    "textui_input_context_select_all",
                    "Select All",
                    &button_options,
                )
                .clicked()
            {
                ctx_select_all = true;
                menu.close();
            }
        });
        if ctx_cut {
            if let Some(sel) = state.editor.copy_selection() {
                push_undo(
                    &mut state.undo_stack,
                    UndoEntry {
                        text: editor_to_string(&state.editor),
                        cursor: state.editor.cursor(),
                        selection: state.editor.selection(),
                    },
                );
                state.redo_stack.clear();
                state.last_undo_op = UndoOpKind::None;
                copy_sanitized(ui.ctx(), sel);
                state.editor.delete_selection();
                state.preferred_cursor_x_px = None;
                changed = true;
            }
        }
        if ctx_copy {
            if let Some(sel) = state.editor.copy_selection() {
                copy_sanitized(ui.ctx(), sel);
            }
        }
        if ctx_paste {
            if let Ok(mut cb) = arboard::Clipboard::new() {
                if let Ok(paste_text) = cb.get_text() {
                    let paste_text = if multiline {
                        paste_text
                    } else {
                        paste_text.replace(['\n', '\r'], " ")
                    };
                    if !paste_text.is_empty() {
                        push_undo(
                            &mut state.undo_stack,
                            UndoEntry {
                                text: editor_to_string(&state.editor),
                                cursor: state.editor.cursor(),
                                selection: state.editor.selection(),
                            },
                        );
                        state.redo_stack.clear();
                        state.last_undo_op = UndoOpKind::None;
                        state.editor.insert_string(&paste_text, None);
                        state.preferred_cursor_x_px = None;
                        changed = true;
                    }
                }
            }
        }
        if ctx_select_all {
            state.preferred_cursor_x_px = None;
            changed |= select_all(&mut state.editor);
        }

        let latest_text = editor_to_string(&state.editor);
        if latest_text != *text {
            *text = latest_text.clone();
            state.last_text = latest_text;
            state.preferred_cursor_x_px = None;
            changed = true;
        }

        if changed {
            response.mark_changed();
        }

        state.last_used_frame = self.current_frame;

        let frame_fill = if has_focus {
            options
                .background_color_focused
                .or(options.background_color_hovered)
                .unwrap_or(options.background_color)
        } else if response.hovered() {
            options
                .background_color_hovered
                .unwrap_or(options.background_color)
        } else {
            options.background_color
        };
        let frame_stroke = if has_focus {
            options
                .stroke_focused
                .or(options.stroke_hovered)
                .unwrap_or(options.stroke)
        } else if response.hovered() {
            options.stroke_hovered.unwrap_or(options.stroke)
        } else {
            options.stroke
        };
        let corner_radius = CornerRadius::same(options.corner_radius);

        ui.painter().rect_filled(rect, corner_radius, frame_fill);
        ui.painter()
            .rect_stroke(rect, corner_radius, frame_stroke, egui::StrokeKind::Inside);

        {
            let painter = ui
                .painter()
                .with_clip_rect(content_rect.intersect(ui.clip_rect()));
            self.paint_editor_gpu(
                &painter,
                content_rect,
                &state.editor,
                options,
                scale,
                has_focus,
                false,
            );
        }
        self.input_states.insert(id, state);
        if !has_focus
            && text.is_empty()
            && let Some(placeholder_text) = options
                .placeholder_text
                .as_deref()
                .filter(|placeholder| !placeholder.is_empty())
        {
            let placeholder_style = LabelOptions {
                font_size: options.font_size,
                line_height: options.line_height,
                color: options
                    .placeholder_color
                    .unwrap_or_else(|| options.text_color.gamma_multiply(0.5)),
                wrap: multiline,
                monospace: options.monospace,
                fundamentals: options.fundamentals.clone(),
                ..LabelOptions::default()
            };
            let placeholder_scene = self.prepare_label_scene(
                ui.ctx(),
                id.with("placeholder"),
                placeholder_text,
                &placeholder_style,
                multiline.then_some(content_rect.width()),
            );
            let placeholder_size = egui_vec_from_text(placeholder_scene.size_points);
            let y_offset = if multiline {
                0.0
            } else {
                ((content_rect.height() - placeholder_size.y) * 0.5).max(0.0)
            };
            let placeholder_rect = Rect::from_min_size(
                Pos2::new(content_rect.min.x, content_rect.min.y + y_offset),
                placeholder_size.min(content_rect.size()),
            );
            let painter = ui
                .painter()
                .with_clip_rect(content_rect.intersect(ui.clip_rect()));
            self.paint_scene_in_rect(&painter, placeholder_rect, &placeholder_scene);
        }

        apply_gamepad_scroll_if_focused(ui, &response);

        response
    }

    pub(crate) fn new_input_state(
        font_system: &mut FontSystem,
        text: &str,
        multiline: bool,
    ) -> InputState {
        let mut buffer = Buffer::new(font_system, Metrics::new(16.0, 22.0));
        {
            let mut borrowed = buffer.borrow_with(font_system);
            borrowed.set_wrap(if multiline {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            });
            borrowed.set_text(text, &Attrs::new(), Shaping::Advanced, None);
            borrowed.shape_until_scroll(true);
        }

        InputState {
            editor: Editor::new(buffer),
            last_text: text.to_owned(),
            attrs_fingerprint: 0,
            multiline,
            preferred_cursor_x_px: None,
            scroll_metrics: EditorScrollMetrics::default(),
            last_used_frame: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_undo_op: UndoOpKind::None,
        }
    }

    pub(crate) fn replace_editor_text(
        &mut self,
        editor: &mut Editor<'static>,
        text: &str,
        options: &InputOptions,
        multiline: bool,
        width_px: f32,
        height_px: f32,
        scale: f32,
    ) -> EditorScrollMetrics {
        let attrs_owned = self.input_attrs_owned(options, scale);
        let effective_font_size = self.effective_font_size(options.font_size) * scale;
        let effective_line_height = self.effective_line_height(options.line_height) * scale;
        let previous_cursor = editor.cursor();
        let previous_selection = editor.selection();
        let previous_scroll = editor.with_buffer(|buffer| buffer.scroll());
        let mut scroll_metrics = EditorScrollMetrics::default();
        editor.with_buffer_mut(|buffer| {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            borrowed.set_metrics_and_size(
                Metrics::new(effective_font_size, effective_line_height),
                Some(width_px),
                Some(height_px),
            );
            borrowed.set_wrap(if multiline {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            });
            let attrs = attrs_owned.as_attrs();
            borrowed.set_text(text, &attrs, Shaping::Advanced, None);
            borrowed.set_scroll(previous_scroll);
            borrowed.shape_until_scroll(true);
            scroll_metrics =
                clamp_borrowed_buffer_scroll(&mut borrowed, &options.fundamentals, scale);
        });
        editor.set_cursor(clamp_cursor_to_editor(editor, previous_cursor));
        editor.set_selection(clamp_selection_to_editor(editor, previous_selection));
        scroll_metrics
    }

    pub(crate) fn configure_editor(
        &mut self,
        editor: &mut Editor<'static>,
        options: &InputOptions,
        multiline: bool,
        width_px: f32,
        height_px: f32,
        scale: f32,
    ) -> EditorScrollMetrics {
        let effective_font_size = self.effective_font_size(options.font_size) * scale;
        let effective_line_height = self.effective_line_height(options.line_height) * scale;
        let mut scroll_metrics = EditorScrollMetrics::default();
        editor.with_buffer_mut(|buffer| {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            borrowed.set_metrics_and_size(
                Metrics::new(effective_font_size, effective_line_height),
                Some(width_px),
                Some(height_px),
            );
            borrowed.set_wrap(if multiline {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            });
            borrowed.shape_until_scroll(true);
            scroll_metrics =
                clamp_borrowed_buffer_scroll(&mut borrowed, &options.fundamentals, scale);
        });
        scroll_metrics
    }

    pub(crate) fn replace_editor_rich_text(
        &mut self,
        editor: &mut Editor<'static>,
        spans: &[RichTextSpan],
        options: &InputOptions,
        width_px: f32,
        height_px: f32,
        scale: f32,
        wrap: bool,
    ) -> EditorScrollMetrics {
        let effective_font_size = self.effective_font_size(options.font_size) * scale;
        let effective_line_height = self.effective_line_height(options.line_height) * scale;
        let previous_cursor = editor.cursor();
        let previous_selection = editor.selection();
        let previous_scroll = editor.with_buffer(|buffer| buffer.scroll());
        let default_attrs = self.input_attrs_owned(options, scale);
        let span_attrs_owned = spans
            .iter()
            .map(|span| self.input_span_attrs_owned(&span.style, options, scale))
            .collect::<Vec<_>>();
        let mut scroll_metrics = EditorScrollMetrics::default();

        editor.with_buffer_mut(|buffer| {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            borrowed.set_metrics_and_size(
                Metrics::new(effective_font_size, effective_line_height),
                Some(width_px),
                Some(height_px),
            );
            borrowed.set_wrap(if wrap { Wrap::WordOrGlyph } else { Wrap::None });
            let rich_text = spans
                .iter()
                .zip(span_attrs_owned.iter())
                .map(|(span, attrs)| (span.text.as_str(), attrs.as_attrs()))
                .collect::<Vec<_>>();
            borrowed.set_rich_text(
                rich_text,
                &default_attrs.as_attrs(),
                Shaping::Advanced,
                None,
            );
            borrowed.set_scroll(previous_scroll);
            borrowed.shape_until_scroll(true);
            scroll_metrics =
                clamp_borrowed_buffer_scroll(&mut borrowed, &options.fundamentals, scale);
        });
        editor.set_cursor(clamp_cursor_to_editor(editor, previous_cursor));
        editor.set_selection(clamp_selection_to_editor(editor, previous_selection));
        scroll_metrics
    }

    pub(crate) fn configure_viewer(
        &mut self,
        editor: &mut Editor<'static>,
        options: &InputOptions,
        width_px: f32,
        height_px: f32,
        scale: f32,
        wrap: bool,
    ) -> EditorScrollMetrics {
        let effective_font_size = self.effective_font_size(options.font_size) * scale;
        let effective_line_height = self.effective_line_height(options.line_height) * scale;
        let mut scroll_metrics = EditorScrollMetrics::default();
        editor.with_buffer_mut(|buffer| {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            borrowed.set_metrics_and_size(
                Metrics::new(effective_font_size, effective_line_height),
                Some(width_px),
                Some(height_px),
            );
            borrowed.set_wrap(if wrap { Wrap::WordOrGlyph } else { Wrap::None });
            borrowed.shape_until_scroll(true);
            scroll_metrics =
                clamp_borrowed_buffer_scroll(&mut borrowed, &options.fundamentals, scale);
        });
        scroll_metrics
    }

    pub(crate) fn handle_viewer_events(
        &mut self,
        ui: &Ui,
        response: &Response,
        editor: &mut Editor<'static>,
        content_rect: Rect,
        scale: f32,
        preferred_cursor_x_px: &mut Option<f32>,
        fundamentals: &TextFundamentals,
        process_keyboard: bool,
        pointer_over_scrollbar: bool,
        scroll_metrics: &mut EditorScrollMetrics,
    ) -> bool {
        let mut changed = false;
        let (modifiers, primary_pressed, smooth_scroll_delta) = ui.ctx().input(|i| {
            (
                i.modifiers,
                i.pointer.primary_pressed(),
                i.smooth_scroll_delta,
            )
        });
        let pointer_pressed_on_widget = primary_pressed && response.is_pointer_button_down_on();
        let horizontal_scroll = editor_horizontal_scroll(editor);

        if !pointer_over_scrollbar && let Some(pointer_pos) = response.interact_pointer_pos() {
            let x =
                (((pointer_pos.x - content_rect.min.x) * scale) + horizontal_scroll).round() as i32;
            let y = ((pointer_pos.y - content_rect.min.y) * scale).round() as i32;

            if response.triple_clicked() {
                changed |= triple_click_editor_to_pointer(
                    editor,
                    x,
                    y,
                    preferred_cursor_x_px,
                    fundamentals,
                    scale,
                );
            } else if response.double_clicked() {
                changed |= double_click_editor_to_pointer(
                    editor,
                    x,
                    y,
                    preferred_cursor_x_px,
                    fundamentals,
                    scale,
                );
            } else if pointer_pressed_on_widget {
                if modifiers.shift {
                    changed |= extend_selection_to_pointer(
                        editor,
                        x,
                        y,
                        preferred_cursor_x_px,
                        fundamentals,
                        scale,
                    );
                } else {
                    changed |= click_editor_to_pointer(
                        editor,
                        x,
                        y,
                        preferred_cursor_x_px,
                        fundamentals,
                        scale,
                    );
                }
            } else if response.clicked() {
                if modifiers.shift {
                    changed |= extend_selection_to_pointer(
                        editor,
                        x,
                        y,
                        preferred_cursor_x_px,
                        fundamentals,
                        scale,
                    );
                } else {
                    changed |= click_editor_to_pointer(
                        editor,
                        x,
                        y,
                        preferred_cursor_x_px,
                        fundamentals,
                        scale,
                    );
                }
            }

            if response.dragged() {
                changed |= drag_editor_selection_to_pointer(
                    editor,
                    x,
                    y,
                    preferred_cursor_x_px,
                    fundamentals,
                    scale,
                );
            }
        }

        if response.hovered() {
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

            if !horizontal_uses_vertical_wheel && vertical_scroll_delta.abs() > f32::EPSILON {
                editor
                    .borrow_with(&mut self.font_system)
                    .action(Action::Scroll {
                        pixels: -vertical_scroll_delta * scale,
                    });
                changed = true;
            }
            if horizontal_scroll_delta.abs() > f32::EPSILON {
                self.adjust_editor_horizontal_scroll(
                    editor,
                    -horizontal_scroll_delta * scale,
                    scroll_metrics.max_horizontal_scroll_px,
                );
                changed = true;
            }
        }

        if process_keyboard {
            for event in &self.frame_events {
                match event {
                    TextInputEvent::Copy | TextInputEvent::Cut => {
                        if let Some(selection) = editor.copy_selection() {
                            copy_sanitized(ui.ctx(), selection);
                        }
                    }
                    TextInputEvent::Key {
                        key,
                        pressed,
                        modifiers,
                    } if *pressed => {
                        changed |= handle_read_only_editor_key_event(
                            &mut self.font_system,
                            editor,
                            egui_key_from_text(*key),
                            egui_modifiers_from_text(*modifiers),
                            preferred_cursor_x_px,
                            fundamentals,
                            scale,
                        );
                    }
                    _ => {}
                }
            }
        }

        if changed {
            editor
                .borrow_with(&mut self.font_system)
                .shape_as_needed(false);
            *scroll_metrics = self.measure_editor_scroll_metrics(editor, fundamentals, scale);
        }

        changed
    }

    pub(crate) fn adjust_editor_horizontal_scroll(
        &mut self,
        editor: &mut Editor<'static>,
        delta_px: f32,
        max_horizontal_scroll_px: f32,
    ) {
        editor.with_buffer_mut(|buffer| {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            let mut scroll = borrowed.scroll();
            scroll.horizontal = (scroll.horizontal + delta_px).clamp(0.0, max_horizontal_scroll_px);
            borrowed.set_scroll(scroll);
            borrowed.shape_until_scroll(true);
        });
    }

    pub(crate) fn adjust_editor_vertical_scroll(
        &mut self,
        editor: &mut Editor<'static>,
        delta_px: f32,
    ) {
        editor.with_buffer_mut(|buffer| {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            let mut scroll = borrowed.scroll();
            scroll.vertical += delta_px;
            borrowed.set_scroll(scroll);
            borrowed.shape_until_scroll(true);
        });
    }

    pub(crate) fn measure_editor_scroll_metrics(
        &mut self,
        editor: &mut Editor<'static>,
        fundamentals: &TextFundamentals,
        scale: f32,
    ) -> EditorScrollMetrics {
        editor.with_buffer_mut(|buffer| {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            measure_borrowed_buffer_scroll_metrics(&mut borrowed, fundamentals, scale)
        })
    }

    pub(crate) fn sync_viewer_scrollbars(
        &mut self,
        ui: &mut Ui,
        id: Id,
        editor: &mut Editor<'static>,
        content_rect: Rect,
        scale: f32,
        fundamentals: &TextFundamentals,
        scroll_metrics: &mut EditorScrollMetrics,
    ) -> bool {
        let has_horizontal_scroll = scroll_metrics.max_horizontal_scroll_px > f32::EPSILON;
        let has_vertical_scroll = scroll_metrics.max_vertical_scroll_px > f32::EPSILON;
        if !has_horizontal_scroll && !has_vertical_scroll {
            return false;
        }

        let content_width_points =
            content_rect.width() + (scroll_metrics.max_horizontal_scroll_px / scale.max(1.0));
        let content_height_points =
            content_rect.height() + (scroll_metrics.max_vertical_scroll_px / scale.max(1.0));
        let current_horizontal_scroll_points = scroll_metrics.current_horizontal_scroll_px / scale;
        let current_vertical_scroll_points = scroll_metrics.current_vertical_scroll_px / scale;
        let scroll_output = ui
            .scope_builder(egui::UiBuilder::new().max_rect(content_rect), |ui| {
                egui::ScrollArea::both()
                    .id_salt(id.with("egui_scrollbars"))
                    .max_width(content_rect.width())
                    .max_height(content_rect.height())
                    .scroll_source(egui::containers::scroll_area::ScrollSource::SCROLL_BAR)
                    .scroll_bar_visibility(
                        egui::containers::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                    )
                    .scroll_offset(egui::vec2(
                        current_horizontal_scroll_points,
                        current_vertical_scroll_points,
                    ))
                    .show_viewport(ui, |ui, _viewport| {
                        ui.allocate_space(egui::vec2(
                            content_width_points.max(content_rect.width()),
                            content_height_points.max(content_rect.height()),
                        ));
                    })
            })
            .inner;
        let next_horizontal_scroll_px = (scroll_output.state.offset.x * scale)
            .clamp(0.0, scroll_metrics.max_horizontal_scroll_px);
        let next_vertical_scroll_px = (scroll_output.state.offset.y * scale)
            .clamp(0.0, scroll_metrics.max_vertical_scroll_px);
        let horizontal_delta_px =
            next_horizontal_scroll_px - scroll_metrics.current_horizontal_scroll_px;
        let vertical_delta_px = next_vertical_scroll_px - scroll_metrics.current_vertical_scroll_px;

        let horizontal_changed = horizontal_delta_px.abs() > 0.25;
        let vertical_changed = vertical_delta_px.abs() > 0.25;
        if !horizontal_changed && !vertical_changed {
            return false;
        }

        if horizontal_changed {
            self.adjust_editor_horizontal_scroll(
                editor,
                horizontal_delta_px,
                scroll_metrics.max_horizontal_scroll_px,
            );
        }
        if vertical_changed {
            self.adjust_editor_vertical_scroll(editor, vertical_delta_px);
        }
        *scroll_metrics = self.measure_editor_scroll_metrics(editor, fundamentals, scale);
        ui.ctx().request_repaint();
        true
    }

    pub(crate) fn paint_editor_gpu(
        &mut self,
        painter: &egui::Painter,
        content_rect: Rect,
        editor: &Editor<'static>,
        options: &InputOptions,
        scale: f32,
        has_focus: bool,
        show_selection_without_focus: bool,
    ) {
        let horizontal_scroll_px = editor.with_buffer(|b| b.scroll().horizontal.max(0.0));
        let selection_visible =
            has_focus || (show_selection_without_focus && editor.selection() != Selection::None);
        let selection_bounds = if selection_visible {
            editor.selection_bounds()
        } else {
            None
        };

        let origin = content_rect.min;
        let painter = painter.with_clip_rect(content_rect);

        struct GlyphCmd {
            cache_key: GlyphRasterKey,
            x_px: f32,
            y_px: f32,
            color: Color32,
        }

        let mut sel_rects: Vec<Rect> = Vec::new();
        let mut cursor_rect: Option<Rect> = None;
        let mut glyph_cmds: Vec<GlyphCmd> = Vec::new();
        let variation_settings = shared_variation_settings(&options.fundamentals);

        editor.with_buffer(|buffer| {
            let buf_width = buffer.size().0.unwrap_or(0.0);

            for run in buffer.layout_runs() {
                let line_i = run.line_i;
                let line_top = run.line_top;
                let line_y = run.line_y;
                let line_height = run.line_height;
                let prefixes = collect_glyph_spacing_prefixes_px(
                    run.text,
                    run.glyphs,
                    &options.fundamentals,
                    scale,
                );

                if let Some((start, end)) = selection_bounds
                    && line_i >= start.line
                    && line_i <= end.line
                {
                    let mut range_opt: Option<(i32, i32)> = None;

                    for (glyph_index, glyph) in run.glyphs.iter().enumerate() {
                        let cluster = &run.text[glyph.start..glyph.end];
                        let total = cluster.grapheme_indices(true).count().max(1);
                        let mut c_x = adjusted_glyph_x_px(glyph, prefixes[glyph_index]);
                        let c_w = glyph.w / total as f32;

                        for (i, c) in cluster.grapheme_indices(true) {
                            let c_start = glyph.start + i;
                            let c_end = glyph.start + i + c.len();
                            if (start.line != line_i || c_end > start.index)
                                && (end.line != line_i || c_start < end.index)
                            {
                                range_opt = match range_opt.take() {
                                    Some((mn, mx)) => {
                                        Some((mn.min(c_x as i32), mx.max((c_x + c_w) as i32)))
                                    }
                                    None => Some((c_x as i32, (c_x + c_w) as i32)),
                                };
                            } else if let Some((mn, mx)) = range_opt.take() {
                                sel_rects.push(editor_sel_rect(
                                    mn,
                                    mx,
                                    line_top,
                                    line_height,
                                    horizontal_scroll_px,
                                    origin,
                                    scale,
                                ));
                            }
                            c_x += c_w;
                        }
                    }

                    if run.glyphs.is_empty() && end.line > line_i {
                        range_opt = Some((0, buf_width as i32));
                    }

                    if let Some((mut mn, mut mx)) = range_opt.take() {
                        if end.line > line_i {
                            if run.rtl {
                                mn = 0;
                            } else {
                                mx = buf_width as i32;
                            }
                        }
                        sel_rects.push(editor_sel_rect(
                            mn,
                            mx,
                            line_top,
                            line_height,
                            horizontal_scroll_px,
                            origin,
                            scale,
                        ));
                    }
                }

                if has_focus
                    && let Some(cx) =
                        editor_cursor_x_in_run(&editor.cursor(), &run, &options.fundamentals, scale)
                {
                    let x_pts = (cx as f32 - horizontal_scroll_px) / scale + origin.x;
                    let y_pts = line_top / scale + origin.y;
                    let h_pts = line_height / scale;
                    cursor_rect = Some(Rect::from_min_size(
                        Pos2::new(x_pts, y_pts),
                        Vec2::new((1.0_f32 / scale).max(0.5), h_pts),
                    ));
                }

                for (glyph_index, glyph) in run.glyphs.iter().enumerate() {
                    let physical = glyph.physical((0.0, 0.0), 1.0);
                    let color = if selection_visible {
                        if let Some((start, end)) = selection_bounds {
                            if line_i >= start.line
                                && line_i <= end.line
                                && (start.line != line_i || glyph.end > start.index)
                                && (end.line != line_i || glyph.start < end.index)
                            {
                                options.selected_text_color
                            } else {
                                glyph
                                    .color_opt
                                    .map_or(options.text_color, cosmic_to_egui_color)
                            }
                        } else {
                            glyph
                                .color_opt
                                .map_or(options.text_color, cosmic_to_egui_color)
                        }
                    } else {
                        glyph
                            .color_opt
                            .map_or(options.text_color, cosmic_to_egui_color)
                    };

                    glyph_cmds.push(GlyphCmd {
                        cache_key: GlyphRasterKey::new(
                            physical.cache_key,
                            scale,
                            options.fundamentals.stem_darkening,
                            GlyphContentMode::AlphaMask,
                            0.0,
                            Arc::clone(&variation_settings),
                        ),
                        x_px: physical.x as f32 + prefixes[glyph_index] - horizontal_scroll_px,
                        y_px: line_y + physical.y as f32,
                        color,
                    });
                }
            }
        });

        for sel in sel_rects {
            painter.add(egui::Shape::rect_filled(
                sel,
                CornerRadius::ZERO,
                options.selection_color,
            ));
        }

        let graphics_config = self.resolved_graphics_config(self.max_texture_side_px.max(1));
        let field_range_px = graphics_config.rasterization.field_range_px.max(1.0);
        let mut quads = Vec::with_capacity(glyph_cmds.len());
        for cmd in glyph_cmds {
            let content_mode = self.resolved_glyph_content_mode(graphics_config, &cmd.cache_key);
            let raster_key = cmd.cache_key.for_content_mode(content_mode, field_range_px);
            let Some(atlas_entry) = self.glyph_atlas.resolve_or_queue(
                painter.ctx(),
                &mut self.font_system,
                &mut self.scale_context,
                raster_key,
                self.current_frame,
            ) else {
                continue;
            };

            let glyph_rect = Rect::from_min_size(
                Pos2::new(
                    (cmd.x_px + atlas_entry.placement_left_px as f32) / scale + origin.x,
                    (cmd.y_px - atlas_entry.placement_top_px as f32) / scale + origin.y,
                ),
                Vec2::new(
                    atlas_entry.size_px[0] as f32 / scale,
                    atlas_entry.size_px[1] as f32 / scale,
                ),
            );

            let tint = if atlas_entry.is_color {
                Color32::WHITE
            } else {
                cmd.color
            };

            quads.push(PaintTextQuad {
                page_index: atlas_entry.page_index,
                positions: quad_positions_from_min_size(glyph_rect.min, glyph_rect.size()),
                uvs: uv_quad_points(atlas_entry.uv),
                tint,
                content_mode: atlas_entry.content_mode,
            });
        }

        self.paint_text_quads(&painter, content_rect, &quads);

        if let Some(cursor_rect) = cursor_rect {
            painter.add(egui::Shape::rect_filled(
                cursor_rect,
                CornerRadius::ZERO,
                options.cursor_color,
            ));
        }
    }

    pub(crate) fn input_span_attrs_owned(
        &self,
        style: &RichTextStyle,
        options: &InputOptions,
        scale: f32,
    ) -> AttrsOwned {
        let mut attrs = Attrs::new()
            .color(to_cosmic_text_color(style.color))
            .weight(Weight(self.effective_weight(style.weight)))
            .metrics(Metrics::new(
                (self.effective_font_size(options.font_size) * scale).max(1.0),
                (self.effective_line_height(options.line_height) * scale).max(1.0),
            ));

        if style.monospace {
            attrs = attrs.family(Family::Monospace);
        } else if let Some(family) = self.ui_font_family.as_deref() {
            attrs = attrs.family(Family::Name(family));
        }
        if style.italic {
            attrs = attrs.style(FontStyle::Italic);
        }
        if let Some(features) =
            compose_font_features(&self.open_type_feature_tags, &options.fundamentals)
        {
            attrs = attrs.font_features(features);
        }

        AttrsOwned::new(&attrs)
    }

    pub(crate) fn rich_viewer_attrs_fingerprint(
        &self,
        spans: &[RichTextSpan],
        options: &InputOptions,
        scale: f32,
        wrap: bool,
    ) -> u64 {
        let mut hasher = new_fingerprint_hasher();
        "rich_viewer_attrs".hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        scale.to_bits().hash(&mut hasher);
        wrap.hash(&mut hasher);
        hash_text_fundamentals(&options.fundamentals, &mut hasher);
        self.ui_font_family.hash(&mut hasher);
        self.ui_font_size_scale.to_bits().hash(&mut hasher);
        self.ui_font_weight.hash(&mut hasher);
        self.open_type_features_enabled.hash(&mut hasher);
        self.open_type_features_to_enable.hash(&mut hasher);
        for span in spans {
            span.text.hash(&mut hasher);
            span.style.color.hash(&mut hasher);
            span.style.monospace.hash(&mut hasher);
            span.style.italic.hash(&mut hasher);
            span.style.weight.hash(&mut hasher);
        }
        hasher.finish()
    }

    pub(crate) fn input_attrs_owned(&self, options: &InputOptions, scale: f32) -> AttrsOwned {
        let mut attrs = Attrs::new()
            .color(to_cosmic_color(options.text_color))
            .metrics(Metrics::new(
                (self.effective_font_size(options.font_size) * scale).max(1.0),
                (self.effective_line_height(options.line_height) * scale).max(1.0),
            ))
            .weight(Weight(self.effective_weight(400)));

        if options.monospace {
            attrs = attrs.family(Family::Monospace);
        } else if let Some(family) = self.ui_font_family.as_deref() {
            attrs = attrs.family(Family::Name(family));
        }
        if let Some(features) =
            compose_font_features(&self.open_type_feature_tags, &options.fundamentals)
        {
            attrs = attrs.font_features(features);
        }

        AttrsOwned::new(&attrs)
    }

    pub(crate) fn input_attrs_fingerprint(&self, options: &InputOptions, scale: f32) -> u64 {
        let mut hasher = new_fingerprint_hasher();
        "input_attrs".hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.text_color.hash(&mut hasher);
        options.monospace.hash(&mut hasher);
        hash_text_fundamentals(&options.fundamentals, &mut hasher);
        scale.to_bits().hash(&mut hasher);
        self.ui_font_family.hash(&mut hasher);
        self.ui_font_size_scale.to_bits().hash(&mut hasher);
        self.ui_font_weight.hash(&mut hasher);
        self.open_type_features_enabled.hash(&mut hasher);
        self.open_type_features_to_enable.hash(&mut hasher);
        hasher.finish()
    }
}
