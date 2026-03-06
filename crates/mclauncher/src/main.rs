use config::{
    Config, ConfigFormat, DropdownSettingId, LoadConfigResult, UiFontFamily, create_default_config,
    load_config, save_config,
};
use eframe::{self, egui};
use egui::CentralPanel;
use fontloader::{FontCatalog, FontSpec, Slant, Stretch, Weight};
use textui::{ButtonOptions, LabelOptions, TextUi};

mod assets;
mod screens;
mod ui;
mod window_effects;

const MAPLE_MONO_NF_REGULAR_TTF: &[u8] = include_bytes!("included_fonts/MapleMono-NF-Regular.ttf");

#[derive(Clone, Copy, Debug, PartialEq)]
struct AppliedFontSignature {
    family: UiFontFamily,
    size: f32,
    weight: i32,
}

#[derive(Clone, Debug, PartialEq)]
struct AppliedTextSignature {
    family: UiFontFamily,
    size: f32,
    weight: i32,
    open_type_features_enabled: bool,
    open_type_features_to_enable: String,
}

struct VertexApp {
    font_catalog: FontCatalog,
    available_ui_fonts: Vec<UiFontFamily>,
    config: Config,
    applied_font_signature: Option<AppliedFontSignature>,
    applied_text_signature: Option<AppliedTextSignature>,
    effective_ui_font_family: UiFontFamily,
    theme_catalog: ui::theme::ThemeCatalog,
    theme: ui::theme::Theme,
    show_config_format_modal: bool,
    selected_config_format: ConfigFormat,
    default_config_format: ConfigFormat,
    config_creation_error: Option<String>,
    active_screen: screens::AppScreen,
    profile_shortcuts: Vec<ui::sidebar::ProfileShortcut>,
    selected_profile_id: Option<String>,
    text_ui: TextUi,
}

impl VertexApp {
    fn new(cc: &eframe::CreationContext<'_>, config_state: LoadConfigResult) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let (mut config, show_config_format_modal, selected_config_format, default_config_format) =
            match config_state {
                LoadConfigResult::Loaded(config) => {
                    (config, false, ConfigFormat::Json, ConfigFormat::Json)
                }
                LoadConfigResult::Missing { default_format } => {
                    (Config::default(), true, default_format, default_format)
                }
            };
        config.normalize();
        window_effects::apply(cc, config.window_blur_enabled());
        let theme_catalog = ui::theme::ThemeCatalog::load();
        if !theme_catalog.contains(config.theme_id()) {
            config.set_theme_id(theme_catalog.default_theme_id().to_owned());
        }
        let theme = theme_catalog.resolve(config.theme_id()).clone();

        let mut cat = FontCatalog::new();
        cat.load_system();
        let available_ui_fonts = detect_available_ui_fonts(&cat);
        let mut text_ui = TextUi::new();
        text_ui.register_font_data(MAPLE_MONO_NF_REGULAR_TTF.to_vec());
        let effective_ui_font_family = config.ui_font_family();
        let mut app = Self {
            font_catalog: cat,
            available_ui_fonts,
            config,
            applied_font_signature: None,
            applied_text_signature: None,
            effective_ui_font_family,
            theme_catalog,
            theme,
            show_config_format_modal,
            selected_config_format,
            default_config_format,
            config_creation_error: None,
            active_screen: screens::AppScreen::Library,
            profile_shortcuts: Vec::new(),
            selected_profile_id: None,
            text_ui,
        };
        app.ensure_selected_font_is_available();
        app.apply_ui_font_from_config(&cc.egui_ctx);
        app
    }

    fn create_config_with_choice(&mut self, choice: ConfigFormat) {
        match create_default_config(choice) {
            Ok(config) => {
                self.config = config;
                self.config.normalize();
                self.ensure_selected_font_is_available();
                self.show_config_format_modal = false;
                self.config_creation_error = None;
            }
            Err(err) => {
                self.config_creation_error = Some(format!("Failed to create config: {err}"));
            }
        }
    }

