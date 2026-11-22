//! Running in the background, as the person chooses in Settings → Notifications: KChess's icon
//! and menu in the menu bar (macOS) or the system tray (Windows, Linux), what closing the last
//! window does, and opening KChess when the person logs in.
//!
//! Closing the window while KChess keeps running leaves its models, its core connection and the
//! Lichess event stream as they are, so challenges and "your move" notifications keep arriving;
//! the core stops what this app started (its analysis, watching, voice input) as it does for any
//! client with no window (`suspend`, `docs/shared-core.md`). On Windows and Linux it keeps running
//! only while its tray icon shows: otherwise nothing would show it runs, so closing quits.
//!
//! The icon and the login entry are the app's own (`kchess-background`): the icon belongs to this
//! process, and the entry starts this executable from a folder of the person's session, not the
//! profile, so neither is a core method.

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt as _;
use gpui_kit::*;
use kchess_background::{autostart, tray};
use kchess_contracts::Level;
use kchess_contracts::types::{ChallengeDirection, CloseAction};
use kchess_nav::page::PageTab as _;

use crate::log::ResultExt as _;
use crate::pages::Page;

/// The argument a login entry starts KChess with: in the background, without a window, when
/// closing the window keeps it running.
pub const QUIET: &str = "--background";

/// Whether the person has been told, once, that a closed KChess keeps running.
const TOLD: &str = "kchess:background-told";

/// What the menu holds at most of each list, so it stays a menu.
const LISTED: usize = 5;

/// What a menu entry does.
#[derive(Clone, Debug, PartialEq)]
enum Choice {
    Open,
    Page(Page),
    DailyPuzzle,
    Game { account: String, id: String },
    Accept(String),
    Decline(String),
    DoNotDisturb,
    Quit,
}

/// The icon as it is shown: the platform's, or in tests the menu it would show.
enum Shown {
    #[cfg(not(test))]
    Real(tray::Tray),
    #[cfg(test)]
    Recorded,
}

pub struct Background {
    shown: Option<Shown>,
    /// Why the icon could not show the last time it was asked to (a Linux desktop without a
    /// tray); asked again once the setting is turned off and on.
    unavailable: Option<String>,
    /// The menu shown, and what each of its entries does.
    menu: Vec<tray::Item>,
    choices: HashMap<u32, Choice>,
    /// Whether the login entry is written, as last made so; `None` until the first look.
    login: Option<bool>,
    /// Where the icon's clicks go, to the task below on the main thread.
    events: futures::channel::mpsc::UnboundedSender<tray::Event>,
    _clicks: Task<()>,
    _subscriptions: Vec<Subscription>,
    /// Closing the window asked the app to quit (read by the tests).
    pub(crate) quit_asked: bool,
}

impl Global for Background {}

/// Show the icon and write the login entry as the settings say, and follow them from now on.
pub fn init(cx: &mut App) {
    let (events, mut clicks) = futures::channel::mpsc::unbounded();
    let task = cx.spawn(async move |cx| {
        while let Some(event) = clicks.next().await {
            cx.update(|cx| clicked(event, cx));
        }
    });
    let settings = cx.global::<crate::settings::SettingsHandle>().0.clone();
    let subscriptions = vec![
        cx.observe(&settings, |_, cx| refresh(cx)),
        cx.observe(&crate::online::handle(cx), |_, cx| refresh(cx)),
        cx.observe(&crate::accounts::handle(cx), |_, cx| refresh(cx)),
    ];
    cx.set_global(Background {
        shown: None,
        unavailable: None,
        menu: Vec::new(),
        choices: HashMap::new(),
        login: None,
        events,
        _clicks: task,
        _subscriptions: subscriptions,
        quit_asked: false,
    });
    refresh(cx);
}

/// Whether closing the last window with `action` keeps KChess running, `tray` its icon shown:
/// as chosen, by default as the platform's apps do (macOS keeps them running), and on Windows
/// and Linux only while the icon shows it runs.
pub fn keeps_running_with(action: CloseAction, tray: bool) -> bool {
    let background = match action {
        CloseAction::Background => true,
        CloseAction::Quit => false,
        CloseAction::Default => cfg!(target_os = "macos"),
    };
    background && (cfg!(target_os = "macos") || tray)
}

