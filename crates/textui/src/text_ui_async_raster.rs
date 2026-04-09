use super::*;

#[path = "text_ui_async_raster/async_raster_cache_entry.rs"]
mod async_raster_cache_entry;
#[path = "text_ui_async_raster/async_raster_kind.rs"]
mod async_raster_kind;
#[path = "text_ui_async_raster/async_raster_request.rs"]
mod async_raster_request;
#[path = "text_ui_async_raster/async_raster_response.rs"]
mod async_raster_response;
#[path = "text_ui_async_raster/async_raster_state.rs"]
mod async_raster_state;
#[path = "text_ui_async_raster/async_raster_worker_message.rs"]
mod async_raster_worker_message;
#[path = "text_ui_async_raster/typography_snapshot.rs"]
mod typography_snapshot;

pub(super) use self::async_raster_cache_entry::AsyncRasterCacheEntry;
pub(super) use self::async_raster_kind::AsyncRasterKind;
pub(super) use self::async_raster_request::AsyncRasterRequest;
pub(super) use self::async_raster_response::AsyncRasterResponse;
pub(super) use self::async_raster_state::AsyncRasterState;
pub(super) use self::async_raster_worker_message::AsyncRasterWorkerMessage;
pub(super) use self::typography_snapshot::TypographySnapshot;

pub(super) fn new_async_raster_state() -> AsyncRasterState {
    let (worker_tx, worker_rx) = mpsc::channel::<AsyncRasterWorkerMessage>();
    let (result_tx, result_rx) = mpsc::channel::<AsyncRasterResponse>();
    let _ = tokio_runtime::spawn_blocking_detached(move || {
        async_raster_worker_loop(worker_rx, result_tx)
    });
    AsyncRasterState {
        tx: Some(worker_tx),
        rx: Some(result_rx),
        pending: FxHashSet::default(),
        cache: ThreadSafeLru::new(ASYNC_RASTER_CACHE_MAX_BYTES),
    }
}

impl TextUi {
    pub(super) fn typography_snapshot(&self) -> TypographySnapshot {
        TypographySnapshot {
            ui_font_family: self.ui_font_family.clone(),
            ui_font_size_scale: self.ui_font_size_scale,
            ui_font_weight: self.ui_font_weight,
            open_type_feature_tags: self.open_type_feature_tags.clone(),
        }
    }

    pub(super) fn poll_async_raster_results(&mut self) {
        let mut should_reset_worker = false;
        let Some(rx) = self.async_raster.rx.as_ref() else {
            return;
        };
        let current_frame = self.current_frame;
        loop {
            match rx.try_recv() {
                Ok(response) => {
                    self.async_raster.pending.remove(&response.key_hash);
                    let layout = Arc::new(response.layout);
                    let approx_bytes = layout.approx_bytes;
                    self.async_raster.cache.write(|state| {
                        let _ = state.insert(
                            response.key_hash,
                            AsyncRasterCacheEntry {
                                layout,
                                last_used_frame: current_frame,
                            },
                            approx_bytes,
                        );
                    });
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    should_reset_worker = true;
                    break;
                }
            }
        }
        if should_reset_worker {
            self.async_raster.tx = None;
            self.async_raster.rx = None;
            self.async_raster.pending.clear();
        }
        self.enforce_async_raster_cache_budget();
    }

    pub(super) fn enforce_async_raster_cache_budget(&mut self) {
        self.async_raster.cache.write(|state| {
            let _ = state.evict_to_budget();
        });
    }

