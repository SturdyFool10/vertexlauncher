//! The skin preview as data for a GPU renderer: which parts the character is made of, the quads of each in the part's
//! own space (static for a skin), and where each part is this frame (a matrix).
//!
//! The CPU rasteriser's scene builder already works this way: every part is a list of cuboids placed by one
//! [`ModelTransform`]. Instead of rewriting it, [`capture_scene`] runs that same builder with a recorder switched on
//! and keeps what the quad emitters were handed before they projected anything. So the geometry, the texture
//! rectangles, the voxel layers, the cape and the elytra stay exactly what the CPU preview draws.

use std::cell::RefCell;
use std::collections::HashMap;

use super::*;

/// The separately moving pieces of the preview character.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PartTag {
    Body,
    Head,
    /// Eyes and brows drawn on the face. Their quads are placed relative to the head and change as the face animates.
    Face,
    RightArm,
    LeftArm,
    RightLeg,
    LeftLeg,
    Cape,
    ElytraLeft,
    ElytraRight,
}

/// Which image a quad samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneTexture {
    Skin,
    Cape,
}

/// One textured quad, wound top-left, top-right, bottom-right, bottom-left as seen from the side `normal` points to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneQuad {
    /// Column-major matrix from the quad's space into preview world space.
    pub transform: [f32; 16],
    pub vertices: [[f32; 3]; 4],
    pub normal: [f32; 3],
    /// Normalised texture coordinates of the four corners, in the same order as `vertices`.
    pub uv: [[f32; 2]; 4],
    pub texture: SceneTexture,
}

/// One part of the character for one frame.
#[derive(Clone, Debug)]
pub struct ScenePart {
    pub tag: PartTag,
    /// The part was in the `skip` list of the capture: `quads` holds a single quad that only carries the matrix.
    pub skipped: bool,
    /// Where the part is now (the transform of its first quad), column-major.
    pub transform: [f32; 16],
    pub quads: Vec<SceneQuad>,
}

/// The preview camera: the character stands still and the camera orbits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneCamera {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub up: [f32; 3],
    pub vertical_fov_radians: f32,
    pub near: f32,
    /// Direction towards the light, unit length; the lit side of a face is brighter by `0.42 * (n . light)`.
    pub light_dir: [f32; 3],
}

/// Brightness of a face with world normal `normal` under the preview's one light: `0.58 + 0.42 * max(n . light, 0)`.
pub const SHADE_AMBIENT: f32 = 0.58;
pub const SHADE_DIFFUSE: f32 = 0.42;

/// The camera for the orbit angle `yaw` (the same one the CPU preview uses).
pub fn scene_camera(yaw: f32) -> SceneCamera {
    let (camera, projection, _, light) = build_character_scene_camera(yaw);
    SceneCamera {
        eye: [camera.position.x, camera.position.y, camera.position.z],
        target: [0.0, 19.5, 0.0],
        up: [0.0, 1.0, 0.0],
        vertical_fov_radians: projection.fov_y_radians,
        near: projection.near,
        light_dir: [light.x, light.y, light.z],
    }
}

struct Recorder {
    tag: PartTag,
    parts: Vec<(PartTag, Vec<SceneQuad>)>,
    /// Parts whose quads the caller already has: only their first quad is kept (for the part's matrix).
    skip: Vec<PartTag>,
}

/// Whether the part being emitted is one whose geometry the caller does not want again. The voxel layers are the
/// expensive geometry to generate; they ask before doing the work.
pub(in crate::screens) fn skipping_current_part() -> bool {
    RECORDER.with(|recorder| recorder.borrow().as_ref().is_some_and(|recorder| recorder.skip.contains(&recorder.tag)))
}

thread_local! {
    static RECORDER: RefCell<Option<Recorder>> = const { RefCell::new(None) };
}

/// Marks the quads emitted from here on as belonging to `tag`. Does nothing unless a capture is running.
pub(in crate::screens) fn set_part(tag: PartTag) {
    RECORDER.with(|recorder| {
        if let Some(recorder) = recorder.borrow_mut().as_mut() {
            recorder.tag = tag;
        }
    });
}

