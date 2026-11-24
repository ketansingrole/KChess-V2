//! The startup screen. The app's models start from the settings and library
//! the core reads from the profile; when the core cannot read them, starting with defaults would
//! write them over the profile at the next save, so the window says why instead, with Try again.

use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::*;
use kchess_contracts::types::{CustomThemeReport, Library, Settings};

use crate::core::AppCore;
use kchess_contracts::{CoreError, calls};

/// The models have started from the profile.
pub struct Started;

impl Global for Started {}

/// What the models start from, as the core read it from the profile. The custom themes and view
/// preferences may fail to read: the app then starts without them.
pub struct Profile {
    pub settings: Settings,
    pub library: Library,
    pub themes: Result<CustomThemeReport, CoreError>,
    pub view_preferences: Result<serde_json::Map<String, serde_json::Value>, CoreError>,
}

/// The settings and library the models start from, when the core can read them, or why not.
pub fn check(cx: &App) -> Result<Profile, String> {
    let bridge = &cx.global::<AppCore>().bridge;
    readable(
        || bridge.request_now(calls::DesktopSettings {}),
        || bridge.request_now(calls::Library {}),
    )
    .map(|(settings, library)| Profile {
        settings,
        library,
        themes: bridge.request_now(calls::CustomThemes {}),
        view_preferences: bridge.request_now(calls::ViewPreferences {}),
    })
    .map_err(|cause| cause.message)
}

/// `check`, for the startup screen's Try again: the core answers on its own threads, so the
/// window keeps drawing while it reads everything the models start from. 2026-11-24: the custom
/// themes and view preferences were read on the main thread once this answered, and a slow disk
/// froze the app with no window shown.
fn check_later(cx: &App) -> impl Future<Output = Result<Profile, String>> + 'static + use<> {
    let bridge = cx.global::<AppCore>().bridge.clone();
    async move {
        let settings = bridge.request(calls::DesktopSettings {}).await;
        let checked = match settings {
            Ok(settings) => match bridge.request(calls::Library {}).await {
                Ok(library) => Ok(Profile {
                    settings,
                    library,
                    themes: bridge.request(calls::CustomThemes {}).await,
                    view_preferences: bridge.request(calls::ViewPreferences {}).await,
                }),
                Err(cause) => Err(cause),
            },
            Err(cause) => Err(cause),
        };
        checked.map_err(|cause| cause.message)
    }
}

/// The settings must be readable, and each is read once for every model that starts from it:
/// Settings starts from these (it read them a second time before), and each library read
/// decodes every study and replays every played game.
fn readable<S, T>(
    settings: impl FnOnce() -> Result<S, CoreError>,
    library: impl FnOnce() -> Result<T, CoreError>,
) -> Result<(S, T), CoreError> {
    let settings = settings()?;
    Ok((settings, library()?))
}

/// Start the models and open the window, or the startup screen when the profile cannot be read.
pub fn start(cx: &mut App) {
    match check(cx) {
        Ok(profile) => crate::start_models(profile, cx),
        Err(message) => show(message, cx),
    }
}

fn show(message: String, cx: &mut App) {
    crate::log::write(
        kchess_contracts::Level::Error,
        "app",
        &format!("Loading the library failed: {message}"),
    );
    let options = WindowOptions {
        // It draws no title bar of its own, so macOS moves and zooms it.
        app_owns_titlebar_drag: false,
        ..crate::main_window_options(cx)
    };
    let opened = gpui_kit::open_window(options, cx, |_, cx| {
        cx.new(|_| StartupScreen {
            error: message.into(),
            checking: None,
        })
    });
    if let Err(cause) = opened {
        crate::log::write(
            kchess_contracts::Level::Error,
            "app",
            &format!("The window could not open: {cause}"),
        );
        cx.quit();
    }
}

struct StartupScreen {
    error: SharedString,
    /// Try again's check of the profile, while the core reads it.
    checking: Option<Task<()>>,
}

impl StartupScreen {
    /// Checks the profile again off the main thread: it once waited for the core's library read
    /// in the click, and the window froze until the answer came.
    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.checking.is_some() {
            return;
        }
        let checked = check_later(cx);
        self.checking = Some(cx.spawn_in(window, async move |this, cx| {
            let checked = checked.await;
            this.update_in(cx, |this, window, cx| {
                this.checking = None;
                match checked {
                    Ok(profile) => {
                        // The workspace opens in its own window where this one stood.
                        window.remove_window();
                        cx.defer(move |cx| crate::start_models(profile, cx));
                    }
                    Err(message) => {
                        this.error = message.into();
                        cx.notify();
                    }
                }
            })
            .ok();
        }));
        cx.notify();
    }
}

