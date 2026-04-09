use super::*;

/// Find the glyph index and x-offset within that glyph for a cursor on the given layout run.
/// Replicates cosmic-text's private `cursor_glyph_opt` function.
fn editor_cursor_glyph_opt(cursor: &Cursor, run: &LayoutRun<'_>) -> Option<(usize, f32)> {
    if cursor.line != run.line_i {
        return None;
    }
    for (glyph_i, glyph) in run.glyphs.iter().enumerate() {
        if cursor.index == glyph.start {
            return Some((glyph_i, 0.0));
        } else if cursor.index > glyph.start && cursor.index < glyph.end {
            let cluster = &run.text[glyph.start..glyph.end];
            let total = cluster.grapheme_indices(true).count().max(1);
            let before = run.text[glyph.start..cursor.index]
                .grapheme_indices(true)
                .count();
            return Some((glyph_i, glyph.w * before as f32 / total as f32));
        }
    }
    if let Some(last) = run.glyphs.last() {
        if cursor.index == last.end {
            return Some((run.glyphs.len(), 0.0));
        }
    } else {
        return Some((0, 0.0));
    }
    None
}

pub(crate) fn editor_cursor_x_in_run(
    cursor: &Cursor,
    run: &LayoutRun<'_>,
    fundamentals: &TextFundamentals,
    scale: f32,
) -> Option<i32> {
    let (cursor_glyph, cursor_glyph_offset) = editor_cursor_glyph_opt(cursor, run)?;
    let prefixes = collect_glyph_spacing_prefixes_px(run.text, run.glyphs, fundamentals, scale);
    let x = run.glyphs.get(cursor_glyph).map_or_else(
        || {
            run.glyphs.last().map_or(0, |g| {
                let prefix_px = prefixes.last().copied().unwrap_or(0.0);
                let glyph_x = adjusted_glyph_x_px(g, prefix_px);
                if g.level.is_rtl() {
                    glyph_x as i32
                } else {
                    (glyph_x + g.w) as i32
                }
            })
        },
        |g| {
            let prefix_px = prefixes.get(cursor_glyph).copied().unwrap_or(0.0);
            let glyph_x = adjusted_glyph_x_px(g, prefix_px);
            if g.level.is_rtl() {
                (glyph_x + g.w - cursor_glyph_offset) as i32
            } else {
                (glyph_x + cursor_glyph_offset) as i32
            }
        },
    );
    Some(x)
}

pub(crate) fn editor_sel_rect(
    min_x: i32,
    max_x: i32,
    line_top: f32,
    line_height: f32,
    horiz_scroll_px: f32,
    origin: Pos2,
    scale: f32,
) -> Rect {
    Rect::from_min_size(
        Pos2::new(
            (min_x as f32 - horiz_scroll_px) / scale + origin.x,
            line_top / scale + origin.y,
        ),
        Vec2::new((max_x - min_x).max(0) as f32 / scale, line_height / scale),
    )
}
