use vertex_i18n::tr;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use crate::app::tokio_runtime;

use super::instance_screen_state::{InstanceScreenState, MoveInstanceProgress, MoveInstanceResult};

struct MoveChildReport {
    file_rel_path: String,
    bytes_copied: u64,
    done: bool,
    error: Option<String>,
}

fn collect_files_recursively(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursively(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

pub(super) fn request_move_instance(
    state: &mut InstanceScreenState,
    source_root: PathBuf,
    dest_root: PathBuf,
) {
    if state.move_instance_in_flight {
        return;
    }
    let progress_tx = state.move_instance_progress.sender();
    let results_tx = state.move_instance_results.sender();

    state.move_instance_in_flight = true;
    state.move_instance_latest_progress = None;
    state.move_instance_completion_message = None;
    state.move_instance_completion_failed = false;
    state.move_instance_last_layout_log_at = None;
    state.move_instance_pending_result = None;
    state.move_instance_progress_visible_until = Some(Instant::now() + Duration::from_secs(2));

    let _ = tokio_runtime::spawn_detached(async move {
        // Create the destination root directory
        if let Err(err) = launcher_runtime::fs::create_dir_all(&dest_root).await {
            tracing::warn!(
                target: "vertexlauncher/move_instance",
                path = %dest_root.display(),
                error = %err,
                "failed to create destination root directory"
            );
            if let Err(send_err) = results_tx.send(MoveInstanceResult::Failed {
                reason: tr!("Could not create destination folder: {err}", err = err).into_owned(),
            }) {
                tracing::warn!(
                    target: "vertexlauncher/move_instance",
                    error = %send_err,
                    "Failed to deliver move-instance failure result to UI."
                );
            }
            return;
        }

        // Walk the tree and total its size on a blocking thread; both are pure filesystem work.
        let scan_root = source_root.clone();
        let Ok((files, total_bytes)) = launcher_runtime::spawn_blocking(move || {
            let mut files: Vec<PathBuf> = Vec::new();
            collect_files_recursively(&scan_root, &mut files);
            let total_bytes: u64 = files
                .iter()
                .filter_map(|f| f.metadata().ok().map(|m| m.len()))
                .sum();
            (files, total_bytes)
        })
        .await
        else {
            let _ = results_tx.send(MoveInstanceResult::Failed {
                reason: tr!("Could not scan the instance folder.").into_owned(),
            });
            return;
        };

        if files.is_empty() {
            if let Err(send_err) = results_tx.send(MoveInstanceResult::Complete {
                dest_path: dest_root,
            }) {
                tracing::warn!(
                    target: "vertexlauncher/move_instance",
                    error = %send_err,
                    "Failed to deliver move-instance completion result to UI."
                );
            }
            return;
        }

        let total_files = files.len();
        let max_concurrent_copies = std::thread::available_parallelism()
            .map(|count| count.get().saturating_mul(2))
            .unwrap_or(8)
            .clamp(4, 32);
        let copy_permits = Arc::new(launcher_runtime::Semaphore::new(max_concurrent_copies));

        // One channel for all child reports back to the parent aggregator
        let (child_tx, child_rx) = mpsc::channel::<MoveChildReport>();

        // Spawn one child per file
        for source_path in files {
            let child_tx = child_tx.clone();
            let source_root = source_root.clone();
            let dest_root = dest_root.clone();
            let copy_permits = Arc::clone(&copy_permits);
            let _ = tokio_runtime::spawn_detached(async move {
                let rel = source_path
                    .strip_prefix(&source_root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| source_path.display().to_string());
                let Ok(_permit) = copy_permits.acquire_owned().await;

                let result = launcher_runtime::spawn_blocking({
                    let rel = rel.clone();
                    let child_tx = child_tx.clone();
                    move || {
                        let fail = |message: String| {
                            let _ = child_tx.send(MoveChildReport {
                                file_rel_path: rel.clone(),
                                bytes_copied: 0,
                                done: true,
                                error: Some(message),
                            });
                        };
                        let dest_path = dest_root.join(
                            source_path
                                .strip_prefix(&source_root)
                                .unwrap_or(source_path.as_path()),
                        );
                        if let Some(parent) = dest_path.parent() {
                            if let Err(err) = std::fs::create_dir_all(parent) {
                                tracing::warn!(
                                    target: "vertexlauncher/move_instance",
                                    path = %parent.display(),
                                    error = %err,
                                    "failed to create destination directory"
                                );
                                fail(tr!("Failed to create destination directory {parent}: {err}", parent = parent.display(), err = err).into_owned());
                                return;
                            }
                        }
                        let mut reader = match std::fs::File::open(&source_path) {
                            Ok(f) => f,
                            Err(err) => {
                                tracing::warn!(
                                    target: "vertexlauncher/move_instance",
                                    path = %source_path.display(),
                                    error = %err,
                                    "failed to open source file"
                                );
                                fail(tr!("Failed to open source file {source_path}: {err}", source_path = source_path.display(), err = err).into_owned());
                                return;
                            }
                        };
                        let mut writer = match std::fs::File::create(&dest_path) {
                            Ok(f) => f,
                            Err(err) => {
                                tracing::warn!(
                                    target: "vertexlauncher/move_instance",
                                    path = %dest_path.display(),
                                    error = %err,
                                    "failed to create destination file"
                                );
                                fail(tr!("Failed to create destination file {dest_path}: {err}", dest_path = dest_path.display(), err = err).into_owned());
                                return;
                            }
                        };
                        let mut buf = vec![0u8; 65536];
                        let mut bytes_copied = 0u64;
                        loop {
                            let n = match reader.read(&mut buf) {
                                Ok(n) => n,
                                Err(err) => {
                                    tracing::warn!(
                                        target: "vertexlauncher/move_instance",
                                        path = %source_path.display(),
                                        error = %err,
                                        "failed while reading source file"
                                    );
                                    fail(tr!("Failed while reading source file {source_path}: {err}", source_path = source_path.display(), err = err).into_owned());
                                    return;
                                }
                            };
                            if n == 0 {
                                break;
                            }
                            if let Err(err) = writer.write_all(&buf[..n]) {
                                tracing::warn!(
                                    target: "vertexlauncher/move_instance",
                                    path = %dest_path.display(),
                                    error = %err,
                                    "failed while writing destination file"
                                );
                                fail(tr!("Failed while writing destination file {dest_path}: {err}", dest_path = dest_path.display(), err = err).into_owned());
                                return;
                            }
                            bytes_copied += n as u64;
                            let _ = child_tx.send(MoveChildReport {
                                file_rel_path: rel.clone(),
                                bytes_copied,
                                done: false,
                                error: None,
                            });
                        }
                        let _ = child_tx.send(MoveChildReport {
                            file_rel_path: rel,
                            bytes_copied,
                            done: true,
                            error: None,
                        });
                    }
                })
                .await;
                if let Err(err) = result {
                    tracing::warn!(
                        target: "vertexlauncher/move_instance",
                        error = %err,
                        "move child task panicked"
                    );
                    let _ = child_tx.send(MoveChildReport {
                        file_rel_path: rel,
                        bytes_copied: 0,
                        done: true,
                        error: Some(tr!("Move worker panicked: {err}", err = err).into_owned()),
                    });
                }
            });
        }

        // Drop the original sender so the receiver eventually sees Disconnected
        drop(child_tx);

        // Aggregate reports and forward to UI
        let mut active_file_bytes: HashMap<String, u64> = HashMap::new();
        let mut aggregate_done = 0u64;
        let mut files_done = 0usize;
        let mut first_error: Option<String> = None;

        // Run the aggregation loop in a blocking task since recv() blocks
        let _ = launcher_runtime::spawn_blocking(move || {
            loop {
                match child_rx.recv() {
                    Ok(report) => {
                        if let Some(error) = report.error {
                            tracing::warn!(
                                target: "vertexlauncher/move_instance",
                                file_rel_path = report.file_rel_path.as_str(),
                                error = error.as_str(),
                                "move child reported failure"
                            );
                            if first_error.is_none() {
                                first_error = Some(error);
                            }
                            active_file_bytes.remove(report.file_rel_path.as_str());
                            continue;
                        }
                        let previous_bytes = active_file_bytes
                            .get(report.file_rel_path.as_str())
                            .copied()
                            .unwrap_or(0);
                        if report.bytes_copied > previous_bytes {
                            aggregate_done =
                                aggregate_done.saturating_add(report.bytes_copied - previous_bytes);
                        }

                        if report.done {
                            active_file_bytes.remove(report.file_rel_path.as_str());
                            files_done = files_done.saturating_add(1);
                        } else {
                            active_file_bytes
                                .insert(report.file_rel_path.clone(), report.bytes_copied);
                        }
                        let active_files: Vec<String> =
                            active_file_bytes.keys().take(3).cloned().collect();

                        let _ = progress_tx.send(MoveInstanceProgress {
                            total_bytes,
                            bytes_done: aggregate_done,
                            total_files,
                            files_done,
                            active_file_count: active_file_bytes.len(),
                            active_files,
                        });
                    }
                    Err(_) => break, // all children dropped their senders
                }
            }
            let final_result = if let Some(reason) = first_error {
                MoveInstanceResult::Failed { reason }
            } else {
                MoveInstanceResult::Complete {
                    dest_path: dest_root,
                }
            };
            if let Err(send_err) = results_tx.send(final_result) {
                tracing::warn!(
                    target: "vertexlauncher/move_instance",
                    error = %send_err,
                    "Failed to deliver move-instance final result to UI."
                );
            }
        })
        .await;
    });
}

pub(super) fn poll_move_instance_progress(state: &mut InstanceScreenState) {
    // Only the newest progress update matters.
    if let Some(progress) = state.move_instance_progress.drain().items.pop() {
        state.move_instance_latest_progress = Some(progress);
    }
}

pub(super) fn poll_move_instance_results(
    state: &mut InstanceScreenState,
) -> Option<MoveInstanceResult> {
    let now = Instant::now();
    let visible_until = state.move_instance_progress_visible_until.unwrap_or(now);
    if state.move_instance_pending_result.is_some() && now >= visible_until {
        let result = state.move_instance_pending_result.take();
        state.move_instance_in_flight = false;
        state.move_instance_progress_visible_until = None;
        state.move_instance_progress.reset();
        state.move_instance_results.reset();
        return result;
    }

    let drained = state.move_instance_results.drain();
    let mut channel_disconnected = drained.disconnected;
    let result: Option<MoveInstanceResult> = drained.items.into_iter().next();

    if result.is_some() {
        if now < visible_until {
            state.move_instance_pending_result = result;
            return None;
        }
        state.move_instance_in_flight = false;
        state.move_instance_pending_result = None;
        state.move_instance_progress_visible_until = None;
        channel_disconnected = true;
    }

    if channel_disconnected {
        state.move_instance_progress.reset();
        state.move_instance_results.reset();
        if state.move_instance_in_flight {
            // channel closed without a result — treat as unexpected failure
            state.move_instance_in_flight = false;
            state.move_instance_pending_result = None;
            state.move_instance_progress_visible_until = None;
        }
    }

    result
}
