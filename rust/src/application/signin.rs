//! Providers that can't be read because you're signed out: opening their sign-in, and what their screens say meanwhile.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use chrono::Local;

use super::screen::Screen;
use super::usage::UsageService;
use super::{config, Notify};
use crate::core::model::ErrorScreen;
use crate::core::ports::TerminalLauncher;
use crate::core::registry::ProviderRegistry;
use crate::util::{mono, Event, LockExt};

#[derive(Default)]
struct Inner {
    login_at: HashMap<String, f64>, // provider -> when its sign-in window was last opened (monotonic)
    notes: HashMap<String, String>, // provider -> what its screen says about that window
    missing: HashSet<String>,       // providers whose CLI isn't installed, so there's nothing to sign in to
}

pub struct SignInCoordinator {
    providers: Arc<ProviderRegistry>,
    terminal: Arc<dyn TerminalLauncher>,
    usage: Arc<UsageService>,
    screen: Arc<Screen>,
    notify: Notify,
    wake: Arc<Event>,
    inner: Mutex<Inner>,
}

impl SignInCoordinator {
    pub fn new(providers: Arc<ProviderRegistry>, usage: Arc<UsageService>, screen: Arc<Screen>, notify: Notify, wake: Arc<Event>, terminal: Arc<dyn TerminalLauncher>) -> Self {
        Self { providers, terminal, usage, screen, notify, wake, inner: Mutex::new(Inner::default()) }
    }

    /// Its sign-in window was opened a moment ago and the usage can't be read yet.
    pub fn pending(&self, provider: &str) -> bool {
        let i = self.inner.lk();
        i.notes.contains_key(provider) && mono() - i.login_at.get(provider).copied().unwrap_or(0.0) < config::LOGIN_COOLDOWN
    }

    pub fn note(&self, provider: &str) -> Option<String> {
        self.pending(provider).then(|| self.inner.lk().notes.get(provider).cloned()).flatten()
    }

    /// A read worked: whatever was said about signing in is over.
    pub fn on_read(&self, provider: &str) {
        self.inner.lk().notes.remove(provider);
        self.screen.clear_error_shown(provider);
    }

    /// A read failed: while a sign-in window is open the provider's screen keeps saying so.
    pub fn on_failure(&self, provider: &str, _error: &str) {
        if let Some(note) = self.note(provider) {
            self.usage.set_error(provider, &note);
        }
    }

    /// Open the provider's sign-in in a terminal window when it's signed out (the menu's Options > Sign in does it even if
    /// we can't tell). Not again for a few minutes after, so cycling past it with the click doesn't pile up windows. The
    /// provider's screen says what to do, and fills in by itself once you've signed in.
    pub fn ask(&self, provider: &str, force: bool) {
        if !force && (!self.usage.signed_out(provider) || self.pending(provider)) {
            return;
        }
        let Some(p) = self.providers.get(provider) else { return };
        let title = p.title().to_string();
        let outcome = match p.find_cli() {
            Some(cli) => {
                let mut argv = vec![cli];
                argv.extend(p.login_args().iter().map(|a| a.to_string()));
                self.terminal.launch(&argv, &format!("{title} sign-in"))
            }
            None => "missing".to_string(),
        };
        self.inner.lk().login_at.insert(provider.to_string(), mono());
        let note = match outcome.as_str() {
            "started" => {
                self.inner.lk().missing.remove(provider);
                (self.notify)(&title, "Inicia sesión en la ventana que se abrió");
                format!("A window opened: finish signing in to {title} there. This screen fills in by itself.")
            }
            "missing" => {
                self.inner.lk().missing.insert(provider.to_string());
                (self.notify)(&title, "No se encontró su CLI: instálalo para poder iniciar sesión");
                format!("{title}'s CLI isn't installed. {}", p.install_hint())
            }
            _ => format!("Couldn't open a terminal. Sign in to {title} by running its CLI yourself."),
        };
        log::info!("sign-in for {provider}: {outcome}");
        self.inner.lk().notes.insert(provider.to_string(), note.clone());
        self.usage.set_error(provider, &note);
        self.screen.clear_error_shown(provider);
        self.wake.set();
    }

