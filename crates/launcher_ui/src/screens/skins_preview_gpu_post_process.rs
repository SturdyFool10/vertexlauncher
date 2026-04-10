use super::*;

#[path = "skins_preview_gpu_post_process/callback.rs"]
mod callback;
#[path = "skins_preview_gpu_post_process/callback_execution.rs"]
mod callback_execution;
#[path = "skins_preview_gpu_post_process/post_process_passes.rs"]
mod post_process_passes;
#[path = "skins_preview_gpu_post_process/present_source.rs"]
mod present_source;
#[path = "skins_preview_gpu_post_process/render_targets.rs"]
mod render_targets;
#[path = "skins_preview_gpu_post_process/resource_initialization.rs"]
mod resource_initialization;
#[path = "skins_preview_gpu_post_process/resource_runtime.rs"]
mod resource_runtime;
#[path = "skins_preview_gpu_post_process/scene_rendering.rs"]
mod scene_rendering;
#[path = "skins_preview_gpu_post_process/shader_modules.rs"]
mod shader_modules;
#[path = "skins_preview_gpu_post_process/skin_preview_post_process_wgpu_resources.rs"]
mod skin_preview_post_process_wgpu_resources;
#[path = "skins_preview_gpu_post_process/source_textures.rs"]
mod source_textures;
#[path = "skins_preview_gpu_post_process/uniform_resources.rs"]
mod uniform_resources;

pub(super) use self::callback::SkinPreviewPostProcessWgpuCallback;
use self::present_source::PresentSource;
use self::render_targets::SkinPreviewPostProcessRenderTargets;
use self::shader_modules::SkinPreviewPostProcessShaderModules;
use self::skin_preview_post_process_wgpu_resources::SkinPreviewPostProcessWgpuResources;
use self::source_textures::SkinPreviewPostProcessSourceTextures;
use self::uniform_resources::SkinPreviewPostProcessUniformResources;
