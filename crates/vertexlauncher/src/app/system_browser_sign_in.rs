use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use url::Url;

const CALLBACK_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const AUTO_CLOSE_DELAY_SECS: u64 = 4;

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
            &render_callback_page(
                "Sign-in Request Invalid",
                "Vertex Launcher could not understand the browser callback request.",
                Some("Try starting the browser sign-in flow again from the launcher."),
                CallbackPageTone::Error,
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
        )
    } else {
        render_callback_page(
            "Sign-in Incomplete",
            "Microsoft returned to Vertex Launcher, but no authorization code was included.",
            Some("You can close this tab and retry the sign-in flow."),
            CallbackPageTone::Error,
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
) -> String {
    let accent = match tone {
        CallbackPageTone::Success => "#2d6a4f",
        CallbackPageTone::Error => "#a61e4d",
    };
    let badge = match tone {
        CallbackPageTone::Success => "Connected",
        CallbackPageTone::Error => "Action needed",
    };
    let detail_html = detail
        .map(|detail| format!(r#"<p class="detail">{}</p>"#, escape_html(detail)))
        .unwrap_or_default();
    let title = escape_html(title);
    let message = escape_html(message);

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
  <style>
    :root {{
      color-scheme: light;
      --bg: #f4f1ea;
      --panel: rgba(255, 252, 246, 0.94);
      --text: #1d1b19;
      --muted: #625b53;
      --accent: {accent};
      --border: rgba(29, 27, 25, 0.12);
      --shadow: 0 22px 60px rgba(30, 20, 10, 0.16);
    }}
    * {{ box-sizing: border-box; }}
    body {{
      margin: 0;
      min-height: 100vh;
      display: grid;
      place-items: center;
      padding: 24px;
      font-family: "Segoe UI", "Helvetica Neue", Arial, sans-serif;
      background:
        radial-gradient(circle at top, rgba(255,255,255,0.9), transparent 42%),
        linear-gradient(160deg, #e8e1d3 0%, var(--bg) 52%, #ddd4c3 100%);
      color: var(--text);
    }}
    .panel {{
      width: min(560px, 100%);
      background: var(--panel);
      border: 1px solid var(--border);
      border-radius: 24px;
      box-shadow: var(--shadow);
      overflow: hidden;
      backdrop-filter: blur(10px);
    }}
    .stripe {{
      height: 10px;
      background: linear-gradient(90deg, var(--accent), color-mix(in srgb, var(--accent) 36%, white));
    }}
    .content {{
      padding: 28px 30px 26px;
    }}
    .badge {{
      display: inline-flex;
      align-items: center;
      gap: 8px;
      padding: 7px 12px;
      border-radius: 999px;
      background: color-mix(in srgb, var(--accent) 14%, white);
      color: var(--accent);
      font-size: 13px;
      font-weight: 700;
      letter-spacing: 0.04em;
      text-transform: uppercase;
    }}
    h1 {{
      margin: 18px 0 10px;
      font-size: clamp(30px, 5vw, 40px);
      line-height: 1.05;
      letter-spacing: -0.04em;
    }}
    p {{
      margin: 0;
      font-size: 16px;
      line-height: 1.6;
      color: var(--muted);
    }}
    .detail {{
      margin-top: 16px;
      padding: 14px 16px;
      border-radius: 16px;
      background: rgba(255,255,255,0.72);
      border: 1px solid var(--border);
      color: var(--text);
      font-size: 14px;
      line-height: 1.55;
      word-break: break-word;
    }}
    .footer {{
      margin-top: 22px;
      display: flex;
      justify-content: space-between;
      gap: 12px;
      align-items: center;
      flex-wrap: wrap;
      font-size: 14px;
      color: var(--muted);
    }}
    .timer {{
      font-weight: 700;
      color: var(--accent);
    }}
    .hint {{
      opacity: 0.9;
    }}
  </style>
</head>
<body>
  <main class="panel">
    <div class="stripe"></div>
    <section class="content">
      <div class="badge">{badge}</div>
      <h1>{title}</h1>
      <p>{message}</p>
      {detail_html}
      <div class="footer">
        <span class="hint">This tab should close automatically.</span>
        <span class="timer">Closing in <span id="countdown">{AUTO_CLOSE_DELAY_SECS}</span>s</span>
      </div>
    </section>
  </main>
  <script>
    let remaining = {AUTO_CLOSE_DELAY_SECS};
    const countdown = document.getElementById("countdown");
    const tick = () => {{
      remaining -= 1;
      if (remaining >= 0 && countdown) countdown.textContent = String(remaining);
      if (remaining <= 0) {{
        window.close();
        setTimeout(() => window.location.replace("about:blank"), 150);
        return;
      }}
      setTimeout(tick, 1000);
    }};
    setTimeout(tick, 1000);
  </script>
</body>
</html>"#
    )
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
