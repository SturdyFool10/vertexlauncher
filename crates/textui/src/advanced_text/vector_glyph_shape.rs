use super::*;

#[derive(Clone, Debug)]
pub struct VectorGlyphShape {
    pub bounds: TextRect,
    pub color: TextColor,
    pub commands: Vec<VectorPathCommand>,
}

impl VectorGlyphShape {
    pub fn to_svg_path_data(&self) -> String {
        let mut path = String::new();
        for command in &self.commands {
            match command {
                VectorPathCommand::MoveTo(point) => {
                    let _ = write!(path, "M{} {} ", point.x, point.y);
                }
                VectorPathCommand::LineTo(point) => {
                    let _ = write!(path, "L{} {} ", point.x, point.y);
                }
                VectorPathCommand::QuadTo(control, point) => {
                    let _ = write!(
                        path,
                        "Q{} {} {} {} ",
                        control.x, control.y, point.x, point.y
                    );
                }
                VectorPathCommand::CurveTo(control_a, control_b, point) => {
                    let _ = write!(
                        path,
                        "C{} {} {} {} {} {} ",
                        control_a.x, control_a.y, control_b.x, control_b.y, point.x, point.y
                    );
                }
                VectorPathCommand::Close => path.push_str("Z "),
            }
        }
        path.trim_end().to_owned()
    }
}