    fn apply_ui_font_from_config(&mut self, ctx: &egui::Context) {
        let desired_font = AppliedFontSignature {
            family: self.config.ui_font_family(),
            size: self.config.ui_font_size(),
            weight: self.config.ui_font_weight(),
        };

        if self.applied_font_signature != Some(desired_font) {
            let mut applied_family = desired_font.family;
            if desired_font.family.is_included_default() {
                Self::install_included_maple_font(ctx, desired_font.size);
            } else {
                let spec = FontSpec::new(desired_font.family.query_families())
                    .weight(Weight(desired_font.weight.clamp(100, 900) as u16))
                    .slant(Slant::Upright)
                    .stretch(Stretch::Normal);

                if let Ok((bytes, _face_index)) = self.font_catalog.query_bytes(&spec) {
                    fontloader::egui_integration::install_font_as_primary(
                        ctx,
                        font_key(desired_font.family),
                        bytes,
                        desired_font.size,
                    );
                } else {
                    eprintln!(
                        "Configured font '{}' not available; falling back to included default.",
                        desired_font.family.label(),
                    );
                    Self::install_included_maple_font(ctx, desired_font.size);
                    applied_family = UiFontFamily::MapleMonoNf;
                }
            }

            self.effective_ui_font_family = applied_family;
            self.applied_font_signature = Some(desired_font);
        }

        let desired_text = AppliedTextSignature {
            family: self.effective_ui_font_family,
            size: self.config.ui_font_size(),
            weight: self.config.ui_font_weight(),
            open_type_features_enabled: self.config.open_type_features_enabled(),
            open_type_features_to_enable: self.config.open_type_features_to_enable().to_owned(),
        };

        if self.applied_text_signature == Some(desired_text.clone()) {
            return;
        }

        self.text_ui.apply_typography(
            self.effective_ui_font_family.query_families(),
            desired_text.size,
            desired_text.weight,
        );
        self.text_ui.apply_open_type_features(
            desired_text.open_type_features_enabled,
            &desired_text.open_type_features_to_enable,
            self.effective_ui_font_family.query_families(),
        );
        self.applied_text_signature = Some(desired_text);
    }

    fn ensure_selected_font_is_available(&mut self) {
        let available_ui_fonts = &self.available_ui_fonts;
        self.config.for_each_dropdown_mut(|setting, value| {
            if matches!(setting.id, DropdownSettingId::UiFontFamily)
                && !available_ui_fonts.contains(value)
            {
                *value = UiFontFamily::MapleMonoNf;
            }
        });
    }

    fn install_included_maple_font(ctx: &egui::Context, size_pt: f32) {
        fontloader::egui_integration::install_font_as_primary(
            ctx,
            font_key(UiFontFamily::MapleMonoNf),
            MAPLE_MONO_NF_REGULAR_TTF.to_vec(),
            size_pt,
        );
    }

    fn sync_theme_from_config(&mut self) {
        if !self.theme_catalog.contains(self.config.theme_id()) {
            self.config
                .set_theme_id(self.theme_catalog.default_theme_id().to_owned());
        }

        let resolved = self.theme_catalog.resolve(self.config.theme_id());
        if self.theme.id != resolved.id {
            self.theme = resolved.clone();
        }
    }

