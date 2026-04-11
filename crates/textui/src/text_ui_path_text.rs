use super::*;

impl TextUi {
    pub fn prepare_label_path_layout(
        &mut self,
        text: &str,
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError> {
        let options = core_label_options(options);
        let layout = self.prepare_plain_text_layout(text, &options, width_points_opt, 1.0);
        build_path_layout_from_prepared_layout(
            &layout,
            options.font_size,
            options.line_height,
            path,
            path_options,
        )
    }

    pub fn prepare_rich_text_path_layout(
        &mut self,
        spans: &[RichTextSpan],
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError> {
        let options = core_label_options(options);
        let layout = self.prepare_rich_text_layout(spans, &options, width_points_opt, 1.0);
        build_path_layout_from_prepared_layout(
            &layout,
            options.font_size,
            options.line_height,
            path,
            path_options,
        )
    }

    pub fn export_label_as_shapes(
        &mut self,
        text: &str,
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
    ) -> VectorTextShape {
        let options = core_label_options(options);
        let layout = self.prepare_plain_text_layout(text, &options, width_points_opt, 1.0);
        export_prepared_layout_as_shapes(
            &layout,
            &mut self.font_system,
            &mut self.scale_context,
            options.line_height,
            self.graphics_config.rasterization,
        )
    }

    pub fn export_rich_text_as_shapes(
        &mut self,
        spans: &[RichTextSpan],
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
    ) -> VectorTextShape {
        let options = core_label_options(options);
        let layout = self.prepare_rich_text_layout(spans, &options, width_points_opt, 1.0);
        export_prepared_layout_as_shapes(
            &layout,
            &mut self.font_system,
            &mut self.scale_context,
            options.line_height,
            self.graphics_config.rasterization,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn paint_label_on_path(
        &mut self,
        painter: &egui::Painter,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError> {
        let scale = painter.pixels_per_point();
        let layout = self.get_or_prepare_label_layout(
            Id::new(id_source).with("textui_path_label"),
            text,
            options,
            width_points_opt,
            scale,
        );
        self.paint_prepared_layout_on_path(
            painter,
            &layout,
            options.font_size,
            options.line_height,
            path,
            path_options,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn paint_rich_text_on_path(
        &mut self,
        painter: &egui::Painter,
        id_source: impl Hash,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError> {
        let scale = painter.pixels_per_point();
        let layout = self.get_or_prepare_rich_layout(
            Id::new(id_source).with("textui_path_rich"),
            spans,
            options,
            width_points_opt,
            scale,
        );
        self.paint_prepared_layout_on_path(
            painter,
            &layout,
            options.font_size,
            options.line_height,
            path,
            path_options,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn paint_prepared_layout_on_path(
        &mut self,
        painter: &egui::Painter,
        layout: &PreparedTextLayout,
        fallback_advance_points: f32,
        line_height_points: f32,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError> {
        let path_layout = build_path_layout_from_prepared_layout(
            layout,
            fallback_advance_points,
            line_height_points,
            path,
            path_options,
        )?;
        let scale = painter.pixels_per_point();
        let graphics_config = self.resolved_graphics_config(self.max_texture_side_px.max(1));
        let field_range_px = graphics_config.rasterization.field_range_px.max(1.0);
        let mut quads = Vec::with_capacity(layout.glyphs.len());

        for (glyph, path_glyph) in layout.glyphs.iter().zip(path_layout.glyphs.iter()) {
            let content_mode = self.resolved_glyph_content_mode(graphics_config, &glyph.cache_key);
            let raster_key = glyph
                .cache_key
                .for_content_mode(content_mode, field_range_px);
            let Some(atlas_entry) = self.glyph_atlas.resolve_or_queue(
                painter.ctx(),
                &mut self.font_system,
                &mut self.scale_context,
                raster_key,
                self.current_frame,
            ) else {
                continue;
            };

            let size_points = egui::vec2(
                atlas_entry.size_px[0] as f32 / scale,
                atlas_entry.size_px[1] as f32 / scale,
            );
            let origin_offset = egui::vec2(
                atlas_entry.placement_left_px as f32 / scale,
                -(atlas_entry.placement_top_px as f32) / scale,
            );
            let tint = if atlas_entry.is_color {
                Color32::WHITE
            } else {
                glyph.color
            };

            quads.push(PaintTextQuad {
                page_index: atlas_entry.page_index,
                positions: rotated_quad_positions(
                    egui_point_from_text(path_glyph.anchor),
                    origin_offset,
                    size_points,
                    path_glyph.rotation_radians,
                ),
                uvs: uv_quad_points(atlas_entry.uv),
                tint,
                content_mode: atlas_entry.content_mode,
            });
        }

        self.paint_text_quads(painter, egui_rect_from_text(path_layout.bounds), &quads);

        Ok(path_layout)
    }

    #[allow(dead_code)]
    pub(crate) fn prepare_label_path_scene(
        &mut self,
        ctx: &Context,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextRenderScene, TextPathError> {
        let scale = ctx.pixels_per_point();
        let layout = self.get_or_prepare_label_layout(
            Id::new(id_source).with("textui_prepare_path_label_scene"),
            text,
            options,
            width_points_opt,
            scale,
        );
        self.build_text_scene_on_path(
            ctx,
            &layout,
            options.font_size,
            options.line_height,
            scale,
            path,
            path_options,
        )
    }

    pub fn prepare_label_path_gpu_scene_at_scale(
        &mut self,
        id_source: impl Hash,
        text: &str,
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextGpuScene, TextPathError> {
        let options = core_label_options(options);
        let layout = self.get_or_prepare_label_layout(
            Id::new(id_source).with("textui_prepare_path_label_gpu_scene"),
            text,
            &options,
            width_points_opt,
            scale,
        );
        self.build_text_gpu_scene_on_path(
            &layout,
            options.font_size,
            options.line_height,
            scale,
            path,
            path_options,
        )
    }

    pub fn prepare_rich_text_path_gpu_scene_at_scale(
        &mut self,
        id_source: impl Hash,
        spans: &[RichTextSpan],
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextGpuScene, TextPathError> {
        let options = core_label_options(options);
        let layout = self.get_or_prepare_rich_layout(
            Id::new(id_source).with("textui_prepare_path_rich_gpu_scene"),
            spans,
            &options,
            width_points_opt,
            scale,
        );
        self.build_text_gpu_scene_on_path(
            &layout,
            options.font_size,
            options.line_height,
            scale,
            path,
            path_options,
        )
    }

    pub fn prepare_rich_text_path_scene(
        &mut self,
        ctx: &Context,
        id_source: impl Hash,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextRenderScene, TextPathError> {
        let scale = ctx.pixels_per_point();
        let layout = self.get_or_prepare_rich_layout(
            Id::new(id_source).with("textui_prepare_path_rich_scene"),
            spans,
            options,
            width_points_opt,
            scale,
        );
        self.build_text_scene_on_path(
            ctx,
            &layout,
            options.font_size,
            options.line_height,
            scale,
            path,
            path_options,
        )
    }
}
