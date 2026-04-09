#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextGraphicsApi {
    Auto,
    Vulkan,
    Metal,
    Dx12,
    Gl,
}
