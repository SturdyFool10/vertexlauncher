#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;

fn main() -> eframe::Result<()> {
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

    app::run()
}

fn report_startup_message(message: impl Into<String>, level: rfd::MessageLevel) {
    let message = message.into();
    eprintln!("{message}");
    let _ = rfd::MessageDialog::new()
        .set_title("Vertex Launcher")
        .set_description(message)
        .set_level(level)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}
