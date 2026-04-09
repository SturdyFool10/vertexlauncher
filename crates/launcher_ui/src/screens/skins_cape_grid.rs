use super::*;

pub(super) fn render_cape_grid(ui: &mut Ui, text_ui: &mut TextUi, state: &mut SkinManagerState) {
    let label_font = egui::TextStyle::Body.resolve(ui.style());
    let label_color = ui.visuals().text_color();
    let mut max_label_width = ui
        .painter()
        .layout_no_wrap("No Cape".to_owned(), label_font.clone(), label_color)
        .size()
        .x;
    for cape in &state.available_capes {
        let width = ui
            .painter()
            .layout_no_wrap(cape.label.clone(), label_font.clone(), label_color)
            .size()
            .x;
        max_label_width = max_label_width.max(width);
    }

    let available_width = ui
        .available_width()
        .min(ui.clip_rect().width().max(1.0))
        .max(1.0);
    let tile_gap = egui::vec2(style::SPACE_MD, style::SPACE_MD);
    let tile_width = (max_label_width + 24.0)
        .max(CAPE_TILE_WIDTH_MIN)
        .min(available_width);
    let columns =
        (((available_width + tile_gap.x) / (tile_width + tile_gap.x)).floor() as usize).max(1);
    let total_items = state.available_capes.len() + 1;
    let mut pending_selection = None;
    for row_start in (0..total_items).step_by(columns) {
        let row_end = (row_start + columns).min(total_items);
        let row_count = row_end.saturating_sub(row_start);
        let fallback_row_width =
            (row_count as f32 * tile_width) + (row_count.saturating_sub(1) as f32 * tile_gap.x);
        let row_width_id = egui::Id::new(("skins_cape_row_width", row_start));
        let measured_row_width = ui
            .ctx()
            .data(|data| data.get_temp::<f32>(row_width_id))
            .unwrap_or(fallback_row_width)
            .min(available_width);
        let (row_rect, _) = ui.allocate_exact_size(
            egui::vec2(available_width, CAPE_TILE_HEIGHT),
            Sense::hover(),
        );
        let box_rect = Rect::from_center_size(
            row_rect.center(),
            egui::vec2(measured_row_width, CAPE_TILE_HEIGHT),
        );

        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(box_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing.x = tile_gap.x;
                let content = ui.horizontal(|ui| {
                    for item_index in row_start..row_end {
                        if item_index == 0 {
                            let no_cape_selected = state.pending_cape_id.is_none();
                            if draw_cape_tile(
                                ui,
                                text_ui,
                                tile_width,
                                "No Cape",
                                no_cape_selected,
                                true,
                                None,
                                None,
                            ) {
                                pending_selection = Some(None);
                            }
                            continue;
                        }

                        let cape = &state.available_capes[item_index - 1];
                        let selected = state.pending_cape_id.as_deref() == Some(cape.id.as_str());
                        let preview = cape.texture_bytes.as_deref();
                        if draw_cape_tile(
                            ui,
                            text_ui,
                            tile_width,
                            cape.label.as_str(),
                            selected,
                            false,
                            preview,
                            cape.texture_size,
                        ) {
                            pending_selection = Some(Some(cape.id.clone()));
                        }
                    }
                });
                ui.ctx()
                    .data_mut(|data| data.insert_temp(row_width_id, content.response.rect.width()));
            },
        );

        if row_end < total_items {
            ui.add_space(tile_gap.y);
        }
    }

    if let Some(selection) = pending_selection {
        state.pending_cape_id = selection;
    }
}

