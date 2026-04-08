use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExpressionOffset {
    Bottom,
    LowerMid,
    UpperMid,
    Top,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EyeFamily {
    TwoByOne,
    TwoByTwo,
    TwoByThree,
    ThreeByOne,
    OneByOne,
    OneByTwo,
    OneByThree,
    Spread,
    FarOneByOne,
    OneByOneInner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BrowKind {
    Standard,
    Hat,
    Spread,
    Villager,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct TextureRectU32 {
    pub(super) x: u32,
    pub(super) y: u32,
    pub(super) w: u32,
    pub(super) h: u32,
}

const fn tex(x: u32, y: u32, w: u32, h: u32) -> TextureRectU32 {
    TextureRectU32 { x, y, w, h }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct EyeExpressionSpec {
    pub(super) id: &'static str,
    pub(super) family: EyeFamily,
    pub(super) offset: ExpressionOffset,
    pub(super) right_eye: TextureRectU32,
    pub(super) left_eye: TextureRectU32,
    pub(super) right_white: Option<TextureRectU32>,
    pub(super) left_white: Option<TextureRectU32>,
    pub(super) right_pupil: Option<TextureRectU32>,
    pub(super) left_pupil: Option<TextureRectU32>,
    pub(super) blink: Option<TextureRectU32>,
    pub(super) right_center_x: f32,
    pub(super) left_center_x: f32,
    pub(super) center_y: f32,
    #[allow(dead_code)]
    pub(super) z: f32,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) pupil_width: f32,
    pub(super) pupil_height: f32,
    pub(super) gaze_scale_x: f32,
    pub(super) gaze_scale_y: f32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct BrowExpressionSpec {
    pub(super) id: &'static str,
    pub(super) kind: BrowKind,
    pub(super) offset: ExpressionOffset,
    pub(super) right_brow: TextureRectU32,
    pub(super) left_brow: Option<TextureRectU32>,
    pub(super) center_x: f32,
    pub(super) center_y: f32,
    #[allow(dead_code)]
    pub(super) z: f32,
    pub(super) width: f32,
    pub(super) height: f32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct DetectedExpressionsLayout {
    pub(super) eye: EyeExpressionSpec,
    pub(super) brow: Option<BrowExpressionSpec>,
}

const SUPPORTED_EYE_SPECS: &[EyeExpressionSpec] = &[
    EyeExpressionSpec {
        id: "eye_16",
        family: EyeFamily::Spread,
        offset: ExpressionOffset::LowerMid,
        right_eye: tex(36, 6, 2, 1),
        left_eye: tex(38, 6, 2, 1),
        right_white: Some(tex(36, 7, 1, 1)),
        left_white: Some(tex(39, 7, 1, 1)),
        right_pupil: Some(tex(37, 7, 1, 1)),
        left_pupil: Some(tex(38, 7, 1, 1)),
        blink: Some(tex(36, 6, 2, 1)),
        right_center_x: 2.975,
        left_center_x: -2.975,
        center_y: 27.000,
        z: 3.000,
        width: 2.025,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_10",
        family: EyeFamily::ThreeByOne,
        offset: ExpressionOffset::LowerMid,
        right_eye: tex(32, 2, 3, 1),
        left_eye: tex(37, 2, 3, 1),
        right_white: Some(tex(32, 3, 3, 1)),
        left_white: Some(tex(37, 3, 3, 1)),
        right_pupil: Some(tex(35, 3, 1, 1)),
        left_pupil: Some(tex(36, 3, 1, 1)),
        blink: Some(tex(32, 2, 3, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 27.000,
        z: 4.000,
        width: 3.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.450,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_11",
        family: EyeFamily::ThreeByOne,
        offset: ExpressionOffset::UpperMid,
        right_eye: tex(32, 0, 3, 1),
        left_eye: tex(37, 0, 3, 1),
        right_white: Some(tex(32, 1, 3, 1)),
        left_white: Some(tex(37, 1, 3, 1)),
        right_pupil: Some(tex(35, 1, 1, 1)),
        left_pupil: Some(tex(36, 1, 1, 1)),
        blink: Some(tex(32, 0, 3, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 28.000,
        z: 4.000,
        width: 3.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.450,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_9",
        family: EyeFamily::TwoByThree,
        offset: ExpressionOffset::UpperMid,
        right_eye: tex(32, 4, 2, 3),
        left_eye: tex(34, 4, 2, 3),
        right_white: Some(tex(32, 5, 1, 3)),
        left_white: Some(tex(35, 5, 1, 3)),
        right_pupil: Some(tex(33, 5, 1, 2)),
        left_pupil: Some(tex(34, 5, 1, 2)),
        blink: Some(tex(32, 4, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 26.500,
        z: 3.000,
        width: 2.000,
        height: 3.000,
        pupil_width: 1.000,
        pupil_height: 2.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.180,
    },
    EyeExpressionSpec {
        id: "eye_5",
        family: EyeFamily::TwoByTwo,
        offset: ExpressionOffset::Bottom,
        right_eye: tex(24, 5, 2, 2),
        left_eye: tex(26, 5, 2, 2),
        right_white: Some(tex(24, 6, 1, 2)),
        left_white: Some(tex(27, 6, 1, 2)),
        right_pupil: Some(tex(25, 6, 1, 2)),
        left_pupil: Some(tex(26, 6, 1, 2)),
        blink: Some(tex(24, 5, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 26.000,
        z: 3.000,
        width: 2.000,
        height: 2.000,
        pupil_width: 1.000,
        pupil_height: 2.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.180,
    },
    EyeExpressionSpec {
        id: "eye_6",
        family: EyeFamily::TwoByTwo,
        offset: ExpressionOffset::LowerMid,
        right_eye: tex(24, 2, 2, 2),
        left_eye: tex(26, 2, 2, 2),
        right_white: Some(tex(24, 3, 1, 2)),
        left_white: Some(tex(27, 3, 1, 2)),
        right_pupil: Some(tex(25, 3, 1, 2)),
        left_pupil: Some(tex(26, 3, 1, 2)),
        blink: Some(tex(24, 2, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 27.000,
        z: 3.000,
        width: 2.000,
        height: 2.000,
        pupil_width: 1.000,
        pupil_height: 2.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.180,
    },
    EyeExpressionSpec {
        id: "eye_7",
        family: EyeFamily::TwoByTwo,
        offset: ExpressionOffset::UpperMid,
        right_eye: tex(28, 5, 2, 2),
        left_eye: tex(30, 5, 2, 2),
        right_white: Some(tex(28, 6, 1, 2)),
        left_white: Some(tex(31, 6, 1, 2)),
        right_pupil: Some(tex(29, 6, 1, 2)),
        left_pupil: Some(tex(30, 6, 1, 2)),
        blink: Some(tex(28, 5, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 28.000,
        z: 3.000,
        width: 2.000,
        height: 2.000,
        pupil_width: 1.000,
        pupil_height: 2.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.180,
    },
    EyeExpressionSpec {
        id: "eye_8",
        family: EyeFamily::TwoByTwo,
        offset: ExpressionOffset::Top,
        right_eye: tex(28, 2, 2, 2),
        left_eye: tex(30, 2, 2, 2),
        right_white: Some(tex(28, 3, 1, 2)),
        left_white: Some(tex(31, 3, 1, 2)),
        right_pupil: Some(tex(29, 3, 1, 2)),
        left_pupil: Some(tex(30, 3, 1, 2)),
        blink: Some(tex(28, 2, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 29.000,
        z: 3.000,
        width: 2.000,
        height: 2.000,
        pupil_width: 1.000,
        pupil_height: 2.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.180,
    },
    EyeExpressionSpec {
        id: "eye_1",
        family: EyeFamily::TwoByOne,
        offset: ExpressionOffset::Bottom,
        right_eye: tex(4, 6, 2, 1),
        left_eye: tex(6, 6, 2, 1),
        right_white: Some(tex(4, 7, 1, 1)),
        left_white: Some(tex(7, 7, 1, 1)),
        right_pupil: Some(tex(5, 7, 1, 1)),
        left_pupil: Some(tex(6, 7, 1, 1)),
        blink: Some(tex(4, 6, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 26.000,
        z: 3.000,
        width: 2.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_2",
        family: EyeFamily::TwoByOne,
        offset: ExpressionOffset::LowerMid,
        right_eye: tex(4, 4, 2, 1),
        left_eye: tex(6, 4, 2, 1),
        right_white: Some(tex(4, 5, 1, 1)),
        left_white: Some(tex(7, 5, 1, 1)),
        right_pupil: Some(tex(5, 5, 1, 1)),
        left_pupil: Some(tex(6, 5, 1, 1)),
        blink: Some(tex(4, 4, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 27.000,
        z: 3.000,
        width: 2.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_3",
        family: EyeFamily::TwoByOne,
        offset: ExpressionOffset::UpperMid,
        right_eye: tex(4, 2, 2, 1),
        left_eye: tex(6, 2, 2, 1),
        right_white: Some(tex(4, 3, 1, 1)),
        left_white: Some(tex(7, 3, 1, 1)),
        right_pupil: Some(tex(5, 3, 1, 1)),
        left_pupil: Some(tex(6, 3, 1, 1)),
        blink: Some(tex(4, 2, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 28.000,
        z: 3.000,
        width: 2.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_4",
        family: EyeFamily::TwoByOne,
        offset: ExpressionOffset::Top,
        right_eye: tex(4, 0, 2, 1),
        left_eye: tex(6, 0, 2, 1),
        right_white: Some(tex(4, 1, 1, 1)),
        left_white: Some(tex(7, 1, 1, 1)),
        right_pupil: Some(tex(5, 1, 1, 1)),
        left_pupil: Some(tex(6, 1, 1, 1)),
        blink: Some(tex(4, 0, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 29.000,
        z: 3.000,
        width: 2.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_12",
        family: EyeFamily::OneByOne,
        offset: ExpressionOffset::Bottom,
        right_eye: tex(2, 7, 1, 1),
        left_eye: tex(3, 7, 1, 1),
        right_white: None,
        left_white: None,
        right_pupil: Some(tex(2, 7, 1, 1)),
        left_pupil: Some(tex(3, 7, 1, 1)),
        blink: None,
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 26.000,
        z: 3.000,
        width: 1.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_13",
        family: EyeFamily::OneByOne,
        offset: ExpressionOffset::LowerMid,
        right_eye: tex(2, 5, 1, 1),
        left_eye: tex(3, 5, 1, 1),
        right_white: None,
        left_white: None,
        right_pupil: Some(tex(2, 5, 1, 1)),
        left_pupil: Some(tex(3, 5, 1, 1)),
        blink: None,
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 27.000,
        z: 3.000,
        width: 1.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_14",
        family: EyeFamily::OneByOne,
        offset: ExpressionOffset::UpperMid,
        right_eye: tex(2, 3, 1, 1),
        left_eye: tex(3, 3, 1, 1),
        right_white: None,
        left_white: None,
        right_pupil: Some(tex(2, 3, 1, 1)),
        left_pupil: Some(tex(3, 3, 1, 1)),
        blink: None,
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 28.000,
        z: 3.000,
        width: 1.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_15",
        family: EyeFamily::OneByOne,
        offset: ExpressionOffset::Top,
        right_eye: tex(2, 1, 1, 1),
        left_eye: tex(3, 1, 1, 1),
        right_white: None,
        left_white: None,
        right_pupil: Some(tex(2, 1, 1, 1)),
        left_pupil: Some(tex(3, 1, 1, 1)),
        blink: None,
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 29.000,
        z: 3.000,
        width: 1.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_17",
        family: EyeFamily::FarOneByOne,
        offset: ExpressionOffset::UpperMid,
        right_eye: tex(0, 7, 1, 1),
        left_eye: tex(1, 7, 1, 1),
        right_white: None,
        left_white: None,
        right_pupil: None,
        left_pupil: None,
        blink: Some(tex(0, 7, 1, 1)),
        right_center_x: 2.475,
        left_center_x: -2.475,
        center_y: 28.000,
        z: 3.000,
        width: 1.025,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_18",
        family: EyeFamily::FarOneByOne,
        offset: ExpressionOffset::Top,
        right_eye: tex(0, 6, 1, 1),
        left_eye: tex(1, 6, 1, 1),
        right_white: None,
        left_white: None,
        right_pupil: None,
        left_pupil: None,
        blink: Some(tex(0, 6, 1, 1)),
        right_center_x: 2.475,
        left_center_x: -2.475,
        center_y: 29.000,
        z: 3.000,
        width: 1.025,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_19",
        family: EyeFamily::OneByOneInner,
        offset: ExpressionOffset::UpperMid,
        right_eye: tex(0, 5, 1, 1),
        left_eye: tex(1, 5, 1, 1),
        right_white: None,
        left_white: None,
        right_pupil: None,
        left_pupil: None,
        blink: Some(tex(0, 5, 1, 1)),
        right_center_x: 1.475,
        left_center_x: -1.475,
        center_y: 28.000,
        z: 3.000,
        width: 1.025,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_20",
        family: EyeFamily::OneByOneInner,
        offset: ExpressionOffset::Top,
        right_eye: tex(0, 4, 1, 1),
        left_eye: tex(1, 4, 1, 1),
        right_white: None,
        left_white: None,
        right_pupil: None,
        left_pupil: None,
        blink: Some(tex(0, 4, 1, 1)),
        right_center_x: 1.475,
        left_center_x: -1.475,
        center_y: 29.000,
        z: 3.000,
        width: 1.025,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_21",
        family: EyeFamily::OneByTwo,
        offset: ExpressionOffset::UpperMid,
        right_eye: tex(0, 3, 1, 2),
        left_eye: tex(1, 3, 1, 2),
        right_white: None,
        left_white: None,
        right_pupil: None,
        left_pupil: None,
        blink: Some(tex(0, 3, 1, 1)),
        right_center_x: 1.475,
        left_center_x: -1.475,
        center_y: 28.000,
        z: 3.000,
        width: 1.025,
        height: 2.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.180,
    },
    EyeExpressionSpec {
        id: "eye_22",
        family: EyeFamily::OneByTwo,
        offset: ExpressionOffset::Top,
        right_eye: tex(0, 2, 1, 2),
        left_eye: tex(1, 2, 1, 2),
        right_white: None,
        left_white: None,
        right_pupil: None,
        left_pupil: None,
        blink: Some(tex(0, 2, 1, 1)),
        right_center_x: 1.475,
        left_center_x: -1.475,
        center_y: 29.000,
        z: 3.000,
        width: 1.025,
        height: 2.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.180,
    },
    EyeExpressionSpec {
        id: "eye_23",
        family: EyeFamily::OneByThree,
        offset: ExpressionOffset::Top,
        right_eye: tex(0, 1, 1, 3),
        left_eye: tex(1, 1, 1, 3),
        right_white: None,
        left_white: None,
        right_pupil: None,
        left_pupil: None,
        blink: Some(tex(0, 1, 1, 1)),
        right_center_x: 1.475,
        left_center_x: -1.475,
        center_y: 29.000,
        z: 3.000,
        width: 1.025,
        height: 3.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.180,
    },
    EyeExpressionSpec {
        id: "eye_24",
        family: EyeFamily::OneByThree,
        offset: ExpressionOffset::Top,
        right_eye: tex(0, 0, 1, 3),
        left_eye: tex(1, 0, 1, 3),
        right_white: None,
        left_white: None,
        right_pupil: None,
        left_pupil: None,
        blink: Some(tex(0, 0, 1, 1)),
        right_center_x: 1.475,
        left_center_x: -1.475,
        center_y: 30.000,
        z: 3.000,
        width: 1.025,
        height: 3.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.180,
    },
];

const SUPPORTED_BROW_SPECS: &[BrowExpressionSpec] = &[
    BrowExpressionSpec {
        id: "brow_bottom",
        kind: BrowKind::Standard,
        offset: ExpressionOffset::Bottom,
        right_brow: tex(60, 7, 2, 1),
        left_brow: Some(tex(62, 7, 2, 1)),
        center_x: 2.000,
        center_y: 26.500,
        z: 4.000,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_lower_mid",
        kind: BrowKind::Standard,
        offset: ExpressionOffset::LowerMid,
        right_brow: tex(60, 5, 2, 1),
        left_brow: Some(tex(62, 5, 2, 1)),
        center_x: 2.000,
        center_y: 27.500,
        z: 4.000,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_upper_mid",
        kind: BrowKind::Standard,
        offset: ExpressionOffset::UpperMid,
        right_brow: tex(60, 3, 2, 1),
        left_brow: Some(tex(62, 3, 2, 1)),
        center_x: 2.000,
        center_y: 28.500,
        z: 4.000,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_top",
        kind: BrowKind::Standard,
        offset: ExpressionOffset::Top,
        right_brow: tex(60, 1, 2, 1),
        left_brow: Some(tex(62, 1, 2, 1)),
        center_x: 2.000,
        center_y: 29.500,
        z: 4.000,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_hat_bottom",
        kind: BrowKind::Hat,
        offset: ExpressionOffset::Bottom,
        right_brow: tex(56, 7, 2, 1),
        left_brow: Some(tex(58, 7, 2, 1)),
        center_x: 2.000,
        center_y: 26.500,
        z: 4.200,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_hat_lower_mid",
        kind: BrowKind::Hat,
        offset: ExpressionOffset::LowerMid,
        right_brow: tex(56, 6, 2, 1),
        left_brow: Some(tex(58, 6, 2, 1)),
        center_x: 2.000,
        center_y: 27.500,
        z: 4.200,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_hat_upper_mid",
        kind: BrowKind::Hat,
        offset: ExpressionOffset::UpperMid,
        right_brow: tex(56, 5, 2, 1),
        left_brow: Some(tex(58, 5, 2, 1)),
        center_x: 2.000,
        center_y: 28.500,
        z: 4.200,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_hat_top",
        kind: BrowKind::Hat,
        offset: ExpressionOffset::Top,
        right_brow: tex(56, 4, 2, 1),
        left_brow: Some(tex(58, 4, 2, 1)),
        center_x: 2.000,
        center_y: 29.500,
        z: 4.200,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_spread",
        kind: BrowKind::Spread,
        offset: ExpressionOffset::LowerMid,
        right_brow: tex(36, 5, 2, 1),
        left_brow: Some(tex(38, 5, 2, 1)),
        center_x: 3.000,
        center_y: 27.500,
        z: 4.000,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_villager",
        kind: BrowKind::Villager,
        offset: ExpressionOffset::UpperMid,
        right_brow: tex(24, 1, 5, 1),
        left_brow: None,
        center_x: 0.000,
        center_y: 28.500,
        z: 4.000,
        width: 6.000,
        height: 1.0,
    },
];

pub(super) fn detect_expression_layout(image: &RgbaImage) -> Option<DetectedExpressionsLayout> {
    let eye = SUPPORTED_EYE_SPECS
        .iter()
        .copied()
        .filter_map(|spec| eye_layout_score(image, spec).map(|score| (score, spec)))
        .max_by(|(score_a, _), (score_b, _)| score_a.total_cmp(score_b))
        .map(|(_, spec)| spec)?;
    let brow = SUPPORTED_BROW_SPECS
        .iter()
        .copied()
        .filter_map(|spec| brow_layout_score(image, spec).map(|score| (score, spec)))
        .max_by(|(score_a, spec_a), (score_b, spec_b)| {
            score_a.total_cmp(score_b).then_with(|| {
                compatibility_score(eye, *spec_a).cmp(&compatibility_score(eye, *spec_b))
            })
        })
        .map(|(_, spec)| spec);

    Some(DetectedExpressionsLayout { eye, brow })
}

fn eye_layout_score(image: &RgbaImage, spec: EyeExpressionSpec) -> Option<f32> {
    let right_pixels = region_alpha_pixels(image, spec.right_eye);
    let left_pixels = region_alpha_pixels(image, spec.left_eye);
    if right_pixels == 0 || left_pixels == 0 {
        return None;
    }

    let right_coverage = region_alpha_coverage(image, spec.right_eye);
    let left_coverage = region_alpha_coverage(image, spec.left_eye);
    let mut score = right_coverage + left_coverage;
    score += (right_pixels + left_pixels) as f32 * 0.12;
    score -= (right_coverage - left_coverage).abs() * 0.35;

    if let (Some(right_white), Some(left_white)) = (spec.right_white, spec.left_white) {
        let right_white_pixels = region_alpha_pixels(image, right_white);
        let left_white_pixels = region_alpha_pixels(image, left_white);
        if right_white_pixels > 0 && left_white_pixels > 0 {
            score += region_alpha_coverage(image, right_white)
                + region_alpha_coverage(image, left_white);
            score += (right_white_pixels + left_white_pixels) as f32 * 0.08;
        }
    }
    if let (Some(right_pupil), Some(left_pupil)) = (spec.right_pupil, spec.left_pupil) {
        let right_pupil_pixels = region_alpha_pixels(image, right_pupil);
        let left_pupil_pixels = region_alpha_pixels(image, left_pupil);
        if right_pupil_pixels > 0 && left_pupil_pixels > 0 {
            score += region_alpha_coverage(image, right_pupil)
                + region_alpha_coverage(image, left_pupil);
            score += (right_pupil_pixels + left_pupil_pixels) as f32 * 0.06;
        }
    }
    if let Some((right_lid, left_lid)) = eye_lid_rects_if_present(spec) {
        let right_lid_pixels = region_alpha_pixels(image, right_lid);
        let left_lid_pixels = region_alpha_pixels(image, left_lid);
        if right_lid_pixels > 0 && left_lid_pixels > 0 {
            score += (region_alpha_coverage(image, right_lid)
                + region_alpha_coverage(image, left_lid))
                * 0.35;
            score += (right_lid_pixels + left_lid_pixels) as f32 * 0.05;
        }
    }

    (score >= 0.18).then_some(score)
}

fn eye_lid_rects_if_present(spec: EyeExpressionSpec) -> Option<(TextureRectU32, TextureRectU32)> {
    spec.blink.map(|_| eye_lid_rects(spec))
}

fn brow_layout_score(image: &RgbaImage, spec: BrowExpressionSpec) -> Option<f32> {
    let right_pixels = region_alpha_pixels(image, spec.right_brow);
    if right_pixels == 0 {
        return None;
    }
    let mut score = region_alpha_coverage(image, spec.right_brow) + right_pixels as f32 * 0.1;
    if let Some(left_brow) = spec.left_brow {
        let left_pixels = region_alpha_pixels(image, left_brow);
        if left_pixels == 0 {
            return None;
        }
        score += region_alpha_coverage(image, left_brow) + left_pixels as f32 * 0.1;
    }
    Some(score)
}

fn region_alpha_pixels(image: &RgbaImage, rect: TextureRectU32) -> u32 {
    let max_x = (rect.x + rect.w).min(image.width());
    let max_y = (rect.y + rect.h).min(image.height());
    let mut covered = 0u32;
    for py in rect.y..max_y {
        for px in rect.x..max_x {
            if image.get_pixel(px, py).0[3] > 24 {
                covered += 1;
            }
        }
    }
    covered
}

fn region_alpha_coverage(image: &RgbaImage, rect: TextureRectU32) -> f32 {
    let max_x = (rect.x + rect.w).min(image.width());
    let max_y = (rect.y + rect.h).min(image.height());
    let mut covered = 0u32;
    let mut total = 0u32;
    for py in rect.y..max_y {
        for px in rect.x..max_x {
            total += 1;
            if image.get_pixel(px, py).0[3] > 24 {
                covered += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        covered as f32 / total as f32
    }
}
