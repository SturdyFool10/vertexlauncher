#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#[cfg(all(target_os = "windows", target_env = "gnu"))]
compile_error!(
    "Windows GNU builds are not supported for Vertex Launcher. \
Use the MSVC target (for example: x86_64-pc-windows-msvc) so WebView2 loader libraries are statically linked."
);

mod app;

fn main() -> std::process::ExitCode {
    // First, so the startup timeline (logged when the first frame is out) counts from the start of `main`.
    vertex_shell::watchdog::start();
    let log_path = app::init_tracing();
    vertex_shell::watchdog::step("logging ready");

    match app::maybe_run_webview_helper() {
        Ok(true) => return std::process::ExitCode::SUCCESS,
        Ok(false) => {}
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
    match app::maybe_run_cli_command() {
        Ok(true) => return std::process::ExitCode::SUCCESS,
        Ok(false) => {}
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }

    // Fatal signals and Windows unhandled exceptions: the engine prints its scopes and native stack into the log.
    vertex_shell::sturdy::trace::install_crash_handler(log_path.as_deref());

    let _single_instance_guard = match app::acquire_single_instance() {
        Ok(guard) => guard,
        Err(app::SingleInstanceError::AlreadyRunning) => {
            report_startup_message(
                "Vertex Launcher is already running on this machine.",
                MessageLevel::Warning,
            );
            return std::process::ExitCode::SUCCESS;
        }
        Err(app::SingleInstanceError::Unavailable(err)) => {
            report_startup_message(
                format!("Vertex Launcher could not verify single-instance startup: {err}"),
                MessageLevel::Error,
            );
            std::process::exit(1);
        }
    };

    let result = app::run_se5();
    match result {
        Ok(code) => code,
        Err(app::RunError::RuntimeBootstrap(err)) => {
            report_startup_message(
                format!("Vertex Launcher could not start its background runtime: {err}"),
                MessageLevel::Error,
            );
            std::process::exit(1);
        }
    }
}

#[derive(Clone, Copy)]
enum MessageLevel {
    Warning,
    Error,
}

fn report_startup_message(message: impl Into<String>, level: MessageLevel) {
    use vertex_shell::sturdy::desktop::{MessageBox, MessageBoxKind, message_box};

    let message = message.into();
    let kind = match level {
        MessageLevel::Warning => {
            tracing::warn!("{message}");
            MessageBoxKind::Warning
        }
        MessageLevel::Error => {
            tracing::error!("{message}");
            MessageBoxKind::Error
        }
    };
    eprintln!("{message}");
    let _ = message_box(&MessageBox {
        kind,
        title: "Vertex Launcher".to_owned(),
        message,
        default_button: 0,
        escape_button: 0,
        ..Default::default()
    });
}
