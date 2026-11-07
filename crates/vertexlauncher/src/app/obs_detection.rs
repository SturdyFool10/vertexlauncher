//! Detects a running OBS so streamer mode can be switched on while it runs.
//!
//! OBS is found by process name. A background thread lists processes every few seconds and
//! publishes whether OBS is running; the UI thread only reads an atomic flag. Nothing here is
//! saved: streamer mode follows OBS for as long as it is running.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Process names that identify OBS Studio, lowercased and without a `.exe` suffix.
const OBS_PROCESS_NAMES: &[&str] = &["obs", "obs64", "obs32", "obs-studio"];

/// Whether `process_name` is the OBS Studio executable. Matching is exact (ignoring case and
/// a `.exe` suffix), so helper processes such as `obs-ffmpeg-mux` do not count.
pub(super) fn is_obs_process_name(process_name: &str) -> bool {
    let name = process_name.trim().to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(name.as_str());
    OBS_PROCESS_NAMES.contains(&name)
}

/// Whether any line of `output` (one executable name per line, as `ps -axco comm` or the
/// `comm` files under `/proc` print them) is OBS.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn ps_output_contains_obs(output: &str) -> bool {
    output.lines().any(is_obs_process_name)
}

/// Whether any row of `tasklist /FO CSV /NH` output is OBS. The image name is the first
/// quoted field of each row.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn tasklist_output_contains_obs(output: &str) -> bool {
    output.lines().any(|line| {
        line.trim()
            .strip_prefix('"')
            .and_then(|rest| rest.split('"').next())
            .is_some_and(is_obs_process_name)
    })
}

/// How long the host process listing may take before it is given up on.
#[cfg(target_os = "linux")]
const HOST_LISTING_TIMEOUT: Duration = Duration::from_secs(3);

/// The only thing run on the host: prints the executable name of every process.
#[cfg(target_os = "linux")]
const HOST_LISTING_SCRIPT: &str = "cat /proc/[0-9]*/comm 2>/dev/null";

#[cfg(target_os = "linux")]
fn obs_is_running() -> bool {
    let in_sandbox = Path::new("/.flatpak-info").exists();
    obs_in_local_proc() || (in_sandbox && obs_on_flatpak_host(HOST_LISTING_TIMEOUT))
}

/// OBS among the processes this one can see. Inside a Flatpak sandbox that is only the
/// sandbox's own processes, so OBS on the host is not visible here.
#[cfg(target_os = "linux")]
fn obs_in_local_proc() -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|entry| {
        let file_name = entry.file_name();
        let is_pid = file_name
            .to_str()
            .is_some_and(|name| !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit()));
        is_pid
            && fs::read_to_string(entry.path().join("comm"))
                .is_ok_and(|comm| is_obs_process_name(comm.as_str()))
    })
}

/// OBS among the host's processes, asked through `flatpak-spawn`, which runs a command outside
/// the Flatpak sandbox. The Flatpak build is allowed to do that with its
/// `--talk-name=org.freedesktop.Flatpak` permission, which it also needs to open links and
/// folders. If the permission has been revoked the command fails and OBS is simply not found.
///
/// The command runs `sh` with a fixed script that only reads each process's `comm` (its
/// executable name); nothing user-controlled reaches the host.
#[cfg(target_os = "linux")]
fn obs_on_flatpak_host(timeout: Duration) -> bool {
    host_listing_contains_obs(
        "flatpak-spawn",
        &["--host", "sh", "-c", HOST_LISTING_SCRIPT],
        timeout,
    )
}

/// Runs `program` with `args` and checks the process names it prints for OBS.
#[cfg(target_os = "linux")]
fn host_listing_contains_obs(program: &str, args: &[&str], timeout: Duration) -> bool {
    let mut command = std::process::Command::new(program);
    command.args(args);
    match command_stdout_with_timeout(command, timeout) {
        Some(listing) => ps_output_contains_obs(&listing),
        None => {
            static REPORTED: AtomicBool = AtomicBool::new(false);
            if !REPORTED.swap(true, Ordering::Relaxed) {
                tracing::info!(
                    target: "vertexlauncher/obs_detection",
                    "Could not list host processes from the Flatpak sandbox, so OBS cannot be \
                     detected. This needs the org.freedesktop.Flatpak permission."
                );
            }
            false
        }
    }
}

