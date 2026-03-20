#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#[cfg(all(target_os = "windows", target_env = "gnu"))]
compile_error!(
    "Windows GNU builds are not supported for Vertex Launcher. \
Use the MSVC target (for example: x86_64-pc-windows-msvc) so WebView2 loader libraries are statically linked."
);

mod app;

fn main() -> eframe::Result<()> {
    let _ = app::init_tracing();

    match app::maybe_run_webview_helper() {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
    match app::maybe_run_cli_command() {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }

    let _single_instance_guard = match app::acquire_single_instance() {
        Ok(guard) => guard,
        Err(app::SingleInstanceError::AlreadyRunning) => {
            report_startup_message(
                "Vertex Launcher is already running on this machine.",
                rfd::MessageLevel::Warning,
            );
            return Ok(());
        }
        Err(app::SingleInstanceError::Unavailable(err)) => {
            report_startup_message(
                format!("Vertex Launcher could not verify single-instance startup: {err}"),
                rfd::MessageLevel::Error,
            );
            std::process::exit(1);
        }
    };

    match app::run() {
        Ok(()) => Ok(()),
        Err(app::RunError::RuntimeBootstrap(err)) => {
            report_startup_message(
                format!("Vertex Launcher could not start its background runtime: {err}"),
                rfd::MessageLevel::Error,
            );
            std::process::exit(1);
        }
        Err(app::RunError::Ui(err)) => Err(err),
    }
}

fn report_startup_message(message: impl Into<String>, level: rfd::MessageLevel) {
    let message = message.into();
    match level {
        rfd::MessageLevel::Info => tracing::info!("{message}"),
        rfd::MessageLevel::Warning => tracing::warn!("{message}"),
        rfd::MessageLevel::Error => tracing::error!("{message}"),
    }
    eprintln!("{message}");
    let _ = rfd::MessageDialog::new()
        .set_title("Vertex Launcher")
        .set_description(message)
        .set_level(level)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}
