//! The GeekMagic SmallTV's own web API (stock firmware): store an image, show it, set the backlight.
//!
//! The device serves one request at a time and is slow (~0.3 s per small request, ~36-46 ms per KB uploaded), which is
//! why the app keeps its requests few and lets a click cancel an upload in flight. The HTTP is spoken by hand over a
//! plain TCP socket: the device's needs are tiny, and it lets an upload be sent at the device's pace and be cut off at once.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use regex::Regex;
use socket2::SockRef;

use crate::core::errors::DeviceError;
use crate::core::ports::Display;
use crate::util::LockExt;

/// Seconds to reach the device; a switched-off one fails fast.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Seconds to get the reply to a small request: a busy device answers in 2-7 s (0.3 s when idle).
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
/// Seconds to let the device finish storing an uploaded GIF (~3 s in practice).
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(15);
/// The slider range of the device's own settings page.
pub const BRIGHTNESS_RANGE: (i64, i64) = (-10, 100);
const CHUNK: usize = 2048;
const POLL: Duration = Duration::from_millis(100);

/// Devices this process has already put on the Photo Album theme.
static THEME_READY: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

static IMAGE_NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)[\w.\-]+\.(?:gif|jpe?g|png)").unwrap());

fn clamp_brightness(level: f64) -> i64 {
    (level as i64).clamp(BRIGHTNESS_RANGE.0, BRIGHTNESS_RANGE.1)
}

fn invalid_response() -> DeviceError {
    DeviceError::Offline("incomplete or invalid response from the device".into())
}

pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

/// The address of `host`, which is an IP ("192.168.1.18") or, for tests, "ip:port".
pub fn socket_addr(host: &str) -> std::io::Result<SocketAddr> {
    let with_port = if host.contains(':') { host.to_string() } else { format!("{host}:80") };
    with_port.to_socket_addrs()?.next().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no such address"))
}

pub fn connect(host: &str, timeout: Duration) -> std::io::Result<TcpStream> {
    let stream = TcpStream::connect_timeout(&socket_addr(host)?, timeout)?;
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

fn decode_chunked(mut data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = data.windows(2).position(|w| w == b"\r\n")?;
        let size = usize::from_str_radix(std::str::from_utf8(&data[..eol]).ok()?.split(';').next()?.trim(), 16).ok()?;
        data = &data[eol + 2..];
        if size == 0 {
            return Some(out);
        }
        if data.len() < size + 2 {
            return None;
        }
        out.extend_from_slice(&data[..size]);
        data = &data[size + 2..];
    }
}

/// Read a whole HTTP response (the request said `Connection: close`). Gives up after `timeout` overall, or at once when
/// `cancel` is set.
pub fn read_response(stream: &mut TcpStream, timeout: Duration, cancel: Option<&AtomicBool>) -> Result<Response, DeviceError> {
    let deadline = Instant::now() + timeout;
    let _ = stream.set_read_timeout(Some(POLL));
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if cancel.is_some_and(|c| c.load(Ordering::SeqCst)) {
                    return Err(DeviceError::Cancelled);
                }
                if Instant::now() >= deadline {
                    return Err(DeviceError::Offline("timed out waiting for the device".into()));
                }
            }
            Err(e) => return Err(e.into()),
        }
        // (a reply with a Content-Length is complete once its body is in, whether or not the device closes the socket)
        if let Some(head_end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&raw[..head_end]).to_lowercase();
            if let Some(len) = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()) {
                if raw.len() >= head_end + 4 + len {
                    break;
                }
            }
        }
    }
    let head_end = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(invalid_response)?;
    let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
    let status: u16 = head.lines().next().and_then(|l| l.split_whitespace().nth(1)).and_then(|s| s.parse().ok()).ok_or_else(invalid_response)?;
    let mut body = raw[head_end + 4..].to_vec();
    let lowered = head.to_lowercase();
    if lowered.contains("transfer-encoding: chunked") {
        body = decode_chunked(&body).ok_or_else(invalid_response)?;
    } else if let Some(len) = lowered.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()) {
        if body.len() < len {
            return Err(invalid_response());
        }
        body.truncate(len);
    }
    Ok(Response { status, body })
}

/// One screen, at one address. Every method fails with `Offline` when the device doesn't answer properly.
pub struct GeekMagicDevice {
    ip: String,
}

impl GeekMagicDevice {
    pub fn new(ip: &str) -> Self {
        Self { ip: ip.to_string() }
    }

    /// GET `path` and return the response. Reaching the device fails fast, but a reply may take a while.
    fn fetch(&self, path: &str) -> Result<Vec<u8>, DeviceError> {
        let mut stream = connect(&self.ip, CONNECT_TIMEOUT)?;
        stream.set_write_timeout(Some(REPLY_TIMEOUT))?;
        write!(stream, "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", self.ip)?;
        let response = read_response(&mut stream, REPLY_TIMEOUT, None)?;
        if response.status >= 400 {
            return Err(DeviceError::Offline(format!("the device answered HTTP {} to {path}", response.status)));
        }
        Ok(response.body)
    }

    fn get(&self, path: &str) -> Result<String, DeviceError> {
        Ok(String::from_utf8_lossy(&self.fetch(path)?).into_owned())
    }

    /// GET /set?<query> and return what the device answered: "OK", or "FAIL" for what it doesn't understand.
    fn reply(&self, query: &str) -> Result<String, DeviceError> {
        Ok(self.get(&format!("/set?{query}"))?.trim().to_string())
    }

