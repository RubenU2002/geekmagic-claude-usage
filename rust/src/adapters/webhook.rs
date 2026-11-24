//! The webhook: a tiny HTTP server on localhost that tells the app something happened, so it refreshes the screen then and
//! not on a timer.
//!
//! ```text
//! POST /refresh                      read every provider's usage again (or `?provider=claude`)
//! POST /hook/<provider>/<event>      a tool says something happened: `/hook/claude/Stop`, `/hook/claude/UserPromptSubmit`
//! POST /hook/<provider>              the same, the event named in the body (`{"hook_event_name": "Stop"}`) or `?event=`
//! GET  /health                       "ok"
//! ```
//!
//! It only listens on 127.0.0.1, only answers a request whose `Host` is the loopback address it was sent to (a web page can't
//! reach it through DNS rebinding), and, when a token is set, only one that carries it in `X-GeekMagic-Token`.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::application::controller::WebhookEvent;

pub const MAX_BODY: usize = 64 * 1024;
const MAX_HEAD: usize = 16 * 1024;
const MAX_CONNECTIONS: usize = 8;
const READ_TIMEOUT: Duration = Duration::from_secs(3);
pub const TOKEN_HEADER: &str = "X-GeekMagic-Token";

pub type Handler = Arc<dyn Fn(WebhookEvent) + Send + Sync>;

pub struct WebhookServer {
    pub addr: SocketAddr,
}

struct Request {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    host: String,
    token: Option<String>,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> Result<Request, &'static str> {
    let deadline = Instant::now() + READ_TIMEOUT;
    let mut raw = Vec::new();
    let mut buf = [0u8; 2048];
    let head_end = loop {
        if let Some(p) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        if raw.len() > MAX_HEAD {
            return Err("431 Request Header Fields Too Large");
        }
        stream.set_read_timeout(Some(deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1)))).map_err(|_| "500 Internal Server Error")?;
        match stream.read(&mut buf) {
            Ok(0) => return Err("400 Bad Request"),
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(_) => return Err("408 Request Timeout"),
        }
    };
    let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let (method, target) = (first.next().unwrap_or("").to_string(), first.next().unwrap_or("").to_string());
    if method.is_empty() || target.is_empty() {
        return Err("400 Bad Request");
    }
    let header = |name: &str| {
        lines.clone().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    };
    let length: usize = header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    if length > MAX_BODY {
        return Err("413 Payload Too Large");
    }
    let mut body = raw[head_end + 4..].to_vec();
    while body.len() < length {
        stream.set_read_timeout(Some(deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1)))).map_err(|_| "500 Internal Server Error")?;
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&buf[..n]),
            Err(_) => return Err("408 Request Timeout"),
        }
    }
    body.truncate(length);
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()),
        None => (target, Vec::new()),
    };
    Ok(Request { method, path, query, host: header("host").unwrap_or_default(), token: header(TOKEN_HEADER), body })
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
}

/// What a request asks for, or the status to refuse it with.
fn route(req: &Request, port: u16, token: Option<&str>, providers: &[String]) -> Result<Option<WebhookEvent>, &'static str> {
    let loopback = [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")];
    if !loopback.iter().any(|h| h.eq_ignore_ascii_case(&req.host)) {
        return Err("403 Forbidden");
    }
    if let Some(expected) = token.filter(|t| !t.is_empty()) {
        if req.token.as_deref() != Some(expected) {
            return Err("401 Unauthorized");
        }
    }
    let query = |name: &str| req.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
    let segments: Vec<&str> = req.path.trim_matches('/').split('/').collect();
    match (req.method.as_str(), segments.as_slice()) {
        ("GET", ["health"]) => Ok(None),
        ("POST", ["refresh"]) => {
            let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let provider = query("provider").or_else(|| body.get("provider").and_then(Value::as_str).map(String::from));
            Ok(Some(WebhookEvent::Refresh { provider }))
        }
        ("POST", ["hook", provider, rest @ ..]) if rest.len() <= 1 => {
            let provider = provider.to_lowercase();
            if !providers.contains(&provider) {
                return Err("404 Not Found");
            }
            let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let event = rest
                .first()
                .map(|e| e.to_string())
                .or_else(|| query("event"))
                .or_else(|| body.get("hook_event_name").and_then(Value::as_str).map(String::from))
                .unwrap_or_else(|| "Stop".into());
            Ok(Some(WebhookEvent::Hook { provider, event }))
        }
        ("GET" | "POST", _) => Err("404 Not Found"),
        _ => Err("405 Method Not Allowed"),
    }
}

