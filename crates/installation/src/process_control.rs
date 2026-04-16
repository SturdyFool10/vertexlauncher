use super::*;

struct RunningInstanceProcess {
    child: Child,
    account_key: Option<String>,
}

static RUNNING_INSTANCE_PROCESSES: OnceLock<Mutex<HashMap<String, Vec<RunningInstanceProcess>>>> =
    OnceLock::new();
static FINISHED_INSTANCE_PROCESSES: OnceLock<Mutex<Vec<FinishedInstanceProcess>>> = OnceLock::new();

fn process_registry() -> &'static Mutex<HashMap<String, Vec<RunningInstanceProcess>>> {
    RUNNING_INSTANCE_PROCESSES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn finished_process_queue() -> &'static Mutex<Vec<FinishedInstanceProcess>> {
    FINISHED_INSTANCE_PROCESSES.get_or_init(|| Mutex::new(Vec::new()))
}

/// Maximum number of finished-process entries held between polls.
/// Under normal conditions the UI drains this every frame via
/// `take_finished_instance_processes`. The cap protects against the
/// (unlikely) case where polling is suspended for a long time while
/// many short-lived instances complete.
const MAX_FINISHED_PROCESS_BACKLOG: usize = 256;

fn push_finished_instance_process(process: FinishedInstanceProcess) {
    if let Ok(mut finished) = finished_process_queue().lock() {
        if finished.len() < MAX_FINISHED_PROCESS_BACKLOG {
            finished.push(process);
        }
    }
}

fn finished_instance_process(
    instance_root: &str,
    process: RunningInstanceProcess,
    status: ExitStatus,
) -> FinishedInstanceProcess {
    let pid = process.child.id();
    FinishedInstanceProcess {
        instance_root: instance_root.to_owned(),
        account_key: process.account_key,
        pid,
        exit_code: status.code(),
    }
}

pub fn take_finished_instance_processes() -> Vec<FinishedInstanceProcess> {
    if let Ok(mut processes) = process_registry().lock() {
        prune_finished_processes(&mut processes);
    }
    match finished_process_queue().lock() {
        Ok(mut finished) => std::mem::take(&mut *finished),
        Err(_) => Vec::new(),
    }
}

