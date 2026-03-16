use egui::{self, Frame, ScrollArea, Stroke, Ui};
use textui::{CodeBlockOptions, LabelOptions, TextUi};

use crate::{
    assets,
    ui::{motion, style},
};

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

const LEGAL_NOTICES: &[LegalNotice] = &[
    legal_notice!("Tabler Icons (MIT)", "../legal/TABLER_LICENSE.txt"),
    legal_notice!("Maple Mono NF (SIL OFL 1.1)", "../legal/MAPLE_LICENSE.txt"),
];

pub fn render(ui: &mut Ui, text_ui: &mut TextUi) {
    ui.add_space(style::SPACE_MD);

    let content_height = ui.available_height().max(0.0);
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), content_height),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            section_frame(ui).show(ui, |ui| {
                let text_color = ui.visuals().text_color();
                let heading = LabelOptions {
                    font_size: 26.0,
                    line_height: 30.0,
                    weight: 700,
                    color: text_color,
                    wrap: false,
                    ..LabelOptions::default()
                };
                let body = LabelOptions {
                    color: text_color,
                    ..LabelOptions::default()
                };

                let _ = text_ui.label(ui, "legal_included_heading", "Included Notices", &heading);
                ui.add_space(style::SPACE_XS);
                ui.separator();
                ui.add_space(style::SPACE_SM);
                let _ = text_ui.label(
                    ui,
                    "legal_open_notice",
                    "Open a notice below to read its full legal text.",
                    &body,
                );
                let mut weak = body.clone();
                weak.color = ui.visuals().weak_text_color();
                weak.font_size = 13.0;
                weak.line_height = 16.0;
                let _ = text_ui.label(
                    ui,
                    "legal_notice_count",
                    &format!("{} total", LEGAL_NOTICES.len()),
                    &weak,
                );
            });

            ui.add_space(style::SPACE_XL);

            let notices_height = ui.available_height().max(0.0);
            ScrollArea::vertical()
                .id_salt("legal_notice_accordion")
                .max_height(notices_height)
                .show(ui, |ui| {
                    for (index, notice) in LEGAL_NOTICES.iter().enumerate() {
                        section_frame(ui).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let open_id =
                                ui.make_persistent_id(("legal_notice_open", notice.source_path));
                            let mut is_open = ui
                                .ctx()
                                .data_mut(|d| d.get_persisted::<bool>(open_id))
                                .unwrap_or(false);

                            let header_height = 34.0;
                            let (header_rect, header_response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), header_height),
                                egui::Sense::click(),
                            );
                            if header_response.clicked() {
                                is_open = !is_open;
                                ui.ctx().data_mut(|d| d.insert_persisted(open_id, is_open));
                            }

                            let interact = ui.style().interact(&header_response);
                            let header_fill = interact.bg_fill;
                            let header_stroke = interact.bg_stroke;
                            let header_text_color = interact.text_color();
                            ui.painter().rect(
                                header_rect,
                                7.0,
                                header_fill,
                                header_stroke,
                                egui::StrokeKind::Inside,
                            );

                            let icon_size = ui
                                .text_style_height(&egui::TextStyle::Button)
                                .clamp(14.0, 20.0);
                            let icon_rect = egui::Rect::from_min_size(
                                egui::pos2(
                                    header_rect.left() + 8.0,
                                    header_rect.center().y - icon_size * 0.5,
                                ),
                                egui::vec2(icon_size, icon_size),
                            );
                            let (icon_id, icon_bytes) = if is_open {
                                ("legal-chevron-down", assets::CHEVRON_DOWN_SVG)
                            } else {
                                ("legal-chevron-right", assets::CHEVRON_RIGHT_SVG)
                            };
                            let icon =
                                themed_svg_image(icon_id, icon_bytes, header_text_color, icon_size);
                            ui.put(icon_rect, icon);

                            let text_rect = egui::Rect::from_min_max(
                                egui::pos2(icon_rect.right() + 8.0, header_rect.top()),
                                egui::pos2(header_rect.right() - 8.0, header_rect.bottom()),
                            );
                            let mut title_style = LabelOptions::default();
                            title_style.font_size = 17.0;
                            title_style.line_height = 22.0;
                            title_style.weight = 700;
                            title_style.wrap = false;
                            title_style.color = header_text_color;
                            ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
                                ui.with_layout(
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        let _ = text_ui.label(
                                            ui,
                                            ("legal_notice_title", index),
                                            notice.friendly_name,
                                            &title_style,
                                        );
                                    },
                                );
                            });

                            let openness =
                                motion::progress(ui.ctx(), open_id.with("anim"), is_open);
                            if motion::is_animating(openness) {
                                ui.ctx().request_repaint();
                            }
                            if openness > 0.001 {
                                ui.add_space(style::SPACE_XS * openness);
                                ui.scope(|ui| {
                                    ui.set_opacity(openness);
                                    let mut weak = LabelOptions::default();
                                    weak.color = ui.visuals().weak_text_color();
                                    weak.font_size = 13.0;
                                    weak.line_height = 16.0;
                                    weak.wrap = true;
                                    let _ = text_ui.label(
                                        ui,
                                        ("notice_path", index),
                                        notice.source_path,
                                        &weak,
                                    );
                                    ui.add_space(style::SPACE_MD);
                                    ui.separator();
                                    ui.add_space(style::SPACE_MD);

                                    let full_text_height =
                                        (ui.available_height() * 0.45).clamp(180.0, 480.0);
                                    let text_height = full_text_height * openness;
                                    let code_options = CodeBlockOptions {
                                        text_color: ui.visuals().text_color(),
                                        background_color: ui.visuals().code_bg_color,
                                        stroke: ui.visuals().widgets.noninteractive.bg_stroke,
                                        language: Some("text".to_owned()),
                                        wrap: true,
                                        ..CodeBlockOptions::default()
                                    };
                                    if openness >= 0.98 {
                                        ScrollArea::vertical()
                                            .id_salt(format!("legal_notice_text_{index}"))
                                            .max_height(text_height)
                                            .auto_shrink([false, false])
                                            .show(ui, |ui| {
                                                let _ = text_ui.code_block_async(
                                                    ui,
                                                    ("notice_text_async", index),
                                                    notice.text,
                                                    &code_options,
                                                );
                                            });
                                    } else {
                                        let (placeholder_rect, _) = ui.allocate_exact_size(
                                            egui::vec2(ui.available_width(), text_height.max(0.0)),
                                            egui::Sense::hover(),
                                        );
                                        if text_height > 1.0 {
                                            ui.painter().rect_filled(
                                                placeholder_rect,
                                                egui::CornerRadius::same(style::CORNER_RADIUS_SM),
                                                ui.visuals().faint_bg_color,
                                            );
                                        }
                                    }
                                });
                            }
                        });
                        if index + 1 < LEGAL_NOTICES.len() {
                            ui.add_space(style::SPACE_LG);
                        }
                    }
                });
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
        .corner_radius(egui::CornerRadius::same(style::CORNER_RADIUS_MD))
        .inner_margin(egui::Margin::same(style::SPACE_XL as i8))
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
