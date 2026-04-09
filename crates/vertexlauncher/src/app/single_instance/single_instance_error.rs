#[derive(Debug)]
pub enum SingleInstanceError {
    AlreadyRunning,
    Unavailable(String),
}
