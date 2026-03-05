use egui::{self, Frame, RichText, ScrollArea, Stroke, Ui};

use crate::assets;

struct LegalNotice {
    friendly_name: &'static str,
    source_path: &'static str,
    text: &'static str,
}

macro_rules! legal_notice {
    ($friendly_name:literal, $path:literal) => {
        LegalNotice {
            friendly_name: $friendly_name,
            source_path: $path,
            text: include_str!($path),
        }
    };
}

// Add new legal files here using (Friendly Name, include_str path).
const LEGAL_NOTICES: &[LegalNotice] = &[
    legal_notice!("Tabler Icons (MIT)", "../legal/TABLER_LICENSE.txt"),
    legal_notice!("Maple Mono NF (SIL OFL 1.1)", "../legal/MAPLE_LICENSE.txt"),
];

pub fn render(ui: &mut Ui) {
    ui.heading("Legal");
    ui.add_space(8.0);
    ui.label("Third-party license notices bundled with Vertex.");
    ui.add_space(12.0);

    let content_height = ui.available_height().max(0.0);
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), content_height),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            ui.add_space(5.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width().max(0.0), content_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    section_frame(ui).show(ui, |ui| {
                        ui.label(RichText::new("Included Notices").strong());
                        ui.add_space(4.0);
                        ui.separator();
                        ui.add_space(4.0);
                        ui.label("Open a notice below to read its full legal text.");
                        ui.label(
                            RichText::new(format!("{} total", LEGAL_NOTICES.len()))
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    });

                    ui.add_space(12.0);

                    let notices_height = ui.available_height().max(0.0);
                    ScrollArea::vertical()
                        .id_salt("legal_notice_accordion")
                        .max_height(notices_height)
                        .show(ui, |ui| {
                            for (index, notice) in LEGAL_NOTICES.iter().enumerate() {
                                section_frame(ui).show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    let title =
                                        RichText::new(notice.friendly_name).strong().size(17.0);
                                    egui::CollapsingHeader::new(title)
                                        .id_salt(notice.source_path)
                                        .show_background(true)
                                        .icon(paint_chevron_icon)
                                        .show(ui, |ui| {
                                            ui.add_space(2.0);
                                            ui.label(
                                                RichText::new(notice.source_path)
                                                    .small()
                                                    .color(ui.visuals().weak_text_color()),
                                            );
                                            ui.add_space(8.0);
                                            ui.separator();
                                            ui.add_space(8.0);

                                            let text_height =
                                                (ui.available_height() * 0.45).clamp(180.0, 480.0);
                                            let text_width = ui.available_width();
                                            ScrollArea::vertical()
                                                .id_salt(format!("legal_notice_text_{index}"))
                                                .max_height(text_height)
                                                .auto_shrink([false, false])
                                                .show(ui, |ui| {
                                                    ui.set_min_width(text_width);
                                                    ui.label(
                                                        RichText::new(notice.text).monospace(),
                                                    );
                                                });
                                        });
                                });

                                if index + 1 < LEGAL_NOTICES.len() {
                                    ui.add_space(10.0);
                                }
                            }
                        });
                },
            );
        },
    );
}

fn section_frame(ui: &Ui) -> Frame {
    Frame::new()
        .fill(ui.visuals().widgets.noninteractive.bg_fill)
        .stroke(Stroke::new(
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(12))
}

fn paint_chevron_icon(ui: &mut Ui, openness: f32, response: &egui::Response) {
    let (icon_id, icon_bytes) = if openness >= 0.5 {
        ("legal-chevron-down", assets::CHEVRON_DOWN_SVG)
    } else {
        ("legal-chevron-right", assets::CHEVRON_RIGHT_SVG)
    };
    let icon_color = ui.style().interact(response).fg_stroke.color;
    let icon_size = ui
        .text_style_height(&egui::TextStyle::Button)
        .clamp(14.0, 20.0);
    let icon_rect =
        egui::Rect::from_center_size(response.rect.center(), egui::vec2(icon_size, icon_size));
    let icon = themed_svg_image(icon_id, icon_bytes, icon_color, icon_size);
    ui.put(icon_rect, icon);
}

fn themed_svg_image(
    icon_id: &str,
    svg_bytes: &[u8],
    color: egui::Color32,
    icon_size: f32,
) -> egui::Image<'static> {
    let themed_svg = apply_svg_color(svg_bytes, color);
    let uri = format!(
        "bytes://vertex-legal-icons/{icon_id}-{:02x}{:02x}{:02x}.svg",
        color.r(),
        color.g(),
        color.b()
    );
    egui::Image::from_bytes(uri, themed_svg).fit_to_exact_size(egui::vec2(icon_size, icon_size))
}

fn apply_svg_color(svg_bytes: &[u8], color: egui::Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    String::from_utf8_lossy(svg_bytes)
        .replace("currentColor", &color_hex)
        .into_bytes()
}
