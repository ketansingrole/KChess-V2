//! The app's menus and their shortcuts, as the Electron app sets them (`setupAppMenu`): on macOS
//! the KChess menu (Services, Hide, Quit), then File, Edit, View and Window.

use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::*;

gpui_kit::actions!(
    kchess,
    [
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        CloseWindow,
        Minimize,
        Zoom,
        ToggleFullScreen,
        Search,
        Commands,
        OpenSettings,
        ZoomIn,
        ZoomOut,
        ActualSize,
        GoBack,
        GoForward,
        ShowShortcuts
    ]
);

/// Act on the window in front. A key reaches its action while that window is being updated, so
/// the window is changed just after.
fn on_window(cx: &mut App, change: fn(&mut Window)) {
    cx.defer(move |cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| change(window));
        }
    });
}

/// Interface size one step either way, or back to Auto; changed after the key's dispatch.
fn zoom(direction: i8, cx: &mut App) {
    cx.defer(move |cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, cx| {
                crate::interface_zoom::step_window(direction, window, cx)
            });
        }
    });
}

/// Back (-1) or forward (1) through the pages visited, as a swipe goes; after the key's dispatch.
fn go(direction: crate::swipe::Direction, cx: &mut App) {
    cx.defer(move |cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, cx| crate::shell::go(direction, window, cx));
        }
    });
}

/// ⌘K (`prefix` empty) or ⇧⌘P (`>`): the palette, open.
fn show_palette(prefix: &'static str, cx: &mut App) {
    let Some(palette) = cx
        .try_global::<crate::palette::PaletteHandle>()
        .map(|handle| handle.0.clone())
    else {
        return;
    };
    // The key reaches here while the window is being updated; it is updated next.
    cx.defer(move |cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, cx| {
                palette.update(cx, |palette, cx| palette.show(prefix, window, cx))
            });
        }
    });
}

/// Close `window` once everything is saved, with it still open to say what could not be: its
/// pages own the unsaved work, and on macOS closing it does not quit the app.
pub fn close_window(window: AnyWindowHandle, cx: &mut App) {
    crate::core::prepare_quit(
        cx,
        "Close without saving",
        move |cx| {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        },
        |_| {},
    );
}

/// A binding whose `secondary-` is the host's Cmd or Ctrl. GPUI resolves `secondary-` only when
/// it loads a binding for the running platform (`KeyBinding::new` refuses it).
fn bind(keystrokes: &str, action: impl Action, cx: &App) -> KeyBinding {
    KeyBinding::load(keystrokes, Box::new(action), None, false, None, cx)
        .unwrap_or_else(|error| panic!("menu binding {keystrokes:?}: {error:?}"))
}

pub fn init(cx: &mut App) {
    let bindings = [
        // Cmd on macOS and Ctrl elsewhere, as Electron's `CmdOrCtrl`; the others are macOS's own.
        bind("secondary-q", Quit, cx),
        bind("cmd-h", Hide, cx),
        bind("alt-cmd-h", HideOthers, cx),
        bind("secondary-w", CloseWindow, cx),
        bind("cmd-m", Minimize, cx),
        bind("ctrl-cmd-f", ToggleFullScreen, cx),
        bind("secondary-k", Search, cx),
        bind("shift-secondary-p", Commands, cx),
        bind("secondary-,", OpenSettings, cx),
        // Interface size, as Electron takes them: with or without Shift, and the keypad's.
        bind("secondary-=", ZoomIn, cx),
        bind("secondary-+", ZoomIn, cx),
        bind("secondary-shift-=", ZoomIn, cx),
        bind("secondary--", ZoomOut, cx),
        bind("secondary-shift--", ZoomOut, cx),
        bind("secondary-0", ActualSize, cx),
        bind("secondary-/", ShowShortcuts, cx),
    ];
    cx.bind_keys(bindings);
    // Back and forward through the pages, as a browser takes them: ⌘[ and ⌘] on macOS, Alt
    // with an arrow elsewhere.
    cx.bind_keys(if cfg!(target_os = "macos") {
        [bind("cmd-[", GoBack, cx), bind("cmd-]", GoForward, cx)]
    } else {
        [
            bind("alt-left", GoBack, cx),
            bind("alt-right", GoForward, cx),
        ]
    });
    cx.on_action(|_: &ShowShortcuts, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, cx| crate::shortcuts::open(window, cx));
            }
        })
    });
    cx.on_action(|_: &GoBack, cx| go(-1, cx));
    cx.on_action(|_: &GoForward, cx| go(1, cx));
    cx.on_action(|_: &ZoomIn, cx| zoom(1, cx));
    cx.on_action(|_: &ZoomOut, cx| zoom(-1, cx));
    cx.on_action(|_: &ActualSize, cx| zoom(0, cx));
    cx.on_action(|_: &Search, cx| show_palette("", cx));
    cx.on_action(|_: &Commands, cx| show_palette(">", cx));
    cx.on_action(|_: &OpenSettings, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, cx| {
                    crate::shell::open(crate::pages::Page::Settings, window, cx)
                });
            }
        })
    });
    // Everything is saved, with the window still open to say what could not be, before quitting.
    cx.on_action(|_: &Quit, cx| {
        crate::core::prepare_quit(cx, "Quit without saving", |cx| cx.quit(), |_| {});
    });
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &CloseWindow, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                close_window(window, cx);
            }
        })
    });
    cx.on_action(|_: &Minimize, cx| on_window(cx, |window| window.minimize_window()));
    cx.on_action(|_: &Zoom, cx| on_window(cx, |window| window.zoom_window()));
    cx.on_action(|_: &ToggleFullScreen, cx| on_window(cx, |window| window.toggle_fullscreen()));
    // Closing the window ends the app, which then saves and closes the core; macOS keeps it
    // running, as its apps do, and the Dock icon opens the window again.
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() && !cfg!(target_os = "macos") {
            cx.quit();
        }
    })
    .detach();
    cx.set_menus([
        Menu::new(crate::profile::APP_NAME).items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide KChess", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit KChess", Quit),
        ]),
        Menu::new("File").items([MenuItem::action("Close Window", CloseWindow)]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        ]),
        Menu::new("View").items([
            MenuItem::action("Search…", Search),
            MenuItem::action("Run a Command…", Commands),
            MenuItem::separator(),
            MenuItem::action("Back", GoBack),
            MenuItem::action("Forward", GoForward),
            MenuItem::separator(),
            MenuItem::action("Actual Size", ActualSize),
            MenuItem::action("Zoom In", ZoomIn),
            MenuItem::action("Zoom Out", ZoomOut),
            MenuItem::separator(),
            MenuItem::action("Toggle Full Screen", ToggleFullScreen),
            MenuItem::separator(),
            MenuItem::action("Keyboard Shortcuts", ShowShortcuts),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
        ]),
    ]);
}
