use super::*;

#[derive(Clone, Debug)]
pub(crate) struct AsyncRasterResponse {
    pub(crate) key_hash: u64,
    pub(crate) layout: PreparedTextLayout,
}
