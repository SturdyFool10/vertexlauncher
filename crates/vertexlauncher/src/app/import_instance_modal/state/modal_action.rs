use super::*;

#[derive(Clone, Debug)]
pub enum ModalAction {
    None,
    Cancel,
    Import(ImportRequest),
}
