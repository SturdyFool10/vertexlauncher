use config::{Config, ConfigFormat, LoadConfigResult, create_default_config, load_config};
use eframe::{self, egui};
use egui::CentralPanel;
use fontloader::{FontCatalog, FontSpec, Slant, Stretch, Weight};

mod assets;
mod screens;
mod ui;

struct VertexApp {
    _font_catalog: FontCatalog,
    config: Config,
    theme: ui::theme::Theme,
    show_config_format_modal: bool,
    selected_config_format: ConfigFormat,
    default_config_format: ConfigFormat,
    config_creation_error: Option<String>,
    active_screen: screens::AppScreen,
    profile_shortcuts: Vec<ui::sidebar::ProfileShortcut>,
    selected_profile_id: Option<String>,
}

impl VertexApp {
    fn new(cc: &eframe::CreationContext<'_>, config_state: LoadConfigResult) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let mut cat = FontCatalog::new();
        cat.load_system();

        let spec = FontSpec::new(&["Maple Mono NF"])
            .weight(Weight::REGULAR)
            .slant(Slant::Upright)
            .stretch(Stretch::Normal);

        if let Ok((bytes, _face_index)) = cat.query_bytes(&spec) {
            fontloader::egui_integration::install_font_as_primary(
                &cc.egui_ctx,
                "maple_mono_nf_regular",
                bytes,
                18.0,
            );
        } else {
            eprintln!("Maple Mono NF Regular not found; using egui default fonts.");
        }

        let (config, show_config_format_modal, selected_config_format, default_config_format) =
            match config_state {
                LoadConfigResult::Loaded(config) => {
                    (config, false, ConfigFormat::Json, ConfigFormat::Json)
                }
                LoadConfigResult::Missing { default_format } => {
                    (Config::default(), true, default_format, default_format)
                }
            };

        Self {
            _font_catalog: cat,
            config,
            theme: ui::theme::Theme::default(),
            show_config_format_modal,
            selected_config_format,
            default_config_format,
            config_creation_error: None,
            active_screen: screens::AppScreen::Library,
            profile_shortcuts: Vec::new(),
            selected_profile_id: None,
        }
    }

    fn create_config_with_choice(&mut self, choice: ConfigFormat) {
        match create_default_config(choice) {
            Ok(config) => {
                self.config = config;
                self.show_config_format_modal = false;
                self.config_creation_error = None;
            }
            Err(err) => {
                self.config_creation_error = Some(format!("Failed to create config: {err}"));
            }
        }
    }

    fn render_config_format_modal(&mut self, ctx: &egui::Context) {
        egui::Window::new("Select config format")
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .show(ctx, |ui| {
                ui.set_min_width(360.0);
                ui.heading("Config format");
                ui.add_space(8.0);

                ui.radio_value(
                    &mut self.selected_config_format,
                    ConfigFormat::Toml,
                    ConfigFormat::Toml.label(),
                );
                ui.radio_value(
                    &mut self.selected_config_format,
                    ConfigFormat::Json,
                    ConfigFormat::Json.label(),
                );

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);
                ui.label("Choose a format to create your initial launcher config.");

                if let Some(err) = &self.config_creation_error {
                    ui.add_space(6.0);
                    ui.colored_label(egui::Color32::from_rgb(220, 80, 80), err);
                }

                ui.add_space(12.0);
                let mut create_clicked = false;
                let mut cancel_clicked = false;

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    create_clicked = ui
                        .add_sized([120.0, 28.0], egui::Button::new("Create config"))
                        .clicked();
                    cancel_clicked = ui
                        .add_sized([90.0, 28.0], egui::Button::new("Cancel"))
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
        self.theme.apply(ctx);
        ui::top_bar::render(ctx, self.active_screen);

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
                );
            });

        if modal_open {
            self.render_config_format_modal(ctx);
        }
    }
}

fn main() -> eframe::Result<()> {
    let options: eframe::NativeOptions = eframe::NativeOptions {
        viewport: egui::ViewportBuilder {
            title: Some("Vertex Launcher".into()),
            inner_size: Some(egui::vec2(1280.0, 800.0)),
            min_inner_size: Some(egui::vec2(320.0, 240.0)),
            resizable: Some(true),
            decorations: Some(false),
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
                    power_preference: eframe::egui_wgpu::wgpu::PowerPreference::LowPower,
                    ..Default::default()
                },
            ),
            on_surface_error: std::sync::Arc::new(|_| {
                eframe::egui_wgpu::SurfaceErrorAction::RecreateSurface
            }),
        },
        ..Default::default()
    };

    let config_state = load_config();
    eframe::run_native(
        "Vertex Launcher",
        options,
        Box::new(|cc| Ok(Box::new(VertexApp::new(cc, config_state)))),
    )
}
