use super::*;

impl TextUi {
    #[allow(dead_code)]
    pub(crate) fn label(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
    ) -> Response {
        self.label_impl(ui, id_source, text, options, Sense::hover(), false)
    }

    #[allow(dead_code)]
    pub(crate) fn clickable_label(
        &mut self,
        ui: &mut Ui,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
    ) -> Response {
        self.label_impl(ui, id_source, text, options, Sense::click(), false)
    }

    #[allow(dead_code)]
    pub(crate) fn measure_text_size(
        &mut self,
        ui: &Ui,
        text: &str,
        options: &LabelOptions,
    ) -> Vec2 {
        self.measure_text_size_at_scale(
            ui.ctx().pixels_per_point(),
            text,
            &TextLabelOptions {
                font_size: options.font_size,
                line_height: options.line_height,
                color: options.color.into(),
                wrap: options.wrap,
                monospace: options.monospace,
                weight: options.weight,
                italic: options.italic,
                padding: options.padding.into(),
                fundamentals: options.fundamentals.clone(),
                ellipsis: options.ellipsis.clone(),
            },
        )
        .into()
    }

    pub fn measure_text_size_at_scale(
        &mut self,
        scale: f32,
        text: &str,
        options: &TextLabelOptions,
    ) -> TextVector {
        let options = core_label_options(options);
        let metrics = Metrics::new(
            (self.effective_font_size(options.font_size) * scale).max(1.0),
            (self.effective_line_height(options.line_height) * scale).max(1.0),
        );
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let attrs_owned = self.build_text_attrs_owned(
            &SpanStyle {
                color: options.color.into(),
                monospace: options.monospace,
                italic: options.italic,
                weight: options.weight,
            },
            options.font_size,
            options.line_height,
            &options.fundamentals,
        );

        {
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            borrowed.set_wrap(if options.wrap {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            });
            let attrs = attrs_owned.as_attrs();
            borrowed.set_text(text, &attrs, Shaping::Advanced, None);
            borrowed.shape_until_scroll(true);
        }

        let (width_px, height_px) = measure_buffer_pixels(&buffer);
        TextVector::new(width_px as f32 / scale, height_px as f32 / scale)
    }