fn serve(mut stream: TcpStream, port: u16, token: Option<&str>, providers: &[String], handler: &Handler) {
    let outcome = read_request(&mut stream).and_then(|req| route(&req, port, token, providers));
    match outcome {
        Ok(Some(event)) => {
            handler(event);
            respond(&mut stream, "202 Accepted", "{\"ok\":true}");
        }
        Ok(None) => respond(&mut stream, "200 OK", "{\"ok\":true}"),
        Err(status) => respond(&mut stream, status, &format!("{{\"error\":\"{status}\"}}")),
    }
}

/// Start listening on 127.0.0.1:`port` (0: any free port). `providers` are the keys a hook may name.
pub fn start(port: u16, token: Option<String>, providers: Vec<String>, handler: Handler) -> std::io::Result<WebhookServer> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))?;
    let addr = listener.local_addr()?;
    let busy = Arc::new(AtomicUsize::new(0));
    let (providers, token) = (Arc::new(providers), Arc::new(token));
    std::thread::Builder::new().name("webhook".into()).stack_size(256 * 1024).spawn(move || {
        for stream in listener.incoming().flatten() {
            if busy.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                busy.fetch_sub(1, Ordering::SeqCst);
                let mut stream = stream;
                respond(&mut stream, "503 Service Unavailable", "{\"error\":\"busy\"}");
                continue;
            }
            let (done, providers, token, handler) = (busy.clone(), providers.clone(), token.clone(), handler.clone());
            let spawned = std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
                serve(stream, addr.port(), token.as_deref(), &providers, &handler);
                done.fetch_sub(1, Ordering::SeqCst);
            });
            if spawned.is_err() {
                busy.fetch_sub(1, Ordering::SeqCst);
            }
        }
    })?;
    Ok(WebhookServer { addr })
}