    fn render_config_format_modal(&mut self, ctx: &egui::Context) {
        egui::Window::new("")
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .title_bar(false)
            .show(ctx, |ui| {
                ui.set_min_width(360.0);
                let text_color = ui.visuals().text_color();
                let heading = LabelOptions {
                    font_size: 28.0,
                    line_height: 32.0,
                    weight: 700,
                    color: text_color,
                    wrap: false,
                    ..LabelOptions::default()
                };
                let body = LabelOptions {
                    color: text_color,
                    ..LabelOptions::default()
                };
                let _ = self
                    .text_ui
                    .label(ui, "config_modal_heading", "Config format", &heading);
                ui.add_space(8.0);

                let radio_style = ButtonOptions {
                    min_size: egui::vec2(ui.available_width(), 32.0),
                    corner_radius: 6,
                    padding: egui::vec2(8.0, 4.0),
                    text_color: text_color,
                    fill: ui.visuals().widgets.inactive.bg_fill,
                    fill_hovered: ui.visuals().widgets.hovered.bg_fill,
                    fill_active: ui.visuals().widgets.active.bg_fill,
                    fill_selected: ui.visuals().selection.bg_fill,
                    stroke: ui.visuals().widgets.inactive.bg_stroke,
                    ..ButtonOptions::default()
                };
                if self
                    .text_ui
                    .selectable_button(
                        ui,
                        "config_modal_fmt_toml",
                        ConfigFormat::Toml.label(),
                        self.selected_config_format == ConfigFormat::Toml,
                        &radio_style,
                    )
                    .clicked()
                {
                    self.selected_config_format = ConfigFormat::Toml;
                }
                ui.add_space(4.0);
                if self
                    .text_ui
                    .selectable_button(
                        ui,
                        "config_modal_fmt_json",
                        ConfigFormat::Json.label(),
                        self.selected_config_format == ConfigFormat::Json,
                        &radio_style,
                    )
                    .clicked()
                {
                    self.selected_config_format = ConfigFormat::Json;
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);
                let _ = self.text_ui.label(
                    ui,
                    "config_modal_desc",
                    "Choose a format to create your initial launcher config.",
                    &body,
                );

                if let Some(err) = &self.config_creation_error {
                    ui.add_space(6.0);
                    let mut err_style = body.clone();
                    err_style.color = ui.visuals().error_fg_color;
                    let _ = self.text_ui.label(ui, "config_modal_err", err, &err_style);
                }

                ui.add_space(12.0);
                let mut create_clicked = false;
                let mut cancel_clicked = false;

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let create_style = ButtonOptions {
                        min_size: egui::vec2(120.0, 30.0),
                        text_color: text_color,
                        fill: ui.visuals().widgets.inactive.bg_fill,
                        fill_hovered: ui.visuals().widgets.hovered.bg_fill,
                        fill_active: ui.visuals().widgets.active.bg_fill,
                        fill_selected: ui.visuals().selection.bg_fill,
                        stroke: ui.visuals().widgets.inactive.bg_stroke,
                        ..ButtonOptions::default()
                    };
                    let cancel_style = ButtonOptions {
                        min_size: egui::vec2(90.0, 30.0),
                        text_color: text_color,
                        fill: ui.visuals().widgets.inactive.bg_fill,
                        fill_hovered: ui.visuals().widgets.hovered.bg_fill,
                        fill_active: ui.visuals().widgets.active.bg_fill,
                        fill_selected: ui.visuals().selection.bg_fill,
                        stroke: ui.visuals().widgets.inactive.bg_stroke,
                        ..ButtonOptions::default()
                    };
                    create_clicked = self
                        .text_ui
                        .button(ui, "config_modal_create", "Create config", &create_style)
                        .clicked();
                    cancel_clicked = self
                        .text_ui
                        .button(ui, "config_modal_cancel", "Cancel", &cancel_style)
                        .clicked();
                });

                if cancel_clicked {
                    self.create_config_with_choice(self.default_config_format);
                } else if create_clicked {
                    self.create_config_with_choice(self.selected_config_format);
                }
            });
    }
}

impl eframe::App for VertexApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.text_ui.begin_frame(ctx);
        let previous_config = self.config.clone();
        self.sync_theme_from_config();
        self.theme.apply(ctx, self.config.window_blur_enabled());
        self.ensure_selected_font_is_available();
        self.apply_ui_font_from_config(ctx);
        ui::top_bar::render(ctx, self.active_screen, &mut self.text_ui);

        let modal_open = self.show_config_format_modal;
        let sidebar_output = ui::sidebar::render(ctx, self.active_screen, &self.profile_shortcuts);

        if let Some(next_screen) = sidebar_output.selected_screen {
            self.active_screen = next_screen;
        }

        if let Some(profile_id) = sidebar_output.selected_profile_id {
            self.selected_profile_id = Some(profile_id);
        }

        CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(egui::Margin::ZERO)
                    .outer_margin(egui::Margin::ZERO)
                    .stroke(egui::Stroke::new(
                        1.0,
                        ctx.style().visuals.widgets.noninteractive.bg_stroke.color,
                    )),
            )
            .show(ctx, |ui| {
                screens::render(
                    ui,
                    self.active_screen,
                    self.selected_profile_id.as_deref(),
                    &mut self.config,
                    &self.available_ui_fonts,
                    self.theme_catalog.themes(),
                    &mut self.text_ui,
                );
            });

        if modal_open {
            self.render_config_format_modal(ctx);
        }

        self.config.normalize();
        self.ensure_selected_font_is_available();
        if self.config != previous_config {
            if let Err(err) = save_config(&self.config) {
                eprintln!("Failed to save config: {err}");
            }
            self.apply_ui_font_from_config(ctx);
        }

        ui::top_bar::handle_window_resize(ctx);
    }
}

