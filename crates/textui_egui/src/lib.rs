use egui::{Context, Id, Painter, Rect, Response, Ui, Vec2};
use egui_wgpu::RenderState;
use std::hash::Hash;
use textui::{
    TextFrameInfo, TextFrameOutput, TextPath, TextPathError, TextPathLayout, TextPathOptions,
    TextRenderScene, TextUi,
};

pub use textui::{
    ButtonOptions, CodeBlockOptions, InputOptions, LabelOptions, MarkdownOptions, RichTextSpan,
    RichTextStyle, TextColor, TooltipOptions, normalize_inline_whitespace,
    truncate_single_line_text_with_ellipsis,
    truncate_single_line_text_with_ellipsis_preserving_whitespace,
};

#[derive(Clone)]
pub struct TextTextureHandle {
    scene: TextRenderScene,
    pub size_points: Vec2,
}

impl TextTextureHandle {
    pub fn scene(&self) -> &TextRenderScene {
        &self.scene
    }

    pub fn into_scene(self) -> TextRenderScene {
        self.scene
    }

    pub fn paint(&self, text_ui: &mut TextUi, ui: &Ui, rect: Rect) {
        let painter = ui.painter().with_clip_rect(ui.clip_rect());
        text_ui.paint_scene_in_rect(&painter, rect, &self.scene);
    }

    pub fn paint_tinted(&self, text_ui: &mut TextUi, ui: &Ui, rect: Rect, tint: egui::Color32) {
        let painter = ui.painter().with_clip_rect(ui.clip_rect());
        text_ui.paint_scene_in_rect_tinted(&painter, rect, &self.scene, tint);
    }

    pub fn paint_on(
        &self,
        text_ui: &mut TextUi,
        painter: &egui::Painter,
        rect: Rect,
        tint: egui::Color32,
    ) {
        text_ui.paint_scene_in_rect_tinted(painter, rect, &self.scene, tint);
    }
}

pub trait TextUiEguiExt {
    fn label_async<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        options: &LabelOptions,
    ) -> Response;
    fn code_block_async<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        code: &str,
        options: &CodeBlockOptions,
    ) -> Response;
    fn label<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        options: &LabelOptions,
    ) -> Response;
    fn clickable_label<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        options: &LabelOptions,
    ) -> Response;
    fn measure_text_size(&mut self, ui: &Ui, text: &str, options: &LabelOptions) -> Vec2;
    fn prepare_label_texture<H: Hash>(
        &mut self,
        ctx: &Context,
        id_source: H,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
    ) -> TextTextureHandle;
    fn prepare_rich_text_texture<H: Hash>(
        &mut self,
        ctx: &Context,
        id_source: H,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
    ) -> TextTextureHandle;
    fn paint_label_on_path<H: Hash>(
        &mut self,
        painter: &Painter,
        id_source: H,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError>;
    fn paint_rich_text_on_path<H: Hash>(
        &mut self,
        painter: &Painter,
        id_source: H,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError>;
    fn paint_scene_in_rect(&mut self, painter: &Painter, rect: Rect, scene: &TextRenderScene);
    fn paint_scene_in_rect_tinted(
        &mut self,
        painter: &Painter,
        rect: Rect,
        scene: &TextRenderScene,
        tint: egui::Color32,
    );
    fn button<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        options: &ButtonOptions,
    ) -> Response;
    fn selectable_button<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        selected: bool,
        options: &ButtonOptions,
    ) -> Response;
    fn tooltip_for_response<H: Hash>(
        &mut self,
        ui: &Ui,
        id_source: H,
        response: &Response,
        text: &str,
        options: &TooltipOptions,
    );
    fn code_block<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        code: &str,
        options: &CodeBlockOptions,
    ) -> Response;
    fn markdown<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        markdown: &str,
        options: &MarkdownOptions,
    );
    fn singleline_input<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &mut String,
        options: &InputOptions,
    ) -> Response;
    fn multiline_input<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &mut String,
        options: &InputOptions,
    ) -> Response;
    fn multiline_rich_viewer<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        spans: &[RichTextSpan],
        options: &InputOptions,
        stick_to_bottom: bool,
        wrap: bool,
    ) -> Response;
}

