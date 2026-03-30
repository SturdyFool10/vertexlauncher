use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use launcher_ui::ui::theme::{Oklch, Theme};
use url::Url;

const CALLBACK_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const AUTO_CLOSE_DELAY_SECS: u64 = 4;
const CALLBACK_PAGE_TEMPLATE: &str = include_str!("callback_page_template.html");

pub struct CallbackPageColors {
    pub bg: String,
    pub panel: String,
    pub border: String,
    pub text: String,
    pub muted: String,
    pub success: String,
    pub danger: String,
    pub primary_tint: String,
}

impl CallbackPageColors {
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            bg: fmt_oklch(theme.bg_dark),
            panel: fmt_oklch(theme.bg),
            border: fmt_oklch(theme.border_muted),
            text: fmt_oklch(theme.text),
            muted: fmt_oklch(theme.text_muted),
            success: fmt_oklch(theme.success),
            danger: fmt_oklch(theme.danger),
            primary_tint: fmt_oklch_alpha(theme.primary, 0.09),
        }
    }
}

fn fmt_oklch(c: Oklch) -> String {
    format!("oklch({:.3} {:.3} {:.1})", c.l, c.c, c.h)
}

fn fmt_oklch_alpha(c: Oklch, alpha: f32) -> String {
    format!("oklch({:.3} {:.3} {:.1} / {:.2})", c.l, c.c, c.h, alpha)
}

pub struct LoopbackCallbackListener {
    listener: TcpListener,
    redirect_uri: String,
}

impl LoopbackCallbackListener {
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }
}

pub fn prepare_loopback_callback_listener() -> Result<LoopbackCallbackListener, String> {
    let listener = bind_loopback_listener()?;
    let port = listener
        .local_addr()
        .map_err(|err| format!("Failed to read loopback listener address: {err}"))?
        .port();
    Ok(LoopbackCallbackListener {
        listener,
        redirect_uri: format!("http://localhost:{port}"),
    })
}

pub fn open_microsoft_sign_in(
    auth_request_uri: &str,
    callback_listener: LoopbackCallbackListener,
    colors: &CallbackPageColors,
) -> Result<String, String> {
    launcher_ui::desktop::open_url(auth_request_uri)?;
    wait_for_loopback_callback(
        callback_listener.listener,
        &callback_listener.redirect_uri,
        colors,
    )
}

fn bind_loopback_listener() -> Result<TcpListener, String> {
    TcpListener::bind(("127.0.0.1", 0))
        .map_err(|err| format!("Failed to bind localhost loopback listener: {err}"))
}

fn wait_for_loopback_callback(
    listener: TcpListener,
    redirect_uri: &str,
    colors: &CallbackPageColors,
) -> Result<String, String> {
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("Failed to configure loopback listener: {err}"))?;
    let started_at = Instant::now();

    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(callback_url) = handle_callback_stream(stream, redirect_uri, colors)? {
                    return Ok(callback_url);
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if started_at.elapsed() >= CALLBACK_TIMEOUT {
                    return Err(format!(
                        "Timed out waiting for Microsoft sign-in callback on localhost after {} seconds",
                        CALLBACK_TIMEOUT.as_secs()
                    ));
                }
                thread::sleep(ACCEPT_POLL_INTERVAL);
            }
            Err(err) => {
                return Err(format!(
                    "Failed while waiting for localhost sign-in callback: {err}"
                ));
            }
        }
    }
}

