//! KChess as a GPUI Kit application over the Rust core: the same profile, games, settings and
//! accounts as the Electron app, drawn natively.

mod accounts;
mod activity;
mod assets;
#[cfg(feature = "bench")]
mod bench;
mod board;
mod chart;
mod core;
mod dock_icon;
#[cfg(test)]
mod e2e;
mod engine_view;
mod explorer;
mod export;
mod game;
mod game_image;
mod interface_zoom;
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
mod position_engine;
mod power;
mod prefs;
mod profile;
mod puzzle_board;
mod puzzle_db;
mod quick_pick;
mod rating_history;
mod rating_speeds;
mod remote_image;
mod replay;
mod reviews;
mod settings;
mod shell;
mod sound;
mod startup;
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
            "KChess started (GPUI) {{ sessionId: {}, version: {}, client: gpui, platform: {}, arch: {} }}",
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
            if cx.has_global::<startup::Started>() {
                open_main_window(cx);
            } else if cx.has_global::<core::AppCore>() {
                startup::start(cx);
            }
        }
        cx.activate(true);
    });
    app.run(move |cx| {
        gpui_kit::init(cx);
        menu::init(cx);
        if let Err(message) = core::start(data_dir, cx) {
            log::write(Level::Error, "app", &message);
            explain_and_quit(message, cx);
            return;
        }
        cx.on_app_quit(core::quit).detach();
        dock_icon::apply(cx);
        startup::start(cx);
        cx.activate(true);
    });
}

/// The models, from the profile the core has read, then the window.
fn start_models(cx: &mut App) {
    settings::init(cx);
    interface_zoom::init(profile::data_dir().unwrap_or_default(), cx);
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
    cx.set_global(startup::Started);
    open_main_window(cx);
}

/// A blank stand-in window for the dialog below.
struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The core could not open the profile (another KChess release has it open, naming the app to
/// update; `docs/shared-core.md`): say so in a dialog, then quit.
fn explain_and_quit(message: String, cx: &mut App) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(420.), px(160.)), cx)),
        ..WindowOptions::default()
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        let answer = window.prompt(
            PromptLevel::Warning,
            "KChess cannot open this profile",
            Some(&message),
            &["Quit"],
            cx,
        );
        cx.spawn(async move |cx| {
            let _ = answer.await;
            cx.update(|cx| cx.quit());
        })
        .detach();
        cx.new(|_| Blank)
    });
    if opened.is_err() {
        cx.quit();
    }
}

/// The window, where and as it was left (`createWindow`).
fn main_window_options(cx: &mut App) -> WindowOptions {
    let dir = profile::data_dir().unwrap_or_default();
    let (window_bounds, _) = window_state::bounds(&dir, cx);
    WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(profile::APP_NAME.into()),
            appears_transparent: true,
            // As Electron's, beside the sidebar toggle; `interface_zoom::apply` keeps them on the
            // top bar's middle at every interface size.
            traffic_light_position: Some(point(px(12.), px(21.))),
        }),
        window_bounds,
        window_min_size: Some(size(
            px(window_state::MIN_SIZE.0),
            px(window_state::MIN_SIZE.1),
        )),
        app_id: Some("com.kchess.app".into()),
        ..WindowOptions::default()
    }
}

fn open_main_window(cx: &mut App) {
    let options = main_window_options(cx);
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        // The window's own close button saves first too (`menu::close_window`).
        window.on_window_should_close(cx, |window, cx| {
            let handle = window.window_handle();
            cx.defer(move |cx| menu::close_window(handle, cx));
            false
        });
        cx.new(|cx| shell::Workspace::new(window, cx))
    });
    #[cfg(feature = "bench")]
    if let Ok(window) = &opened {
        bench::start(window.0, cx);
    }
    if let Err(cause) = opened {
        log::write(
            Level::Error,
            "app",
            &format!("The window could not open: {cause}"),
        );
        cx.quit();
    }
}
