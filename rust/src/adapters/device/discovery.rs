//! Find the GeekMagic SmallTV on the local network (stock firmware: it answers GET /space.json).

use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use super::client::{connect, read_response};
use crate::core::ports::DeviceLocator;
use crate::util::LockExt;

/// Seconds per host; hosts that don't exist simply time out. The device itself can take 2 s to answer when busy.
pub const PROBE_TIMEOUT: f64 = 2.0;
/// /24s scanned in one go (the main one first).
pub const MAX_NETWORKS: usize = 4;
pub const WORKERS: usize = 128;

/// True if `host` answers like a SmallTV: /space.json is JSON with the `total` and `free` byte counts.
pub fn probe(host: &str, timeout: f64) -> bool {
    let timeout = Duration::from_secs_f64(timeout);
    let Ok(mut stream) = connect(host, timeout) else { return false };
    if write!(stream, "GET /space.json HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").is_err() {
        return false;
    }
    let Ok(response) = read_response(&mut stream, timeout, None) else { return false };
    if response.status >= 400 {
        return false;
    }
    let body = &response.body[..response.body.len().min(512)];
    serde_json::from_slice::<Value>(body).is_ok_and(|v| v.get("total").is_some() && v.get("free").is_some())
}

/// Private /24 prefixes this machine is on ("192.168.1."), the one used to reach the router first.
pub fn local_networks() -> Vec<String> {
    let mut addresses: Vec<Ipv4Addr> = Vec::new();
    // asks the OS which interface it would use to get out (sends nothing)
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("192.0.2.1:9").is_ok() {
            if let Ok(IpAddr::V4(ip)) = socket.local_addr().map(|a| a.ip()) {
                addresses.push(ip);
            }
        }
    }
    let host = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
    if !host.is_empty() {
        if let Ok(found) = (host.as_str(), 0).to_socket_addrs() {
            addresses.extend(found.filter_map(|a| match a.ip() {
                IpAddr::V4(ip) => Some(ip),
                IpAddr::V6(_) => None,
            }));
        }
    }
    let mut prefixes: Vec<String> = Vec::new();
    for ip in addresses {
        if ip.is_private() && !ip.is_loopback() && !ip.is_link_local() {
            let o = ip.octets();
            let prefix = format!("{}.{}.{}.", o[0], o[1], o[2]);
            if !prefixes.contains(&prefix) {
                prefixes.push(prefix);
            }
        }
    }
    prefixes.truncate(MAX_NETWORKS);
    prefixes
}

/// Every host in `hosts` that answers like a SmallTV, probed in parallel.
fn probe_all(hosts: &[String], timeout: f64, workers: usize) -> Vec<String> {
    let next = Arc::new(AtomicUsize::new(0));
    let hosts: Arc<Vec<String>> = Arc::new(hosts.to_vec());
    let found = Arc::new(Mutex::new(vec![false; hosts.len()]));
    let handles: Vec<_> = (0..workers.min(hosts.len()))
        .filter_map(|_| {
            let (next, hosts, found) = (next.clone(), hosts.clone(), found.clone());
            std::thread::Builder::new()
                .stack_size(128 * 1024)
                .spawn(move || loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(host) = hosts.get(i) else { return };
                    if probe(host, timeout) {
                        found.lk()[i] = true;
                    }
                })
                .ok()
        })
        .collect();
    for h in handles {
        let _ = h.join();
    }
    let flags = found.lk().clone();
    hosts.iter().zip(flags).filter(|(_, ok)| *ok).map(|(h, _)| h.clone()).collect()
}

/// Every host in `hosts` (default: all of this machine's /24 networks, plus the device's own hotspot address) that
/// answers like a SmallTV. The main network is scanned first and, if it has one, we stop there.
pub fn scan(hosts: Option<Vec<String>>, timeout: f64) -> Vec<String> {
    let batches: Vec<Vec<String>> = match hosts {
        Some(h) => vec![h],
        None => {
            let mut b: Vec<Vec<String>> = local_networks().iter().map(|p| (1..=254).map(|n| format!("{p}{n}")).collect()).collect();
            b.push(vec!["192.168.4.1".into()]); // what the device itself answers on when you're on its own WiFi
            b
        }
    };
    for batch in batches {
        let found = probe_all(&batch, timeout, WORKERS);
        if !found.is_empty() {
            return found;
        }
    }
    Vec::new()
}

/// The device's IP: `prefer` if it still answers, else the first one a scan finds.
pub fn find_device(prefer: Option<&str>) -> Option<String> {
    if let Some(p) = prefer {
        if probe(p, 8.0) {
            // (a busy device is slow, not gone)
            return Some(p.to_string());
        }
    }
    let found = scan(None, PROBE_TIMEOUT);
    if let Some(p) = prefer {
        if found.iter().any(|f| f == p) {
            return Some(p.to_string());
        }
    }
    found.into_iter().next()
}

/// The DeviceLocator port, over the functions above.
pub struct Locator;

/// A locator that never scans: it only asks whether the address it already has still answers (`--no-discovery`).
pub struct FixedLocator;

impl DeviceLocator for FixedLocator {
    fn probe(&self, host: &str, timeout: f64) -> bool {
        probe(host, timeout)
    }
    fn find_device(&self, prefer: Option<&str>) -> Option<String> {
        prefer.filter(|p| probe(p, 8.0)).map(String::from)
    }
    fn networks(&self) -> Vec<String> {
        local_networks()
    }
}

impl DeviceLocator for Locator {
    fn probe(&self, host: &str, timeout: f64) -> bool {
        probe(host, timeout)
    }
    fn find_device(&self, prefer: Option<&str>) -> Option<String> {
        find_device(prefer)
    }
    fn networks(&self) -> Vec<String> {
        local_networks()
    }
}
