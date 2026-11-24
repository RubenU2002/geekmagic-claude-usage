//! A "tray" with no icon: the app runs with webhooks only (`--no-tray`, and wherever there is no tray implementation).

use std::sync::Arc;

use crate::core::ports::Ui;
use crate::util::Event;

pub struct HeadlessUi {
    stop: Arc<Event>,
}

impl HeadlessUi {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { stop: Arc::new(Event::new()) })
    }
}

impl Ui for HeadlessUi {
    fn set_title(&self, title: &str) {
        log::debug!("title: {title}");
    }
    fn set_icon(&self, key: &str) {
        log::debug!("icon: {key}");
    }
    fn refresh_menu(&self) {}
    fn ask_save_path(&self, _suggested_name: &str) -> Option<String> {
        None // (no window to ask in)
    }
    fn run(&self) {
        while !self.stop.wait(3600.0) {}
    }
    fn stop(&self) {
        self.stop.set();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_until_stopped() {
        let ui = HeadlessUi::new();
        let runner = ui.clone();
        let t = std::thread::spawn(move || runner.run());
        std::thread::sleep(std::time::Duration::from_millis(50));
        ui.stop();
        t.join().unwrap();
        assert_eq!(ui.ask_save_path("x.png"), None);
    }
}
