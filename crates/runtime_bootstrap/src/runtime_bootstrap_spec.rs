#[derive(Clone, Copy, Debug)]
pub struct RuntimeBootstrapSpec<'a> {
    pub thread_name: &'a str,
    pub log_target: &'a str,
    pub runtime_name: &'a str,
}

impl<'a> RuntimeBootstrapSpec<'a> {
    pub const fn new(thread_name: &'a str, log_target: &'a str, runtime_name: &'a str) -> Self {
        Self {
            thread_name,
            log_target,
            runtime_name,
        }
    }
}
