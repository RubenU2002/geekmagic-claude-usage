//! The worker loop: read the usage, put it on the screen, and keep the screens that aren't showing fresh.
//!
//! One cycle reads what's on screen (or both providers, for a view of two), uploads it, then refreshes at most one other
//! thing, so the device (which serves one request at a time) is left free for the next click.
//!
//! What starts a cycle is an event, not a timer: a webhook, a click, an agent starting or stopping, the screen coming
//! back. The only timer is the slow fallback `interval` (none when it is 0), so the screen is still refreshed now and
//! then if no webhook ever arrives.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::time::Duration;

use super::activity::ActivityCounter;
use super::agents::AgentMonitor;
use super::power::Power;
use super::screen::{Outcome, Screen};
use super::signin::SignInCoordinator;
use super::usage::UsageService;
use super::viewstate::ViewState;
use super::config;
use crate::core::model::{Data, Usage, Window};
use crate::core::registry::ProviderRegistry;
use crate::core::views::{is_panel_key, is_stats_view, SPLIT};
use crate::util::{mono, Event};

/// Waiting for an event with no timer at all: a day is as good as forever, and wakes the loop up now and then.
const NO_TIMER: f64 = 86_400.0;

pub struct Scheduler {
    providers: Arc<ProviderRegistry>,
    interval: f64, // the slow fallback refresh; 0 = only events
    usage: Arc<UsageService>,
    screen: Arc<Screen>,
    view: Arc<ViewState>,
    power: Arc<Power>,
    agents: Arc<AgentMonitor>,
    activity: Arc<ActivityCounter>,
    signin: Arc<SignInCoordinator>,
    wake: Arc<Event>,
    stop: Arc<Event>,
    tooltip: Arc<dyn Fn() -> String + Send + Sync>,
    set_title: Arc<dyn Fn(&str) + Send + Sync>,
}

/// A reading awaiting delivery: (provider, what to draw, read at).
type Pending = Option<(String, Data, f64)>;

