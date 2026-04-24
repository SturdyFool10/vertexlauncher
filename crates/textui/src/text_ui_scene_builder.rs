use super::*;

fn target_format_is_hdr(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgb10a2Unorm
    )
}

impl TextUi {
    pub(crate) fn get_cached_prepared_layout(
        &mut self,
        id: Id,
        fingerprint: u64,
    ) -> Option<Arc<PreparedTextLayout>> {
        let current_frame = self.current_frame;
        self.prepared_texts.write(|state| {
            let entry = state.touch(&id)?;
            if entry.value.fingerprint != fingerprint {
                return None;
            }
            entry.value.last_used_frame = current_frame;
            Some(Arc::clone(&entry.value.layout))
        })
    }

    pub(crate) fn cache_prepared_layout(
        &mut self,
        id: Id,
        fingerprint: u64,
        layout: Arc<PreparedTextLayout>,
    ) {
        let approx_bytes = layout.approx_bytes;
        let current_frame = self.current_frame;
        self.prepared_texts.write(|state| {
            let _ = state.insert(
                id,
                PreparedTextCacheEntry {
                    fingerprint,
                    layout,
                    last_used_frame: current_frame,
                },
                approx_bytes,
            );
        });
    }

    pub(crate) fn prepare_plain_text_layout(
        &mut self,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> PreparedTextLayout {
        let spans = vec![RichSpan {
            text: text.to_owned(),
            style: SpanStyle {
                color: options.color.into(),
                monospace: options.monospace,
                italic: options.italic,
                weight: options.weight,
            },
        }];
        self.prepare_rich_text_layout(&spans, options, width_points_opt, scale)
    }

    pub(crate) fn prepare_rich_text_layout(
        &mut self,
        spans: &[RichSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> PreparedTextLayout {
        let metrics = Metrics::new(
            (self.effective_font_size(options.font_size) * scale).max(1.0),
            (self.effective_line_height(options.line_height) * scale).max(1.0),
        );

        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let default_attrs_owned = self.build_text_attrs_owned(
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
        let span_attrs_owned = spans
            .iter()
            .map(|span| {
                self.build_text_attrs_owned(
                    &span.style,
                    options.font_size,
                    options.line_height,
                    &options.fundamentals,
                )
            })
            .collect::<Vec<_>>();

        {
            let width_px_opt = width_points_opt.map(|w| (w * scale).max(1.0));
            let mut borrowed = buffer.borrow_with(&mut self.font_system);
            borrowed.set_wrap(if options.wrap {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            });
            borrowed.set_size(width_px_opt, None);
            let rich_text = spans
                .iter()
                .zip(span_attrs_owned.iter())
                .map(|(span, attrs)| (span.text.as_str(), attrs.as_attrs()))
                .collect::<Vec<_>>();
            let default_attrs = default_attrs_owned.as_attrs();
            borrowed.set_rich_text(rich_text, &default_attrs, Shaping::Advanced, None);
            borrowed.shape_until_scroll(true);
        }

        let (mut measured_width_px, measured_height_px) = measure_buffer_pixels(&buffer);
        if let Some(width_points) = width_points_opt {
            measured_width_px = (width_points * scale).ceil() as usize;
        }

        self.prepare_text_layout_from_buffer(
            &buffer,
            measured_width_px.max(1),
            measured_height_px.max(1),
            scale,
            options.color,
            options.fundamentals.stem_darkening,
            &options.fundamentals,
        )
    }

    pub(crate) fn prepare_text_layout_from_buffer(
        &self,
        buffer: &Buffer,
        width_px: usize,
        height_px: usize,
        scale: f32,
        default_color: Color32,
        stem_darkening: bool,
        fundamentals: &TextFundamentals,
    ) -> PreparedTextLayout {
        let mut effective_fundamentals = fundamentals.clone();
        effective_fundamentals.stem_darkening = stem_darkening;
        let (glyphs, extra_width_points) = collect_prepared_glyphs_from_buffer(
            buffer,
            scale,
            default_color,
            &effective_fundamentals,
        );
        let approx_bytes = glyphs.len().saturating_mul(mem::size_of::<PreparedGlyph>());
        PreparedTextLayout {
            glyphs: Arc::from(glyphs),
            size_points: egui::vec2(
                width_px as f32 / scale + extra_width_points,
                height_px as f32 / scale,
            ),
            approx_bytes,
        }
    }

    pub(crate) fn build_text_scene_from_layout(
        &mut self,
        ctx: &Context,
        layout: &PreparedTextLayout,
        scale: f32,
    ) -> TextRenderScene {
        let graphics_config = self.resolved_graphics_config(self.max_texture_side_px.max(1));
        let field_range_px = graphics_config.rasterization.field_range_px.max(1.0);
        let mut quads = Vec::with_capacity(layout.glyphs.len());
        let mut bounds: Option<Rect> = None;

        for glyph in layout.glyphs.iter() {
            let content_mode = self.resolved_glyph_content_mode(graphics_config, &glyph.cache_key);
            let raster_key = glyph
                .cache_key
                .for_content_mode(content_mode, field_range_px);
            let Some(atlas_entry) = self.glyph_atlas.resolve_sync(
                ctx,
                &mut self.font_system,
                &mut self.scale_context,
                raster_key,
                self.current_frame,
            ) else {
                continue;
            };

            let min = Pos2::new(
                glyph.offset_points.x + atlas_entry.placement_left_px as f32 / scale,
                glyph.offset_points.y - atlas_entry.placement_top_px as f32 / scale,
            );
            let size_points = egui::vec2(
                atlas_entry.size_px[0] as f32 / scale,
                atlas_entry.size_px[1] as f32 / scale,
            );
            let positions = quad_positions_from_min_size(min, size_points);
            let quad_bounds = rect_from_points(positions);
            bounds = Some(bounds.map_or(quad_bounds, |current| current.union(quad_bounds)));
            quads.push(TextAtlasQuad {
                atlas_page_index: atlas_entry.page_index,
                positions: positions.map(Into::into),
                uvs: uv_quad_points(atlas_entry.uv).map(Into::into),
                tint: if atlas_entry.is_color {
                    Color32::WHITE.into()
                } else {
                    glyph.color.into()
                },
                is_color: atlas_entry.is_color,
            });
        }

        TextRenderScene {
            quads,
            bounds: bounds.unwrap_or(Rect::NOTHING).into(),
            size_points: layout.size_points.into(),
        }
    }

    pub(crate) fn build_text_gpu_scene_from_layout(
        &mut self,
        layout: &PreparedTextLayout,
        scale: f32,
    ) -> TextGpuScene {
        let target_page_side_px =
            default_gpu_scene_page_side(self.resolved_graphics_config(self.max_texture_side_px));
        let graphics_config = self.resolved_graphics_config(self.max_texture_side_px);
        let mut page_pool = std::mem::take(&mut self.cpu_page_pool);
        let mut pages = Vec::<CpuSceneAtlasPage>::new();
        let mut page_hashers = Vec::<FxHasher>::new();
        let mut quads = Vec::with_capacity(layout.glyphs.len());
        let mut bounds: Option<Rect> = None;

        for glyph in layout.glyphs.iter() {
            let Some(atlas_glyph) = self.get_or_rasterize_gpu_scene_glyph(
                &glyph.cache_key,
                graphics_config.rasterization,
                graphics_config.atlas_padding_px,
            ) else {
                continue;
            };

            let allocation_size = size2(
                atlas_glyph.upload_image.size[0] as i32,
                atlas_glyph.upload_image.size[1] as i32,
            );
            let Some((page_index, allocation)) = allocate_cpu_scene_page_slot(
                &mut pages,
                &mut page_pool,
                target_page_side_px,
                allocation_size,
            ) else {
                continue;
            };

            // Initialize hasher for any newly created pages.
            while page_hashers.len() < pages.len() {
                let new_idx = page_hashers.len();
                let mut h = FxHasher::default();
                new_idx.hash(&mut h);
                pages[new_idx].size.hash(&mut h);
                page_hashers.push(h);
            }

            let pos = [
                allocation.rectangle.min.x.max(0) as usize,
                allocation.rectangle.min.y.max(0) as usize,
            ];
            // Hash glyph identity + position — O(glyphs) instead of O(pixels) at finalisation.
            glyph.cache_key.hash(&mut page_hashers[page_index]);
            pos[0].hash(&mut page_hashers[page_index]);
            pos[1].hash(&mut page_hashers[page_index]);

            let page_size = pages[page_index].size;
            blit_to_page(
                &mut pages[page_index].rgba8,
                page_size,
                &atlas_glyph.upload_image,
                pos[0],
                pos[1],
            );

            let uv = Rect::from_min_max(
                Pos2::new(
                    (pos[0] + graphics_config.atlas_padding_px) as f32 / page_size[0] as f32,
                    (pos[1] + graphics_config.atlas_padding_px) as f32 / page_size[1] as f32,
                ),
                Pos2::new(
                    (pos[0] + graphics_config.atlas_padding_px + atlas_glyph.size_px[0]) as f32
                        / page_size[0] as f32,
                    (pos[1] + graphics_config.atlas_padding_px + atlas_glyph.size_px[1]) as f32
                        / page_size[1] as f32,
                ),
            );

            let min = Pos2::new(
                glyph.offset_points.x + atlas_glyph.placement_left_px as f32 / scale,
                glyph.offset_points.y - atlas_glyph.placement_top_px as f32 / scale,
            );
            let size_points = egui::vec2(
                atlas_glyph.size_px[0] as f32 / scale,
                atlas_glyph.size_px[1] as f32 / scale,
            );
            let positions = quad_positions_from_min_size(min, size_points);
            let quad_bounds = rect_from_points(positions);
            bounds = Some(bounds.map_or(quad_bounds, |current| current.union(quad_bounds)));
            quads.push(TextGpuQuad {
                atlas_page_index: page_index,
                positions: positions.map(|point| [point.x, point.y]),
                uvs: uv_quad_points(uv).map(|point| [point.x, point.y]),
                tint_rgba: [
                    if atlas_glyph.is_color {
                        Color32::WHITE.r()
                    } else {
                        glyph.color.r()
                    },
                    if atlas_glyph.is_color {
                        Color32::WHITE.g()
                    } else {
                        glyph.color.g()
                    },
                    if atlas_glyph.is_color {
                        Color32::WHITE.b()
                    } else {
                        glyph.color.b()
                    },
                    if atlas_glyph.is_color {
                        Color32::WHITE.a()
                    } else {
                        glyph.color.a()
                    },
                ],
            });
        }

        let bounds = bounds.unwrap_or(Rect::NOTHING);
        let atlas_pages = pages
            .iter()
            .enumerate()
            .map(|(i, page)| {
                let content_hash = page_hashers.get(i).map(|h| h.finish()).unwrap_or(0);
                cpu_page_to_page_data(page, i, content_hash)
            })
            .collect();

        // Return pages to the pool for reuse next call (pixel buffer allocations survive).
        const CPU_PAGE_POOL_MAX: usize = 4;
        let return_count = CPU_PAGE_POOL_MAX
            .saturating_sub(page_pool.len())
            .min(pages.len());
        page_pool.extend(pages.drain(..return_count));
        self.cpu_page_pool = page_pool;

        TextGpuScene {
            atlas_pages,
            quads,
            bounds_min: [bounds.min.x, bounds.min.y],
            bounds_max: [bounds.max.x, bounds.max.y],
            size_points: [layout.size_points.x, layout.size_points.y],
            fingerprint: 0,
        }
    }

    pub(crate) fn build_text_scene_on_path(
        &mut self,
        ctx: &Context,
        layout: &PreparedTextLayout,
        fallback_advance_points: f32,
        line_height_points: f32,
        scale: f32,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextRenderScene, TextPathError> {
        let path_layout = build_path_layout_from_prepared_layout(
            layout,
            fallback_advance_points,
            line_height_points,
            path,
            path_options,
        )?;
        let graphics_config = self.resolved_graphics_config(self.max_texture_side_px.max(1));
        let field_range_px = graphics_config.rasterization.field_range_px.max(1.0);
        let mut quads = Vec::with_capacity(layout.glyphs.len());
        let mut bounds: Option<Rect> = None;

        for (glyph, path_glyph) in layout.glyphs.iter().zip(path_layout.glyphs.iter()) {
            let content_mode = self.resolved_glyph_content_mode(graphics_config, &glyph.cache_key);
            let raster_key = glyph
                .cache_key
                .for_content_mode(content_mode, field_range_px);
            let Some(atlas_entry) = self.glyph_atlas.resolve_sync(
                ctx,
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
            let positions = rotated_quad_positions(
                egui_point_from_text(path_glyph.anchor),
                origin_offset,
                size_points,
                path_glyph.rotation_radians,
            );
            let quad_bounds = rect_from_points(positions);
            bounds = Some(bounds.map_or(quad_bounds, |current| current.union(quad_bounds)));
            quads.push(TextAtlasQuad {
                atlas_page_index: atlas_entry.page_index,
                positions: positions.map(Into::into),
                uvs: uv_quad_points(atlas_entry.uv).map(Into::into),
                tint: if atlas_entry.is_color {
                    Color32::WHITE.into()
                } else {
                    glyph.color.into()
                },
                is_color: atlas_entry.is_color,
            });
        }

        Ok(TextRenderScene {
            quads,
            bounds: bounds
                .unwrap_or(egui_rect_from_text(path_layout.bounds))
                .into(),
            size_points: layout.size_points.into(),
        })
    }

    pub(crate) fn build_text_gpu_scene_on_path(
        &mut self,
        layout: &PreparedTextLayout,
        fallback_advance_points: f32,
        line_height_points: f32,
        scale: f32,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextGpuScene, TextPathError> {
        let path_layout = build_path_layout_from_prepared_layout(
            layout,
            fallback_advance_points,
            line_height_points,
            path,
            path_options,
        )?;
        let target_page_side_px =
            default_gpu_scene_page_side(self.resolved_graphics_config(self.max_texture_side_px));
        let graphics_config = self.resolved_graphics_config(self.max_texture_side_px);
        let mut page_pool = std::mem::take(&mut self.cpu_page_pool);
        let mut pages = Vec::<CpuSceneAtlasPage>::new();
        let mut page_hashers = Vec::<FxHasher>::new();
        let mut quads = Vec::with_capacity(layout.glyphs.len());
        let mut bounds: Option<Rect> = None;

        for (glyph, path_glyph) in layout.glyphs.iter().zip(path_layout.glyphs.iter()) {
            let Some(atlas_glyph) = self.get_or_rasterize_gpu_scene_glyph(
                &glyph.cache_key,
                graphics_config.rasterization,
                graphics_config.atlas_padding_px,
            ) else {
                continue;
            };

            let allocation_size = size2(
                atlas_glyph.upload_image.size[0] as i32,
                atlas_glyph.upload_image.size[1] as i32,
            );
            let Some((page_index, allocation)) = allocate_cpu_scene_page_slot(
                &mut pages,
                &mut page_pool,
                target_page_side_px,
                allocation_size,
            ) else {
                continue;
            };

            while page_hashers.len() < pages.len() {
                let new_idx = page_hashers.len();
                let mut h = FxHasher::default();
                new_idx.hash(&mut h);
                pages[new_idx].size.hash(&mut h);
                page_hashers.push(h);
            }

            let pos = [
                allocation.rectangle.min.x.max(0) as usize,
                allocation.rectangle.min.y.max(0) as usize,
            ];
            glyph.cache_key.hash(&mut page_hashers[page_index]);
            pos[0].hash(&mut page_hashers[page_index]);
            pos[1].hash(&mut page_hashers[page_index]);

            let page_size = pages[page_index].size;
            blit_to_page(
                &mut pages[page_index].rgba8,
                page_size,
                &atlas_glyph.upload_image,
                pos[0],
                pos[1],
            );

            let uv = Rect::from_min_max(
                Pos2::new(
                    (pos[0] + graphics_config.atlas_padding_px) as f32 / page_size[0] as f32,
                    (pos[1] + graphics_config.atlas_padding_px) as f32 / page_size[1] as f32,
                ),
                Pos2::new(
                    (pos[0] + graphics_config.atlas_padding_px + atlas_glyph.size_px[0]) as f32
                        / page_size[0] as f32,
                    (pos[1] + graphics_config.atlas_padding_px + atlas_glyph.size_px[1]) as f32
                        / page_size[1] as f32,
                ),
            );

            let size_points = egui::vec2(
                atlas_glyph.size_px[0] as f32 / scale,
                atlas_glyph.size_px[1] as f32 / scale,
            );
            let origin_offset = egui::vec2(
                atlas_glyph.placement_left_px as f32 / scale,
                -(atlas_glyph.placement_top_px as f32) / scale,
            );
            let positions = rotated_quad_positions(
                egui_point_from_text(path_glyph.anchor),
                origin_offset,
                size_points,
                path_glyph.rotation_radians,
            );
            let quad_bounds = rect_from_points(positions);
            bounds = Some(bounds.map_or(quad_bounds, |current| current.union(quad_bounds)));
            quads.push(TextGpuQuad {
                atlas_page_index: page_index,
                positions: positions.map(|point| [point.x, point.y]),
                uvs: uv_quad_points(uv).map(|point| [point.x, point.y]),
                tint_rgba: [
                    if atlas_glyph.is_color {
                        Color32::WHITE.r()
                    } else {
                        glyph.color.r()
                    },
                    if atlas_glyph.is_color {
                        Color32::WHITE.g()
                    } else {
                        glyph.color.g()
                    },
                    if atlas_glyph.is_color {
                        Color32::WHITE.b()
                    } else {
                        glyph.color.b()
                    },
                    if atlas_glyph.is_color {
                        Color32::WHITE.a()
                    } else {
                        glyph.color.a()
                    },
                ],
            });
        }

        let bounds = bounds.unwrap_or(egui_rect_from_text(path_layout.bounds));
        let atlas_pages = pages
            .iter()
            .enumerate()
            .map(|(i, page)| {
                let content_hash = page_hashers.get(i).map(|h| h.finish()).unwrap_or(0);
                cpu_page_to_page_data(page, i, content_hash)
            })
            .collect();

        const CPU_PAGE_POOL_MAX: usize = 4;
        let return_count = CPU_PAGE_POOL_MAX
            .saturating_sub(page_pool.len())
            .min(pages.len());
        page_pool.extend(pages.drain(..return_count));
        self.cpu_page_pool = page_pool;

        Ok(TextGpuScene {
            atlas_pages,
            quads,
            bounds_min: [bounds.min.x, bounds.min.y],
            bounds_max: [bounds.max.x, bounds.max.y],
            size_points: [layout.size_points.x, layout.size_points.y],
            fingerprint: 0,
        })
    }

    pub(crate) fn build_text_wgpu_scene_callback(
        &self,
        quads: &[PaintTextQuad],
    ) -> Option<TextWgpuSceneCallback> {
        let graphics_config = self.resolved_graphics_config(self.max_texture_side_px.max(1));
        if graphics_config.renderer_backend != ResolvedTextRendererBackend::WgpuInstanced {
            return None;
        }
        let target_format = self.glyph_atlas.wgpu_render_state.as_ref()?.target_format;
        let mut grouped = FxHashMap::<usize, Vec<TextWgpuInstance>>::default();
        for quad in quads {
            grouped
                .entry(quad.page_index)
                .or_default()
                .push(TextWgpuInstance::from_quad(quad));
        }

        let mut page_indices = grouped.keys().copied().collect::<Vec<_>>();
        page_indices.sort_unstable();
        let mut batches = Vec::with_capacity(page_indices.len());
        for page_index in page_indices {
            let texture = self.glyph_atlas.native_texture_for_page(page_index)?;
            let instances = grouped.remove(&page_index).unwrap_or_default();
            if instances.is_empty() {
                continue;
            }
            batches.push(TextWgpuSceneBatchSource {
                atlas_generation: self.glyph_atlas.generation(),
                page_index,
                texture,
                instances: Arc::from(instances.into_boxed_slice()),
            });
        }

        if batches.is_empty() {
            return None;
        }

        Some(TextWgpuSceneCallback {
            target_format,
            atlas_sampling: graphics_config.atlas_sampling,
            linear_pipeline: self.graphics_config.linear_pipeline,
            output_is_hdr: target_format_is_hdr(target_format),
            batches: Arc::from(batches.into_boxed_slice()),
            prepared: Arc::new(Mutex::new(TextWgpuPreparedScene::default())),
        })
    }

    pub(crate) fn get_or_rasterize_gpu_scene_glyph(
        &mut self,
        cache_key: &GlyphRasterKey,
        rasterization: TextRasterizationConfig,
        padding_px: usize,
    ) -> Option<Arc<PreparedAtlasGlyph>> {
        if let Some(glyph) = self
            .gpu_scene_glyph_cache
            .write(|state| state.touch(cache_key).map(|entry| Arc::clone(&entry.value)))
        {
            return Some(glyph);
        }

        let glyph = Arc::new(rasterize_atlas_glyph(
            &mut self.font_system,
            &mut self.scale_context,
            cache_key,
            rasterization,
            padding_px,
        )?);
        self.gpu_scene_glyph_cache.write(|state| {
            let _ = state.insert(cache_key.clone(), Arc::clone(&glyph), glyph.approx_bytes);
        });
        Some(glyph)
    }
}
