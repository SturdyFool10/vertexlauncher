#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
use std::{
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
use crate::app::webview_runtime::{tao, wry};
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
use serde::Deserialize;
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
use tao::dpi::LogicalSize;
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
use tao::event::StartCause;
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
use tao::event::{Event, WindowEvent};
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
use tao::platform::run_return::EventLoopExtRunReturn;
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
use tao::window::WindowBuilder;
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
use wry::webview::WebViewBuilder;

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy)]
enum UserEvent {
    Finish,
    CheckHealth,
}

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
#[derive(Debug)]
struct AttemptFailure {
    retryable: bool,
    message: String,
}

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
#[derive(Debug)]
enum AttemptOutcome {
    Success(String),
    Failure(AttemptFailure),
}

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
#[derive(Debug)]
struct AttemptState {
    started_at: Instant,
    first_activity_at: Option<Instant>,
    last_activity_at: Option<Instant>,
    first_content_signal_at: Option<Instant>,
    last_content_signal_at: Option<Instant>,
    blank_reload_count: u8,
    navigation_count: usize,
    ipc_event_count: usize,
    title_change_count: usize,
    last_observed_url: Option<String>,
    last_document_title_fingerprint: Option<String>,
    last_document_title_len: Option<usize>,
    last_ipc_kind: Option<String>,
}

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
impl AttemptState {
    fn new(started_at: Instant) -> Self {
        Self {
            started_at,
            first_activity_at: None,
            last_activity_at: None,
            first_content_signal_at: None,
            last_content_signal_at: None,
            blank_reload_count: 0,
            navigation_count: 0,
            ipc_event_count: 0,
            title_change_count: 0,
            last_observed_url: None,
            last_document_title_fingerprint: None,
            last_document_title_len: None,
            last_ipc_kind: None,
        }
    }

    fn note_activity(&mut self, now: Instant) {
        if self.first_activity_at.is_none() {
            self.first_activity_at = Some(now);
        }
        self.last_activity_at = Some(now);
    }

    fn note_content_signal(&mut self, now: Instant) {
        self.note_activity(now);
        if self.first_content_signal_at.is_none() {
            self.first_content_signal_at = Some(now);
        }
        self.last_content_signal_at = Some(now);
    }

    fn note_navigation(&mut self, now: Instant, uri: &str) {
        self.note_activity(now);
        self.navigation_count += 1;
        self.last_observed_url = Some(uri.to_owned());
    }

    fn note_title_change(&mut self, now: Instant, title: &str) {
        self.note_content_signal(now);
        self.title_change_count += 1;
        self.last_document_title_fingerprint = Some(super::fingerprint_for_log(title));
        self.last_document_title_len = Some(title.chars().count());
    }

    fn note_ipc(&mut self, now: Instant, kind: &str, href: Option<&str>, title: Option<&str>) {
        self.note_content_signal(now);
        self.ipc_event_count += 1;
        self.last_ipc_kind = Some(kind.to_owned());
        if let Some(href) = href {
            self.last_observed_url = Some(href.to_owned());
        }
        if let Some(title) = title {
            self.last_document_title_fingerprint = Some(super::fingerprint_for_log(title));
            self.last_document_title_len = Some(title.chars().count());
        }
    }

    fn saw_render_signal(&self) -> bool {
        self.first_content_signal_at.is_some()
    }

