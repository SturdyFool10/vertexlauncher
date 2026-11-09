use super::*;

/// How far behind the body's center each wing hinges, in pixels.
///
/// The stock elytra layer pushes the model 2px back; the animated elytra in the game cancels
/// that and hinges the wings 1px behind the body center instead. The wings therefore sit 1px
/// closer to the body than the unmodified model, which is what makes their top edge sink
/// further into the back.
const ELYTRA_WING_HINGE_DEPTH: f32 = 1.0;

/// Procedural elytra motion layered on top of the vanilla standing pose, as Minecraft
/// model-space rotations (y down, front -z).
#[derive(Clone, Copy)]
pub(in super::super::super) struct ElytraMotion {
    pub(in super::super::super) wings_rx: f32,
    pub(in super::super::super) wing_ry: f32,
    pub(in super::super::super) right_wing_rz: f32,
    pub(in super::super::super) left_wing_rz: f32,
}

/// Builds both wings as `(transform, box)` pairs. Each transform maps the box's Minecraft
/// model space into preview space; `body_transform` is the body part's pivot at the neck.
///
/// Both wings share a group pivoted at the body center (which carries the lift swing), and
/// each wing hangs from its shoulder hinge inside it. Each is a vanilla `ElytraModel` box:
/// `texOffs(22, 0)`, 10×20×2 with a 1.0 cube inflation on a 64×32 texture, the right wing
/// mirrored.
fn elytra_wing_parts(
    body_transform: ModelTransform,
    motion: ElytraMotion,
) -> [(ModelTransform, MinecraftBox); 2] {
    let neutral_pose = VANILLA_ELYTRA_POSE_STANDING;
    let wings = body_transform
        .then(ModelTransform::from_pivot_rotation(
            Vec3::new(0.0, 0.0, 0.0),
            motion.wings_rx,
            0.0,
            0.0,
        ))
        .then(ModelTransform::minecraft_model_axes());

    let wing_specs = [
        (neutral_pose.left, 5.0, motion.left_wing_rz, -10.0, false),
        (neutral_pose.right, -5.0, motion.right_wing_rz, 0.0, true),
    ];
    wing_specs.map(|(wing_pose, pivot_x, roll, box_x, mirror)| {
        let wing = wings.then(ModelTransform::from_pivot_rotation(
            Vec3::new(pivot_x, wing_pose.pivot_y_offset, ELYTRA_WING_HINGE_DEPTH),
            wing_pose.rotate_x,
            wing_pose.rotate_y + motion.wing_ry,
            wing_pose.rotate_z + roll,
        ));
        let cube = MinecraftBox {
            origin: Vec3::new(box_x, 0.0, 0.0),
            size: Vec3::new(10.0, 20.0, 2.0),
            inflate: 1.0,
            tex_offset: [22.0, 0.0],
            texture_size: [64.0, 32.0],
            mirror,
        };
        (wing, cube)
    })
}

/// Adds both elytra wings attached to `body_transform` (the body part's pivot at the neck).
pub(in super::super::super) fn add_elytra_triangles(
    out: &mut Vec<RenderTriangle>,
    texture: TriangleTexture,
    camera: &Camera,
    projection: Projection,
    rect: Rect,
    body_transform: ModelTransform,
    motion: ElytraMotion,
    light_dir: Vec3,
) {
    let _ = VANILLA_ELYTRA_POSE_SNEAKING;
    let _ = VANILLA_ELYTRA_POSE_GLIDE_OPEN;
    for (index, (wing, cube)) in elytra_wing_parts(body_transform, motion).into_iter().enumerate() {
        // `elytra_wing_parts` returns the left wing first.
        capture_part(if index == 0 { PartTag::ElytraLeft } else { PartTag::ElytraRight });
        add_minecraft_box_triangles(
            out, texture, wing, cube, camera, projection, rect, light_dir,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: Vec3, expected: [f32; 3]) {
        let delta = (actual.x - expected[0]).abs()
            + (actual.y - expected[1]).abs()
            + (actual.z - expected[2]).abs();
        assert!(
            delta < 0.01,
            "got ({:.4}, {:.4}, {:.4}), expected {expected:?}",
            actual.x,
            actual.y,
            actual.z
        );
    }

    fn corner(wing: &(ModelTransform, MinecraftBox), x: f32, y: f32, z: f32) -> Vec3 {
        wing.0.transform_point(Vec3::new(x, y, z))
    }

    /// Reference positions come from the game's elytra model chain worked out
    /// independently of this code: a vanilla wing box hinged at (∓5, 0, 1) behind the
    /// body center, tilted 15° on x and ±15° on z, optionally under the lift swing.
    #[test]
    fn wings_hinge_one_pixel_behind_the_body_center() {
        let neck = ModelTransform::translation(Vec3::new(0.0, 24.0, 0.0));
        let at_rest = ElytraMotion {
            wings_rx: 0.0,
            wing_ry: 0.0,
            right_wing_rz: 0.0,
            left_wing_rz: 0.0,
        };
        let [left, right] = elytra_wing_parts(neck, at_rest);
        assert_close(corner(&right, -1.0, -1.0, -1.0), [-5.7829, 24.9418, 0.2247]);
        assert_close(corner(&right, 11.0, 21.0, 3.0), [0.5761, 2.3097, -9.3330]);
        assert_close(corner(&left, 1.0, -1.0, -1.0), [5.7829, 24.9418, 0.2247]);
        assert_close(corner(&left, -11.0, 21.0, 3.0), [-0.5761, 2.3097, -9.3330]);
    }

    #[test]
    fn lift_swing_rotates_the_wings_about_the_body_center() {
        let neck = ModelTransform::translation(Vec3::new(0.0, 24.0, 0.0));
        let lifted = ElytraMotion {
            wings_rx: 0.3,
            wing_ry: 0.0,
            right_wing_rz: 0.0,
            left_wing_rz: 0.0,
        };
        let [_, right] = elytra_wing_parts(neck, lifted);
        assert_close(corner(&right, -1.0, -1.0, -1.0), [-5.7829, 24.8333, 0.4930]);
        assert_close(corner(&right, 11.0, 21.0, 3.0), [0.5761, 6.0366, -15.3260]);
    }
}
