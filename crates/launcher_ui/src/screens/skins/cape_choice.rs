#[derive(Clone, Debug, Default)]
pub(super) struct CapeChoice {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) texture_bytes: Option<Vec<u8>>,
    pub(super) texture_size: Option<[u32; 2]>,
}
