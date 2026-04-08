use super::*;

#[derive(Clone, Debug, Default)]
pub(super) struct CapeChoice {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) texture_bytes: Option<Vec<u8>>,
    pub(super) texture_size: Option<[u32; 2]>,
}

#[derive(Clone)]
pub(super) struct SkinManagerState {
    pub(super) active_profile_id: Option<String>,
    pub(super) active_player_name: Option<String>,
    pub(super) access_token: Option<String>,
    pub(super) base_skin_png: Option<Vec<u8>>,
    pub(super) pending_skin_png: Option<Vec<u8>>,
    pub(super) pending_skin_path: Option<PathBuf>,
    pub(super) initial_variant: MinecraftSkinVariant,
    pub(super) pending_variant: MinecraftSkinVariant,
    pub(super) available_capes: Vec<CapeChoice>,
    pub(super) initial_cape_id: Option<String>,
    pub(super) pending_cape_id: Option<String>,
    pub(super) show_elytra: bool,
    pub(super) status_message: Option<String>,
    pub(super) save_in_progress: bool,
    pub(super) refresh_in_progress: bool,
    pub(super) worker_rx: Option<Arc<Mutex<Receiver<WorkerEvent>>>>,
    pub(super) pick_skin_in_progress: bool,
    pub(super) pick_skin_results_rx:
        Option<Arc<Mutex<Receiver<Result<(PathBuf, Vec<u8>), String>>>>>,
    pub(super) wgpu_target_format: Option<wgpu::TextureFormat>,
    pub(super) preview_msaa_samples: u32,
    pub(super) preview_aa_mode: SkinPreviewAaMode,
    pub(super) last_preview_aa_mode: SkinPreviewAaMode,
    pub(super) preview_texel_aa_mode: SkinPreviewTexelAaMode,
    pub(super) last_preview_texel_aa_mode: SkinPreviewTexelAaMode,
    pub(super) preview_motion_blur_enabled: bool,
    pub(super) last_preview_motion_blur_enabled: bool,
    pub(super) preview_motion_blur_amount: f32,
    pub(super) last_preview_motion_blur_amount: f32,
    pub(super) preview_motion_blur_shutter_frames: f32,
    pub(super) last_preview_motion_blur_shutter_frames: f32,
    pub(super) preview_motion_blur_sample_count: usize,
    pub(super) last_preview_motion_blur_sample_count: usize,
    pub(super) preview_3d_layers_enabled: bool,
    pub(super) last_preview_3d_layers_enabled: bool,
    pub(super) expressions_enabled: bool,
    pub(super) last_expressions_enabled: bool,
    pub(super) cached_expression_layout_hash: Option<u64>,
    pub(super) cached_expression_layout: Option<DetectedExpressionsLayout>,
    pub(super) preview_motion_mode: PreviewMotionMode,
    pub(super) preview_motion_blend: f32,
    pub(super) skin_texture_hash: Option<u64>,
    pub(super) skin_texture: Option<TextureHandle>,
    pub(super) skin_sample: Option<Arc<RgbaImage>>,
    pub(super) cape_texture_hash: Option<u64>,
    pub(super) cape_texture: Option<TextureHandle>,
    pub(super) cape_sample: Option<Arc<RgbaImage>>,
    pub(super) default_elytra_texture: Option<TextureHandle>,
    pub(super) default_elytra_sample: Option<Arc<RgbaImage>>,
    pub(super) preview_texture: Option<TextureHandle>,
    pub(super) preview_history: Option<PreviewHistory>,
    pub(super) cape_uv: FaceUvs,
    pub(super) camera_yaw_offset: f32,
    pub(super) camera_inertial_velocity: f32,
    pub(super) camera_drag_velocity: f32,
    pub(super) camera_drag_active: bool,
    pub(super) orbit_pause_started_at: Option<f64>,
    pub(super) orbit_pause_accumulated_secs: f64,
    pub(super) camera_last_frame_time: Option<f64>,
    pub(super) refresh_on_open_pending: bool,
}