pub fn launch_instance(request: &LaunchRequest) -> Result<LaunchResult, InstallationError> {
    let instance_root = fs_canonicalize(request.instance_root.as_path())
        .unwrap_or_else(|_| request.instance_root.clone());
    let instance_key = normalize_path_key(instance_root.as_path());
    let requested_account = normalize_account_key(request.account_key.as_deref());
    if let Ok(mut processes) = process_registry().lock() {
        prune_finished_processes(&mut processes);
        if let Some(instance_processes) = processes.get_mut(instance_key.as_str()) {
            let same_account_already_running = instance_processes.iter_mut().find_map(|process| {
                if !matches!(process.child.try_wait(), Ok(None)) {
                    return None;
                }
                let matches_account = match requested_account.as_deref() {
                    Some(account) => process
                        .account_key
                        .as_deref()
                        .is_some_and(|running| running == account),
                    None => process.account_key.is_none(),
                };
                if matches_account {
                    Some(process.child.id())
                } else {
                    None
                }
            });
            if let Some(pid) = same_account_already_running {
                return Err(InstallationError::InstanceAlreadyRunning {
                    instance_root: instance_key,
                    pid,
                });
            }
        }
        if let Some(account) = requested_account.as_deref() {
            for (running_instance_root, instance_processes) in processes.iter_mut() {
                if running_instance_root == &instance_key {
                    continue;
                }
                for process in instance_processes {
                    if process
                        .account_key
                        .as_deref()
                        .is_some_and(|in_use| in_use == account)
                        && let Ok(None) = process.child.try_wait()
                    {
                        return Err(InstallationError::AccountAlreadyInUse {
                            account: request
                                .account_key
                                .clone()
                                .unwrap_or_else(|| account.to_owned()),
                            instance_root: running_instance_root.clone(),
                        });
                    }
                }
            }
        }
    }

    let java_resolution = resolve_java_executable(request.java_executable.as_deref());
    let java = java_resolution.executable.as_str();
    let (profile_id, profile_path) = resolve_launch_profile_path(
        instance_root.as_path(),
        request.game_version.as_str(),
        request.modloader.as_str(),
        request.modloader_version.as_deref(),
    )?;
    let profile_chain = load_profile_chain(instance_root.as_path(), profile_path.as_path())?;
    let main_class = resolve_main_class(&profile_chain).ok_or_else(|| {
        InstallationError::LaunchMainClassMissing {
            profile_id: profile_id.clone(),
        }
    })?;
    let natives_dir =
        prepare_natives_dir(instance_root.as_path(), profile_id.as_str(), &profile_chain)?;
    let classpath_entries = build_classpath_entries(
        instance_root.as_path(),
        profile_id.as_str(),
        request.game_version.as_str(),
        main_class.as_str(),
        &profile_chain,
    )?;
    let classpath = prepare_launch_classpath(
        instance_root.as_path(),
        profile_id.as_str(),
        &classpath_entries,
    )?;
    let (mut launch_log_file, launch_log_path) = prepare_launch_log_file(instance_root.as_path())?;
    let launch_log_for_error = display_user_path(launch_log_path.as_path());
    let _ = writeln!(
        launch_log_file,
        "[vertexlauncher] Launching Minecraft {} with profile {} in {}",
        request.game_version,
        profile_id,
        display_user_path(instance_root.as_path())
    );
    let _ = writeln!(
        launch_log_file,
        "[vertexlauncher] Java resolution: {}",
        format_java_executable_resolution(&java_resolution)
    );
    log_java_executable_resolution(
        "vertexlauncher/installation/launch",
        "minecraft launch",
        &java_resolution,
    );
    let stderr_log = launch_log_file.try_clone()?;
    let mut command_log = launch_log_file.try_clone()?;

    let mut command = Command::new(java);
    command
        .current_dir(instance_root.as_path())
        .stdin(Stdio::null())
        .stdout(Stdio::from(launch_log_file))
        .stderr(Stdio::from(stderr_log));
    apply_linux_opengl_driver_env(
        &mut command,
        request.game_version.as_str(),
        request.linux_set_opengl_driver,
        request.linux_use_zink_driver,
    );
    apply_extra_environment_vars(
        &mut command,
        request.extra_env_vars.as_deref(),
        &mut command_log,
    );

    command.arg(format!("-Xmx{}M", request.max_memory_mib.max(512)));
    let user_jvm_args = parse_user_args(request.extra_jvm_args.as_deref());
    for arg in user_jvm_args {
        command.arg(arg);
    }

    let launch_context = build_launch_context(
        instance_root.as_path(),
        request.game_version.as_str(),
        profile_id.as_str(),
        resolve_assets_index_name(&profile_chain, request.game_version.as_str()),
        classpath.resolved.as_str(),
        natives_dir.as_path(),
        request.player_name.as_deref(),
        request.player_uuid.as_deref(),
        request.auth_access_token.as_deref(),
        request.auth_xuid.as_deref(),
        request.auth_user_type.as_deref(),
        request.quick_play_singleplayer.as_deref(),
        request.quick_play_multiplayer.as_deref(),
    );

    let mut jvm_args = collect_jvm_arguments(&profile_chain, &launch_context);
    if should_use_environment_classpath() {
        command.env("CLASSPATH", classpath.resolved.as_str());
        strip_explicit_classpath_args(&mut jvm_args);
    } else if let Some(argfile) = classpath.argfile.as_ref() {
        strip_explicit_classpath_args(&mut jvm_args);
        command.arg(format!("@{}", display_user_path(argfile)));
    } else if !has_explicit_classpath_args(&jvm_args) {
        jvm_args.push("-cp".to_owned());
        jvm_args.push(classpath.resolved.clone());
    }
    for arg in jvm_args {
        command.arg(arg);
    }

    command.arg(main_class);

    let game_args = collect_game_arguments(&profile_chain, &launch_context);
    for arg in game_args {
        command.arg(arg);
    }

    let raw_args: Vec<std::borrow::Cow<str>> = command
        .get_args()
        .map(|arg| arg.to_string_lossy())
        .collect();
    let mut command_args: Vec<String> = Vec::with_capacity(raw_args.len());
    let mut redact_next = false;
    for arg in &raw_args {
        if redact_next {
            command_args.push("[redacted]".to_owned());
            redact_next = false;
        } else {
            command_args.push(quote_command_arg(arg.as_ref()));
            if arg.as_ref() == "--accessToken" {
                redact_next = true;
            }
        }
    }
    let _ = writeln!(
        command_log,
        "[vertexlauncher] Command: {} {}",
        quote_command_arg(java),
        command_args.join(" ")
    );

    let mut child = spawn_command_child(&mut command, &java_resolution)?;
    thread::sleep(Duration::from_millis(1200));
    if let Some(status) = child.try_wait()? {
        return Err(InstallationError::LaunchExitedImmediately {
            status: status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "terminated by signal".to_owned()),
            log_path: PathBuf::from(launch_log_for_error),
        });
    }
    let pid = child.id();
    if let Ok(mut processes) = process_registry().lock() {
        processes
            .entry(instance_key.clone())
            .or_default()
            .push(RunningInstanceProcess {
                child,
                account_key: requested_account,
            });
    }
    Ok(LaunchResult {
        pid,
        profile_id,
        launch_log_path,
    })
}

