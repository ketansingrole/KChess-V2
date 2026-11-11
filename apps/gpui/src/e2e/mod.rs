//! End-to-end tests of the GPUI app: the whole app (embedded core, models, the workspace window)
//! started in a headless GPUI window over a fresh profile, then driven by clicks and keys as a
//! person would, and checked by what the window shows.
//!
//! Each test gets its own profile (`profile::TEST_DATA_DIR`), the scripted engine from
//! `kchess-core` in place of Stockfish (`kchess-fake-uci` in `search` mode: it always answers
//! `e2e4`), and a stand-in Lichess (`fake_lichess`) that answers only a test that signed in, so
//! nothing leaves the machine.
//!
//! Elements are found by their GPUI `ElementId` among those registered with `.test_support()`
//! (`gpui_kit::test`); `Harness::paths` lists them when a lookup fails.

mod analysis;
mod audit;
mod chesscom;
mod computer;
mod fake_chesscom;
mod fake_lichess;
mod history;
mod live_game;
mod local;
mod memory;
mod online;
mod openings;
mod palette;
mod perf;
mod practice;
mod puzzles;
mod review;
#[cfg(target_os = "macos")]
mod screens;
mod settings;
mod shell;
mod studies;
mod tournaments;
mod whats_new;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui_kit::test::{ElementSnapshot, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, ElementId, InputEvent as _, Modifiers, MouseButton,
    MouseDownEvent, MouseUpEvent, Pixels, Point, TestAppContext, Window,
};

/// How long a step may take to show: real time, since the core answers on its own threads. A
/// loaded CI runner took over 10 s for an import (2026.10.15); a passing step never waits this out.
const TIMEOUT: Duration = Duration::from_secs(30);

