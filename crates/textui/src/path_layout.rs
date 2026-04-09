use super::*;

#[path = "path_layout/text_path_sample.rs"]
mod text_path_sample;

use self::text_path_sample::TextPathSample;

pub(super) fn build_path_layout_from_prepared_layout(
    layout: &PreparedTextLayout,
    fallback_advance_points: f32,
    line_height_points: f32,
    path: &TextPath,
    path_options: &TextPathOptions,
) -> Result<TextPathLayout, TextPathError> {
    if path.points.is_empty() {
        return Err(TextPathError::EmptyPath);
    }
    if layout.glyphs.is_empty() {
        return Err(TextPathError::EmptyText);
    }
    let path_length = text_path_length(path);
    if path_length <= f32::EPSILON {
        return Err(TextPathError::PathTooShort);
    }

    let baseline_y = layout.glyphs[0].offset_points.y;
    let mut glyphs = Vec::with_capacity(layout.glyphs.len());
    let mut bounds: Option<Rect> = None;
    for (index, glyph) in layout.glyphs.iter().enumerate() {
        let advance_points =
            estimated_glyph_advance_points(&layout.glyphs, index, fallback_advance_points);
        let distance =
            (path_options.start_offset_points + glyph.offset_points.x).clamp(0.0, path_length);
        let sample = sample_text_path(path, distance).ok_or(TextPathError::PathTooShort)?;
        let baseline_offset =
            glyph.offset_points.y - baseline_y + path_options.normal_offset_points;
        let anchor = sample.position + sample.normal * baseline_offset;
        let rotation_radians = if path_options.rotate_glyphs {
            sample.tangent.y.atan2(sample.tangent.x)
        } else {
            0.0
        };
        let glyph_rect = Rect::from_center_size(
            anchor,
            egui::vec2(advance_points.max(1.0), line_height_points.max(1.0)),
        );
        bounds = Some(bounds.map_or(glyph_rect, |current| current.union(glyph_rect)));
        glyphs.push(TextPathGlyph {
            anchor: anchor.into(),
            tangent: sample.tangent.into(),
            normal: sample.normal.into(),
            rotation_radians,
            local_offset: egui::vec2(0.0, baseline_offset).into(),
            advance_points,
            color: glyph.color.into(),
        });
    }

    Ok(TextPathLayout {
        glyphs,
        bounds: bounds.unwrap_or(Rect::NOTHING).into(),
        total_advance_points: layout.size_points.x,
        path_length_points: path_length,
    })
}

pub(super) fn export_prepared_layout_as_shapes(
    layout: &PreparedTextLayout,
    font_system: &mut FontSystem,
    scale_context: &mut ScaleContext,
    fallback_line_height: f32,
    rasterization: TextRasterizationConfig,
) -> VectorTextShape {
    let mut glyphs = Vec::with_capacity(layout.glyphs.len());
    let mut bounds: Option<Rect> = None;
    for glyph in layout.glyphs.iter() {
        let Some(shape) = export_vector_glyph_shape(
            font_system,
            scale_context,
            glyph,
            fallback_line_height,
            rasterization,
        ) else {
            continue;
        };
        bounds = Some(bounds.map_or(shape.bounds.into(), |current| {
            current.union(shape.bounds.into())
        }));
        glyphs.push(shape);
    }
    VectorTextShape {
        glyphs,
        bounds: bounds.unwrap_or(Rect::NOTHING).into(),
    }
}

fn text_path_length(path: &TextPath) -> f32 {
    iter_text_path_segments(path)
        .map(|(from, to)| (to - from).length())
        .sum()
}

fn iter_text_path_segments(path: &TextPath) -> impl Iterator<Item = (Pos2, Pos2)> + '_ {
    let open_segments = path.points.windows(2).map(|points| {
        (
            egui_point_from_text(points[0]),
            egui_point_from_text(points[1]),
        )
    });
    let closing_segment = if path.closed && path.points.len() > 2 {
        Some((
            egui_point_from_text(*path.points.last().unwrap_or(&TextPoint::ZERO)),
            egui_point_from_text(path.points[0]),
        ))
    } else {
        None
    };
    open_segments.chain(closing_segment)
}