    pub(crate) fn get_or_prepare_label_layout(
        &mut self,
        cache_id: Id,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Arc<PreparedTextLayout> {
        let binned_width = width_points_opt.map(|w| snap_width_to_bin(w.max(1.0), scale));

        let mut hasher = new_fingerprint_hasher();
        "prepare_label".hash(&mut hasher);
        text.hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.wrap.hash(&mut hasher);
        options.monospace.hash(&mut hasher);
        options.weight.hash(&mut hasher);
        options.italic.hash(&mut hasher);
        options.color.hash(&mut hasher);
        hash_text_fundamentals(&options.fundamentals, &mut hasher);
        scale.to_bits().hash(&mut hasher);
        binned_width
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();

        self.get_cached_prepared_layout(cache_id, fingerprint)
            .unwrap_or_else(|| {
                let layout =
                    Arc::new(self.prepare_plain_text_layout(text, options, binned_width, scale));
                self.cache_prepared_layout(cache_id, fingerprint, Arc::clone(&layout));
                layout
            })
    }

    pub(crate) fn get_or_prepare_rich_layout(
        &mut self,
        cache_id: Id,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Arc<PreparedTextLayout> {
        let binned_width = width_points_opt.map(|w| snap_width_to_bin(w.max(1.0), scale));

        let mut hasher = new_fingerprint_hasher();
        "prepare_rich".hash(&mut hasher);
        for span in spans {
            span.text.hash(&mut hasher);
            span.style.color.hash(&mut hasher);
            span.style.monospace.hash(&mut hasher);
            span.style.italic.hash(&mut hasher);
            span.style.weight.hash(&mut hasher);
        }
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.wrap.hash(&mut hasher);
        hash_text_fundamentals(&options.fundamentals, &mut hasher);
        scale.to_bits().hash(&mut hasher);
        binned_width
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();

        self.get_cached_prepared_layout(cache_id, fingerprint)
            .unwrap_or_else(|| {
                let layout =
                    Arc::new(self.prepare_rich_text_layout(spans, options, binned_width, scale));
                self.cache_prepared_layout(cache_id, fingerprint, Arc::clone(&layout));
                layout
            })
    }

    pub(crate) fn prepare_label_scene(
        &mut self,
        ctx: &Context,
        id_source: impl Hash,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
    ) -> TextRenderScene {
        let scale = ctx.pixels_per_point();
        let layout = self.get_or_prepare_label_layout(
            Id::new(id_source).with("textui_prepare_label_scene"),
            text,
            options,
            width_points_opt,
            scale,
        );
        self.build_text_scene_from_layout(ctx, &layout, scale)
    }

    #[allow(dead_code)]
    pub(crate) fn prepare_rich_text_scene(
        &mut self,
        ctx: &Context,
        id_source: impl Hash,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
    ) -> TextRenderScene {
        let scale = ctx.pixels_per_point();
        let layout = self.get_or_prepare_rich_layout(
            Id::new(id_source).with("textui_prepare_rich_scene"),
            spans,
            options,
            width_points_opt,
            scale,
        );
        self.build_text_scene_from_layout(ctx, &layout, scale)
    }

    pub fn prepare_label_gpu_scene_at_scale(
        &mut self,
        id_source: impl Hash,
        text: &str,
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Arc<TextGpuScene> {
        let options = core_label_options(options);
        let binned_width = width_points_opt.map(|w| snap_width_to_bin(w.max(1.0), scale));
        let mut hasher = new_fingerprint_hasher();
        "label_gpu_scene".hash(&mut hasher);
        text.hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.wrap.hash(&mut hasher);
        options.monospace.hash(&mut hasher);
        options.weight.hash(&mut hasher);
        options.italic.hash(&mut hasher);
        options.color.hash(&mut hasher);
        hash_text_fundamentals(&options.fundamentals, &mut hasher);
        scale.to_bits().hash(&mut hasher);
        binned_width
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();

        if let Some(scene) = self
            .gpu_scene_cache
            .write(|state| state.touch(&fingerprint).map(|e| Arc::clone(&e.value)))
        {
            return scene;
        }

        let layout = self.get_or_prepare_label_layout(
            Id::new(id_source).with("textui_prepare_label_gpu_scene"),
            text,
            &options,
            width_points_opt,
            scale,
        );
        let mut scene = Arc::new(self.build_text_gpu_scene_from_layout(&layout, scale));
        if let Some(s) = Arc::get_mut(&mut scene) {
            s.fingerprint = fingerprint;
        }
        let approx_bytes = gpu_scene_approx_bytes(&scene);
        self.gpu_scene_cache.write(|state| {
            let _ = state.insert(fingerprint, Arc::clone(&scene), approx_bytes);
        });
        scene
    }

    pub fn prepare_rich_text_gpu_scene_at_scale(
        &mut self,
        id_source: impl Hash,
        spans: &[RichTextSpan],
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Arc<TextGpuScene> {
        let options = core_label_options(options);
        let binned_width = width_points_opt.map(|w| snap_width_to_bin(w.max(1.0), scale));
        let mut hasher = new_fingerprint_hasher();
        "rich_gpu_scene".hash(&mut hasher);
        for span in spans {
            span.text.hash(&mut hasher);
            span.style.color.hash(&mut hasher);
            span.style.monospace.hash(&mut hasher);
            span.style.italic.hash(&mut hasher);
            span.style.weight.hash(&mut hasher);
        }
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.wrap.hash(&mut hasher);
        hash_text_fundamentals(&options.fundamentals, &mut hasher);
        scale.to_bits().hash(&mut hasher);
        binned_width
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();

        if let Some(scene) = self
            .gpu_scene_cache
            .write(|state| state.touch(&fingerprint).map(|e| Arc::clone(&e.value)))
        {
            return scene;
        }

        let layout = self.get_or_prepare_rich_layout(
            Id::new(id_source).with("textui_prepare_rich_gpu_scene"),
            spans,
            &options,
            width_points_opt,
            scale,
        );
        let mut scene = Arc::new(self.build_text_gpu_scene_from_layout(&layout, scale));
        if let Some(s) = Arc::get_mut(&mut scene) {
            s.fingerprint = fingerprint;
        }
        let approx_bytes = gpu_scene_approx_bytes(&scene);
        self.gpu_scene_cache.write(|state| {
            let _ = state.insert(fingerprint, Arc::clone(&scene), approx_bytes);
        });
        scene
    }

    pub fn prepare_label_gpu_scene_async_at_scale(
        &mut self,
        _id_source: impl Hash,
        text: &str,
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Option<Arc<TextGpuScene>> {
        let options = core_label_options(options);
        let binned_width = width_points_opt.map(|w| snap_width_to_bin(w.max(1.0), scale));
        let mut hasher = new_fingerprint_hasher();
        "label_async_gpu_scene".hash(&mut hasher);
        text.hash(&mut hasher);
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.wrap.hash(&mut hasher);
        options.monospace.hash(&mut hasher);
        options.weight.hash(&mut hasher);
        options.italic.hash(&mut hasher);
        options.color.hash(&mut hasher);
        hash_text_fundamentals(&options.fundamentals, &mut hasher);
        scale.to_bits().hash(&mut hasher);
        binned_width
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();

        if let Some(scene) = self
            .gpu_scene_cache
            .write(|state| state.touch(&fingerprint).map(|e| Arc::clone(&e.value)))
        {
            return Some(scene);
        }

        let layout_opt = self.get_or_queue_async_plain_layout(
            fingerprint,
            text.to_owned(),
            &options,
            binned_width,
            scale,
        );
        layout_opt.map(|layout| {
            let scene = Arc::new(self.build_text_gpu_scene_from_layout(&layout, scale));
            let approx_bytes = gpu_scene_approx_bytes(&scene);
            self.gpu_scene_cache.write(|state| {
                let _ = state.insert(fingerprint, Arc::clone(&scene), approx_bytes);
            });
            scene
        })
    }

    pub fn prepare_rich_text_gpu_scene_async_at_scale(
        &mut self,
        id_source: impl Hash,
        spans: &[RichTextSpan],
        options: &TextLabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Option<Arc<TextGpuScene>> {
        let options = core_label_options(options);
        let _cache_id = Id::new(id_source).with("textui_prepare_rich_gpu_scene_async");
        let binned_width = width_points_opt.map(|w| snap_width_to_bin(w.max(1.0), scale));
        let mut hasher = new_fingerprint_hasher();
        "prepare_rich_gpu_async".hash(&mut hasher);
        for span in spans {
            span.text.hash(&mut hasher);
            span.style.color.hash(&mut hasher);
            span.style.monospace.hash(&mut hasher);
            span.style.italic.hash(&mut hasher);
            span.style.weight.hash(&mut hasher);
        }
        options.font_size.to_bits().hash(&mut hasher);
        options.line_height.to_bits().hash(&mut hasher);
        options.wrap.hash(&mut hasher);
        hash_text_fundamentals(&options.fundamentals, &mut hasher);
        scale.to_bits().hash(&mut hasher);
        binned_width
            .map(f32::to_bits)
            .unwrap_or(0)
            .hash(&mut hasher);
        self.hash_typography(&mut hasher);
        let fingerprint = hasher.finish();

        if let Some(scene) = self
            .gpu_scene_cache
            .write(|state| state.touch(&fingerprint).map(|e| Arc::clone(&e.value)))
        {
            return Some(scene);
        }

        let layout_opt = self.get_or_queue_async_rich_layout(
            fingerprint,
            spans.to_vec(),
            &options,
            binned_width,
            scale,
        );
        layout_opt.map(|layout| {
            let scene = Arc::new(self.build_text_gpu_scene_from_layout(&layout, scale));
            let approx_bytes = gpu_scene_approx_bytes(&scene);
            self.gpu_scene_cache.write(|state| {
                let _ = state.insert(fingerprint, Arc::clone(&scene), approx_bytes);
            });
            scene
        })
    }
}