#[cfg(target_os = "linux")]
fn apply_linux_opengl_driver_env(
    command: &mut Command,
    game_version: &str,
    set_linux_opengl_driver: bool,
    use_zink_driver: bool,
) {
    if !set_linux_opengl_driver {
        return;
    }
    if should_skip_linux_opengl_driver_env_for_vulkan_capable_version(game_version) {
        tracing::info!(
            target: "vertexlauncher/installation/launch",
            game_version,
            "Skipping Linux OpenGL driver environment overrides for a Vulkan-capable Minecraft version."
        );
        return;
    }

    command.env_remove("MESA_LOADER_DRIVER_OVERRIDE");
    command.env_remove("GALLIUM_DRIVER");

    if use_zink_driver {
        command.env("MESA_LOADER_DRIVER_OVERRIDE", "zink");
        command.env("GALLIUM_DRIVER", "zink");
    }
}

#[cfg(target_os = "linux")]
fn should_skip_linux_opengl_driver_env_for_vulkan_capable_version(game_version: &str) -> bool {
    let Some(version) = parse_vulkan_capable_version_key(game_version) else {
        return false;
    };
    version.major > 26 || (version.major == 26 && (version.is_snapshot || version.minor >= 2))
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
struct VulkanCapableVersionKey {
    major: u32,
    minor: u32,
    is_snapshot: bool,
}

#[cfg(target_os = "linux")]
fn parse_vulkan_capable_version_key(game_version: &str) -> Option<VulkanCapableVersionKey> {
    let trimmed = game_version.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some((year, week)) = trimmed.split_once('w') {
        let major = parse_ascii_u32_prefix(year)?;
        if major >= 26 && parse_ascii_u32_prefix(week).is_some() {
            return Some(VulkanCapableVersionKey {
                major,
                minor: 0,
                is_snapshot: true,
            });
        }
    }

    let mut parts = trimmed.split(['.', '-']);
    let major = parts.next().and_then(parse_ascii_u32_prefix)?;
    let minor = parts.next().and_then(parse_ascii_u32_prefix).unwrap_or(0);
    Some(VulkanCapableVersionKey {
        major,
        minor,
        is_snapshot: false,
    })
}

#[cfg(target_os = "linux")]
fn parse_ascii_u32_prefix(value: &str) -> Option<u32> {
    let digits_len = value
        .as_bytes()
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digits_len == 0 {
        return None;
    }
    value.get(..digits_len)?.parse().ok()
}

#[cfg(not(target_os = "linux"))]
fn apply_linux_opengl_driver_env(
    _command: &mut Command,
    _game_version: &str,
    _set_linux_opengl_driver: bool,
    _use_zink_driver: bool,
) {
}

fn apply_extra_environment_vars(
    command: &mut Command,
    raw_env_vars: Option<&str>,
    command_log: &mut impl Write,
) {
    let Some(raw_env_vars) = raw_env_vars else {
        return;
    };

    for (line_index, raw_line) in raw_env_vars.lines().enumerate() {
        let line_number = line_index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            let _ = writeln!(
                command_log,
                "[vertexlauncher] Ignored environment override on line {line_number}: expected KEY=value"
            );
            tracing::warn!(
                target: "vertexlauncher/installation/launch",
                line_number,
                "Ignored instance environment override without KEY=value syntax."
            );
            continue;
        };
        let key = raw_key.trim();
        let value = raw_value.trim();
        if !is_valid_environment_key(key) || value.contains('\0') {
            let _ = writeln!(
                command_log,
                "[vertexlauncher] Ignored environment override on line {line_number}: invalid key or value"
            );
            tracing::warn!(
                target: "vertexlauncher/installation/launch",
                line_number,
                key,
                "Ignored invalid instance environment override."
            );
            continue;
        }
        command.env(key, value);
        let _ = writeln!(
            command_log,
            "[vertexlauncher] Applied environment override: {key}=<redacted>"
        );
        tracing::info!(
            target: "vertexlauncher/installation/launch",
            key,
            "Applied instance environment override to Minecraft process."
        );
    }
}