/// Called by the quad emitter with the quad as it received it. Does nothing unless a capture is running.
pub(in crate::screens) fn record_quad(
    texture: TriangleTexture,
    transform: ModelTransform,
    quad: [Vec3; 4],
    normal: Vec3,
    uv_rect: Rect,
) {
    RECORDER.with(|recorder| {
        let mut recorder = recorder.borrow_mut();
        let Some(recorder) = recorder.as_mut() else {
            return;
        };
        let corners = [
            uv_rect.left_top(),
            uv_rect.right_top(),
            uv_rect.right_bottom(),
            uv_rect.left_bottom(),
        ];
        let quad = SceneQuad {
            transform: matrix_of(transform),
            vertices: quad.map(|v| [v.x, v.y, v.z]),
            normal: [normal.x, normal.y, normal.z],
            uv: corners.map(|c| [c.x, c.y]),
            texture: match texture {
                TriangleTexture::Skin => SceneTexture::Skin,
                TriangleTexture::Cape => SceneTexture::Cape,
            },
        };
        let tag = recorder.tag;
        let skipping = recorder.skip.contains(&tag);
        match recorder.parts.last_mut() {
            // A skipped part keeps one quad, whose matrix places the part.
            Some((last, quads)) if *last == tag => {
                if !(skipping && !quads.is_empty()) {
                    quads.push(quad);
                }
            }
            _ => recorder.parts.push((tag, vec![quad])),
        }
    });
}

fn matrix_of(transform: ModelTransform) -> [f32; 16] {
    let (x, y, z, o) = (transform.x_axis, transform.y_axis, transform.z_axis, transform.origin);
    [x.x, x.y, x.z, 0.0, y.x, y.y, y.z, 0.0, z.x, z.y, z.z, 0.0, o.x, o.y, o.z, 1.0]
}

/// Everything the scene builder needs besides the pose.
pub struct SceneInput {
    pub pose: PreviewPose,
    pub variant: MinecraftSkinVariant,
    pub layers_3d: bool,
    pub show_elytra: bool,
    pub expressions: bool,
    pub expression_layout: Option<DetectedExpressionsLayout>,
    pub skin: Arc<RgbaImage>,
    pub cape: Option<Arc<RgbaImage>>,
    pub default_elytra: Option<Arc<RgbaImage>>,
    pub cape_uv: FaceUvs,
}

/// Runs the scene builder and returns the character as parts. `yaw` only matters to the CPU projection that the
/// builder performs and the capture ignores; any value gives the same parts.
pub fn capture_scene(input: &SceneInput) -> Vec<ScenePart> {
    capture_scene_with_cape(input, &[]).0
}

/// [`capture_scene`], plus the image the cape and elytra quads sample. Parts listed in `skip` come back with only their
/// matrix (and the cost of generating their quads is not paid), for a caller that already holds their geometry.
pub fn capture_scene_with_cape(input: &SceneInput, skip: &[PartTag]) -> (Vec<ScenePart>, Option<Arc<RgbaImage>>) {
    RECORDER.with(|recorder| {
        *recorder.borrow_mut() = Some(Recorder { tag: PartTag::Body, parts: Vec::new(), skip: skip.to_vec() });
    });
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(256.0, 256.0));
    let built = build_character_scene(
        rect,
        input.cape_uv,
        0.0,
        input.pose,
        input.variant,
        input.layers_3d,
        input.show_elytra,
        input.expressions,
        input.expression_layout,
        Some(Arc::clone(&input.skin)),
        input.cape.clone(),
        input.default_elytra.clone(),
    );
    let recorded = RECORDER.with(|recorder| recorder.borrow_mut().take());
    let mut merged: HashMap<PartTag, Vec<SceneQuad>> = HashMap::new();
    let mut order = Vec::new();
    for (tag, quads) in recorded.map(|recorder| recorder.parts).unwrap_or_default() {
        if !merged.contains_key(&tag) {
            order.push(tag);
        }
        merged.entry(tag).or_default().extend(quads);
    }
    let mut parts: Vec<ScenePart> = order
        .into_iter()
        .filter_map(|tag| {
            let quads = merged.remove(&tag)?;
            let transform = quads.first()?.transform;
            Some(ScenePart { tag, skipped: skip.contains(&tag), transform, quads })
        })
        .collect();
    attach_face_to_head(&mut parts);
    (parts, built.cape_render_sample)
}

