//! KChess as a GPUI Kit application over the Rust core: the profile's games, settings and
//! accounts, drawn natively.

// A Windows GUI program: release builds open no console window beside the app (logs go to the
// profile's log file). Debug builds keep the console for `cargo run` output.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

mod account_menu;
mod activity;
mod assets;
mod background;
#[cfg(feature = "bench")]
mod bench;
mod core;
mod dock_icon;
#[cfg(test)]
mod e2e;
mod explorer;
#[cfg(debug_assertions)]
mod gallery;
#[cfg(test)]
mod gpui_patches;
mod interface_zoom;
mod links;
mod menu;
mod notifications;
mod online;
mod pages;
mod palette;
mod profile;
mod setting_notice;
mod shell;
mod shortcuts;
mod startup;
mod theme;
mod updates;
mod whats_new;
mod window_state;
// Only the app itself stamps the shortcuts: a test run would change its runner's.

use gpui_kit::*;
use kchess_contracts::Level;
// The design system (`crates/kchess-ui`), under the names the app's modules use.
use kchess_ui::{chart, dates, motion, name_dialog, quick_pick};
// The app's services (`crates/kchess-app`), under the names the app's modules use.
use kchess_app::{
    accounts, loading, log, network, played, prefs, quit, reviews, settings, slow_calls, studies,
    timer,
};
// The board and the games on it (`crates/kchess-board`).
use kchess_board::{
    ai, board, engine_view, export, game, game_db, microphone, notation, player_search,
    position_engine, puzzle_db, rating_history, room, sound, voice, widgets,
};
// The pages as the shell drives them (`crates/kchess-nav`).
use kchess_nav::{part, play_layout, swipe};

fn main() {
    pages::register_pages();
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
    if updates::leave_bridge_folder() {
        return;
    }
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
        cx.set_global(kchess_ui::Logger(log::write));
        motion::init(cx);
        menu::init(cx);
        notifications::init(cx);
        if let Err(message) = core::start(data_dir.clone(), cx) {
            log::write(Level::Error, "app", &message);
            explain_and_quit(message, cx);
            return;
        }
        // From here on a quit removes the mark: an exit before it is not a crash.
        kchess_app::crashes::watch(&data_dir, &updates::current_version());
        cx.on_app_quit(quit::quit).detach();
        dock_icon::apply(cx);
        startup::start(cx);
        // Started at login in the background, KChess stays out of the way.
        if !cx.windows().is_empty() {
            cx.activate(true);
        }
    });
}

/// The models, from the profile the core has read (its settings and library read once for
/// them), then the window.
fn start_models(profile: startup::Profile, cx: &mut App) {
    let startup::Profile {
        settings,
        library,
        themes,
        view_preferences,
    } = profile;
    let library = &library;
    settings::init(settings, themes, cx);
    prefs::start(view_preferences, cx);
    kchess_app::notifications::init(cx);
    let profile_dir = profile::data_dir().unwrap_or_default();
    interface_zoom::init(&profile_dir, cx);
    window_state::init(&profile_dir, cx);
    played::init(library, cx);
    studies::init(library, cx);
    reviews::init(cx);
    pages::analysis::init(cx);
    cx.set_global(accounts::AccountHooks {
        connected: Some(online::account_connected),
        signed_out: Some(pages::puzzles::signed_out),
        restart_pages: Some(shell::restart_pages),
    });
    // A player's name opens their profile on the Players page.
    cx.set_global(player_search::OpenPlayer(pages::players::open_player));
    accounts::init(cx);
    ai::init(cx);
    puzzle_db::init(cx);
    game_db::init(cx);
    updates::init(cx);
    sound::init(cx);
    // Before the online state, which checks Lichess again when the network returns.
    network::init(cx);
    online::init(cx);
    notifications::alert_on_broadcasts(cx);
    voice::init(cx);
    cx.set_global(startup::Started);
    // After the online state and settings, whose state its menu shows.
    background::init(cx);
    if background::starts_hidden(cx) {
        background::went_to_background(cx);
    } else {
        open_main_window(cx);
    }
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
        window_bounds: Some(WindowBounds::centered(None, size(px(420.), px(160.)), cx)),
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

/// The window, where and as it was left.
fn main_window_options(cx: &mut App) -> WindowOptions {
    let (window_bounds, _) = window_state::bounds(cx);
    WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(profile::APP_NAME.into()),
            appears_transparent: true,
            // Beside the sidebar toggle; `interface_zoom::apply` keeps them on the
            // top bar's middle at every interface size.
            traffic_light_position: Some(point(px(12.), px(21.))),
        }),
        window_bounds,
        window_min_size: Some(size(
            px(window_state::MIN_SIZE.0),
            px(window_state::MIN_SIZE.1),
        )),
        app_id: Some("com.kchess.app".into()),
        // The window draws its own title bar, which moves and zooms it (`shell::title_bar`,
        // GPUI Kit's `TitleBar`); otherwise macOS answers a double click on its top 28px too,
        // zooming the window a second time, and delays clicks there to tell double ones apart.
        app_owns_titlebar_drag: true,
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
