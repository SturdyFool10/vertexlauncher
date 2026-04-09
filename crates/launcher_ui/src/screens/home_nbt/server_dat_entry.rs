#[derive(Debug, Clone)]
pub(crate) struct ServerDatEntry {
    pub(crate) name: String,
    pub(crate) ip: String,
    pub(crate) icon: Option<String>,
}
