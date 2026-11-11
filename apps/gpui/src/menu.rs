//! The app's menus and their shortcuts: on macOS
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
        ShowShortcuts,
        ShowWhatsNew,
        ToggleSidebar,
        Find,
        New,
        ToggleZen,
        FlipBoard
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
    unless_leaving_a_game("Close", cx, move |cx| {
        crate::quit::prepare_quit(
            cx,
            "Close without saving",
            move |cx| {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            },
            |_| {},
        );
    });
}

/// Run `then`, after asking first while a real-time Lichess game is on: its clock keeps running
/// once KChess is gone, and the game is lost on time.
fn unless_leaving_a_game(leave: &'static str, cx: &mut App, then: impl FnOnce(&mut App) + 'static) {
    let on = cx
        .try_global::<crate::online::OnlineHandle>()
        .is_some_and(|_| crate::online::handle(cx).read(cx).live_game_on());
    if !on {
        return then(cx);
    }
    // After the update that brought the key or the click: the window cannot ask during it.
    cx.defer(move |cx| {
        let window = cx.active_window().or_else(|| cx.windows().first().copied());
        let answer = window.and_then(|window| {
            window
                .update(cx, |_, window, cx| {
                    window.prompt(
                        PromptLevel::Warning,
                        "Your Lichess game is still on",
                        Some("Its clock keeps running on Lichess."),
                        &[leave, "Cancel"],
                        cx,
                    )
                })
                .ok()
        });
        let Some(answer) = answer else {
            return then(cx);
        };
        cx.spawn(async move |cx| {
            if answer.await == Ok(0) {
                cx.update(then);
            }
        })
        .detach();
    });
}

/// The keys bound to `action`, as the shortcuts list, Quick Pick and tooltips show them: "⌘ K" on
/// macOS, "Ctrl K" elsewhere. The first binding made for it outside any context wins (`init`
/// makes the one to show first), else its first.
pub fn keys(action: &dyn Action, cx: &App) -> Option<String> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let first = keymap.bindings_for_action(action).next().map(binding_label);
    keymap
        .bindings_for_action(action)
        .find(|binding| binding.predicate().is_none())
        .map(binding_label)
        .or(first)
}

/// "Flip board", with the key that turns the board.
pub fn flip_label(cx: &App) -> String {
    match keys(&FlipBoard, cx) {
        Some(key) => format!("Flip board ({key})"),
        None => "Flip board".into(),
    }
}

/// Every binding of `action`, in the order `init` makes them.
pub fn all_keys(action: &dyn Action, cx: &App) -> Vec<String> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    keymap
        .bindings_for_action(action)
        .map(binding_label)
        .collect()
}

/// The key of `action`'s binding alone, without its modifiers (the − of ⌘ −).
pub fn key_alone(action: &dyn Action, cx: &App) -> Option<String> {
    keys(action, cx)?.rsplit(' ').next().map(str::to_string)
}

