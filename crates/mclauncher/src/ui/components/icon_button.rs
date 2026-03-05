use egui::{Button, Color32, Image, Response, Stroke, Ui, vec2};

pub fn svg(
    ui: &mut Ui,
    icon_id: &str,
    svg_bytes: &'static [u8],
    tooltip: &str,
    selected: bool,
    max_button_width: f32,
) -> Response {
    let text_color = ui.visuals().text_color();
    let themed_svg = apply_text_color(svg_bytes, text_color);
    let uri = format!(
        "bytes://vertex-icons/{icon_id}-{:02x}{:02x}{:02x}.svg",
        text_color.r(),
        text_color.g(),
        text_color.b()
    );
    let button_size = ui.available_width().min(max_button_width).max(1.0);
    let icon_size = (button_size - 8.0).clamp(10.0, button_size);
    let icon = Image::from_bytes(uri, themed_svg).fit_to_exact_size(vec2(icon_size, icon_size));

    let button = Button::image(icon)
        .frame(true)
        .stroke(Stroke::new(
            1.0,
            ui.visuals().widgets.inactive.bg_stroke.color,
        ))
        .fill(if selected {
            ui.visuals().selection.bg_fill
        } else {
            ui.visuals().widgets.inactive.weak_bg_fill
        });

    ui.add_sized([button_size, button_size], button)
        .on_hover_text(tooltip)
}

fn apply_text_color(svg_bytes: &[u8], color: Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let svg = String::from_utf8_lossy(svg_bytes).replace("currentColor", &color_hex);
    svg.into_bytes()
}