    fn set(&self, query: &str) -> Result<(), DeviceError> {
        let body = self.reply(query)?;
        if body != "OK" {
            return Err(DeviceError::Offline(format!("the device refused the setting ({})", if body.is_empty() { "no answer" } else { &body })));
        }
        Ok(())
    }

    fn show(&self, filename: &str) -> Result<(), DeviceError> {
        if !THEME_READY.lk().contains(&self.ip) {
            self.set("theme=3")?;
            THEME_READY.lk().insert(self.ip.clone());
        }
        if self.reply(&format!("img=/image/{filename}"))? != "OK" {
            self.set("theme=3")?;
            if self.reply(&format!("img=/image/{filename}"))? != "OK" {
                return Err(DeviceError::Offline(format!("the device wouldn't show {filename}")));
            }
        }
        Ok(())
    }
}

/// Abort an upload: reset the connection (RST, dropping what is buffered) so the device stops receiving at once.
fn reset(stream: TcpStream) {
    let _ = SockRef::from(&stream).set_linger(Some(Duration::ZERO));
    drop(stream);
}

impl Display for GeekMagicDevice {
    fn ip(&self) -> &str {
        &self.ip
    }

    /// POST the image to the device. Setting `cancel` aborts the transfer and gives `Cancelled`.
    fn upload(&self, image: &[u8], filename: &str, content_type: &str, cancel: Option<&AtomicBool>) -> Result<(), DeviceError> {
        let boundary = "----geekmagicclaude";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(image);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let cancelled = || cancel.is_some_and(|c| c.load(Ordering::SeqCst));
        if cancelled() {
            return Err(DeviceError::Cancelled);
        }
        let mut stream = connect(&self.ip, CONNECT_TIMEOUT)?;
        // A tiny send buffer makes us send at the device's pace instead of dumping the whole file into the OS buffer at
        // once, so a cancel can still stop the transfer while the device is busy writing.
        let _ = SockRef::from(&stream).set_send_buffer_size(CHUNK);
        stream.set_write_timeout(Some(POLL))?;
        let head = format!(
            "POST /doUpload?dir=/image/ HTTP/1.1\r\nHost: {}\r\nContent-Type: multipart/form-data; boundary={boundary}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.ip,
            body.len()
        );
        let mut last_progress = Instant::now();
        let mut pending: Vec<&[u8]> = vec![head.as_bytes()];
        pending.extend(body.chunks(CHUNK));
        for chunk in pending {
            let mut sent = 0;
            while sent < chunk.len() {
                if cancelled() {
                    reset(stream);
                    return Err(DeviceError::Cancelled);
                }
                match stream.write(&chunk[sent..]) {
                    Ok(0) => return Err(DeviceError::Offline("the device closed the connection".into())),
                    Ok(n) => {
                        sent += n;
                        last_progress = Instant::now();
                    }
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                        // (the device is busy writing: wait for room, as long as it keeps making progress)
                        if last_progress.elapsed() >= UPLOAD_TIMEOUT {
                            return Err(DeviceError::Offline("timed out sending to the device".into()));
                        }
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
        let response = match read_response(&mut stream, UPLOAD_TIMEOUT, cancel) {
            Err(DeviceError::Cancelled) => {
                reset(stream);
                return Err(DeviceError::Cancelled);
            }
            other => other?,
        };
        if response.status >= 300 {
            return Err(DeviceError::Other(format!("Upload failed: HTTP {}", response.status)));
        }
        Ok(())
    }

    /// Names of the image files stored on the device (parsed from its file-list page).
    fn list_images(&self) -> Result<HashSet<String>, DeviceError> {
        let html = self.get("/filelist?dir=/image/")?;
        Ok(IMAGE_NAME_RE.find_iter(&html).map(|m| m.as_str().to_string()).collect())
    }

    fn delete_image(&self, filename: &str) -> Result<(), DeviceError> {
        self.get(&format!("/delete?file=/image/{filename}")).map(|_| ())
    }

    fn download_image(&self, filename: &str) -> Result<Vec<u8>, DeviceError> {
        self.fetch(&format!("/image/{filename}"))
    }

    /// Pin an already-uploaded image on screen. One request (~0.3 s, the device is slow): the Photo Album theme is only
    /// selected the first time, or again if the device ever refuses the image (the theme was changed on the device
    /// itself, or it restarted).
    fn show_image(&self, filename: &str) -> Result<(), DeviceError> {
        let result = self.show(filename);
        if result.is_err() {
            THEME_READY.lk().remove(&self.ip); // it may have restarted: select the theme again next time
        }
        result
    }

    /// Backlight level, -10..100 (the device doesn't let you read the current one back).
    fn set_brightness(&self, level: f64) -> Result<(), DeviceError> {
        self.set(&format!("brt={}", clamp_brightness(level)))
    }

    /// The device's own night schedule: between the two hours it runs at `night_level`, else at `day_level`. It keeps
    /// working with the computer off. (Same call as the "Night Mode" section of the device's settings page.)
    fn set_night_mode(&self, start_hour: i64, end_hour: i64, night_level: f64, day_level: f64, enabled: bool) -> Result<(), DeviceError> {
        self.set(&format!(
            "t1={}&t2={}&b1={}&b2={}&en={}",
            start_hour.rem_euclid(24),
            end_hour.rem_euclid(24),
            clamp_brightness(day_level),
            clamp_brightness(night_level),
            i32::from(enabled)
        ))
    }
}