fn is_valid_environment_key(key: &str) -> bool {
    !key.is_empty() && !key.contains('=') && !key.contains('\0')
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::should_skip_linux_opengl_driver_env_for_vulkan_capable_version;

    #[test]
    fn skips_linux_opengl_driver_env_for_vulkan_capable_snapshots() {
        assert!(should_skip_linux_opengl_driver_env_for_vulkan_capable_version("26w15a"));
        assert!(should_skip_linux_opengl_driver_env_for_vulkan_capable_version("27w01a"));
    }

    #[test]
    fn skips_linux_opengl_driver_env_for_26_2_and_later_releases() {
        assert!(!should_skip_linux_opengl_driver_env_for_vulkan_capable_version("26.1"));
        assert!(should_skip_linux_opengl_driver_env_for_vulkan_capable_version("26.2"));
        assert!(should_skip_linux_opengl_driver_env_for_vulkan_capable_version("26.2.1"));
        assert!(should_skip_linux_opengl_driver_env_for_vulkan_capable_version("27.0"));
    }
}

pub fn stop_running_instance(instance_root: &Path) -> bool {
    let key = instance_process_key(instance_root);
    let Ok(mut processes) = process_registry().lock() else {
        return false;
    };
    let Some(mut instance_processes) = processes.remove(key.as_str()) else {
        return false;
    };
    let mut stopped = false;
    for process in &mut instance_processes {
        if matches!(process.child.try_wait(), Ok(None)) {
            let _ = process.child.kill();
            let _ = process.child.wait();
            stopped = true;
        }
    }
    stopped
}

