//! A thread that watches the UI from outside it, so a launcher that never gets going still says where it stopped.
//!
//! The UI thread only ever stores a step name and a few timestamps here ([`step`], [`frame_begin`], [`frame_end`],
//! [`engine_frame`]); this thread reads them and logs when something stops moving. It uses a plain OS thread and no
//! executor, so it keeps working however the UI thread or the async runtime is stuck.
//!
//! What it reports:
//! - **Never started:** no frame has finished a few seconds after launch. The last step reached is logged, and it is
//!   repeated with growing gaps, so the log shows how far startup got and how long it has been there.
//! - **Stuck inside a frame:** a draw began and has not ended; the last draw phase to finish is named.
//! - **Engine loop silent:** the engine stopped calling the shell's `frame` (it is blocked in the window system, for
//!   example during a Windows resize drag, or inside the graphics driver).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

const NEVER_STARTED_AFTER: Duration = Duration::from_secs(3);
const FRAME_STUCK_AFTER: Duration = Duration::from_millis(1500);
const ENGINE_SILENT_AFTER: Duration = Duration::from_secs(2);
const TICK: Duration = Duration::from_millis(500);

struct Marks {
    started: Instant,
    step: &'static str,
    step_at: Instant,
    frame_began: Option<Instant>,
    engine_frame_at: Option<Instant>,
    /// The engine's most recent errors, oldest first, so a startup that stalls can say what the engine complained about.
    engine_errors: Vec<String>,
}

static MARKS: Mutex<Option<Marks>> = Mutex::new(None);
static FRAMES_DONE: AtomicU64 = AtomicU64::new(0);
static ENGINE_FRAMES: AtomicU64 = AtomicU64::new(0);
static STOPPED: AtomicBool = AtomicBool::new(false);
static START: Once = Once::new();

fn with_marks(update: impl FnOnce(&mut Marks)) {
    if let Ok(mut marks) = MARKS.lock()
        && let Some(marks) = marks.as_mut()
    {
        update(marks);
    }
}

/// Starts the watching thread. Safe to call more than once; call it as early as possible so startup is covered.
pub fn start() {
    START.call_once(|| {
        let now = Instant::now();
        if let Ok(mut marks) = MARKS.lock() {
            *marks = Some(Marks {
                started: now,
                step: "watchdog started",
                step_at: now,
                frame_began: None,
                engine_frame_at: None,
                engine_errors: Vec::new(),
            });
        }
        let spawned = std::thread::Builder::new().name("vertex-watchdog".to_owned()).spawn(watch);
        if let Err(err) = spawned {
            tracing::warn!(target: "vertexlauncher/watchdog", "Could not start the UI watchdog: {err}");
        }
    });
}

/// Stops reporting (the window is closing, so silence is expected).
pub fn stop() {
    STOPPED.store(true, Ordering::Relaxed);
}

/// Records the last thing that finished, in startup or in a frame. Cheap enough to call often.
pub fn step(name: &'static str) {
    with_marks(|marks| {
        marks.step = name;
        marks.step_at = Instant::now();
    });
}

/// The draw closure is about to run.
pub fn frame_begin() {
    with_marks(|marks| {
        let now = Instant::now();
        marks.frame_began = Some(now);
        marks.step = "frame started";
        marks.step_at = now;
    });
}

/// The draw closure returned.
pub fn frame_end() {
    FRAMES_DONE.fetch_add(1, Ordering::Relaxed);
    with_marks(|marks| {
        marks.frame_began = None;
        marks.step = "frame finished";
        marks.step_at = Instant::now();
    });
}

/// The engine logged an error. Only the last few are kept, and only to quote them if the UI never starts.
pub fn engine_error(message: &str) {
    const KEEP: usize = 4;
    with_marks(|marks| {
        if marks.engine_errors.last().is_some_and(|last| last == message) {
            return;
        }
        if marks.engine_errors.len() == KEEP {
            marks.engine_errors.remove(0);
        }
        marks.engine_errors.push(message.chars().take(300).collect());
    });
}

