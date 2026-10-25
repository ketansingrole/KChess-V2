//! KChess as a GPUI Kit application over the Rust core: the same profile, games, settings and
//! accounts as the Electron app (`apps/desktop`), drawn natively.

mod accounts;
mod assets;
mod board;
mod chart;
mod core;
mod explorer;
mod export;
mod game;
mod game_image;
mod log;
mod menu;
mod microphone;
mod name_dialog;
mod network;
mod notifications;
mod online;
mod pages;
mod palette;
mod play_layout;
mod played;
mod power;
mod prefs;
mod profile;
mod puzzle_board;
mod puzzle_db;
mod quick_pick;
mod rating_history;
mod replay;
mod reviews;
mod settings;
mod shell;
mod sound;
mod studies;
mod swipe;
mod theme;
mod updates;
mod voice;
mod widgets;
mod window_state;

use gpui_kit::*;
use kchess_core::Level;

fn main() {
    let data_dir = match profile::data_dir() {
        Ok(dir) => dir,
        Err(cause) => {
            log::write(
                Level::Error,
                "app",
                &format!("The profile could not be created: {cause}"),
            );
            std::process::exit(1);
        }
    };
    log::init(&data_dir);
    log::write(
        Level::Info,
        "diagnostics",
        &format!(
            "KChess started (GPUI) {{ sessionId: {}, version: {}, platform: {}, arch: {} }}",
            log::session_id(),
            updates::current_version(),
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    );
    let app = gpui_kit::application().with_assets(assets::Assets);
    // macOS keeps KChess running with no window; the Dock icon brings one back.
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            open_main_window(cx);
        }
        cx.activate(true);
    });
    app.run(move |cx| {
        gpui_kit::init(cx);
        menu::init(cx);
        if let Err(message) = core::start(data_dir, cx) {
            log::write(Level::Error, "app", &message);
            cx.quit();
            return;
        }
        settings::init(cx);
        prefs::init(cx);
        played::init(cx);
        studies::init(cx);
        reviews::init(cx);
        accounts::init(cx);
        puzzle_db::init(cx);
        updates::init(cx);
        sound::init(cx);
        online::init(cx);
        network::init(cx);
        voice::init(cx);
        cx.on_app_quit(core::quit).detach();
        open_main_window(cx);
        cx.activate(true);
    });
}

/// The window, where and as it was left (`createWindow`).
fn open_main_window(cx: &mut App) {
    let dir = profile::data_dir().unwrap_or_default();
    let (window_bounds, _) = window_state::bounds(&dir, cx);
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(profile::APP_NAME.into()),
            appears_transparent: true,
            // Centred in the sidebar header, as Electron's hiddenInset lights sit.
            traffic_light_position: Some(point(px(16.), px(18.))),
        }),
        window_bounds,
        window_min_size: Some(size(
            px(window_state::MIN_SIZE.0),
            px(window_state::MIN_SIZE.1),
        )),
        app_id: Some("com.kchess.app".into()),
        ..WindowOptions::default()
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| shell::Workspace::new(window, cx))
    });
    if let Err(cause) = opened {
        log::write(
            Level::Error,
            "app",
            &format!("The window could not open: {cause}"),
        );
        cx.quit();
    }
}