thread_local! {
    /// Apps a test started again (`Harness::restart`), held until the test's thread ends.
    static KEPT_APPS: std::cell::RefCell<Vec<TestAppContext>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The window sizes the audit and the screenshots look at: the minimum, a small laptop's and the
/// default (`window_state`).
const SIZES: [(f32, f32); 3] = [(640., 620.), (1024., 768.), (1320., 880.)];

/// Draw the window as it is drawn on screen: the next-frame callbacks the platform's frame
/// delivers, then only the views that changed, cached ones reused. (`render_frame` redraws every
/// view, which would hide a view that caching leaves stale.)
fn draw(window: &mut Window, cx: &mut App) {
    window.simulate_next_frame(cx);
    window.draw(cx).clear(cx);
}

/// How often a wait looks again.
const POLL: Duration = Duration::from_millis(5);

/// The app, started over its own profile in a headless window.
pub struct Harness {
    pub cx: TestAppContext,
    pub window: AnyWindowHandle,
    /// Kept by `restart`, removed with the harness.
    profile: Option<tempfile::TempDir>,
    /// Whether this harness made the app (`restart`) rather than `#[gpui_kit::test]`.
    owns_app: bool,
}

/// A page's tab by name, as its pills are found: `tab("practice-tabs", Tab::Guess)`.
pub fn tab<T: crate::pages::PageTab>(tabs: &'static str, tab: T) -> (&'static str, usize) {
    (tabs, tab.index())
}

impl Harness {
    /// Start the app as `main` does, over a fresh profile.
    pub fn start(cx: &mut TestAppContext) -> Harness {
        Self::start_with(cx, |_| {})
    }

    /// Start the app after `prepare` has written to the fresh profile (settings, games).
    pub fn start_with(cx: &mut TestAppContext, prepare: impl FnOnce(&Path)) -> Harness {
        environment();
        let profile = tempfile::tempdir().expect("a temporary profile");
        prepare(profile.path());
        Self::boot(cx.clone(), profile, false)
    }

    /// Start the app signed in to the stand-in Lichess; `game` puts a game under way there.
    pub fn signed_in(cx: &mut TestAppContext, game: bool) -> (Harness, fake_lichess::Handle) {
        let mut app = Self::start(cx);
        let session = app.connect("Tester", game);
        (app, session)
    }

    /// Start the app signed in to the stand-in Lichess's showcase account: ratings in every
    /// speed, a rating history and synced games.
    pub fn signed_in_showcase(cx: &mut TestAppContext) -> (Harness, fake_lichess::Handle) {
        let mut app = Self::start(cx);
        let session = app.connect_with("Showcase", false, |session| session.showcase = true);
        (app, session)
    }

    /// Connect an account as a person would: the core opens the login page (`opened-urls.txt`,
    /// its test switch) and the "browser" returns with a code.
    pub fn connect(&mut self, username: &str, game: bool) -> fake_lichess::Handle {
        self.connect_with(username, game, |_| {})
    }

    /// `connect`, with `prepare` setting up what Lichess holds before the app first asks for it
    /// (set after signing in, it could miss the sign-in's own requests).
    pub fn connect_with(
        &mut self,
        username: &str,
        game: bool,
        prepare: impl FnOnce(&mut fake_lichess::Session),
    ) -> fake_lichess::Handle {
        let (code, session) = fake_lichess::session(username, game);
        prepare(&mut session.lock());
        let opened = self
            .profile
            .as_ref()
            .expect("the profile")
            .path()
            .join("opened-urls.txt");
        // Only this sign-in's login page counts.
        let _ = std::fs::remove_file(&opened);
        // A sign-in asked for while the accounts are busy (an earlier one still syncing) is
        // dropped, as the disabled button would be: Windows CI, slow enough to ask while the first
        // account synced, timed out `switching_accounts_moves_the_event_stream` waiting for it.
        self.wait_for("the accounts to be free for a sign-in", |_, cx| {
            !crate::accounts::handle(cx).read(cx).working()
        });
        self.cx
            .update_window(self.window, |_, window, cx| {
                crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.connect(window, cx))
            })
            .expect("the main window is open");
        std::thread::spawn(move || {
            let started = Instant::now();
            while started.elapsed() < TIMEOUT {
                let url = std::fs::read_to_string(&opened).ok().and_then(|text| {
                    let line = text.lines().find(|line| line.contains("/oauth"))?;
                    Some(line.to_string())
                });
                if let Some(url) = url {
                    fake_lichess::return_from_login(&url, &code);
                    return;
                }
                std::thread::sleep(POLL);
            }
        });
        self.wait_for("the account to connect", |_, cx| {
            crate::accounts::handle(cx).read(cx).active(cx).as_deref() == Some(username)
        });
        session
    }

    /// Take the computer to be on a network, whatever the machine running the test says.
    pub fn online(&mut self) {
        self.set_online(true);
    }

    /// Take the computer to be on a network or off it for the rest of the test, whatever the
    /// machine running it says (`network::init` looks at it again every 5 s).
    pub fn set_online(&mut self, online: bool) {
        self.cx.update(|cx| {
            let network = cx.global::<crate::network::NetworkHandle>().0.clone();
            network.update(cx, |network, cx| {
                network.pin(online);
                cx.notify();
            });
        });
    }

    /// Take the system to say `host` about the microphone for the rest of the test, whatever the
    /// machine running it says (`microphone::Host::TEST` until then). Settings reads it again
    /// when Voice input is shown.
    pub fn set_microphone(&mut self, host: crate::microphone::Host) {
        self.cx
            .update(|cx| cx.set_global(crate::microphone::Pinned(host)));
    }

    /// Quit the app as a person would (its saves flushed, the core closed), then start it again
    /// over the same profile.
    pub fn restart(mut self) -> Harness {
        let profile = self.profile.take().expect("the profile");
        let cx = self.cx.new_app();
        drop(self);
        Self::boot(cx, profile, true)
    }

    fn boot(mut cx: TestAppContext, profile: tempfile::TempDir, owns_app: bool) -> Harness {
        crate::profile::TEST_DATA_DIR.set(Some(profile.path().to_path_buf()));
        // The core answers on its Tokio threads; the test waits for them in real time.
        cx.executor().allow_parking();
        let dir = profile.path().to_path_buf();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::menu::init(cx);
            crate::notifications::init(cx);
            // Dialogs and popovers appear at rest: their entrance runs on the real clock, so a
            // click could otherwise land on a dialog still sliding in and be lost (seen once
            // dependencies were optimized and the tests outran the 250 ms slide).
            cx.set_reduce_motion(true);
        });
        // A restart opens the profile in the same process, where the earlier core may still be
        // letting it go (quit waits for it only so long); a relaunched app is a new process.
        let started = Instant::now();
        loop {
            match cx.update(|cx| crate::core::start(dir.clone(), cx)) {
                Ok(()) => break,
                Err(cause)
                    if owns_app
                        && cause.contains("already running")
                        && started.elapsed() < TIMEOUT =>
                {
                    std::thread::sleep(POLL * 10)
                }
                Err(cause) => panic!("the core opens the profile: {cause:?}"),
            }
        }
        cx.update(crate::startup::start);
        cx.run_until_parked();
        let window = cx
            .update(|cx| cx.windows().first().copied())
            .expect("the main window opened");
        // In front, as a launched app's window is: menu shortcuts act on the active window.
        cx.update_window(window, |_, window, _| window.activate_window())
            .expect("the main window is open");
        let mut harness = Harness {
            cx,
            window,
            profile: Some(profile),
            owns_app,
        };
        // Every test runs on a network, as the fixtures stand in for it: on a runner without a
        // route out, the app read the machine as offline (`e2e::shell::the_app_starts_on_home`
        // found Connect Lichess disabled).
        harness.online();
        harness.find("settings");
        harness
    }

    /// Let time pass as it would for a running app: the core's threads in real time, and the
    /// app's own timers (a save 400 ms after a change, animations) on GPUI's test clock.
    fn tick(&mut self) {
        std::thread::sleep(POLL);
        self.cx.executor().advance_clock(POLL);
        self.cx.run_until_parked();
    }

    /// Let the page come to rest: at least 20 looks, then until its controls hold still for 10.
    /// A fixed number of looks measured a page loading on a loaded machine and loaded on a quiet
    /// one, so a layout check saw a different page from run to run.
    pub fn settle(&mut self) {
        let layout = |app: &mut Harness| {
            app.with_window(|window, _| {
                window
                    .elements()
                    .iter()
                    .map(ElementSnapshot::bounds)
                    .collect::<Vec<_>>()
            })
        };
        for _ in 0..20 {
            self.tick();
        }
        let started = Instant::now();
        let mut last = layout(self);
        let mut still = 0;
        while still < 10 && started.elapsed() < TIMEOUT {
            self.tick();
            let now = layout(self);
            still = if now == last { still + 1 } else { 0 };
            last = now;
        }
    }

    /// Run `f` on the window after a fresh frame.
    pub fn with_window<R>(&mut self, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        self.cx.run_until_parked();
        self.cx
            .update_window(self.window, |_, window, cx| {
                draw(window, cx);
                f(window, cx)
            })
            .expect("the main window is open")
    }

    /// Drop a file named `name` holding `text` on the window, as Finder or Explorer would; the
    /// folder holding it lasts as long as what this returns.
    pub fn drop_file(&mut self, name: &str, text: &[u8]) -> tempfile::TempDir {
        let folder = tempfile::tempdir().expect("a folder");
        let path = folder.path().join(name);
        std::fs::write(&path, text).expect("the file");
        self.with_window(|window, cx| {
            let paths = gpui_kit::ExternalPaths(std::iter::once(path).collect());
            crate::links::open_dropped(&paths, window, cx);
        });
        folder
    }

    /// Wait, in real time, until `ready` holds for a fresh frame; `what` names it on failure.
    pub fn wait_for(&mut self, what: &str, ready: impl FnMut(&mut Window, &mut App) -> bool) {
        self.wait(what, ready, false);
    }

    /// `wait_for`, with GPUI's test clock kept up with real time: for a page whose clock reads
    /// real time but redraws on a timer (a game clock's tick). `tick` moves the test clock 5 ms a
    /// look, so under load it falls far behind: a 100 ms tick came seconds apart, a clock skipped
    /// the second a test waited for, and a low-time warning came late or never.
    pub fn wait_for_in_real_time(
        &mut self,
        what: &str,
        ready: impl FnMut(&mut Window, &mut App) -> bool,
    ) {
        self.wait(what, ready, true);
    }

    fn wait(
        &mut self,
        what: &str,
        mut ready: impl FnMut(&mut Window, &mut App) -> bool,
        real_time: bool,
    ) {
        let started = Instant::now();
        let mut last = started;
        loop {
            if self.with_window(&mut ready) {
                return;
            }
            if started.elapsed() > TIMEOUT {
                let paths = self.paths();
                panic!("timed out after {TIMEOUT:?} waiting for {what}. Registered: {paths}");
            }
            if real_time {
                std::thread::sleep(POLL);
                let now = Instant::now();
                self.cx.executor().advance_clock(now - last);
                last = now;
                self.cx.run_until_parked();
            } else {
                self.tick();
            }
        }
    }

    /// Wait until the app has shown a system notification `matches` takes, and return the latest
    /// such. The system's notifications are drawn by the app on Windows and Linux alone.
    #[cfg(any(windows, target_os = "linux"))]
    pub fn wait_for_notification(
        &mut self,
        what: &str,
        mut matches: impl FnMut(&gpui_kit::SystemNotification) -> bool,
    ) -> gpui_kit::SystemNotification {
        let started = Instant::now();
        loop {
            self.cx.run_until_parked();
            let shown = self.cx.shown_system_notifications();
            if let Some(found) = shown.into_iter().rev().find(|shown| matches(shown)) {
                return found;
            }
            if started.elapsed() > TIMEOUT {
                panic!("timed out after {TIMEOUT:?} waiting for {what}");
            }
            self.tick();
        }
    }

    /// Type `text` into the dialog's field labelled `label`, in place of what it holds, once the
    /// dialog has stopped moving.
    pub fn fill_in_dialog(&mut self, label: &str, text: &str) {
        use gpui_kit::test::TestQueryExt as _;
        let mut last = None;
        self.wait_for(&format!("{label:?} to stop moving"), |window, _| {
            let bounds = window.try_find_by_label(label).map(|input| input.bounds());
            let still = bounds.is_some() && bounds == last;
            last = bounds;
            still
        });
        self.click_label(label);
        self.press("secondary-a");
        self.type_text(text);
    }

    /// Wait until the element is shown, then return it.
    pub fn find(&mut self, id: impl Into<ElementId>) -> ElementSnapshot {
        let id = id.into();
        let mut found = None;
        self.wait_for(&format!("{id:?}"), |window, _| {
            found = window.try_find(id.clone()).filter(ElementSnapshot::visible);
            found.is_some()
        });
        found.expect("found")
    }

    /// Wait until the element is shown and has stopped moving (a dialog slides in), as a
    /// person's click would wait for it.
    fn settled(&mut self, id: &ElementId) -> ElementSnapshot {
        let mut last = None;
        let mut found = None;
        self.wait_for(&format!("{id:?} to stop moving"), |window, _| {
            found = window.try_find(id.clone()).filter(ElementSnapshot::visible);
            let bounds = found.as_ref().map(ElementSnapshot::bounds);
            let still = bounds.is_some() && bounds == last;
            last = bounds;
            still
        });
        found.expect("found")
    }

    /// Whether the control takes clicks: kit controls report only being disabled.
    pub fn enabled(&mut self, id: impl Into<ElementId>) -> bool {
        self.find(id).disabled() != Some(true)
    }

    /// Whether the element is in the current frame.
    pub fn shows(&mut self, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        self.with_window(|window, _| window.try_find(id).is_some_and(|e| e.visible()))
    }

    /// Wait until the element is no longer shown, as a dialog that closes on the core's answer.
    pub fn wait_gone(&mut self, id: impl Into<ElementId>) {
        let id = id.into();
        self.wait_for(&format!("{id:?} to go"), |window, _| {
            !window.try_find(id.clone()).is_some_and(|e| e.visible())
        });
    }

    /// Click the element once it is shown.
    pub fn click(&mut self, id: impl Into<ElementId>) {
        let id = id.into();
        self.settled(&id);
        self.with_window(|window, cx| window.click(id, cx));
        self.cx.run_until_parked();
    }

    /// Open a page from the sidebar by its label ("Play with computer"), once no dialog covers it.
    /// A dialog waiting on the core (naming a study) stays up until the core answers; a click on
    /// the sidebar under it landed on the dialog's overlay, so the page never opened, and Linux CI
    /// timed out in `studies_are_found_by_name` (2026.11.10).
    pub fn open(&mut self, label: &str) {
        self.wait_for("the dialog to close", |window, cx| {
            use gpui_kit::component::WindowExt as _;
            !window.has_active_dialog(cx)
        });
        let (group, row) = crate::shell::nav_groups()
            .iter()
            .enumerate()
            .find_map(|(group, (_, pages))| {
                let row = pages.iter().position(|page| page.info().label == label)?;
                Some((group, row))
            })
            .unwrap_or_else(|| panic!("no sidebar page {label:?}"));
        self.click(format!("{group}-{row}"));
    }

    /// Click the element with this accessible name, for elements whose ID is generated (inputs).
    pub fn click_label(&mut self, label: &str) {
        let mut target = None;
        self.wait_for(&format!("an element named {label:?}"), |window, _| {
            target = window
                .try_find_by_label(label)
                .filter(ElementSnapshot::visible);
            target.is_some()
        });
        self.click_point(target.expect("found").bounds().center());
    }

    /// Press and release at a point of the window, whatever lies there.
    pub fn click_point(&mut self, position: Point<Pixels>) {
        self.with_window(|window, cx| {
            for event in [
                MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }
                .to_platform_input(),
            ] {
                window.dispatch_event(event, cx);
                draw(window, cx);
            }
        });
        self.cx.run_until_parked();
    }

    /// Where a square ("e2") is in the window, wherever the board's orientation puts it.
    fn square(&mut self, square: &str, white_at_bottom: bool) -> Point<Pixels> {
        let board = self.find("chess-board").bounds();
        let side = board.size.width / 8.;
        let file = f32::from(square.as_bytes()[0] - b'a');
        let rank = f32::from(square.as_bytes()[1] - b'1');
        let (column, row) = if white_at_bottom {
            (file, 7. - rank)
        } else {
            (7. - file, rank)
        };
        board.origin + gpui_kit::point(side * (column + 0.5), side * (row + 0.5))
    }

    /// Click a square of the board ("e2").
    pub fn click_square(&mut self, square: &str, white_at_bottom: bool) {
        let offset =
            self.square(square, white_at_bottom) - self.find("chess-board").bounds().origin;
        self.with_window(|window, cx| window.click_at("chess-board", offset, cx));
        self.cx.run_until_parked();
    }

    /// Drag a piece from one square to another.
    pub fn drag_square(&mut self, from: &str, to: &str, white_at_bottom: bool) {
        let (from, to) = (
            self.square(from, white_at_bottom),
            self.square(to, white_at_bottom),
        );
        self.with_window(|window, cx| window.drag(from, to, cx));
        self.cx.run_until_parked();
    }

    /// Wait until the move list shows exactly these moves.
    pub fn wait_for_moves(&mut self, expected: &[&str]) {
        let started = Instant::now();
        loop {
            let shown = self.moves();
            if shown == expected {
                return;
            }
            if started.elapsed() >= TIMEOUT {
                let paths = self.paths();
                panic!("the moves stayed {shown:?}, expected {expected:?}. Registered: {paths}");
            }
            self.tick();
        }
    }

    /// The status banner's title, once one is shown.
    pub fn status(&mut self) -> String {
        let banner = self.find("status-banner");
        banner.label().unwrap_or_default().to_string()
    }

    /// The moves in the game's move list, in order.
    pub fn moves(&mut self) -> Vec<String> {
        self.with_window(|window, _| {
            (0usize..)
                .map_while(|index| window.try_find(("move", index)))
                .filter_map(|cell| cell.label().map(str::to_string))
                .collect()
        })
    }

    /// Press a key, as GPUI parses it ("escape", "secondary-,").
    pub fn press(&mut self, key: &str) {
        self.with_window(|window, cx| window.press(key, cx));
        self.cx.run_until_parked();
    }

    /// Press Tab until the board has the keys, as a keyboard reaches it.
    pub fn tab_to_board(&mut self) {
        for _ in 0..200 {
            if self.find("chess-board").focused() == Some(true) {
                return;
            }
            self.with_window(|window, cx| window.focus_next(cx));
        }
        panic!("Tab never reached the board");
    }

    /// The accessible name of a board's square ("e4").
    pub fn square_name(&mut self, square: &str) -> String {
        let square: shakmaty::Square = square.parse().expect("a square");
        self.find(("square", usize::from(square)))
            .label()
            .unwrap_or_default()
            .to_string()
    }

    /// Type text into the focused input.
    pub fn type_text(&mut self, text: &str) {
        self.with_window(|window, cx| window.input(text, cx));
        self.cx.run_until_parked();
    }

    /// Every registered element path, for failure messages.
    pub fn paths(&mut self) -> String {
        self.cx
            .update_window(self.window, |_, window, _| {
                gpui_kit::base::test_support::registered_paths(window)
            })
            .unwrap_or_default()
    }

    /// The settings as the core has them (the app saves a change after 400 ms).
    pub fn saved_settings(&mut self) -> serde_json::Value {
        let core = self
            .cx
            .update(|cx| cx.global::<crate::core::AppCore>().bridge.clone());
        core.call_now("desktopSettings", Vec::new())
            .expect("the core reads the settings")
    }

    /// Wait until the core has saved `key` as `value`.
    pub fn wait_for_saved(&mut self, key: &str, value: serde_json::Value) {
        let started = Instant::now();
        loop {
            self.cx.run_until_parked();
            let saved = self.saved_settings();
            if saved[key] == value {
                return;
            }
            assert!(
                started.elapsed() < TIMEOUT,
                "{key} stayed {} instead of {value}",
                saved[key]
            );
            self.tick();
        }
    }

    /// Open a category of Settings from its dialog's list ("Sound").
    pub fn open_settings(&mut self, label: &str) {
        if !self.shows(SETTINGS_SHOWN) {
            self.click("settings");
        }
        let index = crate::pages::settings::sections()
            .iter()
            .position(|(_, name, _)| *name == label)
            .unwrap_or_else(|| panic!("no Settings category {label:?}"));
        // A block for each group of categories, numbered from 1.
        let (mut block, mut row) = (1, index);
        for (_, size) in crate::pages::settings::SECTION_GROUPS {
            if row < size {
                break;
            }
            row -= size;
            block += 1;
        }
        self.click(format!("s{block}-{row}"));
    }
}

