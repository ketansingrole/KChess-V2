//! KChess as a GPUI Kit application over the Rust core: the same profile, games, settings and
//! accounts as the Electron app (`apps/desktop`), drawn natively.

mod assets;
mod board;
mod chart;
mod core;
mod explorer;
mod export;
mod game;
mod log;
mod menu;
mod name_dialog;
mod online;
mod pages;
mod play_layout;
mod played;
mod power;
mod prefs;
mod profile;
mod puzzle_board;
mod replay;
mod reviews;
mod settings;
mod shell;
mod sound;
mod studies;
mod theme;
mod widgets;

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
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(move |cx| {
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
            sound::init(cx);
            online::init(cx);
            cx.on_app_quit(core::quit).detach();
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some(profile::APP_NAME.into()),
                    appears_transparent: true,
                    // Centred in the sidebar header, as Electron's hiddenInset lights sit.
                    traffic_light_position: Some(point(px(16.), px(18.))),
                }),
                window_bounds: Some(WindowBounds::centered(size(px(1280.), px(820.)), cx)),
                window_min_size: Some(size(px(720.), px(520.))),
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
            cx.activate(true);
        });
}
