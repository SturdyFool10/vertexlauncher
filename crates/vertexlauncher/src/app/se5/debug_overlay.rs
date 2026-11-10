//! Debug overlays, switched on in Settings: an FPS counter and a live per-pass frame graph. Both float over the page
//! without taking the pointer. The pass times come from the engine's last completed frame (CPU always, GPU where the
//! device supports timestamp queries), so they trail what is on screen by a frame or two.

use vertex_i18n::tr;
use std::collections::VecDeque;

use config::Config;
use vertex_shell::sturdy::diagnostics::FrameTimings;
use vertex_shell::sturdy::glam::{Vec2, Vec4};
use vertex_shell::sturdy::ui::{
    AlignX, AlignY, AttachPoint, Border, ChildAlignment, CornerRadius, Direction, ElementDesc, FloatTarget, Floating,
    ElementScope, Padding, Sizing, Ui,
};

use super::kit::{self, Kit, TextRole};

/// Frames of history the graph and the FPS average cover.
const HISTORY: usize = 180;
/// The frame time one 60 Hz display refresh allows, for the budget line.
const BUDGET_MS: f32 = 1000.0 / 60.0;
const PANEL_WIDTH: f32 = 400.0;
const MARGIN: f32 = 12.0;
const TITLE_BAR: f32 = 40.0;
const HISTORY_HEIGHT: f32 = 48.0;
const BAR_WIDTH: f32 = 150.0;
const BAR_HEIGHT: f32 = 8.0;
/// The most rows shown per list; the rest are summed into one "more" row.
const MAX_ROWS: usize = 14;
/// The frame graph's pass lists are re-read at this interval so the numbers stay legible.
const TIMINGS_REFRESH_SECONDS: f32 = 0.1;

#[derive(Default)]
pub struct DebugOverlay {
    /// Milliseconds between frames, newest last.
    frame_ms: VecDeque<f32>,
    window_id: Option<u64>,
    timings: FrameTimings,
    since_timings: f32,
}

impl DebugOverlay {
    /// Records this frame's time and draws whichever overlays the settings ask for.
    pub fn show(&mut self, ui: &mut Ui<'_>, kit: &Kit, config: &Config, delta_seconds: f32) {
        if delta_seconds > 0.0 {
            if self.frame_ms.len() == HISTORY {
                self.frame_ms.pop_front();
            }
            self.frame_ms.push_back(delta_seconds * 1000.0);
        }
        let fps = config.debug_fps_counter();
        let graph = config.debug_framegraph();
        if !fps && !graph {
            return;
        }
        if graph {
            self.refresh_timings(ui, delta_seconds);
        }
        let _column = ui.element(
            &ElementDesc::new()
                .id("debug-overlay".into())
                .width(Sizing::Fixed(PANEL_WIDTH))
                .height(Sizing::Fit)
                .direction(Direction::TopToBottom)
                .child_gap(kit::SPACE_SM)
                .floating(Some(Floating {
                    element_attach_point: AttachPoint::LeftTop,
                    parent_attach_point: AttachPoint::LeftTop,
                    offset: Vec2::new(MARGIN, TITLE_BAR + MARGIN),
                    z_index: 1400,
                    // Look, don't touch: whatever is underneath stays clickable.
                    capture_pointer: false,
                    ..Floating::new(FloatTarget::Root)
                })),
        );
        if fps {
            self.fps_card(ui, kit);
        }
        if graph {
            self.graph_card(ui, kit);
        }
    }

    fn refresh_timings(&mut self, ui: &mut Ui<'_>, delta_seconds: f32) {
        self.since_timings += delta_seconds;
        if self.since_timings < TIMINGS_REFRESH_SECONDS && self.timings.has_data {
            return;
        }
        self.since_timings = 0.0;
        let mut engine = ui.engine();
        let diagnostics = engine.diagnostics();
        if self.window_id.is_none() {
            self.window_id = diagnostics.primary_window().map(|window| window.window_id);
        }
        if let Some(window_id) = self.window_id {
            self.timings = diagnostics.frame_timings(window_id);
        }
    }

    /// Frames per second over the history, the latest frame time, and the slowest frame in the history.
    fn stats(&self) -> Option<(f32, f32, f32)> {
        let latest = *self.frame_ms.back()?;
        let total: f32 = self.frame_ms.iter().sum();
        let worst = self.frame_ms.iter().copied().fold(0.0, f32::max);
        Some((self.frame_ms.len() as f32 * 1000.0 / total.max(0.001), latest, worst))
    }

