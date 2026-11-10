use super::*;

/// A live subsystem that has to react when some part of [`Config`] changes.
///
/// Every setting belongs to exactly one domain. No domain means "restart the launcher": the
/// launcher applies every change while it runs, so an app-side applier is registered per domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SettingsDomain {
    /// Graphics API and GPU adapter selection (device recreation).
    GraphicsDevice,
    /// Window transparency, blur/backdrop effects and UI opacity.
    WindowEffect,
    /// Vsync/HDR and the frame limiter.
    Presentation,
    /// Fonts, text rendering, typography and anti-aliasing of UI vector content.
    Text,
    Theme,
    Notifications,
    SkinPreview,
    /// Streamer mode, OBS detection and credential storage.
    Privacy,
    DiscordPresence,
    InstanceSync,
    /// Which Minecraft versions are offered.
    VersionFilters,
    /// Download concurrency, speed limit, installations root and CurseForge key.
    Downloads,
    /// Java paths and default launch arguments/memory.
    LaunchDefaults,
    /// Drag scrolling and gamepad calibration.
    Input,
}

impl SettingsDomain {
    const fn bit(self) -> u32 {
        1 << self as u32
    }
}

/// The set of [`SettingsDomain`]s that differ between two configs.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct SettingsDiff {
    bits: u32,
}

impl SettingsDiff {
    /// Every domain marked changed: used to apply a whole config once at startup.
    pub const fn all() -> Self {
        Self { bits: (1 << 14) - 1 }
    }

    pub const fn is_empty(self) -> bool {
        self.bits == 0
    }

    pub const fn contains(self, domain: SettingsDomain) -> bool {
        self.bits & domain.bit() != 0
    }

    fn mark(&mut self, changed: bool, domain: SettingsDomain) {
        if changed {
            self.bits |= domain.bit();
        }
    }

    /// Changed domains in declaration order.
    pub fn domains(self) -> impl Iterator<Item = SettingsDomain> {
        use SettingsDomain::*;
        [
            GraphicsDevice,
            WindowEffect,
            Presentation,
            Text,
            Theme,
            Notifications,
            SkinPreview,
            Privacy,
            DiscordPresence,
            InstanceSync,
            VersionFilters,
            Downloads,
            LaunchDefaults,
            Input,
        ]
        .into_iter()
        .filter(move |domain| self.contains(*domain))
    }
}

impl std::fmt::Debug for SettingsDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.domains()).finish()
    }
}

