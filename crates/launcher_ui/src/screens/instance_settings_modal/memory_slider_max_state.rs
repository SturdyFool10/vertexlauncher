use std::sync::mpsc;

#[derive(Default)]
pub(super) struct MemorySliderMaxState {
    pub(super) detected_total_mib: Option<u128>,
    pub(super) load_complete: bool,
    pub(super) rx: Option<mpsc::Receiver<Option<u128>>>,
}