    fn card(ui: &mut Ui<'_>, kit: &Kit, id: &str) -> ElementScope {
        ui.element(
            &ElementDesc::new()
                .id(id.into())
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::TopToBottom)
                .child_gap(kit::SPACE_XS)
                .padding(Padding::all(kit::SPACE_SM))
                .background(kit.palette.bg_dark.with_w(0.88))
                .corner_radius(CornerRadius::all(8.0))
                .border(Border::all(kit.palette.border_muted, 1)),
        )
    }

    fn fps_card(&self, ui: &mut Ui<'_>, kit: &Kit) {
        let _card = Self::card(ui, kit, "debug-overlay::fps");
        match self.stats() {
            Some((fps, latest, worst)) => {
                kit.text(ui, TextRole::Body, &tr!("{fps:.0} FPS", fps = fps));
                kit.text(
                    ui,
                    TextRole::Muted,
                    &tr!("{latest:.2} ms frame  ·  {worst:.1} ms worst of {count}", latest = latest, worst = worst, count = self.frame_ms.len()),
                );
            }
            None => kit.text(ui, TextRole::Muted, &tr!("Measuring...")),
        }
    }

    fn graph_card(&self, ui: &mut Ui<'_>, kit: &Kit) {
        let _card = Self::card(ui, kit, "debug-overlay::graph");
        kit.text(ui, TextRole::Body, &tr!("Frame graph"));
        self.history_strip(ui, kit);
        if !self.timings.has_data {
            kit.text(ui, TextRole::Muted, &tr!("Waiting for the engine's first timings..."));
            return;
        }
        pass_list(ui, kit, &tr!("GPU passes"), &self.timings.gpu_passes, &tr!("No GPU timings: this device has no timestamp queries."));
        pass_list(ui, kit, &tr!("CPU passes"), &self.timings.cpu_passes, &tr!("No CPU pass timings."));
        pass_list(ui, kit, &tr!("CPU stages"), &self.timings.cpu_stages, &tr!("No CPU stage timings."));
    }

    /// One bar per recent frame, tall for slow ones, over a line at the 60 Hz budget.
    fn history_strip(&self, ui: &mut Ui<'_>, kit: &Kit) {
        // The strip's top is twice the budget unless a frame was slower than that.
        let ceiling = self.frame_ms.iter().copied().fold(BUDGET_MS * 2.0, f32::max);
        let bar_width = (PANEL_WIDTH - 2.0 * f32::from(kit::SPACE_SM) - 2.0) / HISTORY as f32;
        let _strip = ui.element(
            &ElementDesc::new()
                .width(Sizing::Grow)
                .height(Sizing::Fixed(HISTORY_HEIGHT))
                .direction(Direction::LeftToRight)
                .align(ChildAlignment::new(AlignX::Right, AlignY::Bottom))
                .background(kit.palette.bg_light.with_w(0.35))
                .corner_radius(CornerRadius::all(4.0)),
        );
        for ms in &self.frame_ms {
            let color = if *ms <= BUDGET_MS {
                kit.palette.primary
            } else if *ms <= BUDGET_MS * 2.0 {
                kit.palette.warning
            } else {
                kit.palette.danger
            };
            drop(
                ui.element(
                    &ElementDesc::new()
                        .width(Sizing::Fixed(bar_width))
                        .height(Sizing::Fixed((ms / ceiling * HISTORY_HEIGHT).clamp(1.0, HISTORY_HEIGHT)))
                        .background(color),
                ),
            );
        }
        // The budget line, floated over the strip at the height one 60 Hz frame reaches.
        drop(
            ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fixed(1.0))
                    .background(Vec4::new(1.0, 1.0, 1.0, 0.45))
                    .floating(Some(Floating {
                        element_attach_point: AttachPoint::LeftBottom,
                        parent_attach_point: AttachPoint::LeftBottom,
                        offset: Vec2::new(0.0, -(BUDGET_MS / ceiling * HISTORY_HEIGHT)),
                        z_index: 1401,
                        capture_pointer: false,
                        ..Floating::new(FloatTarget::Parent)
                    })),
            ),
        );
    }
}

/// A titled list of passes with a bar each, scaled so the 60 Hz budget is a full bar (longer passes overflow to full).
fn pass_list(ui: &mut Ui<'_>, kit: &Kit, title: &str, passes: &[(String, f64)], empty: &str) {
    let total: f64 = passes.iter().map(|(_, ms)| ms).sum();
    kit.text(ui, TextRole::Muted, &if passes.is_empty() { title.to_owned() } else { tr!("{title}  ({total:.2} ms)", title = title, total = total).into_owned() });
    if passes.is_empty() {
        kit.text(ui, TextRole::Muted, empty);
        return;
    }
    for (name, ms) in passes.iter().take(MAX_ROWS) {
        row(ui, kit, name, *ms);
    }
    if passes.len() > MAX_ROWS {
        let rest: f64 = passes[MAX_ROWS..].iter().map(|(_, ms)| ms).sum();
        row(ui, kit, &tr!("{count} more", count = passes.len() - MAX_ROWS), rest);
    }
}

fn row(ui: &mut Ui<'_>, kit: &Kit, name: &str, ms: f64) {
    let _row = ui.element(
        &ElementDesc::new()
            .width(Sizing::Grow)
            .height(Sizing::Fit)
            .direction(Direction::LeftToRight)
            .child_gap(kit::SPACE_SM)
            .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
    );
    {
        let _name = ui.element(
            &ElementDesc::new()
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .clip(vertex_shell::sturdy::ui::Clip::horizontal()),
        );
        kit.text_clipped(ui, TextRole::Body, name, PANEL_WIDTH - BAR_WIDTH - 90.0);
    }
    let fraction = (ms as f32 / BUDGET_MS).clamp(0.0, 1.0);
    let color = if ms as f32 > BUDGET_MS * 0.5 {
        kit.palette.danger
    } else if ms as f32 > BUDGET_MS * 0.15 {
        kit.palette.warning
    } else {
        kit.palette.primary
    };
    {
        let _track = ui.element(
            &ElementDesc::new()
                .width(Sizing::Fixed(BAR_WIDTH))
                .height(Sizing::Fixed(BAR_HEIGHT))
                .background(kit.palette.bg_light.with_w(0.5))
                .corner_radius(CornerRadius::all(3.0)),
        );
        drop(
            ui.element(
                &ElementDesc::new()
                    .width(Sizing::Fixed((BAR_WIDTH * fraction).max(1.0)))
                    .height(Sizing::Fixed(BAR_HEIGHT))
                    .background(color)
                    .corner_radius(CornerRadius::all(3.0)),
            ),
        );
    }
    kit.text(ui, TextRole::Muted, &format!("{ms:>6.2}"));
}
