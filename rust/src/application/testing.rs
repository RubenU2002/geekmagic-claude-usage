//! Fakes for the application's tests: a screen, a locator, a renderer and a notifier that record what they were asked and
//! fail on demand. No desktop, no network, no CLI.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value};

use crate::core::errors::DeviceError;
use crate::core::model::Data;
use crate::core::ports::{DeviceLocator, Display, Rendered, Renderer};
use crate::util::LockExt;

#[derive(Default)]
pub struct FakeDisplay {
    pub address: String,
    pub calls: Mutex<Vec<String>>,
    pub stored: Mutex<HashSet<String>>,
    pub fail: Mutex<Option<DeviceError>>,
    pub cancel_uploads: AtomicBool,
    pub downloads: Mutex<Vec<u8>>,
}

impl FakeDisplay {
    pub fn new(ip: &str) -> Arc<Self> {
        Arc::new(Self { address: ip.into(), ..Default::default() })
    }
    pub fn calls(&self) -> Vec<String> {
        self.calls.lk().clone()
    }
    pub fn failing_with(&self, e: Option<DeviceError>) {
        *self.fail.lk() = e;
    }
    fn record(&self, call: String) -> Result<(), DeviceError> {
        self.calls.lk().push(call);
        match self.fail.lk().clone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl Display for FakeDisplay {
    fn ip(&self) -> &str {
        &self.address
    }
    fn upload(&self, _image: &[u8], filename: &str, _ct: &str, _cancel: Option<&AtomicBool>) -> Result<(), DeviceError> {
        self.record(format!("upload {filename}"))?;
        if self.cancel_uploads.load(Ordering::SeqCst) {
            return Err(DeviceError::Cancelled);
        }
        self.stored.lk().insert(filename.into());
        Ok(())
    }
    fn list_images(&self) -> Result<HashSet<String>, DeviceError> {
        self.record("list".into())?;
        Ok(self.stored.lk().clone())
    }
    fn delete_image(&self, filename: &str) -> Result<(), DeviceError> {
        self.record(format!("delete {filename}"))?;
        self.stored.lk().remove(filename);
        Ok(())
    }
    fn download_image(&self, filename: &str) -> Result<Vec<u8>, DeviceError> {
        self.record(format!("download {filename}"))?;
        Ok(self.downloads.lk().clone())
    }
    fn show_image(&self, filename: &str) -> Result<(), DeviceError> {
        self.record(format!("show {filename}"))
    }
    fn set_brightness(&self, level: f64) -> Result<(), DeviceError> {
        self.record(format!("brightness {level}"))
    }
    fn set_night_mode(&self, start: i64, end: i64, night: f64, day: f64, enabled: bool) -> Result<(), DeviceError> {
        self.record(format!("night {start} {end} {night} {day} {enabled}"))
    }
}

#[derive(Default)]
pub struct FakeLocator {
    pub answering: Mutex<HashSet<String>>,
    pub found: Mutex<Option<String>>,
    pub nets: Mutex<Vec<String>>,
    pub scans: Mutex<usize>,
}

impl FakeLocator {
    pub fn answering(hosts: &[&str]) -> Arc<Self> {
        let l = Self::default();
        *l.answering.lk() = hosts.iter().map(|h| h.to_string()).collect();
        Arc::new(l)
    }
}

impl DeviceLocator for FakeLocator {
    fn probe(&self, host: &str, _timeout: f64) -> bool {
        self.answering.lk().contains(host)
    }
    fn find_device(&self, _prefer: Option<&str>) -> Option<String> {
        *self.scans.lk() += 1;
        self.found.lk().clone()
    }
    fn networks(&self) -> Vec<String> {
        self.nets.lk().clone()
    }
}

#[derive(Default)]
pub struct FakeRenderer {
    pub rendered: Mutex<Vec<String>>,
    pub confirmed: Mutex<usize>,
    pub fail: AtomicBool,
    pub saved: Mutex<Map<String, Value>>,
}

impl Renderer for FakeRenderer {
    fn version(&self) -> &str {
        "test-render"
    }
    fn render(&self, key: &str, data: &Data, _animation: &str) -> Result<Rendered, String> {
        if self.fail.load(Ordering::SeqCst) {
            return Err("cannot draw".into());
        }
        let what = match data {
            Data::One(u) => format!("{key}:{}{}", u.title, if u.working { ":working" } else { "" }),
            Data::Panels(p) => format!("{key}:{}", p.iter().map(|u| u.title.as_str()).collect::<Vec<_>>().join("+")),
            Data::Error(e) => format!("{key}:error:{}", e.message),
        };
        self.rendered.lk().push(what);
        Ok(Rendered { gif: b"GIF89a".to_vec(), animations: vec!["idle".into()], picks: vec![] })
    }
    fn to_png(&self, gif: &[u8]) -> Result<Vec<u8>, String> {
        Ok([b"PNG:".as_slice(), gif].concat())
    }
    fn confirm(&self, _rendered: &Rendered) {
        *self.confirmed.lk() += 1;
    }
    fn restore(&self, _saved: &Value) {}
    fn snapshot(&self) -> Map<String, Value> {
        self.saved.lk().clone()
    }
}

/// Collects what the app told the user.
#[derive(Default)]
pub struct Said(pub Mutex<Vec<(String, String)>>);

impl Said {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    pub fn notify(self: &Arc<Self>) -> super::Notify {
        let me = self.clone();
        Arc::new(move |t, m| me.0.lk().push((t.into(), m.into())))
    }
    pub fn all(&self) -> Vec<(String, String)> {
        self.0.lk().clone()
    }
    pub fn titles(&self) -> Vec<String> {
        self.0.lk().iter().map(|(t, _)| t.clone()).collect()
    }
}