fn draw_cape_tile(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    tile_width: f32,
    label: &str,
    selected: bool,
    is_no_cape: bool,
    preview_png: Option<&[u8]>,
    preview_texture_size: Option<[u32; 2]>,
) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(tile_width, CAPE_TILE_HEIGHT), Sense::click());
    let tile_rect = rect.shrink2(egui::vec2(0.0, style::SPACE_XS * 0.5));

    let hover_t = ui
        .ctx()
        .animate_bool(response.id.with("cape_tile_hover"), response.hovered());
    let press_t = ui.ctx().animate_bool(
        response.id.with("cape_tile_press"),
        response.is_pointer_button_down_on(),
    );
    let selected_t = ui
        .ctx()
        .animate_bool(response.id.with("cape_tile_selected"), selected);
    let focused = response.has_focus();

    let fill = if selected || focused {
        ui.visuals().selection.bg_fill.gamma_multiply(
            0.24 + hover_t * 0.06 + press_t * 0.04 + if focused { 0.08 } else { 0.0 },
        )
    } else {
        ui.visuals()
            .widgets
            .inactive
            .bg_fill
            .gamma_multiply(1.0 + hover_t * 0.08 + press_t * 0.04)
    };
    let stroke = if selected || focused {
        let mut stroke = ui.visuals().selection.stroke;
        stroke.width += hover_t * 0.5 + if focused { 0.75 } else { 0.0 };
        stroke
    } else {
        let mut stroke = ui.visuals().widgets.inactive.bg_stroke;
        stroke.color = stroke.color.gamma_multiply(1.0 + hover_t * 0.18);
        stroke
    };

    ui.painter().rect(
        tile_rect,
        CornerRadius::same(10),
        fill,
        stroke,
        egui::StrokeKind::Middle,
    );
    paint_cape_tile_highlight(
        ui,
        tile_rect,
        response.hover_pos().or(response.interact_pointer_pos()),
        hover_t,
        press_t,
        selected_t.max(if focused { 1.0 } else { 0.0 }),
    );

    let preview_rect = Rect::from_min_size(
        egui::pos2(tile_rect.left() + 12.0, tile_rect.top() + 12.0),
        egui::vec2((tile_rect.width() - 24.0).max(0.0), 112.0),
    );

    if is_no_cape {
        ui.painter().rect_stroke(
            preview_rect,
            CornerRadius::same(6),
            Stroke::new(1.5, ui.visuals().weak_text_color()),
            egui::StrokeKind::Middle,
        );
        let dotted_step = 8.0;
        let mut x = preview_rect.left();
        while x <= preview_rect.right() {
            ui.painter().line_segment(
                [
                    egui::pos2(x, preview_rect.top()),
                    egui::pos2((x + 3.0).min(preview_rect.right()), preview_rect.top()),
                ],
                Stroke::new(1.0, ui.visuals().weak_text_color()),
            );
            ui.painter().line_segment(
                [
                    egui::pos2(x, preview_rect.bottom()),
                    egui::pos2((x + 3.0).min(preview_rect.right()), preview_rect.bottom()),
                ],
                Stroke::new(1.0, ui.visuals().weak_text_color()),
            );
            x += dotted_step;
        }
        let mut y = preview_rect.top();
        while y <= preview_rect.bottom() {
            ui.painter().line_segment(
                [
                    egui::pos2(preview_rect.left(), y),
                    egui::pos2(preview_rect.left(), (y + 3.0).min(preview_rect.bottom())),
                ],
                Stroke::new(1.0, ui.visuals().weak_text_color()),
            );
            ui.painter().line_segment(
                [
                    egui::pos2(preview_rect.right(), y),
                    egui::pos2(preview_rect.right(), (y + 3.0).min(preview_rect.bottom())),
                ],
                Stroke::new(1.0, ui.visuals().weak_text_color()),
            );
            y += dotted_step;
        }
    } else if let Some(bytes) = preview_png {
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let uri = format!("bytes://skins/cape/{:016x}.png", hasher.finish());

        if let Some(back_uv) = preview_texture_size.and_then(cape_outer_face_uv) {
            let inner = preview_rect.shrink2(egui::vec2(4.0, 4.0));
            let target_aspect = 10.0 / 16.0;
            let max_h = inner.height();
            let mut face_h = max_h;
            let mut face_w = face_h * target_aspect;
            if face_w > inner.width() {
                face_w = inner.width().max(0.0);
                face_h = face_w / target_aspect;
            }
            let y = inner.center().y - face_h * 0.5;
            let x = inner.center().x - face_w * 0.5;
            let back_rect = Rect::from_min_size(egui::pos2(x, y), egui::vec2(face_w, face_h));

            ui.painter().rect_stroke(
                back_rect,
                CornerRadius::same(4),
                Stroke::new(1.0, ui.visuals().widgets.inactive.bg_stroke.color),
                egui::StrokeKind::Middle,
            );

            if let image_textures::ManagedTextureStatus::Ready(texture) =
                image_textures::request_texture(
                    ui.ctx(),
                    uri.clone(),
                    Arc::<[u8]>::from(bytes.to_vec().into_boxed_slice()),
                    TextureOptions::NEAREST,
                )
            {
                egui::Image::from_texture(&texture)
                    .uv(back_uv)
                    .fit_to_exact_size(back_rect.size())
                    .texture_options(TextureOptions::NEAREST)
                    .paint_at(ui, back_rect);
            }
        } else if let image_textures::ManagedTextureStatus::Ready(texture) =
            image_textures::request_texture(
                ui.ctx(),
                uri,
                Arc::<[u8]>::from(bytes.to_vec().into_boxed_slice()),
                TextureOptions::NEAREST,
            )
        {
            let image = egui::Image::from_texture(&texture)
                .fit_to_exact_size(preview_rect.size())
                .texture_options(TextureOptions::NEAREST);
            image.paint_at(ui, preview_rect);
        }
    } else {
        ui.painter().rect_filled(
            preview_rect,
            CornerRadius::same(6),
            ui.visuals().faint_bg_color,
        );
    }

    let label_rect = Rect::from_min_size(
        Pos2::new(tile_rect.left() + 6.0, tile_rect.bottom() - 44.0),
        egui::vec2(tile_rect.width() - 12.0, 34.0),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(label_rect), |ui| {
        ui.set_clip_rect(label_rect);
        ui.with_layout(
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
            |ui| {
                let mut label_style = style::body(ui);
                label_style.wrap = false;
                let _ = text_ui.label(ui, ("skins_cape_label", label), label, &label_style);
            },
        );
    });

    response.clicked()
}

