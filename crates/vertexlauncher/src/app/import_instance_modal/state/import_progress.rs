#[derive(Clone, Debug)]
pub struct ImportProgress {
    pub message: String,
    pub completed_steps: usize,
    pub total_steps: usize,
}