/// Settings' dialog, shown while Settings is open.
pub const SETTINGS_SHOWN: &str = "settings-dialog";

impl Drop for Harness {
    fn drop(&mut self) {
        // Saves are flushed and the core closed as at quit, before the profile is removed.
        let quit = self.cx.update(crate::core::quit);
        self.cx.foreground_executor().block_test(quit);
        if self.owns_app {
            // `#[gpui_kit::test]` drains the shared scheduler after the test, which runs this
            // app's tasks too: they need it alive until the test's thread ends.
            KEPT_APPS.with_borrow_mut(|kept| kept.push(self.cx.clone()));
        }
        crate::profile::TEST_DATA_DIR.set(None);
    }
}

/// The process-wide switches every test shares: the same values for all, set once
/// (`kchess_core::env`, which the app and its embedded core read in place of the environment).
fn environment() {
    use kchess_core::env::set;
    static SET: OnceLock<()> = OnceLock::new();
    SET.get_or_init(|| {
        let engine = engine();
        let lichess = std::env::var("KCHESS_E2E_LICHESS").unwrap_or_else(|_| fake_lichess::base());
        set("KCHESS_CORE_MODE", "embedded");
        // The scripted engine in its `search` mode, which a copy's name chooses.
        set("KCHESS_ENGINE_PATH", &engine);
        // Never a Fairy-Stockfish this checkout built: the scripted engine stands in for it.
        set("KCHESS_VARIANT_ENGINE_PATH", &engine);
        // The core's test switch: credentials in a key file in the profile, never the OS
        // credential store, and the test Lichess origin below honoured.
        set("KCHESS_STORE_DEBUG", "1");
        // Online features fail fast and offline, unless the test signed in.
        set("KCHESS_TEST_LICHESS_BASE", lichess);
        // Chess.com is a stand-in too: `e2e…` players exist, nobody else does.
        set("KCHESS_TEST_CHESSCOM_BASE", fake_chesscom::base());
        // A voice model that cannot be had: never the real download, and voice fails at once.
        set("KCHESS_TEST_VOICE_ARCHIVE", "/nonexistent/voice-model.zip");
        set("KCHESS_TEST_VOICE_NAME", "no-model");
        set("KCHESS_TEST_VOICE_SHA256", "0");
        // No update feed from the shell the tests run in: Settings → Updates shows one state
        // everywhere.
        kchess_core::env::unset("KCHESS_UPDATE_FEED");
    });
}

