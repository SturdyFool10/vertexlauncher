use std::fmt;

#[derive(Debug, thiserror::Error)]
#[error(
    "failed to build both multi-thread and single-thread Tokio runtimes for {runtime_name}. multi-thread error: {multi_thread_error}; single-thread error: {single_thread_error}"
)]
pub struct RuntimeBootstrapError {
    runtime_name: String,
    multi_thread_error: String,
    single_thread_error: String,
}

impl RuntimeBootstrapError {
    pub fn new(
        runtime_name: &str,
        multi_thread_error: &impl fmt::Display,
        single_thread_error: &impl fmt::Display,
    ) -> Self {
        Self {
            runtime_name: runtime_name.to_owned(),
            multi_thread_error: multi_thread_error.to_string(),
            single_thread_error: single_thread_error.to_string(),
        }
    }
}