impl Config {
    /// Returns which live subsystems must be told about the change from `self` to `new`.
    ///
    /// This is an exhaustive destructure of both configs: adding a `Config` field fails to compile
    /// here until it is assigned to a [`SettingsDomain`].
    pub fn diff(&self, new: &Config) -> SettingsDiff {
        let Self {
            low_power_gpu_preferred,
            graphics_adapter_preference_type,
            graphics_adapter_profile,
            graphics_adapter_explicit_hash,
            graphics_api_preference,
            streamer_mode_enabled,
            obs_detection_enabled,
            insecure_file_account_storage,
            lazy_account_refresh,
            window_transparency,
            fullscreen_style: _,
            legacy_window_blur_enabled,
            windows_backdrop_type,
            linux_blur_protocol,
            macos_visual_effect_material,
            macos_visual_effect_blending_mode,
            macos_visual_effect_state,
            macos_visual_effect_emphasized,
            ui_opacity_percent,
            window_corner_radius,
            windows_blur_tint_rgb,
            windows_blur_tint_opacity_percent,
            legacy_windows_transparency_level,
            linux_set_opengl_driver,
            linux_use_zink_driver,
            theme_id,
            open_type_features_enabled,
            open_type_features_to_enable,
            notification_expiry_bars_empty_left,
            notification_fade_out_seconds,
            notification_easing,
            ui_font_family,
            ui_emoji_font_family,
            ui_cjk_font_family,
            text_rendering_path,
            skin_preview_aa_mode,
            skin_preview_texel_aa_mode,
            svg_aa_mode,
            console_log_level,
            game_console_log_level,
            skin_preview_msaa_samples,
            skin_preview_motion_blur_enabled,
            skin_preview_motion_blur_amount,
            skin_preview_motion_blur_shutter_frames,
            skin_preview_motion_blur_sample_count,
            skin_preview_fresh_format_enabled,
            skin_preview_3d_layers_enabled,
            hdr_when_available,
            hdr_color_space,
            hdr_ui_white_scale,
            hdr_ui_paper_white_nits,
            hdr_max_cll_nits,
            hdr_max_fall_nits,
            frame_limiter_enabled,
            discord_rich_presence_enabled,
            sync_servers_enabled,
            sync_command_history_enabled,
            sync_hotbars_enabled,
            sync_servers_to_all_instances_by_default,
            drag_scroll_enabled,
            follow_console_on_launch,
            start_fullscreen: _,
            steam_deck_handoff,
            displayed_time: _,
            debug_fps_counter: _,
            debug_framegraph: _,
            frame_limit_fps,
            ui_font_size,
            ui_font_weight,
            typography,
            include_snapshots_and_betas,
            include_alpha_versions,
            include_experimental_versions,
            force_java_21_minimum,
            default_instance_max_memory_mib,
            default_instance_cli_args,
            minecraft_installations_root,
            download_max_concurrent,
            download_speed_limit_enabled,
            download_speed_limit,
            curseforge_api_key,
            java_8_jvm_path,
            java_16_jvm_path,
            java_17_jvm_path,
            java_21_jvm_path,
            java_25_jvm_path,
            gamepad_calibrations,
        } = self;

        let mut diff = SettingsDiff::default();
        diff.mark(
            *low_power_gpu_preferred != new.low_power_gpu_preferred
                || *graphics_adapter_preference_type != new.graphics_adapter_preference_type
                || *graphics_adapter_profile != new.graphics_adapter_profile
                || *graphics_adapter_explicit_hash != new.graphics_adapter_explicit_hash
                || *graphics_api_preference != new.graphics_api_preference
                || *linux_set_opengl_driver != new.linux_set_opengl_driver
                || *linux_use_zink_driver != new.linux_use_zink_driver,
            SettingsDomain::GraphicsDevice,
        );
        diff.mark(
            *window_transparency != new.window_transparency
                || *legacy_window_blur_enabled != new.legacy_window_blur_enabled
                || *windows_backdrop_type != new.windows_backdrop_type
                || *linux_blur_protocol != new.linux_blur_protocol
                || *macos_visual_effect_material != new.macos_visual_effect_material
                || *macos_visual_effect_blending_mode != new.macos_visual_effect_blending_mode
                || *macos_visual_effect_state != new.macos_visual_effect_state
                || *macos_visual_effect_emphasized != new.macos_visual_effect_emphasized
                || *ui_opacity_percent != new.ui_opacity_percent
                || *window_corner_radius != new.window_corner_radius
                || *windows_blur_tint_rgb != new.windows_blur_tint_rgb
                || *windows_blur_tint_opacity_percent != new.windows_blur_tint_opacity_percent
                || *legacy_windows_transparency_level != new.legacy_windows_transparency_level,
            SettingsDomain::WindowEffect,
        );
        diff.mark(
            *hdr_when_available != new.hdr_when_available
                || *hdr_color_space != new.hdr_color_space
                || *hdr_ui_white_scale != new.hdr_ui_white_scale
                || *hdr_ui_paper_white_nits != new.hdr_ui_paper_white_nits
                || *hdr_max_cll_nits != new.hdr_max_cll_nits
                || *hdr_max_fall_nits != new.hdr_max_fall_nits
                || *frame_limiter_enabled != new.frame_limiter_enabled
                || *frame_limit_fps != new.frame_limit_fps,
            SettingsDomain::Presentation,
        );
        diff.mark(
            *open_type_features_enabled != new.open_type_features_enabled
                || *open_type_features_to_enable != new.open_type_features_to_enable
                || *ui_font_family != new.ui_font_family
                || *ui_emoji_font_family != new.ui_emoji_font_family
                || *ui_cjk_font_family != new.ui_cjk_font_family
                || *text_rendering_path != new.text_rendering_path
                || *svg_aa_mode != new.svg_aa_mode
                || *ui_font_size != new.ui_font_size
                || *ui_font_weight != new.ui_font_weight
                || *typography != new.typography,
            SettingsDomain::Text,
        );
        diff.mark(*theme_id != new.theme_id, SettingsDomain::Theme);
        diff.mark(
            *notification_expiry_bars_empty_left != new.notification_expiry_bars_empty_left
                || *notification_fade_out_seconds != new.notification_fade_out_seconds
                || *notification_easing != new.notification_easing,
            SettingsDomain::Notifications,
        );
        diff.mark(
            *skin_preview_aa_mode != new.skin_preview_aa_mode
                || *skin_preview_texel_aa_mode != new.skin_preview_texel_aa_mode
                || *skin_preview_msaa_samples != new.skin_preview_msaa_samples
                || *skin_preview_motion_blur_enabled != new.skin_preview_motion_blur_enabled
                || *skin_preview_motion_blur_amount != new.skin_preview_motion_blur_amount
                || *skin_preview_motion_blur_shutter_frames
                    != new.skin_preview_motion_blur_shutter_frames
                || *skin_preview_motion_blur_sample_count
                    != new.skin_preview_motion_blur_sample_count
                || *skin_preview_fresh_format_enabled != new.skin_preview_fresh_format_enabled
                || *skin_preview_3d_layers_enabled != new.skin_preview_3d_layers_enabled,
            SettingsDomain::SkinPreview,
        );
        diff.mark(
            *streamer_mode_enabled != new.streamer_mode_enabled
                || *obs_detection_enabled != new.obs_detection_enabled
                || *insecure_file_account_storage != new.insecure_file_account_storage
                || *lazy_account_refresh != new.lazy_account_refresh,
            SettingsDomain::Privacy,
        );
        diff.mark(
            *discord_rich_presence_enabled != new.discord_rich_presence_enabled,
            SettingsDomain::DiscordPresence,
        );
        diff.mark(
            *sync_servers_enabled != new.sync_servers_enabled
                || *sync_command_history_enabled != new.sync_command_history_enabled
                || *sync_hotbars_enabled != new.sync_hotbars_enabled
                || *sync_servers_to_all_instances_by_default
                    != new.sync_servers_to_all_instances_by_default,
            SettingsDomain::InstanceSync,
        );
        diff.mark(
            *include_snapshots_and_betas != new.include_snapshots_and_betas
                || *include_alpha_versions != new.include_alpha_versions
                || *include_experimental_versions != new.include_experimental_versions,
            SettingsDomain::VersionFilters,
        );
        diff.mark(
            *minecraft_installations_root != new.minecraft_installations_root
                || *download_max_concurrent != new.download_max_concurrent
                || *download_speed_limit_enabled != new.download_speed_limit_enabled
                || *download_speed_limit != new.download_speed_limit
                || *curseforge_api_key != new.curseforge_api_key,
            SettingsDomain::Downloads,
        );
        diff.mark(
            *force_java_21_minimum != new.force_java_21_minimum
                || *follow_console_on_launch != new.follow_console_on_launch
                || *console_log_level != new.console_log_level
                || *game_console_log_level != new.game_console_log_level
                || *steam_deck_handoff != new.steam_deck_handoff
                || *default_instance_max_memory_mib != new.default_instance_max_memory_mib
                || *default_instance_cli_args != new.default_instance_cli_args
                || *java_8_jvm_path != new.java_8_jvm_path
                || *java_16_jvm_path != new.java_16_jvm_path
                || *java_17_jvm_path != new.java_17_jvm_path
                || *java_21_jvm_path != new.java_21_jvm_path
                || *java_25_jvm_path != new.java_25_jvm_path,
            SettingsDomain::LaunchDefaults,
        );
        diff.mark(
            *drag_scroll_enabled != new.drag_scroll_enabled
                || *gamepad_calibrations != new.gamepad_calibrations,
            SettingsDomain::Input,
        );
        diff
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_contains_every_domain() {
        let all = SettingsDiff::all();
        assert_eq!(all.domains().count(), 14);
    }

    use super::*;

    #[test]
    fn identical_configs_have_an_empty_diff() {
        let config = Config::default();
        assert!(config.diff(&config.clone()).is_empty());
    }

    #[test]
    fn a_change_marks_only_its_own_domain() {
        let old = Config::default();
        let mut new = old.clone();
        *new.settings_bindings_mut().frame_limit_fps += 1;
        let diff = old.diff(&new);
        assert_eq!(
            diff.domains().collect::<Vec<_>>(),
            vec![SettingsDomain::Presentation]
        );
    }

    #[test]
    fn graphics_and_text_changes_are_reported_together() {
        let old = Config::default();
        let mut new = old.clone();
        {
            let bindings = new.settings_bindings_mut();
            *bindings.low_power_gpu_preferred = !*bindings.low_power_gpu_preferred;
            *bindings.ui_font_weight += 100;
        }
        let diff = old.diff(&new);
        assert!(diff.contains(SettingsDomain::GraphicsDevice));
        assert!(diff.contains(SettingsDomain::Text));
        assert!(!diff.contains(SettingsDomain::Theme));
    }
}