    fn log_summary(&self) -> String {
        format!(
            "signals=navigation:{} ipc:{} title:{} content_seen={} last_url={} last_title_fp={} last_title_len={} last_ipc={}",
            self.navigation_count,
            self.ipc_event_count,
            self.title_change_count,
            self.saw_render_signal(),
            self.last_observed_url
                .as_deref()
                .map(super::sanitize_url_for_log)
                .unwrap_or_else(|| "<none>".to_owned()),
            self.last_document_title_fingerprint
                .as_deref()
                .map(str::to_owned)
                .unwrap_or_else(|| "<none>".to_owned()),
            self.last_document_title_len
                .map(|len| len.to_string())
                .unwrap_or_else(|| "<none>".to_owned()),
            self.last_ipc_kind.as_deref().unwrap_or("<none>")
        )
    }
}

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
#[derive(Debug, Deserialize)]
struct WebviewIpcEvent {
    kind: Option<String>,
    href: Option<String>,
    title: Option<String>,
    #[serde(rename = "readyState")]
    ready_state: Option<String>,
    #[serde(rename = "hasBody")]
    has_body: Option<bool>,
    #[serde(rename = "visibilityState")]
    visibility_state: Option<String>,
    detail: Option<serde_json::Value>,
}

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
pub(super) fn run_webview_window(
    auth_request_uri: &str,
    redirect_uri: &str,
) -> Result<String, String> {
    run_webview_window_with_retries(auth_request_uri, redirect_uri)
}

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
fn run_webview_window_with_retries(
    auth_request_uri: &str,
    redirect_uri: &str,
) -> Result<String, String> {
    const MAX_WINDOW_ATTEMPTS: usize = 2;

    let mut last_retryable_error = None;
    for attempt in 1..=MAX_WINDOW_ATTEMPTS {
        match run_webview_window_attempt(auth_request_uri, redirect_uri, attempt) {
            Ok(callback_url) => return Ok(callback_url),
            Err(err) if err.retryable && attempt < MAX_WINDOW_ATTEMPTS => {
                tracing::warn!(
                    target: "vertexlauncher/auth/webview/helper",
                    attempt,
                    max_attempts = MAX_WINDOW_ATTEMPTS,
                    error = %super::sanitize_message_for_log(&err.message),
                    "Retrying Microsoft sign-in webview after blank or stalled startup."
                );
                last_retryable_error = Some(err.message);
            }
            Err(err) if err.retryable => {
                return Err(super::mark_retryable_helper_error(&err.message));
            }
            Err(err) => return Err(err.message),
        }
    }

    Err(super::mark_retryable_helper_error(
        last_retryable_error
            .as_deref()
            .unwrap_or("Microsoft sign-in webview failed without reporting page activity."),
    ))
}

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
/// Launch a webview sign-in window. On Linux this will attempt to
/// discover the correct WebKitGTK helper directory (`WebKitWebProcess` and
/// siblings) and set the `WEBKIT_EXEC_PATH` environment variable if it
/// is not already defined. This helps avoid failures where the
/// underlying `webkit2gtk` stack is installed in a non-standard location
/// such as `/usr/lib/webkit2gtk-4.0` instead of `/usr/libexec/webkit2gtk-4.0`.
fn run_webview_window_attempt(
    auth_request_uri: &str,
    redirect_uri: &str,
    attempt: usize,
) -> Result<String, AttemptFailure> {
    // On Linux the wry backend uses WebKitGTK to implement webviews.  When
    // spawning WebKit's helper processes it relies on the `WEBKIT_EXEC_PATH`
    // environment variable to locate `WebKitWebProcess` and related
    // executables.  Distros package these helpers in a variety of
    // directories – for example `/usr/libexec/webkit2gtk-4.0` on
    // Debian/Ubuntu, `/usr/lib64/webkit2gtk-4.0` on some RPM-based
    // distributions, and `/usr/lib/webkit2gtk-4.0` on Arch/CachyOS.  If the
    // variable is unset and the default relative lookup fails, the
    // authentication window will crash immediately with an error like
    // "Unable to fork a new child process: Failed to execute child process
    // \"libexec/webkit2gtk-4.0/WebKitWebProcess\" (No such file or
    // directory)".  To make the launcher robust across distributions
    // install WebKitGTK in different locations, we proactively search for
    // the helpers and populate `WEBKIT_EXEC_PATH` on Linux.
    #[cfg(target_os = "linux")]
    {
        use std::path::Path;
        // Only override the variable if it is currently unset to respect
        // callers that already configured a custom path.
        if std::env::var_os("WEBKIT_EXEC_PATH").is_none() {
            // Prepare a list of candidate directories that commonly contain
            // WebKitGTK helper executables.  These include the standard
            // `libexec` directory as well as distribution-specific
            // locations such as Arch's `/usr/lib/webkit2gtk-4.0`.
            // In a Flatpak bundle the contents of the original AppDir are
            // relocated under `/app` rather than `/usr`, so include
            // `/app/libexec`, `/app/lib` and `/app/lib64` variants as
            // candidate locations.  Without these the launcher may fail to
            // locate the helpers when running inside a Flatpak built from
            // our AppDir.  See build scripts for bundling details.
            let mut candidate_dirs: Vec<String> = vec![
                "/usr/libexec/webkit2gtk-4.0".to_string(),
                "/usr/lib/webkit2gtk-4.0".to_string(),
                "/usr/lib64/webkit2gtk-4.0".to_string(),
                "/usr/lib/x86_64-linux-gnu/webkit2gtk-4.0".to_string(),
                "/usr/lib/aarch64-linux-gnu/webkit2gtk-4.0".to_string(),
                "/app/libexec/webkit2gtk-4.0".to_string(),
                "/app/lib/webkit2gtk-4.0".to_string(),
                "/app/lib64/webkit2gtk-4.0".to_string(),
            ];
            // Additionally scan some common parent directories for any
            // directories that start with `webkit2gtk-` to catch new
            // versions (e.g. webkit2gtk-4.1) or vendor-specific layouts.
            for base in [
                "/usr/libexec",
                "/usr/lib",
                "/usr/lib64",
                "/app/libexec",
                "/app/lib",
                "/app/lib64",
            ]
            .iter()
            {
                if let Ok(entries) = std::fs::read_dir(base) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if name.starts_with("webkit2gtk-") {
                                candidate_dirs.push(path.to_string_lossy().into_owned());
                            }
                        }
                    }
                }
            }
            // Find the first directory containing the helper executable.
            let mut chosen: Option<String> = None;
            for dir in candidate_dirs {
                let helper = Path::new(&dir).join("WebKitWebProcess");
                if helper.exists() {
                    chosen = Some(dir);
                    break;
                }
            }
            if let Some(dir) = chosen {
                // std::env::set_var is unsafe as of Rust 1.94 because the POSIX
                // environment is not thread safe.  The documentation states
                // that it is only safe to call in single-threaded programs or
                // on Windows【584495416157657†L74-L87】.  We call it here
                // prior to spawning any new threads to configure the
                // environment for WebKitGTK helper processes.  See the
                // `std::env::set_var` docs for the full safety discussion【584495416157657†L74-L87】.
                unsafe {
                    std::env::set_var("WEBKIT_EXEC_PATH", &dir);
                }
                tracing::info!(
                    target: "vertexlauncher/auth/webview/helper",
                    path = %dir,
                    "Discovered WebKitGTK helper directory and set WEBKIT_EXEC_PATH."
                );
            }
        }
    }
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let result = Arc::new(Mutex::new(None::<AttemptOutcome>));
    let result_for_nav = Arc::clone(&result);
    let result_for_loop = Arc::clone(&result);
    let redirect_prefix = redirect_uri.to_owned();
    let auth_request_uri = auth_request_uri.to_owned();
    let helper_profile =
        std::env::var("VERTEX_WEBVIEW_SIGNIN_PROFILE").unwrap_or_else(|_| "default".to_owned());
    let helper_profile_for_ipc = helper_profile.clone();
    let helper_profile_for_nav = helper_profile.clone();
    let helper_profile_for_title = helper_profile.clone();
    let helper_profile_for_loop = helper_profile.clone();
    let blank_timeout = blank_page_timeout();
    let activity_state = Arc::new(Mutex::new(AttemptState::new(Instant::now())));

    match wry::webview::webview_version() {
        Ok(version) => tracing::info!(
            target: "vertexlauncher/auth/webview/helper",
            attempt,
            profile = %helper_profile,
            engine_version = %version,
            auth_url = %super::sanitize_url_for_log(&auth_request_uri),
            redirect_url = %super::sanitize_url_for_log(&redirect_prefix),
            "Launching Microsoft sign-in webview attempt."
        ),
        Err(err) => tracing::warn!(
            target: "vertexlauncher/auth/webview/helper",
            attempt,
            profile = %helper_profile,
            error = %super::sanitize_message_for_log(&err.to_string()),
            auth_url = %super::sanitize_url_for_log(&auth_request_uri),
            redirect_url = %super::sanitize_url_for_log(&redirect_prefix),
            "Launching Microsoft sign-in webview attempt without a resolved engine version."
        ),
    }

    tracing::info!(
        target: "vertexlauncher/auth/webview/helper",
        attempt,
        profile = %helper_profile,
        display = ?std::env::var("DISPLAY").ok(),
        wayland_display = ?std::env::var("WAYLAND_DISPLAY").ok(),
        xdg_session_type = ?std::env::var("XDG_SESSION_TYPE").ok(),
        gdk_backend = ?std::env::var("GDK_BACKEND").ok(),
        webkit_disable_compositing_mode = ?std::env::var("WEBKIT_DISABLE_COMPOSITING_MODE").ok(),
        "Webview sign-in helper environment snapshot."
    );

    let window_builder = WindowBuilder::new()
        .with_title("Microsoft Sign-In")
        .with_inner_size(LogicalSize::new(980.0, 760.0));
    #[cfg(not(target_os = "macos"))]
    let window_builder = if let Some(icon) = crate::app::app_icon::tao_icon() {
        window_builder.with_window_icon(Some(icon))
    } else {
        window_builder
    };

    let window = window_builder
        .build(&event_loop)
        .map_err(|err| AttemptFailure {
            retryable: false,
            message: format!("Failed to create sign-in window: {err}"),
        })?;

    let activity_state_for_nav = Arc::clone(&activity_state);
    let activity_state_for_ipc = Arc::clone(&activity_state);
    let activity_state_for_title = Arc::clone(&activity_state);

    let webview = WebViewBuilder::new(window)
        .map_err(|err| AttemptFailure {
            retryable: false,
            message: format!("Failed to initialize webview builder: {err}"),
        })?
        .with_background_color((255, 255, 255, 255))
        .with_initialization_script(WEBVIEW_DIAGNOSTIC_SCRIPT)
        .with_ipc_handler(move |_window, payload| {
            let now = Instant::now();
            let Ok(event) = serde_json::from_str::<WebviewIpcEvent>(&payload) else {
                tracing::debug!(
                    target: "vertexlauncher/auth/webview/helper",
                    attempt,
                    profile = %helper_profile_for_ipc,
                    payload_fingerprint = %super::fingerprint_for_log(&payload),
                    payload_len = payload.len(),
                    "Received unparsed sign-in webview IPC payload."
                );
                return;
            };

            let kind = event.kind.as_deref().unwrap_or("unknown");
            if let Ok(mut state) = activity_state_for_ipc.lock() {
                state.note_ipc(now, kind, event.href.as_deref(), event.title.as_deref());
            }

            let url_for_log = event
                .href
                .as_deref()
                .map(super::sanitize_url_for_log)
                .unwrap_or_else(|| "<none>".to_owned());
            let title_fingerprint_for_log = event
                .title
                .as_deref()
                .map(super::fingerprint_for_log)
                .unwrap_or_else(|| "<none>".to_owned());
            let title_len_for_log = event.title.as_ref().map(|title| title.chars().count());
            let detail_fingerprint_for_log = event
                .detail
                .as_ref()
                .map(|detail| super::fingerprint_for_log(&detail.to_string()))
                .unwrap_or_else(|| "<none>".to_owned());
            let detail_present_for_log = event.detail.is_some();

            match kind {
                "page-error" | "unhandledrejection" => tracing::warn!(
                    target: "vertexlauncher/auth/webview/helper",
                    attempt,
                    profile = %helper_profile_for_ipc,
                    kind,
                    url = %url_for_log,
                    title_fingerprint = %title_fingerprint_for_log,
                    title_len = ?title_len_for_log,
                    ready_state = ?event.ready_state,
                    has_body = ?event.has_body,
                    visibility = ?event.visibility_state,
                    detail_present = detail_present_for_log,
                    detail_fingerprint = %detail_fingerprint_for_log,
                    "Sign-in webview reported a page error."
                ),
                "load" | "domcontentloaded" | "init" => tracing::info!(
                    target: "vertexlauncher/auth/webview/helper",
                    attempt,
                    profile = %helper_profile_for_ipc,
                    kind,
                    url = %url_for_log,
                    title_fingerprint = %title_fingerprint_for_log,
                    title_len = ?title_len_for_log,
                    ready_state = ?event.ready_state,
                    has_body = ?event.has_body,
                    visibility = ?event.visibility_state,
                    detail_present = detail_present_for_log,
                    detail_fingerprint = %detail_fingerprint_for_log,
                    "Sign-in webview lifecycle event."
                ),
                _ => tracing::debug!(
                    target: "vertexlauncher/auth/webview/helper",
                    attempt,
                    profile = %helper_profile_for_ipc,
                    kind,
                    url = %url_for_log,
                    title_fingerprint = %title_fingerprint_for_log,
                    title_len = ?title_len_for_log,
                    ready_state = ?event.ready_state,
                    has_body = ?event.has_body,
                    visibility = ?event.visibility_state,
                    detail_present = detail_present_for_log,
                    detail_fingerprint = %detail_fingerprint_for_log,
                    "Sign-in webview telemetry event."
                ),
            }
        })
        .with_url(&auth_request_uri)
        .map_err(|err| AttemptFailure {
            retryable: false,
            message: format!("Failed to set sign-in URL: {err}"),
        })?
        .with_navigation_handler(move |uri: String| {
            let current_uri = uri;
            let now = Instant::now();
            if let Ok(mut state) = activity_state_for_nav.lock() {
                state.note_navigation(now, &current_uri);
            }

            // Disallow high-risk local file navigation inside auth webview.
            if current_uri.starts_with("file://") {
                tracing::warn!(
                    target: "vertexlauncher/auth/webview/helper",
                    attempt,
                    profile = %helper_profile_for_nav,
                    url = %super::sanitize_url_for_log(&current_uri),
                    "Blocked file navigation inside Microsoft sign-in webview."
                );
                return false;
            }

            if current_uri.starts_with(&redirect_prefix) {
                tracing::info!(
                    target: "vertexlauncher/auth/webview/helper",
                    attempt,
                    profile = %helper_profile_for_nav,
                    callback_url = %super::sanitize_url_for_log(&current_uri),
                    "Microsoft sign-in webview reached the OAuth callback URL."
                );
                if let Ok(mut slot) = result_for_nav.lock() {
                    *slot = Some(AttemptOutcome::Success(current_uri));
                }
                let _ = proxy.send_event(UserEvent::Finish);
                return false;
            }

            tracing::info!(
                target: "vertexlauncher/auth/webview/helper",
                attempt,
                profile = %helper_profile_for_nav,
                url = %super::sanitize_url_for_log(&current_uri),
                "Allowing Microsoft sign-in webview navigation."
            );
            true
        })
        .with_document_title_changed_handler(move |_window, title| {
            let now = Instant::now();
            if let Ok(mut state) = activity_state_for_title.lock() {
                state.note_title_change(now, &title);
            }

            tracing::info!(
                target: "vertexlauncher/auth/webview/helper",
                attempt,
                profile = %helper_profile_for_title,
                title_fingerprint = %super::fingerprint_for_log(&title),
                title_len = title.chars().count(),
                "Microsoft sign-in webview document title changed."
            );
        })
        .build()
        .map_err(|err| AttemptFailure {
            retryable: false,
            message: format!("Failed to build webview: {err}"),
        })?;

    tracing::info!(
        target: "vertexlauncher/auth/webview/helper",
        attempt,
        profile = %helper_profile,
        blank_timeout_secs = blank_timeout.as_secs(),
        "Microsoft sign-in webview created successfully."
    );

    let proxy_for_health = event_loop.create_proxy();
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(2));
            if proxy_for_health.send_event(UserEvent::CheckHealth).is_err() {
                break;
            }
        }
    });

    event_loop.run_return(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(StartCause::Init) => {
                tracing::debug!(
                    target: "vertexlauncher/auth/webview/helper",
                    attempt,
                    profile = %helper_profile_for_loop,
                    "Microsoft sign-in webview event loop started."
                );
            }
            Event::UserEvent(UserEvent::Finish) => {
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(UserEvent::CheckHealth) => {
                let now = Instant::now();
                let mut should_reload = false;
                let mut failure = None;

                if let Ok(mut state) = activity_state.lock() {
                    if !state.saw_render_signal()
                        && now.duration_since(state.started_at) >= blank_timeout
                    {
                        if state.blank_reload_count == 0 {
                            state.blank_reload_count = 1;
                            state.started_at = now;
                            should_reload = true;
                            tracing::warn!(
                                target: "vertexlauncher/auth/webview/helper",
                                attempt,
                                profile = %helper_profile_for_loop,
                                blank_timeout_secs = blank_timeout.as_secs(),
                                summary = %state.log_summary(),
                                "Microsoft sign-in webview showed no page activity; reloading sign-in URL once."
                            );
                        } else {
                            let message = format!(
                                "Microsoft sign-in webview never showed page activity within {} seconds and likely failed to render the login page. See launcher logs for details.",
                                blank_timeout.as_secs()
                            );
                            tracing::error!(
                                target: "vertexlauncher/auth/webview/helper",
                                attempt,
                                profile = %helper_profile_for_loop,
                                blank_timeout_secs = blank_timeout.as_secs(),
                                summary = %state.log_summary(),
                                "Microsoft sign-in webview remained blank after a reload attempt."
                            );
                            failure = Some(AttemptFailure {
                                retryable: true,
                                message,
                            });
                        }
                    }
                }

                if should_reload {
                    tracing::info!(
                        target: "vertexlauncher/auth/webview/helper",
                        attempt,
                        profile = %helper_profile_for_loop,
                        url = %super::sanitize_url_for_log(&auth_request_uri),
                        "Reloading Microsoft sign-in URL after blank page detection."
                    );
                    webview.load_url(&auth_request_uri);
                }
                if let Some(failure) = failure {
                    if let Ok(mut slot) = result_for_loop.lock() {
                        *slot = Some(AttemptOutcome::Failure(failure));
                    }
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if let Ok(mut slot) = result_for_loop.lock() {
                    if slot.is_none() {
                        *slot = Some(AttemptOutcome::Failure(AttemptFailure {
                            retryable: false,
                            message: "Microsoft sign-in was canceled".to_owned(),
                        }));
                    }
                }
                tracing::info!(
                    target: "vertexlauncher/auth/webview/helper",
                    attempt,
                    profile = %helper_profile_for_loop,
                    "Microsoft sign-in webview window was closed by the user."
                );
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });

    match result.lock() {
        Ok(mut slot) => {
            let outcome = slot.take().unwrap_or_else(|| {
                AttemptOutcome::Failure(AttemptFailure {
                    retryable: false,
                    message: "Microsoft sign-in ended without a callback URL".to_owned(),
                })
            });

            match outcome {
                AttemptOutcome::Success(callback_url) => Ok(callback_url),
                AttemptOutcome::Failure(err) => Err(err),
            }
        }
        Err(_) => Err(AttemptFailure {
            retryable: false,
            message: "Sign-in state was poisoned unexpectedly".to_owned(),
        }),
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
pub(super) fn run_webview_window(
    _auth_request_uri: &str,
    _redirect_uri: &str,
) -> Result<String, String> {
    Err("Webview sign-in is not supported on this platform".to_owned())
}

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
fn blank_page_timeout() -> std::time::Duration {
    const DEFAULT_BLANK_TIMEOUT_SECS: u64 = 30;
    const BLANK_TIMEOUT_ENV: &str = "VERTEX_WEBVIEW_SIGNIN_BLANK_TIMEOUT_SECS";

    match std::env::var(BLANK_TIMEOUT_ENV) {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(seconds) if seconds > 0 => std::time::Duration::from_secs(seconds),
            _ => {
                tracing::warn!(
                    target: "vertexlauncher/auth/webview/helper",
                    env = BLANK_TIMEOUT_ENV,
                    value = %raw,
                    "Invalid blank webview timeout override; using default."
                );
                std::time::Duration::from_secs(DEFAULT_BLANK_TIMEOUT_SECS)
            }
        },
        Err(_) => std::time::Duration::from_secs(DEFAULT_BLANK_TIMEOUT_SECS),
    }
}

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
const WEBVIEW_DIAGNOSTIC_SCRIPT: &str = r#"
(function () {
  if (window.__vertexSigninDiagnosticsInstalled) {
    return;
  }
  window.__vertexSigninDiagnosticsInstalled = true;

  function safeString(value) {
    if (typeof value === "string") {
      return value;
    }
    if (value === null || value === undefined) {
      return "";
    }
    try {
      return String(value);
    } catch (_) {
      return "";
    }
  }

  function emit(kind, detail) {
    try {
      if (!window.ipc || typeof window.ipc.postMessage !== "function") {
        return;
      }
      window.ipc.postMessage(JSON.stringify({
        kind: kind,
        href: safeString(window.location && window.location.href),
        title: safeString(document && document.title),
        readyState: safeString(document && document.readyState),
        hasBody: !!(document && document.body),
        visibilityState: safeString(document && document.visibilityState),
        detail: detail || null
      }));
    } catch (_) {}
  }

  emit("init", null);
  if (document) {
    document.addEventListener("readystatechange", function () {
      emit("readystatechange", { readyState: safeString(document.readyState) });
    });
    document.addEventListener("DOMContentLoaded", function () {
      emit("domcontentloaded", null);
    });
  }
  window.addEventListener("load", function () {
    emit("load", null);
  });
  window.addEventListener(
    "error",
    function (event) {
      emit("page-error", {
        message: safeString(event && event.message),
        filename: safeString(event && event.filename),
        lineno: event && event.lineno ? event.lineno : 0,
        colno: event && event.colno ? event.colno : 0
      });
    },
    true
  );
  window.addEventListener("unhandledrejection", function (event) {
    emit("unhandledrejection", {
      reason: safeString(event && event.reason)
    });
  });
  setTimeout(function () {
    emit("heartbeat-5s", null);
  }, 5000);
  setTimeout(function () {
    emit("heartbeat-15s", null);
  }, 15000);
})();
"#;