fn paint_cape_tile_highlight(
    ui: &Ui,
    rect: Rect,
    pointer_pos: Option<Pos2>,
    hover_t: f32,
    press_t: f32,
    selected_t: f32,
) {
    let emphasis = (hover_t * 0.95 + press_t * 0.85 + selected_t * 0.55).clamp(0.0, 1.0);
    if emphasis <= 0.01 {
        return;
    }

    let selection = ui.visuals().selection.bg_fill;
    let glow_rect = rect.shrink2(egui::vec2(1.0, 1.0));
    let glow_center = pointer_pos.unwrap_or_else(|| {
        egui::pos2(
            rect.center().x,
            egui::lerp(
                rect.top() + 28.0..=rect.center().y,
                selected_t.max(hover_t * 0.35),
            ),
        )
    });
    let glow_center = egui::pos2(
        glow_center
            .x
            .clamp(glow_rect.left() + 4.0, glow_rect.right() - 4.0),
        glow_center
            .y
            .clamp(glow_rect.top() + 4.0, glow_rect.bottom() - 4.0),
    );
    let glow_radius = rect.width().max(rect.height()) * egui::lerp(0.34..=0.58, emphasis);
    let center_alpha = (32.0 + hover_t * 34.0 + press_t * 22.0 + selected_t * 10.0) / 255.0;
    let ring_specs = [
        (0.0, center_alpha),
        (0.32, center_alpha * 0.52),
        (0.68, center_alpha * 0.16),
        (1.0, 0.0),
    ];

    let mut mesh = egui::epaint::Mesh::default();
    let center_idx = mesh.vertices.len() as u32;
    let center_color: Color32 = egui::Rgba::from(selection).multiply(center_alpha).into();
    mesh.colored_vertex(glow_center, center_color);

    let segments = 40usize;
    let mut previous_ring = Vec::with_capacity(segments);
    for (ring_index, (radius_t, alpha)) in ring_specs.iter().enumerate().skip(1) {
        let color: Color32 = egui::Rgba::from(selection).multiply(*alpha).into();
        let mut current_ring = Vec::with_capacity(segments);
        for segment in 0..segments {
            let angle = std::f32::consts::TAU * (segment as f32 / segments as f32);
            let unit_x = angle.cos();
            let unit_y = angle.sin();
            let vertex = egui::pos2(
                glow_center.x + unit_x * glow_radius * *radius_t,
                glow_center.y + unit_y * glow_radius * *radius_t,
            );
            let vertex_idx = mesh.vertices.len() as u32;
            mesh.colored_vertex(vertex, color);
            current_ring.push(vertex_idx);
        }

        if ring_index == 1 {
            for segment in 0..segments {
                let next = (segment + 1) % segments;
                mesh.add_triangle(center_idx, current_ring[segment], current_ring[next]);
            }
        } else {
            for segment in 0..segments {
                let next = (segment + 1) % segments;
                mesh.add_triangle(
                    previous_ring[segment],
                    previous_ring[next],
                    current_ring[next],
                );
                mesh.add_triangle(
                    previous_ring[segment],
                    current_ring[next],
                    current_ring[segment],
                );
            }
        }

        previous_ring = current_ring;
    }

    ui.painter()
        .with_clip_rect(glow_rect)
        .add(egui::Shape::mesh(mesh));

    let sheen_rect = Rect::from_min_max(
        glow_rect.min + egui::vec2(0.0, 1.0),
        egui::pos2(glow_rect.max.x, glow_rect.top() + glow_rect.height() * 0.34),
    );
    let sheen_alpha = (14.0 * emphasis) / 255.0;
    ui.painter().rect_filled(
        sheen_rect,
        CornerRadius {
            nw: 10,
            ne: 10,
            sw: 18,
            se: 18,
        },
        egui::Rgba::from_white_alpha(sheen_alpha),
    );
}