/// The face's panels come out placed in the world by the head's pose and their own animation. Re-express them in the
/// head's space, so the face is one mesh carried by the head that only changes when the face does.
fn attach_face_to_head(parts: &mut [ScenePart]) {
    let Some(head) = parts.iter().find(|part| part.tag == PartTag::Head).map(|part| part.transform) else {
        return;
    };
    let Some(to_head) = affine_inverse(&head) else {
        return;
    };
    for part in parts.iter_mut().filter(|part| part.tag == PartTag::Face) {
        for quad in &mut part.quads {
            let relative = affine_mul(&to_head, &quad.transform);
            quad.vertices = quad.vertices.map(|v| affine_point(&relative, v));
            quad.normal = normalize(affine_vector(&relative, quad.normal));
            quad.transform = head;
        }
        part.transform = head;
    }
}

fn affine_point(m: &[f32; 16], v: [f32; 3]) -> [f32; 3] {
    [
        m[0] * v[0] + m[4] * v[1] + m[8] * v[2] + m[12],
        m[1] * v[0] + m[5] * v[1] + m[9] * v[2] + m[13],
        m[2] * v[0] + m[6] * v[1] + m[10] * v[2] + m[14],
    ]
}

fn affine_vector(m: &[f32; 16], v: [f32; 3]) -> [f32; 3] {
    [
        m[0] * v[0] + m[4] * v[1] + m[8] * v[2],
        m[1] * v[0] + m[5] * v[1] + m[9] * v[2],
        m[2] * v[0] + m[6] * v[1] + m[10] * v[2],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length <= 1e-6 { v } else { [v[0] / length, v[1] / length, v[2] / length] }
}

/// `a * b` for two column-major affine matrices.
fn affine_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for column in 0..3 {
        let v = affine_vector(a, [b[column * 4], b[column * 4 + 1], b[column * 4 + 2]]);
        out[column * 4..column * 4 + 3].copy_from_slice(&v);
    }
    let origin = affine_point(a, [b[12], b[13], b[14]]);
    out[12..15].copy_from_slice(&origin);
    out[15] = 1.0;
    out
}

