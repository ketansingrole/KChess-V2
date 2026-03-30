use std::path::PathBuf;

use gpui::{
    App, AppContext, Application, Bounds, Focusable, KeyBinding, Menu, MenuItem, OsAction,
    SystemMenuType, TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowOptions,
    actions, point, px, size,
};
use kchess_board::{BoardTheme, ChessBoardView, VisualBoard};

mod app;
mod app_shell;
mod assets;
mod lichess;
mod lichess_auth;
mod lichess_state;
mod pages;
mod search_input;
mod storage;
mod ui;
mod window_state;

use app_shell::{BOARD_CELL_PX, KChessApp};
use assets::Assets;
use lichess_state::LichessState;
use pages::{HistoryPage, SettingsPage};
use search_input::SearchInput;
use storage::BoardLooksSettings;
use window_state::{load_window_bounds, save_window_bounds};

actions!(
    kchess,
    [
        Quit,
        FocusSearch,
        OpenSettings,
        OpenPlayWithComputer,
        OpenHistory,
        PlayHistoryStepBack,
        PlayHistoryStepForward,
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
    let startup_board_looks = storage::load_board_looks_settings();

    Application::new()
        .with_assets(assets)
        .run(move |cx: &mut App| {
            let default_bounds = Bounds::centered(None, size(px(1320.0), px(860.0)), cx);
            let startup_bounds =
                load_window_bounds().unwrap_or(WindowBounds::Windowed(default_bounds));
            let startup_board_looks = startup_board_looks.clone();

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
                KeyBinding::new("left", PlayHistoryStepBack, Some("PlayMoveHistory")),
                KeyBinding::new("right", PlayHistoryStepForward, Some("PlayMoveHistory")),
                KeyBinding::new("left", PlayHistoryStepBack, None),
                KeyBinding::new("right", PlayHistoryStepForward, None),
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
                        MenuItem::action("History", OpenHistory),
                        MenuItem::action("Settings…", OpenSettings),
                        MenuItem::action("Find…", FocusSearch),
                        MenuItem::separator(),
                        MenuItem::action("Quit KChess", Quit),
                    ],
                },
                Menu {
                    name: "File".into(),
                    items: vec![
                        MenuItem::action("Play with Computer", OpenPlayWithComputer),
                        MenuItem::action("History", OpenHistory),
                    ],
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
                        MenuItem::action("History", OpenHistory),
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
                        let board_looks = startup_board_looks.clone();
                        let board_view = cx.new(|_| {
                            let mut board = ChessBoardView::new(VisualBoard::standard());
                            board.set_cell_size(BOARD_CELL_PX);
                            let light_square =
                                storage::parse_hex_color(&board_looks.light_square_hex).unwrap_or(
                                    storage::parse_hex_color(storage::DEFAULT_LIGHT_SQUARE_HEX)
                                        .unwrap_or_else(|| gpui::rgb(0xf0d9b5)),
                                );
                            let dark_square =
                                storage::parse_hex_color(&board_looks.dark_square_hex).unwrap_or(
                                    storage::parse_hex_color(storage::DEFAULT_DARK_SQUARE_HEX)
                                        .unwrap_or_else(|| gpui::rgb(0xb58863)),
                                );
                            board.set_theme(BoardTheme {
                                light_square,
                                dark_square,
                                selected_outline: BoardTheme::default().selected_outline,
                            });
                            board
                        });

                        let search_input = cx.new(SearchInput::new);
                        let lichess_state = cx.new(LichessState::new);
                        let settings_view = cx.new(|cx| {
                            SettingsPage::new(
                                cx,
                                board_view.clone(),
                                lichess_state.clone(),
                                BoardLooksSettings::new(
                                    board_looks.light_square_hex.clone(),
                                    board_looks.dark_square_hex.clone(),
                                    board_looks.theme_preset.clone(),
                                ),
                            )
                        });
                        let history_view = cx.new(|cx| HistoryPage::new(cx, lichess_state.clone()));
                        cx.new(|cx| {
                            KChessApp::new(
                                cx,
                                board_view,
                                history_view,
                                settings_view,
                                search_input,
                            )
                        })
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
