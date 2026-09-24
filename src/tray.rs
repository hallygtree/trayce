//! System-tray UI: tao event loop + tray-icon, with a background poll thread.

use std::collections::HashMap;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::{
    CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::config::{self, Config, Mode};
use crate::error::WidgetError;
use crate::providers::Provider;
use crate::render::{self, Level};
use crate::usage::Report;

type PollResult = Vec<(Provider, Result<Report, WidgetError>)>;

/// Events delivered into the tao event loop.
enum UserEvent {
    Menu(MenuEvent),
    Poll(PollResult),
}

/// What a clickable menu item does.
#[derive(Clone, Copy)]
enum Action {
    Refresh,
    Quit,
    Toggle(Provider),
    Mode(Mode),
}

// Reading local logs is cheap, so we poll more often than the old 5min network call.
const POLL_INTERVAL: Duration = Duration::from_secs(60);

fn icon(level: Level) -> Icon {
    match Icon::from_rgba(
        render::icon_rgba(level),
        render::ICON_SIZE as u32,
        render::ICON_SIZE as u32,
    ) {
        Ok(icon) => icon,
        Err(e) => {
            eprintln!("fatal: could not build tray icon: {e}");
            std::process::exit(1);
        }
    }
}

/// Wake the macOS run loop so tray changes draw immediately. No-op elsewhere.
fn wake_macos() {
    #[cfg(target_os = "macos")]
    {
        use objc2_core_foundation::CFRunLoop;
        if let Some(rl) = CFRunLoop::main() {
            rl.wake_up();
        }
    }
}

fn disabled(text: &str) -> MenuItem {
    MenuItem::new(text, false, None)
}

/// Latest known state per provider: the last good report survives a failed
/// poll, shown alongside the error note (same as the original single-provider app).
#[derive(Default)]
struct Latest {
    reports: HashMap<Provider, Report>,
    errors: HashMap<Provider, String>,
}

impl Latest {
    fn apply(&mut self, results: PollResult) {
        for (p, r) in results {
            match r {
                Ok(report) => {
                    self.reports.insert(p, report);
                    self.errors.remove(&p);
                }
                Err(e) => {
                    self.errors.insert(p, render::error_text(&e).0);
                }
            }
        }
    }

    /// One-line status: "5h 42% · 7d 18%", "⚠ logs", or "loading…".
    fn summary(&self, p: Provider) -> String {
        match (self.errors.contains_key(&p), self.reports.get(&p)) {
            (true, _) => "⚠ logs".to_string(),
            (false, Some(r)) => render::title_text(r),
            (false, None) => "loading…".to_string(),
        }
    }
}

/// Detail rows for one provider, appended through `add` so the same code
/// fills either the root menu or a submenu.
fn add_details(add: &mut dyn FnMut(&dyn IsMenuItem), latest: &Latest, p: Provider) {
    let now = chrono::Utc::now();
    if let Some(r) = latest.reports.get(&p) {
        for row in &r.rows {
            add(&disabled(&render::window_row(&row.label, &row.window)));
            let reset = render::reset_row(&row.window, row.weekday, now);
            if !reset.is_empty() {
                add(&disabled(&reset));
            }
            add(&PredefinedMenuItem::separator());
        }
        for note in &r.notes {
            add(&disabled(note));
        }
    }
    match latest.errors.get(&p) {
        Some(note) => add(&disabled(note)),
        None if !latest.reports.contains_key(&p) => add(&disabled("loading…")),
        None => {}
    }
}

/// Build the dropdown menu plus the id → action map for its clickable items.
fn build_menu(cfg: &Config, latest: &Latest) -> (Menu, HashMap<MenuId, Action>) {
    let menu = Menu::new();
    let mut actions = HashMap::new();
    let shown = cfg.shown();

    match cfg.mode {
        Mode::Single(_) if shown.len() == 1 => {
            let p = shown[0];
            let _ = menu.append(&disabled(p.name()));
            let _ = menu.append(&PredefinedMenuItem::separator());
            add_details(&mut |i| drop(menu.append(i)), latest, p);
        }
        _ => {
            for &p in &shown {
                let sub = Submenu::new(format!("{}   {}", p.name(), latest.summary(p)), true);
                add_details(&mut |i| drop(sub.append(i)), latest, p);
                let _ = menu.append(&sub);
            }
            if shown.is_empty() {
                let _ = menu.append(&disabled("No AI enabled"));
            }
        }
    }
    let _ = menu.append(&PredefinedMenuItem::separator());

    let mut check = |parent: &Submenu, text: &str, on: bool, action: Action| {
        let item = CheckMenuItem::new(text, true, on, None);
        actions.insert(item.id().clone(), action);
        let _ = parent.append(&item);
    };
    let show = Submenu::new("Show", true);
    check(
        &show,
        "All enabled",
        cfg.mode == Mode::All,
        Action::Mode(Mode::All),
    );
    for &p in &cfg.enabled {
        let only = Mode::Single(p);
        check(
            &show,
            &format!("Only {}", p.name()),
            cfg.mode == only,
            Action::Mode(only),
        );
    }
    let enabled = Submenu::new("Enabled AIs", true);
    for p in Provider::ALL {
        check(
            &enabled,
            p.name(),
            cfg.enabled.contains(&p),
            Action::Toggle(p),
        );
    }
    let _ = menu.append(&show);
    let _ = menu.append(&enabled);

    for (text, action) in [("Refresh now", Action::Refresh), ("Quit", Action::Quit)] {
        let item = MenuItem::new(text, true, None);
        actions.insert(item.id().clone(), action);
        let _ = menu.append(&item);
    }
    (menu, actions)
}

/// Redraw icon, tooltip, title and menu from the current state.
fn redraw(tray: &TrayIcon, cfg: &Config, latest: &Latest) -> HashMap<MenuId, Action> {
    let shown = cfg.shown();
    // Colour by the fullest window among providers with a current reading.
    let pcts: Vec<u32> = shown
        .iter()
        .filter(|p| !latest.errors.contains_key(p))
        .filter_map(|p| latest.reports.get(p))
        .flat_map(|r| r.rows.iter().map(|row| row.window.pct))
        .collect();
    let level = pcts
        .iter()
        .max()
        .map_or(Level::Grey, |&m| render::level_for(m));
    let _ = tray.set_icon(Some(icon(level)));

    let tooltip = if shown.is_empty() {
        "Trayce · no AI enabled".to_string()
    } else {
        shown
            .iter()
            .map(|&p| format!("{} · {}", p.name(), latest.summary(p)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let _ = tray.set_tooltip(Some(tooltip));
    let title = match shown.as_slice() {
        [p] => latest.summary(*p),
        ps => ps
            .iter()
            .map(|&p| format!("{} {}", p.name(), latest.summary(p)))
            .collect::<Vec<_>>()
            .join("  |  "),
    };
    tray.set_title(Some(title));

    let (menu, actions) = build_menu(cfg, latest);
    tray.set_menu(Some(Box::new(menu)));
    actions
}

pub fn run() -> ! {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    // Forward menu events into the event loop.
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));

    let mut cfg = config::load();

    // Background poll thread: collect every enabled provider, push the results,
    // then wait POLL_INTERVAL or until a new enabled-list arrives on `refresh_rx`
    // (sent on "Refresh now" and on settings changes).
    let (refresh_tx, refresh_rx) = mpsc::channel::<Vec<Provider>>();
    let poll_proxy = event_loop.create_proxy();
    let mut polled = cfg.enabled.clone();
    thread::spawn(move || loop {
        let now = chrono::Utc::now();
        let results = polled.iter().map(|&p| (p, p.collect(now))).collect();
        if poll_proxy.send_event(UserEvent::Poll(results)).is_err() {
            return; // event loop has shut down
        }
        match refresh_rx.recv_timeout(POLL_INTERVAL) {
            Ok(list) => polled = list,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    });

    let mut tray: Option<TrayIcon> = None;
    let mut latest = Latest::default();
    let mut actions: HashMap<MenuId, Action> = HashMap::new();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                let (menu, a) = build_menu(&cfg, &latest);
                actions = a;
                tray = Some(
                    match TrayIconBuilder::new()
                        .with_menu(Box::new(menu))
                        .with_tooltip("Trayce · loading…")
                        .with_icon(icon(Level::Grey))
                        .build()
                    {
                        Ok(tray) => tray,
                        Err(e) => {
                            eprintln!("fatal: could not build tray: {e}");
                            std::process::exit(1);
                        }
                    },
                );
                wake_macos();
            }
            Event::UserEvent(UserEvent::Poll(results)) => {
                latest.apply(results);
                if let Some(t) = &tray {
                    actions = redraw(t, &cfg, &latest);
                }
                wake_macos();
            }
            Event::UserEvent(UserEvent::Menu(ev)) => {
                let Some(&action) = actions.get(&ev.id) else {
                    return;
                };
                match action {
                    Action::Quit => {
                        tray.take();
                        *control_flow = ControlFlow::Exit;
                        return;
                    }
                    Action::Refresh => {}
                    Action::Toggle(p) => {
                        cfg.toggle(p);
                        config::save(&cfg);
                    }
                    Action::Mode(m) => {
                        cfg.mode = m;
                        config::save(&cfg);
                    }
                }
                let _ = refresh_tx.send(cfg.enabled.clone());
                // Redraw right away from what we already have; the poll fills the rest.
                if let Some(t) = &tray {
                    actions = redraw(t, &cfg, &latest);
                }
                wake_macos();
            }
            _ => {}
        }
    })
}
