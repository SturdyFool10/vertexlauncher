use super::*;

pub struct LoopbackCallbackListener {
    pub(super) listener: TcpListener,
    pub(super) redirect_uri: String,
}

impl LoopbackCallbackListener {
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }
}