    pub(super) fn get_or_queue_async_plain_layout(
        &mut self,
        key_hash: u64,
        text: String,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Option<Arc<PreparedTextLayout>> {
        let current_frame = self.current_frame;
        if let Some(layout) = self.async_raster.cache.write(|state| {
            let entry = state.touch(&key_hash)?;
            entry.value.last_used_frame = current_frame;
            Some(Arc::clone(&entry.value.layout))
        }) {
            return Some(layout);
        }
        let Some(tx) = self.async_raster.tx.as_ref().cloned() else {
            return Some(Arc::new(self.prepare_plain_text_layout(
                text.as_str(),
                options,
                width_points_opt,
                scale,
            )));
        };
        if self.async_raster.pending.insert(key_hash) {
            let request_text = text.clone();
            let request = AsyncRasterRequest {
                key_hash,
                kind: AsyncRasterKind::Plain(request_text),
                options: options.clone(),
                width_points_opt,
                scale,
                typography: self.typography_snapshot(),
            };
            if tx.send(AsyncRasterWorkerMessage::Render(request)).is_err() {
                self.async_raster.pending.remove(&key_hash);
                self.async_raster.tx = None;
                self.async_raster.rx = None;
                return Some(Arc::new(self.prepare_plain_text_layout(
                    text.as_str(),
                    options,
                    width_points_opt,
                    scale,
                )));
            }
        }
        None
    }

    pub(super) fn get_or_queue_async_rich_layout(
        &mut self,
        key_hash: u64,
        spans: Vec<RichSpan>,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        scale: f32,
    ) -> Option<Arc<PreparedTextLayout>> {
        let current_frame = self.current_frame;
        if let Some(layout) = self.async_raster.cache.write(|state| {
            let entry = state.touch(&key_hash)?;
            entry.value.last_used_frame = current_frame;
            Some(Arc::clone(&entry.value.layout))
        }) {
            return Some(layout);
        }
        let Some(tx) = self.async_raster.tx.as_ref().cloned() else {
            return Some(Arc::new(self.prepare_rich_text_layout(
                spans.as_slice(),
                options,
                width_points_opt,
                scale,
            )));
        };
        if self.async_raster.pending.insert(key_hash) {
            let request_spans = spans.clone();
            let request = AsyncRasterRequest {
                key_hash,
                kind: AsyncRasterKind::Rich(request_spans),
                options: options.clone(),
                width_points_opt,
                scale,
                typography: self.typography_snapshot(),
            };
            if tx.send(AsyncRasterWorkerMessage::Render(request)).is_err() {
                self.async_raster.pending.remove(&key_hash);
                self.async_raster.tx = None;
                self.async_raster.rx = None;
                return Some(Arc::new(self.prepare_rich_text_layout(
                    spans.as_slice(),
                    options,
                    width_points_opt,
                    scale,
                )));
            }
        }
        None
    }
}

fn async_raster_worker_loop(
    rx: mpsc::Receiver<AsyncRasterWorkerMessage>,
    tx: mpsc::Sender<AsyncRasterResponse>,
) {
    let mut font_system = FontSystem::new();
    configure_text_font_defaults(&mut font_system);

    while let Ok(msg) = rx.recv() {
        match msg {
            AsyncRasterWorkerMessage::RegisterFont(bytes) => {
                font_system.db_mut().load_font_data(bytes);
            }
            AsyncRasterWorkerMessage::Render(req) => {
                let layout = async_prepare_text_layout(&mut font_system, &req);
                let _ = tx.send(AsyncRasterResponse {
                    key_hash: req.key_hash,
                    layout,
                });
            }
        }
    }
}

fn async_prepare_text_layout(
    font_system: &mut FontSystem,
    req: &AsyncRasterRequest,
) -> PreparedTextLayout {
    let metrics = Metrics::new(
        (req.options.font_size * req.typography.ui_font_size_scale * req.scale).max(1.0),
        (req.options.line_height * req.typography.ui_font_size_scale * req.scale).max(1.0),
    );
    let mut buffer = Buffer::new(font_system, metrics);
    let width_px_opt = req.width_points_opt.map(|w| (w * req.scale).max(1.0));
    {
        let mut borrowed = buffer.borrow_with(font_system);
        borrowed.set_wrap(if req.options.wrap {
            Wrap::WordOrGlyph
        } else {
            Wrap::None
        });
        borrowed.set_size(width_px_opt, None);

        match &req.kind {
            AsyncRasterKind::Plain(text) => {
                let attrs_owned = async_build_text_attrs_owned(
                    req,
                    &SpanStyle {
                        color: req.options.color.into(),
                        monospace: req.options.monospace,
                        italic: req.options.italic,
                        weight: req.options.weight,
                    },
                );
                let attrs = attrs_owned.as_attrs();
                borrowed.set_text(text, &attrs, Shaping::Advanced, None);
            }
            AsyncRasterKind::Rich(spans) => {
                let default_attrs_owned = async_build_text_attrs_owned(
                    req,
                    &SpanStyle {
                        color: req.options.color.into(),
                        monospace: req.options.monospace,
                        italic: req.options.italic,
                        weight: req.options.weight,
                    },
                );
                let span_attrs_owned = spans
                    .iter()
                    .map(|span| async_build_text_attrs_owned(req, &span.style))
                    .collect::<Vec<_>>();
                let rich_text = spans
                    .iter()
                    .zip(span_attrs_owned.iter())
                    .map(|(span, attrs)| (span.text.as_str(), attrs.as_attrs()))
                    .collect::<Vec<_>>();
                let default_attrs = default_attrs_owned.as_attrs();
                borrowed.set_rich_text(rich_text, &default_attrs, Shaping::Advanced, None);
            }
        }
        borrowed.shape_until_scroll(true);
    }

    let (mut measured_width_px, measured_height_px) = measure_buffer_pixels(&buffer);
    if let Some(width_points) = req.width_points_opt {
        measured_width_px = (width_points * req.scale).ceil() as usize;
    }
    let width_px = measured_width_px.max(1);
    let height_px = measured_height_px.max(1);
    let (glyphs, extra_width_points) = collect_prepared_glyphs_from_buffer(
        &buffer,
        req.scale,
        req.options.color,
        &req.options.fundamentals,
    );

    PreparedTextLayout {
        approx_bytes: glyphs.len().saturating_mul(mem::size_of::<PreparedGlyph>()),
        glyphs: Arc::from(glyphs),
        size_points: egui::vec2(
            width_px as f32 / req.scale + extra_width_points,
            height_px as f32 / req.scale,
        ),
    }
}

fn async_build_text_attrs_owned(req: &AsyncRasterRequest, style: &SpanStyle) -> AttrsOwned {
    let effective_weight =
        (i32::from(style.weight) + (req.typography.ui_font_weight - 400)).clamp(100, 900) as u16;
    let mut attrs = Attrs::new()
        .color(to_cosmic_text_color(style.color))
        .weight(Weight(effective_weight))
        .metrics(Metrics::new(
            (req.options.font_size * req.typography.ui_font_size_scale).max(1.0),
            (req.options.line_height * req.typography.ui_font_size_scale).max(1.0),
        ));

    if style.monospace {
        attrs = attrs.family(Family::Monospace);
    } else if let Some(family) = req.typography.ui_font_family.as_deref() {
        attrs = attrs.family(Family::Name(family));
    }
    if style.italic {
        attrs = attrs.style(FontStyle::Italic);
    }
    if let Some(features) = compose_font_features(
        &req.typography.open_type_feature_tags,
        &req.options.fundamentals,
    ) {
        attrs = attrs.font_features(features);
    }
    AttrsOwned::new(&attrs)
}
