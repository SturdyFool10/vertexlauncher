use egui::{Button, Color32, ColorImage, Image, Response, Stroke, TextureHandle, Ui, vec2};
use image::{RgbaImage, imageops::FilterType};

use crate::ui::{motion, svg_aa};

pub fn svg(
    ui: &mut Ui,
    icon_id: &str,
    svg_bytes: &'static [u8],
    _tooltip: &str,
    selected: bool,
    max_button_width: f32,
) -> Response {
    let text_color = ui.visuals().text_color();
    let button_size = ui.available_width().min(max_button_width).max(1.0);
    let icon_size = (button_size - 8.0).clamp(10.0, button_size);
    let aa_mode = svg_aa::get_svg_aa_mode();
    let icon = if let Some(texture) =
        rasterized_svg_texture_handle(ui, icon_id, svg_bytes, text_color, icon_size, aa_mode)
    {
        Image::new((texture.id(), vec2(icon_size, icon_size)))
    } else {
        let themed_svg = apply_text_color(svg_bytes, text_color);
        let uri = format!(
            "bytes://vertex-icons/{icon_id}-{:02x}{:02x}{:02x}.svg",
            text_color.r(),
            text_color.g(),
            text_color.b()
        );
        Image::from_bytes(uri, themed_svg).fit_to_exact_size(vec2(icon_size, icon_size))
    };

    const CORNER_RADIUS_DEFAULT: f32 = 10.0;
    const CORNER_RADIUS_SELECTED: f32 = 5.0;
    let hover_progress_id = ui.make_persistent_id(icon_id).with("hover_progress");
    let hover_progress = ui
        .ctx()
        .data_mut(|d| d.get_temp::<f32>(hover_progress_id))
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let hover_target_radius =
        CORNER_RADIUS_DEFAULT - ((CORNER_RADIUS_DEFAULT - CORNER_RADIUS_SELECTED) * 0.5);
    let corner_radius = if selected {
        CORNER_RADIUS_SELECTED
    } else {
        CORNER_RADIUS_DEFAULT + (hover_target_radius - CORNER_RADIUS_DEFAULT) * hover_progress
    };
    let button = Button::image(icon)
        .frame(true)
        .corner_radius(egui::CornerRadius::same(corner_radius.round() as u8))
        .stroke(Stroke::new(
            1.0,
            ui.visuals().widgets.inactive.bg_stroke.color,
        ))
        .fill(if selected {
            ui.visuals().selection.bg_fill
        } else {
            ui.visuals().widgets.inactive.weak_bg_fill
        });

    let response = ui.add_sized([button_size, button_size], button);
    let emphasis = response.hovered() || response.has_focus();
    let progress = motion::progress(ui.ctx(), response.id.with("hover_anim"), emphasis);
    ui.ctx()
        .data_mut(|d| d.insert_temp(hover_progress_id, progress));
    if progress > 0.0 {
        let stroke_color = ui
            .visuals()
            .widgets
            .hovered
            .bg_stroke
            .color
            .gamma_multiply((0.35 + (0.65 * progress)).clamp(0.0, 1.0));
        let stroke = Stroke::new(1.0 + (0.8 * progress), stroke_color);
        let radius = if selected {
            corner_radius.round() as u8
        } else {
            ((button_size * 0.2).round() as u8).max(4)
        };
        ui.painter().rect_stroke(
            response.rect,
            egui::CornerRadius::same(radius),
            stroke,
            egui::StrokeKind::Inside,
        );
        if motion::is_animating(progress) {
            ui.ctx().request_repaint();
        }
    }
    response
}

fn rasterized_svg_texture_handle(
    ui: &mut Ui,
    icon_id: &str,
    svg_bytes: &[u8],
    text_color: Color32,
    icon_size: f32,
    aa_mode: config::SvgAaMode,
) -> Option<TextureHandle> {
    let edge = icon_size.max(1.0).round() as u32;
    let cache_id = ui.make_persistent_id((
        "svg_raster_cache",
        icon_id,
        edge,
        text_color.r(),
        text_color.g(),
        text_color.b(),
        aa_mode,
    ));

    if let Some(texture) = ui.ctx().data_mut(|d| d.get_temp::<TextureHandle>(cache_id)) {
        return Some(texture);
    }

    let texture_name = format!(
        "vertex-svg-raster-{icon_id}-{edge}-{:02x}{:02x}{:02x}-{:?}",
        text_color.r(),
        text_color.g(),
        text_color.b(),
        aa_mode
    );
    let themed_svg = apply_text_color(svg_bytes, text_color);
    let texture = rasterize_svg_texture(ui, &texture_name, &themed_svg, edge, aa_mode)?;
    ui.ctx()
        .data_mut(|d| d.insert_temp(cache_id, texture.clone()));
    Some(texture)
}

fn rasterize_svg_texture(
    ui: &Ui,
    texture_name: &str,
    svg_bytes: &[u8],
    edge: u32,
    aa_mode: config::SvgAaMode,
) -> Option<TextureHandle> {
    let scale = aa_mode.supersample_scale().max(1);
    let raster_edge = edge.saturating_mul(scale);
    let options = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(svg_bytes, &options).ok()?;
    let svg_size = tree.size();
    let sx = raster_edge as f32 / svg_size.width();
    let sy = raster_edge as f32 / svg_size.height();
    let transform = resvg::tiny_skia::Transform::from_scale(sx, sy);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(raster_edge, raster_edge)?;
    let mut pixmap_mut = pixmap.as_mut();
    resvg::render(&tree, transform, &mut pixmap_mut);

    let rgba = RgbaImage::from_raw(raster_edge, raster_edge, pixmap.data().to_vec())?;
    let final_rgba = if scale > 1 {
        image::imageops::resize(&rgba, edge, edge, FilterType::CatmullRom)
    } else {
        rgba
    };
    let color_image =
        ColorImage::from_rgba_premultiplied([edge as usize, edge as usize], final_rgba.as_raw());

    Some(ui.ctx().load_texture(
        texture_name.to_owned(),
        color_image,
        egui::TextureOptions::LINEAR,
    ))
}

fn apply_text_color(svg_bytes: &[u8], color: Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let svg = String::from_utf8_lossy(svg_bytes).replace("currentColor", &color_hex);
    svg.into_bytes()
}