impl TextUiEguiExt for TextUi {
    fn label_async<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        options: &LabelOptions,
    ) -> Response {
        TextUi::label_async(self, ui, id_source, text, options)
    }

    fn code_block_async<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        code: &str,
        options: &CodeBlockOptions,
    ) -> Response {
        TextUi::code_block_async(self, ui, id_source, code, options)
    }

    fn label<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        options: &LabelOptions,
    ) -> Response {
        TextUi::label(self, ui, id_source, text, options)
    }

    fn clickable_label<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        options: &LabelOptions,
    ) -> Response {
        TextUi::clickable_label(self, ui, id_source, text, options)
    }

    fn measure_text_size(&mut self, ui: &Ui, text: &str, options: &LabelOptions) -> Vec2 {
        TextUi::measure_text_size(self, ui, text, options)
    }

    fn prepare_label_texture<H: Hash>(
        &mut self,
        ctx: &Context,
        id_source: H,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
    ) -> TextTextureHandle {
        let scene =
            TextUi::prepare_label_scene(self, ctx, id_source, text, options, width_points_opt);
        TextTextureHandle {
            size_points: scene.size_points.into(),
            scene,
        }
    }

    fn prepare_rich_text_texture<H: Hash>(
        &mut self,
        ctx: &Context,
        id_source: H,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
    ) -> TextTextureHandle {
        let scene =
            TextUi::prepare_rich_text_scene(self, ctx, id_source, spans, options, width_points_opt);
        TextTextureHandle {
            size_points: scene.size_points.into(),
            scene,
        }
    }

    fn paint_label_on_path<H: Hash>(
        &mut self,
        painter: &Painter,
        id_source: H,
        text: &str,
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError> {
        TextUi::paint_label_on_path(
            self,
            painter,
            id_source,
            text,
            options,
            width_points_opt,
            path,
            path_options,
        )
    }

    fn paint_rich_text_on_path<H: Hash>(
        &mut self,
        painter: &Painter,
        id_source: H,
        spans: &[RichTextSpan],
        options: &LabelOptions,
        width_points_opt: Option<f32>,
        path: &TextPath,
        path_options: &TextPathOptions,
    ) -> Result<TextPathLayout, TextPathError> {
        TextUi::paint_rich_text_on_path(
            self,
            painter,
            id_source,
            spans,
            options,
            width_points_opt,
            path,
            path_options,
        )
    }

    fn paint_scene_in_rect(&mut self, painter: &Painter, rect: Rect, scene: &TextRenderScene) {
        TextUi::paint_scene_in_rect(self, painter, rect, scene)
    }

    fn paint_scene_in_rect_tinted(
        &mut self,
        painter: &Painter,
        rect: Rect,
        scene: &TextRenderScene,
        tint: egui::Color32,
    ) {
        TextUi::paint_scene_in_rect_tinted(self, painter, rect, scene, tint)
    }

    fn button<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        options: &ButtonOptions,
    ) -> Response {
        TextUi::button(self, ui, id_source, text, options)
    }

    fn selectable_button<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &str,
        selected: bool,
        options: &ButtonOptions,
    ) -> Response {
        TextUi::selectable_button(self, ui, id_source, text, selected, options)
    }

    fn tooltip_for_response<H: Hash>(
        &mut self,
        ui: &Ui,
        id_source: H,
        response: &Response,
        text: &str,
        options: &TooltipOptions,
    ) {
        TextUi::tooltip_for_response(self, ui, id_source, response, text, options)
    }

    fn code_block<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        code: &str,
        options: &CodeBlockOptions,
    ) -> Response {
        TextUi::code_block(self, ui, id_source, code, options)
    }

    fn markdown<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        markdown: &str,
        options: &MarkdownOptions,
    ) {
        TextUi::markdown(self, ui, id_source, markdown, options)
    }

    fn singleline_input<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &mut String,
        options: &InputOptions,
    ) -> Response {
        TextUi::singleline_input(self, ui, id_source, text, options)
    }

    fn multiline_input<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        text: &mut String,
        options: &InputOptions,
    ) -> Response {
        TextUi::multiline_input(self, ui, id_source, text, options)
    }

    fn multiline_rich_viewer<H: Hash>(
        &mut self,
        ui: &mut Ui,
        id_source: H,
        spans: &[RichTextSpan],
        options: &InputOptions,
        stick_to_bottom: bool,
        wrap: bool,
    ) -> Response {
        TextUi::multiline_rich_viewer(self, ui, id_source, spans, options, stick_to_bottom, wrap)
    }
}

