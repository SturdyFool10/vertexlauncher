use super::*;

const GAMEPAD_SCROLL_DELTA_ID: &str = "textui_gamepad_scroll_delta";

fn gamepad_scroll_delta(ctx: &egui::Context) -> egui::Vec2 {
    ctx.data_mut(|data| {
        data.get_temp::<egui::Vec2>(egui::Id::new(GAMEPAD_SCROLL_DELTA_ID))
            .unwrap_or(egui::Vec2::ZERO)
    })
}

pub(super) fn apply_gamepad_scroll_if_focused(ui: &Ui, response: &Response) {
    if response.has_focus() {
        let delta = gamepad_scroll_delta(ui.ctx());
        if delta != egui::Vec2::ZERO {
            ui.scroll_with_delta(delta);
        }
    }
}

impl TextUi {
    pub(super) fn handle_input_events(
        &mut self,
        ui: &Ui,
        response: &Response,
        editor: &mut Editor<'static>,
        multiline: bool,
        content_rect: Rect,
        scale: f32,
        preferred_cursor_x_px: &mut Option<f32>,
        fundamentals: &TextFundamentals,
        process_keyboard: bool,
        scroll_metrics: &mut EditorScrollMetrics,
    ) -> bool {
        let mut changed = false;
        let modifiers = ui.ctx().input(|i| i.modifiers);
        let horizontal_scroll = editor_horizontal_scroll(editor);

        if let Some(pointer_pos) = response.interact_pointer_pos() {
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

        if process_keyboard {
            for event in &self.frame_events {
                match event {
                    TextInputEvent::Text(text) => {
                        let mut text = text.clone();
                        if !multiline {
                            text = text.replace(['\n', '\r'], "");
                        }
                        if !text.is_empty() {
                            *preferred_cursor_x_px = None;
                            editor.insert_string(&text, None);
                            changed = true;
                        }
                    }
                    TextInputEvent::Copy => {
                        if let Some(selection) = editor.copy_selection() {
                            copy_sanitized(ui.ctx(), selection);
                        }
                    }
                    TextInputEvent::Cut => {
                        if let Some(selection) = editor.copy_selection() {
                            copy_sanitized(ui.ctx(), selection);
                            changed |= editor.delete_selection();
                            if changed {
                                *preferred_cursor_x_px = None;
                            }
                        }
                    }
                    TextInputEvent::Paste(pasted) => {
                        let mut pasted = pasted.clone();
                        if !multiline {
                            pasted = pasted.replace(['\n', '\r'], " ");
                        }
                        if !pasted.is_empty() {
                            *preferred_cursor_x_px = None;
                            editor.insert_string(&pasted, None);
                            changed = true;
                        }
                    }
                    TextInputEvent::PointerButton {
                        button: TextPointerButton::Middle,
                        pressed: true,
                        ..
                    } if response.hovered()
                        || response.has_focus()
                        || response.is_pointer_button_down_on() =>
                    {
                        if let Ok(mut cb) = arboard::Clipboard::new() {
                            if let Ok(paste_text) = cb.get_text() {
                                let paste_text = if multiline {
                                    paste_text
                                } else {
                                    paste_text.replace(['\n', '\r'], " ")
                                };
                                if !paste_text.is_empty() {
                                    *preferred_cursor_x_px = None;
                                    editor.insert_string(&paste_text, None);
                                    changed = true;
                                }
                            }
                        }
                    }
                    TextInputEvent::Key {
                        key,
                        pressed,
                        modifiers,
                    } if *pressed => {
                        changed |= handle_editor_key_event(
                            &mut self.font_system,
                            editor,
                            egui_key_from_text(*key),
                            egui_modifiers_from_text(*modifiers),
                            multiline,
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
            self.adjust_editor_horizontal_scroll(
                editor,
                0.0,
                scroll_metrics.max_horizontal_scroll_px,
            );
            *scroll_metrics = self.measure_editor_scroll_metrics(editor, fundamentals, scale);
        }

        changed
    }
}
