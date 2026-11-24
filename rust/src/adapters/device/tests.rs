//! The device client against a fake SmallTV: a tiny HTTP server on localhost that answers like the stock firmware.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::client::*;
use super::discovery::{probe, scan};
use crate::core::errors::DeviceError;
use crate::core::ports::Display;

#[derive(Default)]
struct FakeState {
    requests: Vec<String>,
    uploads: HashMap<String, Vec<u8>>,
    theme_ok: bool,
    refuse_set: bool,
}

struct Fake {
    addr: String,
    state: Arc<Mutex<FakeState>>,
}

fn read_request(stream: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        raw.extend_from_slice(&buf[..n]);
        if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&raw[..end]).into_owned();
            let len = head.to_lowercase().lines().find_map(|l| l.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
            while raw.len() < end + 4 + len {
                let n = stream.read(&mut buf).ok()?;
                if n == 0 {
                    return None;
                }
                raw.extend_from_slice(&buf[..n]);
            }
            return Some((head, raw[end + 4..end + 4 + len].to_vec()));
        }
    }
}

fn respond(stream: &mut TcpStream, status: &str, body: &[u8]) {
    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(body);
}

fn fake_device() -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let state = Arc::new(Mutex::new(FakeState::default()));
    let shared = state.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let Some((head, body)) = read_request(&mut stream) else { continue };
            let line = head.lines().next().unwrap_or("").to_string();
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let mut st = shared.lock().unwrap();
            st.requests.push(line.clone());
            if path.starts_with("/space.json") {
                respond(&mut stream, "200 OK", br#"{"total":1048576,"free":2048}"#);
            } else if path.starts_with("/set?theme=3") {
                st.theme_ok = true;
                respond(&mut stream, "200 OK", b"OK");
            } else if path.starts_with("/set?img=") {
                respond(&mut stream, "200 OK", if st.theme_ok { b"OK" } else { b"FAIL" });
            } else if path.starts_with("/set?") {
                respond(&mut stream, "200 OK", if st.refuse_set { b"FAIL" } else { b"OK" });
            } else if path.starts_with("/filelist") {
                respond(&mut stream, "200 OK", b"<html><a href='/image/claude-usage-a.gif'>claude-usage-a.gif</a> <b>x.JPG</b> junk.txt</html>");
            } else if path.starts_with("/doUpload") {
                let text = String::from_utf8_lossy(&body).into_owned();
                let name = text.split("filename=\"").nth(1).and_then(|s| s.split('"').next()).unwrap_or("?").to_string();
                let start = body.windows(4).position(|w| w == b"\r\n\r\n").map_or(0, |p| p + 4);
                let end = body.len().saturating_sub(b"\r\n------geekmagicclaude--\r\n".len());
                st.uploads.insert(name, body[start..end].to_vec());
                respond(&mut stream, "200 OK", b"ok");
            } else if let Some(name) = path.strip_prefix("/image/") {
                match st.uploads.get(name) {
                    Some(bytes) => respond(&mut stream, "200 OK", bytes),
                    None => respond(&mut stream, "404 Not Found", b"nope"),
                }
            } else if path.starts_with("/delete") {
                respond(&mut stream, "200 OK", b"OK");
            } else {
                respond(&mut stream, "404 Not Found", b"");
            }
        }
    });
    Fake { addr, state }
}

#[test]
fn probe_recognises_a_smalltv_and_nothing_else() {
    let fake = fake_device();
    assert!(probe(&fake.addr, 2.0));
    let other = TcpListener::bind("127.0.0.1:0").unwrap();
    let other_addr = other.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for mut s in other.incoming().flatten() {
            let _ = read_request(&mut s);
            respond(&mut s, "200 OK", b"<html>a router</html>");
        }
    });
    assert!(!probe(&other_addr, 2.0), "an answer that is not the device's JSON");
    drop(TcpListener::bind("127.0.0.1:0").map(|l| l.local_addr().unwrap().to_string()).map(|a| assert!(!probe(&a, 0.3), "nobody listening")));
    assert_eq!(scan(Some(vec![fake.addr.clone(), "127.0.0.1:9".into()]), 0.5), vec![fake.addr.clone()]);
}

