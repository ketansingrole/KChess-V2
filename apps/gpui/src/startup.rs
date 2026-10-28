//! The startup screen. The app's models start from the settings and library
//! the core reads from the profile; when the core cannot read them, starting with defaults would
//! write them over the profile at the next save, so the window says why instead, with Try again.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::*;

use crate::assets::icon;
use crate::core::AppCore;

/// The reads every model starts from.
const READS: [&str; 2] = ["desktopSettings", "library"];

/// The models have started from the profile.
pub struct Started;

impl Global for Started {}

/// Whether the core can read what the models start from, or why not.
pub fn check(cx: &App) -> Result<(), String> {
    let bridge = &cx.global::<AppCore>().bridge;
    readable(|method| {
        bridge
            .call_now(method, Vec::new())
            .map(drop)
            .map_err(|cause| cause.message)
    })
}

fn readable(read: impl Fn(&str) -> Result<(), String>) -> Result<(), String> {
    READS.into_iter().try_for_each(read)
}

/// Start the models and open the window, or the startup screen when the profile cannot be read.
pub fn start(cx: &mut App) {
    match check(cx) {
        Ok(()) => crate::start_models(cx),
        Err(message) => show(message, cx),
    }
}

fn show(message: String, cx: &mut App) {
    crate::log::write(
        kchess_core::Level::Error,
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
            kchess_core::Level::Error,
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
            Ok(()) => {
                // The workspace opens in its own window where this one stood.
                window.remove_window();
                cx.defer(crate::start_models);
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
                    .font_weight(FontWeight::SEMIBOLD)
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
                Button::new("startup-retry")
                    .primary()
                    .icon(icon("rotate-cw"))
                    .label("Try again")
                    .on_click(cx.listener(|this, _, window, cx| this.retry(window, cx))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::readable;

    /// Settings the core could not read started as defaults, which the next save wrote over the
    /// profile's: any failed read must keep the models from starting.
    #[test]
    fn a_failed_read_keeps_the_models_from_starting() {
        assert_eq!(readable(|_| Ok(())), Ok(()));
        let failed = readable(|method| {
            if method == "desktopSettings" {
                Err("The settings file is locked".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(failed, Err("The settings file is locked".into()));
        assert!(
            readable(|method| if method == "library" {
                Err("x".into())
            } else {
                Ok(())
            })
            .is_err()
        );
    }
}
