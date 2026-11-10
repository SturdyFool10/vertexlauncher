use vertex_i18n::tr;
use super::*;
use std::cell::RefCell;

/// Receives the install progress plus the overall completed fraction (`0.0..=1.0`).
pub(in crate::screens::content_browser) type InstallReportSink =
    Arc<dyn Fn(&installation::InstallProgress, f32) + Send + Sync + 'static>;

const REPORT_INTERVAL: Duration = Duration::from_millis(120);
const NEXT_NAMES_SHOWN: usize = 3;

/// Live state of one content install (the project, its dependencies and their files).
///
/// The install routine runs synchronously on one worker thread, so the active reporter is
/// kept in a thread-local and the install code only calls the `report_*` functions below.
/// Installs that run without a reporter (bulk updates, tests) simply report nothing.
pub(in crate::screens::content_browser) struct InstallReporter {
    sink: InstallReportSink,
    /// Every project that will be downloaded, in install order.
    names: Vec<String>,
    /// Other queued operations waiting behind this one.
    queued_after: usize,
    done: u32,
    current: Option<String>,
    started: Instant,
    bytes_before_current: u64,
    current_bytes: u64,
    current_total: Option<u64>,
    last_emit: Option<Instant>,
}

impl InstallReporter {
    pub(in crate::screens::content_browser) fn new(
        sink: InstallReportSink,
        names: Vec<String>,
        queued_after: usize,
    ) -> Self {
        Self {
            sink,
            names,
            queued_after,
            done: 0,
            current: None,
            started: Instant::now(),
            bytes_before_current: 0,
            current_bytes: 0,
            current_total: None,
            last_emit: None,
        }
    }

    fn total_files(&self) -> u32 {
        self.names.len().max(1) as u32
    }

    fn fraction(&self) -> f32 {
        let total = self.total_files() as f32;
        let within_file = match self.current_total {
            Some(total_bytes) if total_bytes > 0 => {
                (self.current_bytes as f32 / total_bytes as f32).clamp(0.0, 1.0)
            }
            _ => 0.0,
        };
        ((self.done as f32 + within_file) / total).clamp(0.0, 1.0)
    }

    fn message(&self, bytes_per_second: f64) -> String {
        let total = self.total_files();
        let position = (self.done + 1).min(total);
        let mut message = match self.current.as_deref() {
            Some(name) => tr!("Installing {name} ({position}/{total}) · {downloaded:.1} MiB · {speed:.1} MiB/s", name = name, position = position, total = total, downloaded = (self.bytes_before_current + self.current_bytes) as f64 / (1024.0 * 1024.0), speed = bytes_per_second / (1024.0 * 1024.0)).into_owned(),
            None => tr!("Resolving dependencies ({done}/{total} done)", done = self.done, total = total).into_owned(),
        };
        let next = self
            .names
            .iter()
            .skip(self.done as usize + 1)
            .take(NEXT_NAMES_SHOWN)
            .cloned()
            .collect::<Vec<_>>();
        if !next.is_empty() {
            let remaining = self.names.len().saturating_sub(self.done as usize + 1);
            message.push('\n');
            message.push_str(&tr!("Next: {names}", names = next.join(", ")));
            if remaining > next.len() {
                message.push(' ');
                message.push_str(&tr!("(+{count} more)", count = remaining - next.len()));
            }
        }
        if self.queued_after > 0 {
            message.push('\n');
            message.push_str(&tr!("{count} more queued", count = self.queued_after));
        }
        message
    }

    fn emit(&mut self, force: bool) {
        let now = Instant::now();
        if !force
            && self
                .last_emit
                .is_some_and(|last| now.duration_since(last) < REPORT_INTERVAL)
        {
            return;
        }
        self.last_emit = Some(now);
        let downloaded_bytes = self.bytes_before_current + self.current_bytes;
        let elapsed = self.started.elapsed().as_secs_f64().max(0.001);
        let bytes_per_second = downloaded_bytes as f64 / elapsed;
        let progress = installation::InstallProgress {
            stage: installation::InstallStage::DownloadingCore,
            message: self.message(bytes_per_second),
            downloaded_files: self.done,
            total_files: self.total_files(),
            downloaded_bytes,
            total_bytes: None,
            bytes_per_second,
            eta_seconds: None,
        };
        (self.sink)(&progress, self.fraction());
    }
}

thread_local! {
    static ACTIVE_REPORTER: RefCell<Option<InstallReporter>> = const { RefCell::new(None) };
}

struct ReporterGuard;

impl Drop for ReporterGuard {
    fn drop(&mut self) {
        ACTIVE_REPORTER.with(|slot| slot.borrow_mut().take());
    }
}

/// Runs `run` with `reporter` active on this thread, removing it afterwards even on panic.
pub(in crate::screens::content_browser) fn with_install_reporter<T>(
    mut reporter: InstallReporter,
    run: impl FnOnce() -> T,
) -> T {
    reporter.emit(true);
    ACTIVE_REPORTER.with(|slot| *slot.borrow_mut() = Some(reporter));
    let _guard = ReporterGuard;
    run()
}

fn with_reporter(update: impl FnOnce(&mut InstallReporter)) {
    ACTIVE_REPORTER.with(|slot| {
        if let Some(reporter) = slot.borrow_mut().as_mut() {
            update(reporter);
        }
    });
}

pub(super) fn report_begin_file(name: &str) {
    with_reporter(|reporter| {
        reporter.current = Some(name.to_owned());
        reporter.current_bytes = 0;
        reporter.current_total = None;
        reporter.emit(true);
    });
}

pub(super) fn report_file_bytes(downloaded: u64, total: Option<u64>) {
    with_reporter(|reporter| {
        reporter.current_bytes = downloaded;
        reporter.current_total = total;
        reporter.emit(false);
    });
}

pub(super) fn report_finish_file() {
    with_reporter(|reporter| {
        reporter.done += 1;
        reporter.bytes_before_current += reporter.current_bytes;
        reporter.current_bytes = 0;
        reporter.current_total = None;
        reporter.emit(true);
    });
}