pub fn stop_running_instance_for_account(instance_root: &Path, account_key: &str) -> bool {
    let Some(account) = normalize_account_key(Some(account_key)) else {
        return false;
    };
    let key = instance_process_key(instance_root);
    let Ok(mut processes) = process_registry().lock() else {
        return false;
    };
    let mut removed_any = false;
    let mut emptied = false;
    if let Some(instance_processes) = processes.get_mut(key.as_str()) {
        let mut index = 0usize;
        while index < instance_processes.len() {
            let matches_account = instance_processes[index]
                .account_key
                .as_deref()
                .is_some_and(|value| value == account);
            if matches_account {
                let mut process = instance_processes.remove(index);
                if matches!(process.child.try_wait(), Ok(None)) {
                    let _ = process.child.kill();
                    let _ = process.child.wait();
                    removed_any = true;
                }
                continue;
            }
            index += 1;
        }
        emptied = instance_processes.is_empty();
    }
    if emptied {
        let _ = processes.remove(key.as_str());
    }
    removed_any
}

pub fn is_instance_running(instance_root: &Path) -> bool {
    let key = instance_process_key(instance_root);
    let Ok(mut processes) = process_registry().lock() else {
        return false;
    };
    prune_finished_processes(&mut processes);
    processes
        .get_mut(key.as_str())
        .is_some_and(|instance_processes| {
            instance_processes
                .iter_mut()
                .any(|process| matches!(process.child.try_wait(), Ok(None)))
        })
}

pub fn is_instance_running_for_account(instance_root: &Path, account_key: &str) -> bool {
    let Some(account) = normalize_account_key(Some(account_key)) else {
        return false;
    };
    let key = instance_process_key(instance_root);
    let Ok(mut processes) = process_registry().lock() else {
        return false;
    };
    prune_finished_processes(&mut processes);
    processes
        .get_mut(key.as_str())
        .is_some_and(|instance_processes| {
            instance_processes.iter_mut().any(|process| {
                process
                    .account_key
                    .as_deref()
                    .is_some_and(|value| value == account)
                    && matches!(process.child.try_wait(), Ok(None))
            })
        })
}

pub fn running_instance_for_account(account_key: &str) -> Option<String> {
    let account = normalize_account_key(Some(account_key))?;
    let Ok(mut processes) = process_registry().lock() else {
        return None;
    };
    prune_finished_processes(&mut processes);
    processes
        .iter_mut()
        .find_map(|(instance_root, instance_processes)| {
            if instance_processes.iter_mut().any(|process| {
                process
                    .account_key
                    .as_deref()
                    .is_some_and(|value| value == account)
                    && matches!(process.child.try_wait(), Ok(None))
            }) {
                Some(instance_root.clone())
            } else {
                None
            }
        })
}

pub fn running_account_for_instance(instance_root: &Path) -> Option<String> {
    let key = instance_process_key(instance_root);
    let Ok(mut processes) = process_registry().lock() else {
        return None;
    };
    prune_finished_processes(&mut processes);
    processes
        .get_mut(key.as_str())?
        .iter_mut()
        .find_map(|process| {
            if matches!(process.child.try_wait(), Ok(None)) {
                process.account_key.clone()
            } else {
                None
            }
        })
}

pub fn running_instance_roots() -> Vec<String> {
    let Ok(mut processes) = process_registry().lock() else {
        return Vec::new();
    };
    prune_finished_processes(&mut processes);
    processes.keys().cloned().collect()
}

fn prune_finished_processes(processes: &mut HashMap<String, Vec<RunningInstanceProcess>>) {
    processes.retain(|instance_root, instance_processes| {
        let mut index = 0usize;
        while index < instance_processes.len() {
            match instance_processes[index].child.try_wait() {
                Ok(None) => index += 1,
                Ok(Some(status)) => {
                    let process = instance_processes.remove(index);
                    push_finished_instance_process(finished_instance_process(
                        instance_root,
                        process,
                        status,
                    ));
                }
                Err(_) => {
                    let _ = instance_processes.remove(index);
                }
            }
        }
        !instance_processes.is_empty()
    });
}

fn instance_process_key(instance_root: &Path) -> String {
    let normalized = fs_canonicalize(instance_root).unwrap_or_else(|_| instance_root.to_path_buf());
    display_user_path(normalized.as_path())
}

fn normalize_account_key(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}