fn handle_callback_stream(
    mut stream: TcpStream,
    redirect_uri: &str,
    colors: &CallbackPageColors,
) -> Result<Option<String>, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|err| format!("Failed to configure callback stream timeout: {err}"))?;

    let mut request_line = String::new();
    {
        let mut reader = BufReader::new(&stream);
        reader
            .read_line(&mut request_line)
            .map_err(|err| format!("Failed reading callback request line: {err}"))?;
    }

    let request_line = request_line.trim();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    if method != "GET" || target.is_empty() {
        write_http_response(
            &mut stream,
            "400 Bad Request",
            &render_callback_page(
                "Sign-in Request Invalid",
                "Vertex Launcher could not understand the browser callback request.",
                Some("Try starting the browser sign-in flow again from the launcher."),
                CallbackPageTone::Error,
                colors,
            ),
        )?;
        return Ok(None);
    }

    let callback_url = rebuild_callback_url(target, redirect_uri)?;
    let callback = Url::parse(&callback_url)
        .map_err(|err| format!("Failed to parse callback URL from browser request: {err}"))?;
    let redirect = Url::parse(redirect_uri)
        .map_err(|err| format!("Failed to parse expected redirect URI: {err}"))?;
    if callback.path() != redirect.path() {
        write_http_response(
            &mut stream,
            "404 Not Found",
            &render_callback_page(
                "Wrong Redirect Page",
                "This localhost page is not being used by Vertex Launcher.",
                Some("You can close this tab."),
                CallbackPageTone::Error,
                colors,
            ),
        )?;
        return Ok(None);
    }

    let query_pairs = callback.query_pairs().collect::<Vec<_>>();
    let body = if query_pairs.iter().any(|(key, _)| key == "code") {
        render_callback_page(
            "Sign-in Complete",
            "Microsoft sign-in finished successfully.",
            Some("This tab should close automatically. You can return to Vertex Launcher."),
            CallbackPageTone::Success,
            colors,
        )
    } else if let Some(error) = query_pairs
        .iter()
        .find(|(key, _)| key == "error")
        .map(|(_, value)| value.as_ref())
    {
        let detail = query_pairs
            .iter()
            .find(|(key, _)| key == "error_description")
            .map(|(_, value)| value.as_ref());
        render_callback_page(
            "Sign-in Failed",
            &format!("Microsoft returned an authorization error: {error}."),
            detail,
            CallbackPageTone::Error,
            colors,
        )
    } else {
        render_callback_page(
            "Sign-in Incomplete",
            "Microsoft returned to Vertex Launcher, but no authorization code was included.",
            Some("You can close this tab and retry the sign-in flow."),
            CallbackPageTone::Error,
            colors,
        )
    };
    write_http_response(&mut stream, "200 OK", &body)?;
    Ok(Some(callback_url))
}

fn rebuild_callback_url(target: &str, redirect_uri: &str) -> Result<String, String> {
    let request_url = Url::parse(&format!("http://localhost{target}"))
        .map_err(|err| format!("Failed to parse callback request target: {err}"))?;
    let mut callback = Url::parse(redirect_uri)
        .map_err(|err| format!("Failed to parse expected redirect URI: {err}"))?;
    callback.set_query(request_url.query());
    callback.set_fragment(request_url.fragment());
    Ok(callback.to_string())
}

fn write_http_response(stream: &mut TcpStream, status: &str, body: &str) -> Result<(), String> {
    let body_bytes = body.as_bytes();
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
        body_bytes.len(),
        body
    )
    .map_err(|err| format!("Failed writing localhost callback response: {err}"))
}

#[derive(Clone, Copy)]
enum CallbackPageTone {
    Success,
    Error,
}

fn render_callback_page(
    title: &str,
    message: &str,
    detail: Option<&str>,
    tone: CallbackPageTone,
    colors: &CallbackPageColors,
) -> String {
    let accent = match tone {
        CallbackPageTone::Success => colors.success.as_str(),
        CallbackPageTone::Error => colors.danger.as_str(),
    };
    let badge = match tone {
        CallbackPageTone::Success => "Connected",
        CallbackPageTone::Error => "Action needed",
    };
    let detail_html = detail
        .map(|d| format!(r#"<p class="detail">{}</p>"#, escape_html(d)))
        .unwrap_or_default();
    let secs = AUTO_CLOSE_DELAY_SECS.to_string();

    CALLBACK_PAGE_TEMPLATE
        .replace("{{BG}}", &colors.bg)
        .replace("{{PANEL}}", &colors.panel)
        .replace("{{BORDER}}", &colors.border)
        .replace("{{TEXT}}", &colors.text)
        .replace("{{MUTED}}", &colors.muted)
        .replace("{{PRIMARY_TINT}}", &colors.primary_tint)
        .replace("{{ACCENT}}", accent)
        .replace("{{BADGE}}", badge)
        .replace("{{TITLE}}", &escape_html(title))
        .replace("{{MESSAGE}}", &escape_html(message))
        .replace("{{DETAIL_HTML}}", &detail_html)
        .replace("{{AUTO_CLOSE_SECS}}", &secs)
}

fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}