/// `geekmagic-usage notify <provider> <event>`: tell a running app (what a tool's hook runs). Never fails loudly: a hook that
/// errors would get in the way of the tool it hooks into, so an app that isn't running is simply not told.
pub fn notify(port: u16, token: Option<&str>, provider: &str, event: &str) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), Duration::from_millis(500)) else { return false };
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let token_line = token.map_or(String::new(), |t| format!("{TOKEN_HEADER}: {t}\r\n"));
    let sent = write!(stream, "POST /hook/{provider}/{event} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{token_line}Content-Length: 0\r\nConnection: close\r\n\r\n").is_ok();
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut reply = String::new();
    let _ = stream.read_to_string(&mut reply);
    sent && reply.starts_with("HTTP/1.1 202")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::LockExt;
    use std::sync::Mutex;

    struct Rig {
        server: WebhookServer,
        events: Arc<Mutex<Vec<WebhookEvent>>>,
    }

    fn rig(token: Option<&str>) -> Rig {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let server = start(0, token.map(String::from), vec!["claude".into(), "codex".into()], Arc::new(move |e| sink.lk().push(e))).unwrap();
        Rig { server, events }
    }

    /// One raw request: (status line, body).
    fn request(addr: SocketAddr, raw: &str) -> (String, String) {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(raw.as_bytes()).unwrap();
        let mut reply = String::new();
        s.read_to_string(&mut reply).unwrap();
        let (head, body) = reply.split_once("\r\n\r\n").unwrap_or((&reply, ""));
        (head.lines().next().unwrap_or("").to_string(), body.to_string())
    }

    fn post(r: &Rig, path: &str, body: &str, extra: &str) -> (String, String) {
        let host = format!("127.0.0.1:{}", r.server.addr.port());
        request(r.server.addr, &format!("POST {path} HTTP/1.1\r\nHost: {host}\r\n{extra}Content-Length: {}\r\n\r\n{body}", body.len()))
    }

    #[test]
    fn hooks_and_refreshes_become_events() {
        let r = rig(None);
        assert_eq!(post(&r, "/hook/claude/Stop", "", "").0, "HTTP/1.1 202 Accepted");
        assert_eq!(post(&r, "/hook/codex", r#"{"hook_event_name":"UserPromptSubmit","session_id":"x"}"#, "").0, "HTTP/1.1 202 Accepted");
        assert_eq!(post(&r, "/hook/claude?event=SessionEnd", "", "").0, "HTTP/1.1 202 Accepted");
        assert_eq!(post(&r, "/hook/Claude", "", "").0, "HTTP/1.1 202 Accepted", "no event named: a turn ended");
        assert_eq!(post(&r, "/refresh", "", "").0, "HTTP/1.1 202 Accepted");
        assert_eq!(post(&r, "/refresh?provider=codex", "", "").0, "HTTP/1.1 202 Accepted");
        assert_eq!(post(&r, "/refresh", r#"{"provider": "claude"}"#, "").0, "HTTP/1.1 202 Accepted");
        let hook = |p: &str, e: &str| WebhookEvent::Hook { provider: p.into(), event: e.into() };
        assert_eq!(
            *r.events.lk(),
            [
                hook("claude", "Stop"),
                hook("codex", "UserPromptSubmit"),
                hook("claude", "SessionEnd"),
                hook("claude", "Stop"),
                WebhookEvent::Refresh { provider: None },
                WebhookEvent::Refresh { provider: Some("codex".into()) },
                WebhookEvent::Refresh { provider: Some("claude".into()) },
            ]
        );
    }

    #[test]
    fn health_and_the_refusals() {
        let r = rig(None);
        let host = format!("127.0.0.1:{}", r.server.addr.port());
        let get = |path: &str, host: &str| request(r.server.addr, &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n")).0;
        assert_eq!(get("/health", &host), "HTTP/1.1 200 OK");
        assert_eq!(get("/nowhere", &host), "HTTP/1.1 404 Not Found");
        assert_eq!(get("/health", "evil.example.com"), "HTTP/1.1 403 Forbidden", "a DNS-rebinding page names another host");
        assert_eq!(post(&r, "/hook/gemini/Stop", "", "").0, "HTTP/1.1 404 Not Found", "not a provider it knows");
        let delete = request(r.server.addr, &format!("DELETE /refresh HTTP/1.1\r\nHost: {host}\r\n\r\n")).0;
        assert_eq!(delete, "HTTP/1.1 405 Method Not Allowed");
        let big = request(r.server.addr, &format!("POST /refresh HTTP/1.1\r\nHost: {host}\r\nContent-Length: 999999\r\n\r\n")).0;
        assert_eq!(big, "HTTP/1.1 413 Payload Too Large");
        assert!(r.events.lk().is_empty(), "none of that reached the app");
    }

    #[test]
    fn a_token_is_required_when_one_is_set() {
        let r = rig(Some("s3cret"));
        assert_eq!(post(&r, "/refresh", "", "").0, "HTTP/1.1 401 Unauthorized");
        assert_eq!(post(&r, "/refresh", "", "X-GeekMagic-Token: wrong\r\n").0, "HTTP/1.1 401 Unauthorized");
        assert_eq!(post(&r, "/refresh", "", "x-geekmagic-token: s3cret\r\n").0, "HTTP/1.1 202 Accepted");
        assert_eq!(r.events.lk().len(), 1);
    }

    #[test]
    fn the_notify_command_tells_a_running_app_and_stays_quiet_when_there_is_none() {
        let r = rig(Some("tok"));
        assert!(notify(r.server.addr.port(), Some("tok"), "claude", "Stop"));
        assert!(!notify(r.server.addr.port(), None, "claude", "Stop"), "refused: no token");
        assert_eq!(r.events.lk().len(), 1);
        let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert!(!notify(free, None, "claude", "Stop"), "nobody listening: false, and no error");
    }

    #[test]
    fn a_stalled_client_does_not_block_the_others() {
        let r = rig(None);
        let _stalled = TcpStream::connect(r.server.addr).unwrap(); // says nothing
        let started = Instant::now();
        assert_eq!(post(&r, "/refresh", "", "").0, "HTTP/1.1 202 Accepted");
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
