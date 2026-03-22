use std::path::PathBuf;

use gpui::{
    App, AppContext, Application, Bounds, Focusable, KeyBinding, Menu, MenuItem, OsAction,
    SystemMenuType, TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowOptions,
    actions, point, px, size,
};
use kchess_board::{ChessBoardView, VisualBoard};

mod app;
mod app_shell;
mod assets;
mod pages;
mod search_input;
mod ui;
mod window_state;

use app_shell::{BOARD_CELL_PX, KChessApp};
use assets::Assets;
use search_input::SearchInput;
use window_state::{load_window_bounds, save_window_bounds};

actions!(
    kchess,
    [
        Quit,
        FocusSearch,
        OpenSettings,
        OpenPlayWithComputer,
        SearchUp,
        SearchDown,
        SearchConfirm,
        SearchCancel,
        SearchBackspace,
        SearchDelete,
        SearchLeft,
        SearchRight,
        SearchSelectLeft,
        SearchSelectRight,
        SearchSelectAll,
        SearchHome,
        SearchEnd,
        SearchPaste,
        SearchCut,
        SearchCopy,
        ShowCharacterPalette,
        ShowHelp,
    ]
);

fn main() {
    let assets = Assets::new(resolve_assets_root());

    Application::new().with_assets(assets).run(|cx: &mut App| {
        let default_bounds = Bounds::centered(None, size(px(1320.0), px(860.0)), cx);
        let startup_bounds = load_window_bounds().unwrap_or(WindowBounds::Windowed(default_bounds));

        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-f", FocusSearch, None),
            KeyBinding::new("cmd-,", OpenSettings, None),
            KeyBinding::new("up", SearchUp, Some("SearchOverlay")),
            KeyBinding::new("down", SearchDown, Some("SearchOverlay")),
            KeyBinding::new("enter", SearchConfirm, Some("SearchOverlay")),
            KeyBinding::new("escape", SearchCancel, Some("SearchOverlay")),
            KeyBinding::new("up", SearchUp, Some("SearchInput")),
            KeyBinding::new("down", SearchDown, Some("SearchInput")),
            KeyBinding::new("enter", SearchConfirm, Some("SearchInput")),
            KeyBinding::new("escape", SearchCancel, Some("SearchInput")),
            KeyBinding::new("backspace", SearchBackspace, Some("SearchInput")),
            KeyBinding::new("delete", SearchDelete, Some("SearchInput")),
            KeyBinding::new("left", SearchLeft, Some("SearchInput")),
            KeyBinding::new("right", SearchRight, Some("SearchInput")),
            KeyBinding::new("shift-left", SearchSelectLeft, Some("SearchInput")),
            KeyBinding::new("shift-right", SearchSelectRight, Some("SearchInput")),
            KeyBinding::new("cmd-a", SearchSelectAll, Some("SearchInput")),
            KeyBinding::new("cmd-v", SearchPaste, Some("SearchInput")),
            KeyBinding::new("cmd-c", SearchCopy, Some("SearchInput")),
            KeyBinding::new("cmd-x", SearchCut, Some("SearchInput")),
            KeyBinding::new("home", SearchHome, Some("SearchInput")),
            KeyBinding::new("end", SearchEnd, Some("SearchInput")),
            KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, Some("SearchInput")),
        ]);

        cx.set_menus(vec![
            Menu {
                name: "KChess".into(),
                items: vec![
                    MenuItem::os_submenu("Services", SystemMenuType::Services),
                    MenuItem::separator(),
                    MenuItem::action("Play with Computer", OpenPlayWithComputer),
                    MenuItem::action("Settings…", OpenSettings),
                    MenuItem::action("Find…", FocusSearch),
                    MenuItem::separator(),
                    MenuItem::action("Quit KChess", Quit),
                ],
            },
            Menu {
                name: "File".into(),
                items: vec![MenuItem::action("Play with Computer", OpenPlayWithComputer)],
            },
            Menu {
                name: "Edit".into(),
                items: vec![
                    MenuItem::os_action("Cut", SearchCut, OsAction::Cut),
                    MenuItem::os_action("Copy", SearchCopy, OsAction::Copy),
                    MenuItem::os_action("Paste", SearchPaste, OsAction::Paste),
                    MenuItem::separator(),
                    MenuItem::os_action("Select All", SearchSelectAll, OsAction::SelectAll),
                    MenuItem::separator(),
                    MenuItem::action("Find…", FocusSearch),
                ],
            },
            Menu {
                name: "View".into(),
                items: vec![
                    MenuItem::action("Play with Computer", OpenPlayWithComputer),
                    MenuItem::action("Settings", OpenSettings),
                ],
            },
            Menu {
                name: "Window".into(),
                items: vec![MenuItem::action("Find…", FocusSearch)],
            },
            Menu {
                name: "Help".into(),
                items: vec![MenuItem::action("KChess Help", ShowHelp)],
            },
        ]);

        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(startup_bounds),
                    window_min_size: Some(size(px(860.0), px(620.0))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("KChess".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(14.0), px(12.0))),
                    }),
                    window_background: WindowBackgroundAppearance::Blurred,
                    ..Default::default()
                },
                |_window, cx| {
                    let board_view = cx.new(|_| {
                        let mut board = ChessBoardView::new(VisualBoard::standard());
                        board.set_cell_size(BOARD_CELL_PX);
                        board
                    });

                    let search_input = cx.new(SearchInput::new);
                    cx.new(|cx| KChessApp::new(cx, board_view, search_input))
                },
            )
            .unwrap();

        window
            .update(cx, |view, window, cx| {
                window.focus(&view.focus_handle(cx));

                window.on_window_should_close(cx, |window, _cx| {
                    let _ = save_window_bounds(window.window_bounds());
                    true
                });

                cx.observe_window_bounds(window, KChessApp::save_current_window_bounds)
                    .detach();
            })
            .unwrap();

        cx.activate(true);
        cx.on_action(|_: &Quit, cx| cx.quit());
    });
}

fn resolve_assets_root() -> PathBuf {
    if let Ok(custom_assets_dir) = std::env::var("KCHESS_ASSETS_DIR") {
        let path = PathBuf::from(custom_assets_dir);
        if path.exists() {
            return path;
        }
    }

    if let Ok(exe_path) = std::env::current_exe()
        && let Some(macos_dir) = exe_path.parent()
        && let Some(contents_dir) = macos_dir.parent()
    {
        let bundled_assets = contents_dir.join("Resources").join("assets");
        if bundled_assets.exists() {
            return contents_dir.join("Resources");
        }
    }

    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