pub mod prelude {
    pub use super::{
        ButtonOptions, CodeBlockOptions, InputOptions, LabelOptions, MarkdownOptions, RichTextSpan,
        RichTextStyle, TextColor, TextTextureHandle, TextUiEguiExt, TooltipOptions,
        normalize_inline_whitespace, truncate_single_line_text_with_ellipsis,
        truncate_single_line_text_with_ellipsis_preserving_whitespace,
    };
}

const GAMEPAD_SCROLL_DELTA_ID: &str = "textui_gamepad_scroll_delta";
const GAMEPAD_SCROLL_TARGETS_ID: &str = "textui_gamepad_scroll_targets";
const GAMEPAD_SCROLL_FRAME_ID: &str = "textui_gamepad_scroll_frame";

#[derive(Clone, Copy)]
struct GamepadScrollTarget {
    id: Id,
    rect: Rect,
    content_size: Vec2,
}

impl GamepadScrollTarget {
    fn max_offset(&self) -> Vec2 {
        Vec2::new(
            (self.content_size.x - self.rect.width()).max(0.0),
            (self.content_size.y - self.rect.height()).max(0.0),
        )
    }

    fn can_scroll_h(&self) -> bool {
        self.max_offset().x > 0.5
    }

    fn can_scroll_v(&self) -> bool {
        self.max_offset().y > 0.5
    }
}

pub fn begin_frame(
    text_ui: &mut TextUi,
    ctx: &Context,
    render_state: Option<&RenderState>,
) -> TextFrameOutput {
    text_ui.set_egui_wgpu_render_state(render_state);
    text_ui.begin_frame_info(TextFrameInfo::new(
        ctx.cumulative_frame_nr(),
        ctx.input(|i| i.max_texture_side).max(1),
    ));
    text_ui.set_frame_events(ctx.input(|i| i.events.clone()));
    text_ui.flush_egui_frame(ctx)
}

pub fn set_gamepad_scroll_delta(ctx: &Context, delta: Vec2) {
    ctx.data_mut(|data| data.insert_temp(Id::new(GAMEPAD_SCROLL_DELTA_ID), delta));
}

pub fn gamepad_scroll_delta(ctx: &Context) -> Vec2 {
    ctx.data_mut(|data| {
        data.get_temp::<Vec2>(Id::new(GAMEPAD_SCROLL_DELTA_ID))
            .unwrap_or(Vec2::ZERO)
    })
}

fn ensure_gamepad_scroll_targets_fresh(ctx: &Context) {
    let current = ctx.cumulative_frame_nr();
    let frame_key = Id::new(GAMEPAD_SCROLL_FRAME_ID);
    let last = ctx.data(|d| d.get_temp::<u64>(frame_key).unwrap_or(u64::MAX));
    if current != last {
        ctx.data_mut(|d| {
            d.remove::<Vec<GamepadScrollTarget>>(Id::new(GAMEPAD_SCROLL_TARGETS_ID));
            d.insert_temp(frame_key, current);
        });
    }
}

fn register_gamepad_scroll_target(ctx: &Context, id: Id, rect: Rect, content_size: Vec2) {
    ctx.data_mut(|data| {
        let key = Id::new(GAMEPAD_SCROLL_TARGETS_ID);
        let mut targets = data
            .get_temp::<Vec<GamepadScrollTarget>>(key)
            .unwrap_or_default();
        targets.retain(|target| target.id != id);
        targets.push(GamepadScrollTarget {
            id,
            rect,
            content_size,
        });
        data.insert_temp(key, targets);
    });
}

pub fn make_gamepad_scrollable<R>(ctx: &Context, output: &egui::scroll_area::ScrollAreaOutput<R>) {
    ensure_gamepad_scroll_targets_fresh(ctx);
    register_gamepad_scroll_target(ctx, output.id, output.inner_rect, output.content_size);
}

pub fn gamepad_scroll<R>(
    scroll_area: egui::ScrollArea,
    ui: &mut Ui,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    let output = scroll_area.show(ui, add_contents);
    make_gamepad_scrollable(ui.ctx(), &output);
    output
}

