//! End-to-end tests of the GPUI app: the whole app (embedded core, models, the workspace window)
//! started in a headless GPUI window over a fresh profile, then driven by clicks and keys as a
//! person would, and checked by what the window shows. The counterpart of the Electron app's
//! Playwright suite (`tests/e2e`).
//!
//! Each test gets its own profile (`profile::TEST_DATA_DIR`), the scripted engine from
//! `kchess-core` in place of Stockfish (`kchess-fake-uci search`: it always answers `e2e4`), and a
//! Lichess origin nobody listens on, so nothing leaves the machine.
//!
//! Elements are found by their GPUI `ElementId` among those registered with `.test_support()`
//! (`gpui_kit::test`); `Harness::paths` lists them when a lookup fails.

mod computer;
mod shell;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui_kit::test::{ElementSnapshot, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, ElementId, InputEvent as _, Modifiers, MouseButton,
    MouseDownEvent, MouseUpEvent, Pixels, Point, TestAppContext, Window,
};

/// How long a step may take to show: real time, since the core answers on its own threads.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The app, started over its own profile in a headless window.
pub struct Harness {
    pub cx: TestAppContext,
    pub window: AnyWindowHandle,
    profile: tempfile::TempDir,
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
        crate::profile::TEST_DATA_DIR.set(Some(profile.path().to_path_buf()));
        // The core answers on its Tokio threads; the test waits for them in real time.
        cx.executor().allow_parking();
        let dir = profile.path().to_path_buf();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::menu::init(cx);
            crate::core::start(dir, cx).expect("the core opens the profile");
            crate::startup::start(cx);
        });
        cx.run_until_parked();
        let window = cx
            .update(|cx| cx.windows().first().copied())
            .expect("the main window opened");
        let mut harness = Harness {
            cx: cx.clone(),
            window,
            profile,
        };
        harness.find("settings");
        harness
    }

    pub fn profile(&self) -> &Path {
        self.profile.path()
    }

    /// Run `f` on the window after a fresh frame.
    pub fn with_window<R>(&mut self, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        self.cx.run_until_parked();
        self.cx
            .update_window(self.window, |_, window, cx| {
                window.render_frame(cx);
                f(window, cx)
            })
            .expect("the main window is open")
    }

    /// Wait, in real time, until `ready` holds for a fresh frame; `what` names it on failure.
    pub fn wait_for(&mut self, what: &str, mut ready: impl FnMut(&mut Window, &mut App) -> bool) {
        let started = Instant::now();
        loop {
            if self.with_window(&mut ready) {
                return;
            }
            if started.elapsed() > TIMEOUT {
                let paths = self.paths();
                panic!("timed out after {TIMEOUT:?} waiting for {what}. Registered: {paths}");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
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

    /// Click the element once it is shown.
    pub fn click(&mut self, id: impl Into<ElementId>) {
        let id = id.into();
        self.settled(&id);
        self.with_window(|window, cx| window.click(id, cx));
        self.cx.run_until_parked();
    }

    /// Drag from one element's centre to another's.
    pub fn drag(&mut self, from: impl Into<ElementId>, to: impl Into<ElementId>) {
        let (from, to) = (from.into(), to.into());
        self.find(from.clone());
        self.find(to.clone());
        self.with_window(|window, cx| window.drag_to(from, to, cx));
        self.cx.run_until_parked();
    }

    /// Open a page from the sidebar by its label ("Play with Computer").
    pub fn open(&mut self, label: &str) {
        let index = crate::shell::nav()
            .iter()
            .position(|(_, name, _)| *name == label)
            .unwrap_or_else(|| panic!("no sidebar page {label:?}"));
        self.click(format!("0-{index}"));
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
        let position = target.expect("found").bounds().center();
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
                window.render_frame(cx);
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
        let mut shown = Vec::new();
        let started = Instant::now();
        loop {
            shown = self.moves();
            if shown == expected {
                return;
            }
            assert!(
                started.elapsed() < TIMEOUT,
                "the moves stayed {shown:?}, expected {expected:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
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

    /// Press a key, as GPUI parses it ("escape", "cmd-,").
    pub fn press(&mut self, key: &str) {
        self.with_window(|window, cx| window.press(key, cx));
        self.cx.run_until_parked();
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

    /// Wait until the settings the app saves (after 400 ms) are on disk, and return them.
    pub fn saved_settings(&mut self) -> serde_json::Value {
        let core = self
            .cx
            .update(|cx| cx.global::<crate::core::AppCore>().bridge.clone());
        core.call_now("desktopSettings", Vec::new())
            .expect("the core reads the settings")
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Saves are flushed and the core closed as at quit, before the profile is removed.
        let quit = self.cx.update(crate::core::quit);
        self.cx.foreground_executor().block_test(quit);
        crate::profile::TEST_DATA_DIR.set(None);
    }
}

/// The process-wide switches every test shares: the same values for all, set once.
#[allow(unsafe_code)]
fn environment() {
    static SET: OnceLock<()> = OnceLock::new();
    SET.get_or_init(|| {
        let engine = engine();
        // SAFETY: set once, before any test's core starts reading the environment.
        unsafe {
            std::env::set_var("KCHESS_CORE_MODE", "embedded");
            std::env::set_var("KCHESS_ENGINE_PATH", engine);
            std::env::set_var("KCHESS_STORE_DEBUG", "1");
            // Lichess at a port nothing listens on: online features fail fast and offline.
            std::env::set_var("KCHESS_TEST_LICHESS_BASE", "http://127.0.0.1:9");
        }
    });
}

/// `kchess-fake-uci search` as an executable the core can start without arguments.
fn engine() -> PathBuf {
    let target = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.to_path_buf()))
        .expect("the test executable is in target/<profile>/deps");
    let fake = target.join(format!("kchess-fake-uci{}", std::env::consts::EXE_SUFFIX));
    if !fake.exists() {
        let release = target.file_name().is_some_and(|name| name == "release");
        let mut build = std::process::Command::new(env!("CARGO"));
        build.args([
            "build",
            "-q",
            "-p",
            "kchess-core",
            "--bin",
            "kchess-fake-uci",
        ]);
        if release {
            build.arg("--release");
        }
        let status = build.status().expect("cargo runs");
        assert!(status.success(), "the scripted engine builds");
    }
    let dir = std::env::temp_dir().join(format!("kchess-gpui-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a folder for the engine wrapper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let wrapper = dir.join("stockfish");
        std::fs::write(
            &wrapper,
            format!("#!/bin/sh\nexec '{}' search \"$@\"\n", fake.display()),
        )
        .expect("the engine wrapper is written");
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))
            .expect("the engine wrapper is executable");
        wrapper
    }
    #[cfg(windows)]
    {
        let wrapper = dir.join("stockfish.cmd");
        std::fs::write(&wrapper, format!("@\"{}\" search %*\r\n", fake.display()))
            .expect("the engine wrapper is written");
        wrapper
    }
}