/// The close choice Settings shows for `action`: the default as the platform takes it.
pub fn chosen(action: CloseAction) -> CloseAction {
    match action {
        CloseAction::Default if cfg!(target_os = "macos") => CloseAction::Background,
        CloseAction::Default => CloseAction::Quit,
        chosen => chosen,
    }
}

/// Whether the icon shows now.
pub fn tray_shown(cx: &App) -> bool {
    cx.try_global::<Background>()
        .is_some_and(|background| background.shown.is_some())
}

/// Why the icon cannot show, when it was asked to and could not.
pub fn tray_unavailable(cx: &App) -> Option<String> {
    cx.try_global::<Background>()
        .and_then(|background| background.unavailable.clone())
}

/// Whether closing the last window now keeps KChess running.
pub fn keeps_running(cx: &App) -> bool {
    let action = crate::settings::frontend(cx).close_action;
    keeps_running_with(action, tray_shown(cx))
}

/// Whether this start opens no window: started at login, while closing it would keep KChess
/// running anyway.
pub fn starts_hidden(cx: &App) -> bool {
    started_at_login() && keeps_running(cx)
}

fn started_at_login() -> bool {
    !cfg!(test) && std::env::args_os().skip(1).any(|arg| arg == QUIET)
}

/// Whether this build can open at login: the App Store's sandbox keeps it from writing the
/// login entry.
pub fn opens_at_login_here() -> bool {
    !kchess_host::app_store()
}

/// The settings or what the menu lists changed: show the icon, its menu and the login entry as
/// they say. The global changes, and Settings redraws, only when one of them does.
fn refresh(cx: &mut App) {
    let frontend = crate::settings::frontend(cx).clone();
    let wanted = frontend.show_tray_icon && tray::SUPPORTED;
    let login = frontend.open_at_login && opens_at_login_here();
    let (menu, choices) = menu(cx);
    let current = cx.global::<Background>();
    let ask = wanted && current.shown.is_none() && current.unavailable.is_none();
    let hide = !wanted && (current.shown.is_some() || current.unavailable.is_some());
    let relabel = current.menu != menu || current.choices != choices;
    let relogin = current.login != Some(login);
    if !(ask || hide || relabel || relogin) {
        return;
    }
    let events = current.events.clone();
    let background = cx.global_mut::<Background>();
    if hide {
        background.shown = None;
        background.unavailable = None;
    }
    if ask {
        match show(menu.clone(), events) {
            Ok(shown) => background.shown = Some(shown),
            Err(why) => {
                crate::log::write(Level::Warn, "background", &why);
                background.unavailable = Some(why);
            }
        }
    } else if relabel {
        #[cfg(not(test))]
        if let Some(Shown::Real(tray)) = background.shown.as_mut() {
            tray.set_menu(menu.clone());
        }
    }
    background.menu = menu;
    background.choices = choices;
    if relogin {
        background.login = Some(login);
        write_login(login, cx);
    }
}

#[cfg(not(test))]
fn show(
    menu: Vec<tray::Item>,
    events: futures::channel::mpsc::UnboundedSender<tray::Event>,
) -> Result<Shown, String> {
    let icon = tray::Icon {
        template_png: include_bytes!("../build/tray-template.png"),
        color_png: include_bytes!("../build/tray.png"),
    };
    let forward: tray::Events = Arc::new(move |event| {
        // Closed only as the app quits, when a click has nowhere to go.
        events.unbounded_send(event).ok();
    });
    tray::Tray::new(crate::profile::APP_NAME, &icon, menu, forward).map(Shown::Real)
}

/// Tests never touch the menu bar or tray of the machine running them: the menu is recorded.
#[cfg(test)]
fn show(
    _: Vec<tray::Item>,
    events: futures::channel::mpsc::UnboundedSender<tray::Event>,
) -> Result<Shown, String> {
    let _: tray::Events = Arc::new(move |event| {
        events.unbounded_send(event).ok();
    });
    Ok(Shown::Recorded)
}

