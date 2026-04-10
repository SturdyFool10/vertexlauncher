use super::super::*;

pub(super) enum PreviewRenderPath {
    MotionBlur { skin_sample: Arc<RgbaImage> },
    SingleScene,
}