fn sample_text_path(path: &TextPath, distance_points: f32) -> Option<TextPathSample> {
    let mut remaining = distance_points.max(0.0);
    for (from, to) in iter_text_path_segments(path) {
        let delta = to - from;
        let segment_length = delta.length();
        if segment_length <= f32::EPSILON {
            continue;
        }
        let tangent = delta / segment_length;
        let normal = egui::vec2(-tangent.y, tangent.x);
        if remaining <= segment_length {
            let position = from + tangent * remaining;
            return Some(TextPathSample {
                position,
                tangent,
                normal,
            });
        }
        remaining -= segment_length;
    }
    let (&last_but_one, &last) = match path.points.as_slice() {
        [.., a, b] => (a, b),
        _ => return None,
    };
    let last_but_one = egui_point_from_text(last_but_one);
    let last = egui_point_from_text(last);
    let delta = last - last_but_one;
    let segment_length = delta.length();
    if segment_length <= f32::EPSILON {
        return None;
    }
    let tangent = delta / segment_length;
    Some(TextPathSample {
        position: last,
        tangent,
        normal: egui::vec2(-tangent.y, tangent.x),
    })
}

fn estimated_glyph_advance_points(
    glyphs: &[PreparedGlyph],
    index: usize,
    fallback_font_size: f32,
) -> f32 {
    if let Some(next) = glyphs.get(index + 1) {
        (next.offset_points.x - glyphs[index].offset_points.x).max(0.0)
    } else {
        fallback_font_size.max(1.0) * 0.5
    }
}

fn export_vector_glyph_shape(
    font_system: &mut FontSystem,
    scale_context: &mut ScaleContext,
    glyph: &PreparedGlyph,
    fallback_line_height: f32,
    rasterization: TextRasterizationConfig,
) -> Option<VectorGlyphShape> {
    let commands =
        render_swash_outline_commands(font_system, scale_context, &glyph.cache_key, rasterization)?;
    let mut vector_commands = Vec::with_capacity(commands.len());
    let mut bounds: Option<Rect> = None;
    for command in commands.iter() {
        let mapped = map_outline_command_to_points(*command, glyph.offset_points);
        update_vector_shape_bounds(&mut bounds, &mapped);
        vector_commands.push(mapped);
    }
    Some(VectorGlyphShape {
        bounds: bounds
            .unwrap_or_else(|| {
                Rect::from_min_size(
                    Pos2::new(
                        glyph.offset_points.x,
                        glyph.offset_points.y - fallback_line_height,
                    ),
                    egui::vec2(1.0, fallback_line_height.max(1.0)),
                )
            })
            .into(),
        color: glyph.color.into(),
        commands: vector_commands,
    })
}

fn map_outline_command_to_points(
    command: swash::zeno::Command,
    glyph_origin: Vec2,
) -> VectorPathCommand {
    let map_point =
        |point: swash::zeno::Point| Pos2::new(glyph_origin.x + point.x, glyph_origin.y - point.y);
    match command {
        swash::zeno::Command::MoveTo(point) => VectorPathCommand::MoveTo(map_point(point).into()),
        swash::zeno::Command::LineTo(point) => VectorPathCommand::LineTo(map_point(point).into()),
        swash::zeno::Command::QuadTo(control, point) => {
            VectorPathCommand::QuadTo(map_point(control).into(), map_point(point).into())
        }
        swash::zeno::Command::CurveTo(control_a, control_b, point) => VectorPathCommand::CurveTo(
            map_point(control_a).into(),
            map_point(control_b).into(),
            map_point(point).into(),
        ),
        swash::zeno::Command::Close => VectorPathCommand::Close,
    }
}

fn update_vector_shape_bounds(bounds: &mut Option<Rect>, command: &VectorPathCommand) {
    let mut include_point = |point: Pos2| {
        let rect = Rect::from_min_max(point, point);
        *bounds = Some(bounds.map_or(rect, |current| current.union(rect)));
    };
    match command {
        VectorPathCommand::MoveTo(point) | VectorPathCommand::LineTo(point) => {
            include_point((*point).into())
        }
        VectorPathCommand::QuadTo(control, point) => {
            include_point((*control).into());
            include_point((*point).into());
        }
        VectorPathCommand::CurveTo(control_a, control_b, point) => {
            include_point((*control_a).into());
            include_point((*control_b).into());
            include_point((*point).into());
        }
        VectorPathCommand::Close => {}
    }
}
