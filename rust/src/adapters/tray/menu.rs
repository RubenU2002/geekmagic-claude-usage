//! The tray icon's right-click menu (its labels are in Spanish), described apart from any toolkit: a tree of items, each
//! with what it shows and what it does. `ui.rs` turns it into a real menu.

use crate::application::config;
use crate::application::controller::TrayController;
use crate::core::views::{BREAKDOWN, HOURS, SPLIT, STATS};

/// What choosing a menu item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Select(String),
    Mode(String),
    RefreshNow,
    Simulator,
    Capture,
    TogglePause,
    Brightness(i64),
    ToggleNight,
    ToggleDimOnLock,
    SignIn(String),
    ToggleNotifications,
    ToggleAgentEvents,
    TogglePauseOnLock,
    OpenLogs,
    Quit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// Shown, but not something to choose.
    Label(String),
    /// Something to choose; `checked` is `Some` for the ones that are on or off.
    Command { text: String, checked: Option<bool>, action: Action },
    Separator,
    Submenu { text: String, items: Vec<Item> },
}

fn command(text: &str, checked: Option<bool>, action: Action) -> Item {
    Item::Command { text: text.to_string(), checked, action }
}

/// The menu for the app's state right now.
pub fn build(app: &TrayController) -> Vec<Item> {
    let (view, power, backlight, notifications) = (&app.view, &app.power, &app.backlight, &app.notifications);
    let (mode, provider) = (view.mode(), view.provider());
    let titles = app.providers.titles();
    let mut items = vec![Item::Label(app.screen.status_line()), Item::Separator];
    for (key, title) in &titles {
        items.push(command(title, Some(mode.is_none() && provider == *key), Action::Select(key.clone())));
    }
    items.push(command("Vista dividida", Some(mode.as_deref() == Some(SPLIT)), Action::Mode(SPLIT.into())));
    items.push(command("Estadísticas", Some(mode.as_deref() == Some(STATS)), Action::Mode(STATS.into())));
    items.push(Item::Submenu {
        text: "Más vistas".into(),
        items: vec![
            command("Proyectos y modelos", Some(mode.as_deref() == Some(BREAKDOWN)), Action::Mode(BREAKDOWN.into())),
            command("Horas pico", Some(mode.as_deref() == Some(HOURS)), Action::Mode(HOURS.into())),
        ],
    });
    items.extend([
        Item::Separator,
        command("Actualizar ahora", None, Action::RefreshNow),
        command("Pantalla simulada", None, Action::Simulator),
        command("Guardar pantallazo...", None, Action::Capture),
        command("Pausar", Some(power.paused()), Action::TogglePause),
    ]);
    let mut screen_items: Vec<Item> = config::DEFAULT_BRIGHTNESS_CHOICES
        .iter()
        .map(|level| command(&format!("Brillo {level} %"), Some(backlight.brightness() == Some(*level)), Action::Brightness(*level)))
        .collect();
    screen_items.extend([
        Item::Separator,
        command(&backlight.night_label(), Some(backlight.night().enabled), Action::ToggleNight),
        command("Atenuar al bloquear el PC", Some(backlight.dim_on_lock()), Action::ToggleDimOnLock),
    ]);
    items.push(Item::Submenu { text: "Pantalla".into(), items: screen_items });
    items.push(Item::Submenu {
        text: "Opciones".into(),
        items: vec![
            Item::Submenu { text: "Iniciar sesión".into(), items: titles.iter().map(|(key, title)| command(title, None, Action::SignIn(key.clone()))).collect() },
            command("Notificaciones", Some(notifications.enabled()), Action::ToggleNotifications),
            command("Avisar cuando un agente termine o te espere", Some(notifications.agent_events()), Action::ToggleAgentEvents),
            command("Pausar al bloquear el PC", Some(power.pause_on_lock()), Action::TogglePauseOnLock),
        ],
    });
    items.push(command("Ver logs", None, Action::OpenLogs));
    items.push(command("Salir", None, Action::Quit));
    items
}

/// Every action in a menu, with its checkmark.
pub fn commands(items: &[Item]) -> Vec<(String, Option<bool>, Action)> {
    let mut out = Vec::new();
    for item in items {
        match item {
            Item::Command { text, checked, action } => out.push((text.clone(), *checked, action.clone())),
            Item::Submenu { items, .. } => out.extend(commands(items)),
            Item::Label(_) | Item::Separator => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::controller::tests::app;
    use crate::application::controller::Settings;
    use serde_json::json;

    fn labels(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|i| match i {
                Item::Label(t) => format!("= {t}"),
                Item::Command { text, .. } => text.clone(),
                Item::Separator => "-".into(),
                Item::Submenu { text, .. } => format!("> {text}"),
            })
            .collect()
    }

    fn checked(items: &[Item], text: &str) -> Option<bool> {
        commands(items).into_iter().find(|(t, _, _)| t == text).and_then(|(_, c, _)| c)
    }

    #[test]
    fn the_menu_has_the_python_apps_shape_and_labels() {
        let a = app(Settings::default(), json!({}));
        let menu = build(&a.app);
        assert_eq!(
            labels(&menu),
            ["= Pantalla: 10.0.0.5", "-", "Claude", "Codex", "Vista dividida", "Estadísticas", "> Más vistas", "-", "Actualizar ahora", "Pantalla simulada", "Guardar pantallazo...", "Pausar", "> Pantalla", "> Opciones", "Ver logs", "Salir"]
        );
        let Item::Submenu { items, .. } = &menu[12] else { panic!() };
        assert_eq!(labels(items), ["Brillo 100 %", "Brillo 75 %", "Brillo 50 %", "Brillo 25 %", "Brillo 10 %", "Brillo 0 %", "-", "Modo nocturno (10 PM - 7 AM, brillo 10 %)", "Atenuar al bloquear el PC"]);
    }

    #[test]
    fn checkmarks_follow_the_state() {
        let a = app(Settings::default(), json!({"view": "split", "provider": "codex"}));
        let menu = build(&a.app);
        assert_eq!((checked(&menu, "Vista dividida"), checked(&menu, "Claude"), checked(&menu, "Codex")), (Some(true), Some(false), Some(false)));
        assert_eq!((checked(&menu, "Pausar"), checked(&menu, "Notificaciones"), checked(&menu, "Pausar al bloquear el PC")), (Some(false), Some(true), Some(true)));
        a.app.select("codex");
        a.app.backlight.set_brightness(50);
        let menu = build(&a.app);
        assert_eq!((checked(&menu, "Codex"), checked(&menu, "Vista dividida"), checked(&menu, "Brillo 50 %"), checked(&menu, "Brillo 75 %")), (Some(true), Some(false), Some(true), Some(false)));
        assert_eq!(checked(&menu, "Actualizar ahora"), None, "a plain command has no checkmark");
    }

    #[test]
    fn every_command_does_what_its_name_says() {
        let a = app(Settings::default(), json!({}));
        let all = commands(&build(&a.app));
        let action = |text: &str| all.iter().find(|(t, _, _)| t == text).map(|(_, _, a)| a.clone()).unwrap();
        assert_eq!(action("Codex"), Action::Select("codex".into()));
        assert_eq!(action("Horas pico"), Action::Mode("hours".into()));
        assert_eq!(action("Brillo 25 %"), Action::Brightness(25));
        assert_eq!(action("Salir"), Action::Quit);
        assert_eq!(action("Pantalla simulada"), Action::Simulator);
        assert_eq!(all.iter().filter(|(_, _, a)| matches!(a, Action::SignIn(_))).count(), 2, "one sign-in per provider");
    }
}