/// The menu as KChess's state is now, and what each entry does.
fn menu(cx: &App) -> (Vec<tray::Item>, HashMap<u32, Choice>) {
    let mut built = Menu::default();
    built.action("Open KChess", Choice::Open);
    built.separator();
    if !crate::accounts::handle(cx).read(cx).connected().is_empty() {
        built.action("Play on Lichess", Choice::Page(Page::Online));
    }
    built.action("Play with computer", Choice::Page(Page::Computer));
    built.action("Daily puzzle", Choice::DailyPuzzle);
    built.action("Puzzles", Choice::Page(Page::Puzzles));
    let online = crate::online::handle(cx).read(cx);
    let waiting: Vec<_> = online
        .ongoing
        .iter()
        .filter(|game| game.is_my_turn)
        .take(LISTED)
        .collect();
    if !waiting.is_empty() {
        built.separator();
        for game in waiting {
            built.action(
                &format!("Your move against {}", game.opponent.name),
                Choice::Game {
                    account: game.account.clone(),
                    id: game.game_id.clone(),
                },
            );
        }
    }
    let challenges: Vec<_> = online
        .challenges
        .iter()
        .filter(|challenge| challenge.direction == ChallengeDirection::In)
        .take(LISTED)
        .collect();
    if !challenges.is_empty() {
        built.separator();
        for challenge in challenges {
            let (title, terms) = crate::online::challenge_alert(challenge);
            let items = vec![
                tray::Item::note(terms),
                built.item("Accept", Choice::Accept(challenge.id.clone())),
                built.item("Decline", Choice::Decline(challenge.id.clone())),
            ];
            built.items.push(tray::Item::Submenu {
                label: title,
                items,
            });
        }
    }
    built.separator();
    let quiet = !crate::settings::frontend(cx).notifications_enabled;
    let id = built.choose(Choice::DoNotDisturb);
    built
        .items
        .push(tray::Item::check(id, "Do not disturb", quiet));
    built.action(
        if cfg!(target_os = "macos") {
            "Settings…"
        } else {
            "Settings"
        },
        Choice::Page(Page::Settings),
    );
    built.separator();
    built.action("Quit KChess", Choice::Quit);
    (built.items, built.choices)
}

/// A menu being built: its entries, and what each id does.
#[derive(Default)]
struct Menu {
    items: Vec<tray::Item>,
    choices: HashMap<u32, Choice>,
}

impl Menu {
    fn choose(&mut self, choice: Choice) -> u32 {
        let id = self.choices.len() as u32 + 1;
        self.choices.insert(id, choice);
        id
    }

    fn item(&mut self, label: &str, choice: Choice) -> tray::Item {
        tray::Item::action(self.choose(choice), label)
    }

    fn action(&mut self, label: &str, choice: Choice) {
        let item = self.item(label, choice);
        self.items.push(item);
    }

    fn separator(&mut self) {
        self.items.push(tray::Item::Separator);
    }
}

/// The person clicked the icon or picked an entry.
fn clicked(event: tray::Event, cx: &mut App) {
    let choice = match event {
        tray::Event::Open => Some(Choice::Open),
        tray::Event::Chose(id) => cx.global::<Background>().choices.get(&id).cloned(),
    };
    if let Some(choice) = choice {
        act(choice, cx);
    }
}

fn act(choice: Choice, cx: &mut App) {
    match choice {
        Choice::Open => show_window(None, cx),
        Choice::Page(page) => show_window(Some(page), cx),
        Choice::DailyPuzzle => {
            crate::pages::puzzles::Tab::Daily.store(cx);
            show_window(Some(Page::Puzzles), cx);
        }
        Choice::Game { account, id } => {
            crate::notifications::open_game(&format!("{account}/{id}"), cx);
        }
        Choice::Accept(id) => crate::notifications::answer_challenge(&id, Some("accept"), cx),
        Choice::Decline(id) => crate::notifications::answer_challenge(&id, Some("decline"), cx),
        Choice::DoNotDisturb => crate::settings::change(cx, |settings| {
            let frontend = &mut settings.frontend;
            frontend.notifications_enabled = !frontend.notifications_enabled;
        }),
        Choice::Quit => cx.dispatch_action(&crate::menu::Quit),
    }
}

