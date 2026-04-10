use super::*;

pub(in super::super) struct SkinPreviewPostProcessWgpuCallback {
    pub(super) scene_batches: Vec<GpuPreviewSceneBatch>,
    pub(super) skin_sample: Arc<RgbaImage>,
    pub(super) cape_sample: Option<Arc<RgbaImage>>,
    pub(super) skin_hash: u64,
    pub(super) cape_hash: Option<u64>,
    pub(super) target_format: wgpu::TextureFormat,
    pub(super) scene_msaa_samples: u32,
    pub(super) present_msaa_samples: u32,
    pub(super) aa_mode: SkinPreviewAaMode,
    pub(super) texel_aa_mode: SkinPreviewTexelAaMode,
}

impl SkinPreviewPostProcessWgpuCallback {
    pub(in super::super) fn from_scene(
        triangles: &[RenderTriangle],
        skin_sample: Arc<RgbaImage>,
        cape_sample: Option<Arc<RgbaImage>>,
        target_format: wgpu::TextureFormat,
        scene_msaa_samples: u32,
        present_msaa_samples: u32,
        aa_mode: SkinPreviewAaMode,
        texel_aa_mode: SkinPreviewTexelAaMode,
    ) -> Self {
        Self {
            scene_batches: vec![build_preview_scene_batch(triangles, 1.0)],
            skin_hash: hash_preview_image(&skin_sample),
            cape_hash: cape_sample
                .as_ref()
                .map(|image| hash_preview_image(image.as_ref())),
            skin_sample,
            cape_sample,
            target_format,
            scene_msaa_samples,
            present_msaa_samples,
            aa_mode,
            texel_aa_mode,
        }
    }

    pub(in super::super) fn from_weighted_scenes(
        scenes: &[WeightedPreviewScene],
        skin_sample: Arc<RgbaImage>,
        cape_sample: Option<Arc<RgbaImage>>,
        target_format: wgpu::TextureFormat,
        scene_msaa_samples: u32,
        present_msaa_samples: u32,
        aa_mode: SkinPreviewAaMode,
        texel_aa_mode: SkinPreviewTexelAaMode,
    ) -> Self {
        let scene_batches = scenes
            .iter()
            .map(|scene| build_preview_scene_batch(&scene.triangles, scene.weight))
            .collect();
        Self {
            scene_batches,
            skin_hash: hash_preview_image(&skin_sample),
            cape_hash: cape_sample
                .as_ref()
                .map(|image| hash_preview_image(image.as_ref())),
            skin_sample,
            cape_sample,
            target_format,
            scene_msaa_samples,
            present_msaa_samples,
            aa_mode,
            texel_aa_mode,
        }
    }
}
