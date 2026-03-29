use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use url::Url;

const CALLBACK_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(100);

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
) -> Result<String, String> {
    launcher_ui::desktop::open_url(auth_request_uri)?;
    wait_for_loopback_callback(callback_listener.listener, &callback_listener.redirect_uri)
}

fn bind_loopback_listener() -> Result<TcpListener, String> {
    TcpListener::bind(("127.0.0.1", 0))
        .map_err(|err| format!("Failed to bind localhost loopback listener: {err}"))
}

fn wait_for_loopback_callback(listener: TcpListener, redirect_uri: &str) -> Result<String, String> {
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("Failed to configure loopback listener: {err}"))?;
    let started_at = Instant::now();

    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(callback_url) = handle_callback_stream(stream, redirect_uri)? {
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
            "Invalid sign-in callback request.",
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
            "This page is not used by Vertex Launcher.",
        )?;
        return Ok(None);
    }

    let body = if callback.query_pairs().any(|(key, _)| key == "code") {
        "Microsoft sign-in completed. You can close this tab and return to Vertex Launcher."
    } else {
        "Microsoft sign-in returned to Vertex Launcher, but no authorization code was included."
    };
    write_http_response(&mut stream, "200 OK", body)?;
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
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
        body_bytes.len(),
        body
    )
    .map_err(|err| format!("Failed writing localhost callback response: {err}"))
}