impl Render for StartupScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap(px(12.))
            .p(px(24.))
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(img("icon-splash.png").size(px(72.)))
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(FontWeight::MEDIUM)
                    .child("KChess couldn't start"),
            )
            .child(
                div()
                    .max_w(px(420.))
                    .text_center()
                    .text_size(px(14.))
                    .text_color(theme.muted_foreground)
                    .child(self.error.clone()),
            )
            .child(
                crate::widgets::try_again("startup-retry")
                    .loading(self.checking.is_some())
                    .on_click(cx.listener(|this, _, window, cx| this.retry(window, cx))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::readable;
    use kchess_contracts::CoreError;
    use serde_json::{Value, json};

    /// Settings the core could not read started as defaults, which the next save wrote over the
    /// profile's: any failed read must keep the models from starting.
    #[test]
    fn a_failed_read_keeps_the_models_from_starting() {
        assert_eq!(
            readable(|| Ok(json!("settings")), || Ok(json!("library"))).map_err(|e| e.message),
            Ok((json!("settings"), json!("library")))
        );
        let failed = readable(
            || Err::<(), _>(CoreError::new("The settings file is locked")),
            || Ok(Value::Null),
        );
        assert_eq!(
            failed.map_err(|e| e.message),
            Err("The settings file is locked".into())
        );
        assert!(readable(|| Ok(()), || Err::<(), _>(CoreError::new("x"))).is_err());
    }

    /// Try again returns at once and checks while the window keeps drawing: the click once waited
    /// for the core's library read, so the screen froze until it came.
    #[gpui_kit::test]
    fn try_again_checks_the_profile_without_waiting(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let mut app = crate::e2e::Harness::start(cx);
        let (window, screen) = app
            .cx
            .update(|cx| {
                gpui_kit::open_window(crate::main_window_options(cx), cx, |_, cx| {
                    cx.new(|_| super::StartupScreen {
                        error: "The settings file is locked".into(),
                        checking: None,
                    })
                })
            })
            .expect("the startup screen");
        let retry = |app: &mut crate::e2e::Harness| {
            app.cx
                .update_window(window, |_, window, cx| {
                    screen.update(cx, |screen, cx| {
                        screen.retry(window, cx);
                        screen.checking.is_some()
                    })
                })
                .expect("the screen stays open while it checks")
        };
        assert!(retry(&mut app), "the check is under way");
        // A second click starts no second check.
        assert!(retry(&mut app));
        // Closed before the answer: the check finds no window and opens nothing.
        app.cx
            .update_window(window, |_, window, _| window.remove_window())
            .expect("still open");
        app.settle();
    }

    /// 2026-11-24: after Try again answered, Settings read the custom themes and the view
    /// preferences read themselves on the main thread, so a slow disk froze the app with no window
    /// shown. Try again reads them with the rest, and the models start without waiting.
    #[gpui_kit::test]
    fn the_models_start_from_the_retried_check_without_waiting(cx: &mut gpui_kit::TestAppContext) {
        let mut app = crate::e2e::Harness::start(cx);
        let checked = app.cx.update(|cx| super::check_later(cx));
        let profile = app
            .cx
            .foreground_executor()
            .block_test(checked)
            .expect("the profile reads");
        assert!(profile.themes.is_ok(), "the custom themes are read");
        assert!(
            profile.view_preferences.is_ok(),
            "the view preferences are read"
        );
        app.cx.update(|cx| {
            // Fails naming any call that waits for the core.
            crate::slow_calls::drawing(|| {
                crate::settings::init(profile.settings, profile.themes, cx);
                crate::prefs::start(profile.view_preferences, cx);
            })
        });
        app.settle();
    }

    /// The library is read once to start, not once for each model that starts from it, and not
    /// at all when the settings cannot be read.
    #[test]
    fn the_library_is_read_once_to_start() {
        let reads = std::cell::Cell::new(0);
        readable(
            || Ok(()),
            || {
                reads.set(reads.get() + 1);
                Ok(Value::Null)
            },
        )
        .expect("readable");
        assert_eq!(reads.get(), 1);
        let _ = readable(
            || Err::<(), _>(CoreError::new("locked")),
            || {
                reads.set(reads.get() + 1);
                Ok(Value::Null)
            },
        );
        assert_eq!(reads.get(), 1);
    }
}