/// The engine called the shell's per-frame callback.
pub fn engine_frame() {
    let first = ENGINE_FRAMES.fetch_add(1, Ordering::Relaxed) == 0;
    with_marks(|marks| {
        let now = Instant::now();
        marks.engine_frame_at = Some(now);
        if first {
            marks.step = "engine delivered its first frame";
            marks.step_at = now;
        }
    });
}

fn watch() {
    let mut next_never_report = NEVER_STARTED_AFTER;
    let mut reported_stuck_frame = false;
    let mut reported_silent_engine = false;
    loop {
        std::thread::sleep(TICK);
        if STOPPED.load(Ordering::Relaxed) {
            return;
        }
        let Ok(guard) = MARKS.try_lock() else { continue };
        let Some(marks) = guard.as_ref() else { continue };
        let now = Instant::now();
        let since_launch = now.duration_since(marks.started);
        let since_step = now.duration_since(marks.step_at);
        let frames = FRAMES_DONE.load(Ordering::Relaxed);
        let engine_frames = ENGINE_FRAMES.load(Ordering::Relaxed);

        if frames == 0 {
            if since_launch >= next_never_report {
                tracing::warn!(
                    target: "vertexlauncher/watchdog",
                    "No frame has been drawn {since_launch:.1?} after launch ({engine_frames} engine frames delivered). Last step reached: \"{}\", {since_step:.1?} ago. What never finished is whatever comes after that step.",
                    marks.step
                );
                if engine_frames > 0 {
                    // The engine is running its loop but never called the launcher's draw: it skips the UI frame when its UI
                    // renderer could not be created (it tries once), or there is no graphics device.
                    tracing::warn!(
                        target: "vertexlauncher/watchdog",
                        "The engine is delivering frames but never started the UI draw, which means the engine's UI renderer failed to start (it is tried once, so it stays off). {}",
                        if marks.engine_errors.is_empty() {
                            "The engine has logged no error; the cause is a failure it does not report.".to_owned()
                        } else {
                            format!("Its errors so far: {}", marks.engine_errors.join(" | "))
                        }
                    );
                }
                // Quickly at first, then less often, so a long hang does not fill the log.
                let gap = if since_launch < Duration::from_secs(30) { Duration::from_secs(3) } else { Duration::from_secs(20) };
                next_never_report = since_launch + gap;
            }
            continue;
        }

        match marks.frame_began {
            Some(began) if now.duration_since(began) >= FRAME_STUCK_AFTER => {
                if !reported_stuck_frame {
                    reported_stuck_frame = true;
                    tracing::warn!(
                        target: "vertexlauncher/watchdog",
                        "The UI thread has been inside one frame for {:.1?} and is still there. Last draw step finished: \"{}\" ({since_step:.1?} ago); it is stuck in the step after it.",
                        now.duration_since(began),
                        marks.step
                    );
                }
            }
            Some(_) => {}
            None => {
                if reported_stuck_frame {
                    reported_stuck_frame = false;
                    tracing::info!(target: "vertexlauncher/watchdog", "The UI thread is drawing frames again.");
                }
            }
        }

        match marks.engine_frame_at {
            Some(at) if now.duration_since(at) >= ENGINE_SILENT_AFTER && marks.frame_began.is_none() => {
                if !reported_silent_engine {
                    reported_silent_engine = true;
                    tracing::warn!(
                        target: "vertexlauncher/watchdog",
                        "The engine has not asked for a frame in {:.1?}. It is blocked in the window system or the graphics driver (a resize or move drag does this on Windows), or the window is hidden or minimized.",
                        now.duration_since(at)
                    );
                }
            }
            Some(at) if now.duration_since(at) < ENGINE_SILENT_AFTER => {
                if reported_silent_engine {
                    reported_silent_engine = false;
                    tracing::info!(target: "vertexlauncher/watchdog", "The engine is delivering frames again.");
                }
            }
            _ => {}
        }
    }
}