    /// What to put on a provider's screen when it has never been read: the reason it failed. None when there's nothing to
    /// say, or the screen already says it (it would only be uploaded again every cycle for nothing).
    pub fn error_usage(&self, provider: &str) -> Option<ErrorScreen> {
        let message = self.usage.error_of(provider).filter(|m| !m.is_empty())?;
        if self.usage.has_good(provider) {
            return None; // (one that has been read before is dimmed as stale instead)
        }
        if self.screen.error_shown(provider).as_deref() == Some(message.as_str()) && self.screen.has_slot(provider) && !self.screen.is_outdated(provider) {
            return None;
        }
        let signin = self.usage.signed_out(provider) && !self.inner.lk().missing.contains(provider);
        Some(ErrorScreen { title: self.providers.title_of(provider), message, now: Local::now(), signin })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::persistence::Part;
    use crate::application::screen::tests::{rig, Rig};
    use crate::application::usage::tests::{registry, Scripted};
    use crate::application::usage::{AlertTracker, UsageHistory};
    use crate::core::errors::UsageError;
    use crate::core::model::Data;
    use serde_json::json;

    struct Launcher {
        outcome: Mutex<String>,
        launched: Mutex<Vec<(Vec<String>, String)>>,
    }

    impl TerminalLauncher for Launcher {
        fn launch(&self, argv: &[String], title: &str) -> String {
            self.launched.lk().push((argv.to_vec(), title.to_string()));
            self.outcome.lk().clone()
        }
    }

    fn signin(outcome: &str, usage_outcomes: Vec<Result<(f64, f64), UsageError>>) -> (Arc<SignInCoordinator>, Arc<UsageService>, Arc<Launcher>, Rig) {
        let r = rig(Some("10.0.0.5"));
        r.screen.restore(&json!({}));
        let reg = registry(vec![Scripted::new("claude", "Claude", usage_outcomes)]);
        let alerts = Arc::new(AlertTracker::new(reg.clone(), Arc::new(|_, _| {}), Arc::new(|| {})));
        let usage = Arc::new(UsageService::new(reg.clone(), Arc::new(UsageHistory::default()), alerts));
        let launcher = Arc::new(Launcher { outcome: Mutex::new(outcome.into()), launched: Mutex::new(Vec::new()) });
        let s = Arc::new(SignInCoordinator::new(reg, usage.clone(), r.screen.clone(), r.said.notify(), r.wake.clone(), launcher.clone()));
        (s, usage, launcher, r)
    }

    #[test]
    fn signed_out_opens_the_cli_login_once_and_the_screen_says_so() {
        let (s, usage, launcher, r) = signin("started", vec![Err(UsageError::sign_in("log in"))]);
        usage.fetch("claude");
        s.ask("claude", false);
        s.ask("claude", false); // cycling past it again: no second window
        let launched = launcher.launched.lk().clone();
        assert_eq!(launched.len(), 1);
        assert_eq!((launched[0].0.as_slice(), launched[0].1.as_str()), (["/bin/claude".to_string(), "login".to_string()].as_slice(), "Claude sign-in"));
        assert!(s.pending("claude") && r.wake.is_set());
        assert_eq!(r.said.titles(), ["Claude"]);
        let screen = s.error_usage("claude").unwrap();
        assert!(screen.message.starts_with("A window opened") && screen.signin);
    }

    #[test]
    fn nothing_opens_when_signed_in_unless_forced() {
        let (s, _, launcher, _) = signin("started", vec![Ok((1.0, 1.0))]);
        s.ask("claude", false);
        assert!(launcher.launched.lk().is_empty());
        s.ask("claude", true); // the menu's Options > Sign in
        assert_eq!(launcher.launched.lk().len(), 1);
    }

    #[test]
    fn a_missing_cli_or_terminal_is_explained_and_a_good_read_ends_the_note() {
        let (s, usage, _, _) = signin("failed", vec![Err(UsageError::sign_in("log in")), Ok((1.0, 1.0))]);
        usage.fetch("claude");
        s.ask("claude", false);
        assert!(s.error_usage("claude").unwrap().message.starts_with("Couldn't open a terminal"));
        usage.fetch("claude");
        s.on_read("claude");
        assert!(s.note("claude").is_none() && s.error_usage("claude").is_none(), "read fine: nothing to say");
    }

    #[test]
    fn an_already_shown_message_is_not_uploaded_again_and_a_stale_reading_is_not_an_error() {
        let (s, usage, _, r) = signin("failed", vec![Err(UsageError::new("no internet"))]);
        usage.fetch("claude");
        let first = s.error_usage("claude").unwrap();
        assert!(!first.signin, "not a sign-in problem");
        r.screen.deliver("claude", Data::Error(first));
        assert!(s.error_usage("claude").is_none(), "the screen already says it");
        usage.set_error("claude", "something else");
        assert!(s.error_usage("claude").is_some(), "but a new reason is worth a new picture");
    }
}
