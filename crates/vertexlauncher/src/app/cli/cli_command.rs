use super::*;

#[derive(Debug, Clone)]
pub(super) enum CliCommand {
    Launch {
        mode: QuickLaunchMode,
        instance: String,
        user: String,
        world: Option<String>,
        server: Option<String>,
    },
    BuildArgs {
        mode: QuickLaunchMode,
        instance: String,
        user: String,
        world: Option<String>,
        server: Option<String>,
    },
    ListTargets {
        instance: String,
    },
    Help,
}
