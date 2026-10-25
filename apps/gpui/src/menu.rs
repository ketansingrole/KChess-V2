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
        OpenSettings
    ]
);

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

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
        KeyBinding::new("cmd-k", Search, None),
        KeyBinding::new("shift-cmd-p", Commands, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
    ]);
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
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &CloseWindow, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        }
    });
    cx.on_action(|_: &Minimize, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.minimize_window());
        }
    });
    cx.on_action(|_: &Zoom, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.zoom_window());
        }
    });
    cx.on_action(|_: &ToggleFullScreen, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.toggle_fullscreen());
        }
    });
    // Closing the window ends the app, which then saves and closes the core.
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
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
            MenuItem::action("Toggle Full Screen", ToggleFullScreen),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
        ]),
    ]);
}