fn binding_label(binding: &KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(|keystroke| keystroke_label(keystroke.modifiers(), keystroke.key()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// One keystroke as its chips read: the modifiers in the platform's order, then the key.
fn keystroke_label(modifiers: &Modifiers, key: &str) -> String {
    let mac = cfg!(target_os = "macos");
    let mut parts: Vec<String> = Vec::new();
    if modifiers.control {
        parts.push(if mac { "⌃" } else { "Ctrl" }.into());
    }
    if modifiers.alt {
        parts.push(if mac { "⌥" } else { "Alt" }.into());
    }
    if modifiers.shift {
        parts.push("⇧".into());
    }
    if modifiers.platform {
        parts.push(if mac { "⌘" } else { "Win" }.into());
    }
    parts.push(match key {
        "left" => "←".into(),
        "right" => "→".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "-" => "−".into(),
        "escape" => "Esc".into(),
        "enter" => "Enter".into(),
        "space" => "Space".into(),
        "delete" => "Delete".into(),
        key => key.to_uppercase(),
    });
    parts.join(" ")
}

/// A binding whose `secondary-` is the host's Cmd or Ctrl. GPUI resolves `secondary-` only when
/// it loads a binding for the running platform (`KeyBinding::new` refuses it).
fn bind(keystrokes: &str, action: impl Action, cx: &App) -> KeyBinding {
    KeyBinding::load(keystrokes, Box::new(action), None, false, None, cx)
        .unwrap_or_else(|error| panic!("menu binding {keystrokes:?}: {error:?}"))
}

pub fn init(cx: &mut App) {
    // Ctrl+Y redoes too, as text fields take it outside macOS; the shortcuts list shows it.
    if !cfg!(target_os = "macos") {
        cx.bind_keys([bind("ctrl-y", Redo, cx)]);
    }
    let bindings = [
        // Cmd on macOS and Ctrl elsewhere; the others are macOS's own.
        bind("secondary-q", Quit, cx),
        bind("cmd-h", Hide, cx),
        bind("alt-cmd-h", HideOthers, cx),
        bind("secondary-w", CloseWindow, cx),
        bind("cmd-m", Minimize, cx),
        bind("ctrl-cmd-f", ToggleFullScreen, cx),
        bind("secondary-k", Search, cx),
        bind("secondary-f", Find, cx),
        bind("secondary-n", New, cx),
        bind("shift-secondary-p", Commands, cx),
        bind("secondary-,", OpenSettings, cx),
        // Interface size: with or without Shift, and the keypad's.
        bind("secondary-+", ZoomIn, cx),
        bind("secondary-=", ZoomIn, cx),
        bind("secondary-shift-=", ZoomIn, cx),
        bind("secondary--", ZoomOut, cx),
        bind("secondary-shift--", ZoomOut, cx),
        bind("secondary-0", ActualSize, cx),
        bind("secondary-/", ShowShortcuts, cx),
        // Outside a text field, Undo and Redo reach the page (the analysis board's edits); a
        // field keeps its own binding for its text.
        bind("secondary-z", Undo, cx),
        bind("shift-secondary-z", Redo, cx),
        // Outside a text field, Paste opens a copied game, position or Lichess link, and Copy
        // copies the board's game or position.
        bind("secondary-v", Paste, cx),
        bind("secondary-c", Copy, cx),
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
    // The sidebar as macOS apps take it (View > Show Sidebar), and as editors do elsewhere.
    cx.bind_keys([if cfg!(target_os = "macos") {
        bind("ctrl-cmd-s", ToggleSidebar, cx)
    } else {
        bind("ctrl-b", ToggleSidebar, cx)
    }]);
    // ? lists the shortcuts as on Lichess, Z turns zen mode on or off on a game page and F turns
    // the board, but only outside a text field, where they are typed.
    cx.bind_keys([
        KeyBinding::new("?", ShowShortcuts, Some("!Input")),
        KeyBinding::new("z", ToggleZen, Some("!Input")),
        KeyBinding::new("f", FlipBoard, Some("!Input")),
    ]);
    cx.on_action(|_: &ShowShortcuts, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, cx| crate::shortcuts::open(window, cx));
            }
        })
    });
    cx.on_action(|_: &ShowWhatsNew, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, cx| crate::whats_new::open(window, cx));
            }
        })
    });
    cx.on_action(|_: &Copy, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, cx| crate::shell::copy_on_page(window, cx));
            }
        })
    });
    cx.on_action(|_: &Paste, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, cx| crate::links::open_pasted(window, cx));
            }
        })
    });
    cx.on_action(|_: &New, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, cx| crate::shell::new_on_page(window, cx));
            }
        })
    });
    cx.on_action(|_: &ToggleSidebar, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, cx| crate::shell::toggle_sidebar(window, cx));
            }
        })
    });
    cx.on_action(|_: &GoBack, cx| go(-1, cx));
    cx.on_action(|_: &GoForward, cx| go(1, cx));
    cx.on_action(|_: &ZoomIn, cx| zoom(1, cx));
    cx.on_action(|_: &ZoomOut, cx| zoom(-1, cx));
    cx.on_action(|_: &ActualSize, cx| zoom(0, cx));
    cx.on_action(|_: &Search, cx| show_palette("", cx));
    // Find searches the page where it has a search field of its own, and everything elsewhere.
    cx.on_action(|_: &Find, cx| {
        cx.defer(|cx| {
            let found = cx.active_window().is_some_and(|window| {
                window
                    .update(cx, |_, window, cx| crate::shell::find_on_page(window, cx))
                    .unwrap_or(false)
            });
            if !found {
                show_palette("", cx);
            }
        })
    });
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
        unless_leaving_a_game("Quit", cx, |cx| {
            crate::quit::prepare_quit(cx, "Quit without saving", crate::quit::quit_saved, |_| {});
        });
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
            MenuItem::action("What's New", ShowWhatsNew),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide KChess", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit KChess", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New", New),
            MenuItem::separator(),
            MenuItem::action("Close Window", CloseWindow),
        ]),
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
            MenuItem::action("Find on Page", Find),
            MenuItem::action("Run a Command…", Commands),
            MenuItem::separator(),
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
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