/// Runs `command` and returns its standard output if it exits successfully within `timeout`;
/// a command that hangs is killed.
#[cfg(target_os = "linux")]
fn command_stdout_with_timeout(
    mut command: std::process::Command,
    timeout: Duration,
) -> Option<String> {
    use std::io::Read;
    use std::process::Stdio;

    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Read on another thread so a large listing cannot fill the pipe and block the child. Its
    // result comes back over a channel and is never waited on for long: a stray process that
    // inherited the pipe can hold it open long after the command itself is gone.
    let mut stdout = child.stdout.take()?;
    let (listing_tx, listing_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        let _ = listing_tx.send(String::from_utf8_lossy(&bytes).into_owned());
    });
    let started = Instant::now();
    let finished_successfully = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    if !finished_successfully {
        return None;
    }
    listing_rx.recv_timeout(Duration::from_secs(1)).ok()
}

#[cfg(target_os = "macos")]
fn obs_is_running() -> bool {
    std::process::Command::new("ps")
        .args(["-axco", "comm"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| ps_output_contains_obs(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(target_os = "windows")]
fn obs_is_running() -> bool {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| {
            tasklist_output_contains_obs(&String::from_utf8_lossy(&output.stdout))
        })
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn obs_is_running() -> bool {
    false
}

/// A change in whether OBS is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ObsChange {
    Started,
    Stopped,
}

/// Watches for OBS on a background thread.
pub(super) struct ObsWatcher {
    enabled: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    seen_running: bool,
}

impl ObsWatcher {
    /// Starts the polling thread, scanning from the start if `enabled`.
    pub(super) fn spawn(enabled: bool) -> Self {
        let enabled = Arc::new(AtomicBool::new(enabled));
        let running = Arc::new(AtomicBool::new(false));
        let thread_enabled = Arc::clone(&enabled);
        let thread_running = Arc::clone(&running);
        let spawned = std::thread::Builder::new()
            .name("obs-detection".to_owned())
            .spawn(move || {
                loop {
                    let found = thread_enabled.load(Ordering::Relaxed) && obs_is_running();
                    thread_running.store(found, Ordering::Relaxed);
                    std::thread::sleep(POLL_INTERVAL);
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(
                target: "vertexlauncher/obs_detection",
                error = %error,
                "Could not start the OBS detection thread."
            );
        }
        Self {
            enabled,
            running,
            seen_running: false,
        }
    }

    /// Turns scanning on or off. While off, OBS is treated as not running.
    pub(super) fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        if !enabled {
            self.running.store(false, Ordering::Relaxed);
        }
    }

    /// Whether OBS was running as of the last [`ObsWatcher::poll_change`].
    pub(super) fn is_active(&self) -> bool {
        self.seen_running
    }

    /// Reports OBS starting or stopping since the last call.
    pub(super) fn poll_change(&mut self) -> Option<ObsChange> {
        let running = self.running.load(Ordering::Relaxed);
        let change = match (self.seen_running, running) {
            (false, true) => Some(ObsChange::Started),
            (true, false) => Some(ObsChange::Stopped),
            _ => None,
        };
        self.seen_running = running;
        change
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obs_names_match_across_platforms() {
        for name in [
            "obs",
            "OBS",
            "obs64.exe",
            "OBS64.EXE",
            "obs32.exe",
            "obs-studio\n",
        ] {
            assert!(is_obs_process_name(name), "{name:?} should be OBS");
        }
    }

    #[test]
    fn obs_helpers_and_lookalikes_do_not_match() {
        for name in [
            "",
            "obs-ffmpeg-mux",
            "obs-browser-page",
            "obsidian",
            "robs",
            "jobs",
            "vertexlauncher",
        ] {
            assert!(!is_obs_process_name(name), "{name:?} should not be OBS");
        }
    }

    #[test]
    fn ps_output_is_scanned_line_by_line() {
        assert!(ps_output_contains_obs("launchd\nFinder\nOBS\nSafari\n"));
        assert!(!ps_output_contains_obs("launchd\nFinder\nObsidian\n"));
    }

    #[test]
    fn tasklist_csv_uses_the_first_field() {
        let running = "\"System\",\"4\",\"Services\",\"0\",\"20 K\"\r\n\
                       \"obs64.exe\",\"1234\",\"Console\",\"1\",\"250,000 K\"\r\n";
        assert!(tasklist_output_contains_obs(running));
        // The name must be the image name, not some other column.
        let other_column = "\"notepad.exe\",\"99\",\"obs64.exe\",\"1\",\"1 K\"\r\n";
        assert!(!tasklist_output_contains_obs(other_column));
    }

    /// Runs `script` in `sh` as the stand-in for `flatpak-spawn --host sh -c ...`, so the tests
    /// never execute a freshly written file (which can race with other threads forking).
    #[cfg(target_os = "linux")]
    fn host_lists(script: &str, timeout: Duration) -> bool {
        host_listing_contains_obs("sh", &["-c", script], timeout)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sandboxed_launcher_finds_obs_in_the_hosts_process_list() {
        let five = Duration::from_secs(5);
        assert!(host_lists("printf 'systemd\\nobs\\nbash\\n'", five));
        assert!(
            !host_lists("printf 'systemd\\nobsidian\\n'", five),
            "lookalike names do not count"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_or_failing_host_access_means_not_found() {
        let five = Duration::from_secs(5);
        // The permission revoked: flatpak-spawn exists but refuses.
        assert!(!host_lists("echo obs; exit 1", five));
        // No flatpak-spawn at all.
        assert!(!host_listing_contains_obs(
            "/nonexistent/flatpak-spawn",
            &[],
            five
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_hung_host_listing_is_given_up_on_quickly() {
        let started = Instant::now();
        let found = host_lists("sleep 30", Duration::from_millis(200));
        assert!(!found);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_large_host_listing_does_not_block_the_command() {
        // Far more than a pipe buffer holds, with OBS at the very end.
        let script =
            "i=0; while [ $i -lt 20000 ]; do echo some-process-name; i=$((i+1)); done; echo obs";
        assert!(host_lists(script, Duration::from_secs(20)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_host_command_is_the_fixed_listing_script() {
        // Guards what runs outside the sandbox: only `comm` names are read.
        assert_eq!(HOST_LISTING_SCRIPT, "cat /proc/[0-9]*/comm 2>/dev/null");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_scan_finds_a_process_named_obs() {
        let dir = std::env::temp_dir().join(format!("vertex-obs-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let fake_obs = dir.join("obs");
        let sleep = ["/bin/sleep", "/usr/bin/sleep"]
            .into_iter()
            .find(|path| Path::new(path).exists())
            .expect("a sleep binary");
        fs::copy(sleep, &fake_obs).expect("copy sleep as obs");
        // Executing a file that was just written can briefly fail with "text file busy" when
        // another thread forks at the same moment, so retry that.
        let mut child = (0..50)
            .find_map(
                |_| match std::process::Command::new(&fake_obs).arg("30").spawn() {
                    Ok(child) => Some(child),
                    Err(error) if error.raw_os_error() == Some(26) => {
                        std::thread::sleep(Duration::from_millis(20));
                        None
                    }
                    Err(error) => panic!("spawn fake obs: {error}"),
                },
            )
            .expect("spawn fake obs");
        let found = obs_is_running();
        let _ = child.kill();
        let _ = child.wait();
        let _ = fs::remove_dir_all(&dir);
        assert!(found, "a process named `obs` should be detected");
    }

    fn watcher(running: bool) -> ObsWatcher {
        ObsWatcher {
            enabled: Arc::new(AtomicBool::new(true)),
            running: Arc::new(AtomicBool::new(running)),
            seen_running: false,
        }
    }

    #[test]
    fn changes_are_reported_once_each_way() {
        let mut watcher = watcher(false);
        assert_eq!(watcher.poll_change(), None);
        assert!(!watcher.is_active());

        watcher.running.store(true, Ordering::Relaxed);
        assert_eq!(watcher.poll_change(), Some(ObsChange::Started));
        assert!(watcher.is_active());
        assert_eq!(watcher.poll_change(), None, "still running: no repeat");
        assert!(watcher.is_active());

        watcher.running.store(false, Ordering::Relaxed);
        assert_eq!(watcher.poll_change(), Some(ObsChange::Stopped));
        assert!(!watcher.is_active(), "streamer mode ends with OBS");
        assert_eq!(watcher.poll_change(), None);

        watcher.running.store(true, Ordering::Relaxed);
        assert_eq!(watcher.poll_change(), Some(ObsChange::Started));
    }

    #[test]
    fn disabling_detection_ends_the_active_state() {
        let mut watcher = watcher(true);
        assert_eq!(watcher.poll_change(), Some(ObsChange::Started));
        watcher.set_enabled(false);
        assert_eq!(watcher.poll_change(), Some(ObsChange::Stopped));
        assert!(!watcher.is_active());

        // Re-enabling while OBS is open: the scan thread finds it again.
        watcher.set_enabled(true);
        watcher.running.store(true, Ordering::Relaxed);
        assert_eq!(watcher.poll_change(), Some(ObsChange::Started));
    }
}