/// The inverse of a column-major affine matrix, `None` when it is singular.
fn affine_inverse(m: &[f32; 16]) -> Option<[f32; 16]> {
    let (a, b, c) = ([m[0], m[1], m[2]], [m[4], m[5], m[6]], [m[8], m[9], m[10]]);
    let cross = |u: [f32; 3], v: [f32; 3]| [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let (bc, ca, ab) = (cross(b, c), cross(c, a), cross(a, b));
    let det = a[0] * bc[0] + a[1] * bc[1] + a[2] * bc[2];
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    // Rows of the inverse 3x3 are bc, ca, ab (scaled); in column-major storage they become the columns' components.
    let mut out = [0.0; 16];
    out[0] = bc[0] * inv;
    out[1] = ca[0] * inv;
    out[2] = ab[0] * inv;
    out[4] = bc[1] * inv;
    out[5] = ca[1] * inv;
    out[6] = ab[1] * inv;
    out[8] = bc[2] * inv;
    out[9] = ca[2] * inv;
    out[10] = ab[2] * inv;
    let origin = [m[12], m[13], m[14]];
    let moved = affine_vector(&out, origin);
    out[12] = -moved[0];
    out[13] = -moved[1];
    out[14] = -moved[2];
    out[15] = 1.0;
    Some(out)
}

/// A cheap fingerprint of a part's geometry in its own space (everything but where the part is), so a renderer can
/// keep its mesh across frames and only move it.
pub fn local_geometry_hash(part: &ScenePart) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    part.quads.len().hash(&mut hasher);
    // The vertices are in the part's own space (the face's were moved there by `attach_face_to_head`), so a part that
    // merely moved hashes the same.
    for quad in &part.quads {
        for vertex in quad.vertices {
            vertex.map(f32::to_bits).hash(&mut hasher);
        }
        for uv in quad.uv {
            uv.map(f32::to_bits).hash(&mut hasher);
        }
        (quad.texture as u8).hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(layers: bool, elytra: bool) -> SceneInput {
        SceneInput {
            pose: PreviewPose { time_seconds: 1.0, locomotion_blend: 0.0, limb_swing: 0.0 },
            variant: MinecraftSkinVariant::Classic,
            layers_3d: layers,
            show_elytra: elytra,
            expressions: false,
            expression_layout: None,
            skin: Arc::new(RgbaImage::from_pixel(64, 64, image::Rgba([200, 120, 80, 255]))),
            cape: None,
            default_elytra: None,
            cape_uv: crate::screens::skins::cape_uv_layout::default_cape_uv_layout(),
        }
    }

    fn tags(parts: &[ScenePart]) -> Vec<PartTag> {
        parts.iter().map(|part| part.tag).collect()
    }

    #[test]
    fn the_character_is_six_parts_with_a_base_and_an_overlay_box_each() {
        let parts = capture_scene(&input(false, false));
        assert_eq!(
            tags(&parts),
            [PartTag::Body, PartTag::Head, PartTag::RightArm, PartTag::LeftArm, PartTag::RightLeg, PartTag::LeftLeg]
        );
        // Two cuboids (base and outer layer) of six faces.
        assert!(parts.iter().all(|part| part.quads.len() == 12), "{:?}", parts.iter().map(|p| p.quads.len()).collect::<Vec<_>>());
    }

    #[test]
    fn moving_the_pose_moves_parts_but_not_their_geometry() {
        let rest = capture_scene(&input(false, false));
        let mut moved_input = input(false, false);
        moved_input.pose = PreviewPose { time_seconds: 2.7, locomotion_blend: 1.0, limb_swing: 3.1 };
        let moved = capture_scene(&moved_input);
        for (a, b) in rest.iter().zip(&moved) {
            assert_eq!(a.tag, b.tag);
            assert_eq!(local_geometry_hash(a), local_geometry_hash(b), "{:?}'s geometry changed with the pose", a.tag);
        }
        let leg = |parts: &[ScenePart]| parts.iter().find(|part| part.tag == PartTag::LeftLeg).unwrap().transform;
        assert_ne!(leg(&rest), leg(&moved), "a walking leg is somewhere else");
    }

    #[test]
    fn voxel_layers_add_quads_to_the_same_parts() {
        let flat = capture_scene(&input(false, false));
        let voxels = capture_scene(&input(true, false));
        assert_eq!(tags(&flat), tags(&voxels));
        let head = |parts: &[ScenePart]| parts.iter().find(|part| part.tag == PartTag::Head).unwrap().quads.len();
        assert!(head(&voxels) > head(&flat), "an opaque skin has hundreds of voxels on the head");
    }

    #[test]
    fn elytra_adds_two_wings_and_drops_the_cape() {
        let parts = capture_scene(&input(false, true));
        assert!(tags(&parts).contains(&PartTag::ElytraLeft));
        assert!(tags(&parts).contains(&PartTag::ElytraRight));
        assert!(!tags(&parts).contains(&PartTag::Cape));
    }

    #[test]
    fn a_capture_leaves_nothing_recorded_behind() {
        let _ = capture_scene(&input(false, false));
        // Emitting a quad now (as the CPU preview does all the time) records nothing and does not panic.
        record_quad(
            TriangleTexture::Skin,
            ModelTransform::translation(Vec3::new(0.0, 0.0, 0.0)),
            [Vec3::new(0.0, 0.0, 0.0); 4],
            Vec3::new(0.0, 1.0, 0.0),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        );
        RECORDER.with(|recorder| assert!(recorder.borrow().is_none()));
    }

    #[test]
    fn the_affine_inverse_undoes_the_matrix() {
        let m = matrix_of(ModelTransform::from_pivot_rotation(Vec3::new(1.0, 2.0, 3.0), 0.4, -0.7, 0.2));
        let inverse = affine_inverse(&m).expect("a rotation is invertible");
        let point = affine_point(&inverse, affine_point(&m, [3.0, -1.0, 2.5]));
        for (a, b) in point.iter().zip([3.0, -1.0, 2.5]) {
            assert!((a - b).abs() < 1e-4, "{point:?}");
        }
    }
}
