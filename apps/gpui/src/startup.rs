//! The startup screen. The app's models start from the settings and library
//! the core reads from the profile; when the core cannot read them, starting with defaults would
//! write them over the profile at the next save, so the window says why instead, with Try again.

use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::*;
use serde_json::Value;

use crate::core::AppCore;
use kchess_contracts::{CoreError, calls};

/// The models have started from the profile.
pub struct Started;

impl Global for Started {}

/// The library the models start from, when the core can read what they need, or why not.
pub fn check(cx: &App) -> Result<Value, String> {
    let bridge = &cx.global::<AppCore>().bridge;
    readable(
        || bridge.request_now(calls::DesktopSettings {}).map(drop),
        || bridge.request_now(calls::Library {}),
    )
    .map_err(|cause| cause.message)
}

/// The settings must be readable, and the library is read once for every model that starts
/// from it: each read decodes every study and replays every played game.
fn readable(
    settings: impl FnOnce() -> Result<(), CoreError>,
    library: impl FnOnce() -> Result<Value, CoreError>,
) -> Result<Value, CoreError> {
    settings()?;
    library()
}

/// Start the models and open the window, or the startup screen when the profile cannot be read.
pub fn start(cx: &mut App) {
    match check(cx) {
        Ok(library) => crate::start_models(&library, cx),
        Err(message) => show(message, cx),
    }
}

fn show(message: String, cx: &mut App) {
    crate::log::write(
        kchess_contracts::Level::Error,
        "app",
        &format!("Loading the library failed: {message}"),
    );
    let options = crate::main_window_options(cx);
    let opened = gpui_kit::open_window(options, cx, |_, cx| {
        cx.new(|_| StartupScreen {
            error: message.into(),
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
}

impl StartupScreen {
    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match check(cx) {
            Ok(library) => {
                // The workspace opens in its own window where this one stood.
                window.remove_window();
                cx.defer(move |cx| crate::start_models(&library, cx));
            }
            Err(message) => {
                self.error = message.into();
                cx.notify();
            }
        }
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
            readable(|| Ok(()), || Ok(json!("library"))).map_err(|e| e.message),
            Ok(json!("library"))
        );
        let failed = readable(
            || Err(CoreError::new("The settings file is locked")),
            || Ok(Value::Null),
        );
        assert_eq!(
            failed.map_err(|e| e.message),
            Err("The settings file is locked".into())
        );
        assert!(readable(|| Ok(()), || Err(CoreError::new("x"))).is_err());
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
            || Err(CoreError::new("locked")),
            || {
                reads.set(reads.get() + 1);
                Ok(Value::Null)
            },
        );
        assert_eq!(reads.get(), 1);
    }
}
