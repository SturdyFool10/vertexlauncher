use tokio::runtime::{Builder, Runtime};

use crate::{RuntimeBootstrapError, RuntimeBootstrapSpec};

pub fn build_runtime(spec: &RuntimeBootstrapSpec<'_>) -> Result<Runtime, RuntimeBootstrapError> {
    match Builder::new_multi_thread()
        .enable_all()
        .thread_name(spec.thread_name)
        .build()
    {
        Ok(runtime) => Ok(runtime),
        Err(multi_thread_error) => {
            log_runtime_warning(spec, &multi_thread_error);
            Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|single_thread_error| {
                    RuntimeBootstrapError::new(
                        spec.runtime_name,
                        &multi_thread_error,
                        &single_thread_error,
                    )
                })
        }
    }
}

fn log_runtime_warning(
    spec: &RuntimeBootstrapSpec<'_>,
    multi_thread_error: &impl std::fmt::Display,
) {
    let message = format!(
        "Failed to build multi-thread {}; falling back to single-thread runtime",
        spec.runtime_name,
    );
    if tracing::dispatcher::has_been_set() {
        tracing::warn!(
            target: "vertexlauncher/runtime_bootstrap",
            runtime_target = spec.log_target,
            error = %multi_thread_error,
            "{message}"
        );
    } else {
        eprintln!("{message}: {multi_thread_error}");
    }
}