/// The scripted engine (`kchess-fake-uci`, built first), as the copy `engine-search`
/// beside it, which the engine reads its mode from.
fn engine() -> PathBuf {
    let target = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.to_path_buf()))
        .expect("the test executable is in target/<profile>/deps");
    let fake = target.join(format!("kchess-fake-uci{}", std::env::consts::EXE_SUFFIX));
    // Built every time: Cargo does nothing when it is fresh, and one left over from before a
    // change is never run.
    let mut build = std::process::Command::new(env!("CARGO"));
    build.args([
        "build",
        "-q",
        "--locked",
        "-p",
        "kchess-core",
        "--bin",
        "kchess-fake-uci",
    ]);
    // Without the variables Cargo set for this package (`CARGO_MANIFEST_DIR`, `OUT_DIR`, `CARGO_PKG_*`…):
    // build scripts that watch one (ring's) would rebuild for it, and the next outer build
    // rebuild them back.
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(|key| {
            key.starts_with("CARGO_PKG_")
                || key.starts_with("CARGO_BIN_EXE_")
                || matches!(
                    key,
                    "CARGO_MANIFEST_DIR"
                        | "CARGO_MANIFEST_PATH"
                        | "CARGO_CRATE_NAME"
                        | "CARGO_PRIMARY_PACKAGE"
                        | "CARGO_TARGET_TMPDIR"
                        | "CARGO_BIN_NAME"
                        | "OUT_DIR"
                )
        }) {
            build.env_remove(key);
        }
    }
    // Into the target folder this test runs from, which `cargo llvm-cov` moves with a flag that
    // a nested build does not inherit.
    build
        .arg("--target-dir")
        .arg(target.parent().expect("the target folder"));
    if target.file_name().is_some_and(|name| name == "release") {
        build.arg("--release");
    }
    let status = build.status().expect("cargo runs");
    assert!(status.success(), "the scripted engine builds");
    let copy = target.join(format!("engine-search{}", std::env::consts::EXE_SUFFIX));
    let same = |a: &Path, b: &Path| {
        std::fs::read(a)
            .ok()
            .zip(std::fs::read(b).ok())
            .is_some_and(|(a, b)| a == b)
    };
    if !same(&fake, &copy) {
        // Under another name first, so a test binary starting at the same time never runs half a copy.
        let partial = target.join(format!(
            "engine-search-{}{}",
            std::process::id(),
            std::env::consts::EXE_SUFFIX
        ));
        std::fs::copy(&fake, &partial).expect("copy the scripted engine");
        std::fs::rename(&partial, &copy).expect("name the scripted engine's copy");
    }
    copy
}
