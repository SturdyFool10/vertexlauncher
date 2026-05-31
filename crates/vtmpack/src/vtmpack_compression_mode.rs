#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VtmpackCompressionMode {
    Standard,
    Extreme,
}

impl VtmpackCompressionMode {
    pub fn label(self) -> &'static str {
        match self {
            VtmpackCompressionMode::Standard => "Standard XZ compression",
            VtmpackCompressionMode::Extreme => "Extreme XZ compression",
        }
    }
}
