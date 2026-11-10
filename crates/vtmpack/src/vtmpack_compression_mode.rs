#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VtmpackCompressionMode {
    Standard,
    Extreme,
}

impl VtmpackCompressionMode {
    pub fn label(self) -> &'static str {
        match self {
            VtmpackCompressionMode::Standard => vertex_i18n::src!("Standard XZ compression"),
            VtmpackCompressionMode::Extreme => vertex_i18n::src!("Extreme XZ compression"),
        }
    }
}
