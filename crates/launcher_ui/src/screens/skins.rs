use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use auth::{CachedAccount, MinecraftProfileState, MinecraftSkinVariant};
use config::{SkinPreviewAaMode, SkinPreviewTexelAaMode};
use image::{RgbaImage, imageops::FilterType};
use launcher_runtime as tokio_runtime;

use super::LaunchAuthContext;

#[path = "skins/skins_api.rs"]
pub mod api;
#[path = "skins/skins_gpu_scene.rs"]
pub mod gpu_scene;
#[path = "skins/image_hash.rs"]
mod image_hash;
#[path = "skins_preview_gpu_geometry.rs"]
mod skins_preview_gpu_geometry;
#[path = "skins/auth_error_format.rs"]
mod auth_error_format;
#[path = "skins/cape_choice.rs"]
mod cape_choice;
#[path = "skins/cape_uv_layout.rs"]
mod cape_uv_layout;
#[path = "skins/default_elytra_texture.rs"]
mod default_elytra_texture;
#[path = "skins/loaded_profile.rs"]
mod loaded_profile;
#[path = "skins/preview_motion_mode.rs"]
mod preview_motion_mode;
#[path = "skins/preview_pose.rs"]
mod preview_pose;
#[path = "skins/profile_cache.rs"]
mod profile_cache;
#[path = "skins/profile_variant.rs"]
mod profile_variant;
#[path = "skins/skin_image.rs"]
mod skin_image;
#[path = "skins_expressions.rs"]
mod skins_expressions;
#[path = "skins_preview.rs"]
mod skins_preview;
#[path = "skins_state.rs"]
mod skins_state;
#[path = "skins/worker_event.rs"]
mod worker_event;
use self::auth_error_format::format_auth_error;
use self::cape_choice::CapeChoice;
use self::cape_uv_layout::{cape_uv_layout, default_cape_uv_layout};
use self::default_elytra_texture::default_elytra_texture_image;
use self::loaded_profile::LoadedProfile;
use self::profile_cache::{fetch_and_cache_profile, update_cached_profile};
use self::profile_variant::parse_variant;
use self::skin_image::{decode_generic_rgba, decode_image_dimensions, decode_skin_rgba};
use self::skins_expressions::*;
use self::skins_preview::*;
pub(crate) use crate::geom::*;
use crate::notification;
use self::image_hash::hash_preview_image;
use self::skins_preview_gpu_geometry::{ElytraMotion, add_cape_triangles, add_elytra_triangles};
use self::skins_state::*;
use self::gpu_scene::{PartTag, record_quad as capture_quad, set_part as capture_part, skipping_current_part as capture_skipping};
use self::worker_event::WorkerEvent;
use self::{preview_motion_mode::PreviewMotionMode, preview_pose::PreviewPose};

const PREVIEW_ORBIT_SECONDS: f64 = 45.0;
/// The animation clock is kept in f32, which stops resolving frames after a few hours; it restarts at this many seconds.
const PREVIEW_TIME_WRAP_SECONDS: f64 = 3600.0;

/// Minecraft's walk-animation speed for a player walking on flat ground (≈0.216 blocks per
/// tick × 4, capped at 1). Drives the preview's walk cycle.
const PREVIEW_WALK_LIMB_SPEED: f32 = 0.86;
const CAMERA_DRAG_SENSITIVITY_RAD_PER_POINT: f32 = 0.0046;
const GAMEPAD_ORBIT_MAX_RAD_PER_SEC: f32 = 3.4;
const GAMEPAD_ORBIT_DEADZONE: f32 = 0.18;
const CAMERA_INERTIA_FRICTION_PER_SEC: f32 = 2.0;
const CAMERA_INERTIA_STOP_THRESHOLD_RAD_PER_SEC: f32 = 0.015;
const UV_EDGE_INSET_BASE_TEXELS: f32 = 0.08;
const UV_EDGE_INSET_OVERLAY_TEXELS: f32 = 0.5;
/// Just enough inset to keep nearest sampling inside a region while staying aligned to the
/// texel grid (used where panels must meet face texels exactly).
const FACE_TEXEL_EXACT_INSET_TEXELS: f32 = 0.002;






/// Minecraft's `Mth.clamp`, used by the preview animations: returns `min` when below
/// it, otherwise caps at `max` (so `min > max` yields `max` for in-range values).
fn mc_clamp(value: f32, min: f32, max: f32) -> f32 {
    if value < min { min } else { value.min(max) }
}