pub fn apply_gamepad_scroll_to_focused_target(ctx: &Context, delta: Vec2) -> bool {
    if delta == Vec2::ZERO {
        return false;
    }

    let Some(focused_id) = ctx.memory(|memory| memory.focused()) else {
        return false;
    };

    let focused_screen_rect = ctx.read_response(focused_id).map(|r| r.rect);
    let targets = ctx.data_mut(|data| {
        data.get_temp::<Vec<GamepadScrollTarget>>(Id::new(GAMEPAD_SCROLL_TARGETS_ID))
            .unwrap_or_default()
    });

    let sort_by_area = |a: &GamepadScrollTarget, b: &GamepadScrollTarget| {
        let a_area = a.rect.width() * a.rect.height();
        let b_area = b.rect.width() * b.rect.height();
        a_area
            .partial_cmp(&b_area)
            .unwrap_or(std::cmp::Ordering::Equal)
    };

    let mut candidates: Vec<GamepadScrollTarget> = if let Some(fr) = focused_screen_rect {
        let fp = fr.center();
        let positional: Vec<_> = targets
            .iter()
            .copied()
            .filter(|t| t.rect.contains(fp) || t.rect.intersects(fr))
            .collect();
        if positional.is_empty() {
            targets
        } else {
            positional
        }
    } else {
        targets
    };
    candidates.sort_by(sort_by_area);

    if let Some(pos) = candidates.iter().position(|t| t.id == focused_id) {
        let direct = candidates.remove(pos);
        candidates.insert(0, direct);
    }

    let mut need_x = delta.x != 0.0;
    let mut need_y = delta.y != 0.0;
    let mut applied = false;

    for target in &candidates {
        if !need_x && !need_y {
            break;
        }

        let max_offset = target.max_offset();
        let can_x = need_x && target.can_scroll_h();
        let can_y = need_y && target.can_scroll_v();

        if !can_x && !can_y {
            continue;
        }

        let mut state = egui::scroll_area::State::load(ctx, target.id).unwrap_or_default();
        let mut changed = false;

        if can_x {
            let new_x = (state.offset.x - delta.x).clamp(0.0, max_offset.x);
            if new_x != state.offset.x {
                state.offset.x = new_x;
                need_x = false;
                applied = true;
                changed = true;
            }
        }
        if can_y {
            let new_y = (state.offset.y - delta.y).clamp(0.0, max_offset.y);
            if new_y != state.offset.y {
                state.offset.y = new_y;
                need_y = false;
                applied = true;
                changed = true;
            }
        }

        if changed {
            state.store(ctx, target.id);
        }
    }

    applied
}

pub fn apply_gamepad_scroll_to_registered_id(ctx: &Context, scroll_id: Id, delta: Vec2) -> bool {
    if delta == Vec2::ZERO {
        return false;
    }
    let targets = ctx.data(|d| {
        d.get_temp::<Vec<GamepadScrollTarget>>(Id::new(GAMEPAD_SCROLL_TARGETS_ID))
            .unwrap_or_default()
    });
    let Some(target) = targets.iter().find(|t| t.id == scroll_id).copied() else {
        return false;
    };
    let max_offset = target.max_offset();
    let mut state = egui::scroll_area::State::load(ctx, scroll_id).unwrap_or_default();
    let mut changed = false;
    if delta.x != 0.0 && target.can_scroll_h() {
        let new_x = (state.offset.x - delta.x).clamp(0.0, max_offset.x);
        if new_x != state.offset.x {
            state.offset.x = new_x;
            changed = true;
        }
    }
    if delta.y != 0.0 && target.can_scroll_v() {
        let new_y = (state.offset.y - delta.y).clamp(0.0, max_offset.y);
        if new_y != state.offset.y {
            state.offset.y = new_y;
            changed = true;
        }
    }
    if changed {
        state.store(ctx, scroll_id);
    }
    changed
}

pub fn apply_gamepad_scroll_if_focused(ui: &Ui, response: &Response) {
    if response.has_focus() {
        let delta = gamepad_scroll_delta(ui.ctx());
        if delta != Vec2::ZERO {
            ui.scroll_with_delta(delta);
        }
    }
}
