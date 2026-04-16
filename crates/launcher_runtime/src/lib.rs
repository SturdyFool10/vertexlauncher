//! Shared Tokio runtime used by launcher UI/background tasks.

use std::{
    future::Future,
    pin::Pin,
    sync::OnceLock,
    task::{Context, Poll},
};

pub use runtime_bootstrap::RuntimeBootstrapError;
use runtime_bootstrap::{RuntimeBootstrapSpec, build_runtime};
use tokio::runtime::Runtime;

static TOKIO_RUNTIME: OnceLock<Result<Runtime, RuntimeBootstrapError>> = OnceLock::new();
static DETACHED_TASK_REPORTER: OnceLock<fn(&str, &TaskError)> = OnceLock::new();
const RUNTIME_SPEC: RuntimeBootstrapSpec<'static> =
    RuntimeBootstrapSpec::new("vertex-tokio", "vertexlauncher/runtime", "launcher runtime");

#[derive(Debug, Clone)]
pub enum TaskError {
    RuntimeUnavailable(RuntimeBootstrapError),
    JoinFailed(String),
}

impl std::fmt::Display for TaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RuntimeUnavailable(error) => write!(f, "{error}"),
            Self::JoinFailed(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for TaskError {}

pub fn set_detached_task_reporter(reporter: fn(&str, &TaskError)) {
    let _ = DETACHED_TASK_REPORTER.set(reporter);
}

fn log_detached_task_failure(task_kind: &str, error: &TaskError) {
    if let Some(reporter) = DETACHED_TASK_REPORTER.get().copied() {
        reporter(task_kind, error);
    }
    if tracing::dispatcher::has_been_set() {
        tracing::error!(
            target: "vertexlauncher/runtime_bootstrap",
            error = %error,
            task_kind,
            "Detached launcher task failed"
        );
    } else {
        eprintln!("Detached launcher task failed ({task_kind}): {error}");
    }
}

enum TaskHandleInner<T> {
    Running(tokio::task::JoinHandle<T>),
    Failed(TaskError),
}

pub struct TaskHandle<T> {
    inner: TaskHandleInner<T>,
}

impl<T> TaskHandle<T> {
    fn from_join_handle(handle: tokio::task::JoinHandle<T>) -> Self {
        Self {
            inner: TaskHandleInner::Running(handle),
        }
    }

    fn failed(error: TaskError) -> Self {
        Self {
            inner: TaskHandleInner::Failed(error),
        }
    }
}

impl<T> Future for TaskHandle<T> {
    type Output = Result<T, TaskError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match &mut self.inner {
            TaskHandleInner::Running(handle) => Pin::new(handle)
                .poll(cx)
                .map(|result| result.map_err(|error| TaskError::JoinFailed(error.to_string()))),
            TaskHandleInner::Failed(error) => Poll::Ready(Err(error.clone())),
        }
    }
}

fn runtime_state() -> &'static Result<Runtime, RuntimeBootstrapError> {
    TOKIO_RUNTIME.get_or_init(|| build_runtime(&RUNTIME_SPEC))
}

fn runtime() -> Result<&'static Runtime, RuntimeBootstrapError> {
    match runtime_state() {
        Ok(runtime) => Ok(runtime),
        Err(error) => Err(error.clone()),
    }
}

fn log_runtime_unavailable(task_kind: &str, error: &RuntimeBootstrapError) {
    let message = format!("Unable to {task_kind}: launcher runtime initialization failed");
    if tracing::dispatcher::has_been_set() {
        tracing::error!(
            target: "vertexlauncher/runtime_bootstrap",
            error = %error,
            task_kind,
            "{message}"
        );
    } else {
        eprintln!("{message}: {error}");
    }
}

/// Eagerly initializes the shared runtime.
///
/// This is optional because calling [`spawn`] or [`spawn_blocking`] will lazily
/// initialize the runtime as needed.
pub fn init() -> Result<(), RuntimeBootstrapError> {
    runtime().map(|_| ())
}

/// Spawns an asynchronous task onto the shared launcher runtime.
pub fn spawn<F>(future: F) -> TaskHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    match runtime() {
        Ok(runtime) => TaskHandle::from_join_handle(runtime.spawn(future)),
        Err(error) => {
            log_runtime_unavailable("spawn async task", &error);
            TaskHandle::failed(TaskError::RuntimeUnavailable(error))
        }
    }
}

/// Runs an asynchronous operation to completion on the shared launcher runtime.
pub fn block_on<F>(future: F) -> Result<F::Output, TaskError>
where
    F: Future,
{
    match runtime() {
        Ok(runtime) => Ok(runtime.block_on(future)),
        Err(error) => {
            log_runtime_unavailable("block on async task", &error);
            Err(TaskError::RuntimeUnavailable(error))
        }
    }
}

/// Runs a blocking closure on the runtime blocking thread pool.
pub fn spawn_blocking<F, R>(task: F) -> TaskHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    match runtime() {
        Ok(runtime) => TaskHandle::from_join_handle(runtime.spawn_blocking(task)),
        Err(error) => {
            log_runtime_unavailable("spawn blocking task", &error);
            TaskHandle::failed(TaskError::RuntimeUnavailable(error))
        }
    }
}

/// Spawns an asynchronous task and logs any runtime initialization or join failure.
pub fn spawn_detached<F>(future: F)
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    match runtime() {
        Ok(runtime) => {
            let handle = runtime.spawn(future);
            runtime.spawn(async move {
                if let Err(error) = handle.await {
                    log_detached_task_failure(
                        "detached async task",
                        &TaskError::JoinFailed(error.to_string()),
                    );
                }
            });
        }
        Err(error) => {
            log_detached_task_failure("detached async task", &TaskError::RuntimeUnavailable(error));
        }
    }
}

/// Runs a blocking closure detached from the caller and logs runtime/join failures.
pub fn spawn_blocking_detached<F, R>(task: F)
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    match runtime() {
        Ok(runtime) => {
            let handle = runtime.spawn_blocking(task);
            runtime.spawn(async move {
                if let Err(error) = handle.await {
                    log_detached_task_failure(
                        "detached blocking task",
                        &TaskError::JoinFailed(error.to_string()),
                    );
                }
            });
        }
        Err(error) => {
            log_detached_task_failure(
                "detached blocking task",
                &TaskError::RuntimeUnavailable(error),
            );
        }
    }
}
