use super::*;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum TextInputEvent {
    Text(String),
    Copy,
    Cut,
    Paste(String),
    Key {
        key: TextKey,
        pressed: bool,
        modifiers: TextModifiers,
    },
    PointerButton {
        button: TextPointerButton,
        pressed: bool,
        modifiers: TextModifiers,
    },
}