fn detect_available_ui_fonts(font_catalog: &FontCatalog) -> Vec<UiFontFamily> {
    let mut available = vec![UiFontFamily::MapleMonoNf];

    for candidate in UiFontFamily::system_options() {
        let spec = FontSpec::new(candidate.query_families())
            .weight(Weight::REGULAR)
            .slant(Slant::Upright)
            .stretch(Stretch::Normal);

        if font_catalog.query(&spec).is_ok() {
            available.push(*candidate);
        }
    }

    available
}

fn main() -> eframe::Result<()> {
    let config_state = load_config();
    let startup_config = match &config_state {
        LoadConfigResult::Loaded(config) => config.clone(),
        LoadConfigResult::Missing { .. } => Config::default(),
    };
    let startup_power_preference = if startup_config.low_power_gpu_preferred() {
        eframe::egui_wgpu::wgpu::PowerPreference::LowPower
    } else {
        eframe::egui_wgpu::wgpu::PowerPreference::HighPerformance
    };

    let options: eframe::NativeOptions = eframe::NativeOptions {
        viewport: egui::ViewportBuilder {
            title: Some("Vertex Launcher".into()),
            inner_size: Some(egui::vec2(1280.0, 800.0)),
            min_inner_size: Some(egui::vec2(320.0, 240.0)),
            resizable: Some(true),
            decorations: Some(false),
            transparent: Some(startup_config.window_blur_enabled()),
            ..Default::default()
        },
        renderer: eframe::Renderer::Wgpu,
        hardware_acceleration: eframe::HardwareAcceleration::Required,
        vsync: false,
        multisampling: 4,
        depth_buffer: 0,
        stencil_buffer: 0,
        dithering: false,
        centered: false,
        persist_window: false,
        event_loop_builder: None,
        window_builder: None,
        shader_version: None,
        run_and_return: false,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            present_mode: eframe::egui_wgpu::wgpu::PresentMode::AutoNoVsync,
            desired_maximum_frame_latency: Some(1),
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(
                eframe::egui_wgpu::WgpuSetupCreateNew {
                    instance_descriptor: eframe::egui_wgpu::wgpu::InstanceDescriptor {
                        backends: eframe::egui_wgpu::wgpu::Backends::VULKAN,
                        ..Default::default()
                    },
                    power_preference: startup_power_preference,
                    ..Default::default()
                },
            ),
            on_surface_error: std::sync::Arc::new(|_| {
                eframe::egui_wgpu::SurfaceErrorAction::RecreateSurface
            }),
        },
        ..Default::default()
    };

    eframe::run_native(
        "Vertex Launcher",
        options,
        Box::new(|cc| Ok(Box::new(VertexApp::new(cc, config_state)))),
    )
}

fn font_key(family: UiFontFamily) -> &'static str {
    match family {
        UiFontFamily::MapleMonoNf => "ui_font_maple_mono_nf",
        UiFontFamily::JetBrainsMono => "ui_font_jetbrains_mono",
        UiFontFamily::FiraCode => "ui_font_fira_code",
        UiFontFamily::CascadiaCode => "ui_font_cascadia_code",
        UiFontFamily::Iosevka => "ui_font_iosevka",
    }
}
