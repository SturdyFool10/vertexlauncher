use vertex_i18n::tr;
use std::{
    io,
    path::Path,
    process::{Command, Stdio},
};

pub fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        return std::process::Command::new("rundll32")
            .arg("url.dll,FileProtocolHandler")
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string());
    }

    #[cfg(target_os = "macos")]
    {
        return std::process::Command::new("open")
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string());
    }

    #[cfg(target_os = "linux")]
    {
        if Path::new("/.flatpak-info").exists() {
            return std::process::Command::new("flatpak-spawn")
                .args(["--host", "xdg-open", url])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map(|_| ())
                .map_err(|e| e.to_string());
        }
        for program in ["xdg-open", "gio open", "kde-open"] {
            let mut parts = program.split_whitespace();
            let cmd = parts.next().unwrap();
            let mut command = std::process::Command::new(cmd);
            command
                .args(parts)
                .arg(url)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            if let Ok(_) = command.spawn() {
                return Ok(());
            }
        }
        return Err(tr!("no supported URL opener found").into_owned());
    }

    #[allow(unreachable_code)]
    Err(tr!("opening URLs is not supported on this platform").into_owned())
}

pub fn open_in_file_manager(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(tr!("path does not exist: {path}", path = path.display()).into_owned());
    }

    #[cfg(target_os = "windows")]
    {
        return spawn_first_available(path, &[ProgramLaunch::new("explorer", &[])]);
    }

    #[cfg(target_os = "macos")]
    {
        return spawn_first_available(path, &[ProgramLaunch::new("open", &[])]);
    }

    #[cfg(target_os = "linux")]
    {
        if Path::new("/.flatpak-info").exists() {
            return spawn_first_available(
                path,
                &[ProgramLaunch::new("flatpak-spawn", &["--host", "xdg-open"])],
            );
        }
        return spawn_first_available(
            path,
            &[
                ProgramLaunch::new("xdg-open", &[]),
                ProgramLaunch::new("gio", &["open"]),
                ProgramLaunch::new("kioclient5", &["exec"]),
                ProgramLaunch::new("kioclient", &["exec"]),
            ],
        );
    }

    #[allow(unreachable_code)]
    Err(tr!("opening folders is not supported on this platform").into_owned())
}

fn spawn_first_available(path: &Path, candidates: &[ProgramLaunch<'_>]) -> Result<(), String> {
    let mut unavailable = Vec::new();
    let mut failures = Vec::new();

    for candidate in candidates {
        match spawn_program(path, candidate) {
            Ok(()) => return Ok(()),
            Err(LaunchOutcome::Unavailable) => unavailable.push(candidate.program),
            Err(LaunchOutcome::Failed(error)) => {
                failures.push(tr!("{program}: {error}", program = candidate.program, error = error).into_owned())
            }
        }
    }

    if !failures.is_empty() {
        return Err(failures.join(" | "));
    }
    if !unavailable.is_empty() {
        return Err(tr!("no supported file manager launcher was found ({launchers})", launchers = unavailable.join(", ")).into_owned());
    }
    Err(tr!("no supported file manager launcher succeeded").into_owned())
}

fn spawn_program(path: &Path, candidate: &ProgramLaunch<'_>) -> Result<(), LaunchOutcome> {
    let mut command = Command::new(candidate.program);
    command
        .args(candidate.args)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.spawn() {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(LaunchOutcome::Unavailable),
        Err(error) => Err(LaunchOutcome::Failed(error)),
    }
}

enum LaunchOutcome {
    Unavailable,
    Failed(io::Error),
}

struct ProgramLaunch<'a> {
    program: &'a str,
    args: &'a [&'a str],
}

impl<'a> ProgramLaunch<'a> {
    const fn new(program: &'a str, args: &'a [&'a str]) -> Self {
        Self { program, args }
    }
}