#[test]
fn upload_round_trips_and_lists_and_downloads() {
    let fake = fake_device();
    let device = GeekMagicDevice::new(&fake.addr);
    let gif: Vec<u8> = (0..30_000u32).map(|i| (i % 251) as u8).collect();
    device.upload(&gif, "claude-usage-a.gif", "image/gif", None).unwrap();
    assert_eq!(fake.state.lock().unwrap().uploads["claude-usage-a.gif"], gif, "every byte arrived");
    assert_eq!(device.download_image("claude-usage-a.gif").unwrap(), gif);
    let names = device.list_images().unwrap();
    assert!(names.contains("claude-usage-a.gif") && names.contains("x.JPG") && !names.contains("junk.txt"), "{names:?}");
    assert!(matches!(device.download_image("missing.gif"), Err(DeviceError::Offline(m)) if m.contains("404")));
    device.delete_image("old.gif").unwrap();
}

#[test]
fn show_selects_the_theme_once_and_again_when_refused() {
    let fake = fake_device();
    let device = GeekMagicDevice::new(&fake.addr);
    device.show_image("a.gif").unwrap();
    device.show_image("a.gif").unwrap();
    let requests = fake.state.lock().unwrap().requests.clone();
    assert_eq!(requests.iter().filter(|r| r.contains("theme=3")).count(), 1, "{requests:?}");
    assert_eq!(requests.iter().filter(|r| r.contains("img=/image/a.gif")).count(), 2);
    // the device forgot the theme (it restarted): one refusal, one theme request, then it works
    fake.state.lock().unwrap().theme_ok = false;
    device.show_image("a.gif").unwrap();
    assert_eq!(fake.state.lock().unwrap().requests.iter().filter(|r| r.contains("theme=3")).count(), 2);
}

#[test]
fn backlight_calls_are_clamped_and_a_refusal_is_an_error() {
    let fake = fake_device();
    let device = GeekMagicDevice::new(&fake.addr);
    device.set_brightness(250.0).unwrap();
    device.set_brightness(-50.0).unwrap();
    device.set_night_mode(22, 31, 10.0, 75.0, true).unwrap();
    let requests = fake.state.lock().unwrap().requests.join("\n");
    assert!(requests.contains("brt=100") && requests.contains("brt=-10"), "{requests}");
    assert!(requests.contains("t1=22&t2=7&b1=75&b2=10&en=1"), "{requests}");
    fake.state.lock().unwrap().refuse_set = true;
    assert!(matches!(device.set_brightness(5.0), Err(DeviceError::Offline(m)) if m.contains("refused")));
}

#[test]
fn a_device_that_is_not_there_fails_fast_as_offline() {
    let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().to_string();
    let started = Instant::now();
    assert!(matches!(GeekMagicDevice::new(&free).show_image("a.gif"), Err(DeviceError::Offline(_))));
    assert!(started.elapsed() < Duration::from_secs(4));
}

#[test]
fn a_cancel_stops_an_upload_in_flight() {
    // a device that accepts the connection and then never reads: the upload would wait for it forever
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let held = Arc::new(Mutex::new(Vec::new()));
    let keep = held.clone();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            keep.lock().unwrap().push(s);
        }
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let trigger = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        flag.store(true, Ordering::SeqCst);
    });
    let started = Instant::now();
    let big = vec![7u8; 8 * 1024 * 1024];
    let result = GeekMagicDevice::new(&addr).upload(&big, "x.gif", "image/gif", Some(&cancel));
    trigger.join().unwrap();
    assert!(matches!(result, Err(DeviceError::Cancelled)), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(3), "stopped promptly: {:?}", started.elapsed());
    let before = GeekMagicDevice::new(&addr).upload(&[1, 2, 3], "y.gif", "image/gif", Some(&cancel));
    assert!(matches!(before, Err(DeviceError::Cancelled)), "already cancelled: nothing is even sent");
}