/// Bring KChess forward, opening its window when it runs without one, on `page` when given.
pub fn show_window(page: Option<Page>, cx: &mut App) {
    if cx.windows().is_empty() && cx.has_global::<crate::startup::Started>() {
        crate::open_main_window(cx);
    }
    cx.activate(true);
    if let Some(window) = cx.windows().into_iter().next() {
        window
            .update(cx, |_, window, cx| {
                window.activate_window();
                if let Some(page) = page {
                    crate::shell::open(page, window, cx);
                }
            })
            .ok();
    }
}

/// The last window closed (its work saved first, `menu::close_window`): keep running in the
/// background, or quit.
pub fn window_closed(cx: &mut App) {
    if !cx.windows().is_empty() {
        return;
    }
    // The startup screen closed, before anything runs in the background: as macOS apps do, or quit.
    if !cx.has_global::<Background>() {
        if !cfg!(target_os = "macos") {
            cx.quit();
        }
        return;
    }
    if keeps_running(cx) {
        went_to_background(cx);
    } else {
        cx.global_mut::<Background>().quit_asked = true;
        cx.quit();
    }
}

/// Running with no window: the core stops what this app started, and the person hears once
/// where KChess went.
pub fn went_to_background(cx: &mut App) {
    let bridge = cx.global::<crate::core::AppCore>().bridge.clone();
    bridge.focus(false);
    bridge.suspend();
    if crate::prefs::get(cx, TOLD, false) {
        return;
    }
    let body = if tray_shown(cx) && cfg!(target_os = "macos") {
        "Open or quit it from the menu bar."
    } else if cfg!(target_os = "macos") {
        "Open it again from the Dock."
    } else {
        "Open or quit it from the system tray."
    };
    if crate::notifications::still_running("KChess is still running", body, cx) {
        crate::prefs::set(cx, TOLD, true);
    }
}

/// The login entry, written or removed off the main thread.
fn write_login(on: bool, cx: &mut App) {
    let Some(folder) = login_folder() else {
        return;
    };
    let entry = login_entry();
    cx.background_spawn(async move {
        let done = match entry {
            Some(entry) if on => autostart::install(&folder, &entry),
            Some(entry) => autostart::remove(&folder, &entry.name),
            None => Ok(()),
        };
        done.map_err(|cause| format!("The login entry could not be changed: {cause}"))
            .warn_on_err();
    })
    .detach();
}

/// Where the login entry goes: the system's folder, or in tests the profile's own.
fn login_folder() -> Option<std::path::PathBuf> {
    if cfg!(test) {
        return crate::profile::data_dir()
            .ok()
            .map(|dir| dir.join("autostart"));
    }
    autostart::folder()
}

/// The login entry's name: a development build's is its own, so it never replaces the installed
/// app's.
fn entry_name() -> &'static str {
    let development = kchess_host::dev_overrides();
    if cfg!(target_os = "macos") {
        if development {
            "com.kchess.app.dev"
        } else {
            "com.kchess.app"
        }
    } else if cfg!(windows) {
        if development {
            "KChess (development)"
        } else {
            "KChess"
        }
    } else if development {
        "kchess-dev"
    } else {
        "kchess"
    }
}