impl Default for SkinManagerState {
    fn default() -> Self {
        Self {
            active_profile_id: None,
            active_player_name: None,
            access_token: None,
            base_skin_png: None,
            pending_skin_png: None,
            pending_skin_path: None,
            initial_variant: MinecraftSkinVariant::Classic,
            pending_variant: MinecraftSkinVariant::Classic,
            available_capes: Vec::new(),
            initial_cape_id: None,
            pending_cape_id: None,
            show_elytra: false,
            status_message: None,
            save_in_progress: false,
            refresh_in_progress: false,
            worker_rx: None,
            pick_skin_in_progress: false,
            pick_skin_results_rx: None,
            wgpu_target_format: None,
            preview_msaa_samples: 1,
            preview_aa_mode: SkinPreviewAaMode::Msaa,
            last_preview_aa_mode: SkinPreviewAaMode::Msaa,
            preview_texel_aa_mode: SkinPreviewTexelAaMode::Off,
            last_preview_texel_aa_mode: SkinPreviewTexelAaMode::Off,
            preview_motion_blur_enabled: false,
            last_preview_motion_blur_enabled: false,
            preview_motion_blur_amount: 0.15,
            last_preview_motion_blur_amount: 0.15,
            preview_motion_blur_shutter_frames: 0.75,
            last_preview_motion_blur_shutter_frames: 0.75,
            preview_motion_blur_sample_count: 5,
            last_preview_motion_blur_sample_count: 5,
            preview_3d_layers_enabled: false,
            last_preview_3d_layers_enabled: false,
            expressions_enabled: false,
            last_expressions_enabled: false,
            cached_expression_layout_hash: None,
            cached_expression_layout: None,
            preview_motion_mode: PreviewMotionMode::Idle,
            preview_motion_blend: 0.0,
            skin_texture_hash: None,
            skin_texture: None,
            skin_sample: None,
            cape_texture_hash: None,
            cape_texture: None,
            cape_sample: None,
            default_elytra_texture: None,
            default_elytra_sample: None,
            preview_texture: None,
            preview_history: None,
            cape_uv: default_cape_uv_layout(),
            camera_yaw_offset: 0.0,
            camera_inertial_velocity: 0.0,
            camera_drag_velocity: 0.0,
            camera_drag_active: false,
            orbit_pause_started_at: None,
            orbit_pause_accumulated_secs: 0.0,
            camera_last_frame_time: None,
            refresh_on_open_pending: false,
        }
    }
}

impl SkinManagerState {
    pub(super) fn sync_active_account(&mut self, active_launch_auth: Option<&LaunchAuthContext>) {
        let Some(auth) = active_launch_auth else {
            if self.active_profile_id.is_some() {
                *self = Self::default();
            }
            return;
        };

        let normalized_profile_id = auth.player_uuid.to_ascii_lowercase();
        let profile_changed =
            self.active_profile_id.as_deref() != Some(normalized_profile_id.as_str());
        let token_changed = self.access_token.as_deref() != auth.access_token.as_deref();
        let name_changed = self.active_player_name.as_deref() != Some(auth.player_name.as_str());

        if !profile_changed && !token_changed && !name_changed {
            return;
        }

        if !profile_changed {
            self.access_token = auth.access_token.clone();
            self.active_player_name = Some(auth.player_name.clone());
            tracing::info!(
                target: "vertexlauncher/skins",
                display_name = auth.player_name.as_str(),
                token_changed,
                name_changed,
                "Updated skin manager auth context for active profile."
            );
            return;
        }

        self.save_in_progress = false;
        self.refresh_in_progress = false;
        self.worker_rx = None;
        self.pick_skin_in_progress = false;
        self.pick_skin_results_rx = None;
        self.status_message = None;
        self.show_elytra = false;
        self.active_profile_id = Some(normalized_profile_id.clone());
        self.active_player_name = Some(auth.player_name.clone());
        self.access_token = auth.access_token.clone();
        self.base_skin_png = None;
        self.pending_skin_png = None;
        self.pending_skin_path = None;
        self.initial_variant = MinecraftSkinVariant::Classic;
        self.pending_variant = MinecraftSkinVariant::Classic;
        self.available_capes.clear();
        self.initial_cape_id = None;
        self.pending_cape_id = None;
        self.skin_texture_hash = None;
        self.skin_texture = None;
        self.skin_sample = None;
        self.cape_texture_hash = None;
        self.cape_texture = None;
        self.cape_sample = None;
        self.preview_texture = None;
        self.preview_history = None;
        self.cached_expression_layout_hash = None;
        self.cached_expression_layout = None;
        self.cape_uv = default_cape_uv_layout();
        self.camera_yaw_offset = 0.0;
        self.camera_inertial_velocity = 0.0;
        self.camera_drag_velocity = 0.0;
        self.camera_drag_active = false;
        self.orbit_pause_started_at = None;
        self.orbit_pause_accumulated_secs = 0.0;
        self.camera_last_frame_time = None;

        self.load_snapshot_from_cache_for_profile(normalized_profile_id.as_str());
        self.start_refresh();
    }

