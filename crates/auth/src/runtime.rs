use std::sync::OnceLock;

use runtime_bootstrap::{RuntimeBootstrapSpec, build_runtime};
use tokio::runtime::{Handle, Runtime};

static AUTH_TOKIO_RUNTIME: OnceLock<Runtime> = OnceLock::new();
const AUTH_RUNTIME_SPEC: RuntimeBootstrapSpec<'static> = RuntimeBootstrapSpec::new(
    "vertex-auth-tokio",
    "vertexlauncher/auth/runtime",
    "auth runtime",
);

pub(crate) fn auth_runtime() -> &'static Runtime {
    AUTH_TOKIO_RUNTIME.get_or_init(|| {
        build_runtime(&AUTH_RUNTIME_SPEC).unwrap_or_else(|error| {
            panic!("Unrecoverable: {error}");
        })
    })
}

pub(crate) fn auth_runtime_handle() -> &'static Handle {
    auth_runtime().handle()
}
