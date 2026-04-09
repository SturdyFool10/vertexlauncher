#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct TextModifiers {
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub command: bool,
    pub mac_cmd: bool,
}
