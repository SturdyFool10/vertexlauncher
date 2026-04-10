use super::*;

pub(super) struct CharacterSceneBuildContext {
    pub(super) rect: Rect,
    pub(super) variant: MinecraftSkinVariant,
    pub(super) cape_uv: FaceUvs,
    pub(super) preview_pose: PreviewPose,
    pub(super) preview_3d_layers_enabled: bool,
    pub(super) show_elytra: bool,
    pub(super) expressions_enabled: bool,
    pub(super) expression_layout: Option<DetectedExpressionsLayout>,
    pub(super) skin_sample: Option<Arc<RgbaImage>>,
    pub(super) cape_sample: Option<Arc<RgbaImage>>,
    pub(super) default_elytra_sample: Option<Arc<RgbaImage>>,
    pub(super) camera: Camera,
    pub(super) projection: Projection,
    pub(super) model_offset: Vec3,
    pub(super) light_dir: Vec3,
    pub(super) motion: CharacterSceneMotion,
    pub(super) base_tris: Vec<RenderTriangle>,
    pub(super) overlay_tris: Vec<RenderTriangle>,
}