/// The login entry for this KChess: its executable (the Snap's or AppImage's own launcher, whose
/// path stays the same across updates), started quietly, on the profile this one uses.
pub fn login_entry() -> Option<autostart::Entry> {
    let var = |name: &str| kchess_host::env::var(name).filter(|value| !value.is_empty());
    let program = if cfg!(target_os = "linux") && var("SNAP").is_some() {
        std::path::PathBuf::from("/snap/bin/kchess")
    } else if let Some(image) = var("APPIMAGE").filter(|_| cfg!(target_os = "linux")) {
        image.into()
    } else {
        std::env::current_exe().ok()?
    };
    // A profile picked for this run (development, tests) goes with it, where the entry can
    // carry it; a Windows shortcut cannot.
    let env = var("KCHESS_USER_DATA_DIR")
        .filter(|_| !cfg!(windows))
        .map(|dir| vec![("KCHESS_USER_DATA_DIR".to_string(), dir)])
        .unwrap_or_default();
    Some(autostart::Entry {
        name: entry_name().into(),
        title: crate::profile::APP_NAME.into(),
        program,
        args: vec![QUIET.into()],
        env,
        bundle_id: bundle_id(),
    })
}

#[cfg(target_os = "macos")]
fn bundle_id() -> Option<String> {
    objc2_foundation::NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|id| id.to_string())
}

#[cfg(not(target_os = "macos"))]
fn bundle_id() -> Option<String> {
    None
}

/// The menu as shown, as labels: entries, `---` for a separator, `>` before a submenu's
/// entries and `[x]` before a checked one.
#[cfg(test)]
pub fn menu_labels(cx: &App) -> Vec<String> {
    fn walk(items: &[tray::Item], depth: usize, out: &mut Vec<String>) {
        for item in items {
            let indent = ">".repeat(depth);
            match item {
                tray::Item::Separator => out.push(format!("{indent}---")),
                tray::Item::Submenu { label, items } => {
                    out.push(format!("{indent}{label}"));
                    walk(items, depth + 1, out);
                }
                tray::Item::Action { label, checked, .. } => {
                    let mark = if *checked == Some(true) { "[x] " } else { "" };
                    out.push(format!("{indent}{mark}{label}"));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&cx.global::<Background>().menu, 0, &mut out);
    out
}

/// Pick the entry labelled `label` (in a submenu titled `under` when given), as a click would.
#[cfg(test)]
pub fn pick(label: &str, under: Option<&str>, cx: &mut App) {
    fn find(items: &[tray::Item], label: &str, under: Option<&str>) -> Option<u32> {
        items.iter().find_map(|item| match item {
            tray::Item::Action {
                id,
                label: shown,
                enabled: true,
                ..
            } if under.is_none() && shown == label => Some(*id),
            tray::Item::Submenu {
                label: title,
                items,
            } => {
                let inner = match under {
                    Some(under) if title.contains(under) => None,
                    other => other,
                };
                find(items, label, inner)
            }
            _ => None,
        })
    }
    let id = find(&cx.global::<Background>().menu, label, under)
        .unwrap_or_else(|| panic!("no menu entry {label:?} under {under:?}"));
    clicked(tray::Event::Chose(id), cx);
}

#[cfg(test)]
mod tests {
    use super::{CloseAction, QUIET, chosen, keeps_running_with, login_entry};

    /// Quit always quits; keeping running needs the icon on Windows and Linux, where nothing
    /// else would show KChess runs; by default macOS keeps running and the others quit.
    #[test]
    fn closing_keeps_running_only_where_something_shows_it() {
        assert!(!keeps_running_with(CloseAction::Quit, true));
        assert!(keeps_running_with(CloseAction::Background, true));
        let mac = cfg!(target_os = "macos");
        assert_eq!(keeps_running_with(CloseAction::Background, false), mac);
        assert_eq!(keeps_running_with(CloseAction::Default, true), mac);
        assert_eq!(keeps_running_with(CloseAction::Default, false), mac);
        assert_eq!(
            chosen(CloseAction::Default),
            if mac {
                CloseAction::Background
            } else {
                CloseAction::Quit
            }
        );
    }

    /// The login entry starts this executable quietly; a development build's has its own name.
    #[test]
    fn the_login_entry_starts_kchess_quietly() {
        let entry = login_entry().expect("this executable");
        assert_eq!(entry.args, [QUIET]);
        assert!(entry.program.is_absolute());
        assert!(entry.name.contains("dev"), "{}", entry.name);
    }
}
