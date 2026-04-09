use super::*;

#[derive(Clone, Debug)]
pub struct TextPath {
    pub points: Vec<TextPoint>,
    pub closed: bool,
}

impl TextPath {
    pub fn new(points: impl Into<Vec<TextPoint>>) -> Self {
        Self {
            points: points.into(),
            closed: false,
        }
    }
}
