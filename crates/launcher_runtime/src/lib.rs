//! Shared Tokio runtime used by launcher UI/background tasks.

use std::{future::Future, sync::OnceLock};

use runtime_bootstrap::{RuntimeBootstrapSpec, build_runtime};
use tokio::runtime::Runtime;

static TOKIO_RUNTIME: OnceLock<Runtime> = OnceLock::new();
const RUNTIME_SPEC: RuntimeBootstrapSpec<'static> =
    RuntimeBootstrapSpec::new("vertex-tokio", "vertexlauncher/runtime", "launcher runtime");

fn runtime() -> &'static Runtime {
    TOKIO_RUNTIME.get_or_init(|| {
        build_runtime(&RUNTIME_SPEC).unwrap_or_else(|error| {
            panic!("Unrecoverable: {error}");
        })
    })
}

/// Eagerly initializes the shared runtime.
///
/// This is optional because calling [`spawn`] or [`spawn_blocking`] will lazily
/// initialize the runtime as needed.
pub fn init() {
    let _ = runtime();
}

/// Spawns an asynchronous task onto the shared launcher runtime.
pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    runtime().spawn(future)
}

/// Runs a blocking closure on the runtime blocking thread pool.
pub fn spawn_blocking<F, R>(task: F) -> tokio::task::JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    runtime().spawn_blocking(task)
}