    pub(super) fn load_snapshot_from_cache_for_profile(&mut self, profile_id: &str) {
        match auth::load_cached_accounts() {
            Ok(accounts) => {
                let profile_id_lower = profile_id.to_ascii_lowercase();
                if let Some(account) = accounts.accounts.iter().find(|account| {
                    account.minecraft_profile.id.to_ascii_lowercase() == profile_id_lower
                }) {
                    self.apply_account_snapshot(account);
                }
            }
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/skins",
                    error = %err,
                    "Failed to load cached accounts while preparing skin-manager snapshot."
                );
                notification::error!("skin_manager", "Failed to load account cache: {err}");
            }
        }
    }

    pub(super) fn apply_account_snapshot(&mut self, account: &CachedAccount) {
        self.active_profile_id = Some(account.minecraft_profile.id.to_ascii_lowercase());
        self.active_player_name = Some(account.minecraft_profile.name.clone());
        self.base_skin_png = None;
        self.initial_variant = MinecraftSkinVariant::Classic;
        self.pending_variant = MinecraftSkinVariant::Classic;
        self.pending_skin_png = None;
        self.pending_skin_path = None;
        self.skin_texture_hash = None;
        self.skin_texture = None;
        self.skin_sample = None;
        self.cape_texture_hash = None;
        self.cape_texture = None;
        self.cape_sample = None;
        self.preview_texture = None;
        self.preview_history = None;
        self.cached_expression_layout_hash = None;
        self.cached_expression_layout = None;
        self.cape_uv = default_cape_uv_layout();

        let mut choices = Vec::with_capacity(account.minecraft_profile.capes.len());
        for cape in &account.minecraft_profile.capes {
            let texture_bytes = cape.texture_png_bytes();
            let texture_size = texture_bytes.as_deref().and_then(decode_image_dimensions);
            choices.push(CapeChoice {
                id: cape.id.clone(),
                label: cape
                    .alias
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(cape.id.as_str())
                    .to_owned(),
                texture_bytes,
                texture_size,
            });
        }

        self.available_capes = choices;
        self.initial_cape_id = None;
        self.pending_cape_id = None;
    }

    pub(super) fn poll_worker(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.worker_rx.take() else {
            return;
        };
        let mut keep_rx = true;
        loop {
            let recv_result = match rx.lock() {
                Ok(guard) => guard.try_recv(),
                Err(_) => {
                    self.save_in_progress = false;
                    self.refresh_in_progress = false;
                    notification::error!(
                        "skin_manager",
                        "Background profile task lock was poisoned."
                    );
                    keep_rx = false;
                    break;
                }
            };
            match recv_result {
                Ok(WorkerEvent::Refreshed(result)) => {
                    tracing::info!(
                        target: "vertexlauncher/skins",
                        "Skin manager refresh worker completed."
                    );
                    self.refresh_in_progress = false;
                    match result {
                        Ok((profile_id, profile)) => {
                            if self.active_profile_id.as_deref() != Some(profile_id.as_str()) {
                                tracing::info!(
                                    target: "vertexlauncher/skins",
                                    display_name = self.active_player_name.as_deref().unwrap_or("unknown"),
                                    "Ignoring refresh result for non-active profile."
                                );
                                keep_rx = false;
                                break;
                            }
                            if self.pending_skin_png.is_none()
                                && self.pending_variant == self.initial_variant
                                && self.pending_cape_id == self.initial_cape_id
                            {
                                self.apply_loaded_profile(profile);
                            }
                        }
                        Err(err) => {
                            tracing::info!(
                                target: "vertexlauncher/skins",
                                error = %err,
                                "Skin manager refresh failed."
                            );
                            notification::error!("skin_manager", "{err}");
                        }
                    }
                    keep_rx = false;
                }
                Ok(WorkerEvent::Saved(result)) => {
                    tracing::info!(
                        target: "vertexlauncher/skins",
                        "Skin manager save worker completed."
                    );
                    self.save_in_progress = false;
                    match result {
                        Ok((profile_id, profile)) => {
                            if self.active_profile_id.as_deref() != Some(profile_id.as_str()) {
                                tracing::info!(
                                    target: "vertexlauncher/skins",
                                    display_name = self.active_player_name.as_deref().unwrap_or("unknown"),
                                    "Ignoring save result for non-active profile."
                                );
                                keep_rx = false;
                                break;
                            }
                            self.apply_loaded_profile(profile);
                            notification::info!("skin_manager", "Saved skin and cape changes.");
                        }
                        Err(err) => {
                            tracing::info!(
                                target: "vertexlauncher/skins",
                                error = %err,
                                "Skin manager save failed."
                            );
                            notification::error!("skin_manager", "{err}");
                        }
                    }
                    keep_rx = false;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.save_in_progress = false;
                    self.refresh_in_progress = false;
                    tracing::error!(
                        target: "vertexlauncher/skins",
                        "Skin manager worker channel disconnected."
                    );
                    keep_rx = false;
                    break;
                }
            }
        }
        if keep_rx {
            self.worker_rx = Some(rx);
        } else {
            ctx.request_repaint();
        }
    }

    pub(super) fn poll_pick_skin_result(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.pick_skin_results_rx.as_ref().cloned() else {
            return;
        };

        let Ok(receiver) = rx.lock() else {
            tracing::error!(
                target: "vertexlauncher/skins",
                "Pick-skin result receiver mutex was poisoned."
            );
            return;
        };
        let Ok(result) = receiver.try_recv() else {
            return;
        };

        self.pick_skin_in_progress = false;
        self.pick_skin_results_rx = None;
        match result {
            Ok((path, bytes)) => {
                self.pending_skin_png = Some(bytes);
                self.pending_skin_path = Some(path);
                self.skin_texture_hash = None;
                self.skin_sample = None;
                self.preview_texture = None;
                self.preview_history = None;
            }
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/skins",
                    error = %err,
                    "Pick-skin operation failed."
                );
                notification::error!("skin_manager", "{err}");
            }
        }
        ctx.request_repaint();
    }

    pub(super) fn ensure_skin_texture(&mut self, ctx: &egui::Context) {
        let Some(bytes) = self.preview_skin_png() else {
            self.skin_texture = None;
            self.skin_texture_hash = None;
            self.skin_sample = None;
            return;
        };

        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let hash = hasher.finish();
        if self.skin_texture_hash == Some(hash) {
            return;
        }

        let Some(image) = decode_skin_rgba(bytes) else {
            self.skin_sample = None;
            return;
        };
        let image = Arc::new(image);
        let size = [image.width() as usize, image.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        let texture = ctx.load_texture(
            format!("skins/preview/{hash:016x}"),
            color_image,
            TextureOptions::NEAREST,
        );

        self.skin_texture = Some(texture);
        self.skin_sample = Some(image);
        self.skin_texture_hash = Some(hash);
    }

    pub(super) fn ensure_default_elytra_texture(&mut self, ctx: &egui::Context) {
        if self.default_elytra_texture.is_some() && self.default_elytra_sample.is_some() {
            return;
        }
        let image = Arc::new(default_elytra_texture_image());
        let size = [image.width() as usize, image.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        let texture =
            ctx.load_texture("skins/default-elytra", color_image, TextureOptions::NEAREST);
        self.default_elytra_texture = Some(texture);
        self.default_elytra_sample = Some(image);
    }

    pub(super) fn ensure_cape_texture(&mut self, ctx: &egui::Context) {
        let Some(bytes) = self.selected_cape_png() else {
            self.cape_texture = None;
            self.cape_texture_hash = None;
            self.cape_sample = None;
            self.cape_uv = default_cape_uv_layout();
            return;
        };

        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let hash = hasher.finish();
        if self.cape_texture_hash == Some(hash) {
            return;
        }

        let Some(image) = decode_generic_rgba(bytes) else {
            self.cape_texture = None;
            self.cape_texture_hash = None;
            self.cape_sample = None;
            self.cape_uv = default_cape_uv_layout();
            return;
        };
        let image = Arc::new(image);
        self.cape_uv =
            cape_uv_layout([image.width(), image.height()]).unwrap_or_else(default_cape_uv_layout);

        let size = [image.width() as usize, image.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        let texture = ctx.load_texture(
            format!("skins/cape-preview/{hash:016x}"),
            color_image,
            TextureOptions::NEAREST,
        );

        self.cape_texture = Some(texture);
        self.cape_sample = Some(image);
        self.cape_texture_hash = Some(hash);
    }

    pub(super) fn preview_skin_png(&self) -> Option<&[u8]> {
        self.pending_skin_png
            .as_deref()
            .or(self.base_skin_png.as_deref())
    }

    pub(super) fn selected_cape_png(&self) -> Option<&[u8]> {
        let selected = self.pending_cape_id.as_deref()?;
        self.available_capes
            .iter()
            .find(|cape| cape.id == selected)
            .and_then(|cape| cape.texture_bytes.as_deref())
    }

    pub(super) fn pick_skin_file(&mut self) {
        if self.pick_skin_in_progress {
            return;
        }

        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG", &["png"])
            .set_title("Select Minecraft Skin")
            .pick_file()
        else {
            return;
        };

        self.begin_loading_skin_from_path(path);
    }

    pub(super) fn begin_loading_skin_from_path(&mut self, path: PathBuf) {
        if self.pick_skin_in_progress {
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.pick_skin_in_progress = true;
        self.pick_skin_results_rx = Some(Arc::new(Mutex::new(rx)));
        let _ = tokio_runtime::spawn_detached(async move {
            let result = tokio::fs::read(path.as_path())
                .await
                .map_err(|err| format!("Failed to read image: {err}"))
                .and_then(|bytes| {
                    if decode_skin_rgba(&bytes).is_none() {
                        Err(
                            "Selected image must be a valid PNG skin (expected 64x64 or 64x32)."
                                .to_owned(),
                        )
                    } else {
                        Ok((path, bytes))
                    }
                });
            if let Err(err) = tx.send(result) {
                tracing::error!(
                    target: "vertexlauncher/skins",
                    error = %err,
                    "Failed to deliver picked skin-file result."
                );
            }
        });
    }

    pub(super) fn begin_loading_skin_from_bytes(&mut self, path: PathBuf, bytes: Vec<u8>) {
        if self.pick_skin_in_progress {
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.pick_skin_in_progress = true;
        self.pick_skin_results_rx = Some(Arc::new(Mutex::new(rx)));
        let _ = tokio_runtime::spawn_detached(async move {
            let result = if decode_skin_rgba(&bytes).is_none() {
                Err("Selected image must be a valid PNG skin (expected 64x64 or 64x32).".to_owned())
            } else {
                Ok((path, bytes))
            };
            if let Err(err) = tx.send(result) {
                tracing::error!(
                    target: "vertexlauncher/skins",
                    error = %err,
                    "Failed to deliver dropped skin-file result."
                );
            }
        });
    }

    pub(super) fn can_save(&self) -> bool {
        self.access_token
            .as_deref()
            .map(str::trim)
            .is_some_and(|token| !token.is_empty())
            && (self.pending_skin_png.is_some()
                || self.pending_variant != self.initial_variant
                || self.pending_cape_id != self.initial_cape_id)
    }

    pub(super) fn start_refresh(&mut self) {
        if self.refresh_in_progress || self.save_in_progress {
            tracing::info!(
                target: "vertexlauncher/skins",
                refresh_in_progress = self.refresh_in_progress,
                save_in_progress = self.save_in_progress,
                "Skipping profile refresh because another skin task is active."
            );
            return;
        }
        let Some(profile_id) = self.active_profile_id.clone() else {
            return;
        };
        let Some(token) = self
            .access_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
        else {
            notification::error!(
                "skin_manager",
                "Missing Minecraft access token for active account."
            );
            return;
        };

        tracing::info!(
            target: "vertexlauncher/skins",
            display_name = self.active_player_name.as_deref().unwrap_or("unknown"),
            "Starting skin manager profile refresh."
        );
        self.refresh_in_progress = true;
        let (tx, rx) = mpsc::channel();
        self.worker_rx = Some(Arc::new(Mutex::new(rx)));
        let profile_id_for_result = profile_id.clone();
        let display_name_for_log = self
            .active_player_name
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());
        tokio_runtime::spawn_blocking_detached(move || {
            let result = fetch_and_cache_profile(profile_id, &token, display_name_for_log.as_str());
            if let Err(err) = tx.send(WorkerEvent::Refreshed(
                result.map(|loaded| (profile_id_for_result, loaded)),
            )) {
                tracing::error!(
                    target: "vertexlauncher/skins",
                    display_name = %display_name_for_log,
                    error = %err,
                    "Failed to deliver skin manager refresh result."
                );
            }
        });
    }

    pub(super) fn try_consume_open_refresh(&mut self) {
        if !self.refresh_on_open_pending {
            return;
        }
        let has_active_profile = self.active_profile_id.is_some();
        let has_token = self
            .access_token
            .as_deref()
            .map(str::trim)
            .is_some_and(|token| !token.is_empty());
        if !has_active_profile || !has_token || self.refresh_in_progress || self.save_in_progress {
            return;
        }

        self.refresh_on_open_pending = false;
        tracing::info!(
            target: "vertexlauncher/skins",
            "Running queued skin manager open-refresh."
        );
        self.start_refresh();
    }

    pub(super) fn start_save(&mut self) {
        if self.save_in_progress || !self.can_save() {
            tracing::info!(
                target: "vertexlauncher/skins",
                save_in_progress = self.save_in_progress,
                can_save = self.can_save(),
                "Skipping skin save request."
            );
            return;
        }
        let Some(profile_id) = self.active_profile_id.clone() else {
            return;
        };
        let Some(token) = self
            .access_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
        else {
            notification::error!(
                "skin_manager",
                "Missing Minecraft access token for active account."
            );
            return;
        };

        if self.refresh_in_progress {
            self.refresh_in_progress = false;
            self.worker_rx = None;
        }

        self.save_in_progress = true;
        let pending_skin = self.pending_skin_png.clone();
        let base_skin = self.base_skin_png.clone();
        let pending_variant = self.pending_variant;
        let initial_variant = self.initial_variant;
        let pending_cape = self.pending_cape_id.clone();
        let initial_cape = self.initial_cape_id.clone();
        tracing::info!(
            target: "vertexlauncher/skins",
            display_name = self.active_player_name.as_deref().unwrap_or("unknown"),
            has_skin_change = pending_skin.is_some() || pending_variant != initial_variant,
            skin_variant = pending_variant.as_api_str(),
            cape_changed = pending_cape != initial_cape,
            cape_selected = pending_cape.is_some(),
            "Starting skin manager save."
        );
        let (tx, rx) = mpsc::channel();
        self.worker_rx = Some(Arc::new(Mutex::new(rx)));
        let profile_id_for_result = profile_id.clone();
        let display_name_for_log = self
            .active_player_name
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());

        let _ = tokio_runtime::spawn_detached(async move {
            let result: Result<LoadedProfile, String> = (|| {
                let mut latest_profile: Option<MinecraftProfileState> = None;
                let skin_bytes_to_upload = pending_skin
                    .as_deref()
                    .or(base_skin.as_deref())
                    .filter(|_| pending_skin.is_some() || pending_variant != initial_variant);
                if let Some(bytes) = skin_bytes_to_upload {
                    tracing::info!(
                        target: "vertexlauncher/skins",
                        display_name = display_name_for_log.as_str(),
                        png_bytes = bytes.len(),
                        variant = pending_variant.as_api_str(),
                        reused_existing_skin = pending_skin.is_none(),
                        "Uploading skin to Mojang profile API."
                    );
                    latest_profile = Some(
                        auth::upload_minecraft_skin(&token, bytes, pending_variant)
                            .map_err(|err| format_auth_error("upload skin", &err))?,
                    );
                    tracing::info!(
                        target: "vertexlauncher/skins",
                        display_name = display_name_for_log.as_str(),
                        "Skin upload completed."
                    );
                }

                if pending_cape != initial_cape {
                    if let Some(cape_id) = pending_cape.as_deref() {
                        tracing::info!(
                            target: "vertexlauncher/skins",
                            display_name = display_name_for_log.as_str(),
                            cape_id_present = !cape_id.is_empty(),
                            "Setting active cape via Mojang profile API."
                        );
                        latest_profile = Some(
                            auth::set_active_minecraft_cape(&token, cape_id)
                                .map_err(|err| format_auth_error("set cape", &err))?,
                        );
                        tracing::info!(
                            target: "vertexlauncher/skins",
                            display_name = display_name_for_log.as_str(),
                            "Cape activation completed."
                        );
                    } else {
                        tracing::info!(
                            target: "vertexlauncher/skins",
                            display_name = display_name_for_log.as_str(),
                            "Clearing active cape via Mojang profile API."
                        );
                        latest_profile = Some(
                            auth::clear_active_minecraft_cape(&token)
                                .map_err(|err| format_auth_error("clear cape", &err))?,
                        );
                        tracing::info!(
                            target: "vertexlauncher/skins",
                            display_name = display_name_for_log.as_str(),
                            "Cape clear completed."
                        );
                    }
                }

                if let Some(profile) = latest_profile {
                    tracing::info!(
                        target: "vertexlauncher/skins",
                        display_name = display_name_for_log.as_str(),
                        skins = profile.skins.len(),
                        capes = profile.capes.len(),
                        "Using profile payload returned by Mojang mutation endpoint."
                    );
                    update_cached_profile(
                        profile_id.as_str(),
                        &profile,
                        display_name_for_log.as_str(),
                    )?;
                    Ok(LoadedProfile::from_profile(profile))
                } else {
                    fetch_and_cache_profile(profile_id, &token, display_name_for_log.as_str())
                }
            })();
            if let Err(err) = tx.send(WorkerEvent::Saved(
                result.map(|loaded| (profile_id_for_result, loaded)),
            )) {
                tracing::error!(
                    target: "vertexlauncher/skins",
                    display_name = %display_name_for_log,
                    error = %err,
                    "Failed to deliver skin manager save result."
                );
            }
        });
    }

    pub(super) fn apply_loaded_profile(&mut self, profile: LoadedProfile) {
        self.active_player_name = Some(profile.player_name);
        self.base_skin_png = profile.active_skin_png;
        self.pending_skin_png = None;
        self.pending_skin_path = None;
        self.initial_variant = profile.skin_variant;
        self.pending_variant = profile.skin_variant;
        self.available_capes = profile.capes;
        self.initial_cape_id = profile.active_cape_id.clone();
        self.pending_cape_id = profile.active_cape_id;
        self.skin_texture_hash = None;
        self.skin_sample = None;
        self.cape_texture_hash = None;
        self.cape_sample = None;
        self.preview_texture = None;
        self.preview_history = None;
        self.cached_expression_layout_hash = None;
        self.cached_expression_layout = None;
        self.cape_uv = default_cape_uv_layout();
    }

    pub(super) fn begin_manual_camera_control(&mut self, now: f64) {
        if self.orbit_pause_started_at.is_none() {
            self.orbit_pause_started_at = Some(now);
        }
    }

    pub(super) fn finish_manual_camera_control(&mut self, now: f64) {
        if let Some(started_at) = self.orbit_pause_started_at.take() {
            self.orbit_pause_accumulated_secs += (now - started_at).max(0.0);
        }
    }

    pub(super) fn effective_orbit_time(&self, now: f64) -> f64 {
        let paused_now = self
            .orbit_pause_started_at
            .map(|started_at| (now - started_at).max(0.0))
            .unwrap_or(0.0);
        (now - self.orbit_pause_accumulated_secs - paused_now).max(0.0)
    }

    pub(super) fn consume_frame_dt(&mut self, now: f64) -> f32 {
        let dt = self
            .camera_last_frame_time
            .map(|previous| (now - previous).max(0.0) as f32)
            .unwrap_or(0.0);
        self.camera_last_frame_time = Some(now);
        dt
    }

    pub(super) fn refresh_expression_layout_cache(&mut self) {
        if !self.expressions_enabled {
            self.cached_expression_layout_hash = None;
            self.cached_expression_layout = None;
            return;
        }
        let Some(sample) = self.skin_sample.as_ref() else {
            self.cached_expression_layout_hash = None;
            self.cached_expression_layout = None;
            return;
        };
        let hash = hash_rgba_image(sample);
        if self.cached_expression_layout_hash == Some(hash) {
            return;
        }
        self.cached_expression_layout_hash = Some(hash);
        self.cached_expression_layout = detect_expression_layout(sample);
        if let Some(layout) = self.cached_expression_layout {
            let (right_eye_rect, left_eye_rect) = eye_face_rects(layout.eye);
            let (right_lid_rect, left_lid_rect) = eye_lid_rects(layout.eye);
            let right_lid_base_h = right_lid_rect.h as f32;
            let left_lid_base_h = left_lid_rect.h as f32;
            let right_upper_travel = (right_eye_rect.height - right_lid_base_h).max(0.0);
            let left_upper_travel = (left_eye_rect.height - left_lid_base_h).max(0.0);
            let right_lower_top_min = right_eye_rect.bottom_y() - right_eye_rect.height;
            let right_lower_top_max = right_eye_rect.bottom_y() - right_lid_base_h;
            let left_lower_top_min = left_eye_rect.bottom_y() - left_eye_rect.height;
            let left_lower_top_max = left_eye_rect.bottom_y() - left_lid_base_h;
            tracing::info!(
                target: "vertexlauncher/skins_expressions",
                eye_id = layout.eye.id,
                eye_family = ?layout.eye.family,
                eye_offset = ?layout.eye.offset,
                eye_width = layout.eye.width,
                eye_height = layout.eye.height,
                gaze_scale_x = layout.eye.gaze_scale_x,
                gaze_scale_y = layout.eye.gaze_scale_y,
                pupil_size = format_args!("{}x{}", layout.eye.pupil_width, layout.eye.pupil_height),
                upper_lid_right_top_range = format_args!(
                    "{:.3}..{:.3}",
                    right_eye_rect.top_y(),
                    right_eye_rect.top_y() + right_upper_travel
                ),
                upper_lid_left_top_range = format_args!(
                    "{:.3}..{:.3}",
                    left_eye_rect.top_y(),
                    left_eye_rect.top_y() + left_upper_travel
                ),
                lower_lid_right_top_range =
                    format_args!("{:.3}..{:.3}", right_lower_top_min, right_lower_top_max),
                lower_lid_left_top_range =
                    format_args!("{:.3}..{:.3}", left_lower_top_min, left_lower_top_max),
                "Detected skin expression layout."
            );
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct LoadedProfile {
    pub(super) player_name: String,
    pub(super) active_skin_png: Option<Vec<u8>>,
    pub(super) skin_variant: MinecraftSkinVariant,
    pub(super) capes: Vec<CapeChoice>,
    pub(super) active_cape_id: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) enum WorkerEvent {
    Refreshed(Result<(String, LoadedProfile), String>),
    Saved(Result<(String, LoadedProfile), String>),
}

pub(super) fn format_auth_error(operation: &str, err: &auth::AuthError) -> String {
    let message = err.to_string();
    if message.contains("HTTP status 401") {
        return format!(
            "Failed to {operation}: {message}. Minecraft auth token may be expired. Sign out and sign back in, then retry."
        );
    }
    format!("Failed to {operation}: {message}")
}

impl LoadedProfile {
    pub(super) fn from_profile(profile: MinecraftProfileState) -> Self {
        let active_skin = profile
            .skins
            .iter()
            .find(|skin| skin.state.eq_ignore_ascii_case("active"))
            .or_else(|| profile.skins.first());

        let active_skin_png = active_skin.and_then(|skin| skin.texture_png_bytes());
        let skin_variant = active_skin
            .and_then(|skin| skin.variant.as_deref())
            .map(parse_variant)
            .unwrap_or(MinecraftSkinVariant::Classic);

        let mut active_cape_id = None;
        let mut capes = Vec::with_capacity(profile.capes.len());
        for cape in profile.capes {
            let texture_bytes = cape.texture_png_bytes();
            let texture_size = texture_bytes.as_deref().and_then(decode_image_dimensions);
            if cape.state.eq_ignore_ascii_case("active") {
                active_cape_id = Some(cape.id.clone());
            }
            capes.push(CapeChoice {
                label: cape
                    .alias
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(cape.id.as_str())
                    .to_owned(),
                id: cape.id,
                texture_bytes,
                texture_size,
            });
        }

        Self {
            player_name: profile.name,
            active_skin_png,
            skin_variant,
            capes,
            active_cape_id,
        }
    }
}

pub(super) fn parse_variant(raw: &str) -> MinecraftSkinVariant {
    if raw.eq_ignore_ascii_case("slim") {
        MinecraftSkinVariant::Slim
    } else {
        MinecraftSkinVariant::Classic
    }
}

pub(super) fn decode_skin_rgba(bytes: &[u8]) -> Option<RgbaImage> {
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = image.dimensions();
    if w == 64 && (h == 64 || h == 32) {
        Some(image)
    } else {
        None
    }
}

pub(super) fn decode_generic_rgba(bytes: &[u8]) -> Option<RgbaImage> {
    image::load_from_memory(bytes)
        .ok()
        .map(|image| image.to_rgba8())
}

pub(super) fn decode_image_dimensions(bytes: &[u8]) -> Option<[u32; 2]> {
    let image = image::load_from_memory(bytes).ok()?;
    Some([image.width(), image.height()])
}

pub(super) fn full_uv_rect() -> Rect {
    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
}

pub(super) fn full_face_uvs() -> FaceUvs {
    let full = full_uv_rect();
    FaceUvs {
        top: full,
        bottom: full,
        left: full,
        right: full,
        front: full,
        back: full,
    }
}

pub(super) fn default_cape_uv_layout() -> FaceUvs {
    cape_uv_layout([64, 32]).unwrap_or_else(full_face_uvs)
}

pub(super) fn default_elytra_wing_uvs() -> ElytraWingUvs {
    elytra_wing_uvs([64, 32]).unwrap_or(ElytraWingUvs {
        left: full_face_uvs(),
        right: full_face_uvs(),
    })
}

pub(super) fn elytra_wing_uvs(texture_size: [u32; 2]) -> Option<ElytraWingUvs> {
    if texture_size[0] < 46 || texture_size[1] < 22 {
        return None;
    }
    let inset = 0.0;
    let left = FaceUvs {
        top: flip_uv_rect_x(uv_rect_with_inset(texture_size, 24, 0, 10, 2, inset)),
        bottom: flip_uv_rect_x(uv_rect_with_inset(texture_size, 34, 1, 10, 2, inset)),
        left: flip_uv_rect_x(uv_rect_with_inset(texture_size, 34, 2, 2, 20, inset)),
        right: flip_uv_rect_x(uv_rect_with_inset(texture_size, 22, 2, 2, 20, inset)),
        front: flip_uv_rect_x(uv_rect_with_inset(texture_size, 24, 2, 10, 20, inset)),
        back: flip_uv_rect_x(uv_rect_with_inset(texture_size, 36, 2, 10, 20, inset)),
    };
    let right = FaceUvs {
        top: uv_rect_with_inset(texture_size, 24, 0, 10, 2, inset),
        bottom: uv_rect_with_inset(texture_size, 34, 1, 10, 2, inset),
        left: uv_rect_with_inset(texture_size, 22, 2, 2, 20, inset),
        right: uv_rect_with_inset(texture_size, 34, 2, 2, 20, inset),
        front: uv_rect_with_inset(texture_size, 24, 2, 10, 20, inset),
        back: uv_rect_with_inset(texture_size, 36, 2, 10, 20, inset),
    };
    Some(ElytraWingUvs { left, right })
}

pub(super) fn default_elytra_texture_image() -> RgbaImage {
    const DEFAULT_ELYTRA_TEXTURE_PNG: &[u8] = include_bytes!("../assets/default_elytra.png");
    if let Some(image) = decode_generic_rgba(DEFAULT_ELYTRA_TEXTURE_PNG) {
        return image;
    }
    let mut image = RgbaImage::from_pixel(64, 32, image::Rgba([0, 0, 0, 0]));
    let base = image::Rgba([141, 141, 141, 255]);
    let edge = image::Rgba([112, 112, 112, 255]);
    fill_rect_rgba(&mut image, 22, 0, 24, 22, base);
    fill_rect_rgba(&mut image, 22, 0, 24, 1, edge);
    fill_rect_rgba(&mut image, 22, 21, 24, 1, edge);
    fill_rect_rgba(&mut image, 22, 0, 1, 22, edge);
    fill_rect_rgba(&mut image, 45, 0, 1, 22, edge);
    image
}

pub(super) fn fill_rect_rgba(
    image: &mut RgbaImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    color: image::Rgba<u8>,
) {
    let max_x = image.width();
    let max_y = image.height();
    for py in y..y.saturating_add(height).min(max_y) {
        for px in x..x.saturating_add(width).min(max_x) {
            image.put_pixel(px, py, color);
        }
    }
}

pub(super) fn cape_outer_face_uv(texture_size: [u32; 2]) -> Option<Rect> {
    if texture_size[0] < 22 || texture_size[1] < 17 {
        return None;
    }
    Some(uv_rect_with_inset(
        texture_size,
        1,
        1,
        10,
        16,
        UV_EDGE_INSET_BASE_TEXELS,
    ))
}

pub(super) fn cape_uv_layout(texture_size: [u32; 2]) -> Option<FaceUvs> {
    let outer = cape_outer_face_uv(texture_size)?;
    let inner = uv_rect_with_inset(texture_size, 12, 1, 10, 16, UV_EDGE_INSET_BASE_TEXELS);
    Some(FaceUvs {
        top: uv_rect_with_inset(texture_size, 1, 0, 10, 1, UV_EDGE_INSET_BASE_TEXELS),
        bottom: uv_rect_with_inset(texture_size, 11, 0, 10, 1, UV_EDGE_INSET_BASE_TEXELS),
        left: uv_rect_with_inset(texture_size, 0, 1, 1, 16, UV_EDGE_INSET_BASE_TEXELS),
        right: uv_rect_with_inset(texture_size, 11, 1, 1, 16, UV_EDGE_INSET_BASE_TEXELS),
        front: inner,
        back: outer,
    })
}