impl Scheduler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        providers: Arc<ProviderRegistry>,
        interval: f64,
        usage: Arc<UsageService>,
        screen: Arc<Screen>,
        view: Arc<ViewState>,
        power: Arc<Power>,
        agents: Arc<AgentMonitor>,
        activity: Arc<ActivityCounter>,
        signin: Arc<SignInCoordinator>,
        wake: Arc<Event>,
        stop: Arc<Event>,
        tooltip: Arc<dyn Fn() -> String + Send + Sync>,
        set_title: Arc<dyn Fn(&str) + Send + Sync>,
    ) -> Self {
        Self { providers, interval, usage, screen, view, power, agents, activity, signin, wake, stop, tooltip, set_title }
    }

    fn heartbeat(&self) -> f64 {
        if self.interval > 0.0 {
            self.interval
        } else {
            NO_TIMER
        }
    }

    // --- reading what's needed for a view -------------------------------------------------------------------

    /// Read both providers at the same time (Claude's `/usage` takes ~3.5 s, Codex's ~1 s): a view of both waits for the
    /// slower one instead of for the two added up.
    pub fn fetch_all(&self) {
        std::thread::scope(|scope| {
            for provider in self.providers.keys() {
                let usage = &self.usage;
                std::thread::Builder::new().stack_size(512 * 1024).spawn_scoped(scope, move || {
                    usage.fetch(&provider);
                }).ok();
            }
        });
    }

    /// The two providers' latest readings for a view of both. One whose read failed keeps its last good numbers, dimmed and
    /// dated once they're a few minutes old; one that has never been read keeps its panel, with dashes.
    pub fn panels(&self, key: &str) -> Vec<Usage> {
        let mut panels = Vec::new();
        let anything = self.providers.iter().any(|p| self.usage.has_good(p.key())); // (two panels of dashes would say nothing at all)
        for provider in self.providers.iter() {
            let provider = provider.key();
            let mut usage = match self.usage.last_good(provider) {
                None => {
                    if self.usage.has_error(provider) && anything {
                        let mut empty = Usage::new(&self.providers.title_of(provider), Window::default(), Window::default(), chrono::Local::now());
                        if is_stats_view(key) {
                            empty.activity = self.activity.activity_of(provider);
                        }
                        panels.push(empty);
                    }
                    continue;
                }
                Some((usage, read_at)) => {
                    let mut usage = usage;
                    if mono() - read_at >= config::STALE_SECONDS {
                        usage.stale = true;
                    }
                    usage
                }
            };
            if is_stats_view(key) {
                usage.activity = self.activity.activity_of(provider);
            }
            panels.push(usage);
        }
        panels
    }

    /// A view of both providers: read both (so both get alerts) and upload one screen with the two of them. `refetch =
    /// false` redraws from the readings we already have (only an agent started or stopped).
    pub fn update_panels(&self, key: &str, refetch: bool) -> Outcome {
        if refetch {
            self.fetch_all();
        }
        if is_stats_view(key) {
            self.activity.refresh();
        }
        let panels = self.panels(key);
        if panels.is_empty() {
            return Outcome::Error;
        }
        self.screen.deliver(key, Data::Panels(panels))
    }

    pub fn update_split(&self) -> Outcome {
        self.update_panels(SPLIT, true)
    }

    fn recently_read(&self, provider: &str) -> bool {
        self.usage.recently_read(provider, config::REUSE_WITHIN)
    }

    // --- keeping the other screens fresh --------------------------------------------------------------------

    /// Numbers that can't be refreshed for a few minutes: put the last ones on screen, dimmed and dated.
    pub fn mark_stale(&self, provider: &str) {
        let s = &self.screen;
        if s.ip().is_empty() || s.is_offline() || self.view.mode().is_some() {
            return; // (the views of two dim stale panels themselves, see panels)
        }
        let Some(mut usage) = self.usage.stale_candidate(provider) else { return };
        self.usage.mark_stale(provider);
        log::warn!("no fresh {provider} data for {} s; showing the last reading dimmed", config::STALE_SECONDS);
        usage.stale = true;
        if let Err(e) = s.upload(provider, Data::One(usage)) {
            log::warn!("could not upload the stale image ({provider}): {e}");
        }
    }

    /// The provider that isn't on screen: read it now and then (its alerts matter too) and keep its image fresh. One per
    /// cycle.
    pub fn refresh_other(&self, provider: Option<&str>) {
        let s = &self.screen;
        for other in self.providers.keys().into_iter().filter(|p| Some(p.as_str()) != provider) {
            // nothing (or only an out-of-date image) of it on the device yet: do it now, so the first click is instant too
            let needs_image = (!s.has_slot(&other) || s.is_outdated(&other)) && !s.is_offline();
            if !needs_image && mono() - self.usage.last_fetch(&other) < config::OTHER_REFRESH {
                continue;
            }
            let data = self.usage.fetch(&other).map(Data::One).or_else(|| self.signin.error_usage(&other).map(Data::Error));
            if let Some(data) = data {
                if !s.ip().is_empty() && !s.is_offline() {
                    if let Err(e) = s.upload(&other, data) {
                        log::warn!("could not upload the {other} image: {e}");
                    }
                }
            }
            return;
        }
    }

    /// After an update the images stored on the device were drawn by the old code (old labels, colours...), and a view is
    /// only redrawn while it's on screen. Redraw the ones not on screen, from the latest readings, without showing them.
    /// One per cycle, so the device stays free for clicks.
    pub fn refresh_outdated_views(&self) {
        let s = &self.screen;
        if s.is_offline() || s.ip().is_empty() {
            return;
        }
        let on_screen = self.view.key();
        for key in s.outdated_keys().into_iter().filter(|k| *k != on_screen) {
            let data = if is_panel_key(&key) {
                if is_stats_view(&key) {
                    self.activity.refresh();
                }
                let panels = self.panels(&key);
                // (half a view would be stored as if it were whole)
                (panels.len() == self.providers.len()).then_some(Data::Panels(panels))
            } else {
                self.usage.last_good(&key).map(|(usage, _)| Data::One(usage))
            };
            let Some(data) = data else { continue }; // nothing read for it yet; a later cycle will have something
            if let Err(e) = s.upload(&key, data) {
                log::warn!("could not refresh the stored {key} image: {e}");
            }
            return;
        }
    }

    // --- the loop -------------------------------------------------------------------------------------------

    pub fn run(&self) {
        let s = &self.screen;
        let mut pending: Pending = None;
        if !s.ip().is_empty() {
            // the remembered view goes up first, in one request: it needs neither the search nor the file list
            s.show_uploaded(&self.view.key());
        }
        s.resolve_device();
        s.sync_with_device();
        if !s.has_slot(&self.view.key()) || s.shown_failed() {
            s.show_uploaded(&self.view.key()); // (the address changed, or the first try didn't get through)
        }
        while !self.stop.is_set() {
            if catch_unwind(AssertUnwindSafe(|| self.cycle(&mut pending))).is_err() {
                log::error!("the update cycle crashed; trying again shortly");
                self.wake.wait(config::RETRY_SECONDS);
            }
        }
    }

    /// One turn of the loop: do what is due, then wait for the next event.
    fn cycle(&self, pending: &mut Pending) {
        let s = &self.screen;
        self.wake.clear();
        // An upload keeps the device busy for seconds; wait out rapid toggling so a click never queues behind it.
        while !self.stop.is_set() && mono() - s.last_click() < config::SETTLE_SECONDS {
            std::thread::sleep(Duration::from_millis(500));
        }
        if self.power.is_paused() {
            // nothing is read or uploaded; coming back (resume / unlock) wakes the loop
            (self.set_title)(&(self.tooltip)());
            self.wake.wait(config::PAUSE_POLL);
            return;
        }
        let provider = self.view.provider();
        let mode = self.view.mode();
        let mut wait = self.heartbeat();
        let watched = if mode.is_some() { self.providers.keys() } else { vec![provider.clone()] };
        if watched.iter().any(|p| self.signin.pending(p)) {
            wait = config::LOGIN_POLL; // someone is signing in right now: notice it as soon as it works
        }
        // An agent started or stopped (and nothing else changed): redraw from the readings we already have.
        let redraw_only = self.agents.take_dirty();
        if let Some(mode) = mode {
            let recent = self.providers.keys().iter().all(|p| self.recently_read(p));
            if self.update_panels(&mode, !(redraw_only && recent)) == Outcome::Offline {
                wait = config::RETRY_SECONDS;
                s.maybe_rediscover();
            }
            self.refresh_outdated_views();
            self.wake.wait(wait);
            return;
        }
        let data: Option<Data> = match pending.take() {
            // read moments ago but not delivered yet: don't query again
            Some((p, data, at)) if p == provider && mono() - at < config::REUSE_WITHIN => Some(data),
            _ if redraw_only && self.recently_read(&provider) => self.usage.last_good(&provider).map(|(u, _)| Data::One(u)),
            _ => self.usage.fetch(&provider).map(Data::One).or_else(|| self.signin.error_usage(&provider).map(Data::Error)),
        };
        if let Some(data) = data {
            match s.deliver(&provider, data.clone()) {
                // retry with the same data once the clicking settles
                Outcome::Cancelled => *pending = Some((provider.clone(), data, mono())),
                // retry soon, without querying usage again
                Outcome::Offline => {
                    *pending = Some((provider.clone(), data, mono()));
                    wait = config::RETRY_SECONDS;
                    s.maybe_rediscover();
                }
                Outcome::Ok | Outcome::Error => {}
            }
        }
        self.refresh_other(Some(&provider));
        self.refresh_outdated_views();
        self.wake.wait(wait);
    }
}
